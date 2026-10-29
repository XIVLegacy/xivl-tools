# Animation bank and MTB framing

The explicit animation reader reports bounded bank entries and version `0x43`
MTB section framing. Read support is `partial`. Scheduler and motion payloads
remain opaque. No writer, animation export, pose sampling, or playback is
implemented.

```text
xivl inspect <input> --as animation
xivl validate <input> --as animation
```

Automatic inspection retains its existing container selection. Animation
inspection requires `--as animation`.

## Evidence

`xivl-decomp:docs/actor/animation-bank-routing.md`, "Packed selector", establishes
that character and effect banks are loaded independently. Its "Raid-dungeon
warp bank", "Food and drink bank composition", and "Arrival type and revival
motions" identify retail bank resources, their SHA-256 identities, and their
scheduler/motion relationships. These relationships establish bank contents
and routing capability, without establishing a selected actor or playback.

The binary framing comes from
`xivl-decomp:docs/resource/mtb-container-framing.md`, "Header and tables".
It checks the retail executable with SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.
Setup `0x00A2D5E0` and accessors `0x00A2D510`, `0x00A2D750`, and
`0x00A2D4B0` establish the three tables and section-name lookup. The version
and property-count fields are read at `0x00A2D700` and `0x00A302C1`.
The [PWIB contract](gtex-pwib.md#pwib-contract) owns its big-endian segment
boundaries. The [SEDB/RES contract](sedb-res.md) owns the little-endian fixed
header and direct resource directory.

## Retail census

`tools/research/census_animation.py` independently reads the resource trees
under explicit `data` and `client` roots. It reads every resource header and
scans complete PWIB, RES, and standalone animation resources for MTB, MCB, and
SCB signatures. It also scans complete files below character `act`
directories, `client/vfx`, and `client/cut`, and selects every PWIB there, so
control-only banks do not depend on an embedded animation signature. The
character/effect routes come from the routing note. Cutscene SCB framing is
recorded in `xivl-decomp:docs/resource/cut-scheduler-clip-layout.md`, "Resource
header and directory". The CIB families in
`xivl-decomp:docs/resource/monster-action-scheduler-corpus.md`, "Banks without
SCB", remain opaque to this reader. Only bounded resources contribute decoded
framing measurements. `--cli` also invokes explicit animation inspection on
every candidate, including those the independent framing walk refuses.

```text
python tools/research/census_animation.py --client-root <install> --cli <xivl>
```

The install's executable matched the identity above and `game.ver` read
`2012.09.19.0001`. The census scanned all 140180 DAT files and 51772 client
files, selecting 19475 candidate resources. The sorted identity list has
SHA-256 `12a5668cd23568211a049a331ade8586bf6d7ed6b321691b57f6d08c2f8d255a`.
The digest covers ASCII lines of `resource size sha256` followed by LF, sorted
by resource identity, as the census command emits them. DAT identities use
their eight-digit resource ID and client identities use install-relative
resource paths.

The candidate predicate includes signature hits and route-selected PWIB
files. It does not identify every candidate as an animation bank:

| Candidate shape | Resources |
|---|---:|
| RES bank with direct animation entries | 18751 |
| Standalone MTB | 145 |
| Other PWIB/RES input with no direct animation entry | 133 |
| Unwrapped RES signature hit with no direct animation entry | 2 |
| PWIB envelope whose boundaries exceed the supplied file | 444 |

The independent walk framed 35667 MTB resources, using whole slices for
standalone MTBs and complete directory entries for bank members. Every one
had version 67, header size 64, and advisory size zero. These counts exclude
nested MTBs. The directory walk found 11491 unavailable opaque entry extents
and no unavailable selected animation extent. The animation reader clamps
those opaque extents without validating or claiming them. The 444 unavailable
PWIB envelopes received no independent framing walk and are not counted as
framed banks.

Explicit CLI inspection accepted 18896 candidates, including 35667 MTBs, and
refused 579. All input sizes and SHA-256 identities were checked against the
census before inspection. Refusals were:

| Condition | Error | Resources |
|---|---|---:|
| No direct recognized animation entry: 133 other PWIB/RES inputs and two unwrapped RES signature hits | `unexpected-chunk` | 135 |
| PWIB boundaries exceeded the input | `declared-size-out-of-range` | 444 |

These first-hand witnesses tie the census to the routing note and to a
standalone MTB:

| Resource identity | Size | SHA-256 | Inspection |
|---|---:|---|---|
| `0x5CE60000` | 303 | `adcb48ed7b5c13c19890abb9b1d824e841be4effa742219fec20022094dc85dd` | Accepted standalone MTB |
| `client/chara/pc/c001/act/cmn/em1/base/4002` | 17408 | `53677f20d6c74f810dbbfd3dd274ae446ad30232e0bb1e06a93e6c84df5afda4` | Accepted bank with one MTB, one MCB, and one SCB |
| `client/chara/bgobj/b936/act/cmn/lib/base/0094` | 1232 | `ba71a7d66808704200a99eb508d306e8fae2ffe5fdd98bc4135f8f8f0dd74cd0` | Accepted scheduler bank with two clamped opaque rows |
| `client/vfx/pop/0007` | 130240 | `ffd87484a18d27242fc25e5202d7459702815063ec9e33873dc555e82474d950` | Accepted scheduler bank with one clamped opaque row |
| `client/chara/mon/m999/act/cmn/fid/base/0001` | 4009 | `8fbabefe9eba3f43fffb65ba46c6531996241a0324e6dd04cb79b64de0c7b4a0` | Refused PWIB extent at offset 4 |

The accepted drink bank's MTB has version 67, header size 64, advisory size
zero, two sections, two section names, and 17 group names. Its sections select
`Header` and `SpuBinary`. Refusing a declared extent outside the supplied
segment does not establish that a retail asset is invalid.
The m999 control-only bank declares a 4016-byte PWIB envelope, seven bytes
longer than the file. It has no SEDB animation signature. Directory-based
selection includes it without guessing at its control payload.

## Accepted framing

An input is a standalone SEDB `mtb` NUL, `MTB `, `MCB` NUL, `MCB `,
`SCB` NUL, or `SCB ` resource, or a SEDB `RES ` bank containing at least one
direct resource with one of those tags. A PWIB wrapper is accepted around
either shape. Only entries whose first eight available bytes are SEDB plus
one of these animation tags are selected. Selected entries must fit wholly
inside the first segment and must not overlap or alias another selected
entry. Nested banks and the second PWIB segment remain opaque.

MCB and SCB receive only a fixed SEDB header check and an opaque entry span
and digest. Their commands, scheduler blocks, and timing fields are not read.
Unrecognized entries remain unvalidated and unclaimed. Their reported spans
are clamped to the first segment, with `clamped` recording an unavailable
start or end. A SEDB-like prefix in one does not authorize interpreting its
internal size or payload fields. Opaque rows receive no individual digest:
the file's opaque complement accounts for their unclaimed bytes once, even
when directory rows alias or overlap.

The MTB reader accepts version `0x43` with header size `0x40`. These fields
are little-endian and relative offsets are measured from that MTB's SEDB
signature, including when the resource is embedded in a bank:

| Offset | Width | Read |
|---|---|---|
| `0x08` | u32 | Version |
| `0x0E` | u16 | Header size and first table offset |
| `0x10` | u32 | Advisory SEDB size, not an MTB extent |
| `0x30` | u16 | Raw total-property field; the native allocation uses its low 12 bits |
| `0x40` | three consecutive tables | Each table has a u32 count followed by that many u32 offsets |

Table 1 contains section offsets. Table 2 contains NUL-terminated section-name
offsets. Table 3 contains NUL-terminated group-name offsets. Names must start
after the tables and terminate inside the bounded resource. Name extents,
including their NUL terminators, must not overlap or alias across either
table. Each section starts at a distinct 16-byte-aligned offset after the
tables and begins with a table-2 name index. The version-`0x43` section
header occupies 16 bytes. At least one section must select the first
table-2 index whose exact byte string is `Header`. The cited lookup returns
the index of an exact match without establishing which duplicate it
returns, so the reader conservatively treats only the first match as the
named kind; later duplicate names do not select it.

No section-length field is established. A section's reported `view` ends at
the next section offset, name start, or resource end. This is a bounded view,
including any alignment slack, rather than a decoded payload length. The
first dword is interpreted as the name index. Everything after that dword,
including the rest of the section header, remains opaque. `Header`, `Curve`,
and `SpuBinary` are recognized structural names. All other names report kind
`opaque`. Reports expose name spans and digests, never their contents.

The library retains table offsets and section indexes. Inspection reports
table counts, spans and digests, section views, entry subtypes, and opaque
spans and digests. The supplied slice is the authoritative extent for a
standalone MTB whose advisory size is zero. Embedded MTB reads use the RES
entry's declared bounded slice instead of searching for another signature.

## Refusals

| Condition | Error | Offset |
|---|---|---|
| A required fixed field is missing | `unexpected-end-of-input` | Start of the missing field |
| Signature differs | `bad-magic` | SEDB signature |
| SEDB header is below `0x14`, exceeds the input, or declares unavailable bytes | `header-too-small`, `header-size-out-of-range`, or `declared-size-out-of-range` | Header size or declared size field |
| PWIB boundaries escape the input or are unordered | `declared-size-out-of-range` | `0x04` |
| RES counts disagree or its directory cannot fit | `subresource-count-mismatch` or `subresource-count-out-of-range` | Count field |
| Selected animation entry escapes the first segment, overlaps, or aliases another selected entry | `ambiguous-payload-span` | Offending entry offset field |
| Unsupported subtype, no direct animation entry, unsupported MTB version, or missing Header section | `unexpected-chunk` | Subtype, version, or section table |
| MTB header size differs from `0x40` | `invalid-chunk-length` | Header-size field |
| Offset table, name start, or section header has no valid bounded view | `table-out-of-range` | Count or offset entry |
| Section offsets alias | `ambiguous-payload-span` | First section-offset entry |
| Name extents overlap or alias | `ambiguous-payload-span` | Offending name-offset entry |
| Section name index exceeds table 2 | `table-index-out-of-range` | Section's name index |
| A name has no bounded terminator | `unexpected-end-of-input` | Name start |

Offsets in failures and reports are absolute within the supplied input,
including failures in an embedded MTB. The exact version `0x43`, 16-byte
section alignment, and names starting after the tables are stricter checks
than the client's pointer-based accessors. Independent, bounded name and
selected-entry extents also prevent repeated scans and hashing of aliased
storage. The reader never adds missing padding or infers a resource extent
from an embedded signature.

## Gap statement

Only the container directory and MTB section framing are read. Header payload
fields, group-to-curve selection, curve records and sampling, SpuBinary motion
data, transforms, bone bindings, MCB commands, SCB scheduler blocks and clips,
time units, CIB control tables, effect payloads, and runtime routing and
playback remain outside this claim. Version `0x42` and older MTBs are refused.
SEDB flag meanings and unread header bytes remain opaque.
Other SEDB entries and nested banks remain opaque. Banks containing only CIB
controls have no direct recognized animation entry and are refused.

Public authored fixtures cover standalone MTB, opaque SCB, a PWIB/RES bank,
and malformed boundaries, table counts, indexes, offsets, names, and version.
They are generated by `tools/make_public_fixtures.py`. They contain invented
payloads and do not establish curve or scheduler validity. No private
conformance case establishes retail parity for this partial read claim.
