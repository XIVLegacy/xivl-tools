"""Read-only extraction from a modern client (SqPack index1, LGB, SGB, MDL).

Never writes into the install; callers write the results elsewhere. Plain
(type 2) and model (type 3, reassembled into a plain .mdl) entries are
supported. Shared groups are flattened to world matrices,
world = parent * T * R * S. Type-3 reassembly follows Physis (GPL-3.0, https://github.com/redstrate/Physis)
(attribution for translated behaviour).
"""

import json
import math
import os
import struct
import zlib

CATEGORIES = {
    "common": 0x00,
    "bgcommon": 0x01,
    "bg": 0x02,
    "cut": 0x03,
    "chara": 0x04,
    "shader": 0x05,
    "ui": 0x06,
    "sound": 0x07,
    "vfx": 0x08,
    "ui_script": 0x09,
    "exd": 0x0A,
    "game_script": 0x0B,
    "music": 0x0C,
}


class ExtractError(Exception):
    pass


def _crc(s):
    return (~zlib.crc32(s.lower().encode())) & 0xFFFFFFFF


class Pack:
    def __init__(self, sqpack, repo, prefix):
        self.base = os.path.join(sqpack, repo, prefix + ".win32")
        with open(self.base + ".index", "rb") as f:
            data = f.read()
        hsize = struct.unpack_from("<I", data, 0x0C)[0]
        off, size = struct.unpack_from("<II", data, hsize + 0x08)
        self.entries = {}
        for i in range(off, off + size, 16):
            fh, dh, val = struct.unpack_from("<III", data, i)
            self.entries[(dh, fh)] = val

    def locate(self, path):
        folder, _, name = path.lower().rpartition("/")
        return self.entries.get((_crc(folder), _crc(name)))

    def read(self, path):
        val = self.locate(path)
        if val is None:
            return None
        dat, offset = (val >> 1) & 7, (val & ~0xF) * 8
        with open("%s.dat%d" % (self.base, dat), "rb") as f:
            f.seek(offset)
            hsize, ftype, usize, _, _, nblocks = struct.unpack("<6I", f.read(0x18))
            if ftype == 3:
                return _read_type3(f, offset)
            if ftype != 2:
                raise ExtractError("%s: entry type %d not supported" % (path, ftype))
            blocks = [struct.unpack("<IHH", f.read(8)) for _ in range(nblocks)]
            out = bytearray()
            for boff, _, _ in blocks:
                f.seek(offset + hsize + boff)
                _, _, csize, bsize = struct.unpack("<4I", f.read(16))
                out += (
                    f.read(bsize)
                    if csize == 32000
                    else zlib.decompress(f.read(csize), -15)
                )
            return bytes(out[:usize])


def _read_block(f, pos):
    f.seek(pos)
    _size, _pad, x, y = struct.unpack("<4I", f.read(16))
    return zlib.decompress(f.read(x), -15) if x < 32000 else f.read(y)


def _read_type3(f, offset):
    """Reassemble a type-3 entry into a plain .mdl (Physis (GPL-3.0, https://github.com/redstrate/Physis) SqPackData::read_model_file)."""
    f.seek(offset)
    head = f.read(0xD0)
    hsize, ftype, _fsize, _nblocks, _nused, version = struct.unpack_from("<6I", head, 0)
    off = struct.unpack_from(
        "<11I", head, 112
    )  # stack, runtime, vertex[3], edge[3], index[3]
    num = struct.unpack_from("<11H", head, 178)
    vdecl_num, mat_num, num_lods, idx_stream, edge_geo = struct.unpack_from(
        "<HHBBB", head, 200
    )
    if any(num[5:8]):
        raise ExtractError("edge geometry blocks present; not supported")
    total = sum(num)
    csizes = struct.unpack("<%dH" % total, f.read(2 * total))
    base = offset + hsize
    buf = bytearray(0x44)
    cur = [0]

    def read_blocks(start, count):
        pos = base + start
        for _ in range(count):
            buf.extend(_read_block(f, pos))
            pos += csizes[cur[0]]
            cur[0] += 1

    p0 = len(buf)
    read_blocks(off[0], num[0])
    stack_size = len(buf) - p0
    p0 = len(buf)
    read_blocks(off[1], num[1])
    runtime_size = len(buf) - p0
    voffs, ioffs, vsz, isz = [0] * 3, [0] * 3, [0] * 3, [0] * 3
    for i in range(3):
        for offs, szs, k in ((voffs, vsz, 2 + i), (ioffs, isz, 8 + i)):
            if num[k]:
                here = len(buf)
                offs[i] = (
                    here if (i == 0 or here != offs[i - 1]) else 0
                )  # 0 when equal to the previous LOD
                read_blocks(off[k], num[k])
                szs[i] = len(buf) - here
    buf[0:0x44] = struct.pack(
        "<IIIHH3I3I3I3IBBBB",
        version,
        stack_size,
        runtime_size,
        vdecl_num,
        mat_num,
        *voffs,
        *ioffs,
        *vsz,
        *isz,
        num_lods,
        idx_stream,
        edge_geo,
        0,
    )
    return bytes(buf)


class Client:
    """The modern client's sqpack folder; finds the pack that holds a game path."""

    def __init__(self, sqpack):
        if not os.path.isdir(sqpack):
            raise ExtractError("no sqpack folder: " + sqpack)
        self.sqpack, self.packs = sqpack, {}

    def _packs_for(self, path):
        parts = path.lower().split("/")
        cat = CATEGORIES.get(parts[0])
        if cat is None:
            raise ExtractError("unknown category in path: " + path)
        repo = (
            parts[1]
            if len(parts) > 1 and parts[1].startswith("ex") and parts[1][2:].isdigit()
            else "ffxiv"
        )
        folder = os.path.join(self.sqpack, repo)
        for name in sorted(os.listdir(folder)):
            if name.startswith("%02x" % cat) and name.endswith(".win32.index"):
                key = (repo, name[: -len(".win32.index")])
                if key not in self.packs:
                    self.packs[key] = Pack(self.sqpack, *key)
                yield self.packs[key]

    def read(self, path):
        for pk in self._packs_for(path):
            if pk.locate(path) is not None:
                return pk.read(path)
        raise ExtractError("not in the client: " + path)


# ---- level files ---------------------------------------------------------------------------------------------


def lgb_instances(d):
    """BG instances (type 1) and shared-group placements (type 6) of a bg.lgb, in file order."""

    def cstr(o):
        return d[o : d.index(b"\0", o)].decode("latin1")

    if not (d[:4] == b"LGB1" and d[12:16] == b"LGP1"):
        raise ExtractError("not an LGB1/LGP1 file")
    base = 0x14
    _, _, layers_off, layer_count = struct.unpack_from("<4i", d, base)
    arr = base + layers_off
    out = []
    for li in range(layer_count):
        lo = arr + struct.unpack_from("<i", d, arr + 4 * li)[0]
        _lid, lname, objs_off, obj_count = struct.unpack_from("<Iiii", d, lo)
        oarr = lo + objs_off
        for oi in range(obj_count):
            oo = oarr + struct.unpack_from("<i", d, oarr + 4 * oi)[0]
            atype, iid, _noff = struct.unpack_from("<iIi", d, oo)
            t = struct.unpack_from("<9f", d, oo + 12)
            layer = cstr(lo + lname) if lname else ""
            if atype == 1:
                apath_off, cpath_off, ctype = struct.unpack_from("<iii", d, oo + 48)
                out.append(
                    {
                        "layer": layer,
                        "id": iid,
                        "mdl": cstr(oo + apath_off) if apath_off else "",
                        "pcb": cstr(oo + cpath_off) if cpath_off else "",
                        "ctype": ctype,
                        "t": t[0:3],
                        "r": t[3:6],
                        "s": t[6:9],
                    }
                )
            elif atype == 6:
                out.append(
                    {
                        "layer": layer,
                        "id": iid,
                        "sgb": cstr(oo + struct.unpack_from("<i", d, oo + 48)[0]),
                        "t": t[0:3],
                        "r": t[3:6],
                        "s": t[6:9],
                    }
                )
    return out


def sgb_layers(d):
    """Layers of a shared-group file that hold models or nested groups (found by structure)."""

    def cstr(o):
        if not (0 <= o < len(d)):
            return None
        e = d.find(b"\0", o)
        if e < 0 or e - o > 300:
            return None
        try:
            return d[o:e].decode("ascii")
        except UnicodeDecodeError:
            return None

    found, seen = [], set()
    for lo in range(0x14, len(d) - 16, 4):
        _lid, lname, objs_off, n = struct.unpack_from("<Iiii", d, lo)
        if not (0 < n <= 2000 and 0 < objs_off < len(d)):
            continue
        oarr = lo + objs_off
        if oarr + 4 * n > len(d):
            continue
        objs, ok = [], True
        for i in range(n):
            oo = oarr + struct.unpack_from("<i", d, oarr + 4 * i)[0]
            if not (0 < oo < len(d) - 60):
                ok = False
                break
            at, _iid, _noff = struct.unpack_from("<iIi", d, oo)
            if not (0 < at < 100):
                ok = False
                break
            t = struct.unpack_from("<9f", d, oo + 12)
            if any(v != v or abs(v) > 1e5 for v in t):
                ok = False
                break
            rec = {"type": at, "t": t[0:3], "r": t[3:6], "s": t[6:9]}
            if at == 1:
                ap, cp = struct.unpack_from("<ii", d, oo + 48)
                m = cstr(oo + ap)
                if not m or not m.endswith(".mdl"):
                    ok = False
                    break
                rec["mdl"] = m
                rec["pcb"] = (cstr(oo + cp) or "") if cp else ""
            elif at == 6:
                s = cstr(oo + struct.unpack_from("<i", d, oo + 48)[0])
                if not s or not s.endswith(".sgb"):
                    ok = False
                    break
                rec["sgb"] = s
            objs.append(rec)
        if ok and any("mdl" in o or "sgb" in o for o in objs):
            key = tuple(
                oarr + struct.unpack_from("<i", d, oarr + 4 * i)[0] for i in range(n)
            )
            if key in seen:
                continue
            seen.add(key)
            found.append(
                {
                    "layer": cstr(lo + lname) or "",
                    "objects": [o for o in objs if "mdl" in o or "sgb" in o],
                    "other_types": sorted(
                        {o["type"] for o in objs if "mdl" not in o and "sgb" not in o}
                    ),
                }
            )
    return found


def sgb_models(instances, read):
    """Every shared group the level places, and the groups nested in them: {sgb path: layers}.

    Order: the placed groups sorted, then each further nesting level sorted .
    read(path) returns the file bytes; a group that cannot be read stops the run.
    """
    out = {}
    level = sorted({o["sgb"] for o in instances if "sgb" in o})
    while level:
        nxt = set()
        for p in level:
            out[p] = sgb_layers(read(p))
            for layer in out[p]:
                for o in layer["objects"]:
                    if (
                        "sgb" in o
                        and o["sgb"] not in out
                        and o["sgb"] not in level
                        and not _is_vfx(o["sgb"])
                    ):
                        nxt.add(o["sgb"])
        level = sorted(nxt)
    return out


def _is_vfx(sgb):
    return sgb.split("/")[-1].startswith("sgvf_")


def _mm(A, B):
    return [
        [sum(A[i][k] * B[k][j] for k in range(4)) for j in range(4)] for i in range(4)
    ]


def _local(t, r, s, order):
    cx, sx = math.cos(r[0]), math.sin(r[0])
    cy, sy = math.cos(r[1]), math.sin(r[1])
    cz, sz = math.cos(r[2]), math.sin(r[2])
    Rx = [[1, 0, 0, 0], [0, cx, -sx, 0], [0, sx, cx, 0], [0, 0, 0, 1]]
    Ry = [[cy, 0, sy, 0], [0, 1, 0, 0], [-sy, 0, cy, 0], [0, 0, 0, 1]]
    Rz = [[cz, -sz, 0, 0], [sz, cz, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]
    R = _mm(Rx, _mm(Ry, Rz)) if order == "xyz" else _mm(Rz, _mm(Ry, Rx))
    M = [[R[i][j] * (s[j] if j < 3 else 1) for j in range(4)] for i in range(4)]
    for i in range(3):
        M[i][3] = t[i]
    return M


def _decompose(M, order):
    sc = [math.sqrt(sum(M[i][j] ** 2 for i in range(3))) for j in range(3)]
    det = (
        M[0][0] * (M[1][1] * M[2][2] - M[1][2] * M[2][1])
        - M[0][1] * (M[1][0] * M[2][2] - M[1][2] * M[2][0])
        + M[0][2] * (M[1][0] * M[2][1] - M[1][1] * M[2][0])
    )
    if det < 0:
        sc[0] = -sc[0]
    R = [[M[i][j] / sc[j] if sc[j] else 0.0 for j in range(3)] for i in range(3)]
    if order == "xyz":
        y = math.asin(max(-1.0, min(1.0, R[0][2])))
        if abs(R[0][2]) < 0.999999:
            x, z = math.atan2(-R[1][2], R[2][2]), math.atan2(-R[0][1], R[0][0])
        else:
            x, z = math.atan2(R[2][1], R[1][1]), 0.0
    else:
        y = math.asin(max(-1.0, min(1.0, -R[2][0])))
        if abs(R[2][0]) < 0.999999:
            x, z = math.atan2(R[2][1], R[2][2]), math.atan2(R[1][0], R[0][0])
        else:
            x, z = math.atan2(-R[1][2], R[1][1]), 0.0
    x2, y2, z2 = (
        x - math.copysign(math.pi, x),
        math.pi - y if y >= 0 else -math.pi - y,
        z - math.copysign(math.pi, z),
    )
    if abs(x2) + abs(z2) < abs(x) + abs(z) - 1e-9:
        x, y, z = x2, y2, z2
    return [M[0][3], M[1][3], M[2][3]], [x, y, z], sc


def placements(instances, models, order="xyz"):
    """Flatten placed shared groups into one row per model (and per VFX placeholder). Returns (rows, missing)."""
    rows, missing = [], {}

    def walk(sgb, M, chain):
        if sgb not in models:
            missing[sgb] = missing.get(sgb, 0) + 1
            return
        for layer in models[sgb]:
            for o in layer["objects"]:
                W = _mm(M, _local(o["t"], o["r"], o["s"], order))
                if "mdl" in o:
                    rows.append((o, W, chain))
                elif o["sgb"] in models:
                    walk(o["sgb"], W, chain + [o["sgb"]])
                elif _is_vfx(o["sgb"]):
                    rows.append((o, W, chain))
                else:
                    missing[o["sgb"]] = missing.get(o["sgb"], 0) + 1

    for top in (o for o in instances if "sgb" in o):
        walk(top["sgb"], _local(top["t"], top["r"], top["s"], order), [top["sgb"]])
    out = []
    for o, W, chain in rows:
        t, r, s = _decompose(W, order)
        isvfx = "mdl" not in o
        name = o["sgb"] if isvfx else o["mdl"]
        out.append(
            {
                "model": name,
                "pcb": (o.get("pcb") or None) if not isvfx else None,
                "sgb": chain[0],
                "chain": chain,
                "matrix": [round(W[i][j], 6) for i in range(4) for j in range(4)],
                "translation": [round(v, 4) for v in t],
                "rotation_euler": [round(v, 5) for v in r],
                "scale": [round(v, 5) for v in s],
                "vfx": isvfx or _is_vfx(name),
            }
        )
    return out, missing


def to_json(obj):
    """JSON with indent 0 and CRLF line ends (stable form for hash comparison)."""
    return json.dumps(obj, indent=0).replace("\n", "\r\n").encode("ascii")
