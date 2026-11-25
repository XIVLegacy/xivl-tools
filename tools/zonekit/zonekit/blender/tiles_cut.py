"""zonekit tiles cut (cell size from --cell-w/--cell-d).
Rebuild one city tile as several 1.0 models with MANY small MDL cull boxes (pop-in fix, tile 24 first).

blender -b -P build_multi_tile.py -- <tile.glb (tile-local, pre-vertex-fix)> <out-dir> [--tag t24] [--ext 12]
    [--cellx 26] [--cellz 23] [--fary -60] [--maxtris 20000] [--maxmdl 39] [--sectors 8] [--flat 1.5 --flatext 32]
    [--cell [--grid 16] [--bisect [--nx 4 --nz 3]] [--onemesh | --first0]]
    [--thin [--band 1.5 --run 16 --cube 4 --vert 0.3 --small 20 --wall 1.5 --slope-floor --single
     --run-floor 16 --run-wall 16 --merge-max 0 --weld [--weld-dist 0.0005] [--slotmap slotmap-fnd.tsv [--slot-min-tris 0]]]]

Why: with MICT = 1 the client culls each MDL by its MDL-level AABB; a box that contains the camera makes the whole
MDL vanish. The donor w0d0_tub_ed1_h (89\\7C\\00\\1D) has 39 MDLs, so one model file = 39 small boxes.
- far  = faces whose centroid is outside the cell (|x| > cellx or |z| > cellz, glTF frame) or below Y fary.
- wide = other faces that alone do not fit a box (X/Z <= ext, or <= flatext if thinner than flat on Y): one MDL each,
         so the triangle count stays exact.
Default mode (one mesh per MDL): far per material and angular sector; city in 2 x 2 subcells (x = 0, z = 0) per
  material, median split on face centroids until the piece's VERTEX box fits and <= maxtris (Y free).
--cell mode (several meshes per MDL, one per material): a --grid m grid aligned to the local origin; per grid cell
  "flat" = the thickest-populated Y band (< flat m) of thin faces, flats of 2 x 2 cells merged while the union stays
  <= flatext on X/Z and thin; "solid" = the cell's other faces, median split while X/Z > ext or a material mesh
  > maxtris; far = one MDL per sector (split only for maxtris).
Shaders: fixed per donor MDL (even 0-34 rock6, odd 1-35 sand3, 36 wood1/metl1, 37 rock5/rock7, 38 sdjr1/jrix1/rock4).
xiv1tt (ModelImport.cs) keeps the donor mesh's shader for a mesh index < the MDL's donor mesh count, and for a higher
index takes the shader by GLB material name from THAT MDL's meshes only (else its last mesh). Default mode: a piece
goes to a free MDL with its intended shader (node mdlK_meshJ), else rock6, then sand3. --cell mode: MDLs 36/37/38 go
to the pieces with the most wood+metal / mortar / item tris, real meshes start at the MDL's donor mesh count (named
by intended shader); flat MDLs prefer odd (sand3), others even (rock6). Unused MDLs get a 1 cm triangle at the file's
centre (no donor geometry left).
Writes <out-dir>/tile_<k>.glb (k = file index, named for fix_tile_verts.py) and <out-dir>/manifest.json.
"""

import bpy
import bmesh
import sys
import os
import re
import math
import json
import time
from collections import defaultdict, Counter

T0 = time.time()


def log(*a):
    print(f"[{time.time() - T0:6.1f}s]", *a, flush=True)


a = sys.argv[sys.argv.index("--") + 1 :]
src, outdir = a[0], a[1]


def opt(n, d):
    return type(d)(a[a.index(n) + 1]) if n in a else d


TAG, EXT, CX, CZ, FARY = (
    opt("--tag", "t24"),
    opt("--ext", 12.0),
    opt("--cellx", 26.0),
    opt("--cellz", 23.0),
    opt("--fary", -60.0),
)
MAXT, MAXM, NSECT = opt("--maxtris", 20000), opt("--maxmdl", 39), opt("--sectors", 8)
# --flat H --flatext E: a piece whose vertex box is thinner than H m on Y may be up to E m on X/Z (camera never
# inside a thin box). Default off (flat = ext).
FLAT, FLATEXT = opt("--flat", 0.0), opt("--flatext", EXT)
CELL, GRID = "--cell" in a, opt("--grid", 16.0)
# --onemesh (cell mode): ONE real mesh per MDL at index 0 (all materials of the piece merged; shader = the donor MDL's
#   mesh 0). --first0: several meshes per MDL, the first always at index 0 (also in MDLs
#   36-38), the rest appended.
ONEMESH, FIRST0 = "--onemesh" in a, "--first0" in a
os.makedirs(outdir, exist_ok=True)

ROCK6, SAND3 = "w0d_c0_rock6_2h", "w0d_c0_sand3_4h"
DONOR = {k: [ROCK6 if k % 2 == 0 else SAND3] for k in range(36)}
DONOR[36] = ["w0d_b0_wood1_1h", "w0d_b0_metl1_1h"]
DONOR[37] = ["w0d_c0_rock5_4h", "w0d_c0_rock7_3h"]
DONOR[38] = ["w0d_c0_sdjr1_2h", "w0d_c0_jrix1_2h", "w0d_c0_rock4_2h"]
# Foundation material -> intended donor shader (slotmap.tsv intent: idr11 slot 0 ground, 1/4/7 stone walls,
# 2/3 wood and objects, 5 metal, 6 mortar walls, 8 items/signs).
INTENT = {
    "pw0_00_stpv1_1h": SAND3,
    "pw0_00_stbl2_2h": ROCK6,
    "pw0_00_stbl4_1h": ROCK6,
    "pw0_00_mtst3_1h": ROCK6,
    "pw0_00_mort1_1h": "w0d_c0_rock7_3h",
    "pw0_m0_wdst1_1h": "w0d_b0_wood1_1h",
    "pw0_m0_mtwd1_1h": "w0d_b0_wood1_1h",
    "pw0_00_mtst1_1h": "w0d_b0_metl1_1h",
    "pw0_00_g0011_1h": "w0d_c0_sdjr1_2h",
}
SCARCE = {"w0d_b0_wood1_1h", "w0d_b0_metl1_1h", "w0d_c0_rock7_3h", "w0d_c0_sdjr1_2h"}

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
objs = [o for o in bpy.data.objects if o.type == "MESH"]


def base(n):
    return re.sub(r"\.\d{3}$", "", n)


by_mat = defaultdict(list)
for o in objs:
    by_mat[base(o.data.materials[0].name)].append(o)
for m in by_mat:
    if "--slotmap" in a:
        INTENT.setdefault(
            m, ROCK6
        )  # real source names: INTENT only matters without --single
    assert m in INTENT, "no intent for material " + m
total_in = sum(len(o.data.polygons) for o in objs)
assert all(len(p.vertices) == 3 for o in objs for p in o.data.polygons), (
    "input not triangulated"
)
log(
    "input tris",
    total_in,
    {k: len(v) for k, v in by_mat.items()},
    "mode",
    "cell" if CELL else "default",
)
joined = {}
for mat, os_ in by_mat.items():
    bpy.ops.object.select_all(action="DESELECT")
    for o in os_:
        o.select_set(True)
    bpy.context.view_layer.objects.active = os_[0]
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    if len(os_) > 1:
        bpy.ops.object.join()
    j = bpy.context.view_layer.objects.active
    j.name = "J_" + mat
    m0 = j.data.materials[0]
    j.data.materials.clear()
    j.data.materials.append(m0)
    joined[mat] = j


def gl(co):
    return (co.x, co.z, -co.y)  # Blender Z-up -> glTF (x, y, z)


def fits(dx, dz, dy, ext):
    lim = FLATEXT if dy < FLAT else ext
    return dx <= lim and dz <= lim


# --bisect (with --cell): cut the city faces along every line of an --nx x --nz grid over the tile cell
# (360/7 x 320/7 m, centre at the origin, outer border included), so every city face lies inside one grid cell.
# Far = centroid outside the exact cell or below fary (not cut). Cells whose solid part has a material mesh
# > maxtris are cut once more at their midline (longer side) and get two solid MDLs.
BISECT, NXC, NZC = "--bisect" in a, opt("--nx", 4), opt("--nz", 3)
W, D = (
    opt("--cell-w", 360 / 7),
    opt("--cell-d", 320 / 7),
)  # zonekit: cell size from the zone grid
if BISECT:
    assert CELL, "--bisect needs --cell"
    CX, CZ = W / 2, D / 2
area_in = sum(p.area for j in joined.values() for p in j.data.polygons)

# --slotmap <tsv> (with --thin): every source face keeps an INT face attribute "slot" (index into SLOTS) through weld,
# join and cuts; after the cuts the joined mesh is separated by slot, and every output piece gets one primitive per slot
# present, named as the slot (w0d_c0_fndNN_2h). The tsv maps a source material name (regex, re.search, first match
# wins) to a slot. Tiles from `tiles build` carry the real names (r2t0_a0_obi1a[.mtrl]). The nine pw0_* proxy
# materials are first replaced by a representative material (PW0_REP). Mesh/node names of the input are never read.
PW0_REP = {
    "pw0_00_stpv1_1h": "r2t0_t0_snw1a",
    "pw0_00_stbl2_2h": "r2t0_a0_obi1a",
    "pw0_m0_wdst1_1h": "r2t0_a0_wod1a",
    "pw0_m0_mtwd1_1h": "r200_a0_obj01a",
    "pw0_00_stbl4_1h": "r2t0_a0_obi2b",
    "pw0_00_mtst1_1h": "r2t0_a0_mtl4a",
    "pw0_00_mort1_1h": "r2t0_a0_mor3a",
    "pw0_00_mtst3_1h": "r2t0_t0_wal2a",
    "pw0_00_g0011_1h": "w_itm_012_bas1a",
}
SLOTMAP = opt("--slotmap", "")
# --slot-min-tris N: inside each piece a slot with fewer than N triangles is written with the piece's largest slot
# (by triangles) instead, so slivers of a second material do not cost a primitive each (default 0 = off).
SLOTMIN, SLOTCUT = opt("--slot-min-tris", 0), [0]
# --slot-min-area A (m2, with --slot-min-tris): a slot is merged only if it ALSO covers less than A m2 in the piece, and
# it goes to the piece's largest slot BY AREA. Without it a big flat floor (few triangles) took the prop atlas of the
# many small triangles beside it.
SLOTMINAREA = float(opt("--slot-min-area", 0))
# --slot-group REGEX (on slot names): the matching slots are tested for smallness as ONE group per piece, so splitting
# the item atlas over many slots (fnd40, fnd82..93) does not hand each small prop to the wall beside it.
SLOTGROUP = opt("--slot-group", "")
SLOTS, KSLOT = (
    [],
    {},
)  # SLOTS: slot names in tsv order; KSLOT: joined-mesh key -> slot name (set by resplit_by_slot)
if SLOTMAP:
    assert "--thin" in a, "--slotmap needs --thin"
    SLOTRULES = [
        r.rstrip("\n").split("\t")
        for r in open(SLOTMAP, encoding="utf-8")
        if r.strip() and not r.startswith("#")
    ]
    SLOTS = list(dict.fromkeys(r[1] for r in SLOTRULES))

    def slot_of_mat(m):
        fnd = PW0_REP.get(m) or re.sub(
            r"\.mtrl$", "", m
        )  # pw0_* proxies via PW0_REP, real names directly
        for r in SLOTRULES:
            if re.search(r[0], fnd):
                return SLOTS.index(r[1])
        raise AssertionError("no slot for material " + m)

    for m, j in joined.items():
        sa = j.data.attributes.new("slot", "INT", "FACE")
        sa.data.foreach_set("value", [slot_of_mat(m)] * len(j.data.polygons))
    log("slotmap", SLOTMAP, {m: SLOTS[slot_of_mat(m)] for m in joined})


def resplit_by_slot():
    """separate every joined mesh by its face attribute "slot": new keys '<key>|<slot name>'."""
    global joined
    new = []
    for key, j in joined.items():
        me = j.data
        vals = [0] * len(me.polygons)
        me.attributes["slot"].data.foreach_get("value", vals)
        used = sorted(set(vals))
        if len(used) == 1:
            me.attributes.remove(me.attributes["slot"])
            new.append((used[0], key, j))
            continue
        m0 = me.materials[0]
        for _ in range(len(used) - 1):
            me.materials.append(m0)
        me.polygons.foreach_set("material_index", [used.index(v) for v in vals])
        me.update()
        bpy.ops.object.select_all(action="DESELECT")
        j.select_set(True)
        bpy.context.view_layer.objects.active = j
        bpy.ops.object.mode_set(mode="EDIT")
        bpy.ops.mesh.select_all(action="SELECT")
        bpy.ops.mesh.separate(type="MATERIAL")
        bpy.ops.object.mode_set(mode="OBJECT")
        for c in list(bpy.context.selected_objects):
            s = c.data.attributes["slot"].data[0].value
            c.data.attributes.remove(c.data.attributes["slot"])
            c.data.materials.clear()
            c.data.materials.append(m0)
            new.append((s, f"{key}|{SLOTS[s]}", c))
            INTENT[f"{key}|{SLOTS[s]}"] = INTENT[key]
    joined = {}
    for s, key, c in sorted(new, key=lambda t: (t[1].split("|")[0], t[0])):
        c.name = "J_" + key
        joined[key] = c
        KSLOT[key] = SLOTS[s]


# --weld (with --thin --single): join every material into ONE mesh and merge vertices within --weld-dist (0.5 mm)
# BEFORE classifying and cutting, so edges shared across material seams are truly shared and a cut splits both sides
# (no T-junction cracks). Shading normals are kept in a corner attribute "cn" (interpolated by the cuts) and
# reapplied as custom normals after cutting, since welding changes the spaces custom normals are encoded in.
WELD, WELDD = "--weld" in a, opt("--weld-dist", 0.0005)
if WELD:
    assert "--thin" in a and "--single" in a, (
        "--weld needs --thin --single (materials are merged)"
    )
    for j in joined.values():
        me = j.data
        cn = me.attributes.new("cn", "FLOAT_VECTOR", "CORNER")
        cn.data.foreach_set(
            "vector", [c for ln in me.corner_normals for c in ln.vector]
        )
    bpy.ops.object.select_all(action="DESELECT")
    js = list(joined.values())
    for j in js:
        j.select_set(True)
    bpy.context.view_layer.objects.active = js[0]
    if len(js) > 1:
        bpy.ops.object.join()
    jall = bpy.context.view_layer.objects.active
    jall.name = "J_ALL"
    m0 = jall.data.materials[0]
    jall.data.materials.clear()
    jall.data.materials.append(m0)
    bm = bmesh.new()
    bm.from_mesh(jall.data)
    nv = len(bm.verts)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=WELDD)
    log(
        f"weld: {len(js)} material meshes -> 1, vertices {nv} -> {len(bm.verts)}, faces {len(bm.faces)}"
    )
    bm.to_mesh(jall.data)
    bm.free()
    jall.data.update()
    INTENT["ALL"] = ROCK6
    joined = {"ALL": jall}


def restore_normals(j):
    me = j.data
    cn = me.attributes.get("cn")
    if cn is None:
        return
    v = [0.0] * (3 * len(me.loops))
    cn.data.foreach_get("vector", v)
    ns = []
    for i in range(len(me.loops)):
        x, y, z = v[3 * i : 3 * i + 3]
        lp = math.sqrt(x * x + y * y + z * z) or 1.0
        ns.append((x / lp, y / lp, z / lp))
    me.normals_split_custom_set(ns)
    me.attributes.remove(me.attributes["cn"])
    me.update()


def is_city(c):
    return abs(c[0]) <= CX and abs(c[2]) <= CZ and c[1] >= FARY


def cell_of(x, z):
    return (
        min(max(math.floor((x + W / 2) / (W / NXC)), 0), NXC - 1),
        min(max(math.floor((z + D / 2) / (D / NZC)), 0), NZC - 1),
    )


def cut(j, planes, sel):
    """bisect_plane (no clearing) on the faces sel() picks, per plane; then triangulate. UVs are interpolated."""
    bm = bmesh.new()
    bm.from_mesh(j.data)

    def crossing(f, k, v):
        return (
            min(gl(q.co)[k] for q in f.verts) < v - 1e-4
            and max(gl(q.co)[k] for q in f.verts) > v + 1e-4
        )

    for _ in range(4):
        # rounds over all planes: a later cut can move a fragment's centroid back inside; n-gons left by a cut are
        # sometimes not split, so triangulate after every plane and repeat until no selected face crosses
        did = 0
        for ax, v in planes:
            co, no = ((v, 0, 0), (1, 0, 0)) if ax == "x" else ((0, -v, 0), (0, 1, 0))
            k = 0 if ax == "x" else 2
            fs = [
                f
                for f in bm.faces
                if crossing(f, k, v) and sel(gl(f.calc_center_median()))
            ]
            if not fs:
                continue
            did += len(fs)
            geom = (
                fs
                + list({e for f in fs for e in f.edges})
                + list({vv for f in fs for vv in f.verts})
            )
            bmesh.ops.bisect_plane(
                bm,
                geom=geom,
                dist=1e-5,
                plane_co=co,
                plane_no=no,
                clear_inner=False,
                clear_outer=False,
            )
            bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 3])
        if not did:
            break
    bm.to_mesh(j.data)
    bm.free()
    j.data.update()


def faces():
    """faces: global id g -> (mat, local index); FB[g] = (cx, cz, x0, x1, z0, z1, y0, y1, cy)"""
    fm, fb = [], []
    for mat, j in joined.items():
        me = j.data
        for p in me.polygons:
            vs = [gl(me.vertices[v].co) for v in p.vertices]
            xs = [v[0] for v in vs]
            zs = [v[2] for v in vs]
            ys = [v[1] for v in vs]
            fm.append((mat, p.index))
            fb.append(
                (
                    sum(xs) / 3,
                    sum(zs) / 3,
                    min(xs),
                    max(xs),
                    min(zs),
                    max(zs),
                    min(ys),
                    max(ys),
                    sum(ys) / 3,
                )
            )
    return fm, fb


# --thin (GLB only): every piece box thin in one axis. Faces by glTF normal: |ny| >= 0.7 floor (thin in Y),
# |ny| < --vert (0.3) vertical: xwall if |nx| >= |nz| (thin in X) else zwall (thin in Z); the rest slope.
# City faces are cut (bisect_plane, UVs interpolated) along their class's grid, aligned to the local origin:
# floor Y every --band (1.5) then X, Z every --run (16); xwall X every band then Y, Z every run; zwall Z every band
# then X, Y every run; slope X, Y, Z every --cube (4). Piece = one grid cell of one class, one primitive per shader.
# Far faces (centroid outside the exact tile cell or below fary): 8 sector pieces, not cut. Pieces under 20 tris
# merge into the nearest same-class neighbour band if the union stays <= 2 bands thick. Shader meshes > maxtris are
# median-split. Writes <out-dir>/<tag>.glb (nodes p<k>_<class>) and <out-dir>/manifest.json.
# Thin v2 options: --wall T (wall slab thickness, default = band), --slope-floor (slopes cut like floors: Y bands +
# runs, instead of cubes), --single (every piece exported as ONE primitive on rock6, so meshes = pieces; the 20k-tri
# limit then counts the whole piece). Small pieces merge while the union stays <= 2 x the class's thin step.
THIN = "--thin" in a
BAND, RUN, CUBE, VERT, SMALL = (
    opt("--band", 1.5),
    opt("--run", 16.0),
    opt("--cube", 4.0),
    opt("--vert", 0.3),
    opt("--small", 20),
)
WALL, SLOPEFLOOR, SINGLE = opt("--wall", BAND), "--slope-floor" in a, "--single" in a
# v3: --run-floor / --run-wall: run length per class (default --run); --merge-max M: small-piece merges only while the
# union's thin axis stays <= M m (default 0 = 2 x the class step)
RUNF, RUNW, MERGEMAX = (
    opt("--run-floor", RUN),
    opt("--run-wall", RUN),
    opt("--merge-max", 0.0),
)
if THIN:
    CX, CZ = W / 2, D / 2
SPEC = {
    "floor": [(1, BAND), (0, RUNF), (2, RUNF)],
    "xwall": [(0, WALL), (1, RUNW), (2, RUNW)],
    "zwall": [(2, WALL), (0, RUNW), (1, RUNW)],
    "slope": [(1, BAND), (0, RUNF), (2, RUNF)]
    if SLOPEFLOOR
    else [(0, CUBE), (1, CUBE), (2, CUBE)],
}


def klass(n):  # n = glTF normal
    ax, ay, az = abs(n[0]), abs(n[1]), abs(n[2])
    if ay >= 0.7:
        return "floor"
    if ay < VERT:
        return "xwall" if ax >= az else "zwall"
    return "slope"


def cut_thin(j):
    bm = bmesh.new()
    bm.from_mesh(j.data)
    bm.normal_update()

    def co(q, k):
        return q.co.x if k == 0 else (q.co.z if k == 1 else -q.co.y)

    def plane(k, v):
        return (
            ((v, 0, 0), (1, 0, 0))
            if k == 0
            else (((0, 0, v), (0, 0, 1)) if k == 1 else ((0, -v, 0), (0, 1, 0)))
        )

    def cls_of(f):
        return klass(gl(f.normal))

    for c, axes in SPEC.items():
        for k, s in axes:
            for _ in range(
                4
            ):  # rounds: n-gons left by a cut are sometimes not split (see cut())
                # neighbours of cut faces become n-gons; triangulate so classification uses real triangles
                ng = [f for f in bm.faces if len(f.verts) > 3]
                if ng:
                    bmesh.ops.triangulate(bm, faces=ng)
                L = {
                    f
                    for f in bm.faces
                    if cls_of(f) == c and is_city(gl(f.calc_center_median()))
                }
                if not L:
                    break
                lo = min(co(q, k) for f in L for q in f.verts)
                hi = max(co(q, k) for f in L for q in f.verts)
                did = 0
                for i in range(math.floor(lo / s) + 1, math.ceil(hi / s)):
                    v = i * s
                    cand = [
                        f
                        for f in L
                        if f.is_valid
                        and min(co(q, k) for q in f.verts) < v - 1e-4
                        and max(co(q, k) for q in f.verts) > v + 1e-4
                    ]
                    if not cand:
                        continue
                    did += len(cand)
                    geom = (
                        cand
                        + list({e for f in cand for e in f.edges})
                        + list({q for f in cand for q in f.verts})
                    )
                    pc, pn = plane(k, v)
                    res = bmesh.ops.bisect_plane(
                        bm,
                        geom=geom,
                        dist=1e-5,
                        plane_co=pc,
                        plane_no=pn,
                        clear_inner=False,
                        clear_outer=False,
                    )
                    nf = [
                        e
                        for e in res["geom"]
                        if isinstance(e, bmesh.types.BMFace) and e.is_valid
                    ]
                    ng = [f for f in nf if len(f.verts) > 3]
                    tri = bmesh.ops.triangulate(bm, faces=ng)["faces"] if ng else []
                    L = {f for f in L if f.is_valid}
                    L.update(f for f in nf if f.is_valid)
                    L.update(tri)
                if not did:
                    break
    bmesh.ops.triangulate(bm, faces=[f for f in bm.faces if len(f.verts) > 3])
    bm.to_mesh(j.data)
    bm.free()
    j.data.update()


SPLIT = {}  # cell -> (axis, value) of the extra midline cut
if BISECT:
    planes = [("x", -W / 2 + i * W / NXC) for i in range(NXC + 1)] + [
        ("z", -D / 2 + k * D / NZC) for k in range(NZC + 1)
    ]
    for j in joined.values():
        cut(j, planes, is_city)
    FM, FB = faces()
    load = Counter()
    for g, f in enumerate(FB):
        c = (f[0], f[8], f[1])
        if is_city(c) and f[7] - f[6] >= FLAT:
            load[(cell_of(f[0], f[1]), "*" if ONEMESH else FM[g][0])] += 1
    for (c, mat), n in load.items():
        if n > MAXT and c not in SPLIT:
            x0 = -W / 2 + c[0] * W / NXC
            z0 = -D / 2 + c[1] * D / NZC
            SPLIT[c] = (
                ("x", x0 + W / NXC / 2)
                if W / NXC >= D / NZC
                else ("z", z0 + D / NZC / 2)
            )
    for c, pl in SPLIT.items():
        for j in joined.values():
            cut(j, [pl], lambda p, c=c: is_city(p) and cell_of(p[0], p[2]) == c)
    log(
        "bisect: tris",
        total_in,
        "->",
        sum(len(j.data.polygons) for j in joined.values()),
        "extra cell cuts",
        SPLIT,
    )
elif THIN:
    for j in joined.values():
        cut_thin(j)
        restore_normals(j)
    log(
        "thin cuts: tris",
        total_in,
        "->",
        sum(len(j.data.polygons) for j in joined.values()),
    )
    if SLOTMAP:
        resplit_by_slot()
        log("slot meshes:", {k: len(j.data.polygons) for k, j in joined.items()})
FM, FB = faces()
TOT = len(FB)
area_out = sum(p.area for j in joined.values() for p in j.data.polygons)


def box(ids):
    return (
        min(FB[i][2] for i in ids),
        max(FB[i][3] for i in ids),
        min(FB[i][4] for i in ids),
        max(FB[i][5] for i in ids),
        min(FB[i][6] for i in ids),
        max(FB[i][7] for i in ids),
    )


def split(ids, ok):
    """Median split on face centroids (axis = longer vertex-box side) until ok(ids) or one face."""
    if len(ids) == 1 or ok(ids):
        return [ids]
    x0, x1, z0, z1, _, _ = box(ids)
    ax = 0 if x1 - x0 >= z1 - z0 else 1
    ids = sorted(ids, key=lambda i: FB[i][ax])
    h = len(ids) // 2
    return split(ids[:h], ok) + split(ids[h:], ok)


def mat_ok(ids):
    return (
        len(ids) if ONEMESH else max(Counter(FM[i][0] for i in ids).values())
    ) <= MAXT


def box_ok(ext, thin=False):
    def ok(ids):
        x0, x1, z0, z1, y0, y1 = box(ids)
        if thin:
            return (
                x1 - x0 <= FLATEXT
                and z1 - z0 <= FLATEXT
                and y1 - y0 < FLAT
                and mat_ok(ids)
            )
        return fits(x1 - x0, z1 - z0, y1 - y0, ext) and (
            mat_ok(ids) if CELL else len(ids) <= MAXT
        )

    return ok


GX = [-W / 2 + i * W / NXC for i in range(NXC + 1)]
GZ = [-D / 2 + k * D / NZC for k in range(NZC + 1)]


def uncut(
    f,
):  # --bisect: a city face that still crosses a grid line (bisect refused it) -> far group
    return BISECT and (
        any(f[2] < v - 1e-4 and f[3] > v + 1e-4 for v in GX)
        or any(f[4] < v - 1e-4 and f[5] > v + 1e-4 for v in GZ)
    )


UNCUT = sum(
    1 for f in FB if abs(f[0]) <= CX and abs(f[1]) <= CZ and f[8] >= FARY and uncut(f)
)
if BISECT:
    log("uncut city faces moved to far:", UNCUT)
far, wide, rest = defaultdict(list), [], []
for g, f in enumerate(FB):
    if abs(f[0]) > CX or abs(f[1]) > CZ or f[8] < FARY or uncut(f):
        far[int((math.degrees(math.atan2(f[1], f[0])) % 360) // (360 / NSECT))].append(
            g
        )
    elif not fits(f[3] - f[2], f[5] - f[4], f[7] - f[6], EXT):
        wide.append(g)
    else:
        rest.append(g)


def subcell(g):
    return (1 if FB[g][1] >= 0 else 0) * 2 + (1 if FB[g][0] >= 0 else 0)


leaves = []  # (kind, group, [global face ids])
if not THIN:
    leaves += [("wide", f"sc{subcell(g)}" if not CELL else "cell", [g]) for g in wide]
MERGED = {}  # thin: leaf index -> number of small pieces merged into it
if THIN:
    FC = [
        klass(gl(p.normal)) for j in joined.values() for p in j.data.polygons
    ]  # same order as faces()

    def cen(g, k):
        return FB[g][0] if k == 0 else (FB[g][8] if k == 1 else FB[g][1])

    def ext(g, k):
        return (
            (FB[g][2], FB[g][3])
            if k == 0
            else ((FB[g][6], FB[g][7]) if k == 1 else (FB[g][4], FB[g][5]))
        )

    def sh_ok(ids):
        return (
            len(ids) if SINGLE else max(Counter(INTENT[FM[i][0]] for i in ids).values())
        ) <= MAXT

    far_t, cells_t, n_uncut, n_reclass = defaultdict(list), defaultdict(list), 0, 0
    for g, f in enumerate(FB):
        key = None
        if is_city((f[0], f[8], f[1])):
            # own class first; a fragment whose normal changed class (slivers) takes any class whose cell it fits
            for c in [FC[g]] + [x for x in SPEC if x != FC[g]]:
                idx = tuple(math.floor(cen(g, k) / s) for k, s in SPEC[c])
                if all(
                    ext(g, k)[0] >= i * s - 1e-3 and ext(g, k)[1] <= (i + 1) * s + 1e-3
                    for (k, s), i in zip(SPEC[c], idx)
                ):
                    key = (c,) + idx
                    break
            if key is None:
                n_uncut += 1
                key = ("uncut", g)  # bisect refused it: its own piece, box = the face
            elif key[0] != FC[g]:
                n_reclass += 1
        if key:
            cells_t[key].append(g)
        else:
            far_t[
                int((math.degrees(math.atan2(f[1], f[0])) % 360) // (360 / NSECT))
            ].append(g)
    UNCUT = n_uncut
    # merge small pieces into the nearest same-class neighbour band (union <= 2 bands thick)
    merged_into, n_merged = Counter(), 0
    nomerge = ("uncut",) if SLOPEFLOOR else ("slope", "uncut")
    for key in sorted(
        [k for k, v in cells_t.items() if len(v) < SMALL and k[0] not in nomerge],
        key=lambda k: len(cells_t[k]),
    ):
        if key not in cells_t or len(cells_t[key]) >= SMALL:
            continue
        ids = cells_t[key]
        k0, step0 = SPEC[key[0]][0]
        mc = sum(cen(g, k0) for g in ids) / len(ids)
        best = None
        for dd in (-1, 1):
            nb = (key[0], key[1] + dd) + key[2:]
            if nb not in cells_t:
                continue
            u = ids + cells_t[nb]
            if (
                max(ext(g, k0)[1] for g in u) - min(ext(g, k0)[0] for g in u)
                > (MERGEMAX or 2 * step0) + 1e-3
            ):
                continue
            dist = abs(sum(cen(g, k0) for g in cells_t[nb]) / len(cells_t[nb]) - mc)
            if best is None or dist < best[0]:
                best = (dist, nb)
        if best:
            cells_t[best[1]] += ids
            merged_into[best[1]] += 1 + merged_into.pop(key, 0)
            del cells_t[key]
            n_merged += 1
    for key in sorted(cells_t):
        for lp in split(cells_t[key], sh_ok):
            if key in merged_into:
                MERGED[len(leaves)] = merged_into[key]
            leaves.append((key[0], key[0], lp))
    for s in sorted(far_t):
        leaves += [("far", "far", lp) for lp in split(far_t[s], sh_ok)]
    RECLASS = n_reclass

    def farea(g):
        j = joined[FM[g][0]]
        return j.data.polygons[FM[g][1]].area

    unc = [k[1] for k in cells_t if k[0] == "uncut"]
    UNCUT_AREA = sum(farea(g) for g in unc)
    UNCUT_FACES = [
        dict(
            mat=FM[g][0],
            cls=FC[g],
            area=round(farea(g), 3),
            min=[FB[g][2], FB[g][6], FB[g][4]],
            max=[FB[g][3], FB[g][7], FB[g][5]],
        )
        for g in sorted(unc, key=farea, reverse=True)[:20]
    ]
    log(
        f"thin pieces {len(leaves)}: reclassed {n_reclass}, uncut faces kept as own pieces {n_uncut} ({UNCUT_AREA:.2f} m2), "
        f"small pieces merged {n_merged}"
    )
elif not CELL:
    # default mode: one material per leaf, per subcell / sector
    for mat in joined:
        for s in sorted(far):
            ids = [g for g in far[s] if FM[g][0] == mat]
            if ids:
                leaves += [
                    ("far", "far", lp)
                    for lp in split(ids, lambda ids: len(ids) <= MAXT)
                ]
        sub = defaultdict(list)
        for g in rest:
            if FM[g][0] == mat:
                sub[subcell(g)].append(g)
        for sc in sorted(sub):
            leaves += [("city", f"sc{sc}", lp) for lp in split(sub[sc], box_ok(EXT))]
elif BISECT:
    for s in sorted(far):
        leaves += [("far", "far", lp) for lp in split(far[s], mat_ok)]
    cells = defaultdict(list)
    for g in rest:
        cells[cell_of(FB[g][0], FB[g][1])].append(g)
    for c in sorted(cells):
        ids = cells[c]
        flat = [g for g in ids if FB[g][7] - FB[g][6] < FLAT]
        fs = set(flat)
        solid = [g for g in ids if g not in fs]
        if c in SPLIT:
            ax, v = SPLIT[c]
            k = 0 if ax == "x" else 1
            halves = [
                [g for g in solid if FB[g][k] < v],
                [g for g in solid if FB[g][k] >= v],
            ]
        else:
            halves = [solid]
        leaves += [("solid", "cell", h) for h in halves if h]
        if flat:
            leaves.append(("flat", "cell", flat))
    over = [lp for lp in leaves if not mat_ok(lp[2])]
    if over:
        log("WARNING meshes over maxtris:", [(lp[0], len(lp[2])) for lp in over])
    if len(leaves) > MAXM:
        json.dump(
            dict(
                src=src,
                error=f"{len(leaves)} MDLs > {MAXM}",
                input_tris=total_in,
                files=[],
            ),
            open(os.path.join(outdir, "manifest.json"), "w"),
            indent=1,
        )
        log("OVERFLOW", len(leaves), "MDLs")
        sys.exit(0)
else:
    for s in sorted(far):
        leaves += [("far", "far", lp) for lp in split(far[s], mat_ok)]
    cells = defaultdict(list)
    for g in rest:
        cells[(math.floor(FB[g][0] / GRID), math.floor(FB[g][1] / GRID))].append(g)
    flats = defaultdict(list)  # 2x2 block -> flat face lists
    for c in sorted(cells):
        ids = cells[c]
        thin = sorted(
            [g for g in ids if FB[g][7] - FB[g][6] < FLAT], key=lambda g: FB[g][6]
        )
        bands, cur, top = [], [], -1e9
        for (
            g
        ) in thin:  # greedy Y bands (sorted by y0) whose union stays thinner than FLAT
            if cur and max(top, FB[g][7]) - FB[cur[0]][6] >= FLAT:
                bands.append(cur)
                cur = []
                top = -1e9
            cur.append(g)
            top = max(top, FB[g][7])
        if cur:
            bands.append(cur)
        flat = max(bands, key=len) if bands else []
        fs = set(flat)
        solid = [g for g in ids if g not in fs]
        if flat:
            flats[(c[0] // 2, c[1] // 2)] += split(flat, box_ok(FLATEXT, thin=True))

        def solid_ok(ids):
            x0, x1, z0, z1, _, _ = box(ids)
            return x1 - x0 <= EXT and z1 - z0 <= EXT and mat_ok(ids)

        for lp in (
            split(solid, solid_ok) if solid else []
        ):  # a thin face > ext outside the flat band -> wide
            leaves.append(("solid" if solid_ok(lp) else "wide", "cell", lp))
    for blk in sorted(flats):
        groups = []
        for fl in sorted(flats[blk], key=len, reverse=True):
            for gr in groups:
                if box_ok(FLATEXT, thin=True)(gr + fl):
                    gr += fl
                    break
            else:
                groups.append(list(fl))
        leaves += [("flat", "cell", gr) for gr in groups]
assert (
    sum(len(lp[2]) for lp in leaves) == TOT
    and len({g for lp in leaves for g in lp[2]}) == TOT
)

# separate every material object by leaf id; the face attribute "pid" survives mesh.separate
leaf_of = [0] * TOT
for k, lp in enumerate(leaves):
    for g in lp[2]:
        leaf_of[g] = k
gid = defaultdict(dict)
for g, (mat, i) in enumerate(FM):
    gid[mat][i] = g
parts = defaultdict(dict)  # leaf -> mat -> object
for mat, j in joined.items():
    me = j.data
    vals = [leaf_of[gid[mat][p.index]] for p in me.polygons]
    used = sorted(set(vals))
    slot = {k: s for s, k in enumerate(used)}
    pid = me.attributes.new("pid", "INT", "FACE")
    pid.data.foreach_set("value", vals)
    m0 = me.materials[0]
    for _ in range(len(used) - 1):
        me.materials.append(m0)
    me.polygons.foreach_set("material_index", [slot[v] for v in vals])
    me.update()
    bpy.ops.object.select_all(action="DESELECT")
    j.select_set(True)
    bpy.context.view_layer.objects.active = j
    if len(used) > 1:
        bpy.ops.object.mode_set(mode="EDIT")
        bpy.ops.mesh.select_all(action="SELECT")
        bpy.ops.mesh.separate(type="MATERIAL")
        bpy.ops.object.mode_set(mode="OBJECT")
    seen = set()
    for c in list(bpy.context.selected_objects):
        k = c.data.attributes["pid"].data[0].value
        assert k not in seen
        seen.add(k)
        assert len(c.data.polygons) == sum(
            1 for g in leaves[k][2] if FM[g][0] == mat
        ), "piece face count mismatch"
        c.data.attributes.remove(c.data.attributes["pid"])
        c.data.materials.clear()
        parts[k][mat] = c
    assert seen == set(used)
    log("separated", mat, len(used))

pieces = []
for k, (kind, grp, ids) in enumerate(leaves):
    x0, x1, z0, z1, y0, y1 = box(ids)
    pieces.append(
        dict(
            kind=kind,
            group=grp,
            objs=parts[k],
            tris=len(ids),
            min=[x0, y0, z0],
            max=[x1, y1, z1],
            mt={m: len(o.data.polygons) for m, o in parts[k].items()},
        )
    )
assert sum(p["tris"] for p in pieces) == TOT
# --block G: merge every piece whose box centre falls in the same G x G m grid cell (X/Z, any
# height, any kind) into one MDL, so each slot is ONE mesh per cell instead of one per floor/slope/wall/far piece.
# Large MDLs do not vanish with the camera inside them; frame time follows the mesh count.
BLOCK = opt("--block", 0.0)
if BLOCK:
    cellp = defaultdict(list)
    for p in pieces:
        cx, cz = (p["min"][0] + p["max"][0]) / 2, (p["min"][2] + p["max"][2]) / 2
        cellp[(math.floor(cx / BLOCK), math.floor(cz / BLOCK))].append(p)
    merged_pieces = []
    for c in sorted(cellp):
        ps = cellp[c]
        mt = Counter()
        for p in ps:
            mt.update(p["mt"])
        merged_pieces.append(
            dict(
                kind="block",
                group="block",
                objs=[(m, o) for p in ps for m, o in p["objs"].items()],
                tris=sum(p["tris"] for p in ps),
                mt=dict(mt),
                min=[min(p["min"][i] for p in ps) for i in range(3)],
                max=[max(p["max"][i] for p in ps) for i in range(3)],
            )
        )
    log(f"BLOCK {BLOCK}: {len(pieces)} pieces -> {len(merged_pieces)} cells")
    pieces = merged_pieces
    assert sum(p["tris"] for p in pieces) == TOT

mats = {}


def shader_mat(name):
    if name not in mats:
        mats[name] = bpy.data.materials.new(name)
    return mats[name]


if THIN:  # one GLB: node p<k>_<class>, one primitive per donor shader (material name = shader)
    bpy.ops.object.select_all(action="DESELECT")
    rows, objs_out = [], []
    for k, p in enumerate(pieces):
        os_ = p["objs"] if isinstance(p["objs"], list) else list(p["objs"].items())
        eff = {}  # --slot-min-tris: joined-mesh key -> output slot (a slot with < SLOTMIN tris in this piece takes the largest slot's)
        if SLOTMAP:
            st = Counter()
            for mat, n in p["mt"].items():
                st[KSLOT[mat]] += n
            if SLOTMINAREA:
                ar = Counter()
                for mat, o in os_:
                    ar[KSLOT[mat]] += sum(f.area for f in o.data.polygons)
                big = max(sorted(ar), key=lambda s: ar[s])

                def small(s):
                    return st[s] < SLOTMIN and ar[s] < SLOTMINAREA
            else:
                big = max(sorted(st), key=lambda s: st[s])

                def small(s):
                    return st[s] < SLOTMIN

            if SLOTGROUP:  # grouped slots pass or fail the smallness test together (sum of the group in this piece)
                grp = [s for s in st if re.search(SLOTGROUP, s)]
                gt, ga = (
                    sum(st[s] for s in grp),
                    (sum(ar[s] for s in grp) if SLOTMINAREA else 0),
                )
                gsmall = gt < SLOTMIN and (ga < SLOTMINAREA if SLOTMINAREA else True)
                small0 = small

                def small(s):
                    return gsmall if s in grp else small0(s)

            for mat in p["mt"]:
                s = KSLOT[mat]
                eff[mat] = big if small(s) else s
            SLOTCUT[0] += len(set(KSLOT[m] for m in p["mt"])) - len(set(eff.values()))
        for mat, o in os_:
            o.data.materials.clear()
            o.data.materials.append(
                shader_mat(eff[mat] if SLOTMAP else ROCK6 if SINGLE else INTENT[mat])
            )
        o0 = os_[0][1]
        if len(os_) > 1:
            bpy.ops.object.select_all(action="DESELECT")
            for _, o in os_:
                o.select_set(True)
            bpy.context.view_layer.objects.active = o0
            bpy.ops.object.join()
        o0.name = f"p{k}_{p['kind']}"
        o0.data.name = o0.name
        assert len(o0.data.polygons) == p["tris"]
        objs_out.append(o0)
        sh = Counter()
        for mat, n in p["mt"].items():
            sh[eff[mat] if SLOTMAP else ROCK6 if SINGLE else INTENT[mat]] += n
        rows.append(
            dict(
                k=k,
                name=o0.name,
                kind=p["kind"],
                tris=p["tris"],
                min=p["min"],
                max=p["max"],
                shaders=dict(sh),
                merged=MERGED.get(k, 0),
            )
        )
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs_out:
        o.select_set(True)
    path = os.path.join(outdir, f"{TAG}.glb")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        export_yup=True,
        export_texcoords=True,
        export_normals=True,
        export_materials="EXPORT",
        use_selection=True,
    )
    json.dump(
        dict(
            src=src,
            mode="thin",
            glb=path,
            input_tris=total_in,
            output_tris=TOT,
            input_area=area_in,
            output_area=area_out,
            band=BAND,
            run=RUN,
            cube=CUBE,
            vert=VERT,
            wall=WALL,
            slope_floor=SLOPEFLOOR,
            single=SINGLE,
            run_floor=RUNF,
            run_wall=RUNW,
            merge_max=MERGEMAX,
            spec={c: v for c, v in SPEC.items()},
            uncut=UNCUT,
            uncut_area=UNCUT_AREA,
            uncut_faces=UNCUT_FACES,
            reclassed=RECLASS,
            merged=sum(MERGED.values()),
            pieces=rows,
            **(
                dict(
                    slotmap=SLOTMAP,
                    slot_min_tris=SLOTMIN,
                    slot_min_area=SLOTMINAREA,
                    slot_prims_removed=SLOTCUT[0],
                )
                if SLOTMAP
                else {}
            ),
        ),
        open(os.path.join(outdir, "manifest.json"), "w"),
        indent=1,
    )
    log(
        f"THIN {TAG}: pieces {len(rows)} {dict(Counter(r['kind'] for r in rows))} tris {total_in} -> {TOT} "
        f"area {area_in:.3f} -> {area_out:.3f}"
    )
    sys.exit(0)


def ctr(p):
    return (p["min"][0] + p["max"][0], p["min"][2] + p["max"][2])


files = []
if not CELL:
    for grp in sorted({p["group"] for p in pieces}):
        ps = sorted([p for p in pieces if p["group"] == grp], key=ctr)
        nf = math.ceil(len(ps) / MAXM)
        for f in range(nf):
            files.append((grp, ps[f * len(ps) // nf : (f + 1) * len(ps) // nf]))
else:
    order = {"solid": 0, "flat": 1, "wide": 2, "far": 3}
    ps = sorted(pieces, key=lambda p: (order[p["kind"]],) + ctr(p))
    nf = math.ceil(len(ps) / MAXM)
    for f in range(nf):
        files.append(("cell", ps[f * len(ps) // nf : (f + 1) * len(ps) // nf]))


def assign_default(ps, free):
    for p in sorted(
        ps, key=lambda p: (INTENT[next(iter(p["mt"]))] not in SCARCE, -p["tris"])
    ):
        want = INTENT[next(iter(p["mt"]))]
        slot = None
        for sh in (want, ROCK6, SAND3):
            hit = [m for m in sorted(free) if sh in DONOR[m]]
            if hit:
                slot = (hit[0], DONOR[hit[0]].index(sh))
                break
        if slot is None:
            slot = (min(free), 0)
        free.discard(slot[0])
        p["mdl"] = slot[0]
        mat = next(iter(p["mt"]))
        p["meshes"] = [
            dict(mesh=slot[1], mat=mat, shader=DONOR[slot[0]][slot[1]], tris=p["tris"])
        ]


def assign_cell(ps, free):
    left = list(ps)
    for m in (
        36,
        37,
        38,
    ):  # special MDLs: the piece with the most tris of their shaders

        def score(p):
            return sum(n for mt, n in p["mt"].items() if INTENT[mt] in DONOR[m])

        best = max(left, key=score, default=None)
        if best is not None and score(best) > 0:
            best["mdl"] = m
            free.discard(m)
            left.remove(best)
    for p in sorted(left, key=lambda p: -p["tris"]):
        par = [1, 0] if p["kind"] == "flat" else [0, 1]
        cand = [
            m for want in par for m in sorted(free) if m < 36 and m % 2 == want
        ] + sorted(free)
        p["mdl"] = cand[0]
        free.discard(cand[0])
    for p in ps:
        m = p["mdl"]
        c = len(DONOR[m])
        start = c if c > 1 and not FIRST0 else 0
        if ONEMESH:  # merge the piece's material objects into one mesh at index 0
            os_ = list(p["objs"].values())
            if len(os_) > 1:
                bpy.ops.object.select_all(action="DESELECT")
                for o in os_:
                    o.select_set(True)
                bpy.context.view_layer.objects.active = os_[0]
                bpy.ops.object.join()
            p["objs"] = {"*": os_[0]}
            p["meshes"] = [
                dict(
                    mesh=0,
                    mat="*",
                    shader=DONOR[m][0],
                    tris=p["tris"],
                    want=DONOR[m][0],
                )
            ]
            assert len(os_[0].data.polygons) == p["tris"]
            continue
        p["meshes"] = []
        for j, mat in enumerate(sorted(p["mt"])):
            want = INTENT[mat]
            sh = want if want in DONOR[m] else DONOR[m][-1]
            if start + j < c:
                sh = DONOR[m][start + j]
            p["meshes"].append(
                dict(mesh=start + j, mat=mat, shader=sh, tris=p["mt"][mat], want=want)
            )


manifest = dict(
    src=src,
    input_tris=total_in,
    output_tris=TOT,
    input_area=area_in,
    output_area=area_out,
    ext=EXT,
    mode=("bisect" if BISECT else "cell") if CELL else "default",
    uncut=UNCUT,
    split_cells=[list(c) for c in SPLIT],
    files=[],
)
for k, (grp, ps) in enumerate(files):
    free = set(range(39))
    (assign_cell if CELL else assign_default)(ps, free)
    lo = [min(p["min"][i] for p in ps) for i in range(3)]
    hi = [max(p["max"][i] for p in ps) for i in range(3)]
    cen = [(lo[i] + hi[i]) / 2 for i in range(3)]
    fillers = []
    for m in sorted(free):
        me = bpy.data.meshes.new(f"F{k}_{m}")
        x, y, z = cen  # glTF -> Blender (x, -z, y); 1 cm triangle
        me.from_pydata(
            [(x, -z, y), (x + 0.01, -z, y), (x, -z - 0.01, y)], [], [(0, 1, 2)]
        )
        me.uv_layers.new(name="UVMap")
        me.update()
        o = bpy.data.objects.new(f"F{k}_{m}", me)
        bpy.context.scene.collection.objects.link(o)
        fillers.append(
            dict(
                kind="filler",
                mdl=m,
                objs={"": o},
                tris=1,
                min=[x, y, z],
                max=[x + 0.01, y, z + 0.01],
                meshes=[dict(mesh=0, mat="", shader=DONOR[m][0], tris=1)],
            )
        )
    allp = ps + fillers
    assert len({p["mdl"] for p in allp}) == 39 == len(allp)
    bpy.ops.object.select_all(action="DESELECT")
    for p in allp:
        for e in p["meshes"]:
            o = p["objs"][e["mat"]]
            o.data.materials.clear()
            o.data.materials.append(shader_mat(e.get("want", e["shader"])))
            o.name = f"mdl{p['mdl']}_mesh{e['mesh']}"
            o.data.name = o.name
            o.select_set(True)
    path = os.path.join(outdir, f"tile_{k}.glb")
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        export_yup=True,
        export_texcoords=True,
        export_normals=True,
        export_materials="EXPORT",
        use_selection=True,
    )
    name = f"{TAG}_k{k:02d}"
    mdls = []
    for p in sorted(allp, key=lambda p: p["mdl"]):
        e0 = p["meshes"][0]
        mdls.append(
            dict(
                kind=p["kind"],
                mdl=p["mdl"],
                tris=p["tris"],
                min=p["min"],
                max=p["max"],
                meshes=p["meshes"],
                mat=e0["mat"],
                mesh=e0["mesh"],
                shader=e0["shader"],
            )
        )
    manifest["files"].append(dict(k=k, name=name, group=grp, glb=path, mdls=mdls))
    log(
        f"FILE {k} {name} {grp}: pieces {len(ps)} meshes {sum(len(p['meshes']) for p in ps)} fillers {len(fillers)}"
        f" tris {sum(p['tris'] for p in ps)} kinds {dict(Counter(p['kind'] for p in ps))}"
    )
    for p in allp:
        for o in p["objs"].values():
            o.name = f"done{k}_{o.name}"
            o.data.name = o.name
json.dump(manifest, open(os.path.join(outdir, "manifest.json"), "w"), indent=1)
log(
    "files",
    len(files),
    "pieces",
    len(pieces),
    "tris",
    total_in,
    "->",
    TOT,
    "area",
    round(area_in, 3),
    "->",
    round(area_out, 3),
)
