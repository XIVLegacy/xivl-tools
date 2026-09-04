//! Loader-backed GTEX fields and PWIB segment boundaries.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{escape_tag, Reader, Span};
use crate::sedb;

pub const GTEX_MAGIC: &[u8; 4] = b"GTEX";
pub const PWIB_MAGIC: &[u8; 4] = b"PWIB";
pub const GTEX_FIXED_FIELDS_SIZE: usize = 0x18;
pub const PWIB_HEADER_SIZE: usize = 0x10;
pub const SURFACE_OFFSET_ENTRY_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaggedResourceKind {
    Gtex,
    Pwib,
}

impl TaggedResourceKind {
    pub const fn format_id(self) -> &'static str {
        match self {
            Self::Gtex => "gtex",
            Self::Pwib => "pwib",
        }
    }

    pub const fn magic(self) -> &'static [u8; 4] {
        match self {
            Self::Gtex => GTEX_MAGIC,
            Self::Pwib => PWIB_MAGIC,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    Texture2d,
    Cube,
    Volume,
}

impl TextureKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Texture2d => "2d",
            Self::Cube => "cube",
            Self::Volume => "volume",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceOffset {
    pub index: u32,
    pub field_span: Span,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtexResource {
    pub signature: Span,
    pub header: Span,
    pub format_index: u8,
    pub mip_levels: u8,
    pub flags: u8,
    pub texture_kind: TextureKind,
    pub width: u16,
    pub height: u16,
    pub depth: u16,
    pub offset_table_base: u32,
    pub data_base: u32,
    pub surface_offsets: Vec<SurfaceOffset>,
    pub header_unknown: Vec<Span>,
    pub data: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SedbPrefix {
    pub span: Span,
    pub subtype: String,
    pub unknown_a: u32,
    pub flags: u16,
    pub header_size: u16,
    pub declared_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PwibResource {
    pub signature: Span,
    pub header: Span,
    pub total_size: u32,
    pub first_offset: u32,
    pub second_offset: u32,
    pub first_segment: Span,
    pub second_segment: Span,
    pub trailing: Span,
    pub sedb_prefix: SedbPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggedResource {
    Gtex(GtexResource),
    Pwib(PwibResource),
}

pub fn detect(data: &[u8]) -> Option<TaggedResourceKind> {
    if data.starts_with(GTEX_MAGIC) {
        Some(TaggedResourceKind::Gtex)
    } else if data.starts_with(PWIB_MAGIC) {
        Some(TaggedResourceKind::Pwib)
    } else {
        None
    }
}

pub fn parse(data: &[u8], expected: TaggedResourceKind) -> Result<TaggedResource> {
    if data.len() < 4 {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            0,
            format!("{} signature is truncated", expected.format_id()),
        ));
    }
    if !data.starts_with(expected.magic()) {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            format!("expected {} signature", expected.magic().escape_ascii()),
        ));
    }
    match expected {
        TaggedResourceKind::Gtex => parse_gtex(data).map(TaggedResource::Gtex),
        TaggedResourceKind::Pwib => parse_pwib(data).map(TaggedResource::Pwib),
    }
}

fn parse_gtex(data: &[u8]) -> Result<GtexResource> {
    if data.len() < GTEX_FIXED_FIELDS_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "GTEX fixed fields are truncated: wanted {GTEX_FIXED_FIELDS_SIZE} bytes, found {}",
                data.len()
            ),
        ));
    }

    let mut reader = Reader::new(data);
    reader.seek(0x06)?;
    let format_index = reader.u8()?;
    let mip_levels = reader.u8()?;
    reader.seek(0x09)?;
    let flags = reader.u8()?;
    let width = reader.u16_be()?;
    let height = reader.u16_be()?;
    let depth = reader.u16_be()?;
    let offset_table_base = reader.u32_be()?;
    let data_base = reader.u32_be()?;

    let data_start = usize::try_from(data_base).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x14,
            "GTEX data base does not fit this platform",
        )
    })?;
    if data_start < GTEX_FIXED_FIELDS_SIZE || data_start > data.len() {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x14,
            format!("GTEX data base {data_base} is outside the input"),
        ));
    }

    let texture_kind = if flags & 1 != 0 {
        TextureKind::Cube
    } else if flags & 2 != 0 {
        TextureKind::Volume
    } else {
        TextureKind::Texture2d
    };
    let face_count = if texture_kind == TextureKind::Cube {
        6
    } else {
        1
    };
    let entry_count = usize::from(mip_levels) * face_count;
    let mut surface_offsets = Vec::new();
    let mut header_unknown = vec![Span::new(4, 2), Span::new(8, 1)];
    if offset_table_base != 0 {
        let table_start = usize::try_from(offset_table_base).map_err(|_| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0x10,
                "GTEX offset-table base does not fit this platform",
            )
        })?;
        let table_length = entry_count
            .checked_mul(SURFACE_OFFSET_ENTRY_SIZE)
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    0x10,
                    "GTEX offset-table length overflows the address space",
                )
            })?;
        let table_end = table_start.checked_add(table_length).ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0x10,
                "GTEX offset-table end overflows the address space",
            )
        })?;
        if table_start < GTEX_FIXED_FIELDS_SIZE || table_end > data_start {
            return Err(FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                0x10,
                format!(
                    "GTEX offset table [{table_start}, {table_end}) is outside the header ending at {data_start}"
                ),
            ));
        }
        if table_start > GTEX_FIXED_FIELDS_SIZE {
            header_unknown.push(Span::new(
                GTEX_FIXED_FIELDS_SIZE as u64,
                (table_start - GTEX_FIXED_FIELDS_SIZE) as u64,
            ));
        }
        for index in 0..entry_count {
            let field_offset = table_start + index * SURFACE_OFFSET_ENTRY_SIZE;
            reader.seek(field_offset)?;
            let value = reader.u32_be()?;
            let source = data_start.checked_add(value as usize).ok_or_else(|| {
                FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    field_offset as u64,
                    "GTEX surface source offset overflows the address space",
                )
            })?;
            if source > data.len() {
                return Err(FormatError::new(
                    ErrorKind::DeclaredSizeOutOfRange,
                    field_offset as u64,
                    format!("GTEX surface source offset {value} escapes the input"),
                ));
            }
            surface_offsets.push(SurfaceOffset {
                index: index as u32,
                field_span: Span::new(field_offset as u64, 4),
                value,
            });
            header_unknown.push(Span::new((field_offset + 4) as u64, 4));
        }
        if table_end < data_start {
            header_unknown.push(Span::new(table_end as u64, (data_start - table_end) as u64));
        }
    } else if data_start > GTEX_FIXED_FIELDS_SIZE {
        header_unknown.push(Span::new(
            GTEX_FIXED_FIELDS_SIZE as u64,
            (data_start - GTEX_FIXED_FIELDS_SIZE) as u64,
        ));
    }

    Ok(GtexResource {
        signature: Span::new(0, 4),
        header: Span::new(0, data_base as u64),
        format_index,
        mip_levels,
        flags,
        texture_kind,
        width,
        height,
        depth,
        offset_table_base,
        data_base,
        surface_offsets,
        header_unknown,
        data: Span::new(data_base as u64, (data.len() - data_start) as u64),
    })
}

fn parse_pwib(data: &[u8]) -> Result<PwibResource> {
    if data.len() < PWIB_HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "PWIB header is truncated: wanted {PWIB_HEADER_SIZE} bytes, found {}",
                data.len()
            ),
        ));
    }
    let mut reader = Reader::new(data);
    reader.seek(4)?;
    let total_size = reader.u32_be()?;
    let first_offset = reader.u32_be()?;
    let second_offset = reader.u32_be()?;
    let total = total_size as usize;
    let first = first_offset as usize;
    let second = second_offset as usize;
    if first < PWIB_HEADER_SIZE || first > second || second > total || total > data.len() {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            4,
            format!(
                "PWIB boundaries must satisfy 16 <= first ({first}) <= second ({second}) <= total ({total}) <= input ({})",
                data.len()
            ),
        ));
    }
    let first_length = second - first;
    if first_length < sedb::FIXED_HEADER_SIZE as usize {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            first_offset as u64,
            "PWIB first segment cannot hold the fixed SEDB header",
        ));
    }
    let mut prefix = Reader::with_base(&data[first..second], first_offset as u64);
    let magic = prefix.take(4)?;
    if magic != sedb::MAGIC {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            first_offset as u64,
            format!("expected a SEDB signature, found '{}'", escape_tag(magic)),
        ));
    }
    let subtype = escape_tag(prefix.take(4)?);
    let unknown_a = prefix.u32_le()?;
    let flags = prefix.u16_le()?;
    let header_size = prefix.u16_le()?;
    let declared_size = prefix.u32_le()?;

    Ok(PwibResource {
        signature: Span::new(0, 4),
        header: Span::new(0, PWIB_HEADER_SIZE as u64),
        total_size,
        first_offset,
        second_offset,
        first_segment: Span::new(first_offset as u64, (second - first) as u64),
        second_segment: Span::new(second_offset as u64, (total - second) as u64),
        trailing: Span::new(total_size as u64, (data.len() - total) as u64),
        sedb_prefix: SedbPrefix {
            span: Span::new(first_offset as u64, sedb::FIXED_HEADER_SIZE as u64),
            subtype,
            unknown_a,
            flags,
            header_size,
            declared_size,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gtex() -> Vec<u8> {
        let mut bytes = vec![0x5a; 0x30];
        bytes[0..4].copy_from_slice(GTEX_MAGIC);
        bytes[6] = 4;
        bytes[7] = 2;
        bytes[9] = 1;
        bytes[0x0a..0x0c].copy_from_slice(&64u16.to_be_bytes());
        bytes[0x0c..0x0e].copy_from_slice(&32u16.to_be_bytes());
        bytes[0x0e..0x10].copy_from_slice(&1u16.to_be_bytes());
        bytes[0x10..0x14].copy_from_slice(&0u32.to_be_bytes());
        bytes[0x14..0x18].copy_from_slice(&0x20u32.to_be_bytes());
        bytes
    }

    fn pwib() -> Vec<u8> {
        let mut bytes = vec![0x3c; 0x35];
        bytes[0..4].copy_from_slice(PWIB_MAGIC);
        bytes[4..8].copy_from_slice(&0x30u32.to_be_bytes());
        bytes[8..12].copy_from_slice(&0x10u32.to_be_bytes());
        bytes[12..16].copy_from_slice(&0x28u32.to_be_bytes());
        bytes[0x10..0x18].copy_from_slice(b"SEDBsyn\0");
        bytes[0x18..0x1c].copy_from_slice(&7u32.to_le_bytes());
        bytes[0x1c..0x1e].copy_from_slice(&2u16.to_le_bytes());
        bytes[0x1e..0x20].copy_from_slice(&0x14u16.to_le_bytes());
        bytes[0x20..0x24].copy_from_slice(&0x20u32.to_le_bytes());
        bytes
    }

    #[test]
    fn recognizes_only_the_two_exact_tags() {
        assert_eq!(detect(b"GTEXbody"), Some(TaggedResourceKind::Gtex));
        assert_eq!(detect(b"PWIBbody"), Some(TaggedResourceKind::Pwib));
        assert_eq!(detect(b"GTExbody"), None);
    }

    #[test]
    fn reads_loader_backed_gtex_fields() {
        let TaggedResource::Gtex(parsed) = parse(&gtex(), TaggedResourceKind::Gtex).unwrap() else {
            panic!("expected GTEX");
        };
        assert_eq!(parsed.header, Span::new(0, 0x20));
        assert_eq!(parsed.data, Span::new(0x20, 0x10));
        assert_eq!(parsed.texture_kind, TextureKind::Cube);
        assert_eq!((parsed.width, parsed.height, parsed.depth), (64, 32, 1));
    }

    #[test]
    fn reads_loader_backed_pwib_segments() {
        let TaggedResource::Pwib(parsed) = parse(&pwib(), TaggedResourceKind::Pwib).unwrap() else {
            panic!("expected PWIB");
        };
        assert_eq!(parsed.first_segment, Span::new(0x10, 0x18));
        assert_eq!(parsed.second_segment, Span::new(0x28, 8));
        assert_eq!(parsed.trailing, Span::new(0x30, 5));
        assert_eq!(parsed.sedb_prefix.subtype, "syn\\x00");
    }

    #[test]
    fn reports_truncation_wrong_magic_and_bad_boundaries() {
        let truncated = parse(b"GTE", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(truncated.kind(), ErrorKind::UnexpectedEndOfInput);
        let wrong = parse(b"PWIB", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(wrong.kind(), ErrorKind::BadMagic);
        let short = parse(b"GTEX", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(short.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(short.offset(), 4);
        let mut bad = pwib();
        bad[12..16].copy_from_slice(&0x31u32.to_be_bytes());
        let error = parse(&bad, TaggedResourceKind::Pwib).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DeclaredSizeOutOfRange);
        assert_eq!(error.offset(), 4);
    }
}
