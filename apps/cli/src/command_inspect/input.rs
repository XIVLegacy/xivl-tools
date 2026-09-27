use crate::Failure;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use xivl_formats::digest::sha256_hex;

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
pub(super) const MAX_CATALOG_ROWS: usize = 100_000;
const MAX_SLOT_CONTEXT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MONSTER_ATTACK_PROFILES_BYTES: u64 = 4 * 1024 * 1024;

const MONSTER_ATTACK_CLASS_PATH: &str = "/Command/Game/WeaponSkill/MonsterAttackWeaponSkill";
pub(super) const MONSTER_ATTACK_PARENT_PATH: &str =
    "/Command/Game/WeaponSkill/WeaponSkillBaseClass";
const MONSTER_ATTACK_SOURCE_SCRIPT: &str =
    "lua/scripts/command/game/weaponskill/monsterattackweaponskill.lua";
pub(super) const MONSTER_ATTACK_SOURCE_SHA256: &str =
    "d5b8e884aad2ca2cfe5cfa96cf5e029d975a32bb0bc1742873ded2f3a78b668e";
const MONSTER_ATTACK_RULES_SHA256: &str =
    "446bb12571d90c6a5095feb49ed9f5056c2ccee84f902204e70ec985833e25f2";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackProfilesManifest {
    pub(super) version: String,
    pub(super) game_version: String,
    pub(super) extraction: String,
    pub(super) class_path: String,
    pub(super) parent_path: String,
    pub(super) getter_rules_sha256: String,
    pub(super) source: MonsterAttackProfileSource,
    pub(super) summary: MonsterAttackProfileSummary,
    pub(super) getter_rules: MonsterAttackGetterRules,
    pub(super) unresolved: Vec<String>,
    #[serde(skip)]
    pub(super) input_byte_length: u64,
    #[serde(skip)]
    pub(super) input_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackProfileSource {
    pub(super) script: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) line_count: u32,
    pub(super) manifest: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackProfileSummary {
    pub(super) getter_count: u32,
    pub(super) override_group_count: u32,
    pub(super) override_command_count: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackGetterRules {
    pub(super) get_command_information: MonsterAttackInformationRule,
    pub(super) get_frequency: MonsterAttackScalarRule,
    pub(super) get_range_width: MonsterAttackScalarRule,
    pub(super) get_range_rotate: MonsterAttackScalarRule,
    pub(super) get_command_range_height: MonsterAttackScalarRule,
    pub(super) get_parts_damage_adjust: MonsterAttackPairRule,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackInformationRule {
    pub(super) default: i64,
    pub(super) selector: u32,
    pub(super) other_selectors: String,
    pub(super) overrides: Vec<MonsterAttackOverride>,
    pub(super) definition_line: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackScalarRule {
    pub(super) default: i64,
    pub(super) overrides: Vec<MonsterAttackOverride>,
    pub(super) definition_line: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackPairRule {
    pub(super) default: [i64; 2],
    pub(super) return_arity: u32,
    pub(super) overrides: Vec<MonsterAttackOverride>,
    pub(super) definition_line: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct MonsterAttackOverride {
    pub(super) command_ids: Vec<u32>,
    pub(super) result: Value,
    pub(super) source_lines: Vec<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SlotContextManifest {
    pub(super) schema_version: u32,
    pub(super) kind: String,
    pub(super) game_version: String,
    pub(super) status: String,
    pub(super) source_snapshots: SourceSnapshots,
    pub(super) derivation: Derivation,
    pub(super) coverage: Coverage,
    pub(super) rows_sha256: String,
    pub(super) rows: Vec<SlotContextRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) write_corpus: Option<WriteCorpus>,
    pub(super) unresolved: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SourceSnapshots {
    pub(super) captures: CaptureSnapshot,
    pub(super) client_structs: ClientStructSnapshot,
    pub(super) client_data: ClientDataSnapshot,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CaptureSnapshot {
    pub(super) repository: String,
    pub(super) commit: String,
    pub(super) artifact: String,
    pub(super) sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ClientStructSnapshot {
    pub(super) repository: String,
    pub(super) generator_artifact: String,
    pub(super) generator_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) generator_hash_normalization: Option<String>,
    pub(super) hash_names_artifact: String,
    pub(super) hash_names_sha256: String,
    pub(super) actor_identity_artifact: String,
    pub(super) actor_identity_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct ClientDataSnapshot {
    pub(super) repository: String,
    pub(super) commit: String,
    pub(super) static_actor_artifact: String,
    pub(super) static_actor_sha256: String,
    pub(super) command_catalog_artifact: String,
    pub(super) command_catalog_sha256: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Derivation {
    pub(super) carrier: String,
    pub(super) state_partition: Vec<String>,
    pub(super) state_order: String,
    pub(super) state_rule: String,
    pub(super) static_actor_test: String,
    pub(super) command_id_decode: String,
    pub(super) identity_boundary: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Coverage {
    pub(super) command_records: u64,
    pub(super) nonzero_command_occurrences: u64,
    pub(super) unique_nonzero_command_actors: u64,
    pub(super) static_actor_prefix_hits: u64,
    pub(super) static_actor_catalog_hits: u64,
    pub(super) command_catalog_hits: u64,
    pub(super) category_records: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) border_records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) relevant_write_records: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) zero_command_writes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) state_partitions: Option<u64>,
    pub(super) category_hashes: u64,
    pub(super) category_value_distribution: Vec<CategoryValueDistribution>,
    pub(super) stateful_category_observations: u64,
    pub(super) commands_with_category_observations: u64,
    pub(super) category_writes_without_current_command: Vec<CategoryWriteSummary>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CategoryValueDistribution {
    pub(super) value: u8,
    pub(super) occurrences: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CategoryWriteSummary {
    pub(super) slot: u8,
    pub(super) occurrences: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SlotContextRow {
    pub(super) actor_id_hex: String,
    pub(super) command_id: u32,
    pub(super) class_path: String,
    pub(super) name_english: String,
    pub(super) command_occurrences: u64,
    pub(super) slot_observations: Vec<SlotObservation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct SlotObservation {
    pub(super) slot: u8,
    pub(super) command_occurrences: u64,
    pub(super) category_observations: Vec<CategoryObservation>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct CategoryObservation {
    pub(super) value: u8,
    pub(super) occurrences: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct WriteCorpus {
    pub(super) scope: String,
    pub(super) record_encoding: String,
    pub(super) property_hash_encoding: String,
    pub(super) state_partition: Vec<String>,
    pub(super) state_order: String,
    pub(super) partial_state: bool,
    pub(super) initial_state: String,
    pub(super) final_state: String,
    pub(super) server_authoritative: bool,
    pub(super) packet_replay: bool,
    pub(super) writes_sha256: String,
    pub(super) writes: Vec<Value>,
}

impl SlotContextManifest {
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 && self.schema_version != 2 {
            return Err(format!(
                "slot context schemaVersion must be 1 or 2, got {}",
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
        if self.schema_version == 2 {
            let normalization = self
                .source_snapshots
                .client_structs
                .generator_hash_normalization
                .as_deref()
                .ok_or_else(|| {
                    "slot context schema 2 requires generatorHashNormalization".to_owned()
                })?;
            if normalization != "UTF-8 text with CRLF and CR normalized to LF" {
                return Err(
                    "slot context generatorHashNormalization does not match schema 2".to_owned(),
                );
            }
            if self.write_corpus.is_none() {
                return Err("slot context schema 2 requires writeCorpus".to_owned());
            }
        } else if self.write_corpus.is_some()
            || self
                .source_snapshots
                .client_structs
                .generator_hash_normalization
                .is_some()
            || self.coverage.border_records.is_some()
            || self.coverage.relevant_write_records.is_some()
            || self.coverage.zero_command_writes.is_some()
            || self.coverage.state_partitions.is_some()
        {
            return Err("slot context schema 1 does not permit schema 2 fields".to_owned());
        }
        validate_coverage(&self.coverage, self.schema_version)?;
        let row_value = serde_json::to_value(&self.rows)
            .map_err(|error| format!("cannot canonicalize slot context rows: {error}"))?;
        let row_bytes = serde_json::to_vec(&row_value)
            .map_err(|error| format!("cannot canonicalize slot context rows: {error}"))?;
        if self.rows_sha256 != sha256_hex(&row_bytes) {
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
        if let Some(write_corpus) = &self.write_corpus {
            validate_write_corpus(write_corpus, &self.coverage, &self.rows)?;
        }
        Ok(())
    }

    pub(super) fn report_for_commands(
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

fn validate_coverage(coverage: &Coverage, schema_version: u32) -> Result<(), String> {
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
    if schema_version == 2 {
        for (label, value) in [
            ("borderRecords", coverage.border_records),
            ("relevantWriteRecords", coverage.relevant_write_records),
            ("zeroCommandWrites", coverage.zero_command_writes),
            ("statePartitions", coverage.state_partitions),
        ] {
            if value.is_none() {
                return Err(format!("slot context coverage {label} is missing"));
            }
            if value == Some(0) {
                return Err(format!("slot context coverage {label} must be nonzero"));
            }
        }
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

#[derive(Debug, Clone)]
pub(super) struct ParsedWrite {
    pub(super) record_index: u64,
    pub(super) capture: String,
    pub(super) lane_index: u32,
    pub(super) source_actor_id: u32,
    pub(super) operation: String,
    pub(super) property_hash: u32,
    pub(super) slot: Option<u8>,
    pub(super) command_id: Option<u32>,
    pub(super) actor_id: Option<u32>,
    pub(super) class_path: Option<String>,
    pub(super) category_value: Option<u8>,
    pub(super) joined_command_record_index: Option<u64>,
    pub(super) record_fragment: Vec<u8>,
}

pub(super) type TraceKey = (String, u32, u32);

fn validate_write_corpus(
    corpus: &WriteCorpus,
    coverage: &Coverage,
    rows: &[SlotContextRow],
) -> Result<(), String> {
    if corpus.scope != "observed-filtered-property-record-fragments"
        || corpus.record_encoding != "valueWidth:u8 + propertyHash:u32le + value[valueWidth]"
        || corpus.property_hash_encoding != "little-endian u32"
        || corpus.state_partition
            != ["capture", "laneIndex", "sourceActorId"]
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        || corpus.state_order != "increasing recordIndex within each partition"
    {
        return Err("slot context writeCorpus metadata is inconsistent".to_owned());
    }
    if !corpus.partial_state
        || corpus.initial_state != "unknown"
        || corpus.final_state != "unasserted"
        || corpus.server_authoritative
        || corpus.packet_replay
    {
        return Err("slot context writeCorpus evidence boundary is inconsistent".to_owned());
    }
    let writes_bytes = serde_json::to_vec(&corpus.writes)
        .map_err(|error| format!("cannot canonicalize writeCorpus writes: {error}"))?;
    if corpus.writes_sha256 != sha256_hex(&writes_bytes) {
        return Err("slot context writeCorpus writesSha256 does not match writes".to_owned());
    }

    let mut parsed = Vec::with_capacity(corpus.writes.len());
    let mut last_record_by_trace: BTreeMap<TraceKey, u64> = BTreeMap::new();
    let mut writes_by_trace_record: HashMap<(TraceKey, u64), ParsedWrite> = HashMap::new();
    let mut operation_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut category_hashes = HashSet::new();
    let mut category_values: BTreeMap<u8, u64> = BTreeMap::new();
    let mut unjoined_categories: BTreeMap<u8, u64> = BTreeMap::new();
    let mut partitions = BTreeSet::new();

    for (index, value) in corpus.writes.iter().enumerate() {
        let write = parse_write(value, index)?;
        let trace = (
            write.capture.clone(),
            write.lane_index,
            write.source_actor_id,
        );
        if let Some(previous) = last_record_by_trace.get(&trace) {
            if write.record_index <= *previous {
                return Err(format!(
                    "slot context writeCorpus record order is not increasing in trace at write {}",
                    index + 1
                ));
            }
        }
        last_record_by_trace.insert(trace.clone(), write.record_index);
        partitions.insert(trace.clone());
        *operation_counts.entry(write.operation.clone()).or_default() += 1;
        if write.operation == "set-category" {
            category_hashes.insert(write.property_hash);
            *category_values
                .entry(write.category_value.unwrap())
                .or_default() += 1;
            if write.joined_command_record_index.is_none() {
                *unjoined_categories.entry(write.slot.unwrap()).or_default() += 1;
            }
        }
        if writes_by_trace_record
            .insert((trace, write.record_index), write.clone())
            .is_some()
        {
            return Err(format!(
                "slot context writeCorpus contains duplicate trace record at write {}",
                index + 1
            ));
        }
        parsed.push(write);
    }

    let count = |name: &str| operation_counts.get(name).copied().unwrap_or_default();
    let expected_counts = [
        (
            "commandRecords",
            count("set-command") + count("clear"),
            coverage.command_records,
        ),
        (
            "nonzeroCommandOccurrences",
            count("set-command"),
            coverage.nonzero_command_occurrences,
        ),
        (
            "categoryRecords",
            count("set-category"),
            coverage.category_records,
        ),
        (
            "borderRecords",
            count("set-border"),
            coverage.border_records.unwrap(),
        ),
        (
            "relevantWriteRecords",
            corpus.writes.len() as u64,
            coverage.relevant_write_records.unwrap(),
        ),
        (
            "zeroCommandWrites",
            count("clear"),
            coverage.zero_command_writes.unwrap(),
        ),
        (
            "statePartitions",
            partitions.len() as u64,
            coverage.state_partitions.unwrap(),
        ),
        (
            "categoryHashes",
            category_hashes.len() as u64,
            coverage.category_hashes,
        ),
    ];
    if let Some((name, actual, expected)) = expected_counts
        .into_iter()
        .find(|(_, actual, expected)| actual != expected)
    {
        return Err(format!(
            "slot context writeCorpus {name} does not match coverage ({actual} != {expected})"
        ));
    }

    let declared_categories: BTreeMap<u8, u64> = coverage
        .category_value_distribution
        .iter()
        .map(|entry| (entry.value, entry.occurrences))
        .collect();
    if declared_categories != category_values {
        return Err(
            "slot context writeCorpus category value distribution does not match coverage"
                .to_owned(),
        );
    }
    let declared_unjoined: BTreeMap<u8, u64> = coverage
        .category_writes_without_current_command
        .iter()
        .map(|entry| (entry.slot, entry.occurrences))
        .collect();
    if declared_unjoined != unjoined_categories {
        return Err(
            "slot context writeCorpus unjoined category totals do not match coverage".to_owned(),
        );
    }

    let identities: HashMap<u32, (u32, String)> = rows
        .iter()
        .map(|row| {
            (
                row.command_id,
                (
                    parse_actor_id(&row.actor_id_hex).unwrap(),
                    row.class_path.clone(),
                ),
            )
        })
        .collect();
    let mut stateful_category_observations = 0_u64;
    let mut current_commands: HashMap<(TraceKey, u8), u64> = HashMap::new();
    for write in &parsed {
        if write.operation == "set-command" {
            let command_id = write.command_id.unwrap();
            let (expected_actor, expected_class) =
                identities.get(&command_id).ok_or_else(|| {
                    format!(
                        "slot context writeCorpus command {} has no matching row identity",
                        command_id
                    )
                })?;
            if write.actor_id != Some(*expected_actor)
                || write.class_path.as_deref() != Some(expected_class.as_str())
            {
                return Err(format!(
                    "slot context writeCorpus command identity does not match row for command {}",
                    command_id
                ));
            }
            current_commands.insert(
                (
                    (
                        write.capture.clone(),
                        write.lane_index,
                        write.source_actor_id,
                    ),
                    write.slot.unwrap(),
                ),
                write.record_index,
            );
        } else if write.operation == "clear" {
            current_commands.remove(&(
                (
                    write.capture.clone(),
                    write.lane_index,
                    write.source_actor_id,
                ),
                write.slot.unwrap(),
            ));
        } else if write.operation == "set-category" {
            let current = current_commands.get(&(
                (
                    write.capture.clone(),
                    write.lane_index,
                    write.source_actor_id,
                ),
                write.slot.unwrap(),
            ));
            if current.copied() != write.joined_command_record_index {
                return Err(
                    "slot context writeCorpus category join does not match current command state"
                        .to_owned(),
                );
            }
            if let Some(joined_record_index) = write.joined_command_record_index {
                stateful_category_observations += 1;
                let trace = (
                    write.capture.clone(),
                    write.lane_index,
                    write.source_actor_id,
                );
                let joined = writes_by_trace_record
                    .get(&(trace, joined_record_index))
                    .ok_or_else(|| {
                        format!(
                            "slot context writeCorpus category join references missing record {}",
                            joined_record_index
                        )
                    })?;
                if joined.operation != "set-command"
                    || joined.slot != write.slot
                    || joined.record_index >= write.record_index
                {
                    return Err(
                        "slot context writeCorpus category join is inconsistent with command order"
                            .to_owned(),
                    );
                }
            }
        }
    }
    if stateful_category_observations != coverage.stateful_category_observations {
        return Err(
            "slot context writeCorpus stateful category total does not match coverage".to_owned(),
        );
    }
    Ok(())
}

pub(super) fn parse_write(value: &Value, index: usize) -> Result<ParsedWrite, String> {
    let object = value.as_object().ok_or_else(|| {
        format!(
            "slot context writeCorpus write {} must be an object",
            index + 1
        )
    })?;
    let operation = object
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "slot context writeCorpus write {} has no operation",
                index + 1
            )
        })?;
    let expected_keys: &[&str] = match operation {
        "clear" => &[
            "recordIndex",
            "capture",
            "laneIndex",
            "sourceActorId",
            "propertyPath",
            "propertyHash",
            "valueWidth",
            "valueHex",
            "recordFragmentHex",
            "operation",
            "slot",
        ],
        "set-command" => &[
            "recordIndex",
            "capture",
            "laneIndex",
            "sourceActorId",
            "propertyPath",
            "propertyHash",
            "valueWidth",
            "valueHex",
            "recordFragmentHex",
            "operation",
            "slot",
            "actorIdHex",
            "commandId",
            "classPath",
        ],
        "set-category" => &[
            "recordIndex",
            "capture",
            "laneIndex",
            "sourceActorId",
            "propertyPath",
            "propertyHash",
            "valueWidth",
            "valueHex",
            "recordFragmentHex",
            "operation",
            "slot",
            "categoryValue",
            "joinedCommandRecordIndex",
        ],
        "set-border" => &[
            "recordIndex",
            "capture",
            "laneIndex",
            "sourceActorId",
            "propertyPath",
            "propertyHash",
            "valueWidth",
            "valueHex",
            "recordFragmentHex",
            "operation",
            "borderValue",
        ],
        _ => {
            return Err(format!(
                "slot context writeCorpus write {} has unsupported operation '{}',",
                index + 1,
                operation
            ));
        }
    };
    if object.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !object.contains_key(*key))
    {
        return Err(format!(
            "slot context writeCorpus write {} has invalid {} operation shape",
            index + 1,
            operation
        ));
    }

    let record_index = required_u64(object, "recordIndex", index)?;
    let capture = required_string(object, "capture", index)?;
    if capture.is_empty() {
        return Err(format!(
            "slot context writeCorpus write {} capture must not be empty",
            index + 1
        ));
    }
    let lane_index = required_u32(object, "laneIndex", index)?;
    let source_actor_id = required_u32(object, "sourceActorId", index)?;
    let property_path = required_string(object, "propertyPath", index)?;
    let property_hash = parse_actor_id(required_string(object, "propertyHash", index)?.as_str())
        .map_err(|error| {
            format!(
                "slot context writeCorpus write {} propertyHash {}",
                index + 1,
                error
            )
        })?;
    let value_width = required_u8(object, "valueWidth", index)?;
    let value_hex = required_string(object, "valueHex", index)?;
    let value = parse_hex_bytes(value_hex.as_str()).map_err(|error| {
        format!(
            "slot context writeCorpus write {} valueHex {}",
            index + 1,
            error
        )
    })?;
    if value.len() != usize::from(value_width) {
        return Err(format!(
            "slot context writeCorpus write {} valueHex length does not match valueWidth",
            index + 1
        ));
    }
    let fragment = parse_hex_bytes(required_string(object, "recordFragmentHex", index)?.as_str())
        .map_err(|error| {
        format!(
            "slot context writeCorpus write {} recordFragmentHex {}",
            index + 1,
            error
        )
    })?;
    let mut expected_fragment = Vec::with_capacity(5 + value.len());
    expected_fragment.push(value_width);
    expected_fragment.extend_from_slice(&property_hash.to_le_bytes());
    expected_fragment.extend_from_slice(&value);
    if fragment != expected_fragment {
        return Err(format!(
            "slot context writeCorpus write {} record fragment does not match encoding",
            index + 1
        ));
    }

    let slot = match operation {
        "clear" | "set-command" | "set-category" => {
            let slot = required_u8(object, "slot", index)?;
            validate_slot(
                slot,
                &format!("slot context writeCorpus write {}", index + 1),
            )?;
            let prefix = if operation == "set-category" {
                "charaWork.commandCategory["
            } else {
                "charaWork.command["
            };
            if parse_indexed_property(&property_path, prefix)? != slot {
                return Err(format!(
                    "slot context writeCorpus write {} property path and slot disagree",
                    index + 1
                ));
            }
            Some(slot)
        }
        "set-border" => {
            if property_path != "charaWork.commandBorder" {
                return Err(format!(
                    "slot context writeCorpus write {} set-border property path is invalid",
                    index + 1
                ));
            }
            None
        }
        _ => unreachable!(),
    };

    let (command_id, actor_id, class_path, category_value, joined) = match operation {
        "clear" => {
            if value_width != 4 || value.iter().any(|byte| *byte != 0) {
                return Err(format!(
                    "slot context writeCorpus write {} clear must contain four zero bytes",
                    index + 1
                ));
            }
            (None, None, None, None, None)
        }
        "set-command" => {
            if value_width != 4 {
                return Err(format!(
                    "slot context writeCorpus write {} set-command must have valueWidth 4",
                    index + 1
                ));
            }
            let actor_id_hex = required_string(object, "actorIdHex", index)?;
            let actor_id = parse_actor_id(actor_id_hex.as_str()).map_err(|error| {
                format!(
                    "slot context writeCorpus write {} actorIdHex {}",
                    index + 1,
                    error
                )
            })?;
            if actor_id & 0xffff_0000 != 0xa0f0_0000 {
                return Err(format!(
                    "slot context writeCorpus write {} actorIdHex must use static actor prefix",
                    index + 1
                ));
            }
            if value != actor_id.to_le_bytes() {
                return Err(format!(
                    "slot context writeCorpus write {} valueHex does not match actorIdHex",
                    index + 1
                ));
            }
            let command_id = required_u32(object, "commandId", index)?;
            if command_id == 0 || actor_id & 0xffff != command_id {
                return Err(format!(
                    "slot context writeCorpus write {} command identity is inconsistent",
                    index + 1
                ));
            }
            let class_path = required_string(object, "classPath", index)?;
            if !class_path.starts_with("/Command/") {
                return Err(format!(
                    "slot context writeCorpus write {} classPath must start with /Command/",
                    index + 1
                ));
            }
            (
                Some(command_id),
                Some(actor_id),
                Some(class_path),
                None,
                None,
            )
        }
        "set-category" => {
            if value_width != 1 {
                return Err(format!(
                    "slot context writeCorpus write {} set-category must have valueWidth 1",
                    index + 1
                ));
            }
            let category_value = required_u8(object, "categoryValue", index)?;
            if value != [category_value] {
                return Err(format!(
                    "slot context writeCorpus write {} valueHex does not match categoryValue",
                    index + 1
                ));
            }
            let joined = match object.get("joinedCommandRecordIndex") {
                Some(Value::Null) => None,
                Some(value) => Some(value.as_u64().ok_or_else(|| {
                    format!(
                        "slot context writeCorpus write {} joinedCommandRecordIndex must be an integer or null",
                        index + 1
                    )
                })?),
                None => unreachable!(),
            };
            (None, None, None, Some(category_value), joined)
        }
        "set-border" => {
            if value_width != 1 {
                return Err(format!(
                    "slot context writeCorpus write {} set-border must have valueWidth 1",
                    index + 1
                ));
            }
            let border_value = required_u8(object, "borderValue", index)?;
            if value != [border_value] {
                return Err(format!(
                    "slot context writeCorpus write {} valueHex does not match borderValue",
                    index + 1
                ));
            }
            (None, None, None, None, None)
        }
        _ => unreachable!(),
    };

    Ok(ParsedWrite {
        record_index,
        capture,
        lane_index,
        source_actor_id,
        operation: operation.to_owned(),
        property_hash,
        slot,
        command_id,
        actor_id,
        class_path,
        category_value,
        joined_command_record_index: joined,
        record_fragment: fragment,
    })
}

fn parse_indexed_property(path: &str, prefix: &str) -> Result<u8, String> {
    let digits = path
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| "property path is not an indexed command property".to_owned())?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("property path has an invalid slot index".to_owned());
    }
    let slot = digits
        .parse::<u8>()
        .map_err(|_| "property path slot index is out of range".to_owned())?;
    validate_slot(slot, "property path")?;
    Ok(slot)
}

fn required_string(object: &Map<String, Value>, key: &str, index: usize) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "slot context writeCorpus write {} {key} must be a string",
                index + 1
            )
        })
}

fn required_u64(object: &Map<String, Value>, key: &str, index: usize) -> Result<u64, String> {
    object.get(key).and_then(Value::as_u64).ok_or_else(|| {
        format!(
            "slot context writeCorpus write {} {key} must be an integer",
            index + 1
        )
    })
}

fn required_u32(object: &Map<String, Value>, key: &str, index: usize) -> Result<u32, String> {
    let value = required_u64(object, key, index)?;
    u32::try_from(value).map_err(|_| {
        format!(
            "slot context writeCorpus write {} {key} is out of range",
            index + 1
        )
    })
}

fn required_u8(object: &Map<String, Value>, key: &str, index: usize) -> Result<u8, String> {
    let value = required_u64(object, key, index)?;
    u8::try_from(value).map_err(|_| {
        format!(
            "slot context writeCorpus write {} {key} is out of range",
            index + 1
        )
    })
}

pub(super) fn parse_hex_bytes(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("must contain an even number of hexadecimal digits".to_owned());
    }
    if !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("contains invalid hexadecimal digits".to_owned());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| "contains invalid hexadecimal digits".to_owned())
        })
        .collect()
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

pub(super) fn read_catalog(path: &str) -> Result<Vec<u8>, Failure> {
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

pub(super) fn read_slot_context(path: &str) -> Result<SlotContextManifest, Failure> {
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

pub(super) fn parse_slot_context(data: &[u8]) -> Result<SlotContextManifest, String> {
    let manifest = serde_json::from_slice::<SlotContextManifest>(data)
        .map_err(|error| format!("invalid slot context JSON: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}

pub(super) fn read_monster_attack_profiles(
    path: &str,
) -> Result<MonsterAttackProfilesManifest, Failure> {
    let file = std::fs::File::open(path)
        .map_err(|error| Failure::usage(format!("cannot read '{path}': {error}")))?;
    let mut data = Vec::new();
    file.take(MAX_MONSTER_ATTACK_PROFILES_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| Failure::usage(format!("cannot read '{path}': {error}")))?;
    if data.len() as u64 > MAX_MONSTER_ATTACK_PROFILES_BYTES {
        return Err(Failure::usage(format!(
            "monster attack profiles is larger than the {MAX_MONSTER_ATTACK_PROFILES_BYTES}-byte limit"
        )));
    }
    let mut manifest = parse_monster_attack_profiles(&data).map_err(|error| {
        Failure::usage(format!(
            "cannot parse monster attack profiles '{path}': {error}"
        ))
    })?;
    manifest.input_byte_length = data.len() as u64;
    manifest.input_sha256 = sha256_hex(&data);
    Ok(manifest)
}

pub(super) fn parse_monster_attack_profiles(
    data: &[u8],
) -> Result<MonsterAttackProfilesManifest, String> {
    let manifest = serde_json::from_slice::<MonsterAttackProfilesManifest>(data)
        .map_err(|error| format!("invalid JSON: {error}"))?;
    if manifest.version != "1" {
        return Err(format!("version must be 1, got {}", manifest.version));
    }
    if manifest.game_version != "1.23b" {
        return Err(format!(
            "gameVersion must be 1.23b, got {}",
            manifest.game_version
        ));
    }
    if manifest.extraction != "2012.09.19.0001" {
        return Err(format!(
            "extraction must be 2012.09.19.0001, got {}",
            manifest.extraction
        ));
    }
    if manifest.class_path != MONSTER_ATTACK_CLASS_PATH {
        return Err(format!(
            "classPath must be {MONSTER_ATTACK_CLASS_PATH}, got {}",
            manifest.class_path
        ));
    }
    if manifest.parent_path != MONSTER_ATTACK_PARENT_PATH {
        return Err(format!(
            "parentPath must be {MONSTER_ATTACK_PARENT_PATH}, got {}",
            manifest.parent_path
        ));
    }
    if manifest.getter_rules_sha256 != MONSTER_ATTACK_RULES_SHA256 {
        return Err(format!(
            "getterRulesSha256 must be {MONSTER_ATTACK_RULES_SHA256}, got {}",
            manifest.getter_rules_sha256
        ));
    }
    if manifest.summary.getter_count != 6
        || manifest.summary.override_group_count != 12
        || manifest.summary.override_command_count != 57
    {
        return Err("summary does not match the producer contract".to_owned());
    }
    if manifest.source.script != MONSTER_ATTACK_SOURCE_SCRIPT
        || manifest.source.sha256 != MONSTER_ATTACK_SOURCE_SHA256
        || manifest.source.bytes != 59_652
        || manifest.source.line_count != 3_469
        || manifest.source.manifest != "manifests/scripts.json"
    {
        return Err("source does not match the producer contract".to_owned());
    }
    if manifest.unresolved.is_empty()
        || manifest.unresolved.iter().any(|value| value.is_empty())
        || manifest.unresolved.iter().collect::<HashSet<_>>().len() != manifest.unresolved.len()
    {
        return Err("unresolved must contain unique nonempty strings".to_owned());
    }
    validate_monster_attack_rules(&manifest.getter_rules)?;
    let getter_rules = serde_json::to_value(&manifest.getter_rules)
        .map_err(|error| format!("cannot canonicalize getterRules: {error}"))?;
    let canonical_getter_rules = serde_json::to_vec(&getter_rules)
        .map_err(|error| format!("cannot encode canonical getterRules: {error}"))?;
    if sha256_hex(&canonical_getter_rules) != manifest.getter_rules_sha256 {
        return Err("getterRulesSha256 does not match getterRules".to_owned());
    }
    Ok(manifest)
}

fn validate_monster_attack_rules(rules: &MonsterAttackGetterRules) -> Result<(), String> {
    if rules.get_command_information.default != 1
        || rules.get_command_information.selector != 8
        || rules.get_command_information.other_selectors != "nil"
        || rules.get_command_information.definition_line == 0
    {
        return Err("getCommandInformation defaults or selector drifted".to_owned());
    }
    if rules.get_frequency.definition_line == 0
        || rules.get_range_width.definition_line == 0
        || rules.get_range_rotate.definition_line == 0
        || rules.get_command_range_height.definition_line == 0
        || rules.get_parts_damage_adjust.definition_line == 0
    {
        return Err("getter definitionLine must be positive".to_owned());
    }
    if rules.get_parts_damage_adjust.return_arity != 2 {
        return Err("getPartsDamageAdjust returnArity must be 2".to_owned());
    }
    let mut groups = 0_u32;
    let mut command_count = 0_u32;
    validate_monster_attack_rule_overrides(
        "getCommandInformation",
        &rules.get_command_information.overrides,
        |result| result.as_i64().is_some(),
        &mut groups,
        &mut command_count,
    )?;
    for (name, rule) in [
        ("getFrequency", &rules.get_frequency.overrides),
        ("getRangeWidth", &rules.get_range_width.overrides),
        ("getRangeRotate", &rules.get_range_rotate.overrides),
        (
            "getCommandRangeHeight",
            &rules.get_command_range_height.overrides,
        ),
    ] {
        validate_monster_attack_rule_overrides(
            name,
            rule,
            |result| result.as_i64().is_some(),
            &mut groups,
            &mut command_count,
        )?;
    }
    validate_monster_attack_rule_overrides(
        "getPartsDamageAdjust",
        &rules.get_parts_damage_adjust.overrides,
        |result| {
            result.as_array().is_some_and(|values| {
                values.len() == 2 && values.iter().all(|value| value.as_i64().is_some())
            })
        },
        &mut groups,
        &mut command_count,
    )?;
    if groups != 12 || command_count != 57 {
        return Err(format!(
            "getter rule override summary does not match (groups {groups}, commands {command_count})"
        ));
    }
    Ok(())
}

fn validate_monster_attack_rule_overrides(
    getter: &str,
    overrides: &[MonsterAttackOverride],
    valid_result: impl Fn(&Value) -> bool,
    groups: &mut u32,
    command_count: &mut u32,
) -> Result<(), String> {
    for (index, override_rule) in overrides.iter().enumerate() {
        if override_rule.command_ids.is_empty()
            || override_rule
                .command_ids
                .iter()
                .any(|command_id| *command_id < 10_000)
            || override_rule
                .command_ids
                .iter()
                .collect::<HashSet<_>>()
                .len()
                != override_rule.command_ids.len()
        {
            return Err(format!(
                "{getter} override {} has invalid commandIds",
                index + 1
            ));
        }
        if !valid_result(&override_rule.result) {
            return Err(format!(
                "{getter} override {} has an invalid result",
                index + 1
            ));
        }
        if override_rule.source_lines.is_empty()
            || override_rule.source_lines.contains(&0)
            || override_rule
                .source_lines
                .iter()
                .collect::<HashSet<_>>()
                .len()
                != override_rule.source_lines.len()
        {
            return Err(format!(
                "{getter} override {} has invalid sourceLines",
                index + 1
            ));
        }
        *groups += 1;
        *command_count += override_rule.command_ids.len() as u32;
    }
    Ok(())
}
