# Command parameter getter profiles

`inspect-command` selects `getCommandParam1` through `getCommandParam4` and
their `LevelAdjustGrow` getters for each known game-command class path.
All eight getters are inherited from `GameCommandBaseClass`; the frozen
command corpus contains no subclass definitions of these methods.
The existing [level profiles](command-formula-profiles.md) select the separate
level limits and blend overrides used inside contextual parameter calls.

## Call modes

The four parameter getters share the following recovered control flow.
Argument positions below exclude the command receiver. The first three
arguments form the context tested by the method. The first is the actor, the
second is the hand selector, and the third is the target whose `_isAlive`
method is called. A fourth optional argument is not used.

| Call mode | Recovered behavior | Report kind |
|---|---|---|
| Any of the first three arguments is nil | Read the raw sheet base, set compatibility/TP factors to 1, and skip adjustment helpers | `catalog-input` |
| All three are present and the target's `_isAlive` result is truthy | Apply level adjustment, compatibility, and TP factors | `actor-target-required` |
| All three are present and `_isAlive` is falsy | The recovered body reaches multiplication with factors uninitialized | `unresolved` |

The last row is a boundary in the extracted Lua evidence. The CLI does not
repair it into the missing-context fallback or claim a confirmed retail crash.
It reports reason `recovered-factors-uninitialized` and calculates no value.

A `flat` parameter classification concerns the raw grow selector only.
It does not prove that the contextual getter returns the raw base: compatibility
can still change the result. Conversely, a receiver-only call takes the raw
base path even if the retained grow selector is nonnegative. The HP calls
described in [cost profiles](command-cost-profiles.md) use this mode.
The general `parameterExpression` is scoped to complete context with a live
target, rather than every invocation of a parameter getter.

## Report contract

The JSON/YAML report schema is version 8. Each match's `parameterProfile`
reports getter selection, not evaluated parameter values:

- `status: resolved` and `definedBy: GameCommandBaseClass` identify the owner
  of both the parameter and grow-selector getters in the selected hierarchy.
- Each `getters` entry names its parameter number, method, original CSV
  `inputField`, and `growSelectorMethod`. The input value remains in the
  corresponding `parameters` entry's `base` field; no missing input is filled.
- `callModes` reports the conditions and kinds from the table above. It does
  not infer actor state or select a call mode from the catalog alone.
- `argumentRoles` records the actor, hand-selector, target, and unused
  positions established by the shared getter bodies.

The exact-path lookup covers 70 game-command paths and 1,606 frozen catalog
rows. Four known paths outside the GameCommandBaseClass hierarchy report
`not-applicable`. Unknown paths and missing identity, including legacy v1 CSV
input, report `unresolved` without invented getters or call modes. Identity
and ancestry use the [existing source pins](command-profile-sources.md).
The CLI trusts the explicit catalog's class-path field and records its hash.

## Source evidence

The getter bodies and call modes are from
`xivl-client-scripts:lua/scripts/command/game/gamecommandbaseclass.lua`, sha256
`75f366ca597f77a8e4b506fa8d7b214171cfdbb8d913fa12aa685d72a0b3256b`:

| Parameter | Getter lines | Grow-selector getter lines | Base/grow/compatibility/TP sheet columns |
|---|---|---|---|
| 1 | 1596-1665 | 1508-1527 | 43 / 42 / 44 / 45 |
| 2 | 1668-1737 | 1530-1549 | 48 / 47 / 49 / 50 |
| 3 | 1740-1809 | 1552-1571 | 53 / 52 / 54 / 55 |
| 4 | 1812-1881 | 1574-1593 | 58 / 57 / 59 / 60 |

Each grow-selector getter returns nil for a negative raw selector; otherwise
it calls the actor's `judgeGrowColumn` with the target and raw selector.
The catalog's grow field is therefore the raw selector, not a claim about the
selector used by every actor/target pairing. Native `getGrowData` values remain
unresolved. Compatibility and TP helpers are at 1327-1347 and 1350-1362;
level adjustment is at 1375-1433 in the same source.

No contextual call to any of the four getters appears in the complete frozen
Lua corpus. Three subclass HP-cost methods call `getCommandParam3` with only
the receiver and therefore take the raw-input path. The caller that supplies
the actor, hand selector, and target remains native or dynamically dispatched;
static Lua establishes the argument flow but not the runtime values.

The CSV names and column mapping are defined by
`xivl-client-data:tools/build_command_battle_params.py`, sha256
`e37379579f6c2ee4d24e2962e195bc40baf8407820c5a0123415456bb5c23e93`,
lines 63-68 and 196-199. These profiles promote method and input identities;
they do not copy the extracted implementation or execute Lua.

`cargo test --locked -p xivl-cli command_inspect` checks inherited owners and
all four input mappings, distinguishes call modes from flat growth coverage,
preserves the non-live-target boundary, and checks unknown, inapplicable, and
legacy identity using authored inputs.
