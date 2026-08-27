//! Bounded reading of the static-actor SAN table.
//!
//! The retail file establishes only framing here. Bytes from offset 4 onward
//! are XOR-0x73 encoded. The decoded header has five uninterpreted bytes, a
//! big-endian record count, then records made of a big-endian four-byte value
//! and a zero-terminated byte string. Neither record member has a claimed
//! semantic name.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

pub const MAGIC: &[u8; 4] = b"sane";
pub const XOR_KEY: u8 = 0x73;
pub const HEADER_SIZE: usize = 13;
pub const MAX_RECORDS: u32 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticActorRecord {
    pub index: u32,
    pub span: Span,
    pub value_span: Span,
    pub value_be: u32,
    pub string_span: Span,
    pub terminator_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticActorSan {
    pub header: Span,
    pub unknown_header: Span,
    pub count_span: Span,
    pub declared_count: u32,
    pub encoded_body: Span,
    pub records: Vec<StaticActorRecord>,
}

pub fn has_signature(data: &[u8]) -> bool {
    data.starts_with(MAGIC)
}

pub fn parse(data: &[u8]) -> Result<StaticActorSan> {
    if data.len() < MAGIC.len() {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            0,
            "static-actor SAN magic is truncated",
        ));
    }
    if !has_signature(data) {
        return Err(FormatError::new(
            ErrorKind::BadMagic,
            0,
            "expected static-actor SAN magic 'sane'",
        ));
    }
    if data.len() < 9 {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            4,
            "static-actor SAN unknown header field is truncated",
        ));
    }
    if data.len() < HEADER_SIZE {
        return Err(FormatError::new(
            ErrorKind::UnexpectedEndOfInput,
            9,
            "static-actor SAN record count is truncated",
        ));
    }

    let declared_count = decoded_u32_be(&data[9..13]);
    if declared_count > MAX_RECORDS {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            9,
            format!("static-actor SAN declares {declared_count} records; limit is {MAX_RECORDS}"),
        ));
    }

    // Do not reserve from an untrusted count. The input itself bounds how
    // many complete records can be discovered before allocation.
    let mut records = Vec::new();
    let mut position = HEADER_SIZE;
    for index in 0..declared_count {
        let record_start = position;
        let value_end = position.checked_add(4).ok_or_else(|| {
            FormatError::new(
                ErrorKind::ResourceLimitExceeded,
                position as u64,
                "record offset overflows the address space",
            )
        })?;
        if value_end > data.len() {
            return Err(partial_record(record_start, index));
        }
        let value_be = decoded_u32_be(&data[position..value_end]);
        position = value_end;

        let Some(relative_end) = data[position..].iter().position(|byte| *byte == XOR_KEY) else {
            return Err(partial_record(record_start, index));
        };
        let string_end = position + relative_end;
        position = string_end + 1;
        records.push(StaticActorRecord {
            index,
            span: Span::new(record_start as u64, (position - record_start) as u64),
            value_span: Span::new(record_start as u64, 4),
            value_be,
            string_span: Span::new(value_end as u64, relative_end as u64),
            terminator_span: Span::new(string_end as u64, 1),
        });
    }

    if position != data.len() {
        let remaining = data.len() - position;
        let kind = if remaining < 5 {
            ErrorKind::TrailingPartialRecord
        } else {
            ErrorKind::TrailingBytes
        };
        return Err(FormatError::new(
            kind,
            position as u64,
            format!("{remaining} byte(s) remain after the declared record count"),
        ));
    }

    Ok(StaticActorSan {
        header: Span::new(0, HEADER_SIZE as u64),
        unknown_header: Span::new(4, 5),
        count_span: Span::new(9, 4),
        declared_count,
        encoded_body: Span::new(HEADER_SIZE as u64, (data.len() - HEADER_SIZE) as u64),
        records,
    })
}

fn decoded_u32_be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([
        bytes[0] ^ XOR_KEY,
        bytes[1] ^ XOR_KEY,
        bytes[2] ^ XOR_KEY,
        bytes[3] ^ XOR_KEY,
    ])
}

fn partial_record(offset: usize, index: u32) -> FormatError {
    FormatError::new(
        ErrorKind::TrailingPartialRecord,
        offset as u64,
        format!("static-actor SAN record {index} is truncated or unterminated"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(decoded: &[u8]) -> Vec<u8> {
        decoded.iter().map(|byte| byte ^ XOR_KEY).collect()
    }

    fn fixture() -> Vec<u8> {
        let mut data = MAGIC.to_vec();
        data.extend(encode(&[1, 2, 3, 4, 5]));
        data.extend(encode(&2u32.to_be_bytes()));
        data.extend(encode(&7u32.to_be_bytes()));
        data.extend(encode(b"/Synthetic/One\0"));
        data.extend(encode(&0x1020_3040u32.to_be_bytes()));
        data.extend(encode(b"/Synthetic/Two\0"));
        data
    }

    #[test]
    fn parses_the_evidenced_framing() {
        let data = fixture();
        let parsed = parse(&data).unwrap();
        assert_eq!(parsed.declared_count, 2);
        assert_eq!(parsed.unknown_header, Span::new(4, 5));
        assert_eq!(parsed.records[0].value_be, 7);
        let span = parsed.records[1].string_span;
        let body = &data[span.offset as usize..span.end() as usize];
        assert_eq!(
            body.iter().map(|byte| byte ^ XOR_KEY).collect::<Vec<_>>(),
            b"/Synthetic/Two"
        );
        assert_eq!(parsed.records[1].span.end(), data.len() as u64);
    }

    #[test]
    fn refuses_an_allocation_bomb_before_reading_records() {
        let mut data = MAGIC.to_vec();
        data.extend(encode(&[0; 5]));
        data.extend(encode(&(MAX_RECORDS + 1).to_be_bytes()));
        let error = parse(&data).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimitExceeded);
        assert_eq!(error.offset(), 9);
    }

    #[test]
    fn header_truncations_report_the_field_start() {
        let data = fixture();
        for (length, offset) in [(0, 0), (3, 0), (4, 4), (8, 4), (9, 9), (12, 9)] {
            let error = parse(&data[..length]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::UnexpectedEndOfInput);
            assert_eq!(error.offset(), offset, "cut at {length}");
        }
    }
}
