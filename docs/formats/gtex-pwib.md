# GTEX fields and PWIB segments

The retail 1.23b client loader establishes GTEX texture metadata and source
data addressing, plus the three boundaries of the PWIB split container. The
canonical promoted finding is:

- `xivl-decomp:docs/resource/gtex-pwib-loader.md` at commit
  `80ee4c29dc7847bd558404572cd516a11cbb221d`.

The source that identifies both tags as file-type resources rather than
PackRead chunks remains:

- `xivl-decomp:docs/resource/sqpack.md`, SHA-256
  `7e1ece3fe37f78582b82e7fce4c017bde6cd79d1f63affedda6a293dec32932d`.

## GTEX contract

GTEX multibyte fields are big-endian. The reader reports:

| Offset | Width | Meaning |
|---|---:|---|
| `0x06` | 1 | Client format-table index |
| `0x07` | 1 | Mip level count |
| `0x09` | 1 | Texture flags |
| `0x0a` | 2 | Width |
| `0x0c` | 2 | Height |
| `0x0e` | 2 | Depth |
| `0x10` | 4 | Optional surface-offset table base |
| `0x14` | 4 | Source-data base |

Flag bit 0 selects cube, otherwise bit 1 selects volume, otherwise the texture
is 2D. Bit 2 is reported without a semantic name. A nonzero offset-table base
selects eight-byte entries for every face and mip. The first big-endian dword
of each entry is a source offset relative to the data base; the second dword
remains unknown. Header gaps retain spans and digests. The data region extends
from the loader-backed data base to end of input.

The reader does not interpret the client format-table index, assign a meaning
to flag bit 2 or header offset `0x1c`, calculate encoded surface lengths, or
decode, convert, or materialize texture bytes.

## PWIB contract

PWIB boundary fields are big-endian:

| Offset | Width | Meaning |
|---|---:|---|
| `0x04` | 4 | Total size and second-segment end |
| `0x08` | 4 | First-segment offset |
| `0x0c` | 4 | Second-segment offset |

The parser requires `16 <= first <= second <= total <= input length`. It
reports `[first, second)` as an `SEDB`-prefixed first segment and
`[second, total)` as an opaque continuation. Only the fixed SEDB header fields
in the first segment are reported. The ordinary SEDB parser is deliberately
not used: retail PWIB files split the logical resource across both segments,
so the first span is not a standalone bounded SEDB container. Bytes after the
PWIB total size, if present, remain a separate trailing span.

The purpose and internal structure of the second segment remain unresolved.
No texture or index-buffer interpretation is claimed.

## Retail parity

On 2026-09-04, a header-only census of client build `2012.09.19.0001` found:

- 21,161 GTEX files, with no zero or out-of-file data bases. Observed data
  bases were 32, 48, 64, and 96.
- 3,544 PWIB files, with no unordered boundaries, total-size mismatch, or
  missing `SEDB` signature at the first offset. Every observed first offset
  was 16.

The private representatives are recorded in
`tests/fixtures/private-manifest.json`:

| Fixture | Client path | Size | SHA-256 |
|---|---|---:|---|
| `retail-gtex-61c10005` | `data/61/C1/00/05.DAT` | 40 | `6663eafa5248c68d9804c4f4ca0677d4f24434f5c014b53889c70b2ccba204ef` |
| `retail-pwib-89b0005a` | `data/89/B0/00/5A.DAT` | 536 | `42cd46946d39812f32d17fb683e0ab45c53bdff9948610feb5228e93f856f99a` |

The GTEX representative is a 4 by 4 2D texture with one mip, data base 32,
one surface-offset entry, and eight source-data bytes. The PWIB representative
has a 144-byte first segment and a 376-byte second segment. Its SEDB prefix
declares 520 bytes, demonstrating why the 144-byte first segment cannot be
parsed as an independent SEDB container.

## Coverage

Automatic inspection recognizes either exact tag. Explicit `--as gtex` and
`--as pwib` require the matching tag. Public authored fixtures exercise the
fields, both PWIB segments, preserved trailing bytes, tag and header
truncation, invalid GTEX data bases, invalid PWIB boundaries, validation,
catalog extraction, and source replay. Private cases verify the same report
shape against the two retail resources without retaining recoverable bytes.

Metadata extraction remains non-materializing. Its manifest preserves the
parsed fields, spans, and SHA-256 digests, and source replay compares that
report. DDS/PNG conversion and exact payload materialization remain
unsupported.
