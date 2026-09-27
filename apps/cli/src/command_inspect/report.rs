use super::input::{
    MonsterAttackOverride, MonsterAttackProfilesManifest, SlotContextManifest, MAX_CATALOG_ROWS,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use xivl_formats::digest::sha256_hex;

pub(super) const HEADER: &[&str] = &[
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

#[cfg(test)]
pub(super) fn build_report(data: &[u8], query: &str) -> Result<Value, String> {
    build_report_with_inputs(data, query, None, None)
}

#[cfg(test)]
pub(super) fn build_report_with_slot_context(
    data: &[u8],
    query: &str,
    slot_context: Option<&SlotContextManifest>,
) -> Result<Value, String> {
    build_report_with_inputs(data, query, slot_context, None)
}

#[cfg(test)]
pub(super) fn build_report_with_monster_attack_profiles(
    data: &[u8],
    query: &str,
    profiles: Option<&MonsterAttackProfilesManifest>,
) -> Result<Value, String> {
    build_report_with_inputs(data, query, None, profiles)
}

pub(super) fn build_report_with_inputs(
    data: &[u8],
    query: &str,
    slot_context: Option<&SlotContextManifest>,
    monster_attack_profiles: Option<&MonsterAttackProfilesManifest>,
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
            matches.push(command_document(
                &record,
                id,
                &compatibility,
                monster_attack_profiles,
            )?);
        }
    }

    if matches.is_empty() {
        return Err(format!("command query '{query}' did not match the catalog"));
    }

    let catalog_sha256 = sha256_hex(data);
    let observed_command_slot_context = match slot_context {
        Some(context) => context.report_for_commands(&catalog_sha256, &matched_identities)?,
        None => json!({
            "status": "unavailable",
            "reason": "slot-context-input-not-supplied",
        }),
    };

    Ok(json!({
        "schemaVersion": 13,
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

pub(super) fn command_document(
    record: &csv::StringRecord,
    id: u32,
    compatibility: &[i8],
    monster_attack_profiles: Option<&MonsterAttackProfilesManifest>,
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
        "subclassGetterProfile": subclass_getter_profile(
            class_path,
            id,
            monster_attack_profiles,
        ),
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
            "resolution": {
                "status": "unresolved",
                "reason": "native magnitude scale and combine step",
            },
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

// Exact command-specific Lua getter results supplied by the producer manifest.
pub(super) fn subclass_getter_profile(
    class_path: &str,
    command_id: u32,
    profiles: Option<&MonsterAttackProfilesManifest>,
) -> Value {
    if class_path.is_empty() {
        return json!({
            "status": "unavailable",
            "reason": "missing-class-path",
        });
    }
    let Some(manifest) = profiles else {
        return json!({
            "status": "unavailable",
            "reason": "monster-attack-profiles-input-not-supplied",
        });
    };
    if class_path != manifest.class_path {
        return json!({
            "status": "unavailable",
            "reason": "monster-attack-profiles-class-path-mismatch",
            "expectedClassPath": manifest.class_path,
        });
    }

    let rules = &manifest.getter_rules;
    let information = &rules.get_command_information;
    let information_result = monster_attack_scalar_result(&information.overrides, command_id)
        .unwrap_or(information.default);
    let frequency = monster_attack_scalar_result(&rules.get_frequency.overrides, command_id)
        .unwrap_or(rules.get_frequency.default);
    let range_width = monster_attack_scalar_result(&rules.get_range_width.overrides, command_id)
        .unwrap_or(rules.get_range_width.default);
    let range_rotate = monster_attack_scalar_result(&rules.get_range_rotate.overrides, command_id)
        .unwrap_or(rules.get_range_rotate.default);
    let command_range_height =
        monster_attack_scalar_result(&rules.get_command_range_height.overrides, command_id)
            .unwrap_or(rules.get_command_range_height.default);
    let parts_damage_adjust =
        monster_attack_pair_result(&rules.get_parts_damage_adjust.overrides, command_id)
            .unwrap_or(rules.get_parts_damage_adjust.default);
    json!({
        "status": "resolved",
        "scope": "producer-rule-manifest-defaults-and-sparse-overrides",
        "definedBy": "MonsterAttackWeaponSkill",
        "profileIdentity": {
            "version": manifest.version,
            "gameVersion": manifest.game_version,
            "extraction": manifest.extraction,
            "classPath": manifest.class_path,
            "parentPath": manifest.parent_path,
            "getterRulesSha256": manifest.getter_rules_sha256,
        },
        "input": {
            "byteLength": manifest.input_byte_length,
            "sha256": manifest.input_sha256,
        },
        "source": manifest.source,
        "summary": manifest.summary,
        "unresolved": manifest.unresolved,
        "getters": {
            "getCommandInformation": {
                "selector": information.selector,
                "result": information_result,
                "otherSelectors": information.other_selectors,
            },
            "getFrequency": frequency,
            "getRangeWidth": range_width,
            "getRangeRotate": range_rotate,
            "getCommandRangeHeight": command_range_height,
            "getPartsDamageAdjust": {
                "result": parts_damage_adjust,
                "consumer": "unresolved",
            },
        },
    })
}

fn monster_attack_scalar_result(
    overrides: &[MonsterAttackOverride],
    command_id: u32,
) -> Option<i64> {
    overrides.iter().find_map(|override_rule| {
        if override_rule.command_ids.contains(&command_id) {
            override_rule.result.as_i64()
        } else {
            None
        }
    })
}

fn monster_attack_pair_result(
    overrides: &[MonsterAttackOverride],
    command_id: u32,
) -> Option<[i64; 2]> {
    overrides.iter().find_map(|override_rule| {
        if !override_rule.command_ids.contains(&command_id) {
            return None;
        }
        let values = override_rule.result.as_array()?;
        Some([values[0].as_i64()?, values[1].as_i64()?])
    })
}

// Matrix selection and actor-dependent shortcuts: docs/command-compatibility-profiles.md.
pub(super) fn compatibility_profile(
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

pub(super) fn command_uses_action_slot(command_id: u32) -> bool {
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
pub(super) fn parameter_profile(class_path: &str) -> Value {
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
pub(super) fn cost_profile(class_path: &str, id: u32) -> Value {
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
pub(super) fn level_adjustment_profile(class_path: &str) -> Value {
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

pub(super) fn field<'a>(record: &'a csv::StringRecord, name: &str) -> &'a str {
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

pub(super) fn parse_compatibility_values(raw: &str) -> Result<Vec<i8>, String> {
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
