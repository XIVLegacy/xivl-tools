"""Read 1.23b background model files: WRB chunks and MDL/MESH boxes.

An MDL owns an AABB container holding an AABB leaf and a COMP leaf; every MESH
owns an AABB container holding one AABB leaf. A leaf body is six big-endian
f32 (min xyz, max xyz). Positions are SHORT4N decoded through the MDL COMP box:
decoded = COMP centre + raw / 32767 * COMP half extent. See
docs/formats/wrb-model.md.
"""

import math
import struct
import sys

CONTAINERS = {b"WRB\0", b"MDL\0", b"MDLC", b"MESH", b"AABB"}


class Chunk:
    def __init__(self, tag, at, size, children):
        self.tag = tag
        self.at = at
        self.size = size
        self.children = children

    @property
    def body(self):
        return self.at + 16


def be32(d, a):
    return struct.unpack_from(">I", d, a)[0]


def parse_chunks(d, start, end, depth=0):
    if depth > 32:
        raise ValueError("chunk depth at 0x%X" % start)
    out = []
    at = start
    while at < end:
        if d[at] == 0:
            break
        if at + 16 > end:
            raise ValueError("truncated chunk header at 0x%X" % at)
        size, padded = be32(d, at + 8), be32(d, at + 12)
        if size < 16 or at + size > end:
            raise ValueError("bad chunk size 0x%X at 0x%X" % (size, at))
        advance = padded if padded >= size else (size + 15) & ~15
        tag = d[at : at + 4]
        kids = []
        # A 0x28 AABB is a leaf (six floats); every other AABB is a container.
        if tag in CONTAINERS and not (tag == b"AABB" and size == 0x28):
            kids = parse_chunks(d, at + 32, at + size, depth + 1)
        out.append(Chunk(tag, at, size, kids))
        at += advance
    return out


def kids(c, tag):
    return [k for k in c.children if k.tag == tag]


# ---- resource container (SEDBRES) -> WRB roots -----------------------------


def _cs(d, a):
    return d[a : d.find(b"\0", a)].decode("latin1")


def _resources(d, sec, depth=0):
    if d[sec : sec + 8] != b"SEDBRES ":
        return
    count = struct.unpack_from("<I", d, sec + 48)[0]
    table = sec + 64
    base = table + count * 16
    tb = base + struct.unpack_from("<I", d, table + (count - 2) * 16 + 4)[0]
    for i in range(count):
        row = table + i * 16
        t = d[tb + i * 4 : tb + i * 4 + 4].rstrip(b"\0")[::-1].decode("latin1")
        off = base + struct.unpack_from("<I", d, row + 4)[0]
        size, kind = struct.unpack_from("<II", d, row + 8)
        if kind == 0 or size == 0:
            continue
        yield t, off, size
        if t in ("bin", "trb") and size >= 8 and d[off : off + 8] == b"SEDBRES ":
            yield from _resources(d, off, depth + 1)


def find_wrb_roots(d):
    """Return top-level WRB chunks (parsed) found in the file."""
    spots = []
    if d[:4] == b"WRB\0":
        spots.append(0)
    else:
        sec = 16 if d[:4] == b"PWIB" and d[16:24] == b"SEDBRES " else 0
        if d[sec : sec + 8] != b"SEDBRES ":
            raise ValueError("not a SEDBRES/PWIB/WRB file")
        for t, off, size in _resources(d, sec):
            if t == "wrb":
                i = d.find(b"WRB\0", off, off + min(size, 256))
                if i >= 0:
                    spots.append(i)
    roots = []
    for at in spots:
        size = be32(d, at + 8)
        roots.append(Chunk(b"WRB\0", at, size, parse_chunks(d, at + 32, at + size)))
    if not roots:
        raise ValueError("no WRB resource found")
    return roots


def walk(chunks):
    for c in chunks:
        yield c
        yield from walk(c.children)


# ---- model structure --------------------------------------------------------


class Box:
    """A 24-byte box leaf body at absolute file offset `at`."""

    def __init__(self, d, at):
        self.at = at
        self.v = struct.unpack_from(">6f", d, at)

    @property
    def centre(self):
        return [(self.v[i] + self.v[i + 3]) / 2 for i in range(3)]

    @property
    def size(self):
        return [self.v[i + 3] - self.v[i] for i in range(3)]


def box_of(d, owner):
    """(AABB leaf Box, COMP leaf Box) of an owner chunk (MDL or MESH)."""
    aabb = comp = None
    for cont in kids(owner, b"AABB"):
        for leaf in cont.children:
            if leaf.tag == b"AABB" and aabb is None:
                aabb = Box(d, leaf.body)
            elif leaf.tag == b"COMP" and comp is None:
                comp = Box(d, leaf.body)
    return aabb, comp


class Mesh:
    def __init__(self, d, chunk, comp):
        self.chunk = chunk
        self.aabb, _ = box_of(d, chunk)
        self.count = 0
        self.vmin = self.vmax = None
        if comp is None:
            return
        pos = None
        for s in kids(chunk, b"STMS"):
            n, items, stride, _ = struct.unpack_from(">4I", d, s.body)
            descs = [
                struct.unpack_from(">4I", d, s.body + 16 + 16 * i) for i in range(n)
            ]
            p = [x for x in descs if x[3] >> 16 == 0 and x[1:3] == (4, 4)]
            if p and n and (descs[0][3] >> 16) != 255:
                pos = (s.body + 16 + 16 * n, items, stride, p[0][0])
                break
        if pos is None:
            return
        start, items, stride, off = pos
        c = comp.centre
        half = [comp.size[i] / 2 for i in range(3)]
        lo = [math.inf] * 3
        hi = [-math.inf] * 3
        for i in range(items):
            raw = struct.unpack_from(">4h", d, start + i * stride + off)
            for a in range(3):
                x = c[a] + raw[a] / 32767.0 * half[a]
                lo[a] = min(lo[a], x)
                hi[a] = max(hi[a], x)
        self.count = items
        self.vmin, self.vmax = lo, hi


class Mdl:
    def __init__(self, d, chunk):
        self.chunk = chunk
        self.aabb, self.comp = box_of(d, chunk)
        self.meshes = [Mesh(d, m, self.comp) for m in kids(chunk, b"MESH")]


def load(d):
    mdls = []
    for root in find_wrb_roots(d):
        for c in walk(root.children):
            if c.tag == b"MDL\0":
                mdls.append(Mdl(d, c))
    return mdls


def fmt(v):
    return "(" + ", ".join("%.4f" % x for x in v) + ")"


def fbox(b):
    if b is None:
        return "none"
    return "min %s max %s size %s @0x%X" % (
        fmt(b.v[:3]),
        fmt(b.v[3:]),
        fmt(b.size),
        b.at,
    )


def show(d, out=sys.stdout):
    mdls = load(d)
    print("file size %d, MDLs %d" % (len(d), len(mdls)), file=out)
    for mi, m in enumerate(mdls):
        print("MDL %d  meshes %d" % (mi, len(m.meshes)), file=out)
        print("  AABB leaf: " + fbox(m.aabb), file=out)
        print("  COMP leaf: " + fbox(m.comp), file=out)
        for k, me in enumerate(m.meshes):
            print("  mesh %d  vertices %d" % (k, me.count), file=out)
            print("    AABB: " + fbox(me.aabb), file=out)
            if me.vmin is not None:
                size = [me.vmax[i] - me.vmin[i] for i in range(3)]
                print(
                    "    decoded vertices: min %s max %s size %s"
                    % (fmt(me.vmin), fmt(me.vmax), fmt(size)),
                    file=out,
                )
    return mdls
