//! Exact encoding of the static-actor SAN record table.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;
use crate::staticactor::{StaticActorSan, HEADER_SIZE, MAGIC, MAX_RECORDS, XOR_KEY};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticActorEntry {
    pub value: u32,
    /// Decoded bytes without the zero terminator.
    pub string: Vec<u8>,
}

/// Preserve supplied order and byte strings; character and path rules are not imposed.
pub fn encode_staticactor(
    unknown_header: [u8; 5],
    entries: &[StaticActorEntry],
) -> Result<Vec<u8>> {
    if entries.len() > MAX_RECORDS as usize {
        return Err(FormatError::new(
            ErrorKind::ResourceLimitExceeded,
            9,
            format!("static-actor SAN record limit is {MAX_RECORDS}"),
        ));
    }
    let mut length = HEADER_SIZE;
    for entry in entries {
        if let Some(index) = entry.string.iter().position(|byte| *byte == 0) {
            return Err(FormatError::new(
                ErrorKind::InvalidAttributeValue,
                (length + 4 + index) as u64,
                "static-actor SAN string contains NUL",
            ));
        }
        length = length
            .checked_add(5)
            .and_then(|n| n.checked_add(entry.string.len()))
            .ok_or_else(|| {
                FormatError::new(
                    ErrorKind::ResourceLimitExceeded,
                    length as u64,
                    "static-actor SAN size overflows the address space",
                )
            })?;
    }
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&unknown_header);
    bytes.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for entry in entries {
        bytes.extend_from_slice(&entry.value.to_be_bytes());
        bytes.extend_from_slice(&entry.string);
        bytes.push(0);
    }
    for byte in &mut bytes[4..] {
        *byte ^= XOR_KEY;
    }
    Ok(bytes)
}

pub fn decoded_entries(data: &[u8], san: &StaticActorSan) -> Result<Vec<StaticActorEntry>> {
    san.records
        .iter()
        .map(|record| {
            Ok(StaticActorEntry {
                value: record.value_be,
                string: decoded_span(data, record.string_span)?,
            })
        })
        .collect()
}

pub fn decoded_unknown_header(data: &[u8], san: &StaticActorSan) -> Result<[u8; 5]> {
    decoded_span(data, san.unknown_header)?
        .try_into()
        .map_err(|_| {
            FormatError::new(
                ErrorKind::InvalidAttributeValue,
                san.unknown_header.offset,
                "static-actor SAN unknown header must have five bytes",
            )
        })
}

fn decoded_span(data: &[u8], span: Span) -> Result<Vec<u8>> {
    let end = span
        .offset
        .checked_add(span.length)
        .filter(|end| *end <= data.len() as u64)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::DeclaredSizeOutOfRange,
                span.offset,
                "static-actor SAN decoded span escapes the input",
            )
        })?;
    Ok(data[span.offset as usize..end as usize]
        .iter()
        .map(|byte| byte ^ XOR_KEY)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{validate_named_bytes_as, InspectAs};
    use crate::staticactor::parse;

    #[test]
    fn encodes_literal_bytes_and_rebuilds_without_path_or_order_rules() {
        let entries = vec![
            StaticActorEntry {
                value: 7,
                string: vec![0xff, 0x01],
            },
            StaticActorEntry {
                value: 0,
                string: vec![],
            },
            StaticActorEntry {
                value: 7,
                string: b"plain".to_vec(),
            },
        ];
        let bytes = encode_staticactor([1, 2, 3, 4, 5], &entries).unwrap();
        assert_eq!(
            bytes.as_slice(),
            &[
                b's', b'a', b'n', b'e', 0x72, 0x71, 0x70, 0x77, 0x76, 0x73, 0x73, 0x73, 0x70, 0x73,
                0x73, 0x73, 0x74, 0x8c, 0x72, 0x73, 0x73, 0x73, 0x73, 0x73, 0x73, 0x73, 0x73, 0x73,
                0x74, 0x03, 0x1f, 0x12, 0x1a, 0x1d, 0x73
            ]
        );
        let san = parse(&bytes).unwrap();
        assert_eq!(
            decoded_unknown_header(&bytes, &san).unwrap(),
            [1, 2, 3, 4, 5]
        );
        assert_eq!(decoded_entries(&bytes, &san).unwrap(), entries);
        assert_eq!(
            encode_staticactor(
                decoded_unknown_header(&bytes, &san).unwrap(),
                &decoded_entries(&bytes, &san).unwrap()
            )
            .unwrap(),
            bytes
        );
        for how in [InspectAs::Auto, InspectAs::StaticActorSan] {
            let report = validate_named_bytes_as(&bytes, "test.san", &how).unwrap();
            assert_eq!(report["checks"][1]["status"], "pass");
        }
        let empty = encode_staticactor([0; 5], &[]).unwrap();
        assert_eq!(empty, b"sanesssssssss");
        assert!(decoded_entries(&empty, &parse(&empty).unwrap())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn refuses_nul_and_excess_records() {
        let entry = StaticActorEntry {
            value: 0,
            string: b"a\0b".to_vec(),
        };
        let error = encode_staticactor([0; 5], &[entry]).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::InvalidAttributeValue, 18)
        );
        let entries = vec![
            StaticActorEntry {
                value: 0,
                string: vec![]
            };
            MAX_RECORDS as usize
        ];
        assert_eq!(
            parse(&encode_staticactor([0; 5], &entries).unwrap())
                .unwrap()
                .declared_count,
            MAX_RECORDS
        );
        let mut entries = entries;
        entries.push(StaticActorEntry {
            value: 0,
            string: vec![],
        });
        let error = encode_staticactor([0; 5], &entries).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::ResourceLimitExceeded, 9)
        );
    }

    #[test]
    fn refuses_spans_outside_the_supplied_input() {
        let bytes = encode_staticactor(
            [0; 5],
            &[StaticActorEntry {
                value: 1,
                string: b"x".to_vec(),
            }],
        )
        .unwrap();
        let san = parse(&bytes).unwrap();
        let error = decoded_entries(&bytes[..13], &san).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::DeclaredSizeOutOfRange, 17)
        );
        let error = decoded_unknown_header(&bytes[..4], &san).unwrap_err();
        assert_eq!(
            (error.kind(), error.offset()),
            (ErrorKind::DeclaredSizeOutOfRange, 4)
        );
    }
}
