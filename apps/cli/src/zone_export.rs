//! Explicit-client-root world geometry export.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::json;
use xivl_formats::digest::sha256_hex;
use xivl_formats::zone::{flatten_zone, parse_layout, parse_model, parse_phb, SourceInfo};
use xivl_formats::{parse_dat_path, to_canonical_json, ResourceId};

use crate::{read_capped, Failure};

pub const COLLECTION_SCHEMA_VERSION: u32 = 1;

struct Asset {
    id: ResourceId,
    relative_path: String,
    path: PathBuf,
}

#[derive(Debug)]
pub struct ZoneExportSummary {
    pub zones: usize,
    pub output: String,
}

pub fn run(arguments: &[String]) -> Result<ZoneExportSummary, Failure> {
    let Some(root) = arguments.first() else {
        return Err(Failure::usage(
            "usage: xivl export-zones <client-root> --output <directory> [--layout <resource-id>]",
        ));
    };
    let mut output = None;
    let mut selected_layout = None;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--output" if index + 1 < arguments.len() => {
                if output.replace(arguments[index + 1].clone()).is_some() {
                    return Err(Failure::usage("--output was supplied more than once"));
                }
                index += 2;
            }
            "--layout" if index + 1 < arguments.len() => {
                if selected_layout.is_some() {
                    return Err(Failure::usage("--layout was supplied more than once"));
                }
                let text = &arguments[index + 1];
                let id = parse_dat_id(text)?;
                selected_layout = Some(id);
                index += 2;
            }
            option => {
                return Err(Failure::usage(format!(
                    "unknown export-zones option '{option}'"
                )))
            }
        }
    }
    let output =
        output.ok_or_else(|| Failure::usage("export-zones requires --output <directory>"))?;
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        return Err(Failure::usage(format!(
            "client root '{}' is not a directory",
            root_path.display()
        )));
    }
    let output_path = Path::new(&output);
    crate::scan::require_empty_output(output_path)?;
    let assets = inventory(root_path)?;
    let mut layouts = Vec::new();
    for asset in assets.values() {
        if selected_layout.is_some_and(|id| id != asset.id) {
            continue;
        }
        if !is_layout_asset(asset)? {
            continue;
        }
        let bytes = read_capped(&asset.path.display().to_string())?;
        let layout = parse_layout(&bytes).map_err(|error| parse_failure(asset, error))?;
        layouts.push((asset.id, layout));
    }
    layouts.sort_by_key(|(id, layout)| (layout.zone_name.to_ascii_lowercase(), id.value()));
    if layouts.is_empty() {
        return Err(Failure::parse(
            "client root contains no supported lyb layouts",
        ));
    }

    // All decoding happens before the first write so a malformed model or PHB
    // cannot leave a partial collection behind.
    let mut products = Vec::with_capacity(layouts.len());
    for (layout_id, layout) in layouts {
        let mut phbs = Vec::new();
        for object in &layout.objects {
            if let Some(model_id) = object.model {
                let model = assets.get(&model_id.value()).ok_or_else(|| {
                    Failure::parse(format!(
                        "{}: layout references missing model {}",
                        layout_id.to_hex(),
                        model_id.to_hex()
                    ))
                })?;
                let model_bytes = read_capped(&model.path.display().to_string())?;
                parse_model(&model_bytes).map_err(|error| parse_failure(model, error))?;
            }
            if let Some(phb_id) = object.phb {
                if phbs.iter().any(|(existing_id, _)| *existing_id == phb_id) {
                    continue;
                }
                let phb = assets.get(&phb_id.value()).ok_or_else(|| {
                    Failure::parse(format!(
                        "{}: layout references missing PHB {}",
                        layout_id.to_hex(),
                        phb_id.to_hex()
                    ))
                })?;
                let phb_bytes = read_capped(&phb.path.display().to_string())?;
                let parsed_phb =
                    parse_phb(&phb_bytes).map_err(|error| parse_failure(phb, error))?;
                phbs.push((phb_id, parsed_phb));
            }
        }
        let flattened =
            flatten_zone(&layout, &phbs).map_err(|error| Failure::parse(error.to_string()))?;
        let mut sources = Vec::new();
        for (kind, id) in used_ids_for_layout(&layout, layout_id) {
            let asset = assets.get(&id.value()).expect("validated resource id");
            let source_bytes = read_capped(&asset.path.display().to_string())?;
            sources.push(SourceInfo {
                kind: kind.into(),
                resource_id: id,
                path: asset.relative_path.clone(),
                sha256: sha256_hex(&source_bytes),
            });
        }
        sources.sort_by_key(|source| source.resource_id.value());
        let obj = flattened.obj();
        let metadata = flattened.metadata(&layout.zone_name, &sources);
        let metadata_text = to_canonical_json(&metadata);
        products.push(Product {
            zone_name: layout.zone_name.clone(),
            layout_id,
            output_name: String::new(),
            sources,
            obj,
            metadata_text,
        });
    }

    let mut manifest_zones = Vec::new();
    let mut name_counts = BTreeMap::<String, usize>::new();
    for product in &products {
        let base = safe_zone_name(&product.zone_name, product.layout_id);
        *name_counts.entry(base.to_ascii_lowercase()).or_default() += 1;
    }
    let mut output_names = BTreeSet::new();
    let mut allocate_name = |base: String, layout_id: ResourceId| {
        let mut candidate = base;
        let mut collision_index = 0u32;
        loop {
            if output_names.insert(candidate.to_ascii_lowercase()) {
                return candidate;
            }
            collision_index += 1;
            candidate = format!(
                "{}-{:08X}-{}",
                candidate,
                layout_id.value(),
                collision_index
            );
        }
    };
    for product in products.iter_mut().filter(|product| {
        let base = safe_zone_name(&product.zone_name, product.layout_id);
        name_counts[&base.to_ascii_lowercase()] > 1
    }) {
        let base = safe_zone_name(&product.zone_name, product.layout_id);
        product.output_name = allocate_name(
            format!("{base}-{:08X}", product.layout_id.value()),
            product.layout_id,
        );
    }
    for product in products.iter_mut().filter(|product| {
        let base = safe_zone_name(&product.zone_name, product.layout_id);
        name_counts[&base.to_ascii_lowercase()] == 1
    }) {
        let base = safe_zone_name(&product.zone_name, product.layout_id);
        product.output_name = allocate_name(base, product.layout_id);
    }
    fs::create_dir_all(output_path).map_err(|error| {
        Failure::usage(format!(
            "cannot create output directory '{}': {error}",
            output_path.display()
        ))
    })?;
    for product in &products {
        let zone_name = &product.output_name;
        let obj_path = format!("zones/{zone_name}.obj");
        let metadata_path = format!("zones/{zone_name}.metadata.json");
        let obj_bytes = product.obj.as_bytes();
        let metadata_bytes = product.metadata_text.as_bytes();
        let obj_destination = output_path.join(&obj_path);
        let metadata_destination = output_path.join(&metadata_path);
        fs::create_dir_all(obj_destination.parent().expect("zone path has a parent"))
            .map_err(|error| Failure::usage(error.to_string()))?;
        fs::write(&obj_destination, obj_bytes).map_err(|error| {
            Failure::usage(format!(
                "cannot write '{}': {error}",
                obj_destination.display()
            ))
        })?;
        fs::write(&metadata_destination, metadata_bytes).map_err(|error| {
            Failure::usage(format!(
                "cannot write '{}': {error}",
                metadata_destination.display()
            ))
        })?;
        manifest_zones.push(json!({
            "layoutName": product.zone_name,
            "outputName": zone_name,
            "layoutResourceId": product.layout_id.to_hex(),
            "layoutPath": product.layout_id.dat_path(),
            "source": product.sources.iter().map(SourceInfo::json).collect::<Vec<_>>(),
            "output": {
                "obj": {"path": obj_path, "size": obj_bytes.len(), "sha256": sha256_hex(obj_bytes)},
                "metadata": {"path": metadata_path, "size": metadata_bytes.len(), "sha256": sha256_hex(metadata_bytes)}
            }
        }));
    }
    let selection = json!({
        "mode": if selected_layout.is_some() { "explicit-layout" } else { "all-supported-layouts" },
        "layoutResourceId": selected_layout.map(ResourceId::to_hex),
        "layoutCount": products.len(),
    });
    let manifest = json!({
        "schemaVersion": COLLECTION_SCHEMA_VERSION,
        "format": "xivl-zone-geometry-collection",
        "identity": {
            "clientVersion": "1.23b",
            "target": "Final Fantasy XIV 1.23b",
            "tool": "xivl",
            "exporterVersion": env!("CARGO_PKG_VERSION"),
            "geometrySchemaVersion": xivl_formats::zone::GEOMETRY_SCHEMA_VERSION
        },
        "selection": selection,
        "settings": {
            "coordinateSystem": "y-up",
            "transformOrder": "T*Rz*Ry*Rx*S",
            "collisionWinding": "two-sided-source-order",
            "geometry": "collision-only",
            "modelMeshes": "decoded-for-source-association-only",
            "sourceClassifications": ["render", "collision"]
        },
        "zoneCount": manifest_zones.len(),
        "zones": manifest_zones
    });
    let manifest_destination = output_path.join("manifest.json");
    fs::write(&manifest_destination, to_canonical_json(&manifest)).map_err(|error| {
        Failure::usage(format!(
            "cannot write '{}': {error}",
            manifest_destination.display()
        ))
    })?;
    Ok(ZoneExportSummary {
        zones: products.len(),
        output: manifest_destination.display().to_string(),
    })
}

struct Product {
    zone_name: String,
    layout_id: ResourceId,
    output_name: String,
    sources: Vec<SourceInfo>,
    obj: String,
    metadata_text: String,
}

fn inventory(root: &Path) -> Result<BTreeMap<u32, Asset>, Failure> {
    let data_root = root.join("data");
    let mut paths = Vec::new();
    collect_dat_paths(&data_root, &mut paths)?;
    paths.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    let mut assets = BTreeMap::new();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Failure::usage("resource path escaped the client root"))?
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(id) = parse_dat_path(&relative, 0) else {
            continue;
        };
        if assets.contains_key(&id.value()) {
            return Err(Failure::usage(format!(
                "duplicate resource path for {}",
                id.to_hex()
            )));
        }
        assets.insert(
            id.value(),
            Asset {
                id,
                relative_path: relative,
                path,
            },
        );
    }
    Ok(assets)
}

fn collect_dat_paths(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), Failure> {
    for entry in fs::read_dir(directory).map_err(|error| Failure::usage(error.to_string()))? {
        let entry = entry.map_err(|error| Failure::usage(error.to_string()))?;
        let kind = entry
            .file_type()
            .map_err(|error| Failure::usage(error.to_string()))?;
        let path = entry.path();
        if kind.is_symlink() {
            return Err(Failure::usage(format!(
                "export-zones does not follow symbolic link '{}'",
                path.display()
            )));
        }
        if kind.is_dir() {
            collect_dat_paths(&path, output)?;
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("DAT"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn looks_like_layout(bytes: &[u8]) -> bool {
    bytes.len() >= 0x40 && bytes.starts_with(b"MapLayoutResourceData")
}

fn is_layout_asset(asset: &Asset) -> Result<bool, Failure> {
    let mut file = fs::File::open(&asset.path).map_err(|error| {
        Failure::usage(format!("cannot read '{}': {error}", asset.path.display()))
    })?;
    let mut prefix = [0u8; 0x40];
    match file.read_exact(&mut prefix) {
        Ok(()) => Ok(looks_like_layout(&prefix)),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(Failure::usage(format!(
            "cannot read '{}': {error}",
            asset.path.display()
        ))),
    }
}

fn used_ids_for_layout(
    layout: &xivl_formats::zone::Layout,
    layout_id: ResourceId,
) -> Vec<(&'static str, ResourceId)> {
    let mut ids = vec![("layout", layout_id)];
    for object in &layout.objects {
        if let Some(model) = object.model {
            ids.push(("model", model));
        }
        if let Some(id) = object.phb {
            ids.push(("phb", id));
        }
    }
    ids.sort_by_key(|(_, id)| id.value());
    ids.dedup_by_key(|(_, id)| id.value());
    ids
}

fn parse_dat_id(text: &str) -> Result<ResourceId, Failure> {
    xivl_formats::resource::parse_resource_id(text, 0)
        .map_err(|error| Failure::usage(format!("invalid layout resource id: {error}")))
}

fn parse_failure(asset: &Asset, error: xivl_formats::FormatError) -> Failure {
    Failure::parse(format!("{}: {error}", asset.relative_path))
}

fn safe_zone_name(name: &str, id: ResourceId) -> String {
    let mut output: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if output.is_empty() || output == "." || output == ".." {
        output = format!("zone-{:08X}", id.value());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "xivl-zone-export-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_string(bytes: &mut [u8], offset: usize, value: &str) {
        bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());
    }

    fn layout_bytes() -> Vec<u8> {
        let mut layout = vec![0u8; 0x60];
        layout[..21].copy_from_slice(b"MapLayoutResourceData");
        put_u32(&mut layout, 0x20, 0x60);
        put_u32(&mut layout, 0x24, 1);
        layout[0x40..0x44].copy_from_slice(b"mesh");
        layout[0x50..0x53].copy_from_slice(b"bhp");
        put_u32(&mut layout, 0x54, 0x05060708);

        let mut payload = vec![0u8; 0xa00];
        payload[..4].copy_from_slice(b"lyb\0");
        let payload_len = (0x30 + payload.len()) as u32;
        put_u32(&mut payload, 0x04, payload_len);
        put_u32(&mut payload, 0x10, 0x20);
        put_u32(&mut payload, 0x14, 0x18);
        // The scene-settings entry is followed by the flat InstanceObject
        // enumeration used by retail lyb files.
        put_u32(&mut payload, 0x18, 0x680);
        put_u32(&mut payload, 0x1c, 0x500);
        let rows = [
            (0x500, 0x40),
            (0x540, 0x50),
            (0x590, 0x30),
            (0x5c0, 0x30),
            (0x5f0, 0x2c),
            (0x700, 0x14),
            (0x714, 0x14),
            (0x728, 0x14),
            (0x73c, 0x14),
            (0x680, 0x70),
            (0x750, 0x14),
            // InstanceObject child ranges use the retail 0x0C descriptor
            // rather than a direct ChildObject run.
            (0x900, 0x0C),
        ];
        let fields: [&[(u16, u16)]; 12] = [
            &[
                (0x04, 0),
                (0x08, 2),
                (0x10, 0),
                (0x2c, 0),
                (0x30, 0),
                (0x3c, 0),
                (0x34, 0),
            ],
            &[
                (0x04, 0),
                (0x08, 2),
                (0x1c, 0),
                (0x20, 0),
                (0x2c, 2),
                (0x24, 0),
                (0x30, 0),
            ],
            &[
                (0x00, 0),
                (0x08, 2),
                (0x0c, 2),
                (0x10, 2),
                (0x14, 0),
                (0x1c, 2),
                (0x20, 2),
                (0x24, 2),
                (0x28, 2),
            ],
            &[(0x14, 2), (0x0c, 0), (0x10, 0), (0x18, 0), (0x20, 0)],
            &[(0x04, 0), (0x08, 2), (0x24, 2), (0x14, 0), (0x1c, 0)],
            &[(0x04, 0), (0x08, 2), (0x10, 0)],
            &[(0x04, 0), (0x08, 2), (0x10, 0)],
            &[(0x04, 0), (0x08, 2), (0x10, 0)],
            &[(0x04, 0), (0x08, 2), (0x10, 0)],
            &[
                (0x04, 0),
                (0x08, 2),
                (0x4c, 0),
                (0x50, 0),
                (0x5c, 0),
                (0x1c, 2),
                (0x54, 0),
                (0x60, 0),
            ],
            &[(0x04, 0), (0x08, 2), (0x10, 0)],
            &[(0x00, 0)],
        ];
        let mut field_list = 0xe0;
        for (index, (offset, size)) in rows.into_iter().enumerate() {
            let at = 0x20 + index * 16;
            let extent = 0x220 + index * 12;
            put_u32(&mut payload, at, extent as u32);
            put_u32(&mut payload, at + 4, field_list);
            put_u32(&mut payload, at + 8, 1);
            put_u32(&mut payload, at + 12, fields[index].len() as u32);
            for &(field_offset, field_kind) in fields[index] {
                payload[field_list as usize..field_list as usize + 2]
                    .copy_from_slice(&field_offset.to_le_bytes());
                payload[field_list as usize + 2..field_list as usize + 4]
                    .copy_from_slice(&field_kind.to_le_bytes());
                field_list += 4;
            }
            put_u32(&mut payload, extent, offset);
            put_u32(&mut payload, extent + 4, 1);
            put_u32(&mut payload, extent + 8, size);
        }
        put_u32(&mut payload, 0x504, 0x700);
        put_f32(&mut payload, 0x520, 1.0);
        put_u32(&mut payload, 0x53c, 0x540);
        put_u32(&mut payload, 0x534, 0x900);
        put_u32(&mut payload, 0x538, 1);
        put_u32(&mut payload, 0x544, 0x714);
        put_f32(&mut payload, 0x550, 2.0);
        put_u32(&mut payload, 0x564, 0x590);
        put_u32(&mut payload, 0x568, 1);
        put_u32(&mut payload, 0x594, 0x728);
        put_f32(&mut payload, 0x590, 3.0);
        put_u32(&mut payload, 0x5b0, 0x5c0);
        put_u32(&mut payload, 0x5b4, 1);
        put_f32(&mut payload, 0x5c0, 4.0);
        put_u32(&mut payload, 0x5d8, 0x5f0);
        put_u32(&mut payload, 0x5f4, 0x73c);
        put_u32(&mut payload, 0x614, 0x820);
        put_u32(&mut payload, 0x684, 0x750);
        put_u32(&mut payload, 0x688, 0x860);
        put_f32(&mut payload, 0x6c0, 10.0);
        put_u32(&mut payload, 0x6cc, 0);
        put_u32(&mut payload, 0x6d0, 0);
        put_u32(&mut payload, 0x708, 0x760);
        put_u32(&mut payload, 0x71c, 0x780);
        put_u32(&mut payload, 0x730, 0x7c0);
        put_u32(&mut payload, 0x744, 0x7e0);
        put_u32(&mut payload, 0x758, 0x830);
        put_u32(&mut payload, 0x900, 0x5c0);
        put_u32(&mut payload, 0x904, 1);
        put_u32(&mut payload, 0x908, 15);
        put_string(&mut payload, 0x760, "RefObjects/InstanceObject\0");
        put_string(&mut payload, 0x780, "RefObjects/UnitTree/UnitTreeObject\0");
        put_string(&mut payload, 0x7c0, "RefObjects/Group/GroupObject\0");
        put_string(
            &mut payload,
            0x7e0,
            "BaseObjects/Attribute/AttributeBaseObject\0",
        );
        put_string(&mut payload, 0x820, "mesh\0");
        put_string(
            &mut payload,
            0x830,
            "MiscObjects/LaySettings/LaySettingsObject\0",
        );
        put_string(&mut payload, 0x860, "test-zone\0");

        let total = 0x30 + payload.len();
        let mut scene = vec![0u8; 0x30];
        scene[..4].copy_from_slice(b"SEDB");
        scene[4..8].copy_from_slice(b"lyb\0");
        scene[0x0e..0x10].copy_from_slice(&0x30u16.to_le_bytes());
        scene[0x10..0x14].copy_from_slice(&(total as u32).to_le_bytes());
        scene.extend_from_slice(&payload);
        layout.extend_from_slice(&scene);
        layout
    }

    fn set_layout_name(layout: &mut [u8], name: &str) {
        let start = 0x60 + 0x30 + 0x860;
        layout[start..start + 32].fill(0);
        put_string(layout, start, name);
    }

    fn phb_bytes() -> Vec<u8> {
        let chunk_size = 0xc0;
        let mut bytes = vec![0u8; 0xc0 + chunk_size + 0x10];
        bytes[..4].copy_from_slice(b"SEDB");
        bytes[4..8].copy_from_slice(b"PHB\0");
        bytes[0x0e..0x10].copy_from_slice(&0x48u16.to_le_bytes());
        bytes[0x10..0x14].copy_from_slice(&0x48u32.to_le_bytes());
        let chunk = 0xc0;
        bytes[chunk..chunk + 8].copy_from_slice(b"PHB.GBD\0");
        put_u32(&mut bytes, chunk + 8, 0xc0);
        put_f32(&mut bytes, chunk + 0x40, 0.0);
        put_f32(&mut bytes, chunk + 0x44, 0.0);
        put_f32(&mut bytes, chunk + 0x48, 0.0);
        put_f32(&mut bytes, chunk + 0x50, 1.0);
        put_f32(&mut bytes, chunk + 0x54, 1.0);
        put_f32(&mut bytes, chunk + 0x58, 0.0);
        put_u32(&mut bytes, chunk + 0x38, 0x80);
        put_u32(&mut bytes, chunk + 0x3c, 3);
        put_u32(&mut bytes, chunk + 0x60, 0xb0);
        put_u32(&mut bytes, chunk + 0x64, 1);
        for (index, value) in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .enumerate()
        {
            put_f32(&mut bytes, chunk + 0x80 + index * 4, value);
        }
        bytes[chunk + 0xb0..chunk + 0xb8].copy_from_slice(&[0, 0, 1, 0, 2, 0, 0xfe, 0xca]);
        bytes
    }

    #[test]
    fn export_requires_output_and_writes_traced_collection() {
        let root = temp_root();
        fs::create_dir_all(root.join("data")).unwrap();
        let layout_path = root.join(ResourceId::new(0x29D90003).dat_path());
        let phb_path = root.join(ResourceId::new(0x05060708).dat_path());
        let unrelated_path = root.join(ResourceId::new(0x8AD00000).dat_path());
        for path in [&layout_path, &phb_path, &unrelated_path] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        fs::write(layout_path, layout_bytes()).unwrap();
        fs::write(phb_path, phb_bytes()).unwrap();
        fs::File::create(unrelated_path)
            .unwrap()
            .set_len(268_435_457)
            .unwrap();
        let output = root.join("out");
        let summary = run(&[
            root.display().to_string(),
            "--output".into(),
            output.display().to_string(),
        ])
        .unwrap();
        assert_eq!(summary.zones, 1);
        let obj = fs::read_to_string(output.join("zones/test-zone.obj")).unwrap();
        assert!(obj
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("v 20 ")));
        assert!(obj
            .lines()
            .nth(1)
            .is_some_and(|line| line.starts_with("v 21 ")));
        assert!(obj.contains("f 1 2 3\n"));
        let metadata: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(output.join("zones/test-zone.metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["schemaVersion"], 2);
        assert!(metadata.get("faces").is_none());
        assert_eq!(metadata["faceRanges"][0]["faceStart"], 1);
        assert_eq!(metadata["faceRanges"][0]["faceCount"], 1);
        assert_eq!(metadata["faceRanges"][0]["source"]["triangleCount"], 1);
        assert_eq!(metadata["faceRanges"][0]["source"]["rawSurface"], 0xCAFE);
        assert_eq!(metadata["faceRanges"][0]["source"]["placementIndex"], 0);
        assert_eq!(metadata["placements"][0]["source"]["nodeOffset"], 0x5F0);
        assert_eq!(metadata["placements"][0]["source"]["blockOffset"], 0x5F0);
        assert_eq!(metadata["placements"][0]["source"]["resourceKey"], "mesh");
        assert_eq!(metadata["placements"][0]["classifications"][0], "collision");
        assert_eq!(
            metadata["placements"][0]["transformChain"]["zone"]["translation"][0],
            10.0
        );
        assert_eq!(metadata["placements"][0]["worldMatrix"][0][3], 20.0);
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(output.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["zoneCount"], 1);
        assert_eq!(manifest["zones"][0]["layoutResourceId"], "0x29D90003");
        assert_eq!(manifest["identity"]["target"], "Final Fantasy XIV 1.23b");
        assert_eq!(manifest["identity"]["clientVersion"], "1.23b");
        assert_eq!(
            manifest["identity"]["exporterVersion"],
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(manifest["selection"]["mode"], "all-supported-layouts");
        assert_eq!(manifest["selection"]["layoutCount"], 1);
        assert_eq!(manifest["settings"]["geometry"], "collision-only");
        let second_output = root.join("out-second");
        run(&[
            root.display().to_string(),
            "--output".into(),
            second_output.display().to_string(),
        ])
        .unwrap();
        for relative in [
            "zones/test-zone.obj",
            "zones/test-zone.metadata.json",
            "manifest.json",
        ] {
            assert_eq!(
                fs::read(output.join(relative)).unwrap(),
                fs::read(second_output.join(relative)).unwrap()
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_rejects_missing_output_option() {
        let error = run(&["client".into()]).unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.message.contains("requires --output"));
    }

    #[test]
    fn duplicate_zone_names_use_layout_id_suffixes() {
        let root = temp_root();
        fs::create_dir_all(root.join("data")).unwrap();
        let first = root.join(ResourceId::new(0x29D90003).dat_path());
        let second = root.join(ResourceId::new(0x29D90004).dat_path());
        let phb = root.join(ResourceId::new(0x05060708).dat_path());
        for path in [&first, &second, &phb] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        fs::write(first, layout_bytes()).unwrap();
        fs::write(second, layout_bytes()).unwrap();
        fs::write(phb, phb_bytes()).unwrap();
        let output = root.join("out");
        let summary = run(&[
            root.display().to_string(),
            "--output".into(),
            output.display().to_string(),
        ])
        .unwrap();
        assert_eq!(summary.zones, 2);
        assert!(output.join("zones/test-zone-29D90003.obj").is_file());
        assert!(output.join("zones/test-zone-29D90004.obj").is_file());
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(output.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["zones"][0]["layoutName"], "test-zone");
        assert_eq!(manifest["zones"][0]["outputName"], "test-zone-29D90003");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unique_name_collision_with_duplicate_suffix_is_disambiguated() {
        let root = temp_root();
        fs::create_dir_all(root.join("data")).unwrap();
        let first = root.join(ResourceId::new(0x29D90003).dat_path());
        let second = root.join(ResourceId::new(0x29D90004).dat_path());
        let colliding = root.join(ResourceId::new(0x29D90005).dat_path());
        let phb = root.join(ResourceId::new(0x05060708).dat_path());
        for path in [&first, &second, &colliding, &phb] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        fs::write(first, layout_bytes()).unwrap();
        fs::write(second, layout_bytes()).unwrap();
        let mut unique = layout_bytes();
        set_layout_name(&mut unique, "test-zone-29D90003");
        fs::write(colliding, unique).unwrap();
        fs::write(phb, phb_bytes()).unwrap();
        let output = root.join("out");
        run(&[
            root.display().to_string(),
            "--output".into(),
            output.display().to_string(),
        ])
        .unwrap();
        assert!(output
            .join("zones/test-zone-29D90003-29D90005-1.obj")
            .is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
