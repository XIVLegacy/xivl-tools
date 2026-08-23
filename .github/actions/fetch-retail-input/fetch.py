#!/usr/bin/env python3
"""Fetch one immutable, content-pinned retail input."""

from __future__ import annotations

import argparse
import base64
import binascii
import hashlib
import json
import os
import re
import tempfile
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path, PurePosixPath
from typing import Any


REPOSITORY = "XIVLegacy/xivl-private-assets"
API_ROOT = f"https://api.github.com/repos/{REPOSITORY}"
HEX40_RE = re.compile(r"[0-9a-f]{40}")
HEX64_RE = re.compile(r"[0-9a-f]{64}")
METADATA_LIMIT = 30_000_000
RequestJson = Callable[[str, int], dict[str, Any]]


class RetailInputError(RuntimeError):
    """A safe, token-free retail-input validation failure."""


def _require_hex(value: object, pattern: re.Pattern[str], label: str) -> str:
    if not isinstance(value, str) or pattern.fullmatch(value) is None:
        raise RetailInputError(f"{label} identity failed")
    return value


def _validate_path(value: str, label: str) -> str:
    candidate = PurePosixPath(value)
    if (
        not value
        or value.startswith("/")
        or "\\" in value
        or candidate.as_posix() != value
        or any(part in {"", ".", ".."} for part in candidate.parts)
    ):
        raise RetailInputError(f"{label} path failed")
    return value


def _read_json_response(response: Any, limit: int) -> dict[str, Any]:
    chunks: list[bytes] = []
    total = 0
    while True:
        chunk = response.read(min(65536, limit + 1 - total))
        if not chunk:
            break
        chunks.append(chunk)
        total += len(chunk)
        if total > limit:
            raise RetailInputError("retail-input API response exceeded its limit")
    try:
        document = json.loads(b"".join(chunks).decode("utf-8"))
    except (UnicodeError, json.JSONDecodeError) as exc:
        raise RetailInputError("retail-input API response was not valid JSON") from exc
    if not isinstance(document, dict):
        raise RetailInputError("retail-input API response shape failed")
    return document


def github_request(token: str, endpoint: str, limit: int) -> dict[str, Any]:
    request = urllib.request.Request(
        f"{API_ROOT}/{endpoint}",
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "User-Agent": "XIVLegacy-retail-input-validation",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=180) as response:
            status = getattr(response, "status", 200)
            if status < 200 or status >= 300:
                raise RetailInputError("retail-input API request failed")
            return _read_json_response(response, limit)
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as exc:
        raise RetailInputError("retail-input API request failed") from exc


def _parse_parent_trees(raw: str) -> list[str]:
    try:
        document = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise RetailInputError("parent tree declaration was not valid JSON") from exc
    if not isinstance(document, list) or not all(
        isinstance(item, str) for item in document
    ):
        raise RetailInputError("parent tree declaration shape failed")
    paths = [_validate_path(item, "parent tree") for item in document]
    if len(set(paths)) != len(paths):
        raise RetailInputError("parent tree declaration contained duplicates")
    return paths


def _validate_output(output: Path, runner_temp: Path) -> Path:
    root = runner_temp.resolve(strict=True)
    resolved = output.resolve(strict=False)
    if resolved == root or root not in resolved.parents:
        raise RetailInputError("retail-input output must be under RUNNER_TEMP")
    return resolved


def fetch_retail_input(
    request_json: RequestJson,
    *,
    commit: str,
    path: str,
    size: int,
    sha256: str,
    output: Path,
    runner_temp: Path,
    parent_trees: list[str],
) -> None:
    commit = _require_hex(commit, HEX40_RE, "retail-input commit")
    path = _validate_path(path, "retail-input")
    sha256 = _require_hex(sha256, HEX64_RE, "retail-input SHA-256")
    if size < 0:
        raise RetailInputError("retail-input size failed")
    output = _validate_output(output, runner_temp)
    for parent in parent_trees:
        _validate_path(parent, "parent tree")

    commit_doc = request_json(f"git/commits/{commit}", METADATA_LIMIT)
    compare_doc = request_json(f"compare/{commit}...main", METADATA_LIMIT)
    if commit_doc.get("sha") != commit:
        raise RetailInputError("retail-input commit identity failed")
    tree_sha = _require_hex(
        commit_doc.get("tree", {}).get("sha")
        if isinstance(commit_doc.get("tree"), dict)
        else None,
        HEX40_RE,
        "retail-input tree",
    )
    if compare_doc.get("status") not in {"ahead", "identical"}:
        raise RetailInputError("retail-input commit reachability failed")
    merge_base = compare_doc.get("merge_base_commit")
    if not isinstance(merge_base, dict) or merge_base.get("sha") != commit:
        raise RetailInputError("retail-input commit reachability failed")

    tree_doc = request_json(f"git/trees/{tree_sha}?recursive=1", METADATA_LIMIT)
    if tree_doc.get("sha") != tree_sha or tree_doc.get("truncated") is not False:
        raise RetailInputError("retail-input tree identity failed")
    entries = tree_doc.get("tree")
    if not isinstance(entries, list):
        raise RetailInputError("retail-input tree shape failed")
    by_path: dict[str, dict[str, Any]] = {}
    for entry in entries:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
            raise RetailInputError("retail-input tree shape failed")
        entry_path = entry["path"]
        if entry_path in by_path:
            raise RetailInputError("retail-input tree contained duplicate paths")
        by_path[entry_path] = entry

    entry = by_path.get(path)
    if (
        not isinstance(entry, dict)
        or entry.get("type") != "blob"
        or entry.get("mode") != "100644"
        or entry.get("size") != size
    ):
        raise RetailInputError("retail-input tree entry failed")
    blob_sha = _require_hex(entry.get("sha"), HEX40_RE, "retail-input blob")
    for parent in parent_trees:
        parent_entry = by_path.get(parent)
        if (
            not isinstance(parent_entry, dict)
            or parent_entry.get("type") != "tree"
            or parent_entry.get("mode") != "040000"
        ):
            raise RetailInputError("retail-input parent tree failed")

    encoded_limit = max(METADATA_LIMIT, ((size + 2) // 3) * 4 + 1_000_000)
    blob_doc = request_json(f"git/blobs/{blob_sha}", encoded_limit)
    if (
        blob_doc.get("sha") != blob_sha
        or blob_doc.get("size") != size
        or blob_doc.get("encoding") != "base64"
        or not isinstance(blob_doc.get("content"), str)
    ):
        raise RetailInputError("retail-input blob identity failed")
    try:
        data = base64.b64decode(
            "".join(blob_doc["content"].split()), validate=True
        )
    except (ValueError, binascii.Error) as exc:
        raise RetailInputError("retail-input blob encoding failed") from exc
    if len(data) != size:
        raise RetailInputError("retail-input decoded size failed")
    git_digest = hashlib.sha1(
        f"blob {len(data)}\0".encode("ascii") + data
    ).hexdigest()
    if git_digest != blob_sha:
        raise RetailInputError("retail-input Git blob identity failed")
    if hashlib.sha256(data).hexdigest() != sha256:
        raise RetailInputError("retail-input SHA-256 failed")

    output.parent.mkdir(parents=True, exist_ok=True)
    output.unlink(missing_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        dir=output.parent, prefix=f".{output.name}.", suffix=".tmp"
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--commit", required=True)
    parser.add_argument("--path", required=True)
    parser.add_argument("--size", type=int, required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--parent-trees", default="[]")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    token = os.environ.get("RETAIL_INPUTS_TOKEN", "")
    runner_temp = os.environ.get("RUNNER_TEMP", "")
    if not token:
        raise RetailInputError("retail-input token was not provided")
    if not runner_temp:
        raise RetailInputError("RUNNER_TEMP was not provided")
    os.umask(0o077)
    parents = _parse_parent_trees(args.parent_trees)
    fetch_retail_input(
        lambda endpoint, limit: github_request(token, endpoint, limit),
        commit=args.commit,
        path=args.path,
        size=args.size,
        sha256=args.sha256,
        output=args.output,
        runner_temp=Path(runner_temp),
        parent_trees=parents,
    )
    print(f"Validated retail input: {args.path}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except RetailInputError as exc:
        raise SystemExit(str(exc)) from exc
