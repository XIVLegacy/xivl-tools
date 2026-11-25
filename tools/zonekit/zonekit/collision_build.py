"""Collision geometry: clean-up, edge walls, floor-hole patch, prop collision, merge.

Frames: parts are in layout space; a collision file is in the frame of its
collision tile (layout minus that tile's cell centre, zone `collision.tile`).
Modern source data (terrain plates, object collision) sits `source_y_shift`
above layout space.
"""

import json
import math
import struct
from collections import defaultdict

import numpy as np


def cell_centre(z, n):
    g = z["tile_grid"]
    sx, sz = g["width"] / g["nx"], g["depth"] / g["nz"]
    return g["x0"] + (n % g["nx"] + 0.5) * sx, g["z0"] + (n // g["nx"] + 0.5) * sz


# ---- fix: clean-up + edge walls (fix_single.py) ----------------------------------------------------------------


def fix(
    g,
    keep_parts,
    z,
    target,
    sliver=0.2,
    weld=0.3,
    drop=0.6,
    height=2.0,
    seg=1.5,
    maxt=65000,
    walk_ny=0.7,
    tol=0.15,
    bridge=(),
    btol=0.3,
    spawn=None,
    cap_reach=0.6,
    prune=None,
):
    """g: packed collision (layout frame); keep_parts: geometry dicts whose walls are real. Returns (out dict, stats).

    1. generated ledge walls (near-vertical tris not in any keep part) are removed; 2. near-vertical tris shorter than
    sliver are removed; 3. walkable boundary vertices within weld (XZ) and 0.35 m (Y) are merged to their mean;
    4. new double-sided edge walls on every walkable boundary edge reachable from spawn, unless ground lies outward
    within drop + 0.5*dist, capped 5 cm under any walkable surface above. Output in the target tile's local frame.
    """
    SLIVER, WELD, DROP, H, SEG, MAX, WALK, TOL = (
        sliver,
        weld,
        drop,
        height,
        seg,
        maxt,
        walk_ny,
        tol,
    )
    tx, tz = cell_centre(z, target)

    def nrm(a, b, c):
        u = [b[k] - a[k] for k in range(3)]
        v = [c[k] - a[k] for k in range(3)]
        n = (
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        )
        L = math.sqrt(sum(q * q for q in n)) or 1
        return [q / L for q in n]

    def K(p):
        return tuple(round(c, 2) for c in p)

    V = [list(v) for v in g["vertices"]]
    T = [list(t["indices"]) for t in g["triangles"]]
    kept = set()
    for kg in keep_parts:
        KV = kg["vertices"]
        for t in kg["triangles"]:
            kept.add(frozenset(K(KV[i]) for i in t["indices"]))

    stats = defaultdict(int)
    T2 = []
    for t in T:
        p = [V[i] for i in t]
        if len({K(q) for q in p}) < 3:
            stats["degenerate"] += 1
            continue
        ny = abs(nrm(*p)[1])
        if ny < 0.3 and frozenset(K(q) for q in p) not in kept:
            stats["old walls"] += 1
            continue
        if ny < 0.7 and max(q[1] for q in p) - min(q[1] for q in p) < SLIVER:
            stats["slivers"] += 1
            continue
        T2.append(t)
    T = T2

    def walk_tris():
        return [t for t in T if nrm(*[V[i] for i in t])[1] > 0.7]

    ecount = defaultdict(int)
    for t in walk_tris():
        for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0])):
            ecount[frozenset((K(V[a]), K(V[b])))] += 1
    bverts = set()
    for t in walk_tris():
        for a, b in ((t[0], t[1]), (t[1], t[2]), (t[2], t[0])):
            if ecount[frozenset((K(V[a]), K(V[b])))] == 1:
                bverts.update((a, b))
    bverts = sorted(bverts)
    parent = {i: i for i in bverts}

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    cell = defaultdict(list)
    for i in bverts:
        cell[(int(V[i][0] // WELD), int(V[i][2] // WELD))].append(i)
    for i in bverts:
        cx, cz = int(V[i][0] // WELD), int(V[i][2] // WELD)
        for dx in (-1, 0, 1):
            for dz in (-1, 0, 1):
                for j in cell.get((cx + dx, cz + dz), ()):
                    if j <= i:
                        continue
                    if (
                        math.hypot(V[i][0] - V[j][0], V[i][2] - V[j][2]) <= WELD
                        and abs(V[i][1] - V[j][1]) <= 0.35
                    ):
                        a, b = find(i), find(j)
                        if a != b:
                            parent[b] = a
    groups = defaultdict(list)
    for i in bverts:
        groups[find(i)].append(i)
    for gi in groups.values():
        if len(gi) < 2:
            continue
        pts = {K(V[i]): V[i] for i in gi}
        if len(pts) < 2:
            continue
        m = [sum(p[k] for p in pts.values()) / len(pts) for k in range(3)]
        for i in gi:
            V[i] = list(m)
        stats["welded groups"] += 1
    T = [
        t
        for t in T
        if len({K(V[i]) for i in t}) == 3
        and abs(sum(q * q for q in nrm(*[V[i] for i in t]))) > 0.5
    ]

    def is_sliver(t):
        p = [V[i] for i in t]
        return (
            abs(nrm(*p)[1]) < 0.7
            and max(q[1] for q in p) - min(q[1] for q in p) < SLIVER
        )

    n0 = len(T)
    T = [t for t in T if not is_sliver(t)]
    stats["slivers after weld"] = n0 - len(T)

    C = 2.0
    grid = defaultdict(list)
    walk = []
    for t in T:
        a, b, c = (V[i] for i in t)
        if nrm(a, b, c)[1] <= WALK:
            continue
        walk.append((a, b, c))
        xs = [a[0], b[0], c[0]]
        zs = [a[2], b[2], c[2]]
        for i in range(int(min(xs) // C), int(max(xs) // C) + 1):
            for k in range(int(min(zs) // C), int(max(zs) // C) + 1):
                grid[(i, k)].append(len(walk) - 1)

    def heights_id(x, zz):
        hs = []
        for ti in grid.get((int(x // C), int(zz // C)), ()):
            a, b, c = walk[ti]
            d = (b[0] - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (b[2] - a[2])
            if abs(d) < 1e-12:
                continue
            u = ((x - a[0]) * (c[2] - a[2]) - (c[0] - a[0]) * (zz - a[2])) / d
            v = ((b[0] - a[0]) * (zz - a[2]) - (x - a[0]) * (b[2] - a[2])) / d
            if u >= -1e-4 and v >= -1e-4 and u + v <= 1 + 1e-4:
                hs.append((a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]), ti))
        return hs

    def heights(x, zz):
        return [h for h, _ in heights_id(x, zz)]

    up = list(range(len(walk)))

    def root(i):
        while up[i] != i:
            up[i] = up[up[i]]
            i = up[i]
        return i

    def join(i, j):
        a, b = root(i), root(j)
        if a != b:
            up[b] = a

    first = {}
    for ti, tri in enumerate(walk):
        for p in tri:
            k = K(p)
            if k in first:
                join(ti, first[k])
            else:
                first[k] = ti
    ec = defaultdict(int)
    for a, b, c in walk:
        for p, q in ((a, b), (b, c), (c, a)):
            ec[frozenset((K(p), K(q)))] += 1
    walls = []
    BRIDGE = list(bridge)
    BTOL = btol
    bridges = []
    for ti, (a, b, c) in enumerate(walk):
        gx, gz = (a[0] + b[0] + c[0]) / 3, (a[2] + b[2] + c[2]) / 3
        for p, q in ((a, b), (b, c), (c, a)):
            if ec[frozenset((K(p), K(q)))] != 1:
                continue
            ex, ez = q[0] - p[0], q[2] - p[2]
            L = math.hypot(ex, ez)
            if L < 0.05:
                continue
            nx, nz = -ez / L, ex / L
            if (gx - (p[0] + q[0]) / 2) * nx + (gz - (p[2] + q[2]) / 2) * nz > 0:
                nx, nz = -nx, -nz
            n = max(1, math.ceil(L / SEG))
            run = None
            for s in range(n):
                f0, f1 = s / n, (s + 1) / n
                need = False
                bd = []
                for f in (f0 + 0.15 * (f1 - f0), (f0 + f1) / 2, f0 + 0.85 * (f1 - f0)):
                    y = p[1] + f * (q[1] - p[1])
                    found = False
                    for dist in (0.35, 0.7, 1.2):
                        x = p[0] + f * ex + nx * dist
                        zz = p[2] + f * ez + nz * dist
                        hit = [
                            tj
                            for h, tj in heights_id(x, zz)
                            if abs(h - y) <= DROP + 0.5 * dist
                        ]
                        if hit:
                            found = True
                            for tj in hit:
                                join(ti, tj)
                            break
                    if not found:
                        for dist in BRIDGE:
                            x = p[0] + f * ex + nx * dist
                            zz = p[2] + f * ez + nz * dist
                            hit = [
                                (abs(h - y), h, tj)
                                for h, tj in heights_id(x, zz)
                                if abs(h - y) <= BTOL
                            ]
                            if hit:
                                found = True
                                bd.append((dist, min(hit)[1] - y))
                                for _, _, tj in hit:
                                    join(ti, tj)
                                break
                    if not found:
                        need = True
                        break
                if not need and bd:
                    bridges.append(
                        (
                            p,
                            q,
                            f0,
                            f1,
                            nx,
                            nz,
                            max(d for d, _ in bd),
                            sum(dy for _, dy in bd) / len(bd),
                            ti,
                        )
                    )
                if need:
                    run = (run[0], f1) if run else (f0, f1)
                if (not need or s == n - 1) and run:
                    walls.append((p, q, run[0], run[1], ti))
                    run = None
    SPAWN = list(spawn)
    cand = [(h, tj) for h, tj in heights_id(SPAWN[0], SPAWN[2]) if h <= SPAWN[1] + 0.6]
    if not cand:
        raise SystemExit("no floor at spawn")
    reach = root(max(cand)[1])
    stats["walkable tris"] = len(walk)
    stats["reachable tris"] = sum(1 for i in range(len(walk)) if root(i) == reach)
    stats["wall candidates"] = len(walls)
    walls = [(p, q, f0, f1) for p, q, f0, f1, ti in walls if root(ti) == reach]
    segs = []
    for p, q, f0, f1 in walls:
        A = tuple(p[k] + f0 * (q[k] - p[k]) for k in range(3))
        B = tuple(p[k] + f1 * (q[k] - p[k]) for k in range(3))
        segs.append([A, B])
    by_end = defaultdict(list)
    for i, s in enumerate(segs):
        by_end[K(s[0])].append(i)
        by_end[K(s[1])].append(i)
    alive = [True] * len(segs)
    merged = 0
    for i in range(len(segs)):
        if not alive[i]:
            continue
        changed = True
        while changed:
            changed = False
            for end in (0, 1):
                for j in by_end[K(segs[i][end])]:
                    if j == i or not alive[j]:
                        continue
                    a, b = segs[i], segs[j]
                    if end == 1:
                        bb = b if K(b[0]) == K(a[1]) else [b[1], b[0]]
                    else:
                        bb = b if K(b[1]) == K(a[0]) else [b[1], b[0]]
                    s0, s1 = (a[0], bb[1]) if end == 1 else (bb[0], a[1])
                    mid = a[1] if end == 1 else a[0]
                    L = math.dist(s0, s1)
                    if L < 1e-6 or L > 12:
                        continue
                    t = sum((mid[k] - s0[k]) * (s1[k] - s0[k]) for k in range(3)) / (
                        L * L
                    )
                    if (
                        not 0 < t < 1
                        or math.dist(
                            mid, [s0[k] + t * (s1[k] - s0[k]) for k in range(3)]
                        )
                        > TOL
                    ):
                        continue
                    segs[i] = [a[0], bb[1]] if end == 1 else [bb[0], a[1]]
                    alive[j] = False
                    merged += 1
                    by_end[K(segs[i][0])].append(i)
                    by_end[K(segs[i][1])].append(i)
                    changed = True
                    break
                if changed:
                    break
    walls = [(list(s[0]), list(s[1]), 0.0, 1.0) for s, ok in zip(segs, alive) if ok]
    stats["wall pieces merged"] = merged
    NV, NT = [], []
    CAPR = cap_reach

    def clearance(A, B):
        L = math.hypot(B[0] - A[0], B[2] - A[2]) or 1e-9
        nx, nz = -(B[2] - A[2]) / L, (B[0] - A[0]) / L
        best = H
        for s in range(max(1, math.ceil(L / 0.5)) + 1):
            f = s / max(1, math.ceil(L / 0.5))
            x, y, zz = (A[k] + f * (B[k] - A[k]) for k in range(3))
            for o in (0.0, CAPR / 2, -CAPR / 2, CAPR, -CAPR):
                for h in heights(x + nx * o, zz + nz * o):
                    if y + (0.05 if o == 0 else 0.4) < h < y + H + 0.05:
                        best = min(best, h - 0.05 - y)
        return best

    for p, q, f0, f1 in walls:
        A = [p[k] + f0 * (q[k] - p[k]) for k in range(3)]
        B = [p[k] + f1 * (q[k] - p[k]) for k in range(3)]
        c = clearance(A, B)
        ta, tb = A[1] + c, B[1] + c
        if ta - A[1] < 0.15 and tb - B[1] < 0.15:
            stats["walls capped away"] += 1
            continue
        i = len(NV)
        NV += [
            [A[0], A[1] - 0.1, A[2]],
            [B[0], B[1] - 0.1, B[2]],
            [B[0], tb, B[2]],
            [A[0], ta, A[2]],
        ]
        NT += [
            (i, i + 1, i + 2),
            (i, i + 2, i + 3),
            (i, i + 2, i + 1),
            (i, i + 3, i + 2),
        ]
    stats["new walls"] = len(walls)
    for p, q, f0, f1, nx, nz, dist, dy, ti in bridges:
        if root(ti) != reach:
            continue
        A = [p[k] + f0 * (q[k] - p[k]) for k in range(3)]
        B = [p[k] + f1 * (q[k] - p[k]) for k in range(3)]
        o = dist + 0.2
        A2 = [A[0] + nx * o, A[1] + dy, A[2] + nz * o]
        B2 = [B[0] + nx * o, B[1] + dy, B[2] + nz * o]
        i = len(NV)
        NV += [A, B, B2, A2]
        tri = (i, i + 1, i + 2)
        if nrm(*[NV[k] for k in tri])[1] < 0:
            NT += [(i, i + 2, i + 1), (i, i + 3, i + 2)]
        else:
            NT += [(i, i + 1, i + 2), (i, i + 2, i + 3)]
        stats["bridge strips"] += 1
    if prune is not None:
        ABOVE = prune
        band = defaultdict(lambda: [1e9, -1e9])
        for ti, (a, b, c) in enumerate(walk):
            if root(ti) != reach:
                continue
            ys = (a[1], b[1], c[1])
            for i in range(
                int(min(a[0], b[0], c[0]) // 1) - 1, int(max(a[0], b[0], c[0]) // 1) + 2
            ):
                for k in range(
                    int(min(a[2], b[2], c[2]) // 1) - 1,
                    int(max(a[2], b[2], c[2]) // 1) + 2,
                ):
                    s = band[(i, k)]
                    s[0] = min(s[0], min(ys))
                    s[1] = max(s[1], max(ys))

        def touchable(p):
            lo, hi = min(q[1] for q in p), max(q[1] for q in p)
            for i in range(
                int(min(q[0] for q in p) // 1), int(max(q[0] for q in p) // 1) + 1
            ):
                for k in range(
                    int(min(q[2] for q in p) // 1), int(max(q[2] for q in p) // 1) + 1
                ):
                    s = band.get((i, k))
                    if s and lo <= s[1] + ABOVE and hi >= s[0] - 0.5:
                        return True
            return False

        n0 = len(T)
        T = [t for t in T if touchable([V[i] for i in t])]
        stats["pruned untouchable"] = n0 - len(T)
    OV, idx, OT = [], {}, []

    def vid(p):
        k = (round(p[0] - tx, 3), round(p[1], 3), round(p[2] - tz, 3))
        if k not in idx:
            idx[k] = len(OV)
            OV.append(list(k))
        return idx[k]

    for t in T:
        ids = [vid(V[i]) for i in t]
        if len(set(ids)) == 3:
            OT.append(ids)
    for t in NT:
        ids = [vid(NV[i]) for i in t]
        if len(set(ids)) == 3:
            OT.append(ids)
    stats["tris"], stats["verts"] = len(OT), len(OV)
    if len(OT) > MAX or len(OV) > 65535:
        raise SystemExit("over the PHB cap: %d tris, %d verts" % (len(OT), len(OV)))
    return {
        "schemaVersion": 1,
        "vertices": OV,
        "triangles": [{"indices": t, "surface": 0} for t in OT],
    }, dict(stats)


# ---- spurious walls (spurious_walls.py) --------------------------------------------------------------------------


def spurious_walls(col, parts, off, step=0.35):
    """Drop generated walls (not in parts, which are in layout frame) with walkable floor on both sides along the
    whole base. col is in the collision tile frame = layout + off. Returns (out dict, log lines)."""
    STEP = step
    OFF = np.array(off, float)
    V = np.array(col["vertices"], float)
    TRI = np.array([t["indices"] for t in col["triangles"]])
    P = V[TRI]
    fn = np.cross(P[:, 1] - P[:, 0], P[:, 2] - P[:, 0])
    L = np.linalg.norm(fn, axis=1)
    fn /= np.maximum(L, 1e-12)[:, None]
    wall = (np.abs(fn[:, 1]) < 0.2) & (L > 1e-9)
    floor = fn[:, 1] > 0.4
    floor |= fn[:, 1] < -0.4

    def K(X):
        return frozenset(tuple(np.round(v, 2)) for v in X)

    known = set()
    for g in parts:
        W = np.array(g["vertices"], float) + OFF
        for t in g["triangles"]:
            known.add(K(W[t["indices"]]))
    gen = np.array([wall[i] and K(P[i]) not in known for i in range(len(P))])
    log = [f"tris {len(P)}, walls {wall.sum()}, generated walls {gen.sum()}"]
    F = P[floor]
    cell = 2.0
    grid = {}
    mn = np.floor(F[:, :, [0, 2]].min(1) / cell).astype(int)
    mx = np.floor(F[:, :, [0, 2]].max(1) / cell).astype(int)
    for i in range(len(F)):
        for gx in range(mn[i, 0], mx[i, 0] + 1):
            for gz in range(mn[i, 1], mx[i, 1] + 1):
                grid.setdefault((gx, gz), []).append(i)

    def floor_at(x, z, y, tol=STEP):
        ids = grid.get((int(np.floor(x / cell)), int(np.floor(z / cell))))
        if not ids:
            return False
        t = F[ids]
        a0, b0, c0 = t[:, 0], t[:, 1], t[:, 2]
        v0x, v0z, v1x, v1z = (
            c0[:, 0] - a0[:, 0],
            c0[:, 2] - a0[:, 2],
            b0[:, 0] - a0[:, 0],
            b0[:, 2] - a0[:, 2],
        )
        v2x, v2z = x - a0[:, 0], z - a0[:, 2]
        den = v0x * v1z - v1x * v0z
        ok = np.abs(den) > 1e-12
        d = np.where(ok, den, 1)
        u = (v2x * v1z - v1x * v2z) / d
        v = (v0x * v2z - v2x * v0z) / d
        ins = ok & (u >= -1e-6) & (v >= -1e-6) & (u + v <= 1 + 1e-6)
        h = a0[:, 1] + u * (c0[:, 1] - a0[:, 1]) + v * (b0[:, 1] - a0[:, 1])
        return bool((ins & (np.abs(h - y) <= tol)).any())

    gi = np.nonzero(gen)[0]
    par = {i: i for i in gi}

    def root(i):
        while par[i] != i:
            par[i] = par[par[i]]
            i = par[i]
        return i

    edges = {}
    for i in gi:
        for e in ((0, 1), (1, 2), (0, 2)):
            k = tuple(sorted((TRI[i][e[0]], TRI[i][e[1]])))
            for j in edges.get(k, []):
                if abs(fn[i] @ fn[j]) > 0.99:
                    par[root(i)] = root(j)
            edges.setdefault(k, []).append(i)
    panels = {}
    for i in gi:
        panels.setdefault(root(i), []).append(i)

    def sides(X, nvec):
        foot = X[:, 1].min()
        pts = X[:, [0, 2]]
        d = ((pts[:, None] - pts[None]) ** 2).sum(-1)
        a_, b_ = np.unravel_index(d.argmax(), d.shape)
        res = []
        for f in (0.2, 0.5, 0.8):
            m = pts[a_] + f * (pts[b_] - pts[a_])
            res.append(
                (
                    floor_at(*(m + 0.6 * nvec), foot + 0.1),
                    floor_at(*(m - 0.6 * nvec), foot + 0.1),
                )
            )
        return foot, res

    spur = np.zeros(len(P), bool)
    for r_, ids in panels.items():
        X = P[ids].reshape(-1, 3)
        n = fn[ids[0]][[0, 2]]
        n /= max(np.linalg.norm(n), 1e-9)
        foot, res = sides(X, n)
        if all(s1 and s2 for s1, s2 in res):
            spur[ids] = True
    log.append(
        f"panels {len(panels)}; spurious (floor on both sides along the whole base): {spur.sum()} tris"
    )
    keep = [t for i, t in enumerate(col["triangles"]) if not spur[i]]
    used = sorted({j for t in keep for j in t["indices"]})
    remap = {o: n for n, o in enumerate(used)}
    res = {
        "schemaVersion": col.get("schemaVersion", 1),
        "vertices": [col["vertices"][j] for j in used],
        "triangles": [dict(t, indices=[remap[j] for j in t["indices"]]) for t in keep],
    }
    log.append(f"result {len(res['triangles'])} tris / {len(res['vertices'])} verts")
    return res, log


# ---- hole patch from terrain plates (collision_patch.py) ---------------------------------------------------------


def patch_holes(col, plates, cover=0.35, clear=2.0):
    """Add walkable plate triangles (N x 3 x 3, already in the collision frame) that cover collision floor holes:
    no collision floor within cover m and nothing from there up to clear m. Returns (floor dict, log line)."""
    COVER, CLEAR = cover, clear
    V = np.array(col["vertices"], float)
    TRI = np.array([t["indices"] for t in col["triangles"]])
    C = V[TRI]
    T = plates

    def fn(X):
        return np.cross(X[:, 1] - X[:, 0], X[:, 2] - X[:, 0])

    n = fn(T)
    n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-12)
    walk = np.abs(n[:, 1]) > 0.7
    cell = 2.0
    grid = {}
    mn = np.floor(C[:, :, [0, 2]].min(1) / cell).astype(int)
    mx = np.floor(C[:, :, [0, 2]].max(1) / cell).astype(int)
    for i in range(len(C)):
        for gx in range(mn[i, 0], mx[i, 0] + 1):
            for gz in range(mn[i, 1], mx[i, 1] + 1):
                grid.setdefault((gx, gz), []).append(i)

    def heights(x, z):
        ids = grid.get((int(np.floor(x / cell)), int(np.floor(z / cell))))
        if not ids:
            return np.zeros(0)
        t = C[ids]
        a0, b0, c0 = t[:, 0], t[:, 1], t[:, 2]
        v0x, v0z, v1x, v1z = (
            c0[:, 0] - a0[:, 0],
            c0[:, 2] - a0[:, 2],
            b0[:, 0] - a0[:, 0],
            b0[:, 2] - a0[:, 2],
        )
        v2x, v2z = x - a0[:, 0], z - a0[:, 2]
        den = v0x * v1z - v1x * v0z
        ok = np.abs(den) > 1e-12
        d = np.where(ok, den, 1)
        u = (v2x * v1z - v1x * v2z) / d
        v = (v0x * v2z - v2x * v0z) / d
        ins = ok & (u >= -1e-6) & (v >= -1e-6) & (u + v <= 1 + 1e-6)
        return (a0[:, 1] + u * (c0[:, 1] - a0[:, 1]) + v * (b0[:, 1] - a0[:, 1]))[ins]

    add = []
    for i in np.nonzero(walk)[0]:
        A, B, Cc = T[i]
        L = max(np.linalg.norm(B - A), np.linalg.norm(Cc - A), np.linalg.norm(Cc - B))
        k = max(2, int(np.ceil(L / 0.5)))
        holes = 0
        for p in range(k + 1):
            for q in range(k + 1 - p):
                s = (
                    A + (0.05 + 0.9 * p / k) * (B - A) + (0.05 + 0.9 * q / k) * (Cc - A)
                    if p + q <= k
                    else None
                )
                h = heights(s[0], s[2])
                if (np.abs(h - s[1]) <= COVER).any():
                    continue
                if ((h > s[1]) & (h < s[1] + CLEAR)).any():
                    continue
                holes += 1
        if holes:
            add.append(i)
    add = np.array(add, int)
    area = 0.5 * np.linalg.norm(fn(T[add]), axis=1).sum() if len(add) else 0
    line = f"walkable plate tris {walk.sum()}; with collision holes {len(add)} ({area:.0f} m2)"
    NV = [list(map(float, v)) for v in col["vertices"]]
    NT = list(col["triangles"])
    for i in add:
        X = T[i] if n[i, 1] > 0 else T[i][[0, 2, 1]]
        base = len(NV)
        NV += [list(map(float, np.round(v, 4))) for v in X]
        NT.append({"indices": [base, base + 1, base + 2], "surface": 0})
    return {
        "schemaVersion": col.get("schemaVersion", 1),
        "vertices": NV,
        "triangles": NT,
    }, line


def terrain_plates(client, bg_path, decode):
    """Terrain plate triangles of a modern zone (bgplate/terrain.tera + NNNN.mdl) in the source frame, N x 3 x 3."""
    return terrain_plates_full(client, bg_path, decode)[0]


def terrain_plates_full(client, bg_path, decode):
    """(triangles N x 3 x 3, uvs N x 3 x 2, material paths N) of the terrain plates (terrain_plates.py load)."""
    base = bg_path.rstrip("/") + "/bgplate/"
    d = client.read(base + "terrain.tera")
    count, size = struct.unpack_from("<2I", d, 4)
    plates = [struct.unpack_from("<2h", d, 52 + 4 * i) for i in range(count)]
    T, U, M = [], [], []
    for i, (px, pz) in enumerate(plates):
        m = decode(client.read(base + f"{i:04d}.mdl"))
        off = np.array([size * (px + 0.5), 0.0, size * (pz + 0.5)])
        for me in m.meshes:
            if not me["positions"] or len(me["indices"]) < 3:
                continue
            P = np.array(me["positions"], float) + off
            tri_idx = np.array(me["indices"][: len(me["indices"]) // 3 * 3]).reshape(
                -1, 3
            )
            T.append(P[tri_idx])
            M += [me["material"]] * len(tri_idx)
            uv = np.array(me["uvs"], float) if me["uvs"] else np.zeros((len(P), 2))
            U.append(uv[tri_idx])
    return np.concatenate(T), np.concatenate(U), np.array(M)


# ---- prop collision (object_collision.py) ------------------------------------------------------------------------


def parse_pcb(buf):
    tris = []

    def node(at):
        magic, version, c1, c2 = struct.unpack_from("<4I", buf, at)
        bb = struct.unpack_from("<6f", buf, at + 16)
        n16, npoly, n32, _ = struct.unpack_from("<4H", buf, at + 40)
        p = at + 48
        pts = []
        for _ in range(n32):
            pts.append(struct.unpack_from("<3f", buf, p))
            p += 12
        lo, hi = bb[0:3], bb[3:6]
        for _ in range(n16):
            q = struct.unpack_from("<3H", buf, p)
            p += 6
            pts.append(
                tuple(lo[k] + (hi[k] - lo[k]) * q[k] / 65535.0 for k in range(3))
            )
        for _ in range(npoly):
            a, b, c, _pad = struct.unpack_from("<4B", buf, p)
            p += 12
            if max(a, b, c) < len(pts):
                tris.append((pts[a], pts[b], pts[c]))
        for c in (c1, c2):
            if c:
                node(at + c)

    node(16)
    return tris


def objects(
    insts,
    read_pcb,
    yshift,
    conv="A",
    cluster=0.0,
    box=None,
    box_min_real=0,
    box_shrink=1.0,
    cluster_only=None,
    unbox_extent=0.0,
):
    """Place each instance's .pcb (insts: rows with pcb, t, r, s) into layout space. read_pcb(path) -> bytes or None.

    Transform p = T + R * (S * p_local) + (0, yshift, 0); conv A = Rz*Ry*Rx (column vectors), B = its transpose.
    box (regex): matching objects become their local AABB (12 tris), only if they have more than box_min_real
    triangles or are at most unbox_extent across; box_shrink narrows boxes in X/Z. cluster: vertex clustering size."""
    insts = [o for o in insts if o.get("pcb")]

    def simplify(tris, c):
        if c <= 0:
            return tris
        acc = {}
        for tri in tris:
            for p in tri:
                k = (math.floor(p[0] / c), math.floor(p[1] / c), math.floor(p[2] / c))
                s = acc.setdefault(k, [0.0, 0.0, 0.0, 0])
                s[0] += p[0]
                s[1] += p[1]
                s[2] += p[2]
                s[3] += 1
        rep = {k: (s[0] / s[3], s[1] / s[3], s[2] / s[3]) for k, s in acc.items()}
        out, seen = [], set()
        for tri in tris:
            ks = [
                (math.floor(p[0] / c), math.floor(p[1] / c), math.floor(p[2] / c))
                for p in tri
            ]
            if len(set(ks)) < 3:
                continue
            key = tuple(sorted(ks))
            if key in seen:
                continue
            seen.add(key)
            out.append(tuple(rep[k] for k in ks))
        return out

    def box_of(tris):
        pts = [p for t in tris for p in t]
        if not pts:
            return []
        lo = [min(p[i] for p in pts) for i in range(3)]
        hi = [max(p[i] for p in pts) for i in range(3)]
        for i in (0, 2):
            m, h = (lo[i] + hi[i]) / 2, (hi[i] - lo[i]) / 2 * box_shrink
            lo[i], hi[i] = m - h, m + h
        c = [
            (x, y, z)
            for x in (lo[0], hi[0])
            for y in (lo[1], hi[1])
            for z in (lo[2], hi[2])
        ]
        faces = [
            (0, 1, 3, 2),
            (4, 6, 7, 5),
            (0, 4, 5, 1),
            (2, 3, 7, 6),
            (0, 2, 6, 4),
            (1, 5, 7, 3),
        ]
        out = []
        for a, b, cc, d in faces:
            out += [(c[a], c[b], c[cc]), (c[a], c[cc], c[d])]
        return out

    cache = {}

    def local(path):
        if path not in cache:
            data = read_pcb(path)
            tris = parse_pcb(data) if data else []
            pts = [p for t in tris for p in t]
            ext = max(
                (max(p[i] for p in pts) - min(p[i] for p in pts) for i in range(3)),
                default=0,
            )
            name = path.split("/")[-1]
            if (
                box
                and box.search(name)
                and (len(tris) > box_min_real or ext <= unbox_extent)
            ):
                tris = box_of(tris)
            elif cluster_only is None or cluster_only.search(name):
                tris = simplify(tris, cluster)
            cache[path] = tris
        return cache[path]

    def rot(r):
        cx, sx = math.cos(r[0]), math.sin(r[0])
        cy, sy = math.cos(r[1]), math.sin(r[1])
        cz, sz = math.cos(r[2]), math.sin(r[2])
        Rx = [[1, 0, 0], [0, cx, -sx], [0, sx, cx]]
        Ry = [[cy, 0, sy], [0, 1, 0], [-sy, 0, cy]]
        Rz = [[cz, -sz, 0], [sz, cz, 0], [0, 0, 1]]

        def mm(A, B):
            return [
                [sum(A[i][k] * B[k][j] for k in range(3)) for j in range(3)]
                for i in range(3)
            ]

        M = mm(Rz, mm(Ry, Rx))
        return M if conv == "A" else [[M[j][i] for j in range(3)] for i in range(3)]

    tris = []
    for o in insts:
        R = rot(o["r"])
        s = o["s"]
        t = o["t"]
        for tri in local(o["pcb"]):
            w = []
            for p in tri:
                q = (p[0] * s[0], p[1] * s[1], p[2] * s[2])
                w.append(
                    (
                        t[0] + R[0][0] * q[0] + R[0][1] * q[1] + R[0][2] * q[2],
                        t[1]
                        + R[1][0] * q[0]
                        + R[1][1] * q[1]
                        + R[1][2] * q[2]
                        + yshift,
                        t[2] + R[2][0] * q[0] + R[2][1] * q[1] + R[2][2] * q[2],
                    )
                )
            tris.append(w)
    verts, index, out_t = [], {}, []
    for tri in tris:
        ids = []
        for p in tri:
            k = tuple(round(c, 3) for c in p)
            if k not in index:
                index[k] = len(verts)
                verts.append(list(k))
            ids.append(index[k])
        if len(set(ids)) == 3:
            out_t.append({"indices": ids, "surface": 0})
    return {
        "schemaVersion": 1,
        "vertices": verts,
        "triangles": out_t,
    }, "instances %d, unique pcb %d, triangles %d, vertices %d" % (
        len(insts),
        len(cache),
        len(out_t),
        len(verts),
    )


# ---- merge -------------------------------------------------------------------------------------------------------


def merge(base, extras):
    """Append geometry to base: extras = [(geometry, (dx, dy, dz))], vertices moved by the offset (not rounded)."""
    V = list(base["vertices"])
    T = list(base["triangles"])
    for g, off in extras:
        n = len(V)
        V += [[v[k] + off[k] for k in range(3)] for v in g["vertices"]]
        T += [dict(t, indices=[i + n for i in t["indices"]]) for t in g["triangles"]]
    return {
        "schemaVersion": base.get("schemaVersion", 1),
        "vertices": V,
        "triangles": T,
    }


def dumps(g):
    """json.dump defaults (no indent), the form the PHB author reads."""
    return json.dumps(g).encode("ascii")
