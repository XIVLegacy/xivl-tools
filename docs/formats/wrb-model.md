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
bytes, based on the [retail model and collision evidence](https://github.com/BahamutXIV/bahamut-navmesh/blob/e9f384c9d8c37e1942d942fb1566dc2967163f3e/docs/model-and-collision-data.md#index-stream-and-triangle-winding);
the importer rejects overlapping fields and its local tests cover the byte
encoder.

## Claim boundary

WRB model read is `partial`. Public authored cases exercise the accepted
structure, unknown ranges, exact tags, no-MESH resources, malformed
descriptors, truncation, counts, and nesting. The pinned private cases
require an explicit frozen root and preserve only structure and digests.

Structural JSON is a report and does not establish lossless export support.
This reader does not establish glTF output, rendering, material or texture
meaning, skeletons, animation, client runtime use, or a WRB writer.
