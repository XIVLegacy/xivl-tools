# Command level-adjustment profiles

`inspect-command` recognizes all 74 class paths in the frozen command catalog.
The 70 paths that inherit `GameCommandBaseClass` cover 1,606 command rows,
including all 21 native-grow rows. Their selected getters resolve to constants.
Four paths covering one row each are outside that hierarchy and report
`not-applicable`. Row 0 has no class path and remains `unresolved`.

The selected getters are `getCommandLevelAdjustLevelMax` and the eight
`getCommandParamNAdjustForHighLevelUse` / `LowLevelUse` methods. Defaults are
distance limits -1 / 15, four low blends of 1, and four high blends of 0.7.
The following exact paths override those defaults:

| Path | Low/high distance limits | Low blends, parameters 1-4 | High blends, parameters 1-4 |
|---|---|---|---|
| `/Command/Game/AttackCommand` | -1 / -1 | 1, 1, 1, 1 | 0.7, 0.7, 0.7, 0.7 |
| `/Command/Game/Basic/MonsterAttackCommand` | -1 / -1 | 1, 1, 1, 1 | 0.7, 0.7, 0.7, 0.7 |
| `/Command/Game/ShotCommand` | -1 / -1 | 1, 1, 1, 1 | 0.7, 0.7, 0.7, 0.7 |
| `/Command/Game/ThrowCommand` | -1 / -1 | 1, 1, 1, 1 | 0.7, 0.7, 0.7, 0.7 |
| `/Command/Game/Magic/AncientMagic` | -1 / 10 | 0, 0, 0, 0 | 0, 0, 0, 0 |
| `/Command/Game/Magic/CmnAttackMagic` | -1 / 10 | 1, 1, 1, 1 | 0.25, 0, 0, 0.7 |
| `/Command/Game/Magic/CmnDrainMagic` | -1 / 10 | 1, 1, 1, 1 | 0, 0, 0, 0.7 |
| `/Command/Game/Magic/CmnBadStatusMagic` | -1 / 15 | 1, 1, 1, 1 | 0, 0, 0, 0.7 |
| `/Command/Game/Magic/CmnCureMagic` | -1 / 15 | 1, 1, 1, 1 | 0, 0, 0, 0.7 |
| `/Command/Game/Magic/CmnGoodStatusMagic` | -1 / 15 | 1, 1, 1, 1 | 0, 0, 0, 0.7 |

A distance limit of -1 disables the corresponding distance cap, not the grow
calculation. AncientMagic overrides all eight blends, including parameter 4.
The other magic overrides leave parameter 4 inherited from the base class.

Each `levelAdjustmentProfile` reports the effective values, the declared
inheritance chain through `GameCommandBaseClass`, and each getter's defining
class. The top-level `formulaModel.levelAdjustment` records the base defaults
for comparison. It is not the selected command's effective profile.

## Scope and exceptions

The four `not-applicable` paths are:

- `/Command/AutoAttackTargetChangeCommand`
- `/Command/DebugInputCommand`
- `/Command/Game/BonusPointCommand`
- `/Command/ItemCommand`

Their profiles report the declared chain and reason
`outside-game-command-hierarchy`, without fabricated limits or blends. This
classification concerns the selected Lua model, not all possible native
methods. A `/Game/` directory alone does not imply GameCommandBaseClass
inheritance; conversely, some commands outside that directory inherit it.

Unknown paths, including paths absent from the retained catalog, stay
unresolved. There is no prefix or class-name fallback. The lookup uses the
declared parents of each exact script path; this matters when two source
files declare the same leaf class name.

These level profiles do not evaluate the complete parameter formula,
native grow values, or damage. A zero blend does not remove the
native grow requirement: the recovered caller still performs the lookup and
division. [Cost getter profiles](command-cost-profiles.md) separately identify
selected HP/MP/TP getters and their actor/runtime dependencies.
[Parameter getter profiles](command-parameter-profiles.md) distinguish raw
input calls from contextual adjustment and unresolved non-live-target calls.

## MonsterAttackWeaponSkill rule manifest

The optional `--monster-attack-profiles <json>` input supplies retained Lua
getter rules for the exact class path
`/Command/Game/WeaponSkill/MonsterAttackWeaponSkill`. It is a bounded JSON
manifest with `version: "1"`, `gameVersion: "1.23b"`, extraction
`"2012.09.19.0001"`, the exact `classPath` and `parentPath`, a source object,
summary counts, six `getterRules`, and nonempty `unresolved` notes. Each getter
rule contains its compact default and grouped sparse command-id overrides; the
CLI selects the matching group for the catalog row before producing its
`subclassGetterProfile`.

The source object records the producer script, its byte length, line count,
manifest, and 64-digit `sha256`. The report carries that source object and the
manifest's profile identity (version, game version, extraction, class path,
parent path, and `getterRulesSha256`), so a resolved getter profile remains
tied to the exact producer output. It also records the supplied profile file's
own byte length and SHA-256 under `subclassGetterProfile.input`. The consumer
validates the strict manifest shape, version, game and extraction identity,
source metadata, summary counts, getter-rule digest, and class path before
evaluating it. A different catalog class path cannot select these rules.

The values describe exact Lua getter results, rather than a damage formula.
The consumer and combination rule for `getPartsDamageAdjust` remain
unresolved, and `damage.resolution` continues to mark the native magnitude
scale and combination step unresolved. Without the option,
`subclassGetterProfile` remains `unavailable`; the CLI does not restore a
command-specific rule from a built-in id check.

## Identity and inheritance evidence

The catalog producer joins command row ids to static-actor class paths.
`CommandBaseClass.getCommandId` returns `_getStaticActorID`, and the game
command sheet getters use that key. The identity finding is
`xivl-client-data:docs/command-script-identity.md`, sha256
`52b5e0f5585f3d937b4366d9f8bbfe224461abbeece99b9fd0994c9e057314d3`.
The corresponding static-actor product is
`xivl-client-data:manifests/staticactor_class_paths.json`, sha256
`d612438827e5997422ab6f64a807e567ddf1b953c532e8a319d67b93c53c9db0`.

[Command profile source identities](command-profile-sources.md) pins all
selected scripts and ancestors, their declared parent paths, and the getter
definition lines. Every parent chain and selected getter was checked against
those exact extracted sources. A class without a selected override inherits
the corresponding getter from its declared ancestor.

## Input and verification contract

The v2 CSV header appends `lua_class_path` after `effect_block_raw`; v3 appends
`compatibility_percent_by_skill`. All earlier columns retain their positions.
Legacy v1 and v2 catalogs remain accepted with explicit unresolved profile
fields where their inputs are absent. The JSON/YAML report schema is version 13,
including [compatibility profiles](command-compatibility-profiles.md) and exact
command-specific Lua getter profiles. The CLI validates the explicit manifest
identity and records its source metadata and getter-rule SHA-256; it does not
infer a profile for a different class path.

`cargo test --locked -p xivl-cli command_inspect` checks the distinct getter
overrides, inherited parameter 4, AncientMagic's overridden parameter 4, the
exact MonsterAttackWeaponSkill rule manifest, declared ancestry, non-applicable paths,
unknown paths, and legacy input.
The producer's synthetic tests distinguish command-id joins from column-36
joins. No decoded corpus bytes are embedded in CLI fixtures.
