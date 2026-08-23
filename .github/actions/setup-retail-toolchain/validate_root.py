#!/usr/bin/env python3
"""Validate and print a restricted toolchain scratch root."""

from __future__ import annotations

import os
from pathlib import Path


def validate_scratch_root(scratch_root: Path, runner_temp: Path) -> Path:
    runner = runner_temp.resolve(strict=True)
    scratch = scratch_root.resolve(strict=False)
    if scratch_root.is_symlink():
        raise RuntimeError("toolchain scratch root must not be a symlink")
    if scratch == runner or runner not in scratch.parents:
        raise RuntimeError("toolchain scratch root must be under RUNNER_TEMP")
    return scratch


def main() -> int:
    scratch_root = os.environ.get("RETAIL_SCRATCH_ROOT")
    runner_temp = os.environ.get("RUNNER_TEMP")
    if not scratch_root or not runner_temp:
        raise SystemExit("toolchain runner roots were not provided")
    print(validate_scratch_root(Path(scratch_root), Path(runner_temp)))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
