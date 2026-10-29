# Bounded material samplers

[Documentation index](../README.md)

The material reader reports explicitly selected version-29 `SEDBshd\0`
containers and their `SHD\0` -> `PRAM` sampler framing. Read support is
`partial`. It decodes the sampler count, record offset table, flag byte,
bind argument, and environment-slot byte. Parameter values, strings,
shader programs, and sampler-state bytes remain opaque. There is no writer,
lossless export, or rendering claim.

The [shader reader](shader.md) reports FILE envelopes separately while
retaining opaque program bodies.

```text
xivl inspect <input> --as material
xivl validate <input> --as material
```

The input may be a standalone shd container, a RES tree, or a PWIB resource
whose first segment contains a RES tree. Only explicit material mode selects
these blocks. `entryPath` contains physical RES directory ordinals, without
asserting native selection or ownership. Offsets are absolute input offsets.
All other available bytes remain opaque spans with SHA-256 digests. Generic
SEDB inspection retains its own behavior.

## Evidence

`xivl-decomp:docs/resource/draw-env-texture-binding-boundary.md`,
"Interpretation and unknowns", establishes the runtime handoff boundary and
links the sampler consumer evidence. It establishes no FILE resource
selection, texture default, shader alias, or live renderer result.

`xivl-decomp:docs/resource/chara-engine-samplers-and-light-inputs.md`,
"Sampler records and the engine bind", records PRAM body fields and sampler
reads at VA `0x00C402B0`. Body offsets `+0x14` and `+0x18` supply the sampler
count and record-offset table. Record `+0x03` supplies flags, `+0x0c` the
bind argument, and `+0x0d` the environment slot. The first candidate string
starts at record `+0x20`, supporting a minimum 32-byte prefix. Flag bit
`0x80` selects the environment branch and bit `0x20` passes the state block
at `+0x10`. An argument of `0xff` skips the record. The environment branch
skips slot bytes above `0x1f`. The reader reports the raw bytes, including
those skipped values, without resolving any texture.

`xivl-decomp:docs/resource/chara-material-shader-colour-flow.md`, "Method",
identifies `SEDBshd` containers with an SHD root, FILE children, and PRAM.
`xivl-decomp:docs/resource/pram-packed-render-state.md`, "Payload handoff",
records the inline PRAM body at section pointer plus `0x10` and explicitly
leaves preprocessing unspecified. These observations concern retail 1.23b
`ffxivgame.exe`, build `2012.09.19.0001`, SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.

An independent read of 140180 DAT files and 51772 files under `client/`
enumerated bounded standalone SEDB containers, RES directories, and PWIB
first-segment RES trees. Resource type is established by the exact
`SEDBshd\0` tag and SHD/PRAM framing, not by a filename extension or an
arbitrary signature search.

| Tree | Material resources | shd blocks | Sampler prefixes |
|---|---:|---:|---:|
| DAT | 9969 | 26436 | 138264 |
| client | 6184 | 9542 | 44632 |

These are separate tree counts, without claiming distinct content. Every
selected shd block had version 29, conversion byte zero, header size 0x30,
a declared extent equal to its directory-selected extent, and an SHD root
filling that payload. Each root had FILE, VCAP, PCAP, and one PRAM child.
The root's unmodeled child-count word agreed with the independent walk.
Every sampler offset table and minimum prefix fit its PRAM body. Prefixes
did not overlap one another, the body header, or the offset table. Counts
ranged from zero through ten in DAT and zero through nine in client files.
The reader does not promote those observed maxima to format limits.

Representative full resource identities:

| Resource | Bytes | SHA-256 |
|---|---:|---|
| `data/5C/E5/00/00.DAT` | 106452 | `91dbc69f7dd6772c1be567a0ed7b10da48f321e50fd3d929ddac8388f5f14ef6` |
| `client/chara/bgobj/b001/equ/e001/top_mdl/0001` | 36228 | `5760fd9c3fce477c8f1a9f157bcb5c0c6885f1e33940aad768d43636d6b1ef6e` |
| `client/chara/pc/c001/equ/e216/pch_mdl/0001` | 35280 | `dd5e2f98859bcee333f2a74e5ab9a4ed76fb6847f781c4ba7d7519c951ed02f4` |

The third identity reproduces the decomp example: shd at offset 288,
PRAM section at 13760, and five sampler prefixes. The resource identity
inventories are sorted ASCII lines `path<TAB>size<TAB>sha256<LF>`, with
install-relative POSIX paths and decimal sizes. Their SHA-256 identities
are `45918488f226217bbaa37adfcae9d8bd4deea8d7b1e1efb088029ce0307bda54`
for DAT and
`a948fbc0b6e969059bf8922da24a0fd993ca75ce3f091839afc98cc9de535566`
for client resources. This is first-hand structural evidence, not private
conformance parity or a `verified` status.

CLI inspection replayed every selected full resource:

| Tree | Resources | Accepted | Refused |
|---|---:|---:|---:|
| DAT | 9969 | 9969 | 0 |
| client | 6184 | 5509 | 675 |

Every accepted report matched the independent census's block digests,
physical paths, sampler counts, prefix offsets, and binding bytes. All
675 refusals were `declared-size-out-of-range` at PWIB offset 4, where the
wrapper declared unavailable bytes. The second representative above had
36228 available bytes and a PWIB total and first-segment end of 36240.
The independent census enumerated its available bounded prefix. The CLI
applied the stricter wrapper boundary and refused before material parsing.
The third representative was accepted. These refusals remain outside the
bounded subset rather than being repaired.

## Accepted framing

The shd SEDB header is little-endian. The exact tag includes the NUL byte.
Version at `+0x08` must be 29, conversion byte `+0x0c` zero, and header
size at `+0x0e` 0x30. The declared extent at `+0x10` must fit the selected
slice. Any remaining selected bytes stay opaque. No endian conversion or
preprocessing is inferred.

The exact SHD root begins at shd `+0x30`. Chunk headers are 16 bytes, with
big-endian inclusive size at `+0x08` and padded size at `+0x0c`, following
the [chunk framing contract](wrb-model.md#accepted-structure). The root
size must fill the declared shd payload and include a 16-byte opaque info
block. Children start at shd `+0x50`. Each logical child and its advance
must fit the SHD parent. A padded size at least as large as size supplies
the advance. Otherwise intermediate children advance by size rounded up
to 16 bytes. A final logical child may end exactly at the parent boundary
without that rounding. Only exact PRAM children are interpreted. At least
one is required. Other children, their headers, padding, and root info
remain opaque. The reader does not recurse into shader program bytes.

Let P be the PRAM body start, 16 bytes after its chunk header. PRAM fields
in this subset are little-endian. The body must have at least 32 bytes.

| Body offset | Width | Reader contract |
|---|---:|---|
| 0x14 | 4 | Sampler count N |
| 0x18 | 4 | Body-relative record-offset table T |
| T | N * 4 | Body-relative offsets of minimum 32-byte sampler prefixes |

T must be at least 0x20 and the complete table must fit the body. Each
prefix must fit the body and be disjoint from the header, offset table,
and every other prefix. The reader claims all prefixes before decoding
record fields. It does not infer full variable record lengths.

| Sampler offset | Width | Reported raw field |
|---|---:|---|
| 0x03 | 1 | Flags |
| 0x0c | 1 | Bind argument |
| 0x0d | 1 | Environment-slot byte |

The `+0x0d` byte has evidenced environment-slot meaning only when flag bit
`0x80` is set, and the `0xff` argument skip is evidenced on that branch. For
other records, `environmentSlot` and `argument` are raw bytes without an
established meaning.

Inspection reports these fields, body and prefix spans, offset-table spans,
and digests. All bytes outside these fields and the offset table remain
explicit opaque ranges. It never reports strings or parameter values.

RES headers must be 0x30 or 0x40, with directories at 0x40 and equal
counts at 0x30 and 0x38. Selected shd and RES rows must fit their exact
parent extents. All selected sibling extents are claimed before recursion.
Repeated extents and overlapping selected siblings are refused at the
row's length field. Each byte can be claimed only once at each bounded
RES depth, preventing aliased rows from multiplying recursive work.
Unselected rows may alias opaque data or declare unavailable alignment
extents. Those rows remain opaque. Outer PWIB boundaries follow the
[PWIB contract](gtex-pwib.md).

## Refusals and gaps

Wrong outer tags use `bad-magic`. Unsupported versions and conversion use
`unexpected-chunk`. Header sizes use `invalid-chunk-length`, escaped shd
extents use `declared-size-out-of-range`, and SHD child or root bounds use
`chunk-length-out-of-range`. Missing SHD or PRAM uses `unexpected-chunk`.
Sampler table and prefix bounds use `table-out-of-range` at the body table
field or offending table entry. Overlaps use `ambiguous-payload-span` at
the offending entry. Truncated fields use `unexpected-end-of-input`.
Typed errors carry absolute input offsets. Container errors retain their
existing kinds and offsets.

The packed state word, alpha reference, parameter count and value tables,
sampler name-offset table at body `+0x1c`, candidate strings, string
encoding, prefix magic, stage metadata, sampler-state bytes, and shader
program contents are opaque. The reader validates no name reference or
parameter index and applies no runtime skip or binding rule. Native
selection, resource ownership, default textures, aliases, shader semantics,
rendering parity, writing, and lossless export remain outside the claim.
No parent or hierarchy indices are interpreted by this material subset.

Authored public fixtures cover empty and populated sampler tables, nested
RES paths, PWIB, opaque aliases, validation, and typed refusals for tags,
version, conversion, header sizes, truncation, parent bounds, table bounds,
prefix overlaps, and selected RES aliases. They come from
`tools/make_public_fixtures.py` and contain no retail content.
