//! Read-only verification of single-resource and catalog extraction outputs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::{Draft, JSONSchema};
use same_file::Handle;
use serde_json::{json, Value};
use xivl_formats::dds::{self, DdsPixelFormat};
use xivl_formats::digest::sha256_hex;
use xivl_formats::gtex_pwib;
use xivl_formats::texture_preview;
use xivl_formats::{extract_lpb, parse_dat_path};

use crate::batch_extract::{
    normalize_relative_path, parse_catalog, reject_link_if_present, secure_root, secure_source,
    CatalogEntry,
};
use crate::resource_export::{plan_bytes, plan_bytes_options, DocumentFormat};
use crate::{read_capped, Failure};

const SINGLE_SCHEMA: &str = include_str!("../../../schemas/resource-extraction.schema.json");
const BATCH_SCHEMA: &str = include_str!("../../../schemas/catalog-extraction.schema.json");

#[derive(Debug)]
pub struct VerifySummary {
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReportFormat {
    Text,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManifestKind {
    Single,
    Batch,
}

struct Options {
    directory: PathBuf,
    source: Option<PathBuf>,
    catalog: Option<PathBuf>,
    root: Option<PathBuf>,
    report: ReportFormat,
}

struct Inventory {
    files: BTreeMap<String, FileRecord>,
    directories: BTreeSet<String>,
}

struct FileRecord {
    path: PathBuf,
    size: u64,
    sha256: String,
}

struct SingleResult {
    format: String,
    source_size: u64,
    payloads: usize,
    output_bytes: u64,
    expected_files: BTreeSet<String>,
    expected_directories: BTreeSet<String>,
    document: Value,
}

pub fn run(arguments: &[String]) -> Result<VerifySummary, Failure> {
    let options = parse_options(arguments)?;
    reject_link_if_present(&options.directory, "extraction directory")?;
    if !options.directory.is_dir() {
        return Err(fail(
            "invalid-extraction-directory",
            format!("'{}' is not a directory", options.directory.display()),
        ));
    }
    let inventory = inventory(&options.directory)?;
    let (kind, manifest) = detect_manifest(&inventory)?;
    let result = match kind {
        ManifestKind::Single => {
            if options.catalog.is_some() || options.root.is_some() {
                return Err(fail(
                    "incompatible-option",
                    "--catalog and --root apply only to batch extractions",
                ));
            }
            let source = options.source.as_deref().map(read_source).transpose()?;
            let verified = verify_single(&inventory, &manifest, "", source.as_ref(), true)?;
            json!({
                "kind": "resource",
                "format": verified.format,
                "payloads": verified.payloads,
                "sourceBytes": verified.source_size,
                "outputBytes": verified.output_bytes,
                "sourceReplay": source.is_some(),
                "status": "verified"
            })
        }
        ManifestKind::Batch => {
            if options.source.is_some() {
                return Err(fail(
                    "incompatible-option",
                    "--source applies only to single-resource extractions",
                ));
            }
            if options.catalog.is_some() != options.root.is_some() {
                return Err(fail(
                    "incomplete-replay-options",
                    "--catalog and --root must be supplied together",
                ));
            }
            verify_batch(&options, &inventory, &manifest)?
        }
    };
    let text = if options.report == ReportFormat::Json {
        serde_json::to_string(&result)
            .map_err(|error| fail("report-serialization-failed", error.to_string()))?
    } else if result["kind"] == "resource" {
        format!(
            "verified resource extraction: format {}, {} payloads, {} source bytes, {} output bytes{}",
            string(&result, "format")?,
            integer(&result, "payloads")?,
            integer(&result, "sourceBytes")?,
            integer(&result, "outputBytes")?,
            if result["sourceReplay"] == Value::Bool(true) {
                ", source replayed"
            } else {
                ""
            }
        )
    } else {
        format!(
            "verified catalog extraction: {} resources, {} payloads, {} source bytes, {} output bytes{}",
            integer(&result, "resources")?,
            integer(&result, "payloads")?,
            integer(&result, "sourceBytes")?,
            integer(&result, "outputBytes")?,
            if result["sourceReplay"] == Value::Bool(true) {
                ", catalog sources replayed"
            } else {
                ""
            }
        )
    };
    Ok(VerifySummary { text })
}

fn parse_options(arguments: &[String]) -> Result<Options, Failure> {
    let Some(directory) = arguments.first() else {
        return Err(Failure::usage(usage()));
    };
    let mut source = None;
    let mut catalog = None;
    let mut root = None;
    let mut report = ReportFormat::Text;
    let mut index = 1;
    while index < arguments.len() {
        let target = match arguments[index].as_str() {
            "--source" => &mut source,
            "--catalog" => &mut catalog,
            "--root" => &mut root,
            "--report" if index + 1 < arguments.len() => {
                if arguments[index + 1] != "json" {
                    return Err(fail(
                        "invalid-report-format",
                        "--report accepts only 'json'",
                    ));
                }
                if report == ReportFormat::Json {
                    return Err(fail("duplicate-option", "--report"));
                }
                report = ReportFormat::Json;
                index += 2;
                continue;
            }
            option => return Err(fail("unknown-option", option)),
        };
        let Some(value) = arguments.get(index + 1) else {
            return Err(fail("missing-option-value", arguments[index].as_str()));
        };
        if target.replace(PathBuf::from(value)).is_some() {
            return Err(fail("duplicate-option", arguments[index].as_str()));
        }
        index += 2;
    }
    Ok(Options {
        directory: PathBuf::from(directory),
        source,
        catalog,
        root,
        report,
    })
}

fn inventory(root: &Path) -> Result<Inventory, Failure> {
    let mut result = Inventory {
        files: BTreeMap::new(),
        directories: BTreeSet::new(),
    };
    let mut folded = BTreeMap::new();
    let mut handles: Vec<(String, Handle)> = Vec::new();
    walk(root, root, &mut result, &mut folded, &mut handles)?;
    Ok(result)
}

fn walk(
    root: &Path,
    directory: &Path,
    result: &mut Inventory,
    folded: &mut BTreeMap<String, String>,
    handles: &mut Vec<(String, Handle)>,
) -> Result<(), Failure> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .map_err(|error| {
            fail(
                "directory-read-failed",
                format!("{}: {error}", directory.display()),
            )
        })?
        .collect::<Result<_, _>>()
        .map_err(|error| fail("directory-read-failed", error.to_string()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        reject_link_if_present(&path, "extraction member")?;
        let relative = slash_relative(root, &path)?;
        let canonical = normalize_relative_path(&relative)
            .map_err(|_| fail("unsafe-extraction-path", relative.clone()))?;
        let lower = canonical.to_ascii_lowercase();
        if let Some(previous) = folded.insert(lower, canonical.clone()) {
            return Err(fail(
                "case-collision",
                format!("'{previous}' and '{canonical}'"),
            ));
        }
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            fail(
                "member-metadata-failed",
                format!("{}: {error}", path.display()),
            )
        })?;
        if metadata.is_dir() {
            result.directories.insert(canonical);
            walk(root, &path, result, folded, handles)?;
        } else if metadata.is_file() {
            let handle = Handle::from_path(&path)
                .map_err(|error| fail("file-identity-failed", format!("{canonical}: {error}")))?;
            if let Some((previous, _)) = handles.iter().find(|(_, other)| *other == handle) {
                return Err(fail(
                    "file-alias-refused",
                    format!("'{previous}' and '{canonical}' name the same file"),
                ));
            }
            handles.push((canonical.clone(), handle));
            let bytes = fs::read(&path)
                .map_err(|error| fail("file-read-failed", format!("{canonical}: {error}")))?;
            result.files.insert(
                canonical,
                FileRecord {
                    path,
                    size: bytes.len() as u64,
                    sha256: sha256_hex(&bytes),
                },
            );
        } else {
            return Err(fail("non-regular-member", canonical));
        }
    }
    Ok(())
}

fn detect_manifest(inventory: &Inventory) -> Result<(ManifestKind, String), Failure> {
    let candidates = [
        (ManifestKind::Single, "extraction.yaml"),
        (ManifestKind::Single, "extraction.json"),
        (ManifestKind::Batch, "batch.yaml"),
        (ManifestKind::Batch, "batch.json"),
    ];
    let found: Vec<_> = candidates
        .into_iter()
        .filter(|(_, path)| inventory.files.contains_key(*path))
        .collect();
    match found.as_slice() {
        [(kind, path)] => Ok((*kind, (*path).to_string())),
        [] => Err(fail(
            "manifest-not-found",
            "expected one of extraction.yaml, extraction.json, batch.yaml, or batch.json",
        )),
        _ => Err(fail(
            "ambiguous-manifest",
            found
                .iter()
                .map(|(_, path)| *path)
                .collect::<Vec<_>>()
                .join(", "),
        )),
    }
}

fn load_manifest(
    record: &FileRecord,
    relative: &str,
    kind: ManifestKind,
) -> Result<Value, Failure> {
    let bytes = fs::read(&record.path)
        .map_err(|error| fail("manifest-read-failed", format!("{relative}: {error}")))?;
    let document: Value = if relative.ends_with(".json") {
        serde_json::from_slice(&bytes)
            .map_err(|error| fail("manifest-syntax-invalid", format!("{relative}: {error}")))?
    } else {
        serde_yaml::from_slice(&bytes)
            .map_err(|error| fail("manifest-syntax-invalid", format!("{relative}: {error}")))?
    };
    let version = document.get("schemaVersion").and_then(Value::as_u64);
    if version != Some(1) {
        return Err(fail(
            "unsupported-manifest-schema",
            format!("{relative}: expected version 1, found {version:?}"),
        ));
    }
    validate_schema(
        &document,
        if kind == ManifestKind::Single {
            SINGLE_SCHEMA
        } else {
            BATCH_SCHEMA
        },
        relative,
    )?;
    Ok(document)
}

fn validate_schema(document: &Value, schema_text: &str, relative: &str) -> Result<(), Failure> {
    let schema: Value = serde_json::from_str(schema_text)
        .map_err(|error| fail("embedded-schema-invalid", error.to_string()))?;
    let validator = JSONSchema::options()
        .with_draft(Draft::Draft202012)
        .compile(&schema)
        .map_err(|error| fail("embedded-schema-invalid", error.to_string()))?;
    if let Err(errors) = validator.validate(document) {
        let mut messages: Vec<String> = errors.map(|error| error.to_string()).collect();
        messages.sort();
        return Err(fail(
            "schema-validation-failed",
            format!("{relative}: {}", messages.join("; ")),
        ));
    }
    Ok(())
}

fn verify_single(
    inventory: &Inventory,
    manifest: &str,
    prefix: &str,
    source: Option<&(PathBuf, Vec<u8>)>,
    exact_membership: bool,
) -> Result<SingleResult, Failure> {
    let manifest_path = join_relative(prefix, manifest);
    let record = inventory
        .files
        .get(&manifest_path)
        .ok_or_else(|| fail("missing-file", manifest_path.clone()))?;
    let document = load_manifest(record, &manifest_path, ManifestKind::Single)?;
    let mut expected_files = BTreeSet::from([manifest_path.clone()]);
    let mut expected_directories = BTreeSet::new();
    let mut payload_paths = BTreeSet::new();
    let mut folded = BTreeMap::new();
    let mut entry_paths = BTreeSet::new();
    let mut source_spans = Vec::new();
    let mut output_bytes = record.size;
    let recorded_source_size = integer(object(&document, "source")?, "size")?;
    let payloads = array(&document, "payloads")?;
    let mut dds_count = 0usize;
    let mut png_count = 0usize;
    for payload in payloads {
        let role = string(payload, "role")?;
        let path = string(payload, "path")?;
        let is_dds_role = role == "gtex-dds-texture";
        let has_dds_metadata = payload.get("dds").is_some();
        let is_dds_path = path == "payloads/texture.dds";
        if is_dds_role || has_dds_metadata || is_dds_path {
            if !is_dds_role || !has_dds_metadata || !is_dds_path {
                return Err(fail("dds-artifact-contract", format!("payload {path}")));
            }
            dds_count = dds_count
                .checked_add(1)
                .ok_or_else(|| fail("dds-artifact-contract", "DDS artifact count overflow"))?;
        }
        let is_png_role = role == "gtex-top-mip-png-preview";
        let has_png_metadata = payload.get("png").is_some();
        let is_png_path = path == "payloads/preview.png";
        if is_png_role || has_png_metadata || is_png_path {
            if !is_png_role || !has_png_metadata || !is_png_path {
                return Err(fail("png-artifact-contract", format!("payload {path}")));
            }
            png_count = png_count
                .checked_add(1)
                .ok_or_else(|| fail("png-artifact-contract", "PNG artifact count overflow"))?;
        }
    }
    if dds_count > 1 {
        return Err(fail(
            "duplicate-dds-artifact",
            "DDS export has one texture artifact per extraction",
        ));
    }
    if png_count > 1 {
        return Err(fail(
            "duplicate-png-artifact",
            "PNG preview has one texture artifact per extraction",
        ));
    }
    if let Some((source_path, source_bytes)) = source {
        verify_source_identity(&document, source_path, source_bytes)?;
    }
    for (index, payload) in payloads.iter().enumerate() {
        let relative = string(payload, "path")?;
        let normalized =
            normalize_relative_path(relative).map_err(|_| fail("unsafe-payload-path", relative))?;
        if normalized != relative || !normalized.starts_with("payloads/") {
            return Err(fail("unsafe-payload-path", relative));
        }
        if !payload_paths.insert(normalized.clone()) {
            return Err(fail("duplicate-payload-path", normalized));
        }
        if let Some(previous) = folded.insert(normalized.to_ascii_lowercase(), normalized.clone()) {
            return Err(fail(
                "case-collision",
                format!("'{previous}' and '{normalized}'"),
            ));
        }
        let full = join_relative(prefix, &normalized);
        let file = inventory
            .files
            .get(&full)
            .ok_or_else(|| fail("missing-file", full.clone()))?;
        if file.size != integer(payload, "size")? {
            return Err(fail("payload-size-mismatch", full));
        }
        if file.sha256 != string(payload, "sha256")? {
            return Err(fail("payload-sha256-mismatch", full));
        }
        if string(payload, "role")? == "gtex-dds-texture" {
            verify_dds_payload(payload, file, &document, index, recorded_source_size)?;
        }
        if string(payload, "role")? == "gtex-top-mip-png-preview" {
            verify_png_payload(payload, file, &document, index, recorded_source_size)?;
        }
        output_bytes = checked_add(output_bytes, file.size, "output-byte-overflow")?;
        expected_files.insert(full);
        expected_directories.insert(join_relative(prefix, "payloads"));
        if let Some(entry_path) = verify_payload_relationship(payload, &document, index)? {
            let derived_preview = string(payload, "role")? == "gtex-top-mip-png-preview";
            if !derived_preview {
                if !entry_paths.insert(entry_path.clone()) {
                    return Err(fail("duplicate-entry-relationship", entry_path));
                }
                let span = object(payload, "sourceSpan")?;
                let start = integer(span, "offset")?;
                let end = integer(span, "endExclusive")?;
                if end > recorded_source_size {
                    return Err(fail("payload-span-out-of-range", string(payload, "path")?));
                }
                source_spans.push((start, end, string(payload, "path")?.to_string()));
            }
        }
        if let Some((_, source_bytes)) = source {
            replay_payload(payload, file, source_bytes, &document)?;
        }
    }
    if let Some((source_path, source_bytes)) = source {
        verify_source(&document, source_path, source_bytes)?;
    }
    source_spans.sort();
    for pair in source_spans.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(fail(
                "payload-span-overlap",
                format!("'{}' and '{}'", pair[0].2, pair[1].2),
            ));
        }
    }
    if exact_membership {
        compare_membership(inventory, &expected_files, &expected_directories)?;
    }
    Ok(SingleResult {
        format: string(object(&document, "format")?, "id")?.to_string(),
        source_size: integer(object(&document, "source")?, "size")?,
        payloads: payloads.len(),
        output_bytes,
        expected_files,
        expected_directories,
        document,
    })
}

fn verify_payload_relationship(
    payload: &Value,
    document: &Value,
    index: usize,
) -> Result<Option<String>, Failure> {
    let Some(span) = payload.get("sourceSpan") else {
        if payload.get("container").is_some() || payload.get("entry").is_some() {
            return Err(fail(
                "incomplete-payload-relationship",
                format!("payload {index}"),
            ));
        }
        return Ok(None);
    };
    let offset = integer(span, "offset")?;
    let length = integer(span, "length")?;
    let end = checked_add(offset, length, "payload-span-overflow")?;
    let preview = string(payload, "role")? == "gtex-top-mip-png-preview";
    if integer(span, "endExclusive")? != end || (!preview && integer(payload, "size")? != length) {
        return Err(fail("payload-span-mismatch", format!("payload {index}")));
    }
    if string(payload, "role")? == "gtex-encoded-surface" {
        let entry = object(payload, "entry")?;
        let entry_path = string(entry, "path")?;
        let entry_index = entry_path
            .strip_prefix("$.parsed.offsetTable.entries[")
            .and_then(|value| value.strip_suffix(']'))
            .and_then(|value| value.parse::<usize>().ok())
            .ok_or_else(|| fail("entry-path-invalid", entry_path))?;
        let parsed_entry = document
            .pointer(&format!("/parsed/offsetTable/entries/{entry_index}"))
            .ok_or_else(|| fail("relationship-target-missing", entry_path))?;
        let mut expected_span = span.clone();
        expected_span
            .as_object_mut()
            .expect("schema validated sourceSpan as an object")
            .remove("endExclusive");
        let parsed_format = document
            .pointer("/parsed/texture/formatIndex/mapping")
            .ok_or_else(|| {
                fail(
                    "relationship-target-missing",
                    "$.parsed.texture.formatIndex.mapping",
                )
            })?;
        let recorded_format = object(entry, "gtexFormat")?;
        if entry.get("kind").and_then(Value::as_str) != Some("gtex-encoded-surface")
            || entry.get("face") != parsed_entry.get("face")
            || entry.get("mipLevel") != parsed_entry.get("mipLevel")
            || parsed_entry.pointer("/source/span") != Some(&expected_span)
            || recorded_format.get("clientIndex")
                != document.pointer("/parsed/texture/formatIndex/value")
            || recorded_format.get("d3dName") != parsed_format.get("d3dName")
            || recorded_format.get("d3dValue") != parsed_format.get("d3dValue")
        {
            return Err(fail(
                "entry-relationship-mismatch",
                format!("payload {index}"),
            ));
        }
        return Ok(Some(entry_path.to_string()));
    }
    let container = object(payload, "container")?;
    if string(container, "path")? != "$.parsed.root" {
        return Err(fail("container-path-mismatch", format!("payload {index}")));
    }
    let parsed_root = document
        .pointer("/parsed/root")
        .ok_or_else(|| fail("relationship-target-missing", "$.parsed.root"))?;
    if container.get("format") != document.pointer("/parsed/format")
        || container.get("subtype") != parsed_root.get("subtype")
        || container.get("span") != parsed_root.get("span")
    {
        return Err(fail(
            "container-relationship-mismatch",
            format!("payload {index}"),
        ));
    }
    let entry = object(payload, "entry")?;
    let entry_path = string(entry, "path")?;
    let entry_index = entry_path
        .strip_prefix("$.parsed.root.entries[")
        .and_then(|value| value.strip_suffix(']'))
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| fail("entry-path-invalid", entry_path))?;
    let parsed_entry = document
        .pointer(&format!("/parsed/root/entries/{entry_index}"))
        .ok_or_else(|| fail("relationship-target-missing", entry_path))?;
    let mut expected_span = span.clone();
    expected_span
        .as_object_mut()
        .expect("schema validated sourceSpan as an object")
        .remove("endExclusive");
    if entry.get("kind") != parsed_entry.get("kind")
        || parsed_entry.get("span") != Some(&expected_span)
        || entry.get("index") != parsed_entry.get("index")
        || entry.get("declaredOffset") != parsed_entry.get("declaredOffset")
        || entry.get("declaredSize") != parsed_entry.get("declaredSize")
        || entry.get("declaredKind") != parsed_entry.get("declaredKind")
    {
        return Err(fail(
            "entry-relationship-mismatch",
            format!("payload {index}"),
        ));
    }
    match (entry.get("childContainer"), parsed_entry.get("child")) {
        (None, None) => {}
        (Some(recorded), Some(child)) => {
            let kind = string(child, "kind")?;
            let format = kind.strip_suffix("-container").unwrap_or(kind);
            if recorded.get("format").and_then(Value::as_str) != Some(format)
                || recorded.get("subtype") != child.get("subtype")
                || recorded.get("span") != child.get("span")
            {
                return Err(fail(
                    "child-container-relationship-mismatch",
                    format!("payload {index}"),
                ));
            }
        }
        (Some(_), None) => {
            return Err(fail(
                "child-container-relationship-mismatch",
                format!("payload {index}"),
            ))
        }
        (None, Some(_)) => {
            return Err(fail(
                "child-container-relationship-mismatch",
                format!("payload {index}"),
            ))
        }
    }
    Ok(Some(entry_path.to_string()))
}

fn verify_dds_payload(
    payload: &Value,
    file: &FileRecord,
    document: &Value,
    index: usize,
    source_size: u64,
) -> Result<(), Failure> {
    if string(payload, "path")? != "payloads/texture.dds"
        || string(payload, "role")? != "gtex-dds-texture"
    {
        return Err(fail("dds-artifact-contract", format!("payload {index}")));
    }
    let dds_record = object(payload, "dds")?;
    let bytes =
        fs::read(&file.path).map_err(|error| fail("payload-read-failed", error.to_string()))?;
    let image = dds::parse(&bytes)
        .map_err(|error| fail("dds-payload-invalid", format!("payload {index}: {error}")))?;
    let parsed = object(document, "parsed")?;
    if string(object(document, "format")?, "id")? != "gtex"
        || string(parsed, "format")? != "gtex"
        || string(object(parsed, "surfaceMaterialization")?, "status")? != "supported"
    {
        return Err(fail("dds-gtex-eligibility", format!("payload {index}")));
    }
    let geometry = verify_gtex_geometry(parsed, index, source_size, "dds")?;
    let input_length = geometry.input_length;
    let width = u64::from(geometry.width);
    let height = u64::from(geometry.height);
    let mip_levels = u64::try_from(geometry.mip_levels).map_err(|_| {
        fail(
            "dds-gtex-layout",
            format!("payload {index}: mip count does not fit this platform"),
        )
    })?;
    let canonical = geometry.canonical;
    let offset_table = object(parsed, "offsetTable")?;
    let texture = object(parsed, "texture")?;
    if image.width as u64 != width
        || image.height as u64 != height
        || image.mip_levels as u64 != mip_levels
        || integer(dds_record, "width")? != width
        || integer(dds_record, "height")? != height
        || integer(dds_record, "mipLevels")? != mip_levels
    {
        return Err(fail("dds-metadata-mismatch", format!("payload {index}")));
    }
    let format_index_value = object(texture, "formatIndex")?
        .get("value")
        .ok_or_else(|| fail("manifest-semantic-error", "formatIndex value is missing"))?;
    let recorded_format = object(dds_record, "format")?;
    if recorded_format.get("clientIndex") != Some(format_index_value)
        || recorded_format.get("d3dName").and_then(Value::as_str) != Some(canonical.d3d_name)
        || recorded_format.get("d3dValue").and_then(Value::as_u64)
            != Some(u64::from(canonical.d3d_value))
    {
        return Err(fail("dds-format-mismatch", format!("payload {index}")));
    }
    let expected_pixel = match canonical.index {
        4 => DdsPixelFormat::A8R8G8B8,
        24 => DdsPixelFormat::Dxt1,
        26 => DdsPixelFormat::Dxt5,
        _ => return Err(fail("dds-format-mismatch", format!("payload {index}"))),
    };
    if image.format != expected_pixel {
        return Err(fail("dds-format-mismatch", format!("payload {index}")));
    }
    let header = object(dds_record, "headerSpan")?;
    if integer(header, "offset")? != 0
        || integer(header, "length")? != dds::DDS_FILE_HEADER_SIZE as u64
    {
        return Err(fail("dds-header-span-mismatch", format!("payload {index}")));
    }
    let mips = array(dds_record, "mips")?;
    let mip_count = usize::try_from(mip_levels).map_err(|_| {
        fail(
            "dds-gtex-layout",
            format!("payload {index}: mip count does not fit this platform"),
        )
    })?;
    if mips.len() != image.mips.len() || mips.len() != mip_count {
        return Err(fail("dds-mip-count-mismatch", format!("payload {index}")));
    }
    let entries = array(offset_table, "entries")?;
    if entries.len() != image.mips.len() || entries.len() != mip_count {
        return Err(fail("dds-mip-count-mismatch", format!("payload {index}")));
    }
    for (mip, (record, layout)) in mips.iter().zip(&image.mips).enumerate() {
        let entry = entries.get(mip).ok_or_else(|| {
            fail(
                "dds-source-entry-missing",
                format!("payload {index} mip {mip}"),
            )
        })?;
        if integer(entry, "index")? != mip as u64
            || integer(entry, "face")? != 0
            || integer(entry, "mipLevel")? != mip as u64
            || integer(record, "mipLevel")? != layout.mip_level as u64
            || integer(record, "width")? != layout.width as u64
            || integer(record, "height")? != layout.height as u64
        {
            return Err(fail(
                "dds-mip-metadata-mismatch",
                format!("payload {index} mip {mip}"),
            ));
        }
        let source_span = object(record, "sourceSpan")?;
        let dds_span = object(record, "ddsSpan")?;
        let expected_source = object(object(entry, "source")?, "span")?;
        let expected_source_end = checked_add(
            integer(expected_source, "offset")?,
            integer(expected_source, "length")?,
            "dds-source-span-overflow",
        )?;
        let source_length = integer(expected_source, "length")?;
        if source_length != layout.span.length || expected_source_end > input_length {
            return Err(fail(
                "dds-gtex-layout",
                format!("payload {index} mip {mip}: source table geometry is inconsistent"),
            ));
        }
        if source_span.get("offset") != expected_source.get("offset")
            || source_span.get("length") != expected_source.get("length")
            || integer(source_span, "length")? != layout.span.length
            || integer(source_span, "endExclusive")? != expected_source_end
            || integer(dds_span, "offset")? != layout.span.offset
            || integer(dds_span, "length")? != layout.span.length
            || integer(dds_span, "endExclusive")?
                != checked_add(layout.span.offset, layout.span.length, "dds-span-overflow")?
        {
            return Err(fail(
                "dds-mip-span-mismatch",
                format!("payload {index} mip {mip}"),
            ));
        }
        let source_offset = integer(source_span, "offset")?;
        let source_length = integer(source_span, "length")?;
        if checked_add(source_offset, source_length, "dds-source-span-overflow")? > source_size {
            return Err(fail(
                "dds-source-span-out-of-range",
                format!("payload {index} mip {mip}"),
            ));
        }
        let dds_start = usize::try_from(layout.span.offset).map_err(|_| {
            fail(
                "dds-span-out-of-range",
                format!("payload {index} mip {mip}"),
            )
        })?;
        let dds_end = usize::try_from(checked_add(
            layout.span.offset,
            layout.span.length,
            "dds-span-overflow",
        )?)
        .map_err(|_| {
            fail(
                "dds-span-out-of-range",
                format!("payload {index} mip {mip}"),
            )
        })?;
        let digest = sha256_hex(bytes.get(dds_start..dds_end).ok_or_else(|| {
            fail(
                "dds-span-out-of-range",
                format!("payload {index} mip {mip}"),
            )
        })?);
        if string(record, "sha256")? != digest {
            return Err(fail(
                "dds-mip-sha256-mismatch",
                format!("payload {index} mip {mip}"),
            ));
        }
        if string(record, "sha256")? != string(object(entry, "source")?, "sha256")? {
            return Err(fail(
                "dds-source-digest-mismatch",
                format!("payload {index} mip {mip}"),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct GtexGeometry {
    input_length: u64,
    width: u32,
    height: u32,
    mip_levels: usize,
    canonical: gtex_pwib::GtexFormat,
}

fn gtex_error(prefix: &str, kind: &str, index: usize, detail: impl std::fmt::Display) -> Failure {
    let code = format!("{prefix}-{kind}");
    fail(&code, format!("payload {index}: {detail}"))
}

fn verify_gtex_geometry(
    parsed: &Value,
    index: usize,
    source_size: u64,
    prefix: &str,
) -> Result<GtexGeometry, Failure> {
    let input_length = integer(object(parsed, "input")?, "length")?;
    if input_length != source_size {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "parsed input length differs from source size",
        ));
    }
    let data_base_record = object(parsed, "dataBase")?;
    let data_base = integer(data_base_record, "value")?;
    if data_base < 0x18 || data_base > source_size {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "invalid data base",
        ));
    }
    let data_base_span = object(data_base_record, "span")?;
    if integer(data_base_span, "offset")? != 0x14 || integer(data_base_span, "length")? != 4 {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "invalid data base span",
        ));
    }
    let header_span = object(object(parsed, "header")?, "span")?;
    if integer(header_span, "offset")? != 0 || integer(header_span, "length")? != data_base {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "invalid header span",
        ));
    }
    let data_region_span = object(object(parsed, "dataRegion")?, "span")?;
    let data_region_length = source_size.checked_sub(data_base).ok_or_else(|| {
        gtex_error(
            prefix,
            "gtex-layout",
            index,
            "data base exceeds source size",
        )
    })?;
    if integer(data_region_span, "offset")? != data_base
        || integer(data_region_span, "length")? != data_region_length
    {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "invalid data region span",
        ));
    }

    let texture = object(parsed, "texture")?;
    if string(texture, "kind")? != "2d"
        || integer(object(texture, "flags")?, "value")? != 0
        || integer(object(texture, "depth")?, "value")? != 1
    {
        return Err(gtex_error(
            prefix,
            "gtex-eligibility",
            index,
            "unsupported texture shape",
        ));
    }
    let width_value = integer(object(texture, "width")?, "value")?;
    let height_value = integer(object(texture, "height")?, "value")?;
    let mip_levels_value = integer(object(texture, "mipLevels")?, "value")?;
    if width_value == 0
        || height_value == 0
        || width_value > u64::from(u16::MAX)
        || height_value > u64::from(u16::MAX)
        || mip_levels_value == 0
        || mip_levels_value > u64::from(u8::MAX)
    {
        return Err(gtex_error(
            prefix,
            "gtex-eligibility",
            index,
            "invalid dimensions or mip count",
        ));
    }
    let width = u32::try_from(width_value).map_err(|_| {
        gtex_error(
            prefix,
            "gtex-eligibility",
            index,
            "width does not fit GTEX field",
        )
    })?;
    let height = u32::try_from(height_value).map_err(|_| {
        gtex_error(
            prefix,
            "gtex-eligibility",
            index,
            "height does not fit GTEX field",
        )
    })?;
    let mip_levels_value = u8::try_from(mip_levels_value).map_err(|_| {
        gtex_error(
            prefix,
            "gtex-eligibility",
            index,
            "mip count does not fit GTEX field",
        )
    })?;
    let mip_levels = usize::from(mip_levels_value);
    if u32::from(mip_levels_value) > dds::max_mip_levels(width, height) {
        return Err(gtex_error(
            prefix,
            "gtex-eligibility",
            index,
            "mip count exceeds geometric maximum",
        ));
    }

    let offset_table = object(parsed, "offsetTable")?;
    let table_base_record = object(offset_table, "base")?;
    let table_base = integer(table_base_record, "value")?;
    if table_base < 0x18 || table_base > data_base || integer(offset_table, "entryStride")? != 8 {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "invalid offset table",
        ));
    }
    let table_base_span = object(table_base_record, "span")?;
    if integer(table_base_span, "offset")? != 0x10 || integer(table_base_span, "length")? != 4 {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "invalid offset table span",
        ));
    }
    let entries = array(offset_table, "entries")?;
    if entries.len() != mip_levels {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "table entry count mismatch",
        ));
    }
    let format_record = object(texture, "formatIndex")?;
    let format_index = integer(format_record, "value")?;
    let canonical = u8::try_from(format_index)
        .ok()
        .and_then(gtex_pwib::gtex_format)
        .ok_or_else(|| gtex_error(prefix, "gtex-eligibility", index, "unmapped format"))?;
    let mapping = object(format_record, "mapping")?;
    let mapping_block_ok = match (canonical.block_bytes, mapping.get("blockBytes")) {
        (Some(expected), Some(value)) => value.as_u64() == Some(u64::from(expected)),
        (None, Some(Value::Null)) => true,
        _ => false,
    };
    if mapping.get("d3dName").and_then(Value::as_str) != Some(canonical.d3d_name)
        || mapping.get("d3dValue").and_then(Value::as_u64) != Some(u64::from(canonical.d3d_value))
        || mapping.get("bitsPerPixel").and_then(Value::as_u64)
            != Some(u64::from(canonical.bits_per_pixel))
        || !mapping_block_ok
    {
        return Err(gtex_error(
            prefix,
            "format-mismatch",
            index,
            "format mapping is not canonical",
        ));
    }

    let table_length = u64::try_from(mip_levels)
        .ok()
        .and_then(|count| count.checked_mul(8))
        .ok_or_else(|| gtex_error(prefix, "gtex-layout", index, "table length overflows"))?;
    let table_end = table_base
        .checked_add(table_length)
        .ok_or_else(|| gtex_error(prefix, "gtex-layout", index, "offset table end overflows"))?;
    if table_end > data_base {
        return Err(gtex_error(
            prefix,
            "gtex-layout",
            index,
            "table exceeds data base",
        ));
    }

    let mut previous_end = data_base;
    for (mip, entry) in entries.iter().enumerate() {
        let mip_offset = u64::try_from(mip)
            .ok()
            .and_then(|value| value.checked_mul(8))
            .ok_or_else(|| gtex_error(prefix, "gtex-layout", index, "table offset overflows"))?;
        let entry_offset = table_base.checked_add(mip_offset).ok_or_else(|| {
            gtex_error(prefix, "gtex-layout", index, "table entry offset overflows")
        })?;
        let size_offset = entry_offset.checked_add(4).ok_or_else(|| {
            gtex_error(prefix, "gtex-layout", index, "table size offset overflows")
        })?;
        let offset_field = object(entry, "offsetField")?;
        let size_field = object(entry, "sizeField")?;
        let offset_field_span = object(offset_field, "span")?;
        let size_field_span = object(size_field, "span")?;
        if integer(offset_field_span, "offset")? != entry_offset
            || integer(offset_field_span, "length")? != 4
            || integer(size_field_span, "offset")? != size_offset
            || integer(size_field_span, "length")? != 4
        {
            return Err(gtex_error(
                prefix,
                "gtex-layout",
                index,
                format!("mip {mip}: table field spans are inconsistent"),
            ));
        }
        let mip_u64 = u64::try_from(mip)
            .map_err(|_| gtex_error(prefix, "gtex-layout", index, "mip index does not fit"))?;
        if integer(entry, "index")? != mip_u64
            || integer(entry, "face")? != 0
            || integer(entry, "mipLevel")? != mip_u64
        {
            return Err(gtex_error(
                prefix,
                "gtex-layout",
                index,
                format!("mip {mip}: invalid table order"),
            ));
        }
        let mip_shift = u32::try_from(mip)
            .map_err(|_| gtex_error(prefix, "gtex-layout", index, "mip shift does not fit"))?;
        let mip_width = width.checked_shr(mip_shift).unwrap_or(0).max(1);
        let mip_height = height.checked_shr(mip_shift).unwrap_or(0).max(1);
        let calculated = if let Some(block_bytes) = canonical.block_bytes {
            u64::from(mip_width.div_ceil(4))
                .checked_mul(u64::from(mip_height.div_ceil(4)))
                .and_then(|value| value.checked_mul(u64::from(block_bytes)))
                .ok_or_else(|| gtex_error(prefix, "gtex-layout", index, "mip size overflows"))?
        } else {
            u64::from(mip_width)
                .checked_mul(u64::from(mip_height))
                .and_then(|value| value.checked_mul(4))
                .ok_or_else(|| gtex_error(prefix, "gtex-layout", index, "mip size overflows"))?
        };
        let declared = integer(size_field, "value")?;
        let calculated_record = integer(entry, "calculatedSize")?;
        let source = object(entry, "source")?;
        let source_span = object(source, "span")?;
        let source_offset = data_base
            .checked_add(integer(offset_field, "value")?)
            .ok_or_else(|| {
                gtex_error(
                    prefix,
                    "gtex-layout",
                    index,
                    format!("mip {mip}: source offset overflows"),
                )
            })?;
        let source_end = source_offset.checked_add(calculated).ok_or_else(|| {
            gtex_error(
                prefix,
                "gtex-layout",
                index,
                format!("mip {mip}: source span overflows"),
            )
        })?;
        if declared != calculated
            || calculated_record != calculated
            || integer(source_span, "offset")? != source_offset
            || integer(source_span, "length")? != calculated
            || source_offset < previous_end
            || source_end > source_size
        {
            return Err(gtex_error(
                prefix,
                "gtex-layout",
                index,
                format!("mip {mip}: source geometry mismatch"),
            ));
        }
        previous_end = source_end;
    }
    Ok(GtexGeometry {
        input_length,
        width,
        height,
        mip_levels,
        canonical,
    })
}

fn verify_png_payload(
    payload: &Value,
    file: &FileRecord,
    document: &Value,
    index: usize,
    source_size: u64,
) -> Result<(), Failure> {
    if string(payload, "path")? != "payloads/preview.png"
        || string(payload, "role")? != "gtex-top-mip-png-preview"
    {
        return Err(fail("png-artifact-contract", format!("payload {index}")));
    }
    let png_record = object(payload, "png")?;
    let bytes =
        fs::read(&file.path).map_err(|error| fail("payload-read-failed", error.to_string()))?;
    let decoded = texture_preview::decode_png_rgba(&bytes)
        .map_err(|error| fail("png-payload-invalid", format!("payload {index}: {error}")))?;
    let canonical = texture_preview::encode_png_rgba(decoded.width, decoded.height, &decoded.rgba)
        .map_err(|error| fail("png-payload-invalid", format!("payload {index}: {error}")))?;
    if canonical != bytes {
        return Err(fail(
            "png-payload-noncanonical",
            format!("payload {index}: PNG has metadata, interlace, or trailing bytes"),
        ));
    }
    let rgba_sha256 = sha256_hex(&decoded.rgba);
    if string(png_record, "rgbaSha256")? != rgba_sha256
        || decoded.width as u64 != integer(png_record, "width")?
        || decoded.height as u64 != integer(png_record, "height")?
        || integer(png_record, "mipLevel")? != 0
    {
        return Err(fail("png-metadata-mismatch", format!("payload {index}")));
    }
    let parsed = object(document, "parsed")?;
    if string(object(document, "format")?, "id")? != "gtex"
        || string(parsed, "format")? != "gtex"
        || string(object(parsed, "surfaceMaterialization")?, "status")? != "supported"
    {
        return Err(fail("png-gtex-eligibility", format!("payload {index}")));
    }
    let geometry = verify_gtex_geometry(parsed, index, source_size, "png")?;
    let texture = object(parsed, "texture")?;
    if decoded.width != geometry.width || decoded.height != geometry.height {
        return Err(fail("png-metadata-mismatch", format!("payload {index}")));
    }
    let format_index_value = object(texture, "formatIndex")?
        .get("value")
        .ok_or_else(|| fail("manifest-semantic-error", "formatIndex value is missing"))?;
    let canonical_format = geometry.canonical;
    let expected_format = object(object(texture, "formatIndex")?, "mapping")?;
    let recorded_format = object(png_record, "format")?;
    if recorded_format.get("clientIndex") != Some(format_index_value)
        || recorded_format.get("d3dName") != expected_format.get("d3dName")
        || recorded_format.get("d3dValue") != expected_format.get("d3dValue")
        || expected_format.get("d3dName").and_then(Value::as_str) != Some(canonical_format.d3d_name)
        || expected_format.get("d3dValue").and_then(Value::as_u64)
            != Some(u64::from(canonical_format.d3d_value))
    {
        return Err(fail("png-format-mismatch", format!("payload {index}")));
    }
    let source_span = object(png_record, "sourceSpan")?;
    let source_end = checked_add(
        integer(source_span, "offset")?,
        integer(source_span, "length")?,
        "png-source-span-overflow",
    )?;
    if integer(source_span, "offset")? >= source_size
        || source_end > source_size
        || integer(source_span, "endExclusive")? != source_end
    {
        return Err(fail("png-source-span-mismatch", format!("payload {index}")));
    }
    let parsed_entry = parsed
        .pointer("/offsetTable/entries/0")
        .ok_or_else(|| fail("png-source-entry-missing", format!("payload {index}")))?;
    let expected_source = object(parsed_entry, "source")?;
    if source_span.get("offset")
        != expected_source
            .get("span")
            .and_then(|value| value.get("offset"))
        || source_span.get("length")
            != expected_source
                .get("span")
                .and_then(|value| value.get("length"))
        || string(png_record, "sourceSha256")? != string(expected_source, "sha256")?
    {
        return Err(fail(
            "png-source-entry-mismatch",
            format!("payload {index}"),
        ));
    }
    Ok(())
}

fn replay_payload(
    payload: &Value,
    file: &FileRecord,
    source: &[u8],
    document: &Value,
) -> Result<(), Failure> {
    let bytes =
        fs::read(&file.path).map_err(|error| fail("payload-read-failed", error.to_string()))?;
    if string(payload, "role")? == "gtex-dds-texture" {
        let source_name = string(object(document, "source")?, "fileName")?;
        let replay = plan_bytes(
            source_name,
            source,
            string(object(document, "source")?, "sha256")?,
            DocumentFormat::Json,
            false,
            true,
            &["--as".into(), "gtex".into()],
        )?;
        let expected = replay
            .artifact_bytes("payloads/texture.dds")
            .ok_or_else(|| fail("payload-replay-unsupported", "DDS artifact missing"))?;
        if expected != bytes {
            return Err(fail("payload-replay-mismatch", string(payload, "path")?));
        }
        return Ok(());
    }
    if string(payload, "role")? == "gtex-top-mip-png-preview" {
        let source_name = string(object(document, "source")?, "fileName")?;
        let replay = plan_bytes_options(
            source_name,
            source,
            string(object(document, "source")?, "sha256")?,
            DocumentFormat::Json,
            false,
            false,
            true,
            &["--as".into(), "gtex".into()],
        )?;
        let expected = replay
            .artifact_bytes("payloads/preview.png")
            .ok_or_else(|| fail("payload-replay-unsupported", "PNG preview artifact missing"))?;
        if expected != bytes {
            return Err(fail("payload-replay-mismatch", string(payload, "path")?));
        }
        return Ok(());
    }
    if let Some(span) = payload.get("sourceSpan") {
        let payload_path = string(payload, "path")?;
        let start = usize::try_from(integer(span, "offset")?)
            .map_err(|_| fail("payload-span-out-of-range", payload_path))?;
        let end = usize::try_from(integer(span, "endExclusive")?)
            .map_err(|_| fail("payload-span-out-of-range", payload_path))?;
        if source.get(start..end) != Some(bytes.as_slice()) {
            return Err(fail("payload-replay-mismatch", string(payload, "path")?));
        }
    } else {
        let role = string(payload, "role")?;
        let expected = match role {
            "decoded-lua-5.1-chunk" => {
                extract_lpb(source)
                    .map_err(|error| fail("source-replay-failed", error.to_string()))?
                    .decoded
            }
            "decoded-xml-document" => {
                let format = string(object(document, "format")?, "id")?;
                match format {
                    "sqwt" => {
                        let name = string(object(document, "source")?, "fileName")?;
                        xivl_formats::sqwt::decode(source, name)
                            .map_err(|error| fail("source-replay-failed", error.to_string()))?
                            .document
                    }
                    "scrambled-xml" => {
                        xivl_formats::scrambled::decode(source)
                            .map_err(|error| fail("source-replay-failed", error.to_string()))?
                            .document
                    }
                    other => {
                        return Err(fail(
                            "payload-replay-unsupported",
                            format!("decoded XML payload under format '{other}'"),
                        ))
                    }
                }
            }
            _ => return Ok(()),
        };
        if expected != bytes {
            return Err(fail("payload-replay-mismatch", string(payload, "path")?));
        }
    }
    Ok(())
}

fn verify_source(document: &Value, path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let source = object(document, "source")?;
    let format = string(object(document, "format")?, "id")?;
    let inspect_format = if format == "res" { "sedb" } else { format };
    let inspect_arguments = ["--as".to_string(), inspect_format.to_string()];
    let materialize = array(document, "payloads")?.iter().any(|payload| {
        payload.get("role").and_then(Value::as_str) == Some("gtex-encoded-surface")
            || payload.get("container").is_some()
    });
    let export_dds = array(document, "payloads")?
        .iter()
        .any(|payload| payload.get("role").and_then(Value::as_str) == Some("gtex-dds-texture"));
    let preview_png = array(document, "payloads")?.iter().any(|payload| {
        payload.get("role").and_then(Value::as_str) == Some("gtex-top-mip-png-preview")
    });
    let replay = plan_bytes_options(
        &path.display().to_string(),
        bytes,
        string(source, "sha256")?,
        DocumentFormat::Json,
        materialize,
        export_dds,
        preview_png,
        &inspect_arguments,
    )?;
    if replay.format_id() != format {
        return Err(fail(
            "stale-source-format",
            format!("manifest '{format}', current '{}'", replay.format_id()),
        ));
    }
    let replay_document: Value = serde_json::from_str(replay.document())
        .map_err(|error| fail("source-replay-failed", error.to_string()))?;
    for key in ["parsed", "anomalies", "format", "payloads"] {
        if document.get(key) != replay_document.get(key) {
            return Err(fail(
                "source-replay-structure-mismatch",
                format!("field {key}"),
            ));
        }
    }
    let recorded_id = source.get("resourceId").and_then(Value::as_str);
    if let Ok(id) = parse_dat_path(&path.display().to_string(), 0) {
        if recorded_id != Some(id.to_hex().as_str()) {
            return Err(fail("stale-source-resource-id", path.display().to_string()));
        }
    }
    Ok(())
}

fn verify_source_identity(document: &Value, path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let source = object(document, "source")?;
    if integer(source, "size")? != bytes.len() as u64 {
        return Err(fail("stale-source-size", path.display().to_string()));
    }
    let digest = sha256_hex(bytes);
    if string(source, "sha256")? != digest {
        return Err(fail("stale-source-sha256", path.display().to_string()));
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| fail("source-name-invalid", path.display().to_string()))?;
    if string(source, "fileName")? != file_name {
        return Err(fail("stale-source-name", path.display().to_string()));
    }
    Ok(())
}

fn verify_batch(
    options: &Options,
    inventory: &Inventory,
    manifest: &str,
) -> Result<Value, Failure> {
    let record = inventory
        .files
        .get(manifest)
        .ok_or_else(|| fail("missing-file", manifest))?;
    let document = load_manifest(record, manifest, ManifestKind::Batch)?;
    let resources = array(&document, "resources")?;
    let export_dds = match document.get("exportDds") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| fail("manifest-semantic-error", "exportDds is not a boolean"))?,
    };
    let preview_png = match document.get("previewPng") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| fail("manifest-semantic-error", "previewPng is not a boolean"))?,
    };
    let replay = match (&options.catalog, &options.root) {
        (Some(catalog), Some(root)) => Some(load_replay(catalog, root, &document)?),
        _ => None,
    };
    let mut expected_files = BTreeSet::from([manifest.to_string()]);
    let mut expected_directories = BTreeSet::new();
    let mut ordinals = BTreeSet::new();
    let mut indices = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut folded = BTreeMap::new();
    let mut source_bytes = 0u64;
    let mut output_bytes = record.size;
    let mut payload_count = 0usize;
    for (position, resource) in resources.iter().enumerate() {
        let ordinal = integer(resource, "ordinal")?;
        if ordinal != position as u64 + 1 || !ordinals.insert(ordinal) {
            return Err(fail("invalid-resource-ordinal", ordinal.to_string()));
        }
        let catalog_index = integer(resource, "catalogIndex")?;
        if !indices.insert(catalog_index) {
            return Err(fail("duplicate-catalog-index", catalog_index.to_string()));
        }
        let directory =
            normalize_relative_path(string(resource, "outputDirectory")?).map_err(|_| {
                fail(
                    "unsafe-output-directory",
                    string(resource, "outputDirectory").unwrap_or(""),
                )
            })?;
        if directory.contains('/') || !directories.insert(directory.clone()) {
            return Err(fail("invalid-output-directory", directory));
        }
        if let Some(previous) = folded.insert(directory.to_ascii_lowercase(), directory.clone()) {
            return Err(fail(
                "case-collision",
                format!("'{previous}' and '{directory}'"),
            ));
        }
        let nested_manifest = string(resource, "manifest")?;
        let expected_manifest = format!(
            "{directory}/extraction.{}",
            if nested_manifest.ends_with(".json") {
                "json"
            } else {
                "yaml"
            }
        );
        if nested_manifest != expected_manifest {
            return Err(fail("manifest-path-mismatch", nested_manifest));
        }
        let nested_name = nested_manifest
            .rsplit('/')
            .next()
            .expect("normalized path has a name");
        let source = if let Some((entries, root, canonical_root)) = &replay {
            let catalog_entry = entries
                .get(catalog_index as usize)
                .ok_or_else(|| fail("catalog-index-out-of-range", catalog_index.to_string()))?;
            verify_catalog_record(resource, catalog_entry)?;
            let source_path = secure_source(root, canonical_root, &catalog_entry.source_path)?;
            Some(read_source(&source_path)?)
        } else {
            None
        };
        let nested = verify_single(inventory, nested_name, &directory, source.as_ref(), false)?;
        let nested_export_dds = array(&nested.document, "payloads")?
            .iter()
            .any(|payload| payload.get("role").and_then(Value::as_str) == Some("gtex-dds-texture"));
        if nested_export_dds != export_dds {
            return Err(fail("batch-dds-selection-mismatch", directory));
        }
        let nested_preview_png = array(&nested.document, "payloads")?.iter().any(|payload| {
            payload.get("role").and_then(Value::as_str) == Some("gtex-top-mip-png-preview")
        });
        if nested_preview_png != preview_png {
            return Err(fail("batch-png-selection-mismatch", directory));
        }
        let nested_source = object(&nested.document, "source")?;
        if string(resource, "sourcePath")?.rsplit('/').next()
            != Some(string(nested_source, "fileName")?)
            || resource.get("resourceId") != nested_source.get("resourceId")
            || integer(resource, "sourceSize")? != nested.source_size
            || string(resource, "sourceSha256")? != string(nested_source, "sha256")?
            || string(resource, "detectedFormat")? != nested.format
            || integer(resource, "outputBytes")? != nested.output_bytes
        {
            return Err(fail("resource-record-mismatch", directory));
        }
        source_bytes = checked_add(source_bytes, nested.source_size, "source-byte-overflow")?;
        output_bytes = checked_add(output_bytes, nested.output_bytes, "output-byte-overflow")?;
        payload_count = payload_count
            .checked_add(nested.payloads)
            .ok_or_else(|| fail("payload-count-overflow", "batch"))?;
        expected_directories.insert(directory.clone());
        expected_files.extend(nested.expected_files);
        expected_directories.extend(nested.expected_directories);
    }
    let totals = object(&document, "totals")?;
    if integer(totals, "resourceCount")? != resources.len() as u64
        || integer(totals, "sourceBytes")? != source_bytes
        || integer(totals, "outputBytes")? != output_bytes
    {
        return Err(fail(
            "batch-totals-mismatch",
            "recorded totals disagree with files",
        ));
    }
    compare_membership(inventory, &expected_files, &expected_directories)?;
    Ok(json!({
        "kind": "catalog",
        "resources": resources.len(),
        "payloads": payload_count,
        "sourceBytes": source_bytes,
        "outputBytes": output_bytes,
        "sourceReplay": replay.is_some(),
        "status": "verified"
    }))
}

fn load_replay(
    catalog: &Path,
    root: &Path,
    document: &Value,
) -> Result<(Vec<CatalogEntry>, PathBuf, PathBuf), Failure> {
    reject_link_if_present(catalog, "catalog")?;
    let catalog_bytes = read_capped(&catalog.display().to_string())?;
    let recorded = object(document, "catalog")?;
    let name = catalog
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| fail("catalog-name-invalid", catalog.display().to_string()))?;
    if string(recorded, "fileName")? != name
        || string(recorded, "sha256")? != sha256_hex(&catalog_bytes)
    {
        return Err(fail("stale-catalog", catalog.display().to_string()));
    }
    let entries = parse_catalog(&catalog_bytes)?;
    let canonical_root = secure_root(root)?;
    Ok((entries, root.to_path_buf(), canonical_root))
}

fn verify_catalog_record(resource: &Value, entry: &CatalogEntry) -> Result<(), Failure> {
    if string(resource, "sourcePath")? != entry.source_path
        || resource.get("resourceId").and_then(Value::as_str) != entry.resource_id.as_deref()
        || integer(resource, "sourceSize")? != entry.size
        || string(resource, "sourceSha256")? != entry.sha256
        || string(resource, "detectedFormat")? != entry.detected_format
    {
        return Err(fail("catalog-resource-mismatch", entry.index.to_string()));
    }
    Ok(())
}

fn compare_membership(
    inventory: &Inventory,
    expected_files: &BTreeSet<String>,
    expected_directories: &BTreeSet<String>,
) -> Result<(), Failure> {
    if let Some(path) = expected_files
        .difference(&inventory.files.keys().cloned().collect())
        .next()
    {
        return Err(fail("missing-file", path));
    }
    if let Some(path) = inventory
        .files
        .keys()
        .find(|path| !expected_files.contains(*path))
    {
        return Err(fail("extra-file", path));
    }
    if let Some(path) = expected_directories
        .difference(&inventory.directories)
        .next()
    {
        return Err(fail("missing-directory", path));
    }
    if let Some(path) = inventory
        .directories
        .iter()
        .find(|path| !expected_directories.contains(*path))
    {
        return Err(fail("extra-directory", path));
    }
    Ok(())
}

fn read_source(path: &Path) -> Result<(PathBuf, Vec<u8>), Failure> {
    reject_link_if_present(path, "source")?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        fail(
            "source-metadata-failed",
            format!("{}: {error}", path.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(fail("source-not-regular-file", path.display().to_string()));
    }
    let bytes = read_capped(&path.display().to_string())?;
    Ok((path.to_path_buf(), bytes))
}

fn slash_relative(root: &Path, path: &Path) -> Result<String, Failure> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| fail("member-outside-root", path.display().to_string()))?;
    relative
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| fail("non-utf8-path", path.display().to_string()))
}

fn join_relative(prefix: &str, relative: &str) -> String {
    if prefix.is_empty() {
        relative.to_string()
    } else {
        format!("{prefix}/{relative}")
    }
}

fn object<'a>(value: &'a Value, key: &str) -> Result<&'a Value, Failure> {
    value
        .get(key)
        .filter(|value| value.is_object())
        .ok_or_else(|| fail("manifest-semantic-error", format!("{key} is not an object")))
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, Failure> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| fail("manifest-semantic-error", format!("{key} is not an array")))
}

fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, Failure> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| fail("manifest-semantic-error", format!("{key} is not a string")))
}

fn integer(value: &Value, key: &str) -> Result<u64, Failure> {
    value.get(key).and_then(Value::as_u64).ok_or_else(|| {
        fail(
            "manifest-semantic-error",
            format!("{key} is not an unsigned integer"),
        )
    })
}

fn checked_add(left: u64, right: u64, code: &str) -> Result<u64, Failure> {
    left.checked_add(right)
        .ok_or_else(|| fail(code, "u64 overflow"))
}

fn fail(code: &str, detail: impl std::fmt::Display) -> Failure {
    Failure::usage(format!("{code}: {detail}"))
}

fn usage() -> &'static str {
    "usage: xivl verify-extraction <directory> [--source <file> | --catalog <catalog.json|catalog.jsonl> --root <directory>] [--report json]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use xivl_formats::to_canonical_json;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "xivl-verify-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn single_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let work = temp_root(name);
        fs::create_dir_all(&work).unwrap();
        let source = work.join("source.DAT");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--materialize-payloads".into(),
            "--as".into(),
            "sedb".into(),
        ])
        .unwrap();
        (work, source, output)
    }

    fn verify_arguments(output: &Path, source: Option<&Path>) -> Vec<String> {
        let mut arguments = vec![output.display().to_string()];
        if let Some(source) = source {
            arguments.extend(["--source".into(), source.display().to_string()]);
        }
        arguments
    }

    fn manifest(output: &Path) -> Value {
        serde_yaml::from_str(&fs::read_to_string(output.join("extraction.yaml")).unwrap()).unwrap()
    }

    fn write_manifest(output: &Path, document: &Value) {
        fs::write(
            output.join("extraction.yaml"),
            serde_yaml::to_string(document).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn verifies_single_with_replay_and_json_report_without_writing() {
        let (work, source, output) = single_fixture("single-positive");
        let before = inventory(&output).unwrap();
        let text = run(&verify_arguments(&output, Some(&source))).unwrap();
        assert!(text.text.contains("verified resource extraction"));
        let mut json_arguments = verify_arguments(&output, Some(&source));
        json_arguments.extend(["--report".into(), "json".into()]);
        let report: Value = serde_json::from_str(&run(&json_arguments).unwrap().text).unwrap();
        assert_eq!(report["status"], "verified");
        assert_eq!(report["sourceReplay"], true);
        let after = inventory(&output).unwrap();
        assert_eq!(
            before
                .files
                .iter()
                .map(|(path, file)| (path, (&file.sha256, file.size)))
                .collect::<Vec<_>>(),
            after
                .files
                .iter()
                .map(|(path, file)| (path, (&file.sha256, file.size)))
                .collect::<Vec<_>>()
        );
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn replays_decoded_xml_exactly_and_uses_the_recorded_sqwt_name() {
        let work = temp_root("decoded-document-replay");
        fs::create_dir_all(&work).unwrap();
        let source = work.join("window.bin");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/sqwt/window.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--as".into(),
            "sqwt".into(),
        ])
        .unwrap();
        assert!(run(&verify_arguments(&output, Some(&source))).is_ok());

        let renamed = work.join("renamed.bin");
        fs::copy(&source, &renamed).unwrap();
        assert!(run(&verify_arguments(&output, Some(&renamed)))
            .unwrap_err()
            .message
            .contains("stale-source-name"));

        let mut changed = manifest(&output);
        let payload_path = changed["payloads"][0]["path"].as_str().unwrap().to_string();
        let mut payload = fs::read(output.join(&payload_path)).unwrap();
        payload[0] ^= 1;
        fs::write(output.join(&payload_path), &payload).unwrap();
        changed["payloads"][0]["sha256"] = json!(sha256_hex(&payload));
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, Some(&source)))
            .unwrap_err()
            .message
            .contains("payload-replay-mismatch"));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn replays_scrambled_decoded_xml_without_the_trailer() {
        let work = temp_root("scrambled-document-replay");
        fs::create_dir_all(&work).unwrap();
        let source = work.join("residue-1.bin");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/scrambled/residue-1.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--as".into(),
            "scrambled-xml".into(),
        ])
        .unwrap();
        assert!(run(&verify_arguments(&output, Some(&source))).is_ok());
        let payload = fs::read(output.join("payloads/decoded.xml")).unwrap();
        assert!(!payload.ends_with(&[xivl_formats::scrambled::TRAILER]));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn rejects_changed_missing_extra_and_aliased_payloads() {
        let (work, _, output) = single_fixture("payload-failures");
        let payload = manifest(&output)["payloads"][0]["path"]
            .as_str()
            .unwrap()
            .to_string();
        fs::write(output.join(&payload), b"changed").unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("payload-size-mismatch"));
        fs::remove_dir_all(work).unwrap();

        let (work, _, output) = single_fixture("missing");
        let payload = manifest(&output)["payloads"][0]["path"]
            .as_str()
            .unwrap()
            .to_string();
        fs::remove_file(output.join(&payload)).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("missing-file"));
        fs::remove_dir_all(work).unwrap();

        let (work, _, output) = single_fixture("extra");
        fs::write(output.join("extra.bin"), b"extra").unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("extra-file"));
        fs::remove_dir_all(work).unwrap();

        let (work, _, output) = single_fixture("hardlink");
        let payload = manifest(&output)["payloads"][0]["path"]
            .as_str()
            .unwrap()
            .to_string();
        fs::hard_link(output.join(&payload), output.join("alias.bin")).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("file-alias-refused"));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn rejects_schema_version_relationship_and_stale_source_changes() {
        let (work, source, output) = single_fixture("manifest-failures");
        let original = manifest(&output);
        let mut changed = original.clone();
        changed["schemaVersion"] = json!(2);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("unsupported-manifest-schema"));

        let mut changed = original.clone();
        changed.as_object_mut().unwrap().remove("tool");
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("schema-validation-failed"));

        let mut changed = original.clone();
        changed["payloads"][0]["entry"]["path"] = json!("$.parsed.root.entries[99]");
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("relationship-target-missing"));

        write_manifest(&output, &original);
        let mut source_bytes = fs::read(&source).unwrap();
        *source_bytes.last_mut().unwrap() ^= 1;
        fs::write(&source, source_bytes).unwrap();
        assert!(run(&verify_arguments(&output, Some(&source)))
            .unwrap_err()
            .message
            .contains("stale-source-sha256"));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn rejects_ambiguous_manifest_and_incompatible_replay_options() {
        let (work, source, output) = single_fixture("ambiguity");
        fs::write(output.join("extraction.json"), b"{}").unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("ambiguous-manifest"));
        fs::remove_file(output.join("extraction.json")).unwrap();
        let arguments = vec![
            output.display().to_string(),
            "--catalog".into(),
            source.display().to_string(),
            "--root".into(),
            work.display().to_string(),
        ];
        assert!(run(&arguments)
            .unwrap_err()
            .message
            .contains("incompatible-option"));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn auto_detects_json_single_and_batch_manifests() {
        let (work, source, _) = single_fixture("json-single");
        let single = work.join("json-output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            single.display().to_string(),
            "--format".into(),
            "json".into(),
            "--as".into(),
            "sedb".into(),
        ])
        .unwrap();
        assert!(run(&verify_arguments(&single, Some(&source)))
            .unwrap()
            .text
            .contains("verified resource extraction"));
        fs::remove_dir_all(work).unwrap();

        let (work, root, catalog, _) = batch_fixture("json-batch");
        let batch = work.join("json-output");
        crate::batch_extract::run(&[
            catalog.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--output".into(),
            batch.display().to_string(),
            "--id".into(),
            "0x12345678".into(),
            "--format".into(),
            "json".into(),
        ])
        .unwrap();
        assert!(run(&[batch.display().to_string()])
            .unwrap()
            .text
            .contains("verified catalog extraction"));
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn replays_lpb_decoding() {
        let work = temp_root("lpb");
        fs::create_dir_all(&work).unwrap();
        let source = work.join("script.lpb");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/lpb/raw.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--as".into(),
            "lpb".into(),
        ])
        .unwrap();
        assert!(run(&verify_arguments(&output, Some(&source)))
            .unwrap()
            .text
            .contains("source replayed"));
        fs::remove_dir_all(work).unwrap();
    }

    fn gtex_dds_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let work = temp_root(name);
        fs::create_dir_all(&work).unwrap();
        let source = work.join("texture.DAT");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/gtex/tagged.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--export-dds".into(),
        ])
        .unwrap();
        (work, source, output)
    }

    fn gtex_png_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let work = temp_root(name);
        fs::create_dir_all(&work).unwrap();
        let source = work.join("texture.DAT");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/gtex/tagged.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--preview-png".into(),
        ])
        .unwrap();
        (work, source, output)
    }

    #[test]
    fn rejects_changed_missing_and_metadata_mismatched_png_previews() {
        let (work, source, output) = gtex_png_fixture("png-verifier-mutations");
        let original = manifest(&output);
        let preview_path = output.join("payloads/preview.png");
        let preview_index = original["payloads"]
            .as_array()
            .unwrap()
            .iter()
            .position(|payload| payload["role"] == "gtex-top-mip-png-preview")
            .unwrap();
        let original_bytes = fs::read(&preview_path).unwrap();
        let mut altered = fs::read(&preview_path).unwrap();
        *altered.last_mut().unwrap() ^= 1;
        fs::write(&preview_path, &altered).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("payload-sha256-mismatch"));
        fs::write(&preview_path, original_bytes).unwrap();
        let mut changed = original.clone();
        changed["payloads"][preview_index]["png"]["rgbaSha256"] =
            json!("0000000000000000000000000000000000000000000000000000000000000000");
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("png-metadata-mismatch"));
        write_manifest(&output, &original);
        fs::remove_file(&preview_path).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("missing-file"));
        assert!(run(&verify_arguments(&output, Some(&source))).is_err());
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn rejects_malformed_png_source_geometry_without_panicking() {
        let (work, _, output) = gtex_png_fixture("png-geometry-mutations");
        let original = manifest(&output);
        type Mutation = (&'static str, fn(&mut Value), &'static str);
        let cases: [Mutation; 6] = [
            (
                "mapping block bytes",
                |document: &mut Value| {
                    document["parsed"]["texture"]["formatIndex"]["mapping"]["blockBytes"] =
                        json!(99);
                },
                "png-format-mismatch",
            ),
            (
                "offset field span length",
                |document: &mut Value| {
                    document["parsed"]["offsetTable"]["entries"][0]["offsetField"]["span"]
                        ["length"] = json!(0);
                },
                "png-gtex-layout",
            ),
            (
                "size field span length",
                |document: &mut Value| {
                    document["parsed"]["offsetTable"]["entries"][0]["sizeField"]["span"]
                        ["length"] = json!(0);
                },
                "png-gtex-layout",
            ),
            (
                "oversized dimensions",
                |document: &mut Value| {
                    document["parsed"]["texture"]["width"] = json!({"value": u64::from(u32::MAX)});
                    document["parsed"]["texture"]["height"] = json!({"value": u64::from(u32::MAX)});
                },
                "png-gtex-eligibility",
            ),
            (
                "size field value",
                |document: &mut Value| {
                    document["parsed"]["offsetTable"]["entries"][0]["sizeField"]["value"] =
                        json!(1);
                },
                "png-gtex-layout",
            ),
            (
                "preview source offset",
                |document: &mut Value| {
                    document["payloads"][0]["png"]["sourceSpan"]["offset"] = json!(1);
                },
                "png-source-span-mismatch",
            ),
        ];
        for (name, mutate, expected) in cases {
            let mut changed = original.clone();
            mutate(&mut changed);
            write_manifest(&output, &changed);
            let error = run(&verify_arguments(&output, None)).unwrap_err();
            assert!(
                error.message.contains(expected),
                "{name}: {}",
                error.message
            );
        }
        write_manifest(&output, &original);
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn rejects_inconsistent_dds_metadata_and_artifacts_without_source() {
        let (work, _, output) = gtex_dds_fixture("dds-verifier-mutations");
        let original = manifest(&output);

        let mut changed = original.clone();
        changed["parsed"]["offsetTable"]["entries"][0]["source"]["span"]["length"] = json!(31);
        changed["parsed"]["offsetTable"]["entries"][0]["source"]["span"]["endExclusive"] =
            json!(71);
        changed["payloads"][0]["dds"]["mips"][0]["sourceSpan"]["length"] = json!(31);
        changed["payloads"][0]["dds"]["mips"][0]["sourceSpan"]["endExclusive"] = json!(71);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["offsetTable"]["entries"][0]["sizeField"]["value"] = json!(1);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["dataBase"]["value"] = json!(0);
        changed["parsed"]["offsetTable"]["entries"][0]["source"]["span"]["offset"] = json!(0);
        changed["payloads"][0]["dds"]["mips"][0]["sourceSpan"]["offset"] = json!(0);
        changed["payloads"][0]["dds"]["mips"][0]["sourceSpan"]["endExclusive"] = json!(32);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["offsetTable"]["entries"][1]["offsetField"]["value"] = json!(8);
        changed["parsed"]["offsetTable"]["entries"][1]["source"]["span"]["offset"] = json!(48);
        changed["payloads"][0]["dds"]["mips"][1]["sourceSpan"]["offset"] = json!(48);
        changed["payloads"][0]["dds"]["mips"][1]["sourceSpan"]["endExclusive"] = json!(56);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["input"]["length"] = json!(79);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["offsetTable"]["entries"][0]["offsetField"]["value"] = json!(1);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["offsetTable"]["base"]["value"] = json!(32);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["offsetTable"]["entries"][0]["calculatedSize"] = json!(1);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-layout"));

        let mut changed = original.clone();
        changed["parsed"]["texture"]["kind"] = json!("cube");
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-gtex-eligibility"));

        let mut changed = original.clone();
        changed["payloads"][0]["dds"]["format"]["d3dValue"] = json!(0x31545844u64);
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-format-mismatch"));

        let mut changed = original.clone();
        changed["payloads"][0]["role"] = json!("gtex-encoded-surface");
        write_manifest(&output, &changed);
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("dds-artifact-contract"));

        write_manifest(&output, &original);
        let dds_path = output.join("payloads/texture.dds");
        let dds_bytes = fs::read(&dds_path).unwrap();
        fs::remove_file(&dds_path).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("missing-file"));
        fs::write(&dds_path, &dds_bytes).unwrap();

        let mut altered = dds_bytes.clone();
        altered[128] ^= 1;
        fs::write(&dds_path, &altered).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("payload-sha256-mismatch"));
        fs::write(&dds_path, &dds_bytes).unwrap();

        fs::write(output.join("payloads/unlisted.dds"), b"extra").unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("extra-file"));
        fs::remove_file(output.join("payloads/unlisted.dds")).unwrap();

        let mut changed = original.clone();
        changed["payloads"][0]["sha256"] = json!(sha256_hex(&altered));
        changed["payloads"][0]["dds"]["mips"][0]["sha256"] = json!(sha256_hex(&altered[128..160]));
        changed["parsed"]["offsetTable"]["entries"][0]["source"]["sha256"] =
            json!(sha256_hex(&altered[128..160]));
        write_manifest(&output, &changed);
        fs::write(&dds_path, &altered).unwrap();
        assert!(
            run(&verify_arguments(&output, Some(&work.join("texture.DAT"))))
                .unwrap_err()
                .message
                .contains("payload-replay-mismatch")
        );
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn verifies_gapped_dds_with_and_without_source() {
        let work = temp_root("dds-gapped-verification");
        fs::create_dir_all(&work).unwrap();
        let source = work.join("texture.DAT");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/gtex/dds-dxt1-gapped.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--export-dds".into(),
        ])
        .unwrap();
        assert!(run(&verify_arguments(&output, None)).is_ok());
        assert!(run(&verify_arguments(&output, Some(&source))).is_ok());
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn verifies_gapped_png_with_and_without_source() {
        let work = temp_root("png-gapped-verification");
        fs::create_dir_all(&work).unwrap();
        let source = work.join("texture.DAT");
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/gtex/dds-dxt1-gapped.bin"),
        )
        .unwrap();
        let output = work.join("output");
        crate::resource_export::run(&[
            source.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--preview-png".into(),
        ])
        .unwrap();
        assert!(run(&verify_arguments(&output, None)).is_ok());
        assert!(run(&verify_arguments(&output, Some(&source))).is_ok());
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn verifies_gtex_catalog_extraction_with_exact_surface_payloads() {
        let work = temp_root("gtex-batch");
        let root = work.join("root");
        let source = root.join("data/12/34/56/78.DAT");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            include_bytes!("../../../tests/fixtures/public/gtex/tagged.bin"),
        )
        .unwrap();
        let catalog_output = work.join("catalog");
        let catalog_summary = crate::scan::run(&[
            root.display().to_string(),
            "--output".into(),
            catalog_output.display().to_string(),
        ])
        .unwrap();
        let catalog: Value =
            serde_json::from_str(&fs::read_to_string(&catalog_summary.output).unwrap()).unwrap();
        assert_eq!(catalog["resources"][0]["detectedFormat"], "gtex");
        assert_eq!(catalog["resources"][0]["formatStatus"], "parsed");
        assert_eq!(catalog["resources"][0]["supportStatus"], "partial");

        let output = work.join("output");
        crate::batch_extract::run(&[
            catalog_summary.output.clone(),
            "--root".into(),
            root.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--id".into(),
            "0x12345678".into(),
            "--materialize-payloads".into(),
            "--export-dds".into(),
        ])
        .unwrap();
        let report = run(&[
            output.display().to_string(),
            "--catalog".into(),
            catalog_summary.output,
            "--root".into(),
            root.display().to_string(),
            "--report".into(),
            "json".into(),
        ])
        .unwrap();
        let report: Value = serde_json::from_str(&report.text).unwrap();
        assert_eq!(report["sourceReplay"], true);
        assert_eq!(report["payloads"], 3);

        let batch: Value =
            serde_yaml::from_str(&fs::read_to_string(output.join("batch.yaml")).unwrap()).unwrap();
        assert_eq!(batch["exportDds"], true);
        let nested = output.join(batch["resources"][0]["manifest"].as_str().unwrap());
        let extraction: Value = serde_yaml::from_str(&fs::read_to_string(nested).unwrap()).unwrap();
        assert_eq!(extraction["format"]["id"], "gtex");
        assert_eq!(extraction["payloads"].as_array().unwrap().len(), 3);
        assert_eq!(extraction["payloads"][0]["role"], "gtex-encoded-surface");
        assert_eq!(extraction["payloads"][2]["role"], "gtex-dds-texture");
        assert_eq!(
            extraction["parsed"]["dataRegion"]["kind"],
            "texture-source-data"
        );
        fs::remove_dir_all(work).unwrap();
    }

    fn catalog_row(path: &str, bytes: &[u8]) -> Value {
        json!({
            "anomalies": [],
            "detectedFormat": "sedb",
            "formatStatus": "parsed",
            "resourceId": "0x12345678",
            "schemaVersion": 1,
            "sha256": sha256_hex(bytes),
            "size": bytes.len() as u64,
            "sourcePath": path,
            "spans": [],
            "supportStatus": "partial"
        })
    }

    fn batch_fixture(name: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let work = temp_root(name);
        let root = work.join("root");
        let relative = "data/12/34/56/78.DAT";
        let source = root.join("data/12/34/56/78.DAT");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let bytes = include_bytes!("../../../tests/fixtures/public/sedb/plain-container.bin");
        fs::write(&source, bytes).unwrap();
        let catalog = work.join("catalog.json");
        fs::write(
            &catalog,
            to_canonical_json(&json!({
                "resourceCount": 1,
                "resources": [catalog_row(relative, bytes)],
                "schemaVersion": 1
            })),
        )
        .unwrap();
        let output = work.join("output");
        crate::batch_extract::run(&[
            catalog.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--output".into(),
            output.display().to_string(),
            "--id".into(),
            "0x12345678".into(),
            "--materialize-payloads".into(),
        ])
        .unwrap();
        (work, root, catalog, output)
    }

    #[test]
    fn verifies_batch_internally_and_with_catalog_replay() {
        let (work, root, catalog, output) = batch_fixture("batch-positive");
        let internal = run(&[output.display().to_string()]).unwrap();
        assert!(internal.text.contains("verified catalog extraction"));
        let batch_path = output.join("batch.yaml");
        let mut legacy: Value =
            serde_yaml::from_str(&fs::read_to_string(&batch_path).unwrap()).unwrap();
        legacy.as_object_mut().unwrap().remove("exportDds");
        fs::write(&batch_path, serde_yaml::to_string(&legacy).unwrap()).unwrap();
        assert!(run(&[output.display().to_string()]).is_ok());
        let replayed = run(&[
            output.display().to_string(),
            "--catalog".into(),
            catalog.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--report".into(),
            "json".into(),
        ])
        .unwrap();
        let report: Value = serde_json::from_str(&replayed.text).unwrap();
        assert_eq!(report["resources"], 1);
        assert_eq!(report["sourceReplay"], true);
        fs::remove_dir_all(work).unwrap();
    }

    #[test]
    fn rejects_batch_totals_nested_changes_and_stale_catalog() {
        let (work, root, catalog, output) = batch_fixture("batch-failures");
        let batch_path = output.join("batch.yaml");
        let original: Value =
            serde_yaml::from_str(&fs::read_to_string(&batch_path).unwrap()).unwrap();
        let mut changed = original.clone();
        changed["totals"]["sourceBytes"] = json!(0);
        fs::write(&batch_path, serde_yaml::to_string(&changed).unwrap()).unwrap();
        assert!(run(&[output.display().to_string()])
            .unwrap_err()
            .message
            .contains("batch-totals-mismatch"));

        fs::write(&batch_path, serde_yaml::to_string(&original).unwrap()).unwrap();
        let directory = original["resources"][0]["outputDirectory"]
            .as_str()
            .unwrap();
        fs::write(output.join(directory).join("unlisted.bin"), b"extra").unwrap();
        assert!(run(&[output.display().to_string()])
            .unwrap_err()
            .message
            .contains("extra-file"));
        fs::remove_file(output.join(directory).join("unlisted.bin")).unwrap();

        fs::write(&catalog, b"{}").unwrap();
        assert!(run(&[
            output.display().to_string(),
            "--catalog".into(),
            catalog.display().to_string(),
            "--root".into(),
            root.display().to_string(),
        ])
        .unwrap_err()
        .message
        .contains("stale-catalog"));
        fs::remove_dir_all(work).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_case_collisions() {
        use std::os::unix::fs::symlink;

        let (work, _, output) = single_fixture("links");
        fs::write(output.join("outside"), b"outside").unwrap();
        symlink(output.join("outside"), output.join("linked")).unwrap();
        assert!(run(&verify_arguments(&output, None))
            .unwrap_err()
            .message
            .contains("link-or-reparse-point-refused"));
        fs::remove_dir_all(work).unwrap();

        let (work, _, output) = single_fixture("case");
        fs::write(output.join("EXTRA"), b"one").unwrap();
        fs::write(output.join("extra"), b"two").unwrap();
        let matching_entries = fs::read_dir(&output)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("extra")
            })
            .count();
        let message = run(&verify_arguments(&output, None)).unwrap_err().message;
        if matching_entries == 2 {
            assert!(message.contains("case-collision"));
        } else {
            assert_eq!(matching_entries, 1);
            assert!(message.contains("extra-file"));
        }
        fs::remove_dir_all(work).unwrap();
    }
}
