//! Deterministic DAT resource cataloging.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::{inspect_named_bytes_as, parse_dat_path, to_canonical_json, InspectAs};

use crate::{base_name, read_capped, Failure};

pub const CATALOG_SCHEMA_VERSION: u64 = 1;

pub struct CatalogSummary {
    pub resources: usize,
    pub output: String,
}

#[derive(Clone, Copy)]
enum CatalogFormat {
    Json,
    Jsonl,
}

pub fn run(arguments: &[String]) -> Result<CatalogSummary, Failure> {
    let Some(root) = arguments.first() else {
        return Err(Failure::usage(
            "usage: xivl catalog <game-or-resource-directory> --output <directory> [--format json|jsonl]",
        ));
    };
    let mut output = None;
    let mut format = CatalogFormat::Json;
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
                    "json" => CatalogFormat::Json,
                    "jsonl" => CatalogFormat::Jsonl,
                    other => {
                        return Err(Failure::usage(format!(
                            "unknown catalog format '{other}'; expected json or jsonl"
                        )))
                    }
                };
                index += 2;
            }
            option => return Err(Failure::usage(format!("unknown catalog option '{option}'"))),
        }
    }
    let output = output.ok_or_else(|| Failure::usage("catalog requires --output <directory>"))?;
    let output_path = Path::new(&output);
    require_empty_output(output_path)?;
    let entries = scan(Path::new(root))?;
    fs::create_dir_all(output_path).map_err(|error| {
        Failure::usage(format!(
            "cannot create output directory '{}': {error}",
            output_path.display()
        ))
    })?;
    let (name, text) = match format {
        CatalogFormat::Json => {
            let document = json!({
                "schemaVersion": CATALOG_SCHEMA_VERSION,
                "resourceCount": entries.len() as u64,
                "resources": entries,
            });
            ("catalog.json", to_canonical_json(&document))
        }
        CatalogFormat::Jsonl => {
            let mut lines = String::new();
            for entry in &entries {
                lines.push_str(
                    &serde_json::to_string(entry)
                        .expect("catalog entries built from JSON values serialize"),
                );
                lines.push('\n');
            }
            ("catalog.jsonl", lines)
        }
    };
    let destination = output_path.join(name);
    fs::write(&destination, text).map_err(|error| {
        Failure::usage(format!("cannot write '{}': {error}", destination.display()))
    })?;
    Ok(CatalogSummary {
        resources: entries.len(),
        output: destination.display().to_string(),
    })
}

fn scan(root: &Path) -> Result<Vec<Value>, Failure> {
    if !root.is_dir() {
        return Err(Failure::usage(format!(
            "catalog root '{}' is not a directory",
            root.display()
        )));
    }
    let scan_root = if root.join("data").is_dir() {
        root.join("data")
    } else {
        root.to_path_buf()
    };
    let mut paths = Vec::new();
    collect_dat_paths(&scan_root, &mut paths)?;
    paths.sort_by_key(|path| {
        let relative = relative_path(root, path);
        (relative.to_ascii_lowercase(), relative)
    });
    paths.iter().map(|path| catalog_entry(root, path)).collect()
}

fn collect_dat_paths(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), Failure> {
    let entries = fs::read_dir(directory).map_err(|error| {
        Failure::usage(format!(
            "cannot read directory '{}': {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| Failure::usage(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| Failure::usage(error.to_string()))?;
        let path = entry.path();
        if kind.is_symlink() {
            return Err(Failure::usage(format!(
                "catalog does not follow symbolic link '{}'",
                path.display()
            )));
        }
        if kind.is_dir() {
            collect_dat_paths(&path, output)?;
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("dat"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn catalog_entry(root: &Path, path: &Path) -> Result<Value, Failure> {
    let bytes = read_capped(&path.display().to_string())?;
    let source_path = relative_path(root, path);
    let resource_id = parse_dat_path(&source_path, 0).ok().map(|id| id.to_hex());
    let Some((detected_format, how)) = detect(&bytes) else {
        return Ok(json!({
            "anomalies": [],
            "detectedFormat": "unknown",
            "formatStatus": "unknown",
            "resourceId": resource_id,
            "schemaVersion": CATALOG_SCHEMA_VERSION,
            "sha256": sha256_hex(&bytes),
            "size": bytes.len() as u64,
            "sourcePath": source_path,
            "spans": [],
            "supportStatus": "none",
        }));
    };
    match inspect_named_bytes_as(&bytes, base_name(&source_path), &how) {
        Ok(parsed) => Ok(json!({
            "anomalies": collect_anomalies(&parsed),
            "detectedFormat": parsed.get("format").and_then(Value::as_str).unwrap_or(detected_format),
            "formatStatus": "parsed",
            "resourceId": resource_id,
            "schemaVersion": CATALOG_SCHEMA_VERSION,
            "sha256": sha256_hex(&bytes),
            "size": bytes.len() as u64,
            "sourcePath": source_path,
            "spans": collect_spans(&parsed),
            "supportStatus": read_support(parsed.get("format").and_then(Value::as_str).unwrap_or(detected_format)),
        })),
        Err(error) => Ok(json!({
            "anomalies": [{
                "kind": error.kind().as_str(),
                "offset": error.offset(),
                "detail": error.detail(),
            }],
            "detectedFormat": detected_format,
            "formatStatus": "malformed",
            "resourceId": resource_id,
            "schemaVersion": CATALOG_SCHEMA_VERSION,
            "sha256": sha256_hex(&bytes),
            "size": bytes.len() as u64,
            "sourcePath": source_path,
            "spans": [],
            "supportStatus": read_support(detected_format),
        })),
    }
}

pub fn read_support(format: &str) -> &'static str {
    static READ_SUPPORT: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    READ_SUPPORT
        .get_or_init(|| {
            let matrix: Value =
                serde_json::from_str(include_str!("../../../data/support-matrix.json"))
                    .expect("the contract gate validates the embedded support matrix");
            matrix["formats"]
                .as_array()
                .expect("the support matrix has a formats array")
                .iter()
                .map(|entry| {
                    (
                        entry["id"].as_str().expect("format id is a string").into(),
                        entry["read"]
                            .as_str()
                            .expect("read status is a string")
                            .into(),
                    )
                })
                .collect()
        })
        .get(format)
        .map(String::as_str)
        .unwrap_or("none")
}

pub fn detect(data: &[u8]) -> Option<(&'static str, InspectAs)> {
    if xivl_formats::staticactor::has_signature(data) {
        Some(("staticactor-san", InspectAs::StaticActorSan))
    } else if xivl_formats::ssd::has_document_signature(data) {
        Some(("ssd", InspectAs::Ssd))
    } else if xivl_formats::sqwt::has_signature(data) {
        Some(("sqwt", InspectAs::Sqwt))
    } else if xivl_formats::scrambled::has_signature(data) {
        Some(("scrambled-xml", InspectAs::ScrambledXml))
    } else if xivl_formats::lpb::has_signature(data) {
        Some(("lpb", InspectAs::Lpb))
    } else if xivl_formats::sedb::has_magic(data) {
        Some(("sedb", InspectAs::Sedb))
    } else {
        None
    }
}

pub fn collect_spans(value: &Value) -> Vec<Value> {
    let mut output = Vec::new();
    collect_spans_at(value, "$", &mut output);
    output
}

fn collect_spans_at(value: &Value, path: &str, output: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            if let (Some(offset), Some(length)) = (
                object.get("offset").and_then(Value::as_u64),
                object.get("length").and_then(Value::as_u64),
            ) {
                output.push(json!({ "path": path, "offset": offset, "length": length }));
                return;
            }
            for (key, child) in object {
                collect_spans_at(child, &format!("{path}.{key}"), output);
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_spans_at(child, &format!("{path}[{index}]"), output);
            }
        }
        _ => {}
    }
}

pub fn collect_anomalies(value: &Value) -> Vec<Value> {
    let mut output = Vec::new();
    collect_named_arrays(value, "anomalies", &mut output);
    output
}

fn collect_named_arrays(value: &Value, name: &str, output: &mut Vec<Value>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == name {
                    if let Value::Array(values) = child {
                        output.extend(values.iter().cloned());
                    }
                } else {
                    collect_named_arrays(child, name, output);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_named_arrays(child, name, output);
            }
        }
        _ => {}
    }
}

pub fn require_empty_output(output: &Path) -> Result<(), Failure> {
    if output.exists() && !output.is_dir() {
        return Err(Failure::usage(format!(
            "output path '{}' exists and is not a directory",
            output.display()
        )));
    }
    if output.is_dir()
        && fs::read_dir(output)
            .map_err(|error| Failure::usage(error.to_string()))?
            .next()
            .is_some()
    {
        return Err(Failure::usage(format!(
            "output directory '{}' is not empty",
            output.display()
        )));
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn catalogs_positive_malformed_and_unknown_resources_deterministically() {
        let root = temp_root("catalog");
        let directory = root.join("data/12/34/56");
        fs::create_dir_all(&directory).unwrap();
        let fixture = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        fs::write(directory.join("78.DAT"), fixture).unwrap();
        fs::write(directory.join("79.DAT"), b"SEDB").unwrap();
        fs::write(directory.join("7A.DAT"), b"unknown").unwrap();

        let first = scan(&root).unwrap();
        let second = scan(&root).unwrap();
        assert_eq!(
            to_canonical_json(&json!(first)),
            to_canonical_json(&json!(second))
        );
        assert_eq!(second[0]["formatStatus"], "parsed");
        assert_eq!(second[0]["supportStatus"], "partial");
        assert_eq!(second[0]["resourceId"], "0x12345678");
        assert!(!second[0]["spans"].as_array().unwrap().is_empty());
        assert_eq!(second[1]["formatStatus"], "malformed");
        assert_eq!(second[2]["detectedFormat"], "unknown");

        fs::remove_dir_all(root).unwrap();
    }
}
