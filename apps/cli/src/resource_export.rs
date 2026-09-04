//! Schema-versioned JSON or YAML extraction for one understood resource.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::{
    extract_lpb, inspect_named_bytes_as, parse_dat_path, to_canonical_json, InspectAs,
};

use crate::scan::{collect_anomalies, detect, read_support, require_empty_output};
use crate::{base_name, read_capped, Failure, EXIT_PARSE_FAILURE};

pub const EXTRACTION_SCHEMA_VERSION: u64 = 1;

#[derive(Debug)]
pub struct ExtractResourceSummary {
    pub output: String,
}

#[derive(Clone, Copy)]
enum DocumentFormat {
    Yaml,
    Json,
}

pub fn run(arguments: &[String]) -> Result<ExtractResourceSummary, Failure> {
    let Some(input) = arguments.first() else {
        return Err(Failure::usage(
            "usage: xivl extract-resource <file> --output <directory> [--format yaml|json] [--as <format>] [--columns <list>]",
        ));
    };
    let mut output = None;
    let mut format = DocumentFormat::Yaml;
    let mut inspect_arguments = Vec::new();
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--output" if index + 1 < arguments.len() => {
                if output.replace(arguments[index + 1].clone()).is_some() {
                    return Err(Failure::usage("--output was supplied more than once"));
                }
                index += 2;
            }
            "--format" if index + 1 < arguments.len() => {
                format = match arguments[index + 1].as_str() {
                    "yaml" => DocumentFormat::Yaml,
                    "json" => DocumentFormat::Json,
                    other => {
                        return Err(Failure::usage(format!(
                            "unknown extraction format '{other}'; expected yaml or json"
                        )))
                    }
                };
                index += 2;
            }
            "--as" | "--columns" if index + 1 < arguments.len() => {
                inspect_arguments.push(arguments[index].clone());
                inspect_arguments.push(arguments[index + 1].clone());
                index += 2;
            }
            option => {
                return Err(Failure::usage(format!(
                    "unknown extract-resource option '{option}'"
                )))
            }
        }
    }
    let output =
        output.ok_or_else(|| Failure::usage("extract-resource requires --output <directory>"))?;
    let output_path = Path::new(&output);
    require_empty_output(output_path)?;
    let data = read_capped(input)?;
    let selected = if inspect_arguments.is_empty() {
        detect(&data).map(|(_, how)| how).ok_or_else(|| {
            Failure::usage(format!(
                "{input}: unrecognized format; use --as for a signatureless supported format"
            ))
        })?
    } else {
        InspectAs::from_arguments(&inspect_arguments).map_err(Failure::usage)?
    };
    let name = base_name(input);
    let parsed = inspect_named_bytes_as(&data, name, &selected).map_err(|error| Failure {
        message: format!("{input}: {error}"),
        code: EXIT_PARSE_FAILURE,
    })?;
    let resource_id = parse_dat_path(input, 0).ok().map(|id| id.to_hex());
    let mut payloads = Vec::new();
    let lpb = if matches!(selected, InspectAs::Lpb | InspectAs::LpbBytecode) {
        Some(extract_lpb(&data).map_err(|error| Failure {
            message: format!("{input}: {error}"),
            code: EXIT_PARSE_FAILURE,
        })?)
    } else {
        None
    };
    if let Some(file) = &lpb {
        payloads.push(json!({
            "path": "payloads/decoded.luac",
            "role": "decoded-lua-5.1-chunk",
            "sha256": sha256_hex(&file.decoded),
            "size": file.decoded.len() as u64,
        }));
    }
    let parsed_format = parsed
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let document = json!({
        "anomalies": collect_anomalies(&parsed),
        "format": {
            "id": parsed_format,
            "parseStatus": "parsed",
            "supportStatus": read_support(parsed_format),
        },
        "parsed": parsed,
        "payloads": payloads,
        "schemaVersion": EXTRACTION_SCHEMA_VERSION,
        "source": {
            "fileName": name,
            "resourceId": resource_id,
            "sha256": sha256_hex(&data),
            "size": data.len() as u64,
        },
        "tool": {
            "commit": option_env!("XIVL_GIT_COMMIT"),
            "name": "xivl",
            "version": env!("CARGO_PKG_VERSION"),
        },
    });
    fs::create_dir_all(output_path).map_err(|error| {
        Failure::usage(format!(
            "cannot create output directory '{}': {error}",
            output_path.display()
        ))
    })?;
    if let Some(file) = lpb {
        let payload_directory = output_path.join("payloads");
        fs::create_dir(&payload_directory).map_err(|error| {
            Failure::usage(format!(
                "cannot create payload directory '{}': {error}",
                payload_directory.display()
            ))
        })?;
        fs::write(payload_directory.join("decoded.luac"), file.decoded).map_err(|error| {
            Failure::usage(format!("cannot write decoded LPB payload: {error}"))
        })?;
    }
    let (name, text) = match format {
        DocumentFormat::Yaml => (
            "extraction.yaml",
            serde_yaml::to_string(&document)
                .map_err(|error| Failure::usage(format!("cannot render YAML: {error}")))?,
        ),
        DocumentFormat::Json => ("extraction.json", to_canonical_json(&document)),
    };
    let destination = output_path.join(name);
    fs::write(&destination, text).map_err(|error| {
        Failure::usage(format!("cannot write '{}': {error}", destination.display()))
    })?;
    Ok(ExtractResourceSummary {
        output: destination.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "xivl-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn writes_yaml_and_keeps_lpb_payload_separate() {
        let root = temp_root("extract");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("script.lpb");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/lpb/raw.bin"),
        )
        .unwrap();
        let output = root.join("out");
        run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--as".into(),
            "lpb".into(),
        ])
        .unwrap();
        let yaml = fs::read_to_string(output.join("extraction.yaml")).unwrap();
        let document: Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(document["schemaVersion"], EXTRACTION_SCHEMA_VERSION);
        assert_eq!(document["format"]["id"], "lpb");
        assert_eq!(document["payloads"][0]["path"], "payloads/decoded.luac");
        assert!(output.join("payloads/decoded.luac").is_file());
        assert!(!yaml.contains("base64"));

        let second_output = root.join("out-second");
        run(&[
            source.display().to_string(),
            "--output".into(),
            second_output.display().to_string(),
            "--as".into(),
            "lpb".into(),
        ])
        .unwrap();
        assert_eq!(
            fs::read(output.join("extraction.yaml")).unwrap(),
            fs::read(second_output.join("extraction.yaml")).unwrap()
        );
        assert_eq!(
            fs::read(output.join("payloads/decoded.luac")).unwrap(),
            fs::read(second_output.join("payloads/decoded.luac")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_input_is_a_parse_failure_and_writes_nothing() {
        let root = temp_root("extract-malformed");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("bad.DAT");
        fs::write(&source, b"SEDB").unwrap();
        let output = root.join("out");
        let error = run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
        ])
        .unwrap_err();
        assert_eq!(error.code, EXIT_PARSE_FAILURE);
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
