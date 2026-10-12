# Bitmap font container (fdt)

[Documentation index](../README.md) | [Format evidence](../format-evidence.md)

The `fdt` reader walks a bitmap font container in the chunk order the retail
client reads. It reports chunk spans, digests, version gates, lookup-table
counts, and GLYP record checks, and it supplies structural inspection and
validation. The library decodes each GLYP record into the fields the client's
glyph lookup and draw path consume. Read support is `partial`. Metric scalar
meanings, texture payloads, rasterization, writing, and export are outside
this contract.

## Evidence and identity

Promoted facts come from
`xivl-decomp:docs/resource/bitmap-font-container.md`. That page reads retail
1.23b `ffxivgame.exe`, SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`, image
base `0x00400000`, with Capstone 5.0.7. It records two FontData readers,
`FUN_00CA65C0` and `FUN_004EAD50`, that consume one tag sequence through a
shared chunk cursor at `FUN_004EBD40`. Both clear the cursor's byte-swap flag
and never set it, so chunk lengths are little-endian. The same page records
a census of every file below the 1.23b `data/` directory.

The lookup roles come from
`xivl-decomp:docs/resource/bitmap-font-glyph-consumers.md`, "Lookup and
normalized output". A 16-bit key's high byte selects a page byte, where
`0xFF` fails. The page times 256 plus the key's low byte selects a 16-bit
record index, where `0xFFFF` fails. The index times 14 selects a record.

The census found seven font resources. Running `xivl inspect --as fdt` on
each accepted all seven, with these structural counts:

| Path | Size | SHA-256 | GLYP records | Records reached by a key |
|---|---|---|---|---|
| `data/00/0C/00/00.DAT` | 823966 | `0cadca25ed8087289fb47732b5d8ca0fb109c73c758256ccd33eec2dddbf1b1b` | 197 | 197 |
| `data/00/0C/00/01.DAT` | 16674822 | `8d378a99aacc9ee785ef89af795f02a97f726b606df06c61c8e227cdcbf74f38` | 15721 | 15721 |
| `data/00/0C/00/02.DAT` | 14116294 | `c4ac2eface64285bee0a6c2ade77c0780cafba313b4e7c940eee42114dbea3d8` | 9609 | 9390 |
| `data/00/0C/00/03.DAT` | 823966 | `6065adf8b51668ffbff058dc22e3efdcae8b58d54fce5d8825646cf539809782` | 197 | 197 |
| `data/00/0C/00/04.DAT` | 410724 | `fc1f64c16825749c22d7a32c8275759856aa6b8118f10e8dbbb7f8c2614af5cd` | 10 | 10 |
| `data/27/95/00/01.DAT` | 16641612 | `07bc3368e07dd0c6a44df2666530c8587fc683f65c9b9873b77cb64895f3ba12` | 14446 | 14244 |
| `data/27/95/00/02.DAT` | 823938 | `aa022ebd8610ae8ba0dfa6b4c356b58c32c09945d8f4452d73157debcc26b804` | 195 | 195 |

Every one has VERS 4, two or four GTEX chunks, no DDSF chunk, and a final
zero-length TERM chunk, and its walk ends at EOF. No private conformance case
pins these files, so the table is not a retail parity claim.

## Container layout

Each chunk is a four-byte tag, a little-endian 32-bit payload length, and the
payload. The next chunk starts after the payload. The file begins with the
first chunk.

| Order | Tag | When read | Reader rule |
|---|---|---|---|
| 1 | `VERS` | When present | Four-byte payload; its little-endian value is the version. Absent, the version is zero. |
| 2 | `DDSF` | Repeats while present | Opaque payload, reported by span and digest |
| 3 | `GTEX` | Repeats while present | Opaque payload, reported by span and digest |
| 4 | `TEXC` | Version at least 4 | Four-byte payload |
| 5 | `FSIZ` | Always | Four-byte payload |
| 6 | `FSMX` | Version at least 3 | Four-byte payload |
| 7 | `BSLN` | Always | Four-byte payload |
| 8 | `HMGN` | Version at least 2 | Four-byte payload |
| 9 | `HTBL` | Always | 256 page bytes, one per high key byte |
| 10 | `LTBL` | Always | Whole 512-byte pages of little-endian 16-bit record indexes |
| 11 | `GLYP` | Always | Whole 14-byte records |

Version gates compare the version as an unsigned value, as the client does.
Chunks after GLYP are listed in file order and marked as unread by the
client. When fewer than eight bytes remain, or a following chunk's length
runs past EOF, the rest of the input is one opaque trailing span.

## Glyph records

The client's glyph lookup copies a record into a normalized output and selects
one of two copy layouts by whether the stored VERS value is zero, as
`xivl-decomp:docs/resource/bitmap-font-glyph-consumers.md` records in "Lookup
and normalized output". The reader applies the same choice. Field roles come
from that page's "Placement, extents, and advance", "Source rectangle and
normalization", and "Resource and selector indices" sections, and from
"BR-02" in `xivl-decomp:docs/resource/bitmap-font-runtime-bridge-consumers.md`.

| Record offset, nonzero version | Record offset, zero version | Width | Library field | Consumer evidence |
|---|---|---|---|---|
| `0x00` | `0x00` | signed 8 | `resource_index` | Index into the DDSF and GTEX resource list; a second access adds TEXC |
| `0x01` | `0x01` | signed 8 | `selector` | Index into four selector vectors, with no client bounds check |
| `0x02` | `0x02` | signed 8 | `offset_x` | Added, scaled, to the first drawing coordinate |
| `0x03` | `0x03` | signed 8 | `offset_y` | Added, scaled, to the second drawing coordinate |
| `0x04` | `0x04` | signed 8 | `width` | Source and drawing extent on the first axis |
| `0x05` | `0x05` | signed 8 | `height` | Source and drawing extent on the second axis |
| `0x06` | `0x06` | signed 8 | `advance` | Horizontal advance before caller-flag adjustments |
| `0x07` | `0x07` | 8 | `byte_07` | Copied; no consumer established |
| `0x08` | - | 8 | `flag_02_advance_addend` | Added to the advance modulo 256 when caller flag `0x02` is set |
| `0x09` | - | 8 | `byte_09` | Copied; no consumer established |
| `0x0A` | `0x08` | unsigned 16 | `source_x` | Source rectangle origin on the first axis, in texture pixels |
| `0x0C` | `0x0A` | unsigned 16 | `source_y` | Source rectangle origin on the second axis, in texture pixels |
| - | `0x0C..0x0E` | 16 | `unread` | Not read by lookup |

Multibyte fields are little-endian. With a zero version, lookup clears output
bytes `0x08` and `0x09`, so the library reports no addend or byte 9. The
caller-flag adjustments and the scaling formulas stay with the decomp pages.
`FdtFile::glyph(key)` applies the two-table lookup and returns the record a
key reaches.

Inspection reports the layout and counts records that fail one of these
checks. None of them is an error:

| Check | Counted when |
|---|---|
| `resourceOutsideTextures` | Byte 0 is negative or not below the texture chunk count |
| `offsetResourceOutsideTextures` | Byte 0 plus TEXC, or zero without TEXC, is outside the list |
| `selectorOutsideVectors` | Byte 1 is outside 0 through 3 |
| `negativeExtent` | Width or height is negative |
| `sourceOutsideTexture` | The source origin plus extent passes the selected GTEX width or height |
| `sourceUnchecked` | The selected texture has no GTEX header, or the check above cannot apply |

GTEX width and height are the big-endian words at `0x0A` and `0x0C` of a
payload that starts with `GTEX`, as the [GTEX contract](gtex-pwib.md)
records.

The census in `tools/research/census_fdt.py` reads the same seven retail files
with an independent decoder:

```text
python tools/research/census_fdt.py --client-root <install>
```

Every record in every file passes all five checks. Resource indexes are 0, or
0 and 1 where TEXC is 2. Selectors cover 0 through 3, except
`data/00/0C/00/04.DAT`, whose ten records use selector 0. Bytes `0x07` and
`0x08` each hold one nonzero value per file, and byte `0x09` is zero in every
record. Each texture that byte 0 plus TEXC selects has half the width and
height of the texture byte 0 selects. That pairing is a census observation,
not an established client rule.

## Reader rules

The client reports a tag mismatch and, if reporting returns, stores from the
mismatched chunk. It also reads a scalar dword whatever the payload length.
This reader refuses those inputs instead of guessing. It also requires that
every HTBL page other than `0xFF` has an LTBL page and that every LTBL index
other than `0xFFFF` has a GLYP record. Every retail file meets these rules.

| Condition | Error | Offset |
|---|---|---|
| Fewer than eight bytes for a required chunk header | `unexpected-end-of-input` | First missing field |
| A payload length runs past EOF | `chunk-length-out-of-range` | The length field |
| A required tag differs | `unexpected-chunk` | The chunk |
| A scalar payload is not four bytes, or HTBL is not 256 bytes | `invalid-chunk-length` | The length field |
| LTBL ends in a partial page, or GLYP in a partial record | `trailing-partial-record` | Start of the partial page or record |
| An HTBL page has no LTBL page, or an LTBL index has no record | `table-index-out-of-range` | The table entry |

## Texture payloads

The client appends DDSF and GTEX payloads to one resource list in file order;
`xivl-decomp:docs/resource/bitmap-font-container.md` records the handoff.
Inspection runs the [GTEX reader](gtex-pwib.md) over each GTEX chunk payload
and embeds its report under that texture with the payload's absolute offset.
Spans and offsets inside the embedded report are relative to the payload. A
payload the GTEX reader refuses reports its error kind and payload-relative
offset instead, and the container read still succeeds. DDSF payloads have
no reader and report null.

Running `xivl inspect --as fdt` on the seven retail files parses all 20 GTEX
payloads as bounded 2D textures with GTEX format index 22, two mip levels,
flags zero, two surface-table entries, no anomalies, and no trailing bytes.
Each report's width and height equal the header words the glyph checks read.
The GTEX reader maps index 22 to `D3DFMT_A4R4G4B4`, so each report marks
surface materialization as supported. The two payloads of
`data/00/0C/00/00.DAT`, exported as standalone GTEX files through
`extract-resource --export-dds --preview-png`, produced DDS files and PNG
previews with legible glyphs spread across the color channels, and
`verify-extraction` with source replay passed for both.

## Reports

`--as fdt` selects the reader; it has no automatic recognition.

```text
xivl inspect <input> --as fdt
xivl validate <input> --as fdt
```

Inspection reports every chunk's tag, span, payload span, payload digest, and
whether the client reads it. It reports the version and whether a VERS chunk
was present, the texture chunk count, mapped HTBL pages, LTBL pages, keys
that reach a record, GLYP record count, distinct records reached by a key,
the GTEX dimensions and embedded GTEX report of each texture chunk, the TEXC
offset, the glyph record layout, the record check counts, and the trailing span. It never reports
scalar metric values, table contents, per-record field values, or texture
bytes; the library model carries the decoded records. Validation checks
parsing and marks round-trip as not applicable.

Authored public fixtures cover version 4 with DDSF, two GTEX chunks, and a
TERM chunk; version 1; a file without VERS and its unversioned records;
following chunks and short trailing bytes; one record per glyph check; a
GTEX payload the GTEX reader accepts beside a truncated one; and each
refusal above. Library tests pin decoded fields in both layouts and key
lookup through both tables. The malformed-input sweep truncates and mutates
the version 4 fixture.

## Evidence limits

The DDSF path has no retail example. TEXC semantics, DDSF payloads, the
assignment of glyphs to atlas channels, the metric scalars, bytes `0x07` and
`0x09`, the meaning of caller flag `0x02`, the selector vectors' channel
mapping, the TERM chunk's purpose, and versions other than 4 in retail data
remain outside this claim. No retail file uses the zero-version layout. The
reader's refusals are stricter than the client's observed behavior, not a
description of it. Rasterizing glyphs and exporting atlas textures are
separate future work.
