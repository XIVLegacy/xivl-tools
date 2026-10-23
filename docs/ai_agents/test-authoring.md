# Test authoring

Add tests for a demonstrated defect or distinct supported behavior or failure.
First look for an existing test to extend. Do not repeat a covered branch with
the same assertions or add scenarios solely to match changed line count.

## Choose the boundary

Use the cheapest check that can detect the defect:

1. A focused format-library test for bounded parsing, writing, or conversion.
2. A data or contract check for catalog rows, schemas, and repository shape.
3. An in-process CLI test for materialization, manifests, filesystem failures,
   destination protection, output accounting, or source replay.
4. A CLI child process when option routing, stdin, stdout, stderr, or exit
   status is the behavior under test.
5. A conformance case for a fixture's stable normalized report or refusal.

Keep reusable format semantics in the owning library tests. Do not launch the
CLI repeatedly to prove pure decoding behavior. Keep real command and file I/O
checks when they can catch failures that an in-memory test cannot.

Conformance has a distinct role: it checks normalized structure, spans,
digests, and typed refusals. It does not replace literal assertions for decoded
values, expression trees, exact CSV bytes, or pixel/channel behavior. Follow
[conformance tests](../conformance-tests.md) for case operations and reports.

## Make the assertion independent

- Use authored bytes and literal expected fields, spans, error kinds, and
  offsets. Do not compute an expected answer with the production decoder,
  writer, registry, or formula being checked.
- For malformed bounded inputs, assert the typed error and meaningful offset,
  not just that parsing did not panic. Keep exact failure-offset and nesting
  assertions alongside broad panic and input-range sweeps.
- For lossless readers and writers, assert exact re-encoding. For edits,
  assert intended changes and preservation of opaque bytes, padding, unknown
  spans, and unselected records. A successful reparse alone is insufficient.
- Keep literal output checks alongside round trips: a reader and writer can
  agree on the same wrong interpretation.

The [rich-string contract tests](../../src/formats/tests/richstring_contract.rs)
show independent trees, bytes, and offsets. The
[model importer tests](../../src/formats/src/model_import.rs) check preservation
of opaque fields and explicit allocations.

## Fixtures and input products

Use the smallest authored synthetic fixture that reaches the behavior.
Generate committed public fixture bytes through
[the public fixture generator](../../tools/make_public_fixtures.py). Public
fixtures never contain copied, sliced, transformed, or re-encoded retail data.
Private inputs require explicit roots and manifest identities under the
[fixture contract](../conformance-tests.md#public-and-private-fixtures).
Missing private inputs are skips or required-private failures, not retail
coverage. Keep recoverable private values out of expected reports.

Preserve fixture names when they affect decoding. SQEX uses the basename as
its key, so equal bytes under different names are not necessarily equivalent.

Check every relevant authored catalog row in a data check when row coverage is
the claim. Use representative integrated combinations only when they cover
each distinct branch, mode, filename dependency, boundary, and assertion.
Do not add combinations that return before reaching the intended behavior.

Before reducing repeated inputs or reader products, prove which distinct
fixture/name/mode/assertion coverage remains. The
[malformed-input sweeps](../../src/formats/tests/malformed_inputs.rs) deliberately
exercise multiple readers, typed offsets, depth limits, and round trips.
Retain those contracts. If sampling drops a distinct combination, document the
lost coverage for review instead of calling the reduction equivalent.

## Scaffolding, cost, and verification

Reuse small helpers in the owning test module. Extract only shared
setup; preserve independent literals and distinguishing fixture bytes. Extend
an existing Rust test target when it fits instead of adding a standalone
integration executable or a generic fixture framework.

For a material expansion in mutation products, filesystem work, subprocesses,
or test source, use representative runtime, build, or source-cost evidence to
explain the cost and chosen boundary. Distinguish compilation from execution;
source size alone does not establish build time. Small focused additions do
not require a benchmark.

Run a focused check that would detect the defect and the applicable repository
checks. Commands stay in [maintenance tools](../../tools/README.md) and
[conformance tests](../conformance-tests.md#running). Review generated
expectation diffs before accepting them. Report what ran and any private skips
or platform limits without claiming unexecuted coverage.
