# Zone geometry export

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

`xivl export-zones <client-root> --output <directory>` reads only an explicit
FFXIV 1.23b install root. It maps resource identifiers through the established
`data/AA/BB/CC/DD.DAT` convention, checks the bounded SEDB frame, and reads
the [documented `MapLayoutResourceData` table and embedded `lyb`
graph](world-layout.md). Resource keys bind `bhp` PHB collision hulls and
optional `brt` model resources. The model reader decodes the bounded
RES -> wrb -> MDL -> MESH -> STMS streams needed to retain model source
associations; model render faces are not added
to the collision OBJ. A structurally valid WRB with no MESH is retained as a
render source association with no decoded parts.

The same bounded model reader is available separately through explicit
`inspect --as wrb-model` and `validate --as wrb-model`. The
[WRB inspection contract](wrb-model.md) defines its structural report and
retail authentication. This report retains metadata and opaque ranges
without exporting decoded geometry.

Retail InstanceObject child slots also contain an exact 0x0C flat range record
with the descriptor `[(0,0)]`; its target and count are bounds-checked but its
unresolved range payload is not treated as a ChildObject. Direct ChildObject,
UnitTree, group, Attribute, CollisionBox, and LaySettings dispatch remains
exact-shape validated.

The UnitTree group declaration is the five-field set at offsets `0x0C`,
`0x10`, `0x14`, `0x1C`, and `0x20`. Every group reached from a unit tree in
the 1.23b install uses it; see [world layout](world-layout.md). The reader
rejects any other group declaration. LYB subtypes and model/PHB chunk tags
are four-byte exact matches, not prefix matches.

Model association rejects ambiguous or anomalous spans that intersect the
selected WRB materialization (and any nested WRB content); unrelated trailing
RES directory anomalies are outside the selected model payload.

The scene transform is Y-up and is composed in this order:

```text
world = zone * instance * unitTree * group * child
each transform = T * Rz * Ry * Rx * S
```

The writer emits collision geometry only: Attribute-bound PHB hulls and
CollisionBox unit primitives. It writes `zones/<zone-name>.obj`, a matching
`zones/<zone-name>.metadata.json`, and `manifest.json`. OBJ vertex indices are
one-based as required by the format. Face order is source order. PHB surface
values and CollisionBox classification words are retained as raw sidecar fields;
collision faces are explicitly marked two-sided and are not reversed. A PHB
face records its layout object, PHB resource, hull, triangle, and associated
model resource when one was present.

CollisionBox faces use the retail `col_cube` corner order and source diagonal /
winding, carry raw surface `0`, and retain the separate classification word.

The sidecar schema version is `2`. It records settings, source resource IDs,
source paths and digests, one placement table containing the source layout node
and node-block offsets, resource key, render/collision classifications, the
complete zone/instance/unit-tree/group/child transform chain, and the composed
world matrix. `faceRanges` is a compact contiguous association list: each
range gives its OBJ face span, placement index, optional resource, part,
triangle span, classification, and raw surface. Ranges split when any
association or raw surface changes, and their counts cover every OBJ face
exactly once. `collisionResources` carries PHB declared/decoded vertex bounds
and counts once per collision resource; a declared/decoded bounds mismatch is
recorded rather than rejected. CollisionBox faces have a null resource ID.
The collection manifest records `clientVersion`, `exporterVersion`, extraction
selection, settings, each zone's original layout/zone name and output base
name, source classifications, and output paths, sizes, and SHA-256 digests.

The command does not search for an installation, compile Recast or Detour,
export render meshes, textures, animations, or unrelated model data, or infer
unresolved server-coordinate meaning from LaySettings. The supported reader is
intentionally bounded; malformed or unsupported payload records fail before
output is created. Tests use authored bytes with the documented retail
MapLayout, lyb, RES/wrb, and PHB.GBD framing rather than a private surrogate
wire format. Source evidence entry points are
[SEDB, RES, and resource paths](sedb-res.md) and
[world layout](world-layout.md).

## Retail zone observation

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001),
layout `0x29D90001`, zone `sea_s0_twn01`, `data/29/D9/00/01.DAT`, 1197104
bytes, SHA-256
`bc40d7ff57ab8998978532cce4564e2488ed81d6d10b8f7ede3e1586b0b26523`.
Reproduce with:

```bash
xivl export-zones <install> --output <dir> --layout 0x29D90001
python tools/research/zone_export_stats.py --zone <dir>/zones/sea_s0_twn01
```

The statistics command is research only. It requires an explicit zone export,
makes no support claim, never runs in CI, and prints counts rather than geometry.

The export contained 3311 collision placements: 953 also carried a render
model through the same resource key, and 2358 were collision-only. Its 832
CollisionBox placements retained these raw classification words:

| Classification word | CollisionBox placements |
|---|---|
| `0x02020000` | 689 |
| `0x06020000` | 133 |
| `0x02020100` | 10 |

The OBJ contained 463888 faces: 453904 from PHB hulls and 9984 from
CollisionBox primitives, with 12 faces per box. Of all faces, 126714 had a
downward geometric normal and 289 had zero area. Source winding therefore
does not establish a facing convention for this export, consistent with
marking collision faces two-sided.

For nonzero-area PHB hull faces, the statistics command measured the geometric
normal from the OBJ positions and counted faces within 30 degrees of horizontal
and of vertical:

| Raw surface | Nonzero-area faces | Within 30 degrees of horizontal | Within 30 degrees of vertical |
|---|---|---|---|
| 0 | 142857 | 66867 | 61554 |
| 1 | 932 | 121 | 518 |
| 2 | 194631 | 49361 | 118697 |
| 3 | 622 | 20 | 446 |
| 4 | 14605 | 7871 | 6126 |
| 5 | 18911 | 3822 | 14782 |
| 6 | 81002 | 12808 | 56122 |
| 7 | 55 | 8 | 41 |

Every raw surface value `0..7` occurred on both orientations. These values do
not separate floors from walls in this zone observation; their meaning remains
unresolved.

## Experimental PHB collision authoring

The reusable `phb_author` library and explicit-path `phb_collision_author`
example accept authored triangular collision geometry through a strict input
contract. The [PHB authoring contract](phb-collision.md) owns the accepted
template profile, acceleration evidence, byte-preservation boundary, and
remaining owner-operated client checks.

## Experimental model position edit

The format library includes an experimental, count-preserving byte edit for
one explicitly selected zero-based render `MeshPart`. It scales the first
three signed big-endian 16-bit position components around the existing `COMP`
box center and leaves the fourth component, descriptors, indices, `COMP`,
other streams and parts, padding, and unknown bytes unchanged.

Run the research example with explicit paths:

```powershell
cargo run --locked -p xivl-formats --example model_position_edit -- <input> <new-output> <zero-based-mesh-part> <factor>
```

The factor must be finite and in `(0, 1]`. Each component is quantized as
`round(raw_i16 * factor)` after validation. A factor of `1` returns a
byte-identical copy. The example reports input and output lengths and hashes,
mesh counts, and contiguous changed byte spans. It refuses an existing output
and a source overwrite, and writes only after both input and edited output pass
the bounded parser.

This edit changes render position bytes only. It does not change topology,
materials, collision data, stream sizes, or model descriptors. It is a
research aid for inspecting a custom-zone rendering hypothesis and makes no
claim that a modified file is accepted by the client or by other consumers.

## Experimental OBJ model blockout import

The format library also includes a bounded blockout import that uses one
native RES model as its template. Run it with explicit native input, OBJ input,
and a new output path:

```powershell
cargo run --locked -p xivl-formats --example model_obj_import -- <native-input> <obj-input> <new-output>
```

The template must contain one direct WRB resource with one MDL and one MESH,
one MDL AABB leaf, one format-4 position stream, and one format-0 u16
triangle-list stream. The importer accepts OBJ `v` and triangular `f` records,
ignores comments and the documented `l`, `o`, and `s` records, preserves face
order and winding, and rejects other record types. Position bounds are written
to the MDL AABB, MESH AABB, and MDL COMP boxes, then quantized with the native
signed-16 position encoding. When the template has the confirmed format-3
usage-2 normal field, normals are generated from the OBJ triangles.

The accepted MESH has exactly the position and index STMS streams described
above; extra or ambiguous vertex streams are rejected. Template chunks,
material and shader bytes, descriptors, and opaque padding are retained.
Unresolved per-vertex fields retain template record bytes when the imported
vertex array needs additional records; this does not import OBJ UVs, materials,
textures, skeletons, or animation. The importer rejects bounds that overflow
the native decode arithmetic and preflights a 256 MiB maximum output budget
before allocating replacement vertex data.

When a replacement stream has fewer bytes than its template stream, its
logical size and active item count decrease while its padded allocation stays
the same. The unused template suffix remains byte-identical padding. If neither
stream grows, the WRB extent, RES directory, trailing resources and header
`+0x34` stay at their original offsets. This preserves the container allocation;
it does not prove that the native renderer accepts the enlarged padding.

The command refuses an existing output and parses the generated resource before
writing it. `RES` header `+0x34` remains unresolved and byte-preserved; in the
selected source, `40432 + 0x40 == 40496` matched a trailing directory-entry
start, and relocation may make such a relationship stale. The importer does not
speculate about or rewrite that field. This remains an experimental blockout
aid; client and renderer acceptance are unverified.
