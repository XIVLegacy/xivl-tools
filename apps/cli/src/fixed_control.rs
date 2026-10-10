use serde_json::{json, Value};
use xivl_formats::reader::{to_hex, Span};
use xivl_formats::richstring::fixed;
use xivl_formats::to_canonical_json;

use crate::{fixed_phrase, initial_registry, read_capped, Failure, MAX_INPUT_BYTES};

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let (document, failed) = report(arguments)?;
    use std::io::Write;
    std::io::stdout()
        .write_all(to_canonical_json(&document).as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))?;
    if failed {
        return Err(Failure::parse(
            "fixed-control diagnostic failed; see JSON output",
        ));
    }
    Ok(())
}

fn report(arguments: &[String]) -> Result<(Value, bool), Failure> {
    let usage = "usage: xivl decode-fixed (<file> | --hex <bytes>) [--offset <decimal>] [--fixed-phrase-csv <file> | --initial-registry <client-root>] [--locale ja|en|de|fr] [--noun-inputs <config>:<count>:<attributive-key>:<retained-word>]";
    let mut index = 0;
    let data = match arguments.first().map(String::as_str) {
        Some("--hex") => {
            let hex = arguments.get(1).ok_or_else(|| Failure::usage(usage))?;
            index += 2;
            parse_hex(hex)?
        }
        Some(path) if !path.starts_with('-') => {
            index += 1;
            read_capped(path)?
        }
        _ => return Err(Failure::usage(usage)),
    };
    let mut offset = None;
    let mut table = None;
    let mut locale = None;
    let mut registry = None;
    let mut noun_source = None;
    while index < arguments.len() {
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| Failure::usage(usage))?;
        match arguments[index].as_str() {
            "--offset" if offset.is_none() => {
                offset = Some(value.parse::<usize>().map_err(|_| {
                    Failure::usage("offset must be an unsigned decimal byte offset")
                })?);
            }
            "--fixed-phrase-csv" if table.is_none() => table = Some(value),
            "--initial-registry" if registry.is_none() => registry = Some(value),
            "--locale" if locale.is_none() => {
                locale = Some(fixed_phrase::Locale::parse(value).ok_or_else(|| {
                    Failure::usage("fixedPhrase locale must be ja, en, de or fr")
                })?);
            }
            "--noun-inputs" if noun_source.is_none() => {
                noun_source = Some(parse_noun_inputs(value)?);
            }
            _ => return Err(Failure::usage(usage)),
        }
        index += 2;
    }
    if table.is_some() && registry.is_some() {
        return Err(Failure::usage(
            "choose --fixed-phrase-csv or --initial-registry",
        ));
    }
    if (table.is_some() || registry.is_some()) != locale.is_some() {
        return Err(Failure::usage(
            "a lookup source and --locale must be supplied together",
        ));
    }
    if noun_source.is_some()
        && (registry.is_none() || !matches!(locale, Some(fixed_phrase::Locale::En)))
    {
        return Err(Failure::usage(
            "--noun-inputs requires --initial-registry and --locale en",
        ));
    }
    let offset = offset.unwrap_or(0);
    match fixed::decode(&data, offset) {
        Ok(control) => {
            let mut document = json!({
                "status": if control.numeric_fields.is_some() { "decoded" } else { "unsupported" },
                "spanKind": if control.numeric_fields.is_some() { "control" } else { "opaque-remainder" },
                "span": control.span.to_json(),
                "selector": control.selector,
                "rawHex": to_hex(control.raw),
                "numericFields": control.numeric_fields,
            });
            let mut failed = false;
            if let (Some(path), Some(locale)) = (table, locale) {
                if let Some(numbers) = control.numeric_fields {
                    let data = read_capped(path)?;
                    let (lookup, lookup_failed) = fixed_phrase::report(&data, numbers, locale);
                    document["phraseLookup"] = lookup;
                    failed = lookup_failed;
                } else {
                    document["phraseLookup"] = json!({"status": "unsupported-control"});
                }
            }
            if let (Some(root), Some(locale)) = (registry, locale) {
                if let Some(numbers) = control.numeric_fields {
                    let (lookup, lookup_failed) =
                        initial_registry::report(root, numbers, locale, noun_source)?;
                    document["registryLookup"] = lookup;
                    failed = lookup_failed;
                } else {
                    document["registryLookup"] = json!({"status": "unsupported-control"});
                }
            }
            Ok((document, failed))
        }
        Err(error) => {
            let available = data.get(offset..).unwrap_or_default();
            Ok((
                json!({
                    "status": "error",
                    "errorKind": error.kind().as_str(),
                    "errorOffset": error.offset(),
                    "detail": error.detail(),
                    "availableSpan": Span::new(offset as u64, available.len() as u64).to_json(),
                    "rawHex": to_hex(available),
                    "selector": available.get(1).copied(),
                }),
                true,
            ))
        }
    }
}

fn parse_noun_inputs(value: &str) -> Result<[u32; 4], Failure> {
    let invalid =
        || Failure::usage("noun inputs must be four unsigned decimal words separated by colons");
    let mut parts = value.split(':');
    let mut result = [0; 4];
    for slot in &mut result {
        let word = parts.next().ok_or_else(invalid)?;
        if word.is_empty() || !word.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid());
        }
        *slot = word.parse::<u32>().map_err(|_| invalid())?;
    }
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok(result)
}

fn parse_hex(text: &str) -> Result<Vec<u8>, Failure> {
    let compact: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if compact.len() as u64 > MAX_INPUT_BYTES * 2 {
        return Err(Failure::usage("hex input exceeds the input byte limit"));
    }
    let (pairs, remainder) = compact.as_chunks::<2>();
    if !remainder.is_empty() || !compact.iter().all(u8::is_ascii_hexdigit) {
        return Err(Failure::usage(
            "hex input must contain complete hexadecimal byte pairs",
        ));
    }
    Ok(pairs
        .iter()
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                _ => byte.to_ascii_lowercase() - b'a' + 10,
            };
            (digit(pair[0]) << 4) | digit(pair[1])
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn supplied_hex_and_offset_report_exact_bytes_and_values() {
        let (document, failed) = report(&args(&[
            "--hex",
            "41 02 2E 04 02 F1 01 03 FF",
            "--offset",
            "1",
        ]))
        .unwrap();
        assert!(!failed);
        assert_eq!(document["span"], json!({"offset": 1, "length": 7}));
        assert_eq!(document["selector"], 46);
        assert_eq!(document["rawHex"], "022e0402f10103");
        assert_eq!(document["numericFields"], json!([1, 256]));
    }

    #[test]
    fn malformed_input_is_json_with_parse_failure_status() {
        let (document, failed) = report(&args(&["--hex", "02 2e 04 01 f1 03 01 03"])).unwrap();
        assert!(failed);
        assert_eq!(document["errorOffset"], 6);
        assert_eq!(document["errorKind"], "malformed-rich-string-token");
        assert_eq!(document["rawHex"], "022e0401f1030103");
    }

    #[test]
    fn noun_inputs_require_explicit_english_registry_context() {
        assert_eq!(parse_noun_inputs("0:1:5:0").unwrap(), [0, 1, 5, 0]);
        for value in [
            "0:1:5",
            "0:1:5:0:0",
            "0:+1:5:0",
            "0:1:5:",
            "0:1:4294967296:0",
        ] {
            assert!(parse_noun_inputs(value).is_err());
        }
        for values in [
            vec!["--hex", "02 37 ff", "--noun-inputs", "0:1:5:0"],
            vec![
                "--hex",
                "02 37 ff",
                "--initial-registry",
                "unused",
                "--locale",
                "de",
                "--noun-inputs",
                "0:1:5:0",
            ],
        ] {
            assert_eq!(report(&args(&values)).unwrap_err().code, 1);
        }
        let (result, failed) = report(&args(&[
            "--hex",
            "02 37 ff",
            "--initial-registry",
            "not-loaded",
            "--locale",
            "en",
            "--noun-inputs",
            "0:1:5:0",
        ]))
        .unwrap();
        assert!(!failed);
        assert_eq!(result["registryLookup"]["status"], "unsupported-control");
    }

    #[test]
    fn unknown_controls_are_opaque_and_bad_arguments_are_usage_errors() {
        let (document, failed) = report(&args(&["--hex", "02 37 ff"])).unwrap();
        assert!(!failed);
        assert_eq!(document["status"], "unsupported");
        assert_eq!(document["spanKind"], "opaque-remainder");
        assert_eq!(document["numericFields"], Value::Null);
        for values in [
            &["--hex", "0"][..],
            &["--hex", "gg"],
            &["--hex", "00", "--offset", "-1"],
            &["--hex", "00", "--file", "x"],
            &["--hex", "00", "--locale", "en"],
            &["--hex", "00", "--fixed-phrase-csv", "x"],
            &["--hex", "00", "--fixed-phrase-csv", "x", "--locale", "chs"],
            &["--hex", "00", "--initial-registry", "x"],
            &[
                "--hex",
                "00",
                "--initial-registry",
                "x",
                "--fixed-phrase-csv",
                "y",
                "--locale",
                "en",
            ],
            &[
                "--hex",
                "00",
                "--initial-registry",
                "x",
                "--initial-registry",
                "y",
                "--locale",
                "en",
            ],
            &["--hex", "00", "--offset", "0", "--offset", "1"],
        ] {
            assert_eq!(report(&args(values)).unwrap_err().code, 1);
        }
        let (result, failed) = report(&args(&[
            "--hex",
            "02 37 FF",
            "--initial-registry",
            "not-loaded",
            "--locale",
            "en",
        ]))
        .unwrap();
        assert!(!failed);
        assert_eq!(result["registryLookup"]["status"], "unsupported-control");
    }
}
