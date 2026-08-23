#!/usr/bin/env python3
"""Remove restricted scratch and validate the one-file output envelope."""

from __future__ import annotations

import argparse
import os
import shutil
from pathlib import Path


ATTESTATION_NAME = "retail-evidence-attestation.json"


def _contained_path(path: Path, root: Path, label: str) -> Path:
    resolved_root = root.resolve(strict=True)
    resolved = path.resolve(strict=False)
    if resolved == resolved_root or resolved_root not in resolved.parents:
        raise RuntimeError(f"{label} must be a child of its runner root")
    return resolved


def finalize(scratch_root: Path, staging_root: Path, runner_temp: Path, workspace: Path) -> None:
    scratch = _contained_path(scratch_root, runner_temp, "scratch root")
    staging = _contained_path(staging_root, workspace, "staging root")
    if scratch_root.is_symlink():
        raise RuntimeError("scratch root must not be a symlink")
    if scratch_root.exists() and not scratch_root.is_dir():
        raise RuntimeError("scratch root must be a directory")
    if scratch_root.exists():
        shutil.rmtree(scratch)

    if staging_root.is_symlink() or not staging.is_dir():
        raise RuntimeError("staging root must be a real directory")
    entries = list(staging.iterdir())
    if len(entries) != 1:
        raise RuntimeError("staging root must contain exactly one entry")
    attestation = entries[0]
    if (
        attestation.name != ATTESTATION_NAME
        or attestation.is_symlink()
        or not attestation.is_file()
    ):
        raise RuntimeError("staging root must contain only the regular attestation file")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--scratch-root", type=Path, required=True)
    parser.add_argument("--staging-root", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    runner_temp = os.environ.get("RUNNER_TEMP")
    workspace = os.environ.get("GITHUB_WORKSPACE")
    if not runner_temp or not workspace:
        raise SystemExit("runner roots were not provided")
    finalize(args.scratch_root, args.staging_root, Path(runner_temp), Path(workspace))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
