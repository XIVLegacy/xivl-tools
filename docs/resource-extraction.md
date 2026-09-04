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

Signatureless readers remain explicit through `--as`. SQEX decoding continues
to use the source file's own base name as its key, so extracting through this
command preserves the filename-dependent input rather than renaming a copy.

Both commands require an output directory that is absent or empty. A
non-directory output path, nonempty output directory, symbolic link in a
catalog walk, unreadable input, unrecognized extraction format, or malformed
resource produces a noninteractive diagnostic and a nonzero exit status.
