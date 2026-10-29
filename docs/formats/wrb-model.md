# Bounded WRB model inspection

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

`inspect <file> --as wrb-model` and `validate <file> --as wrb-model` select
the bounded RES -> wrb -> WRB -> MDL/MDLC -> MESH -> STMS reader used by
the [zone geometry contract](zone-geometry.md). Selection is explicit.
Automatic SEDB detection and `--as sedb` retain generic RES enumeration.
The input is a complete RES resource, not a bare WRB chunk payload.

The report contains absolute source spans, structural metadata, counts,
and SHA-256 digests. Unknown chunks and stream descriptors remain visible
without an inferred meaning. Their bytes, other RES resources, padding,
and unparsed tails remain accounted for as bounded opaque ranges. Chunk
parents contain their children; this hierarchy does not imply separate
lossless payloads. Decoded vertex coordinates and index arrays are omitted.

## Accepted structure

The shared reader requires a little-endian `SEDBRES ` root and at least one
direct SEDB child with exact four-byte subtype `wrb\x00`. It uses the RES
directory's bounded source span and the child's resolved container extent.
An anomaly intersecting the selected WRB resource, or within a nested WRB
container, is refused. Unrelated trailing RES directory anomalies remain
reported by the generic container view.

WRB chunk headers are 16 bytes. Their exact four-byte tag is at `+0x00`,
big-endian size at `+0x08`, and big-endian padded size at `+0x0c`.
Size includes the header. When padded size is smaller than size, the shared
reader advances to size rounded up to a 16-byte boundary. The whole chunk
and its advance must fit in the enclosing payload. `WRB\x00`, `MDL\x00`,
`MDLC`, `MESH`, and `AABB` carry a 16-byte info block before child chunks.
Exact tags are required for known dispatch. Other tags remain opaque.

An STMS body begins with a 16-byte header containing big-endian field
count, item count, and stride in its first three dwords. Each descriptor
occupies 16 bytes, followed by exactly `itemCount * stride` data bytes.
Descriptor dwords are retained as metadata. Supported declaration mappings
also report element type, field offset, byte length, method, API usage,
semantic name, and usage index as described below. The model path recognizes
the existing position and index stream layouts for bounded mesh counts.
Descriptor extents, size arithmetic, count limits, and nesting are checked
before reporting.

A valid WRB without MESH is accepted with zero decoded mesh parts. This
does not establish that the unknown chunks describe no geometry. An
unsupported or malformed MESH stream fails with a typed error and source
offset. Validation reports parse success and a round-trip check of
`not-applicable`.

## Vertex declaration metadata

The mapping follows `xivl-decomp:docs/resource/wrb-vertex-declaration.md`,
sections "Descriptor and intermediate record", "API array conversion", and
"Bounded numeric lookups". That finding joins the ordinary STMS descriptor
path to the retail executable's D3D9 declaration converter and constant tables.
The report describes declaration metadata without decoding additional vertex
values or asserting that a shader consumes a field according to its API name.

| Native usage | API semantic | Usage index |
|---:|---|---:|
| 0 | POSITION | 0 |
| 1 | BLENDWEIGHT | 0 |
| 2 | NORMAL | 0 |
| 3, 4 | COLOR | 0, 1 |
| 5 | FOG | 0 |
| 6 | PSIZE | 0 |
| 7 | BLENDINDICES | 0 |
| 8 through 15 | TEXCOORD | Native usage minus 8 |
| 16 | TANGENT | 0 |
| 17 | BINORMAL | 0 |

| Native format | Component count | Element type |
|---:|---|---|
| 0 | 2, 4 | SHORT2N, SHORT4N |
| 1 | 1 through 4 | FLOAT1 through FLOAT4 |
| 2 | 2, 4 | FLOAT16_2, FLOAT16_4 |
| 3 | 4 | UBYTE4N |
| 4 | 2, 4 | SHORT2, SHORT4 |
| 6 | 4 | UBYTE4 |

The element type and usage numbers and names follow Microsoft's
[D3DDECLTYPE](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3ddecltype)
and [D3DDECLUSAGE](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3ddeclusage)
definitions. Method is zero. The offset is reported as the stored value.
The descriptor's upper usage half selects the semantic; the lower half
remains in the raw words, including bit 15, which the converter does not read.

A `declaration` object is emitted only for a mapped type and usage, an offset
representable as u16, and an element that fits within the stream stride.
Other fields retain their raw words and spans without a declaration object
or a new refusal. This includes native usages 18 and above, index usage 255,
unknown format or component indices, and lookup entries with type UNUSED.
Authored cases cover every mapped usage and element type and accepted opaque
descriptors.

## Stored mesh and component counts

Inspection reports these raw stored counts with absolute source spans. Offsets
in the table are relative to the chunk payload, after its 16-byte header.

| Exact parent / chunk | Payload offset | Encoding | Report field |
|---|---:|---|---|
| MESH / HEAD | `+0x02` | u8 | `meshHead.streamCount` |
| MESH / HEAD | `+0x03` | u8 | `meshHead.envelopeCount` |
| WRB / MICT | `+0x00` | big-endian u32 | `componentCount` |

Each field contains `span` and `value`. The stored stream count is distinct
from the MESH report's count of parsed STMS children. The MICT component count
is distinct from the MINS info word that counts its HEAD and NAME children.
The original opaque payload partition remains available, with these scalar
views overlaid on it. A field that does not fit its logical payload is omitted,
without reading alignment padding. A three-byte MESH HEAD still reports its
stream byte. If neither HEAD count byte is available, `meshHead` is omitted.
Short fields do not cause inspection or validation to fail.
Inspection does not allocate or traverse records from these counts, require
count agreement, or infer active meshes, skeleton associations or visibility.

`xivl-decomp:docs/resource/floor-model-render-path.md`, "Streams and active
counts", establishes a gate on MESH HEAD byte `+0x02 > 1` at consumer VA
`0x00C8E01E` before iterating additional streams. The payload-relative byte
position and its equality with the number of STMS children are first-hand
census observations below. The report retains the name `streamCount` on that
basis. The envelope byte follows
`xivl-decomp:docs/resource/mesh-envelope-producer.md`, "Selected mesh and HEAD
count", at `0x00C8E661` and `0x00C8E6C7`. The MICT word follows
`xivl-decomp:docs/resource/floor-model-visibility.md`, "Component hierarchy
matrix", at `0x00C885A0`. MICT placement as a direct WRB child rests on the
first-hand retail census below, rather than that decomp observation. These
native observations use retail 1.23b `ffxivgame.exe`, 15996808 bytes, SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.

The first-hand read-only census is reproducible with
[`tools/research/census_wrb_counts.py`](../../tools/research/census_wrb_counts.py):

```text
python tools/research/census_wrb_counts.py --client-root <install>
```

It reads the explicit install without writing client bytes and prints
aggregate counts, histograms and inventory digests. It covered all 140180
files under `data/` and 51772 under `client/`. It measured 19612 WRB trees:
9969 in standalone data RES resources, three in client RES resources and
9640 in PWIB files. Valid PWIB files contribute WRB trees in their first
segment. For invalid PWIB extents, the command searches WRB signatures and
bounds each hit by its own SEDB extent, including entries whose declared outer
extent exceeded the installed file. This establishes field observations,
not support for wrapped model inspection.

The selected input inventories contain 9969 data resources and 9643 client
resources. Each inventory hashes sorted ASCII lines
`path<TAB>size<TAB>sha256<LF>`, with install-relative POSIX paths and decimal
sizes. Their SHA-256 identities are
`45918488f226217bbaa37adfcae9d8bd4deea8d7b1e1efb088029ce0307bda54`
for data and
`874b520e68677ac824d4c10f2d586132d9b4d10ba2459697707b58a704104fb7`
for client. The census command prints these identities as `inventorySha256`.

All 35394 MESH HEADs had stream count two, matching their STMS children.
Their envelope bytes ranged from zero through 48 and matched all 84255 ENVD
occurrences. All 19612 MICT chunks were direct WRB children. Their component
counts matched their 18155 MINS children. The stored values ranged from zero
through 128. There were no count or field
boundary exceptions. The selected floor resource 0x7F1C0000 is 41088 bytes,
SHA-256 `1af91b0e1e5e34f676f1af2884f82eddb63062e62bee058b0260e9646dfb7d0f`.
The selected body file in
`xivl-decomp:docs/actor/body-shader-constant-join.md`, "Profile and identity",
is 107152 bytes, SHA-256
`21570ade19172245ed9bb541fa3c2c7d4a74d5cd8f19500468abce51f788d093`.
Both selected resources were checked against these sizes and digests
separately; the census command reports only the inventory digests.

Authored cases cover the three count fields, preserved opaque suffixes and
ENVD/MINS bodies, and accepted short fields whose alignment padding cannot
supply the missing bytes. Short fields are omitted independently. The retained
retail case reports only the added count fields.

## Retail authentication

The retained input is `retail-res-89eb0000` in
`tests/fixtures/private-manifest.json`: `data/89/EB/00/00.DAT`, 7711 bytes,
SHA-256 `3218cfb49da670397d1e40664fc130c02712a64b989cbfecb1f4f25da6814592`.
The source and frozen copy matched both identity fields before validation.
Its WRB child occupies `[5996,7212)`, with header `[5996,6044)` and
chunk payload `[6044,7212)`.

Independent big-endian chunk accounting found 22 chunks and two STMS
streams. It partitioned all 1168 WRB payload bytes into chunk headers,
container info, stream headers/descriptors/data, opaque payloads,
padding, and tails without gaps or duplicated leaf bytes. The selected
WRB is separate from the root's retained trailing-region overlap and
clamped extent.

The index STMS chunk is `[6508,6604)`, with 24 two-byte items at
`[6556,6604)`. Its one descriptor is `(0,0,1,0x00ff0000)`.
The position STMS chunk is `[6604,6776)`, with nine 12-byte items at
`[6668,6776)` and descriptors `(0,4,4,0)` and `(8,3,4,0x00020000)`.
These observations authenticate chunk framing, descriptor-array extents,
and stream byte counts for this input. The structural inspection reports the
documented declaration mappings. A narrow format interpretation used by the
experimental importer treats the exact usage-2 format-3 field as packed normal
bytes, based on the [retail normal and winding census](#normals-and-winding);
the importer rejects overlapping fields and its local tests cover the byte
encoder.

## Retail chunk and stream census

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b) over all 140180 files under `data/`. Reproduce with:

```bash
python tools/research/census_wrb.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints counts, offsets,
resource ids, sizes, SHA-256 identities, and a small set of chunk tags.

The install contains 9970 RES resources and 9969 `wrb` trees. The one RES
without `wrb` is 0x15D90001, whose identity and header exception are recorded
in [SEDB and RES](sedb-res.md#res-subresource-directory).

### Chunk tags and headers

Tags below omit trailing NUL bytes. AABB counts containers and nested
leaves separately: 36624 of each, for 73248 chunks in total.

| Tag | Chunks | Parent tags and counts |
|---|---:|---|
| AABB | 73248 | AABB 36624, MDL 11306, MESH 25318 |
| COMP | 11306 | AABB 11306 |
| ENVD | 866 | MESH 866 |
| HEAD | 46593 | MDL 11306, MDLC 9969, MESH 25318 |
| LTCD | 9969 | WRB 9969 |
| MDL | 11306 | MDLC 11306 |
| MDLC | 9969 | WRB 9969 |
| MESH | 25318 | MDL 25318 |
| MICT | 9969 | WRB 9969 |
| NAME | 11306 | MDL 11306 |
| PGRP | 3 | MESH 3 |
| PRID | 25318 | MESH 25318 |
| PRTP | 25318 | MESH 25318 |
| RSID | 25318 | MESH 25318 |
| RSTP | 25318 | MESH 25318 |
| STMS | 50636 | MESH 50636 |
| STR | 25318 | MESH 25318 |
| WRB | 9969 | wrb 9969 |

Chunk `+0x04` is zero on all 397048 chunks. Padded size is zero on 103155
chunks, including all 9969 WRB containers and 9969 of the 11306 MDL
containers. Every MESH has exactly one RSID (size 0x20), RSTP (0x14), PRID
(0x20), and PRTP (0x14). All 101272 of these payloads are zero, with meaning
unresolved. Chunk sizes include the 16-byte chunk header.

All 11306 NAME bodies and all 25318 STR bodies hold a nonempty printable
ASCII string followed by a NUL, with only zero bytes after it. The census
establishes this text shape only.

### Streams and mesh counts

STMS size accounting is exact on all 50636 streams: the body is the
16-byte stream header, the descriptor array, and `itemCount * stride`
data bytes. The header's fourth word is zero throughout.

Each descriptor is four big-endian u32 words: field offset, format,
component count, and usage. The descriptor combinations are:

| Format | Count | usage >> 16 | Usage low half | Descriptors |
|---:|---:|---:|---:|---:|
| 0 | 1 | 255 | 0 | 25318 |
| 2 | 2 | 8 | 0 | 24914 |
| 2 | 2 | 9 | 0 | 11492 |
| 2 | 2 | 10 | 0 | 3152 |
| 2 | 2 | 11 | 0 | 827 |
| 2 | 4 | 3 | 0 | 427 |
| 2 | 4 | 4 | 0 | 138 |
| 3 | 4 | 2 | 0 | 25318 |
| 3 | 4 | 3 | 0 | 24654 |
| 3 | 4 | 4 | 0 | 9543 |
| 3 | 4 | 13 | 0 | 22016 |
| 3 | 4 | 14 | 0 | 63 |
| 4 | 4 | 0 | 0 | 25318 |
| 6 | 4 | 15 | 0 | 63 |

Here, usage names the upper half (`usage >> 16`); the low half is zero
in every observed descriptor. Index streams have one field, format 0,
count 1, usage 0xFF, and stride 2. Vertex streams have 2 through 10 fields
and strides from 0x0C through 0x2C. Position (usage 0) is format 4, count 4,
at offset 0 in all 25318 vertex streams. Its four lanes are big-endian
signed 16-bit values, and the fourth lane is 1 on all 26875219 vertices.
Index values are big-endian u16.

Each MESH has a HEAD of size 0x20, with big-endian u16 vertex count at
payload `+8` and triangle count at `+10`. Both match the sibling streams
on all 25318 meshes. Index count is `3 * triangleCount`, and all 66666603
indices are in range. Model-level HEAD chunks under MDL and MDLC are
0x2C throughout.

### Bounds and position decode

A mesh box uses an AABB container of size 0x50, info word 1, with one
AABB leaf. An MDL box uses size 0x80, info word 2, with AABB then COMP
leaves. The MDL's AABB equals its COMP in 9974 cases and differs in 1332.
Each AABB or COMP leaf holds six big-endian f32 values: minimum x, y, z,
then maximum x, y, z.

The position decode used by the xivl-tools reader in
[`src/formats/src/zone.rs`](../../src/formats/src/zone.rs) is:

```text
decoded = COMP center + raw / 32767 * COMP half extent
```

The center is the midpoint of the COMP minimum and maximum, and the half
extent is half their difference. Comparing the decoded bounds with the
mesh AABB by the maximum absolute error of its six bounds, divisor 32767
fits better on 24971 meshes, divisor 32768 on 346, with one tie. The census
uses float64 decoding from float32 COMP bounds.

Model 0x72E50007, `data/72/E5/00/07.DAT`, 55266 bytes, SHA-256
`df108adf411ef34ce0ff557330243731c92d0c707977b31552b73366d8f89f8f`,
has two MDLs sharing one COMP with different AABB leaves.

Resource 0x89940674, `data/89/94/06/74.DAT`, 15191 bytes,
SHA-256 `6008ffa0944740262dcdf49e164e2d4f782e031a6c9da223d45f8e4907ef8d58`,
has one MDL, no MESH, and inverted MDL AABB and COMP bounds. This is an
observation, with meaning unresolved.

### Normals and winding

Usage 2 (format 3, count 4) decodes as `byte / 127.5 - 1`. The stream-order
face normal `(v1 - v0) x (v2 - v0)` has a positive dot product with the mean
usage-2 normal on 22067339 of 22222201 install triangles. That usage-2
comparison includes 3125 degenerate faces, defined by an exactly zero cross
product. The usage-3 comparison covers only the 24654 meshes whose usage-3
field is format 3, count 4, excluding 664 descriptor exceptions. It has a
positive dot product on 13141893 of 21476050 triangles and a zero dot
product on 4080.

Decoded resource 0x89940554, `data/89/94/05/54.DAT`, 113706 bytes, SHA-256
`be885984f96538e217deb91e7c430def30c264a8b5831f42e16a98ee54ab3adf`,
has 1425 vertices, 896 triangles, and 645 interior edges, each traversed
in opposite directions by its two faces. There are no degenerate faces.

This supports usage 2 as the normal and stream order as counterclockwise
from the front for the large majority of faces.

### Polygon groups

There are three PGRP chunks install-wide, all in model 0x897D0398,
`data/89/7D/03/98.DAT`, 48960 bytes, SHA-256
`65d7f608b206320cadf4989ee3029f10df1834c65adbc7fd342b8bf5b859e8f4`.
The payload is big-endian u32 `triangleCount`, u32 `indexOffset` (0x0C),
u32 `nameOffset`, then u16 triangle indices and a NUL-padded name extending
to the payload end. `indexOffset + 2 * triangleCount == nameOffset`.

The first mesh's two groups, of 24 and 144 triangles, partition its 168
triangles. The second mesh's group covers 32 of its 152 triangles. These
checks establish that the indices name triangles. The other 25316 meshes
have no PGRP.

## Claim boundary

The census observations above are format evidence and do not change the
partial read status.

WRB model read is `partial`. Public authored cases exercise the accepted
structure, unknown ranges, exact tags, no-MESH resources, malformed
descriptors, truncation, counts, and nesting. The pinned private cases
require an explicit frozen root and preserve only structure and digests.

Declaration metadata does not establish stored skeletal weights, bone indices,
palette correspondence, or skinning. Those gaps remain unresolved in
`xivl-decomp:docs/resource/wrb-vertex-declaration.md`, "Reproduction and limits".
Unmapped usages and format/component combinations remain opaque as described
under [vertex declaration metadata](#vertex-declaration-metadata).

ENVD bodies and the remainder of MICT, including MINS HEAD node/mesh indexes
and its mask-selector byte, remain opaque. The census found a leading
big-endian word of 16 and a nonempty ASCII string terminated within the payload
at that offset in all 84255 ENVD records. This does not close the conditional
copy and allocation assumptions in
`xivl-decomp:docs/resource/mesh-envelope-producer.md`, "Existing consumer join
and name boundary" and "Exact surviving producer gap". No envelope name,
bone/palette correspondence or skinning meaning is decoded. Other HEAD bytes,
AABB leaf values, STMS header bytes `+0x0E/+0x0F`, STR text and RSID identifiers
retain their opaque reports. Stored count metadata does not establish
runtime admission, matrix selection, rendering or visibility.

Structural JSON is a report and does not establish lossless export support.
This reader does not establish glTF output, rendering, material or texture
meaning, skeletons, animation, client runtime use, or a WRB writer.

The separate experimental [zone-project builder](../zone-project.md) uses the
bounded OBJ importer with a compatible template. Its offline output does not
change this reader's support status or establish client rendering acceptance.
