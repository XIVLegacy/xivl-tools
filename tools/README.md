# tools

Use these maintenance tools to run the contract gate, regenerate public
fixtures, freeze private fixture snapshots, and perform non-gating research.

## Routine maintenance

- `validate_repo.py` - the public repository boundary gate. It pins the tracked
  tree and `.gitignore`, rejects ignored or private material in the index, and
  verifies the private fixture manifest against local bytes. A public checkout
  must explicitly set `XIVL_TOOLS_PRIVATE_FIXTURES_ABSENT=1`; without that
  declaration, a missing ignored fixture tree is an error.
- `check_contract.py` - the contract gate. Must pass before every commit.
- `make_public_fixtures.py` - generator for every byte under
  `tests/fixtures/public/`. `--check` regenerates in memory and fails if a
  committed fixture differs, which is what makes the fixtures auditable.
- `freeze_private_fixtures.py` - freezes the bytes of one fixture root
  into a snapshot directory, verifying each file against
  `tests/fixtures/private-manifest.json` and refusing to copy one that
  does not match. Needed because the configuration files are live: the
  client rewrites them. Non-gating, every path explicit, no defaults.

`requirements.txt` records the Python dependencies for the gate.

## Shared library

- `blowfish.py` - the block cipher, shared by the fixture generator and
  the SQEX research command so the two cannot disagree about it. Its
  tables are computed from the hexadecimal expansion of pi.

## Evidence replay

These non-gating research commands require an explicit external root, have
no default path, make no support claim, and never run in CI. Together they
produced the tables in `docs/format-evidence.md`.

### Client install root

These commands require `--client-root`:

- `research/census_sedb.py` - census SEDB container headers.
- `research/census_sheet_stack.py` - census scrambled XML documents, the SSD
  sheet stack, and rich strings in one resource-tree walk.
- `research/census_sqwt.py` - census SQEX widget containers.

### Configuration root

- `research/census_config.py` - census the configuration files. It requires
  `--config-root`; optional `--client-root` enables the executable stamp pass.

`oracles/` is created only when a tracked oracle record names an in-house
adapter that converts external output into this project's normalized JSON. A
checkout with no oracle records needs no adapter directory.

The [checks workflow](../.github/workflows/checks.yml) is the authoritative
list of CI-covered checks. The
[verification guide](../docs/ai_agents/verification.md) owns the owner-only
private-fixture and private-conformance procedures and the limits of what each
result proves.
