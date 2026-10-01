//! Legacy DDS containers for exact GTEX surface export.

use crate::digest::sha256_hex;
use crate::error::{ErrorKind, FormatError, Result};
use crate::gtex_pwib::{GtexFormat, GtexResource};
use crate::reader::Span;

pub const DDS_MAGIC: &[u8; 4] = b"DDS ";
pub const DDS_HEADER_SIZE: usize = 124;
pub const DDS_FILE_HEADER_SIZE: usize = 128;
const DDSD_CAPS: u32 = 0x0000_0001;
const DDSD_HEIGHT: u32 = 0x0000_0002;
const DDSD_WIDTH: u32 = 0x0000_0004;
const DDSD_PITCH: u32 = 0x0000_0008;
const DDSD_PIXELFORMAT: u32 = 0x0000_1000;
const DDSD_MIPMAPCOUNT: u32 = 0x0002_0000;
const DDSD_LINEARSIZE: u32 = 0x0008_0000;
const DDPF_ALPHAPIXELS: u32 = 0x0000_0001;
const DDPF_FOURCC: u32 = 0x0000_0004;
const DDPF_RGB: u32 = 0x0000_0040;
const DDSCAPS_COMPLEX: u32 = 0x0000_0008;
const DDSCAPS_TEXTURE: u32 = 0x0000_1000;
const DDSCAPS_MIPMAP: u32 = 0x0040_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsMip {
    pub mip_level: u8,
    pub width: u32,
    pub height: u32,
    pub source_span: Span,
    pub dds_span: Span,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsExport {
    pub bytes: Vec<u8>,
    pub format: GtexFormat,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u8,
    pub mips: Vec<DdsMip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsImage {
    pub format: DdsPixelFormat,
    pub width: u32,
    pub height: u32,
    pub mip_levels: u8,
    pub mips: Vec<DdsMipLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsPixelFormat {
    A8R8G8B8,
    Dxt1,
    Dxt5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsMipLayout {
    pub mip_level: u8,
    pub width: u32,
    pub height: u32,
    pub span: Span,
}

pub const fn max_mip_levels(width: u32, height: u32) -> u32 {
    let max = if width > height { width } else { height };
    if max == 0 {
        0
    } else {
        u32::BITS - max.leading_zeros()
    }
}

pub fn export_gtex(data: &[u8], gtex: &GtexResource) -> Result<DdsExport> {
    export_gtex_from_source(data, gtex)
}

/// Export a validated external-source GTEX descriptor. Surface spans in
/// `gtex` are relative to `source`; the descriptor bytes are intentionally
/// not joined to that view.
pub fn export_gtex_external(source: &[u8], gtex: &GtexResource) -> Result<DdsExport> {
    export_gtex_from_source(source, gtex)
}

fn export_gtex_from_source(data: &[u8], gtex: &GtexResource) -> Result<DdsExport> {
    if let Some(reason) = gtex.materialization_refusal() {
        return Err(FormatError::new(
            ErrorKind::UnsupportedDdsFormat,
            0,
            format!("GTEX cannot be exported as DDS: {reason}"),
        ));
    }
    let format = gtex.format.ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnsupportedDdsFormat,
            0x06,
            "GTEX client format index is not mapped",
        )
    })?;
    let width = u32::from(gtex.width);
    let height = u32::from(gtex.height);
    let mip_levels = gtex.mip_levels;
    if u32::from(mip_levels) > max_mip_levels(width, height) {
        return Err(FormatError::new(
            ErrorKind::UnsupportedDdsFormat,
            0x07,
            "GTEX mip count exceeds the dimension-derived DDS limit",
        ));
    }

    let payload_size = gtex.surfaces.iter().try_fold(0usize, |total, surface| {
        total
            .checked_add(surface.declared_size as usize)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    surface.size_field_span.offset,
                    "DDS payload size overflows the address space",
                )
            })
    })?;
    let total_size = DDS_FILE_HEADER_SIZE
        .checked_add(payload_size)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0,
                "DDS output size overflows the address space",
            )
        })?;
    let mut output = Vec::with_capacity(total_size);
    write_header(
        &mut output,
        format,
        width,
        height,
        mip_levels,
        gtex.surfaces[0].declared_size,
    )?;
    let mut mips = Vec::with_capacity(gtex.surfaces.len());
    for (index, surface) in gtex.surfaces.iter().enumerate() {
        let source_start = usize::try_from(surface.source_span.offset).map_err(|_| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                surface.source_span.offset,
                "GTEX surface offset does not fit this platform",
            )
        })?;
        let source_end = source_start
            .checked_add(surface.declared_size as usize)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    surface.size_field_span.offset,
                    "GTEX surface end overflows the address space",
                )
            })?;
        let source = data.get(source_start..source_end).ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                surface.source_span.offset,
                "GTEX surface span escapes the bounded input",
            )
        })?;
        let dds_start = output.len();
        output.extend_from_slice(source);
        let dds_length = source.len() as u64;
        let mip_level = surface.mip_level;
        let mip_width = width.checked_shr(u32::from(mip_level)).unwrap_or(0).max(1);
        let mip_height = height.checked_shr(u32::from(mip_level)).unwrap_or(0).max(1);
        if index != usize::from(mip_level) || surface.face != 0 {
            return Err(FormatError::new(
                ErrorKind::InvalidDdsHeader,
                surface.offset_field_span.offset,
                "DDS export requires one ordered 2D surface per mip",
            ));
        }
        mips.push(DdsMip {
            mip_level,
            width: mip_width,
            height: mip_height,
            source_span: surface.source_span,
            dds_span: Span::new(dds_start as u64, dds_length),
            sha256: sha256_hex(source),
        });
    }
    Ok(DdsExport {
        bytes: output,
        format,
        width,
        height,
        mip_levels,
        mips,
    })
}

fn write_header(
    output: &mut Vec<u8>,
    format: GtexFormat,
    width: u32,
    height: u32,
    mip_levels: u8,
    top_size: u32,
) -> Result<()> {
    if width == 0 || height == 0 || mip_levels == 0 {
        return Err(FormatError::new(
            ErrorKind::InvalidDdsHeader,
            0,
            "DDS requires nonzero width, height, and mip count",
        ));
    }
    if u32::from(mip_levels) > max_mip_levels(width, height) {
        return Err(FormatError::new(
            ErrorKind::InvalidDdsHeader,
            28,
            "DDS mip count exceeds the dimension-derived limit",
        ));
    }
    output.extend_from_slice(DDS_MAGIC);
    push_u32(output, DDS_HEADER_SIZE as u32);
    let compressed = format.block_bytes.is_some();
    let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;
    flags |= if compressed {
        DDSD_LINEARSIZE
    } else {
        DDSD_PITCH
    };
    if mip_levels > 1 {
        flags |= DDSD_MIPMAPCOUNT;
    }
    push_u32(output, flags);
    push_u32(output, height);
    push_u32(output, width);
    let top_layout = if compressed {
        top_size
    } else {
        width.checked_mul(4).ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                20,
                "DDS pitch overflows u32",
            )
        })?
    };
    push_u32(output, top_layout);
    push_u32(output, 0);
    push_u32(output, u32::from(mip_levels));
    output.extend_from_slice(&[0; 44]);
    push_u32(output, 32);
    match format.block_bytes {
        Some(_) => {
            push_u32(output, DDPF_FOURCC);
            push_u32(output, format.d3d_value);
            push_u32(output, 0);
            push_u32(output, 0);
            push_u32(output, 0);
            push_u32(output, 0);
            push_u32(output, 0);
        }
        None => {
            push_u32(output, DDPF_RGB | DDPF_ALPHAPIXELS);
            push_u32(output, 0);
            push_u32(output, 32);
            push_u32(output, 0x00ff_0000);
            push_u32(output, 0x0000_ff00);
            push_u32(output, 0x0000_00ff);
            push_u32(output, 0xff00_0000);
        }
    }
    let caps = if mip_levels > 1 {
        DDSCAPS_COMPLEX | DDSCAPS_MIPMAP | DDSCAPS_TEXTURE
    } else {
        DDSCAPS_TEXTURE
    };
    push_u32(output, caps);
    push_u32(output, 0);
    push_u32(output, 0);
    push_u32(output, 0);
    push_u32(output, 0);
    debug_assert_eq!(output.len(), DDS_FILE_HEADER_SIZE);
    Ok(())
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}

pub fn parse(data: &[u8]) -> Result<DdsImage> {
    if data.len() < DDS_FILE_HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            "DDS header is truncated",
        ));
    }
    if !data.starts_with(DDS_MAGIC) {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            "expected DDS signature",
        ));
    }
    let header_size = u32_at(data, 4)?;
    if header_size != DDS_HEADER_SIZE as u32 {
        return Err(dds_error(4, "DDS header size must be 124"));
    }
    let flags = u32_at(data, 8)?;
    let height = u32_at(data, 12)?;
    let width = u32_at(data, 16)?;
    let layout_size = u32_at(data, 20)?;
    let depth = u32_at(data, 24)?;
    let mip_levels = u32_at(data, 28)?;
    if width == 0 || height == 0 || depth != 0 || mip_levels == 0 || mip_levels > u32::from(u8::MAX)
    {
        return Err(dds_error(12, "DDS dimensions or mip count are invalid"));
    }
    if mip_levels > max_mip_levels(width, height) {
        return Err(dds_error(
            28,
            "DDS mip count exceeds the dimension-derived limit",
        ));
    }
    let required_flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;
    if flags & required_flags != required_flags {
        return Err(dds_error(8, "DDS required header flags are missing"));
    }
    if data[32..76].iter().any(|byte| *byte != 0) || data[112..128].iter().any(|byte| *byte != 0) {
        return Err(dds_error(32, "DDS reserved header fields must be zero"));
    }
    let pf_size = u32_at(data, 76)?;
    if pf_size != 32 {
        return Err(dds_error(76, "DDS pixel format size must be 32"));
    }
    let pf_flags = u32_at(data, 80)?;
    let fourcc = u32_at(data, 84)?;
    let format = if pf_flags == DDPF_RGB | DDPF_ALPHAPIXELS
        && fourcc == 0
        && u32_at(data, 88)? == 32
        && u32_at(data, 92)? == 0x00ff_0000
        && u32_at(data, 96)? == 0x0000_ff00
        && u32_at(data, 100)? == 0x0000_00ff
        && u32_at(data, 104)? == 0xff00_0000
    {
        if flags & DDSD_PITCH == 0
            || layout_size
                != width
                    .checked_mul(4)
                    .ok_or_else(|| dds_error(20, "DDS pitch overflows u32"))?
        {
            return Err(dds_error(20, "DDS A8R8G8B8 pitch is inconsistent"));
        }
        DdsPixelFormat::A8R8G8B8
    } else if pf_flags == DDPF_FOURCC && fourcc == 0x3154_5844 {
        if flags & DDSD_LINEARSIZE == 0 {
            return Err(dds_error(8, "DDS DXT1 linearsize flag is missing"));
        }
        if data[88..108].iter().any(|byte| *byte != 0) {
            return Err(dds_error(
                88,
                "DDS compressed pixel format fields must be zero",
            ));
        }
        DdsPixelFormat::Dxt1
    } else if pf_flags == DDPF_FOURCC && fourcc == 0x3554_5844 {
        if flags & DDSD_LINEARSIZE == 0 {
            return Err(dds_error(8, "DDS DXT5 linearsize flag is missing"));
        }
        if data[88..108].iter().any(|byte| *byte != 0) {
            return Err(dds_error(
                88,
                "DDS compressed pixel format fields must be zero",
            ));
        }
        DdsPixelFormat::Dxt5
    } else {
        return Err(FormatError::new(
            ErrorKind::UnsupportedDdsFormat,
            80,
            "DDS pixel format is outside the supported legacy set",
        ));
    };
    let expected_top_size = surface_size(format, width, height)?;
    let expected_flags = DDSD_CAPS
        | DDSD_HEIGHT
        | DDSD_WIDTH
        | DDSD_PIXELFORMAT
        | if matches!(format, DdsPixelFormat::A8R8G8B8) {
            DDSD_PITCH
        } else {
            DDSD_LINEARSIZE
        }
        | if mip_levels > 1 { DDSD_MIPMAPCOUNT } else { 0 };
    if flags != expected_flags {
        return Err(dds_error(8, "DDS header flags are not canonical"));
    }
    if matches!(format, DdsPixelFormat::Dxt1 | DdsPixelFormat::Dxt5)
        && layout_size as u64 != expected_top_size
    {
        return Err(dds_error(20, "DDS compressed linearsize is inconsistent"));
    }
    let caps = u32_at(data, 108)?;
    let expected_caps = if mip_levels > 1 {
        DDSCAPS_COMPLEX | DDSCAPS_MIPMAP | DDSCAPS_TEXTURE
    } else {
        DDSCAPS_TEXTURE
    };
    if caps != expected_caps {
        return Err(dds_error(108, "DDS caps are not canonical"));
    }
    let mip_count =
        usize::try_from(mip_levels).map_err(|_| dds_error(28, "DDS mip count does not fit"))?;
    let mut offset = DDS_FILE_HEADER_SIZE as u64;
    let mut mips = Vec::with_capacity(mip_count);
    for level in 0..mip_count {
        let mip_width = width.checked_shr(level as u32).unwrap_or(0).max(1);
        let mip_height = height.checked_shr(level as u32).unwrap_or(0).max(1);
        let size = surface_size(format, mip_width, mip_height)?;
        let end = offset.checked_add(size).ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                offset,
                "DDS mip span overflows",
            )
        })?;
        if end > data.len() as u64 {
            return Err(FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                offset,
                "DDS mip span escapes the bounded input",
            ));
        }
        mips.push(DdsMipLayout {
            mip_level: level as u8,
            width: mip_width,
            height: mip_height,
            span: Span::new(offset, size),
        });
        offset = end;
    }
    if offset != data.len() as u64 {
        return Err(FormatError::new(
            ErrorKind::AmbiguousPayloadSpan,
            offset,
            "DDS contains trailing bytes after the mip chain",
        ));
    }
    Ok(DdsImage {
        format,
        width,
        height,
        mip_levels: mip_levels as u8,
        mips,
    })
}

fn surface_size(format: DdsPixelFormat, width: u32, height: u32) -> Result<u64> {
    match format {
        DdsPixelFormat::A8R8G8B8 => u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or_else(|| dds_error(20, "DDS linear surface size overflows")),
        DdsPixelFormat::Dxt1 => u64::from(width.div_ceil(4))
            .checked_mul(u64::from(height.div_ceil(4)))
            .and_then(|value| value.checked_mul(8))
            .ok_or_else(|| dds_error(20, "DDS DXT1 surface size overflows")),
        DdsPixelFormat::Dxt5 => u64::from(width.div_ceil(4))
            .checked_mul(u64::from(height.div_ceil(4)))
            .and_then(|value| value.checked_mul(16))
            .ok_or_else(|| dds_error(20, "DDS DXT5 surface size overflows")),
    }
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| dds_error(offset, "DDS field offset overflows"))?;
    let bytes = data.get(offset..end).ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            offset as u64,
            "DDS field is truncated",
        )
    })?;
    Ok(u32::from_le_bytes(
        bytes.try_into().expect("checked DDS field width"),
    ))
}

fn dds_error(offset: usize, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidDdsHeader, offset as u64, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gtex_pwib::{parse as parse_gtex, TaggedResource, TaggedResourceKind};

    fn gtex(format: u8, width: u16, height: u16, mips: &[&[u8]]) -> Vec<u8> {
        let base = 0x18 + mips.len() * 8;
        let mut data = vec![0u8; base];
        data[0..4].copy_from_slice(b"GTEX");
        data[6] = format;
        data[7] = mips.len() as u8;
        data[0x0a..0x0c].copy_from_slice(&width.to_be_bytes());
        data[0x0c..0x0e].copy_from_slice(&height.to_be_bytes());
        data[0x0e..0x10].copy_from_slice(&1u16.to_be_bytes());
        data[0x10..0x14].copy_from_slice(&0x18u32.to_be_bytes());
        data[0x14..0x18].copy_from_slice(&(base as u32).to_be_bytes());
        let mut offset = 0u32;
        for (index, mip) in mips.iter().enumerate() {
            let field = 0x18 + index * 8;
            data[field..field + 4].copy_from_slice(&offset.to_be_bytes());
            data[field + 4..field + 8].copy_from_slice(&(mip.len() as u32).to_be_bytes());
            data.extend_from_slice(mip);
            offset += mip.len() as u32;
        }
        data
    }

    #[test]
    fn exports_and_parses_a8_header_and_payload_exactly() {
        let source = gtex(4, 2, 1, &[&[1, 2, 3, 4, 5, 6, 7, 8]]);
        let TaggedResource::Gtex(parsed) = parse_gtex(&source, TaggedResourceKind::Gtex).unwrap()
        else {
            unreachable!()
        };
        let export = export_gtex(&source, &parsed).unwrap();
        assert_eq!(&export.bytes[0..4], b"DDS ");
        assert_eq!(le32(&export.bytes, 4), 124);
        assert_eq!(
            le32(&export.bytes, 8),
            DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_PIXELFORMAT
        );
        assert_eq!(le32(&export.bytes, 12), 1);
        assert_eq!(le32(&export.bytes, 16), 2);
        assert_eq!(le32(&export.bytes, 20), 8);
        assert_eq!(le32(&export.bytes, 28), 1);
        assert_eq!(le32(&export.bytes, 76), 32);
        assert_eq!(le32(&export.bytes, 80), DDPF_RGB | DDPF_ALPHAPIXELS);
        assert_eq!(le32(&export.bytes, 88), 32);
        assert_eq!(le32(&export.bytes, 92), 0x00ff_0000);
        assert_eq!(le32(&export.bytes, 96), 0x0000_ff00);
        assert_eq!(le32(&export.bytes, 100), 0x0000_00ff);
        assert_eq!(le32(&export.bytes, 104), 0xff00_0000);
        assert_eq!(le32(&export.bytes, 108), DDSCAPS_TEXTURE);
        assert_eq!(
            parse(&export.bytes).unwrap().mips[0].span,
            Span::new(128, 8)
        );
        assert_eq!(&export.bytes[128..], &source[0x20..]);
    }

    fn le32(data: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
    }

    #[test]
    fn writes_literal_dxt1_legacy_fields() {
        let source = gtex(24, 4, 4, &[&[0u8; 8]]);
        let TaggedResource::Gtex(parsed) = parse_gtex(&source, TaggedResourceKind::Gtex).unwrap()
        else {
            unreachable!()
        };
        let bytes = export_gtex(&source, &parsed).unwrap().bytes;
        assert_eq!(
            le32(&bytes, 8),
            DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_LINEARSIZE | DDSD_PIXELFORMAT
        );
        assert_eq!(le32(&bytes, 20), 8);
        assert_eq!(le32(&bytes, 80), DDPF_FOURCC);
        assert_eq!(le32(&bytes, 84), 0x3154_5844);
        assert_eq!(le32(&bytes, 108), DDSCAPS_TEXTURE);
    }

    #[test]
    fn writes_literal_dxt5_and_multimip_legacy_fields() {
        let source = gtex(26, 8, 8, &[&[0u8; 64], &[1u8; 16]]);
        let TaggedResource::Gtex(parsed) = parse_gtex(&source, TaggedResourceKind::Gtex).unwrap()
        else {
            unreachable!()
        };
        let bytes = export_gtex(&source, &parsed).unwrap().bytes;
        assert_eq!(le32(&bytes, 8), 0x000a_1007);
        assert_eq!(le32(&bytes, 12), 8);
        assert_eq!(le32(&bytes, 16), 8);
        assert_eq!(le32(&bytes, 20), 64);
        assert_eq!(le32(&bytes, 28), 2);
        assert_eq!(le32(&bytes, 76), 32);
        assert_eq!(le32(&bytes, 80), 0x0000_0004);
        assert_eq!(le32(&bytes, 84), 0x3554_5844);
        assert_eq!(le32(&bytes, 88), 0);
        assert_eq!(le32(&bytes, 92), 0);
        assert_eq!(le32(&bytes, 96), 0);
        assert_eq!(le32(&bytes, 100), 0);
        assert_eq!(le32(&bytes, 104), 0);
        assert_eq!(le32(&bytes, 108), 0x0040_1008);
        assert_eq!(parse(&bytes).unwrap().format, DdsPixelFormat::Dxt5);
    }

    #[test]
    fn refuses_non_2d_or_unmapped_or_tableless_gtex() {
        let cases: &[(u8, usize)] = &[(1, 6), (2, 1), (4, 1)];
        for &(flags, mip_count) in cases {
            let mips = vec![&[0u8; 8][..]; mip_count];
            let mut source = gtex(24, 4, 4, &mips);
            if flags == 1 {
                source[7] = 1;
            }
            source[9] = flags;
            let TaggedResource::Gtex(parsed) =
                parse_gtex(&source, TaggedResourceKind::Gtex).unwrap()
            else {
                unreachable!()
            };
            assert_eq!(
                export_gtex(&source, &parsed).unwrap_err().kind(),
                ErrorKind::UnsupportedDdsFormat
            );
        }
        let mut absent_table = gtex(24, 4, 4, &[&[0u8; 8]]);
        absent_table[0x10..0x14].copy_from_slice(&0u32.to_be_bytes());
        let TaggedResource::Gtex(parsed) =
            parse_gtex(&absent_table, TaggedResourceKind::Gtex).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(
            export_gtex(&absent_table, &parsed).unwrap_err().kind(),
            ErrorKind::UnsupportedDdsFormat
        );
    }

    #[test]
    fn rejects_noncanonical_legacy_header_fields() {
        let source = gtex(24, 4, 4, &[&[0u8; 8]]);
        let TaggedResource::Gtex(parsed) = parse_gtex(&source, TaggedResourceKind::Gtex).unwrap()
        else {
            unreachable!()
        };
        let exported = export_gtex(&source, &parsed).unwrap().bytes;
        for (offset, value) in [(8, 0x000a_1007u32), (108, 0x0000_1008u32), (104, 1)] {
            let mut changed = exported.clone();
            changed[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(
                parse(&changed).unwrap_err().kind(),
                ErrorKind::InvalidDdsHeader
            );
        }
    }

    #[test]
    fn rejects_trailing_dds_bytes() {
        let source = gtex(24, 4, 4, &[&[0u8; 8]]);
        let TaggedResource::Gtex(parsed) = parse_gtex(&source, TaggedResourceKind::Gtex).unwrap()
        else {
            unreachable!()
        };
        let mut bytes = export_gtex(&source, &parsed).unwrap().bytes;
        bytes.push(1);
        assert_eq!(
            parse(&bytes).unwrap_err().kind(),
            ErrorKind::AmbiguousPayloadSpan
        );
    }
}
