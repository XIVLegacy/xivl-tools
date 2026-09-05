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

## Foul Bite subclass getters

In catalog v3, command 23144 resolves to Foul Bite at
`/Command/Game/WeaponSkill/MonsterAttackWeaponSkill`. The catalog source is
`xivl-client-data:derived/command_battle_params.csv`, sha256
`bc043bbd5558916a971de4d3a3a8dac5ec9d8ca36571bb534d0e964cd0b55d6a`.
That subclass returns the following command-specific values:

| Getter | Input | Result |
|---|---:|---:|
| `getCommandInformation` | selector 8 | 1 |
| `getFrequency` | - | 1 |
| `getRangeWidth` | - | 2 |
| `getRangeRotate` | - | 0 |
| `getCommandRangeHeight` | - | 10 |
| `getPartsDamageAdjust` | - | 1, 1 |

Other `getCommandInformation` selectors have no retained return in this
subclass. The consumer and combination rule for `getPartsDamageAdjust` remain
unresolved. These values therefore describe exact Lua getter results rather
than a damage formula. They do not establish how the parts adjustment is
consumed or any final damage amount.

`subclassGetterProfile` exposes this row only when both command id 23144 and
the exact class path match. Other commands report that no command-specific
getter profile has been promoted. `damage.resolution` continues to mark the
native magnitude scale and combination step unresolved.

The source is
`xivl-client-scripts:lua/scripts/command/game/weaponskill/monsterattackweaponskill.lua`,
sha256 `d5b8e884aad2ca2cfe5cfa96cf5e029d975a32bb0bc1742873ded2f3a78b668e`.
The `getCommandInformation` selector return is at lines 3324-3330. The other
definitions are `getFrequency` lines 3333-3359, `getRangeWidth` lines 3362-3378,
`getRangeRotate` lines 3381-3433, `getCommandRangeHeight` lines 3436-3450,
and `getPartsDamageAdjust` lines 3453-3469. The command-23144 assignments are
at lines 1125-1142.

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
fields where their inputs are absent. The JSON/YAML report schema is version 12,
including [compatibility profiles](command-compatibility-profiles.md) and exact
command-specific Lua getter profiles. The CLI
consumes the explicit input as supplied and records its SHA-256; it does not
independently authenticate class-path values.

`cargo test --locked -p xivl-cli command_inspect` checks the distinct getter
overrides, inherited parameter 4, AncientMagic's overridden parameter 4, the
exact Foul Bite subclass profile, declared ancestry, non-applicable paths,
unknown paths, and legacy input.
The producer's synthetic tests distinguish command-id joins from column-36
joins. No decoded corpus bytes are embedded in CLI fixtures.
