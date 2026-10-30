use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use xivl_cli::zone_project::prepare_project;
use xivl_formats::digest::sha256_hex;
use xivl_formats::region::{self, AuxiliaryKey};
use xivl_formats::{to_canonical_json, ResourceId};

const CATALOG: &str = "data/03/C0/00/00.DAT";
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Package {
    root: PathBuf,
}

impl Package {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "xivl-package-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(root.join("floor")).unwrap();
        fs::create_dir(root.join("client")).unwrap();
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/public/zone-project");
        for name in ["project.json", "floor.obj", "collision.json"] {
            fs::copy(fixture.join(name), root.join("floor").join(name)).unwrap();
        }
        for name in [
            "layout.bin",
            "model.bin",
            "old-collision.bin",
            "collision-template.bin",
            "unplaced.bin",
        ] {
            fs::copy(
                fixture.join("client").join(name),
                root.join("client").join(name),
            )
            .unwrap();
        }
        let package = Self { root };
        let example: Value =
            serde_json::from_str(include_str!("../../../docs/zone-package.example.json")).unwrap();
        package.save(example);
        package.catalog(&catalog());
        package
    }

    fn save(&self, mut value: Value) {
        let project = fs::read(self.root.join("floor/project.json")).unwrap();
        value["project"]["size"] = json!(project.len());
        value["project"]["sha256"] = json!(sha256_hex(&project));
        fs::write(self.root.join("package.json"), to_canonical_json(&value)).unwrap();
    }

    fn value(&self) -> Value {
        serde_json::from_slice(&fs::read(self.root.join("package.json")).unwrap()).unwrap()
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.root.join("client").join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn catalog(&self, bytes: &[u8]) {
        self.write(CATALOG, bytes);
    }

    fn invoke(&self, output: Option<&Path>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xivl"));
        command
            .arg(if output.is_some() {
                "build-zone-package"
            } else {
                "validate-zone-package"
            })
            .arg(self.root.join("package.json"))
            .arg("--client-root")
            .arg(self.root.join("client"));
        if let Some(output) = output {
            command.arg("--output").arg(output);
        }
        command.output().unwrap()
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        // This test owns the entire synthetic directory and creates no links.
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn catalog() -> Vec<u8> {
    let mut bytes = vec![0; 0x100];
    bytes[..19].copy_from_slice(b"RegionResourceData\0");
    bytes[0x18..0x1E].copy_from_slice(b"1.1.0\0");
    word(&mut bytes, 0x20, 0x100);
    word(&mut bytes, 0x24, 2);
    for (index, (key, kind, resource, children, name)) in [
        (0x6F, 0, 0x12000000, 2, "template"),
        (0x006F0001, 0x3000, 0x11000010, 7, "first"),
        (0x006F0002, 1, 0x22000020, 9, "second"),
        (0x70, 0, 0, 0, "other"),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = 0x40 + index * 0x30;
        word(&mut bytes, offset, key);
        word(&mut bytes, offset + 4, kind);
        word(&mut bytes, offset + 8, resource);
        word(&mut bytes, offset + 12, children);
        bytes[offset + 16..offset + 16 + name.len()].copy_from_slice(name.as_bytes());
        bytes[offset + 0x24..offset + 0x30].fill(0xA0 + index as u8);
    }
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

fn schema(value: &Value, text: &str) {
    let schema: Value = serde_json::from_str(text).unwrap();
    let validator = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .compile(&schema)
        .unwrap();
    if let Err(errors) = validator.validate(value) {
        panic!(
            "{}",
            errors
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
    };
}

fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn collect(root: &Path, path: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &entry.path(), files);
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(root, root, &mut result);
    result
}

#[test]
fn package_validates_builds_exact_floor_bytes_and_repeats_deterministically() {
    let package = Package::new();
    schema(
        &package.value(),
        include_str!("../../../schemas/zone-package.schema.json"),
    );
    schema(
        &serde_json::from_str(include_str!("../../../docs/zone-package.example.json")).unwrap(),
        include_str!("../../../schemas/zone-package.schema.json"),
    );
    let before = files(&package.root);
    let output = package.invoke(None);
    status(&output, 0);
    let mut manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    schema(
        &manifest,
        include_str!("../../../schemas/zone-package-manifest.schema.json"),
    );
    assert_eq!(manifest["status"], "offline-validated");
    assert_eq!(manifest["complete"], false);
    assert_eq!(manifest["clientVerification"], "unverified");
    assert_eq!(
        manifest["registration"],
        json!({
            "rootKey":"0x0000F000","childKey":"0xF0000001","childType":12288,
            "templateRootKey":"0x0000006F","templateChildIndex":0,
            "rootNameByteLength":8,"childNameByteLength":9,
            "rootCountBefore":2,"rootCountAfter":3,"sizeWordBefore":256,"sizeWordAfter":352,
            "rootRowOffset":256,"childRowOffset":304,"changedHeaderSpan":{"offset":32,"length":8}
        })
    );
    assert_eq!(manifest["allocations"][3]["key"], "0xF1000001");
    let inventory = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .arg("allocation-inventory")
        .arg("--client-root")
        .arg(package.root.join("client"))
        .output()
        .unwrap();
    status(&inventory, 0);
    let inventory: Value = serde_json::from_slice(&inventory.stdout).unwrap();
    assert_eq!(
        &manifest["blockers"].as_array().unwrap()[..4],
        inventory["blockers"].as_array().unwrap()
    );
    assert_eq!(
        manifest["dependencies"]["directResources"],
        manifest["inputs"]["directResources"]
    );
    assert_eq!(
        manifest["dependencies"]["directResources"][2]["retained"],
        true
    );
    assert_eq!(before, files(&package.root));
    let floor = prepare_project(
        &package.root.join("floor/project.json"),
        &package.root.join("client"),
    )
    .unwrap();
    let mut builds = Vec::new();
    for name in ["first", "second"] {
        let path = package.root.join(name);
        let output = package.invoke(Some(&path));
        status(&output, 0);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        let built = files(&path);
        assert_eq!(built.len(), 5);
        assert_eq!(built["data/F1/00/00/00.DAT"], floor.layout);
        assert_eq!(built["data/FF/FF/00/01.DAT"], floor.model);
        assert_eq!(built["data/FF/FF/00/02.DAT"], floor.collision);
        let bytes = &built[CATALOG];
        assert_eq!(&bytes[..0x20], &catalog()[..0x20]);
        assert_eq!(&bytes[0x28..0x100], &catalog()[0x28..]);
        let parsed = region::parse(bytes).unwrap();
        assert_eq!(
            region::auxiliary_key(&parsed, 4),
            Some(AuxiliaryKey::Derived(0xF1000001))
        );
        assert_eq!(&bytes[0x100..0x104], &0xF000u32.to_le_bytes());
        assert_eq!(&bytes[0x130..0x134], &0xF0000001u32.to_le_bytes());
        assert_eq!(&bytes[0x138..0x13C], &0xF1000000u32.to_le_bytes());
        assert_eq!(&bytes[0x110..0x120], b"new-root\0\0\0\0\0\0\0\0");
        assert_eq!(&bytes[0x140..0x150], b"new-child\0\0\0\0\0\0\0");
        let report: Value = serde_json::from_slice(&built["package-manifest.json"]).unwrap();
        schema(
            &report,
            include_str!("../../../schemas/zone-package-manifest.schema.json"),
        );
        manifest["status"] = json!("offline-built");
        assert_eq!(manifest, report);
        assert_eq!(
            built["package-manifest.json"],
            to_canonical_json(&report).as_bytes()
        );
        for item in report["outputs"].as_array().unwrap() {
            let bytes = &built[item["path"].as_str().unwrap()];
            assert_eq!(item["size"], bytes.len());
            assert_eq!(item["sha256"], sha256_hex(bytes));
        }
        assert!(!built.contains_key("data/F1/00/00/01.DAT"));
        builds.push(built);
    }
    assert_eq!(builds[0], builds[1]);
    for (path, bytes) in before {
        assert_eq!(fs::read(package.root.join(path)).unwrap(), bytes);
    }
}

#[test]
fn package_refuses_each_role_in_every_occupied_namespace() {
    for (role, key) in [
        ("layout", 0xF1000010u32),
        ("model", 0xFFFF0001),
        ("collision", 0xFFFF0002),
        ("auxiliary", 0xF1000011),
    ] {
        for source in ["scan", "uppercase", "child", "root08", "root20", "derived"] {
            let package = Package::new();
            let mut value = package.value();
            value["layoutKey"] = json!("0xF1000010");
            package.save(value);
            let mut bytes = catalog();
            let [a, b, c, d] = key.to_be_bytes();
            let path = format!("data/{a:02X}/{b:02X}/{c:02X}/{d:02X}.DAT");
            match source {
                "scan" => package.write(&path.to_ascii_lowercase(), b"occupied"),
                "uppercase" => fs::create_dir_all(package.root.join("client").join(&path)).unwrap(),
                "child" => word(&mut bytes, 0x78, key),
                "root08" => word(&mut bytes, 0xD8, key),
                "root20" => word(&mut bytes, 0xF0, key),
                "derived" => {
                    word(
                        &mut bytes,
                        0x78,
                        (key & 0xFFFF0000)
                            | if role == "model" || role == "collision" {
                                0
                            } else {
                                3
                            },
                    );
                    word(&mut bytes, 0xA8, 0x22000000 | ((key & 0xFFFF) - 1));
                }
                _ => unreachable!(),
            }
            package.catalog(&bytes);
            let destination = package.root.join("refused");
            let output = package.invoke(Some(&destination));
            status(&output, 2);
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains(role), "{source}: {error}");
            assert!(
                error.contains(&ResourceId::new(key).to_hex()),
                "{source}: {error}"
            );
            assert!(!destination.exists());
        }
    }
}

#[test]
fn package_refuses_pairwise_allocations_identity_mismatch_and_output_roots() {
    for (layout, model, collision) in [
        (0xFFFF0001, 0xFFFF0001, 0xFFFF0002),
        (0xFFFF0002, 0xFFFF0001, 0xFFFF0002),
        (0xF1000000, 0xF1000001, 0xFFFF0002),
        (0xF1000000, 0xFFFF0001, 0xF1000001),
        (0xF1000000, 0xFFFF0001, 0xFFFF0001),
    ] {
        let package = Package::new();
        let mut project: Value =
            serde_json::from_slice(&fs::read(package.root.join("floor/project.json")).unwrap())
                .unwrap();
        project["binding"]["outputModelId"] = json!(ResourceId::new(model).to_hex());
        project["binding"]["outputCollisionId"] = json!(ResourceId::new(collision).to_hex());
        fs::write(
            package.root.join("floor/project.json"),
            to_canonical_json(&project),
        )
        .unwrap();
        let mut value = package.value();
        value["layoutKey"] = json!(ResourceId::new(layout).to_hex());
        package.save(value);
        let output = package.invoke(None);
        status(&output, 2);
        let error = String::from_utf8_lossy(&output.stderr);
        let (role, key) = if model == collision {
            ("collision", model)
        } else if layout == model {
            ("model", model)
        } else if layout == collision {
            ("collision", collision)
        } else {
            ("auxiliary", 0xF1000001)
        };
        assert!(error.contains(role), "{error}");
        assert!(error.contains(&ResourceId::new(key).to_hex()), "{error}");
    }
    let package = Package::new();
    let value = package.value();
    for field in ["size", "sha256"] {
        let mut broken = value.clone();
        broken["project"][field] = if field == "size" {
            json!(1)
        } else {
            json!("0".repeat(64))
        };
        fs::write(
            package.root.join("package.json"),
            to_canonical_json(&broken),
        )
        .unwrap();
        let output = package.invoke(None);
        status(&output, 2);
        assert!(String::from_utf8_lossy(&output.stderr).contains("IdentityMismatch"));
    }
    package.save(value);
    let existing = package.root.join("existing");
    fs::create_dir(&existing).unwrap();
    status(&package.invoke(Some(&existing)), 1);
    assert_eq!(fs::read_dir(existing).unwrap().count(), 0);
    let inside = package.root.join("client/output");
    status(&package.invoke(Some(&inside)), 1);
    assert!(!inside.exists());
    let missing = package.root.join("missing/output");
    status(&package.invoke(Some(&missing)), 1);
    assert!(!missing.exists());
}

#[test]
fn package_requires_strict_objects_members_keys_and_region_profile() {
    let package = Package::new();
    let original = package.value();
    let text = to_canonical_json(&original);
    for (needle, replacement) in [
        (
            "\"schemaVersion\": 1",
            "\"schemaVersion\": 1, \"schemaVersion\": 1",
        ),
        ("\"rootKey\":", "\"unknown\": 0, \"rootKey\":"),
        ("\"path\":", "\"size\": 1, \"path\":"),
        ("\"likeChild\": null,", ""),
        ("\"childType\": null,", ""),
    ] {
        assert!(text.contains(needle), "{needle}");
        fs::write(
            package.root.join("package.json"),
            text.replacen(needle, replacement, 1),
        )
        .unwrap();
        status(&package.invoke(None), 2);
    }
    for (pointer, replacement) in [
        ("", json!([])),
        ("/region", json!([])),
        ("/project", json!([])),
        ("/region/like", json!("0x00000000")),
        ("/region/rootKey", json!("0x00010000")),
        ("/region/rootName", json!("template")),
        ("/region/childName", json!("new-root")),
        ("/region/childType", json!(1)),
        ("/region/likeChild", json!(9)),
        ("/profile", json!("other")),
        ("/schemaVersion", json!(1.0)),
        ("/layoutKey", json!("0xf1000000")),
        ("/layoutKey", json!("0x00000000")),
        ("/layoutKey", json!("0xF100FFFF")),
        ("/project/path", json!("../project.json")),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        fs::write(package.root.join("package.json"), to_canonical_json(&value)).unwrap();
        status(&package.invoke(None), 2);
    }
    package.save(original);
    let mut value = package.value();
    value["region"]["likeChild"] = json!(1);
    value["region"]["childType"] = json!(0x4000);
    package.save(value);
    let output = package.invoke(None);
    status(&output, 0);
    let manifest: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(manifest["registration"]["templateChildIndex"], 1);
    assert_eq!(manifest["registration"]["childType"], 0x4000);
    let output = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .args(["validate-zone-package", "file", "--output", "dir"])
        .output()
        .unwrap();
    status(&output, 1);
    let help = Command::new(env!("CARGO_BIN_EXE_xivl"))
        .arg("--help")
        .output()
        .unwrap();
    status(&help, 0);
    assert!(String::from_utf8_lossy(&help.stdout).contains("build-zone-package <package.json>"));
}

fn overlay_package(package: &Package, overlays: &[&Path], output: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xivl"));
    command
        .arg(if output.is_some() {
            "build-zone-package"
        } else {
            "validate-zone-package"
        })
        .arg(package.root.join("package.json"))
        .arg("--client-root")
        .arg(package.root.join("client"));
    for overlay in overlays {
        command.arg("--overlay").arg(overlay);
    }
    if let Some(output) = output {
        command.arg("--output").arg(output);
    }
    command.output().unwrap()
}

#[test]
fn package_extends_effective_overlay_catalog_with_unchanged_floor_outputs() {
    schema(
        &serde_json::from_str(include_str!(
            "../../../docs/zone-package-manifest.example.json"
        ))
        .unwrap(),
        include_str!("../../../schemas/zone-package-manifest.schema.json"),
    );
    let package = Package::new();
    let overlay = package.root.join("overlay");
    let path = overlay.join(CATALOG);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = catalog();
    bytes[0xE0] = b'x';
    fs::write(&path, &bytes).unwrap();
    let result = overlay_package(&package, &[&overlay], None);
    status(&result, 0);
    let validated: Value = serde_json::from_slice(&result.stdout).unwrap();
    schema(
        &validated,
        include_str!("../../../schemas/zone-package-manifest.schema.json"),
    );
    assert_eq!(
        validated["inputs"]["catalog"]["source"],
        json!({"overlay": 0})
    );
    assert_eq!(validated["inputs"]["catalog"]["sha256"], sha256_hex(&bytes));
    assert_eq!(
        validated["inputs"]["overlays"],
        json!([{"index": 0, "datFileCount": 1}])
    );
    let destination = package.root.join("built");
    status(
        &overlay_package(&package, &[&overlay], Some(&destination)),
        0,
    );
    let built = fs::read(destination.join(CATALOG)).unwrap();
    assert_eq!(&built[..0x20], &bytes[..0x20]);
    assert_eq!(&built[0x28..bytes.len()], &bytes[0x28..]);
    assert_ne!(&built[0x28..bytes.len()], &catalog()[0x28..]);
    assert_eq!(built.len(), bytes.len() + 0x60);
    let baseline = package.root.join("baseline");
    status(&package.invoke(Some(&baseline)), 0);
    for path in [
        "data/F1/00/00/00.DAT",
        "data/FF/FF/00/01.DAT",
        "data/FF/FF/00/02.DAT",
    ] {
        assert_eq!(
            fs::read(destination.join(path)).unwrap(),
            fs::read(baseline.join(path)).unwrap()
        );
    }
    let inside = overlay.join("output");
    let result = overlay_package(&package, &[&overlay], Some(&inside));
    status(&result, 1);
    assert!(String::from_utf8_lossy(&result.stderr).contains("overlay 0"));
    assert!(!inside.exists());
}

#[test]
fn package_refuses_overlay_occupancy_and_every_pinned_client_input() {
    let package = Package::new();
    let overlay = package.root.join("overlay");
    let occupied = overlay.join("data/F1/00/00/00.DAT");
    fs::create_dir_all(occupied.parent().unwrap()).unwrap();
    fs::write(&occupied, b"occupied").unwrap();
    let destination = package.root.join("refused");
    let result = overlay_package(&package, &[&overlay], Some(&destination));
    status(&result, 2);
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(
        error.contains("layout") && error.contains("0xF1000000"),
        "{error}"
    );
    assert!(!destination.exists());
    fs::remove_file(occupied).unwrap();
    let empty = package.root.join("empty-overlay");
    fs::create_dir_all(empty.join("data")).unwrap();
    for path in [
        "layout.bin",
        "model.bin",
        "old-collision.bin",
        "unplaced.bin",
        "collision-template.bin",
    ] {
        fs::write(overlay.join(path), b"shadow").unwrap();
        let result = overlay_package(&package, &[&empty, &overlay], Some(&destination));
        status(&result, 2);
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains(path) && error.contains("overlay 1"),
            "{error}"
        );
        assert!(!destination.exists());
        fs::remove_file(overlay.join(path)).unwrap();
    }
}
