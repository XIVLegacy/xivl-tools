//! Bounded extraction of the two evidenced LPB wrapper variants.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};

const RAW_MAGIC: &[u8; 4] = b"rlu\x0b";
const XOR_MAGIC: &[u8; 4] = b"rle\x0c";
const LUA_51_SIGNATURE: &[u8; 5] = b"\x1bLuaQ";
const XOR_KEY: u8 = 0x73;

/// Whether the input begins with one of the evidenced LPB wrappers.
pub fn has_signature(data: &[u8]) -> bool {
    data.starts_with(RAW_MAGIC) || data.starts_with(XOR_MAGIC)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LpbVariant {
    Raw,
    Xor73,
}

impl LpbVariant {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Xor73 => "xor-73",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreservedBytes {
    pub span: Span,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LpbFile {
    pub variant: LpbVariant,
    pub header: Span,
    pub unknown_header: Vec<PreservedBytes>,
    pub advisory_size: Option<u32>,
    pub encoded_prefix: Option<Span>,
    pub encoded_payload: Span,
    pub decoded: Vec<u8>,
}

/// Extract a Lua 5.1 chunk while retaining every uninterpreted header byte.
pub fn extract(data: &[u8]) -> Result<LpbFile> {
    let mut reader = Reader::new(data);
    let magic = reader.take(4)?;
    let (variant, header_length) = match magic {
        bytes if bytes == RAW_MAGIC => (LpbVariant::Raw, 8usize),
        bytes if bytes == XOR_MAGIC => (LpbVariant::Xor73, 16usize),
        _ => {
            return Err(FormatError::new(
                ErrorKind::BadMagic,
                0,
                "expected an rlu\\x0b or rle\\x0c LPB wrapper",
            ))
        }
    };
    reader.take(header_length - 4)?;

    let (unknown_header, advisory_size, encoded_prefix, decoded) = match variant {
        LpbVariant::Raw => (
            vec![preserve(data, 4, 4)],
            None,
            None,
            data[header_length..].to_vec(),
        ),
        LpbVariant::Xor73 => {
            let mut header = Reader::new(data);
            header.seek(8)?;
            let advisory_size = header.u32_le()?;
            let mut decoded = Vec::with_capacity(data.len().saturating_sub(11));
            decoded.extend(data[13..16].iter().map(|byte| byte ^ XOR_KEY));
            decoded.extend(data[16..].iter().map(|byte| byte ^ XOR_KEY));
            (
                vec![preserve(data, 4, 4), preserve(data, 12, 1)],
                Some(advisory_size),
                Some(Span::new(13, 3)),
                decoded,
            )
        }
    };
    if decoded.len() < LUA_51_SIGNATURE.len() {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            data.len() as u64,
            "LPB payload is shorter than a Lua 5.1 chunk signature",
        ));
    }
    if !decoded.starts_with(LUA_51_SIGNATURE) {
        return Err(FormatError::new(
            ErrorKind::InvalidLuaChunk,
            header_length as u64,
            "decoded payload does not begin with the Lua 5.1 signature",
        ));
    }

    Ok(LpbFile {
        variant,
        header: Span::new(0, header_length as u64),
        unknown_header,
        advisory_size,
        encoded_prefix,
        encoded_payload: Span::new(header_length as u64, (data.len() - header_length) as u64),
        decoded,
    })
}

fn preserve(data: &[u8], offset: usize, length: usize) -> PreservedBytes {
    PreservedBytes {
        span: Span::new(offset as u64, length as u64),
        bytes: data[offset..offset + length].to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_raw_and_preserves_header_bytes() {
        let data = b"rlu\x0bABCD\x1bLuaQbody";
        let file = extract(data).unwrap();
        assert_eq!(file.variant, LpbVariant::Raw);
        assert_eq!(file.unknown_header[0].bytes, b"ABCD");
        assert_eq!(file.decoded, b"\x1bLuaQbody");
    }

    #[test]
    fn extracts_xor_variant_without_enforcing_advisory_size() {
        let chunk = b"\x1bLuaQbody";
        let mut data = b"rle\x0cABCD\x02\x00\x00\x00Z".to_vec();
        data.extend(chunk.iter().map(|byte| byte ^ XOR_KEY));
        let file = extract(&data).unwrap();
        assert_eq!(file.variant, LpbVariant::Xor73);
        assert_eq!(file.advisory_size, Some(2));
        assert_eq!(file.unknown_header[0].bytes, b"ABCD");
        assert_eq!(file.unknown_header[1].bytes, b"Z");
        assert_eq!(file.decoded, chunk);
    }

    #[test]
    fn malformed_inputs_fail_at_stable_offsets() {
        let short = extract(b"rle\x0c").unwrap_err();
        assert_eq!(short.kind(), ErrorKind::UnexpectedEndOfInput);
        assert_eq!(short.offset(), 4);
        let bad = extract(b"rlu\x0bABCDxxxxx").unwrap_err();
        assert_eq!(bad.kind(), ErrorKind::InvalidLuaChunk);
        assert_eq!(bad.offset(), 8);
    }
}
