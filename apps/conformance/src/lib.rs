//! The conformance runner.
//!
//! Interface: `docs/conformance-tests.md`. The runner reads case manifests
//! from this checkout, resolves each fixture, runs the operation through
//! the format libraries, and compares against the expected normalized
//! document.
//!
//! Two rules shape the code. A private case whose bytes are not available
//! reports itself skipped with a reason and the run stays green, because
//! the bytes are the owner's and cannot be published. `--require-private`
//! turns that into a failure for the owner's own runs. And a run that
//! silently skips everything and exits zero is the outcome this interface
//! exists to prevent, so every skip is printed with its reason and counted.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use xivl_cli::ExtractFailure;
use xivl_formats::digest::sha256_hex;
use xivl_formats::{
    inspect_named_bytes_as, lua_path_document, resource_path_listing, to_canonical_json,
    validate_named_bytes_as, ErrorKind, FormatError, InspectAs,
};

/// Bounds allocation before parsing any fixture path.
pub const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

pub const CASE_DIR: &str = "tests/conformance/cases";
/// Retained for source compatibility with early runner integrations.
pub const ORACLE_DIR: &str = "tests/conformance/oracles";
pub const PRIVATE_MANIFEST: &str = "tests/fixtures/private-manifest.json";
pub const FIXTURE_ROOT_VARIABLE: &str = "XIVL_TOOLS_FIXTURE_ROOT";

/// The root a manifest entry resolves under when it names none.
///
/// One root was enough while every private fixture was a resource under the
/// client install. The configuration files are not there - the client keeps
/// them in the user's documents - so an entry may name the root it belongs
/// to, and the runner takes one directory per root.
pub const DEFAULT_FIXTURE_ROOT: &str = "client-install";

/// Environment variable for a named root: the default root keeps
/// [`FIXTURE_ROOT_VARIABLE`], and any other appends its own id.
pub fn root_variable(root_id: &str) -> String {
    if root_id == DEFAULT_FIXTURE_ROOT {
        return FIXTURE_ROOT_VARIABLE.to_string();
    }
    format!(
        "{FIXTURE_ROOT_VARIABLE}_{}",
        root_id.to_uppercase().replace('-', "_")
    )
}

/// What the runner was asked to do.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub repo_root: PathBuf,
    pub cases: Vec<String>,
    pub formats: Vec<String>,
    /// One directory per fixture root id. A bare `--fixture-root <dir>`
    /// sets [`DEFAULT_FIXTURE_ROOT`].
    pub fixture_roots: BTreeMap<String, PathBuf>,
    pub require_private: bool,
    /// Retained for source compatibility; the case schema has no oracle
    /// records and the runner does not invoke external implementations.
    pub oracles: BTreeMap<String, PathBuf>,
    pub update_expected: bool,
}

/// How one case ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

#[derive(Debug, Clone)]
pub struct CaseResult {
    pub id: String,
    pub format_id: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub results: Vec<CaseResult>,
    /// Retained for source compatibility; it is always empty because oracle
    /// cases are not part of the conformance schema.
    pub oracle_skips: Vec<String>,
}

impl Report {
    pub fn passed(&self) -> usize {
        self.count(|outcome| matches!(outcome, Outcome::Passed))
    }

    pub fn failed(&self) -> usize {
        self.count(|outcome| matches!(outcome, Outcome::Failed(_)))
    }

    pub fn skipped(&self) -> usize {
        self.count(|outcome| matches!(outcome, Outcome::Skipped(_)))
    }

    pub fn is_success(&self) -> bool {
        !self.results.is_empty() && self.failed() == 0
    }

    fn count(&self, predicate: impl Fn(&Outcome) -> bool) -> usize {
        self.results
            .iter()
            .filter(|result| predicate(&result.outcome))
            .count()
    }
}

/// Discover, run, and report every case the options select.
pub fn run(options: &Options) -> std::io::Result<Report> {
    let mut report = Report::default();
    let manifest = load_private_manifest(&options.repo_root)?;

    for case_path in discover_cases(&options.repo_root)? {
        let case: Value = match read_json(&case_path) {
            Ok(value) => value,
            Err(error) => {
                report.results.push(CaseResult {
                    id: case_path.display().to_string(),
                    format_id: String::new(),
                    outcome: Outcome::Failed(format!("unreadable case manifest: {error}")),
                });
                continue;
            }
        };
        let id = string_field(&case, "id");
        let format_id = string_field(&case, "formatId");
        if !options.cases.is_empty() && !options.cases.contains(&id) {
            continue;
        }
        if !options.formats.is_empty() && !options.formats.contains(&format_id) {
            continue;
        }

        let directory = case_path
            .parent()
            .unwrap_or(&options.repo_root)
            .to_path_buf();
        let outcome = run_case(options, &case, &directory, &manifest);
        report.results.push(CaseResult {
            id,
            format_id,
            outcome,
        });
    }

    report.results.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(report)
}

fn run_case(
    options: &Options,
    case: &Value,
    directory: &Path,
    manifest: &BTreeMap<String, PrivateFixture>,
) -> Outcome {
    if string_field(case.get("fixture").unwrap_or(&Value::Null), "kind") == "public-tree" {
        return run_public_tree_case(options, case, directory);
    }
    // The fixture's own base name travels with its bytes: it is the key of
    // the SQEX container, so a case that renamed its fixture would be
    // reading a different file.
    let (input, name) = match resolve_fixture(options, case, manifest) {
        Ok(Resolution::Bytes(bytes, name)) => (bytes, name),
        Ok(Resolution::Skip(reason)) => return Outcome::Skipped(reason),
        Err(reason) => return Outcome::Failed(reason),
    };

    let operation = string_field(case, "operation");
    let produced = match operation.as_str() {
        "inspect" | "validate" | "extract" => {
            let arguments = case_arguments(case);
            let export_dds = arguments.iter().any(|argument| argument == "--export-dds");
            let preview_png = arguments.iter().any(|argument| argument == "--preview-png");
            let mut pwib_entry = None;
            let mut inspect_arguments = Vec::new();
            let mut argument_index = 0;
            while argument_index < arguments.len() {
                match arguments[argument_index].as_str() {
                    "--export-dds" | "--preview-png" => argument_index += 1,
                    "--pwib-entry" => {
                        let Some(value) = arguments.get(argument_index + 1) else {
                            return Outcome::Failed(
                                "case arguments: --pwib-entry needs an index".into(),
                            );
                        };
                        pwib_entry = match value.parse::<u32>() {
                            Ok(value) => Some(value),
                            Err(_) => {
                                return Outcome::Failed(
                                    "case arguments: --pwib-entry needs an unsigned index".into(),
                                )
                            }
                        };
                        argument_index += 2;
                    }
                    _ => {
                        inspect_arguments.push(arguments[argument_index].clone());
                        argument_index += 1;
                    }
                }
            }
            match InspectAs::from_arguments(&inspect_arguments) {
                Ok(how) => {
                    if operation == "inspect" {
                        inspect_named_bytes_as(&input, &name, &how)
                    } else if operation == "extract" {
                        if let Some(index) = pwib_entry {
                            pwib_selected_export_document(
                                &input,
                                &name,
                                index,
                                export_dds,
                                preview_png,
                            )
                        } else if export_dds {
                            dds_export_document(&input, &name, &how)
                        } else if preview_png {
                            png_preview_document(&input, &name, &how)
                        } else {
                            let is_document =
                                matches!(&how, InspectAs::Sqwt | InspectAs::ScrambledXml)
                                    || matches!(
                                        &how,
                                        InspectAs::Auto
                                            if xivl_formats::sqwt::has_signature(&input)
                                                || xivl_formats::scrambled::has_signature(&input)
                                    );
                            if is_document {
                                decoded_document_export(&input, &name, &how)
                            } else {
                                xivl_formats::export_sheet_data(&input, &how)
                            }
                        }
                    } else {
                        validate_named_bytes_as(&input, &name, &how)
                    }
                }
                Err(reason) => return Outcome::Failed(format!("case arguments: {reason}")),
            }
        }
        "resource-path" => match std::str::from_utf8(&input) {
            Ok(text) => resource_path_listing(text),
            Err(error) => {
                return Outcome::Failed(format!("resource-path fixture is not UTF-8: {error}"))
            }
        },
        "lua-path" => match std::str::from_utf8(&input) {
            Ok(text) => lua_path_document(text),
            Err(error) => Err(FormatError::new(
                ErrorKind::InvalidUtf8,
                error.valid_up_to() as u64,
                "Lua path fixture is not UTF-8",
            )),
        },
        other => {
            return Outcome::Failed(format!(
                "operation '{other}' is named in the case schema but the runner does \
                 not implement it, so this case verifies nothing; implement the \
                 operation or remove the case"
            ));
        }
    };

    let expect = case.get("expect").cloned().unwrap_or(Value::Null);
    let expected_outcome = string_field(&expect, "outcome");
    match (expected_outcome.as_str(), produced) {
        ("ok", Ok(mut document)) => {
            if string_field(&case["fixture"], "kind") == "private"
                && string_field(&document, "format") == "pwib"
            {
                if let Some(selection) = document.get_mut("selection") {
                    normalize_private_pwib_names(selection);
                }
            }
            compare_expected(options, directory, &expect, document)
        }
        ("ok", Err(error)) => Outcome::Failed(format!("expected success, got {error}")),
        ("parse-error", Ok(_)) => Outcome::Failed(format!(
            "expected error kind '{}', the input parsed cleanly",
            string_field(&expect, "errorKind")
        )),
        ("parse-error", Err(error)) => compare_error(&expect, &error),
        (other, _) => Outcome::Failed(format!("unknown expected outcome '{other}'")),
    }
}

fn run_public_tree_case(options: &Options, case: &Value, directory: &Path) -> Outcome {
    if string_field(case, "operation") != "extract-directory" {
        return Outcome::Failed("public-tree fixtures require extract-directory".into());
    }
    let fixture = case.get("fixture").cloned().unwrap_or(Value::Null);
    if !case_arguments(case).is_empty() {
        return Outcome::Failed("extract-directory does not accept arguments".into());
    }
    let relative = string_field(&fixture, "path");
    let descriptor_path = options.repo_root.join(&relative);
    let descriptor = match read_json(&descriptor_path) {
        Ok(value) => value,
        Err(error) => return Outcome::Failed(format!("cannot read public tree: {error}")),
    };
    let scratch = match make_tree_scratch() {
        Ok(path) => path,
        Err(reason) => return Outcome::Failed(reason),
    };
    let result = materialize_tree(
        &descriptor,
        descriptor_path.parent().unwrap_or(Path::new(".")),
        &scratch,
    )
    .map_err(TreeRunError::Setup)
    .and_then(|()| {
        let game = scratch.join("game");
        let output = scratch.join("output");
        xivl_cli::extract::extract_directory(&game, &output)
            .map_err(TreeRunError::Extract)
            .and_then(|summary| {
                directory_extract_document(&summary, &output).map_err(TreeRunError::Output)
            })
    });
    let _ = std::fs::remove_dir_all(&scratch);

    let expect = case.get("expect").cloned().unwrap_or(Value::Null);
    let expected_outcome = string_field(&expect, "outcome");
    match (expected_outcome.as_str(), result) {
        ("ok", Ok(document)) => compare_expected(options, directory, &expect, document),
        ("ok", Err(error)) => Outcome::Failed(format!(
            "expected success, got {}",
            tree_error_message(&error)
        )),
        ("parse-error", Ok(_)) => Outcome::Failed(format!(
            "expected error kind '{}', the tree extracted cleanly",
            string_field(&expect, "errorKind")
        )),
        ("parse-error", Err(TreeRunError::Extract(failure))) => {
            let wanted = string_field(&expect, "errorKind");
            if !failure.kind().is_parse() {
                return Outcome::Failed("expected parse error, got setup or I/O failure".into());
            }
            if expect.get("errorOffset").is_some() {
                return Outcome::Failed(
                    "extract-directory errors do not expose errorOffset".into(),
                );
            }
            if wanted == failure.kind().as_str() {
                Outcome::Passed
            } else {
                Outcome::Failed(format!(
                    "expected error kind '{wanted}', got '{}'",
                    failure.kind().as_str()
                ))
            }
        }
        ("parse-error", Err(error)) => Outcome::Failed(format!(
            "expected parse error, got {}",
            tree_error_message(&error)
        )),
        (other, _) => Outcome::Failed(format!("unknown expected outcome '{other}'")),
    }
}

enum TreeRunError {
    Setup(String),
    Extract(ExtractFailure),
    Output(String),
}

fn tree_error_message(error: &TreeRunError) -> String {
    match error {
        TreeRunError::Setup(reason) => format!("setup failure: {reason}"),
        TreeRunError::Extract(failure) => failure.kind().as_str().to_string(),
        TreeRunError::Output(reason) => format!("output failure: {reason}"),
    }
}

fn make_tree_scratch() -> Result<PathBuf, String> {
    let base = std::env::temp_dir();
    let process = std::process::id();
    for index in 0..100u32 {
        let candidate = base.join(format!("xivl-conformance-ssd-{process}-{index}"));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("cannot create conformance tree scratch".into()),
        }
    }
    Err("cannot allocate conformance tree scratch".into())
}

fn materialize_tree(descriptor: &Value, fixture_root: &Path, scratch: &Path) -> Result<(), String> {
    let descriptor = descriptor
        .as_object()
        .ok_or_else(|| "invalid ssd-extract tree descriptor".to_string())?;
    if descriptor.len() != 3
        || !descriptor.contains_key("schemaVersion")
        || !descriptor.contains_key("format")
        || !descriptor.contains_key("resources")
        || descriptor.get("schemaVersion").and_then(Value::as_u64) != Some(1)
        || descriptor.get("format").and_then(Value::as_str) != Some("ssd-extract")
    {
        return Err("invalid ssd-extract tree descriptor".into());
    }
    let resources = descriptor
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| "invalid ssd-extract tree resources".to_string())?;
    if resources.is_empty() {
        return Err("invalid ssd-extract tree resources".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut files = std::collections::BTreeSet::new();
    for resource in resources {
        let resource = resource
            .as_object()
            .ok_or_else(|| "invalid ssd-extract resource".to_string())?;
        if resource.len() != 2 || !resource.contains_key("id") || !resource.contains_key("file") {
            return Err("invalid ssd-extract resource properties".into());
        }
        let id_text = resource
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "invalid ssd-extract resource id".to_string())?;
        let id = xivl_formats::parse_resource_id(id_text, 0)
            .map_err(|_| "invalid ssd-extract resource id".to_string())?;
        if !ids.insert(id.value()) {
            return Err("duplicate ssd-extract resource id".into());
        }
        let source = resource
            .get("file")
            .and_then(Value::as_str)
            .ok_or_else(|| "invalid ssd-extract resource fixture path".to_string())?;
        let canonical_source = source
            .split('/')
            .filter(|component| !component.is_empty() && *component != ".")
            .collect::<Vec<_>>()
            .join("/");
        if !is_tree_fixture_path(source) || !files.insert(canonical_source) {
            return Err("invalid ssd-extract resource fixture path".into());
        }
        let source_path = Path::new(source);
        let bytes = read_capped(&fixture_root.join(source_path))
            .map_err(|_| "cannot read ssd-extract resource fixture".to_string())?;
        let destination = scratch.join("game").join(id.dat_path());
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| "cannot create ssd-extract resource directory".to_string())?;
        }
        std::fs::write(destination, bytes)
            .map_err(|_| "cannot materialize ssd-extract resource".to_string())?;
    }
    Ok(())
}

fn is_tree_fixture_path(source: &str) -> bool {
    !source.is_empty()
        && source.ends_with(".bin")
        && source.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '/' | '_' | '-')
        })
        && !source.starts_with('/')
        && !source.split('/').any(|component| component == "..")
}

fn directory_extract_document(
    summary: &xivl_cli::extract::ExtractSummary,
    output: &Path,
) -> Result<Value, String> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(output)
        .map_err(|error| format!("cannot inventory extraction output: {error}"))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("cannot inspect extraction output: {error}"))?
            .path();
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("cannot inspect extraction output: {error}"))?;
        if !metadata.is_file() || path.extension().and_then(|value| value.to_str()) != Some("csv") {
            continue;
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("cannot read extraction output: {error}"))?;
        files.push(json!({
            "path": path.file_name().and_then(|value| value.to_str()).unwrap_or_default(),
            "sha256": sha256_hex(&bytes),
            "size": bytes.len() as u64,
        }));
    }
    if summary.files != files.len() {
        return Err("extraction summary/file count mismatch".into());
    }
    files.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    Ok(json!({
        "format": "ssd-sheet",
        "files": files,
        "operation": "extract-directory",
        "summary": {
            "absentBlocks": summary.absent_blocks,
            "conflictingValues": summary.conflicting_values,
            "documents": summary.documents,
            "files": summary.files,
            "missingTrailingValues": summary.missing_trailing_values,
            "rows": summary.rows,
        },
    }))
}

fn png_preview_document(input: &[u8], name: &str, how: &InspectAs) -> Result<Value, FormatError> {
    if !matches!(how, InspectAs::Gtex | InspectAs::Auto) {
        return Err(FormatError::new(
            ErrorKind::UnsupportedGtexPreview,
            0,
            "PNG preview requires a GTEX input",
        ));
    }
    inspect_named_bytes_as(input, name, how)?;
    let parsed =
        xivl_formats::gtex_pwib::parse(input, xivl_formats::gtex_pwib::TaggedResourceKind::Gtex)?;
    let xivl_formats::gtex_pwib::TaggedResource::Gtex(gtex) = parsed else {
        unreachable!("GTEX parser returns GTEX");
    };
    let preview = xivl_formats::texture_preview::export_gtex_top_mip_png(input, &gtex)?;
    Ok(json!({
        "format": "gtex",
        "height": preview.height,
        "mipLevel": preview.mip_level,
        "operation": "extract",
        "png": {
            "format": {
                "clientIndex": preview.format.index,
                "d3dName": preview.format.d3d_name,
                "d3dValue": preview.format.d3d_value,
            },
            "height": preview.height,
            "mipLevel": preview.mip_level,
            "rgbaSha256": preview.rgba_sha256,
            "sourceSha256": preview.source_sha256,
            "sourceSpan": preview.source_span.to_json(),
            "width": preview.width,
        },
        "path": "payloads/preview.png",
        "role": "gtex-top-mip-png-preview",
        "sha256": sha256_hex(&preview.bytes),
        "size": preview.bytes.len() as u64,
        "texture": {
            "format": {
                "clientIndex": preview.format.index,
                "d3dName": preview.format.d3d_name,
                "d3dValue": preview.format.d3d_value,
            },
            "height": preview.height,
            "width": preview.width,
        },
        "width": preview.width,
    }))
}

fn dds_export_document(input: &[u8], name: &str, how: &InspectAs) -> Result<Value, FormatError> {
    if !matches!(how, InspectAs::Gtex | InspectAs::Auto) {
        return Err(FormatError::new(
            ErrorKind::UnsupportedDdsFormat,
            0,
            "DDS export requires a GTEX input",
        ));
    }
    inspect_named_bytes_as(input, name, how)?;
    let parsed =
        xivl_formats::gtex_pwib::parse(input, xivl_formats::gtex_pwib::TaggedResourceKind::Gtex)?;
    let xivl_formats::gtex_pwib::TaggedResource::Gtex(gtex) = parsed else {
        unreachable!("GTEX parser returns GTEX");
    };
    let export = xivl_formats::dds::export_gtex(input, &gtex)?;
    Ok(json!({
        "format": "gtex",
        "mipLevels": export.mip_levels,
        "mips": export.mips.iter().map(|mip| json!({
            "ddsSpan": mip.dds_span.to_json(),
            "height": mip.height,
            "mipLevel": mip.mip_level,
            "sha256": mip.sha256,
            "sourceSpan": mip.source_span.to_json(),
            "width": mip.width,
        })).collect::<Vec<_>>(),
        "operation": "extract",
        "path": "payloads/texture.dds",
        "role": "gtex-dds-texture",
        "sha256": sha256_hex(&export.bytes),
        "size": export.bytes.len() as u64,
        "texture": {
            "format": {
                "clientIndex": export.format.index,
                "d3dName": export.format.d3d_name,
                "d3dValue": export.format.d3d_value,
            },
            "height": export.height,
            "width": export.width,
        },
    }))
}

fn pwib_selected_export_document(
    input: &[u8],
    _name: &str,
    index: u32,
    export_dds: bool,
    preview_png: bool,
) -> Result<Value, FormatError> {
    let mut document = xivl_formats::inspect_selected_pwib(input, index)?;
    if !export_dds && !preview_png {
        return Ok(document);
    }
    let selection = xivl_formats::gtex_pwib::parse_selected_pwib(input, index)?;
    let second_start = usize::try_from(selection.pwib.second_offset).map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selection.pwib.second_offset as u64,
            "PWIB second segment offset does not fit this platform",
        )
    })?;
    let second_end = second_start
        .checked_add(selection.pwib.second_segment.length as usize)
        .ok_or_else(|| {
            FormatError::new(
                ErrorKind::InvalidPwibStructure,
                selection.pwib.second_offset as u64,
                "PWIB second segment end overflows",
            )
        })?;
    let second = input.get(second_start..second_end).ok_or_else(|| {
        FormatError::new(
            ErrorKind::InvalidPwibStructure,
            selection.pwib.second_offset as u64,
            "PWIB second segment escapes the input",
        )
    })?;
    let mut artifacts = Vec::new();
    if export_dds {
        let export = xivl_formats::dds::export_gtex_external(second, &selection.gtex)?;
        artifacts.push(json!({
            "format": "gtex",
            "mipLevels": export.mip_levels,
            "mips": export.mips.iter().map(|mip| json!({
                "ddsSpan": mip.dds_span.to_json(),
                "height": mip.height,
                "mipLevel": mip.mip_level,
                "sha256": mip.sha256,
                "sourceRelativeOffset": mip.source_span.offset,
                "sourceSpan": { "offset": selection.pwib.second_segment.offset + mip.source_span.offset, "length": mip.source_span.length, "endExclusive": selection.pwib.second_segment.offset + mip.source_span.offset + mip.source_span.length },
                "width": mip.width,
            })).collect::<Vec<_>>(),
            "path": "payloads/texture.dds",
            "role": "pwib-gtex-dds-texture",
            "sha256": sha256_hex(&export.bytes),
            "size": export.bytes.len() as u64,
            "texture": { "format": { "clientIndex": export.format.index, "d3dName": export.format.d3d_name, "d3dValue": export.format.d3d_value }, "height": export.height, "width": export.width },
        }));
    }
    if preview_png {
        let preview = xivl_formats::texture_preview::export_gtex_top_mip_png_from_source(
            second,
            &selection.gtex,
        )?;
        artifacts.push(json!({
            "format": "gtex",
            "height": preview.height,
            "mipLevel": preview.mip_level,
            "path": "payloads/preview.png",
            "png": { "format": { "clientIndex": preview.format.index, "d3dName": preview.format.d3d_name, "d3dValue": preview.format.d3d_value }, "height": preview.height, "mipLevel": preview.mip_level, "rgbaSha256": preview.rgba_sha256, "sourceSha256": preview.source_sha256, "sourceSpan": { "offset": selection.pwib.second_segment.offset + preview.source_span.offset, "length": preview.source_span.length, "endExclusive": selection.pwib.second_segment.offset + preview.source_span.offset + preview.source_span.length }, "width": preview.width },
            "role": "pwib-gtex-png-preview",
            "sha256": sha256_hex(&preview.bytes),
            "size": preview.bytes.len() as u64,
            "texture": { "format": { "clientIndex": preview.format.index, "d3dName": preview.format.d3d_name, "d3dValue": preview.format.d3d_value }, "height": preview.height, "width": preview.width },
            "width": preview.width,
        }));
    }
    document
        .as_object_mut()
        .expect("selected PWIB inspection is an object")
        .insert("artifacts".into(), Value::Array(artifacts));
    Ok(document)
}

// Private expectations retain entry identity without publishing decoded names.
fn normalize_private_pwib_names(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(name) = object
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                object.remove("name");
                object.insert("nameByteLength".into(), json!(name.len()));
                object.insert("nameSha256".into(), json!(sha256_hex(name.as_bytes())));
            }
            for child in object.values_mut() {
                normalize_private_pwib_names(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_private_pwib_names(item);
            }
        }
        _ => {}
    }
}

/// Report the safe, normalized identity of a decoded XML export. The
/// document bytes themselves stay in the CLI extraction payload; a public
/// conformance expectation records only the path, role, length, and digest.
fn decoded_document_export(
    input: &[u8],
    name: &str,
    how: &InspectAs,
) -> Result<Value, FormatError> {
    let format = match how {
        InspectAs::Sqwt => "sqwt",
        InspectAs::ScrambledXml => "scrambled-xml",
        InspectAs::Auto if xivl_formats::sqwt::has_signature(input) => "sqwt",
        InspectAs::Auto if xivl_formats::scrambled::has_signature(input) => "scrambled-xml",
        _ => {
            return Err(FormatError::new(
                ErrorKind::BadMagic,
                0,
                "decoded document export requires a SQEX or scrambled-XML input",
            ))
        }
    };
    // Match the CLI's accepted reader boundary before exposing an export
    // identity, including XML grammar and the SQEX filename key.
    inspect_named_bytes_as(input, name, how)?;
    let decoded = match format {
        "sqwt" => xivl_formats::sqwt::decode(input, name)?.document,
        "scrambled-xml" => xivl_formats::scrambled::decode(input)?.document,
        _ => unreachable!("format was selected above"),
    };
    Ok(json!({
        "format": format,
        "keyName": if format == "sqwt" { Value::String(name.to_string()) } else { Value::Null },
        "operation": "extract",
        "path": "payloads/decoded.xml",
        "role": "decoded-xml-document",
        "sha256": sha256_hex(&decoded),
        "size": decoded.len() as u64,
    }))
}

fn compare_expected(
    options: &Options,
    directory: &Path,
    expect: &Value,
    document: Value,
) -> Outcome {
    let name = string_field(expect, "output");
    if name.is_empty() {
        return Outcome::Failed("an 'ok' case needs an expected output file".into());
    }
    let path = directory.join(&name);

    if options.update_expected {
        let text = to_canonical_json(&document);
        return match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => Outcome::Passed,
            Err(error) => Outcome::Failed(format!("cannot write '{name}': {error}")),
        };
    }

    let expected: Value = match read_json(&path) {
        Ok(value) => value,
        Err(error) => return Outcome::Failed(format!("cannot read '{name}': {error}")),
    };

    if document == expected {
        return Outcome::Passed;
    }
    Outcome::Failed(first_difference(&expected, &document))
}

fn compare_error(expect: &Value, error: &FormatError) -> Outcome {
    let wanted = string_field(expect, "errorKind");
    if wanted != error.kind().as_str() {
        return Outcome::Failed(format!(
            "expected error kind '{wanted}', got '{}' at offset {}",
            error.kind(),
            error.offset()
        ));
    }
    if let Some(wanted_offset) = expect.get("errorOffset").and_then(Value::as_u64) {
        if wanted_offset != error.offset() {
            return Outcome::Failed(format!(
                "expected error '{wanted}' at offset {wanted_offset}, got offset {}",
                error.offset()
            ));
        }
    }
    Outcome::Passed
}

#[derive(Debug)]
enum Resolution {
    /// The fixture's bytes and its base name.
    Bytes(Vec<u8>, String),
    Skip(String),
}

/// The part of a fixture path after the last separator. Manifest paths are
/// written with forward slashes and a fixture root may add either, so both
/// are cut.
fn base_name(path: &str) -> String {
    match path.rfind(['/', '\\']) {
        Some(index) => path[index + 1..].to_string(),
        None => path.to_string(),
    }
}

#[derive(Debug, Clone)]
struct PrivateFixture {
    root: String,
    source_path: String,
    sha256: String,
    size: u64,
}

fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(std::io::Error::other(format!(
            "input is larger than the {MAX_INPUT_BYTES}-byte limit"
        )));
    }
    Ok(bytes)
}

fn resolve_fixture(
    options: &Options,
    case: &Value,
    manifest: &BTreeMap<String, PrivateFixture>,
) -> Result<Resolution, String> {
    let fixture = case.get("fixture").cloned().unwrap_or(Value::Null);
    match string_field(&fixture, "kind").as_str() {
        "public" => {
            let relative = string_field(&fixture, "path");
            let path = options.repo_root.join(&relative);
            read_capped(&path)
                .map(|bytes| Resolution::Bytes(bytes, base_name(&relative)))
                .map_err(|error| format!("cannot read public fixture '{relative}': {error}"))
        }
        "private" => {
            let fixture_id = string_field(&fixture, "fixtureId");
            let entry = manifest.get(&fixture_id).ok_or_else(|| {
                format!("private fixture '{fixture_id}' is not in {PRIVATE_MANIFEST}")
            })?;
            resolve_private(options, &fixture_id, entry)
        }
        other => Err(format!("unknown fixture kind '{other}'")),
    }
}

fn resolve_private(
    options: &Options,
    fixture_id: &str,
    entry: &PrivateFixture,
) -> Result<Resolution, String> {
    let Some(root) = options.fixture_roots.get(&entry.root) else {
        let reason = format!(
            "private fixture '{fixture_id}' needs the '{0}' fixture root; pass --fixture-root {1}<dir> or set {2}",
            entry.root,
            if entry.root == DEFAULT_FIXTURE_ROOT {
                String::new()
            } else {
                format!("{}=", entry.root)
            },
            root_variable(&entry.root)
        );
        if options.require_private {
            return Err(reason);
        }
        return Ok(Resolution::Skip(reason));
    };

    let path = root.join(&entry.source_path);
    let bytes = read_capped(&path).map_err(|error| {
        format!(
            "private fixture '{fixture_id}' ({}) is not readable under the supplied root: {error}",
            entry.source_path
        )
    })?;
    if bytes.len() as u64 != entry.size {
        return Err(format!(
            "private fixture '{fixture_id}' is {} byte(s), the manifest records {}",
            bytes.len(),
            entry.size
        ));
    }
    let digest = sha256_hex(&bytes);
    if digest != entry.sha256 {
        // The claim was established against a specific file, and this is
        // not that file. Failing loudly is the whole point of the hash.
        return Err(format!(
            "private fixture '{fixture_id}' hashes to {digest}, the manifest records {}",
            entry.sha256
        ));
    }
    Ok(Resolution::Bytes(bytes, base_name(&entry.source_path)))
}

fn load_private_manifest(repo_root: &Path) -> std::io::Result<BTreeMap<String, PrivateFixture>> {
    let path = repo_root.join(PRIVATE_MANIFEST);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let document: Value = read_json(&path)?;
    let mut manifest = BTreeMap::new();
    for entry in document
        .get("entries")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let root = match entry.get("root").and_then(Value::as_str) {
            Some(named) => named.to_string(),
            None => DEFAULT_FIXTURE_ROOT.to_string(),
        };
        manifest.insert(
            string_field(entry, "id"),
            PrivateFixture {
                root,
                source_path: string_field(entry, "sourcePath"),
                sha256: string_field(entry, "sha256"),
                size: entry.get("size").and_then(Value::as_u64).unwrap_or(0),
            },
        );
    }
    Ok(manifest)
}

fn discover_cases(repo_root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root = repo_root.join(CASE_DIR);
    let mut paths = Vec::new();
    if !root.is_dir() {
        return Ok(paths);
    }
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let manifest = entry.path().join("case.json");
        if manifest.is_file() {
            paths.push(manifest);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_json(path: &Path) -> std::io::Result<Value> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(std::io::Error::other)
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The case's `arguments`, which are the front-end options after the
/// operation and input. The runner and the command line parse them with
/// the same code, so a case cannot describe an invocation the tool does
/// not accept.
fn case_arguments(case: &Value) -> Vec<String> {
    case.get("arguments")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The first place two documents differ, as a JSON Pointer, so a failure
/// says where rather than dumping both documents.
fn first_difference(expected: &Value, produced: &Value) -> String {
    fn walk(expected: &Value, produced: &Value, pointer: &str) -> Option<String> {
        match (expected, produced) {
            (Value::Object(left), Value::Object(right)) => {
                let mut keys: Vec<&String> = left.keys().chain(right.keys()).collect();
                keys.sort();
                keys.dedup();
                for key in keys {
                    let child = format!("{pointer}/{key}");
                    match (left.get(key), right.get(key)) {
                        (Some(left_value), Some(right_value)) => {
                            if let Some(found) = walk(left_value, right_value, &child) {
                                return Some(found);
                            }
                        }
                        (Some(_), None) => return Some(format!("{child}: missing from output")),
                        (None, Some(_)) => return Some(format!("{child}: unexpected in output")),
                        (None, None) => {}
                    }
                }
                None
            }
            (Value::Array(left), Value::Array(right)) => {
                if left.len() != right.len() {
                    return Some(format!(
                        "{pointer}: expected {} item(s), got {}",
                        left.len(),
                        right.len()
                    ));
                }
                for (index, (left_value, right_value)) in left.iter().zip(right).enumerate() {
                    let child = format!("{pointer}/{index}");
                    if let Some(found) = walk(left_value, right_value, &child) {
                        return Some(found);
                    }
                }
                None
            }
            (left, right) if left == right => None,
            (left, right) => Some(format!("{pointer}: expected {left}, got {right}")),
        }
    }
    walk(expected, produced, "").unwrap_or_else(|| "documents differ".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_pwib_names_keep_only_lengths_and_digests() {
        let entry = json!({ "index": 6, "name": "authored-texture", "span": { "offset": 32, "length": 8 } });
        let mut selection = json!({
            "entries": [entry.clone()],
            "selectedEntry": entry,
            "resourceType": { "name": "RESOURCE_TYPE" },
            "resourceId": null,
            "descriptor": { "texture": { "kind": "2d" } }
        });
        normalize_private_pwib_names(&mut selection);
        for entry in [&selection["entries"][0], &selection["selectedEntry"]] {
            assert!(entry.get("name").is_none());
            assert_eq!(entry["nameByteLength"], 16);
            assert_eq!(entry["nameSha256"], sha256_hex(b"authored-texture"));
            assert_eq!(entry["index"], 6);
            assert_eq!(entry["span"], json!({ "offset": 32, "length": 8 }));
        }
        assert_eq!(selection["resourceType"]["nameByteLength"], 13);
        assert_eq!(
            selection["resourceType"]["nameSha256"],
            sha256_hex(b"RESOURCE_TYPE")
        );
        assert_eq!(selection["resourceId"], Value::Null);
        assert_eq!(selection["descriptor"]["texture"]["kind"], "2d");
        assert!(!selection.to_string().contains("authored-texture"));
        assert!(!selection.to_string().contains("RESOURCE_TYPE"));
    }

    fn fixture() -> PrivateFixture {
        PrivateFixture {
            root: DEFAULT_FIXTURE_ROOT.into(),
            source_path: "data/29/D9/00/01.DAT".into(),
            sha256: "0".repeat(64),
            size: 16,
        }
    }

    fn roots(pairs: &[(&str, &Path)]) -> BTreeMap<String, PathBuf> {
        pairs
            .iter()
            .map(|(id, path)| ((*id).to_string(), path.to_path_buf()))
            .collect()
    }

    #[test]
    fn a_private_case_skips_with_a_reason_when_no_root_is_supplied() {
        let options = Options::default();
        let resolution = resolve_private(&options, "example", &fixture()).unwrap();
        match resolution {
            Resolution::Skip(reason) => {
                assert!(reason.contains("needs the 'client-install'"), "{reason}");
                assert!(reason.contains(FIXTURE_ROOT_VARIABLE), "{reason}");
            }
            Resolution::Bytes(..) => panic!("a private fixture resolved without a root"),
        }
    }

    #[test]
    fn require_private_turns_that_skip_into_a_failure() {
        let options = Options {
            require_private: true,
            ..Options::default()
        };
        let error = resolve_private(&options, "example", &fixture()).unwrap_err();
        assert!(error.contains("needs the 'client-install'"), "{error}");
    }

    /// A fixture outside the client install resolves under its own root,
    /// and the install root does not stand in for it.
    #[test]
    fn a_named_root_is_resolved_separately_from_the_default_one() {
        let entry = PrivateFixture {
            root: "user-config".into(),
            source_path: "config.sys".into(),
            ..fixture()
        };
        let install = PathBuf::from("install");
        let options = Options {
            fixture_roots: roots(&[(DEFAULT_FIXTURE_ROOT, &install)]),
            ..Options::default()
        };
        match resolve_private(&options, "example", &entry).unwrap() {
            Resolution::Skip(reason) => {
                assert!(reason.contains("needs the 'user-config'"), "{reason}");
                assert!(reason.contains("--fixture-root user-config="), "{reason}");
                assert!(
                    reason.contains("XIVL_TOOLS_FIXTURE_ROOT_USER_CONFIG"),
                    "{reason}"
                );
            }
            Resolution::Bytes(..) => {
                panic!("a user-config fixture resolved under the install root")
            }
        }
        assert_eq!(root_variable(DEFAULT_FIXTURE_ROOT), FIXTURE_ROOT_VARIABLE);
    }

    #[test]
    fn a_hash_mismatch_fails_loudly() {
        let directory = std::env::temp_dir().join("xivl-conformance-hash-test");
        std::fs::create_dir_all(directory.join("data/29/D9/00")).unwrap();
        std::fs::write(directory.join("data/29/D9/00/01.DAT"), [0u8; 16]).unwrap();
        let options = Options {
            fixture_roots: roots(&[(DEFAULT_FIXTURE_ROOT, directory.as_path())]),
            ..Options::default()
        };
        let error = resolve_private(&options, "example", &fixture()).unwrap_err();
        assert!(error.contains("hashes to"), "{error}");
        std::fs::remove_dir_all(&directory).ok();
    }

    #[test]
    fn nested_differences_report_their_json_pointer() {
        let left = serde_json::json!({ "a": { "b": 1 } });
        let right = serde_json::json!({ "a": { "b": 2 } });
        assert_eq!(first_difference(&left, &right), "/a/b: expected 1, got 2");
    }

    #[test]
    fn an_unimplemented_operation_fails_rather_than_skips() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("apps/conformance sits two levels below the checkout root")
            .to_path_buf();
        let options = Options {
            repo_root: repo_root.clone(),
            ..Options::default()
        };
        let case = serde_json::json!({
            "operation": "unknown",
            "fixture": {
                "kind": "public",
                "path": "tests/fixtures/public/sedb/bad-magic.bin"
            },
            "expect": { "outcome": "ok", "output": "expected.json" }
        });
        let outcome = run_case(&options, &case, &repo_root, &BTreeMap::new());
        match outcome {
            Outcome::Failed(reason) => assert!(reason.contains("unknown"), "{reason}"),
            other => panic!("an unimplemented operation must fail, got {other:?}"),
        }
    }

    #[test]
    fn tree_materialization_rejects_boundary_paths_and_shape_drift() {
        let scratch = make_tree_scratch().unwrap();
        let invalid_paths = [
            "C:README.bin",
            "/tmp/README.bin",
            "../README.bin",
            "nested\\README.bin",
            "README.txt",
        ];
        for source in invalid_paths {
            let descriptor = json!({
                "schemaVersion": 1,
                "format": "ssd-extract",
                "resources": [{"id": "0x14000001", "file": source}],
            });
            let error = materialize_tree(&descriptor, Path::new("missing"), &scratch).unwrap_err();
            assert!(error.contains("fixture path"), "{source}: {error}");
            assert!(!scratch.join("game").exists(), "{source}: {error}");
        }

        let bool_version = json!({
            "schemaVersion": true,
            "format": "ssd-extract",
            "resources": [],
        });
        assert!(materialize_tree(&bool_version, Path::new("missing"), &scratch).is_err());

        let extra_root = json!({
            "schemaVersion": 1,
            "format": "ssd-extract",
            "resources": [],
            "root": "C:/outside",
        });
        assert!(materialize_tree(&extra_root, Path::new("missing"), &scratch).is_err());

        let fixture_root = scratch.join("fixtures");
        std::fs::create_dir_all(&fixture_root).unwrap();
        std::fs::write(fixture_root.join("a.bin"), [0u8]).unwrap();
        let duplicate_id = json!({
            "schemaVersion": 1,
            "format": "ssd-extract",
            "resources": [
                {"id": "0X14000001", "file": "a.bin"},
                {"id": "14000001", "file": "a.bin"},
            ],
        });
        assert!(materialize_tree(&duplicate_id, &fixture_root, &scratch).is_err());

        let duplicate_file = json!({
            "schemaVersion": 1,
            "format": "ssd-extract",
            "resources": [
                {"id": "14000001", "file": "a.bin"},
                {"id": "14000002", "file": "./a.bin"},
            ],
        });
        assert!(materialize_tree(&duplicate_file, &fixture_root, &scratch).is_err());
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
