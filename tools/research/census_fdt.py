#!/usr/bin/env python3
"""Census the GLYP records of the bitmap font containers in a 1.x install.

This is the research command that produced the census in
docs/formats/fdt.md. It is not part of the gate, it makes no support claim,
and it never runs in CI.

    python tools/research/census_fdt.py --client-root <install>

`--client-root` is required and has no default. Every file below its
`data/` directory whose first 512 bytes contain `VERS` followed by `DDSF`,
`GTEX`, or `FSIZ` is walked as tag, little-endian length, and payload.

For each container it reads the 14-byte GLYP records with the layout that a
nonzero VERS selects and checks them against the texture chunks:

- byte 0, the resource index, and byte 0 plus TEXC lie inside the DDSF and
  GTEX chunk list;
- byte 1, the selector, is one of the four selector vectors;
- bytes 4 and 5, the extents, are nonnegative as signed bytes;
- words 0x0A and 0x0C plus the extents lie inside the width and height of
  the GTEX header that byte 0 selects;
- bytes 0x07, 0x08, and 0x09 are counted by distinct value, noting when
  every record holds zero.

It prints paths, counts, and check results. No glyph value is printed.
"""

from __future__ import annotations

import argparse
import pathlib
import struct

TEXTURE_TAGS = (b"DDSF", b"GTEX")
RECORD_LENGTH = 14
SELECTOR_COUNT = 4


def chunks(data: bytes) -> list[tuple[bytes, bytes]]:
    found = []
    offset = 0
    while offset + 8 <= len(data):
        tag = data[offset : offset + 4]
        (length,) = struct.unpack_from("<I", data, offset + 4)
        if offset + 8 + length > len(data):
            break
        found.append((tag, data[offset + 8 : offset + 8 + length]))
        offset += 8 + length
    return found


def selected(data: bytes) -> bool:
    head = data[:512]
    start = head.find(b"VERS")
    return start >= 0 and any(
        head.find(tag, start) > start for tag in (b"DDSF", b"GTEX", b"FSIZ")
    )


def gtex_dimensions(payload: bytes) -> tuple[int, int] | None:
    if len(payload) < 0x0E or payload[:4] != b"GTEX":
        return None
    return struct.unpack_from(">HH", payload, 0x0A)


def census(path: pathlib.Path, relative: str) -> None:
    data = path.read_bytes()
    found = chunks(data)
    tags = [tag for tag, _ in found]
    values = {tag: payload for tag, payload in found}
    (version,) = struct.unpack("<I", values[b"VERS"])
    texc = struct.unpack("<I", values[b"TEXC"])[0] if b"TEXC" in values else 0
    textures = [payload for tag, payload in found if tag in TEXTURE_TAGS]
    dimensions = [gtex_dimensions(payload) for payload in textures]
    glyphs = values[b"GLYP"]
    count = len(glyphs) // RECORD_LENGTH

    failures = {
        "resource": 0,
        "offset-resource": 0,
        "selector": 0,
        "extent": 0,
        "source-x": 0,
        "source-y": 0,
        "dimension-mismatch": 0,
    }
    distinct = {0x07: set(), 0x08: set(), 0x09: set()}
    resources = set()
    selectors = set()
    for index in range(count):
        record = glyphs[index * RECORD_LENGTH : (index + 1) * RECORD_LENGTH]
        resource, selector, _, _, width, height = struct.unpack_from(
            "<bBbbbb", record, 0
        )
        source_x, source_y = struct.unpack_from("<HH", record, 0x0A)
        resources.add(resource)
        selectors.add(selector)
        for offset in distinct:
            distinct[offset].add(record[offset])
        if not 0 <= resource < len(textures):
            failures["resource"] += 1
            continue
        if not 0 <= resource + texc < len(textures):
            failures["offset-resource"] += 1
        if selector >= SELECTOR_COUNT:
            failures["selector"] += 1
        if width < 0 or height < 0:
            failures["extent"] += 1
        size = dimensions[resource]
        if size is None:
            failures["dimension-mismatch"] += 1
            continue
        if source_x + width > size[0]:
            failures["source-x"] += 1
        if source_y + height > size[1]:
            failures["source-y"] += 1

    print(relative)
    print(
        "  version {0}, tags {1}".format(
            version, " ".join(tag.decode("ascii", "replace") for tag in tags)
        )
    )
    print(
        "  textures {0}, TEXC {1}, dimensions {2}".format(
            len(textures), texc, dimensions
        )
    )
    print(
        "  records {0}, resource indexes {1}, selectors {2}".format(
            count, sorted(resources), sorted(selectors)
        )
    )
    print(
        "  distinct values: {0}".format(
            ", ".join(
                "byte {0:#04x} {1}{2}".format(
                    offset, len(values), " (all zero)" if values == {0} else ""
                )
                for offset, values in distinct.items()
            )
        )
    )
    print("  check failures {0}".format(failures))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--client-root", required=True, type=pathlib.Path)
    args = parser.parse_args()
    data_root = args.client_root / "data"
    for path in sorted(data_root.rglob("*.DAT")):
        with path.open("rb") as handle:
            head = handle.read(512)
        if selected(head):
            census(path, path.relative_to(args.client_root).as_posix())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
