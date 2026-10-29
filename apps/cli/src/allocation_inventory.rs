//! Read-only occupied registration inventory. See docs/allocation-inventory.md.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::region::{self, AuxiliaryKey, RegionMembership};
use xivl_formats::to_canonical_json;

use crate::{read_capped, scan, Failure};

const CATALOG_PATH: &str = "data/03/C0/00/00.DAT";
const CASE_RULE: &str = "ASCII case-insensitive hex digits and DAT extension; lowercase data directory; exactly two hex digits per byte";
const DECOMP_REVISION: &str = "cb3639a9c03ad79724b52b710337a166d52de8ea";
const STRUCTS_REVISION: &str = "fff480ab502bfae6b3e555d62679b635479821de";

pub fn run(arguments: &[String]) -> Result<(), Failure> {
    let mut client_root = None;
    let mut output = None;
    let (options, remainder) = arguments.as_chunks::<2>();
    for pair in options {
        let target = match pair[0].as_str() {
            "--client-root" => &mut client_root,
            "--output" => &mut output,
            option => {
                return Err(Failure::usage(format!(
                    "unknown allocation-inventory option '{option}'"
                )))
            }
        };
        if target.replace(PathBuf::from(&pair[1])).is_some() {
            return Err(Failure::usage(format!(
                "{} was supplied more than once",
                pair[0]
            )));
        }
    }
    if !remainder.is_empty() || client_root.is_none() {
        return Err(Failure::usage(
            "usage: xivl allocation-inventory --client-root <root> [--output <new-file>]",
        ));
    }
    let client_root = client_root.ok_or_else(|| Failure::usage("--client-root is required"))?;
    let root = fs::canonicalize(client_root)
        .map_err(|error| Failure::usage(format!("cannot resolve client root: {error}")))?;
    let output = output.map(|path| output_path(&root, &path)).transpose()?;
    let report = inventory(&root)?;
    let text = to_canonical_json(&report);
    if let Some(path) = output {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| Failure::usage(format!("cannot create output: {error}")))?;
        file.write_all(text.as_bytes()).map_err(|error| {
            // This process created the file, so a failed write removes it.
            drop(file);
            let _ = fs::remove_file(&path);
            Failure::usage(format!("cannot write output: {error}"))
        })
    } else {
        std::io::stdout()
            .write_all(text.as_bytes())
            .map_err(|error| Failure::usage(format!("cannot write stdout: {error}")))
    }
}

fn output_path(root: &Path, path: &Path) -> Result<PathBuf, Failure> {
    match fs::symlink_metadata(path) {
        Ok(_) => return Err(Failure::usage("output path already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Failure::usage(format!("cannot inspect output: {error}"))),
    }
    let name = path
        .file_name()
        .ok_or_else(|| Failure::usage("output requires a new filename"))?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = fs::canonicalize(parent.unwrap_or(Path::new(".")))
        .map_err(|error| Failure::usage(format!("cannot resolve output parent: {error}")))?;
    // Compare file identities too, so aliases and Windows case do not bypass containment.
    for ancestor in parent.ancestors() {
        if same_file::is_same_file(root, ancestor)
            .map_err(|error| Failure::usage(format!("cannot compare output parent: {error}")))?
        {
            return Err(Failure::usage("output path is inside the client root"));
        }
    }
    Ok(parent.join(name))
}

fn inventory(root: &Path) -> Result<Value, Failure> {
    let data_root = root.join("data");
    let kind = fs::symlink_metadata(&data_root)
        .map_err(|error| Failure::usage(format!("cannot inspect data directory: {error}")))?;
    if kind.file_type().is_symlink() {
        return Err(Failure::usage("data directory is a symbolic link"));
    }
    let mut paths = Vec::new();
    scan::collect_dat_paths(&data_root, &mut paths)?;
    let mut occupied = HashSet::new();
    let mut unmapped = Vec::new();
    for path in paths {
        let relative = scan::relative_path(root, &path);
        if let Some(key) = numeric_key(&relative) {
            occupied.insert(key);
        } else {
            unmapped.push(relative);
        }
    }
    let mut keys: Vec<_> = occupied.iter().copied().collect();
    keys.sort_unstable();
    unmapped.sort();

    let bytes = read_capped(&root.join(CATALOG_PATH).display().to_string())?;
    let table = region::parse(&bytes)
        .map_err(|error| Failure::parse(format!("{CATALOG_PATH}: {error}")))?;
    let mut root_keys = HashMap::<u32, Vec<usize>>::new();
    let mut child_keys = HashMap::<u32, Vec<usize>>::new();
    let mut names = HashMap::<String, Vec<usize>>::new();
    let mut resources = HashMap::<u32, Vec<usize>>::new();
    let mut explicit_resources = HashMap::<u32, Vec<usize>>::new();
    let mut auxiliary = HashMap::<u32, Vec<usize>>::new();
    let mut roots = Vec::new();
    let mut index = 0;
    while index < table.rows.len() {
        let row = &table.rows[index];
        let RegionMembership::Root {
            index: root_index,
            child_count,
        } = row.membership
        else {
            return Err(Failure::parse("region parser returned an ungrouped child"));
        };
        root_keys.entry(row.id).or_default().push(index);
        if row.word_20 != 0 {
            explicit_resources
                .entry(row.word_20)
                .or_default()
                .push(index);
        }
        let name = row_name(&bytes, row)?;
        names.entry(name.clone()).or_default().push(index);
        let terminated = region::token_terminated(&bytes, row)
            .ok_or_else(|| Failure::parse("region token is outside parsed input"))?;
        let (status, derived) = match region::auxiliary_key(&table, index) {
            Some(AuxiliaryKey::Derived(key)) => {
                auxiliary.entry(key).or_default().push(index);
                ("derived", Some(key))
            }
            Some(AuxiliaryKey::NonzeroLayoutArgument) => ("nonzero-layout-argument", None),
            Some(AuxiliaryKey::NoChildren) => ("no-children", None),
            Some(AuxiliaryKey::LowIndexOverflow) => ("low-index-overflow", None),
            None => return Err(Failure::parse("region root has no auxiliary status")),
        };
        let mut children = Vec::new();
        for child_index in 0..child_count as usize {
            let row_index = index + 1 + child_index;
            let child = &table.rows[row_index];
            child_keys.entry(child.id).or_default().push(row_index);
            resources.entry(child.dat_key).or_default().push(row_index);
            let child_name = row_name(&bytes, child)?;
            names.entry(child_name.clone()).or_default().push(row_index);
            children.push(json!({
                "rowIndex": row_index,
                "childIndex": child_index,
                "key": hex(child.id),
                "type": child.word_04,
                "resourceKey": hex(child.dat_key),
                "kind": child.word_0c,
                "effectiveName": child_name,
                "eligible": !matches!(child.word_04, 0 | 1),
                "numericPath": numeric_path(child.dat_key),
                "numericPathExists": numeric_path_exists(root, child.dat_key)?,
            }));
        }
        roots.push(json!({
            "rowIndex": index,
            "rootIndex": root_index,
            "key": hex(row.id),
            "nonzero": row.id != 0,
            "representable16Bit": row.id <= u16::MAX as u32,
            "word04": row.word_04,
            "word08": row.dat_key,
            "word20": row.word_20,
            "effectiveName": name,
            "tokenTerminated": terminated,
            "childCount": child_count,
            "auxiliary": {
                "status": status,
                "key": derived.map(hex),
                "numericPath": derived.map(numeric_path),
                "numericPathExists": derived.map(|key| numeric_path_exists(root, key)).transpose()?,
            },
            "children": children,
        }));
        index += 1 + child_count as usize;
    }

    let mut auxiliary_keys: Vec<_> = auxiliary.keys().copied().collect();
    auxiliary_keys.sort_unstable();
    let mut resource_collisions = Vec::new();
    for key in auxiliary_keys {
        let file_exists = numeric_path_exists(root, key)?;
        let child_rows = resources.get(&key).cloned().unwrap_or_default();
        let explicit_root_rows = explicit_resources.get(&key).cloned().unwrap_or_default();
        // A scanned key is occupied even if its spelling is not openable in uppercase.
        if file_exists
            || occupied.contains(&key)
            || !child_rows.is_empty()
            || !explicit_root_rows.is_empty()
        {
            resource_collisions.push(json!({
                "key": hex(key),
                "rootRows": auxiliary[&key],
                "childRows": child_rows,
                "explicitRootRows": explicit_root_rows,
                "numericPathExists": file_exists,
            }));
        }
    }
    let mut duplicate_names: Vec<_> = names
        .into_iter()
        .filter(|(_, rows)| rows.len() > 1)
        .collect();
    duplicate_names.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(json!({
        "schemaVersion": 1,
        "catalog": {
            "key": "0x03C00000",
            "path": CATALOG_PATH,
            "size": bytes.len(),
            "sha256": sha256_hex(&bytes),
            "rootCount": table.root_count,
            "rowCount": table.rows.len(),
            "sizeWord": table.declared_size,
            "sizeMatchesInput": u64::from(table.declared_size) == bytes.len() as u64,
            "trailingBytes": table.trailing.length,
        },
        "pathInventory": {
            "caseRule": CASE_RULE,
            "occupiedKeyCount": keys.len(),
            "occupiedKeys": keys.into_iter().map(hex).collect::<Vec<_>>(),
            "unmappedPathCount": unmapped.len(),
            "unmappedPaths": unmapped,
        },
        "roots": roots,
        "collisions": {
            "duplicateRootKeys": duplicate_keys(root_keys),
            "duplicateChildKeys": duplicate_keys(child_keys),
            "duplicateEffectiveNames": duplicate_names.into_iter().map(|(name, rows)| json!({
                "effectiveName": name, "rows": rows,
            })).collect::<Vec<_>>(),
            "auxiliaryResourceKeys": resource_collisions,
            "auxiliaryDerivedKeys": duplicate_keys(auxiliary),
        },
        "blockers": blockers(),
    }))
}

fn duplicate_keys(groups: HashMap<u32, Vec<usize>>) -> Vec<Value> {
    let mut duplicates: Vec<_> = groups
        .into_iter()
        .filter(|(_, rows)| rows.len() > 1)
        .collect();
    duplicates.sort_by_key(|(key, _)| *key);
    duplicates
        .into_iter()
        .map(|(key, rows)| json!({ "key": hex(key), "rows": rows }))
        .collect()
}

fn row_name(bytes: &[u8], row: &region::RegionRow) -> Result<String, Failure> {
    let name = region::effective_name(bytes, row)
        .ok_or_else(|| Failure::parse("region name is outside parsed input"))?;
    let mut text = String::new();
    for byte in name {
        match byte {
            b'\\' => text.push_str("\\\\"),
            0x20..=0x7E => text.push(char::from(*byte)),
            _ => text.push_str(&format!("\\x{byte:02X}")),
        }
    }
    Ok(text)
}

fn hex(key: u32) -> String {
    format!("0x{key:08X}")
}

fn numeric_path(key: u32) -> String {
    let [a, b, c, d] = key.to_be_bytes();
    format!("data/{a:02X}/{b:02X}/{c:02X}/{d:02X}.DAT")
}

fn numeric_path_exists(root: &Path, key: u32) -> Result<bool, Failure> {
    root.join(numeric_path(key))
        .try_exists()
        .map_err(|error| Failure::usage(format!("cannot inspect numeric resource path: {error}")))
}

fn numeric_key(path: &str) -> Option<u32> {
    let mut parts = path.split('/');
    if parts.next()? != "data" {
        return None;
    }
    let mut key = 0;
    for index in 0..4 {
        let part = parts.next()?;
        let digits = if index == 3 {
            if part.len() != 6 || !part.get(2..)?.eq_ignore_ascii_case(".DAT") {
                return None;
            }
            part.get(..2)?
        } else {
            part
        };
        if digits.len() != 2 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        key = (key << 8) | u32::from(u8::from_str_radix(digits, 16).ok()?);
    }
    parts.next().is_none().then_some(key)
}

fn blockers() -> Value {
    json!([
        {
            "id": "numeric-mode-assumed",
            "statement": "Numeric formatter mode is assumed. Mode-zero path-table mappings are unsupported; numeric paths cannot prove free identities outside numeric mode.",
            "source": format!("xivl-decomp:docs/resource/resource-path-producer.md \"Catalog mode\" at {DECOMP_REVISION}"),
        },
        {
            "id": "auxiliary-payload-acceptance",
            "statement": "Authored auxiliary payload acceptance and whether absence is acceptable remain unresolved. File existence does not establish either.",
            "source": format!("xivl-decomp:docs/resource/region-auxiliary-resource.md \"Selection readiness and remaining evidence\" at {DECOMP_REVISION}"),
        },
        {
            "id": "zone-script-join",
            "statement": "The numeric zone, ZoneMaster class and script load join remains unresolved. This report does not inventory or allocate those identities.",
            "source": format!("xivl-client-structs:docs/map-layout-selector.md \"Zone-master and script dependencies\" at {STRUCTS_REVISION}"),
        },
        {
            "id": "integration-zone-limit",
            "statement": "The zone identity range 0..511 is an integration limit, not a demonstrated native bound or evidence that an identity is free.",
            "source": format!("xivl-client-structs:docs/map-layout-selector.md \"Minimal candidate and allocation constraints\" at {STRUCTS_REVISION}"),
        },
    ])
}
