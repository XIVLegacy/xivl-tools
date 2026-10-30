use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::{extract_lpb, rebuild_lpb, to_canonical_json};

struct Scratch {
    root: PathBuf,
    files: RefCell<Vec<PathBuf>>,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "pack-lpb-{}-{}",
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
            .arg("pack-lpb")
            .arg(self.path("chunk.luac"))
            .arg("--output")
            .arg(self.path("new.lpb"))
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

fn chunk() -> Vec<u8> {
    let mut bytes = b"\x1bLuaQ\x00\x01\x04\x04\x04\x08\x00".to_vec();
    bytes.extend_from_slice(&[0; 12]); // No source name or line bounds.
    bytes.extend_from_slice(&[0, 0, 0, 2]);
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&(30u32 | (1 << 23)).to_le_bytes()); // RETURN R0, 1.
    bytes.extend_from_slice(&[0; 20]); // Empty constant, child, and debug tables.
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
    report
}

fn refused(output: Output, code: i32, reason: &str) {
    assert_eq!(output.status.code(), Some(code));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn packs_rle_and_rlu_with_exact_bytes_and_canonical_report() {
    for kind in ["rle", "rlu"] {
        let scratch = Scratch::new();
        let input = chunk();
        fs::write(scratch.path("chunk.luac"), &input).unwrap();
        let options = if kind == "rle" {
            vec!["--tag", "0xC51F"]
        } else {
            vec!["--wrapper", "rlu", "--tag", "c51f"]
        };
        let report = success(scratch.invoke(&options));
        let expected = if kind == "rle" {
            let mut header = b"rle\x0c\x1f\xc5\x00\x00".to_vec();
            header.extend_from_slice(&(input.len() as u32).to_le_bytes());
            header.push(0xff);
            header.extend(input.iter().map(|byte| byte ^ 0x73));
            header
        } else {
            let mut header = b"rlu\x0b\x1f\xc5\x00\x00".to_vec();
            header.extend_from_slice(&input);
            header
        };
        let output = fs::read(scratch.path("new.lpb")).unwrap();
        assert_eq!(output, expected);
        let file = extract_lpb(&output).unwrap();
        assert_eq!(file.decoded, input);
        assert_eq!(rebuild_lpb(&output, &file).unwrap(), output);
        assert_eq!(
            report,
            json!({
                "operation": "pack-lpb", "wrapper": kind, "tag": "0x0000c51f",
                "chunkLength": input.len(), "chunkSha256": sha256_hex(&input),
                "outputLength": expected.len(), "outputSha256": sha256_hex(&expected),
                "lua": { "prototypeCount": 1, "instructionCount": 1 },
            })
        );
    }
}

#[test]
fn like_preserves_all_wrapper_bytes_and_replaces_the_size_word() {
    for kind in ["rle", "rlu"] {
        let scratch = Scratch::new();
        let mut input = chunk();
        input[40..44].copy_from_slice(&1u32.to_le_bytes());
        input.splice(44..44, chunk()[12..].iter().copied());
        fs::write(scratch.path("chunk.luac"), &input).unwrap();
        let template = if kind == "rle" {
            let mut bytes = b"rle\x0cABCD\x02\x00\x00\x00!".to_vec();
            bytes.extend(b"\x1bLuaQother".iter().map(|byte| byte ^ 0x73));
            bytes
        } else {
            b"rlu\x0bABCD\x1bLuaQother".to_vec()
        };
        let like = scratch.path("like.lpb");
        fs::write(&like, &template).unwrap();
        let report = success(scratch.invoke(&["--like", like.to_str().unwrap()]));
        let output = fs::read(scratch.path("new.lpb")).unwrap();
        assert_eq!(&output[..8], &template[..8]);
        assert_eq!(report["wrapper"], kind);
        assert_eq!(report["tag"], "0x44434241");
        assert_eq!(
            report["lua"],
            json!({ "prototypeCount": 2, "instructionCount": 2 })
        );
        if kind == "rle" {
            assert_eq!(output[12], b'!');
            assert_eq!(&output[8..12], &(input.len() as u32).to_le_bytes());
        }
        assert_eq!(extract_lpb(&output).unwrap().decoded, input);
    }
}

#[test]
fn refuses_invalid_chunks_options_and_existing_output() {
    let scratch = Scratch::new();
    let input_path = scratch.path("chunk.luac");
    let output_path = scratch.path("new.lpb");
    for (input, reason) in [
        (vec![0; 12], "unsupported-lua-header"),
        (b"\x1bLuaQ".to_vec(), "unexpected-end-of-input"),
        ([chunk(), vec![0]].concat(), "trailing-bytes"),
        (
            {
                let mut bytes = chunk();
                bytes[32..36].copy_from_slice(&38u32.to_le_bytes());
                bytes
            },
            "malformed-lua-bytecode",
        ),
        (
            {
                let mut bytes = chunk();
                bytes[32..36].copy_from_slice(&0x008000deu32.to_le_bytes());
                bytes
            },
            "malformed-lua-bytecode",
        ),
    ] {
        fs::write(&input_path, input).unwrap();
        refused(scratch.invoke(&["--tag", "c51f"]), 2, reason);
        assert!(!output_path.exists());
    }
    fs::write(&input_path, chunk()).unwrap();
    for (options, reason) in [
        (vec![], "--tag is required"),
        (
            vec!["--like", "unused.lpb", "--tag", "1"],
            "--like cannot be combined",
        ),
        (
            vec!["--like", "unused.lpb", "--wrapper", "rle"],
            "--like cannot be combined",
        ),
        (vec!["--tag", "10000"], "--tag requires a hex u16"),
        (vec!["--tag", "-1"], "--tag requires a hex u16"),
        (
            vec!["--tag", "1", "--wrapper", "other"],
            "--wrapper requires rle or rlu",
        ),
    ] {
        refused(scratch.invoke(&options), 1, reason);
        assert!(!output_path.exists());
    }
    fs::write(&output_path, b"keep").unwrap();
    refused(
        scratch.invoke(&["--tag", "c51f"]),
        1,
        "cannot create output",
    );
    assert_eq!(fs::read(&output_path).unwrap(), b"keep");
}
