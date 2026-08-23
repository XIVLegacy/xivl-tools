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

use serde_json::Value;
use sha2::{Digest, Sha256};
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
            match InspectAs::from_arguments(&case_arguments(case)) {
                Ok(how) => {
                    if operation == "inspect" {
                        inspect_named_bytes_as(&input, &name, &how)
                    } else if operation == "extract" {
                        xivl_formats::export_sheet_data(&input, &how)
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
        ("ok", Ok(document)) => compare_expected(options, directory, &expect, document),
        ("ok", Err(error)) => Outcome::Failed(format!("expected success, got {error}")),
        ("parse-error", Ok(_)) => Outcome::Failed(format!(
            "expected error kind '{}', the input parsed cleanly",
            string_field(&expect, "errorKind")
        )),
        ("parse-error", Err(error)) => compare_error(&expect, &error),
        (other, _) => Outcome::Failed(format!("unknown expected outcome '{other}'")),
    }
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
    let digest = hex(&Sha256::digest(&bytes));
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

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
