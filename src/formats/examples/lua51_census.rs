//! Aggregate-only census of a manifest-owned retail Lua 5.1 corpus.
//!
//! This non-gating research command requires explicit external inputs and
//! prints no paths, payload bytes, string contents, or per-script records.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::lpb;
use xivl_formats::lua51::{self, Lua51Operand, Lua51Operands, Lua51Prototype, OPCODES};
use xivl_formats::to_canonical_json;

#[derive(Default)]
struct Census {
    files: u64,
    accepted_scripts: u64,
    exact_headers: u64,
    decoded_bytes: u64,
    prototypes: u64,
    nested_prototypes: u64,
    max_depth: u64,
    depths: BTreeMap<String, u64>,
    minimum_prototypes_per_script: Option<u64>,
    maximum_prototypes_per_script: u64,
    instruction_words: u64,
    minimum_words_per_script: Option<u64>,
    maximum_words_per_script: u64,
    setlist_extra_words: u64,
    closure_bindings: u64,
    closure_move_bindings: u64,
    closure_upvalue_bindings: u64,
    modes: BTreeMap<String, u64>,
    opcodes: BTreeMap<String, u64>,
    rk_registers: u64,
    rk_constants: u64,
    line_info_empty: u64,
    line_info_full: u64,
    line_info_other: u64,
    locals_empty: u64,
    locals_present: u64,
    upvalue_names_empty: u64,
    upvalue_names_full: u64,
    upvalue_names_partial: u64,
    failures: BTreeMap<String, u64>,
}

fn main() {
    let (script_root, manifest_path, owner_commit, check_path) = arguments();
    let manifest_bytes = fs::read(&manifest_path)
        .unwrap_or_else(|error| panic!("cannot read the coverage manifest: {error}"));
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .unwrap_or_else(|error| panic!("coverage manifest is not JSON: {error}"));
    let resources = manifest["resources"]
        .as_array()
        .expect("coverage manifest resources must be an array");

    let mut census = Census::default();
    for opcode in OPCODES {
        census.opcodes.insert(opcode.name.to_string(), 0);
    }
    for mode in ["iABC", "iABx", "iAsBx"] {
        census.modes.insert(mode.to_string(), 0);
    }

    let mut wrappers = BTreeMap::<String, u64>::new();
    for resource in resources {
        if resource["classification"].as_str() != Some("matched-script") {
            continue;
        }
        census.files += 1;
        let relative = resource["resourcePath"]
            .as_str()
            .expect("matched resourcePath must be a string");
        let expected_hash = resource["sha256"]
            .as_str()
            .expect("matched sha256 must be a string");
        let bytes = fs::read(script_root.join(relative))
            .unwrap_or_else(|error| panic!("cannot read a manifest-owned LPB: {error}"));
        assert_eq!(hex_sha256(&bytes), expected_hash, "LPB identity mismatch");
        match lpb::extract(&bytes) {
            Ok(file) => {
                assert_eq!(
                    file.variant.name(),
                    resource["wrapper"]["variant"]
                        .as_str()
                        .expect("wrapper variant must be a string"),
                    "wrapper classification mismatch"
                );
                assert_eq!(
                    file.decoded.len() as u64,
                    resource["wrapper"]["decodedPayloadBytes"]
                        .as_u64()
                        .expect("decoded payload length must be an integer"),
                    "decoded payload length mismatch"
                );
                assert_eq!(
                    hex_sha256(&file.decoded),
                    resource["wrapper"]["decodedPayloadSha256"]
                        .as_str()
                        .expect("decoded payload hash must be a string"),
                    "decoded payload identity mismatch"
                );
                *wrappers.entry(file.variant.name().to_string()).or_default() += 1;
                census.decoded_bytes += file.decoded.len() as u64;
                if file.decoded.starts_with(lua51::EXPECTED_HEADER) {
                    census.exact_headers += 1;
                }
                match lua51::parse(&file.decoded) {
                    Ok(chunk) => {
                        census.accepted_scripts += 1;
                        let prototypes_before = census.prototypes;
                        let words_before = census.instruction_words;
                        visit(&chunk.root, 0, &mut census);
                        let script_prototypes = census.prototypes - prototypes_before;
                        let script_words = census.instruction_words - words_before;
                        census.minimum_prototypes_per_script = Some(
                            census
                                .minimum_prototypes_per_script
                                .map_or(script_prototypes, |value| value.min(script_prototypes)),
                        );
                        census.maximum_prototypes_per_script =
                            census.maximum_prototypes_per_script.max(script_prototypes);
                        census.minimum_words_per_script = Some(
                            census
                                .minimum_words_per_script
                                .map_or(script_words, |value| value.min(script_words)),
                        );
                        census.maximum_words_per_script =
                            census.maximum_words_per_script.max(script_words);
                    }
                    Err(error) => {
                        *census
                            .failures
                            .entry(error.kind().as_str().to_string())
                            .or_default() += 1;
                    }
                }
            }
            Err(error) => {
                *census
                    .failures
                    .entry(error.kind().as_str().to_string())
                    .or_default() += 1;
            }
        }
    }

    assert_eq!(
        census.files,
        manifest["corpus"]["scriptCount"]
            .as_u64()
            .expect("corpus scriptCount must be an integer"),
        "matched resource accounting differs from the corpus"
    );
    assert_eq!(
        wrappers.get("raw").copied().unwrap_or(0),
        manifest["summary"]["wrapperVariants"]["raw"]
            .as_u64()
            .expect("raw wrapper count must be an integer"),
        "raw wrapper accounting mismatch"
    );
    assert_eq!(
        wrappers.get("xor-73").copied().unwrap_or(0),
        manifest["summary"]["wrapperVariants"]["xor-73"]
            .as_u64()
            .expect("xor-73 wrapper count must be an integer"),
        "xor-73 wrapper accounting mismatch"
    );

    let inventory_digest = manifest["source"]["inventorySha256"]
        .as_str()
        .expect("source inventorySha256 must be a string");
    let output = json!({
        "schemaVersion": 1,
        "target": {
            "gameVersion": manifest["gameVersion"],
            "extraction": manifest["extraction"],
        },
        "provenance": {
            "scriptsCommit": owner_commit,
            "coverageManifestSha256": hex_sha256(&manifest_bytes),
            "inventorySha256": inventory_digest,
            "corpus": manifest["corpus"],
            "decoderSourceSha256": hex_sha256(include_bytes!("../src/lua51.rs")),
            "lpbSourceSha256": hex_sha256(include_bytes!("../src/lpb.rs")),
        },
        "accounting": {
            "manifestOwnedScripts": census.files,
            "acceptedScripts": census.accepted_scripts,
            "exactHeaders": census.exact_headers,
            "decodedPayloadBytes": census.decoded_bytes,
            "parserFailures": census.failures,
        },
        "wrappers": wrappers,
        "prototypes": {
            "total": census.prototypes,
            "nested": census.nested_prototypes,
            "maximumDepth": census.max_depth,
            "byDepth": census.depths,
            "perScript": {
                "minimum": census.minimum_prototypes_per_script,
                "maximum": census.maximum_prototypes_per_script,
            },
        },
        "instructions": {
            "words": census.instruction_words,
            "perScript": {
                "minimum": census.minimum_words_per_script,
                "maximum": census.maximum_words_per_script,
            },
            "decodedOpcodeWords": census.opcodes.values().sum::<u64>(),
            "setlistExtraWords": census.setlist_extra_words,
            "closureBindings": {
                "total": census.closure_bindings,
                "move": census.closure_move_bindings,
                "getupval": census.closure_upvalue_bindings,
            },
            "modes": census.modes,
            "opcodes": census.opcodes,
            "rkOperands": {
                "register": census.rk_registers,
                "constant": census.rk_constants,
            },
        },
        "debugTables": {
            "lineInfo": {
                "empty": census.line_info_empty,
                "matchesInstructionWords": census.line_info_full,
                "other": census.line_info_other,
            },
            "locals": {"empty": census.locals_empty, "present": census.locals_present},
            "upvalueNames": {
                "empty": census.upvalue_names_empty,
                "matchesDeclaredUpvalues": census.upvalue_names_full,
                "partial": census.upvalue_names_partial,
            },
        },
    });
    let rendered = to_canonical_json(&output);
    if let Some(path) = check_path {
        let retained = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("cannot read the retained census: {error}"));
        assert_eq!(rendered, retained, "retained census does not match");
        println!("census: retained aggregate matches");
    } else {
        print!("{rendered}");
    }
}

fn arguments() -> (PathBuf, PathBuf, String, Option<PathBuf>) {
    let mut values = env::args().skip(1);
    let mut script_root = None;
    let mut manifest = None;
    let mut owner_commit = None;
    let mut check_path = None;
    while let Some(argument) = values.next() {
        let value = values
            .next()
            .unwrap_or_else(|| panic!("{argument} requires a value"));
        match argument.as_str() {
            "--client-script-root" => script_root = Some(PathBuf::from(value)),
            "--coverage-manifest" => manifest = Some(PathBuf::from(value)),
            "--owner-commit" => owner_commit = Some(value),
            "--check" => check_path = Some(PathBuf::from(value)),
            _ => panic!("unknown argument: {argument}"),
        }
    }
    let script_root = script_root.expect("--client-script-root is required");
    let manifest = manifest.expect("--coverage-manifest is required");
    let owner_commit = owner_commit.expect("--owner-commit is required");
    assert!(
        script_root.is_absolute() && manifest.is_absolute(),
        "input roots must be explicit absolute paths"
    );
    assert!(
        owner_commit.len() == 40 && owner_commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "--owner-commit must be a full 40-digit Git commit"
    );
    (script_root, manifest, owner_commit, check_path)
}

fn visit(prototype: &Lua51Prototype, depth: u64, census: &mut Census) {
    census.prototypes += 1;
    census.nested_prototypes += prototype.nested.len() as u64;
    census.max_depth = census.max_depth.max(depth);
    *census.depths.entry(depth.to_string()).or_default() += 1;
    census.instruction_words += prototype.instruction_count as u64;
    census.setlist_extra_words += prototype.setlist_extra_words.len() as u64;
    match prototype.line_info_count {
        0 => census.line_info_empty += 1,
        count if count == prototype.instruction_count => census.line_info_full += 1,
        _ => census.line_info_other += 1,
    }
    if prototype.local_count == 0 {
        census.locals_empty += 1;
    } else {
        census.locals_present += 1;
    }
    match prototype.upvalue_name_count {
        0 => census.upvalue_names_empty += 1,
        count if count == u32::from(prototype.upvalue_count) => census.upvalue_names_full += 1,
        _ => census.upvalue_names_partial += 1,
    }
    for instruction in &prototype.decoded_instructions {
        *census
            .opcodes
            .get_mut(instruction.opcode.name)
            .expect("official opcode name is initialized") += 1;
        *census
            .modes
            .get_mut(instruction.opcode.mode.name())
            .expect("official mode name is initialized") += 1;
        match instruction.operands {
            Lua51Operands::Abc { b, c, .. } => {
                count_rk(b, census);
                count_rk(c, census);
            }
            Lua51Operands::Abx { bx, .. } => count_rk(bx, census),
            Lua51Operands::Asbx { .. } => {}
        }
        if instruction.opcode.number == 36 {
            let Lua51Operands::Abx {
                bx: Lua51Operand::Value { value },
                ..
            } = instruction.operands
            else {
                unreachable!("CLOSURE uses a value Bx")
            };
            let child = &prototype.nested[value as usize];
            census.closure_bindings += u64::from(child.upvalue_count);
            for index in 1..=u32::from(child.upvalue_count) {
                let binding = prototype
                    .decoded_instructions
                    .iter()
                    .find(|candidate| candidate.index == instruction.index + index)
                    .expect("validated CLOSURE binding exists");
                match binding.opcode.number {
                    0 => census.closure_move_bindings += 1,
                    4 => census.closure_upvalue_bindings += 1,
                    _ => unreachable!("validated CLOSURE binding opcode"),
                }
            }
        }
    }
    for child in &prototype.nested {
        visit(child, depth + 1, census);
    }
}

fn count_rk(operand: Lua51Operand, census: &mut Census) {
    match operand {
        Lua51Operand::Register { rk: true, .. } => census.rk_registers += 1,
        Lua51Operand::Constant { rk: true, .. } => census.rk_constants += 1,
        _ => {}
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    sha256_hex(bytes).to_ascii_uppercase()
}
