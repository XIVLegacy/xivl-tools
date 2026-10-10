use std::io::Write;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

fn invoke(input: &[u8], options: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .args(["decode-fixed-batch", "-"])
        .args(options)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

fn records(output: &Output) -> Vec<Value> {
    std::str::from_utf8(&output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn mixed_records_keep_order_ids_bytes_and_continue_after_input_errors() {
    let output = invoke(
        concat!(
            "{\"id\":\"first\",\"hex\":\"41 00 ff\"}\r\n",
            "{bad}\n",
            "{\"id\":\"bad-hex\",\"hex\":\"0\"}\n",
            "{\"hex\":\"02 37 FF 02 2E 03 02 03 03\"}\n",
            "{\"hex\":\"02 2E 04 02 F1\"}\n",
            "\n",
            "{\"hex\":\"41\",\"extra\":true}\n",
            "[\"41\",null]\n",
            "{\"id\":null,\"hex\":\"\"}",
        )
        .as_bytes(),
        &[],
    );
    assert_eq!(output.status.code(), Some(2));
    let rows = records(&output);
    assert_eq!(rows.len(), 9);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["line"], index + 1);
        assert_ne!(row.get("result").is_some(), row.get("error").is_some());
    }
    assert_eq!(rows[0]["id"], "first");
    assert_eq!(rows[0]["result"]["outputHex"], "4100ff");
    assert_eq!(rows[0]["result"]["status"], "complete");
    assert!(rows[0]["result"].get("text").is_none());
    assert_eq!(rows[1]["error"]["kind"], "input");
    assert!(rows[1]["error"]["column"].as_u64().unwrap() > 0);
    assert_eq!(rows[2]["id"], "bad-hex");
    assert_eq!(rows[2]["error"]["kind"], "input");
    assert_eq!(rows[3]["result"]["outputHex"], "0237ff022e03020303");
    assert_eq!(rows[3]["result"]["status"], "partial");
    assert_eq!(rows[3]["result"]["segments"][0]["kind"], "opaque-remainder");
    assert_eq!(rows[4]["result"]["segments"][0]["errorOffset"], 5);
    assert_eq!(rows[5]["error"]["kind"], "input");
    assert_eq!(rows[6]["error"]["kind"], "input");
    assert_eq!(rows[7]["error"]["kind"], "input");
    assert_eq!(rows[7]["error"]["column"], 1);
    assert_eq!(rows[8]["result"]["text"], "");
    assert!(rows[8].get("id").is_none());
}

#[test]
fn file_and_stdin_batches_match_single_message_reports() {
    let input = b"{\"id\":\"literal\",\"hex\":\"41\"}\n{\"hex\":\"022e03020303\"}\n";
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&directory).unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = directory.join(format!("fixed-batch-{}-{stamp}.jsonl", std::process::id()));
    std::fs::write(&path, input).unwrap();
    let file = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .arg("decode-fixed-batch")
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let stdin = invoke(input, &[]);
    assert_eq!(file.status.code(), Some(2));
    assert_eq!(file.stdout, stdin.stdout);
    for (index, hex) in ["41", "022e03020303"].iter().enumerate() {
        let single = Command::new(env!("CARGO_BIN_EXE_xivl"))
            .args(["decode-fixed", "--hex", hex, "--message"])
            .output()
            .unwrap();
        assert_eq!(
            records(&file)[index]["result"],
            serde_json::from_slice::<Value>(&single.stdout).unwrap()
        );
    }
}

#[test]
fn source_failure_stops_at_its_line_after_emitted_results() {
    let output = invoke(
        b"{\"hex\":\"41\"}\n{\"hex\":\"022e03020303\"}\n{\"hex\":\"42\"}\n",
        &["--initial-registry", "not-loaded", "--locale", "en"],
    );
    assert_eq!(output.status.code(), Some(1));
    let rows = records(&output);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["result"]["text"], "A");
    assert!(std::str::from_utf8(&output.stderr)
        .unwrap()
        .contains("batch line 2"));
}

#[test]
fn empty_input_and_literal_batches_succeed_with_lazy_registry() {
    for (input, count) in [(&b""[..], 0), (&b"{\"hex\":\"41\"}\n"[..], 1)] {
        let output = invoke(
            input,
            &["--initial-registry", "not-loaded", "--locale", "en"],
        );
        assert_eq!(output.status.code(), Some(0));
        let rows = records(&output);
        assert_eq!(rows.len(), count);
        if count == 1 {
            assert_eq!(rows[0]["line"], 1);
            assert_eq!(rows[0]["result"]["text"], "A");
        }
    }
}

#[test]
fn invalid_options_are_refused_before_reading_input() {
    for options in [
        vec![],
        vec!["-", "--locale", "en"],
        vec!["-", "--initial-registry", "not-loaded"],
        vec!["-", "--locale", "chs"],
        vec!["-", "--message"],
        vec!["-", "--locale", "en", "--locale", "fr"],
        vec!["-", "--initial-registry", "a", "--initial-registry", "b"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
            .arg("decode-fixed-batch")
            .args(options)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
}
