//! Pack validated Lua 5.1 chunks into evidenced LPB wrappers.

use std::fs;
use std::io::Write;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::{
    encode_lpb, extract_lpb, parse_lua51, rebuild_lpb, to_canonical_json, FormatError, LpbWrapper,
    Lua51Prototype,
};

use crate::{read_capped, Failure};

const USAGE: &str = "usage: xivl pack-lpb <chunk.luac> --output <new.lpb> [--like <existing.lpb>] [--wrapper rle|rlu] [--tag <hex u16>]";

struct Options<'a> {
    input: &'a str,
    output: &'a str,
    like: Option<&'a str>,
    wrapper: &'a str,
    tag: Option<u16>,
}

impl<'a> Options<'a> {
    fn parse(arguments: &'a [String]) -> Result<Self, Failure> {
        let Some((input, rest)) = arguments.split_first() else {
            return Err(Failure::usage(USAGE));
        };
        let mut output = None;
        let mut like = None;
        let mut wrapper = None;
        let mut tag = None;
        let (pairs, remainder) = rest.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(Failure::usage(USAGE));
        }
        for [flag, value] in pairs {
            let duplicate = match flag.as_str() {
                "--output" => output.replace(value.as_str()).is_some(),
                "--like" => like.replace(value.as_str()).is_some(),
                "--wrapper" => {
                    if !matches!(value.as_str(), "rle" | "rlu") {
                        return Err(Failure::usage("--wrapper requires rle or rlu"));
                    }
                    wrapper.replace(value.as_str()).is_some()
                }
                "--tag" => {
                    let hex = value
                        .strip_prefix("0x")
                        .or_else(|| value.strip_prefix("0X"))
                        .unwrap_or(value);
                    if hex.is_empty()
                        || hex.len() > 4
                        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err(Failure::usage("--tag requires a hex u16"));
                    }
                    let parsed = u16::from_str_radix(hex, 16)
                        .map_err(|_| Failure::usage("--tag requires a hex u16"))?;
                    tag.replace(parsed).is_some()
                }
                _ => return Err(Failure::usage(format!("unknown pack-lpb option '{flag}'"))),
            };
            if duplicate {
                return Err(Failure::usage(format!(
                    "{flag} was supplied more than once"
                )));
            }
        }
        if like.is_some() && (wrapper.is_some() || tag.is_some()) {
            return Err(Failure::usage(
                "--like cannot be combined with --wrapper or --tag",
            ));
        }
        if like.is_none() && tag.is_none() {
            return Err(Failure::usage("--tag is required without --like"));
        }
        Ok(Self {
            input,
            output: output.ok_or_else(|| Failure::usage("--output is required"))?,
            like,
            wrapper: wrapper.unwrap_or("rle"),
            tag,
        })
    }
}

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let report = pack(Options::parse(arguments)?)?;
    std::io::stdout()
        .write_all(to_canonical_json(&report).as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write stdout: {error}")))
}

fn pack(options: Options<'_>) -> Result<Value, Failure> {
    let input = read_capped(options.input)?;
    let chunk = parse_lua51(&input).map_err(format_failure)?;
    let wrapper = if let Some(path) = options.like {
        extract_lpb(&read_capped(path)?)
            .and_then(|file| file.wrapper())
            .map_err(format_failure)?
    } else {
        let tag =
            u32::from(options.tag.expect("options require a tag without --like")).to_le_bytes();
        match options.wrapper {
            "rlu" => LpbWrapper::Rlu { tag },
            _ => LpbWrapper::Rle { tag, byte12: 0xff },
        }
    };
    let (kind, tag) = match &wrapper {
        LpbWrapper::Rle { tag, .. } => ("rle", tag),
        LpbWrapper::Rlu { tag } => ("rlu", tag),
    };
    let bytes = encode_lpb(&wrapper, &input).map_err(format_failure)?;
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
            let extracted = extract_lpb(&output).map_err(format_failure)?;
            if extracted.decoded != input
                || output != bytes
                || rebuild_lpb(&output, &extracted).map_err(format_failure)? != output
            {
                return Err(Failure::parse(
                    "written LPB failed chunk or exact rebuild verification",
                ));
            }
            let (prototypes, instructions) = lua_counts(&chunk.root);
            Ok(json!({
                "operation": "pack-lpb",
                "wrapper": kind,
                "tag": format!("0x{:08x}", u32::from_le_bytes(*tag)),
                "chunkLength": input.len(),
                "chunkSha256": sha256_hex(&input),
                "outputLength": output.len(),
                "outputSha256": sha256_hex(&output),
                "lua": { "prototypeCount": prototypes, "instructionCount": instructions },
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

fn lua_counts(prototype: &Lua51Prototype) -> (u64, u64) {
    prototype.nested.iter().map(lua_counts).fold(
        (1, u64::from(prototype.instruction_count)),
        |(prototypes, instructions), (children, child_instructions)| {
            (prototypes + children, instructions + child_instructions)
        },
    )
}

fn format_failure(error: FormatError) -> Failure {
    Failure::parse(error.to_string())
}
