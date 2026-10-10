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
    let usage = "usage: xivl decode-fixed (<file> | --hex <bytes>) [--message | --offset <decimal>] [--fixed-phrase-csv <file> | --initial-registry <client-root>] [--locale ja|en|de|fr] [--noun-inputs <config>:<count>:<attributive-key>:<retained-word>]";
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
    let mut message = false;
    while index < arguments.len() {
        if arguments[index] == "--message" && !message {
            message = true;
            index += 1;
            continue;
        }
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
    if message {
        if offset.is_some() || table.is_some() || noun_source.is_some() {
            return Err(Failure::usage(
                "--message cannot be combined with --offset, --fixed-phrase-csv or --noun-inputs",
            ));
        }
        return message_report(&data, |numbers| {
            if let (Some(root), Some(locale)) = (registry, locale) {
                initial_registry::report(root, numbers, locale, None)
                    .map(|(lookup, _)| Some(lookup))
            } else {
                Ok(None)
            }
        });
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

const MAX_MESSAGE_CONTROLS: usize = 1024;

fn message_report(
    data: &[u8],
    mut lookup: impl FnMut([u32; 2]) -> Result<Option<Value>, Failure>,
) -> Result<(Value, bool), Failure> {
    let mut segments = Vec::new();
    let mut output = Vec::new();
    let mut complete = true;
    let mut controls = 0;
    for segment in fixed::message_segments(data) {
        let control_offset = match &segment {
            fixed::MessageSegment::Literal { .. } => None,
            fixed::MessageSegment::Control(control) => Some(control.span.offset),
            fixed::MessageSegment::Error { span, .. } => Some(span.offset),
        };
        if controls == MAX_MESSAGE_CONTROLS {
            if let Some(offset) = control_offset {
                let raw = &data[offset as usize..];
                output.extend_from_slice(raw);
                segments.push(
                    json!({"kind": "opaque-remainder", "status": "control-limit",
                    "span": Span::new(offset, raw.len() as u64).to_json(), "rawHex": to_hex(raw)}),
                );
                complete = false;
                break;
            }
        }
        match segment {
            fixed::MessageSegment::Literal { span, raw } => {
                output.extend_from_slice(raw);
                let mut item =
                    json!({"kind": "literal", "span": span.to_json(), "rawHex": to_hex(raw)});
                if let Ok(text) = std::str::from_utf8(raw) {
                    item["text"] = json!(text);
                }
                segments.push(item);
            }
            fixed::MessageSegment::Control(control) => {
                controls += 1;
                let mut item = json!({"kind": if control.numeric_fields.is_some() { "fixed-control" } else { "opaque-remainder" },
                    "status": "preserved", "span": control.span.to_json(), "rawHex": to_hex(control.raw),
                    "selector": control.selector, "numericFields": control.numeric_fields});
                let mut replacement = None;
                if let Some(numbers) = control.numeric_fields {
                    if let Some(result) = lookup(numbers)? {
                        let resolution = &result["textResolution"];
                        match resolution["status"].as_str() {
                            Some("resolved") => {
                                if let Some(text) = resolution["text"].as_str() {
                                    replacement = Some(text.as_bytes().to_vec());
                                }
                            }
                            Some("resolved-control-stream") => {
                                if let Some(hex) = resolution["outputHex"].as_str() {
                                    replacement = Some(parse_hex(hex)?);
                                }
                            }
                            _ => {}
                        }
                        item["registryLookup"] = result;
                    }
                }
                if let Some(bytes) = replacement {
                    item["status"] = json!("replaced");
                    item["outputHex"] = json!(to_hex(&bytes));
                    output.extend_from_slice(&bytes);
                } else {
                    complete = false;
                    output.extend_from_slice(control.raw);
                }
                segments.push(item);
            }
            fixed::MessageSegment::Error { span, raw, error } => {
                complete = false;
                output.extend_from_slice(raw);
                segments.push(json!({"kind": "opaque-remainder", "status": "error", "span": span.to_json(),
                    "rawHex": to_hex(raw), "errorKind": error.kind().as_str(), "errorOffset": error.offset(),
                    "detail": error.detail()}));
            }
        }
    }
    let mut document = json!({"status": if complete { "complete" } else { "partial" },
        "inputSpan": Span::new(0, data.len() as u64).to_json(), "segments": segments,
        "outputHex": to_hex(&output), "outputEncoding": "hex", "controlCount": controls});
    if complete && !output.contains(&0x02) {
        if let Ok(text) = std::str::from_utf8(&output) {
            document["text"] = json!(text);
        }
    }
    Ok((document, !complete))
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
    fn message_replacements_keep_literal_bytes_and_absolute_spans() {
        let data = b"Meet \x02\x2e\x03\x02\x03\x03 at \x02\x2e\x03\x04\x05\x03!";
        let mut calls = Vec::new();
        let (document, failed) = message_report(data, |numbers| {
            calls.push(numbers);
            Ok(Some(
                json!({"textResolution": {"status": "resolved", "text":
                if numbers == [1, 2] { "Authored friend" } else { "Authored place" }}}),
            ))
        })
        .unwrap();
        assert!(!failed);
        assert_eq!(calls, [[1, 2], [3, 4]]);
        assert_eq!(document["text"], "Meet Authored friend at Authored place!");
        assert_eq!(
            document["outputHex"],
            to_hex(b"Meet Authored friend at Authored place!")
        );
        assert_eq!(
            document["segments"][3]["span"],
            json!({"offset": 15, "length": 6})
        );
        assert_eq!(document["controlCount"], 2);
    }

    #[test]
    fn message_stream_and_refusal_preserve_bytes_and_continue() {
        let data = b"A\x02\x2e\x03\x02\x03\x03B\x02\x2e\x03\x04\x05\x03C\x02\x2e\x03\x06\x07\x03";
        let (document, failed) = message_report(data, |numbers| {
            Ok(Some(match numbers {
                [1, 2] => json!({"textResolution": {"status": "resolved-control-stream", "outputHex": "5802160103"}}),
                [3, 4] => json!({"sourceLookup": {"status": "not-found"}}),
                _ => json!({"textResolution": {"status": "resolved", "text": "Z"}}),
            }))
        }).unwrap();
        assert!(failed);
        assert_eq!(document["status"], "partial");
        assert_eq!(
            document["outputHex"],
            to_hex(b"AX\x02\x16\x01\x03B\x02\x2e\x03\x04\x05\x03CZ")
        );
        assert!(document.get("text").is_none());
        assert_eq!(document["segments"][3]["status"], "preserved");
        assert_eq!(document["segments"][5]["status"], "replaced");
        let (stream, failed) = message_report(&data[..7], |_| {
            Ok(Some(json!({
                "textResolution": {"status": "resolved-control-stream", "outputHex": "5802160103"}
            })))
        })
        .unwrap();
        assert!(!failed);
        assert_eq!(stream["status"], "complete");
        assert!(stream.get("text").is_none());
    }

    #[test]
    fn message_opaque_remainders_never_resynchronize_or_load_sources() {
        for data in [
            b"A\x02\x37\xff\x02\x2e\x03\x02\x03\x03".as_slice(),
            b"A\x02\x2e\x03\x00\x03\x03\x02\x2e\x03\x02\x03\x03".as_slice(),
            b"A\x02".as_slice(),
        ] {
            let (document, failed) = message_report(data, |_| {
                panic!("opaque input must not load lookup sources")
            })
            .unwrap();
            assert!(failed);
            assert_eq!(document["outputHex"], to_hex(data));
            assert_eq!(document["segments"].as_array().unwrap().len(), 2);
            assert_eq!(document["segments"][1]["kind"], "opaque-remainder");
            assert!(document.get("text").is_none());
        }
    }

    #[test]
    fn message_literals_and_control_budget_are_lossless() {
        for data in [
            b"\xff\x00".as_slice(),
            b"".as_slice(),
            b"Authored".as_slice(),
        ] {
            let (document, failed) = message_report(data, |_| panic!("literal input")).unwrap();
            assert!(!failed);
            assert_eq!(document["outputHex"], to_hex(data));
            assert_eq!(
                document.get("text").is_some(),
                std::str::from_utf8(data).is_ok()
            );
        }
        let data = b"\x02\x2e\x03\x02\x03\x03".repeat(MAX_MESSAGE_CONTROLS + 1);
        let mut calls = 0;
        let (document, failed) = message_report(&data, |_| {
            calls += 1;
            Ok(Some(
                json!({"textResolution": {"status": "resolved", "text": "X"}}),
            ))
        })
        .unwrap();
        assert!(failed);
        assert_eq!(calls, MAX_MESSAGE_CONTROLS);
        assert_eq!(
            document["segments"][MAX_MESSAGE_CONTROLS]["status"],
            "control-limit"
        );
        let mut expected = vec![b'X'; MAX_MESSAGE_CONTROLS];
        expected.extend_from_slice(b"\x02\x2e\x03\x02\x03\x03");
        assert_eq!(document["outputHex"], to_hex(&expected));
        let mut data = b"\x02\x2e\x03\x02\x03\x03".repeat(MAX_MESSAGE_CONTROLS);
        data.extend_from_slice(b"Tail\x02\x2e\x03\x00\x03\x03");
        let (document, failed) = message_report(&data, |_| Ok(None)).unwrap();
        assert!(failed);
        assert_eq!(document["outputHex"], to_hex(&data));
        assert_eq!(
            document["segments"][MAX_MESSAGE_CONTROLS + 1]["status"],
            "control-limit"
        );
    }

    #[test]
    fn message_mode_rejects_single_control_context_options() {
        for extra in [
            vec!["--offset", "0"],
            vec!["--fixed-phrase-csv", "unused.csv", "--locale", "en"],
            vec![
                "--initial-registry",
                "unused",
                "--locale",
                "en",
                "--noun-inputs",
                "0:1:5:0",
            ],
        ] {
            let mut values = vec!["--hex", "41", "--message"];
            values.extend(extra);
            assert!(report(&args(&values)).is_err());
        }
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
