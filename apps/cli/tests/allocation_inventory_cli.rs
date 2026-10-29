use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;

const CATALOG: &str = "data/03/C0/00/00.DAT";
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Client {
    directory: PathBuf,
    root: PathBuf,
}

impl Client {
    fn new(rows: &[[u8; 0x30]], roots: u32) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "xivl-allocation-inventory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let root = directory.join("client");
        fs::create_dir(&root).unwrap();
        let client = Self { directory, root };
        let mut bytes = vec![0; 0x40];
        bytes[..19].copy_from_slice(b"RegionResourceData\0");
        bytes[0x18..0x1E].copy_from_slice(b"1.1.0\0");
        bytes[0x24..0x28].copy_from_slice(&roots.to_le_bytes());
        for row in rows {
            bytes.extend(row);
        }
        let size = bytes.len() as u32;
        bytes[0x20..0x24].copy_from_slice(&size.to_le_bytes());
        client.write(CATALOG, &bytes);
        client
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn invoke(&self, output: Option<&Path>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xivl"));
        command
            .arg("allocation-inventory")
            .arg("--client-root")
            .arg(&self.root);
        if let Some(output) = output {
            command.arg("--output").arg(output);
        }
        command.output().unwrap()
    }

    fn report(&self) -> Value {
        let output = self.invoke(None);
        status(&output, 0);
        let report = serde_json::from_slice(&output.stdout).unwrap();
        validate_schema(&report);
        report
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Only this test's authored tree is removed; symlink tests unlink first.
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn row(key: u32, word04: u32, word08: u32, word0c: u32, name: &[u8], word20: u32) -> [u8; 0x30] {
    let mut bytes = [0; 0x30];
    for (offset, value) in [
        (0, key),
        (4, word04),
        (8, word08),
        (12, word0c),
        (32, word20),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x10..0x10 + name.len()].copy_from_slice(name);
    bytes
}

fn status(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn validate_schema(report: &Value) {
    let schema: Value = serde_json::from_str(include_str!(
        "../../../schemas/allocation-inventory.schema.json"
    ))
    .unwrap();
    let validator = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .compile(&schema)
        .unwrap();
    if let Err(errors) = validator.validate(report) {
        panic!(
            "{}",
            errors
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    };
}

#[test]
fn normal_tree_reports_structure_paths_and_blockers_without_writes() {
    let client = Client::new(
        &[
            row(7, 9, 0xAABB0000, 2, b"root", 0),
            row(80, 0x3000, 0xAABB0000, 0x102, b"layout\x01\\\x80", 0),
            row(81, 1, 0xAABB0005, 3, b"weather", 0),
        ],
        1,
    );
    client.write("data/AA/BB/00/00.DAT", b"authored layout");
    client.write("data/aa/bb/00/0a.dat", b"authored lowercase path");
    let before = fs::read(client.root.join(CATALOG)).unwrap();
    let report = client.report();
    assert_eq!(
        report["catalog"],
        json!({
            "key": "0x03C00000", "path": CATALOG, "size": 208,
            "sha256": sha256_hex(&before), "rootCount": 1, "rowCount": 3,
            "sizeWord": 208, "sizeMatchesInput": true, "trailingBytes": 0,
        })
    );
    assert_eq!(
        report["pathInventory"]["occupiedKeys"],
        json!(["0x03C00000", "0xAABB0000", "0xAABB000A"])
    );
    assert_eq!(report["pathInventory"]["occupiedKeyCount"], 3);
    assert_eq!(report["pathInventory"]["unmappedPathCount"], 0);
    let root = &report["roots"][0];
    assert_eq!(root["key"], "0x00000007");
    assert_eq!(root["word04"], 9);
    assert_eq!(root["word08"], 0xAABB0000u32);
    assert_eq!(root["word20"], 0);
    assert_eq!(root["nonzero"], true);
    assert_eq!(root["representable16Bit"], true);
    assert_eq!(root["tokenTerminated"], true);
    assert_eq!(
        root["auxiliary"],
        json!({
            "status": "derived", "key": "0xAABB0006", "numericPath": "data/AA/BB/00/06.DAT", "numericPathExists": false,
        })
    );
    let child = &root["children"][0];
    assert_eq!(child["effectiveName"], "layout\\x01\\\\\\x80");
    assert_eq!(child["key"], "0x00000050");
    assert_eq!(child["resourceKey"], "0xAABB0000");
    assert_eq!(child["type"], 0x3000);
    assert_eq!(child["kind"], 0x102);
    assert_eq!(child["eligible"], true);
    assert_eq!(child["numericPathExists"], true);
    assert_eq!(root["children"][1]["eligible"], false);
    assert_eq!(root["children"][1]["numericPathExists"], false);
    assert_eq!(report["blockers"].as_array().unwrap().len(), 4);
    assert_eq!(report["blockers"][0]["id"], "numeric-mode-assumed");
    assert!(report["blockers"][0]["source"]
        .as_str()
        .unwrap()
        .contains("\"Catalog mode\""));
    assert!(report["blockers"][3]["statement"]
        .as_str()
        .unwrap()
        .contains("integration limit"));
    assert_eq!(client.invoke(None).stdout, client.invoke(None).stdout);
    assert_eq!(fs::read(client.root.join(CATALOG)).unwrap(), before);
    assert_eq!(fs::read_dir(&client.directory).unwrap().count(), 1);

    let output = client.directory.join("inventory.json");
    let result = client.invoke(Some(&output));
    status(&result, 0);
    assert!(result.stdout.is_empty());
    assert_eq!(fs::read(output).unwrap(), client.invoke(None).stdout);
    client.write("data/AA/BB/00/06.DAT", b"authored auxiliary collision");
    assert_eq!(
        client.report()["collisions"]["auxiliaryResourceKeys"],
        json!([
            {"key":"0xAABB0006", "rootRows":[0], "childRows":[], "explicitRootRows":[], "numericPathExists":true},
        ])
    );
}

#[test]
fn collisions_include_truncated_names_all_row_keys_and_auxiliary_reservations() {
    let client = Client::new(
        &[
            row(7, 0, 0, 2, b"123456789012345A", 0),
            row(80, 0x3000, 0xAABB0000, 2, b"layout", 0),
            row(81, 0, 0xAABB0001, 2, b"sky", 0),
            row(7, 0, 0, 1, b"123456789012345B", 0),
            row(80, 0x3000, 0xAABB0001, 2, b"layout", 0),
            row(8, 0, 0, 1, b"third", 0xAABB0002),
            row(82, 0x3000, 0xAABB0002, 2, b"registered", 0),
        ],
        3,
    );
    client.write("data/AA/BB/00/02.DAT", b"authored occupied auxiliary");
    let report = client.report();
    let collisions = &report["collisions"];
    assert_eq!(
        collisions["duplicateRootKeys"],
        json!([{"key":"0x00000007", "rows":[0,3]}])
    );
    assert_eq!(
        collisions["duplicateChildKeys"],
        json!([{"key":"0x00000050", "rows":[1,4]}])
    );
    assert_eq!(
        collisions["duplicateEffectiveNames"],
        json!([
            {"effectiveName":"123456789012345", "rows":[0,3]},
            {"effectiveName":"layout", "rows":[1,4]},
        ])
    );
    assert_eq!(
        collisions["auxiliaryDerivedKeys"],
        json!([{"key":"0xAABB0002", "rows":[0,3]}])
    );
    assert_eq!(
        collisions["auxiliaryResourceKeys"],
        json!([
        {"key":"0xAABB0002", "rootRows":[0,3], "childRows":[6], "explicitRootRows":[5], "numericPathExists":true},
        ])
    );
    assert_eq!(report["roots"][0]["tokenTerminated"], false);
    assert_eq!(
        report["roots"][2]["auxiliary"]["status"],
        "nonzero-layout-argument"
    );
    fs::remove_file(client.root.join("data/AA/BB/00/02.DAT")).unwrap();
    let missing = client.report();
    assert_eq!(
        missing["collisions"]["auxiliaryResourceKeys"][0]["childRows"],
        json!([6])
    );
    assert_eq!(
        missing["collisions"]["auxiliaryResourceKeys"][0]["numericPathExists"],
        false
    );
}

#[test]
fn overflow_nonconforming_paths_and_size_mismatch_are_reported() {
    let client = Client::new(
        &[
            row(0x10000, 0, 0, 1, b"wide", 0),
            row(1, 1, 0xAABBFFFF, 2, b"last", 0),
            row(0, 0, 0, 0, b"empty", 0),
        ],
        2,
    );
    for path in [
        "data/AA/BB/00/001.DAT",
        "data/AA/BB/00/00/01.DAT",
        "data/loose.DAT",
        "data/GG/BB/00/00.DAT",
    ] {
        client.write(path, b"authored unmapped");
    }
    client.write("data/ignored.txt", b"not a DAT");
    let mut bytes = fs::read(client.root.join(CATALOG)).unwrap();
    bytes.extend([0xFA, 0xFB]);
    client.write(CATALOG, &bytes);
    let report = client.report();
    assert_eq!(report["catalog"]["sizeMatchesInput"], false);
    assert_eq!(report["catalog"]["trailingBytes"], 2);
    assert_eq!(report["pathInventory"]["occupiedKeyCount"], 1);
    assert_eq!(report["pathInventory"]["unmappedPathCount"], 4);
    assert_eq!(
        report["pathInventory"]["unmappedPaths"],
        json!([
            "data/AA/BB/00/00/01.DAT",
            "data/AA/BB/00/001.DAT",
            "data/GG/BB/00/00.DAT",
            "data/loose.DAT",
        ])
    );
    assert_eq!(report["roots"][0]["representable16Bit"], false);
    assert_eq!(
        report["roots"][0]["auxiliary"],
        json!({"status":"low-index-overflow", "key":null,"numericPath":null,"numericPathExists":null})
    );
    assert_eq!(report["roots"][1]["nonzero"], false);
    assert_eq!(report["roots"][1]["representable16Bit"], true);
    assert_eq!(report["roots"][1]["auxiliary"]["status"], "no-children");
}

#[test]
fn refuses_existing_and_client_contained_outputs_before_writing() {
    let client = Client::new(&[], 0);
    let occupied = client.directory.join("existing.json");
    fs::write(&occupied, b"preserve").unwrap();
    for output in [
        &occupied,
        &client.root.join("report.json"),
        &client.root.join("data/../report.json"),
    ] {
        let result = client.invoke(Some(output));
        status(&result, 1);
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).contains("output"));
    }
    assert_eq!(fs::read(occupied).unwrap(), b"preserve");
    assert!(!client.root.join("report.json").exists());
    let result = client.invoke(Some(&client.root.join("missing/report.json")));
    status(&result, 1);
    assert!(!client.root.join("missing").exists());
}

#[test]
fn malformed_catalog_retains_parse_kind_offset_and_creates_no_output() {
    let client = Client::new(&[], 1);
    let destination = client.directory.join("report.json");
    let output = client.invoke(Some(&destination));
    status(&output, 2);
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("subresource-count-out-of-range"), "{error}");
    assert!(error.contains("offset 36:"), "{error}");
    assert!(!destination.exists());
}

#[cfg(any(unix, windows))]
#[test]
fn refuses_symlinks_and_client_output_aliases_when_creation_is_permitted() {
    let client = Client::new(&[], 0);
    let link = client.directory.join("alias");
    #[cfg(unix)]
    let created = std::os::unix::fs::symlink(&client.root, &link);
    #[cfg(windows)]
    let created = std::os::windows::fs::symlink_dir(&client.root, &link);
    if let Err(error) = created {
        eprintln!("symlink test skipped: {error}");
        return;
    }
    let output = client.invoke(Some(&link.join("report.json")));
    status(&output, 1);
    assert!(!client.root.join("report.json").exists());
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();

    let target = client.directory.join("target.DAT");
    fs::write(&target, b"authored link target").unwrap();
    let link = client.root.join("data/link.DAT");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&target, &link).unwrap();
    let result = client.invoke(None);
    fs::remove_file(&link).unwrap();
    status(&result, 1);
    assert!(String::from_utf8_lossy(&result.stderr).contains("symbolic link"));
    assert_eq!(fs::read(target).unwrap(), b"authored link target");
}

#[test]
fn authored_example_matches_the_report_and_schema() {
    let example = serde_json::from_str(include_str!(
        "../../../docs/allocation-inventory.example.json"
    ))
    .unwrap();
    validate_schema(&example);
    assert_eq!(Client::new(&[], 0).report(), example);
}
