//! Agent-friendly queries over the explicit command battle-parameter catalog.

use std::collections::HashSet;
use std::io::{Read, Write};

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::Failure;

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CATALOG_ROWS: usize = 100_000;

const HEADER: &[&str] = &[
    "id",
    "name_en",
    "name_jp",
    "description_en",
    "description_jp",
    "id_band",
    "class_job",
    "req_level",
    "compat_key",
    "caster_state_req",
    "dmg_attr",
    "dmg_attr_label",
    "dmg_attr_weight",
    "dmg_elem",
    "dmg_elem_label",
    "dmg_elem_weight",
    "dmg_class",
    "magnitude",
    "hp_cost",
    "mp_cost",
    "tp_cost",
    "cast_time",
    "recast_time",
    "action_gauge",
    "range",
    "best_range",
    "min_range",
    "effect_range",
    "recast_sep_hands",
    "target_state_gate",
    "p1_base",
    "p1_grow",
    "p1_compat_adjust",
    "p1_tp_adjust",
    "p2_base",
    "p2_grow",
    "p2_compat_adjust",
    "p2_tp_adjust",
    "p3_base",
    "p3_grow",
    "p3_compat_adjust",
    "p3_tp_adjust",
    "p4_base",
    "p4_grow",
    "p4_compat_adjust",
    "p4_tp_adjust",
    "effect_block_raw",
];

#[derive(Clone, Copy)]
enum OutputFormat {
    Yaml,
    Json,
}

pub(crate) fn run(arguments: &[String]) -> Result<(), Failure> {
    let (query, catalog_path, format) = parse_arguments(arguments)?;
    let data = read_catalog(&catalog_path)?;
    let report = build_report(&data, &query).map_err(Failure::usage)?;
    let text = match format {
        OutputFormat::Yaml => serde_yaml::to_string(&report)
            .map_err(|error| Failure::usage(format!("cannot encode YAML report: {error}")))?,
        OutputFormat::Json => {
            let mut text = serde_json::to_string_pretty(&report)
                .map_err(|error| Failure::usage(format!("cannot encode JSON report: {error}")))?;
            text.push('\n');
            text
        }
    };
    std::io::stdout()
        .write_all(text.as_bytes())
        .map_err(|error| Failure::usage(format!("cannot write output: {error}")))
}

fn parse_arguments(arguments: &[String]) -> Result<(String, String, OutputFormat), Failure> {
    let Some(query) = arguments.first() else {
        return Err(usage());
    };
    if query.is_empty() || query.starts_with("--") {
        return Err(usage());
    }

    let mut catalog = None;
    let mut format = OutputFormat::Yaml;
    let mut index = 1;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--catalog" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(usage());
                };
                if catalog.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--catalog may be supplied only once"));
                }
            }
            "--format" => {
                index += 1;
                format = match arguments.get(index).map(String::as_str) {
                    Some("yaml") => OutputFormat::Yaml,
                    Some("json") => OutputFormat::Json,
                    _ => return Err(Failure::usage("--format must be yaml or json")),
                };
            }
            option => {
                return Err(Failure::usage(format!(
                    "unknown inspect-command option '{option}'"
                )))
            }
        }
        index += 1;
    }
    let catalog = catalog.ok_or_else(usage)?;
    Ok((query.clone(), catalog, format))
}

fn usage() -> Failure {
    Failure::usage(
        "usage: xivl inspect-command <id-or-name> --catalog <command_battle_params.csv> [--format yaml|json]",
    )
}

fn read_catalog(path: &str) -> Result<Vec<u8>, Failure> {
    let file = std::fs::File::open(path)
        .map_err(|error| Failure::usage(format!("cannot read '{path}': {error}")))?;
    let mut data = Vec::new();
    file.take(MAX_CATALOG_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| Failure::usage(format!("cannot read '{path}': {error}")))?;
    if data.len() as u64 > MAX_CATALOG_BYTES {
        return Err(Failure::usage(format!(
            "catalog is larger than the {MAX_CATALOG_BYTES}-byte limit"
        )));
    }
    Ok(data)
}

fn build_report(data: &[u8], query: &str) -> Result<Value, String> {
    let mut reader = csv::ReaderBuilder::new().from_reader(data);
    let headers = reader
        .headers()
        .map_err(|error| format!("invalid catalog header: {error}"))?;
    if !headers.iter().eq(HEADER.iter().copied()) {
        return Err("catalog header does not match command_battle_params.csv v1".to_owned());
    }

    let numeric_query = query.parse::<u32>().ok();
    let folded_query = query.to_lowercase();
    let mut ids = HashSet::new();
    let mut matches = Vec::new();
    let mut row_count = 0_usize;

    for result in reader.records() {
        let record = result.map_err(|error| format!("invalid catalog row: {error}"))?;
        row_count += 1;
        if row_count > MAX_CATALOG_ROWS {
            return Err(format!("catalog exceeds the {MAX_CATALOG_ROWS}-row limit"));
        }
        if record.len() != HEADER.len() {
            return Err(format!(
                "catalog row {row_count} has {} fields; expected {}",
                record.len(),
                HEADER.len()
            ));
        }
        let id = field(&record, "id")
            .parse::<u32>()
            .map_err(|_| format!("catalog row {row_count} has an invalid command id"))?;
        if !ids.insert(id) {
            return Err(format!("catalog contains duplicate command id {id}"));
        }
        parse_effect_fields(field(&record, "effect_block_raw"))?;

        let matched = match numeric_query {
            Some(wanted) => id == wanted,
            None => {
                field(&record, "name_en").to_lowercase() == folded_query
                    || field(&record, "name_jp").to_lowercase() == folded_query
            }
        };
        if matched {
            matches.push(command_document(&record, id)?);
        }
    }

    if matches.is_empty() {
        return Err(format!("command query '{query}' did not match the catalog"));
    }

    Ok(json!({
        "schemaVersion": 1,
        "kind": "xivl-command-formula-inputs",
        "source": {
            "byteLength": data.len(),
            "sha256": sha256(data),
            "catalogRows": row_count,
        },
        "query": {
            "value": query,
            "mode": if numeric_query.is_some() { "id" } else { "exact-name" },
            "comparison": if numeric_query.is_some() { "numeric" } else { "case-insensitive" },
        },
        "formulaModel": {
            "scope": "client-prediction-inputs",
            "parameterExpression": "levelAdjustedBase * compatibilityFactor * tpFactor",
            "compatibilityFactor": {
                "whenRawAdjustIsZero": 1,
                "otherwise": "1 - (1 - compatibilityByHand) * rawCompatibilityAdjust",
            },
            "tpFactor": {
                "recoveredBaseImplementation": 1,
                "rawAdjustmentRetained": true,
                "luaOverridesInFrozenCorpus": 0,
            },
            "serverAuthoritative": false,
            "unresolved": [
                "native getGrowData curves",
                "native magnitude scale and combine step",
                "command-to-status linkage",
            ],
        },
        "matches": matches,
    }))
}

fn command_document(record: &csv::StringRecord, id: u32) -> Result<Value, String> {
    let parameters: Vec<Value> = (1..=4)
        .map(|number| {
            json!({
                "number": number,
                "base": scalar(field(record, &format!("p{number}_base"))),
                "grow": scalar(field(record, &format!("p{number}_grow"))),
                "compatibilityAdjust": scalar(field(record, &format!("p{number}_compat_adjust"))),
                "tpAdjust": scalar(field(record, &format!("p{number}_tp_adjust"))),
            })
        })
        .collect();

    Ok(json!({
        "identity": {
            "id": id,
            "idBand": field(record, "id_band"),
            "nameEnglish": field(record, "name_en"),
            "nameJapanese": field(record, "name_jp"),
        },
        "description": {
            "english": field(record, "description_en"),
            "japanese": field(record, "description_jp"),
        },
        "requirements": {
            "classJob": scalar(field(record, "class_job")),
            "level": scalar(field(record, "req_level")),
            "compatibilityKey": scalar(field(record, "compat_key")),
            "casterState": scalar(field(record, "caster_state_req")),
        },
        "damage": {
            "class": field(record, "dmg_class"),
            "magnitude": scalar(field(record, "magnitude")),
            "attribute": scalar(field(record, "dmg_attr")),
            "attributeLabel": field(record, "dmg_attr_label"),
            "attributeWeight": scalar(field(record, "dmg_attr_weight")),
            "element": scalar(field(record, "dmg_elem")),
            "elementLabel": field(record, "dmg_elem_label"),
            "elementWeight": scalar(field(record, "dmg_elem_weight")),
        },
        "costs": {
            "hp": scalar(field(record, "hp_cost")),
            "mp": scalar(field(record, "mp_cost")),
            "tp": scalar(field(record, "tp_cost")),
            "actionGauge": scalar(field(record, "action_gauge")),
        },
        "timing": {
            "cast": scalar(field(record, "cast_time")),
            "recast": scalar(field(record, "recast_time")),
            "separateHands": scalar(field(record, "recast_sep_hands")),
        },
        "targeting": {
            "range": scalar(field(record, "range")),
            "bestRange": scalar(field(record, "best_range")),
            "minimumRange": scalar(field(record, "min_range")),
            "effectRange": scalar(field(record, "effect_range")),
            "targetState": scalar(field(record, "target_state_gate")),
        },
        "parameters": parameters,
        "rawEffectFields": parse_effect_fields(field(record, "effect_block_raw"))?,
    }))
}

fn field<'a>(record: &'a csv::StringRecord, name: &str) -> &'a str {
    let index = HEADER
        .iter()
        .position(|candidate| *candidate == name)
        .expect("internal catalog field name");
    record.get(index).expect("validated catalog row width")
}

fn parse_effect_fields(raw: &str) -> Result<Value, String> {
    let mut fields = Map::new();
    if raw.is_empty() {
        return Ok(Value::Object(fields));
    }
    for component in raw.split(';') {
        let Some((column, value)) = component.split_once('=') else {
            return Err("effect_block_raw contains a field without '='".to_owned());
        };
        let parsed_column = column
            .parse::<u16>()
            .map_err(|_| "effect_block_raw contains a non-numeric column".to_owned())?;
        if !((84..=116).contains(&parsed_column) || parsed_column == 120) {
            return Err(format!(
                "effect_block_raw contains out-of-range column {parsed_column}"
            ));
        }
        if fields.insert(column.to_owned(), scalar(value)).is_some() {
            return Err(format!(
                "effect_block_raw contains duplicate column {parsed_column}"
            ));
        }
    }
    Ok(Value::Object(fields))
}

fn scalar(raw: &str) -> Value {
    if raw.is_empty() {
        Value::Null
    } else if raw.eq_ignore_ascii_case("true") {
        Value::Bool(true)
    } else if raw.eq_ignore_ascii_case("false") {
        Value::Bool(false)
    } else if let Ok(value) = raw.parse::<i64>() {
        Value::Number(value.into())
    } else if let Ok(value) = raw.parse::<u64>() {
        Value::Number(value.into())
    } else if let Ok(value) = raw.parse::<f64>() {
        serde_json::Number::from_f64(value)
            .map_or_else(|| Value::String(raw.to_owned()), Value::Number)
    } else {
        Value::String(raw.to_owned())
    }
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(rows: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(Vec::new());
        writer.write_record(HEADER).unwrap();
        for (id, english, japanese) in rows {
            let mut row = vec![String::new(); HEADER.len()];
            row[index("id")] = (*id).to_owned();
            row[index("name_en")] = (*english).to_owned();
            row[index("name_jp")] = (*japanese).to_owned();
            row[index("description_en")] = "Deals fire damage.".to_owned();
            row[index("id_band")] = "27xxx".to_owned();
            row[index("magnitude")] = "950".to_owned();
            row[index("p3_grow")] = "-1".to_owned();
            row[index("p3_base")] = "13".to_owned();
            row[index("p3_compat_adjust")] = "1".to_owned();
            row[index("p3_tp_adjust")] = "0".to_owned();
            row[index("effect_block_raw")] = "84=950;108=13".to_owned();
            writer.write_record(row).unwrap();
        }
        writer.into_inner().unwrap()
    }

    fn index(name: &str) -> usize {
        HEADER.iter().position(|field| *field == name).unwrap()
    }

    #[test]
    fn queries_id_and_duplicate_exact_names() {
        let data = catalog(&[
            ("27310", "Fire", "Fire JP"),
            ("27410", "Fire", "Fire II JP"),
        ]);
        let by_id = build_report(&data, "27310").unwrap();
        assert_eq!(by_id["query"]["mode"], "id");
        assert_eq!(by_id["matches"].as_array().unwrap().len(), 1);
        assert_eq!(by_id["matches"][0]["damage"]["magnitude"], 950);
        assert_eq!(by_id["matches"][0]["rawEffectFields"]["108"], 13);

        let by_name = build_report(&data, "fIrE").unwrap();
        assert_eq!(by_name["query"]["mode"], "exact-name");
        assert_eq!(by_name["matches"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn rejects_wrong_header_duplicate_ids_and_missing_queries() {
        assert!(build_report(b"wrong\nvalue\n", "1")
            .unwrap_err()
            .contains("header"));
        let duplicate = catalog(&[("27310", "Fire", "A"), ("27310", "Fira", "B")]);
        assert!(build_report(&duplicate, "27310")
            .unwrap_err()
            .contains("duplicate command id"));
        let valid = catalog(&[("27310", "Fire", "A")]);
        assert!(build_report(&valid, "Cure")
            .unwrap_err()
            .contains("did not match"));
    }
}
