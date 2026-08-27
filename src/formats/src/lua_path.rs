//! The reversible character transform used by compiled Lua resource paths.
//!
//! The client path corpus is ASCII. Rejecting bytes outside that domain keeps
//! case folding deterministic across platforms and locales.

use crate::error::{ErrorKind, FormatError, Result};

/// Transform one client Lua path.
///
/// Encoding and decoding are the same operation. ASCII letters are folded to
/// lowercase before substitution, digits are paired with `j` through `a`, and
/// ASCII punctuation (including both path separators) passes through.
pub fn transform(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    for (offset, byte) in input.bytes().enumerate() {
        if !byte.is_ascii() {
            return Err(FormatError::new(
                ErrorKind::InvalidLuaPath,
                offset as u64,
                "Lua resource paths are ASCII",
            ));
        }
        let lower = byte.to_ascii_lowercase();
        let transformed = match lower {
            b'a'..=b'j' => b'9' - (lower - b'a'),
            b'k'..=b'z' => b'z' - (lower - b'k'),
            b'0'..=b'9' => b'j' - (lower - b'0'),
            other => other,
        };
        output.push(char::from(transformed));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_paths_match_the_evidence() {
        assert_eq!(transform("ZoneMoveProgTest").unwrap(), "kvw5xvo5usv3q5rq");
        assert_eq!(
            transform("Quest/Scenario/Man").unwrap(),
            "tp5rq/r75w9s1v/x9w"
        );
        assert_eq!(transform("Man0g0").unwrap(), "x9wj3j");
    }

    #[test]
    fn transform_is_an_involution_over_the_ascii_domain() {
        let input: String = (0u8..=127).map(char::from).collect();
        let once = transform(&input).unwrap();
        let twice = transform(&once).unwrap();
        assert_eq!(twice, input.to_ascii_lowercase());
    }

    #[test]
    fn rejects_non_ascii_at_its_byte_offset() {
        let error = transform("ab\u{e9}").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidLuaPath);
        assert_eq!(error.offset(), 2);
    }
}
