//! Bounded structural reading of the evidenced Lua 5.1 chunk representation.
//!
//! This module reads serialization structure, not VM semantics. Instructions
//! remain opaque words and constant payloads remain exact bytes.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};

pub const HEADER_LENGTH: u64 = 12;
pub const MAX_NESTING_DEPTH: usize = 128;
pub const MAX_PROTOTYPES: u64 = 10_000;
pub const MAX_TABLE_ENTRIES: u64 = 1_000_000;
pub const MAX_STRING_BYTES: u64 = 16 * 1024 * 1024;

const EXPECTED_HEADER: &[u8; 12] = b"\x1bLuaQ\x00\x01\x04\x04\x04\x08\x00";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lua51Header {
    pub span: Span,
    pub version: u8,
    pub format: u8,
    pub little_endian: bool,
    pub int_size: u8,
    pub size_t_size: u8,
    pub instruction_size: u8,
    pub number_size: u8,
    pub integral_numbers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuaString {
    /// String body including its required trailing zero byte.
    pub span: Span,
    /// String body without its trailing zero byte.
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaConstant {
    Nil { span: Span },
    Boolean { span: Span, value: bool },
    Number { span: Span, bits: u64 },
    String { span: Span, value: LuaString },
}

impl LuaConstant {
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::Nil { .. } => "nil",
            Self::Boolean { .. } => "boolean",
            Self::Number { .. } => "number",
            Self::String { .. } => "string",
        }
    }

    pub const fn span(&self) -> Span {
        match self {
            Self::Nil { span }
            | Self::Boolean { span, .. }
            | Self::Number { span, .. }
            | Self::String { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lua51Prototype {
    pub span: Span,
    pub source: Option<LuaString>,
    pub line_defined: u32,
    pub last_line_defined: u32,
    pub upvalue_count: u8,
    pub parameter_count: u8,
    pub vararg_flags: u8,
    pub max_stack_size: u8,
    pub instructions: Span,
    pub instruction_count: u32,
    pub constants: Vec<LuaConstant>,
    pub nested: Vec<Lua51Prototype>,
    pub line_info: Span,
    pub line_info_count: u32,
    pub local_count: u32,
    pub upvalue_name_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lua51Chunk {
    pub header: Lua51Header,
    pub root: Lua51Prototype,
}

pub fn parse(data: &[u8]) -> Result<Lua51Chunk> {
    let mut parser = Parser {
        reader: Reader::new(data),
        prototypes: 0,
        table_entries: 0,
        string_bytes: 0,
    };
    let header = parser.header()?;
    let root = parser.prototype(0)?;
    if parser.reader.remaining() != 0 {
        return Err(FormatError::new(
            ErrorKind::TrailingBytes,
            parser.reader.offset(),
            format!(
                "{} byte(s) follow the root Lua prototype",
                parser.reader.remaining()
            ),
        ));
    }
    Ok(Lua51Chunk { header, root })
}

struct Parser<'a> {
    reader: Reader<'a>,
    prototypes: u64,
    table_entries: u64,
    string_bytes: u64,
}

impl Parser<'_> {
    fn header(&mut self) -> Result<Lua51Header> {
        let bytes = self.reader.take(HEADER_LENGTH as usize)?;
        if bytes != EXPECTED_HEADER {
            let mismatch = bytes
                .iter()
                .zip(EXPECTED_HEADER)
                .position(|(actual, expected)| actual != expected)
                .unwrap_or(0);
            return Err(FormatError::new(
                ErrorKind::UnsupportedLuaHeader,
                mismatch as u64,
                "expected official little-endian Lua 5.1 with 32-bit int, size_t, and instruction plus 64-bit floating number",
            ));
        }
        Ok(Lua51Header {
            span: Span::new(0, HEADER_LENGTH),
            version: bytes[4],
            format: bytes[5],
            little_endian: bytes[6] == 1,
            int_size: bytes[7],
            size_t_size: bytes[8],
            instruction_size: bytes[9],
            number_size: bytes[10],
            integral_numbers: bytes[11] != 0,
        })
    }

    fn prototype(&mut self, depth: usize) -> Result<Lua51Prototype> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(self.limit(format!("prototype nesting reaches {MAX_NESTING_DEPTH}")));
        }
        self.prototypes += 1;
        if self.prototypes > MAX_PROTOTYPES {
            return Err(self.limit(format!("prototype count exceeds {MAX_PROTOTYPES}")));
        }
        let start = self.reader.offset();
        let source = self.string()?;
        let line_defined = self.nonnegative_int("line-defined")?;
        let last_line_defined = self.nonnegative_int("last-line-defined")?;
        let upvalue_count = self.reader.u8()?;
        let parameter_count = self.reader.u8()?;
        let vararg_flags = self.reader.u8()?;
        let max_stack_size = self.reader.u8()?;

        let instruction_count = self.count("instruction")?;
        let instructions = self.vector(instruction_count, 4, "instruction")?;

        let constant_count = self.count("constant")?;
        let mut constants = Vec::new();
        for _ in 0..constant_count {
            constants.push(self.constant()?);
        }

        let nested_count = self.count("nested prototype")?;
        let mut nested = Vec::new();
        for _ in 0..nested_count {
            nested.push(self.prototype(depth + 1)?);
        }

        let line_info_count = self.count("line-info")?;
        let line_info = self.vector(line_info_count, 4, "line-info")?;
        let local_count = self.count("local")?;
        for _ in 0..local_count {
            self.string()?;
            self.nonnegative_int("local start pc")?;
            self.nonnegative_int("local end pc")?;
        }
        let upvalue_name_count = self.count("upvalue name")?;
        for _ in 0..upvalue_name_count {
            self.string()?;
        }

        Ok(Lua51Prototype {
            span: Span::new(start, self.reader.offset() - start),
            source,
            line_defined,
            last_line_defined,
            upvalue_count,
            parameter_count,
            vararg_flags,
            max_stack_size,
            instructions,
            instruction_count,
            constants,
            nested,
            line_info,
            line_info_count,
            local_count,
            upvalue_name_count,
        })
    }

    fn constant(&mut self) -> Result<LuaConstant> {
        let start = self.reader.offset();
        match self.reader.u8()? {
            0 => Ok(LuaConstant::Nil {
                span: Span::new(start, 1),
            }),
            1 => {
                let value = self.reader.u8()?;
                Ok(LuaConstant::Boolean {
                    span: Span::new(start, 2),
                    value: value != 0,
                })
            }
            3 => {
                let bits = self.reader.u64_le()?;
                Ok(LuaConstant::Number {
                    span: Span::new(start, 9),
                    bits,
                })
            }
            4 => {
                let Some(value) = self.string()? else {
                    return Err(FormatError::new(
                        ErrorKind::MalformedLuaBytecode,
                        start + 1,
                        "a string constant has a zero length",
                    ));
                };
                Ok(LuaConstant::String {
                    span: Span::new(start, self.reader.offset() - start),
                    value,
                })
            }
            tag => Err(FormatError::new(
                ErrorKind::MalformedLuaBytecode,
                start,
                format!("unknown Lua constant tag {tag}"),
            )),
        }
    }

    fn string(&mut self) -> Result<Option<LuaString>> {
        let size_offset = self.reader.offset();
        let size = self.reader.u32_le()? as u64;
        if size == 0 {
            return Ok(None);
        }
        if size > MAX_STRING_BYTES {
            return Err(FormatError::new(
                ErrorKind::ResourceLimitExceeded,
                size_offset,
                format!("Lua string length {size} exceeds {MAX_STRING_BYTES}"),
            ));
        }
        self.string_bytes = self
            .string_bytes
            .checked_add(size)
            .ok_or_else(|| self.limit("aggregate string size overflows".to_string()))?;
        if self.string_bytes > MAX_STRING_BYTES {
            return Err(self.limit(format!(
                "aggregate Lua string bytes exceed {MAX_STRING_BYTES}"
            )));
        }
        let length = usize::try_from(size)
            .map_err(|_| self.limit("string length does not fit this platform".to_string()))?;
        let start = self.reader.offset();
        let bytes = self.reader.take(length)?;
        if bytes.last() != Some(&0) {
            return Err(FormatError::new(
                ErrorKind::MalformedLuaBytecode,
                start + size - 1,
                "Lua string does not end in a zero byte",
            ));
        }
        Ok(Some(LuaString {
            span: Span::new(start, size),
            bytes: bytes[..bytes.len() - 1].to_vec(),
        }))
    }

    fn count(&mut self, name: &str) -> Result<u32> {
        let offset = self.reader.offset();
        let count = self.nonnegative_int(name)?;
        self.table_entries = self
            .table_entries
            .checked_add(count as u64)
            .ok_or_else(|| self.limit("aggregate table count overflows".to_string()))?;
        if self.table_entries > MAX_TABLE_ENTRIES {
            return Err(FormatError::new(
                ErrorKind::ResourceLimitExceeded,
                offset,
                format!(
                    "aggregate Lua table entries exceed {MAX_TABLE_ENTRIES} while reading {name}"
                ),
            ));
        }
        Ok(count)
    }

    fn nonnegative_int(&mut self, name: &str) -> Result<u32> {
        let offset = self.reader.offset();
        let value = self.reader.u32_le()?;
        if value > i32::MAX as u32 {
            return Err(FormatError::new(
                ErrorKind::MalformedLuaBytecode,
                offset,
                format!("{name} is negative in the signed Lua int representation"),
            ));
        }
        Ok(value)
    }

    fn vector(&mut self, count: u32, width: usize, name: &str) -> Result<Span> {
        let start = self.reader.offset();
        let count = usize::try_from(count)
            .map_err(|_| self.limit(format!("{name} count does not fit this platform")))?;
        let length = count
            .checked_mul(width)
            .ok_or_else(|| self.limit(format!("{name} byte length overflows")))?;
        self.reader.take(length)?;
        Ok(Span::new(start, length as u64))
    }

    fn limit(&self, detail: String) -> FormatError {
        FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            self.reader.offset(),
            detail,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let bytes: &[u8] = match name {
            "valid" => include_bytes!("../../../tests/fixtures/public/lpb/bytecode.bin"),
            "trailing" => {
                include_bytes!("../../../tests/fixtures/public/lpb/bytecode-trailing.bin")
            }
            "string-bomb" => {
                include_bytes!("../../../tests/fixtures/public/lpb/bytecode-string-bomb.bin")
            }
            "nesting-bomb" => {
                include_bytes!("../../../tests/fixtures/public/lpb/bytecode-nesting-bomb.bin")
            }
            _ => unreachable!(),
        };
        crate::lpb::extract(bytes).unwrap().decoded
    }

    #[test]
    fn reads_constants_and_nested_prototypes() {
        let chunk = parse(&fixture("valid")).unwrap();
        assert_eq!(chunk.header.version, 0x51);
        assert_eq!(chunk.root.instruction_count, 2);
        assert_eq!(chunk.root.constants.len(), 4);
        assert_eq!(chunk.root.nested.len(), 1);
        assert_eq!(chunk.root.nested[0].constants.len(), 1);
        assert_eq!(chunk.root.local_count, 1);
        assert_eq!(chunk.root.upvalue_name_count, 1);
    }

    #[test]
    fn rejects_trailing_bytes_and_resource_bombs() {
        assert_eq!(
            parse(&fixture("trailing")).unwrap_err().kind(),
            ErrorKind::TrailingBytes
        );
        assert_eq!(
            parse(&fixture("string-bomb")).unwrap_err().kind(),
            ErrorKind::ResourceLimitExceeded
        );
        assert_eq!(
            parse(&fixture("nesting-bomb")).unwrap_err().kind(),
            ErrorKind::ResourceLimitExceeded
        );
    }

    #[test]
    fn rejects_every_non_target_header_field() {
        let decoded = fixture("valid");
        for offset in 4..HEADER_LENGTH as usize {
            let mut changed = decoded.clone();
            changed[offset] ^= 0xff;
            let error = parse(&changed).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::UnsupportedLuaHeader, "{offset}");
            assert_eq!(error.offset(), offset as u64, "{offset}");
        }
    }
}
