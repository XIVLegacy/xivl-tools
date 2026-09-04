//! Schema-versioned JSON or YAML extraction for one understood resource.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::sedb::{self, EntryBody};
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
pub(crate) enum DocumentFormat {
    Yaml,
    Json,
}

struct PayloadArtifact {
    path: String,
    bytes: Vec<u8>,
    manifest: Value,
}

pub(crate) struct PlannedExtraction {
    document_name: &'static str,
    document: String,
    artifacts: Vec<PayloadArtifact>,
    format_id: String,
}

pub fn run(arguments: &[String]) -> Result<ExtractResourceSummary, Failure> {
    let Some(input) = arguments.first() else {
        return Err(Failure::usage(
            "usage: xivl extract-resource <file> --output <directory> [--format yaml|json] [--materialize-payloads] [--as <format>] [--columns <list>]",
        ));
    };
    let mut output = None;
    let mut format = DocumentFormat::Yaml;
    let mut materialize_payloads = false;
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
            "--materialize-payloads" => {
                if materialize_payloads {
                    return Err(Failure::usage(
                        "--materialize-payloads was supplied more than once",
                    ));
                }
                materialize_payloads = true;
                index += 1;
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
    let planned = plan_bytes(
        input,
        &data,
        format,
        materialize_payloads,
        &inspect_arguments,
    )?;
    planned.write_to(output_path)?;
    Ok(ExtractResourceSummary {
        output: output_path
            .join(planned.document_name)
            .display()
            .to_string(),
    })
}

pub(crate) fn plan_bytes(
    input: &str,
    data: &[u8],
    format: DocumentFormat,
    materialize_payloads: bool,
    inspect_arguments: &[String],
) -> Result<PlannedExtraction, Failure> {
    let selected = if inspect_arguments.is_empty() {
        detect(data).map(|(_, how)| how).ok_or_else(|| {
            Failure::usage(format!(
                "{input}: unrecognized format; use --as for a signatureless supported format"
            ))
        })?
    } else {
        InspectAs::from_arguments(inspect_arguments).map_err(Failure::usage)?
    };
    let name = base_name(input);
    let parsed = inspect_named_bytes_as(data, name, &selected).map_err(|error| Failure {
        message: format!("{input}: {error}"),
        code: EXIT_PARSE_FAILURE,
    })?;
    let resource_id = parse_dat_path(input, 0).ok().map(|id| id.to_hex());
    let mut artifacts = Vec::new();
    let lpb = if matches!(selected, InspectAs::Lpb | InspectAs::LpbBytecode) {
        Some(extract_lpb(data).map_err(|error| Failure {
            message: format!("{input}: {error}"),
            code: EXIT_PARSE_FAILURE,
        })?)
    } else {
        None
    };
    if let Some(file) = &lpb {
        let path = "payloads/decoded.luac".to_string();
        artifacts.push(PayloadArtifact {
            manifest: json!({
                "path": path,
                "role": "decoded-lua-5.1-chunk",
                "sha256": sha256_hex(&file.decoded),
                "size": file.decoded.len() as u64,
            }),
            path,
            bytes: file.decoded.clone(),
        });
    }
    let parsed_format = parsed
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if materialize_payloads {
        if !matches!(parsed_format.as_str(), "sedb" | "res") {
            return Err(Failure::usage(format!(
                "--materialize-payloads applies only to SEDB/RES input, not '{parsed_format}'"
            )));
        }
        artifacts.extend(container_payloads(data, input)?);
    }
    let payloads: Vec<Value> = artifacts
        .iter()
        .map(|artifact| artifact.manifest.clone())
        .collect();
    let document = json!({
        "anomalies": collect_anomalies(&parsed),
        "format": {
            "id": parsed_format,
            "parseStatus": "parsed",
            "supportStatus": read_support(&parsed_format),
        },
        "parsed": parsed,
        "payloads": payloads,
        "schemaVersion": EXTRACTION_SCHEMA_VERSION,
        "source": {
            "fileName": name,
            "resourceId": resource_id,
            "sha256": sha256_hex(data),
            "size": data.len() as u64,
        },
        "tool": {
            "commit": option_env!("XIVL_GIT_COMMIT"),
            "name": "xivl",
            "version": env!("CARGO_PKG_VERSION"),
        },
    });
    // Render before creating the output directory. Parse, span-safety, and
    // document-generation failures therefore leave no partial extraction.
    let (document_name, document) = match format {
        DocumentFormat::Yaml => (
            "extraction.yaml",
            serde_yaml::to_string(&document)
                .map_err(|error| Failure::usage(format!("cannot render YAML: {error}")))?,
        ),
        DocumentFormat::Json => ("extraction.json", to_canonical_json(&document)),
    };
    Ok(PlannedExtraction {
        document_name,
        document,
        artifacts,
        format_id: parsed_format,
    })
}

impl PlannedExtraction {
    pub(crate) fn document(&self) -> &str {
        &self.document
    }

    pub(crate) fn document_name(&self) -> &'static str {
        self.document_name
    }

    pub(crate) fn format_id(&self) -> &str {
        &self.format_id
    }

    pub(crate) fn output_bytes(&self) -> Result<u64, Failure> {
        self.artifacts
            .iter()
            .try_fold(self.document.len() as u64, |total, artifact| {
                total
                    .checked_add(artifact.bytes.len() as u64)
                    .ok_or_else(|| Failure::usage("output byte accounting overflowed u64"))
            })
    }

    pub(crate) fn write_to(&self, output_path: &Path) -> Result<(), Failure> {
        fs::create_dir_all(output_path).map_err(|error| {
            Failure::usage(format!(
                "cannot create output directory '{}': {error}",
                output_path.display()
            ))
        })?;
        if !self.artifacts.is_empty() {
            let payload_directory = output_path.join("payloads");
            fs::create_dir(&payload_directory).map_err(|error| {
                Failure::usage(format!(
                    "cannot create payload directory '{}': {error}",
                    payload_directory.display()
                ))
            })?;
            for artifact in &self.artifacts {
                let destination = output_path.join(&artifact.path);
                fs::write(&destination, &artifact.bytes).map_err(|error| {
                    Failure::usage(format!("cannot write '{}': {error}", destination.display()))
                })?;
            }
        }
        let destination = output_path.join(self.document_name);
        fs::write(&destination, &self.document).map_err(|error| {
            Failure::usage(format!("cannot write '{}': {error}", destination.display()))
        })?;
        Ok(())
    }
}

fn container_payloads(data: &[u8], input: &str) -> Result<Vec<PayloadArtifact>, Failure> {
    let root = sedb::parse_container(data, 0).map_err(|error| Failure {
        message: format!("{input}: {error}"),
        code: EXIT_PARSE_FAILURE,
    })?;
    sedb::validate_payload_materialization(&root).map_err(|error| Failure {
        message: format!("{input}: {error}"),
        code: EXIT_PARSE_FAILURE,
    })?;

    let mut artifacts = Vec::new();
    for (entry_index, entry) in root.entries.iter().enumerate() {
        if matches!(entry.body, EntryBody::Directory { .. }) {
            continue;
        }
        let start = usize::try_from(entry.span.offset).map_err(|_| {
            Failure::parse(format!(
                "{input}: payload offset {} does not fit this platform",
                entry.span.offset
            ))
        })?;
        let end_u64 = entry
            .span
            .offset
            .checked_add(entry.span.length)
            .ok_or_else(|| Failure::parse(format!("{input}: payload span end overflows u64")))?;
        let end = usize::try_from(end_u64).map_err(|_| {
            Failure::parse(format!(
                "{input}: payload end {end_u64} does not fit this platform"
            ))
        })?;
        let bytes = data.get(start..end).ok_or_else(|| {
            Failure::parse(format!(
                "{input}: payload span {}..{end_u64} is outside the {} byte input",
                entry.span.offset,
                data.len()
            ))
        })?;
        let role = entry.body.kind_name();
        let digest_prefix = &entry.sha256[..16];
        let path = format!(
            "payloads/{entry_index:06}-{role}-o{:016x}-l{:016x}-{digest_prefix}.bin",
            entry.span.offset, entry.span.length
        );
        let mut entry_relationship = json!({
            "kind": role,
            "path": format!("$.parsed.root.entries[{entry_index}]"),
        });
        if let EntryBody::Subresource {
            index,
            declared_offset,
            declared_size,
            kind,
            child,
        } = &entry.body
        {
            let object = entry_relationship
                .as_object_mut()
                .expect("the entry relationship is an object");
            object.insert("index".into(), json!(index));
            object.insert("declaredOffset".into(), json!(declared_offset));
            object.insert("declaredSize".into(), json!(declared_size));
            object.insert("declaredKind".into(), json!(kind));
            if let Some(child) = child {
                object.insert(
                    "childContainer".into(),
                    json!({
                        "format": child.format_id(),
                        "span": child.span.to_json(),
                        "subtype": child.subtype,
                    }),
                );
            }
        }
        artifacts.push(PayloadArtifact {
            manifest: json!({
                "container": {
                    "format": root.format_id(),
                    "path": "$.parsed.root",
                    "span": root.span.to_json(),
                    "subtype": root.subtype,
                },
                "entry": entry_relationship,
                "path": path,
                "role": role,
                "sha256": entry.sha256,
                "size": entry.span.length,
                "sourceSpan": {
                    "endExclusive": end_u64,
                    "length": entry.span.length,
                    "offset": entry.span.offset,
                },
            }),
            path,
            bytes: bytes.to_vec(),
        });
    }
    Ok(artifacts)
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

    fn res(entries: &[(u32, u32, u32, u32)], payload: &[u8]) -> Vec<u8> {
        let payload_base = 0x40 + entries.len() * 16;
        let total = payload_base + payload.len();
        let mut bytes = vec![0u8; payload_base];
        bytes[0..4].copy_from_slice(b"SEDB");
        bytes[4..8].copy_from_slice(b"RES ");
        bytes[8..12].copy_from_slice(&0xFA0u32.to_le_bytes());
        bytes[14..16].copy_from_slice(&0x40u16.to_le_bytes());
        bytes[16..20].copy_from_slice(&(total as u32).to_le_bytes());
        bytes[0x30..0x34].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        bytes[0x38..0x3c].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        bytes[0x3c..0x40].copy_from_slice(b"test");
        for (slot, (index, offset, size, kind)) in entries.iter().enumerate() {
            let start = 0x40 + slot * 16;
            bytes[start..start + 4].copy_from_slice(&index.to_le_bytes());
            bytes[start + 4..start + 8].copy_from_slice(&offset.to_le_bytes());
            bytes[start + 8..start + 12].copy_from_slice(&size.to_le_bytes());
            bytes[start + 12..start + 16].copy_from_slice(&kind.to_le_bytes());
        }
        bytes.extend_from_slice(payload);
        bytes
    }

    fn extract_container(source: &Path, output: &Path) -> Result<ExtractResourceSummary, Failure> {
        run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--materialize-payloads".into(),
            "--as".into(),
            "sedb".into(),
        ])
    }

    fn yaml_document(output: &Path) -> Value {
        serde_yaml::from_str(&fs::read_to_string(output.join("extraction.yaml")).unwrap()).unwrap()
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

    #[test]
    fn materializes_plain_sedb_payload_exactly() {
        let root = temp_root("sedb-payload");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.DAT");
        let bytes = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        fs::write(&source, bytes).unwrap();
        let output = root.join("out");
        extract_container(&source, &output).unwrap();

        let document = yaml_document(&output);
        let payload = &document["payloads"][0];
        assert_eq!(payload["role"], "payload");
        let start = payload["sourceSpan"]["offset"].as_u64().unwrap() as usize;
        let end = payload["sourceSpan"]["endExclusive"].as_u64().unwrap() as usize;
        let path = payload["path"].as_str().unwrap();
        assert_eq!(fs::read(output.join(path)).unwrap(), bytes[start..end]);
        assert!(path.contains("-o"));
        assert!(path.contains("-l"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_res_materializes_direct_entries_once_and_deterministically() {
        let root = temp_root("res-nested");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.DAT");
        let bytes = include_bytes!("../../../tests/fixtures/public/res/two-subresources.bin");
        fs::write(&source, bytes).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        extract_container(&source, &first).unwrap();
        extract_container(&source, &second).unwrap();

        let document = yaml_document(&first);
        let payloads = document["payloads"].as_array().unwrap();
        assert_eq!(payloads.len(), 3);
        assert_eq!(payloads[0]["entry"]["childContainer"]["format"], "sedb");
        assert_eq!(payloads[1]["role"], "unknown-gap");
        for payload in payloads {
            let path = payload["path"].as_str().unwrap();
            assert_eq!(
                fs::read(first.join(path)).unwrap(),
                fs::read(second.join(path)).unwrap()
            );
        }
        assert_eq!(
            fs::read(first.join("extraction.yaml")).unwrap(),
            fs::read(second.join("extraction.yaml")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_payload_is_preserved_as_an_empty_file() {
        let root = temp_root("res-empty");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.DAT");
        fs::write(&source, res(&[(7, 0, 0, 4)], b"")).unwrap();
        let output = root.join("out");
        extract_container(&source, &output).unwrap();
        let document = yaml_document(&output);
        assert_eq!(document["payloads"][0]["size"], 0);
        let path = document["payloads"][0]["path"].as_str().unwrap();
        assert_eq!(fs::read(output.join(path)).unwrap(), b"");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overlap_alias_clamp_and_malformed_nested_payloads_are_refused_before_output() {
        let root = temp_root("res-refusals");
        fs::create_dir_all(&root).unwrap();
        let cases = [
            ("overlap.DAT", res(&[(0, 0, 4, 0), (1, 2, 4, 0)], b"abcdef")),
            ("alias.DAT", res(&[(0, 0, 4, 0), (1, 0, 4, 0)], b"abcd")),
            ("out-of-bounds.DAT", res(&[(0, 100, 1, 0)], b"")),
            ("nested.DAT", res(&[(0, 0, 4, 0)], b"SEDB")),
            (
                "clamped.DAT",
                include_bytes!("../../../tests/fixtures/public/res/clamped-extent.bin").to_vec(),
            ),
        ];
        for (number, (name, bytes)) in cases.into_iter().enumerate() {
            let source = root.join(name);
            fs::write(&source, bytes).unwrap();
            let output = root.join(format!("out-{number}"));
            let error = extract_container(&source, &output).unwrap_err();
            assert_eq!(error.code, EXIT_PARSE_FAILURE);
            assert!(error.message.contains("ambiguous-payload-span at offset"));
            assert!(!output.exists());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn truncated_container_with_materialization_writes_nothing() {
        let root = temp_root("payload-truncated");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.DAT");
        fs::write(&source, b"SEDB").unwrap();
        let output = root.join("out");
        let error = extract_container(&source, &output).unwrap_err();
        assert_eq!(error.code, EXIT_PARSE_FAILURE);
        assert!(error
            .message
            .contains("unexpected-end-of-input at offset 4"));
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialize_flag_rejects_non_container_input() {
        let root = temp_root("payload-option");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("script.lpb");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/lpb/raw.bin"),
        )
        .unwrap();
        let output = root.join("out");
        let error = run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--materialize-payloads".into(),
            "--as".into(),
            "lpb".into(),
        ])
        .unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("applies only to SEDB/RES"));
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialize_flag_rejects_signature_only_gtex() {
        let root = temp_root("gtex-payload-option");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("texture.DAT");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/gtex/tagged.bin"),
        )
        .unwrap();
        let output = root.join("out");
        let error = run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--materialize-payloads".into(),
        ])
        .unwrap_err();
        assert!(error.message.contains("applies only to SEDB/RES"));
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
