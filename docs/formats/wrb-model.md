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
Descriptor dwords are retained as metadata without assigning meanings to
unrecognized layouts. The model path recognizes the existing position
and index stream layouts for bounded mesh counts. The remainder remains
opaque. Descriptor extents, size arithmetic, count limits, and nesting
are checked before reporting.

A valid WRB without MESH is accepted with zero decoded mesh parts. This
does not establish that the unknown chunks describe no geometry. An
unsupported or malformed MESH stream fails with a typed error and source
offset. Validation reports parse success and a round-trip check of
`not-applicable`.

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
and stream byte counts for this input. The structural inspection keeps the
descriptor semantics undecoded. A narrow format interpretation used by the
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

Structural JSON is a report and does not establish lossless export support.
This reader does not establish glTF output, rendering, material or texture
meaning, skeletons, animation, client runtime use, or a WRB writer.
