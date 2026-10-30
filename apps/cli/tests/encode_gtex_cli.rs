use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use xivl_formats::digest::sha256_hex;
use xivl_formats::gtex_pwib::{self, GtexResource, TaggedResource, TaggedResourceKind};
use xivl_formats::gtex_write::{build_gtex, rebuild_gtex, GtexBuild};
use xivl_formats::texture_preview::encode_png_rgba;
use xivl_formats::to_canonical_json;

struct Scratch {
    root: PathBuf,
    files: RefCell<Vec<PathBuf>>,
}

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "encode-gtex-{}-{}",
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

    fn invoke(&self, input: &str, output: &str, options: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xivl"))
            .arg("encode-gtex")
            .arg(self.path(input))
            .arg("--output")
            .arg(self.path(output))
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

fn parsed(bytes: &[u8]) -> GtexResource {
    let TaggedResource::Gtex(gtex) = gtex_pwib::parse(bytes, TaggedResourceKind::Gtex).unwrap()
    else {
        panic!("expected GTEX")
    };
    gtex
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

fn png() -> Vec<u8> {
    encode_png_rgba(4, 4, &[18, 52, 86, 120].repeat(16)).unwrap()
}

#[test]
fn png_encodes_each_format_and_reports_verified_metadata_and_rgb_error() {
    let scratch = Scratch::new();
    // The misleading extension checks content detection at the command boundary.
    fs::write(scratch.path("source.dds"), png()).unwrap();
    for (format, index, sizes) in [
        ("a8r8g8b8", 4, [64, 16, 4]),
        ("a4r4g4b4", 22, [32, 8, 2]),
        ("dxt1", 24, [8, 8, 8]),
        ("dxt5", 26, [16, 16, 16]),
    ] {
        let output = format!("{format}.gtex");
        let report =
            success(scratch.invoke("source.dds", &output, &["--format", format, "--mips", "3"]));
        let bytes = fs::read(scratch.path(&output)).unwrap();
        let gtex = parsed(&bytes);
        assert_eq!(
            (gtex.format_index, gtex.width, gtex.height, gtex.mip_levels),
            (index, 4, 4, 3)
        );
        assert_eq!(rebuild_gtex(&bytes, &gtex).unwrap(), bytes);
        assert_eq!(report["operation"], "encode-gtex");
        assert_eq!(report["inputKind"], "png");
        assert_eq!(report["format"], format);
        assert_eq!(report["formatIndex"], index);
        assert_eq!(report["width"], 4);
        assert_eq!(report["height"], 4);
        assert_eq!(report["mips"], 3);
        assert_eq!(report["outputLength"], 48 + sizes.iter().sum::<usize>());
        assert_eq!(report["outputSha256"], sha256_hex(&bytes));
        assert_eq!(
            gtex.surfaces
                .iter()
                .map(|surface| surface.declared_size as usize)
                .collect::<Vec<_>>(),
            sizes
        );
        if index == 4 {
            assert!(report.get("rgbMsePerMip").is_none());
            assert_eq!(&bytes[48..52], &[86, 52, 18, 120]);
        } else {
            let errors = report["rgbMsePerMip"].as_array().unwrap();
            assert_eq!(errors.len(), 3);
            assert!(errors.iter().all(|error| error.as_f64().unwrap() >= 0.0));
            if index == 22 {
                assert_eq!(errors, &vec![Value::from(1.0); 3]);
                assert_eq!(&bytes[48..50], &[0x35, 0x71]);
            } else if index == 24 {
                for error in errors {
                    assert!((error.as_f64().unwrap() - 10424.0 / 3.0).abs() < 1e-9);
                }
            }
        }
        assert_eq!(
            report.as_object().unwrap().len(),
            if index == 4 { 9 } else { 10 }
        );
    }
}

#[test]
fn like_uses_built_template_and_default_mips_is_one() {
    let scratch = Scratch::new();
    fs::write(scratch.path("source.png"), png()).unwrap();
    let template = build_gtex(&GtexBuild {
        format_index: 22,
        width: 4,
        height: 4,
        mips: vec![vec![0; 32], vec![0; 8]],
    })
    .unwrap();
    let template_path = scratch.path("template.gtex");
    fs::write(&template_path, &template).unwrap();
    let report = success(scratch.invoke(
        "source.png",
        "like.gtex",
        &["--like", template_path.to_str().unwrap()],
    ));
    assert_eq!(report["formatIndex"], 22);
    assert_eq!(report["mips"], 2);
    let report = success(scratch.invoke("source.png", "default.gtex", &["--format", "a8r8g8b8"]));
    assert_eq!(report["mips"], 1);
    refused(
        scratch.invoke(
            "source.png",
            "template.gtex",
            &["--like", template_path.to_str().unwrap()],
        ),
        1,
        "cannot create output",
    );
    assert_eq!(fs::read(template_path).unwrap(), template);
    assert_eq!(fs::read(scratch.path("source.png")).unwrap(), png());
}

#[test]
fn lossy_error_compares_each_source_mip_and_excludes_alpha() {
    let scratch = Scratch::new();
    let rgba = [0, 0, 0, 120, 8, 0, 0, 120, 16, 0, 0, 120, 24, 0, 0, 120];
    fs::write(
        scratch.path("source.png"),
        encode_png_rgba(2, 2, &rgba).unwrap(),
    )
    .unwrap();
    let report = success(scratch.invoke(
        "source.png",
        "encoded.gtex",
        &["--format", "a4r4g4b4", "--mips", "2"],
    ));
    let errors = report["rgbMsePerMip"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0], 9.5);
    assert!((errors[1].as_f64().unwrap() - 25.0 / 3.0).abs() < 1e-9);
}

// Literal legacy DDS headers keep the repack oracle independent of the GTEX exporter.
fn dds(format: &str) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(b"DDS ");
    let compressed = format == "dxt1";
    for (offset, value) in [
        (4, 124u32),
        (8, if compressed { 0x000a_1007 } else { 0x0002_100f }),
        (12, 4),
        (16, 4),
        (20, if compressed { 8 } else { 16 }),
        (28, 3),
        (76, 32),
        (80, if compressed { 4 } else { 0x41 }),
        (108, 0x0040_1008),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mips = if compressed {
        bytes[84..88].copy_from_slice(b"DXT1");
        vec![vec![0x31; 8], vec![0x72; 8], vec![0xa4; 8]]
    } else {
        for (offset, value) in [
            (88, 32u32),
            (92, 0x00ff_0000),
            (96, 0x0000_ff00),
            (100, 0x0000_00ff),
            (104, 0xff00_0000),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        vec![vec![0x31; 64], vec![0x72; 16], vec![0xa4; 4]]
    };
    for mip in &mips {
        bytes.extend_from_slice(mip);
    }
    (bytes, mips)
}

#[test]
fn dds_repack_copies_exact_mips_and_can_truncate() {
    let scratch = Scratch::new();
    for (format, index) in [("dxt1", 24), ("a8r8g8b8", 4)] {
        let (input, mips) = dds(format);
        fs::write(scratch.path("source.png"), &input).unwrap();
        for count in [1, 2, 3] {
            let output = format!("{format}-{count}.gtex");
            let report = success(scratch.invoke(
                "source.png",
                &output,
                &["--format", format, "--mips", &count.to_string()],
            ));
            let bytes = fs::read(scratch.path(&output)).unwrap();
            let gtex = parsed(&bytes);
            assert_eq!(gtex.format_index, index);
            assert_eq!(gtex.mip_levels, count);
            assert_eq!(report["inputKind"], "dds");
            assert!(report.get("rgbMsePerMip").is_none());
            for (surface, expected) in gtex.surfaces.iter().zip(&mips) {
                let start = surface.source_span.offset as usize;
                assert_eq!(
                    &bytes[start..start + surface.declared_size as usize],
                    expected
                );
            }
        }
        let output = format!("{format}-default.gtex");
        success(scratch.invoke("source.png", &output, &["--format", format]));
        let gtex = parsed(&fs::read(scratch.path(&output)).unwrap());
        assert_eq!(usize::from(gtex.mip_levels), mips.len());
        assert_eq!(fs::read(scratch.path("source.png")).unwrap(), input);
    }
}

#[test]
fn refuses_invalid_requests_without_creating_or_overwriting_files() {
    let scratch = Scratch::new();
    fs::write(scratch.path("source.png"), png()).unwrap();
    let template_path = scratch.path("template.gtex");
    fs::write(
        &template_path,
        build_gtex(&GtexBuild {
            format_index: 4,
            width: 2,
            height: 2,
            mips: vec![vec![0; 16]],
        })
        .unwrap(),
    )
    .unwrap();
    let like = template_path.to_str().unwrap();
    for (options, code, reason) in [
        (vec!["--like", like], 1, "differ from template"),
        (
            vec!["--like", like, "--format", "dxt1"],
            1,
            "cannot be combined",
        ),
        (vec!["--like", like, "--mips", "1"], 1, "cannot be combined"),
        (
            vec!["--format", "a8r8g8b8", "--mips", "4"],
            2,
            "dimension-derived chain",
        ),
        (
            vec!["--format", "a8r8g8b8", "--mips", "0"],
            2,
            "dimension-derived chain",
        ),
        (vec![], 1, "--format is required"),
        (
            vec!["--format", "dxt1", "--format", "dxt5"],
            1,
            "more than once",
        ),
    ] {
        refused(
            scratch.invoke("source.png", "refused.gtex", &options),
            code,
            reason,
        );
        assert!(!scratch.path("refused.gtex").exists());
    }
    fs::write(scratch.path("existing.gtex"), b"preserve").unwrap();
    refused(
        scratch.invoke("source.png", "existing.gtex", &["--format", "dxt1"]),
        1,
        "cannot create output",
    );
    assert_eq!(
        fs::read(scratch.path("existing.gtex")).unwrap(),
        b"preserve"
    );
    refused(
        scratch.invoke("source.png", "source.png", &["--format", "dxt1"]),
        1,
        "cannot create output",
    );
    assert_eq!(fs::read(scratch.path("source.png")).unwrap(), png());
    let (input, _) = dds("dxt1");
    fs::write(scratch.path("source.dds"), input).unwrap();
    refused(
        scratch.invoke("source.dds", "refused.gtex", &["--format", "dxt5"]),
        1,
        "cannot transcode",
    );
    refused(
        scratch.invoke(
            "source.dds",
            "refused.gtex",
            &["--format", "dxt1", "--mips", "4"],
        ),
        1,
        "cannot generate levels",
    );
    refused(
        scratch.invoke(
            "source.dds",
            "refused.gtex",
            &["--format", "dxt1", "--mips", "0"],
        ),
        2,
        "geometric chain",
    );
    fs::write(scratch.path("unknown.bin"), b"not an image").unwrap();
    refused(
        scratch.invoke("unknown.bin", "refused.gtex", &["--format", "dxt1"]),
        1,
        "unknown input kind",
    );
    assert!(!scratch.path("refused.gtex").exists());
}
