# GTEX and PWIB tagged resources

The 1.23b evidence currently establishes two exact four-byte resource tags:
`GTEX` and `PWIB`. It does not establish a post-signature header layout for
either family.

The source that identifies these as file-type-specific resources rather than
PackRead chunks is:

- `xivl-decomp:docs/resource/sqpack.md`, sha256
  `7e1ece3fe37f78582b82e7fce4c017bde6cd79d1f63affedda6a293dec32932d`

The install sample recorded in [SEDB and RES evidence](sedb-res.md) also found
both tags while separating non-SEDB inputs. Neither source identifies fields
after byte 3. In particular, the evidence does not establish dimensions, mip
or layer counts, versions, pixel or storage format identifiers, payload
offsets, record tables, compression, swizzling, or GPU compatibility. The
purpose of PWIB remains unresolved; this repository does not call it a texture
or an index buffer.

## Reader contract

Automatic inspection recognizes either exact tag. Explicit `--as gtex` and
`--as pwib` readings require the matching tag and report a truncated tag as
`unexpected-end-of-input` at offset 0. A different complete tag is
`bad-magic` at offset 0.

A successful report contains:

- the exact tag and its `0..4` span;
- every byte from offset 4 to end of input as one `opaqueRemainder` span;
- the SHA-256 of that opaque span; and
- `layoutStatus: unresolved` and an empty anomaly list.

This shape is intentionally non-interpretive. A four-byte input containing
only the tag is accepted with an empty opaque remainder because no evidenced
minimum header size exists. Values that resemble dimensions, counts, offsets,
or format identifiers are not read or validated.

`validate` checks the bounded recognition and reports byte round-trip as not
applicable. Catalogs classify a recognized input as parsed with partial read
support. `extract-resource` and catalog-driven extraction write the same
metadata report with no payload files. `verify-extraction` can replay the
source and require exact report parity. Direct `extract-resource`
`--materialize-payloads` remains limited to SEDB and RES and rejects GTEX or
PWIB because no encoded-payload boundary is established. A catalog extraction
with that flag still leaves signature-only resources metadata-only.

## Public coverage and remaining evidence

The public fixtures are authored tag-plus-pattern bytes. Positive GTEX and
PWIB cases verify exact recognition, remainder spans, and digests; explicit
truncation cases verify bounded failures. A GTEX validation case and an
end-to-end catalog, batch extraction, and source-replay test cover the shared
dispatch.

No retail texture bytes, decoded image, or generated retail representation is
stored in this repository. Advancing beyond recognition requires a
reproducible 1.23b source that independently establishes field accesses and
payload ranges. Until then, exact texture payload export, DDS/PNG conversion,
and every texture metadata field remain unsupported.
