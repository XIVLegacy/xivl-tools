# Command-line interface

The `xivl` command reads a file or directory that you name and writes a
structural report or an explicit export. It never searches for a client
installation, infers a path from the workspace, or guesses the meaning of an
unknown resource. The support target is Final Fantasy XIV 1.23b.

## Build and run

The CLI is the `xivl-cli` Cargo package. From the repository root:

```powershell
cargo run --locked -p xivl-cli -- --help
cargo run --release --locked -p xivl-cli -- --version
```

Use a built binary in place of `cargo run ... --` when a script or another
program needs to invoke it. All examples below use Cargo so they work from a
fresh checkout.

## Inspect one file

Use `inspect` when you want to know what a file contains without exporting its
payload:

```powershell
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sedb/plain-container.bin
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/config/lng-words.bin --as config-lng
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sheet/rows-typed.bin --as sheet-data --columns str,s32,bool,float,u8
```

Successful `inspect` output is canonical JSON on standard output. Reports
include the detected format, input size, spans, counts, anomalies, and
digests where those fields apply. They do not include payload bytes, sheet-row
text, or configuration values.

`validate` reads the same way and reports whether the parser's checks passed:

```powershell
cargo run --locked -p xivl-cli -- validate tests/fixtures/public/config/lng-words.bin --as config-lng
```

For the configuration readers, validation also encodes the parsed model and
checks that it reproduces the input bytes. Other readers report that a
round-trip check is not applicable because they do not have a writer.

### Select a reader

Signature-bearing files are detected automatically. Use `--as` when a file has
no identifying signature or when you want a particular view.

| Selector | Reads |
|---|---|
| `sedb` | A SEDB resource container. See [SEDB and RES](formats/sedb-res.md). |
| `ssd` | An SSD sheet document, including a scrambled document after decoding. See [SSD sheets](formats/ssd-sheet.md). |
| `scrambled-xml` | The container and document shape without exposing document content. |
| `sqwt` | A SQEX widget container. The input file name is part of its key; renaming it can make it unreadable. See [SQEX](formats/sqex.md). |
| `lpb` | An LPB wrapper around compiled Lua 5.1 bytecode. |
| `lpb-bytecode` | The LPB wrapper plus bounded Lua 5.1 structure. This is not decompilation or execution. See [Lua and LPB](formats/lua-lpb.md). |
| `staticactor-san` | The static-actor SAN record framing without assigning meanings to its record members. See [SAN records](formats/staticactor-san.md). |
| `gtex` | Loader-backed GTEX fields and surface spans. See [GTEX and PWIB](formats/gtex-pwib.md). |
| `pwib` | The two loader-bounded PWIB segments and the fixed SEDB header in the first segment. |
| `enable-file` | A headerless enable-record array. |
| `row-offsets` | A headerless row-offset array. |
| `sheet-data` | Sheet rows. Add `--columns` for typed rows; without it, values are read as strings. |
| `config-sys`, `config-pad`, `config-lng`, `config-rgn` | One of the client's signatureless configuration-file shapes. See [configuration files](formats/configuration.md). |

`--columns` applies only to `sheet-data`. It is a comma-separated list such as
`str,s32,bool,float,u8`.

## Catalog a DAT tree

Use `catalog` to inventory every `.DAT` file below an explicit game or
resource directory:

```powershell
cargo run --locked -p xivl-cli -- catalog "C:\path\to\FINAL FANTASY XIV" --output catalog
```

The command writes `catalog/catalog.json` by default, or
`catalog/catalog.jsonl` with `--format jsonl`. Each row records the relative
path, size, SHA-256, detected format, support status, spans, and anomalies.
Known resources can be `parsed` or `malformed`; resources without an
established signature remain `unknown` instead of being guessed. The input
tree is read-only, symbolic links are not followed, and the output directory
must be absent or empty.

The full catalog and extraction contract is in the [DAT workflow guide](resource-extraction.md).

## Extract selected resources

For one file, use `extract-resource`:

```powershell
cargo run --locked -p xivl-cli -- extract-resource resource.DAT --output resource
cargo run --locked -p xivl-cli -- extract-resource resource.DAT --output resource --materialize-payloads
cargo run --locked -p xivl-cli -- extract-resource Widget.form --output widget --as sqwt
cargo run --locked -p xivl-cli -- extract-resource scrambled.DAT --output document --as scrambled-xml
```

The default output is a schema-versioned `extraction.yaml` (use
`--format json` for JSON). It contains the inspection report and references to
separate payload files. The optional `--materialize-payloads` flag writes
exact direct payload spans only where the format contract allows it, currently
SEDB, RES, the supported GTEX boundary, and an explicitly selected PWIB
RES/txb texture. It refuses ambiguous or unsupported
payload layouts rather than choosing an owner for the bytes.

SQEX widget and scrambled-XML inputs automatically produce
`payloads/decoded.xml`, preserving the exact decoder output, including BOMs
and document formatting. Keep the original SQEX basename, case, and suffix:
they determine the decode key and are recorded for verification. See the
[extraction contract](resource-extraction.md#extract-one-resource).

Use `--export-dds` for an eligible GTEX texture to write all its encoded
mips to `payloads/texture.dds`. The option is available on `extract-resource`
and `extract-catalog`, and can be combined with `--materialize-payloads` for
separate raw surfaces. It requires table-bearing 2D GTEX with flags zero,
depth one, mapped A8R8G8B8/DXT1/DXT5 pixels, and valid dimensions and mip
count. See the [DDS contract](formats/gtex-pwib.md#lossless-dds-texture-view).

Add `--preview-png` to either extraction command for a decoded mip-0 image
at `payloads/preview.png`. It accepts the same GTEX subset, with a 64 MiB
decoded RGBA limit, and can accompany raw surfaces and DDS. PNG is a
preview rather than a lossless resource view. See the
[pixel conversion contract](formats/gtex-pwib.md#decoded-top-mip-png-preview).

For a PWIB texture, supply `--pwib-entry <zero-based-visible-index>` to
either extraction command. Add the same raw, DDS, and PNG options for the
selected entry. For example:

```powershell
cargo run --locked -p xivl-cli -- extract-resource bank.DAT --output texture --pwib-entry 6 --export-dds --preview-png
```

The bounded path accepts only version-one, little-endian RES/txb with an
externally backed, table-bearing, one-mip DXT1 descriptor, 2D flags zero,
and depth one. Omitting entry selection keeps PWIB metadata-only. See the
[selected-entry contract](formats/gtex-pwib.md#selected-pwib-restxb-texture).

For several resources, first make a catalog and then name each selection:

```powershell
cargo run --locked -p xivl-cli -- extract-catalog catalog/catalog.json `
  --root "C:\path\to\FINAL FANTASY XIV" `
  --output selected `
  --id 0x12345678 `
  --path data/12/34/56/79.DAT
```

`extract-catalog` has no extract-all mode. It checks catalog identity, source
size and SHA-256, current format detection, and output limits before writing.
The defaults are 32 resources, 64 MiB of source bytes, and 128 MiB of output
bytes. Use `--max-resources`, `--max-source-bytes`, and `--max-output-bytes`
to set different positive limits.

Selected scrambled-XML DAT entries also emit their decoded documents.
Use `extract-resource` for original named SQEX widget files.

With `--export-dds`, every selected resource must be an eligible GTEX
texture or, with `--pwib-entry`, an eligible selected PWIB texture.
DDS output is included in the same output limits and atomic batch workflow.
With `--preview-png`, every selected entry must also satisfy the preview
boundary. Preview bytes count toward the same output limit.

## Verify an extraction

`verify-extraction` is read-only. It checks the manifest schema, file inventory,
relative paths, sizes, SHA-256 values, source spans, and relationships between
the manifest and its payload files:

```powershell
cargo run --locked -p xivl-cli -- verify-extraction resource
cargo run --locked -p xivl-cli -- verify-extraction resource --source resource.DAT
cargo run --locked -p xivl-cli -- verify-extraction selected `
  --catalog catalog/catalog.json `
  --root "C:\path\to\FINAL FANTASY XIV" `
  --report json
```

Use `--source` for a single-resource replay. For a catalog extraction, pass
`--catalog` and `--root` together to replay every selected source. The verifier
does not repair, create, remove, or rewrite extraction content.

## Export sheets

`extract` finds SSD sheet definition documents below the `data` directory of
the game root and writes one UTF-8 CSV per document:

```powershell
cargo run --locked -p xivl-cli -- extract "C:\path\to\FINAL FANTASY XIV" --output csv
```

The command checks the data, enable, and row-offset resources that belong to
each sheet. Missing blocks, missing trailing values, and conflicting duplicate
cells are counted in the final summary. Rich-string control tokens remain
reversible markers. The output directory must be absent or empty.
The CSV view accepts at most 4096 declared columns and rejects inconsistent
linked resources. See the [SSD contract](formats/ssd-sheet.md#static-sheet-csv-export)
for the exact boundary and public extraction coverage.

## Export zone collision geometry

`export-zones` reads documented map-layout resources from an explicit client
root and writes collision-only OBJ files, versioned metadata sidecars, and a
collection manifest:

```powershell
cargo run --locked -p xivl-cli -- export-zones "C:\path\to\FINAL FANTASY XIV" --output zones
cargo run --locked -p xivl-cli -- export-zones "C:\path\to\FINAL FANTASY XIV" --output zones --layout 0xAABBCCDD
```

The export preserves source triangle order and collision attributes. It does
not include render meshes, textures, animations, or navigation data. See
[zone geometry export](formats/zone-geometry.md) for the metadata contract.

## Lua helpers

`lua-path` reports the client's reversible transform for one ASCII resource
path:

```powershell
cargo run --locked -p xivl-cli -- lua-path Quest/Scenario/Man0g0.lua
```

`extract-lpb` removes an evidenced raw or XOR-0x73 LPB wrapper, verifies a Lua
5.1 chunk signature, and writes the compiled bytes without interpreting them:

```powershell
cargo run --locked -p xivl-cli -- extract-lpb input.lpb --output chunk.luac
```

The output file must not already exist. The `lpb-bytecode` selector can report
bounded headers, prototypes, constants by type and digest, and instruction
structure, but it does not execute, decompile, recover source, or print string
constant values.

## Advanced reports

The command-report commands are for callers who already have the corresponding
machine-readable input manifests:

- `inspect-command <id-or-name> --catalog <command_battle_params.csv>` reads a
  command catalog by numeric id or exact case-insensitive English/Japanese
  name. It preserves duplicate matches and reports client-side parameters,
  costs, timing, targeting, and evidence limits. Add `--format json` for
  canonical JSON; YAML is the default.
- `inspect-command-loadout --slot-context <command_slot_context.json>` reports
  deterministic traces of observed property writes from schema 2. The report
  marks the state as partial and non-authoritative; it is not a complete
  packet or loadout policy.
- `materialize-command-loadout --slot-context <command_slot_context.json>
  --trace <index>` projects one selected trace and optional record range into
  a 136-byte synthetic `0x0137` application payload. It keeps only selected
  record fragments, zero-fills the rest, and refuses an existing output path.

The command pages explain the evidence limits in detail:
[parameter profiles](command-parameter-profiles.md),
[formula profiles](command-formula-profiles.md),
[cost profiles](command-cost-profiles.md), and
[compatibility profiles](command-compatibility-profiles.md).

## Output and failure behavior

`inspect`, `validate`, and the advanced report commands write successful JSON
or YAML to standard output. File-export commands write their named output and
print a short summary. Diagnostics include the input path and go to standard
error, so a successful report can be piped without mixing it with an error.

| Exit status | Meaning |
|---|---|
| `0` | The requested read, report, or export succeeded. |
| `1` | Usage, input, or output failure. |
| `2` | The input was read but failed a format parse. |

Most direct file reads are bounded to 256 MiB. The command refuses unsafe
paths, ambiguous payload spans, unsupported materialization requests, and
existing output files where overwriting would hide stale data.

Use `xivl --help` for the exact synopsis. The separate [conformance runner](conformance-tests.md)
has its own case and fixture-root interface.
