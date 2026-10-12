#!/usr/bin/env python3
"""Census RES -> wrb chunk trees in an explicitly supplied 1.x install.

Research only: makes no support claim and never runs in CI. Requires
--client-root with no default, discovery, or environment fallback. Reads
client files read-only and never writes client bytes anywhere. Output is
aggregate measurements, offsets, resource IDs, sizes and SHA-256 identities;
strings are limited to chunk tags; model names are never printed.

    python tools/research/census_wrb.py --client-root <install>
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from collections.abc import Iterator
from dataclasses import dataclass, field
import hashlib
import math
import os
from pathlib import Path
import re
import struct
import sys

RESOURCE_PATH = re.compile(
    r"^[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}/[0-9A-Fa-f]{2}\.DAT$"
)
CONTAINERS = {b"WRB\0", b"MDL\0", b"MDLC", b"MESH", b"AABB"}
SAMPLES = {0x89940554, 0x899500BF, 0x897D03A9, 0x72E90455, 0x72E50007}
PGRP_RESOURCE = 0x897D0398
Vec3 = tuple[float, float, float]
Box = tuple[float, float, float, float, float, float]


class Malformed(ValueError):
    def __init__(self, offset: int, reason: str):
        super().__init__(reason)
        self.offset = offset
        self.reason = reason


def need(data: bytes, at: int, size: int, end: int) -> None:
    if at < 0 or size < 0 or end > len(data) or at + size > end:
        raise Malformed(at, "bounded extent")


def words(
    data: bytes, at: int, count: int, end: int, endian: str = ">"
) -> tuple[int, ...]:
    need(data, at, count * 4, end)
    return struct.unpack_from(endian + "I" * count, data, at)


def tag_name(tag: bytes) -> str:
    return "".join(
        chr(value) if 32 <= value < 127 else f"\\x{value:02x}"
        for value in tag.rstrip(b"\0")
    )


@dataclass
class Chunk:
    tag: bytes
    at: int
    size: int
    unknown: int
    padded: int
    children: list[Chunk] = field(default_factory=list)

    @property
    def body(self) -> int:
        return self.at + 16

    @property
    def end(self) -> int:
        return self.at + self.size


def parse_chunks(
    data: bytes,
    start: int,
    end: int,
    depth: int = 0,
    counts: Counter[str] | None = None,
) -> list[Chunk]:
    if depth > 32:
        raise Malformed(start, "chunk depth")
    need(data, start, end - start, len(data))
    result: list[Chunk] = []
    at = start
    while at < end:
        if data[at] == 0:
            nonzero = sum(value != 0 for value in data[at + 1 : end])
            if counts is not None and nonzero:
                counts["nonzero tail regions"] += 1
                counts["nonzero tail bytes"] += nonzero
            break
        need(data, at, 16, end)
        unknown, size, padded = words(data, at + 4, 3, end)
        if size < 16:
            raise Malformed(at + 8, "chunk size below header")
        need(data, at, size, end)
        advance = padded if padded >= size else (size + 15) & ~15
        need(data, at, advance, end)
        chunk = Chunk(data[at : at + 4], at, size, unknown, padded)
        if chunk.tag in CONTAINERS:
            need(data, chunk.body, 16, chunk.end)
            # A 0x28 AABB leaf has six floats, with an eight-byte opaque tail
            # under the generic container framing. It has no child headers.
            if chunk.tag != b"AABB" or size != 0x28:
                chunk.children = parse_chunks(
                    data, at + 32, at + size, depth + 1, counts
                )
        result.append(chunk)
        at += advance
    return result


def flatten(
    chunks: list[Chunk], parent: bytes = b"wrb"
) -> Iterator[tuple[Chunk, bytes]]:
    for chunk in chunks:
        yield chunk, parent
        yield from flatten(chunk.children, chunk.tag)


def children(chunk: Chunk, tag: bytes) -> list[Chunk]:
    return [child for child in chunk.children if child.tag == tag]


@dataclass
class Stream:
    chunk: Chunk
    items: int
    stride: int
    unknown: int
    descriptors: list[tuple[int, int, int, int]]
    start: int
    exact: bool

    def fields(self, usage: int) -> list[tuple[int, int, int, int]]:
        return [desc for desc in self.descriptors if desc[3] >> 16 == usage]


def parse_stream(data: bytes, chunk: Chunk) -> Stream:
    count, items, stride, unknown = words(data, chunk.body, 4, chunk.end)
    start = chunk.body + 16 + 16 * count
    need(data, chunk.body + 16, 16 * count, chunk.end)
    need(data, start, items * stride, chunk.end)
    descriptors = [
        words(data, chunk.body + 16 + 16 * i, 4, chunk.end) for i in range(count)
    ]
    return Stream(
        chunk,
        items,
        stride,
        unknown,
        descriptors,
        start,
        start + items * stride == chunk.end,
    )


def read_box(data: bytes, chunk: Chunk) -> Box:
    need(data, chunk.body, 24, chunk.end)
    box = struct.unpack_from(">6f", data, chunk.body)
    if not all(math.isfinite(value) for value in box):
        raise Malformed(chunk.body, "nonfinite box")
    return box


def boxes(data: bytes, owner: Chunk) -> tuple[Box | None, Box | None]:
    aabb = comp = None
    for container in children(owner, b"AABB"):
        for child in container.children:
            if child.tag == b"AABB":
                aabb = read_box(data, child)
            elif child.tag == b"COMP":
                comp = read_box(data, child)
    return aabb, comp


def decoded(raw: list[tuple[int, int, int, int]], box: Box, divisor: int) -> list[Vec3]:
    center = [(box[i] + box[i + 3]) * 0.5 for i in range(3)]
    half = [(box[i + 3] - box[i]) * 0.5 for i in range(3)]
    return [
        (
            center[0] + value[0] / divisor * half[0],
            center[1] + value[1] / divisor * half[1],
            center[2] + value[2] / divisor * half[2],
        )
        for value in raw
    ]


def bounds_error(raw_bounds: list[int], comp: Box, box: Box, divisor: int) -> float:
    errors = []
    for axis in range(3):
        center = (comp[axis] + comp[axis + 3]) * 0.5
        half = (comp[axis + 3] - comp[axis]) * 0.5
        low = center + raw_bounds[axis] / divisor * half
        high = center + raw_bounds[axis + 3] / divisor * half
        errors.extend(
            (abs(min(low, high) - box[axis]), abs(max(low, high) - box[axis + 3]))
        )
    return max(errors)


def face_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3:
    u = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
    v = (c[0] - a[0], c[1] - a[1], c[2] - a[2])
    return (
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    )


def text_shape(data: bytes, chunk: Chunk) -> bool:
    body = data[chunk.body : chunk.end]
    stop = body.find(b"\0")
    return (
        stop > 0
        and all(32 <= value < 127 for value in body[:stop])
        and not any(body[stop:])
    )


def walk_resources(root: Path) -> Iterator[tuple[Path, int]]:
    stack = [(root, "")]
    while stack:
        directory, prefix = stack.pop()
        with os.scandir(directory) as entries:
            for entry in sorted(entries, key=lambda value: value.name):
                relative = prefix + entry.name
                if entry.is_dir(follow_symlinks=False):
                    stack.append((Path(entry.path), relative + "/"))
                elif entry.is_file(follow_symlinks=False) and RESOURCE_PATH.fullmatch(
                    relative
                ):
                    yield Path(entry.path), int(relative.replace("/", "")[:-4], 16)


class Census:
    def __init__(self) -> None:
        self.n: Counter[str] = Counter()
        self.tags: Counter[str] = Counter()
        self.parents: dict[str, Counter[str]] = defaultdict(Counter)
        self.padded_zero: Counter[str] = Counter()
        self.layouts: Counter[tuple[str, int, int]] = Counter()
        self.descriptors: Counter[tuple[int, int, int, int]] = Counter()
        self.position_offsets: Counter[int] = Counter()
        self.head_sizes: Counter[tuple[str, int]] = Counter()
        self.aabb_forms: Counter[tuple[int, int, str]] = Counter()
        self.anomalies: Counter[str] = Counter()
        self.examples: dict[str, list[tuple[int, int]]] = defaultdict(list)
        self.identities: dict[int, tuple[int, str]] = {}
        self.no_wrb: list[tuple[int, Counter[str]]] = []
        self.max_error = {32767: 0.0, 32768: 0.0}
        self.max_error_at: dict[int, tuple[int, int]] = {}
        self.sample: Counter[str] = Counter()
        self.groups: list[str] = []

    def identify(self, rid: int, data: bytes) -> None:
        if rid not in self.identities:
            self.identities[rid] = (len(data), hashlib.sha256(data).hexdigest())

    def anomaly(self, reason: str, rid: int, data: bytes, at: int) -> None:
        self.anomalies[reason] += 1
        if len(self.examples[reason]) < 3:
            self.examples[reason].append((rid, at))
            self.identify(rid, data)

    def resource(self, rid: int, data: bytes) -> None:
        need(data, 0, 0x40, len(data))
        header = struct.unpack_from("<H", data, 0x0E)[0]
        count = words(data, 0x30, 1, len(data), "<")[0]
        repeat = words(data, 0x38, 1, len(data), "<")[0]
        if header not in (0x30, 0x40) or count != repeat:
            raise Malformed(0x30, "RES header/count")
        if header == 0x30:
            self.anomaly("RES headerSize 0x30 with directory at 0x40", rid, data, 0x0E)
        # The directory starts at 0x40 even when declared headerSize is 0x30.
        base = 0x40 + 16 * count
        need(data, 0x40, count * 16, len(data))
        trees: list[list[Chunk]] = []
        subtypes: Counter[str] = Counter()
        for i in range(count):
            _, offset, size, _ = words(data, 0x40 + i * 16, 4, len(data), "<")
            start = base + offset
            end = start + size
            if end > len(data):
                self.n["RES clamped entries"] += 1
                self.n["RES max clamp"] = max(self.n["RES max clamp"], end - len(data))
                if i != count - 1:
                    self.anomaly("nonfinal RES clamp", rid, data, start)
                end = len(data)
            need(data, start, end - start, len(data))
            if end - start < 8 or data[start : start + 4] != b"SEDB":
                continue
            subtype = data[start + 4 : start + 8]
            subtypes[tag_name(subtype)] += 1
            if subtype != b"wrb\0":
                continue
            need(data, start, 0x14, end)
            child_header = struct.unpack_from("<H", data, start + 0x0E)[0]
            extent = words(data, start + 0x10, 1, end, "<")[0]
            if child_header < 0x14 or extent < child_header:
                raise Malformed(start + 0x0E, "wrb SEDB extent")
            need(data, start, extent, end)
            trees.append(
                parse_chunks(data, start + child_header, start + extent, counts=self.n)
            )
        self.n["parsed RES"] += 1
        self.n["wrb trees"] += len(trees)
        if not trees:
            self.no_wrb.append((rid, subtypes))
            self.identify(rid, data)
        if rid in SAMPLES or rid == PGRP_RESOURCE:
            self.identify(rid, data)
        mesh_index = 1
        model_boxes: list[tuple[Box | None, Box | None]] = []
        for tree in trees:
            streams: dict[int, Stream] = {}
            for chunk, parent in flatten(tree):
                tag = tag_name(chunk.tag)
                self.tags[tag] += 1
                if rid == 0x89940674 and chunk.tag in (b"MDL\0", b"MESH"):
                    self.sample[f"89940674 {tag}s"] += 1
                self.parents[tag][tag_name(parent)] += 1
                self.n["chunks"] += 1
                self.n["nonzero +4"] += chunk.unknown != 0
                if chunk.padded == 0:
                    self.padded_zero[tag] += 1
                if chunk.tag == b"HEAD":
                    self.head_sizes[(tag_name(parent), chunk.size)] += 1
                if chunk.tag in (b"AABB", b"COMP") and chunk.size == 0x28:
                    box = read_box(data, chunk)
                    if any(box[i] > box[i + 3] for i in range(3)):
                        self.anomaly("inverted box", rid, data, chunk.body)
                if chunk.tag == b"AABB" and chunk.children:
                    info = words(data, chunk.body, 1, chunk.end)[0]
                    shape = ",".join(tag_name(child.tag) for child in chunk.children)
                    self.aabb_forms[(chunk.size, info, shape)] += 1
                    self.n["AABB form exceptions"] += not (
                        (
                            chunk.size == 0x50
                            and info == 1
                            and shape == "AABB"
                            and chunk.children[0].size == 0x28
                        )
                        or (
                            chunk.size == 0x80
                            and info == 2
                            and shape == "AABB,COMP"
                            and all(c.size == 0x28 for c in chunk.children)
                        )
                    )
                if chunk.tag == b"STMS":
                    try:
                        stream = parse_stream(data, chunk)
                        streams[chunk.at] = stream
                    except Malformed as error:
                        self.anomaly(error.reason, rid, data, error.offset)
                        continue
                    self.n["STMS parsed"] += 1
                    self.n["STMS size exceptions"] += not stream.exact
                    self.n["STMS unknownD nonzero"] += stream.unknown != 0
                    kind = (
                        "index"
                        if stream.fields(255)
                        else "vertex"
                        if stream.fields(0)
                        else "other"
                    )
                    self.layouts[(kind, len(stream.descriptors), stream.stride)] += 1
                    if kind == "index":
                        self.n["index layout exceptions"] += (
                            len(stream.descriptors) != 1 or stream.stride != 2
                        )
                    for offset, fmt, lanes, usage in stream.descriptors:
                        self.descriptors[(fmt, lanes, usage >> 16, usage & 0xFFFF)] += 1
                        if usage >> 16 == 0:
                            self.position_offsets[offset] += 1
                            self.n["position descriptor exceptions"] += (
                                offset,
                                fmt,
                                lanes,
                                usage,
                            ) != (0, 4, 4, 0)
                if chunk.tag == b"NAME" and parent == b"MDL\0":
                    self.n["MDL NAME strings"] += text_shape(data, chunk)
                if chunk.tag == b"STR\0" and parent == b"MESH":
                    self.n["MESH STR strings"] += text_shape(data, chunk)
            for mdl, _ in flatten(tree):
                if mdl.tag != b"MDL\0":
                    continue
                try:
                    aabb, comp = boxes(data, mdl)
                    model_boxes.append((aabb, comp))
                    self.n["MDL box pairs"] += aabb is not None and comp is not None
                    self.n["MDL AABB equals COMP"] += aabb is not None and aabb == comp
                    for mesh in children(mdl, b"MESH"):
                        try:
                            self.mesh(rid, data, mdl, mesh, streams, comp, mesh_index)
                        except Malformed as error:
                            self.n["mesh measurement failures"] += 1
                            self.anomaly(error.reason, rid, data, error.offset)
                        mesh_index += 1
                except Malformed as error:
                    self.anomaly(error.reason, rid, data, error.offset)
        if rid == 0x72E50007:
            shared = (
                len(model_boxes) == 2
                and model_boxes[0][1] is not None
                and model_boxes[0][1] == model_boxes[1][1]
            )
            differ = (
                len(model_boxes) == 2
                and model_boxes[0][0] is not None
                and model_boxes[0][0] != model_boxes[1][0]
            )
            self.sample["72E50007 MDLs"] = len(model_boxes)
            self.sample["72E50007 shared COMP"] = int(shared)
            self.sample["72E50007 differing AABB"] = int(differ)

    def mesh(
        self,
        rid: int,
        data: bytes,
        mdl: Chunk,
        mesh: Chunk,
        streams: dict[int, Stream],
        comp: Box | None,
        mesh_index: int,
    ) -> None:
        self.n["mesh measured"] += 1
        for tag, size in (
            (b"RSID", 0x20),
            (b"RSTP", 0x14),
            (b"PRID", 0x20),
            (b"PRTP", 0x14),
        ):
            found = children(mesh, tag)
            self.n["ID chunks checked"] += len(found)
            self.n["ID multiplicity exceptions"] += len(found) != 1
            self.n["ID size exceptions"] += sum(child.size != size for child in found)
            self.n["ID nonzero payloads"] += sum(
                any(data[child.body : child.end]) for child in found
            )
        groups = children(mesh, b"PGRP")
        self.n["MESH without PGRP"] += not groups
        mesh_streams = [
            streams[child.at]
            for child in children(mesh, b"STMS")
            if child.at in streams
        ]
        indices = [stream for stream in mesh_streams if stream.fields(255)]
        vertices = [stream for stream in mesh_streams if stream.fields(0)]
        if len(indices) != 1 or len(vertices) != 1:
            raise Malformed(mesh.at, "mesh stream multiplicity")
        index, vertex = indices[0], vertices[0]
        if index.stride != 2 or index.descriptors != [(0, 0, 1, 0xFF0000)]:
            raise Malformed(index.chunk.at, "index descriptor")
        position = vertex.fields(0)
        if (
            len(position) != 1
            or position[0][1:3] != (4, 4)
            or position[0][0] + 8 > vertex.stride
        ):
            raise Malformed(vertex.chunk.at, "position descriptor")
        raw = [
            struct.unpack_from(
                ">4h", data, vertex.start + i * vertex.stride + position[0][0]
            )
            for i in range(vertex.items)
        ]
        self.n["vertices"] += vertex.items
        self.n["fourth lane exceptions"] += sum(value[3] != 1 for value in raw)
        index_values = [
            value[0]
            for value in struct.iter_unpack(
                ">H", data[index.start : index.start + index.items * 2]
            )
        ]
        self.n["indices"] += index.items
        self.n["index range exceptions"] += sum(
            value >= vertex.items for value in index_values
        )
        heads = children(mesh, b"HEAD")
        self.n["MESH HEAD exceptions"] += len(heads) != 1 or any(
            head.size != 0x20 for head in heads
        )
        if len(heads) == 1 and heads[0].size >= 0x1C:
            head_vertices, head_triangles = struct.unpack_from(
                ">2H", data, heads[0].body + 8
            )
            self.n["HEAD vertex count exceptions"] += head_vertices != vertex.items
            self.n["HEAD triangle count exceptions"] += (
                3 * head_triangles != index.items
            )
            self.n["HEAD count comparisons"] += 1
        self.pgrp(rid, data, mdl, mesh, groups, mesh_index, index.items // 3)
        if comp is None or not raw:
            raise Malformed(mesh.at, "missing COMP or empty vertices")
        aabb, _ = boxes(data, mesh)
        if aabb is None:
            raise Malformed(mesh.at, "missing MESH AABB")
        positions = decoded(raw, comp, 32767)
        raw_bounds = [
            operation(value[axis] for value in raw)
            for operation in (min, max)
            for axis in range(3)
        ]
        errors = {
            divisor: bounds_error(raw_bounds, comp, aabb, divisor)
            for divisor in (32767, 32768)
        }
        self.n["bounds comparisons"] += 1
        choice = (
            "32767 better"
            if errors[32767] < errors[32768]
            else "32768 better"
            if errors[32768] < errors[32767]
            else "bounds tie"
        )
        self.n[choice] += 1
        for divisor, error in errors.items():
            if error > self.max_error[divisor]:
                self.max_error[divisor] = error
                self.max_error_at[divisor] = (rid, mesh.at)
                self.identify(rid, data)
        if rid == 0x89940554 and mesh_index == 3:
            self.sample["89940554 mesh3 raw z max"] = max(value[2] for value in raw)
            self.sample["89940554 mesh3 raw z min"] = min(value[2] for value in raw)
            self.sample["89940554 mesh3 raw z abs"] = max(
                abs(value[2]) for value in raw
            )
        if index.items % 3 or any(value >= vertex.items for value in index_values):
            raise Malformed(index.chunk.at, "invalid triangle list")
        normals: dict[int, list[Vec3]] = {}
        for usage in (2, 3):
            fields = vertex.fields(usage)
            self.n[f"usage{usage} descriptor exceptions"] += len(fields) != 1 or any(
                desc[1:3] != (3, 4) for desc in fields
            )
            if len(fields) == 1 and fields[0][1:3] == (3, 4):
                offset = fields[0][0]
                if offset + 4 > vertex.stride:
                    raise Malformed(vertex.chunk.at, "normal field extent")
                normals[usage] = [
                    tuple(
                        data[vertex.start + i * vertex.stride + offset + axis] / 127.5
                        - 1
                        for axis in range(3)
                    )
                    for i in range(vertex.items)
                ]
        is_sample = rid in SAMPLES
        if is_sample:
            self.sample["meshes"] += 1
        edges: dict[tuple[int, int], list[tuple[int, int]]] = defaultdict(list)
        agreement = {usage: [0, 0, 0] for usage in normals}
        degenerate_count = 0
        for at in range(0, index.items, 3):
            a, b, c = index_values[at], index_values[at + 1], index_values[at + 2]
            face = face_normal(positions[a], positions[b], positions[c])
            degenerate = face == (0.0, 0.0, 0.0)
            degenerate_count += degenerate
            for usage, values in normals.items():
                na, nb, nc = values[a], values[b], values[c]
                dot = (
                    face[0] * (na[0] + nb[0] + nc[0])
                    + face[1] * (na[1] + nb[1] + nc[1])
                    + face[2] * (na[2] + nb[2] + nc[2])
                ) / 3
                agreement[usage][0 if dot > 0 else 1 if dot == 0 else 2] += 1
            if rid == 0x89940554:
                for x, y in ((a, b), (b, c), (c, a)):
                    edges[(min(x, y), max(x, y))].append((x, y))
                self.sample["89940554 degenerate"] += degenerate
        self.n["triangles"] += index.items // 3
        self.n["degenerate faces"] += degenerate_count
        if is_sample:
            self.sample["triangles"] += index.items // 3
            self.sample["degenerate faces"] += degenerate_count
        for usage, (positive, zero, negative) in agreement.items():
            self.n[f"usage{usage} triangles"] += index.items // 3
            self.n[f"usage{usage} positive"] += positive
            self.n[f"usage{usage} zero"] += zero
            self.n[f"usage{usage} negative"] += negative
            if is_sample:
                self.sample[f"usage{usage} positive"] += positive
                self.sample[f"usage{usage} triangles"] += index.items // 3
        if rid == 0x89940554:
            self.sample["89940554 vertices"] += vertex.items
            self.sample["89940554 triangles"] += index.items // 3
            self.sample["89940554 interior edges"] += sum(
                len(values) == 2 for values in edges.values()
            )
            self.sample["89940554 opposite edges"] += sum(
                len(values) == 2 and values[0] == values[1][::-1]
                for values in edges.values()
            )
            self.sample["89940554 nonmanifold edges"] += sum(
                len(values) > 2 for values in edges.values()
            )

    def pgrp(
        self,
        rid: int,
        data: bytes,
        mdl: Chunk,
        mesh: Chunk,
        groups: list[Chunk],
        mesh_index: int,
        triangles: int,
    ) -> None:
        all_refs: list[int] = []
        for group in groups:
            count, index_offset, name_offset = words(data, group.body, 3, group.end)
            need(data, group.body + index_offset, count * 2, group.end)
            need(data, group.body + name_offset, 0, group.end)
            refs = [
                value[0]
                for value in struct.iter_unpack(
                    ">H",
                    data[
                        group.body + index_offset : group.body
                        + index_offset
                        + count * 2
                    ],
                )
            ]
            name = data[group.body + name_offset : group.end]
            stop = name.find(b"\0")
            name_chars = len(name) if stop < 0 else stop
            name_to_end = (
                name_chars > 0
                and all(32 <= value < 127 for value in name[:name_chars])
                and not any(name[name_chars:])
            )
            valid = (
                index_offset == 0x0C
                and index_offset + count * 2 == name_offset
                and name_to_end
            )
            self.n["PGRP layout exceptions"] += not valid
            self.n["PGRP other resources"] += rid != PGRP_RESOURCE
            self.n["PGRP out of range refs"] += sum(
                value >= triangles for value in refs
            )
            self.identify(rid, data)
            self.groups.append(
                f"W11 group resource=0x{rid:08X} mesh={mesh_index} offset=0x{group.at:X} triangleCount={count} indexOffset=0x{index_offset:X} nameOffset=0x{name_offset:X} nameBytes={len(name)} nameChars={name_chars} zeroTail={len(name) - name_chars} nameToEnd={int(name_to_end)}"
            )
            all_refs.extend(refs)
        if rid == PGRP_RESOURCE and groups:
            counts = [words(data, group.body, 1, group.end)[0] for group in groups]
            self.n["PGRP mesh assertions"] += 1
            self.n["PGRP partition exceptions"] += not (
                (
                    mesh_index == 1
                    and counts == [24, 144]
                    and triangles == 168
                    and sorted(all_refs) == list(range(triangles))
                )
                or (
                    mesh_index == 2
                    and counts == [32]
                    and triangles == 152
                    and len(set(all_refs)) == 32
                )
            )
            names = children(mdl, b"NAME")
            self.n["PGRP NAME exceptions"] += not names
            self.groups.append(
                f"W11 mesh resource=0x{rid:08X} mesh={mesh_index} MDL_has_NAME={bool(names)} triangles={triangles} groupRefs={len(all_refs)} uniqueRefs={len(set(all_refs))} partition={int(sorted(all_refs) == list(range(triangles)))}"
            )

    def report(self) -> None:
        n, s = self.n, self.sample

        def verdict(claim: str, condition: bool, detail: str) -> None:
            incomplete = (
                n["malformed RES"] or n["read errors"] or n["mesh measurement failures"]
            )
            status = (
                "CONFIRMED"
                if condition and not incomplete
                else "DIFFERS"
                if not condition
                else "NOT MEASURED"
            )
            print(
                f"{claim} {status} {detail}"
                + ("; incomplete coverage" if incomplete else "")
            )

        print(
            f"coverage files={n['files']} RES={n['RES']} parsed_RES={n['parsed RES']} malformed_RES={n['malformed RES']} read_errors={n['read errors']} mesh_failures={n['mesh measurement failures']}"
        )
        print(
            f"W1 RES={n['RES']} wrb_trees={n['wrb trees']} chunks={n['chunks']} no_wrb={len(self.no_wrb)}"
        )
        for tag, count in sorted(self.tags.items()):
            parent = ",".join(
                f"{key}:{value}" for key, value in sorted(self.parents[tag].items())
            )
            print(f"W1 tag={tag} count={count} parents={parent}")
        for rid, subtypes in self.no_wrb:
            print(
                f"W1 no_wrb resource=0x{rid:08X} direct_SEDB="
                + ",".join(f"{tag}:{count}" for tag, count in sorted(subtypes.items()))
            )
        verdict(
            "W2",
            n["nonzero +4"] == 0 and bool(self.padded_zero),
            f"chunks={n['chunks']} nonzero_at_04={n['nonzero +4']} padded_zero={sum(self.padded_zero.values())} by_tag={dict(sorted(self.padded_zero.items()))}",
        )
        print(
            f"W2 nonzero_after_terminator_regions={n['nonzero tail regions']} nonzero_after_terminator_bytes={n['nonzero tail bytes']}"
        )
        verdict(
            "W3",
            not any(
                n[key]
                for key in (
                    "ID multiplicity exceptions",
                    "ID size exceptions",
                    "ID nonzero payloads",
                )
            ),
            f"meshes={n['mesh measured']} ID_chunks={n['ID chunks checked']} multiplicity_exceptions={n['ID multiplicity exceptions']} size_exceptions={n['ID size exceptions']} nonzero_payloads={n['ID nonzero payloads']}",
        )
        verdict(
            "W4",
            n["STMS parsed"] == self.tags["STMS"]
            and not any(
                n[key]
                for key in (
                    "STMS size exceptions",
                    "STMS unknownD nonzero",
                    "index layout exceptions",
                )
            ),
            f"STMS={n['STMS parsed']} size_exceptions={n['STMS size exceptions']} unknownD_nonzero={n['STMS unknownD nonzero']} index_layout_exceptions={n['index layout exceptions']}",
        )
        for (kind, fields, stride), count in sorted(self.layouts.items()):
            print(
                f"W4 layout={kind} fieldCount={fields} stride=0x{stride:X} count={count}"
            )
        for (fmt, count, high, low), total in sorted(self.descriptors.items()):
            print(
                f"W4 descriptor format={fmt} count={count} usage_hi={high} usage_lo={low} total={total}"
            )
        print(f"W4 usage0_offsets={dict(sorted(self.position_offsets.items()))}")
        verdict(
            "W5",
            n["position descriptor exceptions"] == 0
            and n["fourth lane exceptions"] == 0,
            f"position_descriptor_exceptions={n['position descriptor exceptions']} vertices={n['vertices']} fourth_s16_not_1={n['fourth lane exceptions']}",
        )
        verdict(
            "W6",
            n["HEAD count comparisons"] == self.tags["MESH"]
            and not any(
                n[key]
                for key in (
                    "MESH HEAD exceptions",
                    "HEAD vertex count exceptions",
                    "HEAD triangle count exceptions",
                    "index range exceptions",
                )
            )
            and all(
                size in (0x2C, 0x30)
                for (parent, size) in self.head_sizes
                if parent == "MDL"
            ),
            f"HEAD_comparisons={n['HEAD count comparisons']} MESH_HEAD_exceptions={n['MESH HEAD exceptions']} vertex_count_exceptions={n['HEAD vertex count exceptions']} triangle_count_exceptions={n['HEAD triangle count exceptions']} out_of_range_indices={n['index range exceptions']} indices={n['indices']}",
        )
        for (parent, size), count in sorted(self.head_sizes.items()):
            print(f"W6 HEAD parent={parent} size=0x{size:X} count={count}")
        verdict(
            "W7",
            n["AABB form exceptions"] == 0,
            f"form_exceptions={n['AABB form exceptions']} MDL_box_pairs={n['MDL box pairs']} MDL_AABB_equals_COMP={n['MDL AABB equals COMP']} MDL_AABB_differs_COMP={n['MDL box pairs'] - n['MDL AABB equals COMP']}",
        )
        for (size, info, shape), count in sorted(self.aabb_forms.items()):
            print(
                f"W7 AABB size=0x{size:X} info_word={info} children={shape} count={count}"
            )
        print(
            f"W7 resource=0x89940674 MDLs={s['89940674 MDLs']} MESHes={s['89940674 MESHs']}"
        )
        verdict(
            "W8",
            n["bounds comparisons"] == self.tags["MESH"]
            and s["89940554 mesh3 raw z abs"] == 30876
            and s["72E50007 shared COMP"] == 1
            and s["72E50007 differing AABB"] == 1,
            f"meshes={n['bounds comparisons']} divisor32767_better={n['32767 better']} divisor32768_better={n['32768 better']} ties={n['bounds tie']} max_error32767={self.max_error[32767]:.12g} max_error32768={self.max_error[32768]:.12g}",
        )
        print(
            f"W8 resource=0x89940554 mesh=3 raw_z_min={s['89940554 mesh3 raw z min']} raw_z_max={s['89940554 mesh3 raw z max']} raw_z_abs_extent={s['89940554 mesh3 raw z abs']}"
        )
        print(
            f"W8 resource=0x72E50007 MDLs={s['72E50007 MDLs']} shared_COMP={s['72E50007 shared COMP']} differing_AABB={s['72E50007 differing AABB']}"
        )
        for divisor, (rid, at) in sorted(self.max_error_at.items()):
            print(
                f"W8 max_error divisor={divisor} resource=0x{rid:08X} MESH_offset=0x{at:X}"
            )
        print(
            "W8 metric=max_absolute_error_of_six_bounds; float64_decode_from_float32_COMP; mesh_numbers_one_based"
        )
        print(
            f"W9 sample_meshes={s['meshes']} sample_triangles={s['triangles']} usage2_positive={s['usage2 positive']}/{s['usage2 triangles']} usage3_positive={s['usage3 positive']}/{s['usage3 triangles']} sample_degenerate={s['degenerate faces']} usage2_descriptor_exceptions={n['usage2 descriptor exceptions']}"
        )
        print(
            f"W9 install triangles={n['triangles']} degenerate={n['degenerate faces']} usage2_positive={n['usage2 positive']}/{n['usage2 triangles']} usage2_zero={n['usage2 zero']} usage2_negative={n['usage2 negative']} usage3_positive={n['usage3 positive']}/{n['usage3 triangles']} usage3_zero={n['usage3 zero']} usage3_negative={n['usage3 negative']} usage3_descriptor_exceptions={n['usage3 descriptor exceptions']}"
        )
        print(
            "W9 degenerate=exact_zero_cross_product; degenerate_faces_included_in_dot_totals; normals=byte/127.5-1"
        )
        verdict(
            "W10",
            s["89940554 vertices"] == 1425
            and s["89940554 triangles"] == 896
            and s["89940554 interior edges"] == s["89940554 opposite edges"] == 645
            and s["89940554 degenerate"] == 0,
            f"resource=0x89940554 vertices={s['89940554 vertices']} triangles={s['89940554 triangles']} interior_shared_edges={s['89940554 interior edges']} opposite_edges={s['89940554 opposite edges']} degenerate={s['89940554 degenerate']} nonmanifold_edges={s['89940554 nonmanifold edges']}",
        )
        verdict(
            "W11",
            self.tags["PGRP"] == 3
            and n["PGRP mesh assertions"] == 2
            and not any(
                n[key]
                for key in (
                    "PGRP layout exceptions",
                    "PGRP other resources",
                    "PGRP NAME exceptions",
                    "PGRP out of range refs",
                    "PGRP partition exceptions",
                )
            ),
            f"PGRP={self.tags['PGRP']} other_resources={n['PGRP other resources']} layout_exceptions={n['PGRP layout exceptions']} NAME_exceptions={n['PGRP NAME exceptions']} out_of_range_refs={n['PGRP out of range refs']} partition_exceptions={n['PGRP partition exceptions']} MESH_without_PGRP={n['MESH without PGRP']}",
        )
        for line in self.groups:
            print(line)
        verdict(
            "W12",
            n["MDL NAME strings"] == self.tags["MDL"]
            and n["MESH STR strings"] == self.tags["MESH"],
            f"MDL={self.tags['MDL']} NAME={self.tags['NAME']} MDL_NAME_strings={n['MDL NAME strings']} MESH={self.tags['MESH']} STR={self.tags['STR']} MESH_STR_strings={n['MESH STR strings']}",
        )
        print(
            f"anomaly RES_clamped_entries={n['RES clamped entries']} max_clamp_bytes={n['RES max clamp']} additional={sum(self.anomalies.values())}"
        )
        for reason, count in sorted(self.anomalies.items()):
            examples = ",".join(
                f"0x{rid:08X}@0x{at:X}" for rid, at in self.examples[reason]
            )
            print(f"anomaly {reason} count={count} examples={examples}")
        named = (
            SAMPLES
            | {PGRP_RESOURCE}
            | {rid for rid, _ in self.no_wrb}
            | {rid for rid, _ in self.max_error_at.values()}
            | {rid for examples in self.examples.values() for rid, _ in examples}
        )
        for rid in sorted(named & self.identities.keys()):
            size, digest = self.identities[rid]
            print(f"identity resource=0x{rid:08X} size={size} sha256={digest}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--client-root",
        type=Path,
        required=True,
        help="explicit install root; no default or discovery",
    )
    args = parser.parse_args()
    data_root = args.client_root / "data"
    if not data_root.is_dir():
        print("census: explicit client root has no data directory", file=sys.stderr)
        return 2
    census = Census()
    try:
        for path, rid in walk_resources(data_root):
            census.n["files"] += 1
            try:
                with path.open("rb") as handle:
                    header = handle.read(8)
                    if header != b"SEDBRES ":
                        continue
                    census.n["RES"] += 1
                    data = header + handle.read()
            except OSError:
                census.n["read errors"] += 1
                continue
            try:
                census.resource(rid, data)
            except Malformed as error:
                census.n["malformed RES"] += 1
                census.anomaly(error.reason, rid, data, error.offset)
    except OSError:
        print("census: resource directory could not be read", file=sys.stderr)
        return 2
    census.report()
    return 0


if __name__ == "__main__":
    sys.exit(main())
