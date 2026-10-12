#!/usr/bin/env python3
"""Summarize one zone written by `xivl export-zones`.

This is the research command that produced the zone collision measurements
in docs/formats/zone-geometry.md. It is not part of validation, it makes no
support claim, and it never runs in CI.

    xivl export-zones <install> --output <dir> --layout <resource-id>
    python tools/research/zone_export_stats.py --zone <dir>/zones/<name>

`--zone` is required and has no default. It names the shared base path of
the exporter's `<name>.obj` and `<name>.metadata.json`; the command reads
those two files and nothing else. It prints counts only.

Three passes:

- the placement pass counts placements by classification and CollisionBox
  placements by their raw classification word;
- the orientation pass counts OBJ faces whose geometric normal
  `(v1 - v0) x (v2 - v0)` points below the horizontal, and zero-area faces;
- the slope pass groups PHB hull faces by raw surface value and counts
  faces within 30 degrees of horizontal and of vertical.
"""

from __future__ import annotations

import argparse
import collections
import json
import math
import pathlib
import sys

NEAR = math.cos(math.radians(30))
STEEP = math.sin(math.radians(30))


def read_faces(obj_path: pathlib.Path):
    """Yield each OBJ face's three positions in file order."""
    vertices: list[tuple[float, float, float]] = []
    with open(obj_path, encoding="ascii") as handle:
        for line in handle:
            if line.startswith("v "):
                _, x, y, z = line.split()[:4]
                vertices.append((float(x), float(y), float(z)))
            elif line.startswith("f "):
                corners = line.split()[1:4]
                yield tuple(vertices[int(c.split("/")[0]) - 1] for c in corners)


def normal(face) -> tuple[float, float, float]:
    a, b, c = face
    u = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
    w = (c[0] - a[0], c[1] - a[1], c[2] - a[2])
    return (
        u[1] * w[2] - u[2] * w[1],
        u[2] * w[0] - u[0] * w[2],
        u[0] * w[1] - u[1] * w[0],
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--zone",
        required=True,
        type=pathlib.Path,
        help="exported zone base path without extension; required",
    )
    args = parser.parse_args()
    obj_path = args.zone.with_name(args.zone.name + ".obj")
    meta_path = args.zone.with_name(args.zone.name + ".metadata.json")
    if not obj_path.is_file() or not meta_path.is_file():
        print(
            "zone stats: missing {0} or {1}".format(obj_path, meta_path),
            file=sys.stderr,
        )
        return 2
    with open(meta_path, encoding="utf-8") as handle:
        metadata = json.load(handle)

    placements: collections.Counter[str] = collections.Counter()
    for placement in metadata["placements"]:
        placements["+".join(placement["classifications"])] += 1

    hull_surface: dict[int, int] = {}
    boxes: dict[int, object] = {}
    kinds: collections.Counter[str] = collections.Counter()
    for face_range in metadata["faceRanges"]:
        source = face_range["source"]
        kinds[source["kind"]] += face_range["faceCount"]
        start = face_range["faceStart"]
        if source["kind"] == "phb-hull":
            for face in range(start, start + face_range["faceCount"]):
                hull_surface[face] = source["rawSurface"]
        else:
            boxes[source["placementIndex"]] = source["classificationRaw"]
    words = collections.Counter(
        "0x{0:08X}".format(w) if isinstance(w, int) else str(w) for w in boxes.values()
    )

    total = downward = degenerate = 0
    slopes: dict[int, list[int]] = collections.defaultdict(lambda: [0, 0, 0])
    for index, face in enumerate(read_faces(obj_path), start=1):
        total += 1
        n = normal(face)
        length = math.sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2])
        if length == 0.0:
            degenerate += 1
            continue
        if n[1] < 0.0:
            downward += 1
        surface = hull_surface.get(index)
        if surface is None:
            continue
        upright = abs(n[1]) / length
        row = slopes[surface]
        row[0] += 1
        row[1] += upright >= NEAR
        row[2] += upright <= STEEP

    print("placements by classification:")
    for name, count in sorted(placements.items()):
        print("  {0:<24} {1}".format(name, count))
    print("faces by source kind:")
    for name, count in sorted(kinds.items()):
        print("  {0:<24} {1}".format(name, count))
    print("collision-box placements by raw classification word:")
    for name, count in sorted(words.items()):
        print("  {0:<24} {1}".format(name, count))
    print("obj faces: {0}".format(total))
    print("  downward geometric normal: {0}".format(downward))
    print("  zero area: {0}".format(degenerate))
    print("hull faces by raw surface (nonzero area):")
    print("  surface  faces  within-30-of-horizontal  within-30-of-vertical")
    for surface in sorted(slopes):
        faces, flat, steep = slopes[surface]
        print("  {0:>7}  {1:>6}  {2:>23}  {3:>21}".format(surface, faces, flat, steep))
    return 0


if __name__ == "__main__":
    sys.exit(main())
