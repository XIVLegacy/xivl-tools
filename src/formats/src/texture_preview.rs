//! Deterministic RGBA PNG previews for the top mip of eligible GTEX textures.

use png::{BitDepth, ColorType, Encoder};

use crate::digest::sha256_hex;
use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::{GtexFormat, GtexResource};
use crate::reader::Span;

/// The maximum decoded RGBA buffer permitted for one preview.
pub const MAX_PREVIEW_RGBA_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtexPngPreview {
    pub bytes: Vec<u8>,
    pub rgba_sha256: String,
    pub source_span: Span,
    pub source_sha256: String,
    pub format: GtexFormat,
    pub width: u32,
    pub height: u32,
    pub mip_level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPngRgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decode a preview PNG and require the canonical 8-bit RGBA representation.
pub fn decode_png_rgba(bytes: &[u8]) -> Result<DecodedPngRgba> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|error| {
        FormatError::new(
            ErrorKind::InvalidGtexPreview,
            0,
            format!("PNG preview header is invalid: {error}"),
        )
    })?;
    let header = reader.info();
    if header.color_type != ColorType::Rgba || header.bit_depth != BitDepth::Eight {
        return Err(invalid_preview("PNG preview must be 8-bit RGBA"));
    }
    let expected_rgba_len = checked_rgba_len(header.width, header.height)?;
    let output_size = reader.output_buffer_size().ok_or_else(|| {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            "PNG preview output size is unavailable",
        )
    })?;
    if output_size != expected_rgba_len
        || u64::try_from(output_size)
            .map(|size| size > MAX_PREVIEW_RGBA_BYTES)
            .unwrap_or(true)
    {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            format!("PNG preview RGBA size exceeds {MAX_PREVIEW_RGBA_BYTES}"),
        ));
    }
    let mut output = Vec::new();
    output.try_reserve_exact(output_size).map_err(|_| {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            "PNG preview output allocation failed",
        )
    })?;
    output.resize(output_size, 0);
    let info = reader.next_frame(&mut output).map_err(|error| {
        FormatError::new(
            ErrorKind::InvalidGtexPreview,
            0,
            format!("PNG preview image is invalid: {error}"),
        )
    })?;
    if info.color_type != ColorType::Rgba || info.bit_depth != BitDepth::Eight {
        return Err(invalid_preview("PNG preview must be 8-bit RGBA"));
    }
    if info.buffer_size() != expected_rgba_len {
        return Err(invalid_preview("PNG preview RGBA size is inconsistent"));
    }
    output.truncate(info.buffer_size());
    Ok(DecodedPngRgba {
        width: info.width,
        height: info.height,
        rgba: output,
    })
}

/// Decode and encode the logical top mip without materializing any other mip.
pub fn export_gtex_top_mip_png(data: &[u8], gtex: &GtexResource) -> Result<GtexPngPreview> {
    export_gtex_top_mip_png_from_source(data, gtex)
}

/// Decode a validated external-source GTEX descriptor without joining the
/// descriptor and surface buffers.
pub fn export_gtex_top_mip_png_external(
    source: &[u8],
    gtex: &GtexResource,
) -> Result<GtexPngPreview> {
    export_gtex_top_mip_png_from_source(source, gtex)
}

pub fn export_gtex_top_mip_png_from_source(
    data: &[u8],
    gtex: &GtexResource,
) -> Result<GtexPngPreview> {
    if let Some(reason) = preview_refusal(gtex) {
        return Err(FormatError::new(
            ErrorKind::UnsupportedGtexPreview,
            0,
            format!("GTEX cannot be previewed as PNG: {reason}"),
        ));
    }
    let format = gtex.format.ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnsupportedGtexPreview,
            0x06,
            "GTEX client format index is not mapped",
        )
    })?;
    let surface = gtex.surfaces.first().ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidGtexPreview,
            0x10,
            "GTEX surface table has no top mip",
        )
    })?;
    let start = usize::try_from(surface.source_span.offset).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            surface.source_span.offset,
            "GTEX top-mip offset does not fit this platform",
        )
    })?;
    let end_u64 = surface
        .source_span
        .offset
        .checked_add(surface.source_span.length)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                surface.source_span.offset,
                "GTEX top-mip span end overflows",
            )
        })?;
    let end = usize::try_from(end_u64).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            surface.source_span.offset,
            "GTEX top-mip end does not fit this platform",
        )
    })?;
    let encoded = data.get(start..end).ok_or_else(|| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            surface.source_span.offset,
            "GTEX top-mip span escapes the bounded input",
        )
    })?;
    let width = u32::from(gtex.width);
    let height = u32::from(gtex.height);
    let rgba_len = checked_rgba_len(width, height)?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(rgba_len).map_err(|_| {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0x0a,
            "GTEX PNG preview RGBA buffer allocation failed",
        )
    })?;
    rgba.resize(rgba_len, 0);
    decode_surface(format.index, width, height, encoded, &mut rgba)?;
    let rgba_sha256 = sha256_hex(&rgba);
    let bytes = encode_png_rgba(width, height, &rgba)?;
    Ok(GtexPngPreview {
        bytes,
        rgba_sha256,
        source_span: surface.source_span,
        source_sha256: sha256_hex(encoded),
        format,
        width,
        height,
        mip_level: surface.mip_level,
    })
}

/// Encode canonical noninterlaced 8-bit RGBA PNG bytes.
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let expected = checked_rgba_len(width, height)?;
    if rgba.len() != expected {
        return Err(invalid_preview("RGBA buffer size is inconsistent"));
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| {
            FormatError::new(
                ErrorKind::PngEncodingFailed,
                0,
                format!("PNG header encoding failed: {error}"),
            )
        })?;
        writer.write_image_data(rgba).map_err(|error| {
            FormatError::new(
                ErrorKind::PngEncodingFailed,
                0,
                format!("PNG image encoding failed: {error}"),
            )
        })?;
    }
    Ok(bytes)
}

fn preview_refusal(gtex: &GtexResource) -> Option<&'static str> {
    if let Some(reason) = gtex.materialization_refusal() {
        return Some(reason);
    }
    if u32::from(gtex.mip_levels) > max_mip_levels(u32::from(gtex.width), u32::from(gtex.height)) {
        return Some("GTEX mip count exceeds the dimension-derived preview limit");
    }
    None
}

fn max_mip_levels(width: u32, height: u32) -> u32 {
    let max = width.max(height);
    if max == 0 {
        0
    } else {
        u32::BITS - max.leading_zeros()
    }
}

fn checked_rgba_len(width: u32, height: u32) -> Result<usize> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::ResourceLimitExceeded,
                0x0a,
                "GTEX PNG preview RGBA size overflows",
            )
        })?;
    if bytes > MAX_PREVIEW_RGBA_BYTES {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0x0a,
            format!("GTEX PNG preview RGBA size {bytes} exceeds {MAX_PREVIEW_RGBA_BYTES}"),
        ));
    }
    usize::try_from(bytes).map_err(|_| {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0x0a,
            "GTEX PNG preview RGBA size does not fit this platform",
        )
    })
}

fn decode_surface(
    format_index: u8,
    width: u32,
    height: u32,
    encoded: &[u8],
    rgba: &mut [u8],
) -> Result<()> {
    match format_index {
        4 => decode_a8r8g8b8(width, height, encoded, rgba),
        24 => decode_dxt1(width, height, encoded, rgba),
        26 => decode_dxt5(width, height, encoded, rgba),
        _ => Err(FormatError::new(
            ErrorKind::UnsupportedGtexPreview,
            0x06,
            "GTEX format is outside the PNG preview subset",
        )),
    }
}

fn decode_a8r8g8b8(width: u32, height: u32, encoded: &[u8], rgba: &mut [u8]) -> Result<()> {
    let expected = checked_rgba_len(width, height)?;
    if encoded.len() != expected {
        return Err(invalid_preview("A8R8G8B8 surface size is inconsistent"));
    }
    for (source, destination) in encoded
        .as_chunks::<4>()
        .0
        .iter()
        .zip(rgba.as_chunks_mut::<4>().0)
    {
        destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
    }
    Ok(())
}

fn decode_dxt1(width: u32, height: u32, encoded: &[u8], rgba: &mut [u8]) -> Result<()> {
    decode_blocks(width, height, encoded, rgba, 8, decode_dxt1_block)
}

fn decode_dxt5(width: u32, height: u32, encoded: &[u8], rgba: &mut [u8]) -> Result<()> {
    decode_blocks(width, height, encoded, rgba, 16, decode_dxt5_block)
}

fn decode_blocks(
    width: u32,
    height: u32,
    encoded: &[u8],
    rgba: &mut [u8],
    block_bytes: usize,
    decode: fn(&[u8], &mut [[u8; 4]; 16]),
) -> Result<()> {
    let blocks_w = usize::try_from(width.div_ceil(4))
        .map_err(|_| invalid_preview("block width does not fit"))?;
    let blocks_h = usize::try_from(height.div_ceil(4))
        .map_err(|_| invalid_preview("block height does not fit"))?;
    let expected = blocks_w
        .checked_mul(blocks_h)
        .and_then(|count| count.checked_mul(block_bytes))
        .ok_or_else(|| invalid_preview("compressed surface size overflows"))?;
    if encoded.len() != expected {
        return Err(invalid_preview("compressed surface size is inconsistent"));
    }
    let width_usize = usize::try_from(width).map_err(|_| invalid_preview("width does not fit"))?;
    let height_usize =
        usize::try_from(height).map_err(|_| invalid_preview("height does not fit"))?;
    let mut colors = [[0u8; 4]; 16];
    for block_y in 0..blocks_h {
        for block_x in 0..blocks_w {
            let index = (block_y * blocks_w + block_x) * block_bytes;
            decode(&encoded[index..index + block_bytes], &mut colors);
            for local_y in 0..4 {
                for local_x in 0..4 {
                    let x = block_x * 4 + local_x;
                    let y = block_y * 4 + local_y;
                    if x < width_usize && y < height_usize {
                        let destination = (y * width_usize + x) * 4;
                        rgba[destination..destination + 4]
                            .copy_from_slice(&colors[local_y * 4 + local_x]);
                    }
                }
            }
        }
    }
    Ok(())
}

fn decode_dxt1_block(block: &[u8], output: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let color0 = rgb565(c0);
    let color1 = rgb565(c1);
    let mut colors = [[0u8; 4]; 4];
    colors[0] = [color0[0], color0[1], color0[2], 255];
    colors[1] = [color1[0], color1[1], color1[2], 255];
    if c0 > c1 {
        colors[2] = [
            mix(color0[0], color1[0], 2, 1, 3),
            mix(color0[1], color1[1], 2, 1, 3),
            mix(color0[2], color1[2], 2, 1, 3),
            255,
        ];
        colors[3] = [
            mix(color0[0], color1[0], 1, 2, 3),
            mix(color0[1], color1[1], 1, 2, 3),
            mix(color0[2], color1[2], 1, 2, 3),
            255,
        ];
    } else {
        colors[2] = [
            average(color0[0], color1[0]),
            average(color0[1], color1[1]),
            average(color0[2], color1[2]),
            255,
        ];
        colors[3] = [0, 0, 0, 0];
    }
    select_colors(block, &colors, output);
}

fn decode_dxt5_block(block: &[u8], output: &mut [[u8; 4]; 16]) {
    let c0 = u16::from_le_bytes([block[8], block[9]]);
    let c1 = u16::from_le_bytes([block[10], block[11]]);
    let color0 = rgb565(c0);
    let color1 = rgb565(c1);
    let mut colors = [[0u8; 4]; 4];
    colors[0] = [color0[0], color0[1], color0[2], 255];
    colors[1] = [color1[0], color1[1], color1[2], 255];
    colors[2] = [
        mix(color0[0], color1[0], 2, 1, 3),
        mix(color0[1], color1[1], 2, 1, 3),
        mix(color0[2], color1[2], 2, 1, 3),
        255,
    ];
    colors[3] = [
        mix(color0[0], color1[0], 1, 2, 3),
        mix(color0[1], color1[1], 1, 2, 3),
        mix(color0[2], color1[2], 1, 2, 3),
        255,
    ];
    let mut alpha = [0u8; 8];
    alpha[0] = block[0];
    alpha[1] = block[1];
    if alpha[0] > alpha[1] {
        for i in 2..8u8 {
            alpha[usize::from(i)] = (((8 - u16::from(i)) * u16::from(alpha[0])
                + (u16::from(i) - 1) * u16::from(alpha[1])
                + 3)
                / 7) as u8;
        }
    } else {
        for i in 2..6u8 {
            alpha[usize::from(i)] = (((6 - u16::from(i)) * u16::from(alpha[0])
                + (u16::from(i) - 1) * u16::from(alpha[1])
                + 2)
                / 5) as u8;
        }
        alpha[6] = 0;
        alpha[7] = 255;
    }
    let selectors = u64::from_le_bytes([
        block[2], block[3], block[4], block[5], block[6], block[7], 0, 0,
    ]);
    let color_selectors = &block[12..16];
    for index in 0..16 {
        let alpha_index = ((selectors >> (index * 3)) & 7) as usize;
        let color_index = usize::from((color_selectors[index / 4] >> ((index % 4) * 2)) & 3);
        output[index] = [
            colors[color_index][0],
            colors[color_index][1],
            colors[color_index][2],
            alpha[alpha_index],
        ];
    }
}

fn select_colors(block: &[u8], colors: &[[u8; 4]; 4], output: &mut [[u8; 4]; 16]) {
    for index in 0..16 {
        output[index] = colors[usize::from((block[4 + index / 4] >> ((index % 4) * 2)) & 3)];
    }
}

fn rgb565(value: u16) -> [u8; 3] {
    let red = (value >> 11) & 0x1f;
    let green = (value >> 5) & 0x3f;
    let blue = value & 0x1f;
    [
        ((red << 3) | (red >> 2)) as u8,
        ((green << 2) | (green >> 4)) as u8,
        ((blue << 3) | (blue >> 2)) as u8,
    ]
}

fn mix(first: u8, second: u8, first_weight: u16, second_weight: u16, denominator: u16) -> u8 {
    ((u16::from(first) * first_weight + u16::from(second) * second_weight + denominator / 2)
        / denominator) as u8
}

fn average(first: u8, second: u8) -> u8 {
    ((u16::from(first) + u16::from(second)) / 2) as u8
}

fn invalid_preview(detail: &str) -> FormatError {
    FormatError::new(ErrorKind::InvalidGtexPreview, 0, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_rgb565_with_bit_replication() {
        assert_eq!(rgb565(0xffff), [255, 255, 255]);
        assert_eq!(rgb565(0x07e0), [0, 255, 0]);
    }

    #[test]
    fn a8r8g8b8_reorders_bgra_without_touching_alpha() {
        let mut rgba = [0u8; 4];
        decode_a8r8g8b8(1, 1, &[0x11, 0x22, 0x33, 0x00], &mut rgba).unwrap();
        assert_eq!(rgba, [0x33, 0x22, 0x11, 0x00]);
    }

    #[test]
    fn dxt1_four_color_mode_uses_rounded_interpolation() {
        let mut block = [0u8; 8];
        block[0..2].copy_from_slice(&0xF800u16.to_le_bytes());
        block[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        block[4] = 0xE4;
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt1_block(&block, &mut pixels);
        assert_eq!(pixels[0], [255, 0, 0, 255]);
        assert_eq!(pixels[1], [0, 0, 255, 255]);
        assert_eq!(pixels[2], [170, 0, 85, 255]);
        assert_eq!(pixels[3], [85, 0, 170, 255]);
    }

    #[test]
    fn dxt1_four_color_mode_uses_literal_rgb565_rounding() {
        let mut block = [0u8; 8];
        block[0..2].copy_from_slice(&0x2244u16.to_le_bytes());
        block[2..4].copy_from_slice(&0x0000u16.to_le_bytes());
        block[4] = 0xE4;
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt1_block(&block, &mut pixels);
        assert_eq!(
            &pixels[..4],
            &[
                [33, 73, 33, 255],
                [0, 0, 0, 255],
                [22, 49, 22, 255],
                [11, 24, 11, 255],
            ]
        );
    }

    #[test]
    fn dxt1_three_color_mode_uses_transparent_selector() {
        let mut block = [0u8; 8];
        block[0..2].copy_from_slice(&0x001Fu16.to_le_bytes());
        block[2..4].copy_from_slice(&0xF800u16.to_le_bytes());
        block[4] = 0xE4;
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt1_block(&block, &mut pixels);
        assert_eq!(pixels[0], [0, 0, 255, 255]);
        assert_eq!(pixels[2], [127, 0, 127, 255]);
        assert_eq!(pixels[3], [0, 0, 0, 0]);
    }

    #[test]
    fn dxt1_equal_endpoints_select_three_color_mode() {
        let mut block = [0u8; 8];
        block[0..2].copy_from_slice(&0x07E0u16.to_le_bytes());
        block[2..4].copy_from_slice(&0x07E0u16.to_le_bytes());
        block[4] = 0xE4;
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt1_block(&block, &mut pixels);
        assert_eq!(pixels[2], [0, 255, 0, 255]);
        assert_eq!(pixels[3], [0, 0, 0, 0]);
    }

    #[test]
    fn dxt1_crops_partial_edges_after_decoding_complete_blocks() {
        let mut encoded = vec![0u8; 16];
        encoded[0..2].copy_from_slice(&0xF800u16.to_le_bytes());
        encoded[2..4].copy_from_slice(&0x001Fu16.to_le_bytes());
        encoded[8..10].copy_from_slice(&0x07E0u16.to_le_bytes());
        encoded[10..12].copy_from_slice(&0x0000u16.to_le_bytes());
        let mut rgba = vec![0u8; 5 * 3 * 4];
        decode_dxt1(5, 3, &encoded, &mut rgba).unwrap();
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(
            &rgba[(2 * 5 + 4) * 4..(2 * 5 + 4) * 4 + 4],
            &[0, 255, 0, 255]
        );
    }

    #[test]
    fn dxt5_alpha_endpoints_and_special_values_are_literal() {
        let mut block = [0u8; 16];
        block[0] = 10;
        block[1] = 10;
        block[2..8].copy_from_slice(&0xFFFF_FFFF_FFFFu64.to_le_bytes()[..6]);
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt5_block(&block, &mut pixels);
        assert_eq!(pixels[0][3], 255);
    }

    #[test]
    fn dxt5_alpha_interpolation_covers_both_branches_and_special_indices() {
        let selectors = (0..8u64).fold(0u64, |value, index| value | (index << (index * 3)));
        let mut interpolated = [0u8; 16];
        interpolated[0] = 255;
        interpolated[1] = 0;
        interpolated[2..8].copy_from_slice(&selectors.to_le_bytes()[..6]);
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt5_block(&interpolated, &mut pixels);
        assert_eq!(
            (0..8).map(|index| pixels[index][3]).collect::<Vec<_>>(),
            vec![255, 0, 219, 182, 146, 109, 73, 36]
        );

        let mut six_mode = interpolated;
        six_mode[0] = 17;
        six_mode[1] = 17;
        decode_dxt5_block(&six_mode, &mut pixels);
        assert_eq!(pixels[0][3], 17);
        assert_eq!(pixels[5][3], 17);
        assert_eq!(pixels[6][3], 0);
        assert_eq!(pixels[7][3], 255);
    }

    #[test]
    fn dxt5_six_alpha_mode_uses_literal_interpolation_rounding() {
        let selectors = (0..8u64).fold(0u64, |value, index| value | (index << (index * 3)));
        let mut block = [0u8; 16];
        block[0] = 10;
        block[1] = 213;
        block[2..8].copy_from_slice(&selectors.to_le_bytes()[..6]);
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt5_block(&block, &mut pixels);
        assert_eq!(
            (0..8).map(|index| pixels[index][3]).collect::<Vec<_>>(),
            vec![10, 213, 51, 91, 132, 172, 0, 255]
        );
    }

    #[test]
    fn dxt5_uses_four_color_mode_even_when_endpoints_are_reversed() {
        let mut block = [0u8; 16];
        block[0] = 0;
        block[1] = 0;
        block[8..10].copy_from_slice(&0x001Fu16.to_le_bytes());
        block[10..12].copy_from_slice(&0xF800u16.to_le_bytes());
        block[12] = 3;
        let mut pixels = [[0u8; 4]; 16];
        decode_dxt5_block(&block, &mut pixels);
        assert_eq!(pixels[0], [170, 0, 85, 0]);
    }
}
