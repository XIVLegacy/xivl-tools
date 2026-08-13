#!/usr/bin/env python3
"""Validate the tracked public repository boundary."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
PRIVATE_MANIFEST = ROOT / "tests" / "fixtures" / "private-manifest.json"
PRIVATE_TREE = ROOT / "tests" / "fixtures" / "private"
FIXTURES_ABSENT = os.environ.get("XIVL_TOOLS_PRIVATE_FIXTURES_ABSENT") == "1"

PERMITTED_TOP_LEVEL_GROUPS = {
    "root",
    ".github",
    "apps",
    "data",
    "docs",
    "schemas",
    "src",
    "tests",
    "tools",
}
REQUIRED_AGENT_TOOLING_IGNORE_LINES = {
    "# Agent / AI tooling",
    ".claude/",
    ".agents/",
    "AGENTS.md",
    "CLAUDE.md",
    "docs/ai_agents/local/",
}
ABSOLUTE_MAINTAINER_PATH_RE = re.compile(
    rb"(?:[A-Za-z]:\\" + rb"Users\\|/" + rb"Users/|/" + rb"home/)",
    re.IGNORECASE,
)


def tracked_paths() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    return sorted(path for path in result.stdout.decode("utf-8").split("\0") if path)


def forbidden_category(path: str) -> str | None:
    lower = path.lower()
    parts = lower.split("/")
    name = parts[-1]

    if lower.startswith("tests/fixtures/private/"):
        return "private retail fixtures"
    if (
        ".claude" in parts
        or ".agents" in parts
        or name in {"agents.md", "claude.md"}
        or lower.startswith("docs/ai_agents/local/")
    ):
        return "agent material"
    if any(part in {"build", "out", "target"} for part in parts):
        return "build trees"
    if any(part in {"export", "extracted"} for part in parts):
        return "generated tool output"
    if "local" in parts or name == "cmakeuserpresets.json":
        return "machine-local settings"
    if "__pycache__" in parts or ".venv" in parts or name.endswith(".pyc"):
        return "Python caches"
    if any(part in {".vs", ".vscode", ".idea"} for part in parts):
        return "IDE state"
    return None


def private_reference_tokens() -> tuple[bytes, ...]:
    slash = b"/"
    return (
        b"docs" + slash + b"ai_agents" + slash + b"local" + slash,
        b"." + b"claude" + slash,
        b"." + b"agents" + slash,
        b"AGENTS" + b".md",
        b"CLAUDE" + b".md",
        b"must stay " + b"private",
        b"private " + b"repository",
        b"private " + b"repositories",
        b"repository is " + b"private",
    )


def check_boundary(paths: list[str], errors: list[str]) -> None:
    for path in paths:
        group = path.split("/", 1)[0] if "/" in path else "root"
        if group not in PERMITTED_TOP_LEVEL_GROUPS:
            errors.append(f"unexpected top-level tracked group: {path}")

    token_exclusions = {
        ".gitignore",
        "docs/ai_agents/README.md",
        "tools/check_contract.py",
        "tools/validate_repo.py",
    }
    tokens = private_reference_tokens()
    for path in paths:
        category = forbidden_category(path)
        if category:
            errors.append(f"forbidden {category}: {path}")
        data = (ROOT / path).read_bytes()
        if data[:2] == b"MZ":
            errors.append(f"PE MZ magic in tracked file: {path}")
        if ABSOLUTE_MAINTAINER_PATH_RE.search(data):
            errors.append(f"absolute maintainer path in tracked file: {path}")
        if path not in token_exclusions:
            lowered = data.lower()
            for token in tokens:
                if token.lower() in lowered:
                    errors.append(
                        "private-reference token "
                        f"{token.decode('ascii')} in tracked file: {path}"
                    )

    ignore_text = (
        (ROOT / ".gitignore").read_text(encoding="utf-8")
        .replace("\r\n", "\n")
    )
    ignore_lines = set(ignore_text.split("\n"))
    for required in sorted(REQUIRED_AGENT_TOOLING_IGNORE_LINES):
        if required not in ignore_lines:
            errors.append(f".gitignore missing required line: {required}")

    if "Cargo.lock" not in paths:
        errors.append("required tracked file missing: Cargo.lock")


def check_json(paths: list[str], errors: list[str]) -> int:
    count = 0
    for path in paths:
        if not path.endswith(".json"):
            continue
        count += 1
        try:
            json.loads((ROOT / path).read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as exc:
            errors.append(f"invalid tracked JSON {path}: {exc}")
    return count


def check_private_fixtures(errors: list[str]) -> int:
    manifest = json.loads(PRIVATE_MANIFEST.read_text(encoding="utf-8"))
    entries = manifest.get("entries", [])
    expected: dict[str, dict] = {}
    ids: set[str] = set()
    for entry in entries:
        fixture_id = entry.get("id")
        root_id = entry.get("root", "client-install")
        source_path = entry.get("sourcePath")
        if not isinstance(fixture_id, str) or not isinstance(source_path, str):
            continue
        if fixture_id in ids:
            errors.append(f"private fixture manifest duplicate id: {fixture_id}")
        ids.add(fixture_id)
        relative = Path(root_id, *source_path.split("/"))
        key = relative.as_posix()
        if key in expected:
            errors.append(f"private fixture manifest duplicate path: {key}")
        expected[key] = entry

    if FIXTURES_ABSENT:
        if PRIVATE_TREE.exists():
            errors.append(
                "private fixture absence declaration: found tests/fixtures/private/ "
                "while XIVL_TOOLS_PRIVATE_FIXTURES_ABSENT=1"
            )
        return len(expected)

    if not PRIVATE_TREE.is_dir():
        errors.append(
            "private fixture tree missing; supply tests/fixtures/private/<root-id>/ "
            "or set XIVL_TOOLS_PRIVATE_FIXTURES_ABSENT=1 for a public-tree-only gate"
        )
        return len(expected)

    actual = {
        path.relative_to(PRIVATE_TREE).as_posix(): path
        for path in PRIVATE_TREE.rglob("*")
        if path.is_file()
    }
    for relative in sorted(set(expected) - set(actual)):
        errors.append(f"private fixture missing: tests/fixtures/private/{relative}")
    for relative in sorted(set(actual) - set(expected)):
        errors.append(f"private fixture absent from manifest: tests/fixtures/private/{relative}")
    for relative in sorted(set(expected) & set(actual)):
        entry = expected[relative]
        data = actual[relative].read_bytes()
        if len(data) != entry.get("size"):
            errors.append(
                f"private fixture size mismatch: tests/fixtures/private/{relative}: "
                f"expected {entry.get('size')}, got {len(data)}"
            )
        digest = hashlib.sha256(data).hexdigest()
        if digest != entry.get("sha256"):
            errors.append(
                f"private fixture SHA-256 mismatch: tests/fixtures/private/{relative}: "
                f"expected {entry.get('sha256')}, got {digest}"
            )
    return len(expected)


def main() -> int:
    errors: list[str] = []
    try:
        paths = tracked_paths()
        check_boundary(paths, errors)
        json_count = check_json(paths, errors)
        fixture_count = check_private_fixtures(errors)
    except (OSError, subprocess.SubprocessError, UnicodeError, json.JSONDecodeError) as exc:
        print(f"repository boundary FAILED: {exc}", file=sys.stderr)
        return 1

    if errors:
        print(f"repository boundary FAILED ({len(errors)} problems):", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1

    fixture_mode = "manifest-only" if FIXTURES_ABSENT else "byte-verified"
    print(
        f"repository boundary OK ({len(paths)} tracked files, {json_count} JSON files, "
        f"{fixture_count} private fixture identities {fixture_mode})."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
