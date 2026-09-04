//! Bounded GTEX extents and PWIB-wrapped SEDB resources.
//!
//! Retail 1.23b comparisons establish only the fixed outer spans and their
//! byte extents. Texture metadata and the purpose of PWIB remain unresolved.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};
use crate::sedb::{self, Container};

pub const GTEX_MAGIC: &[u8; 4] = b"GTEX";
pub const PWIB_MAGIC: &[u8; 4] = b"PWIB";
pub const GTEX_HEADER_SIZE: usize = 0x20;
pub const PWIB_HEADER_SIZE: usize = 0x10;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedResource {
    pub kind: TaggedResourceKind,
    pub signature: Span,
    pub header: Span,
    pub header_unknown: Span,
    pub declared_extent: Span,
    pub trailing: Span,
    pub declared_extent_size: u32,
    pub nested_sedb: Option<Container>,
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
        TaggedResourceKind::Gtex => parse_gtex(data),
        TaggedResourceKind::Pwib => parse_pwib(data),
    }
}

fn parse_gtex(data: &[u8]) -> Result<TaggedResource> {
    if data.len() < GTEX_HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            format!(
                "GTEX header is truncated: wanted {GTEX_HEADER_SIZE} bytes, found {}",
                data.len()
            ),
        ));
    }
    let mut reader = Reader::new(data);
    reader.seek(0x1c)?;
    let declared_extent_size = reader.u32_be()?;
    let available = data.len() - GTEX_HEADER_SIZE;
    let extent_length = usize::try_from(declared_extent_size).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x1c,
            "GTEX extent size does not fit this platform",
        )
    })?;
    if extent_length > available {
        return Err(FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            0x1c,
            format!(
                "GTEX extent size {declared_extent_size} exceeds the {available} byte(s) after the header"
            ),
        ));
    }
    Ok(TaggedResource {
        kind: TaggedResourceKind::Gtex,
        signature: Span::new(0, 4),
        header: Span::new(0, GTEX_HEADER_SIZE as u64),
        header_unknown: Span::new(4, 0x18),
        declared_extent: Span::new(GTEX_HEADER_SIZE as u64, extent_length as u64),
        trailing: Span::new(
            (GTEX_HEADER_SIZE + extent_length) as u64,
            (available - extent_length) as u64,
        ),
        declared_extent_size,
        nested_sedb: None,
    })
}

fn parse_pwib(data: &[u8]) -> Result<TaggedResource> {
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
    let nested = data.get(PWIB_HEADER_SIZE..).ok_or_else(|| {
        FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            PWIB_HEADER_SIZE as u64,
            "PWIB ends before its nested SEDB resource",
        )
    })?;
    let nested_sedb = sedb::parse_container(nested, PWIB_HEADER_SIZE as u64)?;
    let payload_length = nested_sedb.total_size as usize;
    let available = data.len() - PWIB_HEADER_SIZE;
    debug_assert!(payload_length <= available);
    Ok(TaggedResource {
        kind: TaggedResourceKind::Pwib,
        signature: Span::new(0, 4),
        header: Span::new(0, PWIB_HEADER_SIZE as u64),
        header_unknown: Span::new(4, 12),
        declared_extent: Span::new(PWIB_HEADER_SIZE as u64, payload_length as u64),
        trailing: Span::new(
            (PWIB_HEADER_SIZE + payload_length) as u64,
            (available - payload_length) as u64,
        ),
        declared_extent_size: nested_sedb.declared_size,
        nested_sedb: Some(nested_sedb),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gtex(extent: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x5a; GTEX_HEADER_SIZE];
        bytes[0..4].copy_from_slice(GTEX_MAGIC);
        bytes[0x1c..0x20].copy_from_slice(&(extent.len() as u32).to_be_bytes());
        bytes.extend_from_slice(extent);
        bytes
    }

    fn pwib(payload: &[u8]) -> Vec<u8> {
        let total = 0x14 + payload.len();
        let mut bytes = vec![0x3c; PWIB_HEADER_SIZE];
        bytes[0..4].copy_from_slice(PWIB_MAGIC);
        bytes.extend_from_slice(b"SEDBsyn\0");
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&0x14u16.to_le_bytes());
        bytes.extend_from_slice(&(total as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn recognizes_only_the_two_exact_tags() {
        assert_eq!(detect(b"GTEXbody"), Some(TaggedResourceKind::Gtex));
        assert_eq!(detect(b"PWIBbody"), Some(TaggedResourceKind::Pwib));
        assert_eq!(detect(b"GTExbody"), None);
    }

    #[test]
    fn reads_the_gtex_big_endian_declared_extent() {
        let bytes = gtex(b"extent!");
        let parsed = parse(&bytes, TaggedResourceKind::Gtex).unwrap();
        assert_eq!(parsed.header, Span::new(0, 0x20));
        assert_eq!(parsed.declared_extent, Span::new(0x20, 7));
        assert_eq!(parsed.trailing, Span::new(0x27, 0));
        assert_eq!(parsed.declared_extent_size, 7);
    }

    #[test]
    fn reads_the_pwib_nested_sedb_extent() {
        let bytes = pwib(b"payload");
        let parsed = parse(&bytes, TaggedResourceKind::Pwib).unwrap();
        assert_eq!(parsed.header, Span::new(0, 0x10));
        assert_eq!(parsed.declared_extent, Span::new(0x10, 0x1b));
        assert_eq!(parsed.trailing, Span::new(0x2b, 0));
        assert_eq!(parsed.nested_sedb.unwrap().subtype, "syn\\x00");
    }

    #[test]
    fn reports_truncation_wrong_magic_and_oversized_extent() {
        let truncated = parse(b"GTE", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(truncated.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(truncated.offset(), 0);
        let wrong = parse(b"PWIB", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(wrong.kind(), ErrorKind::BadMagic);
        let short = parse(b"GTEX", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(short.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(short.offset(), 4);
        let mut oversized = gtex(b"x");
        oversized[0x1c..0x20].copy_from_slice(&2u32.to_be_bytes());
        let error = parse(&oversized, TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DeclaredSizeOutOfRange);
        assert_eq!(error.offset(), 0x1c);
    }
}
