"""Command line: zonekit <group> <command> ..."""

import argparse
import hashlib
import json
import os

from . import audio, collision, extract, layout, models, tools, zone


def read(path):
    with open(path, "rb") as f:
        return f.read()


def check_target(path):
    full = os.path.normcase(os.path.abspath(path))
    for ro in tools.read_only():
        ro = os.path.normcase(ro)
        if full == ro or full.startswith(ro + os.sep):
            raise SystemExit("refusing to write under a retail install: " + path)


def report(path):
    data = read(path)
    print(
        "wrote %s  %d bytes  SHA-256 %s"
        % (path, len(data), hashlib.sha256(data).hexdigest().upper())
    )


def write_new(path, data):
    """Write a new file; refuses to overwrite or to write under a retail install. Prints size and SHA-256."""
    check_target(path)
    parent = os.path.dirname(os.path.abspath(path))
    os.makedirs(parent, exist_ok=True)
    try:
        with open(path, "xb") as f:
            f.write(data)
    except FileExistsError:
        raise SystemExit("exists, not overwriting: " + path)
    print(
        "wrote %s  %d bytes  SHA-256 %s"
        % (path, len(data), hashlib.sha256(data).hexdigest().upper())
    )


def cmd_audio_bgm(a):
    write_new(a.out, audio.bgm_to_1x(read(a.scd), read(a.template)))
    print("read-back equals the decoded stream")


def cmd_audio_se(a):
    write_new(a.out, audio.se_to_1x(read(a.scd), a.entry, read(a.template)))
    print("read-back equals the source entry")


def cmd_audio_info(a):
    for line in audio.info(read(a.file)):
        print(line)


def cmd_audio_preview(a):
    d = read(a.scd)
    if a.entry is None:
        write_new(a.out, audio.read_vorbis(d)["ogg"])
    else:
        write_new(a.out, audio.se_wav(d, a.entry))


def _specs(items, first_is_offset):
    out = []
    for s in items:
        parts = s.split("=")
        if len(parts) != 3:
            raise SystemExit("expected A=KEY=FILEID, got: " + s)
        a, key, fid = parts
        out.append((int(a, 16) if first_is_offset else a, key, int(fid, 16)))
    return out


def cmd_layout_rows_list(a):
    d = read(a.layout)
    rs = layout.rows(d)
    tags = {}
    for r in rs:
        tags[r[2]] = tags.get(r[2], 0) + 1
    print("%d rows %s" % (len(rs), tags))
    if a.model:
        by_key = {r[1]: r for r in rs}
        for key, name in layout.model_texture_keys(read(a.model)):
            r = by_key.get(key)
            print(
                "TEX %-16s %-28s -> %s"
                % (
                    key,
                    name,
                    "row %#x file %#010x" % (r[0], r[3]) if r else "NOT IN LAYOUT",
                )
            )
        return
    for o, key, tag, fid, flags in rs:
        print("%#06x %-4s %#010x %-16s %s" % (o, tag, fid, key, flags.hex()))


def cmd_layout_rows_set(a):
    data, log = layout.set_rows(read(a.layout), _specs(a.rows, True))
    print("\n".join(log))
    write_new(a.out, data)


def cmd_layout_rows_add(a):
    data, msg = layout.add_rows(read(a.layout), _specs(a.rows, False))
    print(msg)
    write_new(a.out, data)


def cmd_collision_phb(a):
    template = a.template or zone.load(a.zone)["donors"]["phb_template"]
    tris, verts = collision.geometry_counts(a.geometry)
    print(
        "geometry %d triangles, %d vertices (cap %d each)"
        % (tris, verts, collision.CAP)
    )
    if max(tris, verts) > collision.CAP:
        raise SystemExit("over the client cap; split or prune the geometry first")
    check_target(a.out)
    out = collision.phb(template, a.geometry, a.out, a.grid, a.capacity)
    if out:
        print(out)
    report(a.out)


def _client(a):
    if not (a.sqpack or a.zone):
        raise SystemExit("give --zone or --sqpack (the modern client)")
    return extract.Client(a.sqpack or zone.load(a.zone)["sources"]["modern_sqpack"])


def _source(a, path):
    """A local file if it exists, else a game path read from the modern client."""
    return read(path) if os.path.isfile(path) else _client(a).read(path)


def cmd_extract_file(a):
    paths = list(a.paths)
    if a.list:
        with open(a.list, encoding="utf-8") as f:
            paths += [line.strip() for line in f if line.strip()]
    client = _client(a)
    for p in paths:
        write_new(os.path.join(a.out, *p.split("/")), client.read(p))
    print("extracted %d files" % len(paths))


def cmd_extract_instances(a):
    path = a.lgb or zone.load(a.zone)["sources"]["bg_path"] + "/level/bg.lgb"
    inst = extract.lgb_instances(_source(a, path))
    bg = [o for o in inst if "mdl" in o]
    print(
        "%d BG instances (%d with collision, %d unique pcb), %d shared-group placements"
        % (
            len(bg),
            sum(1 for o in bg if o["pcb"]),
            len({o["pcb"] for o in bg if o["pcb"]}),
            len(inst) - len(bg),
        )
    )
    write_new(a.out, extract.to_json(inst))


def cmd_extract_sgb(a):
    with open(a.instances, encoding="utf-8") as f:
        inst = json.load(f)
    client = None if a.sgb_root else _client(a)
    rd = (
        (lambda p: read(os.path.join(a.sgb_root, *p.split("/"))))
        if a.sgb_root
        else client.read
    )
    models = extract.sgb_models(inst, rd)
    n = sum(len(layer["objects"]) for layers in models.values() for layer in layers)
    print("%d shared groups, %d objects" % (len(models), n))
    write_new(a.out, extract.to_json(models))


def cmd_extract_props(a):
    with open(a.instances, encoding="utf-8") as f:
        inst = json.load(f)
    with open(a.sgb_models, encoding="utf-8") as f:
        models = json.load(f)
    rows, missing = extract.placements(inst, models, a.euler)
    print(
        "%d rows (%d vfx), %d unique models, euler %s"
        % (
            len(rows),
            sum(r["vfx"] for r in rows),
            len({r["model"] for r in rows if not r["vfx"]}),
            a.euler,
        )
    )
    if missing:
        print("unresolved shared groups:", missing)
    write_new(a.out, extract.to_json(rows))


def cmd_extract_model(a):
    m = models.decode_mdl(_source(a, a.model))
    name = a.model.replace("\\", "/").rsplit("/", 1)[-1].rsplit(".", 1)[0]
    chk = models.check_model(m)
    print(
        "%d meshes, %d verts, %d tris, materials %s, inside header box %s"
        % (
            len(m.meshes),
            chk["verts"],
            chk["tris"],
            m.materials,
            chk["inside_bbox_half"],
        )
    )
    blob = models.build_glb(m, name)
    models.verify_glb(blob)
    write_new(a.out, blob)


def cmd_layout_author(a):
    from . import layout_author

    args = list(a.args)
    out = None
    if "--check" not in args:
        if len(args) < 3:
            raise SystemExit(
                "layout author: give <in.DAT> <slots.json|none> <out.DAT> [options]"
            )
        out = args[2]
        check_target(out)
    code = layout_author.run(["zonekit layout author"] + args, zone.load(a.zone))
    if out and os.path.exists(out):
        report(out)
    return code


def _load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def _parts(spec):
    return [_load_json(p) for p in spec.split(",") if p]


def cmd_collision_fix(a):
    from . import collision_build as cb

    z = zone.load(a.zone)
    check_target(a.out)
    spawn = (
        [float(v) for v in a.spawn.split(",")]
        if a.spawn
        else z["collision"]["spawn_layout"]
    )
    target = z["collision"]["tile"] if a.target is None else a.target
    out, stats = cb.fix(
        _load_json(a.packed),
        _parts(a.keep or ""),
        z,
        target,
        a.sliver,
        a.weld,
        a.drop,
        a.height,
        a.seg,
        a.max,
        a.walk,
        a.tol,
        [float(v) for v in a.bridge.split(",") if v],
        a.bridge_tol,
        spawn,
        a.cap_reach,
        a.prune,
    )
    print("fix", stats)
    write_new(a.out, cb.dumps(out))


def cmd_collision_patch(a):
    from . import collision_build as cb

    z = zone.load(a.zone)
    check_target(a.out)
    col = _load_json(a.col)
    plates = cb.terrain_plates(_client(a), z["sources"]["bg_path"], models.decode_mdl)
    plates = (
        plates - [0, z["frames"]["source_y_shift"], 0] + z["frames"]["collision_offset"]
    )
    floor, line = cb.patch_holes(col, plates, a.cover, a.clear)
    print(line)
    write_new(a.out + ".floor.json", cb.dumps(floor))
    res, log = cb.spurious_walls(
        floor, _parts(a.parts), z["frames"]["collision_offset"], a.step
    )
    print("\n".join(log))
    if len(res["triangles"]) > a.max:
        raise SystemExit("over the cap: %d triangles" % len(res["triangles"]))
    write_new(a.out, cb.dumps(res))


def cmd_collision_walls(a):
    from . import collision_build as cb

    z = zone.load(a.zone)
    res, log = cb.spurious_walls(
        _load_json(a.col), _parts(a.parts), z["frames"]["collision_offset"], a.step
    )
    print("\n".join(log))
    write_new(a.out, cb.dumps(res))


def cmd_collision_objects(a):
    import re
    from . import collision_build as cb

    z = zone.load(a.zone)
    if a.objects_root:

        def rd(p):
            f = os.path.join(a.objects_root, *p.split("/"))
            return read(f) if os.path.exists(f) else None
    else:
        client = _client(a)

        def rd(p):
            try:
                return client.read(p)
            except extract.ExtractError:
                return None

    ysh = -z["frames"]["source_y_shift"] if a.yshift is None else a.yshift

    def rx(s):
        return re.compile(s, re.I) if s else None

    out, line = cb.objects(
        _load_json(a.instances),
        rd,
        ysh,
        a.conv,
        a.cluster,
        rx(a.box),
        a.box_min_real,
        a.box_shrink,
        rx(a.cluster_only),
        a.unbox_extent,
    )
    print(line)
    write_new(a.out, cb.dumps(out))


def cmd_collision_merge(a):
    from . import collision_build as cb

    off = zone.load(a.zone)["frames"]["collision_offset"]
    res = cb.merge(_load_json(a.base), [(_load_json(p), off) for p in a.add])
    print("%d triangles, %d vertices" % (len(res["triangles"]), len(res["vertices"])))
    write_new(a.out, cb.dumps(res))


def cmd_convert_tile(a):
    from . import convert

    check_target(a.out)
    donor = a.donor
    if not donor:
        if not a.zone:
            raise SystemExit("give --donor or --zone")
        z = zone.load(a.zone)
        donor = os.path.join(
            z["sources"]["pristine_client"], "data", z["donors"]["tile_model"] + ".DAT"
        )
    man = convert.glb_nodes(a.glb) if a.manifest == "-" else _load_json(a.manifest)
    res = convert.convert_tile(
        a.glb,
        man,
        a.scratch,
        a.out,
        a.name,
        donor,
        a.prefixed,
        a.patch_value,
        convert.parse_remap(a.remap),
        a.input,
    )
    print(json.dumps({k: v for k, v in res.items() if k != "census_head"}, indent=1))
    report(a.out)
    if res["problems"]:
        print("PROBLEMS:", res["problems"])
        return 1


def cmd_tiles_shade(a):
    from . import tiles

    check_target(a.out)
    occ = []
    if a.neighbours:
        occ = tiles.neighbours(zone.load(a.zone), a.tile, a.neighbours)
    st = tiles.shade(
        a.glb,
        a.out,
        occ,
        a.rays,
        a.dist,
        a.floor,
        a.gamma,
        a.edge_ref,
        a.json or a.out + ".json",
    )
    print(
        "occluder tiles %d; grey mean %.1f, %.1f %% under 200, min %.0f"
        % (len(occ) // 2, st["mean"], st["under200"], st["min"])
    )
    report(a.out)


def cmd_tiles_build(a):
    from . import tiles

    check_target(a.out)
    z = zone.load(a.zone)
    for line in tiles.build(
        a.fbx or z["sources"]["fbx"], a.out, z, a.cover, a.fix_ground_winding
    ):
        print(line)


def cmd_tiles_terrain(a):
    import re
    from . import collision_build as cb, tiles

    z = zone.load(a.zone)
    check_target(a.out)
    client = _client(a)
    sgb = a.sgb if a.sgb is not None else z["tiles"].get("terrain_sgb")
    if sgb and not a.placements:
        raise SystemExit("--sgb needs --placements (extract props output)")
    plates = cb.terrain_plates_full(client, z["sources"]["bg_path"], models.decode_mdl)
    tiles.terrain(
        a.tiles,
        a.out,
        z,
        plates,
        a.cover,
        a.check,
        re.compile(sgb) if sgb else None,
        _load_json(a.placements) if sgb else None,
        lambda p: models.decode_mdl(client.read(p)),
    )


def cmd_tiles_cut(a):
    from . import tiles

    z = zone.load(a.zone)
    check_target(a.out)
    args = list(a.opts) or (
        z["tiles"]["cut_args"] + ["--slotmap", z["tiles"]["slotmap"]]
    )
    for line in tiles.cut(a.glb, a.out, a.tag, z, args):
        print(line)
    report(os.path.join(a.out, a.tag + ".glb"))


def cmd_zone_new(a):
    print("wrote", zone.new(a.name, a.copy_from))


def cmd_zone_show(a):
    zone.show(zone.load(a.name))


def parser():
    p = argparse.ArgumentParser(
        prog="zonekit",
        description="Build FFXIV 1.0 zones. Every command writes new files only.",
    )
    groups = p.add_subparsers(dest="group", required=True)

    z = groups.add_parser(
        "zone", help="per-zone settings (zones/<name>/zone.json)"
    ).add_subparsers(dest="cmd", required=True)
    c = z.add_parser("new", help="start a zone by copying another zone's settings")
    c.add_argument("name")
    c.add_argument(
        "--from", dest="copy_from", required=True, help="zone whose settings to copy"
    )
    c.set_defaults(fn=cmd_zone_new)
    c = z.add_parser("show", help="check a zone's settings and print the derived grid")
    c.add_argument("name")
    c.set_defaults(fn=cmd_zone_show)

    s = groups.add_parser(
        "audio", help="modern .scd sounds -> 1.0 sound files"
    ).add_subparsers(dest="cmd", required=True)
    c = s.add_parser(
        "bgm", help="music or zone ambience (Vorbis) onto a stock 1.0 version-2 file"
    )
    c.add_argument("scd", help="modern .scd (one Vorbis entry)")
    c.add_argument(
        "--template",
        required=True,
        help=r"stock 1.0 file, e.g. 07\FE\00\02.DAT (music), 8A\26\00\C8.DAT (ambience)",
    )
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_audio_bgm)
    c = s.add_parser(
        "se", help="one mono MS-ADPCM spot sound onto a stock one-entry 1.0 file"
    )
    c.add_argument("scd", help="modern spot-sound .scd")
    c.add_argument(
        "--entry", type=int, default=0, help="entry index (default 0, usually the loop)"
    )
    c.add_argument(
        "--template",
        required=True,
        help=r"stock one-entry 1.0 file, e.g. 8A\26\00\12.DAT",
    )
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_audio_se)
    c = s.add_parser("info", help="list the audio entries of a sound file")
    c.add_argument("file")
    c.set_defaults(fn=cmd_audio_info)
    c = s.add_parser(
        "preview", help="write a playable .ogg (music) or .wav (--entry N, spot sound)"
    )
    c.add_argument("scd")
    c.add_argument("--entry", type=int, help="spot-sound entry; omit for music")
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_audio_preview)

    lay = groups.add_parser("layout", help="1.0 layout files").add_subparsers(
        dest="cmd", required=True
    )
    r = lay.add_parser(
        "rows", help="resource rows (texture/model/collision ids)"
    ).add_subparsers(dest="sub", required=True)
    c = r.add_parser(
        "list",
        help="list rows, or with --model check a model's texture keys against them",
    )
    c.add_argument("layout")
    c.add_argument("--model", help="1.0 model file whose texture keys to look up")
    c.set_defaults(fn=cmd_layout_rows_list)
    c = r.add_parser("set", help="rewrite key and file id of existing rows")
    c.add_argument("layout")
    c.add_argument(
        "rows",
        nargs="+",
        metavar="OFFSET=KEY=FILEID",
        help="row offset hex (from list), key, file id hex",
    )
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_layout_rows_set)
    c = r.add_parser(
        "add", help="append rows (flags copied from the first row of the same tag)"
    )
    c.add_argument("layout")
    c.add_argument(
        "rows",
        nargs="+",
        metavar="TAG=KEY=FILEID",
        help="tag (bxt/brt/bhp), key, file id hex",
    )
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_layout_rows_add)
    c = lay.add_parser(
        "author",
        help="town layout from a retail one: tile carriers, resident hosts, rows (see layout_author.py)",
    )
    c.add_argument("zone", help="zone whose settings to use")
    c.add_argument(
        "args",
        nargs=argparse.REMAINDER,
        help="<in.DAT> <slots.json|none> <out.DAT> [options] | --check <in> <out>",
    )
    c.set_defaults(fn=cmd_layout_author)

    col = groups.add_parser("collision", help="walkable collision").add_subparsers(
        dest="cmd", required=True
    )
    c = col.add_parser("phb", help="geometry JSON -> 1.0 collision file (PHB)")
    c.add_argument("geometry", help="collision geometry JSON (vertices + triangles)")
    src = c.add_mutually_exclusive_group(required=True)
    src.add_argument("--zone", help="take the PHB template from this zone's settings")
    src.add_argument("--template", help="native PHB to use as template")
    c.add_argument("--grid", type=int, default=8, help="bucket grid N (N^3), default 8")
    c.add_argument(
        "--capacity", type=int, default=1000, help="triangles per tree, default 1000"
    )
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_collision_phb)
    c = col.add_parser(
        "fix",
        help="clean packed collision, weld floors, add edge walls; output in the collision tile frame",
    )
    c.add_argument("packed", help="packed collision JSON, layout frame")
    c.add_argument("--keep", help="comma list of part JSONs whose walls are real")
    c.add_argument("--zone", required=True)
    c.add_argument(
        "--target", type=int, help="collision tile (default: zone collision.tile)"
    )
    c.add_argument(
        "--spawn", help="x,y,z layout frame (default: zone collision.spawn_layout)"
    )
    c.add_argument("--sliver", type=float, default=0.2)
    c.add_argument("--weld", type=float, default=0.3)
    c.add_argument("--drop", type=float, default=0.6)
    c.add_argument("--height", type=float, default=2.0)
    c.add_argument("--seg", type=float, default=1.5)
    c.add_argument("--max", type=int, default=65000)
    c.add_argument("--walk", type=float, default=0.7)
    c.add_argument("--tol", type=float, default=0.15)
    c.add_argument(
        "--bridge", default="", help="e.g. 1.8,2.5: fill wider same-height gaps"
    )
    c.add_argument("--bridge-tol", type=float, default=0.3)
    c.add_argument("--cap-reach", type=float, default=0.6)
    c.add_argument("--prune", type=float, nargs="?", const=3.0, default=None)
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_collision_fix)
    c = col.add_parser(
        "patch",
        help="fill floor holes from the modern terrain plates, then drop spurious walls",
    )
    c.add_argument("col", help="collision JSON, collision tile frame")
    c.add_argument(
        "--parts", required=True, help="comma list of source part JSONs (layout frame)"
    )
    c.add_argument("--zone", required=True)
    c.add_argument("--sqpack", help="modern client (default: zone setting)")
    c.add_argument("--cover", type=float, default=0.35)
    c.add_argument("--clear", type=float, default=2.0)
    c.add_argument("--step", type=float, default=0.35)
    c.add_argument("--max", type=int, default=65000)
    c.add_argument(
        "-o",
        "--out",
        required=True,
        help="also writes <out>.floor.json (before the wall pass)",
    )
    c.set_defaults(fn=cmd_collision_patch)
    c = col.add_parser(
        "walls",
        help="drop generated walls with floor on both sides (invisible walls in open ground)",
    )
    c.add_argument("col")
    c.add_argument("--parts", required=True)
    c.add_argument("--zone", required=True)
    c.add_argument("--step", type=float, default=0.35)
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_collision_walls)
    c = col.add_parser(
        "objects", help="object collision (.pcb per instance) placed into layout space"
    )
    c.add_argument(
        "instances", help="instances JSON (extract instances) or a hand-made subset"
    )
    c.add_argument("--zone", required=True)
    c.add_argument("--sqpack")
    c.add_argument(
        "--objects-root",
        help="read .pcb from an extracted folder instead of the client",
    )
    c.add_argument(
        "--yshift", type=float, help="default: minus the zone's source_y_shift"
    )
    c.add_argument("--conv", choices=["A", "B"], default="A")
    c.add_argument("--cluster", type=float, default=0.0)
    c.add_argument("--cluster-only")
    c.add_argument("--box")
    c.add_argument("--box-min-real", type=int, default=0)
    c.add_argument("--box-shrink", type=float, default=1.0)
    c.add_argument("--unbox-extent", type=float, default=0.0)
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_collision_objects)
    c = col.add_parser(
        "merge", help="append layout-frame parts to a collision-frame file"
    )
    c.add_argument("base")
    c.add_argument("--add", action="append", required=True)
    c.add_argument("--zone", required=True)
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_collision_merge)

    cv = groups.add_parser(
        "convert", help="GLB -> 1.0 files (needs Blender and the private importer)"
    ).add_subparsers(dest="cmd", required=True)
    c = cv.add_parser(
        "tile", help="tile GLB -> 1.0 model, one MDL per node, with checks"
    )
    c.add_argument("glb")
    c.add_argument(
        "manifest",
        help="tile manifest JSON (piece boxes, tris), or - to take them from the GLB",
    )
    c.add_argument("scratch", help="work folder (intermediate files, result.json)")
    c.add_argument(
        "--name", required=True, help="model name, 14 characters like w0d0_t24_th9_h"
    )
    c.add_argument(
        "--donor",
        help="donor model DAT (shaders, textures); default: the zone's tile_model donor",
    )
    c.add_argument("--zone")
    c.add_argument(
        "--prefixed",
        action="store_true",
        help="GLB already went through the vertex fix (e.g. a shaded strip.glb)",
    )
    c.add_argument(
        "--patch-value",
        type=int,
        help="MESH HEAD bytes 4-7 = N instead of the donor table",
    )
    c.add_argument(
        "--remap",
        help="rename materials before import: a=b[,c=d] (all=X for every material)",
    )
    c.add_argument(
        "--input", help="GLB for the area check (default: the manifest's src)"
    )
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_convert_tile)

    tl = groups.add_parser("tiles", help="city tiles (Blender)").add_subparsers(
        dest="cmd", required=True
    )
    c = tl.add_parser(
        "shade", help="bake ambient occlusion into vertex colour (geometry untouched)"
    )
    c.add_argument(
        "glb", help="input GLB without COLOR_0 (e.g. the strip.glb of a convert run)"
    )
    c.add_argument("--zone", help="needed with --neighbours (grid)")
    c.add_argument("--tile", type=int, help="this tile's number (with --neighbours)")
    c.add_argument(
        "--neighbours",
        help="folder of tNN.glb; the up to 8 neighbour tiles shade this one",
    )
    c.add_argument("--rays", type=int, default=64)
    c.add_argument("--dist", type=float, default=6.0)
    c.add_argument("--floor", type=float, default=0.35)
    c.add_argument("--gamma", type=float, default=0.8)
    c.add_argument("--edge-ref", type=float, default=3.0)
    c.add_argument("--json", help="stats file (default <out>.json)")
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_tiles_shade)

    c = tl.add_parser(
        "build",
        help="modern scene FBX -> one GLB per grid tile (one mesh per real material)",
    )
    c.add_argument("--fbx", help="default: the zone's sources.fbx")
    c.add_argument("--zone", required=True)
    c.add_argument(
        "--cover",
        type=float,
        default=0.3,
        help="drop a bgs ground face covered by other ground within this (m)",
    )
    c.add_argument(
        "--fix-ground-winding",
        action="store_true",
        help="turn down-facing ground faces up",
    )
    c.add_argument(
        "-o", "--out", required=True, help="new folder: tile_N.glb + manifest.json"
    )
    c.set_defaults(fn=cmd_tiles_build)
    c = tl.add_parser(
        "terrain",
        help="fill ground holes from the modern terrain plates (+ SGB props given by regex)",
    )
    c.add_argument("tiles", help="folder of tile_N.glb (tiles build output)")
    c.add_argument("--zone", required=True)
    c.add_argument("--sqpack")
    c.add_argument("--cover", type=float, default=0.3)
    c.add_argument(
        "--sgb",
        help="regex of SGB prop models to add whole (default: zone tiles.terrain_sgb; '' for none)",
    )
    c.add_argument("--placements", help="sgb placements JSON (extract props)")
    c.add_argument("--check", action="store_true", help="print counts only")
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_tiles_terrain)
    c = tl.add_parser(
        "cut",
        help="one tile GLB -> pieces with tight boxes for the 1.0 culling (Blender)",
    )
    c.add_argument("glb")
    c.add_argument("--tag", required=True, help="tNN")
    c.add_argument("--zone", required=True)
    c.add_argument(
        "-o", "--out", required=True, help="new folder: <tag>.glb + manifest.json"
    )
    c.add_argument(
        "--opts",
        nargs=argparse.REMAINDER,
        default=[],
        help="last: cut options (default: zone tiles.cut_args + slotmap)",
    )
    c.set_defaults(fn=cmd_tiles_cut)

    ex = groups.add_parser(
        "extract", help="read-only extraction from the modern client"
    ).add_subparsers(dest="cmd", required=True)

    def client_args(c):
        g = c.add_mutually_exclusive_group(required=True)
        g.add_argument("--zone", help="use this zone's modern client and source paths")
        g.add_argument("--sqpack", help="modern client sqpack folder")

    c = ex.add_parser(
        "file", help="raw files by game path, written as <out>\\<game path>"
    )
    c.add_argument("paths", nargs="*", metavar="GAMEPATH")
    c.add_argument("--list", help="text file with one game path per line")
    client_args(c)
    c.add_argument("-o", "--out", required=True, help="output folder")
    c.set_defaults(fn=cmd_extract_file)
    c = ex.add_parser(
        "instances",
        help="bg.lgb -> instances JSON (BG models, collision, shared groups)",
    )
    c.add_argument(
        "--lgb", help="local bg.lgb or game path (default: <zone bg_path>/level/bg.lgb)"
    )
    client_args(c)
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_extract_instances)
    c = ex.add_parser(
        "sgb", help="instances JSON -> the models inside every placed shared group"
    )
    c.add_argument("instances")
    c.add_argument(
        "--sgb-root", help="read .sgb from an extracted folder instead of the client"
    )
    g = c.add_mutually_exclusive_group()
    g.add_argument("--zone")
    g.add_argument("--sqpack")
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_extract_sgb)
    c = ex.add_parser(
        "props",
        help="flatten shared groups into world placements (one row per prop model)",
    )
    c.add_argument("instances")
    c.add_argument("sgb_models")
    c.add_argument("--euler", choices=["xyz", "zyx"], default="xyz")
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_extract_props)
    c = ex.add_parser("model", help="modern .mdl (game path or file) -> GLB (LOD0)")
    c.add_argument("model")
    g = c.add_mutually_exclusive_group()
    g.add_argument("--zone")
    g.add_argument("--sqpack")
    c.add_argument("-o", "--out", required=True)
    c.set_defaults(fn=cmd_extract_model)
    return p


def main(argv):
    a = parser().parse_args(argv)
    try:
        return a.fn(a) or 0
    except (
        audio.AudioError,
        layout.LayoutError,
        collision.CollisionError,
        extract.ExtractError,
    ) as e:
        raise SystemExit("error: %s" % e)
