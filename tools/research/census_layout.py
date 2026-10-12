#!/usr/bin/env python3
"""Census MapLayout tables and embedded lyb graphs in an explicit client install.

Research only: makes no support claim and never runs in CI. Requires
--client-root with no default, discovery, or environment fallback. Reads DAT
files read-only, writes no client bytes, and prints structural counts, short
class/tag vocabulary, offsets, resource identities, sizes, and SHA-256 only.

    python tools/research/census_layout.py --client-root <install>
"""

from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from collections.abc import Iterator
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
import hashlib
from itertools import islice
import math
import os
from pathlib import Path
import re
import struct
import sys

MAGIC = b"MapLayoutResourceData"
RESOURCE_PATH = re.compile(
    r"^([0-9a-f]{2})/([0-9a-f]{2})/([0-9a-f]{2})/([0-9a-f]{2})\.dat$", re.I
)
EXAMPLE = 0x29D90001
Fields = tuple[tuple[int, int], ...]
Shape = tuple[int, Fields]


def shape(size: int, descriptors: list[tuple[int, int]]) -> Shape:
    return size, tuple(sorted(descriptors))


CLASS = shape(0x14, [(4, 0), (8, 2), (16, 0)])
INSTANCE = shape(0x40, [(4, 0), (8, 2), (16, 0), (44, 0), (48, 0), (60, 0), (52, 0)])
TREE = shape(0x50, [(4, 0), (8, 2), (28, 0), (32, 0), (44, 2), (36, 0), (48, 0)])
CHILD = shape(0x30, [(20, 2), (12, 0), (16, 0), (24, 0), (32, 0)])
GROUP = shape(0x30, [(12, 0), (16, 0), (20, 0), (28, 2), (32, 0)])
GROUP_EXTENDED = shape(
    0x30,
    [(0, 0), (8, 2), (12, 2), (16, 2), (20, 0), (28, 2), (32, 2), (36, 2), (40, 2)],
)
ATTRIBUTE = shape(0x2C, [(4, 0), (8, 2), (36, 2), (20, 0), (28, 0)])
COLLISION = shape(0x14, [(4, 0), (8, 2)])
SETTINGS = shape(
    0x70, [(4, 0), (8, 2), (76, 0), (80, 0), (92, 0), (28, 2), (84, 0), (96, 0)]
)
DIVIDE = shape(0x70, [(4, 0), (8, 2), (16, 0), (24, 0), (80, 0), (88, 0), (100, 0)])
BLOCK = shape(0x18, [(4, 0), (8, 2), (16, 0)])
BG = shape(0x54, [(4, 0), (8, 2), (20, 0), (32, 2), (36, 2), (40, 0), (44, 2)])
POSITION = shape(0x18, [(4, 0), (8, 2)])
RESOURCE_REFERENCE = shape(0x18, [(4, 0), (8, 2), (16, 2), (20, 0)])
SHARED = shape(0x1C, [(4, 0), (8, 2), (16, 0), (24, 0)])
EXPECTED = {
    "InstanceObject": INSTANCE,
    "UnitTreeObject": TREE,
    "AttributeBaseObject": ATTRIBUTE,
    "CollisionBoxBaseObject": COLLISION,
    "DivideMapFolderObject": DIVIDE,
    "BlockLinkObject": BLOCK,
    "BGPartsBaseObject": BG,
    "BGChipBaseObject": BG,
    "PositionMarkerBaseObject": POSITION,
    "ResourceReferenceObject": RESOURCE_REFERENCE,
    "SharedFolderObject": SHARED,
}


class Malformed(ValueError):
    """Bounded parsing failed; the message contains no client content."""


@dataclass
class Reader:
    data: bytes
    base: int = 0

    def span(self, offset: int, size: int) -> bytes:
        if offset < 0 or size < 0 or offset + size > len(self.data):
            raise Malformed(
                f"span out of bounds at file+0x{self.base + offset:X} size={size}"
            )
        return self.data[offset : offset + size]

    def u32(self, offset: int) -> int:
        return struct.unpack("<I", self.span(offset, 4))[0]

    def u16(self, offset: int) -> int:
        return struct.unpack("<H", self.span(offset, 2))[0]

    def floats(self, offset: int, count: int = 3) -> tuple[float, ...]:
        return struct.unpack(f"<{count}f", self.span(offset, count * 4))

    def text(self, offset: int, width: int) -> bytes:
        return self.span(offset, width).split(b"\0", 1)[0]

    def string(self, offset: int) -> bytes:
        if not offset:
            return b""
        self.span(offset, 1)
        end = self.data.find(b"\0", offset)
        if end < 0:
            raise Malformed(f"unterminated string at file+0x{self.base + offset:X}")
        return self.data[offset:end]

    def words(self, offset: int, count: int) -> tuple[int, ...]:
        if not offset and count:
            raise Malformed("null array with nonzero count")
        return tuple(
            value[0] for value in struct.iter_unpack("<I", self.span(offset, count * 4))
        )


@dataclass(frozen=True)
class NodeType:
    start: int
    count: int
    size: int
    fields: Fields
    one: int

    @property
    def shape(self) -> Shape:
        return self.size, tuple(sorted(self.fields))

    def nodes(self) -> range:
        return range(self.start, self.start + self.count * self.size, self.size)


@dataclass
class Layout:
    rid: int
    raw: Reader
    payload: Reader
    scene: int
    total: int
    rows: list[tuple[bytes, bytes, int]]
    types: list[NodeType]
    objects: tuple[int, ...]
    node_types: dict[int, NodeType]
    class_paths: dict[int, str]

    def kind(self, node: int) -> NodeType:
        try:
            return self.node_types[node]
        except KeyError:
            raise Malformed(
                f"reference not a declared node: payload+0x{node:X}"
            ) from None

    def class_ref(self, node: int) -> int:
        kind = self.kind(node)
        if kind.size < 12 or (4, 0) not in kind.fields or (8, 2) not in kind.fields:
            return 0
        return self.payload.u32(node + 4)

    def path(self, node: int) -> str:
        ref = self.class_ref(node)
        if not ref:
            return ""
        if ref not in self.class_paths:
            raise Malformed(f"class reference not a class node: payload+0x{ref:X}")
        return self.class_paths[ref]

    def run(self, offset: int, count: int) -> tuple[int, ...]:
        if not count:
            return ()
        kind = self.kind(offset)
        if offset + count * kind.size > kind.start + kind.count * kind.size:
            raise Malformed(
                f"node run outside extent: payload+0x{offset:X} count={count}"
            )
        return tuple(range(offset, offset + count * kind.size, kind.size))


def parse_layout(rid: int, data: bytes) -> Layout:
    raw = Reader(data)
    if raw.span(0, 24) != MAGIC.ljust(24, b"\0"):
        raise Malformed("MapLayout signature padding differs")
    scene, count = raw.u32(32), raw.u32(36)
    raw.span(0x40, count * 32)
    rows = [
        (
            raw.text(0x40 + i * 32, 16),
            raw.text(0x50 + i * 32, 4),
            raw.u32(0x54 + i * 32),
        )
        for i in range(count)
    ]
    if raw.span(scene, 8) != b"SEDBlyb\0":
        raise Malformed(f"missing SEDB lyb at file+0x{scene:X}")
    header, total = raw.u16(scene + 14), raw.u32(scene + 16)
    if header < 0x14 or total < header:
        raise Malformed("invalid SEDB header/total size")
    raw.span(scene, total)
    payload = Reader(raw.span(scene + header, total - header), scene + header)
    if payload.span(0, 4) != b"lyb\0":
        raise Malformed("missing payload lyb tag")
    table, objects = payload.u32(16), payload.u32(20)
    if objects < 24 or table < objects or (table - objects) % 4:
        raise Malformed("invalid object-list offsets")
    object_list = payload.words(objects, (table - objects) // 4)
    if not object_list:
        raise Malformed("empty object list")
    end = payload.u32(table + 4)
    if end <= table or (end - table) % 16:
        raise Malformed("invalid node-type table end")
    payload.span(table, end - table)
    types: list[NodeType] = []
    node_types: dict[int, NodeType] = {}
    for at in range(table, end, 16):
        extent, fields, one, nfields = payload.words(at, 4)
        start, n, size = payload.words(extent, 3)
        if not size:
            raise Malformed("zero node size")
        payload.span(start, n * size)
        descriptors = tuple(
            struct.iter_unpack("<HH", payload.span(fields, nfields * 4))
        )
        kind = NodeType(start, n, size, descriptors, one)
        types.append(kind)
        for node in kind.nodes():
            if node in node_types:
                raise Malformed(f"overlapping node extents at payload+0x{node:X}")
            node_types[node] = kind
    paths: dict[int, str] = {}
    for kind in types:
        if kind.shape == CLASS:
            for node in kind.nodes():
                name = payload.string(payload.u32(node + 8))
                if len(name) > 160 or any(b < 32 or b > 126 for b in name):
                    raise Malformed(
                        f"class path not short ASCII vocabulary at payload+0x{node:X}"
                    )
                paths[node] = name.decode("ascii")
    return Layout(
        rid, raw, payload, scene, total, rows, types, object_list, node_types, paths
    )


@dataclass
class Stats:
    counts: Counter[str] = field(default_factory=Counter)
    distributions: dict[str, Counter[str]] = field(
        default_factory=lambda: defaultdict(Counter)
    )
    shapes: dict[tuple[str, Shape], Counter[int]] = field(
        default_factory=lambda: defaultdict(Counter)
    )
    keys: set[bytes] = field(default_factory=set)
    settings_names: set[bytes] = field(default_factory=set)
    divide_names: set[bytes] = field(default_factory=set)
    pools: dict[tuple[int, int], float] = field(default_factory=dict)

    def test(self, name: str, condition: bool) -> None:
        self.counts[name + ".tested"] += 1
        self.counts[name + (".pass" if condition else ".fail")] += 1
        self.counts.setdefault(name + ".pass", 0)
        self.counts.setdefault(name + ".fail", 0)

    def dist(self, name: str, value: object, count: int = 1) -> None:
        self.distributions[name][str(value)] += count

    def merge(self, other: Stats) -> None:
        self.counts.update(other.counts)
        for name, counts in other.distributions.items():
            self.distributions[name].update(counts)
        for key, counts in other.shapes.items():
            self.shapes[key].update(counts)
        self.keys.update(other.keys)
        self.settings_names.update(other.settings_names)
        self.divide_names.update(other.divide_names)
        self.pools.update(other.pools)


def short_class(path: str) -> str:
    return path.rsplit("/", 1)[-1]


def observe(layout: Layout) -> Stats:
    s = Stats()
    p, raw = layout.payload, layout.raw
    s.test("table_follows_header", 0x40 + len(layout.rows) * 32 == layout.scene)
    s.test("scene_ends_file", layout.scene + layout.total == len(raw.data))
    s.test("reserved_header_zero", not any(raw.span(0x28, 24)))
    s.dist("versions", ascii(raw.text(0x18, 8).decode("ascii", "backslashreplace")))
    s.dist("unknownA", f"0x{p.u32(8):08X}")
    s.dist("unknownB", f"0x{p.u32(12):08X}")
    s.dist("objectListOffset", f"0x{p.u32(20):X}")
    s.test("payload_size_equals_SEDB_total", p.u32(4) == layout.total)
    extents = sorted(
        (kind.start, kind.start + kind.count * kind.size)
        for kind in layout.types
        if kind.count
    )
    for previous, current in zip(extents, extents[1:]):
        s.test("declared_extent_disjoint", previous[1] <= current[0])
        if previous[1] > current[0]:
            s.dist("declared_extent_overlap_sizes", previous[1] - current[0])
    row_names: dict[bytes, set[str]] = defaultdict(set)
    for i, (name, tag, rid) in enumerate(layout.rows):
        s.test("reserved_row_zero", not any(raw.span(0x58 + i * 32, 8)))
        s.test(
            "reserved_row_zero." + ("marker" if not tag else "resource"),
            not any(raw.span(0x58 + i * 32, 8)),
        )
        row_kind = "marker" if not tag else "resource"
        s.dist(f"row_+0x18.{row_kind}", raw.u32(0x58 + i * 32))
        s.dist(f"row_+0x1C.{row_kind}", raw.u32(0x5C + i * 32))
        s.dist("row_tags", tag.decode("ascii", "backslashreplace") or "<empty>")
        row_names[name].add(tag.decode("ascii", "backslashreplace"))
        if not tag or not rid:
            s.test("marker_empty_tag_and_zero_id", not tag and rid == 0)
    for kind in layout.types:
        s.dist("node_type_one", kind.one)
        for _, descriptor_kind in kind.fields:
            s.dist("descriptor_kinds", descriptor_kind)
        refs = {layout.class_ref(n) for n in kind.nodes()}
        if (4, 0) in kind.fields and (8, 2) in kind.fields:
            s.test("type_class_ref_uniform", len(refs) <= 1)
        if kind.shape == CHILD:
            s.shapes[("child", kind.shape)][layout.rid] += kind.count
        if kind.shape == GROUP_EXTENDED:
            s.shapes[("nine_field_group_shape_extent", kind.shape)][layout.rid] += (
                kind.count
            )
        for node in kind.nodes():
            path = layout.path(node)
            if path:
                s.dist("class_instances", path)
                s.shapes[(path, kind.shape)][layout.rid] += 1
                expected = (
                    SETTINGS
                    if "/LaySettings/" in path
                    else EXPECTED.get(short_class(path))
                )
                if expected:
                    s.test(
                        "expected_shape." + short_class(path), kind.shape == expected
                    )
    # A root-folder record shares the class shape. References identify the
    # class extents, including abstract class records with no instances.
    class_types = {
        layout.kind(layout.class_ref(n))
        for n in layout.node_types
        if layout.class_ref(n)
    }
    class_nodes = {node for kind in class_types for node in kind.nodes()}
    for node in class_nodes:
        parent = p.u32(node + 16)
        s.test("class_parent_declared_class", not parent or parent in class_nodes)
    s.counts["class_nodes"] = len(class_nodes)
    for node in class_nodes:
        s.distributions["class_instances"].setdefault(layout.class_paths[node], 0)
    s.counts["object_list_entries"] = len(layout.objects)
    for node in layout.objects:
        s.test("object_list_declared_nodes", node in layout.node_types)
        if node in layout.node_types:
            s.counts["objects_without_class"] += not bool(layout.class_ref(node))
    root = layout.objects[0]
    layout.kind(root)
    root_children = p.words(p.u32(root), p.u32(root + 4))
    s.test("root_children_equal_object_tail", root_children == layout.objects[1:])
    settings = [n for n in layout.node_types if "/LaySettings/" in layout.path(n)]
    s.dist("settings_nodes_per_layout", len(settings))
    if len(settings) == 1:
        name = p.string(p.u32(settings[0] + 8))
        s.settings_names.add(name)
        s.test("root_name_settings_Test", p.string(p.u32(root + 8)) == name + b"/Test")

    def u(node: int, offset: int) -> int:
        layout.kind(node)
        if offset + 4 > layout.kind(node).size:
            raise Malformed(f"field exceeds node at payload+0x{node:X}")
        return p.u32(node + offset)

    def text(node: int, offset: int) -> bytes:
        return p.string(u(node, offset))

    def array(node: int, offset: int, count_offset: int) -> tuple[int, ...]:
        return p.words(u(node, offset), u(node, count_offset))

    def box(node: int, minimum: int, maximum: int) -> bool:
        low, high = p.floats(node + minimum), p.floats(node + maximum)
        return all(
            math.isfinite(a) and math.isfinite(b) and a <= b for a, b in zip(low, high)
        )

    def binding(node: int, key: bytes, category: str) -> None:
        s.keys.add(key)
        tags = row_names.get(key, set())
        s.test("resource_key_resolved", bool(tags))
        s.dist("binding." + category, "+".join(sorted(tags)) or "<unresolved>")

    def transform(
        node: int, translation: int, rotation: int, scale: int
    ) -> tuple[tuple[float, ...], tuple[float, ...], tuple[float, ...]]:
        vectors = []
        for field_offset, default in (
            (rotation, (0.0, 0.0, 0.0)),
            (scale, (1.0, 1.0, 1.0)),
        ):
            ref = u(node, field_offset)
            if ref:
                value = p.floats(ref, 4)
                s.pools[(layout.rid, ref)] = value[3]
                s.dist("transform_fourth_lane_references", value[3])
                vectors.append(value[:3])
            else:
                vectors.append(default)
        return p.floats(node + translation), vectors[0], vectors[1]

    trees: list[int] = []
    member_slots: dict[str, set[int]] = defaultdict(set)
    block_slots: set[int] = set()
    for node, kind in layout.node_types.items():
        path = layout.path(node)
        name = short_class(path)
        if path:
            s.test("object_name_not_table_key", text(node, 8) not in row_names)
            s.test(
                "object_name_not_nonmarker_key",
                not any(row_names.get(text(node, 8), set()) - {""}),
            )
        if "/LaySettings/" in path:
            s.dist("LaySettings.+0x3C_f32", p.floats(node + 0x3C, 1)[0])
            s.dist("LaySettings.+0x3C_u32", u(node, 0x3C))
            transform(node, 0x40, 0x4C, 0x50)
        elif name in ("BGPartsBaseObject", "BGChipBaseObject"):
            s.test(
                "BG_empty_24_2C_null_28",
                not text(node, 0x24) and not text(node, 0x2C) and u(node, 0x28) == 0,
            )
            for offset, value in ((0x30, 0), (0x34, 0), (0x38, 0x100)):
                s.test(f"BG.+0x{offset:X}=={value}", u(node, offset) == value)
            s.test("BG_boxes_ordered", box(node, 0x3C, 0x48))
            binding(
                node,
                text(node, 0x20),
                "BGParts" if name == "BGPartsBaseObject" else "BGChip",
            )
            start, count = u(node, 0x14), u(node, 0x18)
            p.span(start, count * 12)
            for at in range(start, start + count * 12, 12):
                binding(node, p.string(p.u32(at)), "material")
        elif name == "AttributeBaseObject":
            s.test("Attribute.zero_0C_23", not any(p.span(node + 12, 24)))
            s.dist("Attribute.+0x28", u(node, 0x28))
            binding(node, text(node, 0x24), "Attribute")
        elif name == "CollisionBoxBaseObject":
            s.dist("CollisionBox.byte2", p.span(node + 0x0E, 1)[0])
            s.dist("CollisionBox.byte3", p.span(node + 0x0F, 1)[0])
            s.test("CollisionBox.+0x10_zero", u(node, 16) == 0)
        elif name == "PositionMarkerBaseObject":
            s.test("PositionMarker.zero_0C_17", not any(p.span(node + 12, 12)))
        elif name == "ResourceReferenceObject":
            s.test("ResourceReference.+0x0C_zero", u(node, 12) == 0)
        elif name in ("DivideMapFolderObject", "SharedFolderObject"):
            if name == "DivideMapFolderObject":
                s.test("DivideMapFolder_boxes_ordered", box(node, 0x30, 0x40))
                for ref in array(node, 0x50, 0x54):
                    s.divide_names.add(p.string(ref))
            else:
                s.test("SharedFolder.+0x18_null", u(node, 24) == 0)
            for slot in array(node, 16, 20):
                member_slots[name].add(slot)
                target = p.u32(slot)
                s.test(
                    name + "_members_via_slot_to_Instance",
                    layout.kind(slot).shape == shape(4, [(0, 0)])
                    and target in layout.node_types
                    and layout.path(target) == "RefObjects/InstanceObject",
                )
        elif name == "BlockLinkObject":
            start = u(node, 16)
            for index, ref in enumerate(array(node, 16, 20)):
                block_slots.add(start + index * 4)
                value = p.string(ref)
                s.test(
                    "BlockLink_printable_nonempty",
                    bool(value) and all(32 <= b <= 126 for b in value),
                )
        elif name == "InstanceObject":
            ref = u(node, 0x3C)
            valid = ref in layout.node_types and bool(layout.path(ref))
            s.test("Instance_base_class_bearing", valid)
            s.dist("Instance_base_classes", layout.path(ref) if valid else "<invalid>")
            transform(node, 0x20, 0x2C, 0x30)
        elif name == "UnitTreeObject":
            trees.append(node)
            translation, _, scale = transform(node, 0x10, 0x1C, 0x20)
            s.test(
                "unit_tree_identity_translation_scale",
                translation == (0.0, 0.0, 0.0) and scale == (1.0, 1.0, 1.0),
            )
        if kind.shape in (CHILD, GROUP):
            transform(node, 0, 12, 16)

    owned_groups: set[int] = set()
    for tree in trees:
        children = layout.run(u(tree, 0x30), u(tree, 0x34))
        if layout.rid == EXAMPLE:
            s.counts["example_unit_tree_children"] += len(children)
            for child in children:
                ref = u(child, 12)
                rotation = p.floats(ref) if ref else (0.0, 0.0, 0.0)
                s.counts["example_rotated_children"] += rotation != (0.0, 0.0, 0.0)
        siblings = layout.run(u(tree, 0x24), u(tree, 0x28))
        for group in siblings:
            target, count = u(group, 0x20), u(group, 0x24)
            if not target:
                s.test("top_level_group_child_run_null", count == 0)
            else:
                run = layout.run(target, count)
                s.test(
                    "top_level_group_child_run_inside_extent_outside_tree_list",
                    layout.kind(target).shape == CHILD
                    and not set(run).intersection(children),
                )
        for a, b in zip(siblings, siblings[1:]):
            s.dist("group_sibling_step_delta", u(b, 0x28) - u(a, 0x28))
            s.dist("top_level_group_sibling_step_delta", u(b, 0x28) - u(a, 0x28))
        pending = list(siblings)
        seen: set[int] = set()
        while pending:
            group = pending.pop()
            if group in seen:
                s.counts["group_repeated_or_cyclic"] += 1
                continue
            seen.add(group)
            kind = layout.kind(group)
            if group not in owned_groups:
                s.shapes[("group", kind.shape)][layout.rid] += 1
            owned_groups.add(group)
            s.dist(
                "tree_group_shapes",
                "extended"
                if kind.shape == GROUP_EXTENDED
                else "canonical"
                if kind.shape == GROUP
                else "other",
            )
            s.test("group.+0x2C_zero", u(group, 0x2C) == 0)
            s.dist("group.+0x2C", u(group, 0x2C))
            s.dist("group.+0x28", u(group, 0x28))
            target, count = u(group, 0x20), u(group, 0x24)
            if not target:
                s.test("group_child_run_null", count == 0)
            else:
                run = layout.run(target, count)
                s.test(
                    "group_child_run_inside_extent_outside_tree_list",
                    layout.kind(target).shape == CHILD
                    and not set(run).intersection(children),
                )
            siblings = layout.run(u(group, 0x14), u(group, 0x18))
            for a, b in zip(siblings, siblings[1:]):
                s.dist("group_sibling_step_delta", u(b, 0x28) - u(a, 0x28))
            pending.extend(siblings)
            if kind.shape == GROUP_EXTENDED:
                transform(group, 0, 12, 16)
    s.counts["tree_owned_distinct_groups"] = len(owned_groups)
    s.counts["BlockLink_distinct_string_ref_slots"] = len(block_slots)
    for name, slots in member_slots.items():
        s.counts[name + "_distinct_member_slots"] = len(slots)
    s.counts["distinct_resource_keys_per_layout_sum"] = len(s.keys)
    if layout.rid == EXAMPLE:
        s.counts["example_rows"] = len(layout.rows)
        s.counts["example_scene"] = layout.scene
        s.counts["example_file_size"] = len(raw.data)
        s.counts["example_SEDB_total"] = layout.total
        s.counts["example_unknownA"] = p.u32(8)
        s.counts["example_objects"] = len(layout.objects)
        s.counts["example_node_types"] = len(layout.types)
        s.counts["example_class_nodes"] = len(class_nodes)
        s.counts["example_nine_field_group_shape_extent_nodes"] = sum(
            kind.count for kind in layout.types if kind.shape == GROUP_EXTENDED
        )
        s.distributions["example_tags"] = Counter(
            tag.decode("ascii", "backslashreplace") or "<empty>"
            for _, tag, _ in layout.rows
        )
    return s


def walk(data_root: Path, errors: Counter[str]) -> Iterator[tuple[Path, str]]:
    stack = [(data_root, "")]
    while stack:
        directory, prefix = stack.pop()
        try:
            with os.scandir(directory) as entries:
                for entry in entries:
                    relative = prefix + entry.name
                    if entry.is_dir(follow_symlinks=False):
                        stack.append((Path(entry.path), relative + "/"))
                    elif entry.is_file(
                        follow_symlinks=False
                    ) and entry.name.upper().endswith(".DAT"):
                        yield Path(entry.path), relative
        except OSError as error:
            errors[f"directory_read_errno={error.errno}"] += 1


def resource_path(root: Path, rid: int) -> Path:
    h = f"{rid:08X}"
    return root / h[:2] / h[2:4] / h[4:6] / (h[6:] + ".DAT")


def read_signature(entry: tuple[Path, str]) -> tuple[Path, str, bytes, int | None]:
    path, relative = entry
    try:
        with path.open("rb", buffering=0) as handle:
            return path, relative, handle.read(24), None
    except OSError as error:
        return path, relative, b"", error.errno or -1


def signatures(
    root: Path, errors: Counter[str]
) -> Iterator[tuple[Path, str, bytes, int | None]]:
    entries = walk(root, errors)
    # Batches bound queued work and retain no more than 256 signature prefixes.
    with ThreadPoolExecutor(max_workers=8) as pool:
        while batch := list(islice(entries, 256)):
            yield from pool.map(read_signature, batch)


def vocabulary(value: bytes) -> str:
    return value.rstrip(b"\0 ").decode("ascii", "backslashreplace")


def report(s: Stats, layouts: int, malformed: int) -> set[int]:
    print(
        f"coverage layouts={layouts} parsed={layouts - malformed} malformed={malformed}"
    )
    print(
        f"exemplar 0x{EXAMPLE:08X} rows={s.counts['example_rows']} sceneOffset=0x{s.counts['example_scene']:X} SEDB_totalSize={s.counts['example_SEDB_total']} file_size={s.counts['example_file_size']} unknownA=0x{s.counts['example_unknownA']:08X} objects={s.counts['example_objects']} node_types={s.counts['example_node_types']} class_nodes={s.counts['example_class_nodes']}"
    )
    print("counts (tests count observations; pass/fail do not imply support):")
    for name, n in sorted(s.counts.items()):
        print(f"  {name}={n}")
    for name, counts in sorted(s.distributions.items()):
        print(
            f"distribution {name}: "
            + ", ".join(f"{ascii(k)}={v}" for k, v in sorted(counts.items()))
        )
    print(
        f"L7 declared_distinct_class_paths={len(s.distributions['class_instances'])} class_paths_with_instances={sum(n > 0 for n in s.distributions['class_instances'].values())} objects_without_class={s.counts['objects_without_class']}"
    )
    print(
        f"L10 install_distinct_resource_key_strings={len(s.keys)} distinct_layout_key_pairs={s.counts['distinct_resource_keys_per_layout_sum']}"
    )
    print(
        f"L13 distinct_LaySettings_names={len(s.settings_names)} distinct_DivideMapFolder_names={len(s.divide_names)}"
    )
    pool_counts = Counter(str(value) for value in s.pools.values())
    print(
        "L9 distinct_transform_pool_fourth_lane: "
        + ", ".join(f"{k}={v}" for k, v in sorted(pool_counts.items()))
    )
    grouped: dict[str, list[tuple[Shape, Counter[int]]]] = defaultdict(list)
    for (label, node_shape), resources in s.shapes.items():
        grouped[label].append((node_shape, resources))
    identities = {EXAMPLE}
    variants = 0
    for label, entries in sorted(grouped.items()):
        print(f"L8 shape {ascii(label)} variants={len(entries)}")
        for (size, fields), resources in sorted(entries):
            print(
                f"  size=0x{size:X} descriptors="
                + ",".join(f"({o:#x},{k})" for o, k in fields)
                + f" nodes={sum(resources.values())} layouts={len(resources)}"
            )
            if len(entries) > 1 or label == "nine_field_group_shape_extent":
                variants += 1
                identities.update(resources)
                print(
                    "  variant_carriers "
                    + " ".join(
                        f"0x{rid:08X}:{n}" for rid, n in sorted(resources.items())
                    )
                )
    print(
        f"L8 classes_with_multiple_shapes={sum(len(v) > 1 for k, v in grouped.items() if k not in ('group', 'nine_field_group_shape_extent', 'child'))} reported_variant_shapes={variants}"
    )
    return identities


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--client-root",
        required=True,
        type=Path,
        help="explicit client install directory; no default or discovery",
    )
    args = parser.parse_args()
    root = args.client_root / "data"
    if not root.is_dir():
        print("census: explicit client root has no data directory", file=sys.stderr)
        return 2
    errors: Counter[str] = Counter()
    total = off_convention = malformed = 0
    found: list[tuple[int, Path]] = []
    directories: Counter[str] = Counter()
    for path, relative, head, read_error in signatures(root, errors):
        total += 1
        if total % 20000 == 0:
            print(
                f"scan_progress DAT_files={total} MapLayout_magic_at_0={len(found)}",
                flush=True,
            )
        match = RESOURCE_PATH.fullmatch(relative)
        if not match:
            off_convention += 1
        if read_error is not None:
            errors[f"resource_read_errno={read_error}"] += 1
        elif head.startswith(MAGIC):
            if not match:
                errors["layout_off_resource_convention"] += 1
                continue
            rid = int("".join(match.groups()), 16)
            found.append((rid, path))
            directories[relative.split("/")[0].upper()] += 1
    print(
        f"scan DAT_files={total} off_convention={off_convention} MapLayout_magic_at_0={len(found)}"
    )
    print(
        "layout_top_level_data_directories: "
        + ", ".join(f"{d}={n}" for d, n in sorted(directories.items()))
    )
    s = Stats()
    identities: dict[int, tuple[int, str]] = {}
    malformed_ids: set[int] = set()
    subtype_cache: dict[int, tuple[str, str]] = {}
    for rid, path in sorted(found):
        try:
            data = path.read_bytes()
            identities[rid] = len(data), hashlib.sha256(data).hexdigest()
            layout = parse_layout(rid, data)
            measured = observe(layout)
            for _, tag, target in layout.rows:
                if not tag:
                    continue
                if target not in subtype_cache:
                    try:
                        with resource_path(root, target).open(
                            "rb", buffering=0
                        ) as handle:
                            header = handle.read(0x24)
                        start = 16 if header[:4] == b"PWIB" else 0
                        if (
                            header[start : start + 4] == b"SEDB"
                            and len(header) >= start + 8
                        ):
                            subtype_cache[target] = (
                                vocabulary(header[start + 4 : start + 8]),
                                "PWIB" if start else "direct",
                            )
                        else:
                            subtype_cache[target] = "<non-SEDB>", "other"
                    except OSError:
                        subtype_cache[target] = "<unreadable>", "other"
                subtype, wrapper = subtype_cache[target]
                tag_text = vocabulary(tag)
                measured.dist("tag_to_subtype", tag_text + "->" + subtype)
                measured.dist("reference_wrapper", wrapper)
                measured.test(
                    "tag_equals_reversed_subtype_case_insensitive",
                    tag_text.lower() == subtype[::-1].lower(),
                )
                if tag_text.lower() != subtype[::-1].lower():
                    measured.dist("tag_subtype_exceptions", tag_text + "->" + subtype)
            s.merge(measured)
        except (OSError, Malformed) as error:
            malformed += 1
            malformed_ids.add(rid)
            message = (
                str(error)
                if isinstance(error, Malformed)
                else f"read_errno={error.errno}"
            )
            print(f"malformed 0x{rid:08X}: {message}")
    requested = report(s, len(found), malformed)
    requested.update(malformed_ids)
    print(f"reference_unique_files_read={len(subtype_cache)}")
    for message, count in sorted(errors.items()):
        print(f"scan_error {message} count={count}")
    for rid in sorted(requested):
        if rid in identities:
            size, digest = identities[rid]
            print(f"identity 0x{rid:08X} size={size} sha256={digest}")
        else:
            print(f"identity 0x{rid:08X} unavailable")
    return 1 if malformed or errors else 0


if __name__ == "__main__":
    sys.exit(main())
