# GTEX fields and PWIB segments

The retail 1.23b client loader establishes GTEX texture metadata and source
data addressing, plus the three boundaries of the PWIB split container. The
canonical promoted finding is:

- `xivl-decomp:docs/resource/gtex-pwib-loader.md` at commit
  `8ea63af8dbb6ae335d025116e7602006e58f745b`.

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
size dword. PNG conversion remains unsupported.

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

A complete install census of client build `2012.09.19.0001` found:

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
the artifact and source replay. PWIB remains metadata-only.

## Lossless DDS texture view

`extract-resource` and selected `extract-catalog` extraction accept
`--export-dds` to write one `payloads/texture.dds` file containing every
encoded mip. GTEX extraction remains metadata-only by default.
`--materialize-payloads` still writes separate raw surfaces and can be used
alongside DDS export. This is a texture view, not a GTEX round-trip writer:
GTEX headers, unknown fields, and gaps remain in the inspection report and
are excluded from the DDS file.

### Pixel-byte compatibility

The pinned loader finding above establishes the source pointer at
`0x00432500` as the blob's data base plus the table's per-surface offset.
The upload loop at `0x00431e20` passes that pointer through `0x00431080` to
a D3DX load-from-memory call. Creation and upload use the same client
D3DFORMAT table. The source-format assignments establish the pixel-byte
layout. Only GTEX header and table fields use big-endian decoding; pixel
bytes retain their D3DFORMAT layout.

Microsoft's [D3DFORMAT definition](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3dformat)
defines A8R8G8B8 memory order as blue, green, red, alpha. Its
[DDS programming guide](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide)
lists matching legacy masks and DXT FourCCs. Together with the loader's
source-format assignments, these establish the following byte-preserving
views:

| GTEX index | Source format | Legacy DDS pixel format |
|---:|---|---|
| 4 | `D3DFMT_A8R8G8B8` | 32-bit RGB with alpha; R `0x00ff0000`, G `0x0000ff00`, B `0x000000ff`, A `0xff000000` |
| 24 | `D3DFMT_DXT1` | FourCC `DXT1`, 8 bytes per 4 by 4 block |
| 26 | `D3DFMT_DXT5` | FourCC `DXT5`, 16 bytes per 4 by 4 block |

Export copies the source bytes without decoding, recompression, channel
swapping, or inferred color-space changes. Legacy DDS does not introduce a
color-space claim for these GTEX resources.

### Header and mip layout

The writer follows Microsoft's standard definitions:

- [DDS programming guide](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide).
- [DDS_HEADER](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-header).
- [DDS_PIXELFORMAT](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-pixelformat).
- [DDS texture layout](https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dds-file-layout-for-textures).

The file starts with `DDS ` and a little-endian 124-byte legacy header,
including a 32-byte pixel-format structure. Unused fields are zero. The
header records width, height, and the supplied mip count. Uncompressed
textures use row pitch `width * 4` and `DDSD_PITCH`; DXT textures use the
top-level encoded surface size and `DDSD_LINEARSIZE`. Every file sets the
required dimension, pixel-format, and texture-capability flags. Multiple
mips also set `DDSD_MIPMAPCOUNT`, `DDSCAPS_MIPMAP`, and `DDSCAPS_COMPLEX`.
No DX10 extension is emitted.

Surface bytes follow the header in increasing logical mip order, with no
GTEX gap bytes between them. Each manifest explains the mapping from a
source surface span to its DDS output span and digest. Source replay
regenerates and compares the entire DDS, including the header. Verification
without a source also checks the header, mip layout, digests, and agreement
between the DDS metadata and the parsed GTEX report.

### Supported boundary and evidence limits

DDS export requires a mapped, table-bearing 2D texture with flags zero,
depth one, nonzero dimensions, and a nonzero mip count. The count cannot
exceed `1 + floor(log2(max(width, height)))`: each successive level halves
the dimensions, clamping to one, and the chain ends at 1 by 1. A partial
chain is retained as supplied; export does not generate missing levels.
The mip geometry follows Microsoft's
[mipmap description](https://learn.microsoft.com/en-us/windows/win32/direct3d9/texture-filtering-with-mipmaps).
The DDS file boundary does not impose GPU-specific texture-size limits.

Cube, volume, nonzero flags, missing tables, unmapped indices, invalid
dimensions, and excess mip counts are refused before extraction output is
created. In a DDS batch request, every selected resource must be eligible.

Authored tests check the legacy header fields against standard offsets and
values, exact encoded-byte preservation for all three mappings, multiple
mips, and the documented layout with gaps between mip spans. CLI tests
cover extraction, accounting, refusals, verification, and source replay;
public conformance expectations contain only metadata, spans, counts, and
digests. GTEX read and export remain `partial`. No retail DDS parity or GPU
compatibility is claimed.
