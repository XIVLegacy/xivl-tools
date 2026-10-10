//! Literal, three-argument sheet references in conditional default fields.

use serde_json::{json, Value};
use xivl_formats::richstring::{Expression, RichString, Segment, Token};

use crate::Failure;

// Diagnostic budgets bound recursion and expansion; these are not native limits.
const MAX_DEPTH: usize = 16;
const MAX_REFERENCES: usize = 128;
const MAX_TEXT_BYTES: usize = 1024 * 1024;

pub(super) struct Cell {
    pub rich: RichString,
    pub evidence: Value,
}

pub(super) type QueryResult = std::result::Result<Cell, &'static str>;

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
    text: String,
    references: Vec<Value>,
}

fn append(output: &mut Output, text: &str) -> std::result::Result<(), &'static str> {
    if text.len() > MAX_TEXT_BYTES - output.text.len() {
        return Err("sheet-expansion-limit");
    }
    output.text.push_str(text);
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
                if let Err(reason) = append(output, text) {
                    return Ok(Err(reason));
                }
            }
            Segment::Token(token) => {
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
        text: String::new(),
        references: Vec::new(),
    };
    let result = evaluate(rich, 0, &mut output, &mut query)?;
    Ok(match result {
        Ok(()) => json!({"status": "resolved", "association": "conditional-sheet-reference",
            "column": column, "text": output.text, "textEncoding": "utf-8", "references": output.references}),
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
                rich: rich(b"Authored\x02\x1f\x01\x03"),
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
                rich: rich(&vec![b'a'; MAX_TEXT_BYTES + 1]),
                evidence: json!({}),
            }))
        })
        .unwrap();
        assert_eq!(result["reason"], "sheet-expansion-limit");
        assert!(result.get("text").is_none());
    }
}
