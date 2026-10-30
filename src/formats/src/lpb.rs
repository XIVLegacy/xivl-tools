//! Bounded extraction and exact writing of the two evidenced LPB wrappers.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LpbWrapper {
    Rle { tag: [u8; 4], byte12: u8 },
    Rlu { tag: [u8; 4] },
}

impl LpbFile {
    /// Recover the wrapper choice and uninterpreted bytes from an extracted file.
    pub fn wrapper(&self) -> Result<LpbWrapper> {
        let field = |index: usize, offset: u64, length: usize| {
            self.unknown_header
                .get(index)
                .filter(|field| {
                    field.span == Span::new(offset, length as u64) && field.bytes.len() == length
                })
                .map(|field| field.bytes.as_slice())
                .ok_or_else(|| {
                    FormatError::new(
                        ErrorKind::InvalidAttributeValue,
                        offset,
                        "missing or inconsistent preserved LPB header bytes",
                    )
                })
        };
        let tag = field(0, 4, 4)?.try_into().expect("checked four-byte tag");
        Ok(match self.variant {
            LpbVariant::Raw => LpbWrapper::Rlu { tag },
            LpbVariant::Xor73 => LpbWrapper::Rle {
                tag,
                byte12: field(1, 12, 1)?[0],
            },
        })
    }
}

/// Wrap a complete decoded chunk. Only its Lua 5.1 signature is checked here.
pub fn encode_lpb(wrapper: &LpbWrapper, chunk: &[u8]) -> Result<Vec<u8>> {
    if !chunk.starts_with(LUA_51_SIGNATURE) {
        return Err(FormatError::new(
            ErrorKind::InvalidLuaChunk,
            0,
            "chunk does not begin with the Lua 5.1 signature",
        ));
    }
    let size = match wrapper {
        LpbWrapper::Rle { .. } => rle_size(chunk.len())?,
        LpbWrapper::Rlu { .. } => 0,
    };
    Ok(write_lpb(wrapper, chunk, size))
}

fn write_lpb(wrapper: &LpbWrapper, chunk: &[u8], size: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    match wrapper {
        LpbWrapper::Rle { tag, byte12 } => {
            bytes.extend_from_slice(XOR_MAGIC);
            bytes.extend_from_slice(tag);
            bytes.extend_from_slice(&size.to_le_bytes());
            bytes.push(*byte12);
            bytes.extend(chunk.iter().map(|byte| byte ^ XOR_KEY));
        }
        LpbWrapper::Rlu { tag } => {
            bytes.extend_from_slice(RAW_MAGIC);
            bytes.extend_from_slice(tag);
            bytes.extend_from_slice(chunk);
        }
    }
    bytes
}

/// Rebuild an extracted model paired with its source, retaining a noncanonical size word.
pub fn rebuild_lpb(data: &[u8], file: &LpbFile) -> Result<Vec<u8>> {
    if extract(data)? != *file {
        return Err(FormatError::new(
            ErrorKind::InvalidAttributeValue,
            0,
            "LPB model does not match its source",
        ));
    }
    Ok(write_lpb(
        &file.wrapper()?,
        &file.decoded,
        file.advisory_size.unwrap_or(0),
    ))
}

fn rle_size(length: usize) -> Result<u32> {
    u32::try_from(length).map_err(|_| {
        FormatError::new(
            ErrorKind::DeclaredSizeOutOfRange,
            8,
            "RLE chunk length exceeds the u32 size word",
        )
    })
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
        assert_eq!(rebuild_lpb(data, &file).unwrap(), data);
        assert_eq!(
            encode_lpb(&file.wrapper().unwrap(), &file.decoded).unwrap(),
            data
        );
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
        assert_eq!(rebuild_lpb(&data, &file).unwrap(), data);
        let encoded = encode_lpb(&file.wrapper().unwrap(), chunk).unwrap();
        let mut expected = b"rle\x0cABCD\x09\x00\x00\x00Z".to_vec();
        expected.extend_from_slice(b"h?\x06\x12\x22\x11\x1c\x17\x0a");
        assert_eq!(encoded, expected);
        assert_eq!(
            rebuild_lpb(&encoded, &extract(&encoded).unwrap()).unwrap(),
            encoded
        );
    }

    #[test]
    fn refuses_invalid_chunks_and_inconsistent_models() {
        for wrapper in [
            LpbWrapper::Rle {
                tag: *b"ABCD",
                byte12: 0xff,
            },
            LpbWrapper::Rlu { tag: *b"ABCD" },
        ] {
            for chunk in [b"".as_slice(), b"\x1bLua", b"\x1bLuaRbody"] {
                let error = encode_lpb(&wrapper, chunk).unwrap_err();
                assert_eq!(error.kind(), ErrorKind::InvalidLuaChunk);
                assert_eq!(error.offset(), 0);
            }
        }
        let data = b"rlu\x0bABCD\x1bLuaQbody";
        let mut file = extract(data).unwrap();
        file.decoded.push(0);
        assert_eq!(
            rebuild_lpb(data, &file).unwrap_err().kind(),
            ErrorKind::InvalidAttributeValue
        );
        file.unknown_header.clear();
        assert_eq!(file.wrapper().unwrap_err().offset(), 4);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn refuses_rle_size_overflow() {
        let error = rle_size(u32::MAX as usize + 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DeclaredSizeOutOfRange);
        assert_eq!(error.offset(), 8);
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
