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

#[test]
fn explicit_phrase_table_and_locale_reach_the_cli_lookup() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&directory).unwrap();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = directory.join(format!(
        "fixed-phrase-cli-{}-{stamp}.csv",
        std::process::id()
    ));
    std::fs::write(
        &path,
        concat!(
            " ,0,1,2,3,4,5,6,7,8,9,10,11\n",
            " ,u32,str,str,str,str,str,str,str,str,str,str,str\n",
            "101,3,,Ja0,Ja1,Authored phrase,,De0,De1,Fr0,Fr1,,\n",
        ),
    )
    .unwrap();
    for (bytes, status, code) in [
        ("02 2E 03 03 66 03", "matched", 0),
        ("02 2E 03 03 67 03", "not-found", 2),
        ("02 2E 03 02 66 03", "category-mismatch", 2),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
            .args([
                "decode-fixed",
                "--hex",
                bytes,
                "--locale",
                "en",
                "--fixed-phrase-csv",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code));
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(document["status"], "decoded");
        assert_eq!(document["phraseLookup"]["status"], status);
        if code == 0 {
            assert_eq!(document["numericFields"], serde_json::json!([2, 101]));
            assert_eq!(
                document["phraseLookup"]["fields"],
                serde_json::json!([
                    {"column": 4, "value": "Authored phrase"},
                    {"column": 5, "value": ""},
                ])
            );
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn unsupported_controls_do_not_interpret_or_load_a_phrase_table() {
    let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .args([
            "decode-fixed",
            "--hex",
            "02 37 FF",
            "--fixed-phrase-csv",
            "not-loaded.csv",
            "--locale",
            "en",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["status"], "unsupported");
    assert_eq!(document["phraseLookup"]["status"], "unsupported-control");
}
