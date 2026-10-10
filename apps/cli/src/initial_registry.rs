//! Conditional reconstruction from authenticated, locale-specific backing rows.

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::error::{ErrorKind, FormatError, Result};
use xivl_formats::resource::ResourceId;
use xivl_formats::sheet::{self, ColumnType, ColumnValue, Row};

use crate::fixed_phrase::Locale;
use crate::initial_registry_inputs::{
    COMMAND_CONFIGURATION, FIXED, ITEM_DEFAULT_FIELDS, PLACE, PLACE_DEFAULT_FIELDS,
};
use crate::sheet_reference::{self, Cell, QueryResult};
use crate::{read_capped, Failure};

#[derive(Deserialize)]
pub struct Pin<'a> {
    pub id: u32,
    pub size: usize,
    pub sha256: &'a str,
}

#[derive(Deserialize)]
struct ItemBlock<'a> {
    begin: u32,
    count: usize,
    #[serde(borrow)]
    pins: [Pin<'a>; 3],
}

#[derive(Deserialize)]
struct ItemLocale<'a> {
    locale: &'a str,
    columns: Vec<usize>,
    types: Vec<&'a str>,
    #[serde(borrow)]
    blocks: Vec<ItemBlock<'a>>,
}

impl ItemBlock<'_> {
    fn contains(&self, key: u32) -> bool {
        u64::from(key) >= u64::from(self.begin)
            && u64::from(key) < u64::from(self.begin) + self.count as u64
    }
}

fn item_inputs() -> Vec<ItemLocale<'static>> {
    // Identities and layouts only; provenance is owned by Initial registry inputs.
    serde_json::from_str(include_str!("item_name_inputs.json"))
        .expect("validated embedded itemName metadata")
}

#[derive(Deserialize)]
struct CommandLocale<'a> {
    locale: &'a str,
    columns: Vec<usize>,
    types: Vec<&'a str>,
    default: usize,
    #[serde(borrow)]
    blocks: Vec<ItemBlock<'a>>,
}

fn command_inputs() -> Vec<CommandLocale<'static>> {
    serde_json::from_str(include_str!("command_inputs.json"))
        .expect("validated embedded command metadata")
}

fn load_command(
    root: &Path,
    input: &CommandLocale<'_>,
    block: &ItemBlock<'_>,
) -> std::result::Result<Loaded, Failure> {
    let columns: Vec<_> = input
        .types
        .iter()
        .map(|kind| ColumnType::parse(kind, 0).expect("validated embedded command type"))
        .collect();
    load(root, &block.pins, &columns, block.begin, block.count)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NounInputs<'a> {
    #[serde(borrow)]
    target: ItemBlock<'a>,
    #[serde(borrow)]
    attributive: ItemBlock<'a>,
    #[serde(borrow)]
    configuration_pins: [Pin<'a>; 3],
}

fn noun_inputs() -> NounInputs<'static> {
    serde_json::from_str(include_str!("english_noun_inputs.json"))
        .expect("validated embedded English noun metadata")
}

fn english_noun_text(row: &Row, attributive: Option<&Row>, inputs: [u32; 4]) -> Value {
    let deferred = |reason| json!({"status": "requires-evaluation", "reason": reason});
    if inputs != [0, 1, 5, 0] {
        return deferred("unsupported-noun-inputs");
    }
    if attributive.is_none() {
        return deferred("attributive-row-not-found");
    }
    // Conditional English noun identity owns the pinned configuration and branch.
    let [ColumnValue::Text(singular), ColumnValue::Text(_), ColumnValue::Signed8 { value: 0, .. }, ColumnValue::Signed8 { value: 0, .. }, ColumnValue::Signed8 { value: 1, .. }] =
        row.values.as_slice()
    else {
        return deferred("unsupported-noun-row");
    };
    if singular.rich.tokens().next().is_some() {
        return deferred("embedded-controls");
    }
    let value = singular.rich.text_only();
    if value.contains("[n]") {
        return deferred("noun-count-replacement");
    }
    json!({"status": "resolved", "association": "conditional-explicit-NounEn",
        "column": 1, "text": value, "textEncoding": "utf-8"})
}

fn command_query(
    root: &Path,
    input: &CommandLocale<'_>,
    cache: &mut Vec<(usize, Loaded)>,
    name: &str,
    key: u32,
    column: usize,
) -> std::result::Result<QueryResult, Failure> {
    if name != "xtx/command" {
        return Ok(Err("unsupported-sheet-target"));
    }
    let selected = if column == 255 { input.default } else { column };
    let Some(position) = input.columns.iter().position(|&field| field == selected) else {
        return Ok(Err("unsupported-sheet-column"));
    };
    if input.types[position] != "str" {
        return Ok(Err("unsupported-sheet-column"));
    }
    let Some(block_index) = input.blocks.iter().position(|block| block.contains(key)) else {
        return Ok(Err("sheet-row-not-found"));
    };
    let block = &input.blocks[block_index];
    if !cache.iter().any(|(index, _)| *index == block_index) {
        cache.push((block_index, load_command(root, input, block)?));
    }
    let loaded = &cache
        .iter()
        .find(|(index, _)| *index == block_index)
        .unwrap()
        .1;
    let Some((_, row)) = loaded.rows.iter().find(|(row_key, _)| *row_key == key) else {
        return Ok(Err("sheet-row-not-found"));
    };
    let ColumnValue::Text(value) = &row.values[position] else {
        unreachable!("validated command string column")
    };
    Ok(Ok(Cell {
        rich: value.rich.clone(),
        evidence: json!({"sheet": name, "rowKey": key, "requestedColumn": column,
            "column": selected, "sources": loaded.sources,
            "block": {"begin": block.begin, "count": block.count},
            "sourceValue": value.rich.to_lossless_text(), "sourceEncoding": "ssd-lossless-csv"}),
    }))
}

struct Loaded {
    rows: Vec<(u32, Row)>,
    sources: Value,
}

#[derive(Debug)]
struct Category {
    source_key: u32,
    descriptor: Option<usize>,
}

#[derive(Debug)]
struct Descriptor {
    sheet: String,
    source: String,
    noun: bool,
}

struct Registry {
    map: Vec<Option<usize>>,
    categories: Vec<Category>,
    descriptors: Vec<Descriptor>,
}

fn invalid(offset: u64, detail: impl Into<String>) -> FormatError {
    FormatError::new(ErrorKind::InvalidAttributeValue, offset, detail)
}

fn locale_index(locale: Locale) -> usize {
    match locale {
        Locale::Ja => 0,
        Locale::En => 1,
        Locale::De => 2,
        Locale::Fr => 3,
    }
}

fn dispatch(noun: bool, locale: Locale) -> &'static str {
    // The English initial context declares type 1; C9C0A0 needs type 2 for noun mode.
    if !noun || matches!(locale, Locale::En) {
        "query"
    } else {
        "provider-dependent-noun"
    }
}

fn text(value: &ColumnValue) -> String {
    match value {
        ColumnValue::Text(value) => value.rich.to_lossless_text(),
        _ => unreachable!("validated string column"),
    }
}

fn source_cells(values: &[ColumnValue], fields: &[usize]) -> Vec<Value> {
    values
        .iter()
        .zip(fields)
        .map(|(value, column)| {
            let value = match value {
                ColumnValue::Text(_) => json!(text(value)),
                ColumnValue::Unsigned8 { value, .. } => json!(value),
                ColumnValue::Signed8 { value, .. } => json!(value),
                ColumnValue::Signed32 { value, .. } => json!(value),
                _ => unreachable!("supported source schema"),
            };
            json!({"column": column, "value": value})
        })
        .collect()
}

fn plain_text(row: &Row, fields: &[usize], column: usize, noun: bool) -> Value {
    if noun {
        return json!({"status": "requires-evaluation", "reason": "provider-dependent-noun"});
    }
    let selected = fields
        .iter()
        .position(|&field| field == column)
        .and_then(|index| row.values.get(index));
    let Some(ColumnValue::Text(value)) = selected else {
        return json!({"status": "unsupported-default-field", "column": column});
    };
    if value.rich.tokens().next().is_some() {
        return json!({"status": "requires-evaluation", "reason": "embedded-controls", "column": column});
    }
    // text_only is lossless here because the selected field has no tokens.
    json!({"status": "resolved", "association": "conditional-default-field", "column": column,
        "text": value.rich.text_only(), "textEncoding": "utf-8"})
}

fn metadata(row: &Row) -> Result<(u32, String)> {
    let ColumnValue::Unsigned32 { value, .. } = row.values[0] else {
        unreachable!("validated u32 column")
    };
    let ColumnValue::Text(ref descriptor) = row.values[1] else {
        unreachable!("validated string column")
    };
    if descriptor.rich.tokens().next().is_some() {
        return Err(invalid(
            descriptor.span.offset,
            "registry descriptor contains a control",
        ));
    }
    Ok((value, descriptor.rich.text_only()))
}

fn reconstruct(rows: &[(u32, Row)]) -> Result<Registry> {
    let mut registry = Registry {
        map: Vec::new(),
        categories: Vec::new(),
        descriptors: vec![Descriptor {
            sheet: "xtx/_fixedPhrase".into(),
            source: String::new(),
            noun: false,
        }],
    };
    for (key, row) in rows {
        let (field0, source) = metadata(row)?;
        if source.is_empty() {
            continue;
        }
        let category = field0.checked_sub(1).filter(|&n| n < 128).ok_or_else(|| {
            invalid(
                row.span.offset,
                "registry category is outside its nonnegative signed-byte range",
            )
        })? as usize;
        let descriptor = if source.starts_with('@') {
            Some(0)
        } else if source.starts_with('#') {
            // This category is constructed with descriptor index -1. Later
            // dynamic population is outside the declared initial context.
            None
        } else {
            let sheet = source
                .split([':', '[', ',', ']', '#'])
                .next()
                .unwrap_or_default();
            if !sheet.starts_with("xtx/") || sheet.len() == 4 {
                return Err(invalid(
                    row.span.offset,
                    "unsupported initial registry descriptor",
                ));
            }
            let index = registry.descriptors.len();
            registry.descriptors.push(Descriptor {
                sheet: sheet.into(),
                noun: source
                    .split([':', '[', ',', ']', '#'])
                    .any(|part| part == "noun"),
                source,
            });
            Some(index)
        };
        let ordinal = registry.categories.len();
        if ordinal >= 128 || descriptor.is_some_and(|index| index >= 128) {
            return Err(invalid(
                row.span.offset,
                "registry exceeds signed-byte index capacity",
            ));
        }
        // C9D0D0 assigns the map entry only when the vector grows. A hole or
        // duplicate below its existing length is not repaired by this call.
        if category >= registry.map.len() {
            registry.map.resize(category + 1, None);
            registry.map[category] = Some(ordinal);
        }
        registry.categories.push(Category {
            source_key: *key,
            descriptor,
        });
    }
    Ok(registry)
}

fn parse_backing(
    data: &[u8],
    enable: &[u8],
    offsets: &[u8],
    columns: &[ColumnType],
    begin: u32,
    count: usize,
) -> Result<Vec<(u32, Row)>> {
    let offsets = sheet::parse_row_offsets(offsets)?;
    let enable = sheet::parse_enable_file(enable)?;
    if !offsets.anomalies.is_empty()
        || !enable.anomalies.is_empty()
        || offsets.slot_count() != count
        || offsets.data_length() != data.len() as u64
        || enable.ranges.iter().any(|range| {
            u64::from(range.first_row) < u64::from(begin)
                || range.end() > u64::from(begin) + count as u64
        })
    {
        return Err(invalid(0, "backing offsets, ranges or data length differ"));
    }
    let enabled: Vec<u64> = enable
        .ranges
        .iter()
        .flat_map(|range| u64::from(range.first_row)..range.end())
        .collect();
    if enabled.iter().copied().ne(offsets
        .rows
        .iter()
        .map(|slot| u64::from(begin) + slot.index))
    {
        return Err(invalid(0, "enabled keys differ from nonempty row slots"));
    }
    let rows = sheet::parse_rows(data, columns)?;
    if rows.len() != offsets.rows.len()
        || rows
            .iter()
            .zip(&offsets.rows)
            .any(|(row, slot)| row.span != slot.span)
    {
        return Err(invalid(0, "decoded rows differ from backing row spans"));
    }
    rows.into_iter()
        .zip(enabled)
        .map(|(row, key)| {
            u32::try_from(key)
                .map(|key| (key, row))
                .map_err(|_| invalid(0, "backing key exceeds u32"))
        })
        .collect()
}

fn load(
    root: &Path,
    pins: &[Pin<'_>; 3],
    columns: &[ColumnType],
    begin: u32,
    count: usize,
) -> std::result::Result<Loaded, Failure> {
    let mut inputs = Vec::new();
    let mut sources = Vec::new();
    for pin in pins {
        let data = read_pin(root, pin)?;
        sources.push(pin_source(pin));
        inputs.push(data);
    }
    let rows = parse_backing(&inputs[0], &inputs[1], &inputs[2], columns, begin, count).map_err(
        |error| {
            Failure::parse(format!(
                "{} at byte {}: {}",
                error.kind().as_str(),
                error.offset(),
                error.detail()
            ))
        },
    )?;
    Ok(Loaded {
        rows,
        sources: json!(sources),
    })
}

fn pin_source(pin: &Pin<'_>) -> Value {
    json!({"resourceId": ResourceId::new(pin.id).to_hex(), "bytes": pin.size, "sha256": pin.sha256})
}

fn read_pin(root: &Path, pin: &Pin<'_>) -> std::result::Result<Vec<u8>, Failure> {
    let id = ResourceId::new(pin.id);
    let path = root.join(id.dat_path());
    let data = read_capped(&path.to_string_lossy())?;
    if data.len() != pin.size || sha256_hex(&data) != pin.sha256 {
        return Err(Failure::parse(format!(
            "{}: size or SHA-256 differs from authenticated 1.23b input",
            id.to_hex()
        )));
    }
    Ok(data)
}

pub fn report(
    root: &str,
    numbers: [u32; 2],
    locale: Locale,
    noun_source: Option<[u32; 4]>,
) -> std::result::Result<(Value, bool), Failure> {
    if noun_source.is_some() && !matches!(locale, Locale::En) {
        return Err(Failure::usage(
            "explicit NounEn context requires English source rows",
        ));
    }
    let (locale_name, fixed_columns) = locale.columns();
    let index = locale_index(locale);
    let fixed = load(
        Path::new(root),
        &FIXED[index],
        &[
            ColumnType::Unsigned32,
            ColumnType::Text,
            ColumnType::Text,
            ColumnType::Text,
        ],
        100,
        2926,
    )?;
    let registry = reconstruct(&fixed.rows).map_err(|error| {
        Failure::parse(format!(
            "{} at byte {}: {}",
            error.kind().as_str(),
            error.offset(),
            error.detail()
        ))
    })?;
    let [category, key] = numbers;
    let mut document = json!({
        "context": "conditional-initial-1.23b-full-load",
        "locale": locale_name,
        "category": category,
        "nativeKey": key,
        "registrySources": fixed.sources,
        "categoryCount": registry.categories.len(),
        "descriptorCount": registry.descriptors.len(),
    });
    let selected = registry
        .map
        .get(category as usize)
        .copied()
        .flatten()
        .and_then(|ordinal| {
            registry
                .categories
                .get(ordinal)
                .map(|entry| (ordinal, entry))
        });
    let Some((ordinal, entry)) = selected else {
        document["status"] = json!("unmapped-category");
        return Ok((document, true));
    };
    document["categoryOrdinal"] = json!(ordinal);
    document["categorySourceKey"] = json!(entry.source_key);
    let Some(descriptor_index) = entry.descriptor else {
        document["status"] = json!("dynamic-category");
        return Ok((document, true));
    };
    let descriptor = &registry.descriptors[descriptor_index];
    document["descriptorIndex"] = json!(descriptor_index);
    document["sheet"] = json!(descriptor.sheet);
    document["descriptor"] = json!(descriptor.source);
    document["descriptorNounFlag"] = json!(descriptor.noun);
    document["dispatch"] = json!(dispatch(descriptor.noun, locale));
    if descriptor.noun && matches!(locale, Locale::En) {
        document["providerContext"] =
            json!({"type": 1, "association": "conditional-stable-English-provider"});
    }
    document["status"] = json!("selected");
    if let Some([configuration, count, attributive, retained]) = noun_source {
        document["nounContext"] = json!({"context": "explicit-fresh-NounEn", "providerType": 1,
            "sourceWords": [configuration, key, count, attributive, retained],
            "rowAssociation": "assumed-ready-named-source-keys"});
        if !descriptor.noun || descriptor.sheet != "xtx/displayName" {
            document["textResolution"] =
                json!({"status": "requires-evaluation", "reason": "noun-context-not-applicable"});
            return Ok((document, true));
        }
    }
    let (rows, fields) = match descriptor.sheet.as_str() {
        "xtx/_fixedPhrase" => (fixed.rows, fixed_columns.to_vec()),
        "xtx/placeName" => {
            let (columns, fields): (&[_], Vec<usize>) = match locale {
                Locale::Ja => (&[ColumnType::Text], vec![0]),
                Locale::En => (
                    &[ColumnType::Text, ColumnType::Text, ColumnType::Unsigned8],
                    vec![1, 2, 3],
                ),
                Locale::De => (&[ColumnType::Text, ColumnType::Text], vec![4, 5]),
                Locale::Fr => (&[ColumnType::Text, ColumnType::Text], vec![6, 7]),
            };
            let place = load(Path::new(root), &PLACE[index], columns, 0, 8002)?;
            document["targetSources"] = place.sources;
            (place.rows, fields)
        }
        "xtx/itemName" => {
            let inputs = item_inputs();
            let input = &inputs[index];
            debug_assert_eq!(input.locale, locale_name);
            let Some(block) = input.blocks.iter().find(|block| block.contains(key)) else {
                document["sourceLookup"] = json!({"status": "not-found"});
                return Ok((document, true));
            };
            let columns: Vec<_> = input
                .types
                .iter()
                .map(|kind| ColumnType::parse(kind, 0).expect("validated embedded itemName type"))
                .collect();
            let item = load(
                Path::new(root),
                &block.pins,
                &columns,
                block.begin,
                block.count,
            )?;
            document["targetSources"] = item.sources;
            document["targetBlock"] = json!({"begin": block.begin, "count": block.count});
            (item.rows, input.columns.clone())
        }
        "xtx/command" => {
            let inputs = command_inputs();
            let input = &inputs[index];
            debug_assert_eq!(input.locale, locale_name);
            let Some(block) = input.blocks.iter().find(|block| block.contains(key)) else {
                document["sourceLookup"] = json!({"status": "not-found"});
                return Ok((document, true));
            };
            for pin in &COMMAND_CONFIGURATION {
                read_pin(Path::new(root), pin)?;
            }
            document["configurationSources"] = json!(COMMAND_CONFIGURATION
                .iter()
                .map(pin_source)
                .collect::<Vec<_>>());
            let command = load_command(Path::new(root), input, block)?;
            document["targetSources"] = command.sources;
            document["targetBlock"] = json!({"begin": block.begin, "count": block.count});
            (command.rows, input.columns.clone())
        }
        "xtx/displayName" if matches!(locale, Locale::En) => {
            let inputs = noun_inputs();
            if !inputs.target.contains(key) {
                document["sourceLookup"] = json!({"status": "unsupported-noun-block"});
                return Ok((document, true));
            }
            let configuration_pins = if noun_source.is_some() {
                &inputs.configuration_pins[..]
            } else {
                &inputs.configuration_pins[..2]
            };
            for pin in configuration_pins {
                read_pin(Path::new(root), pin)?;
            }
            document["nounConfigurationSources"] = json!(inputs.configuration_pins
                [..configuration_pins.len()]
                .iter()
                .map(pin_source)
                .collect::<Vec<_>>());
            let target = load(
                Path::new(root),
                &inputs.target.pins,
                &[
                    ColumnType::Text,
                    ColumnType::Text,
                    ColumnType::Signed8,
                    ColumnType::Signed8,
                    ColumnType::Signed8,
                ],
                inputs.target.begin,
                inputs.target.count,
            )?;
            document["targetSources"] = target.sources;
            document["targetBlock"] =
                json!({"begin": inputs.target.begin, "count": inputs.target.count});
            if let Some(source) = noun_source {
                let attributive = load(
                    Path::new(root),
                    &inputs.attributive.pins,
                    &[
                        ColumnType::Signed8,
                        ColumnType::Text,
                        ColumnType::Text,
                        ColumnType::Text,
                        ColumnType::Text,
                        ColumnType::Text,
                        ColumnType::Text,
                    ],
                    inputs.attributive.begin,
                    inputs.attributive.count,
                )?;
                document["attributiveSources"] = attributive.sources;
                if let Some((_, row)) = target.rows.iter().find(|(row_key, _)| *row_key == key) {
                    document["textResolution"] = english_noun_text(
                        row,
                        attributive
                            .rows
                            .iter()
                            .find(|(row_key, _)| *row_key == source[2])
                            .map(|(_, row)| row),
                        source,
                    );
                }
            }
            (target.rows, vec![1, 2, 3, 4, 5])
        }
        _ => {
            document["sourceLookup"] = json!({"status": "unsupported-key-association"});
            return Ok((document, false));
        }
    };
    let Some((_, row)) = rows.iter().find(|(row_key, _)| *row_key == key) else {
        document["sourceLookup"] = json!({"status": "not-found"});
        return Ok((document, true));
    };
    let default_field = match descriptor.sheet.as_str() {
        "xtx/placeName" => Some(PLACE_DEFAULT_FIELDS[index]),
        "xtx/itemName" => Some(ITEM_DEFAULT_FIELDS[index]),
        "xtx/command" => Some(command_inputs()[index].default),
        "xtx/displayName" if noun_source.is_none() => Some(1),
        _ => None,
    };
    if let Some(column) = default_field {
        document["textResolution"] = plain_text(
            row,
            &fields,
            column,
            dispatch(descriptor.noun, locale) != "query",
        );
        if document["textResolution"]["reason"] == "embedded-controls" {
            let position = fields.iter().position(|&field| field == column).unwrap();
            let ColumnValue::Text(value) = &row.values[position] else {
                unreachable!("plain_text admitted selected string")
            };
            let inputs = command_inputs();
            let input = &inputs[index];
            debug_assert_eq!(input.locale, locale_name);
            let mut cache = Vec::new();
            document["textResolution"] =
                sheet_reference::resolve(&value.rich, column, |name, key, field| {
                    command_query(Path::new(root), input, &mut cache, name, key, field)
                })?;
        }
    }
    let values = if descriptor_index == 0 {
        let (field0, metadata) = metadata(row).map_err(|error| Failure::parse(error.detail()))?;
        if field0.checked_sub(1) != Some(category) || !metadata.is_empty() {
            document["sourceLookup"] = json!({"status": "not-ordinary-phrase"});
            return Ok((document, true));
        }
        &row.values[2..]
    } else {
        &row.values[..]
    };
    let cells = source_cells(values, &fields);
    let association = if descriptor.sheet == "xtx/displayName" {
        "conditional-ready-map-key"
    } else {
        "conditional-backing-key"
    };
    document["sourceLookup"] = json!({"status": "matched", "association": association, "rowKey": key, "textEncoding": "ssd-lossless-csv", "fields": cells});
    Ok((document, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_source_keeps_signed_fields_and_selects_the_default_name() {
        let inputs = command_inputs();
        let input = &inputs[1];
        let mut data = Vec::new();
        for kind in &input.types {
            match *kind {
                "str" => {
                    let value = b"Authored command";
                    data.extend_from_slice(&((value.len() + 1) as u16).to_le_bytes());
                    data.extend_from_slice(value);
                    data.push(0);
                }
                "s32" => data.extend_from_slice(&(-7i32).to_le_bytes()),
                _ => unreachable!("supported command schema"),
            }
        }
        let types: Vec<_> = input
            .types
            .iter()
            .map(|kind| ColumnType::parse(kind, 0).unwrap())
            .collect();
        let row = sheet::parse_rows(&data, &types).unwrap().remove(0);
        let cells = source_cells(&row.values, &input.columns);
        assert_eq!(cells.len(), 7);
        assert_eq!(cells[3], json!({"column": 5, "value": -7}));
        let resolved = plain_text(&row, &input.columns, input.default, false);
        assert_eq!(resolved["column"], 2);
        assert_eq!(resolved["text"], "Authored command");
    }

    #[test]
    fn english_fixed_query_does_not_treat_the_noun_flag_as_noun_mode() {
        assert_eq!(dispatch(true, Locale::En), "query");
        assert_eq!(dispatch(false, Locale::En), "query");
        assert_eq!(dispatch(true, Locale::De), "provider-dependent-noun");
        assert_eq!(dispatch(false, Locale::De), "query");
        let row = noun_row(b"Authored [n]", [0, 1, 0]);
        let resolved = plain_text(&row, &[1, 2, 3, 4, 5], 1, false);
        assert_eq!(resolved["text"], "Authored [n]");
        assert_eq!(resolved["association"], "conditional-default-field");
    }

    fn noun_row(singular: &[u8], flags: [u8; 3]) -> Row {
        let mut data = Vec::new();
        for value in [singular, b"Authored plural"] {
            data.extend_from_slice(&((value.len() + 1) as u16).to_le_bytes());
            data.extend_from_slice(value);
            data.push(0);
        }
        data.extend_from_slice(&flags);
        sheet::parse_rows(
            &data,
            &[
                ColumnType::Text,
                ColumnType::Text,
                ColumnType::Signed8,
                ColumnType::Signed8,
                ColumnType::Signed8,
            ],
        )
        .unwrap()
        .remove(0)
    }

    #[test]
    fn explicit_english_noun_copies_singular_with_literal_identity_replacement() {
        let row = noun_row(b"Authored [literal]\\name", [0, 0, 1]);
        let resolved = english_noun_text(&row, Some(&row), [0, 1, 5, 0]);
        assert_eq!(resolved["text"], "Authored [literal]\\name");
        assert_eq!(resolved["association"], "conditional-explicit-NounEn");
        assert_eq!(resolved["column"], 1);
        assert_eq!(
            english_noun_text(&noun_row(b"", [0, 0, 1]), Some(&row), [0, 1, 5, 0])["text"],
            ""
        );
    }

    #[test]
    fn explicit_english_noun_defers_other_branches_and_missing_admission() {
        let row = noun_row(b"Authored", [0, 0, 1]);
        for (candidate, admitted, inputs, reason) in [
            (row.clone(), None, [0, 1, 5, 0], "attributive-row-not-found"),
            (
                row.clone(),
                Some(&row),
                [1, 1, 5, 0],
                "unsupported-noun-inputs",
            ),
            (
                row.clone(),
                Some(&row),
                [0, 2, 5, 0],
                "unsupported-noun-inputs",
            ),
            (
                row.clone(),
                Some(&row),
                [0, 1, 1, 0],
                "unsupported-noun-inputs",
            ),
            (
                noun_row(b"Authored", [0, 1, 0]),
                Some(&row),
                [0, 1, 5, 0],
                "unsupported-noun-row",
            ),
            (
                noun_row(b"Authored [n]", [0, 0, 1]),
                Some(&row),
                [0, 1, 5, 0],
                "noun-count-replacement",
            ),
            (
                noun_row(b"A\x02\x1f\x01\x03B", [0, 0, 1]),
                Some(&row),
                [0, 1, 5, 0],
                "embedded-controls",
            ),
        ] {
            let result = english_noun_text(&candidate, admitted, inputs);
            assert_eq!(result["reason"], reason);
            assert!(result.get("text").is_none());
        }
        assert_eq!(text(&row.values[0]), "Authored");
        let inputs = noun_inputs();
        assert_eq!((inputs.target.begin, inputs.target.count), (1_000_000, 601));
        assert!(!inputs.target.contains(1_000_601));
        assert!(inputs.attributive.contains(5));
    }

    const COLUMNS: [ColumnType; 4] = [
        ColumnType::Unsigned32,
        ColumnType::Text,
        ColumnType::Text,
        ColumnType::Text,
    ];

    fn row(field0: u32, descriptor: &str) -> Vec<u8> {
        let mut bytes = field0.to_le_bytes().to_vec();
        for value in [descriptor, "Authored text", ""] {
            bytes.extend_from_slice(&((value.len() + 1) as u16).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn separates_map_indices_ordinals_descriptors_and_dynamic_categories() {
        let data = [
            row(1, "@"),
            row(3, "xtx/authored:noun[2-4]#9"),
            row(4, "#5"),
        ]
        .concat();
        let rows: Vec<_> = sheet::parse_rows(&data, &COLUMNS)
            .unwrap()
            .into_iter()
            .enumerate()
            .map(|(i, row)| (100 + i as u32, row))
            .collect();
        let registry = reconstruct(&rows).unwrap();
        assert_eq!(registry.map, [Some(0), None, Some(1), Some(2)]);
        assert_eq!(registry.categories[0].descriptor, Some(0));
        assert_eq!(registry.categories[1].source_key, 101);
        assert_eq!(registry.categories[1].descriptor, Some(1));
        assert_eq!(registry.descriptors[1].sheet, "xtx/authored");
        assert!(registry.descriptors[1].noun);
        assert_eq!(registry.categories[2].descriptor, None);
        assert_eq!(registry.descriptors.len(), 2);
    }

    #[test]
    fn map_growth_does_not_repair_earlier_holes_or_replace_existing_entries() {
        let data = [row(3, "@"), row(1, "@"), row(3, "xtx/authored")].concat();
        let rows: Vec<_> = sheet::parse_rows(&data, &COLUMNS)
            .unwrap()
            .into_iter()
            .map(|row| (100, row))
            .collect();
        let registry = reconstruct(&rows).unwrap();
        assert_eq!(registry.map, [None, None, Some(0)]);
        assert_eq!(registry.categories.len(), 3);
        assert_eq!(registry.categories[2].descriptor, Some(1));
    }

    #[test]
    fn backing_checks_sparse_keys_row_spans_and_out_of_block_ranges() {
        let data = [row(1, "@"), row(1, "")].concat();
        let first_end = row(1, "@").len() as u32;
        let offsets: Vec<_> = [first_end, first_end, data.len() as u32]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let enable: Vec<_> = [100u32, 1, 102, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let rows = parse_backing(&data, &enable, &offsets, &COLUMNS, 100, 3).unwrap();
        assert_eq!(
            rows.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            [100, 102]
        );
        let wrong_enable: Vec<_> = [100u32, 3].into_iter().flat_map(u32::to_le_bytes).collect();
        assert!(parse_backing(&data, &wrong_enable, &offsets, &COLUMNS, 100, 3).is_err());
        let huge_enable: Vec<_> = [100u32, u32::MAX]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert!(parse_backing(&data, &huge_enable, &offsets, &COLUMNS, 100, 3).is_err());
        let mut wrong_offsets = offsets.clone();
        wrong_offsets[..4].copy_from_slice(&(first_end - 1).to_le_bytes());
        assert!(parse_backing(&data, &enable, &wrong_offsets, &COLUMNS, 100, 3).is_err());
    }

    #[test]
    fn item_blocks_keep_exclusive_bounds_and_locale_layouts() {
        let inputs = item_inputs();
        assert_eq!(inputs.len(), 4);
        for (input, locale) in inputs.iter().zip(["ja", "en", "de", "fr"]) {
            assert_eq!(input.locale, locale);
            assert_eq!(input.columns.len(), input.types.len());
            assert!(input.types.iter().all(|kind| matches!(*kind, "str" | "s8")));
            assert_eq!(input.blocks.len(), 50);
            assert!(input.blocks.windows(2).all(|pair| {
                u64::from(pair[0].begin) + pair[0].count as u64 <= u64::from(pair[1].begin)
            }));
            let block = &input.blocks[1];
            assert_eq!((block.begin, block.count), (1_000_000, 204));
            assert!(!block.contains(999_999));
            assert!(block.contains(1_000_000));
            assert!(block.contains(1_000_203));
            assert!(!block.contains(1_000_204));
            assert!(!input.blocks.iter().any(|block| block.contains(1_500_000)));
        }
        assert_eq!(inputs[1].columns, [5, 6, 7, 8, 9, 10, 26, 30]);
    }

    fn text_row(values: &[&[u8]]) -> Row {
        let data: Vec<_> = values
            .iter()
            .flat_map(|value| {
                let mut bytes = ((value.len() + 1) as u16).to_le_bytes().to_vec();
                bytes.extend_from_slice(value);
                bytes.push(0);
                bytes
            })
            .collect();
        sheet::parse_rows(&data, &vec![ColumnType::Text; values.len()])
            .unwrap()
            .remove(0)
    }

    #[test]
    fn plain_text_selects_global_column_and_preserves_literal_text() {
        let row = text_row(&[b"Unused", b"[Literal]\\Authored", b""]);
        let resolved = plain_text(&row, &[11, 19, 28], 19, false);
        assert_eq!(resolved["status"], "resolved");
        assert_eq!(resolved["column"], 19);
        assert_eq!(resolved["text"], "[Literal]\\Authored");
        assert_eq!(resolved["association"], "conditional-default-field");
        assert_eq!(resolved["textEncoding"], "utf-8");
        assert_eq!(plain_text(&row, &[11, 19, 28], 28, false)["text"], "");
    }

    #[test]
    fn plain_text_defers_controls_and_nouns_without_dropping_tokens() {
        let row = text_row(&[b"A\x02\x1f\x01\x03B", b"Authored"]);
        let deferred = plain_text(&row, &[5, 6], 5, false);
        assert_eq!(deferred["status"], "requires-evaluation");
        assert_eq!(deferred["reason"], "embedded-controls");
        assert!(deferred.get("text").is_none());
        assert_eq!(text(&row.values[0]), "A[@hyphen:021f0103]B");
        assert_eq!(plain_text(&row, &[5, 6], 6, false)["text"], "Authored");
        let noun = plain_text(&row, &[5, 6], 6, true);
        assert_eq!(noun["reason"], "provider-dependent-noun");
        assert!(noun.get("text").is_none());
    }

    #[test]
    fn plain_text_does_not_guess_missing_or_numeric_default_fields() {
        let row = text_row(&[b"Authored"]);
        assert_eq!(
            plain_text(&row, &[5], 6, false)["status"],
            "unsupported-default-field"
        );
        let numeric = sheet::parse_rows(&[1], &[ColumnType::Signed8])
            .unwrap()
            .remove(0);
        assert_eq!(
            plain_text(&numeric, &[8], 8, false)["status"],
            "unsupported-default-field"
        );
    }

    #[test]
    fn command_query_maps_explicit_and_default_columns_and_refuses_unsupported_inputs() {
        let inputs = command_inputs();
        for (input, (locale, default)) in
            inputs
                .iter()
                .zip([("ja", 1), ("en", 2), ("de", 8), ("fr", 16)])
        {
            assert_eq!((input.locale, input.default), (locale, default));
            assert_eq!(input.blocks.len(), 23);
            assert_eq!(input.columns.len(), input.types.len());
            assert!(input.blocks.windows(2).all(|pair| {
                u64::from(pair[0].begin) + pair[0].count as u64 <= u64::from(pair[1].begin)
            }));
            let mut row = text_row(&[b"Authored"]);
            row.values
                .resize(input.columns.len(), row.values[0].clone());
            let mut cache = vec![(
                1,
                Loaded {
                    rows: vec![(10_042, row)],
                    sources: json!([]),
                },
            )];
            for requested in [default, 255] {
                let cell = command_query(
                    Path::new("unused"),
                    input,
                    &mut cache,
                    "xtx/command",
                    10_042,
                    requested,
                )
                .unwrap()
                .unwrap();
                assert_eq!(cell.rich.text_only(), "Authored");
                assert_eq!(cell.evidence["column"], default);
                assert_eq!(cell.evidence["requestedColumn"], requested);
            }
            assert!(matches!(
                command_query(
                    Path::new("unused"),
                    input,
                    &mut cache,
                    "xtx/command",
                    10_043,
                    default
                ),
                Ok(Err("sheet-row-not-found"))
            ));
            assert!(matches!(
                command_query(
                    Path::new("unused"),
                    input,
                    &mut cache,
                    "xtx/command",
                    9_999,
                    default
                ),
                Ok(Err("sheet-row-not-found"))
            ));
            assert!(matches!(
                command_query(
                    Path::new("unused"),
                    input,
                    &mut cache,
                    "xtx/other",
                    10_042,
                    default
                ),
                Ok(Err("unsupported-sheet-target"))
            ));
            assert!(matches!(
                command_query(
                    Path::new("unused"),
                    input,
                    &mut cache,
                    "xtx/command",
                    10_042,
                    254
                ),
                Ok(Err("unsupported-sheet-column"))
            ));
            if let Some(position) = input.types.iter().position(|kind| *kind == "s32") {
                assert!(matches!(
                    command_query(
                        Path::new("unused"),
                        input,
                        &mut cache,
                        "xtx/command",
                        10_042,
                        input.columns[position]
                    ),
                    Ok(Err("unsupported-sheet-column"))
                ));
            }
        }
    }
}
