#!/usr/bin/env python3
"""Authored checks for the research census's PHB tree interpretation."""

from __future__ import annotations

import struct
import unittest

from census_phb_tree import (
    GBD,
    Box,
    Measurements,
    align,
    analyze_phb,
    analyze_tbd,
    candidate_slots,
    leaf_slice,
)


def tbd(nodes: list[tuple[int, ...]], references: list[int]) -> bytearray:
    count = len(nodes)
    ref_at = 0x50 + (count + 1) * 16
    data = bytearray(align(ref_at + len(references) * 2))
    data[:8] = b"PHB.TBD\0"
    struct.pack_into("<6I", data, 8, len(data), count, 0x50, len(references), ref_at, 0)
    struct.pack_into("<8f", data, 0x20, 0, 0, 0, 1, 1, 1, 1, 1)
    for index, node in enumerate(nodes):
        struct.pack_into("<8H", data, 0x50 + index * 16, *node)
    struct.pack_into(
        "<8H",
        data,
        0x50 + count * 16,
        65535,
        65535,
        65535,
        count + len(references),
        0,
        0,
        0,
        65535,
    )
    struct.pack_into(f"<{len(references)}H", data, ref_at, *references)
    return data


ROOT = (0x4000, 0x4000, 0x4000, 3, 0xC000, 0xC000, 0xC000, 0xFFFF)
LEFT = (0x4000, 0x4000, 0x4000, 2, 0x8000, 0xC000, 0xC000, 0x4000)
RIGHT = (0x8000, 0x4000, 0x4000, 3, 0xC000, 0xC000, 0xC000, 0x4002)
TERMINAL = (65535, 65535, 65535, 7, 0, 0, 0, 65535)
BOXES = [Box((0, 0, 0), (0.5, 1, 1))] * 2 + [Box((0.5, 0, 0), (1, 1, 1))] * 2


def two_tbc_phb() -> bytearray:
    data = bytearray(GBD + 0x500 + 16)
    data[:8] = b"SEDBPHB\0"
    struct.pack_into("<IHHI", data, 8, 0x120, 0x400, 0x48, 0x48)
    data[GBD : GBD + 8] = b"PHB.GBD\0"
    for offset, value in (
        (8, 0x500),
        (12, 0x212),
        (16, 1),
        (20, 0x80),
        (24, 0x80),
        (28, 0x100),
        (32, 0x80),
        (36, 0x180),
        (40, 0x80),
        (44, 0x200),
        (48, 0x1E0),
        (52, 2),
        (56, 0x480),
        (60, 6),
        (96, 0x400),
        (100, 2),
        (112, 0x500),
        (116, 0x500),
        (120, 0x500),
    ):
        struct.pack_into("<I", data, GBD + offset, value)
    struct.pack_into("<8f", data, GBD + 64, 0, 0, 0, 1, 1, 1, 1, 1)
    struct.pack_into("<3H", data, GBD + 0x100, 0, 1, 65535)
    for index, relative in enumerate((0, 0xF0)):
        struct.pack_into("<4I", data, GBD + 0x180 + index * 16, relative, 0xF0, 0, 0)
        at = GBD + 0x200 + relative
        data[at : at + 8] = b"PHB.TBC\0"
        struct.pack_into("<II", data, at + 8, 0x100, 0xF0)
        struct.pack_into("<8f", data, at + 16, 0, 0, 0, 1, 1, 1, 1, 1)
        struct.pack_into("<3I", data, at + 48, 1, 0x50, 0x60)
        struct.pack_into("<I", data, at + 0x50, 0x70)
        struct.pack_into("<I", data, at + 0x60, 0x80)
        tree = tbd([(0x4000, 0x4000, 0x4000, 1, 0xC000, 0xC000, 0xC000, 0)], [index])
        data[at + 0x70 : at + 0xF0] = tree
    struct.pack_into("<8H", data, GBD + 0x400, 0, 1, 2, 0, 3, 4, 5, 0)
    struct.pack_into(
        "<18f",
        data,
        GBD + 0x480,
        0,
        0,
        0,
        0.5,
        0,
        0,
        0,
        1,
        1,
        0.5,
        0,
        0,
        1,
        0,
        0,
        1,
        1,
        1,
    )
    return data


class TreeChecks(unittest.TestCase):
    def test_one_grid_cell_with_two_tbc_global_references(self):
        data = two_tbc_phb()
        m = Measurements()
        result = analyze_phb(data, m)
        self.assertEqual((result["tbcs"], result["tbds"], result["nodes"]), (2, 2, 2))
        self.assertEqual(
            (result["grid_dimension"], result["no_optional_tails"]), (1, True)
        )
        self.assertTrue(result["reference_coverage_complete"])
        self.assertEqual(result["maximum_reference_multiplicity"], 1)
        self.assertEqual(m.histograms["grid cell TBC reference count"], {"2": 1})
        self.assertEqual(m.histograms["grid lists cover all TBC IDs"], {"True": 1})
        for offset, fmt, value, message in (
            (GBD + 0x102, "H", 2, "grid list TBC ID out of range"),
            (GBD + 0x190, "I", 0xF1, "TBC descriptor differs from chunk chain"),
        ):
            with self.subTest(message=message):
                malformed = data.copy()
                struct.pack_into("<" + fmt, malformed, offset, value)
                with self.assertRaisesRegex(ValueError, message):
                    analyze_phb(malformed, Measurements())

    def test_two_leaf_literal_layout_and_inclusive_queries(self):
        data = tbd([ROOT, LEFT, RIGHT], [0, 1, 2, 3])
        m = Measurements()
        tree = analyze_tbd(data, 0, len(data), BOXES, m)
        self.assertEqual((tree.nodes, tree.leaves, tree.depth), (3, 2, 1))
        self.assertEqual(tree.references, [0, 1, 2, 3])
        self.assertEqual(m.maxima["node quantization excess units"], 0)
        self.assertEqual(
            m.counts["nodes enclosing geometry tolerance 0 quantized units"], 3
        )
        self.assertEqual(struct.unpack_from("<8H", data, 0x80), TERMINAL)
        nodes = [ROOT, LEFT, RIGHT, TERMINAL]
        for query, expected in (
            (Box((0x5000,) * 3, (0x5000,) * 3), [0, 1]),
            (Box((0xA000,) * 3, (0xA000,) * 3), [2, 3]),
            (Box((0x8000,) * 3, (0x8000,) * 3), [0, 1, 2, 3]),
            (Box((0,) * 3, (0x3FFF,) * 3), []),
        ):
            with self.subTest(expected=expected):
                self.assertEqual(candidate_slots(nodes, query), expected)

    def test_quantization_distinguishes_one_unit_from_larger_excursion(self):
        for upper, enclosed in ((0x7FFF, 3), (0x7FFE, 2)):
            with self.subTest(upper=upper):
                data = tbd([ROOT, LEFT, RIGHT], [0, 1, 2, 3])
                struct.pack_into("<H", data, 0x50 + 16 + 8, upper)
                m = Measurements()
                analyze_tbd(data, 0, len(data), BOXES, m)
                self.assertEqual(
                    m.counts["nodes enclosing geometry tolerance 0 quantized units"], 2
                )
                self.assertEqual(
                    m.counts["nodes enclosing geometry tolerance 1.01 quantized units"],
                    enclosed,
                )

    def test_deep_unbalanced_tree_uses_absolute_skips(self):
        nodes = []
        # 1100 branches exceed Python's recursion limit without recursive decoding.
        for level in range(1100):
            index = level * 2
            nodes.append((0x4000, 0x4000, 0x4000, 2201, 0xC000, 0xC000, 0xC000, 65535))
            nodes.append(
                (0x4000, 0x4000, 0x4000, index + 2, 0xC000, 0xC000, 0xC000, level)
            )
        nodes.append((0x4000, 0x4000, 0x4000, 2201, 0xC000, 0xC000, 0xC000, 1100))
        data = tbd(nodes, list(range(1101)))
        boxes = [Box((0, 0, 0), (1, 1, 1))] * 1101
        tree = analyze_tbd(data, 0, len(data), boxes, Measurements())
        self.assertEqual((tree.nodes, tree.leaves, tree.depth), (2201, 1101, 1100))
        self.assertEqual(
            candidate_slots(
                nodes + [(65535, 65535, 65535, 3302, 0, 0, 0, 65535)],
                Box((0x8000,) * 3, (0x8000,) * 3),
            ),
            list(range(1101)),
        )

    def test_reference_packing_boundaries(self):
        self.assertEqual(leaf_slice(0, 1), (0, 1))
        self.assertEqual(leaf_slice(0x4000, 2), (0, 2))
        self.assertEqual(leaf_slice(0xBFFF, 16386), (16383, 3))
        self.assertEqual(leaf_slice(0xFFFE, 16386), (16382, 4))
        with self.assertRaisesRegex(ValueError, "branch word is not a leaf"):
            leaf_slice(0xFFFF, 16387)
        with self.assertRaisesRegex(ValueError, "slice out of range"):
            leaf_slice(0x4002, 3)

    def test_malformed_skip_reference_and_boxes(self):
        for offset, fmt, value, message in (
            (0x10, "I", 0x51, "misaligned TBD node and terminal array"),
            (0x50 + 16 + 6, "H", 3, "leaf skip is not consecutive"),
            (0x50 + 32 + 6, "H", 4, "invalid node skip"),
            (0x90, "H", 4, "triangle reference out of range"),
            (0x50 + 16 + 14, "H", 0x4003, "leaf reference slice out of range"),
            (0x50 + 16, "H", 0x9000, "inverted integer box"),
            (0x20, "f", 2.0, "inverted float box"),
            (0x20, "f", float("nan"), "non-finite box"),
            (0x80 + 6, "H", 6, "invalid terminal record"),
        ):
            with self.subTest(message=message):
                data = tbd([ROOT, LEFT, RIGHT], [0, 1, 2, 3])
                struct.pack_into("<" + fmt, data, offset, value)
                with self.assertRaisesRegex(ValueError, message):
                    analyze_tbd(data, 0, len(data), BOXES, Measurements())


if __name__ == "__main__":
    unittest.main()
