use std::process::Command;

#[test]
fn file_input_reaches_the_decoder_at_the_requested_offset() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/public/rich-string/public-fixed.bin");
    let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .arg("decode-fixed")
        .arg(fixture)
        .args(["--offset", "6"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["numericFields"], serde_json::json!([1, 207]));
    assert_eq!(document["rawHex"], "022e0402f0cf03");
    assert_eq!(
        document["span"],
        serde_json::json!({"offset": 6, "length": 7})
    );
}

#[test]
fn incomplete_controls_exit_two_with_machine_readable_diagnostics() {
    let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .args(["decode-fixed", "--hex", "02 2E 04 02 F1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["status"], "error");
    assert_eq!(document["errorKind"], "unexpected-end-of-input");
    assert_eq!(document["errorOffset"], 5);
    assert_eq!(document["rawHex"], "022e0402f1");
}
