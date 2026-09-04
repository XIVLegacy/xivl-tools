# DAT catalog and resource extraction

The catalog and extraction commands turn caller-supplied resources into
deterministic, machine-readable records. They never search for an install,
change source files, or infer a meaning for unknown bytes.

## Catalog schema

`xivl catalog` writes `catalog.json` by default. The document conforms to
`schemas/resource-catalog.schema.json`. `--format jsonl` instead writes one
compact resource object per line to `catalog.jsonl`; each line carries the
same schema version and resource shape.

Resources are sorted by their root-relative path using a case-insensitive key.
Every row records the path, size, SHA-256, and a resource ID only when the path
matches the established `AA/BB/CC/DD.DAT` convention. Detection is limited to
formats with evidenced signatures. The three `formatStatus` values are:

- `parsed` - a known reader accepted the complete input.
- `malformed` - a known signature was present, but its reader returned a typed
  error. The error kind, offset, and detail are retained as an anomaly.
- `unknown` - no supported signature was established. The format remains
  unknown and no reader is guessed.

`supportStatus` is the matching read level from the support matrix. `spans`
collects every reported offset and length with its JSON path. Format-specific
anomalies are copied without changing their meaning.

## Extraction schema

`xivl extract-resource` writes `extraction.yaml` by default. `--format json`
writes the same data as canonical JSON. Both forms conform to
`schemas/resource-extraction.schema.json` after YAML is loaded as a data model.
The document records source identity, the tool version and optional build
commit, format and parse status, the existing inspection report under
`parsed`, anomalies, and references to separate payload files.

The build commit is `null` unless the binary was built with
`XIVL_GIT_COMMIT` set. This avoids deriving repository state at run time.
LPB extraction writes its decoded Lua 5.1 chunk to
`payloads/decoded.luac`; the YAML or JSON document contains only its relative
path, role, size, and digest. Other reports do not copy opaque source spans.
Large payloads are never base64-encoded into the document.

`--materialize-payloads` opts a SEDB or RES input into exact binary payload
materialization. Without the flag, container extraction remains a structural
report. A plain SEDB writes its resolved payload entry. A RES writes each
direct root subresource and each unknown gap, but not its directory bytes.
Nested containers remain one direct parent subresource file; the manifest
records the child format, subtype, and span without recursively writing the
same bytes again. Empty entries produce empty files.

Each filename contains the root entry ordinal, role, 16-digit hexadecimal
offset and length, and a 16-digit SHA-256 prefix. The stable entry ordinal makes
the name collision-safe; the manifest retains the full digest. It repeats the
source span as offset, length, and exclusive end, and links it to JSON paths for
the owning container and entry. Subresource records retain their index,
declared offset, declared size, unresolved kind, and optional child-container
relationship.

Exact materialization refuses a container before creating the output directory
when any nested level reports an overlapping or aliased subresource, a clamped
or out-of-range extent, a span past the resolved container end, or a malformed
nested SEDB signature. These conditions remain inspectable as anomalies, but
the extraction command returns `ambiguous-payload-span` rather than choosing
which declaration owns bytes. A malformed or truncated container retains its
ordinary typed parser error.

Signatureless readers remain explicit through `--as`. SQEX decoding continues
to use the source file's own base name as its key, so extracting through this
command preserves the filename-dependent input rather than renaming a copy.

GTEX and PWIB extractions are metadata-only. Their manifests preserve the
signature and the span and SHA-256 of every later byte, but `payloads` remains
empty because the evidence does not establish where an encoded texture or
other payload begins. Source replay through `verify-extraction` reproduces and
compares that report. Exact payload materialization and DDS/PNG conversion are
not supported.

Both commands require an output directory that is absent or empty. A
non-directory output path, nonempty output directory, symbolic link in a
catalog walk, unreadable input, unrecognized extraction format, or malformed
resource produces a noninteractive diagnostic and a nonzero exit status.

## Explicit catalog selections

`xivl extract-catalog` reads either catalog form and extracts only entries
named by `--id` or `--path`. At least one selection is required; there is no
implicit or explicit extract-all option. IDs use the canonical 32-bit resource
identity, while paths are catalog-relative and accept either separator at the
CLI boundary. Resolved entries are deduplicated strictly and sorted by catalog
index, so argument order cannot change output order.

The top-level YAML or JSON data model conforms to
`schemas/catalog-extraction.schema.json`. Every isolated resource manifest
continues to conform to `schemas/resource-extraction.schema.json`.

The required `--root` is the filesystem root used to resolve catalog paths.
Every catalog path must be relative, normalized, traversal-free, and free of
drive prefixes or alternate-stream colons. Duplicate catalog rows, IDs, exact
paths, and case-folded path aliases are refused. Each selected path component
is checked for symbolic links and, on Windows, reparse points. The resolved
regular file must remain below the canonical root.

Before output creation, the command verifies each selected source's catalog
size and SHA-256, plans its extraction with the same code as
`extract-resource`, and checks that current format detection still matches the
catalog. Missing, unknown, malformed, stale, mixed unsupported, or otherwise
failed selections refuse the whole batch. No successful `batch.yaml` or
`batch.json` is written for a refused plan.

The conservative defaults are:

| Limit | Default | Option |
|---|---:|---|
| Selected resources | 32 | `--max-resources` |
| Aggregate source bytes | 67108864 | `--max-source-bytes` |
| Aggregate output bytes | 134217728 | `--max-output-bytes` |

All limits must be positive integers. Count and catalog source-size limits are
checked before source reads. Actual sizes and hashes are then verified.
Aggregate output accounting includes every per-resource manifest, every
payload, and the top-level batch manifest. Every addition is checked for u64
overflow. `resource-count-limit-exceeded`, `source-byte-limit-exceeded`,
`output-byte-limit-exceeded`, and the corresponding accounting-overflow errors
are stable noninteractive diagnostics.

Each resource receives an isolated directory named from its selection ordinal,
resource ID when present, and source-digest prefix. The top-level manifest
records the catalog identity, configured limits, totals, catalog indexes,
source identities, detected formats, isolated directories, and relative
resource-manifest paths. YAML is the default; `--format json` changes both
batch and resource manifests. `--materialize-payloads` enables the existing
SEDB/RES behavior without changing LPB's already separate decoded payload.

The writer remains serial. Planning order and write order are both catalog
order, and no parallel extraction machinery is present. A validated batch is
written to a same-parent staging directory and published by rename only after
all isolated outputs and the batch manifest succeed. A refused or failed batch
does not appear at the requested output path; a stale staging path is refused
rather than overwritten.

## Read-only verification

`xivl verify-extraction <directory>` auto-detects a single
`extraction.yaml`/`extraction.json` or `batch.yaml`/`batch.json`. More than one
root manifest is ambiguous. The verifier accepts schema version 1, validates
the loaded YAML or JSON against the corresponding embedded JSON Schema, then
checks invariants that the schema alone cannot express.

For one resource, it verifies every declared payload's normalized relative
path, regular-file identity, size, SHA-256, source-span arithmetic, and parsed
container and entry relationship. Exact directory membership is required, so
missing and unlisted files or directories fail. `--source <file>` additionally
checks source name, resource identity when derivable, size, digest, parsed
structure, materialization plan, exact source slices, and LPB decoding.

For a batch, every isolated resource receives the same verification. The
top-level resource records, ordinals, catalog indexes, paths, formats, byte
counts, and aggregate totals must agree with their nested manifests and actual
files. `--catalog <file> --root <directory>` must be supplied as a pair. It
checks the recorded catalog identity, resolves each selected regular source
below the explicit root, and performs source replay for every resource.

The inventory refuses path traversal, alternate-stream syntax, case-folded
collisions, symbolic links, Windows reparse points, hardlink aliases, and
non-regular members. Arithmetic is checked for overflow. Failures use stable
typed prefixes such as `schema-validation-failed`, `payload-sha256-mismatch`,
`extra-file`, `file-alias-refused`, `stale-source-sha256`, and
`batch-totals-mismatch`. The command never repairs, creates, removes, or
rewrites extraction content. `--report json` changes only the concise success
summary and never includes payload bytes.
