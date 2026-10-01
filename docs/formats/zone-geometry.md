# Zone geometry export

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

`xivl export-zones <client-root> --output <directory>` reads only an explicit
FFXIV 1.23b install root. It maps resource identifiers through the established
`data/AA/BB/CC/DD.DAT` convention, checks the bounded SEDB frame, and reads
the documented `MapLayoutResourceData` table and embedded `lyb` graph. Resource
keys bind `bhp` PHB collision hulls and optional `brt` model resources. The
model reader decodes the bounded RES -> wrb -> MDL -> MESH -> STMS streams
needed to retain model source associations; model render faces are not added
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

The canonical UnitTree group declaration is the documented five-field set at
offsets `0x0C`, `0x10`, `0x14`, `0x1C`, and `0x20`; one observed 1.23b
serializer variant is accepted only when its complete nine-field declaration
matches the recorded extended set. LYB subtypes and model/PHB chunk tags are
four-byte exact matches, not prefix matches.

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
wire format. The source evidence entry point is
[SEDB, RES, and resource paths](sedb-res.md).

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
