"""Per-zone settings file zones/<name>/zone.json: load, check, copy, show."""

import json
import os

from . import tools

REQUIRED = {
    "name": str,
    "zone_id": int,
    "region_id": int,
    "layout_ids": dict,
    "frames": dict,
    "tile_grid": dict,
    "sources": dict,
    "donors": dict,
    "file_ids": dict,
    "layout": dict,
    "lay_range": (int, float),
}


def path_of(name):
    return os.path.join(tools.setting("zones"), name, "zone.json")


def load(name):
    with open(path_of(name), encoding="utf-8") as f:
        z = json.load(f)
    for key, kind in REQUIRED.items():
        if not isinstance(z.get(key), kind):
            raise SystemExit("%s: '%s' missing or wrong type" % (path_of(name), key))
    if z["name"] != name:
        raise SystemExit(
            "%s: name is '%s', folder is '%s'" % (path_of(name), z["name"], name)
        )
    return z


def new(name, source):
    """Copy zones/<source>/zone.json to zones/<name>/zone.json with the new name; never overwrites."""
    out = path_of(name)
    if os.path.exists(out):
        raise SystemExit("exists, not overwriting: " + out)
    z = load(source)
    z["name"] = name
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "x", encoding="utf-8", newline="\n") as f:
        json.dump(z, f, indent=2)
        f.write("\n")
    return out


def tiles(z):
    """Active tiles as (index, centre x, centre z) in layout coordinates."""
    g = z["tile_grid"]
    sx, sz = g["width"] / g["nx"], g["depth"] / g["nz"]
    off = set(g["inactive"])
    return [
        (n, g["x0"] + (n % g["nx"] + 0.5) * sx, g["z0"] + (n // g["nx"] + 0.5) * sz)
        for n in range(g["nx"] * g["nz"])
        if n not in off
    ]


def show(z):
    g, f = z["tile_grid"], z["frames"]
    wx, wy, wz = f["world_offset"]
    print(
        "zone %d '%s', region %d, layouts %s"
        % (z["zone_id"], z["name"], z["region_id"], z["layout_ids"])
    )
    print(
        "grid %dx%d from (%g, %g), tile %.3f x %.3f m, %d active"
        % (
            g["nx"],
            g["nz"],
            g["x0"],
            g["z0"],
            g["width"] / g["nx"],
            g["depth"] / g["nz"],
            len(tiles(z)),
        )
    )
    x0, z0 = g["x0"], g["z0"]
    x1, z1 = x0 + g["width"], z0 + g["depth"]
    print(
        "grid in-game x %g..%g, z %g..%g (world offset %s)"
        % (x0 + wx, x1 + wx, z0 + wz, z1 + wz, f["world_offset"])
    )
