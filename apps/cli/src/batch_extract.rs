//! Catalog-driven bounded extraction of an explicit resource selection.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::{parse_dat_path, parse_resource_id, to_canonical_json};

use crate::resource_export::{plan_bytes, DocumentFormat, PlannedExtraction};
use crate::scan::require_empty_output;
use crate::{read_capped, Failure};

const DEFAULT_MAX_RESOURCES: usize = 32;
const DEFAULT_MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 128 * 1024 * 1024;
const BATCH_SCHEMA_VERSION: u64 = 1;

#[derive(Debug)]
pub struct BatchSummary {
    pub resources: usize,
    pub source_bytes: u64,
    pub output_bytes: u64,
    pub output: String,
}

enum Selection {
    Id(String),
    Path(String),
}

pub(crate) struct CatalogEntry {
    pub(crate) index: usize,
    pub(crate) source_path: String,
    pub(crate) resource_id: Option<String>,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) detected_format: String,
    pub(crate) format_status: String,
}

struct PlannedResource {
    entry: CatalogEntry,
    directory: String,
    plan: PlannedExtraction,
}

pub fn run(arguments: &[String]) -> Result<BatchSummary, Failure> {
    let Some(catalog_path) = arguments.first() else {
        return Err(Failure::usage(usage()));
    };
    let mut root = None;
    let mut output = None;
    let mut selections = Vec::new();
    let mut max_resources = DEFAULT_MAX_RESOURCES;
    let mut max_source_bytes = DEFAULT_MAX_SOURCE_BYTES;
    let mut max_output_bytes = DEFAULT_MAX_OUTPUT_BYTES;
    let mut format = DocumentFormat::Yaml;
    let mut materialize_payloads = false;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--root" if index + 1 < arguments.len() => {
                set_once(&mut root, &arguments[index + 1], "--root")?;
                index += 2;
            }
            "--output" if index + 1 < arguments.len() => {
                set_once(&mut output, &arguments[index + 1], "--output")?;
                index += 2;
            }
            "--id" if index + 1 < arguments.len() => {
                selections.push(Selection::Id(arguments[index + 1].clone()));
                index += 2;
            }
            "--path" if index + 1 < arguments.len() => {
                selections.push(Selection::Path(arguments[index + 1].clone()));
                index += 2;
            }
            "--max-resources" if index + 1 < arguments.len() => {
                max_resources = parse_positive_usize(&arguments[index + 1], "--max-resources")?;
                index += 2;
            }
            "--max-source-bytes" if index + 1 < arguments.len() => {
                max_source_bytes = parse_positive_u64(&arguments[index + 1], "--max-source-bytes")?;
                index += 2;
            }
            "--max-output-bytes" if index + 1 < arguments.len() => {
                max_output_bytes = parse_positive_u64(&arguments[index + 1], "--max-output-bytes")?;
                index += 2;
            }
            "--format" if index + 1 < arguments.len() => {
                format = match arguments[index + 1].as_str() {
                    "yaml" => DocumentFormat::Yaml,
                    "json" => DocumentFormat::Json,
                    other => {
                        return Err(batch_error(format!(
                            "invalid-output-format: expected yaml or json, found '{other}'"
                        )))
                    }
                };
                index += 2;
            }
            "--materialize-payloads" => {
                if materialize_payloads {
                    return Err(batch_error("duplicate-option: --materialize-payloads"));
                }
                materialize_payloads = true;
                index += 1;
            }
            option => return Err(batch_error(format!("unknown-option: '{option}'"))),
        }
    }

    let root = root.ok_or_else(|| batch_error("missing-option: --root <directory>"))?;
    let output = output.ok_or_else(|| batch_error("missing-option: --output <directory>"))?;
    if selections.is_empty() {
        return Err(batch_error(
            "selection-required: supply at least one --id or --path",
        ));
    }
    let output_path = Path::new(&output);
    require_empty_output(output_path)?;
    reject_link_if_present(output_path, "output")?;
    reject_link_if_present(Path::new(catalog_path), "catalog")?;

    let catalog_bytes = read_capped(catalog_path)?;
    let entries = parse_catalog(&catalog_bytes)?;
    let selected = resolve_selections(&entries, &selections)?;
    if selected.len() > max_resources {
        return Err(batch_error(format!(
            "resource-count-limit-exceeded: selected {} resources, limit {max_resources}",
            selected.len()
        )));
    }
    let source_bytes = selected.iter().try_fold(0u64, |total, entry| {
        total.checked_add(entry.size).ok_or_else(|| {
            batch_error("source-byte-accounting-overflow: selected sizes exceed u64")
        })
    })?;
    if source_bytes > max_source_bytes {
        return Err(batch_error(format!(
            "source-byte-limit-exceeded: selected {source_bytes} bytes, limit {max_source_bytes}"
        )));
    }

    let root_path = Path::new(&root);
    let canonical_root = secure_root(root_path)?;
    let mut planned = Vec::new();
    let mut resource_output_bytes = 0u64;
    for (ordinal, entry) in selected.into_iter().enumerate() {
        if entry.format_status != "parsed" || entry.detected_format == "unknown" {
            return Err(batch_error(format!(
                "selected-resource-not-parsed: '{}' has format status '{}' and detected format '{}'",
                entry.source_path, entry.format_status, entry.detected_format
            )));
        }
        let source = secure_source(root_path, &canonical_root, &entry.source_path)?;
        let metadata = fs::metadata(&source).map_err(|error| {
            batch_error(format!(
                "source-metadata-failed: '{}': {error}",
                entry.source_path
            ))
        })?;
        if metadata.len() != entry.size {
            return Err(batch_error(format!(
                "stale-source-size: '{}' catalog {}, current {}",
                entry.source_path,
                entry.size,
                metadata.len()
            )));
        }
        let data = read_capped(&source.display().to_string())?;
        let digest = sha256_hex(&data);
        if digest != entry.sha256 {
            return Err(batch_error(format!(
                "stale-source-sha256: '{}' catalog {}, current {digest}",
                entry.source_path, entry.sha256
            )));
        }
        let materialize = materialize_payloads
            && matches!(entry.detected_format.as_str(), "sedb" | "res" | "gtex");
        let plan = plan_bytes(
            &source.display().to_string(),
            &data,
            format,
            materialize,
            &[],
        )?;
        if plan.format_id() != entry.detected_format {
            return Err(batch_error(format!(
                "stale-detected-format: '{}' catalog '{}', current '{}'",
                entry.source_path,
                entry.detected_format,
                plan.format_id()
            )));
        }
        resource_output_bytes = resource_output_bytes
            .checked_add(plan.output_bytes()?)
            .ok_or_else(|| batch_error("output-byte-accounting-overflow: outputs exceed u64"))?;
        if resource_output_bytes > max_output_bytes {
            return Err(batch_error(format!(
                "output-byte-limit-exceeded: planned resource outputs already total {resource_output_bytes} bytes, limit {max_output_bytes}"
            )));
        }
        let identity = entry
            .resource_id
            .as_deref()
            .map(|value| format!("id-{}", &value[2..]))
            .unwrap_or_else(|| "path".to_string());
        let directory = format!("{:06}-{identity}-{}", ordinal + 1, &entry.sha256[..16]);
        planned.push(PlannedResource {
            entry: clone_entry(entry),
            directory,
            plan,
        });
    }

    let (batch_name, batch_document, output_bytes) = render_batch(
        format,
        catalog_path,
        &catalog_bytes,
        &planned,
        source_bytes,
        resource_output_bytes,
        max_resources,
        max_source_bytes,
        max_output_bytes,
        materialize_payloads,
    )?;
    if output_bytes > max_output_bytes {
        return Err(batch_error(format!(
            "output-byte-limit-exceeded: planned {output_bytes} bytes, limit {max_output_bytes}"
        )));
    }

    write_batch_atomically(output_path, batch_name, &batch_document, &planned)?;
    let batch_path = output_path.join(batch_name);
    Ok(BatchSummary {
        resources: planned.len(),
        source_bytes,
        output_bytes,
        output: batch_path.display().to_string(),
    })
}

fn write_batch_atomically(
    output: &Path,
    batch_name: &str,
    batch_document: &str,
    planned: &[PlannedResource],
) -> Result<(), Failure> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        batch_error(format!(
            "output-parent-create-failed: '{}': {error}",
            parent.display()
        ))
    })?;
    let leaf = output
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| batch_error("invalid-output-path: output has no UTF-8 final component"))?;
    let staging = parent.join(format!(".{leaf}.xivl-staging-{}", std::process::id()));
    if staging.exists() {
        return Err(batch_error(format!(
            "staging-path-exists: '{}'",
            staging.display()
        )));
    }
    fs::create_dir(&staging).map_err(|error| {
        batch_error(format!(
            "staging-directory-create-failed: '{}': {error}",
            staging.display()
        ))
    })?;

    let write_result = (|| {
        for item in planned {
            item.plan.write_to(&staging.join(&item.directory))?;
        }
        let batch_path = staging.join(batch_name);
        fs::write(&batch_path, batch_document).map_err(|error| {
            batch_error(format!(
                "batch-manifest-write-failed: '{}': {error}",
                batch_path.display()
            ))
        })
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let output_existed = output.exists();
    if output_existed {
        if let Err(error) = require_empty_output(output) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        fs::remove_dir(output).map_err(|error| {
            let _ = fs::remove_dir_all(&staging);
            batch_error(format!(
                "empty-output-replacement-failed: '{}': {error}",
                output.display()
            ))
        })?;
    }
    if let Err(error) = fs::rename(&staging, output) {
        let _ = fs::remove_dir_all(&staging);
        if output_existed {
            let _ = fs::create_dir(output);
        }
        return Err(batch_error(format!(
            "batch-publish-failed: '{}' -> '{}': {error}",
            staging.display(),
            output.display()
        )));
    }
    Ok(())
}

fn usage() -> &'static str {
    "usage: xivl extract-catalog <catalog.json|catalog.jsonl> --root <directory> --output <directory> (--id <resource-id> | --path <catalog-path>)+ [--max-resources <count>] [--max-source-bytes <bytes>] [--max-output-bytes <bytes>] [--format yaml|json] [--materialize-payloads]"
}

fn batch_error(message: impl Into<String>) -> Failure {
    Failure::usage(message)
}

fn set_once(target: &mut Option<String>, value: &str, option: &str) -> Result<(), Failure> {
    if target.replace(value.to_string()).is_some() {
        return Err(batch_error(format!("duplicate-option: {option}")));
    }
    Ok(())
}

fn parse_positive_usize(text: &str, option: &str) -> Result<usize, Failure> {
    let value = text
        .parse::<usize>()
        .map_err(|_| batch_error(format!("invalid-limit: {option} needs a positive integer")))?;
    if value == 0 {
        return Err(batch_error(format!(
            "invalid-limit: {option} must be greater than zero"
        )));
    }
    Ok(value)
}

fn parse_positive_u64(text: &str, option: &str) -> Result<u64, Failure> {
    let value = text
        .parse::<u64>()
        .map_err(|_| batch_error(format!("invalid-limit: {option} needs a positive integer")))?;
    if value == 0 {
        return Err(batch_error(format!(
            "invalid-limit: {option} must be greater than zero"
        )));
    }
    Ok(value)
}

pub(crate) fn parse_catalog(data: &[u8]) -> Result<Vec<CatalogEntry>, Failure> {
    let value = serde_json::from_slice::<Value>(data);
    let resources = match value {
        Ok(Value::Object(object)) if object.contains_key("resources") => {
            if object.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
                return Err(batch_error(
                    "unsupported-catalog-schema: expected version 1",
                ));
            }
            let values = object
                .get("resources")
                .and_then(Value::as_array)
                .ok_or_else(|| batch_error("invalid-catalog: resources is not an array"))?;
            if object.get("resourceCount").and_then(Value::as_u64) != Some(values.len() as u64) {
                return Err(batch_error(
                    "invalid-catalog: resourceCount disagrees with resources length",
                ));
            }
            values.clone()
        }
        Ok(_) | Err(_) => {
            let text = std::str::from_utf8(data)
                .map_err(|_| batch_error("invalid-catalog: input is not UTF-8 JSON"))?;
            let mut values = Vec::new();
            for (line_index, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let value: Value = serde_json::from_str(line).map_err(|error| {
                    batch_error(format!(
                        "invalid-catalog-jsonl: line {}: {error}",
                        line_index + 1
                    ))
                })?;
                values.push(value);
            }
            values
        }
    };
    if resources.is_empty() {
        return Err(batch_error("invalid-catalog: catalog has no resources"));
    }

    let mut entries = Vec::new();
    let mut exact_paths = BTreeSet::new();
    let mut folded_paths = BTreeMap::new();
    let mut ids = BTreeMap::new();
    for (index, value) in resources.into_iter().enumerate() {
        let object = value
            .as_object()
            .ok_or_else(|| batch_error(format!("invalid-catalog-entry: index {index}")))?;
        if object.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
            return Err(batch_error(format!(
                "unsupported-catalog-entry-schema: index {index}"
            )));
        }
        let raw_path = required_string(object, "sourcePath", index)?;
        let source_path = normalize_relative_path(raw_path)?;
        if !exact_paths.insert(source_path.clone()) {
            return Err(batch_error(format!(
                "duplicate-catalog-entry: source path '{source_path}'"
            )));
        }
        let folded = source_path.to_ascii_lowercase();
        if let Some(previous) = folded_paths.insert(folded, source_path.clone()) {
            return Err(batch_error(format!(
                "ambiguous-catalog-path: '{previous}' and '{source_path}'"
            )));
        }
        let resource_id = match object.get("resourceId") {
            Some(Value::String(value)) => {
                let id = parse_resource_id(value, 0).map_err(|error| {
                    batch_error(format!(
                        "invalid-catalog-resource-id: index {index}: {error}"
                    ))
                })?;
                let canonical = id.to_hex();
                if let Some(previous) = ids.insert(canonical.clone(), source_path.clone()) {
                    return Err(batch_error(format!(
                        "ambiguous-resource-id: {canonical} names '{previous}' and '{source_path}'"
                    )));
                }
                Some(canonical)
            }
            Some(Value::Null) => None,
            _ => {
                return Err(batch_error(format!(
                    "invalid-catalog-entry: index {index} resourceId"
                )))
            }
        };
        if let Ok(path_id) = parse_dat_path(&source_path, 0) {
            if resource_id.as_deref() != Some(path_id.to_hex().as_str()) {
                return Err(batch_error(format!(
                    "catalog-identity-mismatch: '{source_path}' does not agree with resourceId"
                )));
            }
        }
        let size = object
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| batch_error(format!("invalid-catalog-entry: index {index} size")))?;
        let sha256 = required_string(object, "sha256", index)?.to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(batch_error(format!(
                "invalid-catalog-entry: index {index} sha256"
            )));
        }
        entries.push(CatalogEntry {
            index,
            source_path,
            resource_id,
            size,
            sha256,
            detected_format: required_string(object, "detectedFormat", index)?.to_string(),
            format_status: required_string(object, "formatStatus", index)?.to_string(),
        });
    }
    Ok(entries)
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    index: usize,
) -> Result<&'a str, Failure> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| batch_error(format!("invalid-catalog-entry: index {index} {key}")))
}

pub(crate) fn normalize_relative_path(text: &str) -> Result<String, Failure> {
    let normalized = text.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.starts_with("//")
        || normalized.as_bytes().get(1) == Some(&b':')
        || normalized.contains('\0')
    {
        return Err(batch_error(format!("unsafe-catalog-path: '{text}'")));
    }
    let components: Vec<&str> = normalized.split('/').collect();
    if components.iter().any(|component| {
        component.is_empty() || *component == "." || *component == ".." || component.contains(':')
    }) {
        return Err(batch_error(format!("unsafe-catalog-path: '{text}'")));
    }
    Ok(components.join("/"))
}

fn resolve_selections<'a>(
    entries: &'a [CatalogEntry],
    selections: &[Selection],
) -> Result<Vec<&'a CatalogEntry>, Failure> {
    let by_id: BTreeMap<&str, &CatalogEntry> = entries
        .iter()
        .filter_map(|entry| entry.resource_id.as_deref().map(|id| (id, entry)))
        .collect();
    let by_path: BTreeMap<String, &CatalogEntry> = entries
        .iter()
        .map(|entry| (entry.source_path.to_ascii_lowercase(), entry))
        .collect();
    let mut selected = BTreeMap::new();
    for selection in selections {
        let entry = match selection {
            Selection::Id(text) => {
                let id = parse_resource_id(text, 0).map_err(|error| {
                    batch_error(format!("invalid-selection-id: '{text}': {error}"))
                })?;
                let canonical = id.to_hex();
                by_id.get(canonical.as_str()).copied().ok_or_else(|| {
                    batch_error(format!("selection-not-found: resource id {canonical}"))
                })?
            }
            Selection::Path(text) => {
                let path = normalize_relative_path(text)?;
                by_path
                    .get(&path.to_ascii_lowercase())
                    .copied()
                    .ok_or_else(|| batch_error(format!("selection-not-found: path '{path}'")))?
            }
        };
        if selected.insert(entry.index, entry).is_some() {
            return Err(batch_error(format!(
                "duplicate-selection: '{}' was selected more than once",
                entry.source_path
            )));
        }
    }
    Ok(selected.into_values().collect())
}

pub(crate) fn secure_root(root: &Path) -> Result<PathBuf, Failure> {
    reject_link_if_present(root, "catalog root")?;
    if !root.is_dir() {
        return Err(batch_error(format!(
            "invalid-catalog-root: '{}' is not a directory",
            root.display()
        )));
    }
    fs::canonicalize(root).map_err(|error| {
        batch_error(format!(
            "catalog-root-resolution-failed: '{}': {error}",
            root.display()
        ))
    })
}

pub(crate) fn secure_source(
    root: &Path,
    canonical_root: &Path,
    relative: &str,
) -> Result<PathBuf, Failure> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
        reject_link_if_present(&path, "source path")?;
    }
    let canonical = fs::canonicalize(&path)
        .map_err(|error| batch_error(format!("source-resolution-failed: '{relative}': {error}")))?;
    if !canonical.starts_with(canonical_root) {
        return Err(batch_error(format!(
            "source-outside-catalog-root: '{relative}'"
        )));
    }
    if !canonical.is_file() {
        return Err(batch_error(format!(
            "source-not-regular-file: '{relative}'"
        )));
    }
    Ok(canonical)
}

pub(crate) fn reject_link_if_present(path: &Path, role: &str) -> Result<(), Failure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(batch_error(format!(
                "path-metadata-failed: {role} '{}': {error}",
                path.display()
            )))
        }
    };
    if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
        return Err(batch_error(format!(
            "link-or-reparse-point-refused: {role} '{}'",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[allow(clippy::too_many_arguments)]
fn render_batch(
    format: DocumentFormat,
    catalog_path: &str,
    catalog_bytes: &[u8],
    planned: &[PlannedResource],
    source_bytes: u64,
    resource_output_bytes: u64,
    max_resources: usize,
    max_source_bytes: u64,
    max_output_bytes: u64,
    materialize_payloads: bool,
) -> Result<(&'static str, String, u64), Failure> {
    let resources: Vec<Value> = planned
        .iter()
        .enumerate()
        .map(|(ordinal, item)| {
            json!({
                "catalogIndex": item.entry.index as u64,
                "detectedFormat": item.entry.detected_format,
                "manifest": format!("{}/{}", item.directory, item.plan.document_name()),
                "ordinal": ordinal as u64 + 1,
                "outputBytes": item.plan.output_bytes().expect("already accounted"),
                "outputDirectory": item.directory,
                "resourceId": item.entry.resource_id,
                "sourcePath": item.entry.source_path,
                "sourceSha256": item.entry.sha256,
                "sourceSize": item.entry.size,
            })
        })
        .collect();
    let name = match format {
        DocumentFormat::Yaml => "batch.yaml",
        DocumentFormat::Json => "batch.json",
    };
    let mut aggregate_output_bytes = resource_output_bytes;
    for _ in 0..8 {
        let document = json!({
            "catalog": {
                "fileName": crate::base_name(catalog_path),
                "sha256": sha256_hex(catalog_bytes),
            },
            "limits": {
                "maxOutputBytes": max_output_bytes,
                "maxResources": max_resources as u64,
                "maxSourceBytes": max_source_bytes,
            },
            "materializePayloads": materialize_payloads,
            "resources": resources,
            "schemaVersion": BATCH_SCHEMA_VERSION,
            "tool": {
                "commit": option_env!("XIVL_GIT_COMMIT"),
                "name": "xivl",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "totals": {
                "outputBytes": aggregate_output_bytes,
                "resourceCount": planned.len() as u64,
                "sourceBytes": source_bytes,
            },
        });
        let text = match format {
            DocumentFormat::Yaml => serde_yaml::to_string(&document)
                .map_err(|error| batch_error(format!("batch-yaml-render-failed: {error}")))?,
            DocumentFormat::Json => to_canonical_json(&document),
        };
        let total = resource_output_bytes
            .checked_add(text.len() as u64)
            .ok_or_else(|| batch_error("output-byte-accounting-overflow: batch manifest"))?;
        if total == aggregate_output_bytes {
            return Ok((name, text, total));
        }
        aggregate_output_bytes = total;
    }
    Err(batch_error(
        "output-byte-accounting-unstable: batch manifest did not converge",
    ))
}

fn clone_entry(entry: &CatalogEntry) -> CatalogEntry {
    CatalogEntry {
        index: entry.index,
        source_path: entry.source_path.clone(),
        resource_id: entry.resource_id.clone(),
        size: entry.size,
        sha256: entry.sha256.clone(),
        detected_format: entry.detected_format.clone(),
        format_status: entry.format_status.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "xivl-batch-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn row(path: &str, id: Option<&str>, bytes: &[u8], format: &str, status: &str) -> Value {
        json!({
            "anomalies": [],
            "detectedFormat": format,
            "formatStatus": status,
            "resourceId": id,
            "schemaVersion": 1,
            "sha256": sha256_hex(bytes),
            "size": bytes.len() as u64,
            "sourcePath": path,
            "spans": [],
            "supportStatus": if status == "parsed" { "partial" } else { "none" },
        })
    }

    fn write_catalog(path: &Path, rows: Vec<Value>) {
        let document = json!({
            "resourceCount": rows.len() as u64,
            "resources": rows,
            "schemaVersion": 1,
        });
        fs::write(path, to_canonical_json(&document)).unwrap();
    }

    fn base_arguments(catalog: &Path, root: &Path, output: &Path) -> Vec<String> {
        vec![
            catalog.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--output".into(),
            output.display().to_string(),
        ]
    }

    fn create_resource(root: &Path, relative: &str, bytes: &[u8]) {
        let path = relative
            .split('/')
            .fold(root.to_path_buf(), |path, component| path.join(component));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn explicit_id_and_path_selection_is_isolated_collision_safe_and_deterministic() {
        let work = temp_root("positive");
        let root = work.join("install");
        fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        create_resource(&root, "data/12/34/56/78.DAT", bytes);
        create_resource(&root, "data/12/34/56/79.DAT", bytes);
        let catalog = work.join("catalog.json");
        write_catalog(
            &catalog,
            vec![
                row(
                    "data/12/34/56/78.DAT",
                    Some("0x12345678"),
                    bytes,
                    "sedb",
                    "parsed",
                ),
                row(
                    "data/12/34/56/79.DAT",
                    Some("0x12345679"),
                    bytes,
                    "sedb",
                    "parsed",
                ),
            ],
        );
        let first = work.join("first");
        let mut arguments = base_arguments(&catalog, &root, &first);
        arguments.extend([
            "--path".into(),
            "data\\12\\34\\56\\79.DAT".into(),
            "--id".into(),
            "0x12345678".into(),
            "--materialize-payloads".into(),
        ]);
        let summary = run(&arguments).unwrap();
        assert_eq!(summary.resources, 2);
        let batch: Value =
            serde_yaml::from_str(&fs::read_to_string(&summary.output).unwrap()).unwrap();
        let resources = batch["resources"].as_array().unwrap();
        assert_eq!(resources[0]["sourcePath"], "data/12/34/56/78.DAT");
        assert_eq!(resources[1]["sourcePath"], "data/12/34/56/79.DAT");
        assert_ne!(
            resources[0]["outputDirectory"],
            resources[1]["outputDirectory"]
        );
        for resource in resources {
            let directory = first.join(resource["outputDirectory"].as_str().unwrap());
            assert!(directory.join("extraction.yaml").is_file());
            assert!(directory.join("payloads").is_dir());
        }

        let second = work.join("second");
        fs::create_dir(&second).unwrap();
        arguments[4] = second.display().to_string();
        let second_summary = run(&arguments).unwrap();
        assert_eq!(
            fs::read(&summary.output).unwrap(),
            fs::read(&second_summary.output).unwrap()
        );

        let jsonl = work.join("catalog.jsonl");
        let jsonl_row = row(
            "data/12/34/56/78.DAT",
            Some("0x12345678"),
            bytes,
            "sedb",
            "parsed",
        );
        fs::write(
            &jsonl,
            format!("{}\n", serde_json::to_string(&jsonl_row).unwrap()),
        )
        .unwrap();
        let jsonl_output = work.join("jsonl");
        let mut jsonl_arguments = base_arguments(&jsonl, &root, &jsonl_output);
        jsonl_arguments.extend(["--id".into(), "0x12345678".into()]);
        assert_eq!(run(&jsonl_arguments).unwrap().resources, 1);
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn selection_and_catalog_ambiguities_are_refused() {
        let work = temp_root("selection-errors");
        let root = work.join("root");
        fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        create_resource(&root, "data/12/34/56/78.DAT", bytes);
        let catalog = work.join("catalog.json");
        write_catalog(
            &catalog,
            vec![row(
                "data/12/34/56/78.DAT",
                Some("0x12345678"),
                bytes,
                "sedb",
                "parsed",
            )],
        );

        let output = work.join("out");
        let mut none = base_arguments(&catalog, &root, &output);
        assert!(run(&none)
            .unwrap_err()
            .message
            .contains("selection-required"));
        none.extend(["--id".into(), "0x12345670".into()]);
        assert!(run(&none)
            .unwrap_err()
            .message
            .contains("selection-not-found"));
        let mut duplicate = base_arguments(&catalog, &root, &output);
        duplicate.extend([
            "--id".into(),
            "0x12345678".into(),
            "--path".into(),
            "data/12/34/56/78.DAT".into(),
        ]);
        assert!(run(&duplicate)
            .unwrap_err()
            .message
            .contains("duplicate-selection"));

        write_catalog(
            &catalog,
            vec![
                row(
                    "data/12/34/56/78.DAT",
                    Some("0x12345678"),
                    bytes,
                    "sedb",
                    "parsed",
                ),
                row(
                    "copy/12/34/56/78.DAT",
                    Some("0x12345678"),
                    bytes,
                    "sedb",
                    "parsed",
                ),
            ],
        );
        assert!(run(&duplicate)
            .unwrap_err()
            .message
            .contains("ambiguous-resource-id"));
        write_catalog(
            &catalog,
            vec![
                row("custom/file.DAT", None, bytes, "sedb", "parsed"),
                row("CUSTOM/FILE.dat", None, bytes, "sedb", "parsed"),
            ],
        );
        assert!(run(&duplicate)
            .unwrap_err()
            .message
            .contains("ambiguous-catalog-path"));
        write_catalog(
            &catalog,
            vec![
                row("custom/file.DAT", None, bytes, "sedb", "parsed"),
                row("custom/file.DAT", None, bytes, "sedb", "parsed"),
            ],
        );
        assert!(run(&duplicate)
            .unwrap_err()
            .message
            .contains("duplicate-catalog-entry"));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn count_source_and_output_limits_refuse_before_output() {
        let work = temp_root("limits");
        let root = work.join("root");
        fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        create_resource(&root, "data/12/34/56/78.DAT", bytes);
        create_resource(&root, "data/12/34/56/79.DAT", bytes);
        let catalog = work.join("catalog.json");
        write_catalog(
            &catalog,
            vec![
                row(
                    "data/12/34/56/78.DAT",
                    Some("0x12345678"),
                    bytes,
                    "sedb",
                    "parsed",
                ),
                row(
                    "data/12/34/56/79.DAT",
                    Some("0x12345679"),
                    bytes,
                    "sedb",
                    "parsed",
                ),
            ],
        );
        for (option, value, expected) in [
            ("--max-resources", "1", "resource-count-limit-exceeded"),
            ("--max-source-bytes", "1", "source-byte-limit-exceeded"),
            ("--max-output-bytes", "1", "output-byte-limit-exceeded"),
        ] {
            let output = work.join(format!("out-{option}"));
            let mut arguments = base_arguments(&catalog, &root, &output);
            arguments.extend([
                "--id".into(),
                "0x12345678".into(),
                "--id".into(),
                "0x12345679".into(),
                option.into(),
                value.into(),
            ]);
            let error = run(&arguments).unwrap_err();
            assert!(error.message.contains(expected), "{}", error.message);
            assert!(!output.exists());
        }

        let mut first = row(
            "data/12/34/56/78.DAT",
            Some("0x12345678"),
            bytes,
            "sedb",
            "parsed",
        );
        let mut second = row(
            "data/12/34/56/79.DAT",
            Some("0x12345679"),
            bytes,
            "sedb",
            "parsed",
        );
        first["size"] = json!(u64::MAX);
        second["size"] = json!(1);
        write_catalog(&catalog, vec![first, second]);
        let overflow_output = work.join("out-overflow");
        let mut overflow = base_arguments(&catalog, &root, &overflow_output);
        overflow.extend([
            "--id".into(),
            "0x12345678".into(),
            "--id".into(),
            "0x12345679".into(),
        ]);
        assert!(run(&overflow)
            .unwrap_err()
            .message
            .contains("source-byte-accounting-overflow"));
        assert!(!overflow_output.exists());
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn stale_size_hash_and_detected_format_are_refused_before_output() {
        let work = temp_root("stale");
        let root = work.join("root");
        fs::create_dir_all(&root).unwrap();
        let original = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        let relative = "data/12/34/56/78.DAT";
        create_resource(&root, relative, original);
        let catalog = work.join("catalog.json");
        let output = work.join("out");

        let mut wrong_size = row(relative, Some("0x12345678"), original, "sedb", "parsed");
        wrong_size["size"] = json!(original.len() as u64 + 1);
        write_catalog(&catalog, vec![wrong_size]);
        let mut arguments = base_arguments(&catalog, &root, &output);
        arguments.extend(["--id".into(), "0x12345678".into()]);
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("stale-source-size"));

        write_catalog(
            &catalog,
            vec![row(
                relative,
                Some("0x12345678"),
                original,
                "sedb",
                "parsed",
            )],
        );
        let mut changed = original.to_vec();
        *changed.last_mut().unwrap() ^= 1;
        create_resource(&root, relative, &changed);
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("stale-source-sha256"));

        create_resource(&root, relative, original);
        write_catalog(
            &catalog,
            vec![row(relative, Some("0x12345678"), original, "res", "parsed")],
        );
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("stale-detected-format"));
        assert!(!output.exists());
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn traversal_unsupported_and_late_malformed_selection_leave_no_output() {
        let work = temp_root("boundaries");
        let root = work.join("root");
        fs::create_dir_all(&root).unwrap();
        let good = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        create_resource(&root, "data/12/34/56/78.DAT", good);
        create_resource(&root, "data/12/34/56/79.DAT", b"unknown");
        create_resource(&root, "data/12/34/56/7A.DAT", b"SEDB");
        let catalog = work.join("catalog.json");
        let output = work.join("out");

        write_catalog(
            &catalog,
            vec![row("../outside.DAT", None, good, "sedb", "parsed")],
        );
        let mut arguments = base_arguments(&catalog, &root, &output);
        arguments.extend(["--path".into(), "../outside.DAT".into()]);
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("unsafe-catalog-path"));

        write_catalog(
            &catalog,
            vec![
                row(
                    "data/12/34/56/78.DAT",
                    Some("0x12345678"),
                    good,
                    "sedb",
                    "parsed",
                ),
                row(
                    "data/12/34/56/79.DAT",
                    Some("0x12345679"),
                    b"unknown",
                    "unknown",
                    "unknown",
                ),
            ],
        );
        arguments = base_arguments(&catalog, &root, &output);
        arguments.extend([
            "--id".into(),
            "0x12345678".into(),
            "--id".into(),
            "0x12345679".into(),
        ]);
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("selected-resource-not-parsed"));

        write_catalog(
            &catalog,
            vec![
                row(
                    "data/12/34/56/78.DAT",
                    Some("0x12345678"),
                    good,
                    "sedb",
                    "parsed",
                ),
                row(
                    "data/12/34/56/7A.DAT",
                    Some("0x1234567A"),
                    b"SEDB",
                    "sedb",
                    "parsed",
                ),
            ],
        );
        arguments = base_arguments(&catalog, &root, &output);
        arguments.extend([
            "--id".into(),
            "0x12345678".into(),
            "--id".into(),
            "0x1234567A".into(),
        ]);
        assert_eq!(run(&arguments).unwrap_err().code, crate::EXIT_PARSE_FAILURE);
        assert!(!output.exists());

        write_catalog(
            &catalog,
            vec![row(
                "data/12/34/56/78.DAT",
                Some("0x12345678"),
                good,
                "sedb",
                "parsed",
            )],
        );
        arguments = base_arguments(&catalog, &root, &output);
        arguments.extend(["--id".into(), "0x12345678".into()]);
        let staging = work.join(format!(".out.xivl-staging-{}", std::process::id()));
        fs::create_dir(&staging).unwrap();
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("staging-path-exists"));
        assert!(!output.exists());
        fs::remove_dir_all(work).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn source_symlink_is_refused() {
        use std::os::unix::fs::symlink;

        let work = temp_root("symlink");
        let root = work.join("root");
        fs::create_dir_all(root.join("data/12/34/56")).unwrap();
        let bytes = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        let outside = work.join("outside.DAT");
        fs::write(&outside, bytes).unwrap();
        symlink(&outside, root.join("data/12/34/56/78.DAT")).unwrap();
        let catalog = work.join("catalog.json");
        write_catalog(
            &catalog,
            vec![row(
                "data/12/34/56/78.DAT",
                Some("0x12345678"),
                bytes,
                "sedb",
                "parsed",
            )],
        );
        let output = work.join("out");
        let mut arguments = base_arguments(&catalog, &root, &output);
        arguments.extend(["--id".into(), "0x12345678".into()]);
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("link-or-reparse-point-refused"));
        assert!(!output.exists());
        fs::remove_dir_all(work).unwrap();
    }
}
