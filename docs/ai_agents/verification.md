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

`validate_repo.py` checks only the tracked public boundary. Private bytes stay
in their owner roots outside this checkout; there is no ignored in-tree mirror.
Use the private-conformance command below to validate the manifest identities
against those explicit roots. The canonical root-resolution rules are in
[conformance tests](../conformance-tests.md#fixture-roots).

## Private conformance

Supply every required owner root explicitly and reject private skips:

```powershell
cargo run --locked -p xivl-conformance --bin conformance -- run --fixture-root client-install=C:\path\to\client --fixture-root user-config=C:\path\to\config --require-private
```

Exit 0 means every selected private case ran without a missing-root skip and
matched its declared size, SHA-256, and normalized expectation. It does not
prove live client acceptance.

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
