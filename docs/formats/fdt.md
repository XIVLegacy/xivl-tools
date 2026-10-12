# Bitmap font container (fdt)

[Documentation index](../README.md) | [Format evidence](../format-evidence.md)

The `fdt` reader walks a bitmap font container in the chunk order the retail
client reads. It reports chunk spans, digests, version gates, and lookup-table
counts, and it supplies structural inspection and validation. Read support is
`partial`. Glyph metrics, texture payloads, rasterization, writing, and
export are outside this contract.

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
and the trailing span. It never reports scalar metric values, table contents,
glyph record fields, or texture bytes. Validation checks parsing and marks
round-trip as not applicable.

Authored public fixtures cover version 4 with DDSF, two GTEX chunks, and a
TERM chunk; version 1; a file without VERS; following chunks and short
trailing bytes; and each refusal above. The malformed-input sweep truncates
and mutates the version 4 fixture.

## Evidence limits

The DDSF path has no retail example. The client's TEXC, GTEX, and DDSF
payload semantics, the metric scalars, the GLYP record fields, the TERM
chunk's purpose, and versions other than 4 in retail data remain outside
this claim. The reader's refusals are stricter than the client's observed
behavior, not a description of it. Rasterizing glyphs and exporting atlas
textures are separate future work.
