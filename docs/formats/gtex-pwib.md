# GTEX and PWIB bounded extents

The 1.23b evidence establishes two exact four-byte resource tags, a fixed
outer-header size for each family, and one bounded extent after each header.
It does not establish texture metadata or the meaning of PWIB.

The source that identifies these as file-type-specific resources rather than
PackRead chunks is:

- `xivl-decomp:docs/resource/sqpack.md`, sha256
  `7e1ece3fe37f78582b82e7fce4c017bde6cd79d1f63affedda6a293dec32932d`

## Retail extent census

On 2026-09-04, an anchored signature scan of client build
`2012.09.19.0001` found 21,161 GTEX files and 3,544 PWIB files. Header-only
reads produced these relationships:

| Family | Header | Extent declaration | Equals remaining file | Shorter than remaining file | Escapes file |
|---|---:|---|---:|---:|---:|
| GTEX | 32 | big-endian u32 at `0x1c` | 6,974 | 14,187 | 0 |
| PWIB | 16 | nested SEDB at `0x10`; its little-endian declared size is at outer offset `0x20` | 3,182 | 362 | 0 |

Every PWIB file carried the exact `SEDB` signature at offset `0x10`. The
ordinary SEDB reader owns the nested container's header and entry rules.

The counts rule out an end-of-file interpretation for either declaration.
The reader therefore exposes the declared extent and preserves all later
bytes as a separate trailing span. It does not call the GTEX extent a texture
payload, interpret bytes `0x04..0x1b`, or assign a purpose to PWIB. A loader
field-access citation is still required before those semantic claims can be
made.

The smallest representatives are private fixtures recorded in
`tests/fixtures/private-manifest.json`:

| Fixture | Client path | Size | SHA-256 |
|---|---|---:|---|
| `retail-gtex-61c10005` | `data/61/C1/00/05.DAT` | 40 | `6663eafa5248c68d9804c4f4ca0677d4f24434f5c014b53889c70b2ccba204ef` |
| `retail-pwib-89b0005a` | `data/89/B0/00/5A.DAT` | 536 | `42cd46946d39812f32d17fb683e0ab45c53bdff9948610feb5228e93f856f99a` |

The GTEX representative declares an 8-byte extent after its 32-byte header.
The PWIB representative contains one 520-byte `txb\x00` SEDB container after
its 16-byte header. Both end at the file boundary, while the census above
guards the required trailing-span path.

## Reader contract

Automatic inspection recognizes either exact tag. Explicit `--as gtex` and
`--as pwib` readings require the matching tag. A successful report contains:

- the exact signature and outer-header span;
- the unresolved outer-header bytes as a SHA-256-pinned unknown span;
- the declared extent's span and SHA-256;
- the parsed child SEDB model for PWIB;
- every byte after the declared extent as a separate trailing span; and
- `layoutStatus: bounded` with no invented semantic fields.

A GTEX input shorter than 32 bytes and a PWIB input shorter than 16 bytes fail
as `unexpected-end-of-input`. A GTEX extent that escapes the input fails as
`declared-size-out-of-range`. A malformed PWIB child reports the ordinary SEDB
failure at its absolute input offset.

`validate`, catalog inspection, metadata extraction, and source-replay
verification use this bounded model. Exact payload materialization remains
disabled: the GTEX extent is not yet semantically identified, and PWIB's
outer-resource purpose remains unresolved.

## Coverage and remaining evidence

Authored public fixtures exercise both bounded layouts without using retail
bytes. They cover a GTEX big-endian extent, a PWIB-wrapped synthetic SEDB
container, tag truncation, validation, catalog extraction, and source replay.
The two private cases verify retail parity without recording recoverable bytes.

No decoded image or generated retail representation is stored here. Dimensions,
mip and layer counts, versions, pixel or storage formats, compression,
swizzling, GPU compatibility, and DDS or PNG conversion remain unsupported.
