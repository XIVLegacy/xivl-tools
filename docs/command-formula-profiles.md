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

These profiles do not evaluate the complete parameter formula, subclass cost
overrides, native grow values, or damage. A zero blend does not remove the
native grow requirement: the recovered caller still performs the lookup and
division. Raw sheet costs and the base HP default remain inputs, not effective
subclass costs.

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

The v2 CSV header appends `lua_class_path` after `effect_block_raw`; all
earlier columns retain their positions. Legacy v1 catalogs are accepted with
missing identity and an unresolved profile. The JSON/YAML report schema is
version 4, including the `not-applicable` profile status. The CLI consumes the
explicit input as supplied and records its SHA-256; it does not independently
authenticate class-path values.

`cargo test --locked -p xivl-cli command_inspect` checks the distinct getter
overrides, inherited parameter 4, AncientMagic's overridden parameter 4,
declared ancestry, non-applicable paths, unknown paths, and legacy input.
The producer's synthetic tests distinguish command-id joins from column-36
joins. No decoded corpus bytes are embedded in CLI fixtures.
