# Conformance tests

The interface compares this project's output against authored expectations and
retail 1.23b data without publishing anyone's client files.

## Pieces

| Piece | Home | Schema |
|---|---|---|
| Case manifest | `tests/conformance/cases/<id>/case.json` | `schemas/conformance-case.schema.json` |
| Expected output | next to its `case.json` | normalized JSON, see below |
| Public fixture | `tests/fixtures/public/` | authored synthetic bytes |
| Private fixture identity | `tests/fixtures/private-manifest.json` | `schemas/private-fixture-manifest.schema.json` |

The case directory name equals the case `id`. Validation enforces that, and
that `formatId` names a row in the support matrix.

## Normalized output

Every comparison happens on one canonical form, so a case never depends on
formatting:

- UTF-8 JSON restricted to ASCII, sorted object keys, two-space indent, LF
  line endings, and a trailing newline;
- numbers as JSON numbers, never as formatted strings;
- byte spans as `{"offset": N, "length": N}`, offsets absolute from the
  start of the input;
- binary values as lowercase hex strings;
- no absolute path, no host name, no timestamp, and no duration anywhere in
  the document;
- unknown chunks, fields, and trailing bytes present as explicit entries.
  Losing an unknown span silently is a conformance failure, not a
  formatting difference.

The `client-lua` instruction list keeps the aggregate code span, count, and
digest. Each item adds its decoded-chunk offset and span, zero-based index, raw
32-bit word, opcode number and name, encoding mode, and an `operands` object.
Only `A`, `B`, and `C`; `A` and `Bx`; or `A` and `sBx` appear according to the
mode. B and C objects carry a stable `kind`; RK register and constant objects
also retain their raw field, decoded index, and `rk: true`. A constant reference
is not resolved to constant-table content in normalized output. A raw word
following `SETLIST C=0` appears in `setlistExtraWords`, not in the decoded
instruction items, because the official VM consumes it as data rather than an opcode.

## Operations

`inspect` reports what an input holds. `validate` reads it the same way and
reports the checks that reading passed, which is how a write claim is
tested: for a format this project can also write, the model is encoded back and the bytes must reproduce the input exactly. A `validate` case is
therefore an ordinary `ok` case whose expected output names the checks and
their results, and a writer that stopped round-tripping fails it rather
than quietly passing an `inspect` case that never wrote anything.

`extract` exercises the lossless CSV view for a `sheet-data` fixture using
the same `--as` and `--columns` arguments as the CLI reader. For `sqwt` and
`scrambled-xml`, it checks the exact decoded-document export contract using
the existing decoders, with normalized structural facts, output length, and
SHA-256 rather than decoded text. SQEX uses the fixture's basename as its
key. CLI tests separately exercise file materialization, manifests, catalog
selection, output accounting, destination protection, and verification
failures. Private fixtures whose root was not supplied are skipped with a
reason.

`rich-string` is a conformance-only operation for an authored public
decoded rich-string fixture. It accepts no arguments and calls production
`RichString::parse` and `Token::expressions` for every token. Its normalized
report retains framing metadata, spans, digests, expression counts or
payload-relative failure offsets, exact re-encoding status, and the length
and digest of lossless text. It retains no text, raw token hex, or decoded
expression values. A malformed expression is a per-token result, so the
raw token remains accounted for rather than turning framing into a failure.
Malformed framing uses the ordinary typed `parse-error` expectation.

The rich-string public cases cover all known macro names, established
length forms, expression productions, unknown codes, malformed expressions,
and bounded nesting. Ordinary `extract --as sheet-data` cases check the
CSV path. Separate literal tests specify expression trees, exact
re-encoding, failure offsets, literal escaping, and complete CSV bytes
without deriving expectations from production decoding. See the
[rich-string contract](formats/ssd-sheet.md#rich-string-read-and-text-export-contract).
The dedicated operation is restricted to public fixtures and establishes
no retail expression parity.

For `gtex`, `extract` with `--export-dds` exercises the DDS texture view.
Reports retain header metadata, mip source and output spans, counts, and SHA-256 digests rather
than encoded pixels. Authored tests check header fields independently
against Microsoft's DDS definitions. CLI tests cover opt-in DDS output,
raw-surface coexistence, catalog extraction, accounting, and verification
with and without source replay. See the
[DDS contract](formats/gtex-pwib.md#lossless-dds-texture-view).

GTEX `extract` cases with `--preview-png` exercise decoded top-mip previews.
Reports retain source spans, dimensions, format metadata, and encoded,
decoded RGBA, and PNG digests. Expected reports contain no pixel arrays.
Small authored pixel oracles separately check A8R8G8B8 channel order, both
DXT1 modes, DXT5 alpha behavior, and partial-block cropping. CLI tests
exercise both extraction commands and verification failures. See the
[preview contract](formats/gtex-pwib.md#decoded-top-mip-png-preview).

The three standalone retail GTEX inspect fixtures also have separate private
DDS and PNG extract cases. Run all nine against a frozen root preserving the
manifest's original `data/` paths:

```text
conformance run --case retail-gtex-61c10005
                --case retail-gtex-a8r8g8b8-1c59027e
                --case retail-gtex-dxt5-1c590028
                --case gtex-retail-61c10005-dds
                --case gtex-retail-61c10005-png
                --case gtex-retail-a8r8g8b8-1c59027e-dds
                --case gtex-retail-a8r8g8b8-1c59027e-png
                --case gtex-retail-dxt5-1c590028-dds
                --case gtex-retail-dxt5-1c590028-png
                --fixture-root client-install=<explicit-snapshot-root>
                --require-private
```

Expectations retain spans, dimensions, mip counts, sizes, and digests, including
the decoded top-mip RGBA identity. They contain no encoded bytes or pixels.
The [standalone GTEX replay record](formats/gtex-pwib.md#standalone-retail-export-replays)
describes the independent raw, DDS, and PNG checks through both extraction
commands. Missing unrelated fixtures do not stand in for these selected checks.

WRB model `inspect` and `validate` cases use explicit `--as wrb-model`.
Public authored fixtures cover accepted mesh structure, no-MESH resources,
unknown chunks, exact tags, malformed descriptors, truncation, counts, and
bounded nesting. Reports retain absolute source spans, structural metadata,
counts, digests, and opaque ranges without vertex or index arrays. Validation
marks round-trip as not applicable. See the
[WRB inspection contract](formats/wrb-model.md).

The two private WRB cases reuse `retail-res-89eb0000` and preserve its
manifest-declared `data/89/EB/00/00.DAT` path. Include its retained generic
RES inspect case to check that the views remain separate:

```text
conformance run --case retail-res-89eb0000
                --case wrb-model-retail-89eb0000
                --case wrb-model-retail-89eb0000-validate
                --fixture-root client-install=<explicit-snapshot-root>
                --require-private
```

The WRB evidence page records independent chunk and stream range accounting
against this exact input. No private bytes or decoded geometry are retained
in these expectations.

PWIB `extract` cases use `--pwib-entry <index>` with the texture output
options to exercise selected RES/txb extraction. Reports retain entry and
type metadata, descriptor and table spans, second-relative offsets,
absolute encoded spans, and artifact digests. Authored fixtures cover
nonzero surface offsets and whole-range refusals, including a surface
whose start fits the second span but whose end does not. Separate command
tests cover raw, DDS, and PNG output, catalog selection, inventory failures,
manifest consistency, and complete source replay. See the
[selected PWIB contract](formats/gtex-pwib.md#selected-pwib-restxb-texture).

Private selected-PWIB expectations replace every decoded entry name with
`nameByteLength` and `nameSha256`, including repeated selected and metadata
entries. The names remain in ordinary extraction manifests outside the
repository. Public authored cases retain their synthetic names.

The pinned m520 entry-6 cases reproduce selected structure, DDS, and PNG
identities without retaining surface bytes or decoded samples. Supply a
root containing the manifest's original client-relative path explicitly:

```text
conformance run --case pwib-retail-m520-entry6
                --case pwib-retail-m520-entry6-dds
                --case pwib-retail-m520-entry6-png
                --fixture-root <explicit-snapshot-root> --require-private
```

The root may be an identity-verified private copy preserving that path.
An arbitrary DAT alias for catalog testing retains the original provenance
outside the repository and is selected by catalog path, not an inferred
resource ID. The exact source and independent output checks are recorded
in the selected PWIB contract above.

`extract-directory` exercises the production whole-directory `extract`
implementation for `ssd-sheet`. Its `public-tree` fixture names a generated
JSON descriptor with `format: "ssd-extract"`, `schemaVersion: 1`, and a
`resources` list. Each resource records an explicit hexadecimal `id` and
the relative `file` of an authored `.bin` fixture. The runner materializes
those bytes at their resource-ID DAT paths in an isolated test directory.
No client install or previously built CLI executable is required.

Directory reports contain output filenames, byte counts, SHA-256 digests,
and the extraction summary. They contain no CSV cells. Separate literal
CSV assertions check the generated trees' values, missing and duplicate
markers, sparse rows, and deterministic repeat extraction. Malformed sets
exercise incomplete resource triples, schema mappings, row offsets,
enable ranges, and row decoding. See the
[SSD CSV contract](formats/ssd-sheet.md#static-sheet-csv-export).

Directory cases accept no extra arguments and pin a typed extraction error
kind instead of `errorOffset`: the operation spans several independently
addressed resources. Fixture setup and output I/O failures cannot satisfy
a `parse-error` expectation.

## Case outcomes

The `region` cases exercise RegionResourceData 1.1.0 root and child walking
through `inspect` and `validate`. Public fixtures include duplicate rows and
a child with nonzero opaque `+0x0C`, plus concrete header and count boundaries.
Private expectations retain membership, counts, spans, and digests rather than
decoded tokens, row values, or recoverable payload bytes. Run the pinned vector
against an explicit frozen root containing its original `data/` path:

```text
conformance run --case retail-region-03c00000-inspect
                --case retail-region-03c00000-validate
                --fixture-root client-install=<explicit-snapshot-root>
                --require-private
```

The [region contract](formats/region.md) owns the vector identity, independent
row accounting, and remaining evidence limits.

A case expects `ok` with an expected output document, or `parse-error` with
a stable `errorKind` and, optionally, the `errorOffset` the error must
carry. Cases with malformed input are first class: the parser contract requires
no panics on malformed public inputs, and the only way to hold that line is
to assert the error, not merely the absence of a crash. The offset is part
of that contract, so a case that pins `errorOffset` asserts the parser
stopped where it should; a case that names only `errorKind` still passes on
the kind alone.

## Public and private fixtures

A public fixture is authored synthetic bytes committed to the repository.
It is never a copy, slice, transformation, or re-encoding of retail data.

A private fixture stays on the owner's machine. The repository stores only
its id, its root, its root-relative source path, sha256, size, and the
formats it covers.

### Fixture roots

One root was enough while every private fixture was a resource under the
client install. The configuration files are not there - the client keeps
them in the user's documents - so an entry names the root it belongs to:

| Root | What it is |
|---|---|
| `client-install` | The client install root. The default when an entry names no root. |
| `user-config` | The directory the client keeps its configuration in, outside the install. |

A root is a name, not a path. The runner takes one directory per root and
has no default for any of them, which is the same rule a single root
already had. Adding a second changes where the bytes are, not who supplies
them.

A root points at a **snapshot**, not at a directory something else writes.
The client rewrites its configuration whenever a setting changes, so a
`user-config` root aimed at the live directory would stop matching the
manifest the moment the owner played, and a case would fail for a reason
that has nothing to do with this project's code. Freeze one, outside this
checkout so `git clean` cannot remove it:

```bash
python tools/freeze_private_fixtures.py --root user-config   --from <live dir> --into <snapshot dir>
python tools/freeze_private_fixtures.py --root user-config --check <snapshot dir>
```

A source file that does not hash to what the manifest records is reported and not copied. Re-establishing a fixture is a deliberate act - copy,
update the manifest, rerun the affected cases with `--update-expected`, and read the diff - because the claim was established against particular
bytes.

The client install needs none of this. Its files change when the client is
patched, and the target is frozen at 1.23b.

Resolution:

- the runner takes `--fixture-root <dir>` for `client-install` and
  `--fixture-root <root-id>=<dir>` for any other, or reads
  `XIVL_TOOLS_FIXTURE_ROOT` and, per further root,
  `XIVL_TOOLS_FIXTURE_ROOT_<ROOT_ID>`;
- an argument is a named root only when what precedes its `=` has the
  root-id shape, so a Windows drive letter is a path and not a root;
- a case skips, or fails under `--require-private`, when *its own* root is
  absent. Supplying one root does not stand in for another;
- there is no default, no workspace-relative fallback, and no download;
- with no root, private cases report themselves skipped with a reason and
  the run is green;
- `--require-private` turns those skips into failures, for the owner's
  pre-release runs;
- a sha256 mismatch fails loudly. The claim was established against a
  specific file, and this is not that file.

An expected output for a private case records derived facts: counts,
offsets, structural summaries, and unknown-span inventories. It never
records recoverable payload bytes. A private case whose expected output
would let a reader reconstruct client data does not land.

## Running

The runner contract is:

```text
conformance run [--case <id>]... [--format <id>]...
                [--fixture-root [<root-id>=]<dir>]... [--require-private]
                [--update-expected]
                [--repo-root <dir>]
```

- default run: every case, public fixtures only;
- `--repo-root` names the checkout to run against and defaults to the working
  directory; the runner does not search parent or sibling directories;
- `--update-expected` rewrites expected outputs and is never used in CI;
- exit status is non-zero on any failure, and skipped cases are listed with
  their reason in the summary. A run that silently skips everything and
  exits zero is the outcome this interface is designed to prevent.

## Illustrative case

This synthetic example shows the manifest shape. Its values are placeholders,
not established facts.

```json
{
  "schemaVersion": 1,
  "id": "example-container-inspect",
  "formatId": "sedb",
  "operation": "inspect",
  "fixture": {
    "kind": "public",
    "path": "tests/fixtures/public/sedb/example-container.bin"
  },
  "expect": {
    "outcome": "ok",
    "output": "expected.json"
  }
}
```

See the [documentation index](README.md) for the support-matrix page that
governs which cases a status claim requires.
