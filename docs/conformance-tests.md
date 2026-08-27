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

`extract` exercises the lossless CSV view for a public `sheet-data` fixture
using the same `--as` and `--columns` arguments as the CLI reader. Private
fixtures whose root was not supplied are skipped with a reason.

## Case outcomes

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
