#!/usr/bin/env python3
"""Census the SEDB container headers of a 1.x client install.

This is the research command that produced the tables in
docs/formats/sedb-res.md. It is not part of validation and makes no support
claim, and it never runs in CI.

    python tools/research/census_sedb.py --client-root <dir>

`--client-root` is required and has no default: there is no client-install
search, no environment fallback, and no workspace-relative path. It reads
the first 0x14 bytes of every file under `<root>/data` matching the
AA/BB/CC/DD.DAT convention and prints counts only. No client bytes are
written anywhere, and nothing it prints is committed by this script.
"""

from __future__ import annotations

import argparse
import collections
import os
import pathlib
import re
import struct
import sys

MAGIC = b"SEDB"
FIXED_HEADER_SIZE = 0x14
RESOURCE_PATH = re.compile(r"^[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}\.DAT$")


def walk_resources(data_root: pathlib.Path):
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
        print("census: no data directory under {0}".format(args.client_root), file=sys.stderr)
        return 2

    total = 0
    off_convention = 0
    signatures: collections.Counter[str] = collections.Counter()
    relations: collections.Counter[str] = collections.Counter()
    by_relation: dict[str, collections.Counter[str]] = collections.defaultdict(
        collections.Counter
    )

    # os.scandir over 140180 files rather than rglob plus a stat call each:
    # the directory entry already carries the size on every platform this
    # runs on, and the AA/BB/CC/DD shape is four levels, not a regex over a
    # rebuilt path string.
    for entry, relative in walk_resources(data_root):
        total += 1
        if not RESOURCE_PATH.match(relative):
            off_convention += 1
            continue
        file_size = entry.stat().st_size
        with open(entry.path, "rb") as handle:
            head = handle.read(FIXED_HEADER_SIZE)
        if len(head) < FIXED_HEADER_SIZE or head[:4] != MAGIC:
            signatures["non-SEDB: " + head[:4].hex()] += 1
            continue
        subtype = head[4:8].rstrip(b"\x00").decode("latin1")
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
    return 0


if __name__ == "__main__":
    sys.exit(main())
