#!/usr/bin/env python3
"""Credential-free tests for the shared retail workflow actions."""

from __future__ import annotations

import base64
import hashlib
import importlib.util
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path
from types import ModuleType
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
FETCH_PATH = ROOT / ".github" / "actions" / "fetch-retail-input" / "fetch.py"
FINALIZE_PATH = (
    ROOT / ".github" / "actions" / "finalize-retail-attestation" / "finalize.py"
)
TOOLCHAIN_ROOT_PATH = (
    ROOT / ".github" / "actions" / "setup-retail-toolchain" / "validate_root.py"
)


def load_module(name: str, path: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


fetch = load_module("retail_fetch_action", FETCH_PATH)
finalize = load_module("retail_finalize_action", FINALIZE_PATH)
toolchain_root = load_module("retail_toolchain_root", TOOLCHAIN_ROOT_PATH)


class FetchActionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.data = b"bounded retail input"
        self.commit = "c" * 40
        self.tree_sha = "d" * 40
        self.blob_sha = hashlib.sha1(
            f"blob {len(self.data)}\0".encode("ascii") + self.data
        ).hexdigest()
        self.sha256 = hashlib.sha256(self.data).hexdigest()
        self.documents: dict[str, dict[str, Any]] = {
            f"git/commits/{self.commit}": {
                "sha": self.commit,
                "tree": {"sha": self.tree_sha},
            },
            f"compare/{self.commit}...main": {
                "status": "ahead",
                "merge_base_commit": {"sha": self.commit},
            },
            f"git/trees/{self.tree_sha}?recursive=1": {
                "sha": self.tree_sha,
                "truncated": False,
                "tree": [
                    {"path": "lane", "type": "tree", "mode": "040000"},
                    {
                        "path": "lane/input.bin",
                        "type": "blob",
                        "mode": "100644",
                        "size": len(self.data),
                        "sha": self.blob_sha,
                    },
                ],
            },
            f"git/blobs/{self.blob_sha}": {
                "sha": self.blob_sha,
                "size": len(self.data),
                "encoding": "base64",
                "content": base64.b64encode(self.data).decode("ascii"),
            },
        }

    def run_fetch(
        self,
        documents: dict[str, dict[str, Any]],
        output: Path,
        runner_temp: Path,
    ) -> None:
        def request(endpoint: str, _limit: int) -> dict[str, Any]:
            return deepcopy(documents[endpoint])

        fetch.fetch_retail_input(
            request,
            commit=self.commit,
            path="lane/input.bin",
            size=len(self.data),
            sha256=self.sha256,
            output=output,
            runner_temp=runner_temp,
            parent_trees=["lane"],
        )

    def test_fetches_only_after_every_identity_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "input.bin"
            self.run_fetch(self.documents, output, Path(temp))
            self.assertEqual(output.read_bytes(), self.data)

    def test_rejects_tree_and_content_drift_without_output(self) -> None:
        mutations = []
        truncated = deepcopy(self.documents)
        truncated[f"git/trees/{self.tree_sha}?recursive=1"]["truncated"] = True
        mutations.append(truncated)
        wrong_size = deepcopy(self.documents)
        wrong_size[f"git/blobs/{self.blob_sha}"]["size"] += 1
        mutations.append(wrong_size)
        wrong_content = deepcopy(self.documents)
        wrong_content[f"git/blobs/{self.blob_sha}"]["content"] = base64.b64encode(
            b"changed retail input"
        ).decode("ascii")
        mutations.append(wrong_content)

        for documents in mutations:
            with self.subTest(documents=documents), tempfile.TemporaryDirectory() as temp:
                output = Path(temp) / "input.bin"
                with self.assertRaises(fetch.RetailInputError):
                    self.run_fetch(documents, output, Path(temp))
                self.assertFalse(output.exists())

    def test_rejects_duplicate_tree_paths(self) -> None:
        documents = deepcopy(self.documents)
        tree = documents[f"git/trees/{self.tree_sha}?recursive=1"]["tree"]
        tree.append(deepcopy(tree[-1]))
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(fetch.RetailInputError):
                self.run_fetch(documents, Path(temp) / "input.bin", Path(temp))

    def test_rejects_noncanonical_paths(self) -> None:
        with self.assertRaises(fetch.RetailInputError):
            fetch.fetch_retail_input(
                lambda _endpoint, _limit: {},
                commit=self.commit,
                path="../input.bin",
                size=len(self.data),
                sha256=self.sha256,
                output=Path.cwd() / "input.bin",
                runner_temp=Path.cwd(),
                parent_trees=[],
            )

    def test_rejects_output_outside_runner_temp(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            runner = Path(temp) / "runner"
            outside = Path(temp) / "outside.bin"
            runner.mkdir()
            with self.assertRaises(fetch.RetailInputError):
                self.run_fetch(self.documents, outside, runner)


class FinalizeActionTests(unittest.TestCase):
    def test_removes_scratch_and_accepts_one_regular_attestation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            runner = root / "runner"
            workspace = root / "workspace"
            scratch = runner / "retail-scratch"
            staging = workspace / "_retail-staging"
            scratch.mkdir(parents=True)
            (scratch / "restricted.bin").write_bytes(b"restricted")
            staging.mkdir(parents=True)
            (staging / finalize.ATTESTATION_NAME).write_text("{}", encoding="ascii")

            finalize.finalize(scratch, staging, runner, workspace)

            self.assertFalse(scratch.exists())
            self.assertEqual(
                [path.name for path in staging.iterdir()],
                [finalize.ATTESTATION_NAME],
            )

    def test_rejects_an_extra_retained_file_after_cleanup(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            runner = root / "runner"
            workspace = root / "workspace"
            scratch = runner / "retail-scratch"
            staging = workspace / "_retail-staging"
            scratch.mkdir(parents=True)
            staging.mkdir(parents=True)
            (staging / finalize.ATTESTATION_NAME).write_text("{}", encoding="ascii")
            (staging / "extra.bin").write_bytes(b"unexpected")

            with self.assertRaises(RuntimeError):
                finalize.finalize(scratch, staging, runner, workspace)
            self.assertFalse(scratch.exists())

    def test_rejects_a_scratch_root_outside_runner_temp(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            runner = root / "runner"
            workspace = root / "workspace"
            staging = workspace / "_retail-staging"
            outside = root / "outside"
            runner.mkdir()
            staging.mkdir(parents=True)
            outside.mkdir()
            (staging / finalize.ATTESTATION_NAME).write_text("{}", encoding="ascii")

            with self.assertRaises(RuntimeError):
                finalize.finalize(outside, staging, runner, workspace)
            self.assertTrue(outside.exists())


class ToolchainRootTests(unittest.TestCase):
    def test_accepts_only_a_runner_temp_child(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            runner = root / "runner"
            runner.mkdir()
            scratch = runner / "toolchain-scratch"
            self.assertEqual(
                toolchain_root.validate_scratch_root(scratch, runner),
                scratch.resolve(),
            )
            with self.assertRaises(RuntimeError):
                toolchain_root.validate_scratch_root(root / "outside", runner)


class ActionMetadataTests(unittest.TestCase):
    def test_toolchain_pins_and_fixed_store(self) -> None:
        fetch_text = (
            ROOT / ".github" / "actions" / "fetch-retail-input" / "fetch.py"
        ).read_text(encoding="utf-8")
        toolchain_text = (
            ROOT / ".github" / "actions" / "setup-retail-toolchain" / "action.yml"
        ).read_text(encoding="utf-8")
        self.assertIn("XIVLegacy/xivl-private-assets", fetch_text)
        self.assertNotIn("RETAIL_INPUTS_REPOSITORY", fetch_text)
        self.assertIn(
            "ce79869e1307ed8ee1e2baa86a412b1eb5b75d10a01006d788a6f968bcfaee94",
            toolchain_text,
        )
        self.assertIn(
            "93a5d11a9ad510622acaaf908c556a7b9b764d338e78a7567f3689bf5081fd54",
            toolchain_text,
        )


if __name__ == "__main__":
    unittest.main()
