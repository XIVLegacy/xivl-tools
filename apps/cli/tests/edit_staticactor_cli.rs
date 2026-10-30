use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::staticactor;
use xivl_formats::staticactor_write::{
    decoded_entries, decoded_unknown_header, encode_staticactor,
};
use xivl_formats::to_canonical_json;

struct Scratch {
    root: PathBuf,
    files: RefCell<Vec<PathBuf>>,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "edit-staticactor-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            root,
            files: RefCell::new(Vec::new()),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        self.files.borrow_mut().push(path.clone());
        path
    }

    fn invoke(&self, options: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xivl"))
            .arg("edit-staticactor")
            .arg(self.path("input.san"))
            .arg("--output")
            .arg(self.path("output.san"))
            .args(options)
            .output()
            .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            for path in self.files.get_mut() {
                let _ = fs::remove_file(path);
            }
            fs::remove_dir(&self.root).unwrap();
        }
    }
}

fn table(records: &[(u32, &[u8])]) -> Vec<u8> {
    let mut bytes = b"sane".to_vec();
    bytes.extend_from_slice(&[0xff, 0x73, 1, 0, 0x12]);
    bytes.extend_from_slice(&(records.len() as u32).to_be_bytes());
    for (value, string) in records {
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes.extend_from_slice(string);
        bytes.push(0);
    }
    for byte in &mut bytes[4..] {
        *byte ^= 0x73;
    }
    bytes
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.stdout, to_canonical_json(&report).as_bytes());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("/Synthetic"));
    report
}

fn refused(output: Output, code: i32, reason: &str) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn adds_one_and_verifies_exact_original_records_and_header() {
    let scratch = Scratch::new();
    let input = table(&[(10, b"/Synthetic/A"), (30, b"\xff\x01")]);
    fs::write(scratch.path("input.san"), &input).unwrap();
    let report = success(scratch.invoke(&["--add", "20=/Synthetic/New"]));
    let output = fs::read(scratch.path("output.san")).unwrap();
    assert_eq!(
        output,
        table(&[
            (10, b"/Synthetic/A"),
            (20, b"/Synthetic/New"),
            (30, b"\xff\x01")
        ])
    );
    assert_eq!(fs::read(scratch.path("input.san")).unwrap(), input);
    assert_eq!(
        report,
        json!({
            "operation": "edit-staticactor", "inputRecordCount": 2,
            "inputSha256": sha256_hex(&input), "outputRecordCount": 3,
            "outputLength": output.len(), "outputSha256": sha256_hex(&output),
            "adds": [{"value": 20, "insertionIndex": 1, "pathLength": 14, "pathExisting": false}],
        })
    );
    let san = staticactor::parse(&output).unwrap();
    assert_eq!(
        encode_staticactor(
            decoded_unknown_header(&output, &san).unwrap(),
            &decoded_entries(&output, &san).unwrap()
        )
        .unwrap(),
        output
    );
}

#[test]
fn adds_several_in_final_order_and_recognizes_only_input_aliases() {
    let scratch = Scratch::new();
    fs::write(
        scratch.path("input.san"),
        table(&[(10, b"/Synthetic/A"), (30, b"/Synthetic/B")]),
    )
    .unwrap();
    let long_path = format!("4294967295=/{}", "x".repeat(254));
    let report = success(scratch.invoke(&[
        "--add",
        "40=/New",
        "--add",
        "0=/Synthetic/A",
        "--add",
        "20=/New",
        "--add",
        &long_path,
    ]));
    let output = fs::read(scratch.path("output.san")).unwrap();
    let long = format!("/{}", "x".repeat(254));
    assert_eq!(
        output,
        table(&[
            (0, b"/Synthetic/A"),
            (10, b"/Synthetic/A"),
            (20, b"/New"),
            (30, b"/Synthetic/B"),
            (40, b"/New"),
            (u32::MAX, long.as_bytes())
        ])
    );
    assert_eq!(
        report["adds"],
        json!([
            {"value":40,"insertionIndex":4,"pathLength":4,"pathExisting":false},
            {"value":0,"insertionIndex":0,"pathLength":12,"pathExisting":true},
            {"value":20,"insertionIndex":2,"pathLength":4,"pathExisting":false},
            {"value":4294967295u32,"insertionIndex":5,"pathLength":255,"pathExisting":false},
        ])
    );
}

#[test]
fn accepts_empty_tables_and_preserves_equal_original_values() {
    let scratch = Scratch::new();
    for records in [vec![], vec![(10, b"a".as_slice()), (10, b"b".as_slice())]] {
        fs::write(scratch.path("input.san"), table(&records)).unwrap();
        success(scratch.invoke(&["--add", "20=/"]));
        let mut expected = records;
        expected.push((20, b"/"));
        assert_eq!(
            fs::read(scratch.path("output.san")).unwrap(),
            table(&expected)
        );
        fs::remove_file(scratch.path("output.san")).unwrap();
    }
}

#[test]
fn refuses_bad_adds_and_options_without_creating_output() {
    let scratch = Scratch::new();
    let input = table(&[(10, b"/Synthetic/A")]);
    fs::write(scratch.path("input.san"), &input).unwrap();
    let too_long = format!("20=/{}", "x".repeat(255));
    let non_ascii = format!("20=/{}", char::from(0xe9));
    for (options, reason) in [
        (vec![], "at least one --add"),
        (vec!["--add"], "usage:"),
        (vec!["--bogus", "x"], "unknown edit-staticactor option"),
        (
            vec!["--output", "x", "--add", "20=/"],
            "--output was supplied more than once",
        ),
        (vec!["--add", "20"], "<value>=<path>"),
        (vec!["--add", "=/"], "decimal u32"),
        (vec!["--add", "-1=/"], "decimal u32"),
        (vec!["--add", "+1=/"], "decimal u32"),
        (vec!["--add", "0x20=/"], "decimal u32"),
        (vec!["--add", "4294967296=/"], "decimal u32"),
        (vec!["--add", "10=/"], "already exists"),
        (vec!["--add", "20=/", "--add", "020=/Other"], "is repeated"),
        (vec!["--add", "20="], "path must be ASCII"),
        (vec!["--add", "20=relative"], "path must be ASCII"),
        (vec!["--add", "20=/a\nb"], "path must be ASCII"),
        (vec!["--add", "20=/a\x7fb"], "path must be ASCII"),
        (vec!["--add", &non_ascii], "path must be ASCII"),
        (vec!["--add", &too_long], "tool limit of 255 bytes"),
    ] {
        refused(scratch.invoke(&options), 1, reason);
        assert!(!scratch.path("output.san").exists());
        assert_eq!(fs::read(scratch.path("input.san")).unwrap(), input);
    }
    for options in [vec![], vec!["input.san", "--add", "20=/"]] {
        refused(
            Command::new(env!("CARGO_BIN_EXE_xivl"))
                .arg("edit-staticactor")
                .args(&options)
                .output()
                .unwrap(),
            1,
            if options.is_empty() {
                "usage:"
            } else {
                "--output is required"
            },
        );
    }
}

#[test]
fn refuses_unsorted_malformed_over_budget_and_existing_destinations() {
    let scratch = Scratch::new();
    fs::write(
        scratch.path("input.san"),
        table(&[(30, b"/A"), (10, b"/B")]),
    )
    .unwrap();
    refused(
        scratch.invoke(&["--add", "20=/"]),
        1,
        "input values are not ascending",
    );
    assert!(!scratch.path("output.san").exists());
    fs::write(scratch.path("input.san"), b"sane").unwrap();
    refused(scratch.invoke(&["--add", "20=/"]), 2, "truncated");
    assert!(!scratch.path("output.san").exists());
    let records: Vec<_> = (0..100_000).map(|value| (value, b"/".as_slice())).collect();
    fs::write(scratch.path("input.san"), table(&records)).unwrap();
    refused(
        scratch.invoke(&["--add", "100000=/"]),
        2,
        "record limit is 100000",
    );
    assert!(!scratch.path("output.san").exists());
    let input = table(&[(10, b"/A")]);
    fs::write(scratch.path("input.san"), &input).unwrap();
    fs::write(scratch.path("output.san"), b"keep").unwrap();
    refused(
        scratch.invoke(&["--add", "20=/"]),
        1,
        "cannot create output",
    );
    assert_eq!(fs::read(scratch.path("output.san")).unwrap(), b"keep");
    refused(
        Command::new(env!("CARGO_BIN_EXE_xivl"))
            .arg("edit-staticactor")
            .arg(scratch.path("input.san"))
            .arg("--output")
            .arg(scratch.path("input.san"))
            .args(["--add", "20=/"])
            .output()
            .unwrap(),
        1,
        "cannot create output",
    );
    assert_eq!(fs::read(scratch.path("input.san")).unwrap(), input);
}
