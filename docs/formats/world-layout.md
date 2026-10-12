# World layout: MapLayout and lyb

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

A MapLayout resource holds a resource table and an embedded SEDB `lyb`
scene graph. `xivl export-zones` reads both to resolve placed collision
resources. [Zone geometry](zone-geometry.md) describes transform composition
and export behavior.

## First-hand retail observations

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001),
all files under `data/` carrying `MapLayoutResourceData` at offset 0.
Reproduce with:

```bash
python tools/research/census_layout.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints structural counts,
offsets, short class/tag vocabulary, and resource identities rather than
client bytes or bulk text.

The exemplar is `data/29/D9/00/01.DAT`, resource `0x29D90001`, 1197104
bytes, SHA-256
`bc40d7ff57ab8998978532cce4564e2488ed81d6d10b8f7ede3e1586b0b26523`.
Counts below cover the install unless identified as exemplar counts.

## Discovery and outer header

The census found 287 files with the magic at offset 0. They occur under
top-level `data/` directories `03`, `25`, `28`, `29`, `2B`, `61`, `72`,
`7F`, `89`, `8B`, `8C`, `92`, `A0`, and `AB`. A reader locates a layout
by resource id using the [resource path mapping](sedb-res.md#resource-id-to-dat-path).
`xivl export-zones` accepts `--layout <resource-id>`.

All integer fields are little-endian. Offsets in this table are file-relative.

```text
0x00  char[24] magic        "MapLayoutResourceData", padded with zeros
0x18  char[8]  version      "1.1.0" on all 287 layouts
0x20  u32      sceneOffset  embedded SEDB lyb start
0x24  u32      entryCount   number of resource-table rows
0x28  byte[24] reserved     zero on all 287 layouts
0x40  ...      rows         entryCount rows, 32 bytes each
```

On all 287 layouts, `0x40 + 32 * entryCount == sceneOffset` and
`sceneOffset + SEDB totalSize == file size`. The exemplar has 1227 rows,
a scene at `0x99A0`, and a SEDB extent of 1157776 bytes.

## Resource-table rows

Offsets here are relative to each row.

```text
+0x00  char[16] name        resource key or marker name
+0x10  char[4]  typeTag     empty for marker rows
+0x14  u32      resourceId  zero for marker rows
+0x18  u32      unknown     meaning unresolved
+0x1C  u32      reserved    zero on all rows
```

All 1622 marker rows have an empty tag, resource id 0, and zeros at
`+0x18` and `+0x1C`. The 50244 resource rows have these `+0x18` values:

| Value | Rows |
|---|---:|
| `2` | 13963 |
| `0x102` | 35548 |
| `0x302` | 733 |

The tag and referenced SEDB subtype agree with this table. Nine tags are
their subtype reversed, ignoring case. `brt` names a RES container.
Of the references, 8596 are PWIB-wrapped and 41648 are direct.

| Type tag | Referenced subtype | Rows | Exemplar rows |
|---|---|---:|---:|
| `bhp` | `PHB` | 9934 | 262 |
| `brt` | `RES` | 16114 | 529 |
| `btm` | `mtb` | 882 | 5 |
| `bxt` | `txb` | 10333 | 145 |
| `fael` | `leaf` | 1326 | 29 |
| `fcss` | `SSCF` | 1871 | 25 |
| `ffev` | `veff` | 1326 | 29 |
| `ldmv` | `vmdl` | 3459 | 73 |
| `sniv` | `vins` | 1335 | 29 |
| `xetv` | `vtex` | 3664 | 71 |

The exemplar also has 30 marker rows.

## lyb payload and object list

The payload base is `SEDB start + headerSize`; the
[SEDB header](sedb-res.md#sedb-container-header) supplies `headerSize`.
References are payload-relative `u32` values. Zero is null.

```text
+0x00  char[4] tag                  "lyb", zero-padded
+0x04  u32     size                 equals SEDB totalSize on all 287
+0x08  u32     unknownA             varies per layout; meaning unresolved
+0x0C  u32     unknownB             0x00020004 on all 287
+0x10  u32     nodeTypeTableOffset  start of node-type rows
+0x14  u32     objectListOffset     0x18 on all 287
+0x18  u32[]   object list          ends at nodeTypeTableOffset
```

The exemplar's payload `unknownA` is `0x15000036`. This is a payload
word, distinct from the SEDB header's field of the same name.

The root is the first object-list entry. On all 287 layouts, the entries
after it equal the root's child array. All 496068 object-list entries
begin a declared node.

## Node declarations

The node-type table runs to its first field list. Each row is 16 bytes:

```text
+0x00  u32 extentRef     reference to an extent record
+0x04  u32 fieldListRef  reference to field descriptors
+0x08  u32 unknown       1 on all 12461 rows; meaning unresolved
+0x0C  u32 fieldCount    number of descriptors

extent record:
+0x00  u32 dataOffset    first node
+0x04  u32 nodeCount     number of nodes
+0x08  u32 nodeSize      byte stride and node width

field descriptor, 4 bytes:
+0x00  u16 offset        field offset within the node
+0x02  u16 kind          0: generic reference, 2: string reference
```

For the classes decoded on this page, descriptors list reference fields,
not inline scalars. The census counted
26875 kind-0 descriptors and 17093 kind-2 descriptors, with no other kind.
Declared extents overlap in 434 places: 287 overlaps of 12 bytes and 147
of 8 bytes. Their meaning is unresolved.

## Objects and classes

The class-bearing object prefix and class-node shape are:

```text
object prefix:
+0x00  u32 id
+0x04  u32 class-node reference
+0x08  u32 name string reference

class node, 0x14 bytes; descriptors (4,0)(8,2)(16,0):
+0x00  u32 class id
+0x04  u32 reference field          meaning unresolved
+0x08  u32 path string reference
+0x0C  u32 word                     meaning unresolved
+0x10  u32 parent class reference
```

For all 7568 class nodes, the parent reference is null or names a declared
class node. Every node in each of the 7339 class-bearing types tested
references the same class node. The object list includes 9027 objects
without a class reference.

There are 37 declared class paths, of which 32 have instances:

| Class path | Instances |
|---|---:|
| `BaseObjects/Attribute/AttributeBaseObject` | 9938 |
| `BaseObjects/BG/BGChipBaseObject` | 3745 |
| `BaseObjects/BG/BGPartsBaseObject` | 12403 |
| `BaseObjects/BG/ResourceReferenceObject` | 287 |
| `BaseObjects/Camera/CameraBaseObject` | 3 |
| `BaseObjects/CollisionBox/CollisionBoxBaseObject` | 541 |
| `BaseObjects/CubeMap/CubeMapBaseObject` | 148 |
| `BaseObjects/DrawEnvObjects/Wind/WindBaseObject` | 197 |
| `BaseObjects/EnvironmentMapDrawEnv/EnvironmentMapDrawEnvObject` | 2077 |
| `BaseObjects/Light/LightBaseObject` | 5193 |
| `BaseObjects/Light/LightSetObject` | 1561 |
| `BaseObjects/PositionMarker/PositionMarkerBaseObject` | 361 |
| `BaseObjects/SE/SEBaseObject` | 5939 |
| `BaseObjects/ScreenEnv/ScreenEnvBaseObject` | 1555 |
| `BaseObjects/TimeLine/TimeLineBaseObject` | 4185 |
| `BaseObjects/TriggerBox/TriggerBoxBaseObject` | 115 |
| `BaseObjects/VFX/VFXBaseObject` | 1335 |
| `DrawEnvObjects/DrawEnv/DrawEnvObject` | 1654 |
| `DrawEnvObjects/Fog/FogObject` | 1835 |
| `MiscObjects/LaySettings/LaySettingsObject` | 287 |
| `MiscObjects/TargetViewerSettings/TargetViewerSettingsObject` | 287 |
| `RefObjects/InstanceObject` | 392915 |
| `RefObjects/SharedFolder/DivideMap/BlockLinkObject` | 2310 |
| `RefObjects/SharedFolder/DivideMap/DivideMapFolderObject` | 22113 |
| `RefObjects/SharedFolder/SharedFolderObject` | 102 |
| `RefObjects/UnitTree/UnitTreeObject` | 11930 |
| `ScreenEnvObjects/Blur/BlurObject` | 297 |
| `ScreenEnvObjects/ColorCorrection/ColorCorrectionObject` | 297 |
| `ScreenEnvObjects/Glare/GlareObject` | 1566 |
| `ScreenEnvObjects/Latitude/LatitudeObject` | 3 |
| `ScreenEnvObjects/Shadow/ShadowObject` | 1565 |
| `ScreenEnvObjects/VolumetricLight/VolumetricLightObject` | 297 |

### Declared shapes

Each class with instances has one declared shape across the layouts that
contain it, except `BaseObjects/TimeLine/TimeLineBaseObject`. Descriptors
below are exact `(offset,kind)` sets from the census.

| Class path | Node size | Descriptors |
|---|---|---|
| `BaseObjects/Attribute/AttributeBaseObject` | `0x2C` | `(0x4,0),(0x8,2),(0x14,0),(0x1c,0),(0x24,2)` |
| `BaseObjects/BG/BGChipBaseObject` | `0x54` | `(0x4,0),(0x8,2),(0x14,0),(0x20,2),(0x24,2),(0x28,0),(0x2c,2)` |
| `BaseObjects/BG/BGPartsBaseObject` | `0x54` | `(0x4,0),(0x8,2),(0x14,0),(0x20,2),(0x24,2),(0x28,0),(0x2c,2)` |
| `BaseObjects/BG/ResourceReferenceObject` | `0x18` | `(0x4,0),(0x8,2),(0x10,2),(0x14,0)` |
| `BaseObjects/Camera/CameraBaseObject` | `0x40` | `(0x4,0),(0x8,2)` |
| `BaseObjects/CollisionBox/CollisionBoxBaseObject` | `0x14` | `(0x4,0),(0x8,2)` |
| `BaseObjects/CubeMap/CubeMapBaseObject` | `0x18` | `(0x4,0),(0x8,2),(0x14,2)` |
| `BaseObjects/DrawEnvObjects/Wind/WindBaseObject` | `0x28` | `(0x4,0),(0x8,2),(0x14,0),(0x1c,2),(0x20,0)` |
| `BaseObjects/EnvironmentMapDrawEnv/EnvironmentMapDrawEnvObject` | `0x1C` | `(0x4,0),(0x8,2),(0x14,0)` |
| `BaseObjects/Light/LightBaseObject` | `0x50` | `(0x4,0),(0x8,2)` |
| `BaseObjects/Light/LightSetObject` | `0x1C` | `(0x4,0),(0x8,2),(0x10,0),(0x18,0)` |
| `BaseObjects/PositionMarker/PositionMarkerBaseObject` | `0x18` | `(0x4,0),(0x8,2)` |
| `BaseObjects/SE/SEBaseObject` | `0x24` | `(0x4,0),(0x8,2),(0x14,2),(0x1c,0)` |
| `BaseObjects/ScreenEnv/ScreenEnvBaseObject` | `0x3C` | `(0x4,0),(0x8,2),(0x14,0),(0x18,0),(0x1c,0),(0x20,0),(0x24,0),(0x28,0),(0x2c,0),(0x30,0),(0x34,0),(0x38,0)` |
| `BaseObjects/TimeLine/TimeLineBaseObject` | `0x24` | `(0x4,0),(0x8,2),(0x14,0),(0x1c,0),(0x20,2)` |
| `BaseObjects/TimeLine/TimeLineBaseObject` | `0x28` | `(0x4,0),(0x8,2),(0x14,0),(0x1c,0),(0x20,2)` |
| `BaseObjects/TriggerBox/TriggerBoxBaseObject` | `0x14` | `(0x4,0),(0x8,2)` |
| `BaseObjects/VFX/VFXBaseObject` | `0x28` | `(0x4,0),(0x8,2),(0x18,2),(0x1c,2),(0x20,2)` |
| `DrawEnvObjects/DrawEnv/DrawEnvObject` | `0x3C` | `(0x4,0),(0x8,2),(0x10,0),(0x18,0),(0x20,0),(0x24,0),(0x28,0),(0x30,0)` |
| `DrawEnvObjects/Fog/FogObject` | `0x18` | `(0x4,0),(0x8,2),(0x10,0)` |
| `MiscObjects/LaySettings/LaySettingsObject` | `0x70` | `(0x4,0),(0x8,2),(0x1c,2),(0x4c,0),(0x50,0),(0x54,0),(0x5c,0),(0x60,0)` |
| `MiscObjects/TargetViewerSettings/TargetViewerSettingsObject` | `0x10` | `(0x4,0),(0x8,2)` |
| `RefObjects/InstanceObject` | `0x40` | `(0x4,0),(0x8,2),(0x10,0),(0x2c,0),(0x30,0),(0x34,0),(0x3c,0)` |
| `RefObjects/SharedFolder/DivideMap/BlockLinkObject` | `0x18` | `(0x4,0),(0x8,2),(0x10,0)` |
| `RefObjects/SharedFolder/DivideMap/DivideMapFolderObject` | `0x70` | `(0x4,0),(0x8,2),(0x10,0),(0x18,0),(0x50,0),(0x58,0),(0x64,0)` |
| `RefObjects/SharedFolder/SharedFolderObject` | `0x1C` | `(0x4,0),(0x8,2),(0x10,0),(0x18,0)` |
| `RefObjects/UnitTree/UnitTreeObject` | `0x50` | `(0x4,0),(0x8,2),(0x1c,0),(0x20,0),(0x24,0),(0x2c,2),(0x30,0)` |
| `ScreenEnvObjects/Blur/BlurObject` | `0x30` | `(0x4,0),(0x8,2)` |
| `ScreenEnvObjects/ColorCorrection/ColorCorrectionObject` | `0x40` | `(0x4,0),(0x8,2)` |
| `ScreenEnvObjects/Glare/GlareObject` | `0x3C` | `(0x4,0),(0x8,2),(0x28,0)` |
| `ScreenEnvObjects/Latitude/LatitudeObject` | `0x18` | `(0x4,0),(0x8,2)` |
| `ScreenEnvObjects/Shadow/ShadowObject` | `0x70` | `(0x4,0),(0x8,2)` |
| `ScreenEnvObjects/VolumetricLight/VolumetricLightObject` | `0x70` | `(0x4,0),(0x8,2)` |

The timeline `0x28` shape covers 4170 nodes in 265 layouts. Its `0x24`
shape covers 15 nodes in `0x25B10002`, `data/25/B1/00/02.DAT`, 99888
bytes, SHA-256
`7e3b84f59250f4559882763390415ec4ce9b941392f40d9e05cb1b1b028bcd5c`.

Unit-tree child and group declarations have these shapes:

| Node | Size | Descriptors | Nodes | Counting basis |
|---|---|---|---:|---|
| Child | `0x30` | `(0xc,0),(0x10,0),(0x14,2),(0x18,0),(0x20,0)` | 41471 | Every node in child-shaped declared node types |
| Group | `0x30` | `(0xc,0),(0x10,0),(0x14,0),(0x1c,2),(0x20,0)` | 2053 | Distinct groups reached from unit trees |

All 2053 groups reached from unit trees use the five-descriptor group
shape. A separate `0x30`-wide node type has the nine-descriptor set
`(0,0)(8,2)(0xC,2)(0x10,2)(0x14,0)(0x1C,2)(0x20,2)(0x24,2)(0x28,2)`.
It holds 117124 nodes in 154 layouts. No group reached from a unit tree
uses that declaration. Its meaning is unresolved.

## Fields consumed by the zone reader

The [zone reader](../../src/formats/src/zone.rs) consumes the following
fields. Translation is inline `f32[3]`. Rotation and scale fields are
references to 16-byte `f32x4` pool records, whose first three lanes the
reader uses. A null rotation reference gives zero rotation; a null scale
reference gives unit scale. List references and counts below describe
contiguous declared node runs.

```text
object prefix:
+0x04  u32    class-node reference used for class dispatch
+0x08  u32    name string reference used for LaySettings and the root

class node:
+0x08  u32    path string reference used for class dispatch

InstanceObject:
+0x20  f32[3] translation
+0x2C  u32    rotation reference
+0x30  u32    scale reference
+0x34  u32    child list reference
+0x38  u32    child count
+0x3C  u32    base reference

UnitTreeObject:
+0x10  f32[3] translation
+0x1C  u32    rotation reference
+0x20  u32    scale reference
+0x24  u32    group list reference
+0x28  u32    group count
+0x30  u32    child list reference
+0x34  u32    child count

unit-tree child:
+0x00  f32[3] translation
+0x0C  u32    rotation reference
+0x10  u32    scale reference
+0x18  u32    base reference

group:
+0x00  f32[3] translation
+0x0C  u32    rotation reference
+0x10  u32    scale reference
+0x14  u32    sub-group list reference
+0x18  u32    sub-group count
+0x20  u32    child run reference
+0x24  u32    child count

LaySettingsObject:
+0x40  f32[3] translation used for the zone transform
+0x4C  u32    rotation reference used for the zone transform
+0x50  u32    scale reference used for the zone transform

AttributeBaseObject:
+0x24  u32    resource-key string reference

CollisionBoxBaseObject:
+0x0C  u32    classification word; byte meanings unresolved
```

The census observes these additional fields, which the zone reader does
not consume:

```text
unit-tree child:
+0x14  u32    name string reference

group:
+0x1C  u32    name string reference
+0x28  u32    word; meaning unresolved
+0x2C  u32    word; meaning unresolved

BGPartsBaseObject and BGChipBaseObject:
+0x14  u32    material-list reference
+0x18  u32    material count
+0x20  u32    resource-key string reference
+0x3C  f32[3] low box corner, inferred
+0x48  f32[3] high box corner, inferred

DivideMapFolderObject:
+0x10  u32    member-list reference (array of slot-node references)
+0x14  u32    member count
+0x30  f32[3] low box corner, inferred
+0x40  f32[3] high box corner, inferred
+0x50  u32    name-list reference (array of string references)
+0x54  u32    name count

SharedFolderObject:
+0x10  u32    member-list reference (array of slot-node references)
+0x14  u32    member count

BlockLinkObject:
+0x10  u32    string-reference list
+0x14  u32    string count
```

Material entries are 12 bytes each; their first word is a key string
reference. The folder and BG box interpretations and Attribute kind are
inferred or unresolved.
No extent meaning is assigned to LaySettings `+0x3C` here.

## Field observations

LaySettings `+0x3C`, interpreted as `f32`, has this distribution. Its
meaning is unresolved.

| Value | Nodes |
|---|---:|
| `64.0` | 2 |
| `96.0` | 16 |
| `128.0` | 219 |
| `160.0` | 1 |
| `192.0` | 34 |
| `256.0` | 1 |
| `290.0` | 6 |
| `320.0` | 8 |

Group `+0x2C` is zero on 2023 nodes and 2 on 30. Group `+0x28` varies
and has no fixed sibling step. Both meanings are unresolved.

On all 16148 BGParts/BGChip nodes, `+0x24` and `+0x2C` reference empty
strings and `+0x28` is null. The following words have unresolved meanings:

| Field | Observed value | Nodes |
|---|---|---:|
| BG `+0x30` | `0` | 15561 |
| BG `+0x34` | `0` | 8643 |
| BG `+0x38` | `0x100` | 10713 |

Attribute `+0x0C..+0x23` is zero on all 9938 nodes. Its `+0x28` word,
whose kind interpretation is unresolved, has these values:

| Value | Nodes |
|---|---:|
| `1` | 8405 |
| `16` | 1212 |
| `2` | 321 |

CollisionBox classification byte 2 (at `+0x0E`) is `0x02` on all 541
nodes. Byte 3 (at `+0x0F`) has this distribution; the classification byte
meanings are unresolved. The word at `+0x10` is zero on all 541 nodes.

| Byte 3 | Nodes |
|---|---:|
| `0` | 19 |
| `2` | 45 |
| `4` | 348 |
| `5` | 16 |
| `6` | 113 |

PositionMarker `+0x0C..+0x17` is zero on all 361 nodes.
ResourceReference `+0x0C` is zero on all 287 nodes. SharedFolder `+0x18`
is null on all 102 nodes.

Among distinct transform pool records, the fourth lane is `0.0` in
275289 records and has bit pattern `0x00000001` in 270. Its meaning is
unresolved.

## Resource-key bindings

BGParts and BGChip keys at `+0x20`, Attribute keys at `+0x24`, and BG
material-list keys match a resource-row name in the same layout. All
27463 bindings resolve. They cover 21998 distinct layout/key pairs and
12595 distinct key strings across the install.

| Binding source | Row tags for the key | Bindings |
|---|---|---:|
| BGParts | `brt` | 10890 |
| BGParts | `bhp+brt` | 1513 |
| BGChip | `brt` | 946 |
| BGChip | `bhp+brt` | 2799 |
| Attribute | `bhp` | 5627 |
| Attribute | `bhp+brt` | 4311 |
| Material | `btm` | 1377 |

Class-bearing object names never match a resource-row name. They match
a marker-row name in 1335 observations.

## Graph cross-checks

The inferred DivideMapFolder boxes are ordered on all 22113 nodes, and
the inferred BG boxes on all 16148 nodes: each low component is finite
and no greater than its corresponding finite high component.

All 392915 Instance base references reach a class-bearing node:

| Target class path | References |
|---|---:|
| `BaseObjects/Attribute/AttributeBaseObject` | 3081 |
| `BaseObjects/BG/BGChipBaseObject` | 1533 |
| `BaseObjects/BG/BGPartsBaseObject` | 230539 |
| `BaseObjects/CollisionBox/CollisionBoxBaseObject` | 5 |
| `BaseObjects/CubeMap/CubeMapBaseObject` | 582 |
| `BaseObjects/DrawEnvObjects/Wind/WindBaseObject` | 197 |
| `BaseObjects/EnvironmentMapDrawEnv/EnvironmentMapDrawEnvObject` | 2077 |
| `BaseObjects/Light/LightBaseObject` | 6426 |
| `BaseObjects/PositionMarker/PositionMarkerBaseObject` | 1764 |
| `BaseObjects/SE/SEBaseObject` | 3587 |
| `BaseObjects/TriggerBox/TriggerBoxBaseObject` | 16 |
| `BaseObjects/VFX/VFXBaseObject` | 190 |
| `RefObjects/UnitTree/UnitTreeObject` | 142918 |

All 38509 distinct BlockLink string-reference slots point to printable,
non-empty strings. DivideMapFolder members (416059 distinct slots) and
SharedFolder members (1723 distinct slots) reach an InstanceObject
through a 4-byte slot node with descriptor `(0,0)`.

For groups, 1546 non-null child runs lie inside the child-node extent and
outside the owning tree's child list. Another 507 runs are null. These
counts include nested groups.

Of 11930 unit trees, 14 carry a non-identity translation or scale. The
exemplar has 1001 unit-tree children, of which 156 have non-zero rotation.

The root object is named `<LaySettings name>/Test` on all 287 layouts.
There are 159 distinct LaySettings names. Names repeat across layouts,
so a name does not identify one layout. DivideMapFolder name lists
carry 123 distinct names.

## Open questions

- Resource-row `+0x18` and payload `lyb unknownA`.
- The nine-descriptor `0x30` node type.
- LaySettings `+0x3C`.
- BG `+0x30`, `+0x34`, and `+0x38`.
- Attribute `+0x28`.
- CollisionBox classification bytes.
- Group `+0x28` and `+0x2C`.
- The fourth transform pool lane.
- Declared extent overlaps.
- Undecoded light, sound, VFX, timeline, trigger, cube-map, and environment
  classes.
