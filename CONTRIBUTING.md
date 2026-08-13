# Contributing

XIVLegacy Tools accepts focused pull requests against `main`. Contributions
are licensed under the repository's AGPL-3.0-or-later license.

## Before contributing

Work from a fork and open a pull request against `main`. Keep the change to
one format, tool behavior, documentation batch, or maintenance concern, and
make sure CI is green before asking for review.

Read the [project scope](docs/project-scope.md),
[source and data policy](docs/source-and-data-policy.md), and
[evidence rules](docs/ai_agents/evidence-and-claims.md) before changing a
format claim or test boundary.

You own every submitted change, including AI-assisted work. Do not open a
pull request for a diff you could not explain yourself: what each material
change does, why it belongs, and how the evidence and verification support it.

## Code and documentation

This is a Rust workspace of clean-room format parsers. Parsers accept bounded
byte slices, return `Result`, never panic on malformed input, and preserve a
typed error and failing offset. Workspace code does not use `unsafe`.

Do not commit client binaries, client assets, decoded retail corpora, credentials,
private fixture bytes, user-written data, or generated build output. Public
fixtures are authored synthetic bytes, never excerpts,
transformations, or re-encodings of retail data.

Keep code, tests, contract data, and documentation in agreement. Format claims
need the evidence and conformance coverage required by the
[support matrix](docs/support-matrix.md). Public prose follows the
[documentation policy](docs/ai_agents/README.md).

## Verification

The [checks workflow](.github/workflows/checks.yml) is the authoritative list
of checks covered by CI. Run the applicable checks immediately before opening a
pull request and report each result accurately. The
[verification guide](docs/ai_agents/verification.md) owns the private-fixture and private-conformance procedures and the limits of what each track proves.
Do not claim private-fixture or retail validation unless that track ran.

## Pull requests

Keep commits reviewable and commit subjects to one line of 50 characters or
fewer. A pull request should explain the behavior or contract changed, cite
the evidence behind format claims, and name the verification performed.

Review feedback should land as follow-up commits after review begins so that
comments remain attached to the diff. Do not force-push during review.

## Issues and community

Use the repository issue tracker for reproducible bugs, format evidence, and
proposals that need a durable record. Include the smallest useful reproduction and distinguish observed facts from inference.

Send suspected security problems to the maintainers privately. Do not include
client data, credentials, or private fixture contents in an issue.
