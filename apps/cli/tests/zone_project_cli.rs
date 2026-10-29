use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::to_canonical_json;

const OUTPUTS: [&str; 4] = [
    "build-report.json",
    "collision.dat",
    "layout.dat",
    "model.dat",
];
const INPUTS: [&str; 8] = [
    "project.json",
    "floor.obj",
    "collision.json",
    "client/layout.bin",
    "client/model.bin",
    "client/old-collision.bin",
    "client/collision-template.bin",
    "client/unplaced.bin",
];

struct Project {
    root: PathBuf,
}

impl Project {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("xivl-zone-cli-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("client")).unwrap();
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/public/zone-project");
        for name in INPUTS {
            fs::copy(fixtures.join(name), root.join(name)).unwrap();
        }
        Self { root }
    }

    fn invoke(&self, build: bool, output: Option<&Path>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xivl"));
        command
            .arg(if build {
                "build-zone-project"
            } else {
                "validate-zone-project"
            })
            .arg(self.root.join("project.json"))
            .arg("--client-root")
            .arg(self.root.join("client"));
        if let Some(output) = output {
            command.arg("--output").arg(output);
        }
        command.output().unwrap()
    }

    fn inputs(&self) -> BTreeMap<String, Vec<u8>> {
        INPUTS
            .into_iter()
            .map(|name| (name.into(), fs::read(self.root.join(name)).unwrap()))
            .collect()
    }

    fn save(&self, project: &Value) {
        fs::write(
            self.root.join("project.json"),
            serde_json::to_vec(project).unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        // This directory contains only this test's authored inputs and outputs.
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn status(output: &Output, expected: i32) {
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn files(directory: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn zone_project_cli_validates_and_builds_identical_outputs_without_mutating_inputs() {
    let project = Project::new();
    let inputs = project.inputs();
    let validated = project.invoke(false, None);
    status(&validated, 0);
    assert!(validated.stderr.is_empty());
    let mut report: Value = serde_json::from_slice(&validated.stdout).unwrap();
    assert_eq!(report["status"], "offline-validated");
    assert_eq!(report["clientVerification"], "unverified");
    assert_eq!(report["checks"]["placementCount"], 4);
    assert_eq!(report["checks"]["flattenedVertexCount"], 16);
    assert_eq!(report["checks"]["triangleCount"], 8);
    assert_eq!(
        report["checks"]["worldBounds"],
        json!([[-32.0, 0.0, -32.0], [32.0, 0.0, 32.0]])
    );
    assert_eq!(report["outputs"]["model"]["id"], "0xFFFF0001");
    assert_eq!(report["outputs"]["collision"]["id"], "0xFFFF0002");
    assert_eq!(fs::read_dir(&project.root).unwrap().count(), 4);
    assert_eq!(
        fs::read_dir(project.root.join("client")).unwrap().count(),
        5
    );
    assert_eq!(project.inputs(), inputs);

    let first = project.root.join("first");
    let second = project.root.join("second");
    status(&project.invoke(true, Some(&first)), 0);
    assert_eq!(project.inputs(), inputs);
    status(&project.invoke(true, Some(&second)), 0);
    assert_eq!(project.inputs(), inputs);
    // Declaration order must not affect report arrays.
    let mut reordered: Value = serde_json::from_slice(&inputs["project.json"]).unwrap();
    reordered["resources"].as_array_mut().unwrap().reverse();
    reordered["expectedPlacements"]
        .as_array_mut()
        .unwrap()
        .reverse();
    project.save(&reordered);
    let reordered_inputs = project.inputs();
    let reordered_validation = project.invoke(false, None);
    status(&reordered_validation, 0);
    assert_eq!(reordered_validation.stdout, validated.stdout);
    assert_eq!(project.inputs(), reordered_inputs);
    fs::write(project.root.join("project.json"), &inputs["project.json"]).unwrap();
    assert_eq!(project.inputs(), inputs);
    let built = files(&first);
    assert_eq!(
        built.keys().map(String::as_str).collect::<Vec<_>>(),
        OUTPUTS
    );
    assert_eq!(built, files(&second));
    report["status"] = json!("offline-built");
    assert_eq!(
        built["build-report.json"],
        to_canonical_json(&report).as_bytes()
    );
    for role in ["layout", "model", "collision"] {
        let output = &report["outputs"][role];
        let bytes = &built[output["path"].as_str().unwrap()];
        assert_eq!(output["size"], bytes.len());
        assert_eq!(output["sha256"], sha256_hex(bytes));
    }
    assert_eq!(report["outputs"].as_object().unwrap().len(), 3);
    let text = String::from_utf8(built["build-report.json"].clone()).unwrap();
    assert!(!text.contains(&project.root.to_string_lossy().to_string()));
    for forbidden in ["timestamp", "duration", "randomId"] {
        assert!(!text.contains(forbidden));
    }
    fs::write(first.join("sentinel"), b"keep").unwrap();
    let before = files(&first);
    status(&project.invoke(true, Some(&first)), 1);
    assert_eq!(files(&first), before);
    let empty = project.root.join("empty");
    fs::create_dir(&empty).unwrap();
    status(&project.invoke(true, Some(&empty)), 1);
    assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
    assert_eq!(project.inputs(), inputs);
}

#[test]
fn zone_project_cli_refuses_input_destinations_and_preserves_diagnostics() {
    let project = Project::new();
    let inputs = project.inputs();
    let inside = project.root.join("client/new-output");
    status(&project.invoke(true, Some(&inside)), 1);
    assert!(!inside.exists());
    #[cfg(windows)]
    {
        let mixed_case = project.root.join("cLiEnT/new-output");
        status(&project.invoke(true, Some(&mixed_case)), 1);
        assert!(!inside.exists());
    }
    status(
        &project.invoke(true, Some(&project.root.join("floor.obj"))),
        1,
    );
    let alias = project.root.join("source-alias");
    fs::hard_link(project.root.join("floor.obj"), &alias).unwrap();
    status(&project.invoke(true, Some(&alias)), 1);
    assert_eq!(fs::read(&alias).unwrap(), inputs["floor.obj"]);
    assert_eq!(project.inputs(), inputs);

    let mut declaration: Value = serde_json::from_slice(&inputs["project.json"]).unwrap();
    declaration["layout"]["sha256"] = json!("0".repeat(64));
    project.save(&declaration);
    let failed_output = project.root.join("bad-identity");
    status(&project.invoke(true, Some(&failed_output)), 2);
    assert!(!failed_output.exists());

    let mut layout = inputs["client/layout.bin"].clone();
    layout[0] = 0;
    fs::write(project.root.join("client/layout.bin"), &layout).unwrap();
    declaration["layout"]["sha256"] = json!(sha256_hex(&layout));
    project.save(&declaration);
    let failure = project.invoke(false, None);
    status(&failure, 2);
    let diagnostic = String::from_utf8(failure.stderr).unwrap();
    assert!(
        diagnostic.contains("layout: Format(InvalidAttributeValue) at offset 0"),
        "{diagnostic}"
    );
    fs::write(
        project.root.join("client/layout.bin"),
        &inputs["client/layout.bin"],
    )
    .unwrap();
    fs::write(project.root.join("project.json"), &inputs["project.json"]).unwrap();
    assert_eq!(project.inputs(), inputs);
    fs::remove_file(project.root.join("client/unplaced.bin")).unwrap();
    status(&project.invoke(false, None), 1);
}

#[cfg(any(unix, windows))]
#[test]
fn zone_project_cli_refuses_output_through_a_directory_alias() {
    let project = Project::new();
    let inputs = project.inputs();
    let alias = project.root.join("directory-alias");
    #[cfg(windows)]
    {
        let created = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(project.root.join("client"))
            .output()
            .unwrap();
        status(&created, 0);
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(project.root.join("client"), &alias).unwrap();
    let failure = project.invoke(true, Some(&alias.join("new-output")));
    // Unlink the alias before assertions and test-directory cleanup.
    #[cfg(windows)]
    fs::remove_dir(&alias).unwrap();
    #[cfg(unix)]
    fs::remove_file(&alias).unwrap();
    status(&failure, 1);
    assert!(String::from_utf8(failure.stderr)
        .unwrap()
        .contains("outside the client input tree"));
    assert!(!project.root.join("client/new-output").exists());
    assert_eq!(project.inputs(), inputs);
}

#[cfg(unix)]
#[test]
fn zone_project_cli_builds_under_a_symlinked_output_ancestor() {
    let project = Project::new();
    let inputs = project.inputs();
    let real = project.root.join("real");
    fs::create_dir(&real).unwrap();
    fs::create_dir(real.join("parent")).unwrap();
    let alias = project.root.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let result = project.invoke(true, Some(&alias.join("parent/new-output")));
    fs::remove_file(&alias).unwrap();
    status(&result, 0);
    assert_eq!(
        files(&real.join("parent/new-output"))
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        OUTPUTS
    );
    assert_eq!(project.inputs(), inputs);
}

#[test]
fn zone_project_cli_accepts_single_hyphen_paths() {
    let project = Project::new();
    fs::copy(
        project.root.join("project.json"),
        project.root.join("-project.json"),
    )
    .unwrap();
    fs::rename(project.root.join("client"), project.root.join("-client")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .current_dir(&project.root)
        .args([
            "build-zone-project",
            "-project.json",
            "--client-root",
            "-client",
            "--output",
            "-output",
        ])
        .output()
        .unwrap();
    status(&output, 0);
    assert!(project.root.join("-output/build-report.json").is_file());
}

#[test]
fn zone_project_cli_missing_or_non_directory_client_root_exits_one() {
    let project = Project::new();
    for root in ["missing", "floor.obj"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
            .arg("validate-zone-project")
            .arg(project.root.join("project.json"))
            .arg("--client-root")
            .arg(project.root.join(root))
            .output()
            .unwrap();
        status(&output, 1);
        assert!(String::from_utf8(output.stderr)
            .unwrap()
            .contains("client root:"));
    }
}

#[test]
fn zone_project_cli_rejects_incomplete_duplicate_and_unknown_options() {
    for arguments in [
        vec!["validate-zone-project"],
        vec!["validate-zone-project", "project.json", "--client-root"],
        vec![
            "validate-zone-project",
            "project.json",
            "--client-root",
            "--output",
        ],
        vec![
            "validate-zone-project",
            "project.json",
            "--client-root",
            "root",
            "--output",
            "new",
        ],
        vec![
            "build-zone-project",
            "project.json",
            "--client-root",
            "root",
        ],
        vec![
            "build-zone-project",
            "project.json",
            "--client-root",
            "root",
            "--output",
            "new",
            "--output",
            "other",
        ],
        vec![
            "validate-zone-project",
            "project.json",
            "--client-root",
            "root",
            "--client-root",
            "other",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
            .args(arguments)
            .output()
            .unwrap();
        status(&output, 1);
        assert!(String::from_utf8(output.stderr).unwrap().contains("usage:"));
    }
}
