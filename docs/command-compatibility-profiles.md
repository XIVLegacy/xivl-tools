# Command compatibility profiles

`inspect-command` joins each known game command to the compatibility matrix
row selected by its `compat_key`. Catalog version 3 appends
`compatibility_percent_by_skill` after `lua_class_path`. The field contains
the selected row's 44 retained skill cells in order as
`1=value;...;44=value`.

Each `compatibilityProfile` keeps the static matrix input separate from the
actor-dependent selection performed by the Lua getters. A resolved profile
contains the row key and, for each retained skill id:

- `percent`: the raw signed integer from `compatibility.csv`;
- `matrixFactor`: the raw percentage divided by 100;
- `cappedFactor`: `min(1, matrixFactor)`, as used by the fallback path.

The frozen command catalog uses 19 compatibility keys. Every key selects a
present matrix row with 44 populated cells. These counts describe the supplied
catalog and do not establish that actor skill ids outside 1 through 44 are
invalid. The CLI validates the ordered serialization but does not discover or
load `compatibility.csv` itself.

## Actor-dependent selection

`getCommandCompatibilityByHand` selects the actor's sub-skill when its hand
argument equals 2; otherwise it selects the actor's main skill. The selected
skill then follows these branches in `getCommandCompatibility`:

| Condition | Result |
|---|---:|
| Selected skill id is 0 | 0 |
| Selected skill matches the command's main skill | 1 |
| Actor reports the selected skill as a job and its main skill matches the command's main skill | 1 |
| Otherwise | Capped matrix factor for the selected skill |

The profile records the actor methods `getStateMainSkill`,
`getStateMainSkillForSub`, and `isJob` and leaves evaluation
`actor-required`. Static command data cannot choose the actor skill, determine
the hand argument at a call site, or decide which shortcut applies.

The compatibility factor used by a contextual parameter getter remains
`1 - (1 - compatibilityByHand) * rawCompatibilityAdjust`. A raw adjustment of
zero bypasses actor compatibility and yields 1. See
[parameter getter profiles](command-parameter-profiles.md) for the surrounding
call modes.

## Identity and evidence

The compatibility getter family is inherited from `GameCommandBaseClass` for
all 70 known game-command paths. The frozen Lua corpus contains no subclass
definitions or aliases for these getters. Four known paths outside that
hierarchy report `not-applicable`. Missing or unrecognized class paths report
`unresolved`; a known game path in an older catalog without the matrix field
reports reason `missing-compatibility-data`.

The getter facts come from
`xivl-client-scripts:lua/scripts/command/game/gamecommandbaseclass.lua`, sha256
`75f366ca597f77a8e4b506fa8d7b214171cfdbb8d913fa12aa685d72a0b3256b`:

- `getCommandCompatibilityKey` at 969-977 reads basic-sheet column 40;
- `getCommandCompatibilityData` at 980-995 reads matrix column
  `8 + (skillId - 1)` and divides by 100;
- `getCommandCompatibility` at 998-1037 applies the shortcuts and cap;
- `getCommandCompatibilityByHand` at 1040-1062 selects main or sub skill;
- `getCommandCompatibilityWithAdjust` at 1327-1347 applies the raw parameter
  adjustment.

The catalog projection is owned by
`xivl-client-data:tools/build_command_battle_params.py`, sha256
`e37379579f6c2ee4d24e2962e195bc40baf8407820c5a0123415456bb5c23e93`.
It joins command basic-sheet column 40 to the exact compatibility row and
serializes columns 8 through 51. The source extraction pins
`compatibility.csv`, sha256
`0b084a0aa4ab01ab2e2a3de4ba0ee9c97257d5766f7855841d705854c0d862fe`,
in `xivl-client-data:manifests/tables.json`, sha256
`239cb63fb00de5f434b92924696c043cb80ddcd9abc7f334075afb6b28d5626d`.

## Input and verification contract

JSON and YAML share report schema version 7. Catalog v1 lacks class identity;
catalog v2 has class identity without compatibility values; both remain
readable with explicit unresolved profiles. Catalog v3 requires compatibility
values exactly when its key is present. A present matrix must contain skill ids
1 through 44 once each in order, with signed 8-bit integer percentages.

`cargo test --locked -p xivl-cli command_inspect` checks matrix parsing,
percentage conversion and capping, shortcut metadata, actor dependencies,
known hierarchy exceptions, and v1/v2 compatibility. Tests use authored data;
no decoded matrix values are embedded in the repository.
