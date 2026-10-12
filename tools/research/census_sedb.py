#!/usr/bin/env python3
"""Census the SEDB container headers of a 1.x client install.

This is the research command that produced the tables in
docs/formats/sedb-res.md. It is not part of validation and makes no support
claim, and it never runs in CI.

    python tools/research/census_sedb.py --client-root <dir>

`--client-root` is required and has no default: there is no client-install
search, no environment fallback, and no workspace-relative path. It reads
the headers of every file under `<root>/data` matching the
AA/BB/CC/DD.DAT convention, direct RES children, and MapLayout scenes.
It prints counts, numeric header fields and resource identities only. No
client bytes are written anywhere, and nothing it prints is committed by this script.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import os
import pathlib
import re
import struct
import sys
from collections.abc import Iterator
from typing import BinaryIO

MAGIC = b"SEDB"
FIXED_HEADER_SIZE = 0x14
RESOURCE_PATH = re.compile(
    r"^[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}\.DAT$"
)


def walk_resources(data_root: pathlib.Path) -> Iterator[tuple[os.DirEntry, str]]:
    """Yield (DirEntry, root-relative posix path) for every .DAT below root."""
    stack = [(str(data_root), "")]
    while stack:
        directory, prefix = stack.pop()
        try:
            entries = list(os.scandir(directory))
        except OSError:
            continue
        for entry in entries:
            relative = prefix + entry.name
            if entry.is_dir(follow_symlinks=False):
                stack.append((entry.path, relative + "/"))
            elif entry.name.upper().endswith(".DAT"):
                yield entry, relative


def classify(declared_size: int, header_size: int, file_size: int) -> str:
    if declared_size < header_size:
        return "declaredSize < headerSize"
    if declared_size == header_size:
        return "declaredSize == headerSize"
    if declared_size == file_size:
        return "declaredSize == file size"
    if declared_size < file_size:
        return "declaredSize < file size"
    return "declaredSize > file size"


HeaderTables = dict[str, dict[str, collections.Counter[int]]]


def read_at(handle: BinaryIO, at: int, size: int, limit: int) -> bytes:
    if at < 0 or size < 0 or at + size > limit:
        raise ValueError(f"span at file 0x{at:X} size {size} exceeds 0x{limit:X}")
    handle.seek(at)
    data = handle.read(size)
    if len(data) != size:
        raise ValueError(f"short read at file 0x{at:X}")
    return data


def subtype_name(tag: bytes) -> str:
    return "".join(
        chr(b) if 32 <= b < 127 else f"\\x{b:02x}" for b in tag.rstrip(b"\0")
    )


def tabulate(head: bytes, available: int, tables: HeaderTables) -> str:
    if len(head) < FIXED_HEADER_SIZE or head[:4] != MAGIC:
        raise ValueError("missing fixed SEDB header")
    subtype = subtype_name(head[4:8])
    unknown_a, flags, header_size, declared_size = struct.unpack_from("<IHHI", head, 8)
    if header_size < FIXED_HEADER_SIZE or max(header_size, declared_size) > available:
        raise ValueError("SEDB header or declared extent out of bounds")
    fields = tables.setdefault(subtype, {})
    for label, value in (
        ("unknownA", unknown_a),
        ("flags", flags),
        ("headerSize", header_size),
    ):
        fields.setdefault(label, collections.Counter())[value] += 1
    return subtype


def nested_headers(
    handle: BinaryIO,
    head: bytes,
    file_size: int,
    tables: HeaderTables,
    counts: collections.Counter[str],
    issues: list[str],
    header_directory_anomalies: list[str],
) -> None:
    header_size = struct.unpack_from("<H", head, 14)[0]
    count, _, repeated, _ = struct.unpack(
        "<III4s", read_at(handle, 0x30, 16, file_size)
    )
    if count != repeated:
        raise ValueError(
            f"invalid RES extended header: headerSize=0x{header_size:X} "
            f"count={count} repeated={repeated}"
        )
    # The extended fields occupy through 0x40, including when headerSize is 0x30.
    directory_start = max(header_size, 0x40)
    if directory_start != header_size:
        counts["RES directory after declared headerSize"] += 1
        header_directory_anomalies.append(
            f"RES directory at file 0x{directory_start:X} after "
            f"declared headerSize=0x{header_size:X}"
        )
    base = directory_start + 16 * count
    directory = read_at(handle, directory_start, 16 * count, file_size)
    counts["RES resources inspected"] += 1
    counts["RES direct directory entries"] += count
    for slot, (_, offset, size, _) in enumerate(struct.iter_unpack("<4I", directory)):
        start = base + offset
        if start > file_size:
            issues.append(f"RES slot={slot} start=0x{start:X} past file size")
            continue
        if start + size > file_size:
            counts["RES extents past file size"] += 1
        available = min(size, file_size - start)
        child = read_at(handle, start, min(FIXED_HEADER_SIZE, available), file_size)
        if child[:4] != MAGIC:
            counts["RES non-SEDB direct entries"] += 1
            continue
        try:
            tabulate(child, available, tables)
            counts["RES direct SEDB children"] += 1
        except ValueError as error:
            issues.append(f"RES slot={slot} file_offset=0x{start:X}: {error}")


def print_tables(label: str, tables: HeaderTables) -> None:
    print("")
    print(label + ":")
    for subtype, fields in sorted(tables.items()):
        for field in ("unknownA", "flags", "headerSize"):
            values = ", ".join(
                f"0x{value:X}={count}" for value, count in sorted(fields[field].items())
            )
            print(f"  {subtype} {field}: {values}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--client-root",
        required=True,
        type=pathlib.Path,
        help="the client install directory; required, with no default",
    )
    args = parser.parse_args()

    data_root = args.client_root / "data"
    if not data_root.is_dir():
        print(
            "census: no data directory under {0}".format(args.client_root),
            file=sys.stderr,
        )
        return 2

    total = 0
    off_convention = 0
    signatures: collections.Counter[str] = collections.Counter()
    relations: collections.Counter[str] = collections.Counter()
    by_relation: dict[str, collections.Counter[str]] = collections.defaultdict(
        collections.Counter
    )
    top_tables: HeaderTables = {}
    res_tables: HeaderTables = {}
    scene_tables: HeaderTables = {}
    nested_counts: collections.Counter[str] = collections.Counter()
    issues: list[tuple[str, str]] = []
    header_directory_anomalies: list[tuple[str, str]] = []
    identities: dict[str, tuple[int, str]] = {}

    # os.scandir over 140180 files rather than rglob plus a stat call each:
    # the directory entry already carries the size on every platform this
    # runs on, and the AA/BB/CC/DD shape is four levels, not a regex over a
    # rebuilt path string.
    for entry, relative in walk_resources(data_root):
        total += 1
        if not RESOURCE_PATH.match(relative):
            off_convention += 1
            continue
        try:
            file_size = entry.stat().st_size
            with open(entry.path, "rb") as handle:
                head = handle.read(FIXED_HEADER_SIZE)
                local_issues: list[str] = []
                local_header_directory_anomalies: list[str] = []
                try:
                    if head[:4] == MAGIC:
                        tabulate(head, file_size, top_tables)
                        if head[4:8] == b"RES ":
                            nested_headers(
                                handle,
                                head,
                                file_size,
                                res_tables,
                                nested_counts,
                                local_issues,
                                local_header_directory_anomalies,
                            )
                    elif head == b"MapLayoutResourceDat":
                        outer = read_at(handle, 0, 0x24, file_size)
                        if outer[:24] != b"MapLayoutResourceData\0\0\0":
                            raise ValueError("invalid MapLayout signature")
                        scene = struct.unpack_from("<I", outer, 0x20)[0]
                        child = read_at(handle, scene, FIXED_HEADER_SIZE, file_size)
                        if child[4:8] != b"lyb\0":
                            raise ValueError(
                                f"MapLayout scene at 0x{scene:X} is not lyb"
                            )
                        tabulate(child, file_size - scene, scene_tables)
                        nested_counts["MapLayout lyb scenes"] += 1
                except (ValueError, struct.error) as error:
                    local_issues.append(str(error))
                if local_issues or local_header_directory_anomalies:
                    issues.extend((relative, issue) for issue in local_issues)
                    header_directory_anomalies.extend(
                        (relative, issue) for issue in local_header_directory_anomalies
                    )
                    handle.seek(0)
                    digest = hashlib.file_digest(handle, "sha256").hexdigest()
                    identities[relative] = (file_size, digest)
        except OSError as error:
            issues.append((relative, f"unreadable resource errno={error.errno}"))
            continue
        if len(head) < FIXED_HEADER_SIZE or head[:4] != MAGIC:
            signatures["non-SEDB: " + head[:4].hex()] += 1
            continue
        subtype = subtype_name(head[4:8])
        _, _, header_size, declared_size = struct.unpack_from("<IHHI", head, 8)
        signatures["SEDB " + subtype] += 1
        relation = classify(declared_size, header_size, file_size)
        relations[relation] += 1
        by_relation[relation][subtype] += 1

    print("resource files: {0}".format(total))
    print("off the AA/BB/CC/DD.DAT convention: {0}".format(off_convention))
    print("")
    print("signatures:")
    for name, count in signatures.most_common(20):
        print("  {0:<24} {1}".format(name, count))
    print("")
    print("the 0x10 field against headerSize and file size:")
    for name, count in relations.most_common():
        subtypes = ", ".join(
            "{0} {1}".format(sub, n) for sub, n in by_relation[name].most_common(8)
        )
        print("  {0:<28} {1:>6}   {2}".format(name, count, subtypes))
    print_tables("top-level SEDB header fields", top_tables)
    print_tables("direct RES SEDB child header fields", res_tables)
    print_tables("MapLayout lyb header fields", scene_tables)
    print("")
    for label, count in sorted(nested_counts.items()):
        print(f"{label}: {count}")
    print(f"malformed or unreadable resources: {len({path for path, _ in issues})}")
    print(
        f"header/directory anomaly resources: {len({path for path, _ in header_directory_anomalies})}"
    )
    for relative, issue in sorted(issues + header_directory_anomalies):
        rid = int(relative.replace("/", "")[:-4], 16)
        print(f"anomaly 0x{rid:08X}: {issue}")
    for relative, (size, digest) in sorted(identities.items()):
        rid = int(relative.replace("/", "")[:-4], 16)
        print(f"identity 0x{rid:08X} size={size} SHA-256={digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
