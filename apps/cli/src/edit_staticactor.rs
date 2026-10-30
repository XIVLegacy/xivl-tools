//! Add records to a new static-actor SAN table.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::staticactor;
use xivl_formats::staticactor_write::{
    decoded_entries, decoded_unknown_header, encode_staticactor, StaticActorEntry,
};
use xivl_formats::{to_canonical_json, FormatError};

use crate::{read_capped, Failure};

const USAGE: &str =
    "usage: xivl edit-staticactor <input.san> --output <new-file> --add <value>=<path> [--add ...]";

struct Options<'a> {
    input: &'a str,
    output: &'a str,
    adds: Vec<StaticActorEntry>,
}

impl<'a> Options<'a> {
    fn parse(arguments: &'a [String]) -> Result<Self, Failure> {
        let Some((input, rest)) = arguments.split_first() else {
            return Err(Failure::usage(USAGE));
        };
        let mut output = None;
        let mut adds = Vec::new();
        let mut values = BTreeSet::new();
        let (pairs, remainder) = rest.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(Failure::usage(USAGE));
        }
        for [flag, argument] in pairs {
            match flag.as_str() {
                "--output" => {
                    if output.replace(argument.as_str()).is_some() {
                        return Err(Failure::usage("--output was supplied more than once"));
                    }
                }
                "--add" => {
                    let (value, path) = argument
                        .split_once('=')
                        .ok_or_else(|| Failure::usage("--add requires <value>=<path>"))?;
                    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(Failure::usage("--add value must be a decimal u32"));
                    }
                    let value = value
                        .parse::<u32>()
                        .map_err(|_| Failure::usage("--add value must be a decimal u32"))?;
                    if !values.insert(value) {
                        return Err(Failure::usage(format!("--add value {value} is repeated")));
                    }
                    if !path.is_ascii()
                        || !path.starts_with('/')
                        || path.bytes().any(|byte| byte.is_ascii_control())
                        || path.len() > 255
                    {
                        return Err(Failure::usage("--add path must be ASCII, start with '/', contain no NUL or control bytes, and fit the tool limit of 255 bytes"));
                    }
                    adds.push(StaticActorEntry {
                        value,
                        string: path.as_bytes().to_vec(),
                    });
                }
                _ => {
                    return Err(Failure::usage(format!(
                        "unknown edit-staticactor option '{flag}'"
                    )))
                }
            }
        }
        if adds.is_empty() {
            return Err(Failure::usage("at least one --add is required"));
        }
        Ok(Self {
            input,
            output: output.ok_or_else(|| Failure::usage("--output is required"))?,
            adds,
        })
    }
}

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let report = edit(Options::parse(arguments)?)?;
    std::io::stdout()
        .write_all(to_canonical_json(&report).as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write stdout: {error}")))
}

fn edit(options: Options<'_>) -> Result<Value, Failure> {
    let input = read_capped(options.input)?;
    let san = staticactor::parse(&input).map_err(format_failure)?;
    let header = decoded_unknown_header(&input, &san).map_err(format_failure)?;
    let original = decoded_entries(&input, &san).map_err(format_failure)?;
    if original
        .windows(2)
        .any(|pair| pair[0].value > pair[1].value)
    {
        return Err(Failure::usage(
            "input values are not ascending; cannot choose insertion positions",
        ));
    }
    for add in &options.adds {
        if original
            .binary_search_by_key(&add.value, |entry| entry.value)
            .is_ok()
        {
            return Err(Failure::usage(format!(
                "--add value {} already exists in input",
                add.value
            )));
        }
    }
    let mut entries = original.clone();
    entries.extend_from_slice(&options.adds);
    entries.sort_by_key(|entry| entry.value);
    let bytes = encode_staticactor(header, &entries).map_err(format_failure)?;
    let paths: BTreeSet<&[u8]> = original
        .iter()
        .map(|entry| entry.string.as_slice())
        .collect();
    let additions: Vec<Value> = options
        .adds
        .iter()
        .map(|add| {
            json!({
                "value": add.value,
                "insertionIndex": entries.partition_point(|entry| entry.value < add.value),
                "pathLength": add.string.len(),
                "pathExisting": paths.contains(add.string.as_slice()),
            })
        })
        .collect();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(options.output)
        .map_err(|error| Failure::usage(format!("cannot create output: {error}")))?;
    let written = file.write_all(&bytes).and_then(|()| file.flush());
    drop(file);
    let verified = written
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
        .and_then(|()| {
            let output = read_capped(options.output)?;
            let parsed = staticactor::parse(&output).map_err(format_failure)?;
            let output_header = decoded_unknown_header(&output, &parsed).map_err(format_failure)?;
            let output_entries = decoded_entries(&output, &parsed).map_err(format_failure)?;
            if parsed.declared_count as usize != original.len() + options.adds.len()
                || output_header != header
                || output_entries != entries
                || output != bytes
                || encode_staticactor(output_header, &output_entries).map_err(format_failure)?
                    != output
            {
                return Err(Failure::parse(
                    "written SAN failed record preservation or exact rebuild verification",
                ));
            }
            Ok(json!({
                "operation": "edit-staticactor",
                "inputRecordCount": original.len(),
                "inputSha256": sha256_hex(&input),
                "outputRecordCount": output_entries.len(),
                "outputLength": output.len(),
                "outputSha256": sha256_hex(&output),
                "adds": additions,
            }))
        });
    if let Err(failure) = verified {
        fs::remove_file(options.output).map_err(|error| Failure {
            message: format!("{}; cannot remove failed output: {error}", failure.message),
            code: failure.code,
        })?;
        return Err(failure);
    }
    verified
}

fn format_failure(error: FormatError) -> Failure {
    Failure::parse(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_nul_in_add_path_before_filesystem_access() {
        let args = ["source.san", "--output", "new.san", "--add", "1=/a\0b"].map(str::to_string);
        let error = Options::parse(&args).err().unwrap();
        assert!(error.message.contains("no NUL or control bytes"));
    }
}
