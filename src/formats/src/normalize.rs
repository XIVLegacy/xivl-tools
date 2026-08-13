//! The one canonical JSON form every comparison happens on.
//!
//! Rules from `docs/conformance-tests.md`: UTF-8 restricted to ASCII,
//! sorted object keys, two-space indent, LF line endings, trailing newline.
//! Key sorting comes from `serde_json`'s default map, which is ordered.

/// Render a value in the normalized form, trailing newline included.
pub fn to_canonical_json(value: &serde_json::Value) -> String {
    let mut text =
        serde_json::to_string_pretty(value).expect("a Value built by this crate always serializes");
    if !text.is_ascii() {
        // Outside string literals a JSON document is ASCII by construction,
        // so escaping every non-ASCII character is always in-string.
        text = text
            .chars()
            .map(|character| {
                if character.is_ascii() {
                    character.to_string()
                } else {
                    character
                        .encode_utf16(&mut [0u16; 2])
                        .iter()
                        .map(|unit| format!("\\u{unit:04x}"))
                        .collect()
                }
            })
            .collect();
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_keys_and_indents_by_two() {
        let value = serde_json::json!({ "b": 1, "a": { "d": 2, "c": 3 } });
        assert_eq!(
            to_canonical_json(&value),
            "{\n  \"a\": {\n    \"c\": 3,\n    \"d\": 2\n  },\n  \"b\": 1\n}\n"
        );
    }

    #[test]
    fn escapes_non_ascii_and_ends_with_one_newline() {
        let value = serde_json::json!({ "name": "a\u{00e9}b" });
        let text = to_canonical_json(&value);
        assert!(text.is_ascii());
        assert!(text.contains("\\u00e9"));
        assert!(text.ends_with("}\n"));
        assert!(!text.contains("\r"));
    }
}
