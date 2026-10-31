//! Append one root and child to a new RegionResourceData catalog.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::region::{self, AuxiliaryKey, RegionMembership, RegionRow};
use xivl_formats::region_write::{append_region_root, RegionAppend, RegionAppendRequest};
use xivl_formats::{to_canonical_json, FormatError, ResourceId};

use crate::{read_capped, Failure};

const USAGE: &str = "usage: xivl append-region <catalog.dat> --output <new-file> --like <root-key> [--like-child <index>] --root-key <key> --root-name <name> --child-key <key> --child-name <name> --child-resource <key> [--child-type <u32>] [--environment-from <root-key>]";

struct Options<'a> {
    input: &'a str,
    output: &'a str,
    template_root: u32,
    template_child: Option<usize>,
    environment_root: Option<u32>,
    append: RegionAppend,
}

impl<'a> Options<'a> {
    fn parse(arguments: &'a [String]) -> Result<Self, Failure> {
        let Some((input, rest)) = arguments.split_first() else {
            return Err(Failure::usage(USAGE));
        };
        let (pairs, remainder) = rest.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(Failure::usage(USAGE));
        }
        let mut values = BTreeMap::new();
        for [flag, value] in pairs {
            if !matches!(
                flag.as_str(),
                "--output"
                    | "--like"
                    | "--like-child"
                    | "--environment-from"
                    | "--root-key"
                    | "--root-name"
                    | "--child-key"
                    | "--child-name"
                    | "--child-resource"
                    | "--child-type"
            ) {
                return Err(Failure::usage(format!(
                    "unknown append-region option '{flag}'"
                )));
            }
            if values.insert(flag.as_str(), value.as_str()).is_some() {
                return Err(Failure::usage(format!(
                    "{flag} was supplied more than once"
                )));
            }
        }
        let required = |flag| {
            values
                .get(flag)
                .copied()
                .ok_or_else(|| Failure::usage(format!("{flag} is required")))
        };
        let template_child = values
            .get("--like-child")
            .map(|value| {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(Failure::usage("--like-child requires a decimal index"));
                }
                value
                    .parse::<usize>()
                    .map_err(|_| Failure::usage("--like-child requires a decimal index"))
            })
            .transpose()?;
        Ok(Self {
            input,
            output: required("--output")?,
            template_root: number("--like", required("--like")?)?,
            template_child,
            environment_root: values
                .get("--environment-from")
                .map(|value| number("--environment-from", value))
                .transpose()?,
            append: RegionAppend {
                root_key: number("--root-key", required("--root-key")?)?,
                root_name: required("--root-name")?.as_bytes().to_vec(),
                child_key: number("--child-key", required("--child-key")?)?,
                child_name: required("--child-name")?.as_bytes().to_vec(),
                child_resource: number("--child-resource", required("--child-resource")?)?,
                child_type: values
                    .get("--child-type")
                    .map(|value| number("--child-type", value))
                    .transpose()?,
            },
        })
    }
}

fn number(flag: &str, text: &str) -> Result<u32, Failure> {
    let (digits, radix) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .map_or((text, 10), |digits| (digits, 16));
    if digits.is_empty()
        || !digits.bytes().all(|byte| {
            if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return Err(Failure::usage(format!(
            "{flag} requires a decimal or 0x hexadecimal u32"
        )));
    }
    u32::from_str_radix(digits, radix)
        .map_err(|_| Failure::usage(format!("{flag} requires a decimal or 0x hexadecimal u32")))
}

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let report = append(Options::parse(arguments)?)?;
    std::io::stdout()
        .write_all(to_canonical_json(&report).as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write stdout: {error}")))
}

fn append(options: Options<'_>) -> Result<Value, Failure> {
    let input = read_capped(options.input)?;
    let bytes = append_region_root(
        &input,
        options.template_root,
        options.template_child,
        RegionAppendRequest {
            append: &options.append,
            environment_root: options.environment_root,
        },
    )
    .map_err(format_failure)?;
    let original = region::parse(&input).map_err(format_failure)?;
    let root_index = original
        .rows
        .iter()
        .position(|row| {
            matches!(row.membership, RegionMembership::Root { .. })
                && row.id == options.template_root
        })
        .ok_or_else(|| Failure::parse("template root disappeared"))?;
    let root = &original.rows[root_index];
    let children = &original.rows[root_index + 1..root_index + 1 + root.word_0c as usize];
    let template_child = options
        .template_child
        .or_else(|| {
            children
                .iter()
                .position(|row| !matches!(row.word_04, 0 | 1))
        })
        .ok_or_else(|| Failure::parse("template child disappeared"))?;
    let child_type = options
        .append
        .child_type
        .unwrap_or(children[template_child].word_04);
    let mut environment = Vec::new();
    if let Some(key) = options.environment_root {
        let index = original
            .rows
            .iter()
            .position(|row| {
                matches!(row.membership, RegionMembership::Root { .. }) && row.id == key
            })
            .ok_or_else(|| Failure::parse("environment root disappeared"))?;
        environment.extend(
            original.rows[index + 1..index + 1 + original.rows[index].word_0c as usize]
                .iter()
                .filter(|row| matches!(row.word_04, 0 | 1)),
        );
    }
    let highest = environment
        .iter()
        .map(|row| row.dat_key & 0xFFFF)
        .fold(options.append.child_resource & 0xFFFF, u32::max);
    let derived = (options.append.child_resource & 0xFFFF_0000) | (highest + 1);
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
            verify_environment(
                &input,
                &output,
                original.root_count,
                &options.append,
                child_type,
                derived,
                &environment,
            )?;
            if output != bytes {
                return Err(Failure::parse("written region differs from prepared bytes"));
            }
            let mut report = json!({
                "operation": "append-region",
                "inputLength": input.len(),
                "inputSha256": sha256_hex(&input),
                "inputRootCount": original.root_count,
                "outputLength": output.len(),
                "outputSha256": sha256_hex(&output),
                "outputRootCount": original.root_count + 1,
                "sizeWord": output.len(),
                "rootRowOffset": input.len(),
                "childRowOffset": input.len() + region::ROW_STRIDE,
                "rootKey": ResourceId::new(options.append.root_key).to_hex(),
                "childKey": ResourceId::new(options.append.child_key).to_hex(),
                "childResource": ResourceId::new(options.append.child_resource).to_hex(),
                "childType": child_type,
                "templateRootKey": ResourceId::new(options.template_root).to_hex(),
                "templateChildIndex": template_child,
                "derivedAuxiliaryKey": ResourceId::new(derived).to_hex(),
                "derivedAuxiliaryNumericPath": ResourceId::new(derived).dat_path(),
                "rootNameByteLength": options.append.root_name.len(),
                "childNameByteLength": options.append.child_name.len(),
            });
            if let Some(key) = options.environment_root {
                report["environmentRootKey"] = json!(ResourceId::new(key).to_hex());
                report["environmentRowCount"] = json!(environment.len());
            }
            Ok(report)
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

pub fn verify(
    input: &[u8],
    output: &[u8],
    root_count: u32,
    append: &RegionAppend,
    child_type: u32,
    derived: u32,
) -> Result<(), Failure> {
    verify_environment(input, output, root_count, append, child_type, derived, &[])
}

fn verify_environment(
    input: &[u8],
    output: &[u8],
    root_count: u32,
    append: &RegionAppend,
    child_type: u32,
    derived: u32,
    environment: &[&RegionRow],
) -> Result<(), Failure> {
    let table = region::parse(output).map_err(format_failure)?;
    let fail = || Failure::parse("written region failed append preservation verification");
    if output.len() != input.len() + (2 + environment.len()) * region::ROW_STRIDE
        || table.trailing.length != 0
        || u64::from(table.declared_size) != output.len() as u64
        || table.root_count != root_count + 1
        || output[..0x20] != input[..0x20]
        || output[0x28..input.len()] != input[0x28..]
    {
        return Err(fail());
    }
    let index = table
        .rows
        .len()
        .checked_sub(2 + environment.len())
        .ok_or_else(fail)?;
    let root = &table.rows[index];
    let child = &table.rows[index + 1];
    if root.span.offset != input.len() as u64
        || child.span.offset != (input.len() + region::ROW_STRIDE) as u64
        || root.membership
            != (RegionMembership::Root {
                index: root_count,
                child_count: 1 + environment.len() as u32,
            })
        || child.membership
            != (RegionMembership::Child {
                root_index: root_count,
                index: 0,
            })
        || root.id != append.root_key
        || child.id != append.child_key
        || child.dat_key != append.child_resource
        || child.word_04 != child_type
        || region::effective_name(output, root) != Some(append.root_name.as_slice())
        || region::effective_name(output, child) != Some(append.child_name.as_slice())
        || region::token_terminated(output, root) != Some(true)
        || region::token_terminated(output, child) != Some(true)
        || region::auxiliary_key(&table, index) != Some(AuxiliaryKey::Derived(derived))
    {
        return Err(fail());
    }
    for (copy_index, source) in environment.iter().enumerate() {
        let source_offset = source.span.offset as usize;
        let copy_offset = input.len() + (2 + copy_index) * region::ROW_STRIDE;
        if output[copy_offset..copy_offset + region::ROW_STRIDE]
            != input[source_offset..source_offset + region::ROW_STRIDE]
        {
            return Err(fail());
        }
    }
    Ok(())
}

fn format_failure(error: FormatError) -> Failure {
    Failure::parse(error.to_string())
}
