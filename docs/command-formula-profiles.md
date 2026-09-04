# Command level-adjustment profiles

`inspect-command` selects recovered level-limit and parameter-blend getters
from the catalog's exact `lua_class_path`. It recognizes these five paths,
which cover all 21 native-grow rows and 520 total rows in the frozen command
catalog. Other paths return an unresolved profile, even when the command name
resembles a known class.

| Path | Low/high distance limits | High blends, parameters 1-4 |
|---|---|---|
| `/Command/Game/Ability/CmnAbility` | -1 / 15 | 0.7, 0.7, 0.7, 0.7 |
| `/Command/Game/WeaponSkill/MonsterAttackWeaponSkill` | -1 / 15 | 0.7, 0.7, 0.7, 0.7 |
| `/Command/Game/Magic/CmnAttackMagic` | -1 / 10 | 0.25, 0, 0, 0.7 |
| `/Command/Game/Magic/CmnBadStatusMagic` | -1 / 15 | 0, 0, 0, 0.7 |
| `/Command/Game/Magic/CmnCureMagic` | -1 / 15 | 0, 0, 0, 0.7 |

All four low blends are 1. A low distance limit of -1 disables the low-side
distance cap; it does not disable the grow calculation. The per-command
`levelAdjustmentProfile` records the effective values, inheritance chain, and
defining class. The top-level `formulaModel.levelAdjustment` remains the
base defaults for comparison, not the selected command's effective profile.

This profile resolves only `getCommandLevelAdjustLevelMax` and the eight
`getCommandParamNAdjustForHighLevelUse` / `LowLevelUse` getters. It does not
evaluate the complete parameter formula, subclass cost overrides, native
grow values, or damage. In particular, a zero high blend does not remove the
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

Each selected leaf declares its parent at lines 1-8. The respective
`AbilityBaseClass`, `MagicBaseClass`, or `WeaponSkillBaseClass` derives from
`BattleCommandBaseClass`, which derives from `GameCommandBaseClass`. These
intermediate classes do not override the selected getters. `CmnAbility` and
`MonsterAttackWeaponSkill` inherit all selected getters. The other three
leaves override high blends 1-3; `CmnAttackMagic` also overrides the limits.

The following exact sources belong to extraction `2012.09.19.0001`. Paths are
relative to `xivl-client-scripts:lua/scripts/`; their identities are retained
in that repository's `manifests/scripts.json`.

| Source | Relevant lines | SHA-256 |
|---|---|---|
| `command/game/gamecommandbaseclass.lua` | 1365-1505, limits and blends | `75f366ca597f77a8e4b506fa8d7b214171cfdbb8d913fa12aa685d72a0b3256b` |
| `command/game/battlecommandbaseclass.lua` | 1-8, inheritance; whole file, no selected override | `0eb0b8c77b05128461d94ca1a9bee9b65bccf397ab8efd60903c448915d1e757` |
| `command/game/ability/abilitybaseclass.lua` | 1-8, inheritance; whole file, no selected override | `2278cde95818f9a29a692d3a4fdecf947b40b7ad5ebf3f9348067b4dfc1d8674` |
| `command/game/magic/magicbaseclass.lua` | 1-8, inheritance; whole file, no selected override | `83729c1db192e8ef524e5d773d92e814c27b1ccd51be3640d7e57125f3d3c90f` |
| `command/game/weaponskill/weaponskillbaseclass.lua` | 1-8, inheritance; whole file, no selected override | `3b81ee1cd014cf1162d9c55e2d441776e804691660eaa32faa96a621af302efa` |
| `command/game/ability/cmnability.lua` | 1-8, inheritance; whole file, no selected override | `f91a2994361aca4cbbc5e3923b3a50cace75a41efe4e7784d7759231bb8c08aa` |
| `command/game/magic/cmnattackmagic.lua` | 1-45, inheritance, limits, high blends 1-3 | `50c661bd35dcd77316108eeef8d63f85cb27800ed27774649e9b5923e3adc30d` |
| `command/game/magic/cmnbadstatusmagic.lua` | 1-35, inheritance and high blends 1-3 | `2106461769b339c9d0f2636fce4e8fee13383aec196f708a47aabd570e985332` |
| `command/game/magic/cmncuremagic.lua` | 1-35, inheritance and high blends 1-3 | `f4bb71c9807b11521e9435e80b78f7951603e56328f4cdb1811cd61243ed1f47` |
| `command/game/weaponskill/monsterattackweaponskill.lua` | 1-8, inheritance; whole file, no selected override | `d5b8e884aad2ca2cfe5cfa96cf5e029d975a32bb0bc1742873ded2f3a78b668e` |

## Input and verification contract

The v2 CSV header appends `lua_class_path` after `effect_block_raw`; all
earlier columns retain their positions. Legacy v1 catalogs are accepted with
missing identity and an unresolved profile. The JSON/YAML report schema is
version 3. The CLI consumes the explicit input as supplied and records its
SHA-256; it does not independently authenticate its class-path values.

`cargo test --locked -p xivl-cli command_inspect` checks every selected
profile, inherited parameter 4, exact-path matching, unknown paths, and legacy
input. The producer's synthetic tests distinguish a command-id join from a
column-36 join. No corpus bytes are embedded in CLI fixtures.
