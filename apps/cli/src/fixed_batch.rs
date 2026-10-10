//! JSONL message conversion. See docs/cli.md#convert-a-batch-of-messages.

use std::io::{Read, Write};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::auto_translate::{Locale, ResolveError as Failure, Resolver, MAX_INPUT_BYTES};
use crate::fixed_control::{message_report, parse_hex};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    hex: String,
    id: Option<String>,
}

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let (path, mut resolver) = options(arguments)?;
    let input = if path == "-" {
        read_input(std::io::stdin().lock(), MAX_INPUT_BYTES)?
    } else {
        let file = std::fs::File::open(path)
            .map_err(|error| Failure::io(format!("cannot open '{path}': {error}")))?;
        read_input(file, MAX_INPUT_BYTES)?
    };
    let failed = process(&input, &mut std::io::stdout().lock(), resolver.as_mut())?;
    if failed {
        return Err(Failure::parse(
            "batch contains input errors or partial messages; see JSONL output",
        ));
    }
    Ok(())
}

fn options(arguments: &[String]) -> Result<(&str, Option<Resolver>), Failure> {
    let usage = "usage: xivl decode-fixed-batch <messages.jsonl | -> [--initial-registry <client-root> --locale ja|en|de|fr]";
    let path = arguments
        .first()
        .filter(|path| path.as_str() == "-" || !path.starts_with('-'))
        .ok_or_else(|| Failure::usage(usage))?;
    let mut root = None;
    let mut locale = None;
    let mut index = 1;
    while index < arguments.len() {
        let value = arguments
            .get(index + 1)
            .ok_or_else(|| Failure::usage(usage))?;
        match arguments[index].as_str() {
            "--initial-registry" if root.is_none() => root = Some(value),
            "--locale" if locale.is_none() => {
                locale = Some(Locale::parse(value).ok_or_else(|| {
                    Failure::usage("fixedPhrase locale must be ja, en, de or fr")
                })?);
            }
            _ => return Err(Failure::usage(usage)),
        }
        index += 2;
    }
    let resolver = match (root, locale) {
        (Some(root), Some(locale)) => Some(Resolver::new(root, locale)?),
        (None, None) => None,
        _ => {
            return Err(Failure::usage(
                "--initial-registry and --locale must be supplied together",
            ));
        }
    };
    Ok((path, resolver))
}

fn read_input(reader: impl Read, limit: u64) -> Result<Vec<u8>, Failure> {
    let mut input = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut input)
        .map_err(|error| Failure::io(format!("cannot read batch input: {error}")))?;
    if input.len() as u64 > limit {
        return Err(Failure::io(format!(
            "batch input is larger than the {limit}-byte limit"
        )));
    }
    Ok(input)
}

fn parse_record(line: &[u8]) -> Result<Record, Value> {
    let start = line.iter().position(|byte| !byte.is_ascii_whitespace());
    if start.and_then(|index| line.get(index)) != Some(&b'{') {
        return Err(
            json!({"kind": "input", "message": "record must be a JSON object",
            "column": start.map_or(0, |index| index + 1)}),
        );
    }
    serde_json::from_slice(line).map_err(
        |error| json!({"kind": "input", "message": error.to_string(), "column": error.column()}),
    )
}

fn process(
    input: &[u8],
    output: &mut impl Write,
    mut resolver: Option<&mut Resolver>,
) -> Result<bool, Failure> {
    let mut failed = false;
    for (index, line) in input.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let number = index + 1;
        let mut document = json!({"line": number});
        match parse_record(line) {
            Err(error) => {
                document["error"] = error;
                failed = true;
            }
            Ok(record) => {
                if let Some(id) = record.id {
                    document["id"] = json!(id);
                }
                match parse_hex(&record.hex) {
                    Err(error) => {
                        document["error"] = json!({"kind": "input", "message": error.message()});
                        failed = true;
                    }
                    Ok(bytes) => {
                        let result = if let Some(context) = resolver.as_deref_mut() {
                            context.resolve_message(&bytes)
                        } else {
                            message_report(&bytes, |_| Ok(None))
                        };
                        let (result, partial) = result.map_err(|error| {
                            let message = format!("batch line {number}: {}", error.message());
                            match error.kind() {
                                crate::auto_translate::ResolveErrorKind::Usage => {
                                    Failure::usage(message)
                                }
                                crate::auto_translate::ResolveErrorKind::Io => Failure::io(message),
                                crate::auto_translate::ResolveErrorKind::Parse => {
                                    Failure::parse(message)
                                }
                            }
                        })?;
                        failed |= partial;
                        document["result"] = result;
                    }
                }
            }
        }
        writeln!(output, "{document}")
            .and_then(|()| output.flush())
            .map_err(|error| Failure::io(format!("cannot write batch line {number}: {error}")))?;
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_accepts_exact_limit_and_refuses_the_next_byte() {
        assert_eq!(read_input(&b"Authored"[..], 8).unwrap(), b"Authored");
        let mut reader = &b"Authored extra"[..];
        assert_eq!(read_input(&mut reader, 8).unwrap_err().code(), 1);
        assert_eq!(reader, b"extra");
    }

    #[test]
    fn batch_stops_on_output_failure() {
        let input = b"{\"hex\":\"41\"}\n";
        let error = process(input, &mut std::io::Cursor::new([0u8; 1]), None).unwrap_err();
        assert_eq!(error.kind(), crate::auto_translate::ResolveErrorKind::Io);
        assert!(error.message().contains("batch line 1"));
    }
}
