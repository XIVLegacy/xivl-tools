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

## Select a reader

Without `--as`, the tool recognizes SEDB containers, SSD documents, SQEX
containers, and scrambled XML. An input with no recognized signature is tried
as SEDB and fails with a parse error if it is not a SEDB container.

Use `--as <format>` when the bytes do not identify their format or when a
specific view is required:

| Selector | Use |
|---|---|
| `sedb` | Read a SEDB container explicitly. |
| `ssd` | Read a plaintext or scrambled SSD document through the document reader. |
| `scrambled-xml` | Report the scrambled container and a census of document shape without reading document content. |
| `sqwt` | Read a SQEX container using the input file's base name as its key. |
| `lpb` | Read an LPB wrapper and report preserved header spans and the decoded payload digest. |
| `enable-file` | Read the headerless enable-record array. |
| `row-offsets` | Read the headerless row-offset array. |
| `sheet-data` | Read sheet rows; add `--columns` for typed rows. |
| `config-sys`, `config-pad`, `config-lng`, `config-rgn` | Read one of the signatureless configuration-file shapes. |

`--columns` applies only to `sheet-data`. Its value is a comma-separated
schema list such as `str,s32,bool,float,u8`; omitting it reads a sheet as a
stream of string values, the representation used by an all-string sheet.

SQEX names are significant: the key is the base name the file was written
under, so renaming a container before reading it produces a parse failure.
Both `/` and `\` are treated as path separators when that base name is taken,
so the same fixture argument behaves consistently across platforms.

Examples using the authored public fixtures:

```text
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sheet/row-offsets.bin --as row-offsets
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sheet/rows-typed.bin --as sheet-data --columns str,s32,bool,float,u8
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sqwt/window.bin --as sqwt
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
