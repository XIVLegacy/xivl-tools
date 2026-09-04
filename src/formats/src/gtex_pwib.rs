//! Recognition of the GTEX and PWIB tagged resource families.
//!
//! The available 1.23b evidence establishes only each four-byte signature.
//! No field after the signature has an evidenced meaning or boundary, so this
//! module deliberately preserves the remainder as one opaque span.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

pub const GTEX_MAGIC: &[u8; 4] = b"GTEX";
pub const PWIB_MAGIC: &[u8; 4] = b"PWIB";

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
    pub opaque_remainder: Span,
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
    Ok(TaggedResource {
        kind: expected,
        signature: Span::new(0, 4),
        opaque_remainder: Span::new(4, (data.len() - 4) as u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_the_two_exact_tags() {
        assert_eq!(detect(b"GTEXbody"), Some(TaggedResourceKind::Gtex));
        assert_eq!(detect(b"PWIBbody"), Some(TaggedResourceKind::Pwib));
        assert_eq!(detect(b"GTExbody"), None);
    }

    #[test]
    fn preserves_every_byte_after_the_tag_as_unresolved() {
        let parsed = parse(b"GTEX\x00\xff\x12", TaggedResourceKind::Gtex).unwrap();
        assert_eq!(parsed.signature, Span::new(0, 4));
        assert_eq!(parsed.opaque_remainder, Span::new(4, 3));
    }

    #[test]
    fn explicit_reading_reports_truncation_and_wrong_magic() {
        let truncated = parse(b"GTE", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(truncated.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(truncated.offset(), 0);
        let wrong = parse(b"PWIB", TaggedResourceKind::Gtex).unwrap_err();
        assert_eq!(wrong.kind(), ErrorKind::BadMagic);
        assert_eq!(wrong.offset(), 0);
    }
}
