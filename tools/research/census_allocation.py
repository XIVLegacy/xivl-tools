#!/usr/bin/env python3
"""Census allocation inventory in an explicit retail install, read-only.

Research only, never run in CI. Emit counts, numeric keys and SHA-256
identities without token names or client bytes. Numeric path existence does
not establish request-time formatter mode, payload validity or free IDs.
Optional JSON output must be a new file outside the client install.

    python tools/research/census_allocation.py --client-root <install>
"""

from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import struct
import sys
from typing import BinaryIO

CATALOG = Path("data/03/C0/00/00.DAT")
HEADER_SIZE = 0x40
ROW_SIZE = 0x30
RESOURCE_PATH = re.compile(
    r"([0-9a-f]{2})/([0-9a-f]{2})/([0-9a-f]{2})/([0-9a-f]{2})\.dat", re.I
)


class Malformed(ValueError):
    """A bounded catalog read failed at a file offset, without client content."""

    def __init__(self, offset: int, reason: str):
        self.offset = offset
        super().__init__(f"{reason} at file+0x{offset:X}")


def read_exact(stream: BinaryIO, size: int, offset: int, end: int) -> bytes:
    if offset < 0 or size < 0 or offset + size > end:
        raise Malformed(offset, f"span out of bounds size={size}")
    data = stream.read(size)
    if len(data) != size:
        raise Malformed(offset, f"short read size={size}")
    return data


@dataclass(frozen=True)
class Row:
    key: int
    word_04: int
    resource_key: int
    word_0c: int
    word_20: int
    token: bytes

    @property
    def effective_name(self) -> bytes:
        return self.token[:15].split(b"\0", 1)[0]

    @property
    def token_terminated(self) -> bool:
        return b"\0" in self.token


@dataclass(frozen=True)
class Root:
    row: Row
    children: tuple[Row, ...]


@dataclass(frozen=True)
class Catalog:
    size: int
    sha256: str
    header_size_word: int
    roots: tuple[Root, ...]
    rows_consumed: int
    trailing_bytes: int


def parse_row(stream: BinaryIO, offset: int, end: int) -> Row:
    data = read_exact(stream, ROW_SIZE, offset, end)
    key, word_04, resource_key, word_0c = struct.unpack_from("<4I", data)
    return Row(
        key,
        word_04,
        resource_key,
        word_0c,
        struct.unpack_from("<I", data, 0x20)[0],
        data[0x10:0x20],
    )


def parse_catalog(source: Path) -> Catalog:
    # xivl-decomp:docs/resource/region-weather-resource-rows.md, "Size and counted-row boundary".
    with source.open("rb") as stream:
        initial = os.fstat(stream.fileno())
        end = initial.st_size
        header = read_exact(stream, HEADER_SIZE, 0, end)
        if header[:18] != b"RegionResourceData":
            raise Malformed(0, "RegionResourceData signature differs")
        if header[0x18:0x1D] != b"1.1.0":
            raise Malformed(0x18, "RegionResourceData version differs")
        size_word, root_count = struct.unpack_from("<2I", header, 0x20)
        capacity = (end - HEADER_SIZE) // ROW_SIZE
        if root_count > capacity:
            raise Malformed(0x24, "root count exceeds available rows")
        offset = HEADER_SIZE
        roots = []
        for index in range(root_count):
            row = parse_row(stream, offset, end)
            offset += ROW_SIZE
            remaining_roots = root_count - index - 1
            if row.word_0c + remaining_roots > (end - offset) // ROW_SIZE:
                raise Malformed(
                    offset - ROW_SIZE + 0x0C, "child count exceeds available rows"
                )
            children = []
            for _ in range(row.word_0c):
                children.append(parse_row(stream, offset, end))
                offset += ROW_SIZE
            roots.append(Root(row, tuple(children)))
        digest = hashlib.sha256()
        stream.seek(0)
        remaining = end
        while remaining:
            block = read_exact(stream, min(1 << 20, remaining), end - remaining, end)
            digest.update(block)
            remaining -= len(block)
        final = os.fstat(stream.fileno())
        if stream.read(1) or (initial.st_size, initial.st_mtime_ns) != (
            final.st_size,
            final.st_mtime_ns,
        ):
            raise Malformed(0, "catalog changed during read")
    return Catalog(
        end,
        digest.hexdigest(),
        size_word,
        tuple(roots),
        (offset - HEADER_SIZE) // ROW_SIZE,
        end - offset,
    )


def auxiliary_key(root: Root) -> tuple[str, int | None]:
    # xivl-decomp:docs/resource/map-layout-selector.md, "Normal construction and name resolution".
    if root.row.word_20:
        return "nonzero_layout_argument", None
    if not root.children:
        return "no_children", None
    maximum = max(child.resource_key & 0xFFFF for child in root.children)
    if maximum >= 0xFFFF:
        return "low_index_overflow", None
    return "derived", (root.children[0].resource_key & 0xFFFF0000) | (maximum + 1)


def inventory(data_root: Path) -> tuple[list[int], int]:
    keys = []
    nonconforming = 0
    pending = [data_root]
    while pending:
        directory = pending.pop()
        with os.scandir(directory) as entries:
            for entry in entries:
                metadata = entry.stat(follow_symlinks=False)
                if stat.S_ISLNK(metadata.st_mode) or getattr(
                    metadata, "st_file_attributes", 0
                ) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                    raise ValueError("data inventory contains a link or reparse point")
                if stat.S_ISDIR(metadata.st_mode):
                    pending.append(Path(entry.path))
                elif stat.S_ISREG(metadata.st_mode) and entry.name.lower().endswith(
                    ".dat"
                ):
                    match = RESOURCE_PATH.fullmatch(
                        Path(entry.path).relative_to(data_root).as_posix()
                    )
                    if match:
                        keys.append(int("".join(match.groups()), 16))
                    else:
                        nonconforming += 1
    return keys, nonconforming


def duplicates(values: Counter) -> dict[str, int]:
    repeated = [count for count in values.values() if count > 1]
    return {
        "groups": len(repeated),
        "rows": sum(repeated),
        "excessRows": sum(count - 1 for count in repeated),
    }


def numeric_path(data_root: Path, key: int) -> Path:
    groups = f"{key:08X}"
    return data_root / groups[:2] / groups[2:4] / groups[4:6] / (groups[6:] + ".DAT")


def census(
    catalog: Catalog, keys: list[int], nonconforming: int, data_root: Path
) -> dict:
    occupied = set(keys)
    root_keys = Counter()
    child_keys = Counter()
    resource_keys = Counter()
    root_names = Counter()
    child_names = Counter()
    child_types = Counter()
    derived_keys = Counter()
    statuses = Counter()
    roots = []
    exists = {}
    total_children = eligible = missing = 0
    terminated_roots = terminated_children = 0
    for root in catalog.roots:
        row = root.row
        root_keys[row.key] += 1
        root_names[row.effective_name] += 1
        terminated_roots += row.token_terminated
        status, derived = auxiliary_key(root)
        statuses[status] += 1
        if derived is not None:
            derived_keys[derived] += 1
            if derived not in exists:
                exists[derived] = numeric_path(data_root, derived).is_file()
        types = Counter()
        root_eligible = root_missing = 0
        for child in root.children:
            child_keys[child.key] += 1
            resource_keys[child.resource_key] += 1
            child_names[child.effective_name] += 1
            terminated_children += child.token_terminated
            types[child.word_04] += 1
            root_eligible += child.word_04 not in (0, 1)
            if child.resource_key not in exists:
                exists[child.resource_key] = numeric_path(
                    data_root, child.resource_key
                ).is_file()
            root_missing += not exists[child.resource_key]
        child_types.update(types)
        total_children += len(root.children)
        eligible += root_eligible
        missing += root_missing
        roots.append(
            {
                "key": row.key,
                "keyRepresentable16": row.key <= 0xFFFF,
                "nonzeroKeyRepresentable16": 0 < row.key <= 0xFFFF,
                "word20": row.word_20,
                "childCount": len(root.children),
                "tokenTerminated": row.token_terminated,
                "effectiveNameSha256": hashlib.sha256(row.effective_name).hexdigest(),
                "auxiliaryStatus": status,
                "derivedAuxiliaryKey": derived,
                "derivedNumericPathExists": (
                    None if derived is None else exists[derived]
                ),
                "childTypeDistribution": dict(sorted(types.items())),
                "eligibleChildren": root_eligible,
                "missingChildResourcePaths": root_missing,
            }
        )
    # Numeric-mode existence only; catalog mode needs separate path-table evidence.
    # xivl-decomp:docs/resource/resource-path-producer.md, "Formatting contract" and "Catalog mode".
    existing_derived = Counter(
        {key: count for key, count in derived_keys.items() if exists[key]}
    )
    return {
        "catalog": {
            "size": catalog.size,
            "sha256": catalog.sha256,
            "headerSizeWord": catalog.header_size_word,
            "headerSizeEqualsFileSize": catalog.header_size_word == catalog.size,
            "rootCount": len(catalog.roots),
            "rowsConsumed": catalog.rows_consumed,
            "trailingBytes": catalog.trailing_bytes,
        },
        "counts": {
            "children": total_children,
            "eligibleChildren": eligible,
            "missingChildResourcePaths": missing,
            "terminatedRootTokens": terminated_roots,
            "unterminatedRootTokens": len(roots) - terminated_roots,
            "terminatedChildTokens": terminated_children,
            "unterminatedChildTokens": total_children - terminated_children,
        },
        "childTypeDistribution": dict(sorted(child_types.items())),
        "duplicates": {
            "rootKeys": duplicates(root_keys),
            "childKeys": duplicates(child_keys),
            "childResourceKeys": duplicates(resource_keys),
            "rootEffectiveNames": duplicates(root_names),
            "childEffectiveNames": duplicates(child_names),
            "allEffectiveNames": duplicates(root_names + child_names),
        },
        "auxiliary": {
            "statuses": dict(sorted(statuses.items())),
            "derivedKeysHittingExistingFiles": len(existing_derived),
            "rootsHittingExistingFiles": sum(existing_derived.values()),
            "duplicateDerivedKeys": duplicates(derived_keys),
        },
        "inventory": {
            "datFiles": len(keys) + nonconforming,
            "numericPathConforming": len(keys),
            "nonconforming": nonconforming,
            "uniqueNumericKeys": len(occupied),
            "duplicateNumericKeys": duplicates(Counter(keys)),
            "inventorySha256": hashlib.sha256(
                "\n".join(f"{key:08x}" for key in sorted(keys)).encode("ascii")
            ).hexdigest(),
        },
        "roots": roots,
    }


def output_path(path: Path, client_root: Path) -> Path:
    resolved = path.resolve()
    absolute = Path(os.path.abspath(path))
    if resolved.is_relative_to(client_root) or absolute.is_relative_to(client_root):
        raise ValueError("output must be outside the client root")
    if os.path.lexists(path):
        raise ValueError("output path already exists")
    return resolved


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--client-root",
        required=True,
        type=Path,
        help="explicit client install directory; no default or discovery",
    )
    parser.add_argument(
        "--output", type=Path, help="new JSON file outside the client root"
    )
    args = parser.parse_args()
    client_root = args.client_root.resolve(strict=True)
    destination = (
        output_path(args.output, client_root) if args.output is not None else None
    )
    data_root = client_root / "data"
    if not data_root.is_dir():
        raise ValueError("explicit client root has no data directory")
    if data_root.is_symlink() or data_root.is_junction():
        raise ValueError("data directory is a link or junction")
    keys, nonconforming = inventory(data_root)
    result = census(
        parse_catalog(client_root / CATALOG), keys, nonconforming, data_root
    )
    if destination is not None:
        destination = output_path(destination, client_root)
        with destination.open("x", encoding="utf-8", newline="\n") as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write("\n")
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Malformed as error:
        print(f"census failed: {error}", file=sys.stderr)
        sys.exit(2)
    except OSError as error:
        print(f"census failed: filesystem errno={error.errno}", file=sys.stderr)
        sys.exit(2)
    except ValueError as error:
        print(f"census failed: {error}", file=sys.stderr)
        sys.exit(2)
