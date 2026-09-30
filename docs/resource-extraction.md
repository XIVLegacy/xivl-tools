# DAT catalog and resource extraction

This guide covers the repeatable workflow for working with a game or resource
directory:

1. Build a catalog of its `.DAT` files.
2. Select the resources you want by id or relative path.
3. Extract structural manifests and, when needed, exact supported payloads.
4. Verify the result later, with or without the original source files.

The commands use only paths you provide. They do not search for an install,
change source files, assign meanings to unknown bytes, or silently extract
everything.

## Quick workflow

Create a catalog first:

```powershell
cargo run --locked -p xivl-cli -- catalog "C:\path\to\FINAL FANTASY XIV" --output catalog
```

Then select resources explicitly and extract them:

```powershell
cargo run --locked -p xivl-cli -- extract-catalog catalog/catalog.json `
  --root "C:\path\to\FINAL FANTASY XIV" `
  --output selected `
  --id 0x12345678 `
  --path data/12/34/56/79.DAT
```

Finally verify the output. The `--catalog` and `--root` options ask the
verifier to replay every selected source and compare it with the catalog:

```powershell
cargo run --locked -p xivl-cli -- verify-extraction selected `
  --catalog catalog/catalog.json `
  --root "C:\path\to\FINAL FANTASY XIV" `
  --report json
```

The catalog is a single `catalog.json` or `catalog.jsonl` file. A selected
output contains one directory per resource and an `extraction.yaml` or
`extraction.json` manifest in each directory. Decoded documents and other
materialized payloads live in separate files under that resource directory
and are referenced by the manifest.

## Catalog files

`xivl catalog <directory> --output <directory>` walks `.DAT` files below the
named directory. When a game root is supplied, the command uses its `data`
tree; a resource directory can be supplied directly. The output directory must
be absent or empty, and symbolic links are not followed.

The default `catalog.json` conforms to `schemas/resource-catalog.schema.json`.
Use `--format jsonl` for one compact resource object per line in
`catalog.jsonl`. Both forms contain the same information.

Each resource row records:

- the root-relative path and file size;
- a SHA-256 digest;
- a resource id only when the path establishes one through the
  `AA/BB/CC/DD.DAT` convention;
- a detected format, parse status, support status, spans, and anomalies.

Detection is limited to formats with an established signature. The
`formatStatus` field makes the result explicit:

| Status | Meaning |
|---|---|
| `parsed` | A known reader accepted the complete input. |
| `malformed` | A known signature was present, but the reader returned a typed error. The error kind, offset, and detail are retained. |
| `unknown` | No supported signature was established, so no reader was guessed. |

`supportStatus` is the matching read level from the [support matrix](support-matrix.md).
The catalog is an inventory, not a promise that every row can be exported.

## Extract one resource

Use `extract-resource` when you already know the input file:

```powershell
cargo run --locked -p xivl-cli -- extract-resource resource.DAT --output resource
cargo run --locked -p xivl-cli -- extract-resource resource.DAT --output resource --format json
```

The default `extraction.yaml` (or JSON with `--format json`) records source
identity, tool version, format and parse status, the inspection report,
anomalies, and references to separate payload files. Large or opaque payloads
are never embedded as base64.

SQEX widget and scrambled-XML extraction writes the exact decoded document to
`payloads/decoded.xml` automatically. The existing decoders supply the bytes;
the writer preserves BOMs, whitespace, comments, and markup without
reserializing XML. The manifest records the payload's relative path, role,
size, and SHA-256, without embedding decoded text.

SQEX uses the input basename, including its case and suffix, as its key.
Extract the original named file and retain that name for source replay:

```powershell
cargo run --locked -p xivl-cli -- extract-resource Widget.form --output widget --as sqwt
cargo run --locked -p xivl-cli -- verify-extraction widget --source Widget.form
```

The manifest records `source.fileName` and the parser's key name. This
workflow does not assign DAT resource ids to named widget files.

LPB extraction writes the decoded Lua 5.1 chunk to
`payloads/decoded.luac`. The manifest records only its relative path, role,
size, and digest. For SEDB and RES files, add `--materialize-payloads` to
write exact direct-root payload entries. GTEX materialization is limited to
the supported table-bearing 2D boundary; PWIB and unsupported GTEX variants
remain metadata-only.

For an eligible GTEX texture, add `--export-dds` to write all encoded mip
levels in one `payloads/texture.dds` file:

```powershell
cargo run --locked -p xivl-cli -- extract-resource texture.DAT --output texture --export-dds
```

DDS export is opt-in and may be combined with `--materialize-payloads` to
retain the separate raw surfaces. The manifest records texture format and
dimensions plus each mip's source span, DDS output span, and digest. DDS
bytes count toward extraction output totals. See the
[GTEX DDS contract](formats/gtex-pwib.md#lossless-dds-texture-view) for the
exact supported boundary and pixel-byte evidence.

Add `--preview-png` for a decoded image of the top mip:

```powershell
cargo run --locked -p xivl-cli -- extract-resource texture.DAT --output texture --preview-png
```

The deterministic `payloads/preview.png` artifact is included in output
accounting and may accompany raw surfaces and DDS. Its manifest records the
encoded top-mip span and digest, source format, dimensions, and decoded RGBA
digest. Preview requests use the eligible GTEX boundary and a 64 MiB RGBA
allocation limit. See the
[PNG preview contract](formats/gtex-pwib.md#decoded-top-mip-png-preview) for
channel, interpolation, alpha, and partial-block rules.

Container payloads use deterministic names containing the entry ordinal, role, source
offset and length, and a SHA-256 prefix. The manifest keeps the full digest and
the source span. Nested SEDB data stays inside its one direct parent payload;
it is linked from the manifest rather than written a second time. Empty spans
are preserved as empty files.

Before creating output, the command rejects overlapping or aliased entries,
clamped or out-of-range spans, spans past the resolved container end, and
malformed nested SEDB signatures. It does not guess ownership, trim, merge,
decompress, or assign a semantic format to opaque bytes.

## Extract selected catalog entries

`extract-catalog` reads either catalog form and requires one or more explicit
`--id` or `--path` selections. There is no extract-all mode. Paths are relative
to the `--root` directory and must be normalized, traversal-free, and free of
drive prefixes or alternate-stream syntax.

Before it writes anything, the command checks:

- the catalog schema, identity, and duplicate rows;
- each selected source's size and SHA-256;
- that the current format detection still matches the catalog;
- the selected resource's extraction plan and output size;
- every selected path for symbolic links or Windows reparse points.

The defaults are deliberately conservative:

| Limit | Default | Option |
|---|---:|---|
| Selected resources | 32 | `--max-resources` |
| Aggregate source bytes | 64 MiB (67108864) | `--max-source-bytes` |
| Aggregate output bytes | 128 MiB (134217728) | `--max-output-bytes` |

All limits must be positive integers. A limit or accounting overflow fails the
whole plan before a successful batch is published. Each resource receives an
isolated directory named from its selection order, resource id when present,
and a source-digest prefix. The top-level batch manifest records catalog
identity, limits, totals, catalog indexes, source identities, detected
formats, and relative resource-manifest paths.

The batch is prepared in a same-parent staging directory and published by
rename only after every resource and the batch manifest succeed. A failed or
refused batch does not appear at the requested output path.

Selected scrambled-XML DAT entries produce the same exact decoded document
as `extract-resource`, included in aggregate output accounting. SQEX widgets
use the named-file workflow above; cataloging walks DAT files.

To export selected GTEX textures as DDS, add `--export-dds`:

```powershell
cargo run --locked -p xivl-cli -- extract-catalog catalog/catalog.json `
  --root "C:\path\to\FINAL FANTASY XIV" `
  --output textures --id 0x12345678 --export-dds
```

Every selected resource in a DDS request must be eligible. An unsupported
selection or an output limit failure refuses the complete batch before
publication. Omitting the option preserves ordinary extraction behavior.

`--preview-png` is also available on `extract-catalog`. Every selection
must satisfy the preview boundary. For both texture options, planning
includes artifact bytes in the output limit before the batch is published.

## Verify extraction output

`xivl verify-extraction <directory>` auto-detects exactly one root manifest:
`extraction.yaml`, `extraction.json`, `batch.yaml`, or `batch.json`. It loads
the embedded schema and then checks relationships the schema alone cannot
express.

For a single-resource extraction it verifies every declared payload's path,
regular-file identity, size, SHA-256, source-span arithmetic, and parsed
container relationship. Exact directory membership is required, so missing or
unlisted files fail. Add `--source <file>` to check source name, resource id,
size, digest, parsed structure, materialization plan, source slices, and
decoded outputs against the original file. For SQEX and scrambled XML, replay
runs the decoder and compares the output bytes exactly. SQEX replay uses the
recorded basename and requires the supplied source to retain that name.

For DDS artifacts, verification also checks the legacy header, every mip's
layout and digest, and metadata relationships to the parsed GTEX table.
Source replay regenerates and compares the complete DDS file, including its
header and exact encoded mip bytes.

For PNG previews, verification checks PNG structure and decoded RGBA
identity, dimensions, the top-mip metadata, and the GTEX source-table
relationships. Source replay regenerates and compares the complete preview.

For a batch, it performs the same checks for every isolated resource and also
checks the top-level records, ordinals, catalog indexes, paths, formats, byte
counts, and aggregate totals. Pass `--catalog <file> --root <directory>` as a
pair to verify the catalog identity and replay every selected source.

The verifier is read-only. It refuses path traversal, alternate-stream syntax,
case-folded collisions, symbolic links, Windows reparse points, hardlink
aliases, non-regular members, digest or size changes, and arithmetic overflow.
Stable failure prefixes include `schema-validation-failed`,
`payload-sha256-mismatch`, `extra-file`, `file-alias-refused`,
`stale-source-sha256`, and `batch-totals-mismatch`. It never repairs, creates,
removes, or rewrites extraction content.

## Terms used in errors

- A **source span** is an offset and length identifying bytes in the input.
- An **ambiguous payload span** is a declaration that overlaps, aliases, or
  runs outside another declared boundary. The tool stops rather than deciding
  which declaration is correct.
- A **symbolic link** or Windows **reparse point** is a filesystem entry that
  can redirect a path elsewhere. Cataloging, extraction, and verification
  refuse them so an explicit root remains an actual boundary.
- A **hardlink alias** is a second name for the same file contents. Verification
  refuses it when it would make the output inventory differ from the manifest.

For the format-specific reports and evidence behind these boundaries, see the
[CLI guide](cli.md), [format evidence](format-evidence.md), and
[support matrix](support-matrix.md).
