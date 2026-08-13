# Verification

`.github/workflows/checks.yml` is the authoritative list of CI-covered checks, and CI runs them on every pull request and push to `main`. The
[policy index](README.md) defines the surrounding contribution doctrine.

## Language and parser posture

Rust is the workspace language. The dominant task is parsing hostile binary
input from a frozen format, so memory safety by construction and first-class
fuzzing serve the no-panic requirement directly. Format overlap with another
repository is deliberate duplication, not a reason to add a cross-repository
dependency.

## Private fixture validation

Public CI declares `XIVL_TOOLS_PRIVATE_FIXTURES_ABSENT=1`. An owner with the
ignored fixture tree restored must leave that variable unset and run:

```powershell
Remove-Item Env:XIVL_TOOLS_PRIVATE_FIXTURES_ABSENT -ErrorAction SilentlyContinue
python tools/validate_repo.py
```

Exit 0 proves `tests/fixtures/private/<root-id>/<sourcePath>` contains exactly
the manifest entries with the declared sizes and hashes.

## Private conformance

Supply every required owner root explicitly and reject private skips:

```powershell
cargo run --locked -p xivl-conformance --bin conformance -- run --fixture-root client-install=C:\path\to\client --fixture-root user-config=C:\path\to\config --require-private
```

Exit 0 means every selected private case ran without a missing-root skip and
matched its normalized expectation. It does not prove live client acceptance.

## Fixture reproduction

Freeze and recheck an owner fixture snapshot with:

```powershell
python tools/freeze_private_fixtures.py --root user-config --from C:\path\to\config --into C:\path\to\snapshot
python tools/freeze_private_fixtures.py --root user-config --check C:\path\to\snapshot
```

Both commands must exit 0. The snapshot remains private and must match
`tests/fixtures/private-manifest.json` exactly.

## Claim limits

A bare checkout proves only the CI-covered build, static checks, authored
public fixtures, registered tests, and public conformance cases. Report every
unverified track and do not claim private-fixture, retail-session, or live
client validation unless that track actually ran.
