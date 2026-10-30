//! Deterministic GTEX pixel encoding and box-filtered RGBA mip generation.

use crate::dds::max_mip_levels;
use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::gtex_format;

/// Encode one RGBA8 surface (row-major, 4 bytes per pixel) into the format's exact byte layout and size.
pub fn encode_surface(format_index: u8, width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let format = gtex_format(format_index)
        .ok_or_else(|| invalid("GTEX client format index is not mapped"))?;
    validate_rgba(width, height, rgba)?;
    let size = if let Some(block_bytes) = format.block_bytes {
        u64::from(width.div_ceil(4)) * u64::from(height.div_ceil(4)) * u64::from(block_bytes)
    } else {
        (rgba.len() / 4) as u64 * u64::from(format.bits_per_pixel / 8)
    };
    let mut encoded = buffer(size)?;
    match format_index {
        4 => {
            for (pixel, output) in rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(encoded.as_chunks_mut::<4>().0.iter_mut())
            {
                output.copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        22 => {
            for (pixel, output) in rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(encoded.as_chunks_mut::<2>().0.iter_mut())
            {
                let q = |channel: usize| (u16::from(pixel[channel]) * 15 + 127) / 255;
                let value = q(2) | (q(1) << 4) | (q(0) << 8) | (q(3) << 12);
                output.copy_from_slice(&value.to_le_bytes());
            }
        }
        _ => {
            let width = width as usize;
            let height = height as usize;
            let block_bytes = if format_index == 24 { 8 } else { 16 };
            for (index, output) in encoded.chunks_exact_mut(block_bytes).enumerate() {
                let block_x = index % width.div_ceil(4);
                let block_y = index / width.div_ceil(4);
                let mut pixels = [[0; 4]; 16];
                for (i, pixel) in pixels.iter_mut().enumerate() {
                    let x = (block_x * 4 + i % 4).min(width - 1);
                    let y = (block_y * 4 + i / 4).min(height - 1);
                    pixel.copy_from_slice(&rgba[(y * width + x) * 4..(y * width + x) * 4 + 4]);
                }
                if format_index == 24 {
                    output
                        .copy_from_slice(&encode_color(&pixels, pixels.iter().any(|p| p[3] < 128)));
                } else {
                    output[..8].copy_from_slice(&encode_alpha(&pixels));
                    output[8..].copy_from_slice(&encode_color(&pixels, false));
                }
            }
        }
    }
    Ok(encoded)
}

/// The top level followed by `levels - 1` box-filtered levels, each halving and clamping to 1.
/// Samples are averaged with round-half-up in stored values, without gamma correction.
pub fn mip_chain_rgba(
    width: u32,
    height: u32,
    rgba: &[u8],
    levels: u8,
) -> Result<Vec<(u32, u32, Vec<u8>)>> {
    validate_rgba(width, height, rgba)?;
    if levels == 0 || u32::from(levels) > max_mip_levels(width, height) {
        return Err(invalid("mip count is outside the dimension-derived chain"));
    }
    let mut top = buffer(rgba.len() as u64)?;
    top.copy_from_slice(rgba);
    let mut chain = vec![(width, height, top)];
    for _ in 1..levels {
        let (width, height, source) = &chain[chain.len() - 1];
        let (next_w, next_h) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = buffer(u64::from(next_w) * u64::from(next_h) * 4)?;
        for y in 0..next_h {
            for x in 0..next_w {
                let mut sum = [0u16; 4];
                let mut count = 0;
                for sy in y * 2..(y * 2 + 2).min(*height) {
                    for sx in x * 2..(x * 2 + 2).min(*width) {
                        let offset =
                            (u64::from(sy) * u64::from(*width) + u64::from(sx)) as usize * 4;
                        for channel in 0..4 {
                            sum[channel] += u16::from(source[offset + channel]);
                        }
                        count += 1;
                    }
                }
                let offset = (u64::from(y) * u64::from(next_w) + u64::from(x)) as usize * 4;
                for channel in 0..4 {
                    next[offset + channel] = ((sum[channel] + count / 2) / count) as u8;
                }
            }
        }
        chain.push((next_w, next_h, next));
    }
    Ok(chain)
}

fn invalid(detail: &str) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, 0, detail)
}

fn validate_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(invalid("RGBA dimensions must be nonzero"));
    }
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|n| n.checked_mul(4));
    if expected != Some(rgba.len() as u64) {
        return Err(invalid("RGBA length must equal width * height * 4"));
    }
    Ok(())
}

fn buffer(size: u64) -> Result<Vec<u8>> {
    let failure = || {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            0,
            "texture allocation failed",
        )
    };
    let size = usize::try_from(size).map_err(|_| failure())?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(|_| failure())?;
    bytes.resize(size, 0);
    Ok(bytes)
}

fn expand565(value: u16) -> [u8; 3] {
    let (r, g, b) = (
        (value >> 11) as u8,
        ((value >> 5) & 63) as u8,
        (value & 31) as u8,
    );
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

fn nearest565(color: [f64; 3]) -> u16 {
    let channel = |value: f64, bits: u16| {
        (0..1u16 << bits)
            .min_by(|a, b| {
                let expand = |v: u16| {
                    if bits == 5 {
                        (v << 3) | (v >> 2)
                    } else {
                        (v << 2) | (v >> 4)
                    }
                };
                (f64::from(expand(*a)) - value)
                    .abs()
                    .total_cmp(&(f64::from(expand(*b)) - value).abs())
            })
            .unwrap_or(0)
    };
    (channel(color[0], 5) << 11) | (channel(color[1], 6) << 5) | channel(color[2], 5)
}

fn palette(c0: u16, c1: u16, transparent: bool) -> [[u8; 3]; 4] {
    let mut colors = [expand565(c0), expand565(c1), [0; 3], [0; 3]];
    colors[2] = std::array::from_fn(|channel| {
        let (a, b) = (u16::from(colors[0][channel]), u16::from(colors[1][channel]));
        if transparent {
            ((a + b) / 2) as u8
        } else {
            ((2 * a + b + 1) / 3) as u8
        }
    });
    if !transparent {
        colors[3] = std::array::from_fn(|channel| {
            ((u16::from(colors[0][channel]) + 2 * u16::from(colors[1][channel]) + 1) / 3) as u8
        });
    }
    colors
}

fn color_candidate(
    pixels: &[[u8; 4]; 16],
    mut c0: u16,
    mut c1: u16,
    transparent: bool,
) -> (u64, [u8; 8]) {
    if (transparent && c0 > c1) || (!transparent && c0 < c1) {
        std::mem::swap(&mut c0, &mut c1);
    }
    let solid_selector = (!transparent && c0 == c1).then_some(if c0 == 0 { 1 } else { 0 });
    if solid_selector.is_some() {
        if c0 == 0 {
            c0 = 1;
        } else {
            c1 -= 1;
        }
    }
    let colors = palette(c0, c1, transparent);
    let mut selectors = 0u32;
    let mut error = 0;
    for (i, pixel) in pixels.iter().enumerate() {
        let selector = if transparent && pixel[3] < 128 {
            3
        } else {
            let (index, distance) = colors[..if transparent { 3 } else { 4 }]
                .iter()
                .enumerate()
                .filter(|&(index, _)| solid_selector.is_none_or(|solid| index == solid))
                .map(|(index, color)| {
                    let distance: u64 = (0..3)
                        .map(|c| i64::from(pixel[c]).abs_diff(i64::from(color[c])).pow(2))
                        .sum();
                    (index, distance)
                })
                .min_by_key(|&(index, distance)| (distance, index))
                .unwrap();
            error += distance;
            index
        };
        selectors |= (selector as u32) << (i * 2);
    }
    let mut block = [0; 8];
    block[..2].copy_from_slice(&c0.to_le_bytes());
    block[2..4].copy_from_slice(&c1.to_le_bytes());
    block[4..].copy_from_slice(&selectors.to_le_bytes());
    (error, block)
}

fn principal_endpoints(pixels: &[[u8; 4]; 16], transparent: bool) -> ([f64; 3], [f64; 3]) {
    let active = |p: &&[u8; 4]| !transparent || p[3] >= 128;
    let mut mean = [0.; 3];
    let count = pixels.iter().filter(active).count().max(1) as f64;
    for pixel in pixels.iter().filter(active) {
        for c in 0..3 {
            mean[c] += f64::from(pixel[c]) / count;
        }
    }
    let mut covariance = [[0.; 3]; 3];
    for pixel in pixels.iter().filter(active) {
        for i in 0..3 {
            for j in 0..3 {
                covariance[i][j] +=
                    (f64::from(pixel[i]) - mean[i]) * (f64::from(pixel[j]) - mean[j]);
            }
        }
    }
    let mut axis = [0.; 3];
    let largest = (0..3)
        .max_by(|&a, &b| covariance[a][a].total_cmp(&covariance[b][b]))
        .unwrap();
    axis[largest] = 1.;
    for _ in 0..8 {
        let next = covariance.map(|row| row.iter().zip(axis).map(|(a, b)| a * b).sum::<f64>());
        let scale = next.iter().map(|v| v.abs()).fold(0., f64::max);
        if scale > 0. {
            axis = next.map(|v| v / scale);
        }
    }
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut first, mut last) = (mean, mean);
    for pixel in pixels.iter().filter(active) {
        let projection: f64 = (0..3)
            .map(|c| (f64::from(pixel[c]) - mean[c]) * axis[c])
            .sum();
        if projection < low {
            low = projection;
            first = [pixel[0].into(), pixel[1].into(), pixel[2].into()];
        }
        if projection > high {
            high = projection;
            last = [pixel[0].into(), pixel[1].into(), pixel[2].into()];
        }
    }
    (first, last)
}

fn encode_color(pixels: &[[u8; 4]; 16], transparent: bool) -> [u8; 8] {
    let (first, last) = principal_endpoints(pixels, transparent);
    if first == last {
        let color = nearest565(first);
        return color_candidate(pixels, color, color, transparent).1;
    }
    let mut current = color_candidate(pixels, nearest565(first), nearest565(last), transparent);
    let mut best = current;
    for _ in 0..4 {
        let selectors = u32::from_le_bytes(current.1[4..].try_into().unwrap());
        let (mut aa, mut ab, mut bb) = (0., 0., 0.);
        let (mut ax, mut bx) = ([0.; 3], [0.; 3]);
        for (i, pixel) in pixels.iter().enumerate() {
            if transparent && pixel[3] < 128 {
                continue;
            }
            let index = (selectors >> (2 * i)) & 3;
            let (a, b) = match index {
                0 => (1., 0.),
                1 => (0., 1.),
                2 if transparent => (0.5, 0.5),
                2 => (2. / 3., 1. / 3.),
                _ => (1. / 3., 2. / 3.),
            };
            aa += a * a;
            ab += a * b;
            bb += b * b;
            for c in 0..3 {
                ax[c] += a * f64::from(pixel[c]);
                bx[c] += b * f64::from(pixel[c]);
            }
        }
        let determinant: f64 = aa * bb - ab * ab;
        if determinant.abs() < 1e-9 {
            break;
        }
        let first =
            std::array::from_fn(|c| ((ax[c] * bb - bx[c] * ab) / determinant).clamp(0., 255.));
        let last =
            std::array::from_fn(|c| ((bx[c] * aa - ax[c] * ab) / determinant).clamp(0., 255.));
        current = color_candidate(pixels, nearest565(first), nearest565(last), transparent);
        if current < best {
            best = current;
        }
    }
    best.1
}

fn alpha_candidate(pixels: &[[u8; 4]; 16], a0: u8, a1: u8) -> (u64, [u8; 8]) {
    let mut palette = [a0, a1, 0, 0, 0, 0, 0, 255];
    let denominator = if a0 > a1 { 7 } else { 5 };
    for (i, alpha) in palette.iter_mut().enumerate().take(denominator + 1).skip(2) {
        *alpha = (((denominator + 1 - i) as u16 * u16::from(a0)
            + (i - 1) as u16 * u16::from(a1)
            + denominator as u16 / 2)
            / denominator as u16) as u8;
    }
    let mut selectors = 0u64;
    let mut error = 0;
    for (i, pixel) in pixels.iter().enumerate() {
        let (index, distance) = palette
            .iter()
            .enumerate()
            .map(|(index, alpha)| {
                (
                    index,
                    i64::from(*alpha).abs_diff(i64::from(pixel[3])).pow(2),
                )
            })
            .min_by_key(|&(index, distance)| (distance, index))
            .unwrap();
        error += distance;
        selectors |= (index as u64) << (i * 3);
    }
    let mut block = selectors.to_le_bytes();
    block.copy_within(..6, 2);
    block[0] = a0;
    block[1] = a1;
    (error, block)
}

fn encode_alpha(pixels: &[[u8; 4]; 16]) -> [u8; 8] {
    let low = pixels.iter().map(|p| p[3]).min().unwrap();
    let high = pixels.iter().map(|p| p[3]).max().unwrap();
    let inner_low = pixels
        .iter()
        .map(|p| p[3])
        .filter(|&a| a > 0)
        .min()
        .unwrap_or(0);
    let inner_high = pixels
        .iter()
        .map(|p| p[3])
        .filter(|&a| a < 255)
        .max()
        .unwrap_or(255);
    let mut best = alpha_candidate(pixels, high, low);
    for (a0, a1) in [(low, high), (inner_low, inner_high)] {
        let candidate = alpha_candidate(pixels, a0, a1);
        if candidate < best {
            best = candidate;
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::texture_preview::decode_surface;

    fn round_trip(format: u8, width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let encoded = encode_surface(format, width, height, pixels).unwrap();
        assert_eq!(
            encoded,
            encode_surface(format, width, height, pixels).unwrap()
        );
        let mut decoded = vec![0; pixels.len()];
        decode_surface(format, width, height, &encoded, &mut decoded).unwrap();
        decoded
    }

    #[test]
    fn uncompressed_layout_quantization_and_round_trips() {
        assert_eq!(
            encode_surface(4, 1, 1, &[1, 2, 3, 4]).unwrap(),
            [3, 2, 1, 4]
        );
        assert_eq!(
            encode_surface(22, 1, 1, &[17, 34, 51, 68]).unwrap(),
            [0x23, 0x41]
        );
        let bytes: Vec<_> = (0..=255u8)
            .flat_map(|v| {
                [
                    v,
                    v.wrapping_add(71),
                    v.wrapping_add(133),
                    v.wrapping_add(191),
                ]
            })
            .collect();
        assert_eq!(round_trip(4, 16, 16, &bytes), bytes);
        let expanded: Vec<_> = (0..=u16::MAX)
            .flat_map(|v| {
                [
                    ((v >> 8) & 15) as u8 * 17,
                    ((v >> 4) & 15) as u8 * 17,
                    (v & 15) as u8 * 17,
                    (v >> 12) as u8 * 17,
                ]
            })
            .collect();
        assert_eq!(round_trip(22, 256, 256, &expanded), expanded);
        let quantized = round_trip(22, 16, 16, &bytes);
        for (&input, &output) in bytes.iter().zip(&quantized) {
            let nearest = (0..=15u16)
                .min_by_key(|&n| (i32::from(input) - i32::from(n * 17)).abs())
                .unwrap();
            assert_eq!(u16::from(output), nearest * 17);
        }
    }

    #[test]
    fn solid_blocks_choose_nearest_rgb565() {
        for value in 0..=255u8 {
            let input = [
                value,
                value.wrapping_mul(71),
                value.wrapping_mul(133),
                value,
            ];
            let nearest = |sample: u8, bits: u32| {
                (0..1u16 << bits)
                    .map(|n| {
                        if bits == 5 {
                            ((n << 3) | (n >> 2)) as u8
                        } else {
                            ((n << 2) | (n >> 4)) as u8
                        }
                    })
                    .min_by_key(|&expanded| i16::from(sample).abs_diff(i16::from(expanded)))
                    .unwrap()
            };
            for format in [24, 26] {
                let mut pixel = input;
                if format == 24 {
                    pixel[3] = 255;
                }
                let decoded = round_trip(format, 4, 4, &pixel.repeat(16));
                let expected = [
                    nearest(pixel[0], 5),
                    nearest(pixel[1], 6),
                    nearest(pixel[2], 5),
                    pixel[3],
                ];
                assert_eq!(
                    decoded,
                    expected.repeat(16),
                    "format {format}, input {pixel:?}"
                );
            }
        }
    }

    #[test]
    fn dxt1_transparency_and_partial_blocks() {
        let pixels: Vec<_> = (0..35)
            .flat_map(|i| [197, 39, 81, if i % 3 == 0 { 127 } else { 128 }])
            .collect();
        let encoded = encode_surface(24, 7, 5, &pixels).unwrap();
        assert_eq!(encoded.len(), 32);
        for block in encoded.as_chunks::<8>().0.iter() {
            assert!(
                u16::from_le_bytes([block[0], block[1]])
                    <= u16::from_le_bytes([block[2], block[3]])
            );
        }
        let decoded = round_trip(24, 7, 5, &pixels);
        for (input, output) in pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(decoded.as_chunks::<4>().0.iter())
        {
            assert_eq!(output[3], if input[3] < 128 { 0 } else { 255 });
            if input[3] < 128 {
                assert_eq!(*output, [0; 4]);
            }
        }
        assert_eq!(round_trip(24, 1, 1, &[9, 77, 203, 0]), [0; 4]);
        for format in [24, 26] {
            let small = [31, 133, 249, 255, 71, 89, 37, 255];
            let padded: Vec<_> = (0..4)
                .flat_map(|_| [small[..4].to_vec(), small[4..].repeat(3)].concat())
                .collect();
            assert_eq!(
                encode_surface(format, 2, 1, &small).unwrap(),
                encode_surface(format, 4, 4, &padded).unwrap()
            );
        }
    }

    #[test]
    fn dxt5_selects_both_alpha_modes() {
        for (alphas, eight) in [
            ([240, 30, 210, 180, 150, 120, 90, 60], true),
            ([60, 160, 80, 100, 120, 140, 0, 255], false),
        ] {
            let pixels: Vec<_> = alphas
                .repeat(2)
                .iter()
                .flat_map(|&a| [33, 77, 199, a])
                .collect();
            let encoded = encode_surface(26, 4, 4, &pixels).unwrap();
            assert_eq!(encoded[0] > encoded[1], eight);
            assert!(
                u16::from_le_bytes([encoded[8], encoded[9]])
                    > u16::from_le_bytes([encoded[10], encoded[11]])
            );
            let decoded = round_trip(26, 4, 4, &pixels);
            for (input, output) in pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(decoded.as_chunks::<4>().0.iter())
            {
                assert_eq!(input[3], output[3]);
            }
        }
    }

    #[test]
    fn mip_dimensions_samples_and_rounding() {
        let input: Vec<_> = (0..15).flat_map(|v| [v, 0, 0, 255]).collect();
        let chain = mip_chain_rgba(3, 5, &input, 3).unwrap();
        assert_eq!(
            chain.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(),
            [(3, 5), (1, 2), (1, 1)]
        );
        assert_eq!(chain[0].2, input);
        assert_eq!(chain[1].2, [2, 0, 0, 255, 8, 0, 0, 255]);
        assert_eq!(chain[2].2, [5, 0, 0, 255]);
        let line = [
            0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 255, 255, 255, 255,
        ];
        for (w, h) in [(1, 5), (5, 1)] {
            let chain = mip_chain_rgba(w, h, &line, 3).unwrap();
            assert_eq!(chain[1].2, [1, 1, 1, 1, 3, 3, 3, 3]);
            assert_eq!(chain[2].2, [2; 4]);
            assert_eq!(chain, mip_chain_rgba(w, h, &line, 3).unwrap());
        }
        assert_eq!(
            mip_chain_rgba(1, 1, &[3; 4], 1).unwrap(),
            [(1, 1, vec![3; 4])]
        );
    }

    #[test]
    fn invalid_requests_return_typed_errors() {
        for result in [
            encode_surface(0, 1, 1, &[0; 4]),
            encode_surface(4, 0, 1, &[]),
            encode_surface(4, 1, 0, &[]),
            encode_surface(4, 1, 1, &[0; 3]),
            encode_surface(4, u32::MAX, u32::MAX, &[]),
            mip_chain_rgba(0, 1, &[], 1).map(|_| vec![]),
            mip_chain_rgba(1, 1, &[0; 4], 0).map(|_| vec![]),
            mip_chain_rgba(3, 5, &[0; 60], 4).map(|_| vec![]),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidAttributeValue);
            assert_eq!(error.offset(), 0);
        }
    }

    #[test]
    fn synthetic_quality() {
        let (width, height) = (32, 20);
        let mut seed = 0x19a4_73d2u32;
        for pattern in ["gradient", "edges", "noise"] {
            let mut pixels = Vec::new();
            for y in 0..height {
                for x in 0..width {
                    let pixel = match pattern {
                        "gradient" => [
                            (x * 255 / (width - 1)) as u8,
                            (y * 255 / (height - 1)) as u8,
                            ((x + y) * 255 / (width + height - 2)) as u8,
                            (x * 255 / (width - 1)) as u8,
                        ],
                        "edges" => {
                            if x * height < y * width {
                                [17, 221, 51, 0]
                            } else {
                                [239, 34, 187, 255]
                            }
                        }
                        _ => std::array::from_fn(|_| {
                            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            (seed >> 24) as u8
                        }),
                    };
                    pixels.extend_from_slice(&pixel);
                }
            }
            for format in [24, 26] {
                let mut input = pixels.clone();
                if format == 24 {
                    for p in input.as_chunks_mut::<4>().0.iter_mut() {
                        p[3] = 255;
                    }
                }
                let decoded = round_trip(format, width, height, &input);
                let mse = |channels: usize| {
                    input
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(decoded.as_chunks::<4>().0.iter())
                        .map(|(a, b)| {
                            (0..channels)
                                .map(|c| (f64::from(a[c]) - f64::from(b[c])).powi(2))
                                .sum::<f64>()
                        })
                        .sum::<f64>()
                        / f64::from(width * height)
                        / channels as f64
                };
                let rgb = mse(3);
                let rgba = mse(4);
                eprintln!("quality {pattern} {format}: RGB {rgb:.6}, RGBA {rgba:.6}");
                let (rgb_bound, rgba_bound) = match pattern {
                    "gradient" => (50., 40.),
                    "edges" => (4., 3.),
                    _ => (2800., 2100.),
                };
                assert!(rgb < rgb_bound, "{pattern}, format {format}: RGB MSE {rgb}");
                assert!(
                    rgba < rgba_bound,
                    "{pattern}, format {format}: RGBA MSE {rgba}"
                );
            }
        }
    }
}
