#!/usr/bin/env python3
"""Contract gate for xivl-tools.

Runs on Windows, Linux, and macOS from this checkout alone. It enforces the
project contract: ASCII authored text, schema-valid contract data, the
data boundary, self-containment, and the support-matrix coverage rule.
"""

from __future__ import annotations

import argparse
import json
import posixpath
import re
import subprocess
import sys
from pathlib import Path

import jsonschema

REPO_ROOT = Path(__file__).resolve().parent.parent

SUPPORT_MATRIX = "data/support-matrix.json"
PRIVATE_FIXTURE_MANIFEST = "tests/fixtures/private-manifest.json"
CASE_DIR = "tests/conformance/cases"
DOCS_INDEX = "docs/README.md"

SCHEMAS = {
    SUPPORT_MATRIX: "schemas/support-matrix.schema.json",
    PRIVATE_FIXTURE_MANIFEST: "schemas/private-fixture-manifest.schema.json",
}

# Paths whose contents must never be tracked. The policy is the rule. This
# is the mechanical backstop.
FORBIDDEN_TRACKED_PREFIXES = (
    "tests/fixtures/private/",
    "build/",
    "out/",
    "target/",
)

# A supported command must never resolve a sibling repository. These are
# path shapes, so a prose mention of a repository name stays legal.
SIBLING_PATH_PATTERN = re.compile(
    r"(\.\./|\.\.\\)(bahamut|xivl-[a-z-]+)(?![a-z-])"
)

# Files whose subject is the workspace relationship itself may name a
# sibling path shape while explaining that it is forbidden.
SIBLING_PATTERN_EXEMPT = (
    "docs/ai_agents/evidence-and-claims.md",
    "docs/source-and-data-policy.md",
)

# Authored synthetic fixtures are bytes, not text: a parser fixture that
# could not contain 0x80 and above could not exercise the parser. The
# exemption is narrow on purpose - one directory, one suffix - so ordinary
# authored files never fall through it.
BINARY_FIXTURE_PREFIX = "tests/fixtures/public/"
BINARY_FIXTURE_SUFFIX = ".bin"


def is_binary_fixture(name: str) -> bool:
    return name.startswith(BINARY_FIXTURE_PREFIX) and name.endswith(BINARY_FIXTURE_SUFFIX)


class Failure:
    def __init__(self, check: str, detail: str) -> None:
        self.check = check
        self.detail = detail

    def __str__(self) -> str:
        return "{0}: {1}".format(self.check, self.detail)


def tracked_files() -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise SystemExit("gate: git ls-files failed: {0}".format(result.stderr.strip()))
    # Validate the worktree contents, so an intentional tracked-file removal
    # can be checked before the change is staged.
    return [line for line in result.stdout.splitlines() if line and (REPO_ROOT / line).is_file()]


def load_json(relative: str) -> object:
    path = REPO_ROOT / relative
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def check_ascii(files: list[str]) -> list[Failure]:
    failures = []
    for name in files:
        if is_binary_fixture(name):
            continue
        data = (REPO_ROOT / name).read_bytes()
        for index, byte in enumerate(data):
            if byte >= 0x80:
                failures.append(
                    Failure(
                        "ascii",
                        "{0}: non-ASCII byte 0x{1:02X} at offset {2}".format(
                            name, byte, index
                        ),
                    )
                )
                break
    return failures


def check_data_boundary(files: list[str]) -> list[Failure]:
    failures = []
    for name in files:
        for prefix in FORBIDDEN_TRACKED_PREFIXES:
            if name.startswith(prefix):
                failures.append(
                    Failure("data-boundary", "{0} is tracked under {1}".format(name, prefix))
                )
    return failures


def check_self_containment(files: list[str]) -> list[Failure]:
    failures = []
    for name in files:
        if name in SIBLING_PATTERN_EXEMPT or is_binary_fixture(name):
            continue
        try:
            text = (REPO_ROOT / name).read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue  # reported by the ASCII check
        for match in SIBLING_PATH_PATTERN.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            failures.append(
                Failure(
                    "self-containment",
                    "{0}:{1}: sibling repository path '{2}'".format(
                        name, line, match.group(0)
                    ),
                )
            )
    return failures


def doc_links(relative: str, text: str) -> set[str]:
    links = set()
    for raw in re.findall(r"\]\(([^)]+)\)", text):
        target = raw.split("#", 1)[0]
        if not target or target.startswith(("http://", "https://")):
            continue
        resolved = posixpath.normpath(posixpath.join(Path(relative).parent.as_posix(), target))
        links.add(resolved)
    return links


def check_docs_index(files: list[str]) -> list[Failure]:
    failures = []
    tracked = set(files)
    public_docs = sorted(
        name
        for name in files
        if name.startswith("docs/")
        and name.endswith(".md")
        and name != DOCS_INDEX
        and not name.startswith("docs/ai_agents/local/")
    )

    if DOCS_INDEX not in tracked:
        return [Failure("docs-index", "{0} is missing".format(DOCS_INDEX))]

    index_links = doc_links(DOCS_INDEX, (REPO_ROOT / DOCS_INDEX).read_text(encoding="utf-8"))
    for name in public_docs:
        if name not in index_links:
            failures.append(
                Failure("docs-index", "{0} is not listed in {1}".format(name, DOCS_INDEX))
            )

    for name in sorted(index_links):
        if name.startswith("docs/") and name.endswith(".md") and name not in tracked:
            failures.append(
                Failure("docs-index", "{0} links to untracked document {1}".format(DOCS_INDEX, name))
            )

    for name in public_docs:
        links = doc_links(name, (REPO_ROOT / name).read_text(encoding="utf-8"))
        if name.startswith("docs/ai_agents/") and name != "docs/ai_agents/README.md":
            if "docs/ai_agents/README.md" not in links:
                failures.append(
                    Failure("docs-index", "{0} does not link to the policy index".format(name))
                )
        elif DOCS_INDEX not in links:
            failures.append(
                Failure("docs-index", "{0} does not link to {1}".format(name, DOCS_INDEX))
            )

    return failures


def validate_against(relative: str, schema_relative: str) -> list[Failure]:
    schema = load_json(schema_relative)
    document = load_json(relative)
    validator = jsonschema.Draft202012Validator(schema)
    failures = []
    for error in sorted(validator.iter_errors(document), key=lambda item: list(item.path)):
        pointer = "/" + "/".join(str(part) for part in error.path)
        failures.append(Failure("schema", "{0}{1}: {2}".format(relative, pointer, error.message)))
    return failures


def check_schemas(files: list[str]) -> list[Failure]:
    failures = []
    tracked = set(files)

    for relative, schema_relative in SCHEMAS.items():
        if relative not in tracked:
            failures.append(Failure("schema", "{0} is missing".format(relative)))
            continue
        failures.extend(validate_against(relative, schema_relative))

    for relative in sorted(name for name in tracked if name.startswith(CASE_DIR + "/")):
        if not relative.endswith("/case.json"):
            continue
        failures.extend(validate_against(relative, "schemas/conformance-case.schema.json"))

    return failures


def read_cases(files: list[str]) -> list[tuple[str, dict]]:
    cases = []
    for relative in sorted(files):
        if relative.startswith(CASE_DIR + "/") and relative.endswith("/case.json"):
            cases.append((relative, load_json(relative)))
    return cases


def check_case_integrity(files: list[str], matrix: dict) -> list[Failure]:
    failures = []
    tracked = set(files)
    format_ids = {entry["id"] for entry in matrix["formats"]}
    fixture_ids = {
        entry["id"] for entry in load_json(PRIVATE_FIXTURE_MANIFEST)["entries"]
    }
    seen_ids = set()

    for relative, case in read_cases(files):
        directory = Path(relative).parent
        case_id = case.get("id")
        if case_id != directory.name:
            failures.append(
                Failure("case", "{0}: id '{1}' does not match its directory".format(relative, case_id))
            )
        if case_id in seen_ids:
            failures.append(Failure("case", "{0}: duplicate case id '{1}'".format(relative, case_id)))
        seen_ids.add(case_id)

        if case.get("formatId") not in format_ids:
            failures.append(
                Failure(
                    "case",
                    "{0}: formatId '{1}' is not in the support matrix".format(
                        relative, case.get("formatId")
                    ),
                )
            )

        fixture = case.get("fixture", {})
        if fixture.get("kind") == "public":
            value = fixture.get("path")
            if value and value not in tracked:
                failures.append(
                    Failure("case", "{0}: fixture '{1}' is not tracked".format(relative, value))
                )
        elif fixture.get("kind") == "private":
            value = fixture.get("fixtureId")
            if value and value not in fixture_ids:
                failures.append(
                    Failure(
                        "case",
                        "{0}: private fixture '{1}' is not in the manifest".format(
                            relative, value
                        ),
                    )
                )

        expected = case.get("expect", {}).get("output")
        if expected:
            output_relative = (directory / expected).as_posix()
            if output_relative not in tracked:
                failures.append(
                    Failure("case", "{0}: expected output '{1}' is not tracked".format(relative, expected))
                )

    return failures


def check_matrix_coverage(files: list[str], matrix: dict) -> list[Failure]:
    """A format may not claim supported or verified without a public case,
    and may not claim verified without a private one. See
    docs/support-matrix.md, Promotion rules."""
    failures = []
    public_covered = set()
    private_covered = set()
    for _, case in read_cases(files):
        target = public_covered if case["fixture"]["kind"] == "public" else private_covered
        target.add(case["formatId"])

    for entry in matrix["formats"]:
        statuses = {key: entry[key] for key in ("read", "write", "export")}
        claimed = {key: value for key, value in statuses.items() if value in ("supported", "verified")}
        if not claimed:
            continue
        if entry["id"] not in public_covered:
            failures.append(
                Failure(
                    "matrix-coverage",
                    "{0} claims {1} with no public conformance case".format(
                        entry["id"], sorted(claimed)
                    ),
                )
            )
        verified = [key for key, value in statuses.items() if value == "verified"]
        if verified and entry["id"] not in private_covered:
            failures.append(
                Failure(
                    "matrix-coverage",
                    "{0} claims verified {1} with no private conformance case".format(
                        entry["id"], sorted(verified)
                    ),
                )
            )
    return failures


def print_matrix(matrix: dict) -> None:
    target = matrix["target"]
    print("target: {0} (frozen: {1})".format(target["title"], target["frozen"]))
    if "dataReference" in target:
        print("data reference: {0}".format(target["dataReference"]))
    print("")
    print("platforms:")
    for platform in matrix["platforms"]:
        print("  {0:<16} {1}".format(platform["id"], platform["tier"]))
    print("")
    header = "  {0:<20} {1:<9} {2:<7} {3:<14} {4:<14} {5}"
    print("formats:")
    print(header.format("id", "category", "phase", "read", "write", "export"))
    for entry in matrix["formats"]:
        print(
            header.format(
                entry["id"],
                entry["category"],
                entry["phase"],
                entry["read"],
                entry["write"],
                entry["export"],
            )
        )


def main() -> int:
    parser = argparse.ArgumentParser(description="xivl-tools contract gate")
    parser.add_argument(
        "--print-matrix",
        action="store_true",
        help="print the support matrix and exit without running the gate",
    )
    args = parser.parse_args()

    if args.print_matrix:
        print_matrix(load_json(SUPPORT_MATRIX))
        return 0

    files = tracked_files()
    if not files:
        print("gate: no tracked files")
        return 1

    failures = []
    failures.extend(check_ascii(files))
    failures.extend(check_data_boundary(files))
    failures.extend(check_self_containment(files))
    failures.extend(check_docs_index(files))
    schema_failures = check_schemas(files)
    failures.extend(schema_failures)

    # The matrix checks read fields the schema has not yet vouched for, so
    # they only run once the schema pass is clean.
    if not schema_failures:
        matrix = load_json(SUPPORT_MATRIX)
        failures.extend(check_case_integrity(files, matrix))
        failures.extend(check_matrix_coverage(files, matrix))

    if failures:
        for failure in failures:
            print(failure)
        print("gate: FAILED ({0} finding(s))".format(len(failures)))
        return 1

    cases = len(read_cases(files))
    print(
        "gate: OK ({0} tracked files, {1} contract data files, {2} conformance case(s))".format(
            len(files), len(SCHEMAS), cases
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
