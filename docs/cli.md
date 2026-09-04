# Command-line interface

`xivl` inspects one caller-supplied file or extracts static sheets from one
caller-supplied game directory. It never searches for an install or infers a
path from the workspace. Inspection writes canonical JSON without payload
bytes, sheet-row text, or configuration values. The SSD document view
preserves sheet names and attribute values; use `--as scrambled-xml` for a
redacted census of document shape.

## Quick start

Run the binary from this checkout with Cargo:

```text
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sedb/plain-container.bin
cargo run --locked -p xivl-cli -- validate tests/fixtures/public/config/lng-words.bin --as config-lng
cargo run --locked -p xivl-cli -- lua-path Quest/Scenario/Man0g0.lua
cargo run --locked -p xivl-cli -- extract-lpb tests/fixtures/public/lpb/raw.bin --output chunk.luac
cargo run --locked -p xivl-cli -- extract "C:\path\to\FINAL FANTASY XIV" --output csv
cargo run --locked -p xivl-cli -- catalog "C:\path\to\FINAL FANTASY XIV" --output catalog
cargo run --locked -p xivl-cli -- extract-resource tests/fixtures/public/lpb/raw.bin --output resource
cargo run --locked -p xivl-cli -- extract-catalog catalog/catalog.json --root "C:\path\to\FINAL FANTASY XIV" --output selected --id 0x12345678
cargo run --locked -p xivl-cli -- verify-extraction selected --catalog catalog/catalog.json --root "C:\path\to\FINAL FANTASY XIV"
```

`inspect` reports the structure the reader found: the format ID, input length,
spans, counts, anomalies, and digests where they apply. `validate` performs
the same read and reports the checks that passed. For a format the tool can
write, it also checks that encoding the model reproduces the input bytes.
Neither command proves that a client accepts a file after it is changed.

`extract` discovers all SSD sheet definition documents under the named game
directory and writes one UTF-8 CSV per document. It validates each present
block's data length and the agreement between its enable and row-offset
resources. Missing block triples, missing trailing values, and disagreeing
duplicate logical cells are counted in the final summary. Rich-string tokens
remain reversible as named hexadecimal markers.
The output directory must be absent or empty so an older export cannot leave
stale CSV files behind.

`lua-path` applies the client's reversible transform to one ASCII resource
path. It lowercases letters, substitutes letters and digits, and leaves ASCII
punctuation and separators unchanged. A non-ASCII path fails instead of using
locale-dependent case rules.

`extract-lpb` accepts either evidenced LPB wrapper, verifies that the decoded
payload starts with a Lua 5.1 chunk signature, and writes those compiled bytes
without interpreting them. It refuses to replace an existing output file.

`catalog` walks only `.DAT` files below an explicit game or resource directory
and writes canonical `catalog.json`, or JSONL with `--format jsonl`. Rows are
path-sorted and record source identity, a resource ID when the path establishes
one, detected format, parse and support status, spans, and anomalies. Unknown
resources remain explicitly unknown. Recognized malformed resources remain in
the catalog with their typed parse failure instead of aborting the walk.

`extract-resource` reuses the selected inspection reader and writes a
schema-versioned `extraction.yaml`, or canonical JSON with `--format json`.
Use the same `--as` and `--columns` selectors as `inspect` for signatureless or
explicit views. LPB decoded bytes are written separately under `payloads/` and
referenced by path, size, role, and SHA-256; opaque data is not embedded as
base64. Add `--materialize-payloads` to a SEDB or RES input to write exact
direct-root payload entries. The output directory must be absent or empty.

For RES, direct subresources and unknown gaps become separate deterministic
files. Directory bytes remain represented in the parsed report. A nested SEDB
container stays inside its one parent subresource file and is linked from the
manifest rather than recursively duplicated. Empty spans remain empty files.
The command refuses overlap, alias, clamping, out-of-range extents, and malformed
nested containers with `ambiguous-payload-span` before writing output. It does
not guess ownership, trim, merge, decompress, or assign a semantic format to
payload bytes.

```text
cargo run --locked -p xivl-cli -- extract-resource resource.DAT --output resource --materialize-payloads
```

The output schemas and complete field semantics are documented in
[DAT catalog and resource extraction](resource-extraction.md).

`extract-catalog` consumes `catalog.json` or `catalog.jsonl` and requires an
explicit selection through one or more `--id` and `--path` options. It has no
extract-all mode. `--root` names the same source root the catalog paths are
relative to. The command validates the complete catalog and selection, source
identities, formats, limits, and every per-resource extraction plan before it
creates the absent-or-empty top-level output directory.

Defaults are 32 resources, 67108864 aggregate source bytes, and 134217728
aggregate output bytes. Override them with `--max-resources`,
`--max-source-bytes`, and `--max-output-bytes`. `--format json` changes the
default YAML batch and resource manifests to JSON. `--materialize-payloads`
passes the existing opt-in behavior to selected SEDB/RES resources.

```text
cargo run --locked -p xivl-cli -- extract-catalog catalog/catalog.json --root install --output selected --id 0x12345678 --path data/12/34/56/79.DAT --max-resources 2 --materialize-payloads
```

`verify-extraction` checks an existing single-resource or catalog extraction
without writing, repairing, or deleting anything. It auto-detects exactly one
supported root manifest, validates its embedded JSON Schema version and
semantic relationships, inventories every member, and rejects missing or extra
files and directories, unsafe or case-colliding paths, symbolic links, Windows
reparse points, hardlink aliases, digest or size changes, span errors, and
incorrect aggregate totals.

Internal verification needs only the extraction directory. Add `--source` for
a single-resource extraction to re-read the original source and reproduce its
parsed structure and payload bytes. For a catalog extraction, supply
`--catalog` and `--root` together to validate the catalog identity and replay
each selected source. `--report json` emits one compact stable summary instead
of the default text line; reports contain counts and identities, never payload
bytes.

```text
cargo run --locked -p xivl-cli -- verify-extraction resource --source resource.DAT
cargo run --locked -p xivl-cli -- verify-extraction selected --catalog catalog/catalog.json --root install --report json
```

## Select a reader

Without `--as`, the tool recognizes static-actor SAN tables, SEDB containers,
SSD documents, SQEX containers, and scrambled XML. An input with no recognized
signature is tried as SEDB and fails with a parse error if it is not a SEDB
container.

Use `--as <format>` when the bytes do not identify their format or when a
specific view is required:

| Selector | Use |
|---|---|
| `sedb` | Read a SEDB container explicitly. |
| `ssd` | Read a plaintext or scrambled SSD document through the document reader. |
| `scrambled-xml` | Report the scrambled container and a census of document shape without reading document content. |
| `sqwt` | Read a SQEX container using the input file's base name as its key. |
| `lpb` | Read an LPB wrapper and report preserved header spans and the decoded payload digest. |
| `lpb-bytecode` | Keep the LPB wrapper report and structurally inspect its evidenced Lua 5.1 payload. |
| `staticactor-san` | Read the XOR-0x73 record framing while leaving both record-member meanings unknown. |
| `enable-file` | Read the headerless enable-record array. |
| `row-offsets` | Read the headerless row-offset array. |
| `sheet-data` | Read sheet rows; add `--columns` for typed rows. |
| `config-sys`, `config-pad`, `config-lng`, `config-rgn` | Read one of the signatureless configuration-file shapes. |

`--columns` applies only to `sheet-data`. Its value is a comma-separated
schema list such as `str,s32,bool,float,u8`; omitting it reads a sheet as a
stream of string values, the representation used by an all-string sheet.

The `staticactor-san` report carries the header and record spans, the
big-endian values, decoded string lengths and digests, and two byte-shape
observations: whether each decoded string is ASCII and starts with `/`. It does
not print the strings or call either record member an actor id or class path.
Those meanings are not established by framing, and the inspection report is
not a client-data export.

SQEX names are significant: the key is the base name the file was written
under, so renaming a container before reading it produces a parse failure.
Both `/` and `\` are treated as path separators when that base name is taken,
so the same fixture argument behaves consistently across platforms.

`lpb-bytecode` is explicit because wrapper inspection and compiled-chunk
inspection are separate claims. The bytecode view reports the fixed header,
function prototypes, constant types and encoded digests, nested functions,
debug-table counts, and each instruction's decoded-chunk offset, index, raw
word, official opcode number and name, encoding mode, and mode-appropriate
operands. An `iABC` B or C operand is structurally marked as unused, a value,
a register, or an RK register/constant reference. Constant references report
only their encoded index; the report does not invent or expose a referenced
constant value.

The reader validates prototype-local references and register spans that the
official Lua 5.1 instruction semantics make unconditional. That includes
constant, upvalue, and nested-prototype indices; direct registers against
`maxStackSize`; jump bounds; and the raw extra word after `SETLIST C=0` plus
the binding words after `CLOSURE`. A SETLIST extra word is preserved separately and is not decoded as an opcode. It does not build control flow, analyze
reachability or register liveness, simulate the stack, enforce compiler-only
branch pairing, or execute VM behavior. It also does not recover source, print
string constants, produce pseudocode, or decompile. Bytecode spans and
instruction offsets are relative to the decoded chunk; wrapper spans remain
relative to the input file. The parser refuses unsupported target headers,
more than 128 nested prototype levels, more than 10000 prototypes, more than
1000000 aggregate table entries, more than 16 MiB of aggregate string storage, and any bytes after the root prototype.

Examples using the authored public fixtures:

```text
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sheet/row-offsets.bin --as row-offsets
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sheet/rows-typed.bin --as sheet-data --columns str,s32,bool,float,u8
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sqwt/window.bin --as sqwt
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/lpb/bytecode.bin --as lpb-bytecode
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/staticactor/records.bin --as staticactor-san
```

## Reports and failures

Successful `inspect` and `validate` reports are canonical JSON on standard
output. `extract` instead prints a plain-text file summary. Diagnostics include
the caller's path and go to standard error, so a caller can pipe a successful
report without mixing it with an error message. The exit statuses are:

| Status | Meaning |
|---|---|
| `0` | The requested read and report succeeded. |
| `1` | Usage, option, input, or output failure. |
| `2` | The input was read but failed a format parse. |

Use `xivl --help` for the exact command synopsis and accepted selector list.
The conformance runner has a separate case and fixture-root interface. See
[conformance tests](conformance-tests.md) for that workflow.

See [format evidence](format-evidence.md) for the facts behind the reports,
[support status](support-matrix.md) for the claim limits, and the
[documentation index](README.md) for the rest of the public documentation.
