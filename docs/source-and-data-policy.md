# Source and data policy

Use this policy to decide which source and data may be committed and how to
test material that must stay outside the repository. Contract validation
enforces its mechanical rules.

## Public code

- Every authored source file is in-house work under GNU AGPL v3 or later.
- External projects may be behavior targets and lead material. Their code is
  not copied, machine-translated, or vendored.
- A published byte layout or field name from an external project is a lead
  until it is independently established against retail 1.23b data. Only
  then may it become a claim in the support matrix.
- Reuse of external code needs a permitting license, explicit owner
  decision, an entry in `NOTICE`, and full compliance with the upstream
  terms.
- A fact needed from a sibling repository is promoted here with a
  mandatory, immutable `repository:path, sha256 <digest>` citation, a local
  test, and local ownership thereafter. No sibling checkout, package,
  release, CI artifact, or cross-repository freshness check is added.

## Public data

Committable:

- contract data under `data/` with a JSON Schema in `schemas/`;
- authored synthetic fixtures under `tests/fixtures/public/`;
- conformance case manifests and expected normalized outputs;
- format evidence documents carrying retail citations.

A synthetic fixture is bytes written by this project to exercise a parser,
including deliberately malformed bytes. It is never a copy, a slice, a
transformation, or a re-encoding of retail data.

## Never committed

- client executables, client binaries, and client assets, including `.dat`
  and `.san` files;
- extracted, decoded, converted, or re-encoded retail data of any kind,
  including textures, models, sheets, strings, and patch payloads;
- retail patch files and any fragment of one;
- private artifact-store contents and credentials of any kind;
- database dumps and generated build output.

`.gitignore` keeps the private fixture root and build outputs untracked.
The boundary check pins that policy and fails if ignored or private material is
tracked. Validation is a backstop, not the rule.

## User-written data

The categories above split what a file is by where it came from: this
project's own bytes, or the client's. A third kind turned up that neither
describes - a file the **client wrote on the owner's machine from the owner's
own input**:
configuration files, the `user/` subtree, screenshots, logs.

It is not a client asset, so "never committed" did not cover it, and it is
not synthetic, so "committable" must not. Its own rule:

- user-written data is never committed, in whole or in part, and never
  reconstructible from what is committed;
- it is testable the same way retail data is, through the private-fixture
  manifest: identity only, bytes on the owner's machine;
- an expected output records its shape - lengths, spans, counts, digests -
  and never a value. A configuration file's values are settings, an input
  device's identifier is hardware the owner owns, and a macro file's text
  is the owner's writing. None of them is a byte layout, so none of them
  is evidence this project needs;
- a report that would carry such a value does not get written in the first
  place. `inspect --as config-sys` reports where the written words are and
  not what they say, which is a property of the document rather than a
  rule applied to it afterwards.

The distinction that matters: a fact about the **format** is public
evidence and belongs on the relevant page under `docs/formats/`; dates and counts
included. A fact about the **owner** is neither, whatever it would prove.

## Private retail fixtures

Conformance against real 1.23b data is required by the program, and the
data cannot be published. The split is:

- the bytes stay on the owner's machine, outside this checkout;
- this repository tracks only identity: a stable fixture id, sha256, size
  in bytes, the client-relative source path, and the frozen client version.
  A selected input copied to the approved restricted fixture store also
  records that store and its exact containing commit;

That record lives in `tests/fixtures/private-manifest.json` and is
validated by `schemas/private-fixture-manifest.schema.json`.

Resolution rules:

- an entry names the **root** it lives under, defaulting to
  `client-install`. The configuration files are not under the install, so
  they name `user-config`; see `docs/conformance-tests.md`;
- a root that something else writes is frozen into a snapshot first, by
  `tools/freeze_private_fixtures.py`, so a fixture stops matching the
  manifest only when the owner meant it to;
- the runner locates the bytes through an explicit `--fixture-root` option
  or an environment variable, one per root;
- there is no default path for any root, no workspace-relative fallback,
  and no download;
- when the root is absent, private cases report themselves skipped with a
  reason and the run stays green;
- `--require-private` turns that skip into a hard failure, for the owner's
  own pre-release runs;
- when the root is present and a file's sha256 does not match the manifest,
  the run fails loudly. A mismatch means the fixture is not the frozen
  1.23b file the claim was established against.

A private fixture is never copied into the repository, quoted in a test
expectation, embedded in a golden file, or reconstructed from an expected
output. An expected output for a private case records derived facts such as
counts, offsets, and structural summaries, never recoverable payload bytes.

## Public and private test parity

Every format claim carries public coverage. A claim proved only by a
private fixture is a claim nobody else can check.

- a public synthetic case exercises the parser shape and its malformed
  inputs;
- a private case establishes retail parity;
- the support matrix separates the two: a format reaches `verified` only
  with a recorded private-fixture case, and it may not reach `supported`
  with no public case at all.

## Enforcement

The boundary check (`tools/validate_repo.py`) pins the tracked tree and raw
`.gitignore`, rejects every ignored content category, scans tracked content for
PE files, maintainer paths, and private references, validates the tracked
fixture declarations, and rejects any retired in-tree private fixture mirror.
Actual fixture bytes are checked only from explicit external roots by private
conformance or the snapshot tool.

The contract check (`tools/check_contract.py`) checks:

- no non-ASCII byte in a tracked file;
- every contract data file validates against its schema;
- no tracked path under the private fixture root or a build output root;
- no tracked file carries a default path to a sibling repository;
- support-matrix claims satisfy the coverage rule above.

Validation cannot detect retail bytes disguised as a synthetic fixture. That
one is on the author.

See the [documentation index](README.md) for the conformance-tests page
that defines the fixture roots this policy constrains.
