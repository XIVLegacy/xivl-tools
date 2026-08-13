#!/usr/bin/env python3
"""Census the SQEX containers of a 1.x client install.

This is the research command that produced the tables in
docs/formats/sqex.md. It is not part of validation,
it makes no support claim, and it never runs in CI.

    python tools/research/census_sqwt.py --client-root <dir>

`--client-root` is required and has no default: there is no client-install
search, no environment fallback, and no workspace-relative path.

Five passes:

- the sweep counts what is in client/sqwt by signature and by suffix;
- the container pass checks the eight-byte signature, the block split, and
  whether the run shorter than a block at the end of a file is plaintext;
- the key pass checks the two facts that identify the key without knowing
  the cipher: no ciphertext block is shared by two files with different
  base names, and the first block is a bijection with the base name;
- the decode pass runs the cipher, counts what decodes as UTF-8 and as a
  document, and checks that re-enciphering reproduces every input byte;
- the grammar pass counts the constructs the decoded documents use, which
  is what the reader's widget profile is held to.

The cipher is tools/blowfish.py, shared with the fixture generator so
the two cannot disagree about it. Its tables are computed from the
hexadecimal expansion of pi rather than transcribed from anywhere.

It prints counts only. No client bytes are written anywhere, no decoded
text is printed, and nothing it prints is committed by this script.
"""

from __future__ import annotations

import argparse
import collections
import os
import pathlib
import re
import sys

SIGNATURE = b"SQEX" + bytes(4)
HEADER_SIZE = len(SIGNATURE)
BYTE_ORDER_MARK = "\ufeff"

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))

from blowfish import BLOCK_SIZE, Blowfish  # noqa: E402


# -- the sweep ------------------------------------------------------------


def sqwt_files(client_root: pathlib.Path) -> list[pathlib.Path]:
    root = client_root / "client" / "sqwt"
    if not root.is_dir():
        raise SystemExit("census: {0} is not a directory".format(root))
    found = []
    for directory, _, names in os.walk(root):
        for name in names:
            found.append(pathlib.Path(directory) / name)
    found.sort()
    return found


def printable(data: bytes) -> bool:
    return all(0x20 <= byte < 0x7F or byte in (0x09, 0x0D, 0x0A) for byte in data)


def sweep(files: list[pathlib.Path]) -> list[pathlib.Path]:
    magics: collections.Counter = collections.Counter()
    suffixes: collections.Counter = collections.Counter()
    for path in files:
        head = path.read_bytes()[:4]
        magics[head] += 1
        suffixes[(head, path.suffix.lower())] += 1
    print("files under client/sqwt {0}".format(len(files)))
    for magic, count in magics.most_common():
        print("  {0!r:<20} {1}".format(magic, count))
    print("SQEX files by suffix")
    for (magic, suffix), count in sorted(suffixes.items(), key=lambda item: -item[1]):
        if magic == b"SQEX":
            print("  {0:<10} {1}".format(suffix, count))
    return [path for path in files if path.read_bytes()[:4] == b"SQEX"]


def container_pass(containers: list[pathlib.Path]) -> None:
    reserved_zero = 0
    remainders: collections.Counter = collections.Counter()
    tail_printable = 0
    tail_total = 0
    wide_printable = 0
    wide_total = 0
    for path in containers:
        data = path.read_bytes()
        if data[:HEADER_SIZE] == SIGNATURE:
            reserved_zero += 1
        body = len(data) - HEADER_SIZE
        remainder = body % BLOCK_SIZE
        remainders[remainder] += 1
        if remainder:
            tail_total += 1
            if printable(data[len(data) - remainder :]):
                tail_printable += 1
        # The same test at sixteen bytes, which is what rules that block
        # size out rather than leaving it to preference.
        wide = body % 16
        if wide:
            wide_total += 1
            if printable(data[len(data) - wide :]):
                wide_printable += 1
    print("")
    print("containers carrying the 8-byte signature {0} of {1}".format(reserved_zero, len(containers)))
    print("plaintext tail is text, 8-byte blocks    {0} of {1}".format(tail_printable, tail_total))
    print("plaintext tail is text, 16-byte blocks   {0} of {1}".format(wide_printable, wide_total))
    print("body length modulo 8")
    for remainder in range(BLOCK_SIZE):
        print("  {0}  {1}".format(remainder, remainders[remainder]))


def key_pass(containers: list[pathlib.Path]) -> None:
    """The two facts that name the key without knowing the cipher."""
    owners: dict[bytes, set] = collections.defaultdict(set)
    first_to_names: dict[bytes, set] = collections.defaultdict(set)
    name_to_first: dict[str, set] = collections.defaultdict(set)
    blocks = 0
    for path in containers:
        body = path.read_bytes()[HEADER_SIZE:]
        name = path.name
        if len(body) >= BLOCK_SIZE:
            first_to_names[body[:BLOCK_SIZE]].add(name)
            name_to_first[name].add(body[:BLOCK_SIZE])
        for index in range(len(body) // BLOCK_SIZE):
            block = body[index * BLOCK_SIZE : (index + 1) * BLOCK_SIZE]
            owners[block].add(name)
            blocks += 1
    shared = sum(1 for names in owners.values() if len(names) > 1)
    print("")
    print("enciphered blocks                  {0}".format(blocks))
    print("  distinct                         {0}".format(len(owners)))
    print("  shared by two different names    {0}".format(shared))
    print("distinct base names                {0}".format(len(name_to_first)))
    print("distinct first blocks              {0}".format(len(first_to_names)))
    print("  first blocks naming two names    {0}".format(sum(1 for v in first_to_names.values() if len(v) > 1)))
    print("  names with two first blocks      {0}".format(sum(1 for v in name_to_first.values() if len(v) > 1)))


def decode_pass(containers: list[pathlib.Path]) -> list[tuple[pathlib.Path, str]]:
    schedules: dict[str, Blowfish] = {}
    decoded_text = []
    utf8 = 0
    round_trip = 0
    marks = 0
    declarations = 0
    for path in containers:
        data = path.read_bytes()
        cipher = schedules.get(path.name)
        if cipher is None:
            cipher = schedules[path.name] = Blowfish(path.name.encode("utf-8"))
        body = cipher.decrypt(data[HEADER_SIZE:])
        if SIGNATURE + cipher.encrypt(body) == data:
            round_trip += 1
        try:
            text = body.decode("utf-8")
        except UnicodeDecodeError:
            continue
        utf8 += 1
        if text.startswith(BYTE_ORDER_MARK):
            marks += 1
            text = text[1:]
        if text.startswith("<?xml"):
            declarations += 1
        decoded_text.append((path, text))
    print("")
    print("containers decoded                 {0}".format(len(containers)))
    print("  body decodes as UTF-8            {0}".format(utf8))
    print("  re-enciphering reproduces input  {0}".format(round_trip))
    print("  opens on a byte order mark       {0}".format(marks))
    print("  opens on an XML declaration      {0}".format(declarations))
    return decoded_text


def grammar_pass(decoded: list[tuple[pathlib.Path, str]]) -> None:
    elements: collections.Counter = collections.Counter()
    attributes: collections.Counter = collections.Counter()
    roots: collections.Counter = collections.Counter()
    constructs: collections.Counter = collections.Counter()
    for _, text in decoded:
        body = text.lstrip(BYTE_ORDER_MARK)
        match = re.match(r"<\s*([A-Za-z_][\w.:-]*)", body)
        if match:
            roots[match.group(1)] += 1
        if "<!--" in body:
            constructs["comment"] += 1
        if "&" in body:
            constructs["ampersand"] += 1
        if re.search(r"<!(?!--)", body):
            constructs["cdata, doctype, or other bang"] += 1
        if re.search(r"<\?", body):
            constructs["processing instruction"] += 1
        if re.search(r"[\w.:-]\s*=\s*'", body):
            constructs["single-quoted attribute value"] += 1
        if re.search(r"</?\s*[A-Za-z_][\w.-]*:", body):
            constructs["namespace-qualified element name"] += 1
        for match in re.finditer(r"<\s*([A-Za-z_][\w.:-]*)", body):
            elements[match.group(1)] += 1
        for match in re.finditer(r'([A-Za-z_][\w.:-]*)\s*=\s*"', body):
            attributes[match.group(1)] += 1
    print("")
    print("document roots")
    for name, count in roots.most_common():
        print("  {0:<24} {1}".format(name, count))
    print("distinct element names             {0}".format(len(elements)))
    print("distinct attribute names           {0}".format(len(attributes)))
    print("  of those, namespace qualified    {0}".format(sum(1 for name in attributes if ":" in name)))
    print("documents using a construct beyond the reader's base grammar")
    for name in (
        "comment",
        "ampersand",
        "cdata, doctype, or other bang",
        "processing instruction",
        "single-quoted attribute value",
        "namespace-qualified element name",
    ):
        print("  {0:<34} {1}".format(name, constructs[name]))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--client-root",
        required=True,
        help="path to a 1.x client install; required, with no default",
    )
    args = parser.parse_args()
    client_root = pathlib.Path(args.client_root)
    if not client_root.is_dir():
        print("census: {0} is not a directory".format(client_root), file=sys.stderr)
        return 1

    files = sqwt_files(client_root)
    containers = sweep(files)
    container_pass(containers)
    key_pass(containers)
    decoded = decode_pass(containers)
    grammar_pass(decoded)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
