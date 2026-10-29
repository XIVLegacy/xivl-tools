# Bounded skeleton framing

[Documentation index](../README.md)

The skeleton reader reports an explicitly selected little-endian `SEDBSKL `
block, including the final tag space. Read support is `partial`. It decodes
176-byte record framing, three string indices, and four hierarchy indices.
String text and transform contents remain opaque. There is no writer,
lossless export, skinning, animation, or rendering claim.

```text
xivl inspect <input> --as skeleton
xivl validate <input> --as skeleton
```

The input may be one exact SKL block, a RES tree, or a PWIB resource whose
first segment contains a RES tree. Only this explicit mode enumerates SKL
blocks. Physical RES directory ordinals identify each block in `entryPath`.
Enumeration does not establish the client's native selection or ownership.
All source offsets are absolute input offsets. Other resource bytes remain
opaque spans with digests. Generic SEDB inspection retains its own behavior.

## Evidence

`xivl-decomp:docs/resource/skeleton-bone-framing.md`, "Native framing
observations" and "Record indices and hierarchy", records the retail 1.23b
consumer equations and accessors. Its executable is `ffxivgame.exe`, build
`2012.09.19.0001`, SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.
The native framing reader is VA `0x00e0dd00`. String accessors are at
`0x00e11de0`, `0x00e11e20`, and `0x00e11e60`. The hierarchy insertion path
is at `0x00e0e150` and `0x00e0e1b0`.

The independently read install contained 140180 DAT files and 51772 files
under `client/`. Bounded enumeration of standalone SEDB resources, RES
directories, and PWIB first-segment RES trees found one SKL block in each
of 9969 DAT resources and 9959 client resources. These are separate tree
counts, without a claim that the resources are distinct content.

Every selected block had scalar 7000, conversion byte zero, header size
0x30, one directory row, zero at 0x44, and a declared extent equal to its
directory-selected extent. The record byte length was divisible by 176,
and the string offset equaled that length. Counted strings terminated
within their selected extents, leaving zero through three opaque bytes.
No encoding or alignment meaning follows from those observations.

DAT blocks contained 1 through 344 records and 4 through 348 strings.
Client blocks contained 0 through 691 records and 1 through 695 strings.
The longest measured strings were 24 and 20 bytes respectively. Every
string index was in range. DAT blocks passed the hierarchy range and self
ordinal checks. In the client tree, 480 blocks failed at least one of
those checks. The reader refuses such indices instead of resolving them
against another skeleton. Primary name indices need not be unique.

CLI inspection replayed every selected full resource:

| Tree | Resources | Accepted | Refused |
|---|---:|---:|---:|
| DAT | 9969 | 9969 | 0 |
| client | 9959 | 5394 | 4565 |

All 336 client resources under `/skl/` paths were accepted. The broader
client census also includes SKL blocks embedded in model resources.
Of its refusals, 4114 were `declared-size-out-of-range` at PWIB offset 4,
where the wrapper declared bytes beyond the file. The remaining 451 were
`table-index-out-of-range` at hierarchy fields whose references exceeded
the local record count. The independent index census found 29 additional
mismatches among the refused wrappers, accounting for its total of 480.
Every accepted report's selected block digest matched the independent
census. Refusal leaves the resource outside this bounded subset.

Representative full resource identities:

| Resource | Bytes | SHA-256 |
|---|---:|---|
| `data/5C/E5/00/00.DAT` | 106452 | `91dbc69f7dd6772c1be567a0ed7b10da48f321e50fd3d929ddac8388f5f14ef6` |
| `client/chara/pc/c001/skl/0001` | 54672 | `d90a756fc1cba6f03bb8dc102b367feb905b5e57a00d209ff4fccddaa17cd031` |
| `client/chara/mon/m001/equ/e001/met_mdl/0001` | 13428 | `c16da8b5f415766828b8dd8ffdb3f089a31cbf8432bc9595a9634ea1a493ccc4` |
| `client/chara/mon/m980/equ/e001/dwn_mdl/0001` | 229520 | `efc4a4aa118516a6226ac7b95812fa352a9d6fcc4a40cc3bef7aff6d1714f770` |

The second resource reproduces the decomp selection `[3,0]`, SKL extent
`[5296,23108)`, 95 records, 99 strings, and one byte after the counted
pool. The third resource's PWIB total exceeds its available file length,
so inspection refuses the wrapper before reading its SKL indices.
The fourth resource is refused at hierarchy field offset 228712.

The resource identity inventories are sorted ASCII lines
`path<TAB>size<TAB>sha256<LF>`, using install-relative POSIX paths and
decimal sizes. Their SHA-256 identities are
`45918488f226217bbaa37adfcae9d8bd4deea8d7b1e1efb088029ce0307bda54`
for DAT resources and
`13492c90c1ded250919ef55937af2ea3a184137541e35c3f57302031ccc42bce`
for client resources. They identify the census, without publishing retail
bytes. This is first-hand structural evidence, not private conformance
parity or a `verified` status.

Replay with an explicit external install and optional built CLI:

```text
python tools/research/census_skeleton.py --client-root <install> --xivl <xivl-executable>
```

The independent census enumerates available bounded prefixes even when
PWIB metadata declares a longer file. CLI replay applies the stricter
PWIB boundary checks and records those refusals separately. The research
command makes no support claim and does not run in CI.

## Accepted block shape

Let B be the selected block start and R its record base. Multibyte SKL
fields are unsigned little-endian values. Outer PWIB boundaries follow
the [PWIB contract](gtex-pwib.md), and RES directories follow the
[RES contract](sedb-res.md#res-subresource-directory).
Accepted RES headers are 0x30 or 0x40 bytes, with the directory at 0x40.

| Offset from B | Width | Reader contract |
|---|---:|---|
| 0x00 | 8 | Exact `SEDBSKL ` tag |
| 0x08 | 4 | Scalar 7000; its purpose is unresolved |
| 0x0c | 1 | Zero; the conversion path is refused |
| 0x0e | 2 | Header size 0x30 |
| 0x10 | 4 | Equals the entire selected extent |
| 0x30 | 4 | One directory row |
| 0x34 | 4 | String offset relative to R, equal to record-array bytes |
| 0x38 | 4 | Number of NUL-terminated strings |
| 0x44 | 4 | Zero; no directory-offset semantics are inferred |
| 0x48 | 4 | Record-array bytes, divisible by 176 |

The native equation is `R = B + 0x30 + 16 * (count + 1)`. The accepted
count makes R equal to B+0x50. Both arrays must fit the selected slice.
The reader scans exactly the declared number of NUL terminators, bounded
by that slice, and retains all remaining bytes as opaque. It does not
apply a text encoding or infer a truncation rule from the native scan.

| Record offset | Width | Decoded field |
|---|---:|---|
| 0x00 | 4 | Primary name index |
| 0x04, 0x08 | 4 each | Auxiliary string indices, purposes unresolved |
| 0x38 | 4 | Parent record index |
| 0x3c | 4 | First-child record index |
| 0x40 | 4 | Next-sibling record index |
| 0x44 | 4 | Self record index |

String indices must be below the counted string total. Parent, child,
and sibling indices must be below the record count or equal to
`0xffffffff`, the no-link sentinel. The self index must equal the record
ordinal. Parent chains must terminate without cycles, including self links.
Inspection reports these references, string spans including
terminators, root counts, array spans and digests, and opaque spans and
digests. It never prints names or transform bytes.

## Refusals and gaps

This is a framing reader, not an owned bone-transform model. Unknown
header bytes, extended words at 0x3c and 0x40, directory word 0x4c,
record bytes `[0x0c,0x38)` and `[0x48,0xb0)`, and post-pool slack remain
opaque. String encoding, auxiliary index purposes, hierarchy ordering,
child and sibling cycles, and parent-to-child-chain consistency are outside
validation. The reader checks parent chains but does not resolve ownership.

The native scalar gate accepts at least 7000, but only 7000 is established
by the census. Other scalars, conversion byte values, header sizes,
directory counts and nonzero 0x44 values are refused. The reader does not
repair clipped wrappers, infer missing array bytes, or search arbitrary
payloads for signatures. Selected SKL and RES directory rows must fit their
exact containing extents. Repeated selected extents and overlapping selected
siblings are refused at the row's length field before following that row.
Malformed selected RES containers are refused. Other directory rows can
alias opaque data or declare alignment extents beyond the available input.
Those rows are not materialized or interpreted. All available bytes outside
selected SKL blocks remain opaque spans with digests.

Typed errors carry the absolute failing offset. Unsupported measured
shapes and parent cycles use `unexpected-chunk`, with cycles pointing to the
parent field that closes the cycle. Wrong tags use `bad-magic`. Header size
uses `invalid-chunk-length`, an extent mismatch uses
`declared-size-out-of-range`, a partial record uses
`trailing-partial-record`, and invalid array framing uses
`table-out-of-range`. String and hierarchy index failures use
`table-index-out-of-range` at the field. A missing NUL uses
`unexpected-end-of-input` at that string's start. Outer container errors
retain their existing kinds and offsets.

Transform translation, rotation, scale, matrices, inverse binds,
coordinate space, handedness, and units are unresolved. Mesh skin
palettes, weights, joint selection, animation channels, actions, timing,
frame rates, interpolation, defaults, and live rendering parity remain
outside the read claim. JSON inspection is a report and establishes no
lossless export status.

Authored public fixtures cover the accepted block with duplicate name
indices and nonzero opaque bytes, nested RES physical paths, PWIB,
validation, and typed refusals for the block gates, array framing,
indices, parent cycles, selected aliases, missing terminators, truncation,
and wrapper bounds. Their bytes come from `tools/make_public_fixtures.py`
and contain no retail content.
