//! Conditional reconstruction from authenticated, locale-specific backing rows.

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};
use xivl_formats::digest::sha256_hex;
use xivl_formats::error::{ErrorKind, FormatError, Result};
use xivl_formats::resource::ResourceId;
use xivl_formats::sheet::{self, ColumnType, ColumnValue, Row};

use crate::fixed_phrase::Locale;
use crate::initial_registry_inputs::{FIXED, PLACE};
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

fn text(value: &ColumnValue) -> String {
    match value {
        ColumnValue::Text(value) => value.rich.to_lossless_text(),
        _ => unreachable!("validated string column"),
    }
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
        let id = ResourceId::new(pin.id);
        let path = root.join(id.dat_path());
        let data = read_capped(&path.to_string_lossy())?;
        if data.len() != pin.size || sha256_hex(&data) != pin.sha256 {
            return Err(Failure::parse(format!(
                "{}: size or SHA-256 differs from authenticated 1.23b input",
                id.to_hex()
            )));
        }
        sources.push(json!({"resourceId": id.to_hex(), "bytes": pin.size, "sha256": pin.sha256}));
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

pub fn report(
    root: &str,
    numbers: [u32; 2],
    locale: Locale,
) -> std::result::Result<(Value, bool), Failure> {
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
    document["dispatch"] = json!(if descriptor.noun {
        "provider-dependent-noun"
    } else {
        "query"
    });
    document["status"] = json!("selected");
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
        _ => {
            document["sourceLookup"] = json!({"status": "unsupported-key-association"});
            return Ok((document, false));
        }
    };
    let Some((_, row)) = rows.iter().find(|(row_key, _)| *row_key == key) else {
        document["sourceLookup"] = json!({"status": "not-found"});
        return Ok((document, true));
    };
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
    let cells: Vec<Value> = values
        .iter()
        .zip(fields)
        .map(|(value, column)| {
            let value = match value {
                ColumnValue::Text(_) => json!(text(value)),
                ColumnValue::Unsigned8 { value, .. } => json!(value),
                ColumnValue::Signed8 { value, .. } => json!(value),
                _ => unreachable!("supported source schema"),
            };
            json!({"column": column, "value": value})
        })
        .collect();
    document["sourceLookup"] = json!({"status": "matched", "association": "conditional-backing-key", "rowKey": key, "textEncoding": "ssd-lossless-csv", "fields": cells});
    Ok((document, false))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
