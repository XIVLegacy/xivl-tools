"""City tiles: cut the modern scene into the grid (Blender), fill ground holes from the terrain plates, cut each
tile into pieces (Blender), bake shading (Blender)."""

import glob
import json
import os
import re
import shutil
import struct
from collections import Counter

import numpy as np

from . import tools

SHADE = dict(rays=64, dist=6.0, floor=0.35, gamma=0.8, edge_ref=3.0)


def neighbours(z, n, folder):
    """--occ arguments for the up to 8 grid neighbours of tile n that exist as tNN.glb in folder, in tile order,
    shifted by whole cells (glTF axes)."""
    g = z["tile_grid"]
    nx = g["nx"]
    have = sorted(
        int(re.search(r"t(\d\d)\.glb$", p).group(1))
        for p in glob.glob(os.path.join(folder, "t??.glb"))
    )
    occ = []
    for m in have:
        dc, dr = m % nx - n % nx, m // nx - n // nx
        if m != n and abs(dc) <= 1 and abs(dr) <= 1:
            occ += [
                "--occ",
                f"{os.path.join(folder, f't{m:02d}.glb')}:{dc * g['width'] / nx},0,{dr * g['depth'] / g['nz']}",
            ]
    return occ


def shade(src, dst, occ, rays, dist, floor, gamma, edge_ref, stats_json):
    """Bake ambient occlusion into COLOR_0 (geometry untouched). Returns the stats dict."""
    if os.path.exists(dst):
        raise SystemExit("exists, not overwriting: " + dst)
    tools.run(
        [
            tools.setting("blender"),
            "-b",
            "-P",
            tools.blender_script("bake_ao.py"),
            "--",
            src,
            dst,
            *occ,
            "--rays",
            str(rays),
            "--dist",
            str(dist),
            "--floor",
            str(floor),
            "--gamma",
            str(gamma),
            "--edge-ref",
            str(edge_ref),
            "--json",
            stats_json,
        ]
    )
    if not os.path.exists(dst):
        raise SystemExit("bake wrote nothing: " + dst)
    with open(stats_json) as f:
        return json.load(f)


def empty_dir(path):
    """Output folders must be new or empty (nothing is overwritten)."""
    if os.path.isdir(path) and os.listdir(path):
        raise SystemExit("output folder is not empty: " + path)
    os.makedirs(path, exist_ok=True)


def grid(z):
    g = z["tile_grid"]
    return (
        g["x0"],
        g["z0"],
        g["width"] / g["nx"],
        g["depth"] / g["nz"],
        g["nx"],
        g["nz"],
    )


# ---- build: FBX -> one GLB per tile -----------------------------------------------


def build(fbx, out, z, cover, fix_ground):
    empty_dir(out)
    g, t = z["tile_grid"], z["tiles"]
    params = dict(
        cover=cover,
        fix_ground=fix_ground,
        skip=t["skip"],
        bgs=t["bgs"],
        ground=t["ground"],
        shell=t["shell"],
        ys=z["frames"]["source_y_shift"],
        x0=g["x0"],
        x1=g["x0"] + g["width"],
        nx=g["nx"],
        z0=g["z0"],
        z1=g["z0"] + g["depth"],
        nz=g["nz"],
        inactive=g["inactive"],
    )
    pfile = os.path.join(out, "params.json")
    with open(pfile, "w", encoding="utf-8") as f:
        json.dump(params, f, indent=1)
    log = tools.run(
        [
            tools.setting("blender"),
            "-b",
            "-P",
            tools.blender_script("tiles_build.py"),
            "--",
            fbx,
            out,
            "--params",
            pfile,
        ]
    )
    with open(os.path.join(out, "build.log"), "w", encoding="utf-8") as f:
        f.write(log)
    return [lp for lp in log.splitlines() if "DONE" in lp or "TILE" in lp]


# ---- terrain: fill ground holes from the terrain plates --------------------------------

CT = {5121: np.uint8, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
NC = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def read_glb(path):
    with open(path, "rb") as f:
        b = f.read()
    jl = struct.unpack_from("<I", b, 12)[0]
    js = json.loads(b[20 : 20 + jl])
    binb = b[20 + jl + 8 : 20 + jl + 8 + struct.unpack_from("<I", b, 20 + jl)[0]]
    return js, bytearray(binb)


def write_glb(path, js, binb):
    while len(binb) % 4:
        binb += b"\0"
    js["buffers"] = [{"byteLength": len(binb)}]
    j = json.dumps(js, separators=(",", ":")).encode()
    j += b" " * (-len(j) % 4)
    out = (
        struct.pack("<3I", 0x46546C67, 2, 28 + len(j) + len(binb))
        + struct.pack("<2I", len(j), 0x4E4F534A)
        + j
        + struct.pack("<2I", len(binb), 0x004E4942)
        + bytes(binb)
    )
    with open(path, "xb") as f:
        f.write(out)


def _centre(z, n):
    X0, Z0, SX, SZ, NX, _ = grid(z)
    return X0 + (n % NX + 0.5) * SX, Z0 + (n // NX + 0.5) * SZ


def tile_tris(js, binb, cx, cz):
    """per triangle: positions (layout frame), uv0, material name"""
    A, BV, M = js["accessors"], js["bufferViews"], js.get("materials", [])

    def arr(i):
        a = A[i]
        bv = BV[a["bufferView"]]
        off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        return np.frombuffer(
            bytes(binb), CT[a["componentType"]], a["count"] * NC[a["type"]], off
        ).reshape(a["count"], -1)

    P, U, N = [], [], []
    for m in js["meshes"]:
        for p in m["primitives"]:
            tri_idx = arr(p["indices"]).reshape(-1, 3)
            P.append(
                (arr(p["attributes"]["POSITION"]).astype(float) + [cx, 0, cz])[tri_idx]
            )
            t = p["attributes"].get("TEXCOORD_0")
            U.append(
                arr(t).astype(float)[tri_idx]
                if t is not None
                else np.zeros((len(tri_idx), 3, 2))
            )
            N += [M[p["material"]]["name"] if "material" in p else m["name"]] * len(
                tri_idx
            )
    return np.concatenate(P), np.concatenate(U), np.array(N)


def ray_cover(T, pts, cover):
    """for each point (x, y, z): is there a triangle of T under/over it within cover m vertically? (XZ-binned)"""
    out = np.zeros(len(pts), bool)
    cell = 4.0
    mn = np.floor(T[:, :, [0, 2]].min(1) / cell).astype(int)
    mx = np.floor(T[:, :, [0, 2]].max(1) / cell).astype(int)
    grd = {}
    for i in range(len(T)):
        for gx in range(mn[i, 0], mx[i, 0] + 1):
            for gz in range(mn[i, 1], mx[i, 1] + 1):
                grd.setdefault((gx, gz), []).append(i)
    keys = np.floor(pts[:, [0, 2]] / cell).astype(int)
    order = np.lexsort((keys[:, 1], keys[:, 0]))
    ks = keys[order]
    brk = np.nonzero(np.any(np.diff(ks, axis=0) != 0, axis=1))[0] + 1
    for grp in np.split(order, brk):
        ids = grd.get(tuple(keys[grp[0]]))
        if not ids:
            continue
        t = T[ids]
        a, b, c = t[:, 0][None], t[:, 1][None], t[:, 2][None]
        x, y, zz = pts[grp, 0][:, None], pts[grp, 1][:, None], pts[grp, 2][:, None]
        v0x, v0z, v1x, v1z = (
            c[..., 0] - a[..., 0],
            c[..., 2] - a[..., 2],
            b[..., 0] - a[..., 0],
            b[..., 2] - a[..., 2],
        )
        v2x, v2z = x - a[..., 0], zz - a[..., 2]
        den = v0x * v1z - v1x * v0z
        ok = np.abs(den) > 1e-12
        d = np.where(ok, den, 1)
        u = (v2x * v1z - v1x * v2z) / d
        v = (v0x * v2z - v2x * v0z) / d
        ins = ok & (u >= -1e-6) & (v >= -1e-6) & (u + v <= 1 + 1e-6)
        h = a[..., 1] + u * (c[..., 1] - a[..., 1]) + v * (b[..., 1] - a[..., 1])
        out[grp] = (ins & (np.abs(h - y) <= cover)).any(1)
    return out


def samples(C, step):
    """interior sample points of each triangle on a barycentric grid of about step m; returns (points, owner index)"""
    P, owner = [], []
    for i, (a, b, c) in enumerate(C):
        n = max(
            2,
            int(
                np.ceil(
                    max(
                        np.linalg.norm(b - a),
                        np.linalg.norm(c - a),
                        np.linalg.norm(c - b),
                    )
                    / step
                )
            ),
        )
        ij = (
            np.array([(p, q) for p in range(n + 1) for q in range(n + 1 - p)], float)
            / n
        )
        ij = (
            0.02 + 0.94 * ij
        )  # pull in from the edges so neighbours' shared edges do not count as cover
        P.append(a + ij[:, :1] * (b - a) + ij[:, 1:] * (c - a))
        owner.append(np.full(len(ij), i))
    return np.concatenate(P), np.concatenate(owner)


def tp_normals(T, add):
    """corner normals for the added triangles: face normal, pointed up (ground)"""
    f = np.cross(T[add, 1] - T[add, 0], T[add, 2] - T[add, 0])
    f /= np.maximum(np.linalg.norm(f, axis=1, keepdims=True), 1e-12)
    f[f[:, 1] < 0] *= -1
    return np.repeat(f[:, None], 3, axis=1)


def terrain(
    src,
    dst,
    z,
    plates,
    cover=0.3,
    check=False,
    sgb=None,
    placements=None,
    model_of=None,
):
    """plates = (T, U, M) in the source frame (collision_build.terrain_plates_full); sgb = compiled regex of SGB prop
    models to add whole from placements (extract props output); model_of(path) -> decoded modern model."""
    X0, Z0, SX, SZ, NX, NZ = grid(z)
    YS = z["frames"]["source_y_shift"]
    wo = z["frames"]["world_offset"]
    tiles = {
        int(re.search(r"(\d+)\.glb$", f).group(1)): os.path.join(src, f)
        for f in os.listdir(src)
        if f.endswith(".glb")
    }
    data = {n: read_glb(p) for n, p in tiles.items()}
    parts = [tile_tris(js, b, *_centre(z, n)) for n, (js, b) in data.items()]
    TT = np.concatenate([p[0] for p in parts])
    TU = np.concatenate([p[1] for p in parts])
    T, U, M = plates
    T = T - [0, YS, 0]
    M = np.array([re.sub(r"\.mtrl$", "", os.path.basename(m)) for m in M])
    q = 0.05
    tk = {}
    for i, k in enumerate(map(tuple, np.round(TT.mean(1) / q).astype(np.int64))):
        tk.setdefault(k, i)
    match = np.full(len(T), -1)
    for i, k in enumerate(map(tuple, np.round(T.mean(1) / q).astype(np.int64))):
        for d in ((0, 0, 0),) + tuple(
            (dx, dy, dz) for dx in (-1, 0, 1) for dy in (-1, 0, 1) for dz in (-1, 0, 1)
        ):
            j = tk.get((k[0] + d[0], k[1] + d[1], k[2] + d[2]))
            # same triangle = every corner within 5 cm of a corner of the tile triangle
            if (
                j is not None
                and np.linalg.norm(T[i][:, None] - TT[j][None], axis=2).min(1).max() < q
            ):
                match[i] = j
                break
    have = match >= 0

    def fn(X):
        return np.cross(X[:, 1] - X[:, 0], X[:, 2] - X[:, 0])

    same_wind = (np.einsum("ij,ij->i", fn(T[have]), fn(TT[match[have]])) > 0).mean()
    pu, tu = [], []
    for i in np.nonzero(have)[0][:2000]:
        j = match[i]
        for v in range(3):
            w = np.argmin(np.linalg.norm(TT[j] - T[i, v], axis=1))
            pu.append(U[i, v])
            tu.append(TU[j, w])
    pu, tu = np.array(pu), np.array(tu)
    du = np.median(tu[:, 0] - pu[:, 0])
    dv_same = np.median(np.abs(tu[:, 1] - pu[:, 1]))
    dv_flip = np.median(np.abs(tu[:, 1] - (1 - pu[:, 1])))
    print(
        f"plate tris {len(T)}; identical in tiles {have.sum()}; winding kept {same_wind:.3f}; "
        f"U offset {du:.4f}; V |same| {dv_same:.4f} |1-v| {dv_flip:.4f}"
    )
    flip_v = dv_flip < dv_same
    cand = np.nonzero(~have)[0]
    S, owner = samples(T[cand], 0.5)
    cov = ray_cover(TT, S, cover)
    unc = np.bincount(owner[~cov], minlength=len(cand))
    add = cand[unc > 0]
    print(f"samples {len(S)}, uncovered {int((~cov).sum())}")
    if (
        sgb is not None
    ):  # SGB props appended after the terrain rows; they skip the coverage test
        PT, PU, PM, PN = [T], [U], [M], []
        for pl in placements:
            if not pl.get("model") or not sgb.search(pl["model"]):
                continue
            mx = np.array(pl["matrix"], float).reshape(4, 4)
            R, t = mx[:3, :3], mx[:3, 3]
            m = model_of(pl["model"])
            for me in m.meshes:
                if not me["positions"] or len(me["indices"]) < 3:
                    continue
                P = np.array(me["positions"], float) @ R.T + t - [0, YS, 0]
                tri_idx = np.array(
                    me["indices"][: len(me["indices"]) // 3 * 3]
                ).reshape(-1, 3)
                PT.append(P[tri_idx])
                PM.append(
                    np.array(
                        [re.sub(r"\.mtrl$", "", os.path.basename(me["material"]))]
                        * len(tri_idx)
                    )
                )
                uv = np.array(me["uvs"], float) if me["uvs"] else np.zeros((len(P), 2))
                PU.append(uv[tri_idx])
                nr = np.array(me["normals"], float) @ R.T if me["normals"] else None
                PN.append(nr[tri_idx] if nr is not None else None)
            print(
                "SGB",
                pl["model"].split("/")[-1],
                "at in-game",
                (t - [0, YS, 0] + wo).round(2).tolist(),
            )
        n0 = len(T)
        T, U, M = np.concatenate(PT), np.concatenate(PU), np.concatenate(PM)
        add = np.concatenate([add, np.arange(n0, len(T))])
        assert all(x is not None for x in PN), "SGB model without normals"
        sgb_n0, sgb_N = n0, np.concatenate(PN) if PN else None
    else:
        sgb_n0, sgb_N = len(T), None
    area = 0.5 * np.linalg.norm(fn(T[add]), axis=1)
    print(
        f"missing {len(cand)}; added (some sample uncovered within {cover} m) {len(add)}, {area.sum():.0f} m2"
    )
    print("added by material:", Counter(M[add].tolist()).most_common())
    if check:
        return
    empty_dir(dst)
    cen = T[add].mean(1)
    cxi = np.clip(np.floor((cen[:, 0] - X0) / SX), 0, NX - 1).astype(int)
    czi = np.clip(np.floor((cen[:, 2] - Z0) / SZ), 0, NZ - 1).astype(int)
    tile_of = czi * NX + cxi
    act = sorted(tiles)
    for k in np.nonzero(~np.isin(tile_of, act))[0]:
        cc = np.array([_centre(z, n) for n in act])
        tile_of[k] = act[int(np.argmin(((cc - cen[k, [0, 2]]) ** 2).sum(1)))]
    nrm_all = tp_normals(T, add)
    if (
        sgb_N is not None
    ):  # SGB props keep their own corner normals (rotated with the placement)
        k = add >= sgb_n0
        nn = sgb_N[add[k] - sgb_n0]
        nrm_all[k] = nn / np.maximum(np.linalg.norm(nn, axis=2, keepdims=True), 1e-12)
    manifest = {
        "src": src,
        "cover": cover,
        "winding_kept": float(same_wind),
        "flip_v": bool(flip_v),
        "tiles": {},
    }
    for n, path in tiles.items():
        out = os.path.join(dst, os.path.basename(path))
        sel = add[tile_of == n]
        if not len(sel):
            shutil.copyfile(path, out)
            continue
        js, binb = data[n]
        cx, cz = _centre(z, n)
        mats = {m["name"]: i for i, m in enumerate(js.setdefault("materials", []))}
        per = {}
        for mname in sorted(set(M[sel].tolist())):
            s = sel[M[sel] == mname]
            P = T[s] - [cx, 0, cz]
            UV = U[s].copy()
            NN = nrm_all[np.searchsorted(add, s)]
            if same_wind < 0.5:
                P, UV, NN = P[:, ::-1], UV[:, ::-1], NN[:, ::-1]
            if flip_v:
                UV[:, :, 1] = 1 - UV[:, :, 1]
            UV[:, :, 0] += du
            P = P.reshape(-1, 3).astype(np.float32)
            UV = UV.reshape(-1, 2).astype(np.float32)
            NN = NN.reshape(-1, 3).astype(np.float32)
            idx = np.arange(len(P), dtype=np.uint32)

            def view(arr, target):
                while len(binb) % 4:
                    binb.extend(b"\0")
                off = len(binb)
                binb.extend(arr.tobytes())
                js["bufferViews"].append(
                    {
                        "buffer": 0,
                        "byteOffset": off,
                        "byteLength": arr.nbytes,
                        "target": target,
                    }
                )
                return len(js["bufferViews"]) - 1

            acc = js["accessors"]
            acc.append(
                {
                    "bufferView": view(P, 34962),
                    "componentType": 5126,
                    "count": len(P),
                    "type": "VEC3",
                    "min": P.min(0).tolist(),
                    "max": P.max(0).tolist(),
                }
            )
            ip = len(acc) - 1
            acc.append(
                {
                    "bufferView": view(NN, 34962),
                    "componentType": 5126,
                    "count": len(NN),
                    "type": "VEC3",
                }
            )
            inn = len(acc) - 1
            acc.append(
                {
                    "bufferView": view(UV, 34962),
                    "componentType": 5126,
                    "count": len(UV),
                    "type": "VEC2",
                }
            )
            iu = len(acc) - 1
            acc.append(
                {
                    "bufferView": view(idx, 34963),
                    "componentType": 5125,
                    "count": len(idx),
                    "type": "SCALAR",
                }
            )
            ii = len(acc) - 1
            if mname not in mats:
                js["materials"].append({"name": mname})
                mats[mname] = len(js["materials"]) - 1
            js["meshes"].append(
                {
                    "name": mname + "_terrain",
                    "primitives": [
                        {
                            "attributes": {
                                "POSITION": ip,
                                "NORMAL": inn,
                                "TEXCOORD_0": iu,
                            },
                            "indices": ii,
                            "material": mats[mname],
                        }
                    ],
                }
            )
            js["nodes"].append(
                {"name": mname + "_terrain", "mesh": len(js["meshes"]) - 1}
            )
            js["scenes"][js.get("scene", 0)]["nodes"].append(len(js["nodes"]) - 1)
            per[mname] = len(s)
        write_glb(out, js, binb)
        manifest["tiles"][n] = per
    with open(os.path.join(dst, "terrain-added.json"), "w") as f:
        json.dump(manifest, f, indent=1)
    print("wrote", dst)


# ---- cut: one tile -> pieces with tight boxes --------------------------------------


def cut(src, out, tag, z, args):
    """args: the build_multi_tile options (zone tiles.cut_args when empty). Writes <out>/<tag>.glb + manifest.json."""
    empty_dir(out)
    X0, Z0, SX, SZ, NX, NZ = grid(z)
    log = tools.run(
        [
            tools.setting("blender"),
            "-b",
            "-P",
            tools.blender_script("tiles_cut.py"),
            "--",
            src,
            out,
            "--tag",
            tag,
            "--cell-w",
            repr(SX),
            "--cell-d",
            repr(SZ),
            *args,
        ]
    )
    with open(os.path.join(out, tag + ".log"), "w", encoding="utf-8") as f:
        f.write(log)
    if "Traceback" in log or not os.path.exists(os.path.join(out, tag + ".glb")):
        raise SystemExit("cut failed, see " + os.path.join(out, tag + ".log"))
    return [lp for lp in log.splitlines()[-3:]]
