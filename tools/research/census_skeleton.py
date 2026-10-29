#!/usr/bin/env python3
"""Census bounded SKL framing in an explicitly supplied retail install.

Optional research only, outside CI. Prints counts, spans, and SHA-256
identities without names, transforms, or resource bytes. --xivl optionally
replays each selected resource through the CLI's explicit skeleton reader.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import os
import pathlib
import re
import stat
import struct
import subprocess
from collections.abc import Iterator


def files(root: pathlib.Path) -> Iterator[pathlib.Path]:
    stack = [root]
    while stack:
        with os.scandir(stack.pop()) as entries:
            for entry in entries:
                attributes = getattr(
                    entry.stat(follow_symlinks=False), "st_file_attributes", 0
                )
                if entry.is_symlink() or attributes & getattr(
                    stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0
                ):
                    raise ValueError("reparse point in input tree")
                if entry.is_dir(follow_symlinks=False):
                    stack.append(pathlib.Path(entry.path))
                elif entry.is_file(follow_symlinks=False):
                    yield pathlib.Path(entry.path)


def u32(data: bytes, offset: int) -> int:
    if offset < 0 or offset + 4 > len(data):
        raise ValueError("truncated dword")
    return struct.unpack_from("<I", data, offset)[0]


def block(data: bytes, base: int, entry_path: list[int]) -> dict:
    if len(data) < 80:
        raise ValueError("truncated SKL")
    count, relative, strings = (u32(data, at) for at in (0x30, 0x34, 0x38))
    start = 0x30 + 16 * (count + 1)
    length = u32(data, 0x48)
    pool = start + relative
    if count != 1 or length % 176 or relative != length or pool > len(data):
        raise ValueError("SKL array framing outside measured shape")
    end = pool
    longest = 0
    for _ in range(strings):
        nul = data.find(b"\0", end)
        if nul < 0:
            raise ValueError("unterminated counted string")
        longest = max(longest, nul - end)
        end = nul + 1
    records = length // 176
    failures: collections.Counter[str] = collections.Counter()
    for index in range(records):
        at = start + 176 * index
        for field in (0, 4, 8):
            if u32(data, at + field) >= strings:
                failures["string-index"] += 1
        for field in (0x38, 0x3C, 0x40):
            link = u32(data, at + field)
            if link != 0xFFFFFFFF and link >= records:
                failures["hierarchy-index"] += 1
        if u32(data, at + 0x44) != index:
            failures["self-index"] += 1
    return {
        "offset": base,
        "length": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "entryPath": entry_path,
        "scalar": u32(data, 8),
        "conversionByte": data[12],
        "headerSize": struct.unpack_from("<H", data, 14)[0],
        "declaredSize": u32(data, 16),
        "directoryCount": count,
        "directoryOffset": u32(data, 0x44),
        "recordCount": records,
        "stringCount": strings,
        "longestString": longest,
        "slack": len(data) - end,
        "indexFailures": dict(failures),
    }


def blocks(
    data: bytes, base: int = 0, entry_path: list[int] | None = None
) -> list[dict]:
    entry_path = [] if entry_path is None else entry_path
    if len(entry_path) > 8:
        raise ValueError("RES nesting exceeds eight")
    if data[:8] == b"SEDBSKL ":
        return [block(data, base, entry_path)]
    if data[:8] != b"SEDBRES ":
        return []
    count = u32(data, 0x30)
    directory = max(struct.unpack_from("<H", data, 14)[0], 64)
    payload = directory + 16 * count
    if count != u32(data, 0x38) or payload > len(data):
        raise ValueError("invalid RES directory")
    found = []
    for index in range(count):
        at = directory + 16 * index
        start = payload + u32(data, at + 4)
        end = min(start + u32(data, at + 8), len(data))
        if start < end:
            found.extend(blocks(data[start:end], base + start, [*entry_path, index]))
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client-root", type=pathlib.Path, required=True)
    parser.add_argument(
        "--xivl",
        type=pathlib.Path,
        help="explicit CLI executable for inspection replay",
    )
    args = parser.parse_args()
    if not all((args.client_root / tree).is_dir() for tree in ("data", "client")):
        parser.error("client root must contain data and client trees")
    results = {}
    for tree in ("data", "client"):
        resources = []
        scanned = 0
        for path in files(args.client_root / tree):
            if tree == "data" and path.suffix.upper() != ".DAT":
                continue
            scanned += 1
            with path.open("rb") as handle:
                head = handle.read(20)
                if head[:4] not in (b"SEDB", b"PWIB"):
                    continue
                handle.seek(0)
                data = handle.read()
            if head[:4] == b"PWIB":
                _, start, end = struct.unpack_from(">III", head, 4)
                found = blocks(data[start : min(end, len(data))], start)
            else:
                declared = u32(data, 16)
                end = declared if 20 <= declared <= len(data) else len(data)
                found = blocks(data[:end])
            if not found:
                continue
            resource = {
                "path": path.relative_to(args.client_root).as_posix(),
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
                "blocks": found,
            }
            if args.xivl:
                run = subprocess.run(
                    [str(args.xivl), "inspect", str(path), "--as", "skeleton"],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                if run.returncode == 0:
                    report = json.loads(run.stdout)
                    if len(report["skeletons"]) != len(found):
                        raise ValueError("CLI block count differs from census")
                    resource["inspection"] = {"outcome": "ok"}
                else:
                    match = re.search(r"([a-z-]+) at offset (\d+):", run.stderr)
                    if not match:
                        raise ValueError("CLI failed without a typed parse refusal")
                    resource["inspection"] = {
                        "outcome": "parse-error",
                        "kind": match[1],
                        "offset": int(match[2]),
                    }
            resources.append(resource)
        results[tree] = {
            "scanned": scanned,
            "resources": sorted(resources, key=lambda row: row["path"]),
        }
    print(json.dumps(results, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
