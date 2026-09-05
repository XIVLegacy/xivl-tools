# Command cost getter profiles

`inspect-command` reports `costProfile` alongside the original `costs` inputs.
The profile selects recovered Lua getters by the exact `lua_class_path` and,
for conditional HP overrides, the command id. `resolved` means getter selection
is known; it does not mean the command's final cost has been calculated.

## Getter results

| Getter | Defining class | Reported result |
|---|---|---|
| `getCommandHPCost` | `GameCommandBaseClass` | Constant 0 |
| `getCommandHPCost` | `CmnAbility` | `p3_base` for id 27591; otherwise 0 |
| `getCommandHPCost` | `CmnAttackMagic` | `p3_base` for id 28623; otherwise 0 |
| `getCommandHPCost` | `CmnCureMagic` | `p3_base` for id 28669; otherwise 0 |
| `getCommandMPCost` | `GameCommandBaseClass` | `mp_cost` passed to the actor's `calculateCommandCost` |
| `getCommandTPCost` | `GameCommandBaseClass` | `tp_cost` |

The three HP override paths are `/Command/Game/Ability/CmnAbility`,
`/Command/Game/Magic/CmnAttackMagic`, and `/Command/Game/Magic/CmnCureMagic`.
Each special branch calls `getCommandParam3` with only the receiver. The
inherited parameter getter skips its actor/target adjustment block on that
call and uses sheet parameter 3 with unit factors. This does not imply that
parameter 3 is generally flat or that its native grow selector can be ignored
when evaluating it with actor arguments.

Empty branches in the recovered HP methods do not assign another cost.
The profile does not extend the special behavior to adjacent command ids.
No MP or TP getter overrides were found in the selected command hierarchies.
The available `CharaBaseClass.calculateCommandCost` implementation depends on
`getStateMainSkillLevel` and rounds upward. The command identity alone does
not establish the receiver actor's effective implementation or level.

`result.kind` distinguishes `constant`, `catalog-input`, and `actor-required`.
A catalog-input result names the original CSV `field`; it does not duplicate,
validate, or fill in that input. `p3_base` appears under `parameters` number 3
in the report; `mp_cost` and `tp_cost` appear under `costs.mp` and `costs.tp`.
The `costs` object has scope `catalog-inputs`; its HP field remains the supplied
base default, even when the selected HP getter uses parameter 3.

## Runtime wrapper boundary

The separate `getCostHP`, `getCostMP`, and `getCostTP` wrappers are inherited
from `GameCommandBaseClass`. They consume getter results and actor methods:

| Wrapper | Additional actor methods |
|---|---|
| `getCostHP` | `getHP` |
| `getCostMP` | `getForceCostMPForCaster`, `getMP` |
| `getCostTP` | `getTP`, `getForceCostTPForCaster` |

These wrappers can return -1 or -2 sentinels. HP rejects a cost greater than
or equal to current HP; MP rejects a cost greater than current MP and floors
the accepted result. The recovered TP wrapper returns current TP on its
sufficient-TP branch, not the sheet cost. The CLI records wrapper dependencies
with `runtime-required` and does not evaluate them or reinterpret sentinels as
resource deductions. MP and TP force-cost modifiers belong to this wrapper
stage, after their corresponding getters.

## Identity and source evidence

The [level profile evidence](command-formula-profiles.md#identity-and-inheritance-evidence)
establishes the command-id join. [Source identities](command-profile-sources.md)
pin the exact scripts and declared ancestry used by both profiles. The 70
known GameCommandBaseClass paths cover 1,606 rows in the frozen catalog.
The four known paths outside that hierarchy report `not-applicable`; missing
or unrecognized paths report `unresolved` and receive no fabricated getters.
The CLI records the supplied catalog's SHA-256 and trusts its class-path field.

The CSV field mapping is defined by
`xivl-client-data:tools/build_command_battle_params.py`, sha256
`e37379579f6c2ee4d24e2962e195bc40baf8407820c5a0123415456bb5c23e93`:
lines 191-198 emit the base HP default, basic-sheet MP/TP columns 114/115,
and game-command-sheet parameter 3 column 53.

Cost facts use these immutable source identities from extraction
`2012.09.19.0001`; line numbers locate definitions within those exact bytes:

- `xivl-client-scripts:lua/scripts/command/game/gamecommandbaseclass.lua`,
  sha256 `75f366ca597f77a8e4b506fa8d7b214171cfdbb8d913fa12aa685d72a0b3256b`:
  parameter 3 at 1740-1809; HP getter/wrapper at 1906-1944;
  MP at 1947-1993; TP at 1996-2038.
- `xivl-client-scripts:lua/scripts/command/game/ability/cmnability.lua`,
  sha256 `f91a2994361aca4cbbc5e3923b3a50cace75a41efe4e7784d7759231bb8c08aa`:
  HP override at 11-37.
- `xivl-client-scripts:lua/scripts/command/game/magic/cmnattackmagic.lua`,
  sha256 `50c661bd35dcd77316108eeef8d63f85cb27800ed27774649e9b5923e3adc30d`:
  HP override at 48-70.
- `xivl-client-scripts:lua/scripts/command/game/magic/cmncuremagic.lua`,
  sha256 `f4bb71c9807b11521e9435e80b78f7951603e56328f4cdb1811cd61243ed1f47`:
  HP override at 38-62.
- `xivl-client-scripts:lua/scripts/chara/charabaseclass_ffxivbattle.lua`,
  sha256 `d1db23d11f911ea3205ca72868cc9fca22cefe0d3f88a6706cbb035d92130bda`:
  actor cost helper at 41-86.

## Report and verification contract

JSON and YAML share report schema version 7. Legacy v1 CSV input is accepted
with unresolved cost identity. No actor inputs, Lua evaluator, or native
runtime capture are added by this profile.

`cargo test --locked -p xivl-cli command_inspect` exercises exact class and id
selection, neighboring ids, raw HP input conflicts, native parameter growth
remaining distinct from receiver-only HP use, MP actor dependence, TP input
selection, wrapper dependencies, and missing or inapplicable identity. These
tests use authored inputs, not decoded corpus rows.
