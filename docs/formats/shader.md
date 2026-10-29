# Bounded shader FILE framing

[Documentation index](../README.md)

The shader reader reports FILE chunk envelopes in explicitly selected
version-29 `SEDBshd\0` containers. Read support is `partial`. Reports retain
physical RES paths, FILE counts, logical chunk and body spans, padded-size
words, SHA-256 digests, and opaque ranges. FILE bodies remain entirely opaque.
There is no shader disassembly, bytecode validation, writer, lossless export,
or rendering claim.

```text
xivl inspect <input> --as shader
xivl validate <input> --as shader
```

Only explicit shader mode selects this view. Inputs may be standalone shd
containers, RES trees, or PWIB resources whose first segment contains a RES
tree. Offsets are absolute input offsets. `entryPath` contains physical
directory ordinals, without asserting native selection or ownership. Generic
SEDB and material inspection retain their separate reports.

## Evidence

`xivl-decomp:docs/resource/bitmap-font-shader-consumers.md`, "Selected program
pair" and "Reproduction boundary", establishes bounded executable-embedded
vertex and pixel programs and their independently measured payload identities.
It establishes no FILE resource framing. Its shader token interpretation does
not extend this reader's claim.

`xivl-decomp:docs/resource/chara-material-shader-colour-flow.md`, "Method",
identifies SEDBshd sections with an SHD root, FILE children, and PRAM in
installed character resources. `xivl-decomp:docs/actor/body-shader-constant-join.md`,
"Profile and identity" and "Resource, selection and final binding", pins
`client/chara/pc/c001/equ/e003/top_mdl/0001` at 107152 bytes with SHA-256
`21570ade19172245ed9bb541fa3c2c7d4a74d5cd8f19500468abce51f788d093`.
It records the first shd at offset 320, declared length 15673, and the first
FILE at offset 400, size 3048, with relative-next value 3056. A first-hand
read reproduced that full resource identity and those boundaries. The cited
inline descriptor and program payload remain opaque in this reader.

These decomp findings concern retail 1.23b `ffxivgame.exe`, build
`2012.09.19.0001`, SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.
Installed-file identity does not establish original retail asset provenance.

An independent first-hand census enumerated exact SEDB tags, bounded RES
directories, PWIB first-segment RES trees, and SHD child envelopes. It scanned
140180 DAT files and 51772 files under `client/`. Resource selection used
container framing, without scanning arbitrary payloads for signatures.

| Tree | Resources containing shd | shd blocks | FILE chunks | Logical FILE size range |
|---|---:|---:|---:|---:|
| DAT | 9969 | 26436 | 101472 | 560..7472 |
| client | 6184 | 9542 | 28626 | 696..7440 |

The trees are counted separately, without claiming distinct content. Every
selected shd had version 29, conversion byte zero, header size 0x30, a bounded
declared extent, and an SHD root filling that extent. Every root contained
FILE children and one PRAM. All logical and advanced child extents fit their
parent. These observations establish framing, not bytecode semantics. The
size ranges are observations, not reader limits.

The resource inventories are sorted ASCII lines
`path<TAB>size<TAB>sha256<LF>`, using install-relative POSIX paths and decimal
sizes. Their SHA-256 identities are
`45918488f226217bbaa37adfcae9d8bd4deea8d7b1e1efb088029ce0307bda54`
for DAT and
`a948fbc0b6e969059bf8922da24a0fd993ca75ce3f091839afc98cc9de535566`
for client resources. The [material census](material.md#evidence) records
representative resource identities and the shared wrapper boundary.

CLI inspection replayed every selected full resource:

| Tree | Resources | Accepted | Refused |
|---|---:|---:|---:|
| DAT | 9969 | 9969 | 0 |
| client | 6184 | 5509 | 675 |

Every accepted report matched the independent census's shd identities, physical
paths, FILE spans, padded-size words, body digests, and opaque byte accounting.
All 675 refusals were `declared-size-out-of-range` at PWIB offset 4, where the
wrapper declared unavailable bytes. The independent census enumerated only
the available bounded prefix. No previously accepted material resource acquired
a refusal, and all prior refusal kinds and offsets were unchanged. This is
first-hand structural evidence, not private conformance parity or a `verified`
status.

## Accepted framing

The reader reuses [material selection](material.md#accepted-framing), including
its version, conversion, header, SHD, PRAM, RES, and PWIB checks. A valid
material sampler table is consequently required even though the shader report
does not expose sampler fields. Selected RES and shd extents are claimed
before recursion. Repeated or overlapping selected sibling extents are refused,
so aliases cannot multiply recursive work. Nesting retains the existing
eight-level bound.

SHD children start at shd `+0x50`. Their 16-byte headers contain an exact
four-byte tag, four opaque bytes, a big-endian inclusive size at `+0x08`, and
a big-endian padded-size word at `+0x0c`. A padded size at least as large as
the inclusive size supplies the advance. Otherwise an intermediate child
advances by its size rounded up to 16 bytes. A final logical child may end
at the parent boundary without rounding. Every logical and advanced extent
must fit the SHD root.

At least one exact `FILE` child is required in each selected shd. The report
retains each FILE's complete logical span, body span after the 16-byte header,
padded-size word, and digests. It does not use a filename extension, candidate
table, string, or guessed signature to locate a program inside that body.
The reserved FILE header word, every FILE body, padding, root info, non-FILE
children including PRAM, unmodeled SEDB fields, and bytes outside selected
containers remain explicit opaque spans with digests. The modeled fields and
opaque spans account for every input byte.

## Refusals and gaps

Missing FILE children use `unexpected-chunk` at the SHD root. All reused
container and material failures retain their typed kinds and absolute offsets,
including child bounds at the size or padding field and selected aliases at
the RES row's length field. Truncated inputs use `unexpected-end-of-input`
or `declared-size-out-of-range`.
`validate` performs the same bounded read and marks round-trip as not applicable.

FILE descriptors, external-code flags, inline program offsets and lengths,
program kind and version tokens, instructions, CTAB, constants, names, candidate
selection, shader aliases, byte order conversion, preprocessing, and runtime
binding are unresolved by this subset. No bytecode is disassembled or
interpreted. Framing success establishes no GPU acceptance, shader semantics,
rendering parity, writing, or lossless export. The PRAM and wrapper restrictions
in the reused material reader also limit this shader subset.

Authored public cases cover multiple FILE envelopes, both advance forms,
nonzero padding and opaque bodies, unknown children, nested RES paths, PWIB,
validation, missing FILE, truncation, header gates, root and child bounds,
selected aliases, and absolute nested error offsets. Their bytes are defined
by `tools/make_public_fixtures.py` and contain no retail content.
