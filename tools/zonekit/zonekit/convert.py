"""Tile GLB -> 1.23b background model, one MDL per GLB node, with checks.

Steps: (unless prefixed) Blender vertex fix (UV0 islands into 0..1, planar UV1,
tangents) -> strip Blender name suffixes -> normalise tangents; optional
material remap; xiv1tt model import onto a donor with --split-mdl; rename
w0d0_tub_ed1_h to the given 14-character name; MESH HEAD bytes 4-7 from the
donor table or a fixed value. Checks: shader fallbacks; one MDL box per node
within 2 cm of the manifest; one component per MDL on its own skeleton node;
triangle count; round-trip area within 0.1 %; tools/research/census_wrb.py
parses the file. Writes <scratch>/result.json.
"""

import hashlib
import json
import math
import os
import re
import shutil
import struct
import subprocess

from . import tools, wrb

OLD = b"w0d0_tub_ed1_h"
SHADERS = [
    "w0d_b0_metl1_1h",
    "w0d_b0_wood1_1h",
    "w0d_c0_jrix1_2h",
    "w0d_c0_rock4_2h",
    "w0d_c0_rock5_4h",
    "w0d_c0_rock6_2h",
    "w0d_c0_rock7_3h",
    "w0d_c0_sand3_4h",
    "w0d_c0_sdjr1_2h",
    "zzz_toumei",
] + [f"w0d_c0_fnd{i:02d}_2h" for i in range(1, 82)]


# ---- GLB helpers -----------------------------------------------------------------------------------------------


def _split(d):
    jl = struct.unpack_from("<I", d, 12)[0]
    return json.loads(d[20 : 20 + jl]), d[20 + jl :]


def _join(d, js, rest):
    j = json.dumps(js, separators=(",", ":")).encode()
    j += b" " * (-len(j) % 4)
    return (
        d[:8]
        + struct.pack("<I", 12 + 8 + len(j) + len(rest))
        + struct.pack("<I", len(j))
        + b"JSON"
        + j
        + rest
    )


def strip_suffix(d):
    """Rename nodes/meshes 'name.001' -> 'name' (glb_strip_suffix.py)."""
    js, rest = _split(d)
    n = 0
    for coll in ("nodes", "meshes"):
        for o in js.get(coll, []):
            if "name" in o and re.search(r"\.\d{3}$", o["name"]):
                o["name"] = o["name"][:-4]
                n += 1
    names = [o["name"] for o in js["nodes"] if "mesh" in o]
    assert len(names) == len(set(names)), "duplicate node names after rename"
    return _join(d, js, rest), n


def fix_tangents(d):
    """Unit-length xyz and w = +-1 for every tangent; drop tangent min/max (fix_glb_tangents.py)."""
    b = bytearray(d)
    jl = struct.unpack_from("<I", b, 12)[0]
    j = json.loads(b[20 : 20 + jl])
    base = 20 + jl + 8
    fixed = 0
    for m in j["meshes"]:
        for pr in m["primitives"]:
            if "TANGENT" not in pr["attributes"]:
                continue
            a = j["accessors"][pr["attributes"]["TANGENT"]]
            bv = j["bufferViews"][a["bufferView"]]
            off = base + bv.get("byteOffset", 0) + a.get("byteOffset", 0)
            st = bv.get("byteStride", 16)
            for k in range(a["count"]):
                x, y, z, w = struct.unpack_from("<4f", b, off + k * st)
                ln = math.sqrt(x * x + y * y + z * z)
                if not (abs(ln - 1) < 0.005 and abs(abs(w) - 1) < 1e-6) or any(
                    map(math.isnan, (x, y, z, w))
                ):
                    if ln > 1e-6 and not math.isnan(ln):
                        x, y, z = x / ln, y / ln, z / ln
                    else:
                        x, y, z = 1.0, 0.0, 0.0
                    w = -1.0 if w < 0 else 1.0
                    struct.pack_into("<4f", b, off + k * st, x, y, z, w)
                    fixed += 1
            a.pop("min", None)
            a.pop("max", None)
    # the original rewrote only the binary values; its JSON (min/max popped) was never written back
    return bytes(b), fixed


def remap_materials(d, remap):
    js, rest = _split(d)
    n_mat = 0
    for m in js.get("materials", []):
        to = remap.get(m.get("name"), remap.get("*"))
        if to and m.get("name") != to:
            m["name"] = to
            n_mat += 1
    n_prim = sum(
        1
        for me in js["meshes"]
        for pr in me["primitives"]
        if js["materials"][pr["material"]]["name"] in remap.values()
    )
    return _join(d, js, rest), dict(
        map=remap, materials_renamed=n_mat, primitives_on_targets=n_prim
    )


def parse_remap(spec):
    def full(tok):
        hit = [s for s in SHADERS if s == tok or f"_{tok}_" in s]
        if len(hit) != 1:
            raise SystemExit(f"--remap: '{tok}' matches {hit}")
        return hit[0]

    return dict(
        (
            "*" if x.split("=")[0] == "all" else full(x.split("=")[0]),
            full(x.split("=")[1]),
        )
        for x in (spec or "").split(",")
        if x
    )


def glb_nodes(path):
    """Manifest stand-in from the GLB itself: node name -> vertex box, total tris."""
    with open(path, "rb") as f:
        dd = f.read()
    j, _ = _split(dd)
    rows, tris = [], 0
    for nd in j["nodes"]:
        if "mesh" not in nd:
            continue
        lo, hi = [1e9] * 3, [-1e9] * 3
        for pr in j["meshes"][nd["mesh"]]["primitives"]:
            x = j["accessors"][pr["attributes"]["POSITION"]]
            lo = [min(lo[q], x["min"][q]) for q in range(3)]
            hi = [max(hi[q], x["max"][q]) for q in range(3)]
            tris += j["accessors"][pr["indices"]]["count"] // 3
        rows.append(dict(name=nd["name"], min=lo, max=hi))
    return dict(src=path, output_tris=tris, pieces=rows)


def glb_area(path):
    with open(path, "rb") as f:
        dd = f.read()
    jl = struct.unpack_from("<I", dd, 12)[0]
    j = json.loads(dd[20 : 20 + jl])
    base = 20 + jl + 8

    def acc(i):
        x = j["accessors"][i]
        bv = j["bufferViews"][x["bufferView"]]
        off = base + bv.get("byteOffset", 0) + x.get("byteOffset", 0)
        fmt = {5125: "I", 5123: "H", 5121: "B", 5126: "f"}[x["componentType"]]
        n = {"SCALAR": 1, "VEC3": 3}[x["type"]]
        st = bv.get("byteStride", struct.calcsize(fmt) * n)
        return [
            struct.unpack_from("<" + fmt * n, dd, off + k * st)
            for k in range(x["count"])
        ]

    def mat(nd):
        if "matrix" in nd:
            m = nd["matrix"]
            return [[m[c * 4 + r] for c in range(4)] for r in range(4)]
        x, y, z, w = nd.get("rotation", [0, 0, 0, 1])
        s = nd.get("scale", [1, 1, 1])
        t = nd.get("translation", [0, 0, 0])
        R = [
            [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
            [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
            [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
        ]
        return [[R[r][c] * s[c] for c in range(3)] + [t[r]] for r in range(3)] + [
            [0, 0, 0, 1]
        ]

    def mul(A, B):
        return [
            [sum(A[r][k] * B[k][c] for k in range(4)) for c in range(4)]
            for r in range(4)
        ]

    world = {}

    def walk(i, M):
        M = mul(M, mat(j["nodes"][i]))
        world[i] = M
        for c in j["nodes"][i].get("children", []):
            walk(c, M)

    for sc in j.get("scenes", [{"nodes": list(range(len(j["nodes"])))}]):
        for i in sc["nodes"]:
            walk(i, [[float(r == c) for c in range(4)] for r in range(4)])
    area = 0.0
    tris = 0
    for i, M in world.items():
        nd = j["nodes"][i]
        if "mesh" not in nd:
            continue
        for pr in j["meshes"][nd["mesh"]]["primitives"]:
            P = [
                [
                    sum(M[r][c] * (v[c] if c < 3 else 1) for c in range(4))
                    for r in range(3)
                ]
                for v in acc(pr["attributes"]["POSITION"])
            ]
            tri_idx = [x[0] for x in acc(pr["indices"])]
            tris += len(tri_idx) // 3
            for t in range(0, len(tri_idx) - 2, 3):
                A, B, C = P[tri_idx[t]], P[tri_idx[t + 1]], P[tri_idx[t + 2]]
                u = [B[q] - A[q] for q in range(3)]
                v = [C[q] - A[q] for q in range(3)]
                area += 0.5 * math.sqrt(
                    (u[1] * v[2] - u[2] * v[1]) ** 2
                    + (u[2] * v[0] - u[0] * v[2]) ** 2
                    + (u[0] * v[1] - u[1] * v[0]) ** 2
                )
    return area, tris


# ---- model helpers ---------------------------------------------------------------------------------------------


def _mesh_fields(d):
    """(shader, absolute offset of MESH HEAD bytes 4-7) for every mesh."""
    out = []
    for m in wrb.load(d):
        for me in m.meshes:
            ks = me.chunk.children
            head = next(k for k in ks if k.tag == b"HEAD")
            s = next(k for k in ks if k.tag == b"STR\0")
            out.append(
                (
                    d[s.body : s.at + s.size].split(b"\0")[0].decode("latin1"),
                    head.body + 4,
                )
            )
    return out


def patch_material(d, donor=None, value=None):
    """MESH HEAD bytes 4-7 = the donor's value for the mesh's shader, or a fixed value (patch_mesh_material.py).
    Only those fields change; checked."""
    out = bytearray(d)
    ms = _mesh_fields(d)
    if value is not None:
        table = {name: value for name, _ in ms}
        msg = f"fixed value {value} for all {len(ms)} meshes"
    else:
        table = {}
        for name, at in _mesh_fields(donor):
            v = struct.unpack_from(">I", donor, at)[0]
            if table.setdefault(name, v) != v:
                raise SystemExit(f"donor maps {name} to both {table[name]} and {v}")
        msg = "donor table: " + ", ".join(
            f"{k}={v}" for k, v in sorted(table.items(), key=lambda kv: kv[1])
        )
    changed = 0
    for name, at in ms:
        if name not in table:
            raise SystemExit(f"shader {name} not in the donor table")
        if struct.unpack_from(">I", d, at)[0] != table[name]:
            struct.pack_into(">I", out, at, table[name])
            changed += 1
    allowed = bytearray(len(d))
    for _, at in ms:
        allowed[at : at + 4] = b"\1" * 4
    diff = [i for i in range(len(d)) if out[i] != d[i]]
    assert len(out) == len(d) and not [i for i in diff if not allowed[i]], (
        "bytes outside MESH HEAD 4-7 differ"
    )
    return bytes(
        out
    ), f"{msg}; {len(ms)} meshes, {changed} fields changed, {len(diff)} bytes differ"


# ---- convert ---------------------------------------------------------------------------------------------------


def convert_tile(
    glb,
    man,
    scratch,
    dst,
    name,
    donor,
    prefixed=False,
    patch_value=None,
    remap=None,
    inp=None,
):
    """Returns the result dict (also written to <scratch>/result.json); its 'problems' list is empty when all checks pass."""
    NAME = name.encode()
    if len(NAME) != len(OLD):
        raise SystemExit("--name must be 14 bytes like w0d0_tub_ed1_h")
    if os.path.exists(dst):
        raise SystemExit("exists, not overwriting: " + dst)
    inp = inp or man["src"]
    dirs = {k: os.path.join(scratch, k) for k in ("in", "fix", "raw", "census")}
    for d in dirs.values():
        os.makedirs(d, exist_ok=True)
    bad, res = [], {}

    # 1-3: vertex fix, strip suffixes, tangents
    g = os.path.join(scratch, "strip.glb")
    if prefixed:
        shutil.copyfile(glb, g)
        res["fixverts"] = ["skipped (--prefixed)"]
    else:
        shutil.copyfile(glb, os.path.join(dirs["in"], "tile_0.glb"))
        log = tools.run(
            [
                tools.setting("blender"),
                "-b",
                "-P",
                tools.blender_script("fix_tile_verts.py"),
                "--",
                dirs["in"],
                dirs["fix"],
            ]
        )
        res["fixverts"] = [lp for lp in log.splitlines() if lp.startswith("FIXVERTS")]
        with open(os.path.join(dirs["fix"], "tile_0.glb"), "rb") as f:
            data, _ = strip_suffix(f.read())
        data, _ = fix_tangents(data)
        with open(g, "wb") as f:
            f.write(data)

    # 3b: material remap
    if remap:
        with open(g, "rb") as f:
            data, res["remap"] = remap_materials(f.read(), remap)
        with open(g, "wb") as f:
            f.write(data)

    # 4: import
    raw = os.path.join(dirs["raw"], "import.DAT")
    if os.path.exists(raw):
        os.remove(raw)
    log = tools.run(
        [tools.setting("xiv1tt"), "model", "import", donor, g, "-o", raw, "--split-mdl"]
    )
    with open(os.path.join(dirs["raw"], "import.txt"), "w") as f:
        f.write(log)
    fallback = [lp for lp in log.splitlines() if "no donor mesh with that shader" in lp]
    res["fallbacks"] = len(fallback)
    res["fallback_examples"] = fallback[:5]
    if "left unchanged" in log:
        bad.append("import log: left unchanged")
    node_of = {
        int(m.group(1)): m.group(2)
        for m in re.finditer(r"mdl(\d+) '[^']*' <- node '([^']+)'", log)
    }

    # 5: rename
    with open(raw, "rb") as f:
        b0 = f.read()
    n_old = b0.count(OLD)
    b1 = b0.replace(OLD, NAME)
    assert len(b1) == len(b0) and b1.count(NAME) == n_old and NAME not in b0
    res["rename_occurrences"] = n_old

    # 6: material index patch
    with open(donor, "rb") as f:
        dn = f.read()
    d, res["patch"] = patch_material(
        b1, None if patch_value is not None else dn, patch_value
    )
    with open(dst, "xb") as f:
        f.write(d)
    res["bytes"] = len(d)
    res["sha256"] = hashlib.sha256(d).hexdigest().upper()

    # checks: boxes (show prints 4 decimals; compared at that precision as before)
    boxes = [
        tuple(round(x, 4) for x in m.aabb.v) for m in wrb.load(d) if m.aabb is not None
    ]
    pieces = {p["name"]: p for p in man["pieces"]}
    res["mdl_boxes"] = len(boxes)
    res["nodes"] = len(pieces)
    if len(boxes) != len(pieces):
        bad.append(f"{len(boxes)} MDL boxes vs {len(pieces)} pieces")
    expected = 4 + 4 * len(pieces)
    if n_old != expected:
        bad.append(f"name occurrences {n_old} != 4 + 4 x {len(pieces)} = {expected}")
    mism = 0
    for k, bx in enumerate(boxes):
        p = pieces.get(node_of.get(k, ""))
        if p is None:
            bad.append(f"mdl{k}: no node mapping")
            continue
        if any(
            abs(bx[i] - p["min"][i]) > 0.02 or abs(bx[i + 3] - p["max"][i]) > 0.02
            for i in range(3)
        ):
            mism += 1
    res["box_mismatches"] = mism
    if mism:
        bad.append(f"{mism} MDL boxes differ from the manifest by > 2 cm")

    # checks: components
    info = tools.run([tools.setting("xiv1tt"), "info", dst])
    with open(os.path.join(dirs["raw"], "info.txt"), "w") as f:
        f.write(info)
    comps = re.findall(
        r"comp(\d+): node (\d+)( '[^']*'| \(out of range\)), mdl(\d+)", info
    )
    cn = re.search(r"components: (\d+)", info)
    res["components"] = int(cn.group(1)) if cn else -1
    nodes = [int(c[1]) for c in comps]
    res["component_nodes_distinct"] = len(set(nodes)) == len(nodes)
    res["component_out_of_range"] = sum(1 for c in comps if "out of range" in c[2])
    res["component_mdl_is_k"] = all(int(c[0]) == int(c[3]) for c in comps)
    if (
        res["components"] != len(pieces)
        or not res["component_nodes_distinct"]
        or res["component_out_of_range"]
        or not res["component_mdl_is_k"]
    ):
        bad.append(
            "MICT components wrong: "
            + json.dumps(
                {
                    k: res[k]
                    for k in (
                        "components",
                        "component_nodes_distinct",
                        "component_out_of_range",
                        "component_mdl_is_k",
                    )
                }
            )
        )
    sk = re.search(r"skeleton: (\d+) bones", info)
    res["skeleton_bones"] = int(sk.group(1)) if sk else None
    res["mesh_lines"] = len(re.findall(r"^\s+mesh\d+: shader", info, re.M))
    res["tris_dat"] = sum(
        int(x)
        for x in re.findall(
            r"^\s+mesh\d+: shader \S+, \d+ verts, (\d+) tris", info, re.M
        )
    )
    if res["tris_dat"] != man["output_tris"]:
        bad.append(f"DAT tris {res['tris_dat']} != manifest {man['output_tris']}")

    # checks: round-trip area (xiv1tt's export drops some degenerate triangles, so its count can be lower)
    back = os.path.join(dirs["raw"], "back.glb")
    if os.path.exists(back):
        os.remove(back)
    tools.run(
        [
            tools.setting("xiv1tt"),
            "model",
            "export",
            dst,
            "-o",
            back,
            "--skeleton",
            "none",
        ]
    )
    a_out, t_out = glb_area(back)
    a_in, t_in = glb_area(inp)
    res.update(
        area_in=a_in,
        area_dat=a_out,
        area_dev=(a_out - a_in) / a_in,
        tris_in=t_in,
        tris_export=t_out,
        tris_manifest=man["output_tris"],
    )
    if abs(res["area_dev"]) > 0.001:
        bad.append(f"area deviation {100 * res['area_dev']:+.4f}%")

    # checks: independent research parser on a one-file client root
    croot = dirs["census"]
    cd = os.path.join(croot, "data", "FF", "FF", "33")
    os.makedirs(cd, exist_ok=True)
    shutil.copyfile(dst, os.path.join(cd, "00.DAT"))
    r = subprocess.run(
        [tools.PY, "-I", tools.CENSUS, "--client-root", croot],
        capture_output=True,
        text=True,
    )
    txt = r.stdout + r.stderr
    with open(os.path.join(dirs["raw"], "census_wrb.txt"), "w") as f:
        f.write(txt)
    res["census_rc"] = r.returncode
    res["census_head"] = txt.strip().splitlines()[:12]
    ok = (
        r.returncode == 0
        and "parsed_RES=1 malformed_RES=0 read_errors=0 mesh_failures=0" in txt
        and re.search(
            r"anomaly RES_clamped_entries=0 max_clamp_bytes=0 additional=0", txt
        )
        and "out_of_range_indices=0" in txt
    )
    res["census_ok"] = bool(ok)
    if not ok:
        bad.append("census_wrb.py: parse problem (see raw/census_wrb.txt)")

    res["problems"] = bad
    with open(os.path.join(scratch, "result.json"), "w") as f:
        json.dump(res, f, indent=1)
    return res
