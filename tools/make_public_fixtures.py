#!/usr/bin/env python3
"""Generator for the authored synthetic fixtures under tests/fixtures/public.

Every byte in that tree comes from this script. Nothing here is a copy, a
slice, a transformation, or a re-encoding of retail data: the layouts come
from the pages under docs/formats/ and the contents are deterministic patterns
this script invents.

    python tools/make_public_fixtures.py            # write the fixtures
    python tools/make_public_fixtures.py --check    # fail if any differ

CI runs --check to require byte-for-byte reproduction of every fixture.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from blowfish import Blowfish  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent
PUBLIC_ROOT = REPO_ROOT / "tests" / "fixtures" / "public"

MAGIC = b"SEDB"
FIXED_HEADER_SIZE = 0x14
RES_EXTENDED_OFFSET = 0x30
RES_HEADER_SIZE = 0x40

BOM = b"\xef\xbb\xbf"
SCRAMBLE_MARKER = 0xFF
SCRAMBLE_KEY = 0x73
TOKEN_START = b"\x02"
TOKEN_END = b"\x03"

# Invented text. Nothing here is copied from a client string: the sheet
# fixtures need multi-byte UTF-8 to exercise the decoder, so they carry a
# two-, a three-, and a four-byte code point of this project's choosing.
MULTIBYTE = "row \u00e9 \u4e2d \U0002000b"


def pattern(length: int, seed: int) -> bytes:
    """A deterministic filler byte string that is obviously not real data."""
    return bytes(((index * 7 + seed) & 0xFF) for index in range(length))


def tag(text: str) -> bytes:
    value = text.encode("ascii")
    if len(value) > 4:
        raise ValueError("tag longer than four bytes: {0}".format(text))
    return value.ljust(4, b"\x00")


def sedb_header(
    subtype: str,
    unknown_a: int,
    flags: int,
    header_size: int,
    total_size: int,
    extended: bytes = b"",
) -> bytes:
    """The fixed SEDB header, zero padded out to header_size.

    `extended` is written at RES_EXTENDED_OFFSET when present.
    """
    header = bytearray(max(header_size, FIXED_HEADER_SIZE))
    header[0:4] = MAGIC
    header[4:8] = tag(subtype)
    struct.pack_into("<IHHI", header, 8, unknown_a, flags, header_size, total_size)
    if extended:
        header[RES_EXTENDED_OFFSET : RES_EXTENDED_OFFSET + len(extended)] = extended
    return bytes(header)


def res_extended(
    count: int, unknown_b: int, type_name: str, repeated: int | None = None
) -> bytes:
    """The RES extended header block at 0x30: count, unknownB, count, tag."""
    if repeated is None:
        repeated = count
    return struct.pack("<III", count, unknown_b, repeated) + tag(type_name)


def directory_entry(index: int, offset: int, size: int, kind: int) -> bytes:
    return struct.pack("<IIII", index, offset, size, kind)


def utf16_field(text: str, width: int) -> bytes:
    """A UTF-16LE string in a fixed-width field, NUL terminated and zero
    padded, which is how the configuration files hold their text."""
    encoded = text.encode("utf-16-le") + b"\x00\x00"
    if len(encoded) > width:
        raise ValueError("text does not fit in {0} bytes: {1}".format(width, text))
    return encoded.ljust(width, b"\x00")


def staticactor_san(
    records: list[tuple[int, bytes]],
    *,
    declared_count: int | None = None,
) -> bytes:
    """An authored SAN record table using the retail framing only."""
    if declared_count is None:
        declared_count = len(records)
    decoded = bytearray(b"SYNTH")
    decoded.extend(struct.pack(">I", declared_count))
    for value, string in records:
        decoded.extend(struct.pack(">I", value))
        decoded.extend(string)
        decoded.append(0)
    return b"sane" + bytes(byte ^ 0x73 for byte in decoded)


def region_row(
    row_id: int,
    dat_key: int,
    field_0c: int,
    token: bytes,
    *,
    word_04: int = 0x13579BDF,
    word_20: int = 0x03020100,
) -> bytes:
    """Authored words; child field_0c is a stored kind, never a count."""
    if len(token) > 16:
        raise ValueError("region token exceeds its span")
    return (
        struct.pack("<IIII", row_id, word_04, dat_key, field_0c)
        + token.ljust(16, b"\x00")
        + struct.pack("<I", word_20)
        + bytes(range(4, 16))
    )


def region_table(rows: list[bytes], root_count: int) -> bytes:
    header = bytearray(0x40)
    header[:19] = b"RegionResourceData\x00"
    header[19:24] = b"SYNTH"
    header[24:30] = b"1.1.0\x00"
    header[30:32] = b"AB"
    struct.pack_into("<II", header, 0x20, 0x40 + 0x30 * len(rows), root_count)
    header[0x28:0x40] = bytes(range(24))
    return bytes(header) + b"".join(rows)


def xml_document(body: str, declaration: bool = True, bom: bool = True) -> bytes:
    """An SSD document in the shape the client writes: BOM, declaration,
    CRLF line endings."""
    head = '<?xml version="1.0" encoding="utf-8"?>\n' if declaration else ""
    return (BOM if bom else b"") + (head + body).replace("\n", "\r\n").encode("utf-8")


# See docs/formats/ssd-sheet.md. The second word key has bits in both halves
# so incorrect key recovery cannot pass by assuming zero.
SCRAMBLE_TRAILER = 0xF1
KNOWN_PLAINTEXT_WORD = 0x6C6D
FIXTURE_WORD_KEY_B = 0xD2E8


def scramble_swap(buffer: bytearray) -> None:
    """The partial reversal, which is its own inverse."""
    low, high = 0, len(buffer) - 1
    while low < high:
        buffer[low], buffer[high] = buffer[high], buffer[low]
        low += 2
        high -= 2


def scramble_word_key(buffer: bytearray, start: int, key: int) -> None:
    for offset in range(start, len(buffer) - 1, 4):
        word = int.from_bytes(buffer[offset : offset + 2], "little") ^ key
        buffer[offset : offset + 2] = word.to_bytes(2, "little")


def derived_key_b(encoded_length: int) -> int:
    """The second key every retail document uses, derived from the first."""
    key_a = (encoded_length * 7) & 0xFFFF
    return ~((key_a + 1) >> ((key_a // 3) & 3)) & 0xFFFF


def scramble_document(document: bytes, key_b: int = FIXTURE_WORD_KEY_B) -> bytes:
    """Encode a document into the scrambled container."""
    encoded_length = len(document)
    buffer = bytearray(document)
    key_a = (encoded_length * 7) & 0xFFFF
    if encoded_length % 4 == 1:
        buffer[encoded_length - 1] ^= (key_a & 0xFF) ^ (key_b & 0xFF)
    scramble_word_key(buffer, 2, key_b)
    scramble_word_key(buffer, 0, key_a)
    scramble_swap(buffer)
    return bytes(buffer) + bytes([SCRAMBLE_TRAILER])


# The SQEX container. See docs/formats/sqex.md.
# The key is the fixture's own base name, so these fixtures are enciphered
# under the file name they are written to and renaming one makes it
# unreadable. The element and attribute names are invented: a synthetic
# vocabulary proves that the reader does not depend on retail names.
SQWT_SIGNATURE = b"SQEX" + bytes(4)


def widget_document(body: str, bom: bool = False) -> bytes:
    """A widget document in the shape the client writes: no declaration,
    CRLF line endings, and a byte order mark only where one is asked for."""
    return (BOM if bom else b"") + body.replace("\n", "\r\n").encode("utf-8")


def sqwt_container(relative: str, document: bytes) -> bytes:
    """Encipher a document under the base name of the path it is written
    to, which is the key the container uses."""
    name = Path(relative).name
    return SQWT_SIGNATURE + Blowfish(name.encode("utf-8")).encrypt(document)


def sheet_string(text: str, scrambled: bool = True) -> bytes:
    """One framed string value: u16 length, body, terminator.

    A scrambled body opens with 0xFF and the rest is XOR SCRAMBLE_KEY, so
    its NUL terminator reads as the key itself.
    """
    return framed_string(text.encode("utf-8"), scrambled)


def framed_string(payload: bytes, scrambled: bool = True) -> bytes:
    if scrambled:
        body = (
            bytes([SCRAMBLE_MARKER])
            + bytes(byte ^ SCRAMBLE_KEY for byte in payload)
            + bytes([SCRAMBLE_KEY])
        )
    else:
        body = payload + b"\x00"
    return struct.pack("<H", len(body)) + body


def token(code: int, payload: bytes, encoding: str = "direct") -> bytes:
    """One control token: 0x02 code length payload 0x03."""
    if encoding == "direct":
        length = bytes([len(payload) + 1])
    elif encoding == "byte":
        length = b"\xf0" + bytes([len(payload)])
    elif encoding == "byte-scaled":
        if len(payload) % 256:
            raise ValueError("the scaled length form carries whole multiples of 256")
        length = b"\xf1" + bytes([len(payload) // 256])
    elif encoding == "word":
        length = b"\xf2" + struct.pack(">H", len(payload))
    else:
        raise ValueError("unknown length encoding: {0}".format(encoding))
    return TOKEN_START + bytes([code]) + length + payload + TOKEN_END


def u32_array(values: list[int]) -> bytes:
    return b"".join(struct.pack("<I", value) for value in values)


def enable_records(pairs: list[tuple[int, int]]) -> bytes:
    return b"".join(struct.pack("<II", first, count) for first, count in pairs)


def tree_descriptor(resources: list[tuple[int, str]]) -> bytes:
    return (
        json.dumps(
            {
                "format": "ssd-extract",
                "resources": [
                    {"file": filename, "id": "0x{0:08X}".format(resource_id)}
                    for resource_id, filename in resources
                ],
                "schemaVersion": 1,
            },
            indent=2,
            sort_keys=True,
        )
        + "\n"
    ).encode("ascii")


LUA51_HEADER = b"\x1bLuaQ\x00\x01\x04\x04\x04\x08\x00"
LUA51_MAXARG_SBX = (1 << 17) - 1


def lua_instruction_abc(opcode: int, a: int, b: int, c: int) -> int:
    return opcode | (a << 6) | (c << 14) | (b << 23)


def lua_instruction_abx(opcode: int, a: int, bx: int) -> int:
    return opcode | (a << 6) | (bx << 14)


def lua_instruction_asbx(opcode: int, a: int, sbx: int) -> int:
    return lua_instruction_abx(opcode, a, sbx + LUA51_MAXARG_SBX)


def lua_string(value: bytes | None) -> bytes:
    if value is None:
        return struct.pack("<I", 0)
    return struct.pack("<I", len(value) + 1) + value + b"\x00"


def lua_proto(
    *,
    source: bytes | None = None,
    lines: tuple[int, int] = (0, 0),
    shape: tuple[int, int, int, int] = (0, 0, 0, 2),
    instructions: tuple[int, ...] = (),
    constants: tuple[bytes, ...] = (),
    nested: tuple[bytes, ...] = (),
    line_info: tuple[int, ...] = (),
    locals_: tuple[tuple[bytes, int, int], ...] = (),
    upvalues: tuple[bytes, ...] = (),
) -> bytes:
    data = bytearray(lua_string(source))
    data.extend(struct.pack("<II4B", *lines, *shape))
    data.extend(struct.pack("<I", len(instructions)))
    for instruction in instructions:
        data.extend(struct.pack("<I", instruction))
    data.extend(struct.pack("<I", len(constants)))
    for constant in constants:
        data.extend(constant)
    data.extend(struct.pack("<I", len(nested)))
    for child in nested:
        data.extend(child)
    data.extend(struct.pack("<I", len(line_info)))
    for line in line_info:
        data.extend(struct.pack("<I", line))
    data.extend(struct.pack("<I", len(locals_)))
    for name, start_pc, end_pc in locals_:
        data.extend(lua_string(name))
        data.extend(struct.pack("<II", start_pc, end_pc))
    data.extend(struct.pack("<I", len(upvalues)))
    for name in upvalues:
        data.extend(lua_string(name))
    return bytes(data)


def build_fixtures() -> dict[str, bytes]:
    fixtures: dict[str, bytes] = {}

    # Authored Lua path and LPB samples contain no retail names or bytes.
    fixtures["lua-path/example.txt"] = b"Quest/Scenario/Man0g0.lua"
    fixtures["lua-path/non-ascii.bin"] = b"ab\xc3\xa9.lua"

    lua_chunk = b"\x1bLuaQ" + pattern(19, 0x31)
    fixtures["lpb/raw.bin"] = b"rlu\x0bABCD" + lua_chunk
    xor_header = b"rle\x0cWXYZ" + struct.pack("<I", 7) + b"!"
    fixtures["lpb/xor.bin"] = xor_header + bytes(byte ^ 0x73 for byte in lua_chunk)
    fixtures["lpb/truncated.bin"] = b"rle\x0cshort"
    fixtures["lpb/bad-chunk.bin"] = b"rlu\x0bABCDxxxxx"

    # -- GTEX and PWIB resources ----------------------------------------
    # The fields and boundaries are loader-evidenced; all values and source
    # bytes are invented.
    def gtex(
        data: bytes,
        *,
        format_index: int = 4,
        flags: int = 0,
        mip_levels: int = 1,
        width: int = 4,
        height: int = 4,
        depth: int = 1,
        data_base: int = 0x20,
        surfaces: tuple[tuple[int, int], ...] = (),
    ) -> bytes:
        header = bytearray(pattern(data_base, 0x31))
        header[0:4] = b"GTEX"
        header[6] = format_index
        header[7] = mip_levels
        header[9] = flags
        header[0x0A:0x10] = struct.pack(">HHH", width, height, depth)
        table_base = 0x18 if surfaces else 0
        header[0x10:0x14] = struct.pack(">I", table_base)
        header[0x14:0x18] = struct.pack(">I", data_base)
        for index, (offset, size) in enumerate(surfaces):
            entry = table_base + index * 8
            header[entry : entry + 8] = struct.pack(">II", offset, size)
        return bytes(header) + data

    for format_index, sizes in [
        (4, (60, 8, 4)),
        (22, (30, 4, 2)),
        (24, (16, 8, 8)),
        (26, (32, 16, 16)),
    ]:
        table_end = 24 + len(sizes) * 8
        data_base = (table_end + 15) // 16 * 16
        header = bytearray(data_base)
        header[:10] = b"GTEX" + bytes((1, 1, format_index, len(sizes), 2, 0))
        struct.pack_into(">HHHII", header, 10, 5, 3, 1, 24, data_base)
        payload = bytearray()
        for level, size in enumerate(sizes):
            struct.pack_into(">II", header, 24 + level * 8, len(payload), size)
            payload.extend(pattern(size, 0x35 + level))
        fixtures[f"gtex/built-{format_index}.bin"] = bytes(header + payload)

    fixtures["gtex/tagged.bin"] = gtex(
        pattern(40, 0x44),
        mip_levels=2,
        width=4,
        height=2,
        data_base=0x28,
        surfaces=((0, 32), (32, 8)),
    )
    # Index 22 is 16-bit A4R4G4B4: 2 by 2 and 1 by 1 mips of 8 and 2 bytes.
    fixtures["gtex/a4r4g4b4.bin"] = gtex(
        pattern(10, 0x45),
        format_index=22,
        mip_levels=2,
        width=2,
        height=2,
        data_base=0x28,
        surfaces=((0, 8), (8, 2)),
    )
    fixtures["gtex/trailing.bin"] = gtex(
        pattern(48, 0x51),
        format_index=24,
        mip_levels=2,
        width=8,
        height=8,
        data_base=0x28,
        surfaces=((0, 32), (40, 8)),
    )
    # Every legacy DDS mapping is covered by an authored, table-bearing 2D
    # texture. The DXT1 chain keeps the two documented eight-byte gaps after
    # eight-byte tail mips; those gaps are excluded from the DDS payload.
    dxt1_sizes = (8192, 2048, 512, 128, 32, 8, 8, 8)
    dxt1_offsets = (0, 8192, 10240, 10752, 10880, 10912, 10928, 10944)
    dxt1_end = dxt1_offsets[-1] + dxt1_sizes[-1]
    fixtures["gtex/dds-dxt1-gapped.bin"] = gtex(
        pattern(dxt1_end, 0x83),
        format_index=24,
        mip_levels=8,
        width=128,
        height=128,
        data_base=0x58,
        surfaces=tuple(zip(dxt1_offsets, dxt1_sizes)),
    )
    fixtures["gtex/dds-dxt5.bin"] = gtex(
        pattern(80, 0x91),
        format_index=26,
        mip_levels=2,
        width=8,
        height=8,
        data_base=0x28,
        surfaces=((0, 64), (64, 16)),
    )
    fixtures["gtex/dds-dxt1-npot.bin"] = gtex(
        pattern(16, 0x96),
        format_index=24,
        width=5,
        height=3,
        surfaces=((0, 16),),
    )

    # Compact block fixtures exercise both DXT1 color modes and DXT5 alpha
    # interpolation branches. The bytes are authored test vectors, not
    # client data.
    def dxt1_block(c0: int, c1: int, selectors: int) -> bytes:
        return struct.pack("<HHI", c0, c1, selectors)

    def dxt5_block(
        a0: int, a1: int, alpha_selectors: int, c0: int, c1: int, color_selectors: int
    ) -> bytes:
        return (
            bytes([a0, a1])
            + alpha_selectors.to_bytes(6, "little")
            + struct.pack("<HHI", c0, c1, color_selectors)
        )

    fixtures["gtex/png-dxt1-four-color.bin"] = gtex(
        dxt1_block(0xF800, 0x001F, 0xE4E4E4E4),
        format_index=24,
        width=4,
        height=4,
        surfaces=((0, 8),),
    )
    fixtures["gtex/png-dxt1-three-color.bin"] = gtex(
        dxt1_block(0x001F, 0xF800, 0xE4E4E4E4),
        format_index=24,
        width=4,
        height=4,
        surfaces=((0, 8),),
    )
    fixtures["gtex/png-dxt5-alpha-branches.bin"] = gtex(
        dxt5_block(255, 0, 0xFAC688, 0xF800, 0x001F, 0xE4E4E4E4)
        + dxt5_block(17, 17, 0xFAC688, 0x07E0, 0x0000, 0xE4E4E4E4),
        format_index=26,
        width=8,
        height=4,
        surfaces=((0, 32),),
    )
    fixtures["gtex/dds-unsupported-flags.bin"] = gtex(
        pattern(128, 0x9A),
        flags=2,
        depth=2,
        surfaces=((0, 128),),
    )
    fixtures["gtex/dds-zero-dimensions.bin"] = gtex(
        pattern(16, 0xA1),
        width=0,
        surfaces=((0, 16),),
    )
    fixtures["gtex/dds-excess-mips.bin"] = gtex(
        pattern(88, 0xA8),
        mip_levels=4,
        width=4,
        height=4,
        data_base=0x38,
        surfaces=((0, 64), (64, 16), (80, 4), (84, 4)),
    )
    fixtures["gtex/dds-unmapped-format.bin"] = gtex(
        pattern(16, 0xB2),
        format_index=3,
        surfaces=((0, 16),),
    )
    fixtures["gtex/truncated-header.bin"] = b"GTEX" + pattern(10, 0x71)
    bad_gtex = bytearray(gtex(b"x"))
    bad_gtex[0x14:0x18] = struct.pack(">I", len(bad_gtex) + 1)
    fixtures["gtex/extent-out-of-range.bin"] = bytes(bad_gtex)

    def pwib(first: bytes, second: bytes, trailing: bytes = b"") -> bytes:
        first_offset = 0x10
        second_offset = first_offset + len(first)
        total_size = second_offset + len(second)
        header = b"PWIB" + struct.pack(">III", total_size, first_offset, second_offset)
        return header + first + second + trailing

    first = sedb_header("syn", 7, 2, 0x14, 37) + pattern(5, 0x62)
    fixtures["pwib/tagged.bin"] = pwib(first, pattern(12, 0x72))
    fixtures["pwib/trailing.bin"] = pwib(
        sedb_header("syn", 8, 2, 0x14, 29), pattern(9, 0x45), pattern(4, 0x55)
    )
    fixtures["pwib/truncated-header.bin"] = b"PWIB" + pattern(5, 0x55)
    fixtures["pwib/bad-nested-magic.bin"] = pwib(
        b"NOPE" + pattern(16, 0x75), pattern(4, 0x85)
    )
    bad_boundaries = bytearray(pwib(first, pattern(12, 0x72)))
    bad_boundaries[8:12] = struct.pack(">I", 0x30)
    fixtures["pwib/bad-boundaries.bin"] = bytes(bad_boundaries)
    fixtures["gtex/truncated-tag.bin"] = b"GTE"
    fixtures["pwib/truncated-tag.bin"] = b"PWI"

    def selected_pwib(
        *,
        descriptor_word: int = 0x40,
        descriptor_pointer: int = 0x40,
        surface_offset: int = 0,
        format_index: int = 24,
        width: int = 256,
        height: int = 256,
        surface_size: int = 32768,
        second_length: int | None = None,
        first_patch: tuple[int, bytes] | None = None,
        txb_patch: tuple[int, bytes] | None = None,
        descriptor_patch: tuple[int, bytes] | None = None,
        table_base: int = 0x18,
        late_resource_type: bool = False,
    ) -> bytes:
        names = [
            "RESOURCE_TYPE",
            "RESOURCE_ID",
            "block0",
            "block1",
            "block2",
            "block3",
            "block4",
            "block5",
            "block6",
            "block7",
            "block8",
        ]
        count = len(names)
        type_data = bytearray(count * 4)
        struct.pack_into("<I", type_data, 6 * 4, 0x00747862)
        id_data = pattern(count * 16, 0xA4)
        txb = bytearray(pattern(96, 0xB1))
        txb[0:8] = b"SEDBtxb\0"
        struct.pack_into("<I", txb, 0x08, 1)
        txb[0x0C] = 0
        struct.pack_into("<H", txb, 0x0E, descriptor_word)
        struct.pack_into("<I", txb, 0x30, descriptor_pointer)
        descriptor_offset = (
            0x30 + descriptor_word if descriptor_word <= 0x30 else descriptor_pointer
        )
        descriptor = bytearray(0x18)
        descriptor[0:4] = b"GTEX"
        descriptor[6] = format_index
        descriptor[7] = 1
        descriptor[9] = 0
        struct.pack_into(">HHH", descriptor, 0x0A, width, height, 1)
        struct.pack_into(">II", descriptor, 0x10, table_base, 0)
        txb[descriptor_offset : descriptor_offset + len(descriptor)] = descriptor
        struct.pack_into(
            ">II", txb, descriptor_offset + table_base, surface_offset, surface_size
        )
        if txb_patch:
            offset, patch = txb_patch
            txb[offset : offset + len(patch)] = patch
        blocks = [pattern(8 + index, 0xC0 + index) for index in range(9)]
        blocks[4] = bytes(txb)
        payload = bytearray()
        entries: list[tuple[int, int, int, int]] = []
        entries.append((0, len(payload), len(type_data), 0))
        payload.extend(type_data)
        entries.append((1, len(payload), len(id_data), 0))
        payload.extend(id_data)
        for index, block in enumerate(blocks):
            entries.append((index + 2, len(payload), len(block), 0))
            payload.extend(block)
        if late_resource_type:
            entries[0], entries[9] = entries[9], entries[0]
        names_offset = len(payload)
        names_bytes = b"".join(name.encode("ascii") + b"\0" for name in names)
        total = 0x40 + count * 16 + len(payload) + len(names_bytes)
        extended = struct.pack("<IIII", count, names_offset, count, 0)
        first = bytearray(sedb_header("RES ", 4000, 0, 0x40, total, extended))
        for index, (name_index, offset, size, kind) in enumerate(entries):
            first.extend(directory_entry(name_index, offset, size, kind))
        first.extend(payload)
        first.extend(names_bytes)
        if first_patch:
            offset, patch = first_patch
            first[offset : offset + len(patch)] = patch
        if second_length is None:
            second = bytearray(pattern(surface_offset, 0xD2))
            second.extend(pattern(surface_size, 0xD8))
            second.extend(pattern(11, 0xDE))
        else:
            second = bytearray(pattern(second_length, 0xD2))
        if descriptor_patch:
            offset, patch = descriptor_patch
            txb_start = entries[2 + 4][1] + (0x40 + count * 16)
            first[
                txb_start + descriptor_offset + offset : txb_start
                + descriptor_offset
                + offset
                + len(patch)
            ] = patch
        return pwib(bytes(first), bytes(second))

    fixtures["pwib/selected.bin"] = selected_pwib(surface_offset=16)
    fixtures["pwib/selected-short-descriptor.bin"] = selected_pwib(
        descriptor_word=0, descriptor_pointer=0x30, surface_offset=0
    )
    fixtures["pwib/selected-later-table.bin"] = selected_pwib(
        descriptor_word=0, descriptor_pointer=0x30, table_base=0x20, surface_offset=0
    )
    fixtures["pwib/selected-table-escapes.bin"] = selected_pwib(
        descriptor_word=0,
        descriptor_pointer=0x30,
        descriptor_patch=(0x10, struct.pack(">I", 0x50)),
        surface_offset=0,
    )
    fixtures["pwib/selected-late-metadata.bin"] = selected_pwib(
        late_resource_type=True, surface_offset=16
    )
    fixtures["pwib/selected-preserved-prefix.bin"] = selected_pwib(
        first_patch=(0x10, struct.pack("<I", 0xFFFF_FFFF)), surface_offset=16
    )
    fixtures["pwib/selected-bad-type.bin"] = selected_pwib(
        first_patch=(0x40 + 16 * 11 + 6 * 4, struct.pack("<I", 0x0074626E))
    )
    fixtures["pwib/selected-big-endian.bin"] = selected_pwib(
        first_patch=(0x0C, b"\x01")
    )
    fixtures["pwib/selected-bad-txb-endian.bin"] = selected_pwib(
        txb_patch=(0x0C, b"\x01")
    )
    fixtures["pwib/selected-bad-version.bin"] = selected_pwib(
        txb_patch=(0x08, struct.pack("<I", 2))
    )
    fixtures["pwib/selected-bad-directory.bin"] = selected_pwib(
        first_patch=(0x30, struct.pack("<I", 0xFFFF))
    )
    fixtures["pwib/selected-bad-names.bin"] = selected_pwib(
        first_patch=(0x34, struct.pack("<I", 0xFFFF))
    )
    fixtures["pwib/selected-bad-metadata.bin"] = selected_pwib(
        first_patch=(0x40 + 8, struct.pack("<I", 1))
    )
    fixtures["pwib/selected-bad-descriptor.bin"] = selected_pwib(
        txb_patch=(0x30, struct.pack("<I", 0xFFFF))
    )
    fixtures["pwib/selected-bad-table.bin"] = selected_pwib(
        descriptor_patch=(0x10, struct.pack(">I", 0x1000))
    )
    fixtures["pwib/selected-unsupported-format.bin"] = selected_pwib(format_index=26)
    fixtures["pwib/selected-zero-dimensions.bin"] = selected_pwib(width=0)
    fixtures["pwib/selected-size-mismatch.bin"] = selected_pwib(surface_size=8)
    fixtures["pwib/selected-surface-start-outside.bin"] = selected_pwib(
        surface_offset=2, second_length=1
    )
    fixtures["pwib/selected-surface-end-outside.bin"] = selected_pwib(
        surface_offset=0, second_length=32767
    )

    # -- the static-actor SAN table --------------------------------------
    # Only the framing is promoted: the authored strings resemble paths so
    # the positive case exercises the retail byte class without assigning a
    # meaning to either record member.
    staticactor = staticactor_san(
        [(7, b"/Synthetic/One"), (0x10203040, b"/Synthetic/Two")]
    )
    fixtures["staticactor/records.bin"] = staticactor
    fixtures["staticactor/bad-magic.bin"] = b"x" + staticactor[1:]
    fixtures["staticactor/truncated-header.bin"] = staticactor[:12]
    fixtures["staticactor/count-bomb.bin"] = staticactor_san([], declared_count=100_001)
    fixtures["staticactor/missing-record.bin"] = staticactor_san([], declared_count=1)
    fixtures["staticactor/unterminated-record.bin"] = staticactor_san(
        [], declared_count=1
    ) + bytes(byte ^ 0x73 for byte in struct.pack(">I", 9) + b"unfinished")
    one_record = staticactor_san([(7, b"/Synthetic/One")])
    fixtures["staticactor/trailing-partial-record.bin"] = one_record + b"\x01\x02"
    fixtures["staticactor/trailing-record.bin"] = staticactor_san(
        [(7, b"/Synthetic/One"), (9, b"/Synthetic/Extra")], declared_count=1
    )

    child = lua_proto(
        lines=(4, 6),
        shape=(1, 1, 0, 2),
        instructions=(
            lua_instruction_asbx(22, 0, -1),  # JMP: iAsBx to this instruction
            lua_instruction_abc(30, 0, 1, 0),  # RETURN: iABC
        ),
        constants=(b"\x04" + lua_string(b"child"),),
        line_info=(5, 6),
    )
    main = lua_proto(
        source=b"@synthetic.lua",
        lines=(0, 12),
        shape=(1, 2, 2, 4),
        instructions=(
            lua_instruction_abc(0, 1, 2, 0),  # MOVE: iABC register and unused
            lua_instruction_abx(1, 0, 0),  # LOADK: iABx constant reference
            # ADD: the RK form keeps constant and register references distinct
            # without resolving either reference to a value.
            lua_instruction_abc(12, 2, 0x100 | 3, 1),
            lua_instruction_abx(36, 0, 0),  # CLOSURE: nested prototype 0
            lua_instruction_abc(4, 0, 0, 0),  # CLOSURE binding: parent upvalue 0
            lua_instruction_abc(34, 0, 0, 0),  # SETLIST: C is the following word
            1,  # SETLIST extra word, not an opcode
            lua_instruction_asbx(22, 0, -1),  # JMP: iAsBx and excess-K bias
            lua_instruction_abc(30, 0, 1, 0),  # RETURN: iABC
        ),
        constants=(
            b"\x00",
            b"\x01\x01",
            b"\x03" + struct.pack("<d", 1.5),
            b"\x04" + lua_string(b"hello"),
        ),
        nested=(child,),
        line_info=(1, 2, 3, 4, 4, 5, 5, 6, 7),
        locals_=((b"value", 0, 9),),
        upvalues=(b"environment",),
    )
    bytecode = LUA51_HEADER + main
    fixtures["lpb/bytecode.bin"] = b"rlu\x0bBCOD" + bytecode
    fixtures["lpb/bytecode-xor.bin"] = (
        b"rle\x0cBCOD"
        + struct.pack("<I", len(bytecode))
        + b"?"
        + bytes(byte ^ 0x73 for byte in bytecode)
    )
    fixtures["lpb/bytecode-trailing.bin"] = b"rlu\x0bBCOD" + bytecode + b"!"
    unsupported = bytearray(bytecode)
    unsupported[6] = 2
    fixtures["lpb/bytecode-unsupported-header.bin"] = b"rlu\x0bBCOD" + unsupported
    invalid_opcode = lua_proto(instructions=(38,))
    fixtures["lpb/bytecode-invalid-opcode.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_opcode
    )
    invalid_constant = lua_proto(
        instructions=(
            lua_instruction_abx(1, 0, 1),
            lua_instruction_abc(30, 0, 1, 0),
        ),
        constants=(b"\x00",),
    )
    fixtures["lpb/bytecode-invalid-constant.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_constant
    )
    invalid_upvalue = lua_proto(
        instructions=(
            lua_instruction_abc(4, 0, 0, 0),
            lua_instruction_abc(30, 0, 1, 0),
        ),
    )
    fixtures["lpb/bytecode-invalid-upvalue.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_upvalue
    )
    invalid_nested = lua_proto(
        instructions=(
            lua_instruction_abx(36, 0, 0),
            lua_instruction_abc(30, 0, 1, 0),
        ),
    )
    fixtures["lpb/bytecode-invalid-nested.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_nested
    )
    invalid_register = lua_proto(
        instructions=(
            lua_instruction_abc(0, 2, 0, 0),
            lua_instruction_abc(30, 0, 1, 0),
        ),
    )
    fixtures["lpb/bytecode-invalid-register.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_register
    )
    invalid_jump = lua_proto(
        instructions=(
            lua_instruction_asbx(22, 0, -2),
            lua_instruction_abc(30, 0, 1, 0),
        ),
    )
    fixtures["lpb/bytecode-invalid-jump.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_jump
    )
    binding_child = lua_proto(
        shape=(1, 0, 0, 2),
        instructions=(lua_instruction_abc(30, 0, 1, 0),),
    )
    invalid_binding = lua_proto(
        shape=(1, 0, 0, 2),
        instructions=(
            lua_instruction_abx(36, 0, 0),
            lua_instruction_abx(1, 0, 0),
            lua_instruction_abc(30, 0, 1, 0),
        ),
        constants=(b"\x00",),
        nested=(binding_child,),
    )
    fixtures["lpb/bytecode-invalid-closure-binding.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_binding
    )
    invalid_setlist = lua_proto(
        instructions=(
            lua_instruction_abc(34, 0, 0, 0),
            lua_instruction_abc(30, 0, 1, 0),
        ),
    )
    fixtures["lpb/bytecode-invalid-setlist.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + invalid_setlist
    )
    truncated_instruction = (
        LUA51_HEADER
        + lua_string(None)
        + struct.pack("<II4BI", 0, 0, 0, 0, 0, 2, 2)
        + struct.pack("<I", lua_instruction_abx(1, 0, 0))
        + b"\x01\x00\x00"
    )
    fixtures["lpb/bytecode-instruction-truncated.bin"] = (
        b"rlu\x0bBCOD" + truncated_instruction
    )
    fixtures["lpb/bytecode-string-bomb.bin"] = (
        b"rlu\x0bBCOD" + LUA51_HEADER + struct.pack("<I", 16 * 1024 * 1024 + 1)
    )
    fixtures["lpb/bytecode-table-bomb.bin"] = (
        b"rlu\x0bBCOD"
        + LUA51_HEADER
        + lua_string(None)
        + struct.pack("<II4BI", 0, 0, 0, 0, 0, 2, 1_000_001)
    )
    deepest = lua_proto()
    for _ in range(128):
        deepest = lua_proto(nested=(deepest,))
    fixtures["lpb/bytecode-nesting-bomb.bin"] = b"rlu\x0bBCOD" + LUA51_HEADER + deepest

    # -- sedb ------------------------------------------------------------
    # -- wrb-model -------------------------------------------------------
    # These resources exercise the validated RES -> WRB model reader. The
    # values are synthetic and the reports retain only structure, counts,
    # spans, and digests.
    def model_chunk(chunk_tag: bytes, payload: bytes) -> bytes:
        size = 16 + len(payload)
        padded = (size + 15) & ~15
        result = bytearray(padded)
        result[0:4] = chunk_tag
        struct.pack_into(">II", result, 8, size, padded)
        result[16 : 16 + len(payload)] = payload
        return bytes(result)

    def model_chunk_with_declared_padded(
        chunk_tag: bytes, payload: bytes, declared_padded: int
    ) -> bytes:
        size = 16 + len(payload)
        resolved_padded = (size + 15) & ~15
        result = bytearray(resolved_padded)
        result[0:4] = chunk_tag
        struct.pack_into(">II", result, 8, size, declared_padded)
        result[16 : 16 + len(payload)] = payload
        return bytes(result)

    def model_container(chunk_tag: bytes, children: list[bytes]) -> bytes:
        return model_chunk(chunk_tag, bytes(16) + b"".join(children))

    def model_stms(
        fields: list[tuple[int, int, int, int]], count: int, stride: int, data: bytes
    ) -> bytes:
        payload = bytearray(16 + 16 * len(fields) + len(data))
        struct.pack_into(">III", payload, 0, len(fields), count, stride)
        for index, field in enumerate(fields):
            struct.pack_into(">IIII", payload, 16 + index * 16, *field)
        payload[16 + 16 * len(fields) :] = data
        return model_chunk(b"STMS", bytes(payload))

    def model_resource(wrb_payload: bytes) -> bytes:
        wrb_sedb = sedb_header("wrb", 0, 0, 0x30, 0x30 + len(wrb_payload)) + wrb_payload
        root_total = 0x50 + len(wrb_sedb)
        return (
            sedb_header(
                "RES ", 0, 0, RES_HEADER_SIZE, root_total, res_extended(1, 0, "brt")
            )
            + directory_entry(0, 0, len(wrb_sedb), 2)
            + wrb_sedb
        )

    comp = struct.pack(">6f", -1.0, -1.0, -1.0, 1.0, 1.0, 1.0)
    position_data = b"".join(
        struct.pack(">hhhh", *values) + b"\x00" * 4
        for values in ((0, 0, 0, 32767), (32767, 0, 0, 32767), (0, 32767, 0, 32767))
    )
    position_stream = model_stms(
        [(0, 3, 4, 0x02000000), (0, 4, 4, 0)], 3, 12, position_data
    )
    index_data = struct.pack(">3H", 0, 1, 2)
    index_stream = model_stms([(0, 0, 1, 0x00FF0000)], 3, 2, index_data)
    mesh = model_container(b"MESH", [position_stream, index_stream])
    model_mdl = model_container(b"MDL\0", [model_chunk(b"COMP", comp), mesh])
    model_mdlc = model_container(b"MDLC", [model_mdl])
    model_wrb = model_container(b"WRB\0", [model_mdlc])
    fixtures["wrb-model/supported.bin"] = model_resource(model_wrb)
    # Stored counts overlay opaque HEAD and MICT bodies without traversing MINS.
    count_head = model_chunk(b"HEAD", b"\xa5\x5a\x02\x03" + pattern(12, 0x61))
    count_mesh = model_container(
        b"MESH",
        [
            count_head,
            position_stream,
            index_stream,
            model_chunk(b"ENVD", pattern(19, 0x31)),
            model_chunk(b"ENVD", pattern(7, 0x41)),
            model_chunk(b"ENVD", pattern(2, 0x51)),
        ],
    )
    count_mdl = model_container(b"MDL\0", [model_chunk(b"COMP", comp), count_mesh])
    count_mict = model_chunk(
        b"MICT",
        struct.pack(">I", 0x01020304)
        + pattern(12, 0x71)
        + model_chunk(b"MINS", pattern(17, 0x81)),
    )
    fixtures["wrb-model/stored-counts.bin"] = model_resource(
        model_container(b"WRB\0", [model_container(b"MDLC", [count_mdl]), count_mict])
    )
    for label, body in (("stream", bytes(2)), ("envelope", bytes(3))):
        short_mesh = model_container(
            b"MESH", [model_chunk(b"HEAD", body), position_stream, index_stream]
        )
        short_mdl = model_container(b"MDL\0", [model_chunk(b"COMP", comp), short_mesh])
        fixtures[f"wrb-model/short-{label}-count.bin"] = model_resource(
            model_container(b"WRB\0", [model_container(b"MDLC", [short_mdl])])
        )
    fixtures["wrb-model/short-component-count.bin"] = model_resource(
        model_container(b"WRB\0", [model_chunk(b"MICT", bytes(3))])
    )
    declaration_fields = [
        (0, 4, 4, 0 << 16),
        (8, 3, 4, 1 << 16),
        (12, 3, 4, 2 << 16),
        (16, 2, 4, 3 << 16),
        (24, 2, 4, 4 << 16),
        (32, 1, 1, 5 << 16),
        (36, 1, 2, 6 << 16),
        (44, 6, 4, 7 << 16),
        (48, 2, 2, 8 << 16),
        (52, 0, 2, 9 << 16),
        (56, 0, 4, 10 << 16),
        (64, 1, 3, 11 << 16),
        (76, 1, 4, 12 << 16),
        (92, 3, 4, 13 << 16),
        (96, 4, 2, 14 << 16),
        (100, 6, 4, 15 << 16),
        (104, 3, 4, 16 << 16),
        (108, 3, 4, (17 << 16) | 0x8000),
    ]
    declaration_mesh = model_container(
        b"MESH", [model_stms(declaration_fields, 3, 112, bytes(336)), index_stream]
    )
    declaration_mdl = model_container(
        b"MDL\0", [model_chunk(b"COMP", comp), declaration_mesh]
    )
    fixtures["wrb-model/vertex-declarations.bin"] = model_resource(
        model_container(b"WRB\0", [model_container(b"MDLC", [declaration_mdl])])
    )
    opaque_fields = [
        (0, 3, 4, 18 << 16),
        (0, 3, 4, 31 << 16),
        (0, 3, 4, 32 << 16),
        (0, 3, 4, 255 << 16),
        (0, 5, 4, 2 << 16),
        (0, 9, 4, 2 << 16),
        (0, 3, 0, 2 << 16),
        (0, 3, 5, 2 << 16),
        (65536, 3, 4, 2 << 16),
        (9, 3, 4, 2 << 16),
    ]
    opaque_mesh = model_container(
        b"MESH",
        [position_stream, index_stream, model_stms(opaque_fields, 1, 12, bytes(12))],
    )
    opaque_mdl = model_container(b"MDL\0", [model_chunk(b"COMP", comp), opaque_mesh])
    fixtures["wrb-model/opaque-declarations.bin"] = model_resource(
        model_container(b"WRB\0", [model_container(b"MDLC", [opaque_mdl])])
    )
    fixtures["wrb-model/no-mesh.bin"] = model_resource(
        model_container(b"WRB\0", [model_chunk(b"HEAD", pattern(13, 0xA2))])
    )
    fixtures["wrb-model/no-mesh-stms.bin"] = model_resource(
        model_container(b"WRB\0", [model_stms([], 0, 0, b"")])
    )
    fixtures["wrb-model/small-padded-size.bin"] = model_resource(
        model_container(
            b"WRB\0",
            [model_chunk_with_declared_padded(b"HEAD", pattern(13, 0xA4), 0)],
        )
    )
    fixtures["wrb-model/unknown.bin"] = model_resource(
        model_container(
            b"WRB\0",
            [model_chunk(b"HEAD", pattern(13, 0xA3)), model_mdlc],
        )
    )
    exact_tag = bytearray(model_wrb)
    exact_tag[0:4] = b"WRBX"
    fixtures["wrb-model/exact-tag.bin"] = model_resource(bytes(exact_tag))
    bad_descriptor_stream = bytearray(position_stream)
    struct.pack_into(">I", bad_descriptor_stream, 16, 0x1000001)
    bad_descriptor_mdl = model_container(
        b"MDL\0",
        [
            model_chunk(b"COMP", comp),
            model_container(b"MESH", [bytes(bad_descriptor_stream), index_stream]),
        ],
    )
    fixtures["wrb-model/bad-descriptor.bin"] = model_resource(
        model_container(b"WRB\0", [model_container(b"MDLC", [bad_descriptor_mdl])])
    )
    bad_count_stream = bytearray(position_stream)
    struct.pack_into(">I", bad_count_stream, 20, 0x1000001)
    bad_count_mdl = model_container(
        b"MDL\0",
        [
            model_chunk(b"COMP", comp),
            model_container(b"MESH", [bytes(bad_count_stream), index_stream]),
        ],
    )
    fixtures["wrb-model/bad-count.bin"] = model_resource(
        model_container(b"WRB\0", [model_container(b"MDLC", [bad_count_mdl])])
    )
    truncated_chunk = bytearray(model_wrb)
    struct.pack_into(">I", truncated_chunk, 8, len(truncated_chunk) + 16)
    fixtures["wrb-model/truncated.bin"] = model_resource(bytes(truncated_chunk))
    deep = model_chunk(b"HEAD", b"")
    for _ in range(66):
        deep = model_container(b"AABB", [deep])
    fixtures["wrb-model/nesting.bin"] = model_resource(
        model_container(b"WRB\0", [deep])
    )

    # -- map-layout ------------------------------------------------------
    # The resource rows and lyb graph exercise the documented collision path
    # without carrying geometry or bytes from a client resource.
    def map_layout_resource(
        rows: list[tuple[str, str, int, int]], payload: bytes
    ) -> bytes:
        scene_offset = 0x40 + 32 * len(rows)
        header = bytearray(scene_offset)
        header[:24] = b"MapLayoutResourceData".ljust(24, b"\0")
        header[0x18:0x20] = b"1.1.0".ljust(8, b"\0")
        struct.pack_into("<II", header, 0x20, scene_offset, len(rows))
        for index, (name, type_tag, resource_id, unknown) in enumerate(rows):
            at = 0x40 + index * 32
            header[at : at + 16] = name.encode("ascii").ljust(16, b"\0")
            header[at + 16 : at + 20] = tag(type_tag)
            struct.pack_into("<II", header, at + 20, resource_id, unknown)
        total_size = 0x30 + len(payload)
        payload = bytearray(payload)
        struct.pack_into("<I", payload, 4, total_size)
        return bytes(header) + sedb_header("lyb", 0, 0, 0x30, total_size) + payload

    layout_rows = [
        ("mesh", "bhp", 0x01020304, 2),
        ("mesh", "brt", 0x05060708, 0x102),
        ("placed", "", 0, 0),
    ]
    layout_types = [
        ("root", 1, 0x14, [(0, 0), (8, 2), (16, 0)]),
        ("class", 5, 0x14, [(4, 0), (8, 2), (16, 0)]),
        (
            "settings",
            1,
            0x70,
            [
                (4, 0),
                (8, 2),
                (0x1C, 2),
                (0x4C, 0),
                (0x50, 0),
                (0x54, 0),
                (0x5C, 0),
                (0x60, 0),
            ],
        ),
        (
            "instance",
            1,
            0x40,
            [
                (4, 0),
                (8, 2),
                (0x10, 0),
                (0x2C, 0),
                (0x30, 0),
                (0x34, 0),
                (0x3C, 0),
            ],
        ),
        (
            "tree",
            1,
            0x50,
            [
                (4, 0),
                (8, 2),
                (0x1C, 0),
                (0x20, 0),
                (0x24, 0),
                (0x2C, 2),
                (0x30, 0),
            ],
        ),
        ("group", 1, 0x30, [(0x0C, 0), (0x10, 0), (0x14, 0), (0x1C, 2), (0x20, 0)]),
        ("child", 2, 0x30, [(0x0C, 0), (0x10, 0), (0x14, 2), (0x18, 0), (0x20, 0)]),
        ("attribute", 1, 0x2C, [(4, 0), (8, 2), (0x14, 0), (0x1C, 0), (0x24, 2)]),
        ("box", 1, 0x14, [(4, 0), (8, 2)]),
        ("range", 1, 0x0C, [(0, 0)]),
    ]
    layout_node_table = 0x18 + 3 * 4
    layout_payload = bytearray(layout_node_table + 16 * len(layout_types))
    layout_payload[:4] = b"lyb\0"
    struct.pack_into("<IIII", layout_payload, 8, 0, 0x00020004, layout_node_table, 0x18)

    def layout_allocate(data: bytes) -> int:
        offset = len(layout_payload)
        layout_payload.extend(data)
        layout_payload.extend(bytes(-len(layout_payload) % 4))
        return offset

    layout_field_lists = {}
    layout_type_rows = {}
    for index, (name, _, _, fields) in enumerate(layout_types):
        layout_type_rows[name] = layout_node_table + index * 16
        descriptors = b"".join(struct.pack("<HH", *field) for field in fields)
        # Reserve the nine-descriptor declaration's width so only the group
        # declaration changes in that malformed fixture, not graph offsets.
        if name == "group":
            descriptors = descriptors.ljust(9 * 4, b"\0")
        layout_field_lists[name] = layout_allocate(descriptors)
    layout_extents = {
        name: layout_allocate(bytes(12)) for name, _, _, _ in layout_types
    }
    layout_nodes = {}
    for name, count, width, fields in layout_types:
        layout_nodes[name] = layout_allocate(bytes(count * width))
        struct.pack_into(
            "<IIII",
            layout_payload,
            layout_type_rows[name],
            layout_extents[name],
            layout_field_lists[name],
            1,
            len(fields),
        )
        struct.pack_into(
            "<III",
            layout_payload,
            layout_extents[name],
            layout_nodes[name],
            count,
            width,
        )
    layout_rotation = layout_allocate(struct.pack("<4f", 0.0, 0.25, 0.0, 0.0))
    layout_scale = layout_allocate(struct.pack("<4f", 1.0, 2.0, 1.0, 0.0))

    def layout_string(text: str) -> int:
        return layout_allocate(text.encode("ascii") + b"\0")

    layout_class_paths = [
        "MiscObjects/LaySettings/LaySettingsObject",
        "RefObjects/InstanceObject",
        "RefObjects/UnitTree/UnitTreeObject",
        "BaseObjects/Attribute/AttributeBaseObject",
        "BaseObjects/CollisionBox/CollisionBoxBaseObject",
    ]
    for index, (name, class_path) in enumerate(
        zip(("settings", "instance", "tree", "attribute", "box"), layout_class_paths)
    ):
        class_node = layout_nodes["class"] + index * 0x14
        path_string = layout_string(class_path)
        struct.pack_into("<I", layout_payload, class_node + 8, path_string)
        struct.pack_into(
            "<II", layout_payload, layout_nodes[name], index + 1, class_node
        )
    layout_zone_name = layout_string("synthetic-zone")
    layout_root_name = layout_string("synthetic-zone/Test")
    layout_instance_name = layout_string("placed")
    layout_key = layout_string("mesh")
    root = layout_nodes["root"]
    settings = layout_nodes["settings"]
    instance = layout_nodes["instance"]
    tree = layout_nodes["tree"]
    group = layout_nodes["group"]
    child = layout_nodes["child"]
    attribute = layout_nodes["attribute"]
    box = layout_nodes["box"]
    child_range = layout_nodes["range"]
    # The root child array is exactly the tail of the object list.
    struct.pack_into("<III", layout_payload, 0x18, root, settings, instance)
    struct.pack_into("<III", layout_payload, root, 0x1C, 2, layout_root_name)
    struct.pack_into("<I", layout_payload, settings + 8, layout_zone_name)
    struct.pack_into("<3f", layout_payload, settings + 0x40, 10.0, 0.0, 0.0)
    struct.pack_into("<I", layout_payload, instance + 8, layout_instance_name)
    struct.pack_into("<3f", layout_payload, instance + 0x20, 1.0, 0.0, 0.0)
    struct.pack_into(
        "<IIIII",
        layout_payload,
        instance + 0x2C,
        layout_rotation,
        layout_scale,
        child_range,
        1,
        tree,
    )
    struct.pack_into("<3f", layout_payload, tree + 0x10, 2.0, 0.0, 0.0)
    struct.pack_into("<II", layout_payload, tree + 0x24, group, 1)
    struct.pack_into("<II", layout_payload, tree + 0x30, child + 0x30, 1)
    struct.pack_into("<3f", layout_payload, group, 3.0, 0.0, 0.0)
    struct.pack_into("<II", layout_payload, group + 0x20, child, 1)
    struct.pack_into("<3f", layout_payload, child, 4.0, 0.0, 0.0)
    struct.pack_into("<I", layout_payload, child + 0x18, attribute)
    struct.pack_into("<3f", layout_payload, child + 0x30, 0.0, 5.0, 0.0)
    struct.pack_into("<I", layout_payload, child + 0x30 + 0x18, box)
    struct.pack_into("<II", layout_payload, attribute + 0x24, layout_key, 1)
    struct.pack_into("<I", layout_payload, box + 0x0C, 0x02020000)
    struct.pack_into("<III", layout_payload, child_range, child, 1, 0)
    layout_supported = map_layout_resource(layout_rows, bytes(layout_payload))
    fixtures["map-layout/supported.bin"] = layout_supported
    # Bytes beyond the SEDB extent remain visible without changing the graph.
    fixtures["map-layout/trailing.bin"] = layout_supported + pattern(16, 0xB3)
    layout_scene_offset = 0x40 + 32 * len(layout_rows)
    layout_payload_start = layout_scene_offset + 0x30
    # Each refusal isolates the named framing field or node declaration.
    layout_bad_magic = bytearray(layout_supported)
    layout_bad_magic[0] = ord("X")
    fixtures["map-layout/bad-magic.bin"] = bytes(layout_bad_magic)
    layout_bad_scene = bytearray(layout_supported)
    struct.pack_into("<I", layout_bad_scene, 0x20, layout_scene_offset + 4)
    fixtures["map-layout/scene-offset.bin"] = bytes(layout_bad_scene)
    layout_not_lyb = bytearray(layout_supported)
    layout_not_lyb[layout_scene_offset + 4 : layout_scene_offset + 8] = tag("txb")
    fixtures["map-layout/not-lyb.bin"] = bytes(layout_not_lyb)
    layout_bad_size = bytearray(layout_supported)
    struct.pack_into(
        "<I", layout_bad_size, layout_payload_start + 4, len(layout_payload)
    )
    fixtures["map-layout/lyb-size.bin"] = bytes(layout_bad_size)
    layout_nine_group = bytearray(layout_supported)
    nine_group_fields = [
        (0, 0),
        (8, 2),
        (0x0C, 2),
        (0x10, 2),
        (0x14, 0),
        (0x1C, 2),
        (0x20, 2),
        (0x24, 2),
        (0x28, 2),
    ]
    struct.pack_into(
        "<I",
        layout_nine_group,
        layout_payload_start + layout_type_rows["group"] + 12,
        9,
    )
    for index, field in enumerate(nine_group_fields):
        struct.pack_into(
            "<HH",
            layout_nine_group,
            layout_payload_start + layout_field_lists["group"] + index * 4,
            *field,
        )
    fixtures["map-layout/nine-descriptor-group.bin"] = bytes(layout_nine_group)
    layout_bad_extent = bytearray(layout_supported)
    struct.pack_into(
        "<I",
        layout_bad_extent,
        layout_payload_start + layout_extents["child"] + 4,
        len(layout_payload) // 0x30 + 1,
    )
    fixtures["map-layout/extent-outside.bin"] = bytes(layout_bad_extent)

    # Keep the original scene and replace its box with an authored BGChip.
    layout_binding = bytearray(layout_payload)

    def binding_allocate(data: bytes) -> int:
        offset = len(layout_binding)
        layout_binding.extend(data)
        layout_binding.extend(bytes(-len(layout_binding) % 4))
        return offset

    chip_fields = [(4, 0), (8, 2), (20, 0), (32, 2), (36, 2), (40, 0), (44, 2)]
    chip_field_list = binding_allocate(
        b"".join(struct.pack("<HH", *field) for field in chip_fields)
    )
    chip = binding_allocate(bytes(0x54))
    chip_path = binding_allocate(b"BaseObjects/BG/BGChipBaseObject\0")
    chip_class = layout_nodes["class"] + 4 * 0x14
    struct.pack_into("<I", layout_binding, chip_class, 0xA1B2C3D4)
    struct.pack_into("<I", layout_binding, chip_class + 8, chip_path)
    struct.pack_into("<I", layout_binding, chip + 4, chip_class)
    struct.pack_into("<I", layout_binding, chip + 0x20, layout_key)
    struct.pack_into("<I", layout_binding, layout_type_rows["box"] + 4, chip_field_list)
    struct.pack_into(
        "<I", layout_binding, layout_type_rows["box"] + 12, len(chip_fields)
    )
    struct.pack_into("<III", layout_binding, layout_extents["box"], chip, 1, 0x54)
    struct.pack_into("<I", layout_binding, child + 0x30 + 0x18, chip)
    struct.pack_into("<I", layout_binding, child + 0x30 + 0x1C, 0x10203040)
    layout_binding[instance + 0x14] = 0xAD
    struct.pack_into("<II", layout_binding, child_range + 4, 0x10005, 0x10203040)
    fixtures["map-layout/binding-fields.bin"] = map_layout_resource(
        layout_rows, bytes(layout_binding)
    )

    # -- zone-project ----------------------------------------------------
    # Authoring templates have the writer's bounds and retained allocations.
    floor_vertices = [(-16, 0, -16), (16, 0, -16), (16, 0, 16), (-16, 0, 16)]
    floor_indices = [(0, 2, 1), (0, 3, 2)]
    floor_obj = (
        "o floor\n"
        + "".join(f"v {x} {y} {z}\n" for x, y, z in floor_vertices)
        + "f 1 3 2\nf 1 4 3\n"
    )
    fixtures["zone-project/floor.obj"] = floor_obj.encode("ascii")
    fixtures["zone-project/collision.json"] = (
        json.dumps(
            {
                "schemaVersion": 1,
                "vertices": floor_vertices,
                "triangles": [
                    {"indices": indices, "surface": 0} for indices in floor_indices
                ],
            },
            indent=2,
        )
        + "\n"
    ).encode("ascii")

    bounds = struct.pack(">6f", -1, -1, -1, 1, 1, 1)
    author_positions = model_stms(
        [(8, 3, 4, 0x00020000), (0, 4, 4, 0)],
        4,
        12,
        b"".join(
            struct.pack(">4h4x", *v, 32767)
            for v in (
                (-32767, 0, -32767),
                (32767, 0, -32767),
                (32767, 0, 32767),
                (-32767, 0, 32767),
            )
        ),
    )
    author_indices = model_stms(
        [(0, 0, 1, 0x00FF0000)], 6, 2, struct.pack(">6H", 0, 2, 1, 0, 3, 2)
    )
    author_mesh = model_container(
        b"MESH",
        [
            model_chunk(b"HEAD", b"KEEP" + struct.pack(">II", 0, 2)),
            author_indices,
            author_positions,
            model_container(b"AABB", [model_chunk(b"AABB", bounds)]),
        ],
    )
    author_mdl = model_container(
        b"MDL\0",
        [
            model_container(b"AABB", [model_chunk(b"AABB", bounds)]),
            model_chunk(b"COMP", bounds),
            author_mesh,
        ],
    )
    fixtures["zone-project/client/model.bin"] = model_resource(
        model_container(b"WRB\0", [model_container(b"MDLC", [author_mdl])])
    )

    phb = bytearray(0x4D0)
    phb[:8] = b"SEDBPHB\0"
    struct.pack_into("<IHHI", phb, 8, 0x120, 0x400, 0x48, 0x48)
    phb[0x20:0x24] = b"KEEP"
    gbd = 0xC0
    phb[gbd : gbd + 8] = b"PHB.GBD\0"
    for at, value in {
        8: 0x400,
        0xC: 0x212,
        0x10: 1,
        0x14: 0x80,
        0x18: 0x80,
        0x1C: 0x100,
        0x20: 0x80,
        0x24: 0x180,
        0x28: 0x80,
        0x2C: 0x200,
        0x30: 0xF0,
        0x34: 1,
        0x38: 0x380,
        0x3C: 4,
        0x60: 0x300,
        0x64: 2,
        0x70: 0x400,
        0x74: 0x400,
        0x78: 0x400,
        0x100: 0xFFFF0000,
        0x184: 0xF0,
        0x208: 0x100,
        0x20C: 0xF0,
        0x230: 1,
        0x234: 0x50,
        0x238: 0x60,
        0x250: 0x70,
        0x260: 0x80,
        0x278: 0x80,
        0x27C: 1,
        0x280: 0x50,
        0x284: 2,
        0x288: 0x70,
    }.items():
        struct.pack_into("<I", phb, gbd + at, value)
    for at, pad in ((0x40, 0.005), (0x210, 0.01), (0x290, 0.005)):
        struct.pack_into(
            "<8f",
            phb,
            gbd + at,
            -16 - pad,
            -pad,
            -16 - pad,
            1,
            16 + pad,
            pad,
            16 + pad,
            1,
        )
    phb[gbd + 0x200 : gbd + 0x208] = b"PHB.TBC\0"
    phb[gbd + 0x270 : gbd + 0x278] = b"PHB.TBD\0"
    struct.pack_into(
        "<18H",
        phb,
        gbd + 0x2C0,
        0x4000,
        0x4000,
        0x8000,
        1,
        0xC000,
        0xC000,
        0x8000,
        0x4000,
        0xFFFF,
        0xFFFF,
        0xFFFF,
        3,
        0,
        0,
        0,
        0xFFFF,
        0,
        1,
    )
    for index, indices in enumerate(floor_indices):
        struct.pack_into("<4H", phb, gbd + 0x300 + index * 8, *indices, 0)
    for index, vertex in enumerate(floor_vertices):
        struct.pack_into("<3f", phb, gbd + 0x380 + index * 12, *vertex)
    phb[gbd + 0x2F0 : gbd + 0x300] = b"\xa5" * 16
    phb[gbd + 0x400 :] = b"OPAQUE_PHB_TAIL_"
    fixtures["zone-project/client/collision-template.bin"] = bytes(phb)
    # The old binding is readable but outside the author's template profile.
    struct.pack_into("<I", phb, gbd + 0xC, 0x213)
    fixtures["zone-project/client/old-collision.bin"] = bytes(phb)
    fixtures["zone-project/client/unplaced.bin"] = (
        b"authored opaque direct dependency\n"
    )

    project_types = [
        (name, 4 if name in ("class", "instance") else 1, width, fields)
        for name, _, width, fields in layout_types
        if name not in ("group", "box")
    ]
    node_table = 0x18 + 6 * 4
    scene = bytearray(node_table + len(project_types) * 16)
    scene[:4] = b"lyb\0"
    struct.pack_into("<IIII", scene, 8, 0, 0x00020004, node_table, 0x18)

    def project_allocate(data: bytes) -> int:
        offset = len(scene)
        scene.extend(data)
        scene.extend(bytes(-len(scene) % 4))
        return offset

    nodes = {}
    for index, (name, count, width, fields) in enumerate(project_types):
        descriptors = project_allocate(b"".join(struct.pack("<HH", *v) for v in fields))
        extent = project_allocate(bytes(12))
        nodes[name] = project_allocate(bytes(count * width))
        struct.pack_into(
            "<4I", scene, node_table + index * 16, extent, descriptors, 1, len(fields)
        )
        struct.pack_into("<3I", scene, extent, nodes[name], count, width)
    for index, (name, class_path) in enumerate(
        zip(("settings", "instance", "tree", "attribute"), layout_class_paths[:4])
    ):
        class_node = nodes["class"] + index * 0x14
        pointer = project_allocate(class_path.encode("ascii") + b"\0")
        struct.pack_into("<I", scene, class_node + 8, pointer)
        for item in range(4 if name == "instance" else 1):
            struct.pack_into(
                "<II", scene, nodes[name] + item * 0x40, index + 1, class_node
            )
    zone_name = project_allocate(b"synthetic-floor\0")
    root_name = project_allocate(b"synthetic-floor/Test\0")
    instance_name = project_allocate(b"tile\0")
    resource_key = project_allocate(b"floor\0")
    struct.pack_into(
        "<6I",
        scene,
        0x18,
        nodes["root"],
        nodes["settings"],
        *(nodes["instance"] + index * 0x40 for index in range(4)),
    )
    struct.pack_into("<3I", scene, nodes["root"], 0x1C, 5, root_name)
    struct.pack_into("<I", scene, nodes["settings"] + 8, zone_name)
    translations = [(16, 0, -16), (16, 0, 16), (-16, 0, 16), (-16, 0, -16)]
    for index, translation in enumerate(translations):
        at = nodes["instance"] + index * 0x40
        struct.pack_into("<I", scene, at + 8, instance_name)
        struct.pack_into("<3f", scene, at + 0x20, *translation)
        struct.pack_into("<3I", scene, at + 0x34, nodes["range"], 1, nodes["tree"])
    struct.pack_into("<2I", scene, nodes["tree"] + 0x30, nodes["child"], 1)
    struct.pack_into("<I", scene, nodes["child"] + 0x18, nodes["attribute"])
    struct.pack_into("<2I", scene, nodes["attribute"] + 0x24, resource_key, 1)
    struct.pack_into("<3I", scene, nodes["range"], nodes["child"], 1, 0)
    project_rows = [
        ("tile", "", 0, 0),
        ("floor", "brt", 0x01020304, 0x102),
        ("floor", "bhp", 0x05060708, 2),
        ("opaque", "tex", 0x090A0B0C, 3),
    ]
    fixtures["zone-project/client/layout.bin"] = map_layout_resource(
        project_rows, bytes(scene)
    )

    def project_reference(path: str, *, client: bool = False) -> dict:
        data = fixtures["zone-project/" + ("client/" if client else "") + path]
        return {
            "path": path,
            "size": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }

    project = {
        "schemaVersion": 1,
        "profile": "floor-blockout-v1",
        "layout": project_reference("layout.bin", client=True),
        "resources": [
            {
                "row": row,
                "id": resource_id,
                "tag": type_tag,
                "file": project_reference(path, client=True),
            }
            for row, resource_id, type_tag, path in (
                (1, "0x01020304", "brt", "model.bin"),
                (2, "0x05060708", "bhp", "old-collision.bin"),
                (3, "0x090A0B0C", "tex", "unplaced.bin"),
            )
        ],
        "collisionTemplate": project_reference("collision-template.bin", client=True),
        "geometry": {
            "renderObj": project_reference("floor.obj"),
            "collisionJson": project_reference("collision.json"),
        },
        "binding": {
            "resourceKey": "floor",
            "modelRow": 1,
            "collisionRow": 2,
            "outputModelId": "0xFFFF0001",
            "outputCollisionId": "0xFFFF0002",
        },
        "expectedPlacements": [
            {
                "sourceNode": nodes["attribute"],
                "sourceBlock": nodes["attribute"],
                "worldMatrix": [[1, 0, 0, x], [0, 1, 0, y], [0, 0, 1, z], [0, 0, 0, 1]],
            }
            for x, y, z in translations
        ],
    }
    fixtures["zone-project/project.json"] = (
        json.dumps(project, indent=2) + "\n"
    ).encode("ascii")

    # A well-formed non-composite container: 0x30 header, 0x20 payload, and
    # a totalSize that covers the whole file as a top-level container does.
    plain_payload = pattern(0x20, 0x11)
    plain = sedb_header("txb", 1, 0x0200, 0x30, 0x50) + plain_payload
    fixtures["sedb/plain-container.bin"] = plain

    # The same container followed by eight unaccounted bytes. They must
    # survive into the report as trailing entries rather than vanish.
    fixtures["sedb/trailing-bytes.bin"] = plain + pattern(8, 0x40)

    # Ends mid-header: the flags field is not there. Expect
    # unexpected-end-of-input at offset 12.
    fixtures["sedb/truncated-header.bin"] = MAGIC + tag("txb") + struct.pack("<I", 1)

    # Wrong signature. Expect bad-magic at offset 0.
    fixtures["sedb/bad-magic.bin"] = b"SEDX" + tag("txb") + bytes(0x18)

    # headerSize below the fixed fields. Expect header-too-small at 0x0E.
    fixtures["sedb/header-too-small.bin"] = sedb_header(
        "txb", 1, 0, 0x10, 0x20
    ) + bytes(0x0C)

    # headerSize past the declared container. Expect
    # header-size-out-of-range at 0x0E.
    fixtures["sedb/header-size-out-of-range.bin"] = sedb_header(
        "txb", 1, 0, 0xFFFF, 0x20
    )[:0x20]

    # The 0x10 field points past the end of the input. Expect
    # declared-size-out-of-range at 0x10.
    fixtures["sedb/declared-size-out-of-range.bin"] = sedb_header(
        "txb", 1, 0, 0x30, 0x1000
    ) + bytes(0x10)

    # The 0x10 field reads zero, below the container's own header. This is
    # the shape of the 145 retail mtb resources: not malformed, so the
    # parser falls back to the header extent, records the anomaly, and
    # reports the remainder as trailing bytes.
    fixtures["sedb/declared-size-below-header.bin"] = sedb_header(
        "mtb", 0x43, 0, 0x40, 0
    ) + pattern(0x20, 0x61)

    # -- res -------------------------------------------------------------
    # A composite container with two subresources, a nested SEDB child, and
    # a deliberate hole between the two extents.
    nested = sedb_header("shd", 0x1D, 0, 0x20, 0x30) + pattern(0x10, 0x21)
    assert len(nested) == 0x30
    gap = pattern(0x10, 0x31)
    tail = b"F00\\synthetic\\name-table".ljust(0x20, b"\x00")
    assert len(tail) == 0x20
    two = (
        sedb_header(
            "RES ", 0xFA0, 0, RES_HEADER_SIZE, 0xC0, res_extended(2, 0x60, "brt")
        )
        + directory_entry(0, 0x00, 0x30, 2)
        + directory_entry(1, 0x40, 0x20, 0)
        + nested
        + gap
        + tail
    )
    assert len(two) == 0xC0
    fixtures["res/two-subresources.bin"] = two

    # The declared 0x30 header leaves the extended fields before the directory at 0x40.
    short_header = (
        sedb_header("RES ", 0xFA0, 0, 0x30, 0x64, res_extended(2, 0, "test"))
        + directory_entry(0, 0, 3, 0)
        + directory_entry(1, 3, 1, 0)
        + pattern(4, 0x37)
    )
    assert len(short_header) == 0x64
    fixtures["res/header-0x30.bin"] = short_header
    short_directory_overrun = bytearray(short_header)
    struct.pack_into("<I", short_directory_overrun, 0x10, 0x58)
    fixtures["res/header-0x30-directory-overrun.bin"] = bytes(short_directory_overrun)

    # One subresource whose declared extent runs 0x16 bytes past the end of
    # the file, the alignment slack the evidence document records. The
    # parser clamps it and reports the clamp. The case still succeeds.
    clamped = (
        sedb_header(
            "RES ", 0xFA0, 0, RES_HEADER_SIZE, 0x6A, res_extended(1, 0x50, "brt")
        )
        + directory_entry(0, 0x00, 0x30, 4)
        + pattern(0x1A, 0x51)
    )
    assert len(clamped) == 0x6A
    fixtures["res/clamped-extent.bin"] = clamped

    # The two subresource counts disagree. Expect
    # subresource-count-mismatch at 0x38.
    fixtures["res/count-mismatch.bin"] = sedb_header(
        "RES ",
        0xFA0,
        0,
        RES_HEADER_SIZE,
        0x60,
        res_extended(2, 0x60, "brt", repeated=3),
    ) + bytes(0x20)

    # A directory that cannot fit in the container. Expect
    # subresource-count-out-of-range at 0x30.
    fixtures["res/count-out-of-range.bin"] = sedb_header(
        "RES ", 0xFA0, 0, RES_HEADER_SIZE, 0x60, res_extended(0x1000, 0x60, "brt")
    ) + bytes(0x20)

    # A 0x30 RES header without a nonempty directory remains unsupported.
    fixtures["res/header-too-small.bin"] = sedb_header(
        "RES ", 0xFA0, 0, 0x30, 0x60
    ) + bytes(0x30)

    # -- resource-path ---------------------------------------------------
    fixtures["resource-path/ids.txt"] = (
        "# Resource identifiers for the resource-path conformance case.\n"
        "# Both text forms, both cases, and both ends of the 32-bit space.\n"
        "\n"
        "0x00000000\n"
        "0xFFFFFFFF\n"
        "0x29D90001\n"
        "89940554\n"
        "0x7eb50002\n"
        "0x000000FF\n"
        "0xFF000000\n"
    ).encode("ascii")

    # Seven hexadecimal digits. Expect invalid-resource-id at the offset of
    # the digits in the file.
    fixtures["resource-path/malformed-id.txt"] = (
        "# One identifier short of eight digits.\n0x29D9000\n"
    ).encode("ascii")

    # -- ssd documents ---------------------------------------------------
    # These carry a byte order mark, so they are byte fixtures rather than
    # text ones and take the .bin suffix the rest of the tree uses.

    # A master: sheets that are references to the documents defining them,
    # by decimal resource id.
    fixtures["ssd/master.bin"] = xml_document(
        '<ssd version="0.1">\n'
        '  <sheet name="synthetic/first" infofile="664076291" />\n'
        '  <sheet name="synthetic/second" infofile="45613057" />\n'
        "</ssd>\n"
    )

    # A schema document: two sheets, one language tagged, covering the five
    # base column types used by the compact fixture.
    fixtures["ssd/sheet-info.bin"] = xml_document(
        '<ssd version="0.1">\n'
        '  <sheet name="synthetic/first" mode="client" column_max="5" column_count="5"'
        ' cache="-1" type="none" lang="ja" infofile="">\n'
        "    <type>\n"
        "      <param>str</param>\n"
        "      <param>s32</param>\n"
        "      <param>bool</param>\n"
        "      <param>float</param>\n"
        "      <param>u8</param>\n"
        "    </type>\n"
        "    <index>\n"
        "      <param>0</param>\n"
        "      <param>3</param>\n"
        "    </index>\n"
        '    <block count="2">\n'
        '      <file begin="10000" count="7" offset="664076320" enable="664076319">664076318</file>\n'
        '      <file begin="30000" count="10" offset="664076393" enable="664076392">664076391</file>\n'
        "    </block>\n"
        "  </sheet>\n"
        '  <sheet name="synthetic/second" mode="client" column_max="1" column_count="1"'
        ' cache="0" type="all" infofile="">\n'
        "    <type><param>str</param></type>\n"
        "    <index><param>0</param></index>\n"
        '    <block count="1">\n'
        '      <file begin="0" count="3" offset="45613059" enable="45613058">45613057</file>\n'
        "    </block>\n"
        "  </sheet>\n"
        "</ssd>\n"
    )

    # An element that never closes. Expect malformed-xml.
    fixtures["ssd/unclosed-element.bin"] = xml_document(
        '<ssd version="0.1">\n  <sheet name="a" infofile="1">\n'
    )

    # A comment, and an entity reference. Both are outside the subset these
    # documents use, so both expect unsupported-xml-construct rather than a
    # reading this project has no evidence for.
    fixtures["ssd/comment.bin"] = xml_document(
        '<ssd version="0.1">\n  <!-- a note -->\n  <sheet name="a" infofile="1" />\n</ssd>\n'
    )
    fixtures["ssd/entity.bin"] = xml_document(
        '<ssd version="0.1">\n  <sheet name="a&amp;b" infofile="1" />\n</ssd>\n'
    )

    # A well-formed document that is not an ssd document. Expect
    # unexpected-element.
    fixtures["ssd/wrong-root.bin"] = xml_document(
        '<sdd version="0.1">\n  <sheet name="a" infofile="1" />\n</sdd>\n'
    )

    # A truncated multi-byte sequence in a name. Expect invalid-utf8.
    fixtures["ssd/not-utf8.bin"] = (
        BOM
        + b'<?xml version="1.0" encoding="utf-8"?>\r\n<ssd version="0.1">\r\n  <sheet name="'
        + b"\xc3\x28"
        + b'" infofile="1" />\r\n</ssd>\r\n'
    )

    # An infofile that is not a decimal id. Expect invalid-attribute-value.
    fixtures["ssd/bad-infofile.bin"] = xml_document(
        '<ssd version="0.1">\n  <sheet name="a" infofile="later" />\n</ssd>\n'
    )

    # -- whole-directory SSD extraction trees ---------------------------
    # These descriptors name authored byte blobs by explicit resource id.
    # The conformance runner materializes the ids under data/AA/BB/CC/DD.DAT
    # before calling the same directory exporter as the CLI.
    def block(begin: int, count: int, data: int, enable: int, offsets: int) -> str:
        return (
            '<block count="1"><file begin="{1}" count="{0}" '
            'offset="{4}" enable="{3}">{2}</file></block>'
        ).format(count, begin, data, enable, offsets)

    def sheet(
        name: str,
        body: str,
        *,
        count: int = 3,
        column_count: int = 3,
        types: str = "<param>str</param><param>u8</param><param>bool</param>",
        indexes: str = "<param>0</param><param>1</param><param>2</param>",
    ) -> str:
        return (
            '<sheet name="{0}" mode="client" column_max="3" '
            'column_count="{1}" cache="0" type="all" infofile="">'
            "<type>{2}</type><index>{3}</index>{4}</sheet>"
        ).format(name, column_count, types, indexes, body)

    def document(sheets: str) -> bytes:
        return xml_document('<ssd version="0.1">{0}</ssd>\n'.format(sheets))

    def typed_row(text: str, number: int, flag: int, *, missing: bool = False) -> bytes:
        value = sheet_string(text, scrambled=False) + bytes([number])
        return value if missing else value + bytes([flag])

    def string_row(text: str, number: int, final: str | None = None) -> bytes:
        value = sheet_string(text, scrambled=False) + struct.pack("<i", number)
        return value if final is None else value + sheet_string(final, scrambled=False)

    def add_tree(name: str, resources: list[tuple[int, str, bytes]]) -> None:
        for resource_id, filename, data in resources:
            fixtures["ssd-extract/{0}".format(filename)] = data
        fixtures["ssd-extract/{0}.json".format(name)] = tree_descriptor(
            [(resource_id, filename) for resource_id, filename, _ in resources]
        )

    main_with_absent = (
        block(100, 3, 0x12000010, 0x1200002A, 0x120000F0)
        .replace('<block count="1">', '<block count="2">', 1)
        .replace(
            "</block>",
            '<file begin="500" count="1" offset="301990000" enable="301989900">301989800</file></block>',
            1,
        )
    )
    positive_a = document(
        sheet(
            "synthetic/main",
            main_with_absent,
        )
        + sheet("synthetic/main", block(100, 3, 0x12000013, 0x1200002D, 0x120000F3))
        + sheet(
            "synthetic/other",
            block(200, 1, 0x12000016, 0x12000031, 0x120000A1),
            count=1,
        )
        + sheet("synthetic/main", block(100, 3, 0x12000010, 0x1200002A, 0x120000F0))
        + sheet("synthetic/main", block(100, 3, 0x12000013, 0x1200002D, 0x120000F3))
    )
    positive_b = document(
        sheet(
            "independent",
            block(300, 1, 0x13000009, 0x13000044, 0x130000E1),
            count=1,
            types="<param>str</param><param>s32</param><param>str</param>",
        )
        + sheet(
            "independent/missing",
            block(301, 1, 0x1300000C, 0x13000047, 0x130000E4),
            count=1,
            types="<param>str</param><param>s32</param><param>str</param>",
        )
    )
    positive_resources = [
        (0x12000001, "complete-document-a.bin", positive_a),
        (0x12000002, "complete-document-b.bin", positive_b),
        (
            0x12000003,
            "complete-unrelated-xml.bin",
            b'<root>text <ssd marker="decoy" /></root>',
        ),
        (
            0x12000010,
            "complete-main-a.bin",
            typed_row("alpha", 7, 1) + typed_row("beta", 9, 0),
        ),
        (
            0x1200002A,
            "complete-main-enable-a.bin",
            enable_records([(100, 1), (102, 1)]),
        ),
        (0x120000F0, "complete-main-offset-a.bin", u32_array([10, 10, 19])),
        (0x12000011, "complete-adjacent-decoy.bin", b"decoy"),
        (
            0x12000013,
            "complete-main-b.bin",
            typed_row("alpha", 7, 1) + typed_row("BETA", 10, 0),
        ),
        (
            0x1200002D,
            "complete-main-enable-b.bin",
            enable_records([(100, 1), (102, 1)]),
        ),
        (0x120000F3, "complete-main-offset-b.bin", u32_array([10, 10, 19])),
        (0x12000016, "complete-other.bin", typed_row("solo", 1, 1)),
        (0x12000031, "complete-other-enable.bin", enable_records([(200, 1)])),
        (0x120000A1, "complete-other-offset.bin", u32_array([9])),
        (0x13000009, "complete-independent.bin", string_row("independent", 5, "final")),
        (0x13000044, "complete-independent-enable.bin", enable_records([(300, 1)])),
        (0x130000E1, "complete-independent-offset.bin", u32_array([26])),
        (0x1300000C, "complete-missing.bin", string_row("missing", 6)),
        (0x13000047, "complete-missing-enable.bin", enable_records([(301, 1)])),
        (0x130000E4, "complete-missing-offset.bin", u32_array([14])),
    ]
    add_tree("complete", positive_resources)

    def one_sheet_document(
        block_text: str,
        *,
        column_max: int = 3,
        column_count: int = 3,
        types: str = "<param>str</param><param>u8</param><param>bool</param>",
        indexes: str = "<param>0</param><param>1</param><param>2</param>",
    ) -> bytes:
        return xml_document(
            '<ssd version="0.1"><sheet name="synthetic/bad" mode="client" '
            'column_max="{0}" column_count="{1}" cache="0" type="all" infofile="">'
            "<type>{2}</type><index>{3}</index>{4}</sheet></ssd>\n".format(
                column_max, column_count, types, indexes, block_text
            )
        )

    def bad_tree(
        name: str, document_bytes: bytes, extras: list[tuple[int, str, bytes]]
    ) -> None:
        add_tree(name, [(0x14000001, name + "-document.bin", document_bytes)] + extras)

    triple = block(400, 1, 0x14000010, 0x14000020, 0x14000030)
    bad_tree(
        "incomplete-triple",
        one_sheet_document(triple),
        [
            (0x14000010, "incomplete-data.bin", typed_row("partial", 1, 1)),
            (0x14000030, "incomplete-offset.bin", u32_array([9])),
        ],
    )
    bad_tree(
        "offset-count",
        one_sheet_document(block(400, 1, 0x14000010, 0x14000020, 0x14000030)),
        [
            (0x14000010, "count-data.bin", typed_row("count", 1, 1)),
            (0x14000020, "count-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "count-offset.bin", u32_array([9, 9])),
        ],
    )
    bad_tree(
        "offset-length",
        one_sheet_document(triple),
        [
            (0x14000010, "length-data.bin", typed_row("length", 1, 1)),
            (0x14000020, "length-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "length-offset.bin", u32_array([8])),
        ],
    )
    bad_tree(
        "enable-rows",
        one_sheet_document(block(400, 2, 0x14000010, 0x14000020, 0x14000030)),
        [
            (0x14000010, "rows-data.bin", typed_row("rows", 1, 1)),
            (0x14000020, "rows-enable.bin", enable_records([(401, 1)])),
            (0x14000030, "rows-offset.bin", u32_array([9, 9])),
        ],
    )
    bad_tree(
        "enable-anomalies",
        one_sheet_document(triple),
        [
            (0x14000010, "anomaly-data.bin", typed_row("anomaly", 1, 1)),
            (0x14000020, "anomaly-enable.bin", enable_records([(400, 0)])),
            (0x14000030, "anomaly-offset.bin", u32_array([9])),
        ],
    )
    bad_tree(
        "offset-anomalies",
        one_sheet_document(triple),
        [
            (0x14000010, "backward-data.bin", typed_row("backward", 1, 1)),
            (0x14000020, "backward-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "backward-offset.bin", u32_array([9, 8])),
        ],
    )
    bad_tree(
        "incomplete-offset",
        one_sheet_document(triple),
        [
            (0x14000010, "incomplete-offset-data.bin", typed_row("partial", 1, 1)),
            (0x14000020, "incomplete-offset-enable.bin", enable_records([(400, 1)])),
        ],
    )
    bad_tree(
        "incomplete-data",
        one_sheet_document(triple),
        [
            (0x14000020, "incomplete-data-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "incomplete-data-offset.bin", u32_array([9])),
        ],
    )
    bad_tree(
        "range-outside",
        one_sheet_document(triple),
        [
            (0x14000010, "range-data.bin", typed_row("range", 1, 1)),
            (0x14000020, "range-enable.bin", enable_records([(0, 0xFFFFFFFF)])),
            (0x14000030, "range-offset.bin", u32_array([10])),
        ],
    )
    bad_tree(
        "column-count",
        one_sheet_document(triple, column_count=2),
        [
            (0x14000010, "columns-data.bin", typed_row("columns", 1, 1)),
            (0x14000020, "columns-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "columns-offset.bin", u32_array([10])),
        ],
    )
    bad_tree(
        "column-index",
        one_sheet_document(
            triple,
            column_max=2,
            indexes="<param>0</param><param>1</param><param>2</param>",
        ),
        [
            (0x14000010, "index-data.bin", typed_row("index", 1, 1)),
            (0x14000020, "index-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "index-offset.bin", u32_array([9])),
        ],
    )
    bad_tree(
        "column-width",
        one_sheet_document(triple, column_max=4294967295),
        [
            (0x14000010, "width-data.bin", typed_row("width", 1, 1)),
            (0x14000020, "width-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "width-offset.bin", u32_array([9])),
        ],
    )
    bad_tree(
        "multiple-missing",
        one_sheet_document(triple),
        [
            (0x14000010, "missing-data.bin", sheet_string("only", scrambled=False)),
            (0x14000020, "missing-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "missing-offset.bin", u32_array([7])),
        ],
    )
    bad_tree(
        "unknown-type",
        one_sheet_document(
            triple,
            types="<param>mystery</param><param>u8</param><param>bool</param>",
        ),
        [
            (0x14000010, "type-data.bin", typed_row("type", 1, 1)),
            (0x14000020, "type-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "type-offset.bin", u32_array([9])),
        ],
    )
    bad_tree(
        "index-count",
        one_sheet_document(
            triple,
            types="<param>str</param><param>u8</param><param>bool</param>",
            indexes="<param>0</param><param>1</param>",
        ),
        [
            (0x14000010, "index-count-data.bin", typed_row("index", 1, 1)),
            (0x14000020, "index-count-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "index-count-offset.bin", u32_array([9])),
        ],
    )
    partial_string = (
        sheet_string("partial", scrambled=False)
        + struct.pack("<i", 3)
        + struct.pack("<H", 8)
        + b"bad"
    )
    bad_tree(
        "partial-final-string",
        one_sheet_document(block(400, 1, 0x14000010, 0x14000020, 0x14000030)).replace(
            b"<param>str</param><param>u8</param><param>bool</param>",
            b"<param>str</param><param>s32</param><param>str</param>",
        ),
        [
            (0x14000010, "partial-string-data.bin", partial_string),
            (0x14000020, "partial-string-enable.bin", enable_records([(400, 1)])),
            (0x14000030, "partial-string-offset.bin", u32_array([len(partial_string)])),
        ],
    )
    mixed_document = xml_document(
        '<ssd version="0.1"><sheet name="master" infofile="123" />'
        + one_sheet_document(triple)
        .decode("utf-8")
        .split('<ssd version="0.1">')[1]
        .rsplit("</ssd>", 1)[0]
        + "</ssd>\n"
    )
    bad_tree("mixed-document", mixed_document, [])
    bad_tree("malformed-xml", b'<ssd version="0.1"><sheet>', [])

    # -- enable files ----------------------------------------------------
    fixtures["sheet/enable-ranges.bin"] = enable_records(
        [(10000, 7), (11000, 39), (12000, 2)]
    )

    # Twelve bytes: one whole record and half of another. Expect
    # trailing-partial-record at offset 8.
    fixtures["sheet/enable-partial-record.bin"] = enable_records(
        [(10000, 7)]
    ) + struct.pack("<I", 11000)

    # A range naming no rows, then one starting inside its predecessor.
    # Both are reported as anomalies. Neither stops the parse.
    fixtures["sheet/enable-anomalies.bin"] = enable_records([(20, 0), (10, 5), (12, 5)])

    # -- row-offset arrays -----------------------------------------------
    # Entry i is the end offset of row i, so a repeat is an empty slot.
    # Rows 0, 1, and 4 carry data. Rows 2 and 3 do not.
    fixtures["sheet/row-offsets.bin"] = u32_array([37, 74, 74, 74, 111])

    # Seven bytes. Expect trailing-partial-record at offset 4.
    fixtures["sheet/row-offsets-partial.bin"] = u32_array([37]) + b"\x00\x00\x00"

    # An entry that goes backwards, which cannot be an end offset. Expect a
    # row-offset-out-of-order anomaly and the entry preserved verbatim.
    fixtures["sheet/row-offsets-backward.bin"] = u32_array([40, 20, 60])

    # -- sheet strings ---------------------------------------------------
    scrambled_strings = (
        sheet_string("public fixture row one")
        + sheet_string(MULTIBYTE)
        + sheet_string("")
    )
    fixtures["sheet/strings-scrambled.bin"] = scrambled_strings
    fixtures["sheet/strings-plain.bin"] = sheet_string(
        "public fixture row one", scrambled=False
    ) + sheet_string(MULTIBYTE, scrambled=False)

    # A scrambled body whose last byte is not the key. Expect
    # malformed-sheet-string.
    broken = bytearray(sheet_string("public fixture row one"))
    broken[-1] = 0x41
    fixtures["sheet/string-bad-terminator.bin"] = bytes(broken)

    # A length prefix that runs past the end of the input. Expect
    # unexpected-end-of-input.
    fixtures["sheet/string-truncated.bin"] = (
        struct.pack("<H", 0x40) + bytes([SCRAMBLE_MARKER]) + pattern(6, 0x13)
    )

    # A truncated multi-byte sequence inside a decoded body. Expect
    # invalid-utf8.
    fixtures["sheet/string-not-utf8.bin"] = framed_string(b"ok\xc3\x28")

    # -- rich strings ----------------------------------------------------
    # Every length encoding, a payload that carries its own 0x02 .. 0x03
    # frame, and a token whose code this project does not interpret.
    rich = (
        "start".encode("utf-8")
        + token(0x10, b"")
        + "middle".encode("utf-8")
        + token(0x13, b"\xfe\xff\xf3\xf3\xf3")
        + token(0x09, b"payload", encoding="byte")
        + token(0x28, token(0x08, b"\x01"), encoding="direct")
        + token(0x1A, pattern(300, 0x07), encoding="word")
        + token(0x2C, pattern(256, 0x09), encoding="byte-scaled")
        + MULTIBYTE.encode("utf-8")
    )
    fixtures["sheet/rich-tokens.bin"] = framed_string(rich)

    # A token whose payload runs past the end of the string. Expect
    # malformed-rich-string-token.
    fixtures["sheet/rich-unterminated-token.bin"] = framed_string(
        b"start" + TOKEN_START + b"\x10\x40" + b"short"
    )

    # A length lead byte in the escape range that this project has not
    # established. Expect malformed-rich-string-token rather than a guess.
    fixtures["sheet/rich-unknown-length-lead.bin"] = framed_string(
        b"start" + TOKEN_START + b"\x10\xf3\x00\x00\x00" + TOKEN_END
    )

    # -- direct rich-string conformance fixtures ------------------------
    # These files are decoded string bytes, without the SSD string framing.
    # Their payloads are authored here independently of the Rust expression
    # reader so the conformance operation tests that reader rather than the
    # sheet stream.
    rich_codes = [
        0x07,
        0x08,
        0x09,
        0x10,
        0x11,
        0x12,
        0x13,
        0x14,
        0x16,
        0x19,
        0x1A,
        0x1D,
        0x1F,
        0x20,
        0x22,
        0x24,
        0x25,
        0x28,
        0x29,
        0x2B,
        0x2C,
        0x2D,
        0x2F,
        0x31,
        0x32,
        0x33,
    ]
    fixtures["rich-string/public-vocabulary.bin"] = (
        b"before" + b"".join(token(code, b"\x01") for code in rich_codes) + b"after"
    )

    # Keep the one synthetic F1 observation compact while exercising all four
    # length encodings. The other payloads are deliberately small.
    fixtures["rich-string/public-lengths.bin"] = b"".join(
        [
            token(0x07, b"\x01"),
            token(0x08, b"\x01\x02\x03", encoding="byte"),
            token(0x09, b"\x01" * 256, encoding="byte-scaled"),
            token(0x10, b"\x01" * 300, encoding="word"),
        ]
    )

    # Every established expression lead appears in a separate small token
    # frame. The bytes are authored independently of the Rust expression
    # reader, and direct lengths keep this fixture compact.
    expression_tokens = [token(0x08, bytes([lead])) for lead in range(0x01, 0xD0)]
    for lead in range(0xF0, 0xFF):
        mask = (lead + 1) & 0x0F
        lanes = [8, 4, 2, 1]
        data = bytes(0x20 + index for index, bit in enumerate(lanes) if mask & bit)
        expression_tokens.append(token(0x08, bytes([lead]) + data))
    expression_tokens.extend(
        token(0x08, bytes([lead])) for lead in list(range(0xD0, 0xE0)) + [0xEC]
    )
    expression_tokens.extend(
        token(0x08, bytes([lead, 0xD0])) for lead in range(0xE8, 0xEC)
    )
    expression_tokens.extend(
        token(0x08, bytes([lead, 0xD0, 0xD1])) for lead in range(0xE0, 0xE6)
    )
    expression_tokens.extend(
        [
            token(0x08, b"\xff\x01"),
            token(0x08, b"\xff\x04A\\["),
            token(0x08, b"\xff\xf0\x03A\\["),
            token(0x08, b"\xff\x05\x02\x10\x01\x03"),
        ]
    )
    fixtures["rich-string/public-expressions.bin"] = b"".join(expression_tokens)

    # A plain sheet string with the literal CSV body used by the independent
    # rich-string export oracle, including every named token and one unknown.
    csv_body = bytearray(b'q,"\\[x]\n\r')
    csv_body.extend(b"".join(token(code, b"\x01") for code in rich_codes))
    csv_body.extend(token(0xAA, b"\x00"))
    csv_body.extend("\u00e9".encode("utf-8"))
    fixtures["rich-string/public-csv.bin"] = framed_string(
        bytes(csv_body), scrambled=False
    )

    # Each malformed expression remains inside a valid token frame, so the
    # conformance report can retain the payload-relative failure offset.
    fixtures["rich-string/public-malformed-operands.bin"] = (
        token(0x08, b"\x01\xe8") + token(0x09, b"\xe0\x01") + token(0x0A, b"\xe0")
    )
    fixtures["rich-string/public-malformed-integers.bin"] = (
        token(0x08, b"\xf0\x00")
        + token(0x09, b"\xf1\x00")
        + token(0x0A, b"\xf2\x00\x01")
        + token(0x0B, b"\xf0")
    )
    fixtures["rich-string/public-malformed-strings.bin"] = (
        token(0x08, b"\xff\x02")
        + token(0x09, b"\xff\x03\xc3\x28")
        + token(0x0A, b"\xff\x02\x02")
        + token(0x0B, b"\xff\xf0")
    )
    fixtures["rich-string/public-reserved-leads.bin"] = b"".join(
        token(code, bytes([code])) for code in [0x00, 0xE6, 0xE7, 0xED, 0xEE, 0xEF]
    )
    fixtures["rich-string/public-unknown-raw.bin"] = (
        b"literal" + token(0xAA, b"\xe7\x02\x03") + b"tail"
    )

    for depth in [64, 65]:
        fixtures[f"rich-string/public-depth-{depth}.bin"] = token(
            0x08, b"\xe8" * depth + b"\x01"
        )

    # Direct framing failures exercise the offset reported by RichString::parse.
    fixtures["rich-string/public-frame-code.bin"] = b"\x02"
    fixtures["rich-string/public-frame-length.bin"] = b"\x02\x10"
    fixtures["rich-string/public-frame-lead.bin"] = b"\x02\x10\xf3\x00\x00\x00\x03"
    fixtures["rich-string/public-frame-payload.bin"] = b"\x02\x10\x04A\x03"
    fixtures["rich-string/public-frame-close.bin"] = b"\x02\x10\x02A\x04"
    fixtures["rich-string/public-frame-zero.bin"] = b"\x02\x10\x00\x03"

    # Literal fixed-control shapes from the authenticated codec, with invented values.
    fixtures["rich-string/public-fixed.bin"] = bytes.fromhex(
        "02 2e 03 01 01 03 "
        "02 2e 04 02 f0 cf 03 "
        "02 2e 04 02 f1 01 03 "
        "02 2e 09 fe 12 34 56 78 f4 12 34 03"
    )
    fixtures["rich-string/public-fixed-malformed.bin"] = bytes.fromhex(
        "02 2e 03 01 d8 03 02 2e 04 01 01 01 03 02 2e 03 01 f1 03"
    )

    # -- typed rows ------------------------------------------------------
    # Two rows of str,s32,bool,float,u8. A string column is
    # self-delimiting and the rest are fixed width, so the column list
    # alone fixes where a row ends.
    def typed_row(
        text: str, scrambled: bool, signed: int, flag: int, real: float, small: int
    ) -> bytes:
        return (
            sheet_string(text, scrambled=scrambled)
            + struct.pack("<i", signed)
            + bytes([flag])
            + struct.pack("<f", real)
            + bytes([small])
        )

    first_row = typed_row("public fixture row one", True, -2, 1, 1.5, 9)
    second_row = typed_row(MULTIBYTE, False, 7, 0, 0.0, 255)
    fixtures["sheet/rows-typed.bin"] = first_row + second_row

    # The same shape with the final row cut short. Expect
    # unexpected-end-of-input rather than a silently dropped row.
    fixtures["sheet/rows-short.bin"] = first_row + second_row[:-3]

    # The five further column types the scrambled documents declare, whose
    # widths the row pass over the install established: one byte for s8,
    # two for s16, u16, and f16, four for u32.
    def widened_row(text: str, tiny: int, word: int, half: int, big: int) -> bytes:
        return (
            sheet_string(text, scrambled=False)
            + struct.pack("<b", tiny)
            + struct.pack("<h", word)
            + struct.pack("<H", half)
            + struct.pack("<I", big)
        )

    fixtures["sheet/rows-widened.bin"] = widened_row(
        "widened row one", -8, -300, 0x3C00, 4000000000
    ) + widened_row(MULTIBYTE, 127, 32767, 0xFFFF, 0)

    # -- the scrambled XML container -------------------------------------
    # Both document kinds, so a decoded document is proved to reach the
    # same reader a plaintext one reaches.
    fixtures["scrambled/master.bin"] = scramble_document(fixtures["ssd/master.bin"])

    scrambled_schema = xml_document(
        '<ssd version="0.1">\n'
        '\t<sheet name="synthetic/widened" mode="client" column_max="10" column_count="10"'
        ' cache="-1" type="none" lang="chs" infofile="">\n'
        "\t\t<type>\n"
        "\t\t\t<param>str</param>\n"
        "\t\t\t<param>u8</param>\n"
        "\t\t\t<param>s8</param>\n"
        "\t\t\t<param>bool</param>\n"
        "\t\t\t<param>u16</param>\n"
        "\t\t\t<param>s16</param>\n"
        "\t\t\t<param>f16</param>\n"
        "\t\t\t<param>u32</param>\n"
        "\t\t\t<param>s32</param>\n"
        "\t\t\t<param>float</param>\n"
        "\t\t</type>\n"
        "\t\t<index><param>0</param></index>\n"
        '\t\t<block count="1">\n'
        '\t\t\t<file begin="10000" count="2" offset="664076320" enable="664076319">664076318</file>\n'
        "\t\t</block>\n"
        "\t</sheet>\n"
        "</ssd>\n"
    )
    fixtures["scrambled/sheet-info.bin"] = scramble_document(scrambled_schema)

    # One fixture per encoded length modulo four. The final byte takes the
    # extra correction in exactly one of the four, so a decoder that gets
    # the rule wrong fails one of these and passes the rest.
    for residue in range(4):
        body = '<ssd version="0.1">{0}</ssd>\n'.format(" " * residue)
        document = xml_document(body)
        padding = (residue - len(document)) % 4
        document = xml_document(body.replace("</ssd>", " " * padding + "</ssd>"))
        assert len(document) % 4 == residue
        fixtures["scrambled/residue-{0}.bin".format(residue)] = scramble_document(
            document
        )
        # The same document under the derived second key, which the encoder
        # reproduces, so validate's round trip passes.
        fixtures["scrambled/derived-residue-{0}.bin".format(residue)] = (
            scramble_document(document, derived_key_b(len(document)))
        )

    # The trailer byte removed. Expect missing-scramble-trailer: without it
    # the client's own reader hands the file on untouched, so calling it a
    # document would be an invention.
    fixtures["scrambled/no-trailer.bin"] = fixtures["scrambled/master.bin"][:-1]

    # A trailer with too few bytes ahead of it to carry the second key.
    # Expect unexpected-end-of-input.
    fixtures["scrambled/short-body.bin"] = pattern(5, 0x11) + bytes([SCRAMBLE_TRAILER])

    # A file that ends in the trailer and is not a document. Expect
    # bad-magic: the trailer alone is not a signature.
    fixtures["scrambled/not-a-document.bin"] = pattern(48, 0x23) + bytes(
        [SCRAMBLE_TRAILER]
    )

    # A decode that opens correctly and then runs into a truncated
    # multi-byte sequence. Expect invalid-utf8.
    fixtures["scrambled/bad-utf8.bin"] = scramble_document(
        BOM
        + b'<?xml version="1.0" encoding="utf-8"?>\r\n<ssd version="0.1">\xc3\x28</ssd>\r\n'
    )

    # A decode that opens correctly and is not well formed. Expect
    # malformed-xml.
    fixtures["scrambled/malformed-xml.bin"] = scramble_document(
        xml_document('<ssd version="0.1">\n  <sheet name="unclosed">\n')
    )

    # -- the SQEX container ----------------------------------------------
    # A widget document with nesting, repeated element names, and a body
    # that does not divide evenly, so the final run stays in the clear.
    window = widget_document(
        '<SyntheticWindow Name="first" Width="320">\n'
        '\t<SyntheticPanel Name="body" Margin="4">\n'
        '\t\t<SyntheticLabel Name="title">public fixture</SyntheticLabel>\n'
        '\t\t<SyntheticLabel Name="detail" IsVisible="true" />\n'
        "\t</SyntheticPanel>\n"
        "</SyntheticWindow>\n"
    )
    fixtures["sqwt/window.bin"] = sqwt_container("sqwt/window.bin", window)

    # The two constructs the widget profile adds to the SSD subset, and
    # nothing else: a comment, and ampersand runs in text and in an
    # attribute value. Both are carried verbatim rather than expanded.
    dictionary = widget_document(
        '<SyntheticDictionary Version="1">\n'
        "\t<!-- an authored note, counted and not kept -->\n"
        '\t<SyntheticEntry Key="a" Label="one &amp; two">three &amp; four &</SyntheticEntry>\n'
        "</SyntheticDictionary>\n"
    )
    fixtures["sqwt/dictionary.bin"] = sqwt_container("sqwt/dictionary.bin", dictionary)

    # A body that divides evenly, so there is no plaintext tail at all. 155
    # retail containers take this shape and a reader that assumes a tail
    # would mishandle every one of them.
    aligned_body = '<SyntheticRoot Name="aligned">{0}</SyntheticRoot>\n'
    padding = -len(widget_document(aligned_body.format(""))) % 8
    fixtures["sqwt/aligned.bin"] = sqwt_container(
        "sqwt/aligned.bin", widget_document(aligned_body.format(" " * padding))
    )
    assert (len(fixtures["sqwt/aligned.bin"]) - 8) % 8 == 0

    # A document opening on a byte order mark, which 11 retail containers
    # do and the rest do not.
    fixtures["sqwt/byte-order-mark.bin"] = sqwt_container(
        "sqwt/byte-order-mark.bin",
        widget_document('<SyntheticRoot Name="marked" />\n', bom=True),
    )

    # A foreign signature. Expect bad-magic at offset 0.
    fixtures["sqwt/bad-magic.bin"] = b"GTEX" + bytes(4) + pattern(0x20, 0x71)

    # The tag with a reserved word that is not zero. Every one of the 1155
    # retail containers sets it to zero, so this is not one of them. Expect
    # bad-magic at offset 4.
    fixtures["sqwt/reserved-not-zero.bin"] = (
        b"SQEX" + struct.pack("<I", 1) + pattern(0x20, 0x73)
    )

    # Half a signature. Expect unexpected-end-of-input at offset 4.
    fixtures["sqwt/truncated-header.bin"] = b"SQEX"

    # A container enciphered under a name that is not the one it is stored
    # under, which is what reading a renamed file amounts to. The key is
    # wrong, so the body is not text: expect invalid-utf8.
    fixtures["sqwt/wrong-name.bin"] = SQWT_SIGNATURE + Blowfish(b"other.bin").encrypt(
        window
    )

    # A body that deciphers cleanly and is not a document. Expect
    # malformed-xml rather than a reading invented for it.
    fixtures["sqwt/not-a-document.bin"] = sqwt_container(
        "sqwt/not-a-document.bin", b"plain text, no element anywhere in it\r\n"
    )

    # A CDATA section: inside the widget profile's reach and outside its
    # grammar, so the profile stays bounded. Expect
    # unsupported-xml-construct.
    fixtures["sqwt/cdata.bin"] = sqwt_container(
        "sqwt/cdata.bin",
        widget_document("<SyntheticRoot><![CDATA[x]]></SyntheticRoot>\n"),
    )

    # A comment that never closes. Expect malformed-xml.
    fixtures["sqwt/unclosed-comment.bin"] = sqwt_container(
        "sqwt/unclosed-comment.bin",
        widget_document("<SyntheticRoot>\n\t<!-- never closed\n</SyntheticRoot>\n"),
    )

    # -- RegionResourceData 1.1.0 ----------------------------------------
    child = region_row(17, 0x10203040, 0xFFFFFFFF, b"SyntheticChild")
    region = region_table(
        [
            region_row(3, 0x11223344, 2, b"SyntheticRoot"),
            child,
            child,
            region_row(5, 0x55667788, 0, b"OtherRoot"),
        ],
        2,
    )
    fixtures["region/rows.bin"] = region
    fixtures["region/empty.bin"] = region_table([], 0)
    fixtures["region/bad-name.bin"] = b"X" + region[1:]
    fixtures["region/bad-version.bin"] = region[:24] + b"2" + region[25:]
    for name, length in [
        ("name", 18),
        ("name-padding", 23),
        ("version", 29),
        ("version-padding", 31),
        ("size", 35),
        ("count", 39),
        ("tail", 63),
    ]:
        fixtures[f"region/truncated-{name}.bin"] = region[:length]
    fixtures["region/root-count-bomb.bin"] = region_table([], 0xFFFFFFFF)
    fixtures["region/missing-root.bin"] = region_table([], 1)
    fixtures["region/partial-root.bin"] = region_table([child], 1)[:-1]
    fixtures["region/child-count-bomb.bin"] = region_table(
        [region_row(3, 0, 0xFFFFFFFF, b"Root")], 1
    )
    fixtures["region/partial-child.bin"] = region_table(
        [region_row(3, 0, 1, b"Root"), child], 1
    )[:-1]
    fixtures["region/children-consume-roots.bin"] = region_table(
        [region_row(3, 0, 1, b"Root"), child], 2
    )
    fixtures["region/trailing.bin"] = region + b"XYZ"
    advisory = bytearray(region)
    struct.pack_into("<I", advisory, 0x20, 1)
    fixtures["region/advisory-size.bin"] = bytes(advisory)
    fixtures["region/unterminated-root.bin"] = region_table(
        [region_row(7, 0, 0, b"abcdefghijklmnop", word_20=0)], 1
    )
    fixtures["region/nonzero-layout-argument.bin"] = region_table(
        [
            region_row(7, 0, 1, b"Root", word_20=5),
            region_row(70, 0x12340005, 9, b"Child", word_04=0x3000),
        ],
        1,
    )
    fixtures["region/auxiliary-overflow.bin"] = region_table(
        [
            region_row(7, 0, 1, b"Root", word_20=0),
            region_row(70, 0x1234FFFF, 9, b"Child", word_04=0x3000),
        ],
        1,
    )
    fixtures["region/auxiliary-all-types.bin"] = region_table(
        [
            region_row(7, 0, 3, b"RootZeroMax", word_20=0),
            region_row(70, 0x12340002, 9, b"LayoutA", word_04=0x3000),
            region_row(71, 0x56780007, 10, b"TypeZeroA", word_04=0),
            region_row(72, 0x9ABC0005, 11, b"TypeOneA", word_04=1),
            region_row(8, 0, 3, b"RootOneMax", word_20=0),
            region_row(80, 0xABCD0001, 12, b"LayoutB", word_04=0x3000),
            region_row(81, 0x56780003, 13, b"TypeZeroB", word_04=0),
            region_row(82, 0x9ABC0009, 14, b"TypeOneB", word_04=1),
        ],
        2,
    )

    # -- bitmap font container (fdt) -------------------------------------
    # Chunk order, table shapes, and record layouts from docs/formats/fdt.md.
    # Every payload is this project's own: two mapped pages, three keys, and
    # three records, one of which no key reaches. A texture payload carries
    # only the GTEX signature and dimensions the record checks read.
    def fdt_chunk(name: str, payload: bytes) -> bytes:
        return name.encode("ascii") + struct.pack("<I", len(payload)) + payload

    def fdt_gtex(width: int, height: int, length: int, seed: int) -> bytes:
        return (
            b"GTEX"
            + bytes(6)
            + struct.pack(">HH", width, height)
            + pattern(length - 14, seed)
        )

    def fdt_record(
        resource: int, selector: int, extent: tuple[int, int], source: tuple[int, int]
    ) -> bytes:
        # Placement 1, -1, advance 6, byte 7 zero, flag-0x02 addend 1, byte 9 zero.
        return struct.pack(
            "<bbbbbbbBBBHH", resource, selector, 1, -1, *extent, 6, 0, 1, 0, *source
        )

    def fdt_unversioned_record(
        resource: int, selector: int, extent: tuple[int, int], source: tuple[int, int]
    ) -> bytes:
        return (
            struct.pack(
                "<bbbbbbbBHH", resource, selector, 1, -1, *extent, 6, 0, *source
            )
            + b"\xa5\x5a"
        )

    fdt_textures = [
        ("DDSF", pattern(12, 0x21)),
        ("GTEX", fdt_gtex(32, 16, 20, 0x22)),
        ("GTEX", fdt_gtex(16, 8, 16, 0x23)),
    ]
    # Resource 1 is the 32x16 GTEX; resource 0 is DDSF, which has no dimensions.
    fdt_records = (
        fdt_record(1, 2, (5, 6), (4, 2))
        + fdt_record(0, 0, (3, 3), (0, 0))
        + fdt_record(1, 3, (7, 9), (20, 7))
    )

    def fdt_chunks(
        version: int | None,
        textures: list[tuple[str, bytes]] | None = None,
        records: bytes = fdt_records,
    ) -> list[tuple[str, bytes]]:
        if textures is None:
            textures = fdt_textures
        pages = bytearray([0xFF] * 256)
        pages[0x00] = 1
        pages[0x30] = 0
        index = bytearray([0xFF] * 1024)
        struct.pack_into("<H", index, 512 + 2 * 0x41, 0)
        struct.pack_into("<H", index, 512 + 2 * 0x42, 2)
        struct.pack_into("<H", index, 0, 2)
        chunks = []
        if version is not None:
            chunks.append(("VERS", struct.pack("<I", version)))
        chunks += textures
        gate = version or 0
        if gate >= 4:
            chunks.append(("TEXC", struct.pack("<I", 1)))
        chunks.append(("FSIZ", struct.pack("<I", 0x18)))
        if gate >= 3:
            chunks.append(("FSMX", struct.pack("<I", 0x20)))
        chunks.append(("BSLN", struct.pack("<I", 0x14)))
        if gate >= 2:
            chunks.append(("HMGN", struct.pack("<I", 0)))
        chunks += [
            ("HTBL", bytes(pages)),
            ("LTBL", bytes(index)),
            ("GLYP", records),
        ]
        return chunks

    def fdt_font(chunks: list[tuple[str, bytes]]) -> bytes:
        return b"".join(fdt_chunk(name, payload) for name, payload in chunks)

    def fdt_replace(
        chunks: list[tuple[str, bytes]], name: str, payload: bytes | None
    ) -> list[tuple[str, bytes]]:
        replaced = []
        for chunk_name, chunk_payload in chunks:
            if chunk_name != name:
                replaced.append((chunk_name, chunk_payload))
            elif payload is not None:
                replaced.append((chunk_name, payload))
        return replaced

    font = fdt_chunks(4) + [("TERM", b"")]
    fixtures["fdt/v4.bin"] = fdt_font(font)
    single_texture = [("GTEX", fdt_gtex(32, 16, 16, 0x24))]
    fixtures["fdt/v1.bin"] = fdt_font(
        fdt_chunks(
            1,
            single_texture,
            fdt_record(0, 2, (5, 6), (4, 2))
            + fdt_record(0, 0, (3, 3), (0, 0))
            + fdt_record(0, 3, (7, 9), (20, 7)),
        )
    )
    # A zero version selects the unversioned record layout.
    fixtures["fdt/no-version.bin"] = fdt_font(
        fdt_chunks(
            None,
            single_texture,
            fdt_unversioned_record(0, 1, (5, 6), (4, 2))
            + fdt_unversioned_record(0, 0, (3, 3), (0, 0))
            + fdt_unversioned_record(0, 3, (7, 9), (20, 7)),
        )
    )
    # A GTEX payload the GTEX reader accepts beside one it refuses as truncated.
    fixtures["fdt/gtex-payloads.bin"] = fdt_font(
        fdt_chunks(
            4,
            [
                ("DDSF", pattern(12, 0x27)),
                (
                    "GTEX",
                    gtex(
                        pattern(40, 0x44),
                        mip_levels=2,
                        width=4,
                        height=2,
                        data_base=0x28,
                        surfaces=((0, 32), (32, 8)),
                    ),
                ),
                ("GTEX", fdt_gtex(16, 8, 16, 0x28)),
            ],
            fdt_record(1, 0, (2, 1), (1, 1))
            + fdt_record(0, 1, (1, 1), (0, 0))
            + fdt_record(1, 2, (4, 2), (0, 0)),
        )
    )
    # Two A4R4G4B4 atlases, the second half the size of the first, as TEXC 1
    # pairs them: 4 by 2 with mips of 16 and 4 bytes, and 2 by 1 with 4 and 2.
    fixtures["fdt/atlases.bin"] = fdt_font(
        fdt_chunks(
            4,
            [
                (
                    "GTEX",
                    gtex(
                        pattern(20, 0x29),
                        format_index=22,
                        mip_levels=2,
                        width=4,
                        height=2,
                        data_base=0x28,
                        surfaces=((0, 16), (16, 4)),
                    ),
                ),
                (
                    "GTEX",
                    gtex(
                        pattern(6, 0x2A),
                        format_index=22,
                        mip_levels=2,
                        width=2,
                        height=1,
                        data_base=0x28,
                        surfaces=((0, 4), (4, 2)),
                    ),
                ),
            ],
            fdt_record(0, 0, (2, 1), (1, 1))
            + fdt_record(0, 1, (1, 1), (0, 0))
            + fdt_record(0, 2, (4, 2), (0, 0)),
        )
    )
    # A third LTBL page that no HTBL byte selects, holding record 1 at low
    # byte 0x20, so its entries are reachable by no key.
    unreachable_page = bytearray([0xFF] * 512)
    struct.pack_into("<H", unreachable_page, 2 * 0x20, 1)
    fixtures["fdt/unreachable-page.bin"] = fdt_font(
        fdt_replace(
            font,
            "LTBL",
            dict(font)["LTBL"] + bytes(unreachable_page),
        )
    )
    # One record per check: a negative resource index; a resource whose TEXC
    # offset leaves the list, with selector 4 and a source past the 16x8
    # texture; and selector -1 with a negative width.
    fixtures["fdt/glyph-checks.bin"] = fdt_font(
        fdt_chunks(
            4,
            records=fdt_record(-1, 0, (3, 3), (0, 0))
            + fdt_record(2, 4, (4, 4), (14, 0))
            + fdt_record(1, -1, (-2, 3), (0, 0)),
        )
    )
    # Chunks after GLYP are listed; bytes too short for a header stay trailing.
    fixtures["fdt/following.bin"] = (
        fdt_font(font + [("XTRA", pattern(4, 0x61))]) + b"\x01\x02\x03"
    )
    fixtures["fdt/truncated-length.bin"] = fixtures["fdt/v4.bin"][:6]
    fixtures["fdt/missing-glyph-chunk.bin"] = fdt_font(
        fdt_replace(font, "GLYP", None)[:-1]
    )
    overrun = bytearray(fixtures["fdt/v4.bin"])
    struct.pack_into("<I", overrun, 4, 0x1000)
    fixtures["fdt/chunk-overrun.bin"] = bytes(overrun)
    fixtures["fdt/missing-texture-count.bin"] = fdt_font(
        fdt_replace(font, "TEXC", None)
    )
    fixtures["fdt/texture-order.bin"] = fdt_font(
        fdt_chunks(4, [("GTEX", pattern(16, 0x25)), ("DDSF", pattern(12, 0x26))])
    )
    fixtures["fdt/scalar-length.bin"] = fdt_font(fdt_replace(font, "FSIZ", b"\x18\x00"))
    fixtures["fdt/page-table-length.bin"] = fdt_font(
        fdt_replace(font, "HTBL", bytes([0xFF] * 255))
    )
    fixtures["fdt/index-partial-page.bin"] = fdt_font(
        fdt_replace(font, "LTBL", bytes([0xFF] * 1023))
    )
    fixtures["fdt/glyph-partial-record.bin"] = fdt_font(
        fdt_replace(font, "GLYP", pattern(3 * 14 + 1, 0x52))
    )
    pages_out = bytearray([0xFF] * 256)
    pages_out[0x7F] = 2
    fixtures["fdt/page-out-of-range.bin"] = fdt_font(
        fdt_replace(font, "HTBL", bytes(pages_out))
    )
    records_out = bytearray([0xFF] * 1024)
    struct.pack_into("<H", records_out, 10, 3)
    fixtures["fdt/record-out-of-range.bin"] = fdt_font(
        fdt_replace(font, "LTBL", bytes(records_out))
    )

    # -- the configuration files -----------------------------------------
    # The shapes come from docs/formats/configuration.md; every value is this
    # project's own. The stamps are deterministic synthetic words rather than
    # the client's constants, because the reader does not test the stamp and a
    # fixture should not read as a copy of one.
    fixtures["config/sys-grid.bin"] = (
        struct.pack("<I", 0x13579BDF)
        + struct.pack("<III", 1, 0, 0x02D0)
        + utf16_field("Synthetic Font", 32)
        + utf16_field("synthetic\\path", 32)
        + b"TAGS"
        + struct.pack("<III", 0, 0xDEADBEEF, 0)
    )
    assert len(fixtures["config/sys-grid.bin"]) % 4 == 0

    # The pad shape: a stamp, a sixteen-byte device identifier, a count, and
    # a byte mapping. Both of the census's false positives are here on
    # purpose - the identifier reads as eight printable UTF-16 units and the
    # mapping as twelve printable bytes, and neither is text.
    fixtures["config/pad-grid.bin"] = (
        struct.pack("<I", 0x2468ACE0)
        + b"\xa1\x41\xa2\x42\xa3\x43\xa4\x44\xa5\x45\xa6\x46"
        + b"TEST"
        + struct.pack("<I", 2)
        + bytes(range(0x30, 0x3C))
        + bytes(4 * 9)
    )
    assert len(fixtures["config/pad-grid.bin"]) % 4 == 0

    # Two words and no stamp, which is the whole of the language file.
    fixtures["config/lng-words.bin"] = struct.pack("<II", 0, 2)

    # A grid with no words at all. The stamped files refuse this and the
    # unstamped ones do not, which is the difference between them.
    fixtures["config/lng-empty.bin"] = b""

    # The region file is not a grid: its bytes are carried whole, so the
    # fixture uses a length no word boundary allows and content that is not
    # zero, which the retail file's five zero bytes would not exercise.
    fixtures["config/rgn-body.bin"] = bytes([1, 2, 3, 4, 5])

    # A grid ending in a run shorter than a word. Nothing states a length
    # and nothing terminates it, so the leftover is unaccountable. Expect
    # trailing-partial-record at the offset the leftover starts.
    fixtures["config/sys-partial-word.bin"] = (
        struct.pack("<II", 0x13579BDF, 7) + b"\x01\x02"
    )
    fixtures["config/lng-partial-word.bin"] = struct.pack("<I", 3) + b"\x09"

    # Less than the leading stamp. Expect unexpected-end-of-input.
    fixtures["config/sys-short-stamp.bin"] = b"\x01\x02\x03"
    fixtures["config/pad-short-stamp.bin"] = b"\x01\x02"

    # -- skeleton -------------------------------------------------------
    # Framing from docs/formats/skeleton.md; names and opaque bytes are invented.
    skeleton = bytearray(pattern(628, 0x53))
    skeleton[:8] = b"SEDBSKL "
    struct.pack_into("<IHHI", skeleton, 8, 7000, 0x0400, 0x30, len(skeleton))
    struct.pack_into("<III", skeleton, 0x30, 1, 528, 4)
    struct.pack_into("<II", skeleton, 0x44, 0, 528)
    for index, (name, parent, child) in enumerate(
        [(0, 0xFFFFFFFF, 1), (0, 0, 2), (2, 1, 0xFFFFFFFF)]
    ):
        at = 80 + index * 176
        struct.pack_into("<III", skeleton, at, name, 3, 3)
        struct.pack_into("<IIII", skeleton, at + 0x38, parent, child, 0xFFFFFFFF, index)
    skeleton[608:625] = b"oak\0limb\0tip\0aux\0"
    fixtures["skeleton/records.bin"] = bytes(skeleton)
    for name, offset, value in [
        ("scalar", 8, 7001),
        ("directory-count", 0x30, 2),
        ("directory-offset", 0x44, 1),
        ("partial-record", 0x48, 527),
        ("string-overlap", 0x34, 527),
        ("string-count", 0x38, 100),
        ("string-index", 80 + 4, 4),
        ("hierarchy-index", 80 + 0x38, 3),
        ("self-index", 80 + 0x44, 1),
        ("parent-self", 80 + 0x38, 0),
        ("parent-cycle", 80 + 0x38, 2),
    ]:
        malformed = bytearray(skeleton)
        struct.pack_into("<I", malformed, offset, value)
        fixtures[f"skeleton/{name}.bin"] = bytes(malformed)
    for name, offset, value in [
        ("bad-tag", 7, ord("!")),
        ("conversion", 12, 1),
        ("header-size", 14, 0x40),
    ]:
        malformed = bytearray(skeleton)
        malformed[offset] = value
        fixtures[f"skeleton/{name}.bin"] = bytes(malformed)
    fixtures["skeleton/truncated.bin"] = bytes(skeleton[:72])
    unterminated = bytearray(skeleton)
    unterminated[624:] = b"xxxx"
    fixtures["skeleton/unterminated.bin"] = bytes(unterminated)

    def skeleton_res(payload: bytes) -> bytes:
        return (
            sedb_header(
                "RES ", 4000, 0x0400, 0x40, 80 + len(payload), res_extended(1, 0, "syn")
            )
            + directory_entry(27, 0, len(payload), 0)
            + payload
        )

    nested_skeleton = skeleton_res(skeleton_res(bytes(skeleton)))
    fixtures["skeleton/nested-res.bin"] = nested_skeleton
    short_header = bytearray(nested_skeleton)
    struct.pack_into("<H", short_header, 14, 0x30)
    struct.pack_into("<H", short_header, 94, 0x30)
    fixtures["skeleton/res-header-0x30.bin"] = bytes(short_header)
    wrapped_skeleton = (
        b"PWIB"
        + struct.pack(">III", 16 + len(nested_skeleton), 16, 16 + len(nested_skeleton))
        + nested_skeleton
    )
    fixtures["skeleton/pwib.bin"] = wrapped_skeleton
    malformed = bytearray(wrapped_skeleton)
    struct.pack_into(">I", malformed, 4, len(wrapped_skeleton) + 16)
    fixtures["skeleton/pwib-boundary.bin"] = bytes(malformed)
    malformed = bytearray(wrapped_skeleton)
    struct.pack_into("<I", malformed, 260, 4)
    fixtures["skeleton/nested-string-index.bin"] = bytes(malformed)
    malformed = bytearray(nested_skeleton)
    struct.pack_into("<I", malformed, 16, len(nested_skeleton) - 16)
    fixtures["skeleton/container-boundary.bin"] = bytes(malformed)
    fixtures["skeleton/opaque-alias.bin"] = (
        sedb_header("RES ", 4000, 0x0400, 0x40, 756, res_extended(3, 0, "syn"))
        + directory_entry(91, 628, 16, 0)
        + directory_entry(27, 0, 628, 0)
        + directory_entry(92, 628, 16, 0)
        + bytes(skeleton)
        + pattern(16, 0x97)
    )
    aliased = bytearray(80)
    aliased[:8] = b"SEDBSKL "
    struct.pack_into("<IHHI", aliased, 8, 7000, 0x0400, 0x30, 80)
    struct.pack_into("<I", aliased, 0x30, 1)
    for _ in range(7):
        aliased = (
            sedb_header(
                "RES ",
                4000,
                0x0400,
                0x40,
                576 + len(aliased),
                res_extended(32, 0, "syn"),
            )
            + directory_entry(27, 0, len(aliased), 0) * 32
            + aliased
        )
    fixtures["skeleton/selected-alias.bin"] = bytes(aliased)
    fixtures.update(material_fixtures())
    fixtures.update(shader_fixtures())
    return fixtures


def material_fixtures() -> dict[str, bytes]:
    """Authored shd/SHD/PRAM sampler framing, without shader or retail content."""
    fixtures = {}

    def chunk(kind: bytes, body: bytes) -> bytes:
        return kind + bytes(4) + struct.pack(">II", 16 + len(body), 0) + body

    def material(body: bytes, include_pram: bool = True) -> bytes:
        children = chunk(b"FILE", pattern(16, 0x57))
        if include_pram:
            children += chunk(b"PRAM", body)
        info = struct.pack(">I", 2 if include_pram else 1) + pattern(12, 0x6D)
        payload = chunk(b"SHD\0", info + children)
        return sedb_header("shd", 29, 0x0200, 0x30, 0x30 + len(payload)) + payload

    body = bytearray(pattern(120, 0x39))
    struct.pack_into("<III", body, 0x14, 2, 32, 112)
    struct.pack_into("<II", body, 32, 40, 80)
    body[43], body[52], body[53] = 0xA0, 0, 0
    body[83], body[92], body[93] = 0x21, 0xFF, 0xE1
    sample = material(bytes(body))
    fixtures["material/samplers.bin"] = sample
    padded = bytearray(sample[:112] + pattern(16, 0x83) + sample[112:])
    struct.pack_into("<I", padded, 16, len(padded))
    struct.pack_into(">I", padded, 56, len(padded) - 48)
    struct.pack_into(">I", padded, 92, 48)
    fixtures["material/padded.bin"] = bytes(padded)
    empty = bytearray(32)
    struct.pack_into("<I", empty, 0x18, 32)
    fixtures["material/empty.bin"] = material(bytes(empty))
    fixtures["material/no-pram.bin"] = material(b"", False)
    fixtures["material/short-body.bin"] = material(bytes(28))
    fixtures["material/truncated.bin"] = sample[:18]
    for name, offset, value in [
        ("version", 8, 28),
        ("extent", 16, len(sample) + 1),
        ("sampler-count", 148, 0xFFFFFFFF),
        ("sampler-table", 152, 28),
        ("sampler-offset", 160, 100),
        ("sampler-alias", 164, 40),
        ("sampler-overlap", 164, 64),
        ("sampler-header", 160, 0),
        ("sampler-table-overlap", 160, 32),
    ]:
        malformed = bytearray(sample)
        struct.pack_into("<I", malformed, offset, value)
        fixtures[f"material/{name}.bin"] = bytes(malformed)
    for name, offset, value in [
        ("root-size", 56, 201),
        ("child-size", 88, 0xFFFFFFFF),
        ("child-padding", 92, 0xFFFFFFFF),
    ]:
        malformed = bytearray(sample)
        struct.pack_into(">I", malformed, offset, value)
        fixtures[f"material/{name}.bin"] = bytes(malformed)
    for name, offset, value in [
        ("bad-tag", 7, ord("!")),
        ("conversion", 12, 1),
        ("header-size", 14, 0x40),
        ("root-tag", 51, ord("!")),
    ]:
        malformed = bytearray(sample)
        malformed[offset] = value
        fixtures[f"material/{name}.bin"] = bytes(malformed)

    def res(payload: bytes) -> bytes:
        return (
            sedb_header(
                "RES ", 4000, 0x0400, 0x40, 80 + len(payload), res_extended(1, 0, "syn")
            )
            + directory_entry(7, 0, len(payload), 0)
            + payload
        )

    nested = res(res(sample))
    fixtures["material/nested-res.bin"] = nested
    fixtures["material/pwib.bin"] = (
        b"PWIB" + struct.pack(">III", 16 + len(nested), 16, 16 + len(nested)) + nested
    )
    malformed = bytearray(fixtures["material/pwib.bin"])
    struct.pack_into(">I", malformed, 4, len(malformed) + 1)
    fixtures["material/pwib-boundary.bin"] = bytes(malformed)
    malformed = bytearray(nested)
    struct.pack_into("<I", malformed, 16, len(nested) - 1)
    fixtures["material/container-boundary.bin"] = bytes(malformed)
    malformed = bytearray(nested)
    struct.pack_into("<I", malformed, 56, 2)
    fixtures["material/container-count.bin"] = bytes(malformed)
    aliased = sample
    for _ in range(7):
        aliased = (
            sedb_header(
                "RES ",
                4000,
                0x0400,
                0x40,
                576 + len(aliased),
                res_extended(32, 0, "syn"),
            )
            + directory_entry(7, 0, len(aliased), 0) * 32
            + aliased
        )
    fixtures["material/selected-alias.bin"] = aliased
    overlap = bytearray(aliased)
    struct.pack_into("<I", overlap, 88, len(aliased) - 577)
    fixtures["material/selected-overlap.bin"] = bytes(overlap)
    fixtures["material/opaque-alias.bin"] = (
        sedb_header(
            "RES ", 4000, 0x0400, 0x40, 128 + len(sample), res_extended(3, 0, "syn")
        )
        + directory_entry(91, len(sample), 16, 0)
        + directory_entry(7, 0, len(sample), 0)
        + directory_entry(92, len(sample), 16, 0)
        + sample
        + pattern(16, 0x97)
    )
    # -- animation bank and MTB framing ---------------------------------
    # Authored section views, offset tables and opaque bodies. See
    # docs/formats/animation.md; no curve or motion payload is decoded.
    animation = bytearray(sedb_header("mtb", 0x43, 0, 0x40, 0))
    struct.pack_into("<H", animation, 0x30, 3)
    animation += struct.pack("<3I", 2, 0x60, 0x80)
    animation += struct.pack("<3I", 2, 0xA0, 0xA7)
    animation += struct.pack("<2I", 1, 0xAD)
    animation += struct.pack("<I", 0) + pattern(28, 0x31)
    animation += struct.pack("<I", 1) + pattern(28, 0x41)
    animation += b"Header\0Curve\0group0\0"
    fixtures["animation/sections.bin"] = bytes(animation)

    animation_scb = sedb_header("SCB ", 2, 0, 0x30, 0x40) + pattern(16, 0x51)
    animation_root = (
        sedb_header(
            "RES ",
            4000,
            0x200,
            0x40,
            0x60 + len(animation) + len(animation_scb),
            res_extended(2, 0, "anim"),
        )
        + directory_entry(0, 0, len(animation), 0)
        + directory_entry(1, len(animation), len(animation_scb), 0)
        + animation
        + animation_scb
    )
    animation_second = pattern(16, 0x61)
    animation_bank = (
        b"PWIB"
        + struct.pack(
            ">III",
            16 + len(animation_root) + len(animation_second),
            16,
            16 + len(animation_root),
        )
        + animation_root
        + animation_second
    )
    fixtures["animation/bank.bin"] = animation_bank
    animation_opaque = bytearray(animation_bank)
    animation_opaque[296:300] = b"XTRA"
    struct.pack_into("<I", animation_opaque, 308, 0xFFFFFFFF)
    fixtures["animation/opaque-entry.bin"] = bytes(animation_opaque)
    for name, at, length in [
        ("opaque-overrun", len(animation), 0xFFFFFFFF),
        ("opaque-outside", 0xFFFFFFFF, 64),
    ]:
        opaque_bank = bytearray(animation_opaque)
        struct.pack_into("<II", opaque_bank, 100, at, length)
        fixtures[f"animation/{name}.bin"] = bytes(opaque_bank)
    fixtures["animation/scb.bin"] = animation_scb
    animation_control = (
        sedb_header("RES ", 4000, 0x200, 0x40, 0x90, res_extended(1, 0, "anim"))
        + directory_entry(0, 0, 0x40, 0)
        + b"CTRL"
        + pattern(60, 0x71)
    )
    fixtures["animation/control-only-bank.bin"] = (
        b"PWIB" + struct.pack(">III", 0xA0, 16, 0xA0) + animation_control
    )
    fixtures["animation/truncated-header.bin"] = bytes(animation[:12])
    for name, offset, value in [
        ("table-overrun", 0x40, 0xFFFFFFFF),
        ("name-overrun", 0x50, len(animation)),
        ("name-alias", 0x54, 0xA0),
        ("name-overlap", 0x54, 0xA1),
        ("group-name-alias", 0x5C, 0xA0),
        ("section-overrun", 0x44, len(animation) + 16),
        ("section-index", 0x60, 2),
        ("section-alias", 0x48, 0x60),
        ("section-unaligned", 0x44, 0x61),
        ("old-version", 8, 0x42),
    ]:
        malformed = bytearray(animation)
        struct.pack_into("<I", malformed, offset, value)
        fixtures[f"animation/{name}.bin"] = bytes(malformed)
    fixtures["animation/unterminated-name.bin"] = bytes(animation[:-1]) + b"X"
    fixtures["animation/missing-header.bin"] = (
        bytes(animation[:0xA0]) + b"Opaque" + bytes(animation[0xA6:])
    )
    malformed_bank = bytearray(animation_bank)
    struct.pack_into("<I", malformed_bank, 16 + 0x40 + 8, 0xFFFFFFFF)
    fixtures["animation/entry-overrun.bin"] = bytes(malformed_bank)
    for name, offset, length in [
        ("entry-alias", 0, len(animation)),
        ("entry-overlap", len(animation), len(animation_scb)),
    ]:
        malformed_bank = bytearray(animation_bank)
        if name == "entry-overlap":
            struct.pack_into(
                "<I", malformed_bank, 88, len(animation) + len(animation_scb)
            )
        struct.pack_into("<II", malformed_bank, 100, offset, length)
        fixtures[f"animation/{name}.bin"] = bytes(malformed_bank)
    duplicate = bytearray(animation[:0xA7] + b"Header\0group0\0")
    struct.pack_into("<I", duplicate, 0x5C, 0xAE)
    fixtures["animation/duplicate-header.bin"] = bytes(duplicate)
    struct.pack_into("<I", duplicate, 0x60, 1)
    fixtures["animation/later-header.bin"] = bytes(duplicate)
    fixtures["animation/wrapper-overrun.bin"] = animation_bank[:-1]

    return fixtures


def shader_fixtures() -> dict[str, bytes]:
    """Authored FILE envelopes with opaque bodies, never compiled shader code."""
    fixtures = {}

    def chunk(kind: bytes, body: bytes, padded: int = 0) -> bytes:
        size = 16 + len(body)
        return kind + pattern(4, 0x37) + struct.pack(">II", size, padded) + body

    pram = bytearray(32)
    struct.pack_into("<I", pram, 0x18, 32)
    children = (
        chunk(b"FILE", pattern(7, 0x51), 32)
        + pattern(9, 0x63)
        + chunk(b"FILE", pattern(13, 0x75))
        + pattern(3, 0x87)
        + chunk(b"JUNK", pattern(8, 0x99), 24)
        + chunk(b"PRAM", bytes(pram))
    )
    payload = chunk(b"SHD\0", struct.pack(">I", 4) + pattern(12, 0xAB) + children)
    sample = sedb_header("shd", 29, 0x0200, 0x30, 0x30 + len(payload)) + payload
    fixtures["shader/files.bin"] = sample
    no_file = bytearray(sample)
    no_file[80:84] = no_file[112:116] = b"NONE"
    fixtures["shader/no-file.bin"] = bytes(no_file)
    fixtures["shader/truncated.bin"] = sample[:18]
    for name, offset, value, order in [
        ("version", 8, 28, "<I"),
        ("extent", 16, len(sample) + 1, "<I"),
        ("root-size", 56, len(sample) - 49, ">I"),
        ("child-size", 88, 0xFFFFFFFF, ">I"),
        ("child-padding", 92, 0xFFFFFFFF, ">I"),
        ("conversion", 12, 1, "B"),
        ("header-size", 14, 0x40, "<H"),
        ("root-tag", 51, ord("!"), "B"),
    ]:
        malformed = bytearray(sample)
        struct.pack_into(order, malformed, offset, value)
        fixtures[f"shader/{name}.bin"] = bytes(malformed)

    def res(body: bytes, count: int = 1) -> bytes:
        return (
            sedb_header(
                "RES ",
                4000,
                0x0400,
                0x40,
                64 + 16 * count + len(body),
                res_extended(count, 0, "syn"),
            )
            + directory_entry(7, 0, len(body), 0) * count
            + body
        )

    nested = res(res(sample))
    fixtures["shader/nested-res.bin"] = nested
    fixtures["shader/selected-alias.bin"] = res(res(sample), 2)
    malformed = bytearray(nested)
    struct.pack_into(">I", malformed, 248, 0xFFFFFFFF)
    fixtures["shader/nested-child-size.bin"] = bytes(malformed)
    wrapped = b"PWIB" + struct.pack(">III", 16 + len(nested), 16, 16 + len(nested))
    fixtures["shader/pwib.bin"] = wrapped + nested
    malformed = bytearray(wrapped + nested)
    struct.pack_into(">I", malformed, 4, len(malformed) + 1)
    fixtures["shader/pwib-boundary.bin"] = bytes(malformed)
    return fixtures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify the committed fixtures match this generator and exit non-zero if not",
    )
    args = parser.parse_args()

    fixtures = build_fixtures()
    differences = []
    for relative, data in sorted(fixtures.items()):
        path = PUBLIC_ROOT / relative
        if args.check:
            if not path.exists():
                differences.append("{0}: missing".format(relative))
            elif path.read_bytes() != data:
                differences.append("{0}: differs from the generator".format(relative))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)

    actual = {
        path.relative_to(PUBLIC_ROOT).as_posix()
        for path in PUBLIC_ROOT.rglob("*")
        if path.is_file() and path.relative_to(PUBLIC_ROOT).as_posix() != "README.md"
    }
    for relative in sorted(actual - fixtures.keys()):
        differences.append("{0}: not produced by the generator".format(relative))

    if differences:
        for line in differences:
            print("fixtures: {0}".format(line))
        print("fixtures: FAILED ({0} finding(s))".format(len(differences)))
        return 1

    if args.check:
        print("fixtures: OK ({0} fixture(s) reproduce)".format(len(fixtures)))
        return 0

    print("fixtures: wrote {0} fixture(s)".format(len(fixtures)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
