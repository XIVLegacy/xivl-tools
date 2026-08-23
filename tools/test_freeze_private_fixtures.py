#!/usr/bin/env python3
"""Boundary tests for explicit private-fixture snapshot paths."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parent / "freeze_private_fixtures.py"
SPEC = importlib.util.spec_from_file_location("freeze_private_fixtures", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
freeze = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(freeze)


class ExternalPathTests(unittest.TestCase):
    def test_accepts_a_path_outside_the_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            freeze.require_external(Path(temp), "snapshot")

    def test_rejects_the_checkout_and_its_descendants(self) -> None:
        for path in (freeze.REPO_ROOT, freeze.REPO_ROOT / "tests" / "fixtures"):
            with self.subTest(path=path), self.assertRaises(SystemExit):
                freeze.require_external(path, "snapshot")

    def test_rejects_manifest_path_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for source_path in ("../outside.bin", "dir/../../outside.bin", "/abs.bin"):
                with self.subTest(source_path=source_path), self.assertRaises(SystemExit):
                    freeze.fixture_path(root, source_path)

    def test_resolves_a_canonical_manifest_path_under_its_root(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.assertEqual(
                freeze.fixture_path(root, "dir/input.bin"),
                (root / "dir" / "input.bin").resolve(),
            )


if __name__ == "__main__":
    unittest.main()
