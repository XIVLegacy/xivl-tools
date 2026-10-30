//! Additive offline packages assembled from the existing floor and catalog writers.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use xivl_cli::zone_project::{self, FileReference, ObjectOnly, ProjectFailure, ProjectFailureKind};
use xivl_formats::digest::sha256_hex;
use xivl_formats::region::{self, AuxiliaryKey, RegionMembership};
use xivl_formats::region_write::{append_region_root, RegionAppend};
use xivl_formats::zone::{parse_layout_with_structure, parse_model, parse_phb};
use xivl_formats::{to_canonical_json, ResourceId};

use crate::{allocation_inventory, append_region, Failure};

const CATALOG: &str = "data/03/C0/00/00.DAT";

#[derive(Clone, Copy)]
struct Key(u32);

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.len() != 10
            || !text.starts_with("0x")
            || !text.as_bytes()[2..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte))
        {
            return Err(serde::de::Error::custom(
                "expected 0x and eight uppercase hexadecimal digits",
            ));
        }
        u32::from_str_radix(&text[2..], 16)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Package {
    schema_version: u32,
    profile: String,
    project: FileReference,
    layout_key: Key,
    region: ObjectOnly<Registration>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Registration {
    like: Key,
    #[serde(deserialize_with = "nullable")]
    like_child: Option<usize>,
    root_key: Key,
    root_name: String,
    child_key: Key,
    child_name: String,
    #[serde(deserialize_with = "nullable")]
    child_type: Option<u32>,
}

fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

struct Prepared {
    files: Vec<(String, Vec<u8>)>,
    manifest: Value,
}

fn invalid(role: &str, detail: impl Into<String>) -> ProjectFailure {
    ProjectFailure::new(ProjectFailureKind::InvalidProject, role, detail)
}

fn project_failure(failure: Failure) -> ProjectFailure {
    ProjectFailure::new(
        if failure.code == 1 {
            ProjectFailureKind::Io
        } else {
            ProjectFailureKind::InvalidProject
        },
        "allocation inventory",
        failure.message,
    )
}

fn identity(path: &str, bytes: &[u8]) -> Value {
    json!({"path": path, "size": bytes.len(), "sha256": sha256_hex(bytes)})
}

fn parse_package(bytes: &[u8]) -> Result<Package, ProjectFailure> {
    let ObjectOnly(package): ObjectOnly<Package> =
        serde_json::from_slice(bytes).map_err(|error| {
            invalid(
                "package",
                format!(
                    "JSON at line {}, column {}: {error}",
                    error.line(),
                    error.column()
                ),
            )
        })?;
    if package.schema_version != 1 || package.profile != "additive-floor-v1" {
        return Err(invalid(
            "package",
            "expected schemaVersion 1 and profile additive-floor-v1",
        ));
    }
    zone_project::validate_reference(&package.project, "project", zone_project::MAX_PROJECT_BYTES)?;
    Ok(package)
}

fn prepare(path: &Path, client_root: &Path) -> Result<Prepared, ProjectFailure> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = zone_project::checked_root(parent, "package")?;
    let name = path
        .file_name()
        .ok_or_else(|| invalid("package", "expected a file name"))?;
    let path = zone_project::checked_path(&parent, Path::new(name), "package")?;
    let bytes = zone_project::read_bounded(&path, "package", zone_project::MAX_PROJECT_BYTES)?;
    let package = parse_package(&bytes)?;
    let project = zone_project::load_input(
        &parent,
        &package.project,
        "project",
        zone_project::MAX_PROJECT_BYTES,
        &mut 0,
    )?;
    let project_path =
        zone_project::checked_path(&parent, Path::new(&package.project.path), "project")?;
    let loaded =
        zone_project::load_project_with_bytes(&project_path, client_root, Some(&project.bytes))
            .map_err(|failure| {
                if !matches!(failure.kind, ProjectFailureKind::InvalidProject) {
                    return failure;
                }
                if let Ok(ObjectOnly(saved)) =
                    serde_json::from_slice::<ObjectOnly<zone_project::ZoneProject>>(&project.bytes)
                {
                    if saved.binding.output_model_id == saved.binding.output_collision_id {
                        return invalid(
                            "collision",
                            format!(
                                "{}: key collides with the model allocation",
                                saved.binding.output_collision_id.0.to_hex()
                            ),
                        );
                    }
                }
                failure
            })?;
    let model_key = loaded.project.binding.output_model_id.0.value();
    let collision_key = loaded.project.binding.output_collision_id.0.value();
    let floor = zone_project::prepare_loaded(loaded)?;
    let client_root = zone_project::checked_root(client_root, "client root")?;
    // The inventory and append writer consume the same catalog bytes.
    zone_project::checked_path(&client_root, Path::new(CATALOG), "catalog")?;
    let inventory = allocation_inventory::inventory(&client_root).map_err(project_failure)?;
    let table = &inventory.catalog;
    let registration = package.region.0;
    let append = RegionAppend {
        root_key: registration.root_key.0,
        root_name: registration.root_name.into_bytes(),
        child_key: registration.child_key.0,
        child_name: registration.child_name.into_bytes(),
        child_resource: package.layout_key.0,
        child_type: registration.child_type,
    };
    if append.child_resource & 0xFFFF == 0xFFFF {
        return Err(invalid(
            "layout",
            format!(
                "{}: low16 must not be 0xFFFF",
                ResourceId::new(append.child_resource).to_hex()
            ),
        ));
    }
    let mut seen = HashSet::new();
    let mut allocations = Vec::new();
    let mut check = |role: &str, key: u32| -> Result<(), ProjectFailure> {
        let hex = ResourceId::new(key).to_hex();
        let refuse = |reason| invalid(role, format!("{hex}: {reason}"));
        if key == 0 {
            return Err(refuse("key must be nonzero"));
        }
        if !seen.insert(key) {
            return Err(refuse("key collides with another package allocation"));
        }
        if inventory.scanned_keys.contains(&key) {
            return Err(refuse("key is occupied by a scanned numeric DAT"));
        }
        if allocation_inventory::numeric_path_exists(&client_root, key).map_err(project_failure)? {
            return Err(refuse("uppercase numeric path already exists"));
        }
        for (index, row) in table.rows.iter().enumerate() {
            if row.dat_key == key {
                return Err(refuse(
                    "key is occupied by a catalog child resource or root +0x08 word",
                ));
            }
            if matches!(row.membership, RegionMembership::Root { .. }) {
                if row.word_20 == key {
                    return Err(refuse("key is occupied by an explicit root +0x20 request"));
                }
                if region::auxiliary_key(table, index) == Some(AuxiliaryKey::Derived(key)) {
                    return Err(refuse(
                        "key is occupied by a derived catalog auxiliary resource",
                    ));
                }
            }
        }
        allocations.push(json!({"role": role, "key": hex, "numericPath": allocation_inventory::numeric_path(key),
            "occupancyChecks": {"nonzero": true, "pairwiseDistinct": true, "scannedNumericDat": "absent",
                "uppercaseNumericPath": "absent", "catalogChildResource": "absent", "catalogRootWord08": "absent",
                "catalogRootWord20": "absent", "catalogDerivedAuxiliary": "absent"}}));
        Ok(())
    };
    // One child with low16 below 0xFFFF derives the next low-index key.
    let expected_auxiliary = append.child_resource + 1;
    for (role, key) in [
        ("layout", append.child_resource),
        ("model", model_key),
        ("collision", collision_key),
        ("auxiliary", expected_auxiliary),
    ] {
        check(role, key)?;
    }
    let catalog = append_region_root(
        &inventory.catalog_bytes,
        registration.like.0,
        registration.like_child,
        &append,
    )
    .map_err(|error| ProjectFailure::format("catalog registration", error))?;
    let appended =
        region::parse(&catalog).map_err(|error| ProjectFailure::format("catalog", error))?;
    let index = appended.rows.len() - 2;
    let Some(AuxiliaryKey::Derived(auxiliary)) = region::auxiliary_key(&appended, index) else {
        return Err(invalid(
            "auxiliary",
            "appended root has no derived auxiliary key",
        ));
    };
    if auxiliary != expected_auxiliary {
        return Err(invalid(
            "auxiliary",
            "appended root derives an unexpected auxiliary key",
        ));
    }
    let child_type = appended.rows[index + 1].word_04;
    append_region::verify(
        &inventory.catalog_bytes,
        &catalog,
        table.root_count,
        &append,
        child_type,
        auxiliary,
    )
    .map_err(|failure| invalid("catalog verification", failure.message))?;
    parse_layout_with_structure(&floor.layout)
        .map_err(|error| ProjectFailure::format("layout", error))?;
    parse_model(&floor.model).map_err(|error| ProjectFailure::format("model", error))?;
    parse_phb(&floor.collision).map_err(|error| ProjectFailure::format("collision", error))?;
    let roles = ["catalog", "layout", "model", "collision"];
    // Move the prepared vectors directly; no re-encoding changes the floor writer's bytes.
    let files = vec![
        (CATALOG.into(), catalog),
        (
            allocation_inventory::numeric_path(append.child_resource),
            floor.layout,
        ),
        (allocation_inventory::numeric_path(model_key), floor.model),
        (
            allocation_inventory::numeric_path(collision_key),
            floor.collision,
        ),
    ];
    let outputs = files
        .iter()
        .zip(roles)
        .map(|((path, bytes), role)| {
            let mut output = identity(path, bytes);
            output["role"] = json!(role);
            output["new"] = json!(true);
            output
        })
        .collect::<Vec<_>>();
    let template_index = table
        .rows
        .iter()
        .position(|row| {
            matches!(row.membership, RegionMembership::Root { .. }) && row.id == registration.like.0
        })
        .ok_or_else(|| invalid("catalog", "template root disappeared"))?;
    let template = &table.rows[template_index];
    let children = &table.rows[template_index + 1..template_index + 1 + template.word_0c as usize];
    let template_child = registration
        .like_child
        .or_else(|| {
            children
                .iter()
                .position(|row| !matches!(row.word_04, 0 | 1))
        })
        .ok_or_else(|| invalid("catalog", "template child disappeared"))?;
    let mut blockers = inventory.report["blockers"]
        .as_array()
        .cloned()
        .ok_or_else(|| invalid("inventory", "missing blockers"))?;
    blockers.extend([
        json!({"id": "auxiliary-payload-absent", "key": ResourceId::new(auxiliary).to_hex(), "statement": "The reserved auxiliary key has no output payload."}),
        json!({"id": "scene-transitive-dependencies", "statement": "Scene and transitive dependency coverage remains unresolved."}),
        json!({"id": "client-acceptance", "statement": "Client acceptance remains unverified."}),
    ]);
    let name = name
        .to_str()
        .ok_or_else(|| invalid("package", "file name must be ASCII"))?;
    zone_project::validate_reference(
        &FileReference {
            path: name.into(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        },
        "package",
        zone_project::MAX_PROJECT_BYTES,
    )?;
    let manifest = json!({
        "schemaVersion": 1, "profile": "additive-floor-v1", "toolVersion": env!("CARGO_PKG_VERSION"),
        "status": "offline-validated", "clientVerification": "unverified", "complete": false,
        "inputs": {"package": identity(name, &bytes), "project": identity(&package.project.path, &project.bytes),
            "catalog": identity(CATALOG, &inventory.catalog_bytes), "zoneProject": floor.report["inputs"], "directResources": floor.report["resources"]},
        "registration": {"rootKey": ResourceId::new(append.root_key).to_hex(), "childKey": ResourceId::new(append.child_key).to_hex(),
            "childType": child_type, "templateRootKey": ResourceId::new(registration.like.0).to_hex(), "templateChildIndex": template_child,
            "rootNameByteLength": append.root_name.len(), "childNameByteLength": append.child_name.len(),
            "rootCountBefore": table.root_count, "rootCountAfter": appended.root_count, "sizeWordBefore": table.declared_size,
            "sizeWordAfter": appended.declared_size, "rootRowOffset": inventory.catalog_bytes.len(), "childRowOffset": inventory.catalog_bytes.len() + region::ROW_STRIDE,
            "changedHeaderSpan": {"offset": 32, "length": 8}},
        "allocations": allocations, "outputs": outputs,
        "dependencies": {"directResources": floor.report["resources"], "sceneDependencies": "unresolved", "transitiveDependencies": "unresolved"},
        "blockers": blockers,
    });
    Ok(Prepared { files, manifest })
}

fn publish(
    mut prepared: Prepared,
    client_root: &Path,
    output: &Path,
) -> Result<(), ProjectFailure> {
    let output = zone_project::new_output_directory(client_root, output)?;
    for (path, bytes) in &prepared.files {
        let path = output.join(path);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| invalid("output", "missing parent"))?,
        )
        .map_err(|error| {
            ProjectFailure::new(ProjectFailureKind::Io, "output", error.to_string())
        })?;
        zone_project::write_new(&path, bytes)?;
        let written = zone_project::read_bounded(&path, "output", bytes.len() as u64)?;
        if written != *bytes {
            return Err(invalid(
                "output",
                "written bytes differ from prepared bytes",
            ));
        }
    }
    prepared.manifest["status"] = json!("offline-built");
    let pending = output.join("package-manifest.json.pending");
    zone_project::write_new(&pending, to_canonical_json(&prepared.manifest).as_bytes())?;
    let manifest = output.join("package-manifest.json");
    match fs::symlink_metadata(&manifest) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ProjectFailure::new(
                ProjectFailureKind::Io,
                "output",
                error.to_string(),
            ))
        }
        Ok(_) => return Err(invalid("output", "package-manifest.json already exists")),
    }
    fs::rename(pending, manifest)
        .map_err(|error| ProjectFailure::new(ProjectFailureKind::Io, "output", error.to_string()))
}

pub fn run(arguments: &[String], build: bool) -> Result<(), Failure> {
    let execute = || -> Result<(), ProjectFailure> {
        let usage = || {
            ProjectFailure::new(
                ProjectFailureKind::Usage,
                "zone package command",
                if build {
                    "usage: xivl build-zone-package <package.json> --client-root <root> --output <new-directory>"
                } else {
                    "usage: xivl validate-zone-package <package.json> --client-root <root>"
                },
            )
        };
        let package = arguments
            .first()
            .filter(|arg| !arg.is_empty() && !arg.starts_with("--"))
            .ok_or_else(usage)?;
        let mut root = None;
        let mut output = None;
        let (pairs, remainder) = arguments[1..].as_chunks::<2>();
        for [flag, value] in pairs {
            if value.is_empty() || value.starts_with("--") {
                return Err(usage());
            }
            match flag.as_str() {
                "--client-root" if root.is_none() => root = Some(Path::new(value)),
                "--output" if build && output.is_none() => output = Some(Path::new(value)),
                _ => return Err(usage()),
            }
        }
        if !remainder.is_empty() || (build && output.is_none()) {
            return Err(usage());
        }
        let root = root.ok_or_else(usage)?;
        let prepared = prepare(Path::new(package), root)?;
        if let Some(output) = output {
            publish(prepared, root, output)
        } else {
            std::io::stdout()
                .write_all(to_canonical_json(&prepared.manifest).as_bytes())
                .map_err(|error| {
                    ProjectFailure::new(ProjectFailureKind::Io, "stdout", error.to_string())
                })
        }
    };
    execute().map_err(|error| Failure {
        message: error.to_string(),
        code: error.code(),
    })
}
