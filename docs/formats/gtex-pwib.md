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
size dword. The decoded PNG preview boundary is described below.

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
The inherited SEDB prefix size fields remain metadata; they do not replace
the PWIB boundaries or impose a standalone first-span extent.

The selected RES/txb path below resolves one encoded texture surface in the
second segment. Other entries and remaining second-segment bytes retain
their spans and digests without an inferred texture or index-buffer meaning.

## Selected PWIB RES/txb texture

The selected consumer contract is promoted from
`xivl-decomp:docs/resource/gtex-pwib-loader.md`, section
`Selected RES/txb consumer`, at revision
`d05b0fdf6e2314fd0c62dd9d7845aa8204ab8889`. The finding identifies
the retail `ffxivgame.exe` by SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`
and records the exact handler, node, and GTEX upload joins.

`extract-resource` and `extract-catalog` accept
`--pwib-entry <zero-based-visible-index>`. Selection is explicit and applies
to each selected PWIB resource. It does not search entries for textures.
Omitting it preserves the ordinary bounded PWIB inspection report.

### Directory and type selection

The first span must contain the little-endian `SEDBRES ` path.
Its directory begins at first-relative `0x40` and has `N` entries of
16 bytes, where `N` is the little-endian dword at `+0x30`. The payload
base is `D = 0x40 + 16*N`. Each directory entry supplies a name index,
payload-relative byte offset, and byte length in its first three dwords.
The fourth dword retains its value without a semantic interpretation.
The RES scalar at `+0x08` is also retained: the promoted reader compares
it against 4000 but does not identify it as the selected txb version.
Names begin at `D + LE32(first+0x34)` and contain the number of consecutive
NUL-terminated strings declared at `+0x38`.

The literal metadata names `RESOURCE_TYPE` and `RESOURCE_ID` each reduce
the exposed entry count by one when present. The selected index must be
within that visible range and indexes the original physical directory.
Metadata entries are not removed to compact or renumber that directory.
`RESOURCE_TYPE` is required and its selected
little-endian dword must equal `0x00747862` (`txb`). Filename, directory
order adjacency, and an embedded GTEX tag do not establish this type.
Directory, name, entry, and metadata extents are checked against the
first span with checked arithmetic before an entry is selected.

### Descriptor and external surface views

The selected block must use the version-one, little-endian `SEDBtxb\0`
path. Let `B` be its first-span block pointer and `w` the little-endian
word at block `+0x0e`. The descriptor location follows the selected node
initializer at `0x00c78a00`:

```text
w <= 0x30: descriptor = B + 0x30 + w
w >  0x30: descriptor = B + LE32(B+0x30)
```

The GTEX fixed fields and eight-byte table entry must fit within the
selected block. Descriptor scalar and table dwords are big-endian.
Extraction accepts only format index 24 (`D3DFMT_DXT1`), one mip, 2D
flags zero, depth one, nonzero dimensions, a present table, and source-data
base zero. The encoded byte count must equal
`ceil(width/4) * ceil(height/4) * 8`.

The source-data base zero selects the separately supplied second view:

```text
table   = descriptor + BE32(descriptor+0x10)
surface = second.offset + BE32(table)
```

The descriptor and encoded surface remain distinct bounded views. The
table offset is descriptor-relative; the encoded surface offset is
second-relative. The offline reader checks the entire encoded range,
including its end, against the second span. The native upload path at
`0x00431ee5` checks only that the start is below the supplied length and
does not establish this whole-range guard.

Add `--materialize-payloads`, `--export-dds`, or `--preview-png` to obtain
the existing raw-surface, legacy DDS, or decoded PNG view. These options
may be combined. Encoded bytes retain their DXT1 layout and are copied
without decoding or recompression for raw and DDS output. PNG uses the
documented DXT1 preview conversion and allocation limit below. Unselected
entries, gaps, and remaining bytes stay accounted for as bounded spans
and digests; extraction does not assign them a new meaning.

The manifest retains the selected entry identity and type metadata,
descriptor location, separately bounded second view, second-relative
surface offset, absolute encoded source span, and digests. Verification
checks these relationships alongside exact output inventory. Source
replay repeats selection and decoding and compares complete output files.
Malformed or unsupported requests fail before extraction output is created.

### Retained check vector and limits

The retained source vector is `m520/equ/e001/top_tex1/0000`, entry 6:
103600 bytes with SHA-256
`23b3f4cbd2a25e3d43f4864bbf8b79f9b7d68322b96849bb044483c62286314d`.
Its descriptor is at file offset 4296 and selects file range
`[5296,38064)`: 256 by 256 DXT1, 32768 encoded bytes. This identity and
range are pinned in `tests/fixtures/private-manifest.json` as
`retail-pwib-m520-top-tex1`. A retail replay requires an explicit input root
and matching size and digest before this vector is used.

The private cases `pwib-retail-m520-entry6`,
`pwib-retail-m520-entry6-dds`, and `pwib-retail-m520-entry6-png` reproduce
selected-entry structure and artifact identities. Their expectations contain
metadata, spans, counts, and digests. Decoded entry names are replaced with
UTF-8 byte lengths and SHA-256 digests; encoded surfaces and decoded pixels
remain outside the repository.

Both extraction commands reproduced raw, DDS, and PNG views for this exact
input and passed complete source replay. Catalog extraction used a
size-and-digest-verified private DAT copy selected by path, retaining the
original client-relative source provenance. The copy's DAT filename does
not establish a retail resource ID. Raw bytes and the DDS payload at output
offset 128 matched the independent source range, whose SHA-256 is
`9e31e16c1ccda881110cbaf555e62ace19cb6d0c9ff7249e2a39441d6b3d0801`.
An independent legacy-header check validated the DDS fields against
Microsoft's definitions linked below.

The PNG is 256 by 256. An independent DXT1 oracle using the preview conversion
below compared every decoded RGBA sample, including both endpoint-order
modes. The decoded row-order RGBA SHA-256 is
`a5ff58f347b087d85ae870f8cf3d38504ba536f326a2dbc3e3e19c1bd57bea02`.
This establishes the documented preview conversion for this input, not
GPU-identical rounding, color space, or premultiplication.

PWIB read and export remain `partial`. Other versions, endian paths,
types, descriptor-relative source data, mip counts, pixel formats, cube
and volume textures, and nonzero flags are refused. The remaining entry
semantics and bytes are unresolved. Runtime activation, appearance
selection, and GPU parity remain unproved. This is a selected texture
view, not a PWIB or GTEX round-trip writer.

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
the artifact and source replay. PWIB texture views require the explicit
RES/txb entry selection described above.

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
digests. GTEX read and export remain `partial`. These synthetic GTEX cases
do not establish retail DDS parity or GPU compatibility.

## Decoded top-mip PNG preview

`extract-resource` and selected `extract-catalog` extraction accept
`--preview-png` to write `payloads/preview.png`. The preview contains mip 0
only. It can accompany separate raw surfaces and the DDS texture view.
Omitting the option preserves the ordinary extraction outputs.

A PNG preview is decoded image data. Lossless resource export claims apply
to the encoded raw surfaces and DDS mip bytes. The preview discards lower
mips and the encoded representation. GTEX read and export remain `partial`.

### Pixel conversion

The loader-backed format assignments in the DDS section above establish
the source pixel layouts. Preview decoding follows these primary definitions:

- Microsoft's [D3DFORMAT definition](https://learn.microsoft.com/en-us/windows/win32/direct3d9/d3dformat).
- Microsoft's [opaque and 1-bit alpha textures](https://learn.microsoft.com/en-us/windows/win32/direct3d9/opaque-and-1-bit-alpha-textures).
- Microsoft's [textures with alpha channels](https://learn.microsoft.com/en-us/windows/win32/direct3d9/textures-with-alpha-channels).
- The [PNG specification](https://www.w3.org/TR/png-3/).

Output samples are 8-bit red, green, blue, alpha in row order from the top
left. A8R8G8B8 source bytes are blue, green, red, alpha and are reordered
without changing sample values. Alpha is retained, including RGB samples
whose alpha is zero.

DXT blocks and their texels are read in row order. Endpoints and packed
selectors are little-endian. RGB565 endpoints contain red in bits 11-15,
green in bits 5-10, and blue in bits 0-4. The preview expands a 5-bit sample
`v` as `(v << 3) | (v >> 2)` and a 6-bit sample as
`(v << 2) | (v >> 4)`. These explicit integer conversion rules define the
preview's sample values without claiming GPU-identical rounding.

For DXT1, unsigned endpoint `c0 > c1` selects four opaque colors. Selectors
0 and 1 choose the expanded endpoints. Per channel, selectors 2 and 3 use
`floor((2*c0 + c1 + 1)/3)` and `floor((c0 + 2*c1 + 1)/3)`.
When `c0 <= c1`, selector 2 uses `floor((c0 + c1)/2)` and selector 3
produces RGBA `(0, 0, 0, 0)`. All other DXT1 colors have alpha 255.

DXT5 always uses the four-color rule, regardless of endpoint ordering.
Its two 8-bit alpha endpoints precede a 48-bit field of sixteen 3-bit
selectors. Alpha selectors 0 and 1 choose endpoints `a0` and `a1`.
For `a0 > a1`, selectors `i = 2..7` use
`floor(((8-i)*a0 + (i-1)*a1 + 3)/7)`. Otherwise selectors `i = 2..5`
use `floor(((6-i)*a0 + (i-1)*a1 + 2)/5)`, selector 6 is zero, and
selector 7 is 255. DXT5 RGB samples remain unchanged when alpha is zero.

Each compressed surface supplies `ceil(width/4) * ceil(height/4)` complete
blocks. Texels beyond the logical width or height are cropped, allowing
partial blocks at image edges and images smaller than 4 by 4.

PNG stores unassociated alpha. Preview generation performs no alpha
multiplication or division and makes no inference about client
premultiplication or color space. No sRGB, gamma, chromaticity, or color
profile chunks are emitted. A viewer may apply its own display defaults.

### Bounds and verification

The preview accepts the same table-bearing 2D GTEX subset as DDS: indices
4, 24, and 26, flags zero, depth one, nonzero dimensions and mips, and a mip
count within the dimension-derived chain. The parser validates all surface
spans before preview generation. Only the validated top-mip span is decoded.
Preview generation checks dimension, block, row, and allocation arithmetic
and limits the RGBA buffer to 64 MiB (67108864 bytes). Unsupported or
oversized requests fail before extraction output is created. Every resource
selected in a preview batch must satisfy this boundary.

The preview is an 8-bit RGBA, noninterlaced PNG. Its bytes are included in
manifest output accounting. The manifest records the source format, mip,
encoded span and digest, dimensions, and decoded RGBA digest. Verification
checks file inventory, PNG structure, dimensions, decoded samples, and
agreement with the GTEX table metadata. Source replay decodes the validated
span again and compares the complete generated PNG exactly.

Authored pixel oracles cover channel order, both DXT1 modes, DXT5 color and
alpha interpolation, transparency, rounding, and partial-block edges.
Synthetic extraction tests cover resource and catalog requests, coexistence
with raw and DDS artifacts, accounting, refusals, and missing or altered
previews. These synthetic GTEX cases do not establish retail preview parity
or GPU compatibility. The selected PWIB replay above covers its pinned
retail vector only.
