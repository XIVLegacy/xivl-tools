# Support matrix

The matrix says what this project claims to handle, for exactly one client
version, in a form a machine can read. It is the answer to "does this tool
support X" and the promotion record behind every claim.

## Home of the data

`data/support-matrix.json`, validated by
`schemas/support-matrix.schema.json`. This document defines the vocabulary and the promotion rules. It deliberately holds no per-format status, so the
two cannot disagree.

Render the current matrix:

```bash
python tools/check_contract.py --print-matrix
```

## Frozen target

The target is Final Fantasy XIV 1.23b and nothing else. The static sheet
data reference is extraction `2012.09.19.0001`.

The target is frozen in the strong sense: no other 1.x version is a support
target, and no multi-version scaffolding is added in anticipation of one.
`version` is metadata, never a tooling branch key, a path segment, or a
schema key. If a second target ever genuinely emerges, it reopens this
decision rather than quietly using scaffolding that was built for it in
advance.

Lawfully held other 1.x versions may still be read by an explicit,
non-gating research invocation. Reading a file is not supporting a version:
such a run makes no matrix claim.

## Status vocabulary

Every format carries a `read`, `write`, and `export` status.

| Status | Meaning |
|---|---|
| `none` | Unsupported, and not named in a phase. |
| `planned` | Named in a program phase. Nothing is implemented. |
| `partial` | Some inputs handled. The gaps are documented and the tool reports them rather than guessing. |
| `supported` | Complete for the frozen target, with public conformance coverage. |
| `verified` | `supported`, plus recorded retail 1.23b parity through a private conformance case. |
| `not-applicable` | The operation has no meaning for this format. |

`export` names lossless views only. A view that discards information is not
an export. It is a report, and it carries no matrix status.

## Promotion rules

A status moves up only on evidence, never on intent:

1. `planned` to `partial` needs a public conformance case that passes and a
   documented statement of what is not handled.
2. `partial` to `supported` needs public conformance coverage for the
   format's full input space as this project understands it, including
   malformed input that must fail cleanly rather than crash.
3. `supported` to `verified` needs a private conformance case against a
   retail 1.23b fixture recorded in `tests/fixtures/private-manifest.json`.
4. A format may never reach `supported` or `verified` with no public case.
   A claim only the owner can check is not a claim. Contract validation
   enforces this rule.

Unknown chunks, fields, and spans are preserved and reported. Silent data
loss demotes a format; it does not stay `supported` because the common path
still works.

Statuses move down as readily as up. A defect that shows a claim was wrong
demotes the row in the same change that records the defect.

## Platform tiers

`supported` platforms build, pass the full test suite, and are required by CI.
`best-effort` platforms are built and tested only while a hosted runner
exists for them. A failure there is a bug worth fixing but does not block a
release.

## Write and round-trip claims

A `write` status of `supported` or better means a round-trip claim:
parsing the tool's own output reproduces the input model, and for formats
where the client is byte-sensitive, the bytes themselves. A writer that
cannot round-trip stays `partial` no matter how useful it is.

## Release notes

Release notes distinguish parser support from verified retail parity by
naming the status transitions in this matrix. "Now supports GTEX" without a
matrix row is not a release note.

See the [documentation index](README.md) for the format-evidence page that
backs each matrix row's status with retail citations.
