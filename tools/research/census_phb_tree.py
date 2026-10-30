#!/usr/bin/env python3
"""Research-only PHB tree census, requiring an explicit --client-root.

Outputs aggregate structure and resource identities, never geometry values or
client bytes. This is not a writer or a support claim and does not run in CI.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
import math
import os
import pathlib
import stat
import struct
import sys
from dataclasses import dataclass

from census_phb import GBD, RESOURCE_PATH, floats, span, u32


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def align(value: int, alignment: int = 16) -> int:
    return (value + alignment - 1) // alignment * alignment


def leaf_slice(packed: int, reference_count: int) -> tuple[int, int]:
    require(0 <= packed < 65535, "branch word is not a leaf")
    start, count = packed & 0x3FFF, (packed >> 14) + 1
    require(start + count <= reference_count, "leaf reference slice out of range")
    return start, count


def candidate_slots(nodes: list[tuple[int, ...]], query: Box) -> list[int]:
    """Interpret absolute skips and inclusive integer overlap without recursion."""
    slots = []
    index = 0
    count = len(nodes) - 1
    while index < count:
        node = nodes[index]
        require(index < node[3] <= count, "invalid node skip")
        if any(
            query.lower[a] > node[a + 4] or query.upper[a] < node[a] for a in range(3)
        ):
            index = node[3]
        elif node[7] == 65535:
            index += 1
        else:
            start, size = node[7] & 0x3FFF, (node[7] >> 14) + 1
            slots.extend(range(start, start + size))
            index += 1
    return slots


class Measurements:
    def __init__(self) -> None:
        self.counts: collections.Counter[str] = collections.Counter()
        self.histograms: dict[str, collections.Counter] = collections.defaultdict(
            collections.Counter
        )
        self.maxima: dict[str, float | int] = {}

    def hist(self, label: str, value: object) -> None:
        self.histograms[label][str(value)] += 1

    def maximum(self, label: str, value: float | int) -> None:
        self.maxima[label] = max(value, self.maxima.get(label, value))

    def report(self) -> dict:
        return {
            "counts": dict(sorted(self.counts.items())),
            "histograms": {
                label: dict(sorted(values.items()))
                for label, values in sorted(self.histograms.items())
            },
            "maxima": dict(sorted(self.maxima.items())),
        }


@dataclass
class Box:
    lower: tuple[float, ...]
    upper: tuple[float, ...]


def read_box(data: bytes, at: int) -> Box:
    lower, upper = floats(data, at), floats(data, at + 16)
    require(all(lower[a] <= upper[a] for a in range(3)), "inverted float box")
    return Box(lower, upper)


def union(boxes: list[Box]) -> Box:
    return Box(
        tuple(min(b.lower[a] for b in boxes) for a in range(3)),
        tuple(max(b.upper[a] for b in boxes) for a in range(3)),
    )


def measure_float_box(label: str, stored: Box, actual: Box, m: Measurements) -> None:
    excess = max(
        0,
        *(stored.lower[a] - actual.lower[a] for a in range(3)),
        *(actual.upper[a] - stored.upper[a] for a in range(3)),
    )
    m.maximum(label + " float box excess", excess)
    m.maximum(
        label + " float box padding",
        max(
            0,
            *(actual.lower[a] - stored.lower[a] for a in range(3)),
            *(stored.upper[a] - actual.upper[a] for a in range(3)),
        ),
    )
    for tolerance in (0, 0.00001, 0.0001, 0.001):
        m.hist(label + f" float containment tolerance {tolerance}", excess <= tolerance)


def measure_aggregate(
    label: str, stored: int, selected: list[int], m: Measurements
) -> None:
    lower, upper = min(selected), max(selected)
    m.hist(
        label + " aggregate matches referenced surfaces",
        stored == (upper << 16 | lower),
    )
    contains = stored & 0xFFFF <= lower and stored >> 16 >= upper
    m.hist(label + " aggregate contains referenced surfaces", contains)
    m.counts["surface ranges outside referenced geometry"] += not contains


@dataclass
class Tree:
    nodes: int
    leaves: int
    depth: int
    references: list[int]
    box: Box


def analyze_tbd(
    data: bytes, at: int, extent: int, triangle_boxes: list[Box], m: Measurements
) -> Tree:
    span(data, at, extent)
    require(at % 16 == 0 and extent % 16 == 0, "misaligned TBD chunk or extent")
    require(span(data, at, 8) == b"PHB.TBD\0", "missing TBD tag")
    require(u32(data, at + 8) == extent, "TBD extent-table mismatch")
    n, no, rc, ro = (u32(data, at + a) for a in (12, 16, 20, 24))
    require(0 < n < 65536, "invalid TBD node count")
    require(0 < rc <= extent // 2, "invalid TBD reference count")
    require(no >= 0x50, "TBD node offset before header end")
    require((at + no) % 16 == 0, "misaligned TBD node and terminal array")
    span(data, at + no, (n + 1) * 16, at + extent)
    require(ro >= no + (n + 1) * 16, "TBD references overlap nodes")
    refs = list(struct.unpack(f"<{rc}H", span(data, at + ro, rc * 2, at + extent)))
    require(
        all(r < len(triangle_boxes) for r in refs), "triangle reference out of range"
    )
    box = read_box(data, at + 32)
    nodes = [struct.unpack_from("<8H", data, at + no + i * 16) for i in range(n + 1)]
    for label, value in (
        ("TBD node offset", no),
        ("TBD node/ref gap", ro - no - (n + 1) * 16),
        ("TBD trailing allocation", extent - ro - rc * 2),
        ("TBD canonical aligned extent", extent == align(ro + rc * 2)),
        ("TBD header +1C", u32(data, at + 28)),
        ("TBD header +44..4C", tuple(u32(data, at + a) for a in (68, 72, 76))),
        ("TBD box fourth lanes", (box.lower[3], box.upper[3])),
    ):
        m.hist(label, value)
    require(nodes[0][3] == n, "root skip does not reach terminal")
    terminal = nodes[n]
    m.hist("terminal coordinate/marker shape", terminal[:3] + terminal[4:])
    m.hist("terminal skip equals nodes+references", terminal[3] == n + rc)
    require(
        terminal[:3] == (65535,) * 3
        and terminal[4:] == (0, 0, 0, 65535)
        and terminal[3] == n + rc,
        "invalid terminal record",
    )
    geometry_boxes: list[Box | None] = [None] * n
    raw_boxes: list[Box | None] = [None] * n
    subtree_refs: list[list[int]] = [[] for _ in range(n)]
    depths = [0] * n
    child_lists: list[list[int]] = [[] for _ in range(n)]
    leaves = 0
    starts = []
    reference_uses = [0] * rc
    for i in range(n - 1, -1, -1):
        node = nodes[i]
        skip, packed = node[3], node[7]
        require(i < skip <= n, "invalid node skip")
        require(all(node[a] <= node[a + 4] for a in range(3)), "inverted integer box")
        raw_boxes[i] = Box(node[:3], node[4:7])
        m.maximum(
            "node coordinate units below 4000",
            max(0, *(16384 - node[a] for a in (0, 1, 2, 4, 5, 6))),
        )
        m.maximum(
            "node coordinate units above C000",
            max(0, *(node[a] - 49152 for a in (0, 1, 2, 4, 5, 6))),
        )
        m.hist(
            "node coordinate range 4000..C000",
            all(16384 <= node[a] <= 49152 for a in (0, 1, 2, 4, 5, 6)),
        )
        if packed != 65535:
            require(skip == i + 1, "leaf skip is not consecutive")
            start, count = leaf_slice(packed, rc)
            starts.append((i, start, count))
            for j in range(start, start + count):
                reference_uses[j] += 1
            subtree_refs[i] = refs[start : start + count]
            geometry_boxes[i] = union([triangle_boxes[r] for r in subtree_refs[i]])
            leaves += 1
            m.hist("leaf reference count", count)
            m.maximum("leaf reference start", start)
            m.hist(
                "leaf references ascending", subtree_refs[i] == sorted(subtree_refs[i])
            )
        else:
            child = i + 1
            while child < skip:
                require(nodes[child][3] <= skip, "child skip escapes parent")
                child_lists[i].append(child)
                child = nodes[child][3]
            require(
                child == skip and len(child_lists[i]) >= 2,
                "invalid branch child partition",
            )
            children = child_lists[i]
            geometry_boxes[i] = union([geometry_boxes[c] for c in children])
            subtree_refs[i] = [r for c in children for r in subtree_refs[c]]
            m.hist("branch child count", len(children))
            child_raw = union([raw_boxes[c] for c in children])
            m.hist("branch raw box equals child union", raw_boxes[i] == child_raw)
            if len(children) == 2:
                a, b = children
                m.hist(
                    "binary child triangle count difference",
                    abs(len(subtree_refs[a]) - len(subtree_refs[b])),
                )
                m.hist(
                    "binary left minus right triangle count",
                    len(subtree_refs[a]) - len(subtree_refs[b]),
                )
                child_boxes = [geometry_boxes[a], geometry_boxes[b]]
                m.hist(
                    "binary child disjoint axes",
                    sum(
                        child_boxes[0].upper[j] <= child_boxes[1].lower[j]
                        or child_boxes[1].upper[j] <= child_boxes[0].lower[j]
                        for j in range(3)
                    ),
                )
                centers = [
                    [
                        (triangle_boxes[r].lower[j] + triangle_boxes[r].upper[j]) / 2
                        for r in subtree_refs[c]
                    ]
                    for c in children
                    for j in range(3)
                ]
                ordering = [max(centers[j]) <= min(centers[3 + j]) for j in range(3)]
                reverse = [min(centers[j]) >= max(centers[3 + j]) for j in range(3)]
                m.hist(
                    "binary triangle AABB center partition forward axis mask",
                    sum(1 << j for j, ok in enumerate(ordering) if ok),
                )
                m.hist(
                    "binary triangle AABB center partition either direction",
                    any(ordering) or any(reverse),
                )
                widths = [
                    geometry_boxes[i].upper[j] - geometry_boxes[i].lower[j]
                    for j in range(3)
                ]
                widest = max(widths)
                m.hist(
                    "binary triangle AABB center partition along longest geometry axis",
                    any(
                        (ordering[j] or reverse[j]) and widths[j] == widest
                        for j in range(3)
                    ),
                )
        geometry_box = geometry_boxes[i]
        raw_box = raw_boxes[i]
        max_excess = 0.0
        for a in range(3):
            width = box.upper[a] - box.lower[a]
            require(width > 0, "zero TBD float box width")
            qlo = 16384 + (geometry_box.lower[a] - box.lower[a]) / width * 32768
            qhi = 16384 + (geometry_box.upper[a] - box.lower[a]) / width * 32768
            low_delta, high_delta = raw_box.lower[a] - qlo, qhi - raw_box.upper[a]
            max_excess = max(max_excess, low_delta, high_delta)
            for label, delta in (("minimum", low_delta), ("maximum", high_delta)):
                m.hist(
                    "node quantization " + label + " excess bucket",
                    "<=0"
                    if delta <= 0
                    else "<=0.001"
                    if delta <= 0.001
                    else "<=0.01"
                    if delta <= 0.01
                    else "<=1"
                    if delta <= 1
                    else ">1",
                )
            m.hist(
                "node minimum equals floor exact geometry",
                raw_box.lower[a] == math.floor(qlo),
            )
            m.hist(
                "node maximum equals ceil exact geometry",
                raw_box.upper[a] == math.ceil(qhi),
            )
            m.hist(
                "node minimum equals truncated geometry",
                raw_box.lower[a] == math.trunc(qlo),
            )
            m.hist(
                "node maximum equals truncated geometry",
                raw_box.upper[a] == math.trunc(qhi),
            )
            m.hist(
                "node minimum minus floor geometry", raw_box.lower[a] - math.floor(qlo)
            )
            m.hist(
                "node maximum minus ceil geometry", raw_box.upper[a] - math.ceil(qhi)
            )
            m.maximum("node quantization excess units", max(0, low_delta, high_delta))
            m.maximum(
                "node quantization excess world distance",
                max(0, low_delta, high_delta) * width / 32768,
            )
            m.maximum(
                "node padding quantization units", max(-low_delta, -high_delta, 0)
            )
        for tolerance in (0, 0.001, 0.01, 1, 1.01):
            m.counts[
                f"nodes enclosing geometry tolerance {tolerance} quantized units"
            ] += max_excess <= tolerance
    for i, children in enumerate(child_lists):
        for child in children:
            depths[child] = depths[i] + 1
    cursor = 0
    contiguous = True
    for _, start, count in sorted(starts):
        contiguous &= start == cursor
        cursor += count
    m.hist("leaf ref slices cover each slot once", all(v == 1 for v in reference_uses))
    require(
        all(v == 1 for v in reference_uses), "leaf reference slots missing or reused"
    )
    m.hist("leaf ref starts contiguous preorder", contiguous)
    m.hist(
        "full integer query covers reference slots",
        sorted(candidate_slots(nodes, Box((0, 0, 0), (65535, 65535, 65535))))
        == list(range(rc)),
    )
    m.hist("TBD triangle references unique", len(set(refs)) == rc)
    measure_float_box("TBD", box, geometry_boxes[0], m)
    m.hist("TBD triangle reference order ascending", refs == sorted(refs))
    m.hist("TBD node count", n)
    m.hist("TBD maximum depth", max(depths))
    m.hist("TBD reference count", rc)
    m.maximum("TBD nodes", n)
    m.maximum("TBD references", rc)
    m.maximum("triangle reference value", max(refs))
    m.maximum("TBD depth", max(depths))
    m.counts["TBDs"] += 1
    m.counts["nodes"] += n
    m.counts["leaves"] += leaves
    m.counts["branches"] += n - leaves
    m.counts["references"] += rc
    return Tree(n, leaves, max(depths), refs, box)


def analyze_phb(data: bytes, m: Measurements) -> dict:
    initial_nodes = m.counts["nodes"]
    initial_enclosing = m.counts[
        "nodes enclosing geometry tolerance 1.01 quantized units"
    ]
    initial_surface_failures = m.counts["surface ranges outside referenced geometry"]
    require(span(data, 0, 8) == b"SEDBPHB\0", "missing SEDB PHB tag")
    require(span(data, GBD, 8) == b"PHB.GBD\0", "missing GBD tag")
    pe = u32(data, GBD + 8)
    require(len(data) == GBD + pe + 16, "invalid GBD/file extent")
    vo, vc, to, tc = (u32(data, GBD + a) for a in (56, 60, 96, 100))
    require(vc > 0 and tc > 0, "empty geometry")
    require(vo >= 0x80 and to >= 0x80, "geometry precedes GBD header end")
    vertices = list(struct.iter_unpack("<3f", span(data, GBD + vo, vc * 12, GBD + pe)))
    require(all(math.isfinite(v) for p in vertices for v in p), "non-finite vertex")
    triangle_boxes = []
    surfaces = []
    for *indices, surface in struct.iter_unpack(
        "<4H", span(data, GBD + to, tc * 8, GBD + pe)
    ):
        require(all(i < vc for i in indices), "invalid triangle vertex index")
        triangle_boxes.append(union([Box(vertices[i], vertices[i]) for i in indices]))
        surfaces.append(surface)
    tbc_start, tbc_extent, tbc_count = (u32(data, GBD + a) for a in (44, 48, 52))
    span(data, GBD + tbc_start, tbc_extent, GBD + pe)
    require(0 < tbc_count <= tbc_extent // 0x50, "invalid TBC count")
    cursor = GBD + tbc_start
    trees = []
    all_refs = []
    tbc_spans = []
    for _ in range(tbc_count):
        require(span(data, cursor, 8) == b"PHB.TBC\0", "missing TBC tag")
        size = u32(data, cursor + 12)
        require(size >= 0x50, "invalid TBC extent")
        require(cursor % 16 == 0 and size % 16 == 0, "misaligned TBC chunk or extent")
        span(data, cursor, size, GBD + tbc_start + tbc_extent)
        tbc_spans.append((cursor - GBD - tbc_start, size))
        count, offsets, sizes = (u32(data, cursor + a) for a in (48, 52, 56))
        require(0 < count <= size // 0x50, "invalid TBD count")
        span(data, cursor + offsets, count * 4, cursor + size)
        span(data, cursor + sizes, count * 4, cursor + size)
        require(
            offsets >= 0x50 and sizes >= align(offsets + count * 4),
            "overlapping TBC tables",
        )
        m.hist("TBC version", u32(data, cursor + 8))
        m.hist("TBC TBD count", count)
        m.hist("TBC offset-table offset", offsets)
        m.hist("TBC extent-table canonical offset", sizes == align(offsets + count * 4))
        m.hist(
            "TBC reserved +40..4C",
            tuple(u32(data, cursor + a) for a in (64, 68, 72, 76)),
        )
        tbc_box = read_box(data, cursor + 16)
        m.hist("TBC box fourth lanes", (tbc_box.lower[3], tbc_box.upper[3]))
        first_reference = len(all_refs)
        end = align(sizes + count * 4)
        for index in range(count):
            offset = u32(data, cursor + offsets + index * 4)
            extent = u32(data, cursor + sizes + index * 4)
            require(offset >= end and extent >= 0x50, "invalid TBD extent or overlap")
            span(data, cursor + offset, extent, cursor + size)
            m.hist("TBD gap before chunk", offset - end)
            tree = analyze_tbd(data, cursor + offset, extent, triangle_boxes, m)
            selected = [surfaces[r] for r in tree.references]
            measure_aggregate("TBD", u32(data, cursor + offset + 0x40), selected, m)
            trees.append(tree)
            all_refs.extend(tree.references)
            end = offset + extent
        measure_float_box(
            "TBC",
            tbc_box,
            union([triangle_boxes[r] for r in all_refs[first_reference:]]),
            m,
        )
        selected = [surfaces[r] for r in all_refs[first_reference:]]
        measure_aggregate("TBC", u32(data, cursor + 0x3C), selected, m)
        m.hist("TBC extent aligned16", size % 16 == 0)
        m.hist("TBC final chunk consumes extent", end == size)
        require(end == size, "TBC trailing extent")
        m.counts["TBCs"] += 1
        cursor += size
    require(cursor == GBD + tbc_start + tbc_extent, "TBC chain does not consume extent")
    coverage = set(all_refs) == set(range(tc))
    multiplicity = max(collections.Counter(all_refs).values())
    m.hist("resource references cover every triangle", coverage)
    m.hist(
        "resource reference multiplicity maximum",
        multiplicity,
    )
    m.hist("resource references ascending", all_refs == sorted(all_refs))
    measure_float_box("GBD", read_box(data, GBD + 64), union(triangle_boxes), m)
    measure_aggregate("GBD", u32(data, GBD + 0x68), surfaces, m)
    m.hist("resource TBC count", tbc_count)
    tails = tuple(u32(data, GBD + a) for a in (112, 116, 120))
    require(all(0x80 <= value <= pe for value in tails), "tail offset out of range")
    no_tail = all(value == pe for value in tails)
    dimension = u32(data, GBD + 16)
    m.hist("GBD grid dimension/tails", (dimension, not no_tail))
    for offset_lane, size_lane in ((20, 24), (28, 32), (36, 40)):
        offset, extent = u32(data, GBD + offset_lane), u32(data, GBD + size_lane)
        span(data, GBD + offset, extent, GBD + pe)
        m.hist(f"GBD prefix +{offset_lane:02X} size", extent)
    grid_at, grid_size, list_at, list_size, desc_at, desc_size = (
        u32(data, GBD + a) for a in (20, 24, 28, 32, 36, 40)
    )
    require(
        dimension > 0 and dimension**3 * 4 <= grid_size,
        "grid dimension exceeds prefix table",
    )
    require(tbc_count * 16 <= desc_size, "TBC descriptors exceed prefix table")
    m.hist(
        "grid offset allocation is aligned128 cell count",
        grid_size == align(dimension**3 * 4, 128),
    )
    m.hist(
        "TBC descriptor allocation is aligned128 count",
        desc_size == align(tbc_count * 16, 128),
    )
    lists = {}
    listed_tbcs = set()
    last_list_end = 0
    for cell in range(dimension**3):
        start = u32(data, GBD + grid_at + cell * 4) * 2
        if start not in lists:
            position = start
            members = []
            while True:
                item = struct.unpack(
                    "<H",
                    span(data, GBD + list_at + position, 2, GBD + list_at + list_size),
                )[0]
                position += 2
                if item == 65535:
                    break
                require(
                    item < tbc_count and item < 512, "grid list TBC ID out of range"
                )
                members.append(item)
            lists[start] = members
            last_list_end = max(last_list_end, position)
        members = lists[start]
        listed_tbcs.update(members)
        m.hist("grid cell TBC reference count", len(members))
        m.hist("grid cell TBC references unique", len(set(members)) == len(members))
        m.hist("grid cell TBC references ascending", members == sorted(members))
    m.hist("grid lists cover all TBC IDs", listed_tbcs == set(range(tbc_count)))
    m.hist("grid unique list offsets", len(lists))
    m.hist(
        "grid list allocation is aligned128 used extent",
        list_size == align(last_list_end, 128),
    )
    for index, expected in enumerate(tbc_spans):
        at = GBD + desc_at + index * 16
        require(
            (u32(data, at), u32(data, at + 4)) == expected,
            "TBC descriptor differs from chunk chain",
        )
        m.hist("TBC descriptor reserved half", (u32(data, at + 8), u32(data, at + 12)))
    m.counts["grid cells"] += dimension**3
    m.counts["resources"] += 1
    m.counts["triangles"] += tc
    return {
        "vertices": vc,
        "triangles": tc,
        "tbcs": tbc_count,
        "tbds": len(trees),
        "nodes": sum(t.nodes for t in trees),
        "maximum_tree_nodes": max(t.nodes for t in trees),
        "maximum_tree_references": max(len(t.references) for t in trees),
        "nodes_exceeding_1_01_quantized_units": (m.counts["nodes"] - initial_nodes)
        - (
            m.counts["nodes enclosing geometry tolerance 1.01 quantized units"]
            - initial_enclosing
        ),
        "depth": max(t.depth for t in trees),
        "grid_dimension": dimension,
        "no_optional_tails": no_tail,
        "reference_coverage_complete": coverage,
        "maximum_reference_multiplicity": multiplicity,
        "surface_ranges_outside_referenced_geometry": m.counts[
            "surface ranges outside referenced geometry"
        ]
        - initial_surface_failures,
    }


def resources(root: pathlib.Path, m: Measurements):
    stack = [(root, "")]
    while stack:
        directory, prefix = stack.pop()
        try:
            with os.scandir(directory) as entries:
                for entry in entries:
                    metadata = entry.stat(follow_symlinks=False)
                    if (
                        stat.S_ISLNK(metadata.st_mode)
                        or getattr(metadata, "st_file_attributes", 0)
                        & stat.FILE_ATTRIBUTE_REPARSE_POINT
                    ):
                        m.counts["skipped links or reparse points"] += 1
                        continue
                    relative = prefix + entry.name
                    if entry.is_dir(follow_symlinks=False):
                        stack.append((pathlib.Path(entry.path), relative + "/"))
                    elif RESOURCE_PATH.fullmatch(relative):
                        yield (
                            pathlib.Path(entry.path),
                            int(relative.replace("/", "")[:-4], 16),
                        )
        except OSError:
            m.counts["directory read errors"] += 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client-root", required=True, type=pathlib.Path)
    args = parser.parse_args()
    m = Measurements()
    identities, anomalies = [], []
    root = args.client_root / "data"
    try:
        require(root.is_dir(), "explicit client root has no data directory")
        for parent in (root, *root.parents):
            require(
                not parent.is_symlink() and not parent.is_junction(),
                "input path traverses a link or junction",
            )
    except (OSError, ValueError):
        print("census: explicit input directory unavailable or linked", file=sys.stderr)
        return 2
    for path, rid in resources(root, m):
        m.counts["numeric DATs inspected"] += 1
        try:
            with path.open("rb") as handle:
                head = handle.read(8)
                if head != b"SEDBPHB\0":
                    continue
                m.counts["PHB signatures observed"] += 1
                data = head + handle.read()
            identity = {
                "id": f"0x{rid:08X}",
                "size": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
            identities.append(identity)
            identity.update(analyze_phb(data, m))
        except OSError:
            anomalies.append({"id": f"0x{rid:08X}", "error": "resource read failed"})
        except (ValueError, struct.error) as error:
            anomalies.append({"id": f"0x{rid:08X}", "error": str(error)})
    identities.sort(key=lambda row: row["id"])
    digest = hashlib.sha256()
    for row in identities:
        digest.update(f"{row['id']} {row['size']} {row['sha256']}\n".encode("ascii"))
    report = m.report()
    report.update(
        {
            "identity_list_sha256": digest.hexdigest(),
            "identities": identities,
            "anomalies": anomalies,
        }
    )
    print(json.dumps(report, indent=2, sort_keys=True))
    return int(bool(anomalies or m.counts["directory read errors"] or not identities))


if __name__ == "__main__":
    sys.exit(main())
