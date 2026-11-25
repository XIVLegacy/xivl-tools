r"""Bake ambient occlusion into COLOR_0 of a GLB without touching its geometry (1.0 tiles carry baked grey vertex
shading in the native COLOR field; ours were constant white).

  blender -b -P bake_ao_glb.py -- <in.glb> <out.glb> [--occ <glb>:dx,dy,dz ...] [--rays 48] [--dist 10]
                                   [--floor 0.25] [--gamma 1.0] [--json stats.json]

Reads the GLB directly (no Blender import/export: accessors, order, names and materials stay byte-identical), builds
one BVH from its triangles plus the --occ files (shifted by dx,dy,dz, glTF axes), and per vertex casts cosine-weighted
rays from position + 2 cm along the normal. open = 1 - mean(hit weight), weight = 1 - d/dist. Grey = floor +
(1 - floor) * open ** gamma, written as a new float VEC4 COLOR_0 accessor (RGB grey, A 1) appended to the buffer.
Needs Blender only for mathutils.bvhtree. Node transforms must be absent (asserted). Refuses to overwrite."""

import json
import math
import os
import struct
import sys
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

a = sys.argv[sys.argv.index("--") + 1 :]


def opt(n, d):
    return type(d)(a[a.index(n) + 1]) if n in a else d


src, dst = a[0], a[1]
assert not os.path.exists(dst), "refusing to overwrite " + dst
RAYS, DIST, FLOOR, GAMMA = (
    opt("--rays", 48),
    opt("--dist", 10.0),
    opt("--floor", 0.25),
    opt("--gamma", 1.0),
)
EDGEREF = opt(
    "--edge-ref", 0.0
)  # metres; 0 = off. Darkening is scaled by min(1, EDGEREF / longest adjacent edge)
occ = [a[i + 1] for i, x in enumerate(a) if x == "--occ"]


def load(p):
    d = open(p, "rb").read()
    (n,) = struct.unpack_from("<I", d, 12)
    return json.loads(d[20 : 20 + n]), bytearray(d[20 + n + 8 :])


def acc(g, b, i):
    x = g["accessors"][i]
    v = g["bufferViews"][x["bufferView"]]
    o = v.get("byteOffset", 0) + x.get("byteOffset", 0)
    k = {"VEC3": 3, "SCALAR": 1, "VEC2": 2, "VEC4": 4}[x["type"]]
    dt = {5126: "<f4", 5123: "<u2", 5125: "<u4"}[x["componentType"]]
    assert not v.get("byteStride") or v["byteStride"] == np.dtype(dt).itemsize * k
    return np.frombuffer(
        bytes(b[o : o + x["count"] * k * np.dtype(dt).itemsize]), dt
    ).reshape(-1, k)


def tris(g, b, shift=(0, 0, 0)):
    V, F, base = [], [], 0
    for nd in g["nodes"]:
        assert not any(
            k in nd for k in ("translation", "rotation", "scale", "matrix")
        ), "node transform in " + nd.get("name", "?")
    for m in g["meshes"]:
        for pr in m["primitives"]:
            P = acc(g, b, pr["attributes"]["POSITION"]).astype(float) + np.array(shift)
            tri_idx = acc(g, b, pr["indices"]).reshape(-1, 3).astype(int)
            V.append(P)
            F.append(tri_idx + base)
            base += len(P)
    return np.vstack(V), np.vstack(F)


g, b = load(src)
V, F = tris(g, b)
allV, allF = [V], [F]
base = len(V)
for o in occ:
    p, sh = o.rsplit(":", 1)
    og, ob = load(p)
    OV, OF = tris(og, ob, tuple(float(x) for x in sh.split(",")))
    allV.append(OV)
    allF.append(OF + base)
    base += len(OV)
AV, AF = np.vstack(allV), np.vstack(allF)
bvh = BVHTree.FromPolygons(
    [Vector(v) for v in AV], [tuple(int(i) for i in f) for f in AF], all_triangles=True
)
print(
    "AO bvh tris", len(AF), "own tris", len(F), "occluder files", len(occ), flush=True
)
rng = np.random.default_rng(7)
u1, u2 = rng.random(RAYS), rng.random(RAYS)
r, phi = np.sqrt(u1), 2 * math.pi * u2
HEMI = np.stack(
    [r * np.cos(phi), r * np.sin(phi), np.sqrt(1 - u1)], 1
)  # cosine-weighted, +Z up
allopen = []
newacc = {}
for mi, m in enumerate(g["meshes"]):
    for pi, pr in enumerate(m["primitives"]):
        assert "COLOR_0" not in pr["attributes"]
        P = acc(g, b, pr["attributes"]["POSITION"]).astype(float)
        N = acc(g, b, pr["attributes"]["NORMAL"]).astype(float)
        op = np.ones(len(P))
        for k in range(len(P)):
            n = N[k]
            ln = np.linalg.norm(n)
            if ln < 1e-6:
                continue
            n = n / ln
            t = np.cross(n, (0.0, 1.0, 0.0) if abs(n[1]) < 0.9 else (1.0, 0.0, 0.0))
            t /= np.linalg.norm(t)
            bt = np.cross(n, t)
            D = HEMI[:, 0:1] * t + HEMI[:, 1:2] * bt + HEMI[:, 2:3] * n
            o = Vector(P[k] + n * 0.02)
            occl = 0.0
            for d in D:
                hit = bvh.ray_cast(o, Vector(d), DIST)
                if hit[0] is not None:
                    occl += 1.0 - hit[3] / DIST
            op[k] = 1.0 - occl / RAYS
        grey = FLOOR + (1 - FLOOR) * np.clip(op, 0, 1) ** GAMMA
        if EDGEREF:  # a vertex of a long triangle smears its value over metres of open floor: darken it less
            tri_idx = acc(g, b, pr["indices"]).reshape(-1, 3).astype(int)
            T = P[tri_idx]
            el = np.maximum.reduce(
                [np.linalg.norm(T[:, i] - T[:, (i + 1) % 3], axis=1) for i in range(3)]
            )
            longest = np.zeros(len(P))
            np.maximum.at(longest, tri_idx.ravel(), np.repeat(el, 3))
            grey = 1 - (1 - grey) * np.clip(EDGEREF / np.maximum(longest, 1e-6), 0, 1)
        allopen.append(grey)
        col = (
            np.concatenate([np.repeat(grey[:, None], 3, 1), np.ones((len(P), 1))], 1)
            .astype("<f4")
            .tobytes()
        )
        while len(b) % 4:
            b.append(0)
        g["bufferViews"].append(
            {"buffer": 0, "byteOffset": len(b), "byteLength": len(col), "target": 34962}
        )
        b += col
        g["accessors"].append(
            {
                "bufferView": len(g["bufferViews"]) - 1,
                "componentType": 5126,
                "count": len(P),
                "type": "VEC4",
            }
        )
        pr["attributes"]["COLOR_0"] = len(g["accessors"]) - 1
while len(b) % 4:
    b.append(0)
g["buffers"][0]["byteLength"] = len(b)
js = json.dumps(g, separators=(",", ":")).encode()
js += b" " * (-len(js) % 4)
out = (
    b"glTF"
    + struct.pack("<II", 2, 12 + 8 + len(js) + 8 + len(b))
    + struct.pack("<I", len(js))
    + b"JSON"
    + js
    + struct.pack("<I", len(b))
    + b"BIN\0"
    + bytes(b)
)
open(dst, "wb").write(out)
G = np.concatenate(allopen) * 255
st = dict(
    verts=int(len(G)),
    mean=float(G.mean()),
    under200=float((G < 200).mean() * 100),
    min=float(G.min()),
    max=float(G.max()),
    quantiles={q: float(np.quantile(G, q / 100)) for q in (5, 25, 50, 75, 95)},
    rays=RAYS,
    dist=DIST,
    floor=FLOOR,
    gamma=GAMMA,
)
print("AO", json.dumps(st))
print("AO wrote", dst, len(out))
if "--json" in a:
    json.dump(st, open(opt("--json", ""), "w"), indent=1)
