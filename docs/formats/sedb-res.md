# Promoted evidence: SEDB, RES, and resource paths

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

Citations:

- [World-layout data](https://github.com/BahamutXIV/bahamut-navmesh/blob/e9f384c9d8c37e1942d942fb1566dc2967163f3e/docs/world-layout-data.md)
- [Model and collision data](https://github.com/BahamutXIV/bahamut-navmesh/blob/e9f384c9d8c37e1942d942fb1566dc2967163f3e/docs/model-and-collision-data.md)

The source statements, quoted verbatim for the client revision they record:

> Retail world-layout evidence behind the MapLayout and scene parsers.
> Source: FFXIV 1.23b client install, `game.ver 2012.09.19.0001`.
> Evidence includes MapLayout 0x29D90001, its complete `lyb` scene
> container, resource-key bindings, unit-tree groups, placed-object
> classes, and transform composition checked across the source install's
> MapLayout resources.

> Retail geometry evidence behind the SEDB model and PHB collision
> parsers. Source: FFXIV 1.23b client install, `game.ver
> 2012.09.19.0001`. Evidence includes models 0x89940554, 0x899500BF,
> 0x897D03A9, 0x72E90455, and 0x72E50007; mesh chunk checks across the install;
> the `col_*` primitive hulls; and PHB collision resources read across the
> source install.

Additional resources in the evidence set are
0x7EB50002 (bhp), 0x89930214 (bxt texture) and 0x898C000D (sniv).

Only the three sections below are promoted. The model, mesh, and chunk-tree
layouts in the cited document are outside this page's claim.

One first-hand retail observation refined a promoted statement: the field at
0x10 is not a container size for every subtype. The citation remains intact
because the provenance of a claim does not stop being true when the claim is
refined.

Local tests own each promoted fact from here on:
`src/formats/src/resource.rs` tests for the path mapping,
`tests/conformance/cases/` for the container layouts, and
`src/formats/tests/malformed_inputs.rs` for the failure behavior.

## Resource ID to DAT path

A 32-bit resource id `0xAABBCCDD` names `data/AA/BB/CC/DD.DAT` under the
client install root, uppercase hexadecimal. Established empirically across
the whole install and by resolving MapLayout table references to existing
files.

The mapping is total: every `u32` names a path, whether or not the file
exists. This repository never resolves a path against a client install on
its own. An install root is always supplied by the caller.

## SEDB container header

All integer fields are little-endian.

```text
0x00  char[4] magic      "SEDB"
0x04  char[4] subtype    "lyb", "PHB", "txb", "vins", "vmdl", "RES ",
                         "shd", "SKL ", "wrb", ...
0x08  u32     unknownA   varies per subtype (8 lyb, 0x120 PHB, 1 txb,
                         0 vins/vmdl, 0xFA0 RES, 0x1D shd, 0x17 wrb);
                         meaning unresolved
0x0C  u16     flags      0x0000, 0x0200, or 0x0400 observed; meaning
                         unresolved
0x0E  u16     headerSize payload begins at container start + headerSize
                         (0x30, 0x40, 0x48 observed)
0x10  u32     declaredSize  advisory; see "The 0x10 field" below. It is
                         the file size for most subtypes, the header size
                         for PHB, zero for mtb, and a genuine sub-file
                         extent for vins and leaf
...   padding to headerSize; subtype-specific extended fields may sit
      here, and RES uses 0x30..0x3F
```

`0x14` is therefore the smallest header a container can declare. The parser
rejects a smaller one, and rejects a `headerSize` or `declaredSize` that
runs past what the input holds, rather than reading what happens to follow.

## RES subresource directory

A `RES ` container is a composite with `headerSize` 0x40 and a subresource
directory. Extended header fields inside the header:

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

Alignment slack is part of the format as observed, not a defect to reject:
the final directory entry of 0x89940554 declares an extent 0x16 bytes past
the end of the file (0x1BC40 against a 0x1BC2A file). The parser clamps
such an extent to the input and reports the clamp as an anomaly. It never
reads past the input and never drops the entry.

## First-hand retail observation

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b), observed 2026-08-01 over all 140180 resource files under
`data/`. Reproduce with:

```bash
python tools/research/census_sedb.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints counts rather than
bytes.

### The resource-path convention

All 140180 files under `data/` match `AA/BB/CC/DD.DAT`, confirming the
promoted convention across the whole install with no exception.

### The 0x10 field

The promoted statement, "container size including the header equals the
remaining file size for top-level containers", holds for 92303 of the
97749 SEDB resources and fails for 5446 of them:

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

Read directly from `data/89/94/05/54.DAT` (resource 0x89940554), the model
the cited document names:

- the payload base is `headerSize + 16 * subresourceCount` (0x40 + 0x70 =
  0xB0). Subresources 0 through 4 begin exactly on `SEDB` signatures at
  0xB0, 0x55E8, 0xA638, 0xEED8, and 0x1B9E0, which is what fixes the base;
- consecutive subresources are separated by up to three bytes of
  alignment padding, which this parser reports as unknown gaps rather than
  folding into either neighbour;
- the final entry declares 0x1BB20..0x1BC40 against a 0x1BC2A file: the
  0x16 bytes of slack the cited document records, reproduced exactly;
- the last two entries overlap (0x1BB20 begins inside 0x1BB04..0x1BBD0).
  Sampling 107 RES resources found this in all 107, so the overlapping
  trailing region is a property of the format, not a defect. Declared
  offsets and sizes are reported verbatim alongside the resolved spans, so
  the overlap is visible rather than silently resolved.

### Parser behavior over the install

A 1508-file sample, every 93rd resource, run through `xivl inspect`: 1056
SEDB resources parsed, 0 errors, 0 panics. The remaining 452 are not SEDB
at all. They are reported as `bad-magic`: GTEX 226, zero-filled 59, PWIB 38,
plus a tail of smaller signatures. None of those formats is handled here.

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
