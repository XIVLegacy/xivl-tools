//! Game-install discovery and SSD sheet CSV export.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use xivl_formats::sheet::{parse_enable_file, parse_row_offsets};
use xivl_formats::ssd::{self, SheetBody};
use xivl_formats::{parse_row_span, scrambled, CsvTable, ResourceId};

use crate::{ExtractFailure as Failure, ExtractFailureKind};

#[derive(Debug, Default)]
pub struct ExtractSummary {
    pub documents: usize,
    pub files: usize,
    pub rows: usize,
    pub missing_trailing_values: usize,
    pub absent_blocks: usize,
    pub conflicting_values: usize,
}

const MAX_EXPORT_COLUMNS: usize = 4096;

pub fn run(arguments: &[String]) -> Result<ExtractSummary, Failure> {
    let Some(game) = arguments.first() else {
        return Err(Failure::usage(
            "usage: xivl extract <game-directory> --output <directory>",
        ));
    };
    let mut output = None;
    let mut position = 1;
    while position < arguments.len() {
        match arguments[position].as_str() {
            "--output" if position + 1 < arguments.len() => {
                if output.replace(arguments[position + 1].clone()).is_some() {
                    return Err(Failure::usage("--output was supplied more than once"));
                }
                position += 2;
            }
            option => return Err(Failure::usage(format!("unknown extract option '{option}'"))),
        }
    }
    let output = output.ok_or_else(|| Failure::usage("extract requires --output <directory>"))?;
    extract(Path::new(game), Path::new(&output))
}

/// Run the directory exporter without parsing command-line arguments.
///
/// The conformance runner uses this narrow entry point so it exercises the
/// same exporter as the CLI without starting a second process.
pub fn extract_directory(game: &Path, output: &Path) -> Result<ExtractSummary, Failure> {
    extract(game, output)
}

fn extract(game: &Path, output: &Path) -> Result<ExtractSummary, Failure> {
    let data_root = game.join("data");
    if !data_root.is_dir() {
        return Err(Failure::io(format!(
            "'{}' has no data directory",
            game.display()
        )));
    }
    if output.is_dir()
        && fs::read_dir(output)
            .map_err(|error| Failure::io(error.to_string()))?
            .next()
            .is_some()
    {
        return Err(Failure::io(format!(
            "output directory '{}' is not empty",
            output.display()
        )));
    }
    fs::create_dir_all(output).map_err(|error| {
        Failure::io(format!(
            "cannot create output directory '{}': {error}",
            output.display()
        ))
    })?;

    let mut candidates = Vec::new();
    collect_dat_paths(&data_root, &mut candidates)?;
    candidates.sort();

    let mut definitions = Vec::new();
    for path in candidates {
        let Some(document) = read_ssd_candidate(&path)? else {
            continue;
        };
        match document.format_id() {
            "ssd-sheet" => definitions.push((path, document)),
            "ssd-mixed" => {
                return Err(Failure::semantic(
                    ExtractFailureKind::InvalidAttributeValue,
                    format!(
                        "{}: mixed SSD master and sheet definitions are unsupported",
                        path.display()
                    ),
                ));
            }
            _ => {}
        }
    }

    let mut used_names = BTreeSet::new();
    let mut summary = ExtractSummary::default();
    for (source, document) in definitions {
        let Some(first) = document.sheets.first() else {
            continue;
        };
        let base = safe_name(&first.name);
        let file_name = unique_name(&base, &mut used_names);
        let mut table = CsvTable::default();
        let mut document_rows = BTreeSet::new();

        for sheet in &document.sheets {
            let SheetBody::Definition {
                columns,
                index,
                blocks,
            } = &sheet.body
            else {
                continue;
            };
            let declared_width = attribute_u32(sheet, "column_max", &source)? as usize;
            if declared_width > MAX_EXPORT_COLUMNS {
                return Err(Failure::semantic(
                    ExtractFailureKind::InvalidAttributeValue,
                    format!("{}: declared column width {declared_width} exceeds export limit {MAX_EXPORT_COLUMNS}", source.display()),
                ));
            }
            let declared_count = attribute_u32(sheet, "column_count", &source)? as usize;
            if declared_count != columns.len() {
                return Err(Failure::semantic(
                    ExtractFailureKind::InvalidAttributeValue,
                    format!(
                        "{}: sheet declares {declared_count} columns but lists {} types",
                        source.display(),
                        columns.len()
                    ),
                ));
            }
            let mut seen_indexes = BTreeSet::new();
            for &column_index in index {
                if column_index as usize >= declared_width {
                    return Err(Failure::semantic(ExtractFailureKind::InvalidAttributeValue, format!(
                        "{}: column index {column_index} is outside declared width {declared_width}",
                        source.display()
                    )));
                }
                if !seen_indexes.insert(column_index) {
                    return Err(Failure::semantic(
                        ExtractFailureKind::InvalidAttributeValue,
                        format!(
                            "{}: column index {column_index} is repeated",
                            source.display()
                        ),
                    ));
                }
            }
            table.ensure_width(declared_width);
            table
                .insert_columns(index, columns)
                .map_err(|error| parse_failure(&source, error))?;

            for block in blocks {
                let present = [block.data, block.enable, block.offsets]
                    .map(|id| resource_path(game, id).is_file());
                if present.iter().all(|value| !value) {
                    summary.absent_blocks += 1;
                    continue;
                }
                if present.iter().any(|value| !value) {
                    return Err(Failure::semantic(
                        ExtractFailureKind::IncompleteResourceTriple,
                        format!(
                            "{}: block has only some of its data, enable, and offset resources",
                            source.display()
                        ),
                    ));
                }
                let data = read_resource(game, block.data)?;
                let enable = read_resource(game, block.enable)?;
                let offsets_data = read_resource(game, block.offsets)?;
                let enable = parse_enable_file(&enable)
                    .map_err(|error| resource_failure(block.enable, error))?;
                let offsets = parse_row_offsets(&offsets_data)
                    .map_err(|error| resource_failure(block.offsets, error))?;
                if !enable.anomalies.is_empty() {
                    return Err(Failure::semantic(
                        ExtractFailureKind::InvalidResourceStructure,
                        format!(
                            "{}: enable file contains structural anomalies",
                            source.display()
                        ),
                    ));
                }
                if !offsets.anomalies.is_empty() {
                    return Err(Failure::semantic(
                        ExtractFailureKind::InvalidResourceStructure,
                        format!(
                            "{}: row-offset file contains structural anomalies",
                            source.display()
                        ),
                    ));
                }
                if offsets.slot_count() != block.count as usize {
                    return Err(Failure::semantic(
                        ExtractFailureKind::OffsetCountMismatch,
                        format!(
                            "{}: block declares {} slots but its offset file has {} entries",
                            source.display(),
                            block.count,
                            offsets.slot_count()
                        ),
                    ));
                }
                if offsets.data_length() != data.len() as u64 {
                    return Err(Failure::semantic(
                        ExtractFailureKind::OffsetDataLengthMismatch,
                        format!(
                            "{}: data length {} disagrees with final row offset {}",
                            source.display(),
                            data.len(),
                            offsets.data_length()
                        ),
                    ));
                }
                for range in &enable.ranges {
                    let end = u64::from(range.first_row)
                        .checked_add(u64::from(range.count))
                        .ok_or_else(|| {
                            Failure::semantic(
                                ExtractFailureKind::InvalidResourceStructure,
                                format!(
                                    "{}: enable range overflows row-id space",
                                    block.enable.dat_path()
                                ),
                            )
                        })?;
                    let block_end = u64::from(block.begin)
                        .checked_add(u64::from(block.count))
                        .ok_or_else(|| {
                            Failure::semantic(
                                ExtractFailureKind::InvalidResourceStructure,
                                format!("{}: block range overflows row-id space", source.display()),
                            )
                        })?;
                    if u64::from(range.first_row) < u64::from(block.begin) || end > block_end {
                        return Err(Failure::semantic(
                            ExtractFailureKind::InvalidResourceStructure,
                            format!(
                                "{}: enable range lies outside block row range",
                                source.display()
                            ),
                        ));
                    }
                }
                if enable
                    .ranges
                    .iter()
                    .map(|range| u64::from(range.count))
                    .sum::<u64>()
                    > u64::from(block.count)
                {
                    return Err(Failure::semantic(
                        ExtractFailureKind::InvalidResourceStructure,
                        format!("{}: enable ranges contain too many rows", source.display()),
                    ));
                }
                let mut enabled = BTreeSet::new();
                for range in &enable.ranges {
                    for offset in 0..range.count {
                        let row_id = range.first_row.checked_add(offset).ok_or_else(|| {
                            Failure::semantic(
                                ExtractFailureKind::InvalidResourceStructure,
                                format!(
                                    "{}: enable range exceeds the u32 row-id space",
                                    block.enable.dat_path()
                                ),
                            )
                        })?;
                        enabled.insert(row_id);
                    }
                }
                let mut stored = BTreeSet::new();
                for row in &offsets.rows {
                    stored.insert(row_id(block.begin, row.index, &source)?);
                }
                if enabled != stored {
                    return Err(Failure::semantic(
                        ExtractFailureKind::EnableRowMismatch,
                        format!(
                            "{}: enable and row-offset resources name different rows",
                            source.display()
                        ),
                    ));
                }

                for row in &offsets.rows {
                    let start = row.span.offset as usize;
                    let end = start + row.span.length as usize;
                    let values = parse_row_span(&data[start..end], columns)
                        .map_err(|error| resource_failure(block.data, error))?;
                    summary.missing_trailing_values +=
                        values.iter().filter(|value| value.is_none()).count();
                    let row_id = row_id(block.begin, row.index, &source)?;
                    summary.conflicting_values += table
                        .insert_row(row_id, index, values)
                        .map_err(|error| parse_failure(&source, error))?;
                    document_rows.insert(row_id);
                }
            }
        }

        let destination = output.join(file_name);
        fs::write(&destination, table.render()).map_err(|error| {
            Failure::io(format!("cannot write '{}': {error}", destination.display()))
        })?;
        summary.documents += 1;
        summary.files += 1;
        summary.rows += document_rows.len();
    }
    Ok(summary)
}

fn collect_dat_paths(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), Failure> {
    let entries = fs::read_dir(directory).map_err(|error| {
        Failure::io(format!(
            "cannot read directory '{}': {error}",
            directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| Failure::io(error.to_string()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|error| Failure::io(error.to_string()))?;
        if kind.is_dir() {
            collect_dat_paths(&path, output)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dat"))
        {
            output.push(path);
        }
    }
    Ok(())
}

fn read_ssd_candidate(path: &Path) -> Result<Option<ssd::SsdDocument>, Failure> {
    let mut file = File::open(path).map_err(|error| read_failure(path, &error))?;
    let length = file
        .metadata()
        .map_err(|error| read_failure(path, &error))?
        .len();
    if length == 0 {
        return Ok(None);
    }
    let mut prefix = [0u8; 8];
    let prefix_length = file
        .read(&mut prefix)
        .map_err(|error| read_failure(path, &error))?;
    file.seek(SeekFrom::End(-1))
        .map_err(|error| read_failure(path, &error))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)
        .map_err(|error| read_failure(path, &error))?;
    let plain = prefix[..prefix_length].starts_with(b"<")
        || prefix[..prefix_length].starts_with(b"\xEF\xBB\xBF<");
    if !plain && last[0] != scrambled::TRAILER {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| read_failure(path, &error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| read_failure(path, &error))?;
    let decoded;
    let document_bytes = if plain {
        bytes.as_slice()
    } else {
        let Ok(value) = scrambled::decode(&bytes) else {
            return Ok(None);
        };
        decoded = value.document;
        decoded.as_slice()
    };
    let looks_like_ssd = has_ssd_root(document_bytes);
    match ssd::parse_document(document_bytes) {
        Ok(document) => Ok(Some(document)),
        Err(error) if !plain || looks_like_ssd => Err(parse_failure(path, error)),
        Err(_) => Ok(None),
    }
}

fn read_resource(game: &Path, id: ResourceId) -> Result<Vec<u8>, Failure> {
    let path = resource_path(game, id);
    fs::read(&path).map_err(|error| read_failure(&path, &error))
}

fn resource_path(game: &Path, id: ResourceId) -> PathBuf {
    let mut path = game.to_path_buf();
    for component in id.dat_path().split('/') {
        path.push(component);
    }
    path
}

fn attribute_u32(sheet: &ssd::Sheet, name: &str, source: &Path) -> Result<u32, Failure> {
    let attribute = sheet
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .ok_or_else(|| {
            Failure::semantic(
                ExtractFailureKind::InvalidAttributeValue,
                format!("{}: sheet has no {name} attribute", source.display()),
            )
        })?;
    attribute.value.parse().map_err(|_| {
        Failure::semantic(
            ExtractFailureKind::InvalidAttributeValue,
            format!(
                "{}: sheet {name} attribute is not an unsigned integer",
                source.display()
            ),
        )
    })
}

fn row_id(begin: u32, index: u64, source: &Path) -> Result<u32, Failure> {
    u64::from(begin)
        .checked_add(index)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            Failure::semantic(
                ExtractFailureKind::RowIdOverflow,
                format!("{}: block row id exceeds the u32 range", source.display()),
            )
        })
}

fn safe_name(name: &str) -> String {
    let mut value: String = name
        .chars()
        .map(|character| match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '_',
        })
        .collect();
    if value.is_empty() || value == "." || value == ".." {
        value = "sheet".into();
    }
    value
}

fn unique_name(base: &str, used: &mut BTreeSet<String>) -> String {
    let first = format!("{base}.csv");
    if used.insert(first.to_ascii_lowercase()) {
        return first;
    }
    for number in 2usize.. {
        let candidate = format!("{base}({number}).csv");
        if used.insert(candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    unreachable!()
}

fn read_failure(path: &Path, error: &std::io::Error) -> Failure {
    Failure::io(format!("cannot read '{}': {error}", path.display()))
}

fn parse_failure(path: &Path, error: xivl_formats::FormatError) -> Failure {
    let message = format!("{}: {error}", path.display());
    Failure::format(error, message)
}

fn resource_failure(id: ResourceId, error: xivl_formats::FormatError) -> Failure {
    let message = format!("{}: {error}", id.dat_path());
    Failure::format(error, message)
}

fn has_ssd_root(bytes: &[u8]) -> bool {
    let mut body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if body.starts_with(b"<?xml") {
        let Some(end) = body.windows(2).position(|window| window == b"?>") else {
            return false;
        };
        body = &body[end + 2..];
    }
    while body.first().is_some_and(|byte| byte.is_ascii_whitespace()) {
        body = &body[1..];
    }
    let Some(rest) = body.strip_prefix(b"<ssd") else {
        return false;
    };
    rest.first()
        .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_and_case_insensitively_unique() {
        let mut used = BTreeSet::new();
        assert_eq!(safe_name("xtx/quest"), "xtx_quest");
        assert_eq!(unique_name("Sheet", &mut used), "Sheet.csv");
        assert_eq!(unique_name("sheet", &mut used), "sheet(2).csv");
    }

    #[test]
    fn directory_extract_is_lossless_and_deterministic() {
        let root = unique_test_directory();
        let game = root.join("game");
        let first = root.join("first");
        let second = root.join("second");
        let descriptor_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/public/ssd-extract/complete.json");
        let descriptor: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&descriptor_path).unwrap()).unwrap();
        for resource in descriptor["resources"].as_array().unwrap() {
            let id = xivl_formats::parse_resource_id(resource["id"].as_str().unwrap(), 0).unwrap();
            let bytes = std::fs::read(
                descriptor_path
                    .parent()
                    .unwrap()
                    .join(resource["file"].as_str().unwrap()),
            )
            .unwrap();
            let path = game.join(id.dat_path());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }

        let summary = extract(&game, &first).unwrap();
        assert_eq!(summary.documents, 2);
        assert_eq!(summary.files, 2);
        assert_eq!(summary.rows, 5);
        assert_eq!(summary.missing_trailing_values, 1);
        assert_eq!(summary.absent_blocks, 1);
        assert_eq!(summary.conflicting_values, 2);
        let expected = " ,0,1,2\n ,str,u8,bool\n100,alpha,7,true\n102,beta[@duplicate:42455441],9[@duplicate:3130],false\n200,solo,1,true\n";
        assert_eq!(
            std::fs::read_to_string(first.join("synthetic_main.csv")).unwrap(),
            expected
        );
        assert_eq!(
            std::fs::read_to_string(first.join("independent.csv")).unwrap(),
            " ,0,1,2\n ,str,s32,str\n300,independent,5,final\n301,missing,6,[@missing]\n"
        );

        let repeat = extract(&game, &second).unwrap();
        assert_eq!(summary.files, repeat.files);
        assert_eq!(summary.documents, repeat.documents);
        assert_eq!(
            std::fs::read(first.join("synthetic_main.csv")).unwrap(),
            std::fs::read(second.join("synthetic_main.csv")).unwrap()
        );
        assert_eq!(
            std::fs::read(first.join("independent.csv")).unwrap(),
            std::fs::read(second.join("independent.csv")).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn unique_test_directory() -> PathBuf {
        for index in 0..1000 {
            let candidate = std::env::temp_dir().join(format!(
                "xivl-cli-ssd-extract-test-{}-{index}",
                std::process::id()
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => return candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create test directory: {error}"),
            }
        }
        panic!("cannot allocate test directory")
    }
}
