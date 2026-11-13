"""1.23b collision (PHB) authoring.

`phb` runs the PHB author example of this repository
(src/formats/examples/phb_collision_author.rs, setting `phb_author`) on a native
PHB template and a geometry JSON. zonekit uses `--profile branching` with
XIVL_PHB_GRID and XIVL_PHB_CAPACITY. The client caps one file at 65535
triangles and 65535 vertices.
"""

import json
import os
import subprocess

from . import tools


CAP = 65535


class CollisionError(Exception):
    pass


def geometry_counts(path):
    """(triangles, vertices) of a geometry JSON: {"schemaVersion": 1, "vertices": [[x,y,z]...], "triangles": [{"indices": [a,b,c], ...}]}."""
    with open(path, encoding="utf-8") as f:
        g = json.load(f)
    return len(g["triangles"]), len(g["vertices"])


def phb(template, geometry, out, grid=8, capacity=1000, profile="branching"):
    author = tools.setting("phb_author")
    if not os.path.exists(author):
        raise CollisionError("PHB author missing: " + author)
    if os.path.exists(out):
        raise CollisionError("exists, not overwriting: " + out)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    env = dict(os.environ, XIVL_PHB_GRID=str(grid), XIVL_PHB_CAPACITY=str(capacity))
    r = subprocess.run(
        [author, "--profile", profile, template, geometry, out],
        env=env,
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        raise CollisionError(
            "PHB author failed (%d): %s"
            % (r.returncode, (r.stderr or r.stdout).strip())
        )
    return r.stdout.strip()
