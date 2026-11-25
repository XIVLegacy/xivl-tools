"""Cut a modern scene FBX into the zone's tile grid, one mesh per real material (zonekit tiles build).

blender -b -P tiles_build.py -- <r2t1.fbx> <out-dir> --params params.json

- --fix-ground-winding: faces of ground-kind materials (grn|bgs|snw|ter) whose geometric normal points down
  (game ny < -0.3) are re-wound to face up, and their down-pointing corner normals negated (G5b; flat ground
  decals drew dark in game). Flip counts per material go to the manifest stats.

Same geometry and frame as the G4 chain (build_section city cut + outside shell -> shift into tile 24 ->
build_ground_city bgs add-back -> build_city_tiles), done in one pass:
- Faces (centroid of the FBX polygon, game units before the Y offset) inside the shell X -620..580,
  Y -5000..5000, Z -580..620. The G2 outside piece was this shell minus the city box X -200..160, Y -60..120,
  Z -140..180, so city box + outside = the shell. SKIP (build_section) on material OR object name.
- bgs ground (build_ground_city, material matches `bgs`, no SKIP test): candidates in the same shell; a face is
  dropped when its centroid and its three inner points all have a kept non-bgs surface within --cover m
  vertically. One pass against the final non-bgs geometry (G4 did two passes: G1 inside the city box against
  the city cut only, G2 everywhere against city + G1 ground + outside).
- Y - 5.47 (build_section --offset 0 5.47 0). Polygons are triangulated (Blender loop triangles); a triangle
  with an edge shorter than 1e-5 m is dropped (build_section welded at 1e-5).
- Tile = 7x7 grid cell of the triangle centroid (world X -200..160, Z -140..180); the clamped cell, or the
  nearest active cell centre when that cell is inactive (build_city_tiles tile_of). Positions in the tile's
  local frame (minus the cell centre in X/Z), which is what tile-24 frame + build_city_tiles' shift gives.
- Per tile: one object per material, object = mesh = material name (Blender `.NNN` suffix removed). Plain
  materials (no textures). Source corner normals kept (custom normals), UV = active layer (+ a second layer
  when the source has one). Writes <out-dir>/tile_<n>.glb and <out-dir>/manifest.json (triangle counts read
  back from the written GLBs; the glTF exporter drops a few zero-area triangles, as it did for G4).
"""

import bpy
import sys
import re
import os
import json
import time
import struct
import numpy as np
from mathutils import Vector
from mathutils.bvhtree import BVHTree

T0 = time.time()


def log(*a):
    print(f"[{time.time() - T0:7.1f}s]", *a, flush=True)


a = sys.argv[sys.argv.index("--") + 1 :]
fbx, outdir = a[0], a[1]

P_ = json.load(
    open(a[a.index("--params") + 1], encoding="utf-8")
)  # zonekit: zone settings (zonekit/tiles.py build)
COVER = float(P_["cover"])
# --fix-ground-winding: ground-kind faces (material matches GROUND) whose geometric normal points down (game ny < -0.3)
# get their winding reversed so they face up; corner normals pointing down on those faces are negated too.
FIX_GROUND = bool(P_["fix_ground"])
GROUND = re.compile(P_["ground"], re.I)
GROUND_NY = -0.3
SKIP = re.compile(P_["skip"], re.I)  # build_section.py lines 18-19
BGS = re.compile(P_["bgs"], re.I)  # build_ground_city --pattern bgs
SHELL = tuple(float(v) for v in P_["shell"])  # game units, before the Y offset
YS = P_["ys"]
X0, X1, NX, Z0, Z1, NZ = (
    float(P_["x0"]),
    float(P_["x1"]),
    P_["nx"],
    float(P_["z0"]),
    float(P_["z1"]),
    P_["nz"],
)
SX, SZ = (X1 - X0) / NX, (Z1 - Z0) / NZ
ACTIVE = [n for n in range(NX * NZ) if n not in P_["inactive"]]
CENT = np.array([(X0 + (n % NX + 0.5) * SX, Z0 + (n // NX + 0.5) * SZ) for n in ACTIVE])
WELD = 1e-5


def base(n):
    return re.sub(
        r"\.mtrl$", "", re.sub(r"\.\d{3}$", "", n), flags=re.I
    )  # FBX names end in .mtrl


os.makedirs(outdir, exist_ok=True)


def fget(coll, attr, n, width, dtype=np.float32):
    buf = np.empty(n * width, dtype=dtype)
    coll.foreach_get(attr, buf)
    return buf.reshape(n, width) if width > 1 else buf


bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=fbx, global_scale=100.0)
fbx_objs = [o for o in bpy.data.objects if o.type == "MESH"]
log("FBX imported", len(fbx_objs), "mesh objects")

mat_id = {}  # base material name -> id
P, N, U0, U1, M, KIND = (
    [],
    [],
    [],
    [],
    [],
    [],
)  # per object arrays; KIND 0 = kept face, 1 = bgs candidate
bgs_polys = []  # per bgs candidate polygon: (list of world verts, object chunk index, polygon index)
TRI_POLY = []  # per chunk: polygon index of each kept triangle
stats = {
    "fbx polygons": 0,
    "selected polygons": 0,
    "bgs candidate polygons": 0,
    "mirrored objects": 0,
}
has_uv1 = False
for o in fbx_objs:
    me = o.data
    nv, npoly = len(me.vertices), len(me.polygons)
    if not npoly:
        continue
    stats["fbx polygons"] += npoly
    mw = np.array(o.matrix_world, dtype=np.float64)
    R, t = mw[:3, :3], mw[:3, 3]
    co = fget(me.vertices, "co", nv, 3).astype(np.float64) @ R.T + t
    cen = fget(me.polygons, "center", npoly, 3).astype(np.float64) @ R.T + t
    gx, gy, gz = cen[:, 0], cen[:, 2], -cen[:, 1]  # Blender -> game (x, z, -y)
    shell = (
        (SHELL[0] <= gx)
        & (gx <= SHELL[1])
        & (SHELL[2] <= gy)
        & (gy <= SHELL[3])
        & (SHELL[4] <= gz)
        & (gz <= SHELL[5])
    )
    shell_post = (
        (SHELL[0] <= gx)
        & (gx <= SHELL[1])
        & (SHELL[2] <= gy - YS)
        & (gy - YS <= SHELL[3])
        & (SHELL[4] <= gz)
        & (gz <= SHELL[5])
    )  # build_ground_city tests Y after the offset
    names = [m.name if m else "-" for m in me.materials] or ["-"]
    mi = np.clip(
        fget(me.polygons, "material_index", npoly, 1, np.int32), 0, len(names) - 1
    )
    skip = np.array([bool(SKIP.search(n)) for n in names])[mi] | bool(
        SKIP.search(o.name)
    )
    isbgs = np.array([bool(BGS.search(n)) for n in names])[mi]
    sel_keep = shell & ~skip
    sel_bgs = shell_post & isbgs
    if not (sel_keep.any() or sel_bgs.any()):
        continue
    stats["selected polygons"] += int(sel_keep.sum())
    ids = np.array(
        [mat_id.setdefault(base(n), len(mat_id)) for n in names], dtype=np.int32
    )
    me.calc_loop_triangles()
    nt = len(me.loop_triangles)
    tl = fget(me.loop_triangles, "loops", nt, 3, np.int32)
    tv = fget(me.loop_triangles, "vertices", nt, 3, np.int32)
    tp = fget(me.loop_triangles, "polygon_index", nt, 1, np.int32)
    tk = sel_keep[tp] | sel_bgs[tp]
    tl, tv, tp = tl[tk], tv[tk], tp[tk]
    if (
        np.linalg.det(R) < 0
    ):  # mirrored instance: keep the winding consistent with the normals
        stats["mirrored objects"] += 1
        tl, tv = tl[:, [0, 2, 1]], tv[:, [0, 2, 1]]
    nl = len(me.loops)
    nrm = fget(me.corner_normals, "vector", nl, 3).astype(np.float64) @ np.linalg.inv(R)
    nrm /= np.maximum(np.linalg.norm(nrm, axis=1, keepdims=True), 1e-12)
    uvl = me.uv_layers
    act = uvl.active
    uv0 = fget(act.data, "uv", nl, 2) if act else np.zeros((nl, 2), np.float32)
    other = [u for u in uvl if act is None or u.name != act.name]
    uv1 = fget(other[0].data, "uv", nl, 2) if other else np.zeros((nl, 2), np.float32)
    has_uv1 |= bool(other)
    P.append(co[tv])
    N.append(nrm[tl])
    U0.append(uv0[tl])
    U1.append(uv1[tl])
    M.append(ids[mi[tp]])
    KIND.append(sel_bgs[tp].astype(np.int8))
    TRI_POLY.append(tp)
    chunk = len(P) - 1
    for pi in np.nonzero(sel_bgs)[0]:
        p = me.polygons[pi]
        bgs_polys.append(
            (
                [Vector(co[me.loops[li].vertex_index]) for li in p.loop_indices],
                chunk,
                int(pi),
            )
        )
stats["bgs candidate polygons"] = len(bgs_polys)
log("gathered", stats, "materials", len(mat_id))

# bgs add-back: BVH over every kept non-bgs triangle (Blender world coords, Z up; vertical rays only)
keep_tris = np.concatenate([p[k == 0] for p, k in zip(P, KIND)])
flat = keep_tris.reshape(-1, 3)
bvh = BVHTree.FromPolygons(
    flat.tolist(), np.arange(len(flat)).reshape(-1, 3).tolist(), all_triangles=True
)
del flat
log("BVH built", len(keep_tris), "tris")
UP, DOWN = Vector((0, 0, 1)), Vector((0, 0, -1))


def covered(p):
    return (
        bvh.ray_cast(p, UP, COVER)[0] is not None
        or bvh.ray_cast(p, DOWN, COVER)[0] is not None
    )


kept_polys = set()
for vs, chunk, pi in bgs_polys:
    cen = sum(vs, Vector()) / len(vs)
    samples = [cen] + [
        (2 * vs[i] + vs[(i + 1) % len(vs)] + vs[(i + 2) % len(vs)]) / 4
        for i in range(min(3, len(vs)))
    ]
    if not all(covered(s) for s in samples):
        kept_polys.add((chunk, pi))
stats["bgs kept polygons"] = len(kept_polys)
bvh = None
for c in range(len(P)):
    k = KIND[c]
    if not k.any():
        continue
    ok = (k == 0) | np.array([(c, int(pi)) in kept_polys for pi in TRI_POLY[c]])
    P[c], N[c], U0[c], U1[c], M[c], KIND[c] = (
        P[c][ok],
        N[c][ok],
        U0[c][ok],
        U1[c][ok],
        M[c][ok],
        KIND[c][ok],
    )
P, N, U0, U1, M, KIND = (np.concatenate(x) for x in (P, N, U0, U1, M, KIND))
stats["bgs kept tris"] = int(KIND.sum())

# weld rule: drop triangles with an edge shorter than WELD
e = np.stack(
    [np.linalg.norm(P[:, i] - P[:, (i + 1) % 3], axis=1) for i in range(3)], axis=1
)
ok = e.min(axis=1) >= WELD
stats["degenerate tris dropped"] = int((~ok).sum())
P, N, U0, U1, M, KIND = P[ok], N[ok], U0[ok], U1[ok], M[ok], KIND[ok]
log("bgs + weld", stats)

if FIX_GROUND:  # Blender import frame here: Z is game up, so game ny = cross(...).z
    names_now = sorted(mat_id, key=mat_id.get)
    ground_ids = np.array([bool(GROUND.search(nm)) for nm in names_now])
    fn = np.cross(P[:, 1] - P[:, 0], P[:, 2] - P[:, 0])
    fn /= np.maximum(np.linalg.norm(fn, axis=1, keepdims=True), 1e-12)
    flip = ground_ids[M] & (fn[:, 2] < GROUND_NY)
    P[flip] = P[flip][:, [0, 2, 1]]
    N[flip] = N[flip][:, [0, 2, 1]]
    U0[flip] = U0[flip][:, [0, 2, 1]]
    U1[flip] = U1[flip][:, [0, 2, 1]]
    ndown = flip[:, None] & (
        N[:, :, 2] < 0
    )  # corner normals still pointing down on flipped faces
    N[ndown] = -N[ndown]
    flips = {
        names_now[m]: int(c) for m, c in zip(*np.unique(M[flip], return_counts=True))
    }
    stats["ground winding flips"] = dict(sorted(flips.items(), key=lambda kv: -kv[1]))
    stats["ground corner normals negated"] = int(ndown.sum())
    log(
        "FIX_GROUND flipped",
        int(flip.sum()),
        "faces",
        stats["ground winding flips"],
        "corner normals negated",
        int(ndown.sum()),
    )

# free the FBX data so the real material names are available for the export materials
bpy.data.batch_remove(list(bpy.data.objects))
bpy.data.batch_remove(list(bpy.data.meshes))
bpy.data.batch_remove(list(bpy.data.materials))
bpy.data.batch_remove(list(bpy.data.images))
names = sorted(mat_id, key=mat_id.get)
mats = {}
for n in names:
    m = bpy.data.materials.new(n)
    assert m.name == n, (m.name, n)
    mats[n] = m

# tile of the triangle centroid (build_city_tiles tile_of); world game X = Blender x, Z = -Blender y
c3 = P.mean(axis=1)
wx, wz = c3[:, 0], -c3[:, 1]
cx = np.clip(np.floor((wx - X0) / SX), 0, NX - 1).astype(np.int32)
cz = np.clip(np.floor((wz - Z0) / SZ), 0, NZ - 1).astype(np.int32)
tile = cz * NX + cx
act_mask = np.isin(tile, ACTIVE)
far = ~act_mask
if far.any():
    d = (CENT[None, :, 0] - wx[far, None]) ** 2 + (
        CENT[None, :, 1] - wz[far, None]
    ) ** 2
    tile[far] = np.array(ACTIVE)[d.argmin(axis=1)]
log("tiles assigned", len(P), "tris")


def make_object(name, mat, p, n, u0, u1):
    uniq, inv = np.unique(
        p.reshape(-1, 3).astype(np.float32), axis=0, return_inverse=True
    )
    inv = inv.reshape(-1, 3)
    ok = (
        (inv[:, 0] != inv[:, 1]) & (inv[:, 1] != inv[:, 2]) & (inv[:, 0] != inv[:, 2])
    )  # float32-degenerate
    if not ok.all():  # the glTF exporter would drop these; drop them here so the manifest matches the GLB
        p, n, u0, u1 = p[ok], n[ok], u0[ok], u1[ok]
        uniq, inv = np.unique(
            p.reshape(-1, 3).astype(np.float32), axis=0, return_inverse=True
        )
    nt = len(p)
    q = uniq[inv.reshape(-1, 3)].astype(np.float64)
    zero = int(
        (
            np.linalg.norm(np.cross(q[:, 1] - q[:, 0], q[:, 2] - q[:, 0]), axis=1)
            < 1e-9
        ).sum()
    )
    stats["zero-area tris (float32)"] = stats.get("zero-area tris (float32)", 0) + zero
    dup = nt - len(np.unique(np.sort(inv.reshape(-1, 3), axis=1), axis=0))
    stats["duplicate tris (same 3 verts)"] = stats.get(
        "duplicate tris (same 3 verts)", 0
    ) + int(dup)
    me = bpy.data.meshes.new(name)
    me.vertices.add(len(uniq))
    me.vertices.foreach_set("co", uniq.ravel())
    me.loops.add(nt * 3)
    me.loops.foreach_set("vertex_index", inv.astype(np.int32).ravel())
    me.polygons.add(nt)
    me.polygons.foreach_set("loop_start", np.arange(0, nt * 3, 3, dtype=np.int32))
    me.update(calc_edges=True)
    me.polygons.foreach_set("use_smooth", np.ones(nt, dtype=bool))
    me.uv_layers.new(name="UVMap").data.foreach_set(
        "uv", u0.reshape(-1).astype(np.float32)
    )
    if has_uv1:
        me.uv_layers.new(name="UVMap2").data.foreach_set(
            "uv", u1.reshape(-1).astype(np.float32)
        )
    me.normals_split_custom_set(n.reshape(-1, 3).astype(np.float32))
    me.materials.append(mat)
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    assert ob.name == name and me.name == name, (ob.name, me.name, name)
    return ob, nt


def glb_counts(
    path,
):  # per-mesh triangle counts from the GLB JSON chunk (mesh name = material name)
    with open(path, "rb") as f:
        f.read(12)
        ln = struct.unpack("<I", f.read(4))[0]
        f.read(4)
        j = json.loads(f.read(ln))
    return {
        m["name"]: sum(
            j["accessors"][p["indices"]]["count"] // 3 for p in m["primitives"]
        )
        for m in j["meshes"]
    }


manifest = {"source": fbx, "cover": COVER, "stats": stats, "tiles": {}, "city": {}}
city = {}
order = np.argsort(tile, kind="stable")
bounds = np.searchsorted(tile[order], np.arange(NX * NZ + 1))
for n in ACTIVE:
    idx = order[bounds[n] : bounds[n + 1]]
    cxn, czn = CENT[ACTIVE.index(n)]
    shift = np.array(
        [cxn, -czn, YS]
    )  # local game (x - cx, y - 5.47, z - cz) in Blender axes
    objs, per = [], {}
    mi_t = M[idx]
    for mid in np.unique(mi_t):
        sel = idx[mi_t == mid]
        nm = names[mid]
        ob, nt = make_object(nm, mats[nm], P[sel] - shift, N[sel], U0[sel], U1[sel])
        objs.append(ob)
        per[nm] = nt
        stats["float32-degenerate tris dropped"] = (
            stats.get("float32-degenerate tris dropped", 0) + len(sel) - nt
        )
    path = os.path.join(outdir, f"tile_{n}.glb")
    if objs:
        bpy.ops.object.select_all(action="DESELECT")
        for ob in objs:
            ob.select_set(True)
        bpy.ops.export_scene.gltf(
            filepath=path,
            export_format="GLB",
            export_yup=True,
            export_texcoords=True,
            export_normals=True,
            export_tangents=False,
            export_materials="EXPORT",
            use_selection=True,
        )
        built = sum(per.values())
        per = glb_counts(
            path
        )  # the exporter drops a few triangles itself: count what was written
        stats["dropped by glTF exporter"] = (
            stats.get("dropped by glTF exporter", 0) + built - sum(per.values())
        )
        for nm in per:
            city[nm] = city.get(nm, 0) + per[nm]
    manifest["tiles"][str(n)] = {
        "tris": sum(per.values()),
        "meshes": len(objs),
        "materials": dict(sorted(per.items(), key=lambda kv: -kv[1])),
    }
    log(f"TILE {n} meshes {len(objs)} tris {sum(per.values())}")
    meshes = [ob.data for ob in objs]
    bpy.data.batch_remove(objs)
    bpy.data.batch_remove(meshes)
manifest["city"] = {
    "tris": sum(city.values()),
    "tiles": len(ACTIVE),
    "materials": len(city),
    "per_material": dict(sorted(city.items(), key=lambda kv: -kv[1])),
}
with open(os.path.join(outdir, "manifest.json"), "w", encoding="utf-8") as f:
    json.dump(manifest, f, indent=1)
log("DONE city tris", sum(city.values()), "materials", len(city), stats)
