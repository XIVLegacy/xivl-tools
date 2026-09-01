# tools

Use these maintenance tools to run the contract checks, regenerate public
fixtures, freeze private fixture snapshots, and perform optional research.

## Routine maintenance

- `validate_repo.py` - the public repository boundary check. It pins the tracked
  tree and `.gitignore` and rejects ignored or private material in the index. It
  also rejects the retired in-tree private mirror. Private bytes are validated
  only from explicit external roots by the conformance runner and snapshot tool.
- `check_contract.py` - the contract check. It must pass before every commit.
- `make_public_fixtures.py` - generator for every byte under
  `tests/fixtures/public/`. `--check` regenerates in memory and fails if a
  committed fixture differs, which is what makes the fixtures auditable.
- `freeze_private_fixtures.py` - freezes the bytes of one fixture root
  into a snapshot directory, verifying each file against
  `tests/fixtures/private-manifest.json` and refusing to copy one that
  does not match. This is needed because the client rewrites the live
  configuration files. Every path is explicit and has no default.

`requirements.txt` records the Python dependencies for validation.

## Shared retail workflow actions

The composite actions under `.github/actions/` serve only the six approved
manual retail-evidence workflows. Consumers pin a full `xivl-tools` commit and
keep their asset declaration, claim verifier, environment, and token locally.

- `fetch-retail-input` fixes the input store and validates commit reachability,
  the complete tree response, blob metadata, Git identity, decoded size, and
  SHA-256 before writing the requested file.
- `setup-retail-toolchain` installs the fixed Temurin JDK and optional Ghidra
  release after verifying both download hashes.
- `finalize-retail-attestation` removes runner scratch before accepting exactly
  one regular sanitized-attestation file. Each consumer still validates its
  own schema before upload.

`test_retail_actions.py` is the credential-free mutation suite for these
shared boundaries. `xivl-tools` never receives a retail-input credential.

## Shared library

- `blowfish.py` - the block cipher shared by the fixture generator and the SQEX
  research command, so the two cannot disagree about it. Its
  tables are computed from the hexadecimal expansion of pi.

## Evidence replay

These optional research commands require an explicit external root, have no
default path, make no support claim, and never run in CI. Together they produced
the tables indexed by `docs/format-evidence.md`.

### Client install root

These commands require `--client-root`:

- `research/census_sedb.py` - census SEDB container headers.
- `research/census_sheet_stack.py` - census scrambled XML documents, the SSD
  sheet stack, and rich strings in one resource-tree walk.
- `research/census_sqwt.py` - census SQEX widget containers.

### Lua script root

- `cargo run --locked -p xivl-formats --example lua51_census -- ...` - validate
  and census the complete manifest-owned LPB corpus. It requires explicit
  client-script and coverage-manifest paths plus the full owner commit, verifies
  every retained LPB and decoded-payload hash, and prints aggregate JSON only.
  `--check data/lua51-retail-census.json` requires exact retained parity.

### Configuration root

- `research/census_config.py` - census the configuration files. It requires
  `--config-root`; optional `--client-root` enables the executable stamp pass.
