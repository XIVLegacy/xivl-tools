#!/usr/bin/env python3
"""Census stored WRB counts in an explicit retail install, read-only.

Research only, never run in CI. Emit aggregate measurements and SHA-256
inventory identities, without model names or client bytes. Select valid PWIB
first segments. When the wrapper extent is invalid, search WRB signatures and
bound each hit by its own SEDB extent. This does not support the wrapper.

    python tools/research/census_wrb_counts.py --client-root <install>
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import struct
import sys

from census_animation import resources
from census_wrb import children, flatten, need, parse_chunks, words


def wrb_ranges(data: bytes, start: int = 0, end: int | None = None, depth: int = 0):
    if depth > 32:
        raise ValueError("resource depth")
    if end is None:
        end = len(data)
    need(data, start, end - start, len(data))
    tag = data[start : min(start + 8, end)]
    if tag[:4] == b"PWIB":
        need(data, start, 16, end)
        total, first, second = struct.unpack_from(">III", data, start + 4)
        if 16 <= first <= second <= total <= end - start:
            for at, stop, _ in wrb_ranges(
                data, start + first, start + second, depth + 1
            ):
                yield at, stop, "PWIB"
        else:
            at = data.find(b"SEDBwrb\0", start, end)
            while at >= 0:
                yield at, end, "PWIB"
                at = data.find(b"SEDBwrb\0", at + 8, end)
    elif tag == b"SEDBRES ":
        need(data, start, 0x40, end)
        count = words(data, start + 0x30, 1, end, "<")[0]
        directory = start + max(0x40, struct.unpack_from("<H", data, start + 14)[0])
        need(data, directory, count * 16, end)
        base = directory + count * 16
        for i in range(count):
            _, offset, size, _ = words(data, directory + i * 16, 4, end, "<")
            at = base + offset
            stop = min(at + size, end)
            if data[at : min(at + 8, stop)] in (b"SEDBwrb\0", b"SEDBRES "):
                for child, child_end, _ in wrb_ranges(data, at, stop, depth + 1):
                    yield child, child_end, "RES"
    elif tag == b"SEDBwrb\0":
        yield start, end, "SEDB"


def measure(data: bytes, start: int, end: int, counts: Counter, hist: dict):
    need(data, start, 0x14, end)
    header = struct.unpack_from("<H", data, start + 14)[0]
    extent = words(data, start + 16, 1, end, "<")[0]
    if header < 0x14 or extent < header:
        raise ValueError("invalid WRB SEDB extent")
    need(data, start, extent, end)
    chunks = parse_chunks(data, start + header, start + extent)

    def tally(name, value):
        hist.setdefault(name, Counter())[str(value)] += 1

    for chunk, parent in flatten(chunks):
        if chunk.tag == b"MESH":
            counts["MESH"] += 1
            heads = children(chunk, b"HEAD")
            counts["MESH HEAD unique"] += len(heads) == 1
            for head in heads:
                counts["MESH HEAD"] += 1
                need(data, head.body, 4, head.end)
                streams = data[head.body + 2]
                envelopes = data[head.body + 3]
                tally("streamCount", streams)
                tally("envelopeCount", envelopes)
                counts["stream count matches STMS"] += streams == len(
                    children(chunk, b"STMS")
                )
                counts["envelope count matches ENVD"] += envelopes == len(
                    children(chunk, b"ENVD")
                )
        elif chunk.tag == b"MICT":
            counts["MICT"] += 1
            counts["MICT direct WRB child"] += parent == b"WRB\0"
            count = words(data, chunk.body, 1, chunk.end)[0]
            tally("componentCount", count)
            mins = parse_chunks(data, chunk.body + 16, chunk.end)
            child_count = sum(item.tag == b"MINS" for item in mins)
            counts["MINS"] += child_count
            counts["component count matches MINS"] += count == child_count
        elif chunk.tag == b"ENVD":
            counts["ENVD"] += 1
            need(data, chunk.body, 2, chunk.end)
            offset = struct.unpack_from(">H", data, chunk.body)[0]
            tally("ENVD leading BE16", offset)
            if 2 <= offset < chunk.end - chunk.body:
                stop = data.find(b"\0", chunk.body + offset, chunk.end)
                counts["ENVD bounded nonempty ASCII"] += (
                    stop > chunk.body + offset
                    and all(
                        32 <= value < 127 for value in data[chunk.body + offset : stop]
                    )
                )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client-root", type=Path, required=True)
    args = parser.parse_args()
    counts = Counter()
    hist = {}
    inventories = {}
    for family in ("data", "client"):
        inventory = []
        for source in resources(args.client_root / family):
            counts[family + " files"] += 1
            with source.open("rb") as stream:
                prefix = stream.read(8)
                if prefix[:4] != b"PWIB" and prefix not in (
                    b"SEDBRES ",
                    b"SEDBwrb\0",
                ):
                    continue
                data = prefix + stream.read()
            selected = False
            for start, end, framing in wrb_ranges(data):
                selected = True
                try:
                    measure(data, start, end, counts, hist)
                except ValueError:
                    counts["field or framing exceptions"] += 1
                    continue
                counts["WRB trees"] += 1
                counts[family + " " + framing + " WRB trees"] += 1
            if selected:
                counts[family + " selected resources"] += 1
                relative = source.relative_to(args.client_root).as_posix()
                digest = hashlib.sha256(data).hexdigest()
                inventory.append(f"{relative}\t{len(data)}\t{digest}\n")
        inventories[family] = hashlib.sha256(
            "".join(sorted(inventory)).encode("ascii")
        ).hexdigest()
    print(
        json.dumps(
            {
                "counts": dict(sorted(counts.items())),
                "histograms": {
                    key: dict(sorted(value.items())) for key, value in hist.items()
                },
                "inventorySha256": inventories,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 1 if counts["field or framing exceptions"] else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError) as error:
        print(f"census failed: {error}", file=sys.stderr)
        sys.exit(2)
