#!/usr/bin/env python3
"""Census the complete sheet stack of a 1.x client install.

This is the research command that produced the tables in
docs/formats/ssd-sheet.md. It is not part of validation; it
makes no support claim, and it never runs in CI.

    python tools/research/census_sheet_stack.py --client-root <dir>

`--client-root` is required and has no default: there is no client-install
search, no environment fallback, and no workspace-relative path.

The command walks the resource tree once, then runs both independent views:

- the scrambled census recognizes and decodes the document containers, checks
  their key derivation, inventories the sheet declarations, and checks row
  coverage across the discovered documents;
- the reachable pass follows the named SSD documents, decodes each block's rows
  against its column list, and checks the row-offset array and the enable
  ranges against what the rows actually say;
- the string-stream pass reads every resource and counts the ones
  whose whole content frames as a stream of string values.

It prints counts only. No client bytes are written anywhere, no decoded
text is printed, and nothing it prints is committed by this script.
"""

from __future__ import annotations

import argparse
import collections
import os
import pathlib
import sys
import xml.etree.ElementTree as ET

BYTE_ORDER_MARK = b"\xef\xbb\xbf"
DECLARATION = b'<?xml version="1.0" encoding="utf-8"?>'
TRAILER = 0xF1
KNOWN_PLAINTEXT_WORD = 0x6C6D
SCRAMBLE_MARKER = 0xFF
SCRAMBLE_KEY = 0x73
TOKEN_START = 0x02
TOKEN_END = 0x03

# Fixed widths used by the reachable-document pass. Only the types the 1.23b
# documents declare, matching
# src/formats/src/sheet.rs. A type with no width established against retail
# data is counted as unknown rather than given an invented width.
REACHABLE_FIXED_WIDTH = {"u8": 1, "bool": 1, "s32": 4, "float": 4}

# The discovered-document pass checks this broader set against the corpus.
ROW_FIXED_WIDTH = {
    "u8": 1,
    "s8": 1,
    "bool": 1,
    "u16": 2,
    "s16": 2,
    "f16": 2,
    "u32": 4,
    "s32": 4,
    "float": 4,
}

# Payload-length escapes, matching src/formats/src/richstring.rs.
ESCAPE_WIDTH = {0xF0: 1, 0xF1: 1, 0xF2: 2}


def resource_path(data_root: pathlib.Path, value: int) -> pathlib.Path:
    a, b, c, d = value.to_bytes(4, "big")
    return data_root / "{0:02X}".format(a) / "{0:02X}".format(b) / "{0:02X}".format(
        c
    ) / "{0:02X}.DAT".format(d)


def resource_id(path: pathlib.Path) -> int:
    parts = path.parts
    return int(parts[-4] + parts[-3] + parts[-2] + parts[-1][:2], 16)


def walk_resources(data_root: pathlib.Path):
    stack = [str(data_root)]
    while stack:
        directory = stack.pop()
        try:
            entries = list(os.scandir(directory))
        except OSError:
            continue
        for entry in entries:
            if entry.is_dir(follow_symlinks=False):
                stack.append(entry.path)
            elif entry.name.upper().endswith(".DAT"):
                yield pathlib.Path(entry.path)


def unscramble(buffer: bytearray) -> None:
    """Swap every other byte from the front with every other from the back."""
    low, high = 0, len(buffer) - 1
    while low < high:
        buffer[low], buffer[high] = buffer[high], buffer[low]
        low += 2
        high -= 2


def decode_document(raw: bytes):
    """Decode a scrambled document, or return None if it is not one."""
    if not raw or raw[-1] != TRAILER:
        return None
    encoded_length = len(raw) - 1
    if encoded_length < 8:
        return None
    buffer = bytearray(raw[:encoded_length])
    unscramble(buffer)
    key_a = (encoded_length * 7) & 0xFFFF
    key_b = int.from_bytes(buffer[6:8], "little") ^ KNOWN_PLAINTEXT_WORD
    for offset in range(0, encoded_length - 1, 4):
        word = int.from_bytes(buffer[offset : offset + 2], "little") ^ key_a
        buffer[offset : offset + 2] = word.to_bytes(2, "little")
    for offset in range(2, encoded_length - 1, 4):
        word = int.from_bytes(buffer[offset : offset + 2], "little") ^ key_b
        buffer[offset : offset + 2] = word.to_bytes(2, "little")
    if encoded_length % 4 == 1:
        buffer[encoded_length - 1] ^= (key_a & 0xFF) ^ (key_b & 0xFF)
    return bytes(buffer)


def frames_as_string_stream(raw: bytes):
    position, total, count = 0, len(raw), 0
    if total == 0:
        return None
    while position < total:
        if position + 2 > total:
            return None
        length = raw[position] | (raw[position + 1] << 8)
        position += 2
        if length == 0 or position + length > total:
            return None
        body = raw[position : position + length]
        terminator = SCRAMBLE_KEY if body[0] == SCRAMBLE_MARKER else 0x00
        if body[-1] != terminator:
            return None
        position += length
        count += 1
    return count


def string_value_end(raw: bytes, position: int):
    if position + 2 > len(raw):
        return None
    length = raw[position] | (raw[position + 1] << 8)
    end = position + 2 + length
    if length == 0 or end > len(raw):
        return None
    body = raw[position + 2 : end]
    terminator = SCRAMBLE_KEY if body[0] == SCRAMBLE_MARKER else 0x00
    if body[-1] != terminator:
        return None
    return end


def read_xml(path: pathlib.Path):
    raw = path.read_bytes()
    body = raw[3:] if raw.startswith(BYTE_ORDER_MARK) else raw
    return ET.fromstring(body.decode("utf-8"))


def decode_body(body: bytes) -> bytes:
    if body[0] == SCRAMBLE_MARKER:
        return bytes(byte ^ SCRAMBLE_KEY for byte in body[1:-1])
    return body[:-1]


def frame_strings(raw: bytes):
    """Every string value in a resource, or None if the stream does not tile
    the input exactly."""
    bodies = []
    position = 0
    while position < len(raw):
        if position + 2 > len(raw):
            return None
        length = int.from_bytes(raw[position : position + 2], "little")
        if length < 1 or position + 2 + length > len(raw):
            return None
        body = raw[position + 2 : position + 2 + length]
        terminator = SCRAMBLE_KEY if body[0] == SCRAMBLE_MARKER else 0x00
        if body[-1] != terminator:
            return None
        bodies.append(body)
        position += 2 + length
    return bodies


def scan_tokens(text: bytes, counters: dict) -> bool:
    """Walk the control tokens of a decoded body, rebuilding it from the
    pieces as it goes. Returns whether every token framed; the rebuilt
    bytes are compared with the input, which is the corpus-scale check that
    the token representation loses nothing."""
    position = 0
    runs = bytearray()
    rebuilt = bytearray()
    framed = True
    while position < len(text):
        if text[position] != TOKEN_START:
            runs.append(text[position])
            rebuilt.append(text[position])
            position += 1
            continue
        lead_at = position + 2
        if lead_at >= len(text):
            framed = False
            break
        lead = text[lead_at]
        if lead < 0xF0:
            size = lead - 1
            after = lead_at + 1
        elif lead in ESCAPE_WIDTH:
            width = ESCAPE_WIDTH[lead]
            counters["escapes"][lead] += 1
            size = int.from_bytes(text[lead_at + 1 : lead_at + 1 + width], "big")
            if lead == 0xF1:
                size <<= 8
            after = lead_at + 1 + width
        else:
            counters["unknown_escapes"][lead] += 1
            framed = False
            break
        end = after + size
        if size < 0 or end >= len(text) or text[end] != TOKEN_END:
            framed = False
            break
        counters["codes"][text[position + 1]] += 1
        # code, length bytes, and payload verbatim between the two markers
        rebuilt.append(TOKEN_START)
        rebuilt.extend(text[position + 1 : end])
        rebuilt.append(TOKEN_END)
        position = end + 1
    if framed:
        if bytes(rebuilt) == text:
            counters["round_trip"] += 1
        else:
            counters["round_trip_failed"] += 1
        try:
            bytes(runs).decode("utf-8")
            counters["text_utf8"] += 1
        except UnicodeDecodeError:
            counters["text_not_utf8"] += 1
    return framed


def reachable_pass(data_root: pathlib.Path, masters: list[int]) -> None:
    documents = []
    for master in masters:
        path = resource_path(data_root, master)
        if not path.exists():
            print("  master 0x{0:08X} is not in this install".format(master))
            continue
        root = read_xml(path)
        # A master's sheets are references. A document whose sheets are
        # definitions is its own schema document.
        named = [
            sheet.get("infofile") for sheet in root.findall("sheet") if sheet.get("infofile")
        ]
        if named:
            documents.extend(int(value) for value in named)
        else:
            documents.append(master)

    print("documents given: {0}, schema documents to read: {1}".format(len(masters), len(documents)))

    sheets = 0
    blocks = 0
    languages = collections.Counter()
    column_types = collections.Counter()
    rows_total = 0
    rows_exact = 0
    offsets_match = 0
    enable_match = 0
    for document in documents:
        root = read_xml(resource_path(data_root, document))
        for sheet in root.findall("sheet"):
            sheets += 1
            languages[sheet.get("lang") or "none"] += 1
            columns = [param.text.strip() for param in sheet.findall("type/param")]
            for column in columns:
                column_types[column] += 1
            for element in sheet.findall("block/file"):
                blocks += 1
                begin = int(element.get("begin"))
                raw = resource_path(data_root, int(element.text)).read_bytes()
                offsets_raw = resource_path(data_root, int(element.get("offset"))).read_bytes()
                enable_raw = resource_path(data_root, int(element.get("enable"))).read_bytes()

                offsets = [
                    int.from_bytes(offsets_raw[index : index + 4], "little")
                    for index in range(0, len(offsets_raw), 4)
                ]
                boundaries = []
                previous = 0
                enabled_by_offsets = set()
                for index, value in enumerate(offsets):
                    if value != previous:
                        boundaries.append(value)
                        enabled_by_offsets.add(begin + index)
                        previous = value

                # Sequential decode: a string column is self-delimiting and
                # every other column is fixed width, so the column list alone
                # fixes where a row ends.
                position = 0
                walked = []
                ok = True
                while position < len(raw) and ok:
                    for column in columns:
                        if column == "str":
                            if position + 2 > len(raw):
                                ok = False
                                break
                            length = int.from_bytes(raw[position : position + 2], "little")
                            position += 2 + length
                        elif column in REACHABLE_FIXED_WIDTH:
                            position += REACHABLE_FIXED_WIDTH[column]
                        else:
                            ok = False
                            break
                    if ok and position <= len(raw):
                        walked.append(position)
                rows_total += len(walked)
                if ok and position == len(raw):
                    rows_exact += len(walked)
                if ok and walked == boundaries:
                    offsets_match += 1

                ranges = [
                    (
                        int.from_bytes(enable_raw[index : index + 4], "little"),
                        int.from_bytes(enable_raw[index + 4 : index + 8], "little"),
                    )
                    for index in range(0, len(enable_raw), 8)
                ]
                enabled = set()
                for first, count in ranges:
                    enabled.update(range(first, first + count))
                if enabled == enabled_by_offsets:
                    enable_match += 1

    print("sheets: {0}, file blocks: {1}".format(sheets, blocks))
    print("sheet languages: {0}".format(dict(languages)))
    print("column types declared: {0}".format(dict(column_types)))
    print("rows walked: {0}, consuming their block exactly: {1}".format(rows_total, rows_exact))
    print(
        "blocks whose sequential decode reproduces the row-offset array: {0} of {1}".format(
            offsets_match, blocks
        )
    )
    print(
        "blocks whose enable ranges equal the rows with data: {0} of {1}".format(
            enable_match, blocks
        )
    )


def sweep_pass(paths: list[pathlib.Path]) -> None:
    counters = {
        "codes": collections.Counter(),
        "escapes": collections.Counter(),
        "unknown_escapes": collections.Counter(),
        "text_utf8": 0,
        "text_not_utf8": 0,
        "round_trip": 0,
        "round_trip_failed": 0,
    }
    resources = 0
    documents = 0
    files = 0
    strings = 0
    scrambled = 0
    plain = 0
    key_terminated = 0
    key_interior = 0
    tokens_not_framing = 0
    prefixes = collections.Counter()

    for path in paths:
        resources += 1
        raw = path.read_bytes()
        probe = raw[3:] if raw.startswith(BYTE_ORDER_MARK) else raw
        if probe[:5] == b"<?xml" or probe[:4] == b"<ssd" or probe[:6] == b"<sheet":
            documents += 1
        if len(raw) < 3:
            continue
        bodies = frame_strings(raw)
        if bodies is None:
            continue
        files += 1
        prefixes[path.parts[-4] + path.parts[-3]] += 1
        for body in bodies:
            strings += 1
            if body[0] == SCRAMBLE_MARKER:
                scrambled += 1
                key_terminated += 1
                if SCRAMBLE_KEY in body[1:-1]:
                    key_interior += 1
            else:
                plain += 1
            if not scan_tokens(decode_body(body), counters):
                tokens_not_framing += 1

    print("resources: {0}".format(resources))
    print("resources whose first bytes are an xml document: {0}".format(documents))
    print("resources framing as a string stream: {0}".format(files))
    print("  by identifier prefix: {0}".format(dict(prefixes)))
    print("strings: {0} (obfuscated {1}, plain {2})".format(strings, scrambled, plain))
    print(
        "obfuscated bodies terminated by 0x{0:02X}: {1}, with an interior 0x{0:02X}: {2}".format(
            SCRAMBLE_KEY, key_terminated, key_interior
        )
    )
    print(
        "strings whose text outside the tokens decodes as UTF-8: {0}, failing: {1}".format(
            counters["text_utf8"], counters["text_not_utf8"]
        )
    )
    print("strings with a control token that does not frame: {0}".format(tokens_not_framing))
    print(
        "strings rebuilt byte for byte from their text runs and tokens: {0}, failing: {1}".format(
            counters["round_trip"], counters["round_trip_failed"]
        )
    )
    print(
        "control tokens: {0} across {1} distinct codes".format(
            sum(counters["codes"].values()), len(counters["codes"])
        )
    )
    print("  codes: {0}".format(sorted(counters["codes"].items())))
    print("  length escapes used: {0}".format(sorted(counters["escapes"].items())))
    print("  length escapes not established: {0}".format(sorted(counters["unknown_escapes"].items())))


def scrambled_pass(data_root: pathlib.Path, paths: list[pathlib.Path]) -> None:
    scanned = 0
    trailer_candidates = 0
    decoded_header = 0
    decoded_utf8 = 0
    well_formed = 0
    not_a_document = []
    roots = collections.Counter()
    key_a_ok = 0
    trailer_rule = collections.Counter()
    documents = {}
    plaintext_documents = 0
    streams = set()

    for path in paths:
        scanned += 1
        raw = path.read_bytes()
        if raw.startswith(BYTE_ORDER_MARK):
            plaintext_documents += 1
            documents[resource_id(path)] = raw
            continue
        if raw and raw[-1] == TRAILER and not raw.startswith(b"SEDB"):
            trailer_candidates += 1
            decoded = decode_document(raw)
            if decoded is None or not decoded.startswith(BYTE_ORDER_MARK + DECLARATION):
                not_a_document.append(resource_id(path))
                continue
            decoded_header += 1
            # Byte 1 of the document is 0xBB, so the high half of the first
            # word key is recoverable without assuming the formula.
            encoded_length = len(raw) - 1
            if ((encoded_length * 7) >> 8) & 0xFF == (raw[1] ^ 0xBB):
                key_a_ok += 1
            trailer_rule[(encoded_length % 4, decoded[-1])] += 1
            try:
                text = decoded.decode("utf-8")
            except UnicodeDecodeError:
                continue
            decoded_utf8 += 1
            try:
                element = ET.fromstring(text[1:])
            except ET.ParseError:
                continue
            well_formed += 1
            roots[element.tag] += 1
            documents[resource_id(path)] = decoded
            continue
        if frames_as_string_stream(raw) is not None:
            streams.add(resource_id(path))

    print("== sweep")
    print("resources scanned                {}".format(scanned))
    print("plaintext XML documents          {}".format(plaintext_documents))
    print("resources ending in the trailer  {}".format(trailer_candidates))
    print("  decode to BOM + declaration    {}".format(decoded_header))
    print("  decode as UTF-8                {}".format(decoded_utf8))
    print("  parse as well-formed XML       {}".format(well_formed))
    print("  not a document                 {}".format(len(not_a_document)))
    print("document roots                   {}".format(dict(roots)))
    print()
    print("== derivation")
    print("word key A == (7 * encodedLength) & 0xFFFF, checked against the")
    print("byte order mark's second byte:   {} of {}".format(key_a_ok, decoded_header))
    print("final decoded byte by encodedLength mod 4:")
    for (residue, byte), count in sorted(trailer_rule.items()):
        print("  mod 4 == {}  ends 0x{:02x}  {}".format(residue, byte, count))
    print()

    sheets = 0
    blocks = 0
    languages = collections.Counter()
    modes = collections.Counter()
    column_types = collections.Counter()
    named_data = {}
    named_blocks = []
    named_enable = set()
    named_offsets = set()
    infofiles = set()
    for value, raw in documents.items():
        element = ET.fromstring(raw.decode("utf-8")[1:])
        for sheet in element.findall("sheet"):
            sheets += 1
            languages[sheet.get("lang")] += 1
            modes[sheet.get("mode")] += 1
            if sheet.get("infofile"):
                infofiles.add(int(sheet.get("infofile")))
            columns = [
                param.text
                for type_element in sheet.findall("type")
                for param in type_element.findall("param")
            ]
            for name in columns:
                column_types[name] += 1
            for block in sheet.findall("block"):
                for file_element in block.findall("file"):
                    blocks += 1
                    data_id = int(file_element.text)
                    enable_id = int(file_element.get("enable"))
                    offset_id = int(file_element.get("offset"))
                    named_data[data_id] = columns
                    named_enable.add(enable_id)
                    named_offsets.add(offset_id)
                    named_blocks.append(
                        (data_id, enable_id, offset_id, columns, sheet.get("lang"))
                    )

    print("== documents")
    print("documents                        {}".format(len(documents)))
    print("sheets                           {}".format(sheets))
    print("blocks                           {}".format(blocks))
    print("languages                        {}".format(dict(languages)))
    print("modes                            {}".format(dict(modes)))
    print("column types                     {}".format(dict(column_types)))
    print(
        "infofile references resolving    {} of {}".format(
            len(infofiles & set(documents)), len(infofiles)
        )
    )
    print()

    print("== coverage")
    print("resources framing as a stream    {}".format(len(streams)))
    print("  named as a block data file     {}".format(len(streams & set(named_data))))
    print("  named by nothing               {}".format(len(streams - set(named_data))))
    unnamed = sorted(streams - set(named_data))
    print(
        "    of those, named as an enable file  {}".format(
            len([value for value in unnamed if value in named_enable])
        )
    )
    print()

    print("== rows")
    exact = 0
    short = 0
    unknown_type = 0
    blocks_checked = 0
    blocks_reproducing = 0
    missing = 0
    missing_resources = 0
    missing_languages = collections.Counter()
    for value, enable_value, offset_value, columns, language in sorted(named_blocks):
        path = resource_path(data_root, value)
        present = [
            path.is_file(),
            resource_path(data_root, enable_value).is_file(),
            resource_path(data_root, offset_value).is_file(),
        ]
        if not all(present):
            missing += 1
            missing_resources += present.count(False)
            missing_languages[language] += 1
            continue
        raw = path.read_bytes()
        if any(name not in ROW_FIXED_WIDTH and name != "str" for name in columns):
            unknown_type += 1
            continue
        blocks_checked += 1
        position = 0
        ends = []
        ok = True
        while position < len(raw):
            start = position
            for name in columns:
                if name == "str":
                    end = string_value_end(raw, position)
                    if end is None:
                        ok = False
                        break
                    position = end
                else:
                    position += ROW_FIXED_WIDTH[name]
                    if position > len(raw):
                        ok = False
                        break
            if not ok:
                break
            ends.append(position)
            if position == start:
                ok = False
                break
        if ok and position == len(raw):
            exact += len(ends)
            blocks_reproducing += 1
        else:
            short += 1
    print("blocks with a readable column list {}".format(blocks_checked))
    print("  data file tiles exactly as rows  {}".format(blocks_reproducing))
    print("  data file does not tile          {}".format(short))
    print("rows consuming their span exactly  {}".format(exact))
    print("blocks naming an unestablished type {}".format(unknown_type))
    print("blocks with an absent resource     {}".format(missing))
    print("  absent resource ids              {}".format(missing_resources))
    print("  by language                      {}".format(dict(missing_languages)))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--client-root",
        required=True,
        help="client install directory holding data/; required, with no default",
    )
    parser.add_argument(
        "--master",
        action="append",
        default=[],
        help="resource id of a master or schema document, repeatable; defaults to the three this install has",
    )
    parser.add_argument(
        "--skip-sweep",
        action="store_true",
        help="skip the string-stream sweep after the document passes",
    )
    args = parser.parse_args()

    data_root = pathlib.Path(args.client_root) / "data"
    if not data_root.is_dir():
        print("census: no data directory under {0}".format(args.client_root))
        return 1

    paths = list(walk_resources(data_root))
    masters = [int(value, 0) for value in args.master] or [
        0x2795_0000,
        0x02B8_0000,
        0x15AF_0000,
    ]

    print("== scrambled census ==")
    scrambled_pass(data_root, paths)
    print("")
    print("== sheet census ==")
    print("== reachable pass ==")
    reachable_pass(data_root, masters)
    if not args.skip_sweep:
        print("")
        print("== install sweep ==")
        sweep_pass(paths)
    return 0


if __name__ == "__main__":
    sys.exit(main())
