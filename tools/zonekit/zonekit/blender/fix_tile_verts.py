"""Make tile vertex data retail-like (crash bisection after test R: xiv1tt writer is fine, our vertex data is not).

blender -b --python fix_tile_verts.py -- <tiles-dir> <out-dir>
- UV0: each UV island shifted by whole tiles so its centre sits in 0..1 (textures repeat, so it looks the same).
- UV1 (lightmap set): planar X/Z projection over the tile box, always inside 0..1 (we had 0 or a copy of UV0).
- tangents exported (we had none).
"""

import bpy
import bmesh
import sys
import os
import glob
import re
import math

a = sys.argv[sys.argv.index("--") + 1 :]
src, out = a[0], a[1]
os.makedirs(out, exist_ok=True)
for f in sorted(glob.glob(os.path.join(src, "tile_*.glb"))):
    n = re.search(r"tile_(\d+)", f).group(1)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=f)
    objs = [o for o in bpy.data.objects if o.type == "MESH"]
    xs = [v.co.x for o in objs for v in o.data.vertices]
    ys = [v.co.y for o in objs for v in o.data.vertices]
    x0, x1, y0, y1 = (
        min(xs),
        max(xs),
        min(ys),
        max(ys),
    )  # Blender Z-up: glTF X/Z -> Blender X/-Y
    span = max(x1 - x0, y1 - y0, 1e-3)
    worst = 0.0
    for o in objs:
        me = o.data
        while len(me.uv_layers) > 1:
            me.uv_layers.remove(me.uv_layers[1])
        if not me.uv_layers:
            me.uv_layers.new(name="UV0")
        bm = bmesh.new()
        bm.from_mesh(me)
        uv = bm.loops.layers.uv[0]
        # islands: faces joined by an edge whose loops share UVs on both ends
        parent = {fc.index: fc.index for fc in bm.faces}

        def find(i):
            while parent[i] != i:
                parent[i] = parent[parent[i]]
                i = parent[i]
            return i

        for e in bm.edges:
            lf = e.link_faces
            if len(lf) != 2:
                continue
            fa, fb = lf

            def uvs(fc):
                return {
                    lp.vert.index: tuple(round(c, 5) for c in lp[uv].uv)
                    for lp in fc.loops
                    if lp.vert in e.verts
                }

            if uvs(fa) == uvs(fb):
                parent[find(fa.index)] = find(fb.index)
        groups = {}
        for fc in bm.faces:
            groups.setdefault(find(fc.index), []).append(fc)
        for fcs in groups.values():
            us = [lp[uv].uv.x for fc in fcs for lp in fc.loops]
            vs = [lp[uv].uv.y for fc in fcs for lp in fc.loops]
            du = math.floor((min(us) + max(us)) / 2)
            dv = math.floor((min(vs) + max(vs)) / 2)
            for fc in fcs:
                for lp in fc.loops:
                    lp[uv].uv.x -= du
                    lp[uv].uv.y -= dv
        for fc in bm.faces:
            for lp in fc.loops:
                worst = max(worst, abs(lp[uv].uv.x), abs(lp[uv].uv.y))
        uv1 = bm.loops.layers.uv.new("UV1")
        for fc in bm.faces:
            for lp in fc.loops:
                lp[uv1].uv = ((lp.vert.co.x - x0) / span, (lp.vert.co.y - y0) / span)
        bm.to_mesh(me)
        bm.free()
    bpy.ops.export_scene.gltf(
        filepath=os.path.join(out, f"tile_{n}.glb"),
        export_format="GLB",
        export_yup=True,
        export_texcoords=True,
        export_normals=True,
        export_tangents=True,
        export_materials="EXPORT",
    )
    print("FIXVERTS tile", n, "max |uv0|", round(worst, 2))
