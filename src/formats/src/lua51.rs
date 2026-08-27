//! Bounded structural reading of the evidenced Lua 5.1 chunk representation.
//!
//! This module reads serialization structure and the official instruction
//! layout. It does not execute instructions or infer control flow. Constant
//! payloads remain exact bytes.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::{Reader, Span};

pub const HEADER_LENGTH: u64 = 12;
pub const MAX_NESTING_DEPTH: usize = 128;
pub const MAX_PROTOTYPES: u64 = 10_000;
pub const MAX_TABLE_ENTRIES: u64 = 1_000_000;
pub const MAX_STRING_BYTES: u64 = 16 * 1024 * 1024;

const EXPECTED_HEADER: &[u8; 12] = b"\x1bLuaQ\x00\x01\x04\x04\x04\x08\x00";

const SIZE_OP: u32 = 6;
const SIZE_A: u32 = 8;
const SIZE_C: u32 = 9;
const SIZE_B: u32 = 9;
const POS_A: u32 = SIZE_OP;
const POS_C: u32 = POS_A + SIZE_A;
const POS_B: u32 = POS_C + SIZE_C;
const POS_BX: u32 = POS_C;
const MASK_OP: u32 = (1 << SIZE_OP) - 1;
const MASK_A: u32 = (1 << SIZE_A) - 1;
const MASK_B: u32 = (1 << SIZE_B) - 1;
const MASK_C: u32 = (1 << SIZE_C) - 1;
const MASK_BX: u32 = (1 << (SIZE_B + SIZE_C)) - 1;
const MAXARG_SBX: i32 = (MASK_BX >> 1) as i32;
const BIT_RK: u32 = 1 << (SIZE_B - 1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lua51InstructionMode {
    Abc,
    Abx,
    Asbx,
}

impl Lua51InstructionMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Abc => "iABC",
            Self::Abx => "iABx",
            Self::Asbx => "iAsBx",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lua51ArgumentMode {
    Unused,
    Value,
    Register,
    ConstantOrRegister,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lua51Opcode {
    pub number: u8,
    pub name: &'static str,
    pub mode: Lua51InstructionMode,
    pub b_mode: Lua51ArgumentMode,
    pub c_mode: Lua51ArgumentMode,
}

macro_rules! opcode {
    ($number:literal, $name:literal, $mode:ident, $b:ident, $c:ident) => {
        Lua51Opcode {
            number: $number,
            name: $name,
            mode: Lua51InstructionMode::$mode,
            b_mode: Lua51ArgumentMode::$b,
            c_mode: Lua51ArgumentMode::$c,
        }
    };
}

// Semantic authority: https://www.lua.org/source/5.1/lopcodes.c.html,
// "ORDER OP". Argument modes are retained because an iABC B or C field is
// not always the same kind of operand.
const OPCODES: [Lua51Opcode; 38] = [
    opcode!(0, "MOVE", Abc, Register, Unused),
    opcode!(1, "LOADK", Abx, ConstantOrRegister, Unused),
    opcode!(2, "LOADBOOL", Abc, Value, Value),
    opcode!(3, "LOADNIL", Abc, Register, Unused),
    opcode!(4, "GETUPVAL", Abc, Value, Unused),
    opcode!(5, "GETGLOBAL", Abx, ConstantOrRegister, Unused),
    opcode!(6, "GETTABLE", Abc, Register, ConstantOrRegister),
    opcode!(7, "SETGLOBAL", Abx, ConstantOrRegister, Unused),
    opcode!(8, "SETUPVAL", Abc, Value, Unused),
    opcode!(9, "SETTABLE", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(10, "NEWTABLE", Abc, Value, Value),
    opcode!(11, "SELF", Abc, Register, ConstantOrRegister),
    opcode!(12, "ADD", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(13, "SUB", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(14, "MUL", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(15, "DIV", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(16, "MOD", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(17, "POW", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(18, "UNM", Abc, Register, Unused),
    opcode!(19, "NOT", Abc, Register, Unused),
    opcode!(20, "LEN", Abc, Register, Unused),
    opcode!(21, "CONCAT", Abc, Register, Register),
    opcode!(22, "JMP", Asbx, Register, Unused),
    opcode!(23, "EQ", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(24, "LT", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(25, "LE", Abc, ConstantOrRegister, ConstantOrRegister),
    opcode!(26, "TEST", Abc, Register, Value),
    opcode!(27, "TESTSET", Abc, Register, Value),
    opcode!(28, "CALL", Abc, Value, Value),
    opcode!(29, "TAILCALL", Abc, Value, Value),
    opcode!(30, "RETURN", Abc, Value, Unused),
    opcode!(31, "FORLOOP", Asbx, Register, Unused),
    opcode!(32, "FORPREP", Asbx, Register, Unused),
    opcode!(33, "TFORLOOP", Abc, Unused, Value),
    opcode!(34, "SETLIST", Abc, Value, Value),
    opcode!(35, "CLOSE", Abc, Unused, Unused),
    opcode!(36, "CLOSURE", Abx, Value, Unused),
    opcode!(37, "VARARG", Abc, Value, Unused),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lua51Operand {
    Unused { raw: u32 },
    Value { value: u32 },
    Register { index: u32, raw: u32, rk: bool },
    Constant { index: u32, raw: u32, rk: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lua51Operands {
    Abc {
        a: u32,
        b: Lua51Operand,
        c: Lua51Operand,
    },
    Abx {
        a: u32,
        bx: Lua51Operand,
    },
    Asbx {
        a: u32,
        sbx: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lua51Instruction {
    pub span: Span,
    pub index: u32,
    pub raw_word: u32,
    pub opcode: Lua51Opcode,
    pub operands: Lua51Operands,
}

impl Lua51Instruction {
    fn decode(span: Span, index: u32, raw_word: u32) -> Result<Self> {
        let opcode_number = (raw_word & MASK_OP) as u8;
        let opcode = OPCODES
            .get(opcode_number as usize)
            .copied()
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::MalformedLuaBytecode,
                    span.offset,
                    format!("Lua instruction opcode {opcode_number} is out of range"),
                )
            })?;
        let a = (raw_word >> POS_A) & MASK_A;
        let operands = match opcode.mode {
            Lua51InstructionMode::Abc => Lua51Operands::Abc {
                a,
                b: decode_argument((raw_word >> POS_B) & MASK_B, opcode.b_mode),
                c: decode_argument((raw_word >> POS_C) & MASK_C, opcode.c_mode),
            },
            Lua51InstructionMode::Abx => {
                let bx = (raw_word >> POS_BX) & MASK_BX;
                let bx = match opcode.b_mode {
                    Lua51ArgumentMode::ConstantOrRegister => Lua51Operand::Constant {
                        index: bx,
                        raw: bx,
                        rk: false,
                    },
                    Lua51ArgumentMode::Value => Lua51Operand::Value { value: bx },
                    Lua51ArgumentMode::Register => Lua51Operand::Register {
                        index: bx,
                        raw: bx,
                        rk: false,
                    },
                    Lua51ArgumentMode::Unused => Lua51Operand::Unused { raw: bx },
                };
                Lua51Operands::Abx { a, bx }
            }
            Lua51InstructionMode::Asbx => Lua51Operands::Asbx {
                a,
                sbx: ((raw_word >> POS_BX) & MASK_BX) as i32 - MAXARG_SBX,
            },
        };
        Ok(Self {
            span,
            index,
            raw_word,
            opcode,
            operands,
        })
    }
}

fn decode_argument(raw: u32, mode: Lua51ArgumentMode) -> Lua51Operand {
    match mode {
        Lua51ArgumentMode::Unused => Lua51Operand::Unused { raw },
        Lua51ArgumentMode::Value => Lua51Operand::Value { value: raw },
        Lua51ArgumentMode::Register => Lua51Operand::Register {
            index: raw,
            raw,
            rk: false,
        },
        Lua51ArgumentMode::ConstantOrRegister if raw & BIT_RK != 0 => Lua51Operand::Constant {
            index: raw & !BIT_RK,
            raw,
            rk: true,
        },
        Lua51ArgumentMode::ConstantOrRegister => Lua51Operand::Register {
            index: raw,
            raw,
            rk: true,
        },
    }
}

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
    pub decoded_instructions: Vec<Lua51Instruction>,
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
        let (instructions, decoded_instructions) = self.instructions(instruction_count)?;

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
            decoded_instructions,
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

    fn instructions(&mut self, count: u32) -> Result<(Span, Vec<Lua51Instruction>)> {
        let start = self.reader.offset();
        let capacity = usize::try_from(count)
            .map_err(|_| self.limit("instruction count does not fit this platform".to_string()))?;
        let length = capacity
            .checked_mul(4)
            .ok_or_else(|| self.limit("instruction byte length overflows".to_string()))?;
        // Prove the complete declared vector is inside the bounded input
        // before allocating its decoded representation.
        let available = self.reader.remaining();
        if available < length {
            let incomplete = available / 4;
            return Err(FormatError::new(
                ErrorKind::UnexpectedEndOfInput,
                start + (incomplete as u64 * 4),
                format!(
                    "instruction {incomplete} needs 4 bytes, {} available",
                    available % 4
                ),
            ));
        }
        let bytes = self.reader.take(length)?;
        let mut decoded = Vec::with_capacity(capacity);
        for (index, word) in bytes.chunks_exact(4).enumerate() {
            let offset = start + (index as u64 * 4);
            let raw_word = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            decoded.push(Lua51Instruction::decode(
                Span::new(offset, 4),
                index as u32,
                raw_word,
            )?);
        }
        Ok((Span::new(start, length as u64), decoded))
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
        assert_eq!(chunk.root.instruction_count, 5);
        assert_eq!(chunk.root.constants.len(), 4);
        assert_eq!(chunk.root.nested.len(), 1);
        assert_eq!(chunk.root.nested[0].constants.len(), 1);
        assert_eq!(chunk.root.decoded_instructions.len(), 5);
        assert_eq!(chunk.root.nested[0].decoded_instructions.len(), 2);
        assert_eq!(chunk.root.local_count, 1);
        assert_eq!(chunk.root.upvalue_name_count, 1);
    }

    #[test]
    fn decodes_official_bit_allocation_names_modes_and_rk() {
        let abc_word = 12 | (0xabu32 << POS_A) | (0x101u32 << POS_B) | (0x055u32 << POS_C);
        let abc = Lua51Instruction::decode(Span::new(40, 4), 7, abc_word).unwrap();
        assert_eq!(abc.span, Span::new(40, 4));
        assert_eq!(abc.index, 7);
        assert_eq!(abc.raw_word, abc_word);
        assert_eq!(abc.opcode.name, "ADD");
        assert_eq!(abc.opcode.mode, Lua51InstructionMode::Abc);
        assert_eq!(
            abc.operands,
            Lua51Operands::Abc {
                a: 0xab,
                b: Lua51Operand::Constant {
                    index: 1,
                    raw: 0x101,
                    rk: true,
                },
                c: Lua51Operand::Register {
                    index: 0x55,
                    raw: 0x55,
                    rk: true,
                },
            }
        );

        let abx_word = 1 | (0x22u32 << POS_A) | (0x2aaaau32 << POS_BX);
        let abx = Lua51Instruction::decode(Span::new(44, 4), 8, abx_word).unwrap();
        assert_eq!(abx.opcode.name, "LOADK");
        assert_eq!(abx.opcode.mode, Lua51InstructionMode::Abx);
        assert_eq!(
            abx.operands,
            Lua51Operands::Abx {
                a: 0x22,
                bx: Lua51Operand::Constant {
                    index: 0x2aaaa,
                    raw: 0x2aaaa,
                    rk: false,
                },
            }
        );

        for (encoded, expected) in [(0, -131_071), (131_071, 0), (262_143, 131_072)] {
            let word = 22 | (encoded << POS_BX);
            let instruction = Lua51Instruction::decode(Span::new(48, 4), 9, word).unwrap();
            assert_eq!(instruction.opcode.name, "JMP");
            assert_eq!(instruction.opcode.mode, Lua51InstructionMode::Asbx);
            assert_eq!(
                instruction.operands,
                Lua51Operands::Asbx {
                    a: 0,
                    sbx: expected,
                }
            );
        }
    }

    #[test]
    fn official_opcode_table_is_complete_and_mode_stable() {
        let names: Vec<&str> = OPCODES.iter().map(|opcode| opcode.name).collect();
        assert_eq!(
            names,
            [
                "MOVE",
                "LOADK",
                "LOADBOOL",
                "LOADNIL",
                "GETUPVAL",
                "GETGLOBAL",
                "GETTABLE",
                "SETGLOBAL",
                "SETUPVAL",
                "SETTABLE",
                "NEWTABLE",
                "SELF",
                "ADD",
                "SUB",
                "MUL",
                "DIV",
                "MOD",
                "POW",
                "UNM",
                "NOT",
                "LEN",
                "CONCAT",
                "JMP",
                "EQ",
                "LT",
                "LE",
                "TEST",
                "TESTSET",
                "CALL",
                "TAILCALL",
                "RETURN",
                "FORLOOP",
                "FORPREP",
                "TFORLOOP",
                "SETLIST",
                "CLOSE",
                "CLOSURE",
                "VARARG",
            ]
        );
        let modes: Vec<Lua51InstructionMode> = OPCODES.iter().map(|opcode| opcode.mode).collect();
        assert_eq!(
            modes,
            [
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abx,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abx,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abx,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Asbx,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Asbx,
                Lua51InstructionMode::Asbx,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abc,
                Lua51InstructionMode::Abx,
                Lua51InstructionMode::Abc,
            ]
        );
        for (number, opcode) in OPCODES.iter().enumerate() {
            assert_eq!(opcode.number as usize, number);
        }
        use Lua51ArgumentMode::{ConstantOrRegister as K, Register as R, Unused as N, Value as U};
        let argument_modes: Vec<(Lua51ArgumentMode, Lua51ArgumentMode)> = OPCODES
            .iter()
            .map(|opcode| (opcode.b_mode, opcode.c_mode))
            .collect();
        assert_eq!(
            argument_modes,
            [
                (R, N),
                (K, N),
                (U, U),
                (R, N),
                (U, N),
                (K, N),
                (R, K),
                (K, N),
                (U, N),
                (K, K),
                (U, U),
                (R, K),
                (K, K),
                (K, K),
                (K, K),
                (K, K),
                (K, K),
                (K, K),
                (R, N),
                (R, N),
                (R, N),
                (R, R),
                (R, N),
                (K, K),
                (K, K),
                (K, K),
                (R, U),
                (R, U),
                (U, U),
                (U, U),
                (U, N),
                (R, N),
                (R, N),
                (N, U),
                (U, U),
                (N, N),
                (U, N),
                (U, N),
            ]
        );
    }

    #[test]
    fn rejects_an_opcode_outside_the_official_table() {
        let error = Lua51Instruction::decode(Span::new(52, 4), 0, 38).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedLuaBytecode);
        assert_eq!(error.offset(), 52);
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
