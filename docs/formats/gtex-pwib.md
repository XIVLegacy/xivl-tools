# GTEX fields and PWIB segments

The retail 1.23b client loader establishes GTEX texture metadata and source
data addressing, plus the three boundaries of the PWIB split container. The
canonical promoted finding is:

- `xivl-decomp:docs/resource/gtex-pwib-loader.md` at commit
  `86d4e7f25653a23f474643fcd78cb56f3c75738a`.

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
selects eight-byte entries for every face and mip. Each entry contains a
big-endian source offset relative to the data base followed by its big-endian
encoded byte size. Header gaps retain spans and digests; data gaps between
surface spans are reported independently.

The observed mappings are index 4 -> `D3DFMT_A8R8G8B8` (numeric 21, 32 bits
per pixel), index 24 -> `D3DFMT_DXT1` (`0x31545844`, 8 bytes per 4 by 4
block), and index 26 -> `D3DFMT_DXT5` (`0x35545844`, 16 bytes per block).
Linear size is `width * height * bitsPerPixel / 8`; block size is
`ceil(width / 4) * ceil(height / 4) * blockBytes`. Each mip clamps width and
height to one. The parser requires declared and calculated sizes to agree for
these mappings, rejects overlapping or out-of-file spans, and permits gaps.

Exact encoded-surface materialization is limited to mapped, table-bearing 2D
textures with flags zero and depth one. Cube, volume, nonzero flags, missing
tables, and unmapped indices remain inspectable but are explicitly unsupported
for materialization. Bit 2 still has no stable semantic name. Offset `0x1c`
is not a fixed header field: with the retail table base of 24 it is entry 0's
size dword. DDS/PNG conversion remains unsupported.

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

On 2026-09-04, a complete install census of client build
`2012.09.19.0001` found:

- 21,161 GTEX files, with no zero or out-of-file data bases. Observed data
  bases were 32, 48, 64, and 96.
- All 41,217 GTEX surface size dwords match the client sizing formula. Every
  surface is non-overlapping and the last ends at EOF. Two adjacent mip pairs
  contain explicit eight-byte gaps; all other adjacent deltas equal size.
- 3,544 PWIB files, with no unordered boundaries, total-size mismatch, or
  missing `SEDB` signature at the first offset. Every observed first offset
  was 16.

The private representatives are recorded in
`tests/fixtures/private-manifest.json`:

| Fixture | Client path | Size | SHA-256 |
|---|---|---:|---|
| `retail-gtex-61c10005` | `data/61/C1/00/05.DAT` | 40 | `6663eafa5248c68d9804c4f4ca0677d4f24434f5c014b53889c70b2ccba204ef` |
| `retail-gtex-a8r8g8b8-1c59027e` | `data/1C/59/02/7E.DAT` | 544 | `2018e48d77ab8529f764682f71d2f8364eb20b6f39e0d620a4978b5e1e9b6d6d` |
| `retail-gtex-dxt5-1c590028` | `data/1C/59/00/28.DAT` | 5,504 | `edff5bb2ba4b5b66ea2ff7f50473be1d23fd02dbf945ef34c4cc5305c80a4f02` |
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
shape against the four retail resources without retaining recoverable bytes.

With `--materialize-payloads`, supported GTEX inputs produce one deterministic
`gtex-encoded-surface` artifact per table entry. Each manifest records the
face, mip, format mapping, source span, and digest; verification checks both
the artifact and source replay. PWIB remains metadata-only. DDS/PNG conversion
is not supported.
