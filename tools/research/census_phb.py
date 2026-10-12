#!/usr/bin/env python3
"""First-hand census of SEDB PHB collision resources in an explicit install.

Research only: makes no support claim and never runs in CI. Requires
--client-root without defaults or discovery. Reads client files read-only,
never writes client bytes, and prints only numeric measurements, resource
identities, and short chunk tags. Offsets below are GBD-relative unless stated.

    python tools/research/census_phb.py --client-root <dir>
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import math
import os
import pathlib
import re
import struct
import sys
from collections.abc import Iterator
from dataclasses import dataclass

GBD = 0xC0
WORDS = (0x0C, 0x10, 0x30, 0x34, 0x68, 0x6C, 0x70, 0x74, 0x78, 0x7C)
RESOURCE_PATH = re.compile(
    r"^[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}\.DAT$"
)


def walk_resources(
    root: pathlib.Path, counts: collections.Counter[str]
) -> Iterator[tuple[os.DirEntry, str]]:
    stack = [(str(root), "")]
    while stack:
        directory, prefix = stack.pop()
        try:
            with os.scandir(directory) as entries:
                for entry in entries:
                    relative = prefix + entry.name
                    if entry.is_dir(follow_symlinks=False):
                        stack.append((entry.path, relative + "/"))
                    elif entry.is_file(
                        follow_symlinks=False
                    ) and entry.name.upper().endswith(".DAT"):
                        yield entry, relative
        except OSError:
            counts["directory read errors"] += 1


def span(data: bytes, at: int, size: int, end: int | None = None) -> bytes:
    limit = len(data) if end is None else min(end, len(data))
    if at < 0 or size < 0 or at + size > limit:
        raise ValueError(f"span at file 0x{at:X} size {size} exceeds 0x{limit:X}")
    return data[at : at + size]


def u32(data: bytes, at: int) -> int:
    return struct.unpack("<I", span(data, at, 4))[0]


def floats(data: bytes, at: int) -> tuple[float, ...]:
    values = struct.unpack("<4f", span(data, at, 16))
    if not all(math.isfinite(value) for value in values):
        raise ValueError(f"non-finite box at file 0x{at:X}")
    return values


@dataclass
class Geometry:
    vertex_count: int
    triangle_count: int
    highest_index: int
    invalid_indices: int
    surfaces: collections.Counter[int]
    declared_min: tuple[float, ...]
    declared_max: tuple[float, ...]
    decoded_min: tuple[float, ...]
    decoded_max: tuple[float, ...]


def geometry(data: bytes, payload_end: int) -> Geometry:
    vo, vc = u32(data, GBD + 0x38), u32(data, GBD + 0x3C)
    to, tc = u32(data, GBD + 0x60), u32(data, GBD + 0x64)
    if not vc or not tc or vo < 0x80 or to < 0x80:
        raise ValueError("empty geometry or geometry offset before GBD+0x80")
    vertices = span(data, GBD + vo, vc * 12, GBD + payload_end)
    triangles = span(data, GBD + to, tc * 8, GBD + payload_end)
    lower = [math.inf] * 3
    upper = [-math.inf] * 3
    for vertex in struct.iter_unpack("<3f", vertices):
        if not all(math.isfinite(v) for v in vertex):
            raise ValueError("non-finite vertex")
        for axis, value in enumerate(vertex):
            lower[axis] = min(lower[axis], value)
            upper[axis] = max(upper[axis], value)
    surfaces: collections.Counter[int] = collections.Counter()
    highest = -1
    invalid = 0
    for a, b, c, surface in struct.iter_unpack("<4H", triangles):
        highest = max(highest, a, b, c)
        invalid += sum(index >= vc for index in (a, b, c))
        surfaces[surface] += 1
    return Geometry(
        vc,
        tc,
        highest,
        invalid,
        surfaces,
        floats(data, GBD + 0x40),
        floats(data, GBD + 0x50),
        tuple(lower),
        tuple(upper),
    )


def distribution(label: str, counts: collections.Counter) -> None:
    print(label + ": " + ", ".join(f"{v}={n}" for v, n in sorted(counts.items())))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client-root", required=True, type=pathlib.Path)
    args = parser.parse_args()
    root = args.client_root / "data"
    if not root.is_dir():
        print("census: explicit client root has no data directory", file=sys.stderr)
        return 2
    counts: collections.Counter[str] = collections.Counter({"directory read errors": 0})
    fields = {at: collections.Counter() for at in WORDS}
    headers: collections.Counter[tuple[int, int]] = collections.Counter()
    lanes = [collections.Counter(), collections.Counter()]
    surface_totals: collections.Counter[int] = collections.Counter()
    aggregates: collections.Counter[tuple[int, int]] = collections.Counter()
    containment: collections.Counter[float] = collections.Counter()
    largest_violation = (0.0, 0)
    tbc_headers: collections.Counter[tuple[int, int]] = collections.Counter()
    tbc_sizes: collections.Counter[int] = collections.Counter()
    tbc_gaps: collections.Counter[int] = collections.Counter()
    tail_relations: collections.Counter[tuple[bool, bool, bool]] = collections.Counter()
    tail_flags: collections.Counter[tuple[int, bool]] = collections.Counter()
    tail_labels: collections.Counter[tuple[bool, bool, bool]] = collections.Counter()
    identities: dict[int, tuple[int, str]] = {}
    named: set[int] = {0x898002F3, 0x7EB50000, 0x7EB50009}
    families: dict[int, Geometry] = {}
    anomalies: list[tuple[int, str]] = []
    padding = 0.0
    padding_cases: list[tuple[int, int, float, float]] = []
    for entry, relative in walk_resources(root, counts):
        if not RESOURCE_PATH.fullmatch(relative):
            continue
        rid = int(relative.replace("/", "")[:-4], 16)
        try:
            with open(entry.path, "rb") as handle:
                head = handle.read(0x14)
                if head[:8] != b"SEDBPHB\0":
                    continue
                counts["resources"] += 1
                data = head + handle.read()
            identities[rid] = (len(data), hashlib.sha256(data).hexdigest())
            if 0x7EB50000 <= rid <= 0x7EB500FF:
                named.add(rid)
                counts["7EB500 directory PHBs"] += 1
            header_size = struct.unpack_from("<H", span(data, 14, 2))[0]
            declared_size = u32(data, 16)
            headers[(header_size, declared_size)] += 1
            if header_size < 0x14 or max(header_size, declared_size) > len(data):
                raise ValueError("SEDB header or declared extent out of bounds")
            if span(data, GBD, 8) != b"PHB.GBD\0":
                raise ValueError("missing PHB.GBD at file 0xC0")
            counts["GBD at file 0xC0"] += 1
            pe = u32(data, GBD + 8)
            if pe < 0x80:
                raise ValueError("payloadEnd precedes GBD header end")
            if len(data) == GBD + pe + 16:
                counts["file size equation"] += 1
            else:
                raise ValueError(f"file size equation differs: payloadEnd=0x{pe:X}")
            span(data, GBD, pe)
            for at, counter in fields.items():
                counter[u32(data, GBD + at)] += 1
            tail = tuple(u32(data, GBD + at) for at in (0x70, 0x74, 0x78))
            tail_relations[tuple(value == pe for value in tail)] += 1
            tail_flags[(u32(data, GBD + 0x6C), all(value == pe for value in tail))] += 1
            tail_labels[
                tuple(
                    value + 8 <= pe and span(data, GBD + value, 8) == b"unnamed\0"
                    for value in tail
                )
            ] += 1
            if any(value > pe for value in tail):
                anomalies.append(
                    (rid, f"tail offset beyond payloadEnd=0x{pe:X}: {tail}")
                )
            # Follow exactly the declared TBC count and its own extent at +0x0C.
            tbc_start = u32(data, GBD + 0x2C)
            extent = u32(data, GBD + 0x30)
            number = u32(data, GBD + 0x34)
            cursor = tbc_start
            tbc_end = tbc_start + extent
            span(data, GBD + tbc_start, extent, GBD + pe)
            if number > extent // 16:
                raise ValueError("TBC count cannot fit declared extent")
            for _ in range(number):
                header = span(data, GBD + cursor, 16, GBD + tbc_end)
                if header[:8] != b"PHB.TBC\0":
                    raise ValueError(f"missing PHB.TBC at GBD+0x{cursor:X}")
                version, size = struct.unpack_from("<II", header, 8)
                if size < 16:
                    raise ValueError("TBC extent smaller than header")
                span(data, GBD + cursor, size, GBD + tbc_end)
                tbc_headers[(version, number)] += 1
                tbc_sizes[size] += 1
                cursor += size
            counts["TBC chain fits extent and count"] += 1
            counts["TBC chain exactly consumes extent"] += cursor == tbc_end
            regions = [
                u32(data, GBD + at)
                for at in (0x14, 0x1C, 0x24, 0x38, 0x60, 0x70, 0x74, 0x78)
            ]
            next_region = min((v for v in regions + [pe] if v >= tbc_end), default=pe)
            tbc_gaps[next_region - tbc_end] += 1
            counts["TBC end equals next region"] += next_region == tbc_end
            counts["next region aligns TBC end to 0x80"] += (
                next_region == (tbc_end + 127) // 128 * 128
            )
            g = geometry(data, pe)
            counts["nonempty bounded finite geometry"] += 1
            counts["no invalid indices"] += not g.invalid_indices
            counts["highest index equals vertexCount-1"] += (
                g.highest_index == g.vertex_count - 1
            )
            if g.invalid_indices or g.highest_index != g.vertex_count - 1:
                anomalies.append(
                    (
                        rid,
                        f"indices invalid={g.invalid_indices} "
                        f"highest={g.highest_index} vertexCount={g.vertex_count}",
                    )
                )
            surface_totals.update(g.surfaces)
            aggregate = (max(g.surfaces) << 16) | min(g.surfaces)
            stored_aggregate = u32(data, GBD + 0x68)
            aggregates[(stored_aggregate, aggregate)] += 1
            counts["GBD surface range contains triangle surfaces"] += (
                stored_aggregate & 0xFFFF <= min(g.surfaces)
                and stored_aggregate >> 16 >= max(g.surfaces)
            )
            counts["GBD surface aggregate matches triangles"] += (
                u32(data, GBD + 0x68) == aggregate
            )
            if u32(data, GBD + 0x68) != aggregate:
                anomalies.append(
                    (
                        rid,
                        f"GBD aggregate actual=0x{stored_aggregate:08X} "
                        f"expected=0x{aggregate:08X}",
                    )
                )
            if number == 1:
                counts["single TBC resources"] += 1
                counts["single TBC aggregate matches triangles"] += (
                    u32(data, GBD + tbc_start + 0x3C) == aggregate
                )
            for index, lane in enumerate((g.declared_min[3], g.declared_max[3])):
                lanes[index][lane] += 1
            contained = all(
                g.declared_min[a]
                <= g.decoded_min[a]
                <= g.decoded_max[a]
                <= g.declared_max[a]
                for a in range(3)
            )
            counts["declared box contains decoded vertices"] += contained
            violation = max(
                max(
                    g.declared_min[a] - g.decoded_min[a],
                    g.decoded_max[a] - g.declared_max[a],
                    0.0,
                )
                for a in range(3)
            )
            for tolerance in (0.0, 0.00001, 0.0001, 0.001, 0.01):
                containment[tolerance] += violation <= tolerance
            if violation > largest_violation[0]:
                largest_violation = (violation, rid)
            if not contained:
                anomalies.append(
                    (
                        rid,
                        f"bounds declared={g.declared_min[:3]}/{g.declared_max[:3]} "
                        f"decoded={g.decoded_min}/{g.decoded_max}",
                    )
                )
            for axis in range(3):
                if g.decoded_min[axis] == g.decoded_max[axis]:
                    low = g.decoded_min[axis] - g.declared_min[axis]
                    high = g.declared_max[axis] - g.decoded_max[axis]
                    value = max(low, high)
                    if value > padding:
                        padding, padding_cases = value, []
                    if value == padding and value > 0:
                        padding_cases.append((rid, axis, low, high))
            if 120 in g.surfaces:
                named.add(rid)
                print(
                    f"P7 surface120 0x{rid:08X}: "
                    f"{g.surfaces[120]}/{g.triangle_count} triangles"
                )
            if 0x7EB50000 <= rid <= 0x7EB500FF:
                families[rid] = g
        except (OSError, ValueError, struct.error) as error:
            counts["malformed or unreadable"] += 1
            anomalies.append((rid, str(error)))
    print(
        "PHB census; offsets GBD-relative; strict decoded containment; "
        "degenerate padding per side"
    )
    for label, count in sorted(counts.items()):
        print(f"count {label}: {count}")
    distribution("P1 SEDB (headerSize,+0x10)", headers)
    for at, counter in fields.items():
        distribution(f"GBD +0x{at:02X}", counter)
    distribution("P3 TBC (+0x08,GBD TBC count), per TBC", tbc_headers)
    distribution("P3 TBC +0x0C extents", tbc_sizes)
    distribution("P3 gap from TBC offset+extent to next declared region", tbc_gaps)
    distribution("P5 tail offsets equal payloadEnd (+0x70,+0x74,+0x78)", tail_relations)
    distribution("P5 (+0x6C,all three tail offsets == payloadEnd)", tail_flags)
    distribution(
        "P5 tail offsets point to unnamed label (+0x70,+0x74,+0x78)", tail_labels
    )
    distribution("P5 min fourth lane", lanes[0])
    distribution("P5 max fourth lane", lanes[1])
    distribution("P4 (stored,triangle min/max) aggregate pairs", aggregates)
    distribution("P6 containment by absolute per-axis tolerance", containment)
    worst, worst_id = largest_violation
    named.add(worst_id)
    print(f"P6 largest containment violation: 0x{worst_id:08X} distance={worst:.12g}")
    print(f"P6 maximum degenerate-axis padding per side: {padding:.12g}")
    for rid, axis, low, high in sorted(padding_cases):
        named.add(rid)
        print(f"P6 padding 0x{rid:08X} axis={axis} low={low:.12g} high={high:.12g}")
    distribution("P7 triangle surface histogram", surface_totals)
    print(
        f"P8 directory PHBs: {counts['7EB500 directory PHBs']}; "
        f"expected interval present={all(rid in families for rid in range(0x7EB50000, 0x7EB50020))}"
    )
    for rid, g in sorted(families.items()):
        family = ("cube", "cylinder", "single plane", "double-sided plane")[
            min((rid & 255) // 8, 3)
        ]
        print(
            f"P8 0x{rid:08X} {family}: vertices={g.vertex_count} "
            f"triangles={g.triangle_count} surfaces={dict(sorted(g.surfaces.items()))} "
            f"declared={g.declared_min[:3]}/{g.declared_max[:3]} "
            f"decoded={g.decoded_min}/{g.decoded_max}"
        )
    print(
        f"anomalies: {len(anomalies)}; "
        f"malformed or unreadable: {counts['malformed or unreadable']}"
    )
    for rid, detail in sorted(anomalies):
        named.add(rid)
        print(f"anomaly 0x{rid:08X}: {detail}")
    for rid in sorted(named):
        if rid in identities:
            size, digest = identities[rid]
            print(f"identity 0x{rid:08X} size={size} SHA-256={digest}")
        else:
            print(f"identity 0x{rid:08X} unavailable")
    return 0


if __name__ == "__main__":
    sys.exit(main())
