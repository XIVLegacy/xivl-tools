#!/usr/bin/env python3
"""Freeze private fixture bytes into a snapshot directory, and check one.

Some private fixtures are live files. The client rewrites its
configuration every time a setting changes, so a fixture root pointed at
the directory the client actually uses stops matching the manifest the
moment the owner plays. A conformance case then fails for a reason that
has nothing to do with this project's code.

The fix is a snapshot: point the root at a copy frozen at the hashes the
manifest records, and let the live files move.

    python tools/freeze_private_fixtures.py --root user-config \\
        --from <live dir> --into <snapshot dir>
    python tools/freeze_private_fixtures.py --root user-config \\
        --check <snapshot dir>

Every path is supplied by the caller. There is no default source, no
default destination, no client-install search, and no workspace-relative
fallback; this script is not part of the gate and never runs in CI.

A source file whose sha256 does not match the manifest is reported and
not copied. That is not an obstacle, it is the point: the manifest pins a
specific file, and replacing it silently would leave the case passing
against different bytes than the claim was established from. Re-establish
deliberately instead:

1. copy the changed file into the snapshot by hand;
2. update its sha256 and size in tests/fixtures/private-manifest.json;
3. rerun the affected cases with --update-expected and read the diff.

The snapshot belongs outside this checkout. Anything under
tests/fixtures/private/ is untracked, but it is also what `git clean`
deletes, and a snapshot that a clean can remove is not frozen.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import sys

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
PRIVATE_MANIFEST = "tests/fixtures/private-manifest.json"
DEFAULT_ROOT = "client-install"


def entries_for(root_id: str) -> list[dict]:
    document = json.loads((REPO_ROOT / PRIVATE_MANIFEST).read_text(encoding="ascii"))
    found = [
        entry
        for entry in document["entries"]
        if entry.get("root", DEFAULT_ROOT) == root_id
    ]
    if not found:
        raise SystemExit(
            "freeze: no manifest entry names the root '{0}'".format(root_id)
        )
    return sorted(found, key=lambda entry: entry["id"])


def digest(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def report(entry: dict, state: str, detail: str = "") -> None:
    line = "  {0:<28} {1:<10} {2}".format(entry["id"], state, entry["sourcePath"])
    print(line.rstrip() + (" - " + detail if detail else ""))


def verify(entry: dict, path: pathlib.Path) -> str | None:
    """The reason this file is not the manifest's file, or None."""
    if not path.is_file():
        return "absent"
    size = path.stat().st_size
    if size != entry["size"]:
        return "{0} bytes, the manifest records {1}".format(size, entry["size"])
    found = digest(path)
    if found != entry["sha256"]:
        return "hashes to {0}, the manifest records {1}".format(
            found[:16], entry["sha256"][:16]
        )
    return None


def require_external(path: pathlib.Path, label: str) -> None:
    repository = REPO_ROOT.resolve()
    resolved = path.resolve()
    if resolved == repository or repository in resolved.parents:
        raise SystemExit("freeze: {0} must be outside this checkout".format(label))


def fixture_path(root: pathlib.Path, source_path: str) -> pathlib.Path:
    relative = pathlib.PurePosixPath(source_path)
    if (
        not source_path
        or source_path.startswith("/")
        or "\\" in source_path
        or relative.as_posix() != source_path
        or any(part in {"", ".", ".."} for part in relative.parts)
    ):
        raise SystemExit("freeze: non-canonical fixture path: {0}".format(source_path))
    resolved_root = root.resolve()
    resolved = (resolved_root / pathlib.Path(*relative.parts)).resolve(strict=False)
    if resolved_root not in resolved.parents:
        raise SystemExit("freeze: fixture path escapes its root: {0}".format(source_path))
    return resolved


def freeze(root_id: str, source: pathlib.Path, destination: pathlib.Path) -> int:
    entries = entries_for(root_id)
    print(
        "freezing {0} fixture(s) of root '{1}'\n  from {2}\n  into {3}".format(
            len(entries), root_id, source, destination
        )
    )
    stale = 0
    for entry in entries:
        origin = fixture_path(source, entry["sourcePath"])
        reason = verify(entry, origin)
        if reason is not None:
            report(entry, "SKIPPED", reason)
            stale += 1
            continue
        target = fixture_path(destination, entry["sourcePath"])
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(origin, target)
        reason = verify(entry, target)
        if reason is not None:
            report(entry, "MISMATCH", "copied file " + reason)
            stale += 1
            continue
        report(entry, "frozen")
    if stale:
        print(
            "freeze: {0} of {1} fixture(s) are not the manifest's bytes and were "
            "not copied. See this script's header for the re-establish steps.".format(
                stale, len(entries)
            )
        )
        return 1
    print("freeze: OK ({0} fixture(s) frozen)".format(len(entries)))
    return 0


def check(root_id: str, snapshot: pathlib.Path) -> int:
    entries = entries_for(root_id)
    print(
        "checking {0} fixture(s) of root '{1}' under {2}".format(
            len(entries), root_id, snapshot
        )
    )
    bad = 0
    for entry in entries:
        reason = verify(entry, fixture_path(snapshot, entry["sourcePath"]))
        if reason is None:
            report(entry, "ok")
        else:
            report(entry, "MISMATCH", reason)
            bad += 1
    if bad:
        print("freeze: FAILED ({0} finding(s))".format(bad))
        return 1
    print("freeze: OK ({0} fixture(s) match the manifest)".format(len(entries)))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--root",
        required=True,
        help="fixture root id from the manifest, for example user-config",
    )
    parser.add_argument(
        "--from",
        dest="source",
        type=pathlib.Path,
        help="directory the live files are in",
    )
    parser.add_argument(
        "--into",
        dest="destination",
        type=pathlib.Path,
        help="snapshot directory to write, created if absent",
    )
    parser.add_argument(
        "--check",
        dest="snapshot",
        type=pathlib.Path,
        help="verify an existing snapshot against the manifest and copy nothing",
    )
    args = parser.parse_args()

    if args.snapshot is not None:
        if args.source is not None or args.destination is not None:
            raise SystemExit("freeze: --check takes neither --from nor --into")
        require_external(args.snapshot, "snapshot")
        return check(args.root, args.snapshot)
    if args.source is None or args.destination is None:
        raise SystemExit("freeze: freezing needs both --from and --into")
    if not args.source.is_dir():
        raise SystemExit("freeze: {0} is not a directory".format(args.source))
    require_external(args.source, "source")
    require_external(args.destination, "destination")
    return freeze(args.root, args.source, args.destination)


if __name__ == "__main__":
    sys.exit(main())
