"""1.23b layout resource rows: list, re-point and append.

Rows are 32 bytes from 0x40; the row count (including DEF_BLK) is a u32 at
0x24 and the table end a u32 at 0x20. Appending grows the header 32 bytes per
row and shifts every later file offset; the payload is unchanged.
"""

import re
import struct

ROWS = 0x40


class LayoutError(Exception):
    pass


def _check(cond, msg):
    if not cond:
        raise LayoutError(msg)


def _text(b):
    return b.split(b"\0")[0].decode("latin1")


def rows(d):
    """List of (file offset, key, tag, file id, flags bytes)."""
    end, count = struct.unpack_from("<II", d, 0x20)
    _check(
        end == ROWS + 32 * count,
        "row table end %#x does not match %d rows" % (end, count),
    )
    _check(d[end : end + 7] == b"SEDBlyb", "row table not followed by SEDBlyb")
    out = []
    for i in range(count):
        o = ROWS + 32 * i
        out.append(
            (
                o,
                _text(d[o : o + 16]),
                _text(d[o + 16 : o + 20]),
                struct.unpack_from("<I", d, o + 20)[0],
                bytes(d[o + 24 : o + 32]),
            )
        )
    return out


def _key(key):
    k = key.encode("latin1")
    _check(0 < len(k) <= 15, "key must be 1..15 characters: " + key)
    return k.ljust(16, b"\0")


def set_rows(d, specs):
    """specs: list of (row offset, key, file id). Rewrites key and file id; tag and flags stay. Returns (bytes, log)."""
    d = bytearray(d)
    table = {r[0]: r for r in rows(d)}
    log = []
    for off, key, fid in specs:
        _check(
            off in table, "%#x is not the start of a row (rows are 0x40 + 32*i)" % off
        )
        old = table[off]
        d[off : off + 16] = _key(key)
        struct.pack_into("<I", d, off + 20, fid)
        log.append(
            "row %#x %s: %s %#010x -> %s %#010x"
            % (off, old[2], old[1], old[3], key, fid)
        )
    return bytes(d), log


def add_rows(d, specs):
    """specs: list of (tag, key, file id). Appends rows before SEDBlyb; flags come from the first row of the same tag."""
    d = bytearray(d)
    existing = rows(d)
    flags, keys = {}, {r[1] for r in existing}
    for r in existing:
        flags.setdefault(r[2], r[4])
    new = bytearray()
    for tag, key, fid in specs:
        t = tag.encode("latin1")
        _check(0 < len(t) <= 4, "tag must be 1..4 characters: " + tag)
        _check(key not in keys, "key already has a row: " + key)
        keys.add(key)
        new += (
            _key(key)
            + t.ljust(4, b"\0")
            + struct.pack("<I", fid)
            + flags.get(tag, b"\x02\x01" + b"\0" * 6)
        )
    end, count = struct.unpack_from("<II", d, 0x20)
    n = len(new) // 32
    d[end:end] = new
    struct.pack_into("<II", d, 0x20, end + 32 * n, count + n)
    return bytes(d), "added %d rows; table end %#x -> %#x, count %d -> %d" % (
        n,
        end,
        end + 32 * n,
        count,
        count + n,
    )


def model_texture_keys(m):
    """(key, texture name) for each texture a 1.0 model's shader block names: '<key>\\0bxt' after its .dds name."""
    out, seen = [], set()
    for mt in re.finditer(rb"([0-9][A-Za-z0-9]{5}[a-z0-9_]{1,9})\x00bxt", m):
        key = mt.group(1).decode()
        if key in seen:
            continue
        seen.add(key)
        names = re.findall(
            rb"([A-Za-z0-9_]+\.dds)", m[max(0, mt.start() - 200) : mt.start()]
        )
        out.append((key, names[-1].decode() if names else "?"))
    return out
