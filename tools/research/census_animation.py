#!/usr/bin/env python3
"""Census animation framing in an explicitly supplied retail install.

Research only, never run in CI. Reads data and client resource trees without
writing client bytes. Reports resource identities, counts and SHA-256, without
decoded names or motion values. Optional --cli inspects every candidate.
"""

from __future__ import annotations

import argparse
from collections import Counter
from collections.abc import Iterator
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys

ANIMATION_TAGS = (b"mtb\0", b"MTB ", b"MCB\0", b"MCB ", b"SCB\0", b"SCB ")


def resources(root: Path) -> Iterator[Path]:
    if root.is_symlink() or getattr(root.lstat(), "st_file_attributes", 0) & 0x400:
        raise ValueError("resource root is a reparse point")
    stack = [root]
    while stack:
        with os.scandir(stack.pop()) as entries:
            for entry in entries:
                if entry.is_symlink():
                    raise ValueError("resource tree contains a symbolic link")
                if entry.is_dir(follow_symlinks=False):
                    attributes = getattr(
                        entry.stat(follow_symlinks=False), "st_file_attributes", 0
                    )
                    if attributes & 0x400:
                        raise ValueError("resource tree contains a reparse point")
                    stack.append(Path(entry.path))
                elif entry.is_file(follow_symlinks=False):
                    yield Path(entry.path)


def u32(data: bytes, at: int) -> int:
    if at < 0 or at + 4 > len(data):
        raise ValueError(f"truncated u32 at {at}")
    return struct.unpack_from("<I", data, at)[0]


def mtb(data: bytes) -> dict:
    if len(data) < 64:
        raise ValueError("truncated MTB header")
    version = u32(data, 8)
    at = struct.unpack_from("<H", data, 14)[0]
    tables = []
    for _ in range(3):
        count = u32(data, at)
        if count > (len(data) - at - 4) // 4:
            raise ValueError(f"table exceeds input at {at}")
        tables.append([u32(data, at + 4 + 4 * i) for i in range(count)])
        at += 4 + 4 * count
    names = []
    for offset in tables[1] + tables[2]:
        if offset < at or offset >= len(data) or data.find(b"\0", offset) < 0:
            raise ValueError("name outside input or unterminated")
        names.append(data[offset : data.index(b"\0", offset)])
    prefix = 16 if version >= 0x42 else 4
    section_kinds = Counter()
    for offset in tables[0]:
        if offset < at or offset + prefix > len(data):
            raise ValueError("section outside input")
        name_index = u32(data, offset)
        if name_index >= len(tables[1]):
            raise ValueError("section name index outside table")
        name = names[name_index]
        kind = (
            name.decode("ascii")
            if name in (b"Header", b"Curve", b"SpuBinary")
            else "opaque"
        )
        section_kinds[kind] += 1
    return {
        "version": version,
        "headerSize": struct.unpack_from("<H", data, 14)[0],
        "declaredSize": u32(data, 16),
        "totalProperties": struct.unpack_from("<H", data, 48)[0],
        "tables": [len(table) for table in tables],
        "sectionKinds": dict(section_kinds),
    }


def census(data: bytes) -> dict:
    first, second, total = 0, len(data), len(data)
    if data.startswith(b"PWIB"):
        total, first, second = struct.unpack_from(">III", data, 4)
        if not 16 <= first <= second <= total <= len(data):
            raise ValueError("PWIB extent outside input")
    segment = data[first:second]
    if segment[:4] != b"SEDB":
        raise ValueError("no first SEDB")
    children = [segment]
    if segment[4:8] == b"RES ":
        count = u32(segment, 48)
        if count != u32(segment, 56):
            raise ValueError("RES count mismatch")
        directory = max(struct.unpack_from("<H", segment, 14)[0], 64)
        base = directory + 16 * count
        children = []
        unavailable = 0
        unavailable_animation = 0
        opaque = 0
        for i in range(count):
            offset, size = struct.unpack_from("<II", segment, directory + 16 * i + 4)
            start = base + offset
            selected = (
                segment[start : start + 4] == b"SEDB"
                and segment[start + 4 : start + 8] in ANIMATION_TAGS
            )
            if not selected:
                opaque += 1
            if base + offset + size > len(segment):
                unavailable += 1
                unavailable_animation += selected
                continue
            if selected:
                children.append(segment[start : start + size])
    else:
        unavailable = 0
        unavailable_animation = 0
        opaque = 0
    tags = Counter()
    motions = []
    for child in children:
        if child[:4] == b"SEDB" and child[4:8] in ANIMATION_TAGS:
            tags[child[4:8].rstrip(b"\0 ").decode("ascii")] += 1
            if child[4:8] in ANIMATION_TAGS[:2]:
                motions.append(mtb(child))
    if tags or unavailable_animation:
        if segment[4:8] == b"RES ":
            kind = "animation-bank"
        elif segment[4:8] in ANIMATION_TAGS[:2]:
            kind = "standalone-mtb"
        else:
            kind = "standalone-mcb-scb"
    else:
        kind = "other-pwib" if data.startswith(b"PWIB") else "other-signature-hit"
    return {
        "candidateKind": kind,
        "tags": dict(tags),
        "mtb": motions,
        "opaqueEntries": opaque,
        "unavailableEntries": unavailable,
        "unavailableAnimationEntries": unavailable_animation,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client-root", type=Path, required=True)
    parser.add_argument("--cli", type=Path)
    parser.add_argument("--tree", choices=("all", "data", "client"), default="all")
    args = parser.parse_args()
    trees = ("data", "client") if args.tree == "all" else (args.tree,)
    counts = Counter()
    records = []
    paths = (
        (path, tree)
        for tree in trees
        for path in resources(args.client_root / tree)
        if tree != "data" or path.suffix == ".DAT"
    )
    for path, tree in paths:
        relative = path.relative_to(args.client_root).as_posix()
        bank_path = tree == "client" and (
            (relative.startswith("client/chara/") and "/act/" in relative)
            or relative.startswith(("client/vfx/", "client/cut/"))
        )
        counter = "datFiles" if tree == "data" else "clientFiles"
        counts[counter] += 1
        if counts[counter] % 10000 == 0:
            print(f"census: {counts[counter]} {tree} files", file=sys.stderr)
        with path.open("rb") as handle:
            head = handle.read(20)
            if not (
                bank_path
                or head[:4] == b"PWIB"
                or (head[:4] == b"SEDB" and head[4:8] in (*ANIMATION_TAGS, b"RES "))
            ):
                continue
            handle.seek(0)
            data = handle.read()
        markers = Counter()
        for tag in ANIMATION_TAGS:
            occurrences = data.count(b"SEDB" + tag)
            if occurrences:
                markers[tag.rstrip(b"\0 ").decode("ascii")] += occurrences
        if not markers and not (head[:4] == b"PWIB" and bank_path):
            continue
        counts["animationResources"] += 1
        if counts["animationResources"] % 1000 == 0:
            print(
                f"census: {counts['animationResources']} animation candidates",
                file=sys.stderr,
            )
        identity = relative
        if tree == "data":
            identity = "0x" + "".join(
                path.relative_to(args.client_root / tree).parts
            ).replace(".DAT", "")
        record = {
            "resource": identity,
            "length": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
            "markers": dict(markers),
        }
        try:
            record["structure"] = census(data)
            counts["censusAccepted"] += 1
            record["candidateKind"] = record["structure"]["candidateKind"]
        except (ValueError, struct.error) as error:
            record["censusRefusal"] = str(error)
            counts["censusRefused"] += 1
            record["candidateKind"] = (
                "unvalidated-pwib"
                if data.startswith(b"PWIB")
                else "unframed-signature-hit"
            )
        counts[record["candidateKind"]] += 1
        if args.cli:
            run = subprocess.run(
                [str(args.cli.resolve()), "inspect", str(path), "--as", "animation"],
                capture_output=True,
                text=True,
                check=False,
            )
            if run.returncode:
                counts["cliRefused"] += 1
                record["cliRefusal"] = run.stderr.strip().replace(
                    str(path), "<resource>"
                )
            else:
                counts["cliAccepted"] += 1
                document = json.loads(run.stdout)
                if document["sha256"] != record["sha256"]:
                    raise ValueError("input identity changed during inspection")
                record["cliMtb"] = sum(
                    entry["motion"] is not None for entry in document["entries"]
                )
                counts["cliMtb"] += record["cliMtb"]
        records.append(record)
    records.sort(key=lambda row: row["resource"])
    identities = "".join(
        f"{row['resource']} {row['length']} {row['sha256']}\n" for row in records
    )
    print(
        json.dumps(
            {
                "counts": dict(counts),
                "identitySha256": hashlib.sha256(
                    identities.encode("ascii")
                ).hexdigest(),
                "resources": records,
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
