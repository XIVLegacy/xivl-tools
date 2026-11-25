"""Modern .mdl (LOD0 normal meshes) -> GLB.

Decoding follows Physis (GPL-3.0, https://github.com/redstrate/Physis) src/model/mod.rs
(MDL::from_existing) and src/model/vertex_declarations.rs (attribution for
translated behaviour). The GLB generator string "modern_model.py" is kept so
output stays stable across versions.
"""

import json
import struct
# ---------------------------------------------------------------- MDL decoder

VT_NAMES = {
    0: "Single1",
    1: "Single2",
    2: "Single3",
    3: "Single4",
    5: "Byte4",
    6: "Short2",
    7: "Short4",
    8: "ByteFloat4",
    9: "Short2n",
    10: "Short4n",
    13: "Half2",
    14: "Half4",
    15: "UnkPS3",
    16: "UnsignedShort2",
    17: "UnsignedShort4",
}
VU_NAMES = {
    0: "Position",
    1: "BlendWeights",
    2: "BlendIndices",
    3: "Normal",
    4: "UV",
    5: "Tangent",
    6: "BiTangent",
    7: "Color",
}
# vertex type -> (struct format, scale or None)
_VT_FMT = {
    0: ("<1f", None),
    1: ("<2f", None),
    2: ("<3f", None),
    3: ("<4f", None),
    5: ("<4B", 255.0),
    6: ("<2h", None),
    7: ("<4h", None),
    8: ("<4B", 255.0),
    9: ("<2h", 32767.0),
    10: ("<4h", 32767.0),
    13: ("<2e", None),
    14: ("<4e", None),
    15: ("<4B", 255.0),
    16: ("<2H", None),
    17: ("<4H", None),
}


class Model:
    pass


def _cstr(strings, off):
    end = strings.find(b"\0", off)
    return strings[off : end if end >= 0 else len(strings)].decode("utf-8", "replace")


def _bbox(vals):
    return {"min": list(vals[0:3]), "max": list(vals[4:7])}


def decode_mdl(data):
    """Parse header, vertex declarations, materials, bounding boxes and LOD0 meshes of a plain .mdl."""
    m = Model()
    (
        m.version,
        m.stack_size,
        m.runtime_size,
        m.vertex_declaration_count,
        m.material_count,
    ) = struct.unpack_from("<IIIHH", data, 0)
    m.vertex_offsets = struct.unpack_from("<3I", data, 16)
    m.index_offsets = struct.unpack_from("<3I", data, 28)
    m.vertex_buffer_size = struct.unpack_from("<3I", data, 40)
    m.index_buffer_size = struct.unpack_from("<3I", data, 52)
    m.lod_count, m.index_buffer_streaming, m.has_edge_geometry = struct.unpack_from(
        "<BBB", data, 64
    )
    p = 0x44

    # vertex declarations: 17 slots of 8 bytes each, terminated by stream == 0xFF
    m.declarations = []
    for _ in range(m.vertex_declaration_count):
        elems = []
        q = p
        while data[q] != 0xFF:
            stream, offset, vtype, usage, uindex = struct.unpack_from("<BBBBB", data, q)
            elems.append(
                {
                    "stream": stream,
                    "offset": offset,
                    "type": vtype,
                    "usage": usage,
                    "index": uindex,
                }
            )
            q += 8
        m.declarations.append(elems)
        p += 17 * 8

    string_count, string_size = struct.unpack_from("<H2xI", data, p)
    p += 8
    strings = data[p : p + string_size]
    p += string_size
    m.string_count = string_count
    (
        m.radius,
        mesh_count,
        attr_count,
        submesh_count,
        material_count,
        bone_count,
        bone_table_count,
        shape_count,
        shape_mesh_count,
        shape_value_count,
        lod_count,
        flags1,
        element_id_count,
        terrain_shadow_mesh_count,
        flags2,
        m.model_clip_out_of_distance,
        _shadow_clip,
        _furniture_bbox_count,
        terrain_shadow_submesh_count,
        _flags3,
        _bgchg,
        _bgcrest,
        _neck,
        _boneset,
        _u13,
        patch72,
        _u15,
        _u16,
        _u17,
    ) = struct.unpack_from("<f9HBBHBBffHHBBBBHHHHHH", data, p)
    p += struct.calcsize("<f9HBBHBBffHHBBBBHHHHHH")
    m.mesh_count = mesh_count
    m.flags1, m.flags2 = flags1, flags2

    p += element_id_count * 32
    # MeshLod (60 bytes): mesh_index, mesh_count, lod_range, tex_range, water(2), shadow(2), terrain(2),
    # fog(2), edge size, edge off, unk6, unk7, vbuf size, ibuf size, vdata off, idata off
    m.lods = []
    for _ in range(3):
        v = struct.unpack_from("<2H2f8H8I", data, p)
        m.lods.append(
            {
                "mesh_index": v[0],
                "mesh_count": v[1],
                "model_lod_range": v[2],
                "water_mesh_index": v[4],
                "water_mesh_count": v[5],
                "vertex_buffer_size": v[16],
                "index_buffer_size": v[17],
                "vertex_data_offset": v[18],
                "index_data_offset": v[19],
            }
        )
        p += 60
    if flags2 & 0x10:  # ExtraLodEnabled
        p += 3 * 40

    meshes = []
    for _ in range(mesh_count):
        (vc, ic, mat, sub_i, sub_c, bt, start, o0, o1, o2, s0, s1, s2, nstreams) = (
            struct.unpack_from("<H2xIHHHHI3I3BB", data, p)
        )
        meshes.append(
            {
                "vertex_count": vc,
                "index_count": ic,
                "material_index": mat,
                "start_index": start,
                "offsets": (o0, o1, o2),
                "strides": (s0, s1, s2),
                "stream_count": nstreams,
            }
        )
        p += 36
    p += attr_count * 4
    p += terrain_shadow_mesh_count * 20
    p += submesh_count * 16
    p += terrain_shadow_submesh_count * 12
    mat_offsets = struct.unpack_from("<%dI" % material_count, data, p)
    p += material_count * 4
    p += bone_count * 4
    if m.version <= 0x1000005:
        p += bone_table_count * 132
    else:
        counts = struct.unpack_from("<%dH" % (2 * bone_table_count), data, p)[1::2]
        p += bone_table_count * 4
        for c in counts:
            p += c * 2
            p = (p + 3) & ~3
    p += shape_count * 16 + shape_mesh_count * 12 + shape_value_count * 4
    (bone_map_size,) = struct.unpack_from("<I", data, p)
    p += 4 + bone_map_size
    p += patch72 * 16
    pad = data[p]
    p += 1 + pad
    boxes = [struct.unpack_from("<8f", data, p + 32 * i) for i in range(4)]
    m.bbox = _bbox(boxes[0])
    m.model_bbox = _bbox(boxes[1])
    m.water_bbox = _bbox(boxes[2])
    m.fog_bbox = _bbox(boxes[3])
    m.header_end = p + 128

    m.materials = [_cstr(strings, o) for o in mat_offsets]

    # LOD0 normal meshes
    lod0 = m.lods[0]
    m.used_types = set()
    m.meshes = []
    for j in range(lod0["mesh_index"], lod0["mesh_index"] + lod0["mesh_count"]):
        mesh = meshes[j]
        decl = m.declarations[j]
        vc = mesh["vertex_count"]
        vbase = m.vertex_offsets[0] or lod0["vertex_data_offset"]
        pos, nrm, uv = [None] * vc, None, None
        col = None  # Color index 0 (ByteFloat4), raw bytes per vertex; decoded only, build_glb does not write it yet
        for e in decl:
            m.used_types.add(
                (
                    VU_NAMES.get(e["usage"], e["usage"]),
                    VT_NAMES.get(e["type"], e["type"]),
                )
            )
            if e["usage"] == 7 and e["index"] == 0 and e["type"] in (5, 8):
                c0 = vbase + mesh["offsets"][e["stream"]] + e["offset"]
                cs = mesh["strides"][e["stream"]]
                col = [tuple(data[c0 + cs * k : c0 + cs * k + 4]) for k in range(vc)]
        for e in decl:
            usage, vtype = e["usage"], e["type"]
            if usage not in (0, 3, 4) or (
                usage == 4 and e["index"] != 0 and uv is not None
            ):
                continue
            if vtype not in _VT_FMT:
                raise ValueError(f"vertex type {vtype} unsupported")
            fmt, scale = _VT_FMT[vtype]
            if usage == 4 and e["index"] != 0:
                continue
            stride = mesh["strides"][e["stream"]]
            first = vbase + mesh["offsets"][e["stream"]] + e["offset"]
            out = []
            for k in range(vc):
                vals = struct.unpack_from(fmt, data, first + stride * k)
                if scale:
                    vals = tuple(v / scale for v in vals)
                out.append(vals)
            if usage == 0:
                if vtype not in (2, 3, 14):
                    raise ValueError(
                        f"position type {VT_NAMES.get(vtype, vtype)} unsupported"
                    )
                pos = [v[:3] for v in out]
            elif usage == 3:
                if vtype in (5, 8, 15):  # unsigned byte normals -> [-1, 1]
                    out = [tuple(c * 2.0 - 1.0 for c in v[:3]) for v in out]
                nrm = [v[:3] for v in out]
            else:
                if vtype in (6, 7, 16, 17):
                    raise ValueError(
                        f"uv type {VT_NAMES.get(vtype, vtype)} unsupported"
                    )
                uv = [v[:2] if len(v) >= 2 else (v[0], 0.0) for v in out]
        ibase = m.index_offsets[0] or lod0["index_data_offset"]
        ic = mesh["index_count"]
        indices = list(
            struct.unpack_from("<%dH" % ic, data, ibase + mesh["start_index"] * 2)
        )
        mat_i = mesh["material_index"]
        m.meshes.append(
            {
                "mesh_index": j,
                "positions": pos,
                "normals": nrm,
                "uvs": uv,
                "indices": indices,
                "material_index": mat_i,
                "colors": col,
                "material": m.materials[mat_i]
                if mat_i < len(m.materials)
                else f"material_{mat_i}",
            }
        )
    return m


def check_model(m, tol=1e-3):
    """Position-in-bbox and index-range checks against the header bounding boxes."""
    mn = [1e30] * 3
    mx = [-1e30] * 3
    verts = tris = 0
    index_ok = True
    for mesh in m.meshes:
        verts += len(mesh["positions"])
        tris += len(mesh["indices"]) // 3
        if mesh["indices"] and max(mesh["indices"]) >= len(mesh["positions"]):
            index_ok = False
        for p in mesh["positions"]:
            for a in range(3):
                mn[a] = min(mn[a], p[a])
                mx[a] = max(mx[a], p[a])
    out = {
        "verts": verts,
        "tris": tris,
        "index_ok": index_ok,
        "bbox_decoded": {"min": mn, "max": mx},
    }
    # Half4 positions round to ~2^-11 of their magnitude, so large models can overshoot the float header
    # box by more than 1e-3; inside_*_half allows that rounding (2^-10 * largest |coordinate|).
    slack = tol + 2.0**-10 * max(abs(v) for v in mn + mx)
    for name, box in (("bbox", m.bbox), ("model_bbox", m.model_bbox)):
        over = max(
            [box["min"][a] - mn[a] for a in range(3)]
            + [mx[a] - box["max"][a] for a in range(3)]
        )
        out["inside_" + name] = over <= tol
        out["inside_" + name + "_half"] = over <= slack
        out["max_outside_" + name] = max(over, 0.0)
    return out


# ---------------------------------------------------------------- GLB writer / checker


def _pad4(b, fill=b"\0"):
    return b + fill * (-len(b) % 4)


def build_glb(m, name):
    binbuf = bytearray()
    views, accessors, prims, materials, mat_ids = [], [], [], [], {}

    def add_view(raw, target):
        binbuf.extend(b"\0" * (-len(binbuf) % 4))
        views.append(
            {
                "buffer": 0,
                "byteOffset": len(binbuf),
                "byteLength": len(raw),
                "target": target,
            }
        )
        binbuf.extend(raw)
        return len(views) - 1

    for mesh in m.meshes:
        pos, idx = mesh["positions"], mesh["indices"]
        if not pos or len(idx) < 3:
            continue
        idx = idx[: len(idx) // 3 * 3]
        mn = [min(p[a] for p in pos) for a in range(3)]
        mx = [max(p[a] for p in pos) for a in range(3)]
        attrs = {}
        accessors.append(
            {
                "bufferView": add_view(
                    b"".join(struct.pack("<3f", *p) for p in pos), 34962
                ),
                "componentType": 5126,
                "count": len(pos),
                "type": "VEC3",
                "min": mn,
                "max": mx,
            }
        )
        attrs["POSITION"] = len(accessors) - 1
        if mesh["normals"]:
            raw = bytearray()
            for n in mesh["normals"]:
                ln = (n[0] ** 2 + n[1] ** 2 + n[2] ** 2) ** 0.5
                n = tuple(c / ln for c in n) if ln > 1e-9 else (0.0, 1.0, 0.0)
                raw += struct.pack("<3f", *n)
            accessors.append(
                {
                    "bufferView": add_view(bytes(raw), 34962),
                    "componentType": 5126,
                    "count": len(pos),
                    "type": "VEC3",
                }
            )
            attrs["NORMAL"] = len(accessors) - 1
        if mesh["uvs"]:
            accessors.append(
                {
                    "bufferView": add_view(
                        b"".join(struct.pack("<2f", *t) for t in mesh["uvs"]), 34962
                    ),
                    "componentType": 5126,
                    "count": len(pos),
                    "type": "VEC2",
                }
            )
            attrs["TEXCOORD_0"] = len(accessors) - 1
        wide = len(pos) > 65535
        raw = struct.pack("<%d%s" % (len(idx), "I" if wide else "H"), *idx)
        accessors.append(
            {
                "bufferView": add_view(raw, 34963),
                "componentType": 5125 if wide else 5123,
                "count": len(idx),
                "type": "SCALAR",
            }
        )
        mname = mesh["material"]
        if mname not in mat_ids:
            mat_ids[mname] = len(materials)
            materials.append(
                {
                    "name": mname,
                    "pbrMetallicRoughness": {"metallicFactor": 0.0},
                    "doubleSided": True,
                }
            )
        prims.append(
            {
                "attributes": attrs,
                "indices": len(accessors) - 1,
                "material": mat_ids[mname],
                "mode": 4,
            }
        )
    if not prims:
        raise ValueError("model has no drawable LOD0 meshes")
    doc = {
        "asset": {"version": "2.0", "generator": "modern_model.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": name, "mesh": 0}],
        "meshes": [{"name": name, "primitives": prims}],
        "materials": materials,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{"byteLength": len(binbuf)}],
    }
    js = _pad4(json.dumps(doc, separators=(",", ":")).encode("ascii"), b" ")
    bn = _pad4(bytes(binbuf))
    total = 12 + 8 + len(js) + 8 + len(bn)
    return (
        struct.pack("<III", 0x46546C67, 2, total)
        + struct.pack("<II", len(js), 0x4E4F534A)
        + js
        + struct.pack("<II", len(bn), 0x004E4942)
        + bn
    )


def verify_glb(blob):
    """Re-parse a GLB: header/chunk lengths, accessor ranges, then recompute tris and bbox from the BIN."""
    magic, ver, total = struct.unpack_from("<III", blob, 0)
    assert magic == 0x46546C67 and ver == 2 and total == len(blob), (
        "bad GLB header/length"
    )
    jl, jt = struct.unpack_from("<II", blob, 12)
    assert jt == 0x4E4F534A and jl % 4 == 0
    doc = json.loads(blob[20 : 20 + jl])
    bp = 20 + jl
    bl, bt = struct.unpack_from("<II", blob, bp)
    assert bt == 0x004E4942 and bl % 4 == 0 and bp + 8 + bl == len(blob), (
        "bad BIN chunk"
    )
    bn = blob[bp + 8 : bp + 8 + bl]
    assert doc["buffers"][0]["byteLength"] <= bl
    mn, mx, tris, verts = [1e30] * 3, [-1e30] * 3, 0, 0
    for prim in doc["meshes"][0]["primitives"]:
        pa = doc["accessors"][prim["attributes"]["POSITION"]]
        ia = doc["accessors"][prim["indices"]]
        pv, iv = (
            doc["bufferViews"][pa["bufferView"]],
            doc["bufferViews"][ia["bufferView"]],
        )
        assert (
            pv["byteOffset"] + pv["byteLength"] <= bl
            and iv["byteOffset"] + iv["byteLength"] <= bl
        )
        n = pa["count"]
        assert pv["byteLength"] == n * 12
        for k in range(n):
            x = struct.unpack_from("<3f", bn, pv["byteOffset"] + 12 * k)
            for a in range(3):
                mn[a], mx[a] = min(mn[a], x[a]), max(mx[a], x[a])
        fmt, sz = ("I", 4) if ia["componentType"] == 5125 else ("H", 2)
        assert iv["byteLength"] == ia["count"] * sz and ia["count"] % 3 == 0
        ids = struct.unpack_from("<%d%s" % (ia["count"], fmt), bn, iv["byteOffset"])
        assert max(ids) < n, "index out of range"
        for key in ("NORMAL", "TEXCOORD_0"):
            if key in prim["attributes"]:
                a = doc["accessors"][prim["attributes"][key]]
                assert a["count"] == n
        tris += ia["count"] // 3
        verts += n
    return {
        "glb_bytes": len(blob),
        "json_chunk": jl,
        "bin_chunk": bl,
        "primitives": len(doc["meshes"][0]["primitives"]),
        "verts": verts,
        "tris": tris,
        "bbox": {"min": mn, "max": mx},
    }
