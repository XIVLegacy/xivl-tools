use super::input::*;
use super::loadout::*;
use super::report::*;
use super::*;
use xivl_formats::digest::sha256_hex;

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
                generator_hash_normalization: None,
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
            border_records: None,
            relevant_write_records: None,
            zero_command_writes: None,
            state_partitions: None,
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
        write_corpus: None,
        unresolved: vec!["category 2 is not observed".to_owned()],
    };
    fixture.rows_sha256 =
        sha256_hex(&serde_json::to_vec(&serde_json::to_value(&fixture.rows).unwrap()).unwrap());
    fixture
}

fn parsed_slot_context_fixture() -> SlotContextManifest {
    let fixture = slot_context_fixture();
    parse_slot_context(&serde_json::to_vec(&fixture).unwrap()).unwrap()
}

fn monster_attack_profiles_fixture_value() -> Value {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/monster_attack_weapon_skill_profiles.json"
    )))
    .unwrap()
}

fn monster_attack_profiles_fixture() -> MonsterAttackProfilesManifest {
    let encoded = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/monster_attack_weapon_skill_profiles.json"
    ));
    let mut parsed = parse_monster_attack_profiles(encoded).unwrap();
    parsed.input_byte_length = encoded.len() as u64;
    parsed.input_sha256 = sha256_hex(encoded);
    parsed
}

fn synthetic_write(
    record_index: u64,
    operation: &str,
    slot: Option<u8>,
    value: &[u8],
    property_hash: u32,
    joined_command_record_index: Option<u64>,
) -> Value {
    let property_path = match (operation, slot) {
        ("set-border", None) => "charaWork.commandBorder".to_owned(),
        ("set-category", Some(slot)) => format!("charaWork.commandCategory[{slot}]"),
        (_, Some(slot)) => format!("charaWork.command[{slot}]"),
        _ => panic!("invalid synthetic write shape"),
    };
    let value_hex = value
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut fragment = vec![value.len() as u8];
    fragment.extend_from_slice(&property_hash.to_le_bytes());
    fragment.extend_from_slice(value);
    let record_fragment_hex = fragment
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut write = json!({
        "recordIndex": record_index,
        "capture": "synthetic.pcapng",
        "laneIndex": 0,
        "sourceActorId": 43723073,
        "propertyPath": property_path,
        "propertyHash": format!("0x{property_hash:08x}"),
        "valueWidth": value.len(),
        "valueHex": value_hex,
        "recordFragmentHex": record_fragment_hex,
        "operation": operation,
    });
    if let Some(slot) = slot {
        write["slot"] = json!(slot);
    }
    match operation {
        "set-command" => {
            write["actorIdHex"] = json!("0xa0f06a04");
            write["commandId"] = json!(27140);
            write["classPath"] = json!("/Command/Game/Ability/Ability");
        }
        "set-category" => {
            write["categoryValue"] = json!(1);
            write["joinedCommandRecordIndex"] =
                joined_command_record_index.map_or(Value::Null, |record| json!(record));
        }
        "set-border" => write["borderValue"] = json!(value[0]),
        "clear" => {}
        _ => unreachable!(),
    }
    write
}

fn parsed_schema2_slot_context_fixture() -> SlotContextManifest {
    let mut fixture = slot_context_fixture();
    fixture.schema_version = 2;
    fixture
        .source_snapshots
        .client_structs
        .generator_hash_normalization =
        Some("UTF-8 text with CRLF and CR normalized to LF".to_owned());
    fixture.coverage.command_records = 9;
    fixture.coverage.nonzero_command_occurrences = 8;
    fixture.coverage.border_records = Some(1);
    fixture.coverage.relevant_write_records = Some(17);
    fixture.coverage.zero_command_writes = Some(1);
    fixture.coverage.state_partitions = Some(1);
    fixture.coverage.category_records = 7;
    fixture.coverage.category_hashes = 1;
    fixture.coverage.category_value_distribution = vec![CategoryValueDistribution {
        value: 1,
        occurrences: 7,
    }];
    fixture.coverage.stateful_category_observations = 6;
    fixture.coverage.category_writes_without_current_command = vec![CategoryWriteSummary {
        slot: 51,
        occurrences: 1,
    }];
    let mut writes = vec![synthetic_write(
        1,
        "clear",
        Some(39),
        &[0, 0, 0, 0],
        1,
        None,
    )];
    let mut record_index = 2;
    for category_record_index in [3, 5, 7, 9, 11, 13] {
        writes.push(synthetic_write(
            record_index,
            "set-command",
            Some(39),
            &[0x04, 0x6a, 0xf0, 0xa0],
            2,
            None,
        ));
        writes.push(synthetic_write(
            category_record_index,
            "set-category",
            Some(39),
            &[1],
            3,
            Some(record_index),
        ));
        record_index += 2;
    }
    writes.push(synthetic_write(
        14,
        "set-command",
        Some(39),
        &[0x04, 0x6a, 0xf0, 0xa0],
        2,
        None,
    ));
    writes.push(synthetic_write(
        15,
        "set-command",
        Some(43),
        &[0x04, 0x6a, 0xf0, 0xa0],
        4,
        None,
    ));
    writes.push(synthetic_write(16, "set-category", Some(51), &[1], 3, None));
    writes.push(synthetic_write(17, "set-border", None, &[32], 5, None));
    let writes_sha256 = sha256_hex(&serde_json::to_vec(&writes).unwrap());
    fixture.write_corpus = Some(WriteCorpus {
        scope: "observed-filtered-property-record-fragments".to_owned(),
        record_encoding: "valueWidth:u8 + propertyHash:u32le + value[valueWidth]".to_owned(),
        property_hash_encoding: "little-endian u32".to_owned(),
        state_partition: vec![
            "capture".to_owned(),
            "laneIndex".to_owned(),
            "sourceActorId".to_owned(),
        ],
        state_order: "increasing recordIndex within each partition".to_owned(),
        partial_state: true,
        initial_state: "unknown".to_owned(),
        final_state: "unasserted".to_owned(),
        server_authoritative: false,
        packet_replay: false,
        writes_sha256,
        writes,
    });
    fixture
}

fn refresh_rows_sha256(value: &mut Value) {
    value["rowsSha256"] = json!(sha256_hex(&serde_json::to_vec(&value["rows"]).unwrap()));
}

fn refresh_writes_sha256(value: &mut Value) {
    value["writeCorpus"]["writesSha256"] = json!(sha256_hex(
        &serde_json::to_vec(&value["writeCorpus"]["writes"]).unwrap()
    ));
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
        parse_compatibility_values(row[index("compatibility_percent_by_skill")].as_str()).unwrap();
    let command =
        command_document(&csv::StringRecord::from(row), 27_346, &compatibility, None).unwrap();
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
        profile["commandHandContext"]["defaultProducer"]["actorWorkBindings"]["commandCategory"]
            ["id"],
        3003
    );
    assert_eq!(
        profile["commandHandContext"]["defaultProducer"]["actorWorkBindings"]["identifierScope"],
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
    let command = command_document(&csv::StringRecord::from(row), 28623, &[], None).unwrap();
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
fn consumes_monster_attack_defaults_and_sparse_overrides_without_damage_inference() {
    let path = "/Command/Game/WeaponSkill/MonsterAttackWeaponSkill";
    let data = catalog_with_class(&[("23144", "Foul Bite", "Foul Bite JP")], path);
    let profiles = monster_attack_profiles_fixture();
    let report =
        build_report_with_monster_attack_profiles(&data, "23144", Some(&profiles)).unwrap();
    let command = &report["matches"][0];
    let profile = &command["subclassGetterProfile"];

    assert_eq!(profile["status"], "resolved");
    assert_eq!(profile["definedBy"], "MonsterAttackWeaponSkill");
    assert_eq!(profile["profileIdentity"]["version"], "1");
    assert_eq!(profile["profileIdentity"]["classPath"], path);
    assert_eq!(
        profile["profileIdentity"]["parentPath"],
        MONSTER_ATTACK_PARENT_PATH
    );
    assert_eq!(profile["source"]["bytes"], 59652);
    assert_eq!(profile["source"]["sha256"], MONSTER_ATTACK_SOURCE_SHA256);
    assert_eq!(profile["input"]["byteLength"], profiles.input_byte_length);
    assert_eq!(profile["input"]["sha256"], profiles.input_sha256);
    assert_eq!(profile["getters"]["getCommandInformation"]["selector"], 8);
    assert_eq!(profile["getters"]["getCommandInformation"]["result"], 1);
    assert_eq!(
        profile["getters"]["getCommandInformation"]["otherSelectors"],
        "nil"
    );
    assert_eq!(profile["getters"]["getFrequency"], 1);
    assert_eq!(profile["getters"]["getRangeWidth"], 2);
    assert_eq!(profile["getters"]["getRangeRotate"], 0);
    assert_eq!(profile["getters"]["getCommandRangeHeight"], 10);
    assert_eq!(
        profile["getters"]["getPartsDamageAdjust"]["result"],
        json!([1, 1])
    );
    assert_eq!(
        profile["getters"]["getPartsDamageAdjust"]["consumer"],
        "unresolved"
    );
    assert_eq!(command["damage"]["resolution"]["status"], "unresolved");
    assert_eq!(command["damage"]["magnitude"], 950);

    let default_data = catalog_with_class(&[("23145", "Other", "Other JP")], path);
    let report =
        build_report_with_monster_attack_profiles(&default_data, "23145", Some(&profiles)).unwrap();
    let defaults = &report["matches"][0]["subclassGetterProfile"];
    assert_eq!(defaults["status"], "resolved");
    assert_eq!(defaults["getters"]["getFrequency"], 1);
    assert_eq!(defaults["getters"]["getRangeWidth"], 2);
    assert_eq!(defaults["getters"]["getCommandRangeHeight"], 10);
    assert_eq!(
        defaults["getters"]["getPartsDamageAdjust"]["result"],
        json!([1, 1])
    );

    let override_data = catalog_with_class(&[("23486", "Override", "Override JP")], path);
    let report =
        build_report_with_monster_attack_profiles(&override_data, "23486", Some(&profiles))
            .unwrap();
    assert_eq!(
        report["matches"][0]["subclassGetterProfile"]["getters"]["getFrequency"],
        2
    );

    let information_override = subclass_getter_profile(path, 23164, Some(&profiles));
    assert_eq!(
        information_override["getters"]["getCommandInformation"]["result"],
        -1
    );
    assert_eq!(information_override["getters"]["getRangeRotate"], 90);
    let pair_override = subclass_getter_profile(path, 23114, Some(&profiles));
    assert_eq!(
        pair_override["getters"]["getPartsDamageAdjust"]["result"],
        json!([1, 0])
    );

    let report = build_report(&data, "23144").unwrap();
    assert_eq!(
        report["matches"][0]["subclassGetterProfile"]["status"],
        "unavailable"
    );
    assert_eq!(
        report["matches"][0]["subclassGetterProfile"]["reason"],
        "monster-attack-profiles-input-not-supplied"
    );

    let wrong_class = catalog_with_class(
        &[("23144", "Foul Bite", "Foul Bite JP")],
        "/Command/Game/WeaponSkill/AttackWeaponSkill",
    );
    let report =
        build_report_with_monster_attack_profiles(&wrong_class, "23144", Some(&profiles)).unwrap();
    assert_eq!(
        report["matches"][0]["subclassGetterProfile"]["status"],
        "unavailable"
    );
    assert_eq!(
        report["matches"][0]["subclassGetterProfile"]["reason"],
        "monster-attack-profiles-class-path-mismatch"
    );
}

#[test]
fn rejects_invalid_monster_attack_profile_manifest_identity_and_shapes() {
    let valid = monster_attack_profiles_fixture_value();
    for (field, value, message) in [
        ("version", json!("2"), "version"),
        ("gameVersion", json!("2.0"), "gameVersion"),
        ("extraction", json!("wrong"), "extraction"),
        (
            "classPath",
            json!("/Command/Game/Ability/CmnAbility"),
            "classPath",
        ),
        (
            "parentPath",
            json!("/Command/Game/WeaponSkill/WeaponSkillBaseClass2"),
            "parentPath",
        ),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            parse_monster_attack_profiles(&serde_json::to_vec(&invalid).unwrap())
                .unwrap_err()
                .contains(message),
            "expected {field} to be rejected"
        );
    }

    let mut unknown = valid.clone();
    unknown["unexpected"] = json!(true);
    assert!(
        parse_monster_attack_profiles(&serde_json::to_vec(&unknown).unwrap())
            .unwrap_err()
            .contains("unknown field")
    );

    let mut source = valid.clone();
    source["source"]["sha256"] = json!("bad");
    assert!(
        parse_monster_attack_profiles(&serde_json::to_vec(&source).unwrap())
            .unwrap_err()
            .contains("source")
    );

    let mut getter = valid.clone();
    getter["getterRules"]["unknownGetter"] = json!(1);
    assert!(
        parse_monster_attack_profiles(&serde_json::to_vec(&getter).unwrap())
            .unwrap_err()
            .contains("unknown field")
    );

    let mut duplicate = valid.clone();
    duplicate["getterRules"]["getFrequency"]["overrides"][0]["commandIds"] = json!([23486, 23486]);
    assert!(
        parse_monster_attack_profiles(&serde_json::to_vec(&duplicate).unwrap())
            .unwrap_err()
            .contains("commandIds")
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
    assert_eq!(by_id["schemaVersion"], 13);
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
    context.source_snapshots.client_data.command_catalog_sha256 = sha256_hex(&data);
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
fn validates_schema2_write_corpus_and_indexes_traces() {
    let fixture = parsed_schema2_slot_context_fixture();
    let report = build_loadout_report(&fixture, None).unwrap();
    assert_eq!(report["writeCorpus"]["writeCount"], 17);
    assert_eq!(report["writeCorpus"]["traceCount"], 1);
    assert_eq!(report["traces"][0]["firstRecordIndex"], 1);
    assert_eq!(report["traces"][0]["lastRecordIndex"], 17);
    assert_eq!(report["partialState"], true);
    assert_eq!(report["initialState"], "unknown");
    assert_eq!(report["finalState"], "unasserted");
    assert_eq!(report["packetReplay"], false);
    assert_eq!(report["serverAuthoritative"], false);

    let trace = build_loadout_report(&fixture, Some(0)).unwrap();
    assert_eq!(trace["trace"]["writes"].as_array().unwrap().len(), 17);
    assert_eq!(trace["trace"]["writes"][0]["operation"], "clear");
    assert_eq!(trace["trace"]["writes"][1]["operation"], "set-command");
    assert_eq!(trace["trace"]["writes"][2]["joinedCommandRecordIndex"], 2);
}

#[test]
fn materializes_exact_payload_and_stable_projection_report() {
    let fixture = parsed_schema2_slot_context_fixture();
    let materialized = materialize_command_loadout(&fixture, 0, Some((1, 16))).unwrap();
    assert_eq!(materialized.payload.len(), COMMAND_LOADOUT_PAYLOAD_SIZE);
    assert_eq!(materialized.payload[0], 123);
    assert!(materialized.payload[124..].iter().all(|byte| *byte == 0));
    assert_eq!(
        sha256_hex(&materialized.payload),
        "9f1ceb8833cfb669d0a04d20fd3231465138146c6ddda19cb28ff5554974facd"
    );
    assert_eq!(materialized.report["syntheticProjection"], true);
    assert_eq!(materialized.report["packetReplay"], false);
    assert_eq!(materialized.report["serverAuthoritative"], false);
    assert_eq!(materialized.report["targetMarkers"], "omitted");
    assert_eq!(materialized.report["partialState"], true);
    assert_eq!(materialized.report["initialState"], "unknown");
    assert_eq!(materialized.report["finalState"], "unasserted");
    assert_eq!(materialized.report["status"], "planned");
    assert_eq!(
        materialized.report["unresolved"][0],
        "category 2 is not observed"
    );
    assert_eq!(materialized.report["trace"]["index"], 0);
    assert_eq!(materialized.report["trace"]["capture"], "synthetic.pcapng");
    assert_eq!(materialized.report["recordRange"]["start"], 1);
    assert_eq!(materialized.report["recordRange"]["end"], 16);
    assert_eq!(materialized.report["fragments"][0]["payloadOffset"], 1);
    assert_eq!(materialized.report["fragments"][0]["payloadLength"], 9);
    assert_eq!(materialized.report["fragments"][15]["payloadOffset"], 118);
    assert_eq!(materialized.report["fragments"][15]["payloadLength"], 6);
    assert_eq!(materialized.report["streamLength"], 123);
    assert_eq!(materialized.report["paddingLength"], 12);
    assert_eq!(materialized.report["payloadSize"], 136);
    assert_eq!(
        materialized.report["payloadSha256"],
        "9f1ceb8833cfb669d0a04d20fd3231465138146c6ddda19cb28ff5554974facd"
    );

    for serialized in [
        serde_json::to_string(&materialized.report).unwrap(),
        serde_yaml::to_string(&materialized.report).unwrap(),
    ] {
        let report: Value = if serialized.starts_with('{') {
            serde_json::from_str(&serialized).unwrap()
        } else {
            serde_yaml::from_str(&serialized).unwrap()
        };
        assert_eq!(report["syntheticProjection"], true);
        assert_eq!(report["targetMarkers"], "omitted");
        assert_eq!(report["status"], "planned");
        assert_eq!(report["payloadSize"], 136);
        assert_eq!(
            report["payloadSha256"],
            materialized.report["payloadSha256"]
        );
    }
}

#[test]
fn cli_materializes_output_with_written_report_state() {
    let fixture = parsed_schema2_slot_context_fixture();
    let test_stem = format!(
        "xivl-materialize-success-{}-{}",
        std::process::id(),
        fixture.rows[0].command_id
    );
    let context_path = std::env::temp_dir().join(format!("{test_stem}.json"));
    let output_path = std::env::temp_dir().join(format!("{test_stem}.bin"));
    let context_string = context_path.to_string_lossy().into_owned();
    let output_string = output_path.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&context_path);
    let _ = std::fs::remove_file(&output_path);
    std::fs::write(&context_path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    let arguments = vec![
        "--slot-context".to_owned(),
        context_string,
        "--trace".to_owned(),
        "0".to_owned(),
        "--record-range".to_owned(),
        "1:16".to_owned(),
        "--output".to_owned(),
        output_string.clone(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    run_materialize_loadout(&arguments).unwrap();

    let bytes = std::fs::read(&output_path).unwrap();
    assert_eq!(bytes.len(), COMMAND_LOADOUT_PAYLOAD_SIZE);
    assert_eq!(bytes[0], 123);
    let mut expected = vec![0_u8; COMMAND_LOADOUT_PAYLOAD_SIZE];
    expected[0] = 123;
    let mut offset = COMMAND_LOADOUT_STREAM_OFFSET;
    for write in &fixture.write_corpus.as_ref().unwrap().writes[..16] {
        let fragment = parse_hex_bytes(write["recordFragmentHex"].as_str().unwrap()).unwrap();
        let end = offset + fragment.len();
        expected[offset..end].copy_from_slice(&fragment);
        offset = end;
    }
    assert_eq!(bytes, expected);
    assert!(bytes[offset..].iter().all(|byte| *byte == 0));

    let materialized = materialize_command_loadout(&fixture, 0, Some((1, 16))).unwrap();
    let written_report = materialized_report(&materialized, Some(&output_string));
    assert_eq!(written_report["status"], "written");
    assert_eq!(written_report["outputPath"], output_string);
    let planned_report = materialized_report(&materialized, None);
    assert_eq!(planned_report["status"], "planned");
    assert!(planned_report.get("outputPath").is_none());

    let _ = std::fs::remove_file(context_path);
    let _ = std::fs::remove_file(output_path);
}

#[test]
fn rejects_materialization_capacity_ranges_and_schema_without_output() {
    let fixture = parsed_schema2_slot_context_fixture();
    assert!(materialize_command_loadout(&fixture, 0, None)
        .unwrap_err()
        .contains("above the capture-observed 128-byte maximum"));
    assert!(materialize_command_loadout(&fixture, 0, Some((100, 101)))
        .unwrap_err()
        .contains("outside trace record range"));

    let schema1 = parsed_slot_context_fixture();
    assert!(materialize_command_loadout(&schema1, 0, Some((1, 1)))
        .unwrap_err()
        .contains("requires slot context schema 2"));

    let test_stem = format!(
        "xivl-materialize-invalid-{}-{}",
        std::process::id(),
        fixture.rows[0].command_id
    );
    let context_path = std::env::temp_dir().join(format!("{test_stem}.json"));
    let output = std::env::temp_dir().join(format!("{test_stem}.bin"));
    let context_string = context_path.to_string_lossy().into_owned();
    let output_string = output.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&context_path);
    let _ = std::fs::remove_file(&output);
    std::fs::write(&context_path, serde_json::to_vec(&fixture).unwrap()).unwrap();
    let arguments = vec![
        "--slot-context".to_owned(),
        context_string,
        "--trace".to_owned(),
        "0".to_owned(),
        "--record-range".to_owned(),
        "1:17".to_owned(),
        "--output".to_owned(),
        output_string,
    ];
    let failure = run_materialize_loadout(&arguments).unwrap_err();
    assert!(failure
        .message
        .contains("above the capture-observed 128-byte maximum"));
    assert!(!output.exists());
    let _ = std::fs::remove_file(context_path);

    let arguments = vec![
        "--slot-context".to_owned(),
        "missing-slot-context.json".to_owned(),
        "--trace".to_owned(),
        "0".to_owned(),
        "--record-range".to_owned(),
        "17:16".to_owned(),
        "--output".to_owned(),
        output.to_string_lossy().into_owned(),
    ];
    assert!(parse_materialize_arguments(&arguments).is_err());
    assert!(!output.exists());
}

#[test]
fn refuses_existing_materialization_output() {
    assert!(reject_existing_output(env!("CARGO_MANIFEST_DIR"))
        .unwrap_err()
        .contains("output path already exists"));
}

#[test]
fn rejects_schema2_write_digest_fragment_order_and_shape_drift() {
    let fixture = parsed_schema2_slot_context_fixture();
    let encoded = |value: SlotContextManifest| serde_json::to_vec(&value).unwrap();

    let mut version = serde_json::to_value(&fixture).unwrap();
    version["schemaVersion"] = json!(1);
    version.as_object_mut().unwrap().remove("writeCorpus");
    version["sourceSnapshots"]["clientStructs"]
        .as_object_mut()
        .unwrap()
        .remove("generatorHashNormalization");
    let parsed: SlotContextManifest = serde_json::from_value(version).unwrap();
    assert!(parsed
        .validate()
        .unwrap_err()
        .contains("schema 1 does not permit schema 2 fields"));

    let mut digest = serde_json::to_value(&fixture).unwrap();
    digest["writeCorpus"]["writes"][0]["valueHex"] = json!("01000000");
    assert!(
        parse_slot_context(&encoded(serde_json::from_value(digest).unwrap()))
            .unwrap_err()
            .contains("writesSha256")
    );

    let mut fragment = serde_json::to_value(&fixture).unwrap();
    fragment["writeCorpus"]["writes"][1]["recordFragmentHex"] = json!("040000000001020304");
    refresh_writes_sha256(&mut fragment);
    let parsed: SlotContextManifest = serde_json::from_value(fragment).unwrap();
    assert!(parsed.validate().unwrap_err().contains("record fragment"));

    let mut order = serde_json::to_value(&fixture).unwrap();
    order["writeCorpus"]["writes"][1]["recordIndex"] = json!(1);
    refresh_writes_sha256(&mut order);
    let parsed: SlotContextManifest = serde_json::from_value(order).unwrap();
    assert!(parsed.validate().unwrap_err().contains("record order"));

    let mut shape = serde_json::to_value(&fixture).unwrap();
    shape["writeCorpus"]["writes"][0]["actorIdHex"] = json!("0xa0f06a04");
    refresh_writes_sha256(&mut shape);
    let parsed: SlotContextManifest = serde_json::from_value(shape).unwrap();
    assert!(parsed.validate().unwrap_err().contains("operation shape"));
}

#[test]
fn reports_missing_slot_context_observation_without_inference() {
    let data = catalog(&[("27141", "Other", "Other")]);
    let mut context = parsed_slot_context_fixture();
    context.source_snapshots.client_data.command_catalog_sha256 = sha256_hex(&data);
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
    for (field, expected) in [("schemaVersion", 0), ("schemaVersion", 3)] {
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
            value["rows"][0]["slotObservations"][0]["categoryObservations"][0]["value"] = json!(2)
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
    context.source_snapshots.client_data.command_catalog_sha256 = sha256_hex(&data);
    context.rows[0].class_path = "/Command/Game/Ability/Other".to_owned();
    context.rows_sha256 =
        sha256_hex(&serde_json::to_vec(&serde_json::to_value(&context.rows).unwrap()).unwrap());
    assert!(
        build_report_with_slot_context(&data, "27140", Some(&context))
            .unwrap_err()
            .contains("identity")
    );
}
