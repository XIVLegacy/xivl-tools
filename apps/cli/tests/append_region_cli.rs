use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::to_canonical_json;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "append-region-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn input(&self) -> PathBuf {
        self.0.join("input.dat")
    }
    fn output(&self) -> PathBuf {
        self.0.join("output.dat")
    }

    fn invoke(&self, options: &[&str], same_path: bool) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xivl"))
            .arg("append-region")
            .arg(self.input())
            .arg("--output")
            .arg(if same_path {
                self.input()
            } else {
                self.output()
            })
            .args(options)
            .output()
            .unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            for path in [self.input(), self.output()] {
                let _ = fs::remove_file(path);
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }
}

fn catalog() -> Vec<u8> {
    let mut data = vec![0xA5; 0xA0];
    data[..19].copy_from_slice(b"RegionResourceData\0");
    data[0x18..0x1E].copy_from_slice(b"1.1.0\0");
    for (offset, word) in [
        (0x20, 0xA0u32),
        (0x24, 1),
        (0x40, 3),
        (0x48, 0x56780020),
        (0x4C, 1),
        (0x60, 0),
        (0x70, 30),
        (0x74, 0x3000),
        (0x78, 0x12340001),
        (0x7C, 0xFFFFFFFF),
    ] {
        data[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    data[0x50..0x60].copy_from_slice(b"old-root\0\0\0\0\0\0\0\0");
    data[0x80..0x90].copy_from_slice(b"old-child\0\0\0\0\0\0\0");
    data
}

fn options() -> Vec<&'static str> {
    vec![
        "--like",
        "0x3",
        "--root-key",
        "4",
        "--root-name",
        "new-root",
        "--child-key",
        "0x28",
        "--child-name",
        "new-child",
        "--child-resource",
        "2882338832",
    ]
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
    for name in ["new-root", "new-child", "old-root", "old-child"] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(name));
    }
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
fn appends_and_reports_literal_values_without_names() {
    let scratch = Scratch::new();
    let input = catalog();
    fs::write(scratch.input(), &input).unwrap();
    let report = success(scratch.invoke(&options(), false));
    let output = fs::read(scratch.output()).unwrap();
    let mut expected = input.clone();
    expected.extend_from_slice(&input[0x40..0xA0]);
    for (offset, word) in [
        (0x20, 0x100u32),
        (0x24, 2),
        (0xA0, 4),
        (0xAC, 1),
        (0xD0, 40),
        (0xD8, 0xABCD0010),
    ] {
        expected[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    expected[0xB0..0xC0].copy_from_slice(b"new-root\0\0\0\0\0\0\0\0");
    expected[0xE0..0xF0].copy_from_slice(b"new-child\0\0\0\0\0\0\0");
    assert_eq!(output, expected);
    assert_eq!(fs::read(scratch.input()).unwrap(), input);
    assert_eq!(
        report,
        json!({
            "operation": "append-region", "inputLength": 160, "inputSha256": sha256_hex(&input),
            "inputRootCount": 1, "outputLength": 256, "outputSha256": sha256_hex(&output),
            "outputRootCount": 2, "sizeWord": 256, "rootRowOffset": 160, "childRowOffset": 208,
            "rootKey": "0x00000004", "childKey": "0x00000028", "childResource": "0xABCD0010",
            "childType": 12288, "templateRootKey": "0x00000003", "templateChildIndex": 0,
            "derivedAuxiliaryKey": "0xABCD0011", "derivedAuxiliaryNumericPath": "data/AB/CD/00/11.DAT",
            "rootNameByteLength": 8, "childNameByteLength": 9,
        })
    );
}

#[test]
fn explicit_selection_and_type_override_accept_hex_and_decimal() {
    let scratch = Scratch::new();
    fs::write(scratch.input(), catalog()).unwrap();
    let mut args = options();
    args.extend(["--like-child", "0", "--child-type", "0x4000"]);
    let report = success(scratch.invoke(&args, false));
    assert_eq!(report["childType"], 16384);
    assert_eq!(report["templateChildIndex"], 0);
    let output = fs::read(scratch.output()).unwrap();
    assert_eq!(&output[0xD4..0xD8], &[0, 0x40, 0, 0]);
}

#[test]
fn refuses_catalog_and_allocation_failures_without_output() {
    let scratch = Scratch::new();
    let input = catalog();
    fs::write(scratch.input(), &input).unwrap();
    for (flag, value, reason) in [
        ("--like", "99", "absent"),
        ("--root-key", "0", "nonzero and at most"),
        ("--root-key", "65536", "nonzero and at most"),
        ("--root-key", "3", "root key already exists"),
        ("--child-key", "3", "child key already exists"),
        ("--child-key", "30", "child key already exists"),
        ("--root-name", "old-child", "already exists"),
        ("--child-name", "old-root", "already exists"),
        ("--child-name", "new-root", "must differ"),
        ("--root-name", "", "printable ASCII"),
        ("--child-name", "1234567890123456", "printable ASCII"),
        ("--child-resource", "0", "nonzero"),
        ("--child-resource", "0x12340001", "child resource collides"),
        ("--child-resource", "0x56780020", "child resource collides"),
        ("--child-resource", "0x12340002", "child resource collides"),
        (
            "--child-resource",
            "0x12340000",
            "derived auxiliary key collides",
        ),
        ("--child-resource", "0xABCDFFFF", "low16"),
    ] {
        let mut args = options();
        let index = args.iter().position(|argument| *argument == flag).unwrap();
        args[index + 1] = value;
        refused(scratch.invoke(&args, false), 2, reason);
        assert!(!scratch.output().exists());
    }
    for (extra, code, reason) in [
        (vec!["--child-type", "0"], 2, "child type"),
        (vec!["--like-child", "1"], 2, "out of range"),
        (vec!["--child-type", "4294967296"], 1, "u32"),
        (vec!["--child-type", "+2"], 1, "u32"),
        (vec!["--like-child", "-1"], 1, "decimal index"),
        (vec!["--root-key", "5"], 1, "more than once"),
        (vec!["--bogus", "x"], 1, "unknown append-region option"),
        (vec!["--child-type"], 1, "usage:"),
    ] {
        let mut args = options();
        args.extend(extra);
        refused(scratch.invoke(&args, false), code, reason);
        assert!(!scratch.output().exists());
    }
    refused(scratch.invoke(&[], false), 1, "--like is required");
    for (mut data, reason) in [
        (input.clone(), "size word"),
        (input.clone(), "no trailing"),
        (vec![], "unexpected-end-of-input"),
    ] {
        if reason == "size word" {
            data[0x20..0x24].fill(0);
        }
        if reason == "no trailing" {
            data.push(0);
        }
        fs::write(scratch.input(), data).unwrap();
        refused(scratch.invoke(&options(), false), 2, reason);
        assert!(!scratch.output().exists());
    }
    fs::write(scratch.input(), &input).unwrap();
    fs::write(scratch.output(), b"keep").unwrap();
    refused(scratch.invoke(&options(), false), 1, "cannot create output");
    assert_eq!(fs::read(scratch.output()).unwrap(), b"keep");
    refused(scratch.invoke(&options(), true), 1, "cannot create output");
    assert_eq!(fs::read(scratch.input()).unwrap(), input);
}
