# SEDB, RES, and resource paths

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b) over all 140180 files under `data/`. Reproduce with:

```bash
python tools/research/census_sedb.py --client-root <install>
python tools/research/census_wrb.py --client-root <install>
```

These commands are research only. They make no support claim, never run in
CI, require an explicit `--client-root`, and print counts, offsets, resource
ids, sizes, SHA-256 identities, and a small set of chunk tags. `census_sedb`
also prints 4-byte signature hex for non-SEDB files.

The RES exemplar is `data/89/94/05/54.DAT` (resource 0x89940554), 113706
bytes, SHA-256
`be885984f96538e217deb91e7c430def30c264a8b5831f42e16a98ee54ab3adf`.
Model, mesh, and chunk-tree layouts are documented in
[WRB model inspection](wrb-model.md), and PHB geometry in
[PHB collision](phb-collision.md).

Local tests own these facts:
`src/formats/src/resource.rs` tests for the path mapping,
`tests/conformance/cases/` for the container layouts, and
`src/formats/tests/malformed_inputs.rs` for the failure behavior.

## Resource ID to DAT path

A 32-bit resource id `0xAABBCCDD` names `data/AA/BB/CC/DD.DAT` under the
client install root, uppercase hexadecimal. Established empirically across
the whole install.

The mapping is total: every `u32` names a path, whether or not the file
exists. This repository never resolves a path against a client install on
its own. An install root is always supplied by the caller.

## SEDB container header

All integer fields are little-endian.

```text
0x00  char[4] magic      "SEDB"
0x04  char[4] subtype    "lyb", "PHB", "txb", "vins", "vmdl", "RES ",
                         "shd", "SKL ", "wrb", ...
0x08  u32     unknownA   observed values by subtype below; meaning unresolved
0x0C  u16     flags      0x0000, 0x0200, or 0x0400 observed;
                         meaning unresolved
0x0E  u16     headerSize payload begins at container start + headerSize
                         (0x30, 0x40, 0x48 observed)
0x10  u32     declaredSize  advisory; see "The 0x10 field" below. It is
                         the file size for most subtypes, the header size
                         for PHB, zero for mtb, and a genuine sub-file
                         extent for vins and leaf
...   padding to headerSize; subtype-specific extended fields may sit
      here, and RES uses 0x30..0x3F
```

Observed header values cover top-level SEDB resources, direct SEDB children
of RES, and MapLayout `lyb` scenes. `unknownA` and `flags` have meaning
unresolved.

| Location | Subtype | unknownA | flags | headerSize |
|---|---|---|---|---|
| Top level | PHB | 0x120 | 0x0400 | 0x48 |
| Top level | RES | 0xFA0 | 0x0400 | 0x30, 0x40 |
| Top level | SSCF | 0x2, 0x3 | 0x0400 | 0x30 |
| Top level | leaf | 0x0 | 0x0400 | 0x30 |
| Top level | mtb | 0x43 | 0x0000 | 0x40 |
| Top level | txb | 0x1 | 0x0200 | 0x40 |
| Top level | veff | 0x0 | 0x0400 | 0x30 |
| Top level | vins | 0x0 | 0x0400 | 0x30 |
| Top level | vmdl | 0x0 | 0x0400 | 0x30 |
| Top level | vtex | 0x0 | 0x0400 | 0x30 |
| RES child | RES | 0xFA0 | 0x0400 | 0x30 |
| RES child | SKL | 0x1B58 | 0x0400 | 0x30 |
| RES child | shd | 0x1D | 0x0200 | 0x30 |
| RES child | wrb | 0x17 | 0x0200 | 0x30 |
| MapLayout scene | lyb | 0x8 | 0x0000 | 0x30 |

`0x14` is therefore the smallest header a container can declare. The parser
rejects a smaller one, and rejects a `headerSize` or `declaredSize` that
runs past what the input holds, rather than reading what happens to follow.

## RES subresource directory

Top-level `RES ` containers declare `headerSize` 0x40 except 0x15D90001,
which declares 0x30 while its directory still starts at 0x40. Nested RES
children declare 0x30. The reader requires a RES header of at least 0x40
bytes and rejects `headerSize` 0x30. The extended header fields are:

```text
0x30  u32     subresourceCount
0x34  u32     unknownB          points near the trailing name table,
                                relative to 0x40 (inferred, unresolved)
0x38  u32     subresourceCount  repeated; both must agree
0x3C  char[4] typeName          the resource type tag, for example "brt"
```

Directory at 0x40, one 16-byte entry per subresource:

```text
0x00  u32 index
0x04  u32 offset   relative to the payload base,
                   headerSize + 16 * subresourceCount
0x08  u32 size
0x0C  u32 kind     0, 2, and 4 observed; semantics unresolved
```

Observed subresource contents include nested SEDB containers (`shd`,
`wrb`, `SKL `) and non-SEDB trailing regions such as a name table.

The final directory entry of 0x89940554 declares an extent 0x16 bytes past
the end of the file (0x1BC40 against a 0x1BC2A file). The parser clamps
such an extent to the input and reports the clamp as an anomaly. It never
reads past the input and never drops the entry.

Across the install, 8518 RES directory entries declare an extent past the
end of the file, by at most 57 bytes.

Resource 0x15D90001, `data/15/D9/00/01.DAT`, 1189004 bytes, SHA-256
`9b1fca85933590e6e4d66a1b670408383519ef70b788a5118c3d61cc3c1df891`,
declares `headerSize` 0x30 while its directory starts at 0x40. This is a
retail observation outside the parser's accepted RES header boundary.

## First-hand retail observation

### The resource-path convention

All 140180 files under `data/` match `AA/BB/CC/DD.DAT`, confirming the
convention across the whole install with no exception.

### The 0x10 field

The 0x10 field equals the file size for 92303 of the 97749 top-level SEDB
resources and differs for 5446 of them:

```text
declaredSize == file size    92303   SSCF 77694, RES 9970, vmdl 1682,
                                     vtex 950, veff 920, txb 783,
                                     leaf 156, vins 148
declaredSize == headerSize    5203   PHB 5203
declaredSize <  headerSize     145   mtb 145 (the field reads zero)
declaredSize <  file size       98   vins 58, leaf 40
```

`vins` and `leaf` appear in two rows: 148 of 206 vins and 156 of 196 leaf
resources declare the file size and the rest a shorter extent. The split
is per file, not per subtype, so no subtype can be special-cased into
trusting the field.

So the field is advisory, not authoritative. The parser treats it that
way: a value below the header is not a malformed file but one with MTB-like
structure, so the container extent falls back to `headerSize`, a
`declared-size-below-header` anomaly is recorded, and the remainder is
reported as trailing bytes. Nothing is rejected and nothing is dropped.
Only a value past the end of the input is still an error, because bytes
that are not there cannot be accounted for.

The parser accepts all 145 mtb resources. `header-size-out-of-range` reports
only that the header itself does not fit in the input.

### RES directory confirmations

Read directly from the RES exemplar `data/89/94/05/54.DAT`
(resource 0x89940554):

- the payload base is `headerSize + 16 * subresourceCount` (0x40 + 0x70 =
  0xB0). Subresources 0 through 4 begin exactly on `SEDB` signatures at
  0xB0, 0x55E8, 0xA638, 0xEED8, and 0x1B9E0, which is what fixes the base;
- consecutive subresources are separated by up to three bytes of
  alignment padding, which this parser reports as unknown gaps rather than
  folding into either neighbour;
- the final entry declares 0x1BB20..0x1BC40, an extent 0x16 bytes past
  the end of the 0x1BC2A file;
- the last two entries overlap (0x1BB20 begins inside 0x1BB04..0x1BBD0).
  Sampling 107 RES resources found this in all 107, so the overlapping
  trailing region is a property of the format, not a defect. Declared
  offsets and sizes are reported verbatim alongside the resolved spans, so
  the overlap is visible rather than silently resolved.

### Parser behavior over the install

A 1508-file sample, every 93rd resource, run through the SEDB-only reader:
1056 SEDB resources parsed, 0 errors, 0 panics. The remaining 452 were not
SEDB at all and took `bad-magic`: GTEX 226, zero-filled 59, PWIB 38, plus a
tail of smaller signatures. GTEX has a separate bounded reader. PWIB's
separate outer reader delegates the child SEDB at offset `0x10` to this model;
its outer layout remains outside the standalone SEDB/RES claim.

## Exact payload materialization

`xivl extract-resource <file> --output <directory> --materialize-payloads`
uses the parser's resolved direct-root entry spans. Plain SEDB payload entries,
RES subresources, unknown RES gaps, and empty entries are copied byte for byte.
The manifest retains offsets, exclusive ends, lengths, SHA-256 digests, owning
container and entry paths, declared RES fields, and nested-container identity.

Nested bytes are emitted only as their direct parent subresource. Recursively
emitting the child payload as another file would duplicate the same source
bytes. RES directory bytes remain in the structural report and are not called
payload.

An overlap includes a complete alias. A clamped extent, out-of-range start,
extent past the resolved container, overlap, alias, or nested parse failure
means the directory does not define independent exact payload spans. Inspection
continues to report those anomalies, but materialization fails with
`ambiguous-payload-span` before any output is created. The exporter does not
choose an owner, merge spans, trim declarations, or infer payload semantics.

## PHB.GBD collision payload

The zone exporter reads the documented PHB collision geometry independently of
the RES reader. A `PHB.GBD` chunk starts at file offset `0xC0`. Its 16-byte
chunk header contains a little-endian `payloadEnd` at chunk offset `0x08`,
relative to the chunk start; the resource must end at `chunk + payloadEnd +
0x10`, leaving the documented 16-byte trailer. The parser bounds every
vertex, triangle, and surface array by `chunk + payloadEnd`, rejects geometry
offsets before the documented `0x80` header, and rejects an array that would
exceed that end.
Within the GBD payload, vertex offset/count are at `0x38`/`0x3C`, declared
minimum bounds are three floats at `0x40`, declared maximum bounds are three
floats at `0x50`, and triangle offset/count are at `0x60`/`0x64`. The exporter
preserves those declared bounds and records decoded bounds and counts once in
the metadata collision-resource table. A declared-versus-decoded bounds
mismatch is recorded as evidence; it is not silently substituted or rejected.
