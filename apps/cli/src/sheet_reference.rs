//! Conditional sheet references and copied controls in default fields.

use serde_json::{json, Value};
use xivl_formats::reader::to_hex;
use xivl_formats::richstring::{Expression, RichString, Segment, Token};

use crate::Failure;

// Diagnostic budgets bound recursion and expansion; these are not native limits.
const MAX_DEPTH: usize = 16;
const MAX_REFERENCES: usize = 128;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

pub(super) struct Cell {
    pub rich: RichString,
    pub evidence: Value,
}

pub(super) type QueryResult = std::result::Result<Cell, &'static str>;

pub(super) fn is_copied_control(token: &Token) -> bool {
    // AT-COMP-020: these constructor slots stay null. Restrict preservation
    // to the observed direct-length forms.
    matches!(
        (
            token.code,
            token.length_bytes.as_slice(),
            token.payload.as_slice()
        ),
        (0x10 | 0x16 | 0x1D | 0x1F, [1], []) | (0x1A, [2], [1 | 2])
    )
}

fn arguments(token: &Token) -> Option<(String, u32, usize)> {
    if token.code != 0x28 {
        return None;
    }
    let expressions = token.expressions().ok()?;
    let [Expression::String { value, .. }, Expression::Integer { value: key, .. }, Expression::Integer { value: column, .. }] =
        expressions.as_slice()
    else {
        return None;
    };
    if value.tokens().next().is_some() || *column > 255 {
        return None;
    }
    Some((value.text_only(), *key, *column as usize))
}

struct Output {
    bytes: Vec<u8>,
    preserved_controls: usize,
    references: Vec<Value>,
}

fn append(output: &mut Output, bytes: &[u8]) -> std::result::Result<(), &'static str> {
    if bytes.len() > MAX_OUTPUT_BYTES - output.bytes.len() {
        return Err("sheet-expansion-limit");
    }
    output.bytes.extend_from_slice(bytes);
    Ok(())
}

fn evaluate(
    rich: &RichString,
    depth: usize,
    output: &mut Output,
    query: &mut impl FnMut(&str, u32, usize) -> std::result::Result<QueryResult, Failure>,
) -> std::result::Result<std::result::Result<(), &'static str>, Failure> {
    for segment in &rich.segments {
        match segment {
            Segment::Text { text, .. } => {
                if let Err(reason) = append(output, text.as_bytes()) {
                    return Ok(Err(reason));
                }
            }
            Segment::Token(token) => {
                if is_copied_control(token) {
                    if let Err(reason) = append(output, &token.raw_bytes()) {
                        return Ok(Err(reason));
                    }
                    output.preserved_controls += 1;
                    continue;
                }
                if token.code != 0x28 {
                    return Ok(Err("embedded-controls"));
                }
                let Some((sheet, key, column)) = arguments(token) else {
                    return Ok(Err("unsupported-sheet-expression"));
                };
                if depth >= MAX_DEPTH || output.references.len() >= MAX_REFERENCES {
                    return Ok(Err("sheet-expansion-limit"));
                }
                let cell = match query(&sheet, key, column)? {
                    Ok(cell) => cell,
                    Err(reason) => return Ok(Err(reason)),
                };
                output.references.push(cell.evidence);
                if let Err(reason) = evaluate(&cell.rich, depth + 1, output, query)? {
                    return Ok(Err(reason));
                }
            }
        }
    }
    Ok(Ok(()))
}

pub(super) fn resolve(
    rich: &RichString,
    column: usize,
    mut query: impl FnMut(&str, u32, usize) -> std::result::Result<QueryResult, Failure>,
) -> std::result::Result<Value, Failure> {
    let mut output = Output {
        bytes: Vec::new(),
        preserved_controls: 0,
        references: Vec::new(),
    };
    let result = evaluate(rich, 0, &mut output, &mut query)?;
    Ok(match result {
        Ok(()) if output.preserved_controls != 0 => json!({"status": "resolved-control-stream",
            "association": "conditional-parser-output", "column": column,
            "outputHex": to_hex(&output.bytes), "outputEncoding": "rich-string-control-bytes",
            "preservedControls": output.preserved_controls, "references": output.references}),
        Ok(()) => json!({"status": "resolved", "association": "conditional-sheet-reference",
            "column": column, "text": String::from_utf8(output.bytes).expect("validated literal UTF-8"),
            "textEncoding": "utf-8", "references": output.references}),
        Err(reason) => json!({"status": "requires-evaluation", "reason": reason,
            "column": column, "references": output.references}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich(bytes: &[u8]) -> RichString {
        RichString::parse(bytes, 0).unwrap()
    }

    // Authored literal source name, row 42 and explicit global column 2.
    const REFERENCE: &[u8] = b"\x02\x28\x10\xff\x0cxtx/command\x2b\x03\x03";

    #[test]
    fn resolves_literal_references_in_order_and_keeps_literal_text() {
        let input = rich(&[b"[A]\\".as_slice(), REFERENCE, b" ", REFERENCE].concat());
        let mut calls = 0;
        let result = resolve(&input, 5, |sheet, key, column| {
            assert_eq!((sheet, key, column), ("xtx/command", 42, 2));
            calls += 1;
            Ok(Ok(Cell {
                rich: rich(b"Authored"),
                evidence: json!({"rowKey": key}),
            }))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result["text"], "[A]\\Authored Authored");
        assert_eq!(result["references"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn refuses_nonliteral_extra_and_unparsed_arguments() {
        for payload in [
            b"\xff\x0cxtx/command\xe8\x01\x03".as_slice(),
            b"\xff\x0cxtx/command\x2b\x03\x01",
            b"\xff\x0cxtx/command\x2b\x00",
        ] {
            let input = rich(&[&[2, 0x28, payload.len() as u8 + 1], payload, &[3]].concat());
            let result = resolve(&input, 5, |_, _, _| panic!("must not query")).unwrap();
            assert_eq!(result["reason"], "unsupported-sheet-expression");
            assert!(result.get("text").is_none());
        }
    }

    #[test]
    fn nested_controls_and_missing_targets_never_return_partial_text() {
        let input = rich(&[b"Prefix".as_slice(), REFERENCE].concat());
        let result = resolve(&input, 5, |_, _, _| {
            Ok(Ok(Cell {
                rich: rich(b"Authored\x02\x12\x01\x03"),
                evidence: json!({"rowKey": 42}),
            }))
        })
        .unwrap();
        assert_eq!(result["reason"], "embedded-controls");
        assert!(result.get("text").is_none());
        let missing = resolve(&input, 5, |_, _, _| Ok(Err("sheet-row-not-found"))).unwrap();
        assert_eq!(missing["reason"], "sheet-row-not-found");
        assert!(missing.get("text").is_none());
        let io = resolve(&input, 5, |_, _, _| Err(Failure::parse("authored failure")));
        assert!(io.is_err());
    }

    #[test]
    fn bounds_cycles_reference_count_and_output_bytes() {
        let input = rich(REFERENCE);
        let cycle = resolve(&input, 5, |_, _, _| {
            Ok(Ok(Cell {
                rich: input.clone(),
                evidence: json!({}),
            }))
        })
        .unwrap();
        assert_eq!(cycle["reason"], "sheet-expansion-limit");
        assert_eq!(cycle["references"].as_array().unwrap().len(), MAX_DEPTH);
        let many = rich(&REFERENCE.repeat(MAX_REFERENCES + 1));
        let result = resolve(&many, 5, |_, _, _| {
            Ok(Ok(Cell {
                rich: rich(b""),
                evidence: json!({}),
            }))
        })
        .unwrap();
        assert_eq!(result["reason"], "sheet-expansion-limit");
        assert_eq!(
            result["references"].as_array().unwrap().len(),
            MAX_REFERENCES
        );
        let result = resolve(&input, 5, |_, _, _| {
            Ok(Ok(Cell {
                rich: rich(&vec![b'a'; MAX_OUTPUT_BYTES + 1]),
                evidence: json!({}),
            }))
        })
        .unwrap();
        assert_eq!(result["reason"], "sheet-expansion-limit");
        assert!(result.get("text").is_none());
    }

    #[test]
    fn preserves_supported_controls_byte_for_byte_without_plain_text() {
        let bytes = b"[A]\\\x02\x10\x01\x03B\x02\x16\x01\x03C\x02\x1f\x01\x03";
        let result = resolve(&rich(bytes), 1, |_, _, _| panic!("must not query")).unwrap();
        assert_eq!(result["status"], "resolved-control-stream");
        assert_eq!(result["association"], "conditional-parser-output");
        assert_eq!(result["outputEncoding"], "rich-string-control-bytes");
        assert_eq!(result["outputHex"], "5b415d5c02100103420216010343021f0103");
        assert_eq!(result["preservedControls"], 3);
        assert!(result.get("text").is_none());
    }

    #[test]
    fn preserves_nested_controls_in_expanded_sheet_output_order() {
        let input = rich(&[b"A".as_slice(), REFERENCE, b"Z"].concat());
        let result = resolve(&input, 5, |_, _, _| {
            Ok(Ok(Cell {
                rich: rich(b"B\x02\x1a\x02\x02\x03C\x02\x1d\x01\x03D\x02\x1a\x02\x01\x03"),
                evidence: json!({"rowKey": 42}),
            }))
        })
        .unwrap();
        assert_eq!(result["status"], "resolved-control-stream");
        assert_eq!(
            result["outputHex"],
            "4142021a02020343021d010344021a0201035a"
        );
        assert_eq!(result["preservedControls"], 3);
        assert_eq!(result["references"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn unsupported_or_oversized_streams_never_return_partial_bytes() {
        for bytes in [
            b"A\x02\x10\x01\x03\x02\x12\x01\x03".as_slice(),
            b"\x02\x10\x02\x01\x03",
            b"\x02\x1a\x01\x03",
            b"\x02\x1a\x02\x03\x03",
            b"\x02\x1a\xf0\x01\x02\x03",
            b"\x02\x1d\x02\x01\x03",
            b"\x02\x17\x01\x03",
        ] {
            let result = resolve(&rich(bytes), 1, |_, _, _| panic!("must not query")).unwrap();
            assert_eq!(result["reason"], "embedded-controls");
            assert!(result.get("outputHex").is_none());
            assert!(result.get("text").is_none());
        }
        let control = b"\x02\x10\x01\x03";
        let exact = rich(&control.repeat(MAX_OUTPUT_BYTES / 4));
        let result = resolve(&exact, 1, |_, _, _| panic!("must not query")).unwrap();
        assert_eq!(result["status"], "resolved-control-stream");
        assert_eq!(
            result["outputHex"].as_str().unwrap().len(),
            MAX_OUTPUT_BYTES * 2
        );
        let overflow = rich(&[exact.encode(), b"A".to_vec()].concat());
        let result = resolve(&overflow, 1, |_, _, _| panic!("must not query")).unwrap();
        assert_eq!(result["reason"], "sheet-expansion-limit");
        assert!(result.get("outputHex").is_none());
    }

    #[test]
    fn preserves_italic_and_nonbreaking_space_forms_exactly() {
        let input = rich(b"A\x02\x1a\x02\x02\x03B\x02\x1d\x01\x03C\x02\x1a\x02\x01\x03D");
        let result = resolve(&input, 5, |_, _, _| panic!("must not query")).unwrap();
        assert_eq!(result["status"], "resolved-control-stream");
        assert_eq!(result["outputHex"], "41021a02020342021d010343021a02010344");
        assert_eq!(result["preservedControls"], 3);
        assert!(result["references"].as_array().unwrap().is_empty());
        assert!(result.get("text").is_none());
    }
}
