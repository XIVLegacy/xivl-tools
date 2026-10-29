//! Import edited lossless CSV views into isolated sheet block resources.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde_json::{json, Value};
use xivl_formats::csv::{parse_cell, read_table};
use xivl_formats::digest::sha256_hex;
use xivl_formats::sheet::{
    encode_enable_file, encode_row_offsets, encode_rows, parse_enable_file, parse_row_offsets,
    parse_rows,
};
use xivl_formats::{ColumnValue, CsvTable, ResourceId, Row, Span};

use crate::extract::{self, LoadedBlock};
use crate::{ExtractFailure as Failure, ExtractFailureKind};

const USAGE: &str = "usage: xivl import-sheets <game-directory> --csv <extract-output-directory> --output <new-directory>";

pub fn run(arguments: &[String]) -> Result<Value, Failure> {
    let Some(game) = arguments.first() else {
        return Err(Failure::usage(USAGE));
    };
    let mut csv = None;
    let mut output = None;
    let mut position = 1;
    while position < arguments.len() {
        let slot = match arguments[position].as_str() {
            "--csv" => &mut csv,
            "--output" => &mut output,
            option => {
                return Err(Failure::usage(format!(
                    "unknown import-sheets option '{option}'"
                )))
            }
        };
        let value = arguments
            .get(position + 1)
            .ok_or_else(|| Failure::usage(USAGE))?;
        if slot.replace(value).is_some() {
            return Err(Failure::usage(
                "import-sheets option was supplied more than once",
            ));
        }
        position += 2;
    }
    import_directory(
        Path::new(game),
        Path::new(csv.ok_or_else(|| Failure::usage(USAGE))?),
        Path::new(output.ok_or_else(|| Failure::usage(USAGE))?),
    )
}

#[derive(Default)]
struct Counts {
    documents: usize,
    csv_files: usize,
    cells: usize,
    enabled: usize,
    disabled: usize,
}

/// Write only resources whose bytes differ from the client. Absent CSVs leave
/// documents untouched; all CSV and block validation precedes materialization.
pub fn import_directory(game: &Path, csv: &Path, output: &Path) -> Result<Value, Failure> {
    let output = output_path(game, output)?;
    import_into_directory(game, csv, &output)
}

fn import_into_directory(game: &Path, csv: &Path, output: &Path) -> Result<Value, Failure> {
    let definitions = extract::discover_definitions(game)?;
    let mut used = BTreeSet::new();
    let mut named = Vec::new();
    for (source, document) in definitions {
        if let Some(first) = document.sheets.first() {
            let name = extract::unique_name(&extract::safe_name(&first.name), &mut used);
            named.push((name, source, document));
        }
    }
    let names: BTreeSet<_> = named.iter().map(|(name, _, _)| name.as_str()).collect();
    for entry in fs::read_dir(csv).map_err(io_failure)? {
        let path = entry.map_err(io_failure)?.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| names.contains(name))
        {
            return Err(invalid(format!(
                "{}: CSV name is not produced by extract",
                path.display()
            )));
        }
    }
    let mut counts = Counts::default();
    let mut resources = BTreeMap::<u32, (ResourceId, &'static str, Vec<u8>)>::new();
    let mut expected_resources = BTreeMap::new();
    let mut intended = Vec::new();
    for (name, source, document) in named {
        counts.documents += 1;
        let (table, blocks) = extract::load_document(
            game,
            &source,
            &document,
            &mut extract::ExtractSummary::default(),
        )?;
        let path = csv.join(name);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                for block in &blocks {
                    register_resources(
                        &mut expected_resources,
                        block,
                        &block.data,
                        &block.enable,
                        &block.offsets,
                    )?;
                }
                continue;
            }
            Err(error) => {
                return Err(Failure::io(format!(
                    "cannot read '{}': {error}",
                    path.display()
                )))
            }
        };
        counts.csv_files += 1;
        let rows = read_table(&text, &table.columns).map_err(|error| {
            let message = format!("{}: {error}", path.display());
            Failure::format(error, message)
        })?;
        validate_cells(&path, &table, &blocks, &rows, &mut counts)?;
        for block in blocks {
            let original_resources =
                resource_signatures(&block, &block.data, &block.enable, &block.offsets);
            if let Some(edited) = edit_block(&path, &table, &rows, block)? {
                register_resources(
                    &mut expected_resources,
                    &edited.block,
                    &edited.data,
                    &edited.enable,
                    &edited.offsets,
                )?;
                for (id, role, old, bytes) in [
                    (
                        edited.block.definition.data,
                        "data",
                        &edited.block.data,
                        &edited.data,
                    ),
                    (
                        edited.block.definition.enable,
                        "enable",
                        &edited.block.enable,
                        &edited.enable,
                    ),
                    (
                        edited.block.definition.offsets,
                        "offsets",
                        &edited.block.offsets,
                        &edited.offsets,
                    ),
                ] {
                    if old == bytes {
                        continue;
                    }
                    if let Some((_, previous_role, previous_bytes)) = resources.get(&id.value()) {
                        if *previous_role != role || previous_bytes != bytes {
                            return Err(invalid(format!(
                                "{}: conflicting edits to shared resource",
                                id.dat_path()
                            )));
                        }
                    } else {
                        resources.insert(id.value(), (id, role, bytes.clone()));
                    }
                }
                intended.push(edited);
            } else {
                register_signatures(&mut expected_resources, original_resources)?;
            }
        }
    }
    let mut created = CreatedPaths::default();
    created.directory(output)?;
    let result: Result<Value, Failure> = (|| {
        let mut report = Vec::new();
        for (id, role, bytes) in resources.values() {
            let path = extract::resource_path(output, *id);
            created.directory(path.parent().unwrap())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(io_failure)?;
            created.files.push(path);
            file.write_all(bytes).map_err(io_failure)?;
            report.push(json!({ "id": format!("{:08X}", id.value()), "path": id.dat_path(), "role": role, "length": bytes.len(), "sha256": sha256_hex(bytes) }));
        }
        for block in &intended {
            verify_written(game, output, &resources, block)?;
        }
        Ok(json!({
            "schemaVersion": 1, "operation": "import-sheets", "resources": report,
            "counts": { "documentsRead": counts.documents, "csvFilesRead": counts.csv_files,
                "cellsChanged": counts.cells, "rowsEnabled": counts.enabled, "rowsDisabled": counts.disabled,
                "resourcesWritten": resources.len() }
        }))
    })();
    match result {
        Ok(report) => {
            created.keep = true;
            Ok(report)
        }
        Err(error) => {
            if let Err(cleanup) = created.cleanup() {
                return Err(Failure::io(format!("{}; {cleanup}", error.message())));
            }
            Err(error)
        }
    }
}

fn resource_signatures(
    block: &LoadedBlock,
    data: &[u8],
    enable: &[u8],
    offsets: &[u8],
) -> [(ResourceId, String); 3] {
    [
        (block.definition.data, sha256_hex(data)),
        (block.definition.enable, sha256_hex(enable)),
        (block.definition.offsets, sha256_hex(offsets)),
    ]
}

fn register_resources(
    expected: &mut BTreeMap<u32, String>,
    block: &LoadedBlock,
    data: &[u8],
    enable: &[u8],
    offsets: &[u8],
) -> Result<(), Failure> {
    register_signatures(expected, resource_signatures(block, data, enable, offsets))
}

fn register_signatures(
    expected: &mut BTreeMap<u32, String>,
    signatures: [(ResourceId, String); 3],
) -> Result<(), Failure> {
    for (id, digest) in signatures {
        if let Some(previous) = expected.get(&id.value()) {
            if previous != &digest {
                return Err(invalid(format!(
                    "{}: definitions require conflicting shared resource contents",
                    id.dat_path()
                )));
            }
        } else {
            expected.insert(id.value(), digest);
        }
    }
    Ok(())
}

fn validate_cells(
    path: &Path,
    table: &CsvTable,
    blocks: &[LoadedBlock],
    rows: &BTreeMap<u32, Vec<String>>,
    counts: &mut Counts,
) -> Result<(), Failure> {
    counts.disabled += table
        .rows
        .keys()
        .filter(|id| !rows.contains_key(id))
        .count();
    for (&id, cells) in rows {
        if !blocks.iter().any(|block| contains(block, id)) {
            return Err(invalid(format!("{}: row {id}: no present block covers this row; adding rows outside the shipped block ranges needs a definition-document change, which import-sheets does not perform", path.display())));
        }
        if !table.rows.contains_key(&id) {
            counts.enabled += 1;
        }
        for (column, text) in cells.iter().enumerate() {
            let original_cell = table.rows.get(&id).and_then(|row| row[column].as_deref());
            let original = original_cell.unwrap_or("");
            let changed = original != text;
            let stored = blocks.iter().any(|block| {
                contains(block, id)
                    && block.indexes.contains(&(column as u32))
                    && (block.rows.contains_key(&id) || !table.rows.contains_key(&id))
            });
            if original_cell.is_none() && text.is_empty() && !stored {
                continue;
            }
            if has_marker(original) {
                if changed {
                    return Err(cell_failure(
                        path,
                        id,
                        column,
                        "missing or duplicate cells cannot be edited",
                    ));
                }
                continue;
            }
            if let Some(kind) = table.columns[column] {
                parse_cell(text, kind, false)
                    .map_err(|error| cell_failure(path, id, column, error))?;
            } else if !text.is_empty() {
                return Err(cell_failure(path, id, column, "column is not declared"));
            }
            if changed || (!table.rows.contains_key(&id) && stored) {
                counts.cells += 1;
            }
            if changed && !stored {
                return Err(cell_failure(
                    path,
                    id,
                    column,
                    "no present block stores this column for this row",
                ));
            }
        }
    }
    Ok(())
}

fn has_marker(text: &str) -> bool {
    // Escaped brackets are literal text, including literal marker spellings.
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            characters.next();
        } else if character == '['
            && (characters.as_str().starts_with("@missing]")
                || characters.as_str().starts_with("@duplicate:"))
        {
            return true;
        }
    }
    false
}

struct EditedBlock {
    block: LoadedBlock,
    rows: Vec<Row>,
    ids: Vec<u32>,
    data: Vec<u8>,
    enable: Vec<u8>,
    offsets: Vec<u8>,
}

fn contains(block: &LoadedBlock, id: u32) -> bool {
    id >= block.definition.begin
        && u64::from(id) < u64::from(block.definition.begin) + u64::from(block.definition.count)
}

fn edit_block(
    path: &Path,
    table: &CsvTable,
    csv: &BTreeMap<u32, Vec<String>>,
    block: LoadedBlock,
) -> Result<Option<EditedBlock>, Failure> {
    let defaults: Vec<bool> = block
        .columns
        .iter()
        .enumerate()
        .map(|(column, _)| {
            block
                .rows
                .first_key_value()
                .and_then(|(_, values)| values[column].as_ref())
                .is_some_and(|value| matches!(value, ColumnValue::Text(string) if string.scrambled))
        })
        .collect();
    let mut rows = Vec::new();
    let mut ids = Vec::new();
    let mut changed = block.rows.keys().any(|id| !csv.contains_key(id));
    for (&id, cells) in csv.range(block.definition.begin..) {
        if !contains(&block, id) {
            break;
        }
        let original = block.rows.get(&id);
        // A merged CSV line does not enable a language row that extract already
        // represented through another definition; only a newly inserted id does.
        if original.is_none() && table.rows.contains_key(&id) {
            continue;
        }
        let mut values = Vec::new();
        for (column, (&logical, &kind)) in block.indexes.iter().zip(&block.columns).enumerate() {
            let text = &cells[logical as usize];
            let previous_text = table
                .rows
                .get(&id)
                .and_then(|row| row[logical as usize].as_deref())
                .unwrap_or("");
            if let Some(old) = original {
                if text == previous_text {
                    if let Some(value) = &old[column] {
                        values.push(value.clone());
                    }
                    continue;
                }
                if old[column].is_none() {
                    return Err(cell_failure(
                        path,
                        id,
                        logical as usize,
                        "omitted trailing value cannot be filled",
                    ));
                }
            }
            let scrambled = original.and_then(|values| values[column].as_ref()).map_or(
                defaults[column],
                |value| matches!(value, ColumnValue::Text(string) if string.scrambled),
            );
            let value = parse_cell(text, kind, scrambled)
                .map_err(|error| cell_failure(path, id, logical as usize, error))?;
            values.push(value);
            changed = true;
        }
        if original.is_none() {
            changed = true;
        }
        rows.push(Row {
            index: u64::from(id - block.definition.begin),
            span: Span::new(0, 0),
            values,
        });
        ids.push(id);
    }
    if !changed {
        return Ok(None);
    }
    let encoded = encode_rows(&rows)
        .map_err(|error| extract::resource_failure(block.definition.data, error))?;
    let slots: Vec<_> = rows
        .iter()
        .zip(encoded.row_ends)
        .map(|(row, end)| (row.index, end))
        .collect();
    // Keep a truncated array truncated unless a newly enabled row needs it.
    let slot_count = slots
        .last()
        .map_or(0, |&(index, _)| index as usize + 1)
        .max(block.offsets.len() / 4);
    let offsets = encode_row_offsets(slot_count, &slots)
        .map_err(|error| extract::resource_failure(block.definition.offsets, error))?;
    let enable = if ids.iter().copied().eq(block.rows.keys().copied()) {
        block.enable.clone()
    } else {
        encode_enable_file(&runs(&ids))
    };
    Ok(Some(EditedBlock {
        block,
        rows,
        ids,
        data: encoded.data,
        enable,
        offsets,
    }))
}

fn runs(ids: &[u32]) -> Vec<(u32, u32)> {
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for &id in ids {
        if let Some((begin, count)) = ranges.last_mut() {
            if u64::from(*begin) + u64::from(*count) == u64::from(id) && *count < u32::MAX {
                *count += 1;
                continue;
            }
        }
        ranges.push((id, 1));
    }
    ranges
}

fn verify_written(
    game: &Path,
    output: &Path,
    resources: &BTreeMap<u32, (ResourceId, &'static str, Vec<u8>)>,
    edited: &EditedBlock,
) -> Result<(), Failure> {
    let read = |id: ResourceId| {
        let root = if resources.contains_key(&id.value()) {
            output
        } else {
            game
        };
        let bytes = fs::read(extract::resource_path(root, id)).map_err(io_failure)?;
        if resources
            .get(&id.value())
            .is_some_and(|(_, _, expected)| expected != &bytes)
        {
            return Err(invalid(format!(
                "{}: written resource bytes disagree with intended bytes",
                id.dat_path()
            )));
        }
        Ok(bytes)
    };
    let definition = &edited.block.definition;
    let data = read(definition.data)?;
    let enable = parse_enable_file(&read(definition.enable)?)
        .map_err(|error| extract::resource_failure(definition.enable, error))?;
    let offsets = parse_row_offsets(&read(definition.offsets)?)
        .map_err(|error| extract::resource_failure(definition.offsets, error))?;
    if !enable.anomalies.is_empty()
        || !offsets.anomalies.is_empty()
        || offsets.slot_count() > definition.count as usize
        || offsets.data_length() != data.len() as u64
        || offsets.rows.len() != edited.rows.len()
        || enable
            .ranges
            .iter()
            .flat_map(|range| {
                (0..range.count).map(move |offset| u64::from(range.first_row) + u64::from(offset))
            })
            .ne(edited.ids.iter().map(|id| u64::from(*id)))
    {
        return Err(invalid(
            "written block resources disagree with intended rows",
        ));
    }
    for ((slot, expected), &id) in offsets.rows.iter().zip(&edited.rows).zip(&edited.ids) {
        if slot.index + u64::from(definition.begin) != u64::from(id) {
            return Err(invalid("written row slot disagrees with intended row id"));
        }
        let bytes = &data[slot.span.offset as usize..slot.span.end() as usize];
        // Omitted trailing columns stay omitted; parse only the intended prefix.
        let parsed = parse_rows(bytes, &edited.block.columns[..expected.values.len()])
            .map_err(|error| extract::resource_failure(definition.data, error))?;
        if parsed.len() != 1
            || encode_rows(&parsed)
                .map_err(|error| extract::resource_failure(definition.data, error))?
                .data
                != encode_rows(std::slice::from_ref(expected))
                    .map_err(|error| extract::resource_failure(definition.data, error))?
                    .data
        {
            return Err(invalid("written row values disagree with intended values"));
        }
    }
    Ok(())
}

fn output_path(game: &Path, path: &Path) -> Result<PathBuf, Failure> {
    let root = fs::canonicalize(game).map_err(io_failure)?;
    let absolute = std::path::absolute(path).map_err(io_failure)?;
    let existing = absolute
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .ok_or_else(|| invalid("output has no existing ancestor"))?;
    let parent = fs::canonicalize(existing).map_err(io_failure)?;
    for ancestor in parent.ancestors() {
        if same_file::is_same_file(&root, ancestor).map_err(|error| {
            Failure::io(format!(
                "cannot compare output ancestor '{}': {error}",
                ancestor.display()
            ))
        })? {
            return Err(invalid("output path is inside the game directory"));
        }
    }
    let suffix = absolute
        .strip_prefix(existing)
        .map_err(|_| invalid("cannot resolve output path"))?;
    if suffix
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid("output has unresolved path components"));
    }
    let resolved = parent.join(suffix);
    if resolved.exists()
        && (!resolved.is_dir()
            || fs::read_dir(&resolved)
                .map_err(io_failure)?
                .next()
                .is_some())
    {
        return Err(invalid(
            "output must not exist or must be an empty directory",
        ));
    }
    Ok(resolved)
}

#[derive(Default)]
struct CreatedPaths {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    keep: bool,
}

impl CreatedPaths {
    fn directory(&mut self, path: &Path) -> Result<(), Failure> {
        if path.is_dir() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            self.directory(parent)?;
        }
        fs::create_dir(path).map_err(io_failure)?;
        self.directories.push(path.to_path_buf());
        Ok(())
    }

    fn cleanup(&mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        for file in self.files.iter().rev() {
            if let Err(error) = fs::remove_file(file) {
                errors.push(format!("cannot remove '{}': {error}", file.display()));
            }
        }
        for directory in self.directories.iter().rev() {
            if let Err(error) = fs::remove_dir(directory) {
                errors.push(format!("cannot remove '{}': {error}", directory.display()));
            }
        }
        self.keep = true;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

impl Drop for CreatedPaths {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let _ = self.cleanup();
    }
}

fn io_failure(error: std::io::Error) -> Failure {
    Failure::io(error.to_string())
}
fn invalid(message: impl Into<String>) -> Failure {
    Failure::semantic(ExtractFailureKind::InvalidAttributeValue, message)
}
fn cell_failure(path: &Path, row: u32, column: usize, message: impl std::fmt::Display) -> Failure {
    invalid(format!(
        "{}: row {row}, column {column}: {message}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        game: PathBuf,
        csv: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = (0..1000)
                .find_map(|index| {
                    let root = std::env::temp_dir()
                        .join(format!("xivl-import-sheets-{}-{index}", std::process::id()));
                    match fs::create_dir(&root) {
                        Ok(()) => Some(root),
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                        Err(error) => panic!("cannot create test directory: {error}"),
                    }
                })
                .unwrap();
            let game = root.join("game");
            let csv = root.join("csv");
            let descriptor_path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/public/ssd-extract/complete.json");
            let descriptor: Value =
                serde_json::from_slice(&fs::read(&descriptor_path).unwrap()).unwrap();
            for resource in descriptor["resources"].as_array().unwrap() {
                let id =
                    xivl_formats::parse_resource_id(resource["id"].as_str().unwrap(), 0).unwrap();
                let path = extract::resource_path(&game, id);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(
                    path,
                    fs::read(
                        descriptor_path
                            .parent()
                            .unwrap()
                            .join(resource["file"].as_str().unwrap()),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            extract::extract_directory(&game, &csv).unwrap();
            Self { root, game, csv }
        }

        fn edit(&self, name: &str, from: &str, to: &str) {
            let path = self.csv.join(name);
            let text = fs::read_to_string(&path).unwrap();
            assert!(text.contains(from));
            fs::write(path, text.replace(from, to)).unwrap();
        }

        fn import(&self, name: &str) -> Result<Value, Failure> {
            import_directory(&self.game, &self.csv, &self.root.join(name))
        }

        fn overlay(&self, output: &str, report: &Value) {
            for resource in report["resources"].as_array().unwrap() {
                let relative = resource["path"].as_str().unwrap();
                fs::copy(
                    self.root.join(output).join(relative),
                    self.game.join(relative),
                )
                .unwrap();
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn unedited_extract_and_absent_csvs_write_zero_resources() {
        let fixture = Fixture::new();
        let report = fixture.import("no-op").unwrap();
        assert_eq!(
            report["counts"],
            json!({ "documentsRead": 2, "csvFilesRead": 2, "cellsChanged": 0, "rowsEnabled": 0, "rowsDisabled": 0, "resourcesWritten": 0 })
        );
        assert_eq!(fs::read_dir(fixture.root.join("no-op")).unwrap().count(), 0);
        fs::remove_file(fixture.csv.join("synthetic_main.csv")).unwrap();
        fs::remove_file(fixture.csv.join("independent.csv")).unwrap();
        let report = fixture.import("absent").unwrap();
        assert_eq!(report["counts"]["resourcesWritten"], 0);
        assert_eq!(report["counts"]["csvFilesRead"], 0);
    }

    #[test]
    fn common_edits_update_all_blocks_and_preserve_conflicts() {
        let fixture = Fixture::new();
        fixture.edit(
            "synthetic_main.csv",
            "100,alpha,7,true",
            "100,\"long,\"\"quoted\"\"\nvalue\",8,true",
        );
        let report = fixture.import("edit").unwrap();
        assert_eq!(report["counts"]["cellsChanged"], 2);
        assert_eq!(report["counts"]["resourcesWritten"], 4);
        assert!(report["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|resource| resource["role"] != "enable"));
        fixture.overlay("edit", &report);
        let destination = fixture.root.join("again");
        extract::extract_directory(&fixture.game, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("synthetic_main.csv")).unwrap(),
            fs::read(fixture.csv.join("synthetic_main.csv")).unwrap()
        );
        assert_eq!(
            fixture.import("repeat").unwrap()["counts"]["resourcesWritten"],
            0
        );
    }

    #[test]
    fn enabling_and_disabling_rows_rebuilds_maximal_ranges() {
        let fixture = Fixture::new();
        fixture.edit(
            "synthetic_main.csv",
            "100,alpha,7,true\n",
            "101,new,4,false\n",
        );
        let report = fixture.import("toggle").unwrap();
        assert_eq!(report["counts"]["rowsEnabled"], 1);
        assert_eq!(report["counts"]["rowsDisabled"], 1);
        assert_eq!(report["counts"]["resourcesWritten"], 6);
        for resource in report["resources"].as_array().unwrap() {
            if resource["role"] == "enable" {
                assert_eq!(
                    fs::read(
                        fixture
                            .root
                            .join("toggle")
                            .join(resource["path"].as_str().unwrap())
                    )
                    .unwrap(),
                    [101, 0, 0, 0, 2, 0, 0, 0]
                );
            }
        }
        fixture.overlay("toggle", &report);
        let destination = fixture.root.join("again");
        extract::extract_directory(&fixture.game, &destination).unwrap();
        assert_eq!(
            fs::read(destination.join("synthetic_main.csv")).unwrap(),
            fs::read(fixture.csv.join("synthetic_main.csv")).unwrap()
        );
    }

    #[test]
    fn editing_a_row_with_an_omitted_tail_preserves_the_omission() {
        let fixture = Fixture::new();
        fixture.edit(
            "independent.csv",
            "301,missing,6,[@missing]",
            "301,missing,7,[@missing]",
        );
        let report = fixture.import("missing").unwrap();
        assert_eq!(report["counts"]["resourcesWritten"], 1);
        let resource = &report["resources"][0];
        assert_eq!(resource["role"], "data");
        assert_eq!(resource["length"], 14);
        fixture.overlay("missing", &report);
        assert_eq!(
            fixture.import("repeat").unwrap()["counts"]["resourcesWritten"],
            0
        );
    }

    #[test]
    fn marker_edits_and_out_of_range_rows_fail_before_writing() {
        let fixture = Fixture::new();
        for (name, from, to, locator) in [
            (
                "independent.csv",
                "301,missing,6,[@missing]",
                "301,missing,6,filled",
                "row 301, column 2",
            ),
            (
                "synthetic_main.csv",
                "beta[@duplicate:42455441]",
                "changed",
                "row 102, column 0",
            ),
            (
                "synthetic_main.csv",
                "100,alpha,7,true",
                "100,[@missing],7,true",
                "row 100, column 0",
            ),
            (
                "synthetic_main.csv",
                "100,alpha,7,true",
                "100,alpha,07,true",
                "row 100, column 1",
            ),
        ] {
            fixture.edit(name, from, to);
            let error = fixture.import("bad").unwrap_err();
            assert!(
                error.message().contains(name) && error.message().contains(locator),
                "{}",
                error.message()
            );
            assert!(!fixture.root.join("bad").exists());
            fixture.edit(name, to, from);
        }
        fixture.edit("synthetic_main.csv", "200,solo,1,true", "999,solo,1,true");
        assert!(fixture
            .import("outside")
            .unwrap_err()
            .message()
            .contains("no present block covers this row"));
    }

    #[test]
    fn unknown_csvs_and_occupied_or_client_outputs_are_refused() {
        let fixture = Fixture::new();
        fs::write(fixture.csv.join("unknown.csv"), "").unwrap();
        assert!(fixture
            .import("unknown")
            .unwrap_err()
            .message()
            .contains("not produced by extract"));
        fs::remove_file(fixture.csv.join("unknown.csv")).unwrap();
        assert!(
            import_directory(&fixture.game, &fixture.csv, &fixture.game.join("new/deep"))
                .unwrap_err()
                .message()
                .contains("inside the game directory")
        );
        fs::create_dir(fixture.root.join("occupied")).unwrap();
        fs::write(fixture.root.join("occupied/keep"), "keep").unwrap();
        assert!(fixture
            .import("occupied")
            .unwrap_err()
            .message()
            .contains("empty directory"));
        assert_eq!(
            fs::read_to_string(fixture.root.join("occupied/keep")).unwrap(),
            "keep"
        );
        fs::create_dir(fixture.root.join("empty")).unwrap();
        assert_eq!(
            fixture.import("empty").unwrap()["counts"]["resourcesWritten"],
            0
        );
    }

    #[test]
    fn failed_materialization_removes_only_entries_created_by_this_run() {
        let fixture = Fixture::new();
        let output = fixture.root.join("empty");
        fs::create_dir(&output).unwrap();
        {
            let mut created = CreatedPaths::default();
            created.directory(&output).unwrap();
            created.directory(&output.join("data/a")).unwrap();
            fs::write(output.join("data/a/ours"), "partial").unwrap();
            created.files.push(output.join("data/a/ours"));
            assert!(created
                .directory(&output.join("data/a/ours/child"))
                .is_err());
            fs::write(output.join("concurrent"), "keep").unwrap();
        }
        assert!(output.exists());
        assert!(!output.join("data").exists());
        assert_eq!(
            fs::read_to_string(output.join("concurrent")).unwrap(),
            "keep"
        );
    }

    fn model(block: &LoadedBlock) -> CsvTable {
        let mut table = CsvTable::default();
        table
            .insert_columns(&block.indexes, &block.columns)
            .unwrap();
        for (&id, values) in &block.rows {
            table
                .insert_row(id, &block.indexes, values.clone())
                .unwrap();
        }
        table
    }

    fn first_block(fixture: &Fixture) -> LoadedBlock {
        let (source, document) = extract::discover_definitions(&fixture.game)
            .unwrap()
            .remove(0);
        extract::load_document(
            &fixture.game,
            &source,
            &document,
            &mut extract::ExtractSummary::default(),
        )
        .unwrap()
        .1
        .remove(0)
    }

    #[test]
    fn block_edits_preserve_existing_flags_and_inherit_new_string_flags() {
        let fixture = Fixture::new();
        let mut block = first_block(&fixture);
        for (&id, values) in &mut block.rows {
            let ColumnValue::Text(string) = values[0].as_mut().unwrap() else {
                panic!("expected string")
            };
            string.scrambled = id == 100;
        }
        let table = model(&block);
        let mut rows = read_table(&table.render(), &table.columns).unwrap();
        rows.get_mut(&100).unwrap()[0] = "changed".into();
        rows.get_mut(&102).unwrap()[0] = "other".into();
        rows.insert(101, vec!["new".into(), "8".into(), "false".into()]);
        validate_cells(
            Path::new("sheet.csv"),
            &table,
            std::slice::from_ref(&block),
            &rows,
            &mut Counts::default(),
        )
        .unwrap();
        let edited = edit_block(Path::new("sheet.csv"), &table, &rows, block)
            .unwrap()
            .unwrap();
        assert_eq!(edited.enable, [100, 0, 0, 0, 3, 0, 0, 0]);
        let values = parse_rows(&edited.data, &edited.block.columns).unwrap();
        for (row, flag) in values.iter().zip([true, true, false]) {
            let ColumnValue::Text(string) = &row.values[0] else {
                panic!("expected string")
            };
            assert_eq!(string.scrambled, flag);
        }
        assert_eq!(xivl_formats::value_text(&values[0].values[0]), "changed");
        assert_eq!(xivl_formats::value_text(&values[1].values[0]), "new");
        assert_eq!(xivl_formats::value_text(&values[2].values[0]), "other");
        let output = fixture.root.join("direct");
        let mut resources = BTreeMap::new();
        for (id, role, bytes) in [
            (edited.block.definition.data, "data", &edited.data),
            (edited.block.definition.enable, "enable", &edited.enable),
            (edited.block.definition.offsets, "offsets", &edited.offsets),
        ] {
            let path = extract::resource_path(&output, id);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            resources.insert(id.value(), (id, role, bytes.clone()));
        }
        verify_written(&fixture.game, &output, &resources, &edited).unwrap();
        let id = edited.block.definition.data;
        fs::write(extract::resource_path(&output, id), []).unwrap();
        assert!(verify_written(&fixture.game, &output, &resources, &edited).is_err());
    }

    #[test]
    fn empty_blocks_enable_plain_strings_and_can_disable_every_row() {
        let fixture = Fixture::new();
        let mut block = first_block(&fixture);
        block.rows.clear();
        block.data.clear();
        block.enable.clear();
        block.offsets = vec![0; 12];
        let table = model(&block);
        let mut rows = BTreeMap::from([(101, vec![String::new(), "0".into(), "true".into()])]);
        rows.get_mut(&101).unwrap()[0] = "[@missing]".into();
        assert!(validate_cells(
            Path::new("sheet.csv"),
            &table,
            std::slice::from_ref(&block),
            &rows,
            &mut Counts::default()
        )
        .unwrap_err()
        .message()
        .contains("row 101, column 0"));
        rows.get_mut(&101).unwrap()[0].clear();
        validate_cells(
            Path::new("sheet.csv"),
            &table,
            std::slice::from_ref(&block),
            &rows,
            &mut Counts::default(),
        )
        .unwrap();
        let edited = edit_block(Path::new("sheet.csv"), &table, &rows, block)
            .unwrap()
            .unwrap();
        assert_eq!(edited.data, [1, 0, 0, 0, 1]);
        assert_eq!(edited.offsets, [0, 0, 0, 0, 5, 0, 0, 0, 5, 0, 0, 0]);
        assert_eq!(edited.enable, [101, 0, 0, 0, 1, 0, 0, 0]);
        let mut block = edited.block;
        block.rows.insert(
            101,
            edited.rows[0].values.iter().cloned().map(Some).collect(),
        );
        block.data = edited.data;
        block.enable = edited.enable;
        block.offsets = edited.offsets;
        let table = model(&block);
        rows.clear();
        let edited = edit_block(Path::new("sheet.csv"), &table, &rows, block)
            .unwrap()
            .unwrap();
        assert!(edited.data.is_empty());
        assert!(edited.enable.is_empty());
        assert_eq!(edited.offsets, [0; 12]);
    }

    #[test]
    fn sparse_language_columns_and_original_enable_boundaries_are_preserved() {
        let fixture = Fixture::new();
        let mut block = first_block(&fixture);
        let last = block.rows.remove(&102).unwrap();
        block.rows.insert(101, last);
        block.enable = encode_enable_file(&[(100, 1), (101, 1)]);
        let mut table = model(&block);
        table
            .insert_columns(&[3], &[xivl_formats::ColumnType::Unsigned32])
            .unwrap();
        let rows = read_table(&table.render(), &table.columns).unwrap();
        validate_cells(
            Path::new("sheet.csv"),
            &table,
            std::slice::from_ref(&block),
            &rows,
            &mut Counts::default(),
        )
        .unwrap();
        let mut rows = rows;
        rows.get_mut(&100).unwrap()[1] = "8".into();
        let original = block.enable.clone();
        let edited = edit_block(Path::new("sheet.csv"), &table, &rows, block)
            .unwrap()
            .unwrap();
        assert_eq!(edited.enable, original);
    }

    #[test]
    fn generated_tree_import_pipeline_preserves_no_ops_and_replays_edits() {
        let fixture = Fixture::new();
        let import = |name: &str| {
            import_into_directory(&fixture.game, &fixture.csv, &fixture.root.join(name))
        };
        let report = import("no-op").unwrap();
        assert_eq!(
            report["counts"],
            json!({ "documentsRead": 2, "csvFilesRead": 2, "cellsChanged": 0, "rowsEnabled": 0, "rowsDisabled": 0, "resourcesWritten": 0 })
        );
        fixture.edit(
            "synthetic_main.csv",
            "100,alpha,7,true",
            "100,\"new,\"\"quoted\"\"\nvalue\",8,true",
        );
        let report = import("edit").unwrap();
        assert_eq!(report["counts"]["cellsChanged"], 2);
        assert_eq!(report["counts"]["resourcesWritten"], 4);
        fixture.overlay("edit", &report);
        assert_eq!(
            import("no-op-edit").unwrap()["counts"]["resourcesWritten"],
            0
        );
        fixture.edit(
            "synthetic_main.csv",
            "100,\"new,\"\"quoted\"\"\nvalue\",8,true\n",
            "101,new,4,false\n",
        );
        let report = import("toggle").unwrap();
        assert_eq!(report["counts"]["rowsEnabled"], 1);
        assert_eq!(report["counts"]["rowsDisabled"], 1);
        assert_eq!(report["counts"]["resourcesWritten"], 6);
        fixture.overlay("toggle", &report);
        let again = fixture.root.join("again");
        extract::extract_directory(&fixture.game, &again).unwrap();
        assert_eq!(
            fs::read(again.join("synthetic_main.csv")).unwrap(),
            fs::read(fixture.csv.join("synthetic_main.csv")).unwrap()
        );
        fixture.edit(
            "independent.csv",
            "301,missing,6,[@missing]",
            "301,missing,7,[@missing]",
        );
        let report = import("missing").unwrap();
        assert_eq!(report["counts"]["resourcesWritten"], 1);
        assert_eq!(report["resources"][0]["length"], 14);
        fixture.overlay("missing", &report);
        fixture.edit(
            "independent.csv",
            "301,missing,7,[@missing]",
            "301,missing,7,filled",
        );
        assert!(import("bad")
            .unwrap_err()
            .message()
            .contains("row 301, column 2"));
        assert!(!fixture.root.join("bad").exists());
        fixture.edit(
            "independent.csv",
            "301,missing,7,filled",
            "301,missing,7,[@missing]",
        );
        fixture.edit("synthetic_main.csv", "beta[@duplicate:42455441]", "changed");
        assert!(import("duplicate")
            .unwrap_err()
            .message()
            .contains("row 102, column 0"));
        fixture.edit("synthetic_main.csv", "changed", "beta[@duplicate:42455441]");
        fixture.edit("synthetic_main.csv", "200,solo,1,true", "999,solo,1,true");
        assert!(import("outside")
            .unwrap_err()
            .message()
            .contains("no present block covers this row"));
        fixture.edit("synthetic_main.csv", "999,solo,1,true", "200,solo,1,true");
        fs::write(fixture.csv.join("unknown.csv"), "").unwrap();
        assert!(import("unknown")
            .unwrap_err()
            .message()
            .contains("not produced by extract"));
        fs::remove_file(fixture.csv.join("unknown.csv")).unwrap();
        fs::remove_file(fixture.csv.join("synthetic_main.csv")).unwrap();
        fs::remove_file(fixture.csv.join("independent.csv")).unwrap();
        assert_eq!(import("absent").unwrap()["counts"]["resourcesWritten"], 0);
    }

    #[test]
    fn edits_cannot_change_shared_resources_of_an_absent_csv() {
        let fixture = Fixture::new();
        let original = xivl_formats::parse_resource_id("12000001", 0).unwrap();
        let alias = xivl_formats::parse_resource_id("15000001", 0).unwrap();
        let alias_path = extract::resource_path(&fixture.game, alias);
        fs::create_dir_all(alias_path.parent().unwrap()).unwrap();
        fs::copy(extract::resource_path(&fixture.game, original), alias_path).unwrap();
        let output = fixture.root.join("no-op");
        assert_eq!(
            import_into_directory(&fixture.game, &fixture.csv, &output).unwrap()["counts"]
                ["resourcesWritten"],
            0
        );
        fixture.edit(
            "synthetic_main.csv",
            "100,alpha,7,true",
            "100,changed,7,true",
        );
        let output = fixture.root.join("conflict");
        assert!(import_into_directory(&fixture.game, &fixture.csv, &output)
            .unwrap_err()
            .message()
            .contains("conflicting shared resource contents"));
        assert!(!output.exists());
    }
}
