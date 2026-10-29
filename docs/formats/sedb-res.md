# SEDB, RES, and resource paths

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b). The DAT-only header and WRB observations cover all 140180
files under `data/`. Reproduce those observations with:

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
0x08  u32     unknownA   subtype-specific version meanings below
0x0C  u16     flags      0x0000, 0x0200, or 0x0400 observed in DAT;
                         low byte has selected endian consumers below;
                         remaining meaning unresolved
0x0E  u16     headerSize declared payload boundary; RES directory rule below
                         (0x30, 0x40, 0x48 observed)
0x10  u32     declaredSize  advisory; see "The 0x10 field" below. It is
                         the file size for most subtypes, the header size
                         for PHB, zero for mtb, and a genuine sub-file
                         extent for vins and leaf
...   padding to headerSize; subtype-specific extended fields may sit
      here, and RES uses 0x30..0x3F
```

Observed header values cover top-level SEDB resources, direct SEDB children
of RES, and MapLayout `lyb` scenes. Values alone do not establish meanings.

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

### Consumer-backed header fields

The native consumer facts below come from retail `ffxivgame.exe`, size
15996808 bytes, SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.
Addresses are VA with image base `0x00400000`.

Generic SEDB/RES inspection adds `version` as a raw alias of `unknownA` only
for these exact subtypes. The native conditions are evidence about those
consumers, not additional parser refusals.

| Subtype | Native use of +0x08 | Source and consumer VA |
|---|---|---|
| RES | Compared against 4000 | `xivl-decomp:docs/resource/gtex-pwib-loader.md`, "Native consumer boundary", `0x00e0d120` |
| PHB | Version in [0x118, 0x120] | `xivl-decomp:docs/resource/attribute-resource-and-scene-binding.md`, "Authored floor descriptor", `0x00ac06e0` |
| SSCF | Version no greater than 3 | `xivl-decomp:docs/resource/music-song-table.md`, "Stream-less containers", `0x00b5eb70`, check at `0x00b5eccf` |
| mtb | Version at least 0x31, with branches at 0x42 | `xivl-decomp:docs/resource/mtb-container-framing.md`, "Header and tables", `0x00a2d700`, `0x00a2d510`, `0x00a30280` |
| txb | Version at least 1 | `xivl-decomp:docs/resource/gtex-pwib-loader.md`, "Selected RES/txb consumer", `0x00c78e60` |
| wrb | Version at least 7 | `xivl-decomp:docs/resource/floor-model-render-path.md`, "Primary resource and concrete receivers", `0x00c77270` |
| shd | Version 29 bypasses an older-version update | `xivl-decomp:docs/resource/floor-model-render-path.md`, "Selected material binding", `0x00c90018` |

Inspection also adds the raw byte at +0x0C as `endianByte` for the following
exact subtypes. This does not enable big-endian parsing or reinterpret the
16-bit `flags` field. Byte +0x0D remains unresolved.

| Subtype | Native use of +0x0C | Source and consumer VA |
|---|---|---|
| RES | Nonzero requests in-place conversion before initialization | `xivl-decomp:docs/resource/gtex-pwib-loader.md`, "Native consumer boundary", `0x00e0d120`, `0x00e0ce20` |
| PHB | Endian selection | `xivl-decomp:docs/resource/attribute-resource-and-scene-binding.md`, `0x00ac06e0` |
| SKL | Byte 1 invokes conversion and is cleared | `xivl-decomp:docs/resource/skeleton-bone-framing.md`, "Native framing observations", `0x00e0dd6f..0x00e0dd7b` |
| wrb | Selects version byte order | `xivl-decomp:docs/resource/floor-model-render-path.md`, `0x00c77270` |
| shd | Zero skips preprocessing conversion | `xivl-decomp:docs/resource/floor-model-render-path.md`, `0x00c902a0` |
| veff, vins, leaf, vmdl, vtex | Nonzero byte-swaps the header-size word during cloning | `xivl-decomp:docs/resource/vfx-qix-resource-container.md`, "Blob header", `0x00b93e60` |

NUL-terminated tags retain their NUL byte, and `RES ` and `SKL ` retain their
space byte. Other tag spellings have no added alias. Existing `unknownA`,
`flags`, and `unknownB` report fields remain present. Specialized format
views retain their own report contracts.

The SKL consumer additionally requires +0x08 at least 7000
(`xivl-decomp:docs/resource/skeleton-bone-framing.md`,
`0x00e0dd7f..0x00e0dd96`), but that source calls it an unnamed scalar.
It remains `unknownA`. The SCB framing predicate requires +0x08 at least 2
and +0x0C zero (`xivl-decomp:docs/resource/cut-scheduler-clip-layout.md`,
"Resource header and directory"), without a consumer VA for those checks.
Neither predicate supplies a new field name. Other +0x08 meanings, other
+0x0C consumers, +0x0D, and subtype payloads remain gaps.

## RES subresource directory

In the DAT census, top-level `RES ` containers declare `headerSize` 0x40
except 0x15D90001, which declares 0x30 while its directory still starts
at 0x40. Nested RES children declare 0x30. The reader accepts exactly
0x30, top-level or nested, as an additional short-header shape when the
directory is nonempty, its counts agree, and
the extended fields and complete directory fit in the resolved container
extent and input. Other RES headers below 0x40 remain refused. The extended
header fields are:

```text
0x30  u32     subresourceCount
0x34  u32     unknownB          byte offset of the name table,
                                relative to the payload base below
0x38  u32     subresourceCount  repeated; both must agree
0x3C  char[4] typeName          the resource type tag, for example "brt"
```

Directory at `max(headerSize, 0x40)`, one 16-byte entry per subresource:

```text
0x00  u32 index
0x04  u32 offset   relative to the payload base,
                   max(headerSize, 0x40) + 16 * subresourceCount
0x08  u32 size
0x0C  u32 kind     0, 1, 2, and 4 observed; semantics unresolved
```

Observed subresource contents include nested SEDB containers (`shd`,
`wrb`, `SKL `) and non-SEDB trailing regions such as a name table.

The final directory entry of 0x89940554 declares an extent 0x16 bytes past
the end of the file (0x1BC40 against a 0x1BC2A file). The parser clamps
such an extent to the input and reports the clamp as an anomaly. It never
reads past the input and never drops the entry.

In the DAT census, 8518 RES directory entries declare an extent past the
end of the file, by at most 57 bytes.

Resource 0x15D90001, `data/15/D9/00/01.DAT`, 1189004 bytes, SHA-256
`9b1fca85933590e6e4d66a1b670408383519ef70b788a5118c3d61cc3c1df891`,
declares `headerSize` 0x30 with five matching directory entries starting
at 0x40 and a payload base of 0x90. The reader retains the declared header
size and records an `unexpected-res-header-size` anomaly: the declared header
size is 0x10 short of the extended fields ending at 0x40 and the directory
starting there. The fields at 0x30..0x3F are parsed as the extended header,
so no bytes in that range are unexplained or reported as a payload gap.
Directory kind values and the other unknown header bytes remain uninterpreted.

The RES native reader establishes `unknownB` as the byte offset of consecutive
NUL-terminated names from `D = S + 0x40 + 16 * count`, where S is the SEDB
start. Reader initializer `0x00e0d120` and name search `0x00e0c8d0` use
`D + field(+0x34)` (`xivl-decomp:docs/resource/gtex-pwib-loader.md`,
"Native consumer boundary", with the executable identity above). Generic
RES inspection adds `res.nameTableOffset` as a raw alias of `unknownB`.
It does not walk names or require the offset to fit. The native directory
is at +0x40; behavior for other larger declared headers remains outside
that consumer evidence.

Supporting observations in
`xivl-decomp:docs/resource/vfx-qix-resource-container.md`, "Banks that hold
the blobs", also place the directory at 0x40 for both declared header sizes.
`xivl-decomp:docs/resource/sqpack.md`, "PackRead's actual scope", distinguishes
the installer chunk reader from runtime DAT resources and establishes no
RES header semantics. Authored public cases `res-header-0x30` and
`res-header-0x30-directory-overrun` cover the accepted shape and a directory
that fits the input but exceeds the declared container extent.

## First-hand retail observation

### Complete header and directory census

A read-only byte scan covered all 191960 installed files: 140180 DAT files,
51772 loose client files, and eight root files. It examined every `SEDB`
signature, including embedded containers, without requiring the advisory
declared size to bound a child. Header candidates required the fixed fields
and declared header to fit the owning file. Each RES directory row was counted
once per RES header, including overlapping rows and rows outside the declared
container. These counts describe stored occurrences, including duplicate
resources in different files, rather than unique resources or runtime loads.

The scan found 529497 headers with the recognized asset tags below. Fifteen
additional signature candidates were kept separate: ten executable matches,
two DAT candidates with non-ASCII tags, and three with header size zero.
The table gives raw value distributions as `value: header count`.

| Subtype | unknownA, decimal | flags |
|---|---|---|
| ACB NUL | 2: 4632 | 0x0000: 4632 |
| CCB NUL | 0: 14795 | 0x0401: 14795 |
| elb NUL | 3: 335 | 0x0200: 335 |
| leaf | 0: 4055 | 0x0400: 4055 |
| lyb NUL | 8: 287 | 0x0000: 287 |
| MCB NUL | 2: 50738 | 0x0000: 50738 |
| mtb NUL | 57: 3, 66: 79, 67: 86811 | 0x0000: 36161, 0x0200: 50732 |
| PHB NUL | 281: 2, 288: 6021 | 0x0400: 6023 |
| RES space | 4000: 83326 | 0x0200: 38446, 0x0400: 44880 |
| SCB NUL | 2: 29621 | 0x0000: 29621 |
| shd NUL | 29: 35978 | 0x0200: 35978 |
| SKL space | 7000: 21884 | 0x0400: 21884 |
| SSCF | 2: 736, 3: 76958 | 0x0400: 77694 |
| txb NUL | 1: 55325 | 0x0200: 55325 |
| veff | 0: 4824 | 0x0400: 4824 |
| vins | 0: 4276 | 0x0400: 4276 |
| vmdl | 0: 14737 | 0x0400: 14737 |
| vtex | 0: 13484 | 0x0400: 13484 |
| wrb NUL | 23: 20590 | 0x0200: 20590 |

Every subtype with an `endianByte` alias had byte +0x0C zero. CCB's observed
0x0401 does not establish its low-byte meaning. The other untraced subtype
fields and the high byte of `flags` remain unresolved regardless of their
distributions.

All 83326 asset RES headers had matching entry and name counts, version 4000,
and a names walk bounded by the owning file when starting at payload base
plus `unknownB`. This checks whole-file bounds, not a nested entry or PWIB
first-segment bound. No new offset constraint follows from this observation.
The raw type tags and offset summaries were:

| Raw type tag | RES headers | Distinct unknownB values | Minimum | Maximum |
|---|---:|---:|---:|---:|
| brt NUL | 31224 | 7608 | 176 | 9927328 |
| nib NUL | 52102 | 9522 | 80 | 34893696 |

The 533540 directory rows had these kind distributions:

| Raw parent type | Kind 0 | Kind 1 | Kind 2 | Kind 4 |
|---|---:|---:|---:|---:|
| brt NUL | 62448 | 0 | 73445 | 33014 |
| nib NUL | 104136 | 463 | 210202 | 49832 |
| Total | 166584 | 463 | 283647 | 82846 |

For example, all 35978 shd and 20590 wrb directory children had kind 2,
whereas RES children had kinds 1, 2 and 4. Non-SEDB children had kinds 0,
1 and 2. These correlations do not assign meanings to kind values.
`xivl-decomp:docs/resource/gtex-pwib-loader.md`, "Native consumer boundary",
leaves the fourth directory dword without a promoted meaning, and
`xivl-decomp:docs/resource/vfx-qix-resource-container.md`, "Banks that hold
the blobs", explicitly leaves kind unresolved.

The file-identity census records relative name, byte size and SHA-256 per
input. Its UTF-8 TSV digest is
`8267068479c7455efdbea006a7aba6b735c1a30fca3202eda95d321bd2eb0b0b`.
An embedded-header witness is `client/vfx/wsc/0001`, 246692 bytes, SHA-256
`76f74e50b94d5975a3fa43dc33486de3dd857591cdc241134dabd14fe218a289`:
the RES header starts at 16, and veff, leaf, vins, vmdl and vtex headers
start at 36768, 61776, 63056, 6016 and 4640 respectively. The separate
RES exemplar below pins the names-offset observation.

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

- `unknownB` is 113376. Adding the payload base 176 gives name-table
  start 113552. Walking the declared seven NUL-terminated names ends at
  113706, exactly EOF, corroborating the native payload-relative names offset;
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
