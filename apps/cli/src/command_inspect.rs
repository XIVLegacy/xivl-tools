//! Agent-friendly queries over the explicit command battle-parameter catalog.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::Failure;

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CATALOG_ROWS: usize = 100_000;
const MAX_SLOT_CONTEXT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SlotContextManifest {
    schema_version: u32,
    kind: String,
    game_version: String,
    status: String,
    source_snapshots: SourceSnapshots,
    derivation: Derivation,
    coverage: Coverage,
    rows_sha256: String,
    rows: Vec<SlotContextRow>,
    unresolved: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SourceSnapshots {
    captures: CaptureSnapshot,
    client_structs: ClientStructSnapshot,
    client_data: ClientDataSnapshot,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CaptureSnapshot {
    repository: String,
    commit: String,
    artifact: String,
    sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClientStructSnapshot {
    repository: String,
    generator_artifact: String,
    generator_sha256: String,
    hash_names_artifact: String,
    hash_names_sha256: String,
    actor_identity_artifact: String,
    actor_identity_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ClientDataSnapshot {
    repository: String,
    commit: String,
    static_actor_artifact: String,
    static_actor_sha256: String,
    command_catalog_artifact: String,
    command_catalog_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Derivation {
    carrier: String,
    state_partition: Vec<String>,
    state_order: String,
    state_rule: String,
    static_actor_test: String,
    command_id_decode: String,
    identity_boundary: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Coverage {
    command_records: u64,
    nonzero_command_occurrences: u64,
    unique_nonzero_command_actors: u64,
    static_actor_prefix_hits: u64,
    static_actor_catalog_hits: u64,
    command_catalog_hits: u64,
    category_records: u64,
    category_hashes: u64,
    category_value_distribution: Vec<CategoryValueDistribution>,
    stateful_category_observations: u64,
    commands_with_category_observations: u64,
    category_writes_without_current_command: Vec<CategoryWriteSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CategoryValueDistribution {
    value: u8,
    occurrences: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CategoryWriteSummary {
    slot: u8,
    occurrences: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SlotContextRow {
    actor_id_hex: String,
    command_id: u32,
    class_path: String,
    name_english: String,
    command_occurrences: u64,
    slot_observations: Vec<SlotObservation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SlotObservation {
    slot: u8,
    command_occurrences: u64,
    category_observations: Vec<CategoryObservation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CategoryObservation {
    value: u8,
    occurrences: u64,
}

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
    "lua_class_path",
    "compatibility_percent_by_skill",
];

#[derive(Clone, Copy)]
enum OutputFormat {
    Yaml,
    Json,
}

impl SlotContextManifest {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "slot context schemaVersion must be 1, got {}",
                self.schema_version
            ));
        }
        if self.kind != "xivl-command-slot-context" {
            return Err(format!(
                "slot context kind must be xivl-command-slot-context, got '{}'",
                self.kind
            ));
        }
        if self.game_version != "1.23b" {
            return Err(format!(
                "slot context gameVersion must be 1.23b, got '{}'",
                self.game_version
            ));
        }
        if self.rows.is_empty() {
            return Err("slot context rows must not be empty".to_owned());
        }
        validate_coverage(&self.coverage)?;
        let row_value = serde_json::to_value(&self.rows)
            .map_err(|error| format!("cannot canonicalize slot context rows: {error}"))?;
        let row_bytes = serde_json::to_vec(&row_value)
            .map_err(|error| format!("cannot canonicalize slot context rows: {error}"))?;
        if self.rows_sha256 != sha256(&row_bytes) {
            return Err("slot context rowsSha256 does not match rows".to_owned());
        }

        let declared_categories: HashMap<u8, u128> = self
            .coverage
            .category_value_distribution
            .iter()
            .map(|entry| (entry.value, u128::from(entry.occurrences)))
            .collect();
        let mut observed_categories: HashMap<u8, u128> = HashMap::new();
        let mut command_ids = HashSet::new();
        let mut command_occurrences = 0_u128;
        let mut category_observations = 0_u128;
        let mut commands_with_categories = 0_u64;
        for (row_index, row) in self.rows.iter().enumerate() {
            let row_label = format!("slot context row {}", row_index + 1);
            if row.command_id == 0 {
                return Err(format!("{row_label} commandId must be nonzero"));
            }
            if !command_ids.insert(row.command_id) {
                return Err(format!(
                    "slot context contains duplicate command id {}",
                    row.command_id
                ));
            }
            if row.command_occurrences == 0 {
                return Err(format!("{row_label} commandOccurrences must be nonzero"));
            }
            command_occurrences += u128::from(row.command_occurrences);
            if !row.class_path.starts_with("/Command/") {
                return Err(format!("{row_label} classPath must start with /Command/"));
            }
            let actor_id = parse_actor_id(&row.actor_id_hex)
                .map_err(|error| format!("{row_label} actorIdHex {error}"))?;
            if actor_id & 0xffff_0000 != 0xa0f0_0000 {
                return Err(format!(
                    "{row_label} actorIdHex must use static actor prefix 0xa0f00000"
                ));
            }
            if actor_id & 0xffff != row.command_id {
                return Err(format!(
                    "{row_label} actorIdHex low16 does not match commandId {}",
                    row.command_id
                ));
            }
            if row.slot_observations.is_empty() {
                return Err(format!("{row_label} slotObservations must not be empty"));
            }
            if row
                .slot_observations
                .iter()
                .map(|observation| u128::from(observation.command_occurrences))
                .sum::<u128>()
                != u128::from(row.command_occurrences)
            {
                return Err(format!(
                    "{row_label} slot commandOccurrences do not sum to commandOccurrences"
                ));
            }
            let mut slots = HashSet::new();
            let mut row_has_categories = false;
            for (slot_index, observation) in row.slot_observations.iter().enumerate() {
                let slot_label = format!("{row_label} slotObservations[{}]", slot_index);
                validate_slot(observation.slot, &slot_label)?;
                if !slots.insert(observation.slot) {
                    return Err(format!(
                        "{row_label} contains duplicate slot {}",
                        observation.slot
                    ));
                }
                if observation.command_occurrences == 0 {
                    return Err(format!("{slot_label} commandOccurrences must be nonzero"));
                }
                let mut categories = HashSet::new();
                for (category_index, category) in
                    observation.category_observations.iter().enumerate()
                {
                    let category_label =
                        format!("{slot_label} categoryObservations[{}]", category_index);
                    if !categories.insert(category.value) {
                        return Err(format!(
                            "{slot_label} contains duplicate category value {}",
                            category.value
                        ));
                    }
                    if category.occurrences == 0 {
                        return Err(format!("{category_label} occurrences must be nonzero"));
                    }
                    row_has_categories = true;
                    category_observations += u128::from(category.occurrences);
                    *observed_categories.entry(category.value).or_default() +=
                        u128::from(category.occurrences);
                }
            }
            commands_with_categories += u64::from(row_has_categories);
        }
        if self.coverage.unique_nonzero_command_actors != self.rows.len() as u64
            || u128::from(self.coverage.nonzero_command_occurrences) != command_occurrences
            || u128::from(self.coverage.stateful_category_observations) != category_observations
            || self.coverage.commands_with_category_observations != commands_with_categories
        {
            return Err("slot context coverage does not match rows".to_owned());
        }
        if observed_categories.iter().any(|(value, occurrences)| {
            declared_categories.get(value).copied().unwrap_or_default() < *occurrences
        }) {
            return Err(
                "slot context row categories do not match categoryValueDistribution".to_owned(),
            );
        }
        Ok(())
    }

    fn report_for_commands(
        &self,
        catalog_sha256: &str,
        catalog_identities: &HashMap<u32, (String, String)>,
    ) -> Result<Value, String> {
        if self.source_snapshots.client_data.command_catalog_sha256 != catalog_sha256 {
            return Err(
                "slot context command catalog pin does not match the supplied catalog".to_owned(),
            );
        }
        let matches: Vec<&SlotContextRow> = self
            .rows
            .iter()
            .filter(|row| catalog_identities.contains_key(&row.command_id))
            .collect();
        for row in &matches {
            let (name, class_path) = &catalog_identities[&row.command_id];
            if &row.name_english != name || &row.class_path != class_path {
                return Err(format!(
                    "slot context identity for command {} does not match the supplied catalog",
                    row.command_id
                ));
            }
        }
        Ok(json!({
            "status": "available",
            "schemaVersion": self.schema_version,
            "kind": self.kind,
            "gameVersion": self.game_version,
            "manifestStatus": self.status,
            "sourceSnapshots": self.source_snapshots,
            "derivation": self.derivation,
            "coverage": self.coverage,
            "rowsSha256": self.rows_sha256,
            "validation": {
                "input": "bounded-canonical-json",
                "rowsSha256": "matched",
                "commandCatalogSha256": "matched",
                "remainingSourceSnapshots": "manifest-declared-not-independently-verified",
            },
            "unresolved": self.unresolved,
            "matches": matches,
        }))
    }
}

fn validate_coverage(coverage: &Coverage) -> Result<(), String> {
    let counts = [
        ("commandRecords", coverage.command_records),
        (
            "nonzeroCommandOccurrences",
            coverage.nonzero_command_occurrences,
        ),
        (
            "uniqueNonzeroCommandActors",
            coverage.unique_nonzero_command_actors,
        ),
        ("staticActorPrefixHits", coverage.static_actor_prefix_hits),
        ("staticActorCatalogHits", coverage.static_actor_catalog_hits),
        ("commandCatalogHits", coverage.command_catalog_hits),
        ("categoryRecords", coverage.category_records),
        ("categoryHashes", coverage.category_hashes),
        (
            "statefulCategoryObservations",
            coverage.stateful_category_observations,
        ),
        (
            "commandsWithCategoryObservations",
            coverage.commands_with_category_observations,
        ),
    ];
    if let Some((label, _)) = counts.into_iter().find(|(_, value)| *value == 0) {
        return Err(format!("slot context coverage {label} must be nonzero"));
    }
    if coverage.category_value_distribution.is_empty() {
        return Err("slot context coverage categoryValueDistribution must not be empty".to_owned());
    }
    let mut values = HashSet::new();
    for (index, distribution) in coverage.category_value_distribution.iter().enumerate() {
        let label = format!("slot context coverage categoryValueDistribution[{index}]");
        if !values.insert(distribution.value) {
            return Err(format!(
                "slot context coverage contains duplicate category value {}",
                distribution.value
            ));
        }
        if distribution.occurrences == 0 {
            return Err(format!("{label} occurrences must be nonzero"));
        }
    }
    let mut slots = HashSet::new();
    for (index, summary) in coverage
        .category_writes_without_current_command
        .iter()
        .enumerate()
    {
        let label = format!("slot context coverage categoryWritesWithoutCurrentCommand[{index}]");
        validate_slot(summary.slot, &label)?;
        if !slots.insert(summary.slot) {
            return Err(format!(
                "slot context coverage contains duplicate slot {}",
                summary.slot
            ));
        }
        if summary.occurrences == 0 {
            return Err(format!("{label} occurrences must be nonzero"));
        }
    }
    let distributed_categories = coverage
        .category_value_distribution
        .iter()
        .map(|entry| u128::from(entry.occurrences))
        .sum::<u128>();
    let unjoined_categories = coverage
        .category_writes_without_current_command
        .iter()
        .map(|entry| u128::from(entry.occurrences))
        .sum::<u128>();
    if distributed_categories != u128::from(coverage.category_records)
        || u128::from(coverage.stateful_category_observations) + unjoined_categories
            != u128::from(coverage.category_records)
    {
        return Err("slot context category coverage is inconsistent".to_owned());
    }
    Ok(())
}

fn validate_slot(slot: u8, label: &str) -> Result<(), String> {
    if slot >= 64 {
        return Err(format!("{label} slot must be in range 0..=63"));
    }
    Ok(())
}

fn parse_actor_id(value: &str) -> Result<u32, String> {
    let digits = value
        .strip_prefix("0x")
        .ok_or_else(|| "must be a 0x-prefixed 8-digit hexadecimal value".to_owned())?;
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("must be a 0x-prefixed 8-digit hexadecimal value".to_owned());
    }
    u32::from_str_radix(digits, 16).map_err(|_| "is not a valid u32 hexadecimal value".to_owned())
}

pub(crate) fn run(arguments: &[String]) -> Result<(), Failure> {
    let (query, catalog_path, slot_context_path, format) = parse_arguments(arguments)?;
    let data = read_catalog(&catalog_path)?;
    let slot_context = slot_context_path
        .as_deref()
        .map(read_slot_context)
        .transpose()?;
    let report = build_report_with_slot_context(&data, &query, slot_context.as_ref())
        .map_err(Failure::usage)?;
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

fn parse_arguments(
    arguments: &[String],
) -> Result<(String, String, Option<String>, OutputFormat), Failure> {
    let Some(query) = arguments.first() else {
        return Err(usage());
    };
    if query.is_empty() || query.starts_with("--") {
        return Err(usage());
    }

    let mut catalog = None;
    let mut slot_context = None;
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
            "--slot-context" => {
                index += 1;
                let Some(path) = arguments.get(index) else {
                    return Err(usage());
                };
                if slot_context.replace(path.clone()).is_some() {
                    return Err(Failure::usage("--slot-context may be supplied only once"));
                }
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
    Ok((query.clone(), catalog, slot_context, format))
}

fn usage() -> Failure {
    Failure::usage(
        "usage: xivl inspect-command <id-or-name> --catalog <command_battle_params.csv> [--slot-context <command_slot_context.json>] [--format yaml|json]",
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

fn read_slot_context(path: &str) -> Result<SlotContextManifest, Failure> {
    let file = std::fs::File::open(path)
        .map_err(|error| Failure::usage(format!("cannot read '{path}': {error}")))?;
    let mut data = Vec::new();
    file.take(MAX_SLOT_CONTEXT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| Failure::usage(format!("cannot read '{path}': {error}")))?;
    if data.len() as u64 > MAX_SLOT_CONTEXT_BYTES {
        return Err(Failure::usage(format!(
            "slot context is larger than the {MAX_SLOT_CONTEXT_BYTES}-byte limit"
        )));
    }
    parse_slot_context(&data)
        .map_err(|error| Failure::usage(format!("cannot parse slot context '{path}': {error}")))
}

fn parse_slot_context(data: &[u8]) -> Result<SlotContextManifest, String> {
    let manifest = serde_json::from_slice::<SlotContextManifest>(data)
        .map_err(|error| format!("invalid slot context JSON: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}

#[cfg(test)]
fn build_report(data: &[u8], query: &str) -> Result<Value, String> {
    build_report_with_slot_context(data, query, None)
}

fn build_report_with_slot_context(
    data: &[u8],
    query: &str,
    slot_context: Option<&SlotContextManifest>,
) -> Result<Value, String> {
    let mut reader = csv::ReaderBuilder::new().from_reader(data);
    let headers = reader
        .headers()
        .map_err(|error| format!("invalid catalog header: {error}"))?;
    let catalog_width = headers.len();
    if !headers.iter().eq(HEADER.iter().copied())
        && !headers
            .iter()
            .eq(HEADER[..HEADER.len() - 1].iter().copied())
        && !headers
            .iter()
            .eq(HEADER[..HEADER.len() - 2].iter().copied())
    {
        return Err(
            "catalog header does not match command_battle_params.csv v1, v2, or v3".to_owned(),
        );
    }

    let numeric_query = query.parse::<u32>().ok();
    let folded_query = query.to_lowercase();
    let mut ids = HashSet::new();
    let mut matched_identities = HashMap::new();
    let mut matches = Vec::new();
    let mut row_count = 0_usize;
    let mut flat_command_count = 0_usize;
    let mut native_grow_command_count = 0_usize;
    let mut native_grow_selectors = BTreeSet::new();

    for result in reader.records() {
        let record = result.map_err(|error| format!("invalid catalog row: {error}"))?;
        row_count += 1;
        if row_count > MAX_CATALOG_ROWS {
            return Err(format!("catalog exceeds the {MAX_CATALOG_ROWS}-row limit"));
        }
        if record.len() != catalog_width {
            return Err(format!(
                "catalog row {row_count} has {} fields; expected {}",
                record.len(),
                catalog_width
            ));
        }
        let id = field(&record, "id")
            .parse::<u32>()
            .map_err(|_| format!("catalog row {row_count} has an invalid command id"))?;
        if !ids.insert(id) {
            return Err(format!("catalog contains duplicate command id {id}"));
        }
        parse_effect_fields(field(&record, "effect_block_raw"))?;
        let compatibility =
            parse_compatibility_values(optional_field(&record, "compatibility_percent_by_skill"))?;
        if catalog_width == HEADER.len() {
            let raw_key = field(&record, "compat_key");
            let has_key = !raw_key.is_empty();
            if has_key && raw_key.parse::<u32>().is_err() {
                return Err(format!(
                    "catalog row {row_count} has an invalid compatibility key"
                ));
            }
            if has_key == compatibility.is_empty() {
                return Err(format!(
                    "catalog row {row_count} must provide compatibility values exactly when its key is present"
                ));
            }
        }

        let growth = command_growth(&record, row_count)?;
        if growth.native_required {
            native_grow_command_count += 1;
            native_grow_selectors.extend(growth.selectors);
        } else {
            flat_command_count += 1;
        }

        let matched = match numeric_query {
            Some(wanted) => id == wanted,
            None => {
                field(&record, "name_en").to_lowercase() == folded_query
                    || field(&record, "name_jp").to_lowercase() == folded_query
            }
        };
        if matched {
            matched_identities.insert(
                id,
                (
                    field(&record, "name_en").to_owned(),
                    optional_field(&record, "lua_class_path").to_owned(),
                ),
            );
            matches.push(command_document(&record, id, &compatibility)?);
        }
    }

    if matches.is_empty() {
        return Err(format!("command query '{query}' did not match the catalog"));
    }

    let catalog_sha256 = sha256(data);
    let observed_command_slot_context = match slot_context {
        Some(context) => context.report_for_commands(&catalog_sha256, &matched_identities)?,
        None => json!({
            "status": "unavailable",
            "reason": "slot-context-input-not-supplied",
        }),
    };

    Ok(json!({
        "schemaVersion": 11,
        "kind": "xivl-command-formula-inputs",
        "source": {
            "byteLength": data.len(),
            "sha256": catalog_sha256,
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
            "parameterExpressionScope": "complete-context-with-live-target",
            "levelAdjustment": {
                "scope": "GameCommandBaseClass defaults",
                "growRatio": "getGrowData(adjustedActorLevel, selector) / getGrowData(commandLevel, selector)",
                "actorLevelBelowCommand": "unbounded",
                "actorLevelAboveCommandCap": 15,
                "lowLevelBlend": 1,
                "highLevelBlend": 0.7,
            },
            "growthCoverage": {
                "flatCommandCount": flat_command_count,
                "nativeGrowCommandCount": native_grow_command_count,
                "nativeGrowSelectors": native_grow_selectors,
            },
            "compatibilityFactor": {
                "whenRawAdjustIsZero": 1,
                "otherwise": "1 - (1 - compatibilityByHand) * rawCompatibilityAdjust",
                "matrixSelection": "compatibilityKey row, skillId column 8 + (skillId - 1), divided by 100 and capped at 1",
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
                "profiles for unrecognized or missing Lua class paths",
                "actor-dependent MP getter and HP/MP/TP cost wrappers",
                "actor/target-dependent parameter evaluation",
                "complete-context parameter calls with non-live targets",
            ],
        },
        "observedCommandSlotContext": observed_command_slot_context,
        "matches": matches,
    }))
}

fn command_document(
    record: &csv::StringRecord,
    id: u32,
    compatibility: &[i8],
) -> Result<Value, String> {
    let class_path = optional_field(record, "lua_class_path");
    let parameters: Vec<Value> = (1..=4)
        .map(|number| {
            let base = field(record, &format!("p{number}_base"));
            let grow = field(record, &format!("p{number}_grow"));
            Ok(json!({
                "number": number,
                "base": scalar(base),
                "grow": scalar(grow),
                "compatibilityAdjust": scalar(field(record, &format!("p{number}_compat_adjust"))),
                "tpAdjust": scalar(field(record, &format!("p{number}_tp_adjust"))),
                "levelAdjustment": parameter_growth(base, grow)?,
            }))
        })
        .collect::<Result<_, String>>()?;

    Ok(json!({
        "identity": {
            "id": id,
            "idBand": field(record, "id_band"),
            "nameEnglish": field(record, "name_en"),
            "nameJapanese": field(record, "name_jp"),
            "luaClassPath": if class_path.is_empty() { Value::Null } else { json!(class_path) },
        },
        "levelAdjustmentProfile": level_adjustment_profile(class_path),
        "parameterProfile": parameter_profile(class_path),
        "compatibilityProfile": compatibility_profile(
            id,
            class_path,
            field(record, "compat_key"),
            field(record, "class_job"),
            compatibility,
        )?,
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
            "scope": "catalog-inputs",
            "hp": scalar(field(record, "hp_cost")),
            "mp": scalar(field(record, "mp_cost")),
            "tp": scalar(field(record, "tp_cost")),
            "actionGauge": scalar(field(record, "action_gauge")),
        },
        "costProfile": cost_profile(class_path, id),
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

// Matrix selection and actor-dependent shortcuts: docs/command-compatibility-profiles.md.
fn compatibility_profile(
    command_id: u32,
    class_path: &str,
    key: &str,
    command_main_skill: &str,
    values: &[i8],
) -> Result<Value, String> {
    let Some(parents) = known_class_parents(class_path) else {
        return Ok(json!({
            "status": "unresolved",
            "reason": if class_path.is_empty() { "missing-class-path" } else { "unrecognized-class-path" },
        }));
    };
    if !parents.contains(&"GameCommandBaseClass") {
        return Ok(json!({
            "status": "not-applicable",
            "reason": "outside-game-command-hierarchy",
        }));
    }
    if values.is_empty() {
        return Ok(json!({
            "status": "unresolved",
            "reason": "missing-compatibility-data",
            "definedBy": "GameCommandBaseClass",
        }));
    }
    let key = key
        .parse::<u32>()
        .map_err(|_| "compatibility key is not an unsigned integer".to_owned())?;
    let command_main_skill = command_main_skill
        .parse::<u32>()
        .map_err(|_| "command main skill is not an unsigned integer".to_owned())?;
    let skill_values: Vec<Value> = values
        .iter()
        .enumerate()
        .map(|(index, percent)| {
            let matrix_factor = f64::from(*percent) / 100.0;
            json!({
                "skillId": index + 1,
                "percent": percent,
                "matrixFactor": matrix_factor,
                "cappedFactor": matrix_factor.min(1.0),
            })
        })
        .collect();
    Ok(json!({
        "status": "resolved",
        "scope": "lua-compatibility-input-selection",
        "definedBy": "GameCommandBaseClass",
        "matrix": {
            "key": key,
            "inputField": "compatibility_percent_by_skill",
            "skillValues": skill_values,
        },
        "commandMainSkill": {
            "inputField": "class_job",
            "skillId": command_main_skill,
        },
        "skillSelection": {
            "handInput": "parameter-getter-argument-2",
            "handEquals2": "actor-sub-skill",
            "otherwise": "actor-main-skill",
        },
        "commandHandContext": {
            "consumer": "GameCommandBaseClass.processCanFire-argument-4",
            "relationshipToParameterGetter": "unresolved-no-direct-lua-call-edge",
            "explicitValue": "caller-supplied",
            "knownExplicitSources": [
                "actor.getReadyCommand(...)-second-return",
                "actor.getCustomCommand(...)-second-return",
            ],
            "defaultProducer": {
                "definedBy": "GameCommandBaseClass.judgeHand",
                "actionSlotEligible": command_uses_action_slot(command_id),
                "eligibilityDefinedBy": "GameCommandBaseClass.isEnableEquipForPlayerActionSlot",
                "eligibleLookup": {
                    "method": "actor.searchCommandSlot(commandId, nil)",
                    "customSlotRange": { "first": 1, "last": 30 },
                    "slotOffset": "actor.charaWork.commandBorder + customSlotIndex",
                    "commandSource": "actor.charaWork.command[slotOffset]",
                    "matchedValue": "actor.charaWork.commandCategory[slotOffset]",
                    "noMatch": "unavailable",
                },
                "actorWorkBindings": {
                    "identifierScope": "local-lua-work-binding-key",
                    "command": { "id": 3002, "path": "actor.charaWork.command" },
                    "commandCategory": { "id": 3003, "path": "actor.charaWork.commandCategory" },
                    "commandBorder": { "id": 3004, "path": "actor.charaWork.commandBorder" },
                    "valueProducer": "unresolved",
                    "matchingPropertyStreamObservations": {
                        "status": "observed-partial-promoted-hash-catalog-snapshot",
                        "sourceArtifact": "xivl-client-structs/manifests/gam_hash_names.json",
                        "direction": "server-to-client",
                        "opcode": "0x0137",
                        "propertyPathPattern": "charaWork.commandCategory[index]",
                        "propertyHash": "seed-0 backward MurmurHash2 over the canonical property path",
                        "valueWidthBytes": 1,
                        "observedIndices": [0, 1, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 51],
                        "observedValues": [1],
                        "observedOccurrenceCount": 220,
                        "captureCount": 8,
                        "scenarioCount": 4,
                        "category2Observation": "not-observed-in-promoted-snapshot",
                        "boundary": "this 220-record promoted snapshot is independent of the optional command-slot context input; observations do not establish the complete category domain, category assignment policy, or native binding-to-sync-cache bridge",
                    },
                },
                "ineligibleValue": 0,
            },
        },
        "jobSkillRule": {
            "definedBy": "CharaBaseClass.isJob",
            "skillIds": [15, 16, 17, 18, 19, 26, 27],
        },
        "shortcuts": [
            { "condition": "selected-skill-id-is-zero", "factor": 0 },
            { "condition": "selected-skill-matches-command-main-skill", "factor": 1 },
            { "condition": "actor-is-selected-job-and-actor-main-skill-matches-command-main-skill", "factor": 1 },
        ],
        "fallback": "capped-matrix-factor",
        "evaluation": {
            "status": "context-dependent",
            "requiredInputs": [
                {
                    "field": "handSelector",
                    "source": "parameter-getter-argument-2",
                },
                {
                    "field": "actorStateMainSkill",
                    "source": "actor.charaWork.parameterSave.state_mainSkill[1]",
                    "valueType": "integer8",
                },
                {
                    "field": "actorStateMainSkillForSub",
                    "source": "actor.charaWork.parameterSave.state_mainSkill[3]",
                    "valueType": "integer8",
                    "condition": "hand-selector-equals-2",
                },
            ],
        },
    }))
}

fn command_uses_action_slot(command_id: u32) -> bool {
    if (26_000..=29_999).contains(&command_id) {
        command_id != 29_497 && command_id != 29_501 && !(29_458..=29_464).contains(&command_id)
    } else if (22_100..=22_499).contains(&command_id) {
        !matches!(
            command_id,
            22_101
                | 22_102
                | 22_103
                | 22_105
                | 22_106
                | 22_107
                | 22_109
                | 22_110
                | 22_111
                | 22_112
                | 22_301
                | 22_304
                | 22_305
                | 22_306
        )
    } else {
        false
    }
}

// Inherited getter selection and call modes: docs/command-parameter-profiles.md.
fn parameter_profile(class_path: &str) -> Value {
    let Some(parents) = known_class_parents(class_path) else {
        return json!({
            "status": "unresolved",
            "reason": if class_path.is_empty() { "missing-class-path" } else { "unrecognized-class-path" },
        });
    };
    if !parents.contains(&"GameCommandBaseClass") {
        return json!({
            "status": "not-applicable",
            "reason": "outside-game-command-hierarchy",
        });
    }
    let getters: Vec<Value> = (1..=4)
        .map(|number| {
            json!({
                "number": number,
                "method": format!("getCommandParam{number}"),
                "inputField": format!("p{number}_base"),
                "growSelectorMethod": format!("getCommandParam{number}LevelAdjustGrow"),
            })
        })
        .collect();
    json!({
        "status": "resolved",
        "scope": "lua-parameter-getter-selection",
        "definedBy": "GameCommandBaseClass",
        "getters": getters,
        "argumentRoles": [
            { "position": 1, "role": "actor" },
            { "position": 2, "role": "hand-selector" },
            { "position": 3, "role": "target", "use": "liveness-and-grow-context" },
            { "position": 4, "role": "unused" },
        ],
        "callModes": {
            "missingContext": {
                "condition": "any-of-first-three-arguments-after-receiver-is-nil",
                "kind": "catalog-input",
            },
            "liveContext": {
                "condition": "first-three-arguments-present-and-third-argument-_isAlive-truthy",
                "kind": "actor-target-required",
            },
            "nonLiveContext": {
                "condition": "first-three-arguments-present-and-third-argument-_isAlive-falsy",
                "kind": "unresolved",
                "reason": "recovered-factors-uninitialized",
            },
        },
    })
}

// Getter selection and wrapper boundaries: docs/command-cost-profiles.md.
fn cost_profile(class_path: &str, id: u32) -> Value {
    let Some(parents) = known_class_parents(class_path) else {
        return json!({
            "status": "unresolved",
            "reason": if class_path.is_empty() { "missing-class-path" } else { "unrecognized-class-path" },
        });
    };
    if !parents.contains(&"GameCommandBaseClass") {
        return json!({
            "status": "not-applicable",
            "reason": "outside-game-command-hierarchy",
        });
    }
    let (hp_owner, parameter_id) = match class_path {
        "/Command/Game/Ability/CmnAbility" => ("CmnAbility", Some(27591)),
        "/Command/Game/Magic/CmnAttackMagic" => ("CmnAttackMagic", Some(28623)),
        "/Command/Game/Magic/CmnCureMagic" => ("CmnCureMagic", Some(28669)),
        _ => ("GameCommandBaseClass", None),
    };
    let hp_result = if parameter_id == Some(id) {
        json!({
            "kind": "catalog-input",
            "field": "p3_base",
            "via": "getCommandParam3",
            "callArguments": "receiver-only",
        })
    } else {
        json!({ "kind": "constant", "value": 0 })
    };
    json!({
        "status": "resolved",
        "scope": "lua-cost-getter-selection",
        "hp": {
            "method": "getCommandHPCost",
            "definedBy": hp_owner,
            "result": hp_result,
        },
        "mp": {
            "method": "getCommandMPCost",
            "definedBy": "GameCommandBaseClass",
            "result": {
                "kind": "actor-required",
                "field": "mp_cost",
                "actorMethod": "calculateCommandCost",
            },
        },
        "tp": {
            "method": "getCommandTPCost",
            "definedBy": "GameCommandBaseClass",
            "result": { "kind": "catalog-input", "field": "tp_cost" },
        },
        "wrappers": {
            "status": "runtime-required",
            "definedBy": "GameCommandBaseClass",
            "hp": { "method": "getCostHP", "actorMethods": ["getHP"] },
            "mp": { "method": "getCostMP", "actorMethods": ["getForceCostMPForCaster", "getMP"] },
            "tp": { "method": "getCostTP", "actorMethods": ["getTP", "getForceCostTPForCaster"] },
        },
    })
}

// Exact class paths and inherited getter facts: docs/command-formula-profiles.md.
fn level_adjustment_profile(class_path: &str) -> Value {
    let Some(parents) = known_class_parents(class_path) else {
        return json!({
            "status": "unresolved",
            "reason": if class_path.is_empty() { "missing-class-path" } else { "unrecognized-class-path" },
        });
    };
    let class = class_path.rsplit('/').next().expect("known class path");
    let mut inheritance = vec![class];
    inheritance.extend_from_slice(parents);
    if !parents.contains(&"GameCommandBaseClass") {
        return json!({
            "status": "not-applicable",
            "reason": "outside-game-command-hierarchy",
            "inheritance": inheritance,
        });
    }
    let (high_cap, limits_owner) = match class_path {
        "/Command/Game/AttackCommand"
        | "/Command/Game/Basic/MonsterAttackCommand"
        | "/Command/Game/ShotCommand"
        | "/Command/Game/ThrowCommand" => (-1, class),
        "/Command/Game/Magic/AncientMagic"
        | "/Command/Game/Magic/CmnAttackMagic"
        | "/Command/Game/Magic/CmnDrainMagic" => (10, class),
        _ => (15, "GameCommandBaseClass"),
    };
    let ancient = class_path == "/Command/Game/Magic/AncientMagic";
    let (high_blends, high_override_count) = match class_path {
        "/Command/Game/Magic/AncientMagic" => ([0.0; 4], 4),
        "/Command/Game/Magic/CmnAttackMagic" => ([0.25, 0.0, 0.0, 0.7], 3),
        "/Command/Game/Magic/CmnBadStatusMagic"
        | "/Command/Game/Magic/CmnCureMagic"
        | "/Command/Game/Magic/CmnDrainMagic"
        | "/Command/Game/Magic/CmnGoodStatusMagic" => ([0.0, 0.0, 0.0, 0.7], 3),
        _ => ([0.7; 4], 0),
    };
    let blends: Vec<Value> = high_blends.iter().enumerate().map(|(index, high)| json!({
        "number": index + 1,
        "lowLevelBlend": if ancient { 0 } else { 1 },
        "highLevelBlend": high,
        "lowLevelDefinedBy": if ancient { class } else { "GameCommandBaseClass" },
        "highLevelDefinedBy": if index < high_override_count { class } else { "GameCommandBaseClass" },
    })).collect();
    json!({
        "status": "resolved",
        "scope": "level-limit-and-parameter-blend-getters",
        "inheritance": inheritance,
        "lowLevelDistanceLimit": -1,
        "highLevelDistanceLimit": high_cap,
        "levelLimitsDefinedBy": limits_owner,
        "parameterBlends": blends,
    })
}

// Declared parent chains for the exact paths in docs/command-profile-sources.md.
fn known_class_parents(class_path: &str) -> Option<&'static [&'static str]> {
    match class_path {
        "/Command/Game/Ability/Ability"
        | "/Command/Game/Ability/AttackAbility"
        | "/Command/Game/Ability/CmnAbility"
        | "/Command/Game/Ability/CmnCrafterAbility"
        | "/Command/Game/Ability/GathererStealthAbility"
        | "/Command/Game/Ability/MonsterAbility"
        | "/Command/Game/Ability/MonsterSubStatAbility"
        | "/Command/Game/Ability/PointSearchAbility" => Some(&[
            "AbilityBaseClass",
            "BattleCommandBaseClass",
            "GameCommandBaseClass",
        ]),
        "/Command/Game/ArrowReloadCommand"
        | "/Command/Game/ArrowStockCommand"
        | "/Command/Game/AttackCommand"
        | "/Command/Game/Basic/GarudaOthers"
        | "/Command/Game/Basic/MonsterAttackCommand"
        | "/Command/Game/Basic/MonsterOthers"
        | "/Command/Game/Basic/MonsterRangeAttack"
        | "/Command/Game/Basic/MonsterShieldCommand"
        | "/Command/Game/Basic/MonsterSubStatOthers"
        | "/Command/Game/ShieldDefenceCommand"
        | "/Command/Game/ShotCommand"
        | "/Command/Game/ThrowCommand" => Some(&["BattleCommandBaseClass", "GameCommandBaseClass"]),
        "/Command/AutoAttackTargetChangeCommand"
        | "/Command/DebugInputCommand"
        | "/Command/ItemCommand" => Some(&["CommandBaseClass"]),
        "/Command/Game/Constance/CmnConstance" => Some(&[
            "ConstanceBaseClass",
            "BattleCommandBaseClass",
            "GameCommandBaseClass",
        ]),
        "/Command/ChangeJobCommand"
        | "/Command/EquipAbilityCommand"
        | "/Command/EquipCommand"
        | "/Command/Game/AcnItemCreateCommand"
        | "/Command/Game/AcnItemPutCommand"
        | "/Command/Game/ActivateCommand"
        | "/Command/Game/BewareCommand"
        | "/Command/Game/BoostPointCommand"
        | "/Command/Game/ChangeEquipSetCommand"
        | "/Command/Game/CombinationManagementCommand"
        | "/Command/Game/CombinationStartCommand"
        | "/Command/Game/CommandCancelCommand"
        | "/Command/Game/CraftCommand"
        | "/Command/Game/DummyCommand"
        | "/Command/Game/HealingCommand"
        | "/Command/Game/HighsenseCommand"
        | "/Command/Game/NegotiationCommand"
        | "/Command/Game/PartyTargetCommand"
        | "/Command/Game/Prog/EquipPartsShowHideCommand"
        | "/Command/Game/ResetOccupiedCommand"
        | "/Command/Game/ShieldEffectCommand"
        | "/Command/Game/WeaponSkill/MonsterTest"
        | "/Command/System/ReserveInputOperationCommand" => Some(&["GameCommandBaseClass"]),
        "/Command/Game/Magic/AncientMagic"
        | "/Command/Game/Magic/AttackMagic"
        | "/Command/Game/Magic/CmnAbsorptionMagic"
        | "/Command/Game/Magic/CmnAttackMagic"
        | "/Command/Game/Magic/CmnBadStatusMagic"
        | "/Command/Game/Magic/CmnCureMagic"
        | "/Command/Game/Magic/CmnDrainMagic"
        | "/Command/Game/Magic/CmnGoodStatusMagic"
        | "/Command/Game/Magic/CmnRemoveStatusMagic"
        | "/Command/Game/Magic/CureMagic"
        | "/Command/Game/Magic/CuregaMagic"
        | "/Command/Game/Magic/EffectMagic"
        | "/Command/Game/Magic/EsunaMagic"
        | "/Command/Game/Magic/RaiseMagic"
        | "/Command/Game/Magic/SongMagic" => Some(&[
            "MagicBaseClass",
            "BattleCommandBaseClass",
            "GameCommandBaseClass",
        ]),
        "/Command/Game/Prog/ChocoboRideCommand" => {
            Some(&["ProgCommandBaseClass", "GameCommandBaseClass"])
        }
        "/Command/Game/BonusPointCommand" => Some(&["SystemCommandBaseClass", "CommandBaseClass"]),
        "/Command/Game/WeaponSkill/AttackWeaponSkill"
        | "/Command/Game/WeaponSkill/CmnAttackWeaponSkill"
        | "/Command/Game/WeaponSkill/DevideAttackWeaponSkill"
        | "/Command/Game/WeaponSkill/GarudaAttackWeaponSkill"
        | "/Command/Game/WeaponSkill/IfritAttackWeaponSkill"
        | "/Command/Game/WeaponSkill/IfritSubStatWeaponSkill"
        | "/Command/Game/WeaponSkill/MonsterAbsorbWeaponSkill"
        | "/Command/Game/WeaponSkill/MonsterAttackWeaponSkill"
        | "/Command/Game/WeaponSkill/MonsterSubStatWeaponSkill"
        | "/Command/Game/WeaponSkill/WhiteGeneralAttackWeaponSkill" => Some(&[
            "WeaponSkillBaseClass",
            "BattleCommandBaseClass",
            "GameCommandBaseClass",
        ]),
        _ => None,
    }
}

struct CommandGrowth {
    native_required: bool,
    selectors: BTreeSet<i64>,
}

fn command_growth(record: &csv::StringRecord, row: usize) -> Result<CommandGrowth, String> {
    let mut native_required = false;
    let mut selectors = BTreeSet::new();
    for number in 1..=4 {
        let raw = field(record, &format!("p{number}_grow"));
        if raw.is_empty() {
            continue;
        }
        let selector = raw.parse::<i64>().map_err(|_| {
            format!("catalog row {row} has an invalid parameter {number} grow selector")
        })?;
        if selector >= 0 {
            native_required = true;
            selectors.insert(selector);
        }
    }
    Ok(CommandGrowth {
        native_required,
        selectors,
    })
}

fn parameter_growth(base: &str, grow: &str) -> Result<Value, String> {
    if base.is_empty() {
        return Ok(json!({ "status": "absent" }));
    }
    if grow.is_empty() {
        return Ok(json!({ "status": "flat", "factor": 1 }));
    }
    let selector = grow
        .parse::<i64>()
        .map_err(|_| "parameter grow selector is not an integer".to_owned())?;
    if selector < 0 {
        Ok(json!({ "status": "flat", "factor": 1 }))
    } else {
        Ok(json!({
            "status": "native-grow-required",
            "selector": selector,
        }))
    }
}

fn field<'a>(record: &'a csv::StringRecord, name: &str) -> &'a str {
    let index = HEADER
        .iter()
        .position(|candidate| *candidate == name)
        .expect("internal catalog field name");
    record.get(index).expect("validated catalog row width")
}

fn optional_field<'a>(record: &'a csv::StringRecord, name: &str) -> &'a str {
    let index = HEADER
        .iter()
        .position(|candidate| *candidate == name)
        .expect("internal catalog field name");
    record.get(index).unwrap_or("")
}

fn parse_compatibility_values(raw: &str) -> Result<Vec<i8>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let mut values = Vec::with_capacity(44);
    for (index, component) in raw.split(';').enumerate() {
        let expected_id = index + 1;
        let Some((skill_id, percent)) = component.split_once('=') else {
            return Err("compatibility values contain a field without '='".to_owned());
        };
        if skill_id.parse::<usize>().ok() != Some(expected_id) {
            return Err(format!(
                "compatibility values expected skill id {expected_id}"
            ));
        }
        let percent = percent
            .parse::<i8>()
            .map_err(|_| format!("compatibility value for skill id {expected_id} is not s8"))?;
        values.push(percent);
    }
    if values.len() != 44 {
        return Err(format!(
            "compatibility values contain {} skills; expected 44",
            values.len()
        ));
    }
    Ok(values)
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

    type InvalidSlotContextCase = (&'static str, fn(&mut Value));

    fn catalog(rows: &[(&str, &str, &str)]) -> Vec<u8> {
        catalog_with_class(rows, "")
    }

    fn catalog_with_class(rows: &[(&str, &str, &str)], class_path: &str) -> Vec<u8> {
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
            row[index("lua_class_path")] = class_path.to_owned();
            writer.write_record(row).unwrap();
        }
        writer.into_inner().unwrap()
    }

    fn index(name: &str) -> usize {
        HEADER.iter().position(|field| *field == name).unwrap()
    }

    fn compatibility_values(value: i8) -> String {
        (1..=44)
            .map(|skill_id| format!("{skill_id}={value}"))
            .collect::<Vec<_>>()
            .join(";")
    }

    fn slot_context_fixture() -> SlotContextManifest {
        let mut fixture = SlotContextManifest {
            schema_version: 1,
            kind: "xivl-command-slot-context".to_owned(),
            game_version: "1.23b".to_owned(),
            status: "qualified-static-actor-identity-partial-category-observation".to_owned(),
            source_snapshots: SourceSnapshots {
                captures: CaptureSnapshot {
                    repository: "XIVLegacy/xivl-captures".to_owned(),
                    commit: "capture-commit".to_owned(),
                    artifact: "records.csv".to_owned(),
                    sha256: "capture-sha".to_owned(),
                },
                client_structs: ClientStructSnapshot {
                    repository: "XIVLegacy/xivl-client-structs".to_owned(),
                    generator_artifact: "generator.py".to_owned(),
                    generator_sha256: "generator-sha".to_owned(),
                    hash_names_artifact: "hash-names.json".to_owned(),
                    hash_names_sha256: "hash-names-sha".to_owned(),
                    actor_identity_artifact: "identity.json#relationship".to_owned(),
                    actor_identity_sha256: "identity-sha".to_owned(),
                },
                client_data: ClientDataSnapshot {
                    repository: "XIVLegacy/xivl-client-data".to_owned(),
                    commit: "data-commit".to_owned(),
                    static_actor_artifact: "actors.json".to_owned(),
                    static_actor_sha256: "actors-sha".to_owned(),
                    command_catalog_artifact: "commands.csv".to_owned(),
                    command_catalog_sha256: "commands-sha".to_owned(),
                },
            },
            derivation: Derivation {
                carrier: "s2c:0x0137".to_owned(),
                state_partition: vec![
                    "capture".to_owned(),
                    "lane_index".to_owned(),
                    "source_actor_id".to_owned(),
                ],
                state_order: "increasing record_index".to_owned(),
                state_rule: "apply writes in order".to_owned(),
                static_actor_test: "prefix".to_owned(),
                command_id_decode: "low16".to_owned(),
                identity_boundary: "qualified join".to_owned(),
            },
            coverage: Coverage {
                command_records: 8,
                nonzero_command_occurrences: 8,
                unique_nonzero_command_actors: 1,
                static_actor_prefix_hits: 1,
                static_actor_catalog_hits: 1,
                command_catalog_hits: 1,
                category_records: 7,
                category_hashes: 1,
                category_value_distribution: vec![CategoryValueDistribution {
                    value: 1,
                    occurrences: 7,
                }],
                stateful_category_observations: 6,
                commands_with_category_observations: 1,
                category_writes_without_current_command: vec![CategoryWriteSummary {
                    slot: 51,
                    occurrences: 1,
                }],
            },
            rows_sha256: String::new(),
            rows: vec![SlotContextRow {
                actor_id_hex: "0xa0f06a04".to_owned(),
                command_id: 27140,
                class_path: "/Command/Game/Ability/Ability".to_owned(),
                name_english: "Sentinel".to_owned(),
                command_occurrences: 8,
                slot_observations: vec![
                    SlotObservation {
                        slot: 39,
                        command_occurrences: 7,
                        category_observations: vec![CategoryObservation {
                            value: 1,
                            occurrences: 6,
                        }],
                    },
                    SlotObservation {
                        slot: 43,
                        command_occurrences: 1,
                        category_observations: Vec::new(),
                    },
                ],
            }],
            unresolved: vec!["category 2 is not observed".to_owned()],
        };
        fixture.rows_sha256 =
            sha256(&serde_json::to_vec(&serde_json::to_value(&fixture.rows).unwrap()).unwrap());
        fixture
    }

    fn parsed_slot_context_fixture() -> SlotContextManifest {
        let fixture = slot_context_fixture();
        parse_slot_context(&serde_json::to_vec(&fixture).unwrap()).unwrap()
    }

    fn refresh_rows_sha256(value: &mut Value) {
        value["rowsSha256"] = json!(sha256(&serde_json::to_vec(&value["rows"]).unwrap()));
    }

    #[test]
    fn joins_compatibility_matrix_without_actor_inference() {
        let mut row = vec![String::new(); HEADER.len()];
        row[index("id")] = "27346".to_owned();
        row[index("compat_key")] = "3".to_owned();
        row[index("class_job")] = "23".to_owned();
        row[index("lua_class_path")] = "/Command/Game/Magic/CmnAttackMagic".to_owned();
        row[index("compatibility_percent_by_skill")] = (1..=44)
            .map(|skill_id| format!("{skill_id}={}", if skill_id == 23 { 120 } else { 45 }))
            .collect::<Vec<_>>()
            .join(";");
        let compatibility =
            parse_compatibility_values(row[index("compatibility_percent_by_skill")].as_str())
                .unwrap();
        let command =
            command_document(&csv::StringRecord::from(row), 27_346, &compatibility).unwrap();
        let profile = &command["compatibilityProfile"];
        assert_eq!(profile["status"], "resolved");
        assert_eq!(profile["definedBy"], "GameCommandBaseClass");
        assert_eq!(profile["matrix"]["key"], 3);
        assert_eq!(profile["matrix"]["skillValues"][0]["skillId"], 1);
        assert_eq!(profile["matrix"]["skillValues"][0]["percent"], 45);
        assert_eq!(profile["matrix"]["skillValues"][0]["matrixFactor"], 0.45);
        assert_eq!(profile["matrix"]["skillValues"][22]["percent"], 120);
        assert_eq!(profile["matrix"]["skillValues"][22]["matrixFactor"], 1.2);
        assert_eq!(profile["matrix"]["skillValues"][22]["cappedFactor"], 1.0);
        assert_eq!(profile["commandMainSkill"]["skillId"], 23);
        assert_eq!(
            profile["skillSelection"]["handInput"],
            "parameter-getter-argument-2"
        );
        assert_eq!(profile["skillSelection"]["handEquals2"], "actor-sub-skill");
        assert_eq!(
            profile["commandHandContext"]["defaultProducer"]["actionSlotEligible"],
            true
        );
        assert_eq!(
            profile["commandHandContext"]["defaultProducer"]["eligibleLookup"]["matchedValue"],
            "actor.charaWork.commandCategory[slotOffset]"
        );
        assert_eq!(
            profile["commandHandContext"]["relationshipToParameterGetter"],
            "unresolved-no-direct-lua-call-edge"
        );
        assert_eq!(
            profile["commandHandContext"]["defaultProducer"]["actorWorkBindings"]
                ["commandCategory"]["id"],
            3003
        );
        assert_eq!(
            profile["commandHandContext"]["defaultProducer"]["actorWorkBindings"]
                ["identifierScope"],
            "local-lua-work-binding-key"
        );
        assert_eq!(
            profile["commandHandContext"]["defaultProducer"]["actorWorkBindings"]["valueProducer"],
            "unresolved"
        );
        let observations = &profile["commandHandContext"]["defaultProducer"]["actorWorkBindings"]
            ["matchingPropertyStreamObservations"];
        assert_eq!(
            observations,
            &json!({
                "status": "observed-partial-promoted-hash-catalog-snapshot",
                "sourceArtifact": "xivl-client-structs/manifests/gam_hash_names.json",
                "direction": "server-to-client",
                "opcode": "0x0137",
                "propertyPathPattern": "charaWork.commandCategory[index]",
                "propertyHash": "seed-0 backward MurmurHash2 over the canonical property path",
                "valueWidthBytes": 1,
                "observedIndices": [0, 1, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 51],
                "observedValues": [1],
                "observedOccurrenceCount": 220,
                "captureCount": 8,
                "scenarioCount": 4,
                "category2Observation": "not-observed-in-promoted-snapshot",
                "boundary": "this 220-record promoted snapshot is independent of the optional command-slot context input; observations do not establish the complete category domain, category assignment policy, or native binding-to-sync-cache bridge",
            })
        );
        assert_eq!(
            profile["jobSkillRule"]["skillIds"],
            json!([15, 16, 17, 18, 19, 26, 27])
        );
        assert_eq!(profile["fallback"], "capped-matrix-factor");
        assert_eq!(profile["evaluation"]["status"], "context-dependent");
        assert_eq!(
            profile["evaluation"]["requiredInputs"],
            json!([
                {
                    "field": "handSelector",
                    "source": "parameter-getter-argument-2",
                },
                {
                    "field": "actorStateMainSkill",
                    "source": "actor.charaWork.parameterSave.state_mainSkill[1]",
                    "valueType": "integer8",
                },
                {
                    "field": "actorStateMainSkillForSub",
                    "source": "actor.charaWork.parameterSave.state_mainSkill[3]",
                    "valueType": "integer8",
                    "condition": "hand-selector-equals-2",
                },
            ])
        );
    }

    #[test]
    fn derives_action_slot_eligibility_from_command_id() {
        for id in [22_100, 22_302, 22_499, 26_000, 29_457, 29_465, 29_999] {
            assert!(
                command_uses_action_slot(id),
                "expected {id} to use an action slot"
            );
        }
        for id in [
            22_099, 22_101, 22_112, 22_301, 22_304, 22_306, 22_500, 25_999, 29_458, 29_464, 29_497,
            29_501, 30_000,
        ] {
            assert!(
                !command_uses_action_slot(id),
                "expected {id} not to use an action slot"
            );
        }
    }

    #[test]
    fn validates_compatibility_shape_and_identity_boundaries() {
        assert_eq!(parse_compatibility_values("").unwrap(), Vec::<i8>::new());
        assert_eq!(
            parse_compatibility_values(&compatibility_values(-8))
                .unwrap()
                .len(),
            44
        );
        for (raw, message) in [
            ("1=10;2", "without '='"),
            ("2=10", "expected skill id 1"),
            ("1=200", "is not s8"),
            ("1=10", "expected 44"),
        ] {
            assert!(parse_compatibility_values(raw)
                .unwrap_err()
                .contains(message));
        }
        for path in ["", "/Unknown/CmnAbility"] {
            let profile = compatibility_profile(27_346, path, "3", "23", &[]).unwrap();
            assert_eq!(profile["status"], "unresolved");
        }
        let values = parse_compatibility_values(&compatibility_values(100)).unwrap();
        for path in [
            "/Command/AutoAttackTargetChangeCommand",
            "/Command/DebugInputCommand",
            "/Command/Game/BonusPointCommand",
            "/Command/ItemCommand",
        ] {
            let profile = compatibility_profile(27_346, path, "3", "23", &values).unwrap();
            assert_eq!(profile["status"], "not-applicable");
            assert!(profile.get("matrix").is_none());
        }
        let profile =
            compatibility_profile(27_346, "/Command/ChangeJobCommand", "3", "23", &[]).unwrap();
        assert_eq!(profile["reason"], "missing-compatibility-data");

        assert!(
            compatibility_profile(27_346, "/Command/ChangeJobCommand", "3", "bad", &values,)
                .unwrap_err()
                .contains("command main skill")
        );

        for (key, values, message) in [
            (
                "bad",
                compatibility_values(100),
                "invalid compatibility key",
            ),
            ("3", String::new(), "exactly when its key is present"),
            (
                "",
                compatibility_values(100),
                "exactly when its key is present",
            ),
        ] {
            let mut row = vec![String::new(); HEADER.len()];
            row[index("id")] = "42".to_owned();
            row[index("compat_key")] = key.to_owned();
            row[index("compatibility_percent_by_skill")] = values;
            let mut writer = csv::Writer::from_writer(Vec::new());
            writer.write_record(HEADER).unwrap();
            writer.write_record(row).unwrap();
            assert!(build_report(&writer.into_inner().unwrap(), "42")
                .unwrap_err()
                .contains(message));
        }
    }

    #[test]
    fn distinguishes_parameter_call_modes_from_growth_coverage() {
        for path in [
            "/Command/Game/Ability/CmnAbility",
            "/Command/Game/Magic/AncientMagic",
            "/Command/Game/Magic/CmnAttackMagic",
            "/Command/ChangeJobCommand",
        ] {
            let data = catalog_with_class(&[("42", "Synthetic", "Example")], path);
            let report = build_report(&data, "42").unwrap();
            let command = &report["matches"][0];
            let profile = &command["parameterProfile"];
            assert_eq!(profile["status"], "resolved");
            assert_eq!(profile["definedBy"], "GameCommandBaseClass");
            for number in 1..=4 {
                let getter = &profile["getters"][number - 1];
                assert_eq!(getter["number"], number);
                assert_eq!(getter["method"], format!("getCommandParam{number}"));
                assert_eq!(getter["inputField"], format!("p{number}_base"));
                assert_eq!(
                    getter["growSelectorMethod"],
                    format!("getCommandParam{number}LevelAdjustGrow")
                );
            }
            assert_eq!(
                command["parameters"][2]["levelAdjustment"]["status"],
                "flat"
            );
            assert_eq!(
                profile["callModes"]["missingContext"]["kind"],
                "catalog-input"
            );
            assert_eq!(profile["argumentRoles"][0]["role"], "actor");
            assert_eq!(profile["argumentRoles"][1]["role"], "hand-selector");
            assert_eq!(profile["argumentRoles"][2]["role"], "target");
            assert_eq!(profile["argumentRoles"][3]["role"], "unused");
            assert_eq!(
                profile["callModes"]["liveContext"]["kind"],
                "actor-target-required"
            );
            assert_eq!(profile["callModes"]["nonLiveContext"]["kind"], "unresolved");
            assert_eq!(
                profile["callModes"]["nonLiveContext"]["reason"],
                "recovered-factors-uninitialized"
            );
            assert_eq!(
                report["formulaModel"]["parameterExpressionScope"],
                "complete-context-with-live-target"
            );
        }
        for path in [
            "",
            "/Unknown/CmnAbility",
            "/command/game/magic/ancientmagic",
        ] {
            let profile = parameter_profile(path);
            assert_eq!(profile["status"], "unresolved");
            assert!(profile.get("getters").is_none());
        }
        for path in [
            "/Command/AutoAttackTargetChangeCommand",
            "/Command/DebugInputCommand",
            "/Command/Game/BonusPointCommand",
            "/Command/ItemCommand",
        ] {
            let profile = parameter_profile(path);
            assert_eq!(profile["status"], "not-applicable");
            assert!(profile.get("callModes").is_none());
        }
    }

    #[test]
    fn selects_hp_cost_by_exact_path_and_conditional_id() {
        for (path, id, owner) in [
            ("/Command/Game/Ability/CmnAbility", 27591, "CmnAbility"),
            (
                "/Command/Game/Magic/CmnAttackMagic",
                28623,
                "CmnAttackMagic",
            ),
            ("/Command/Game/Magic/CmnCureMagic", 28669, "CmnCureMagic"),
        ] {
            let profile = cost_profile(path, id);
            assert_eq!(profile["status"], "resolved");
            assert_eq!(profile["hp"]["definedBy"], owner);
            assert_eq!(
                profile["hp"]["result"],
                json!({
                    "kind": "catalog-input", "field": "p3_base",
                    "via": "getCommandParam3", "callArguments": "receiver-only",
                })
            );
            for other_id in [42, id - 1, id + 1] {
                let other = cost_profile(path, other_id);
                assert_eq!(other["hp"]["definedBy"], owner);
                assert_eq!(
                    other["hp"]["result"],
                    json!({"kind": "constant", "value": 0})
                );
            }
            assert_eq!(
                cost_profile("/Command/Game/Magic/AncientMagic", id)["hp"]["result"],
                json!({"kind": "constant", "value": 0})
            );
        }
    }

    #[test]
    fn keeps_catalog_costs_separate_from_getters_and_wrappers() {
        let mut row = vec![String::new(); HEADER.len()];
        for (field, value) in [
            ("id", "28623"),
            ("hp_cost", "777"),
            ("mp_cost", "23"),
            ("tp_cost", "31"),
            ("p3_base", "17"),
            ("p3_grow", "69"),
            ("lua_class_path", "/Command/Game/Magic/CmnAttackMagic"),
        ] {
            row[index(field)] = value.to_owned();
        }
        let command = command_document(&csv::StringRecord::from(row), 28623, &[]).unwrap();
        assert_eq!(command["costs"]["scope"], "catalog-inputs");
        assert_eq!(command["costs"]["hp"], 777);
        assert_eq!(command["costs"]["mp"], 23);
        assert_eq!(command["costs"]["tp"], 31);
        let profile = &command["costProfile"];
        assert_eq!(profile["hp"]["result"]["field"], "p3_base");
        assert!(profile["hp"]["result"].get("value").is_none());
        assert_eq!(
            command["parameters"][2]["levelAdjustment"]["status"],
            "native-grow-required"
        );
        assert_eq!(profile["mp"]["result"]["kind"], "actor-required");
        assert_eq!(profile["mp"]["result"]["field"], "mp_cost");
        assert_eq!(
            profile["mp"]["result"]["actorMethod"],
            "calculateCommandCost"
        );
        assert_eq!(
            profile["tp"]["result"],
            json!({"kind": "catalog-input", "field": "tp_cost"})
        );
        assert_eq!(profile["wrappers"]["status"], "runtime-required");
        assert_eq!(
            profile["wrappers"]["tp"]["actorMethods"],
            json!(["getTP", "getForceCostTPForCaster"])
        );
    }

    #[test]
    fn leaves_costs_unresolved_without_known_game_identity() {
        for path in [
            "",
            "/Unknown/CmnAbility",
            "/command/game/ability/cmnability",
        ] {
            let profile = cost_profile(path, 27591);
            assert_eq!(profile["status"], "unresolved");
            assert!(profile.get("hp").is_none());
        }
        for path in [
            "/Command/AutoAttackTargetChangeCommand",
            "/Command/DebugInputCommand",
            "/Command/Game/BonusPointCommand",
            "/Command/ItemCommand",
        ] {
            let profile = cost_profile(path, 27591);
            assert_eq!(profile["status"], "not-applicable");
            assert!(profile.get("hp").is_none());
            assert!(profile.get("wrappers").is_none());
        }
        assert_eq!(
            cost_profile("/Command/ChangeJobCommand", 27591)["hp"]["definedBy"],
            "GameCommandBaseClass"
        );
        let data = catalog(&[("27591", "Synthetic", "Example")]);
        let report = build_report(&data, "27591").unwrap();
        assert_eq!(
            report["matches"][0]["costProfile"]["reason"],
            "missing-class-path"
        );
    }

    #[test]
    fn selects_exact_subclass_getters_without_guessing_from_names() {
        for (path, cap, high) in [
            (
                "/Command/Game/Magic/CmnAttackMagic",
                10,
                [0.25, 0.0, 0.0, 0.7],
            ),
            (
                "/Command/Game/Magic/CmnBadStatusMagic",
                15,
                [0.0, 0.0, 0.0, 0.7],
            ),
            ("/Command/Game/Magic/CmnCureMagic", 15, [0.0, 0.0, 0.0, 0.7]),
            ("/Command/Game/Ability/CmnAbility", 15, [0.7; 4]),
            (
                "/Command/Game/WeaponSkill/MonsterAttackWeaponSkill",
                15,
                [0.7; 4],
            ),
        ] {
            let data = catalog_with_class(&[("42", "Synthetic", "Example")], path);
            let report = build_report(&data, "42").unwrap();
            let command = &report["matches"][0];
            assert_eq!(command["identity"]["luaClassPath"], path);
            let profile = &command["levelAdjustmentProfile"];
            assert_eq!(profile["status"], "resolved");
            assert_eq!(profile["lowLevelDistanceLimit"], -1);
            assert_eq!(profile["highLevelDistanceLimit"], cap);
            for (index, expected) in high.iter().enumerate() {
                assert_eq!(profile["parameterBlends"][index]["lowLevelBlend"], 1);
                assert_eq!(
                    profile["parameterBlends"][index]["highLevelBlend"],
                    *expected
                );
            }
        }
        let unknown = catalog_with_class(
            &[("42", "CmnAttackMagic", "Example")],
            "/Command/Game/Magic/Unknown",
        );
        let report = build_report(&unknown, "42").unwrap();
        assert_eq!(
            report["matches"][0]["levelAdjustmentProfile"]["status"],
            "unresolved"
        );
        let data = catalog(&[("42", "Synthetic", "Example")]);
        let mut reader = csv::Reader::from_reader(data.as_slice());
        let mut legacy = csv::Writer::from_writer(Vec::new());
        legacy.write_record(&HEADER[..HEADER.len() - 2]).unwrap();
        for row in reader.records() {
            legacy
                .write_record(row.unwrap().iter().take(HEADER.len() - 2))
                .unwrap();
        }
        let report = build_report(&legacy.into_inner().unwrap(), "42").unwrap();
        assert!(report["matches"][0]["identity"]["luaClassPath"].is_null());
        assert_eq!(
            report["matches"][0]["parameterProfile"]["reason"],
            "missing-class-path"
        );
        assert_eq!(
            report["matches"][0]["levelAdjustmentProfile"]["reason"],
            "missing-class-path"
        );
        let data = catalog_with_class(
            &[("42", "Synthetic", "Example")],
            "/Command/Game/Magic/CmnAttackMagic",
        );
        let mut reader = csv::Reader::from_reader(data.as_slice());
        let mut v2 = csv::Writer::from_writer(Vec::new());
        v2.write_record(&HEADER[..HEADER.len() - 1]).unwrap();
        for row in reader.records() {
            v2.write_record(row.unwrap().iter().take(HEADER.len() - 1))
                .unwrap();
        }
        let report = build_report(&v2.into_inner().unwrap(), "42").unwrap();
        assert_eq!(
            report["matches"][0]["compatibilityProfile"]["reason"],
            "missing-compatibility-data"
        );
    }

    #[test]
    fn resolves_remaining_level_overrides_and_preserves_getter_owners() {
        let ancient = level_adjustment_profile("/Command/Game/Magic/AncientMagic");
        assert_eq!(ancient["highLevelDistanceLimit"], 10);
        assert_eq!(ancient["levelLimitsDefinedBy"], "AncientMagic");
        for blend in ancient["parameterBlends"].as_array().unwrap() {
            assert_eq!(blend["lowLevelBlend"], 0);
            assert_eq!(blend["highLevelBlend"], 0.0);
            assert_eq!(blend["lowLevelDefinedBy"], "AncientMagic");
            assert_eq!(blend["highLevelDefinedBy"], "AncientMagic");
        }
        for (path, high_limit) in [
            ("/Command/Game/Magic/CmnDrainMagic", 10),
            ("/Command/Game/Magic/CmnGoodStatusMagic", 15),
        ] {
            let profile = level_adjustment_profile(path);
            assert_eq!(profile["highLevelDistanceLimit"], high_limit);
            for index in 0..3 {
                assert_eq!(profile["parameterBlends"][index]["highLevelBlend"], 0.0);
                assert_eq!(
                    profile["parameterBlends"][index]["highLevelDefinedBy"],
                    path.rsplit('/').next().unwrap()
                );
            }
            assert_eq!(profile["parameterBlends"][3]["highLevelBlend"], 0.7);
            assert_eq!(
                profile["parameterBlends"][3]["highLevelDefinedBy"],
                "GameCommandBaseClass"
            );
            assert_eq!(
                profile["parameterBlends"][3]["lowLevelDefinedBy"],
                "GameCommandBaseClass"
            );
        }
        for path in [
            "/Command/Game/AttackCommand",
            "/Command/Game/Basic/MonsterAttackCommand",
            "/Command/Game/ShotCommand",
            "/Command/Game/ThrowCommand",
        ] {
            let profile = level_adjustment_profile(path);
            assert_eq!(profile["lowLevelDistanceLimit"], -1);
            assert_eq!(profile["highLevelDistanceLimit"], -1);
            assert_eq!(
                profile["levelLimitsDefinedBy"],
                path.rsplit('/').next().unwrap()
            );
            assert_eq!(profile["parameterBlends"][0]["highLevelBlend"], 0.7);
        }
    }

    #[test]
    fn uses_declared_hierarchy_instead_of_directory_or_class_name() {
        for (path, parents) in [
            ("/Command/ChangeJobCommand", vec!["GameCommandBaseClass"]),
            (
                "/Command/System/ReserveInputOperationCommand",
                vec!["GameCommandBaseClass"],
            ),
            (
                "/Command/Game/Prog/EquipPartsShowHideCommand",
                vec!["GameCommandBaseClass"],
            ),
            (
                "/Command/Game/Prog/ChocoboRideCommand",
                vec!["ProgCommandBaseClass", "GameCommandBaseClass"],
            ),
            (
                "/Command/Game/Constance/CmnConstance",
                vec![
                    "ConstanceBaseClass",
                    "BattleCommandBaseClass",
                    "GameCommandBaseClass",
                ],
            ),
            (
                "/Command/Game/WeaponSkill/MonsterTest",
                vec!["GameCommandBaseClass"],
            ),
        ] {
            let profile = level_adjustment_profile(path);
            assert_eq!(profile["status"], "resolved");
            assert_eq!(profile["highLevelDistanceLimit"], 15);
            let mut expected = vec![path.rsplit('/').next().unwrap()];
            expected.extend(parents);
            assert_eq!(profile["inheritance"], json!(expected));
        }
        for path in [
            "/Command/AutoAttackTargetChangeCommand",
            "/Command/DebugInputCommand",
            "/Command/Game/BonusPointCommand",
            "/Command/ItemCommand",
        ] {
            let data = catalog_with_class(&[("42", "Synthetic", "Example")], path);
            let report = build_report(&data, "42").unwrap();
            let profile = &report["matches"][0]["levelAdjustmentProfile"];
            assert_eq!(profile["status"], "not-applicable");
            assert_eq!(profile["reason"], "outside-game-command-hierarchy");
            assert!(profile.get("highLevelDistanceLimit").is_none());
            assert!(profile.get("parameterBlends").is_none());
        }
        assert_eq!(
            level_adjustment_profile("/Command/EquipPartsShowHideCommand")["status"],
            "unresolved"
        );
        assert_eq!(
            level_adjustment_profile("/command/game/magic/ancientmagic")["status"],
            "unresolved"
        );
    }

    #[test]
    fn queries_id_and_duplicate_exact_names() {
        let data = catalog(&[
            ("27310", "Fire", "Fire JP"),
            ("27410", "Fire", "Fire II JP"),
        ]);
        let by_id = build_report(&data, "27310").unwrap();
        assert_eq!(by_id["schemaVersion"], 11);
        assert_eq!(by_id["query"]["mode"], "id");
        assert_eq!(by_id["matches"].as_array().unwrap().len(), 1);
        assert_eq!(by_id["matches"][0]["damage"]["magnitude"], 950);
        assert_eq!(by_id["matches"][0]["rawEffectFields"]["108"], 13);
        assert_eq!(
            by_id["matches"][0]["parameters"][2]["levelAdjustment"]["status"],
            "flat"
        );
        assert_eq!(
            by_id["formulaModel"]["growthCoverage"]["flatCommandCount"],
            2
        );
        assert_eq!(
            by_id["formulaModel"]["growthCoverage"]["nativeGrowCommandCount"],
            0
        );
        assert_eq!(by_id["formulaModel"]["levelAdjustment"]["lowLevelBlend"], 1);
        assert_eq!(
            by_id["formulaModel"]["levelAdjustment"]["scope"],
            "GameCommandBaseClass defaults"
        );
        assert_eq!(
            by_id["formulaModel"]["levelAdjustment"]["highLevelBlend"],
            0.7
        );

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

    #[test]
    fn reports_native_growth_coverage_and_parameter_status() {
        let mut data = catalog(&[("28602", "Bio", "Bio JP")]);
        let text = String::from_utf8(data)
            .unwrap()
            .replace(",13,-1,1,0,", ",13,69,1,0,");
        data = text.into_bytes();

        let report = build_report(&data, "28602").unwrap();
        assert_eq!(
            report["formulaModel"]["growthCoverage"]["nativeGrowCommandCount"],
            1
        );
        assert_eq!(
            report["formulaModel"]["growthCoverage"]["nativeGrowSelectors"],
            json!([69])
        );
        assert_eq!(
            report["matches"][0]["parameters"][2]["levelAdjustment"],
            json!({ "status": "native-grow-required", "selector": 69 })
        );
        assert_eq!(
            report["matches"][0]["parameters"][0]["levelAdjustment"]["status"],
            "absent"
        );

        let invalid = String::from_utf8(data)
            .unwrap()
            .replace(",13,69,1,0,", ",13,unknown,1,0,");
        assert!(build_report(invalid.as_bytes(), "28602")
            .unwrap_err()
            .contains("invalid parameter 3 grow selector"));
    }

    #[test]
    fn preserves_slot_context_in_json_and_yaml_reports() {
        let data = catalog_with_class(
            &[("27140", "Sentinel", "Sentinelle")],
            "/Command/Game/Ability/Ability",
        );
        let mut context = parsed_slot_context_fixture();
        context.source_snapshots.client_data.command_catalog_sha256 = sha256(&data);
        let report = build_report_with_slot_context(&data, "27140", Some(&context)).unwrap();
        for serialized in [
            serde_json::to_string(&report).unwrap(),
            serde_yaml::to_string(&report).unwrap(),
        ] {
            let report: Value = if serialized.starts_with('{') {
                serde_json::from_str(&serialized).unwrap()
            } else {
                serde_yaml::from_str(&serialized).unwrap()
            };
            let observed = &report["observedCommandSlotContext"];
            assert_eq!(observed["status"], "available");
            assert_eq!(
                observed["sourceSnapshots"]["captures"]["artifact"],
                "records.csv"
            );
            assert_eq!(observed["derivation"]["carrier"], "s2c:0x0137");
            assert_eq!(observed["coverage"]["commandRecords"], 8);
            assert_eq!(observed["unresolved"][0], "category 2 is not observed");
            assert_eq!(observed["matches"].as_array().unwrap().len(), 1);
            assert_eq!(
                observed["matches"][0]["slotObservations"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(observed["matches"][0]["slotObservations"][1]["slot"], 43);
        }
    }

    #[test]
    fn reports_missing_slot_context_observation_without_inference() {
        let data = catalog(&[("27141", "Other", "Other")]);
        let mut context = parsed_slot_context_fixture();
        context.source_snapshots.client_data.command_catalog_sha256 = sha256(&data);
        let report = build_report_with_slot_context(&data, "27141", Some(&context)).unwrap();
        let observed = &report["observedCommandSlotContext"];
        assert_eq!(observed["status"], "available");
        assert!(observed["matches"].as_array().unwrap().is_empty());
        assert_eq!(observed["coverage"]["commandRecords"], 8);

        let report = build_report(&data, "27141").unwrap();
        assert_eq!(
            report["observedCommandSlotContext"]["status"],
            "unavailable"
        );
    }

    #[test]
    fn rejects_invalid_slot_context_schema_identity_and_shapes() {
        let fixture = slot_context_fixture();
        let encoded = |value: Value| serde_json::to_vec(&value).unwrap();
        let valid = serde_json::to_value(&fixture).unwrap();
        for (field, expected) in [("schemaVersion", 2), ("schemaVersion", 0)] {
            let mut value = valid.clone();
            value[field] = json!(expected);
            assert!(parse_slot_context(&encoded(value))
                .unwrap_err()
                .contains("schemaVersion"));
        }
        let mut wrong_kind = valid.clone();
        wrong_kind["kind"] = json!("wrong-kind");
        assert!(parse_slot_context(&encoded(wrong_kind))
            .unwrap_err()
            .contains("kind"));
        let mut wrong_digest = valid.clone();
        wrong_digest["rows"][0]["nameEnglish"] = json!("Changed");
        assert!(parse_slot_context(&encoded(wrong_digest))
            .unwrap_err()
            .contains("rowsSha256"));

        let invalid_cases: &[InvalidSlotContextCase] = &[
            ("duplicate command id", |value: &mut Value| {
                value["rows"] = json!([value["rows"][0].clone(), value["rows"][0].clone()])
            }),
            ("malformed actor id", |value: &mut Value| {
                value["rows"][0]["actorIdHex"] = json!("not-an-actor")
            }),
            ("low16 mismatch", |value: &mut Value| {
                value["rows"][0]["actorIdHex"] = json!("0xa0f06a05")
            }),
            ("zero command count", |value: &mut Value| {
                value["rows"][0]["commandOccurrences"] = json!(0)
            }),
            ("duplicate slot", |value: &mut Value| {
                value["rows"][0]["slotObservations"][1]["slot"] = json!(39)
            }),
            ("bad slot", |value: &mut Value| {
                value["rows"][0]["slotObservations"][0]["slot"] = json!(64)
            }),
            ("bad category", |value: &mut Value| {
                value["rows"][0]["slotObservations"][0]["categoryObservations"][0]["occurrences"] =
                    json!(0)
            }),
            ("undeclared category", |value: &mut Value| {
                value["rows"][0]["slotObservations"][0]["categoryObservations"][0]["value"] =
                    json!(2)
            }),
        ];
        for (name, mutate) in invalid_cases {
            let mut value = valid.clone();
            mutate(&mut value);
            refresh_rows_sha256(&mut value);
            assert!(
                parse_slot_context(&encoded(value)).is_err(),
                "expected {name} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_slot_context_catalog_and_identity_mismatches() {
        let data = catalog_with_class(
            &[("27140", "Sentinel", "Sentinelle")],
            "/Command/Game/Ability/Ability",
        );
        let context = parsed_slot_context_fixture();
        assert!(
            build_report_with_slot_context(&data, "27140", Some(&context))
                .unwrap_err()
                .contains("catalog pin")
        );

        let mut context = slot_context_fixture();
        context.source_snapshots.client_data.command_catalog_sha256 = sha256(&data);
        context.rows[0].class_path = "/Command/Game/Ability/Other".to_owned();
        context.rows_sha256 =
            sha256(&serde_json::to_vec(&serde_json::to_value(&context.rows).unwrap()).unwrap());
        assert!(
            build_report_with_slot_context(&data, "27140", Some(&context))
                .unwrap_err()
                .contains("identity")
        );
    }
}
