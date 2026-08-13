#!/usr/bin/env python3
"""Census the configuration files of a 1.x client install.

This is the research command that produced the tables in
docs/formats/configuration.md. It is not part of the
gate, it makes no support claim, and it never runs in CI.

    python tools/research/census_config.py --config-root <dir>
                                           [--client-root <dir>]

`--config-root` is required and has no default. It is the directory the
client keeps its configuration in, which is not under the install: on
Windows it sits in the user's documents. `--client-root` is optional and
turns on one further pass, the only one that reaches outside the four
files.

Four passes:

- the sweep states each file's length and how it divides into 32-bit
  words;
- the grid pass counts written and unwritten words and reports where the
  written ones sit;
- the run pass counts maximal runs of printable units under both
  encodings, which is a census and not a claim: a run of ordinary binary
  bytes that happens to be printable is counted the same as text;
- the stamp pass, with --client-root, counts occurrences of each leading
  word as an immediate constant in the client executables. That is the
  evidence that the word is compiled in rather than written by this
  install, and it is the one statement about meaning this tool makes.

It prints counts, offsets, lengths, and compiled-in leading stamps only. No
user setting, device identifier, or text is printed: these files are
user-written settings rather than client assets.
"""

from __future__ import annotations

import argparse
import pathlib
import struct

# The four files, and whether each opens with the compiled-in stamp word.
FILES = (
    ("config.sys", True),
    ("config.pad", True),
    ("config.lng", False),
    ("config.rgn", False),
)

WORD_SIZE = 4
MIN_RUN_UNITS = 4


def ascii_runs(data: bytes) -> list[tuple[int, int]]:
    runs = []
    start = None
    for index in range(len(data) + 1):
        printable = index < len(data) and 0x20 <= data[index] < 0x7F
        if printable and start is None:
            start = index
        elif not printable and start is not None:
            if index - start >= MIN_RUN_UNITS:
                runs.append((start, index - start))
            start = None
    return runs


def utf16_runs(data: bytes) -> list[tuple[int, int]]:
    runs = []
    start = None
    units = len(data) // 2
    for index in range(units + 1):
        printable = False
        if index < units:
            value = struct.unpack_from("<H", data, index * 2)[0]
            printable = 0x20 <= value <= 0xFFFD and value != 0xFEFF
        if printable and start is None:
            start = index
        elif not printable and start is not None:
            if index - start >= MIN_RUN_UNITS:
                runs.append((start * 2, index - start))
            start = None
    return runs


def census(config_root: pathlib.Path) -> dict[str, bytes]:
    print("configuration files under the supplied root")
    print("  {0:<12} {1:>7} {2:>7} {3:>9}".format("file", "bytes", "words", "remainder"))
    contents = {}
    for name, _ in FILES:
        path = config_root / name
        if not path.is_file():
            print("  {0:<12} {1:>7}".format(name, "absent"))
            continue
        data = path.read_bytes()
        contents[name] = data
        print(
            "  {0:<12} {1:>7} {2:>7} {3:>9}".format(
                name, len(data), len(data) // WORD_SIZE, len(data) % WORD_SIZE
            )
        )
    return contents


def grid_pass(contents: dict[str, bytes]) -> None:
    print("")
    print("the word grid")
    print(
        "  {0:<12} {1:>10} {2:>7} {3:>9} {4:>8}".format(
            "file", "stamp", "words", "unwritten", "written"
        )
    )
    for name, stamped in FILES:
        data = contents.get(name)
        if data is None or len(data) % WORD_SIZE != 0:
            print("  {0:<12} not a word grid".format(name))
            continue
        body = data[WORD_SIZE:] if stamped else data
        words = [
            struct.unpack_from("<I", body, offset)[0] for offset in range(0, len(body), WORD_SIZE)
        ]
        zero = sum(1 for word in words if word == 0)
        stamp = "0x{0:08X}".format(struct.unpack_from("<I", data)[0]) if stamped else "-"
        print(
            "  {0:<12} {1:>10} {2:>7} {3:>9} {4:>8}".format(
                name, stamp, len(words), zero, len(words) - zero
            )
        )


def run_pass(contents: dict[str, bytes]) -> None:
    print("")
    print("printable runs of {0} units or more, both encodings".format(MIN_RUN_UNITS))
    print("  {0:<12} {1:<8} {2:>8} {3:>7}".format("file", "encoding", "offset", "units"))
    for name, _ in FILES:
        data = contents.get(name)
        if data is None:
            continue
        found = [("ascii", offset, units) for offset, units in ascii_runs(data)]
        found += [("utf16le", offset, units) for offset, units in utf16_runs(data)]
        found.sort(key=lambda run: (run[1], run[0]))
        if not found:
            print("  {0:<12} none".format(name))
        for encoding, offset, units in found:
            print(
                "  {0:<12} {1:<8} {2:>8} {3:>7}".format(
                    name, encoding, "0x{0:04X}".format(offset), units
                )
            )


def stamp_pass(client_root: pathlib.Path, contents: dict[str, bytes]) -> None:
    stamps = {}
    for name, stamped in FILES:
        data = contents.get(name)
        if stamped and data is not None and len(data) >= WORD_SIZE:
            stamps[name] = struct.unpack_from("<I", data)[0]
    if not stamps:
        return

    executables = sorted(path for path in client_root.glob("*.exe") if path.is_file())
    if not executables:
        raise SystemExit("census: no executable under {0}".format(client_root))

    print("")
    print("the leading word as an immediate constant in the client executables")
    header = "  {0:<20}".format("executable") + "".join(
        "{0:>16}".format(name) for name in stamps
    )
    print(header)
    for path in executables:
        image = path.read_bytes()
        counts = "".join(
            "{0:>16}".format(image.count(struct.pack("<I", value))) for value in stamps.values()
        )
        print("  {0:<20}{1}".format(path.name, counts))
    print("")
    for name, value in stamps.items():
        year, month, day = value >> 16, (value >> 8) & 0xFF, value & 0xFF
        print(
            "  {0} opens with 0x{1:08X}, which reads as {2:04X}-{3:02X}-{4:02X}".format(
                name, value, year, month, day
            )
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--config-root",
        required=True,
        type=pathlib.Path,
        help="directory holding config.sys, config.pad, config.lng, and config.rgn",
    )
    parser.add_argument(
        "--client-root",
        type=pathlib.Path,
        help="client install root, for the stamp-constant pass over its executables",
    )
    args = parser.parse_args()

    if not args.config_root.is_dir():
        raise SystemExit("census: {0} is not a directory".format(args.config_root))
    contents = census(args.config_root)
    if not contents:
        raise SystemExit("census: no configuration file under {0}".format(args.config_root))
    grid_pass(contents)
    run_pass(contents)
    if args.client_root is not None:
        if not args.client_root.is_dir():
            raise SystemExit("census: {0} is not a directory".format(args.client_root))
        stamp_pass(args.client_root, contents)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
