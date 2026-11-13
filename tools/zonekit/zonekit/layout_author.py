"""Author a 1.23b town layout from a retail one, keeping the retail streaming structure intact.

The tile grid, empty/tile file ids, identity pools, base layout, low partner, rows-from, avoid files and world
offset come from the zone settings (configure(argv, zone)).

zonekit layout author <zone> <in.DAT> <slots.json|none> <out.DAT> [--report out.md] [--keep-effects]
    [--radius 144] [--orig <base>] [--resident [folder-hex]] [--compact] [--rows-from <file>|none]
    [--single <model-hex> <collision-hex> <x> <y> <z>] [--none] [--extra <json>] [--tile-model n=hex|all=hex]
    [--tile-collision n=hex|all=hex] [--carrier-bg-flags [hex]] [--carrier-range f] [--carrier-bg-box <json>]
    [--lay-range f] [--carrier-rule strict|relaxed] [--avoid-keys k1,k2] [--avoid-hosts REGEX] [--keep-env [N]]
    [--rows-add <json>] [--rows-override <json>] [--rows-for <model files>]
zonekit layout author <zone> --check <in.DAT> <out.DAT> [--radius 144] [--single ...]

Unchanged: every DivideMap folder record, BlockLink, LaySettings (zone translation kept), node-type table,
extents, member arrays and counts, file size. Changed:
- tile carriers (UnitTree instances with one BGParts leaf + same-key Attribute leaf, flat chain): translation =
  cell centre - inner chain, rot/scale -> identity pools; carrier key's brt row -> tile model id + n, bhp ->
  tile collision id + n. A slot whose key has other placements is re-picked to a carrier with a single-placement
  key when one exists; otherwise the co-users are parked (y -= 30000).
- --resident: all carriers go into resident host folders (+0x62 bit 0 set). --compact: host count = carriers only,
  every other folder count 0 (instances in other folders, lights and effects included, are then never created).
- Every other BG resource key is renamed to a unique same-length key, its brt row -> the empty model, bhp -> the
  empty collision.
- Unless --keep-effects: effect-holding instances are parked (y -= 30000).
- Positions (--single, --extra) are in the INPUT file's layout space.
- Rows that --rows-from has over the base layout and the input lacks are appended to the row table.
"""

import bisect
import collections
import math
import os
import struct
import sys
import json
import re as _re_h
from . import tools

sys.path.insert(0, tools.RESEARCH)
import census_layout as C  # noqa: E402

CARRIER_CLASS = "BGPartsBaseObject"
BG = ("BGPartsBaseObject", "BGChipBaseObject")
EFFECT_LEAVES = (
    "VFXBaseObject",
    "LightBaseObject",
    "SEBaseObject",
    "TimeLineBaseObject",
    "CollisionBoxBaseObject",
    "TriggerBoxBaseObject",
)
EXTRA0 = 100
KEEP0 = 1000  # label base of the kept instances (carriers use 0..48, 100+)
ARGV = []


def configure(argv, z):
    """Set the module options from an argument list (argv[0] = program name) and a zone settings dict."""
    global \
        ARGV, \
        ROT_ID, \
        SCALE_ID, \
        EMPTY_MODEL, \
        EMPTY_COLL, \
        X0, \
        Z0, \
        SX, \
        SZ, \
        NX, \
        ACTIVE, \
        SEA_LOW, \
        ORIG, \
        ROWS_FROM
    global \
        SAMPLES, \
        SINGLE, \
        NONE, \
        EXTRAS, \
        TILE_MODEL, \
        TILE_COLL, \
        CARRIER_BG_FLAGS, \
        CARRIER_RANGE, \
        CARRIER_BG_BOX
    global \
        LAY_RANGE, \
        CARRIER_RULE, \
        AVOID_KEYS, \
        AVOID_HOSTS, \
        KEEP_ENV, \
        KEEP_X, \
        KEEP_Z, \
        KEEP_C, \
        AVOID, \
        WORLD
    global TILE_MODEL0, TILE_COLL0
    ARGV = list(argv)
    g, lay, ids = z["tile_grid"], z["layout"], z["file_ids"]
    ROT_ID, SCALE_ID = (
        int(lay["identity_rot_pool"], 16),
        int(lay["identity_scale_pool"], 16),
    )
    EMPTY_MODEL, EMPTY_COLL = (
        int(ids["empty_model"], 16),
        int(ids["empty_collision"], 16),
    )
    TILE_MODEL0, TILE_COLL0 = int(ids["tile_model"], 16), int(ids["tile_collision"], 16)
    X0, Z0, NX = float(g["x0"]), float(g["z0"]), g["nx"]
    SX, SZ = g["width"] / g["nx"], g["depth"] / g["nz"]
    ACTIVE = [n for n in range(g["nx"] * g["nz"]) if n not in g["inactive"]]
    X1, Z1 = g["x0"] + g["width"], g["z0"] + g["depth"]
    SAMPLES = [
        (float(x), float(y), float(zz))
        for y in (0, 20)
        for zz in range(int(g["z0"]), int(Z1) + 1, 16)
        for x in range(int(g["x0"]), int(X1) + 1, 16)
    ]
    KEEP_X, KEEP_Z = (
        (X0, float(X1)),
        (Z0, float(Z1)),
    )  # footprint in layout space, its centre
    KEEP_C = ((X0 + X1) / 2, (Z0 + Z1) / 2)
    SEA_LOW = list(lay["low_partner"])
    ORIG = z["sources"]["base_layout"]
    ROWS_FROM = lay["rows_from"]
    AVOID = list(lay["avoid_files"])  # --single: renamed keys must be absent here too
    WORLD = tuple(float(v) for v in z["frames"]["world_offset"])

    # --single <model-hex> <collision-hex> <x> <y> <z>: one carrier at a layout-space point instead of the tile grid.
    SINGLE = None
    if "--single" in ARGV:
        _i = ARGV.index("--single")
        SINGLE = (
            int(ARGV[_i + 1], 16),
            int(ARGV[_i + 2], 16),
            tuple(float(v) for v in ARGV[_i + 3 : _i + 6]),
        )
    NONE = (
        "--none" in ARGV
    )  # no carrier at all: an empty layout (with --compact: zero counted members anywhere)
    # --extra <json>: extra carriers [{"model": "FFFF3000", "collision": "FFFF002F", "x":, "y":, "z":, "name":}, ...],
    # labelled 100, 101, ... next to the tiles. --tile-model n=hex / --tile-collision n=hex (repeatable): id overrides.
    EXTRAS = (
        json.load(open(ARGV[ARGV.index("--extra") + 1])) if "--extra" in ARGV else []
    )
    TILE_MODEL, TILE_COLL = _overrides("--tile-model"), _overrides("--tile-collision")
    # --carrier-bg-flags [hex]: every carrier's BG leaf gets word +0x38 = value (default 0x100) and f32 +0x34 = 0.0
    # (Limsa nodes with 0x10a-0x10d at +0x38 fade with distance; 0x100 does not).
    CARRIER_BG_FLAGS = None
    if "--carrier-bg-flags" in ARGV:
        _j = ARGV.index("--carrier-bg-flags") + 1
        CARRIER_BG_FLAGS = (
            int(ARGV[_j], 16)
            if _j < len(ARGV) and not ARGV[_j].startswith("--")
            else 0x100
        )
    # --carrier-range <float>: f32 +0x34 of every carrier's BG leaf. Applied after --carrier-bg-flags.
    CARRIER_RANGE = (
        float(ARGV[ARGV.index("--carrier-range") + 1])
        if "--carrier-range" in ARGV
        else None
    )
    # --carrier-bg-box <json>: {"FFFF3000": [x0, y0, z0, x1, y1, z1], ...} per target model id -> the carrier's BG
    # leaf gets f32[3] +0x3C (low corner) and +0x48 (high corner), model-local (tile_boxes.py writes it).
    CARRIER_BG_BOX = (
        {
            int(k, 16): [float(v) for v in vals]
            for k, vals in json.load(open(arg("--carrier-bg-box", ""))).items()
        }
        if "--carrier-bg-box" in ARGV
        else None
    )
    # --lay-range <float>: LaySettings f32 +0x3C = the layer's draw range (retail town 96, low 192); every drawable
    # gets far pixel clip = range minus the camera offset.
    LAY_RANGE = float(arg("--lay-range", "nan")) if "--lay-range" in ARGV else None
    CARRIER_RULE = (
        ARGV[ARGV.index("--carrier-rule") + 1] if "--carrier-rule" in ARGV else "strict"
    )
    # --avoid-keys k1,k2: never use an instance whose BG key is listed as an extra's carrier
    AVOID_KEYS = (
        set(ARGV[ARGV.index("--avoid-keys") + 1].split(","))
        if "--avoid-keys" in ARGV
        else set()
    )
    # --avoid-hosts REGEX: never use a folder whose name matches as a resident host (Bk_isgrp_* drew 1 of 12)
    AVOID_HOSTS = (
        _re_h.compile(ARGV[ARGV.index("--avoid-hosts") + 1])
        if "--avoid-hosts" in ARGV
        else None
    )
    assert CARRIER_RULE in ("strict", "relaxed"), CARRIER_RULE
    # --keep-env [N] (default 12; needs --resident): N non-carrier UnitTree instances whose group/child records bind a
    # DrawEnvObject (dwev_00 preferred) inside the footprint stay alive as counted resident-host members, so the
    # client's per-frame environment query (fog + light set) finds something.
    KEEP_ENV = None
    if "--keep-env" in ARGV:
        _k = ARGV.index("--keep-env") + 1
        KEEP_ENV = int(ARGV[_k]) if _k < len(ARGV) and ARGV[_k].isdigit() else 12


def cell(n):
    return (X0 + (n % NX + 0.5) * SX, 0.0, Z0 + (n // NX + 0.5) * SZ)


def f32(v):
    return struct.unpack("<f", struct.pack("<f", v))[0]


def arg(name, default):
    return ARGV[ARGV.index(name) + 1] if name in ARGV else default


def _overrides(flag):  # n=hex, or all=hex for every active tile
    out = {}
    for i, a in enumerate(ARGV[1:], 1):
        if ARGV[i - 1] == flag:
            k, v = a.split("=")
            for n in ACTIVE if k == "all" else [int(k)]:
                out[n] = int(v, 16)
    return out


def tgt_ids():
    ex = [EXTRA0 + i for i in range(len(EXTRAS))]
    return ex if NONE else [0] if SINGLE else ACTIVE + ex


def tgt_name(n):
    return (
        EXTRAS[n - EXTRA0].get("name", f"extra{n - EXTRA0}")
        if n >= EXTRA0
        else f"tile {n}"
    )


def tgt_pos(n):
    if n >= EXTRA0:
        e = EXTRAS[n - EXTRA0]
        return (float(e["x"]), float(e["y"]), float(e["z"]))
    return SINGLE[2] if SINGLE else cell(n)


def tgt_model(n):
    if n >= EXTRA0:
        return int(EXTRAS[n - EXTRA0]["model"], 16)
    return SINGLE[0] if SINGLE else TILE_MODEL.get(n, TILE_MODEL0 + n)


def tgt_coll(n):
    if n >= EXTRA0:
        return int(EXTRAS[n - EXTRA0]["collision"], 16)
    return SINGLE[1] if SINGLE else TILE_COLL.get(n, TILE_COLL0 + n)


class Model:
    """Read-only view of a layout: classes, placements, folder membership, rows."""

    def __init__(self, data):
        self.d = data
        self.L = L = C.parse_layout(0, data)
        self.P = P = L.payload
        self.base = P.base
        self.cls = lambda n: L.path(n).rsplit("/", 1)[-1] if n else "none"
        self.inst = sorted(n for n in L.node_types if self.cls(n) == "InstanceObject")
        self.div = sorted(
            n for n in L.node_types if self.cls(n) == "DivideMapFolderObject"
        )
        self.blk = sorted(n for n in L.node_types if self.cls(n) == "BlockLinkObject")
        self.lay = [n for n in L.node_types if "/LaySettings/" in L.path(n)]
        assert len(self.lay) == 1
        self.lay = self.lay[0]
        self.rows = [
            (i, k.decode("latin1"), t.decode("latin1"), r)
            for i, (k, t, r) in enumerate(L.rows)
        ]
        self.rows_of = collections.defaultdict(list)
        for i, k, t, r in self.rows:
            self.rows_of[k].append((i, t))
        self.homes = collections.defaultdict(list)  # instance -> [(folder, slot node)]
        for f in self.div:
            arr, cnt = P.u32(f + 0x10), P.u32(f + 0x14)
            for i in range(cnt):
                s = P.u32(arr + 4 * i)
                self.homes[P.u32(s)].append((f, s))
        self.leaves = {n: self._leaves(n) for n in self.inst}
        self.users = collections.defaultdict(list)  # BG node -> instances reaching it
        for n in self.inst:
            for b, _, _ in self.leaves[n]:
                if self.cls(b) in BG:
                    self.users[b].append(n)
        self.fname = {f: P.string(P.u32(f + 8)).decode("latin1") for f in self.div}
        self.fbyname = {v: k for k, v in self.fname.items()}
        self.keycount = collections.Counter(
            self.key(b) for b in L.node_types if self.cls(b) in BG
        )
        self.blink = {}
        for b in self.blk:
            self.blink[P.string(P.u32(b + 8)).decode("latin1")] = [
                self.fbyname[P.string(P.u32(P.u32(b + 0x10) + 4 * i)).decode("latin1")]
                for i in range(P.u32(b + 0x14))
            ]

    def key(self, bg):
        return self.P.string(self.P.u32(bg + 0x20)).decode("latin1")

    def akey(self, at):
        return self.P.string(self.P.u32(at + 0x24)).decode("latin1")

    def vec(self, off):
        return self.P.floats(off)

    def flat(self, node, r, s):
        rr, ss = self.P.u32(node + r), self.P.u32(node + s)
        return (not rr or self.P.floats(rr) == (0.0, 0.0, 0.0)) and (
            not ss or self.P.floats(ss) == (1.0, 1.0, 1.0)
        )

    def _leaves(self, n):
        """[(leaf base node, [inner translations], chain flat)] for an instance; a non-tree base is its own leaf."""
        P, L = self.P, self.L
        b = P.u32(n + 0x3C)
        if not self.cls(b).startswith("UnitTree"):
            return [(b, [], True)]
        t0, fl = self.vec(b + 0x10), self.flat(b, 0x1C, 0x20)
        out = [
            (P.u32(c + 0x18), [t0, self.vec(c)], fl and self.flat(c, 0x0C, 0x10))
            for c in L.run(P.u32(b + 0x30), P.u32(b + 0x34))
        ]
        stack = [(g, [t0], fl) for g in L.run(P.u32(b + 0x24), P.u32(b + 0x28))]
        while stack:
            g, ch, gf = stack.pop()
            ch2, gf2 = ch + [self.vec(g)], gf and self.flat(g, 0x0C, 0x10)
            if P.u32(g + 0x20):
                out += [
                    (
                        P.u32(c + 0x18),
                        ch2 + [self.vec(c)],
                        gf2 and self.flat(c, 0x0C, 0x10),
                    )
                    for c in L.run(P.u32(g + 0x20), P.u32(g + 0x24))
                ]
            stack += [(s, ch2, gf2) for s in L.run(P.u32(g + 0x14), P.u32(g + 0x18))]
        return out

    def folder(self, f):
        P = self.P
        pairs = [
            (
                P.string(P.u32(P.u32(f + 0x58) + 8 * i)).decode("latin1"),
                P.string(P.u32(P.u32(f + 0x58) + 8 * i + 4)).decode("latin1"),
            )
            for i in range(P.u32(f + 0x5C))
        ]
        return dict(
            node=f,
            name=self.fname[f],
            prio=P.u32(f + 0x0C),
            lo=self.vec(f + 0x30),
            hi=self.vec(f + 0x40),
            resident=bool(self.d[self.base + f + 0x62] & 1),
            pairs=pairs,
        )

    def y(self, n):
        return struct.unpack_from("<f", self.d, self.base + n + 0x24)[0]


def pools(M):
    """(rotation ref, scale ref, note): the retail town pools when this file references them as such, else this
    file's most-referenced zero-rotation / unit-scale records, else 0 (null = zero rotation / unit scale)."""
    P, rot, sca = M.P, collections.Counter(), collections.Counter()
    for n in M.L.node_types:
        c = M.cls(n)
        if c == "InstanceObject":
            r, s = P.u32(n + 0x2C), P.u32(n + 0x30)
        elif c.startswith("UnitTree"):
            r, s = P.u32(n + 0x1C), P.u32(n + 0x20)
        else:
            continue
        rot[r] += 1
        sca[s] += 1

    def pick(cnt, retail, value):
        good = [r for r in cnt if r and P.floats(r) == value]
        if retail in good:
            return retail, "retail pool"
        if good:
            return max(
                good, key=lambda r: (cnt[r], -r)
            ), "own pool (retail offset not a pool here)"
        return 0, "null ref (no pool record)"

    r, rn = pick(rot, ROT_ID, (0.0, 0.0, 0.0))
    s, sn = pick(sca, SCALE_ID, (1.0, 1.0, 1.0))
    return r, s, f"rotation {r:#x} ({rn}), scale {s:#x} ({sn})"


def carrier(M, n, strict=True):
    """(BG leaf, inner translation) if instance n can carry a tile, else (None, reason). strict=False drops the
    same-key Attribute and bhp row requirements (a layer without collision)."""
    if not M.cls(M.P.u32(n + 0x3C)).startswith("UnitTree"):
        return None, "base " + M.cls(M.P.u32(n + 0x3C))
    lv = M.leaves[n]
    bgs = [x for x in lv if M.cls(x[0]) in BG]
    if len(bgs) != 1:
        return None, f"{len(bgs)} BG leaves"
    b, ch, fl = bgs[0]
    if M.cls(b) != CARRIER_CLASS:
        return None, "BG leaf is " + M.cls(b)
    k = M.key(b)
    if (
        M.keycount[k] != 1
    ):  # the key is shared by several BG nodes: its rows cannot be carrier-specific
        return None, "key on several BG nodes"
    allatt = [x for x in lv if M.cls(x[0]) == "AttributeBaseObject"]
    atts = [
        x for x in allatt if M.akey(x[0]) == k
    ]  # other-key Attributes get empty collision rows
    if len(lv) != 1 + len(allatt) or (strict and not atts):
        return None, "leaves other than BG + Attribute, or no same-key Attribute"
    if not (fl and all(af for _, _, af in atts)):
        return None, "rotated/scaled chain"
    inner = [sum(v[i] for v in ch) for i in range(3)]
    if any(
        any(abs(sum(v[i] for v in ac) - inner[i]) > 1e-4 for i in range(3))
        for _, ac, _ in atts
    ):
        return None, "Attribute offset differs from BG"
    tags = {t for _, t in M.rows_of[k]}
    if not ({"brt", "bhp"} if strict else {"brt"}) <= tags:
        return None, "key lacks brt+bhp rows"
    if len(M.homes[n]) != 1:
        return None, f"{len(M.homes[n])} homes"
    return (b, inner), None


def carrier_wide(M, n):
    """--carrier-rule relaxed, after the (relaxed-bhp) carriers of carrier(): (a) a UnitTree instance with a flat chain
    to one of its BGParts leaves, other leaves allowed (other BG keys are parked as usual; effect leaves stay and are
    counted); (b) a direct BGParts instance. Returns ((BG leaf, inner), kind) or (None, reason)."""
    if len(M.homes[n]) != 1:
        return None, "homes"
    b0 = M.P.u32(n + 0x3C)
    c0 = M.cls(b0)
    if c0 == "BGPartsBaseObject":
        if M.keycount[M.key(b0)] != 1 or "brt" not in {
            t for _, t in M.rows_of[M.key(b0)]
        }:
            return None, "key on several BG nodes or no brt row"
        return ((b0, [0.0, 0.0, 0.0]), "direct"), None
    if not c0.startswith("UnitTree"):
        return None, "base"
    lv = [x for x, _, _ in M.leaves[n]]
    for b, ch, fl in M.leaves[n]:
        # the node must occur once in the tree, else the carrier itself draws the model several times
        if (
            M.cls(b) == "BGPartsBaseObject"
            and fl
            and lv.count(b) == 1
            and M.keycount[M.key(b)] == 1
            and "brt" in {t for _, t in M.rows_of[M.key(b)]}
        ):
            return ((b, [sum(v[i] for v in ch) for i in range(3)]), "tree+leaves"), None
    return None, "no flat BGParts leaf"


def supply(M):
    """Distinct BGParts nodes usable as carriers under each rule (a carrier needs its own node)."""
    s = {c[0] for n in M.inst for c, _ in [carrier(M, n)] if c}
    r = {c[0] for n in M.inst for c, _ in [carrier(M, n, strict=False)] if c}
    a = set()
    b = set()
    for n in M.inst:
        w, _ = carrier_wide(M, n)
        if w:
            (b if w[1] == "direct" else a).add(w[0][0])
    allbp = {x for x in M.L.node_types if M.cls(x) == "BGPartsBaseObject"}
    return dict(
        bgparts_nodes=len(allbp),
        strict=len(s),
        strict_or_nobhp=len(r | s),
        tree_leaves=len(a),
        direct=len(b),
        relaxed_total=len(r | s | a | b),
    )


def rec_env(M, rec):
    """DrawEnvObject node bound by an environment record, else 0: the 0x30 record (nine descriptors, pointer at
    +0x00) or the 0x1C record (pointer at +0x0C)."""
    k = M.L.node_types.get(rec)
    if k is None:
        return 0
    if k.size == 0x30 and (0, 0) in k.fields:
        v = M.P.u32(rec)
    elif k.size == 0x1C and (0xC, 0) in k.fields and (4, 0) not in k.fields:
        v = M.P.u32(rec + 0xC)
    else:
        return 0
    return v if v in M.L.node_types and M.cls(v) == "DrawEnvObject" else 0


def env_of(M, n):
    """DrawEnvObject nodes bound to UnitTree instance n: through its child descriptors (instance +0x34/+0x38, 12-byte
    entries whose first word is the record) and through the +0x20 record of every child in its group tree."""
    P = M.P
    b = P.u32(n + 0x3C)
    if not M.cls(b).startswith("UnitTree"):
        return []
    recs = [P.u32(P.u32(n + 0x34) + 12 * i) for i in range(P.u32(n + 0x38))]
    ch = list(M.L.run(P.u32(b + 0x30), P.u32(b + 0x34)))
    st = list(M.L.run(P.u32(b + 0x24), P.u32(b + 0x28)))
    while st:
        g = st.pop()
        st += M.L.run(P.u32(g + 0x14), P.u32(g + 0x18))
        if P.u32(g + 0x20):
            ch += M.L.run(P.u32(g + 0x20), P.u32(g + 0x24))
    recs += [P.u32(c + 0x20) for c in ch]
    return sorted({e for e in map(lambda r: rec_env(M, r) if r else 0, recs) if e})


def in_footprint(M, n):
    x, _, z = M.vec(n + 0x20)
    return KEEP_X[0] <= x <= KEEP_X[1] and KEEP_Z[0] <= z <= KEEP_Z[1]


def pick_keep_env(M, count, excluded):
    """count instances spread over the footprint: candidates are UnitTree instances with one home that bind a
    DrawEnv and lie inside it; dwev_00 binders without effect leaves first, then greedy farthest-point (XZ), seeded
    by the candidate nearest the footprint centre. Ties by offset."""
    cands = []
    for n in M.inst:
        if n in excluded or len(M.homes[n]) != 1 or not in_footprint(M, n):
            continue
        envs = env_of(M, n)
        if envs:
            cands.append((n, envs))

    def names(envs):
        return {M.P.string(M.P.u32(e + 8)).decode("latin1") for e in envs}

    d00 = [c for c in cands if "dwev_00" in names(c[1])]
    quiet = [
        c
        for c in d00
        if not any(M.cls(x) in EFFECT_LEAVES for x, _, _ in M.leaves[c[0]])
    ]
    # tiers: dwev_00 without any effect leaf (no lights, VFX, sound, triggers), dwev_00, any env-bound
    pool = next((p for p in (quiet, d00, cands) if len(p) >= count), cands)
    if len(pool) < count:
        sys.exit(
            f"--keep-env {count}: only {len(pool)} candidate instances ({len(cands)} env-bound in the footprint)"
        )

    def xz(n):
        return (M.vec(n + 0x20)[0], M.vec(n + 0x20)[2])

    chosen = [min(pool, key=lambda c: (math.dist(xz(c[0]), KEEP_C), c[0]))]
    while len(chosen) < count:
        rest = [c for c in pool if c not in chosen]
        chosen.append(
            max(
                rest,
                key=lambda c: (
                    min(math.dist(xz(c[0]), xz(k[0])) for k in chosen),
                    -c[0],
                ),
            )
        )
    return chosen, len(cands)


class Stream:
    """Offline per-frame block selection (layout-block-streaming.md, as given in the brief)."""

    def __init__(self, M, radius, xz_best=False, box_gate=False):
        self.M, self.R, self.xz, self.box_gate = M, radius, xz_best, box_gate
        self.F = [M.folder(f) for f in M.div]
        self.cand = [f for f in self.F if not f["resident"]]
        self.res = {f["node"] for f in self.F if f["resident"]}
        for f in self.F:
            f["c"] = tuple((a + b) / 2 for a, b in zip(f["lo"], f["hi"]))

    @staticmethod
    def boxdist(p, lo, hi, dims=(0, 1, 2)):
        return math.sqrt(sum(max(lo[i] - p[i], 0.0, p[i] - hi[i]) ** 2 for i in dims))

    def best(self, p):
        dims = (0, 2) if self.xz else (0, 1, 2)
        return min(
            self.cand,
            key=lambda f: (
                self.boxdist(p, f["lo"], f["hi"], dims),
                -f["prio"],
                math.dist([p[i] for i in dims], [f["c"][i] for i in dims]),
            ),
        )

    def gate(self, p, f):
        R = self.R
        cube = all(f["lo"][i] <= p[i] + R and f["hi"][i] >= p[i] - R for i in range(3))
        if self.box_gate:  # variant: 0x00640040 read as point-to-box distance
            return cube and self.boxdist(p, f["lo"], f["hi"]) <= R
        return cube and math.dist(p, f["c"]) <= R

    def created(self, p, best=None):
        out = set(self.res)
        if not self.cand:
            return out
        b = best or self.best(p)
        for lay, grp in b["pairs"]:
            if lay != "":
                continue
            if grp == "":
                out |= {f["node"] for f in self.cand if self.gate(p, f)}
            elif grp in self.M.blink:
                fb = {f["node"]: f for f in self.F}
                out |= {
                    fn
                    for fn in self.M.blink[grp]
                    if fn in self.res or self.gate(p, fb[fn])
                }
        return out


def lyb_identity(d, orig_p):
    """Compare the lyb payload with the pristine layout; classify differing bytes by declared extent."""
    if not os.path.exists(orig_p):
        return {"orig": orig_p, "available": False}
    o = open(orig_p, "rb").read()
    B = struct.unpack_from("<I", d, 0x20)[0] + 0x30
    Bo = struct.unpack_from("<I", o, 0x20)[0] + 0x30
    pc, po = d[B:], o[Bo:]
    res = {
        "orig": orig_p,
        "available": True,
        "identical": pc == po,
        "len": len(pc),
        "len_orig": len(po),
    }
    if pc == po:
        return res
    Lo = C.parse_layout(0, o)
    owners = collections.Counter()
    for i in range(min(len(pc), len(po))):
        if pc[i] != po[i]:
            for k in Lo.types:
                if k.count and k.start <= i < k.start + k.count * k.size:
                    owners[
                        f"extent {k.start:#x} ({Lo.path(k.start).rsplit('/', 1)[-1] or 'unclassed'} x{k.count} size {k.size:#x})"
                    ] += 1
                    break
            else:
                owners[f"pool/header {i:#x}"] += 1
    res["diff_owners"] = dict(owners)
    res["tail_bytes"] = len(pc) - len(po)
    Lc = C.parse_layout(0, d)
    res["extents_changed"] = [
        (f"{a.start:#x}", a.count, f"{b.start:#x}", b.count)
        for a, b in zip(Lo.types, Lc.types)
        if (a.start, a.count, a.size) != (b.start, b.count, b.size)
    ]
    return res


def author():
    src, slots_p, out = ARGV[1:4]
    report_p = arg("--report", None)
    R = float(arg("--radius", "144"))
    keep_fx = "--keep-effects" in ARGV
    resident = "--resident" in ARGV
    compact = "--compact" in ARGV
    stats_pre = collections.Counter()
    ri = ARGV.index("--resident") + 1 if resident else 0
    resident_at = (
        int(ARGV[ri], 16)
        if resident and ri < len(ARGV) and not ARGV[ri].startswith("--")
        else None
    )
    if os.path.exists(out):
        sys.exit("refusing to overwrite " + out)
    din = open(src, "rb").read()
    d = bytearray(din)
    M = Model(din)
    P, base = M.P, M.base
    rot_ref, scale_ref, pool_note = pools(M)
    print("POOLS", pool_note)
    good = {}
    for n in M.inst:
        c, why = carrier(M, n)
        if c:
            good[n] = c
    if NONE:
        ident = {"skipped": "none mode"}
        tiles = {}
        slots = {}
        unique_pool = []
        print("NONE: no carrier")
    elif SINGLE:
        ident = {"skipped": "single mode"}
        strict = bool(good)
        if not strict:
            good = {n: c for n in M.inst for c, _ in [carrier(M, n, strict=False)] if c}
        if not good:
            sys.exit("no BGParts carrier in this file")
        m = min(good, key=lambda m: (len(M.users[good[m][0]]), m))
        b = good[m][0]
        cu = [u for u in M.users[b] if u != m]
        tiles = {
            0: dict(
                inst=m,
                bg=b,
                inner=good[m][1],
                couse=cu,
                how=f"{'single-use key' if not cu else str(len(cu)) + ' co-users parked'}"
                f"{'' if strict else ' (relaxed: no same-key Attribute/bhp required)'}",
            )
        }
        print(
            f"SINGLE carrier {m:#x} key {M.key(b)}: {tiles[0]['how']}; {len(good)} candidates"
        )
        unique_pool = [x for x in good if len(M.users[good[x][0]]) == 1]
        slots = {}
    else:
        ident = lyb_identity(din, arg("--orig", ORIG))
        slots = {s["tile"]: s for s in json.load(open(slots_p))}
        # Slots are valid for this file when the instance records and their keys match.
        for n in ACTIVE:
            s = slots[n]
            assert s["active"], n
            assert s["instanceNode"] in M.leaves, ("slot not an InstanceObject", n)
        ident["slots_valid"] = all(
            any(
                M.cls(b) in BG + ("AttributeBaseObject",)
                and (M.key(b) if M.cls(b) in BG else M.akey(b)) == slots[n]["key"]
                for b, _, _ in M.leaves[slots[n]["instanceNode"]]
            )
            for n in ACTIVE
        )
        assert ident["slots_valid"], (
            "slot offsets do not match this file: recompute the slot table for this layout"
        )
        unique_pool = sorted(
            (n for n, (b, _) in good.items() if len(M.users[b]) == 1),
            key=lambda n: (M.cls(good[n][0]) != "BGPartsBaseObject", n),
        )
        tiles = {}  # tile -> dict

    # 1. Carriers.
    used = set(slots[n]["instanceNode"] for n in slots if n in ACTIVE)
    for n in [] if SINGLE or NONE else ACTIVE:
        s = slots[n]
        node = s["instanceNode"]
        c, why = carrier(M, node)
        shared = c and len(M.users[c[0]]) > 1
        if c and not shared:
            tiles[n] = dict(
                inst=node, bg=c[0], inner=c[1], how="kept (single-placement key)"
            )
            continue
        pick = next((m for m in unique_pool if m not in used), None)
        if pick is not None:
            used.add(pick)
            tiles[n] = dict(
                inst=pick,
                bg=good[pick][0],
                inner=good[pick][1],
                how=f"re-picked (slot {node:#x}: {why or str(len(M.users[c[0]])) + ' placements of key'})",
                old=node,
            )
        elif c:
            tiles[n] = dict(
                inst=node,
                bg=c[0],
                inner=c[1],
                how=f"kept, {len(M.users[c[0]]) - 1} co-users parked",
                couse=[u for u in M.users[c[0]] if u != node],
            )
        else:
            # Slot of the wrong class and no single-placement carrier left: shared-key carrier with the fewest
            # co-users (none of them a slot or carrier), co-users parked.
            taken = {t["bg"] for t in tiles.values()}
            pool = sorted(
                (
                    m
                    for m, (b, _) in good.items()
                    if m not in used
                    and b not in taken
                    and not (set(M.users[b]) - {m}) & used
                ),
                key=lambda m: (len(M.users[good[m][0]]), m),
            )
            if not pool:
                sys.exit(
                    f"tile {n}: slot {node:#x} cannot carry a tile ({why}) and no carrier is left"
                )
            m = pool[0]
            used.add(m)
            b = good[m][0]
            tiles[n] = dict(
                inst=m,
                bg=b,
                inner=good[m][1],
                old=node,
                couse=[u for u in M.users[b] if u != m],
                how=f"re-picked shared (slot {node:#x}: {why}), {len(M.users[b]) - 1} co-users parked",
            )
    if EXTRAS and not SINGLE:
        # extras with the empty collision need no same-key Attribute / bhp row: relaxed carriers join the pool
        # after the strict ones (strict first in every ordering below)
        for m in M.inst:
            if m not in good:
                c, _ = carrier(M, m, strict=False)
                if c and all(int(e["collision"], 16) == EMPTY_COLL for e in EXTRAS):
                    good[m] = c
        unique_pool = sorted(
            (
                m
                for m, (b, _) in good.items()
                if len(M.users[b]) == 1 and M.key(b) not in AVOID_KEYS
            ),
            key=lambda m: (
                "bhp" not in {t for _, t in M.rows_of[M.key(good[m][0])]},
                m,
            ),
        )
    wide = {}
    if EXTRAS and CARRIER_RULE == "relaxed":
        # second tier, used only when the first is exhausted: trees with other leaves, direct BGParts instances
        for m in M.inst:
            if m not in good:
                w, _ = carrier_wide(M, m)
                if w:
                    wide[m] = w[0]

    def other_bgs(m, b):
        return {x for x, _, _ in M.leaves[m] if M.cls(x) in BG and x != b}

    for n in (
        [] if SINGLE else tgt_ids()[0 if NONE else len(ACTIVE) :]
    ):  # --extra carriers, like the tiles
        taken = {t["bg"] for t in tiles.values()}
        pick = next(
            (m for m in unique_pool if m not in used and good[m][0] not in taken), None
        )
        if pick is not None:
            used.add(pick)
            tiles[n] = dict(
                inst=pick,
                bg=good[pick][0],
                inner=good[pick][1],
                how="extra, single-placement key",
            )
            continue
        busy = used | {u for t in tiles.values() for u in t.get("couse", [])}

        def pool_of(cands, tier):
            return sorted(
                (
                    m
                    for m, (b, _) in cands.items()
                    if m not in busy
                    and b not in taken
                    and M.key(b) not in AVOID_KEYS
                    and not (set(M.users[b]) - {m}) & busy
                    and not other_bgs(m, b) & taken
                ),
                key=lambda m: (
                    len(M.users[cands[m][0]]) > 1,
                    len(M.users[cands[m][0]]),
                    m,
                ),
            )

        pool, tier, src = pool_of(good, 0), 0, good
        if not pool and wide:
            pool, tier, src = pool_of(wide, 1), 1, wide
        if not pool:
            # supply report: every carrier needs its own BG node (its key is the node's pooled string)
            sp = supply(M)
            sys.exit(
                f"{tgt_name(n)}: no carrier left after {len(tiles)} carriers "
                f"({sum(1 for k in tiles if k >= EXTRA0)} extras of {len(EXTRAS)}), rule {CARRIER_RULE}. "
                f"Supply (distinct BGParts nodes): {sp}"
            )
        m = pool[0]
        used.add(m)
        b = src[m][0]
        cu = [u for u in M.users[b] if u != m]
        kind = (
            ""
            if tier == 0
            else (
                " (relaxed: direct BGParts)"
                if M.cls(M.P.u32(m + 0x3C)) == "BGPartsBaseObject"
                else f" (relaxed: tree, {len(other_bgs(m, b))} other BG leaves parked, "
                f"{sum(M.cls(x) in EFFECT_LEAVES for x, _, _ in M.leaves[m])} effect leaves kept)"
            )
        )
        tiles[n] = dict(
            inst=m,
            bg=b,
            inner=src[m][1],
            couse=cu,
            tier_c=tier,
            how=f"extra, {'single-placement' if not cu else 'shared'} key, {len(cu)} co-users parked{kind}",
        )
    carriers = {t["inst"]: n for n, t in tiles.items()}
    carrier_bg = {t["bg"] for t in tiles.values()}
    assert len(carrier_bg) == len(tiles) and not any(
        u in carriers for t in tiles.values() for u in t.get("couse", [])
    )
    for n, t in tiles.items():
        t["key"] = M.key(t["bg"])

    # 1b. --keep-env: instances that stay alive for the environment query (labels KEEP0 + j).
    kenv = {}
    if KEEP_ENV:
        if not resident:
            sys.exit("--keep-env needs --resident")
        chosen, ncand = pick_keep_env(
            M,
            KEEP_ENV,
            set(carriers) | {u for t in tiles.values() for u in t.get("couse", [])},
        )
        for j, (m, envs) in enumerate(chosen):
            kenv[KEEP0 + j] = dict(
                inst=m,
                envs=envs,
                pos=M.vec(m + 0x20),
                lights=sum(M.cls(x) == "LightBaseObject" for x, _, _ in M.leaves[m]),
                fx=sum(
                    M.cls(x) in EFFECT_LEAVES and M.cls(x) != "LightBaseObject"
                    for x, _, _ in M.leaves[m]
                ),
            )
        stats_pre["--keep-env instances kept"] = len(kenv)
        stats_pre["--keep-env candidates in footprint"] = ncand
        stats_pre["--keep-env LightBase leaves brought in"] = sum(
            k["lights"] for k in kenv.values()
        )
        stats_pre["--keep-env other effect leaves brought in"] = sum(
            k["fx"] for k in kenv.values()
        )
        print(
            f"KEEP-ENV {len(kenv)} instances of {ncand} candidates; lights brought in "
            f"{sum(k['lights'] for k in kenv.values())}, other effect leaves {sum(k['fx'] for k in kenv.values())}"
        )
        for k in kenv.values():
            print(
                f"  kept {k['inst']:#x} "
                + ",".join(
                    M.P.string(M.P.u32(e + 8)).decode("latin1") for e in k["envs"]
                )
                + f" pos ({k['pos'][0]:.1f}, {k['pos'][1]:.1f}, {k['pos'][2]:.1f}) lights {k['lights']} other fx {k['fx']}"
            )
    keep_set = {k["inst"] for k in kenv.values()}

    # 2. Streaming simulation and folder choice (on the input: the edits do not change boxes or links).
    S3, S2 = Stream(M, R), Stream(M, R, xz_best=True)
    sim = (
        SAMPLES if not (resident or NONE) else []
    )  # the per-cell sampling only matters without --resident
    samp3 = {p: S3.created(p) for p in sim}
    samp2 = {p: S2.created(p) for p in sim}
    best_diff = sum(S3.best(p)["node"] != S2.best(p)["node"] for p in sim)
    created_diff = sum(samp3[p] != samp2[p] for p in sim)
    fb = {f["node"]: f for f in S3.F}

    def choose(n, S, samp):
        """Folder options for tile n in preference order, with the tier that produced them.
        A: contains the cell centre (XZ) and is created there, by coverage share;
        B: created at the centre, nearest box centre; C: the resident folder; D: best coverage share only."""
        c = cell(n)
        here = S.created(c)
        near = [p for p in SAMPLES if math.dist((p[0], p[2]), (c[0], c[2])) <= 100.0]

        def share(f):
            return sum(f["node"] in samp[p] for p in near) / len(near)

        contain = [
            f
            for f in S.cand
            if f["lo"][0] <= c[0] <= f["hi"][0] and f["lo"][2] <= c[2] <= f["hi"][2]
        ]
        A_ = sorted(
            (f for f in contain if f["node"] in here),
            key=lambda f: (-share(f), math.dist(f["c"], c), f["node"]),
        )
        B_ = sorted(
            (fb[x] for x in here if x not in S.res and fb[x] not in A_),
            key=lambda f: (math.dist(f["c"], c), f["node"]),
        )
        C_ = [fb[x] for x in sorted(S.res)]
        D_ = sorted(
            (f for f in S.cand if f not in A_ and f not in B_),
            key=lambda f: (-share(f), math.dist(f["c"], c), f["node"]),
        )
        opts = (
            [("A", f, share(f)) for f in A_]
            + [("B", f, share(f)) for f in B_]
            + [("C", f, 1.0) for f in C_]
            + [("D", f, share(f)) for f in D_]
        )
        return opts, here, contain

    # 3. Slot swaps into the chosen folders.
    # Every BG key in the file except the carrier keys, placed or not.
    parked_bg_keys = {
        M.key(b) for b in M.L.node_types if M.cls(b) in BG and b not in carrier_bg
    }
    homes = {i: list(h) for i, h in M.homes.items()}
    slot_val = {}
    for i, h in homes.items():
        for f, s in h:
            slot_val[s] = i

    def will_park_fx(i):
        b = M.P.u32(i + 0x3C)
        c = M.cls(b)
        if c.startswith("UnitTree"):
            return any(M.cls(x) in EFFECT_LEAVES for x, _, _ in M.leaves[i])
        return c not in BG + ("AttributeBaseObject", "none")

    def is_partner(i):
        """Harmless to move: a non-tile instance whose BG keys are all parked, or (default mode) one that the
        effects pass parks at y - 30000."""
        if i in carriers or i in keep_set or len(homes.get(i, [])) != 1:
            return 0
        bgs = [b for b, _, _ in M.leaves[i] if M.cls(b) in BG]
        if bgs and all(M.key(b) in parked_bg_keys for b in bgs) and not will_park_fx(i):
            return 2 if M.cls(M.P.u32(i + 0x3C)) == "BGPartsBaseObject" else 1
        return -1 if (not keep_fx and will_park_fx(i)) else 0

    def harmless(F):
        arr, cnt = P.u32(F + 0x10), P.u32(F + 0x14)
        mem = [P.u32(arr + 4 * i) for i in range(cnt)]
        return sorted(
            (s for s in mem if is_partner(slot_val[s])),
            key=lambda s: (-is_partner(slot_val[s]), s),
        )

    def any_members(F):
        return [P.u32(P.u32(F + 0x10) + 4 * i) for i in range(P.u32(F + 0x14))]

    partners = harmless
    swaps = []
    host = None
    host_info = {}
    if resident:
        # --resident: every carrier goes into ONE host folder whose resident bit (+0x62 bit 0) is set.
        fb_ = {f: M.folder(f) for f in M.div}
        nlinks = {f: sum(f in v for v in M.blink.values()) for f in M.div}

        def off_city(f):
            lo, hi = fb_[f]["lo"], fb_[f]["hi"]
            return not (
                lo[0] <= 160 and hi[0] >= -200 and lo[2] <= 180 and hi[2] >= -140
            )

        def far(f):
            c = [(a + b) / 2 for a, b in zip(fb_[f]["lo"], fb_[f]["hi"])]
            return math.dist((c[0], c[2]), (-20.0, 20.0))

        cands = sorted(
            (
                f
                for f in M.div
                if not fb_[f]["resident"]
                and f != 0xE2960
                and (SINGLE or NONE or off_city(f))
                and harmless(f)
                and not (AVOID_HOSTS and AVOID_HOSTS.search(fb_[f]["name"]))
            ),
            key=lambda f: (
                (nlinks[f], f)
                if SINGLE
                else (nlinks[f], -len(harmless(f)), f)
                if NONE
                else (nlinks[f], -far(f), f)
            ),
        )
        any_member = False
        if (
            SINGLE and not cands
        ):  # no folder with a harmless member: any non-resident folder with members
            cands = sorted(
                (f for f in M.div if not fb_[f]["resident"] and P.u32(f + 0x14)),
                key=lambda f: (nlinks[f], f),
            )
            any_member = True

            def partners(F):
                return harmless(F) or any_members(F)

        if resident_at is not None:
            assert (
                resident_at in M.div
                and resident_at != 0xE2960
                and not fb_[resident_at]["resident"]
            ), hex(resident_at)
        order_h = ([resident_at] if resident_at is not None else []) + [
            f for f in cands if f != resident_at
        ]
        print(
            "RESIDENT host candidates in fill order (offset, name, members, harmless members, box, BlockLink lists "
            "naming it, centre distance from city centre):"
        )
        for f in order_h[:5]:
            print(
                f"  {f:#x} {fb_[f]['name']} members {P.u32(f + 0x14)} harmless {len(harmless(f))} box "
                f"{tuple(round(v, 1) for v in fb_[f]['lo'])}..{tuple(round(v, 1) for v in fb_[f]['hi'])} "
                f"links {nlinks[f]} dist {far(f):.0f}"
            )
        cand_rows = [
            (
                f,
                fb_[f]["name"],
                P.u32(f + 0x14),
                len(harmless(f)),
                fb_[f]["lo"],
                fb_[f]["hi"],
                nlinks[f],
                far(f),
            )
            for f in order_h[:5]
        ]
        # Fill hosts in order: carriers already homed in a host stay; the rest swap with its harmless members.
        pending, hosts = (
            list(tgt_ids()) + list(kenv),
            [],
        )  # kept instances fill after the carriers
        ent = {**tiles, **kenv}
        for H in order_h:
            if not pending:
                break
            stay = [n for n in pending if homes[ent[n]["inst"]][0][0] == H]
            rest = [n for n in pending if n not in stay]
            free = partners(H)
            moved = rest[: len(free)]
            for n in stay:
                ent[n].update(folder=H, share=1.0, tier="R", swap=None)
            for n, s_free in zip(moved, free):
                t = ent[n]
                cur = homes[t["inst"]]
                assert len(cur) == 1, (n, cur)
                other = slot_val[s_free]
                s_tile = cur[0][1]
                f_old = cur[0][0]
                struct.pack_into("<I", d, base + s_tile, other)
                struct.pack_into("<I", d, base + s_free, t["inst"])
                slot_val[s_tile], slot_val[s_free] = other, t["inst"]
                homes[other] = [(f_old, s_tile)]
                homes[t["inst"]] = [(H, s_free)]
                swaps.append((n, t["inst"], other, f_old, H))
                t.update(folder=H, share=1.0, tier="R", swap=(other, f_old))
            if stay or moved:
                hosts.append(H)
            pending = [n for n in rest if n not in moved]
        if pending:
            sys.exit(
                f"hosts exhausted: {len(pending)} carriers have no resident host "
                f"(candidates {len(order_h)}, harmless capacity {sum(len(harmless(f)) for f in order_h)})"
            )
        hinfo = []
        for H in hosts:
            old62 = d[base + H + 0x62]
            d[base + H + 0x62] = old62 | 1
            nh = sum(t["folder"] == H for t in tiles.values())
            nk = sum(t["folder"] == H for t in kenv.values())
            hinfo.append(
                dict(
                    node=H,
                    name=fb_[H]["name"],
                    lo=fb_[H]["lo"],
                    hi=fb_[H]["hi"],
                    members=P.u32(H + 0x14),
                    links=nlinks[H],
                    old62=old62,
                    new62=old62 | 1,
                    carriers=nh,
                    kept=nk,
                )
            )
            print(
                f"RESIDENT host {H:#x} {fb_[H]['name']}: {nh} carriers"
                + (f" + {nk} kept env" if kenv else "")
                + f" of {P.u32(H + 0x14)} members; byte +0x62 "
                f"{old62:#04x} -> {old62 | 1:#04x}"
                + (" (given)" if H == resident_at else "")
            )
        host = hosts[0]
        host_info = dict(hinfo[0], cands=cand_rows, any_member=any_member, hosts=hinfo)
        if len(hosts) > 1:
            stats_pre["resident hosts used"] = len(hosts)
    if compact:
        # --compact (compact_folders.py method): each host keeps exactly its carrier slots, moved to the front of its
        # own member array; every other folder (existing resident folders included) gets count 0.
        assert resident or NONE, "--compact needs --resident (or --none)"
        hostset = {h["node"] for h in host_info.get("hosts", [])}
        kept = 0
        for f in M.div:
            arr, cnt = P.u32(f + 0x10), P.u32(f + 0x14)
            sl = [
                struct.unpack_from("<I", d, base + arr + 4 * i)[0] for i in range(cnt)
            ]
            keep = (
                [s for s in sl if slot_val[s] in carriers or slot_val[s] in keep_set]
                if f in hostset
                else []
            )
            kept += len(keep)
            for i, s in enumerate(keep + [s for s in sl if s not in keep]):
                struct.pack_into("<I", d, base + arr + 4 * i, s)
            struct.pack_into("<I", d, base + f + 0x14, len(keep))
            stats_pre["member slots dropped by --compact"] += cnt - len(keep)
        assert kept == len(tiles) + len(kenv), (kept, len(tiles), len(kenv))
        stats_pre["folders compacted"] = len(M.div)
        if host is not None:
            host_info["compact"] = (
                P.u32(host + 0x14),
                sum(t["folder"] == host for t in tiles.values()),
            )
            for h in host_info["hosts"]:
                h["compact"] = (P.u32(h["node"] + 0x14), h["carriers"])
    plans = {} if resident or NONE else {n: choose(n, S3, samp3) for n in ACTIVE}
    # Tiles with no tier-A/B option compete for the resident folder: the worst covered go first.
    no_ab = sorted(
        (n for n in plans if plans[n][0][0][0] not in "AB"),
        key=lambda n: (
            max((sh for tier, f, sh in plans[n][0] if tier == "D"), default=0.0),
            n,
        ),
    )
    order = [] if resident or NONE else [n for n in ACTIVE if n not in no_ab] + no_ab
    for n in order:
        t = tiles[n]
        opts, here, contain = plans[n]
        t["here"], t["contain"] = here, contain
        sc2 = choose(n, S2, samp2)[0]
        t["choice_xz"] = sc2[0][1]["node"]
        cur = homes[t["inst"]]
        assert len(cur) == 1, (n, cur)
        for tier, f, share in opts:
            F = f["node"]
            if cur[0][0] == F:
                t.update(folder=F, share=share, tier=tier, swap=None)
                break
            part = partners(F)
            if not part:
                continue
            s_free = part[0]
            other = slot_val[s_free]
            s_tile = cur[0][1]
            f_old = cur[0][0]
            struct.pack_into("<I", d, base + s_tile, other)
            struct.pack_into("<I", d, base + s_free, t["inst"])
            slot_val[s_tile], slot_val[s_free] = other, t["inst"]
            homes[other] = [(f_old, s_tile)]
            homes[t["inst"]] = [(F, s_free)]
            swaps.append((n, t["inst"], other, f_old, F))
            t.update(folder=F, share=share, tier=tier, swap=(other, f_old))
            break
        else:
            sys.exit(f"tile {n}: no folder has a free harmless member")
        t["first_choice"] = opts[0][1]["node"] == t["folder"]
        t["flagged"] = t["tier"] != "A"

    # 4. Carriers: translation, identity pools, rows.
    stats = collections.Counter(stats_pre)
    stats["pools: " + pool_note] = 1
    world_err = 0.0
    for n, t in tiles.items():
        c = tgt_pos(n)
        i = t["inst"]
        tr = [f32(c[k] - t["inner"][k]) for k in range(3)]
        struct.pack_into("<3f", d, base + i + 0x20, *tr)
        struct.pack_into("<II", d, base + i + 0x2C, rot_ref, scale_ref)
        world = [f32(tr[k] + t["inner"][k]) for k in range(3)]
        world_err = max(world_err, max(abs(world[k] - c[k]) for k in range(3)))
        t["translation"] = tr
        for r, tag in M.rows_of[t["key"]]:
            o = 0x40 + 32 * r
            if tag == "brt":
                struct.pack_into("<I", d, o + 0x14, tgt_model(n))
                stats["brt tile"] += 1
            elif tag == "bhp":
                struct.pack_into("<I", d, o + 0x14, tgt_coll(n))
                stats["bhp tile"] += 1
    if CARRIER_BG_FLAGS is not None:
        before = collections.Counter()
        for n, t in tiles.items():
            b = t["bg"]
            w38, w34 = (
                struct.unpack_from("<I", d, base + b + 0x38)[0],
                struct.unpack_from("<f", d, base + b + 0x34)[0],
            )
            t["bg_before"] = (w38, w34)
            grp = "tile" if n < EXTRA0 else "extra"
            before[f"{grp} +0x38={w38:#x}"] += 1
            if w34 != 0.0:
                before[f"{grp} +0x34={w34:g}"] += 1
            if (w38, w34) != (CARRIER_BG_FLAGS, 0.0):
                stats["carrier BG nodes changed"] += 1
            struct.pack_into("<I", d, base + b + 0x38, CARRIER_BG_FLAGS)
            struct.pack_into("<f", d, base + b + 0x34, 0.0)
        for k, v in sorted(before.items()):
            stats["carrier BG before: " + k] = v
        print(
            "CARRIER BG FLAGS ->", hex(CARRIER_BG_FLAGS), dict(sorted(before.items()))
        )
    if CARRIER_RANGE is not None:
        for n, t in tiles.items():
            b = t["bg"]
            t.setdefault(
                "bg_before",
                (
                    struct.unpack_from("<I", d, base + b + 0x38)[0],
                    struct.unpack_from("<f", d, base + b + 0x34)[0],
                ),
            )
            if struct.unpack_from("<f", d, base + b + 0x34)[0] != f32(CARRIER_RANGE):
                stats["carrier BG range nodes changed"] += 1
            struct.pack_into("<f", d, base + b + 0x34, CARRIER_RANGE)
        print(f"CARRIER BG RANGE -> {CARRIER_RANGE:g} on {len(tiles)} carriers")
    if CARRIER_BG_BOX is not None:
        hit = 0
        for n, t in tiles.items():
            box = CARRIER_BG_BOX.get(tgt_model(n))
            if box is None:
                continue
            b = t["bg"]
            if struct.unpack_from("<6f", d, base + b + 0x3C) != tuple(
                f32(v) for v in box
            ):
                stats["carrier BG box nodes changed"] += 1
            struct.pack_into("<6f", d, base + b + 0x3C, *box)
            hit += 1
        print(
            f"CARRIER BG BOX -> {hit} carriers boxed ({len(CARRIER_BG_BOX)} ids in the map)"
        )
    if LAY_RANGE is not None:
        old = struct.unpack_from("<f", d, base + M.lay + 0x3C)[0]
        struct.pack_into("<f", d, base + M.lay + 0x3C, LAY_RANGE)
        stats["LaySettings +0x3C range"] = f"{old:g} -> {LAY_RANGE:g}"
        print(f"LAY RANGE (LaySettings +0x3C) {old:g} -> {LAY_RANGE:g}")
    for u in sorted({u for t in tiles.values() for u in t.get("couse", [])}):
        x, y, z = struct.unpack_from("<3f", d, base + u + 0x20)
        struct.pack_into("<3f", d, base + u + 0x20, x, y - 30000.0, z)
        stats["co-users parked"] += 1

    # 5. Rename every parked BG key; empty its rows. Empty bhp rows of Attribute-only keys.
    others = [open(p, "rb").read() for p in SEA_LOW if os.path.exists(p)]
    stats["sea_s0_low copies checked"] = len(others)
    rows_arg = arg("--rows-from", ROWS_FROM)
    rows_srcs = [] if rows_arg.lower() == "none" else rows_arg.split(",")
    rsrcs = [open(p, "rb").read() for p in rows_srcs]
    others += rsrcs
    if SINGLE or (NONE and EXTRAS):
        others += [open(p, "rb").read() for p in AVOID if os.path.exists(p)]
        stats["town files checked for key uniqueness"] = sum(
            os.path.exists(p) for p in AVOID
        )
    counter = 0
    renames = {}
    for k in sorted(parked_bg_keys):
        while True:
            digits, v = "", counter
            while True:
                digits = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"[v % 36] + digits
                v //= 36
                if not v:
                    break
            # keep the key's suffix after its 6-character prefix, as make_low_layout.py's tested 8Lw### scheme does
            new = "P" + digits.rjust(5, "0") + k[6:]
            counter += 1
            assert len(new) == len(k) and len(digits) <= 5
            nb = new.encode()
            if nb not in din and nb not in d and all(nb not in o for o in others):
                break
        ob = k.encode()
        expect = len(M.rows_of[k]) + 1  # rows + one pooled string
        assert bytes(d).count(ob) == expect, (k, bytes(d).count(ob), expect)
        d = bytearray(bytes(d).replace(ob, nb))
        assert bytes(d).count(nb) == expect and ob not in d
        renames[k] = new
        for r, tag in M.rows_of[k]:
            o = 0x40 + 32 * r
            if tag == "brt":
                struct.pack_into("<I", d, o + 0x14, EMPTY_MODEL)
                stats["brt parked -> FFFF2100"] += 1
            elif tag == "bhp":
                struct.pack_into("<I", d, o + 0x14, EMPTY_COLL)
                stats["bhp parked -> FFFF002F"] += 1
    stats["keys renamed"] = len(renames)
    tile_keys = {t["key"] for t in tiles.values()}
    for i, k, tag, rid in M.rows:
        if tag == "bhp" and k not in tile_keys and k not in renames:
            struct.pack_into("<I", d, 0x40 + 32 * i + 0x14, EMPTY_COLL)
            stats["bhp other keys (Attribute-only or unplaced) -> FFFF002F"] += 1

    # 6. Effects.
    if not keep_fx:
        couse = {u for t in tiles.values() for u in t.get("couse", [])}
        for n in M.inst:
            if n in carriers or n in couse or n in keep_set:
                continue
            b = M.P.u32(n + 0x3C)
            c = M.cls(b)
            if c.startswith("UnitTree"):
                if not any(M.cls(x) in EFFECT_LEAVES for x, _, _ in M.leaves[n]):
                    continue
                tag = "UnitTree with effect leaves"
            elif c in BG + ("AttributeBaseObject", "none"):
                continue
            else:
                tag = c
            x, y, z = struct.unpack_from("<3f", d, base + n + 0x20)
            struct.pack_into("<3f", d, base + n + 0x20, x, y - 30000.0, z)
            stats["Y-parked " + tag] += 1

    # 7. Rows from --rows-from (default layout-L4.DAT) that it has over the pristine town layout (the zone-500
    # texture rows) and the input lacks (key+tag): inserted at the end of the row table as add_layout_rows.py does.
    # --rows-from a,b,...: several files, in order. --rows-for m1,m2,...: copy only rows whose key+tag a model file
    # references (key regex as layout_rows.py:25, tags bxt/brt/bhp).
    added = []
    if rsrcs:
        have = {(k, t) for _, k, t, _ in M.rows}
        orig = open(arg("--orig", ORIG), "rb").read()
        have |= {
            (
                orig[0x40 + 32 * i : 0x50 + 32 * i].split(b"\0")[0].decode("latin1"),
                orig[0x50 + 32 * i : 0x54 + 32 * i].split(b"\0")[0].decode("latin1"),
            )
            for i in range(struct.unpack_from("<I", orig, 0x24)[0])
        }
        wanted = None
        if "--rows-for" in ARGV:
            import re

            wanted = set()
            for mp in arg("--rows-for", "").split(","):
                for mt in re.finditer(
                    rb"([0-9][A-Za-z0-9]{5}[a-z0-9_]{1,9})\x00(bxt|brt|bhp)",
                    open(mp, "rb").read(),
                ):
                    wanted.add((mt.group(1).decode(), mt.group(2).decode()))
            stats["--rows-for keys referenced by the models"] = len(wanted)
        for rp, rsrc in zip(rows_srcs, rsrcs):
            n0 = len(added)
            rc = struct.unpack_from("<I", rsrc, 0x24)[0]
            for i in range(rc):
                row = rsrc[0x40 + 32 * i : 0x60 + 32 * i]
                k, t = (
                    row[:16].split(b"\0")[0].decode("latin1"),
                    row[16:20].split(b"\0")[0].decode("latin1"),
                )
                if (k, t) not in have and (wanted is None or (k, t) in wanted):
                    assert k not in renames and k not in set(renames.values()), k
                    have.add((k, t))
                    added.append(row)
            stats[
                "rows appended from "
                + os.path.basename(os.path.dirname(rp))
                + "\\"
                + os.path.basename(rp)
            ] = len(added) - n0
        if wanted is not None:
            stats["--rows-for keys still without a row"] = len(
                {w for w in wanted if w not in have}
            )
    # --rows-add <json>: [{"key": "<15-char key>", "tag": "bxt", "id": "FFFF0110"}, ...] = rows for textures that
    # exist in no retail layout (cloned donor shader slots). Built on the input's first row of the same
    # tag (flags copied), key and id replaced; a key+tag the table already has is an error.
    if "--rows-add" in ARGV:
        have_all = {(k, t) for _, k, t, _ in M.rows} | {
            (
                r[:16].split(b"\0")[0].decode("latin1"),
                r[16:20].split(b"\0")[0].decode("latin1"),
            )
            for r in added
        }
        n0 = len(added)
        for spec in json.load(open(arg("--rows-add", ""))):
            k, t, rid = spec["key"], spec.get("tag", "bxt"), int(spec["id"], 16)
            assert 1 <= len(k) <= 15 and len(t) == 3, spec
            if (k, t) in have_all:
                sys.exit(f"--rows-add: row {k} {t} already exists")
            # flags template: an appended row of the same tag first (retail texture rows carry +0x19 = 01, the
            # Limsa town's own bxt rows 00), else the input's first row of that tag
            tmpl = next(
                (
                    r
                    for r in reversed(added)
                    if r[16:20].split(b"\0")[0].decode("latin1") == t
                ),
                None,
            ) or next(
                (
                    d[0x40 + 32 * i : 0x60 + 32 * i]
                    for i, (_, _, tt, _) in enumerate(M.rows)
                    if tt == t
                ),
                None,
            )
            if tmpl is None:
                sys.exit(f"--rows-add: no input row with tag {t} to copy flags from")
            row = bytearray(tmpl)
            row[0:16] = k.encode("latin1").ljust(16, b"\0")
            row[16:20] = t.encode("latin1").ljust(4, b"\0")
            struct.pack_into("<I", row, 20, rid)
            have_all.add((k, t))
            added.append(bytes(row))
        stats["rows added (--rows-add)"] = len(added) - n0
    # --rows-override <json>: {"<key>": "FFFF0110", ...} = new DAT id for APPENDED rows of that key (retail
    # texture keys of the cloned slots re-pointed at our texture files). A key not among the appended rows is an error.
    if "--rows-override" in ARGV:
        ov = {
            k: int(v, 16)
            for k, v in json.load(open(arg("--rows-override", ""))).items()
        }
        hit = 0
        for i, row in enumerate(added):
            k = row[:16].split(b"\0")[0].decode("latin1")
            if k in ov:
                r = bytearray(row)
                struct.pack_into("<I", r, 20, ov[k])
                added[i] = bytes(r)
                hit += 1
                ov.pop(k)
        if ov:
            sys.exit(f"--rows-override: keys not among the appended rows: {sorted(ov)}")
        stats["rows overridden (--rows-override)"] = hit
    if added:
        end, count = struct.unpack_from("<II", d, 0x20)
        assert end == 0x40 + 32 * count and d[end : end + 7] == b"SEDBlyb"
        d[end:end] = b"".join(added)
        struct.pack_into("<II", d, 0x20, end + 32 * len(added), count + len(added))

    open(out, "xb").write(d)
    chk = check(din, bytes(d), R, quiet=True)
    print(
        f"AUTHOR wrote {out} ({len(d)} bytes); {dict(stats)}; check {'OK' if chk['ok'] else 'FAILED'}"
    )
    if report_p and NONE and not resident:
        print("no report in --none mode (use --check)")
    elif report_p and resident:
        write_report_resident(
            report_p,
            src,
            out,
            M,
            tiles,
            swaps,
            stats,
            ident,
            chk,
            host_info,
            world_err,
            len(M.rows),
            len(M.rows) + len(added),
            len(renames),
        )
    elif report_p:
        write_report(
            report_p,
            src,
            out,
            M,
            S3,
            samp3,
            samp2,
            best_diff,
            created_diff,
            tiles,
            swaps,
            stats,
            ident,
            chk,
            R,
            world_err,
            unique_pool,
            renames,
        )
    if not chk["ok"]:
        sys.exit("check failed: " + "; ".join(chk["errors"]))


def classify(din, dout):
    """Changed-byte categories between input and output; anything unexplained lands in OTHER."""
    Mi, Mo = Model(din), Model(dout)
    cat = collections.Counter()
    other = []
    B = Mi.base
    nrows = len(Mi.rows)
    pool_keys = {}
    for b in Mi.L.node_types:
        if Mi.cls(b) in BG:
            ref = Mi.P.u32(b + 0x20)
            pool_keys[ref] = len(Mi.P.string(ref))
    pool_spans = sorted((B + r, B + r + n) for r, n in pool_keys.items())
    starts = [a for a, _ in pool_spans]
    kinds = sorted(
        (k.start, k.start + k.count * k.size, k) for k in Mi.L.types if k.count
    )
    kstarts = [a for a, _, _ in kinds]
    seen_nodes = collections.defaultdict(set)
    member_slots = {
        s for h in Mi.homes.values() for _, s in h
    }  # slot nodes reachable from a member array
    arrays = sorted(
        (B + Mi.P.u32(f + 0x10), B + Mi.P.u32(f + 0x10) + 4 * Mi.P.u32(f + 0x14), f)
        for f in Mi.div
        if Mi.P.u32(f + 0x14)
    )
    astarts = [a for a, _, _ in arrays]
    for i in range(len(din)):
        if din[i] == dout[i]:
            continue
        j = bisect.bisect_right(astarts, i) - 1
        if (
            j >= 0 and arrays[j][0] <= i < arrays[j][1]
        ):  # validated as a permutation in check()
            seen_nodes["member arrays reordered"].add(arrays[j][2])
            continue
        if 0x40 <= i < 0x40 + 32 * nrows:
            r, f = divmod(i - 0x40, 32)
            if f < 16:
                seen_nodes["row names renamed"].add(r)
            elif 0x14 <= f < 0x18:
                seen_nodes[f"row ids changed ({Mi.rows[r][2]})"].add(r)
            else:
                other.append(i)
            continue
        if i >= B:
            j = bisect.bisect_right(starts, i) - 1
            if j >= 0 and pool_spans[j][0] <= i < pool_spans[j][1]:
                seen_nodes["pooled key strings renamed"].add(pool_spans[j][0])
                continue
            p = i - B
            j = bisect.bisect_right(kstarts, p) - 1
            if j >= 0 and kinds[j][0] <= p < kinds[j][1]:
                k = kinds[j][2]
                node = k.start + (p - k.start) // k.size * k.size
                off = p - node
                c = Mi.cls(node) if k.size >= 12 else ""
                if c == "InstanceObject" and 0x20 <= off < 0x2C:
                    seen_nodes["instance translations"].add(node)
                    continue
                if c == "InstanceObject" and 0x2C <= off < 0x34:
                    seen_nodes["instance rot/scale refs"].add(node)
                    continue
                if k.size == 4 and k.fields == ((0, 0),) and node in member_slots:
                    seen_nodes["member slot contents swapped"].add(node)
                    continue
                if (
                    c == "DivideMapFolderObject"
                    and off == 0x62
                    and din[i] ^ dout[i] == 1
                    and dout[i] & 1
                ):
                    seen_nodes["resident bit on host folder"].add(node)
                    continue
                if (
                    c in BG and 0x34 <= off < 0x38 and CARRIER_RANGE is not None
                ):  # validated in check()
                    seen_nodes["carrier BG range"].add(node)
                    continue
                if (
                    c in BG and 0x34 <= off < 0x3C
                ):  # --carrier-bg-flags; validated in check()
                    seen_nodes["carrier BG flags"].add(node)
                    continue
                if (
                    c in BG and 0x3C <= off < 0x54 and CARRIER_BG_BOX is not None
                ):  # validated in check()
                    seen_nodes["carrier BG box"].add(node)
                    continue
                if (
                    c == "LaySettingsObject"
                    and 0x3C <= off < 0x40
                    and LAY_RANGE is not None
                ):  # validated in check()
                    seen_nodes["LaySettings range"].add(node)
                    continue
                if (
                    c == "DivideMapFolderObject" and 0x14 <= off < 0x18
                ):  # validated as a decrease in check()
                    seen_nodes["member counts reduced"].add(node)
                    continue
        other.append(i)
    for k, v in seen_nodes.items():
        cat[k] = len(v)
    return cat, other, Mi, Mo


def check(din, dout_full, R, quiet=False):
    errors, warnings = [], []
    # Rows appended at the end of the row table: verify, then strip them so the rest compares byte for byte.
    nin, nout = (
        struct.unpack_from("<I", din, 0x24)[0],
        struct.unpack_from("<I", dout_full, 0x24)[0],
    )
    added = nout - nin
    end_in = 0x40 + 32 * nin
    if (
        added < 0
        or struct.unpack_from("<I", dout_full, 0x20)[0]
        != struct.unpack_from("<I", din, 0x20)[0] + 32 * added
    ):
        errors.append(f"row header inconsistent: rows {nin} -> {nout}")
        added = max(added, 0)
    if len(dout_full) != len(din) + 32 * added:
        errors.append(
            f"size {len(din)} -> {len(dout_full)} (expected + 32 x {added} rows)"
        )
    appended = [dout_full[end_in + 32 * i : end_in + 32 * i + 32] for i in range(added)]
    have = {
        (din[0x40 + 32 * i : 0x50 + 32 * i], din[0x50 + 32 * i : 0x54 + 32 * i])
        for i in range(nin)
    }
    for r in appended:
        if (r[:16], r[16:20]) in have:
            errors.append(f"appended row duplicates an input row: {r[:16]!r}")
    dout = bytearray(dout_full[:end_in] + dout_full[end_in + 32 * added :])
    dout[0x20:0x28] = din[0x20:0x28]
    dout = bytes(dout)
    if len(din) != len(dout):
        errors.append(f"size after stripping rows {len(din)} -> {len(dout)}")
    cat, other, Mi, Mo = classify(din, dout)
    if added:
        cat["rows appended"] = added
    if other:
        errors.append(f"{len(other)} unexplained changed bytes, first at {other[0]:#x}")
    B = Mi.base

    def span(d, node, size):
        return d[B + node : B + node + size]

    res_set, reduced, reordered = [], [], []
    slots_in, slots_out = collections.Counter(), collections.Counter()
    for f in Mi.div:
        a, b = bytearray(span(din, f, 0x70)), bytearray(span(dout, f, 0x70))
        if a != b and a[0x62] ^ b[0x62] == 1 and b[0x62] & 1:
            res_set.append(f)
            a[0x62] = b[0x62]  # the one expected folder change: resident bit set
        cin, cout = (
            struct.unpack_from("<I", a, 0x14)[0],
            struct.unpack_from("<I", b, 0x14)[0],
        )
        if cout < cin:
            reduced.append(f)
            a[0x14:0x18] = b[0x14:0x18]  # --compact: count decrease only
        if a != b:
            errors.append(f"folder record {f:#x} changed")
        arr = Mi.P.u32(f + 0x10)
        wi = struct.unpack_from(f"<{cin}I", din, B + arr)
        wo = struct.unpack_from(f"<{cin}I", dout, B + arr)
        if wi != wo:
            reordered.append(f)
            if sorted(wi) != sorted(wo):
                errors.append(
                    f"member array {f:#x} is not a reordering of its own slots"
                )
        for s in wi:
            slots_in[struct.unpack_from("<I", din, B + s)[0]] += 1
        for s in wo:
            slots_out[struct.unpack_from("<I", dout, B + s)[0]] += 1
        pr, pc = Mi.P.u32(f + 0x58), Mi.P.u32(f + 0x5C)
        if din[B + pr : B + pr + 8 * pc] != dout[B + pr : B + pr + 8 * pc]:
            errors.append(f"pair array {f:#x} changed")
    for b in Mi.blk:
        if span(din, b, 0x18) != span(dout, b, 0x18):
            errors.append(f"BlockLink {b:#x} changed")
        lr, lc = Mi.P.u32(b + 0x10), Mi.P.u32(b + 0x14)
        if din[B + lr : B + lr + 4 * lc] != dout[B + lr : B + lr + 4 * lc]:
            errors.append(f"BlockLink list {b:#x} changed")
    lay_in, lay_out = (
        bytearray(span(din, Mi.lay, 0x70)),
        bytearray(span(dout, Mi.lay, 0x70)),
    )
    if LAY_RANGE is not None:
        if struct.unpack_from("<f", lay_out, 0x3C)[0] != f32(LAY_RANGE):
            errors.append(f"LaySettings +0x3C != {LAY_RANGE:g}")
        lay_in[0x3C:0x40] = lay_out[0x3C:0x40]
    if lay_in != lay_out:
        errors.append(
            "LaySettings changed" + ("" if LAY_RANGE is None else " outside +0x3C")
        )
    zt = Mo.P.floats(Mo.lay + 0x40)
    if zt != Mi.P.floats(Mi.lay + 0x40):
        errors.append(f"zone translation {zt} differs from input")
    boxes_equal = all(
        Mi.vec(f + 0x30) == Mo.vec(f + 0x30) and Mi.vec(f + 0x40) == Mo.vec(f + 0x40)
        for f in Mi.div
    )
    if not boxes_equal:
        errors.append("folder boxes differ")
    tbl = Mi.P.u32(0x10)
    end = Mi.P.u32(tbl + 4)
    if din[B : B + end] != dout[B : B + end]:
        errors.append("lyb header/object list/node-type table changed")
    if [(k.start, k.count, k.size) for k in Mi.L.types] != [
        (k.start, k.count, k.size) for k in Mo.L.types
    ]:
        errors.append("extents changed")
    # Slot contents over every original member slot (counted or not) must be a permutation (swaps only).
    if slots_in != slots_out:
        errors.append("member slot contents are not a permutation")
    # Census on the output, against the input's own baseline.
    oi, oo = C.observe(Mi.L), C.observe(C.parse_layout(0, bytes(dout_full)))
    fails_i = {k: v for k, v in oi.counts.items() if k.endswith(".fail") and v}
    fails_o = {k: v for k, v in oo.counts.items() if k.endswith(".fail") and v}
    exp = collections.Counter(
        fails_i
    )  # appended rows carry retail-style nonzero +0x18 flags
    for r in appended:
        if any(r[0x18:0x20]):
            exp["reserved_row_zero.fail"] += 1
            exp[
                "reserved_row_zero."
                + ("resource" if r[16:20].strip(b"\0") else "marker")
                + ".fail"
            ] += 1
    # --carrier-bg-flags moves census tests BG.+0x38==0x100 and BG.+0x34==0 for the changed carrier nodes
    for b in (
        x
        for x in Mi.L.node_types
        if Mi.cls(x) in BG
        and din[B + x + 0x34 : B + x + 0x3C] != dout[B + x + 0x34 : B + x + 0x3C]
    ):
        for off, ok_val, name in (
            (0x38, 0x100, "BG.+0x38==256.fail"),
            (0x34, 0, "BG.+0x34==0.fail"),
        ):
            was = struct.unpack_from("<I", din, B + b + off)[0] != ok_val
            now = struct.unpack_from("<I", dout, B + b + off)[0] != ok_val
            exp[name] += int(now) - int(was)
    fails_i = {k: v for k, v in exp.items() if v}
    if fails_o != fails_i:
        errors.append(f"census fails differ: {fails_i} -> {fails_o}")
    census = dict(
        fails_out=fails_o,
        fails_in=fails_i,
        sedb=oo.counts["payload_size_equals_SEDB_total.pass"] == 1,
        disjoint=(
            oo.counts["declared_extent_disjoint.pass"],
            oo.counts["declared_extent_disjoint.fail"],
        ),
        resolved=(
            oo.counts["resource_key_resolved.pass"],
            oo.counts["resource_key_resolved.fail"],
        ),
    )
    # Carriers in the output: brt row with the target model id -> key -> BG node -> live instances -> home folder.
    S = Stream(Mo, R)
    tiles = {}
    for n in tgt_ids():
        # the carrier key is found by whichever of its two ids is unique in the table (a model override may reuse
        # the shared empty model FFFF2100; an extra's collision may be the shared empty FFFF002F)
        ks = [k for _, k, tag, rid in Mo.rows if tag == "brt" and rid == tgt_model(n)]
        if len(ks) != 1:
            ks = [
                k for _, k, tag, rid in Mo.rows if tag == "bhp" and rid == tgt_coll(n)
            ]
        if len(ks) != 1:
            errors.append(
                f"{tgt_name(n)}: neither {tgt_model(n):#x} (brt) nor {tgt_coll(n):#x} (bhp) identifies "
                f"one key"
            )
            continue
        k = ks[0]
        brt = [r for _, kk, tt, r in Mo.rows if kk == k and tt == "brt"]
        if brt != [tgt_model(n)]:
            errors.append(f"{tgt_name(n)}: brt ids {[hex(v) for v in brt]}")
        bgs = [b for b in Mo.L.node_types if Mo.cls(b) in BG and Mo.key(b) == k]
        live = [u for b in bgs for u in Mo.users[b] if Mo.y(u) > -20000]
        hs = [h for u in live for h in Mo.homes[u]]
        c = tgt_pos(n)
        world = (
            [
                Mo.vec(live[0] + 0x20)[j]
                + sum(
                    v[j]
                    for v in next(ch for b, ch, _ in Mo.leaves[live[0]] if b in bgs)
                )
                for j in range(3)
            ]
            if len(live) == 1
            else None
        )
        one = len(live) == 1 and len(hs) == 1
        ok = one and hs[0][0] in S.created(c)
        bhp = [r for _, kk, tt, r in Mo.rows if kk == k and tt == "bhp"]
        tiles[n] = dict(
            key=k,
            live=live,
            homes=[f for f, _ in hs],
            ok=ok,
            world=world,
            bhp=bhp,
            bgs=bgs,
        )
        if not one:
            errors.append(f"tile {n}: live instances {len(live)}, homes {len(hs)}")
        elif not ok:
            warnings.append(
                f"tile {n}: home {Mo.fname[hs[0][0]]} not created at cell centre"
            )
        if bhp != [tgt_coll(n)] and not (
            (SINGLE or (n >= EXTRA0 and tgt_coll(n) == EMPTY_COLL)) and not bhp
        ):
            errors.append(f"tile {n}: bhp ids {[hex(v) for v in bhp]}")
        if not bhp:
            warnings.append(
                f"tile {n}: carrier key has no bhp row (collision id not set)"
            )
        if world is None or any(abs(world[j] - c[j]) > 1e-4 for j in range(3)):
            errors.append(f"tile {n}: layout-space placement {world} != target {c}")
        if res_set and (
            not one or hs[0][0] not in res_set or not dout[B + hs[0][0] + 0x62] & 1
        ):
            errors.append(f"{tgt_name(n)}: home is not a resident host")
    if sorted(tiles) != tgt_ids():
        errors.append(f"tile rows {sorted(tiles)}")
    tile_inst = {u for t in tiles.values() for u in t["live"]}
    # carrier BG flags: only carrier BG nodes may change at +0x34..+0x3B, and with --carrier-bg-flags all must hold it
    carrier_bgs = {b for t in tiles.values() for b in t.get("bgs", [])}
    bg_changed = {
        b
        for b in Mi.L.node_types
        if Mi.cls(b) in BG
        and din[B + b + 0x34 : B + b + 0x3C] != dout[B + b + 0x34 : B + b + 0x3C]
    }
    if bg_changed - carrier_bgs:
        errors.append(
            f"{len(bg_changed - carrier_bgs)} non-carrier BG nodes changed at +0x34..+0x3B"
        )
    want34 = f32(CARRIER_RANGE) if CARRIER_RANGE is not None else 0.0
    if CARRIER_BG_FLAGS is not None:
        bad = [
            b
            for b in carrier_bgs
            if struct.unpack_from("<I", dout, B + b + 0x38)[0] != CARRIER_BG_FLAGS
        ]
        if bad:
            errors.append(
                f"{len(bad)} carrier BG nodes lack +0x38={CARRIER_BG_FLAGS:#x}"
            )
    if CARRIER_BG_FLAGS is not None or CARRIER_RANGE is not None:
        bad = [
            b
            for b in carrier_bgs
            if struct.unpack_from("<f", dout, B + b + 0x34)[0] != want34
        ]
        if bad:
            errors.append(f"{len(bad)} carrier BG nodes lack +0x34={want34:g}")
        if CARRIER_BG_FLAGS is None and any(
            din[B + b + 0x38 : B + b + 0x3C] != dout[B + b + 0x38 : B + b + 0x3C]
            for b in bg_changed
        ):
            errors.append("BG +0x38 changed without --carrier-bg-flags")
    elif bg_changed:
        errors.append(
            f"{len(bg_changed)} BG nodes changed at +0x34..+0x3B without --carrier-bg-flags/--carrier-range"
        )
    # carrier BG box: only carrier BG nodes may change at +0x3C..+0x53, each to its mapped box
    box_changed = {
        b
        for b in Mi.L.node_types
        if Mi.cls(b) in BG
        and din[B + b + 0x3C : B + b + 0x54] != dout[B + b + 0x3C : B + b + 0x54]
    }
    if box_changed - carrier_bgs:
        errors.append(
            f"{len(box_changed - carrier_bgs)} non-carrier BG nodes changed at +0x3C..+0x53"
        )
    if CARRIER_BG_BOX is not None:
        for n, t in tiles.items():
            box = CARRIER_BG_BOX.get(tgt_model(n))
            for b in t.get("bgs", []):
                have = struct.unpack_from("<6f", dout, B + b + 0x3C)
                if box is not None and have != tuple(f32(v) for v in box):
                    errors.append(
                        f"{tgt_name(n)}: carrier BG box {have} != requested {box}"
                    )
                if box is None and b in box_changed:
                    errors.append(
                        f"{tgt_name(n)}: carrier BG box changed without a mapping"
                    )
    elif box_changed:
        errors.append(
            f"{len(box_changed)} BG nodes changed at +0x3C..+0x53 without --carrier-bg-box"
        )
    counted = [(f, i) for i, h in Mo.homes.items() for f, _ in h]
    nontile_l = [(f, i) for f, i in counted if i not in tile_inst]
    nontile = len(nontile_l)
    # --keep-env: the non-carrier counted members must be exactly KEEP_ENV env-bound UnitTree instances inside the
    # footprint, homed once in a resident host, with translation, rotation and scale untouched (Y included).
    kept_info = []
    if KEEP_ENV:
        if nontile != KEEP_ENV or len({i for _, i in nontile_l}) != KEEP_ENV:
            errors.append(
                f"--keep-env {KEEP_ENV}: {nontile} non-carrier counted members ({len({i for _, i in nontile_l})} distinct)"
            )
        for f, i in nontile_l:
            envs = env_of(Mo, i)
            lv = Mo.leaves[i]
            same = din[B + i + 0x20 : B + i + 0x34] == dout[B + i + 0x20 : B + i + 0x34]
            info = dict(
                inst=i,
                host=f,
                envs=[Mo.P.string(Mo.P.u32(e + 8)).decode("latin1") for e in envs],
                pos=Mo.vec(i + 0x20),
                lights=sum(Mo.cls(x) == "LightBaseObject" for x, _, _ in lv),
                fx=sum(
                    Mo.cls(x) in EFFECT_LEAVES and Mo.cls(x) != "LightBaseObject"
                    for x, _, _ in lv
                ),
            )
            kept_info.append(info)
            if not envs:
                errors.append(f"kept {i:#x}: binds no DrawEnv")
            if not in_footprint(Mo, i):
                errors.append(f"kept {i:#x}: outside the footprint")
            if not same:
                errors.append(f"kept {i:#x}: translation/rotation/scale changed")
            if f not in res_set or not dout[B + f + 0x62] & 1:
                errors.append(f"kept {i:#x}: home {f:#x} is not a resident host")
            if Mo.y(i) <= -20000:
                errors.append(f"kept {i:#x}: Y-parked")
        cat["kept env instances (Y unchanged)"] = len(kept_info)
    elif reduced and nontile:
        errors.append(f"{nontile} non-tile instances are counted folder members")
    if reduced and len(counted) != len(tile_inst) + (KEEP_ENV or 0):
        errors.append(
            f"counted members {len(counted)} != carriers"
            + (f" + {KEEP_ENV} kept" if KEEP_ENV else "")
        )
    if NONE and not EXTRAS and counted:
        errors.append(f"--none: {len(counted)} counted folder members remain")
    if not quiet:
        print(
            f"  carrier BG nodes changed at +0x34..+0x3B: {len(bg_changed)} (all carriers: {not (bg_changed - carrier_bgs)}; "
            f"flags {'%#x' % CARRIER_BG_FLAGS if CARRIER_BG_FLAGS is not None else 'not requested'}; range "
            f"{'%g' % CARRIER_RANGE if CARRIER_RANGE is not None else 'not requested'})"
        )
    res = dict(
        ok=not errors,
        errors=errors,
        warnings=warnings,
        resident_set=res_set,
        reduced=len(reduced),
        reordered=len(reordered),
        counted=len(counted),
        nontile=nontile,
        categories=dict(cat),
        other=len(other),
        census=census,
        tiles=tiles,
        kept=kept_info,
        zone=zt,
        boxes_equal=boxes_equal,
        size=(len(din), len(dout_full)),
    )
    if not quiet:
        print(
            f"CHECK size {len(din)} -> {len(dout_full)} (+32 x {added} appended rows)"
        )
        for k, v in sorted(cat.items()):
            print(f"  changed: {k}: {v}")
        print(f"  unexplained changed bytes: {len(other)}")
        for f in res_set:
            nh = sum(1 for i, h in Mo.homes.items() for ff, _ in h if ff == f)
            print(
                f"  resident bit set on host folder {f:#x} {Mi.fname[f]}: +0x62 {din[B + f + 0x62]:#04x} -> "
                f"{dout[B + f + 0x62]:#04x}; counted members {nh} ("
                f"{sum(i in tile_inst for i, h in Mo.homes.items() for ff, _ in h if ff == f)} carriers, "
                f"{sum(i not in tile_inst for i, h in Mo.homes.items() for ff, _ in h if ff == f)} kept env)"
            )
        print(
            f"  member counts reduced on {len(reduced)} folders, arrays reordered within their own range on "
            f"{len(reordered)}; counted members {len(counted)}, of them non-carrier {nontile}"
        )
        print(
            f"  folders {len(Mi.div)} records (apart from that bit and counts)/pairs unchanged, boxes equal: "
            f"{boxes_equal}; BlockLinks {len(Mi.blk)} unchanged; zone translation {zt}"
        )
        print(
            f"  census: parse ok, SEDB size match {census['sedb']}, extent disjoint pass/fail {census['disjoint']}, "
            f"keys resolved pass/fail {census['resolved']}, fails {fails_o} (input {fails_i})"
        )
        print(
            f"  carriers {len(tiles)}: one live instance + one home created at target: "
            f"{sum(t['ok'] for t in tiles.values())}"
        )
        for n in sorted(tiles):
            t = tiles[n]
            print(
                f"    tile {n:2d} key {t['key']} home {[Mo.fname[f] for f in t['homes']]} bhp "
                f"{[hex(v) for v in t['bhp']]} placement "
                f"{None if t['world'] is None else tuple(round(v, 4) for v in t['world'])}"
            )
        if KEEP_ENV:
            print(
                f"  kept env instances {len(kept_info)} (counted members of resident hosts, original Y; LightBase leaves "
                f"{sum(k['lights'] for k in kept_info)}, other effect leaves {sum(k['fx'] for k in kept_info)}):"
            )
            for k in sorted(kept_info, key=lambda k: k["inst"]):
                print(
                    f"    {k['inst']:#x} {','.join(k['envs'])} pos ({k['pos'][0]:.1f}, {k['pos'][1]:.1f}, {k['pos'][2]:.1f}) "
                    f"host {k['host']:#x} {Mo.fname[k['host']]} lights {k['lights']} other fx {k['fx']}"
                )
        for w in warnings:
            print("  note:", w)
        print(
            "CHECK",
            "OK (structure)" if not errors else "FAILED: " + "; ".join(errors),
            f"; carriers home created at target {sum(t['ok'] for t in tiles.values())}/{len(tiles)}",
        )
    return res


def write_report(
    path,
    src,
    out,
    M,
    S,
    samp3,
    samp2,
    best_diff,
    created_diff,
    tiles,
    swaps,
    stats,
    ident,
    chk,
    R,
    world_err,
    unique_pool,
    renames,
):
    fb = {f["node"]: f for f in S.F}
    W = []
    W.append(
        f"# layout authoring report (per-cell)\n\nAll positions are layout space; world = layout + zone "
        f"translation {WORLD}.\n\n"
        f"Input `{src}`, output `{out}` ({chk['size'][1]} bytes). "
        f"Query radius R = {R:g} (scene 16 + manager radius); folder priorities all "
        f"{sorted({f['prio'] for f in S.F})}; resident folders {[fb[r]['name'] for r in S.res]}.\n"
    )
    W.append("## lyb identity\n")
    if ident.get("identical"):
        W.append("Payload byte-identical to the pristine layout; slot offsets valid.\n")
    else:
        W.append(
            f"Payload NOT byte-identical to `{ident['orig']}`: {ident['len']} vs {ident['len_orig']} bytes "
            f"(+{ident.get('tail_bytes')} appended). Differences by owner: {ident.get('diff_owners')}. "
            f"Extent records moved: {ident.get('extents_changed')}. The resource table is identical (1227 rows "
            f"in both); the extra size is in the lyb payload. No InstanceObject, UnitTree, BG, Attribute, "
            f"DivideMap or BlockLink byte differs, so the slot instance offsets are valid "
            f"(slots_valid={ident['slots_valid']}).\n"
        )
    W.append("## Headline\n")
    tiers = collections.Counter(t["tier"] for t in tiles.values())
    holes_total = 0
    for n in ACTIVE:
        t = tiles[n]
        c = cell(n)
        hs = [
            p
            for p in SAMPLES
            if math.dist((p[0], p[2]), (c[0], c[2])) <= 80.0
            and t["folder"] not in samp3[p]
        ]
        t["holes"] = hs
        holes_total += len(hs)
    variants = []
    for label, Sv in (
        ("R 112 (16 + LaySettings +0x3C 96.0)", Stream(M, 112.0)),
        ("box-distance gate", Stream(M, R, box_gate=True)),
        ("XZ best folder", Stream(M, R, xz_best=True)),
    ):
        cen = sum(tiles[n]["folder"] in Sv.created(cell(n)) for n in ACTIVE)
        hv = sum(
            1
            for n in ACTIVE
            for p in SAMPLES
            if math.dist((p[0], p[2]), (cell(n)[0], cell(n)[2])) <= 80.0
            and tiles[n]["folder"] not in Sv.created(p)
        )
        variants.append(
            f"{label}: homes created at centre {cen}/{len(ACTIVE)}, holes {hv}"
        )
    empty_pts = sum(1 for p in SAMPLES if not (samp3[p] - S.res))
    kept = sum(t["how"].startswith("kept (") for t in tiles.values())
    rep = sum(t["how"].startswith("re-picked (") for t in tiles.values())
    cou = sum("co-users" in t["how"] for t in tiles.values())
    W.append(
        f"- tiles with a containing folder created at the cell centre (tier A): {tiers['A']} of {len(ACTIVE)}; "
        f"tier B (created at centre, not containing) {tiers['B']}; tier C (resident folder) {tiers['C']}; "
        f"tier D (nothing but the resident folder is created at the centre; best nearby coverage) {tiers['D']}\n"
        f"- sample points where no non-resident block is created at all: {empty_pts} of {len(SAMPLES)}\n"
        f"- variants (same homes): {'; '.join(variants)}\n- carriers: {kept} kept (single-placement key), {rep} "
        f"re-picked to a single-placement key, {cou} with co-users parked; single-placement carriers "
        f"available {len(unique_pool)}\n- slot swaps {len(swaps)}\n- predicted holes (sample points within 80 m "
        f"of a tile where its folder is not created): {holes_total} over "
        f"{sum(bool(tiles[n]['holes']) for n in ACTIVE)} tiles\n- {', '.join(f'{k} {v}' for k, v in sorted(stats.items()))}"
        f"\n- tile world placement error vs cell centre (float32): {world_err:g}\n- best-folder choice 3D vs XZ "
        f"differs at {best_diff} of {len(SAMPLES)} sample points; created sets differ at {created_diff}; tiles "
        f"whose folder choice differs under XZ: {[n for n in ACTIVE if tiles[n]['choice_xz'] != tiles[n]['folder']]}"
        f"\n- check: {'OK' if chk['ok'] else 'FAILED ' + '; '.join(chk['errors'])}; census fails "
        f"{chk['census']['fails_out']} (input {chk['census']['fails_in']}), SEDB size match "
        f"{chk['census']['sedb']}, extent disjoint pass/fail {chk['census']['disjoint']}\n"
    )
    W.append(
        "## Tiles\n\n| tile | cell centre | carrier | how | folder | box min / max | share <=100 m | contain/created | holes <=80 m |\n|---|---|---|---|---|---|---|---|---|"
    )
    for n in ACTIVE:
        t = tiles[n]
        f = fb[t["folder"]]
        c = cell(n)
        holes = " ".join(f"({p[0]:g},{p[1]:g},{p[2]:g})" for p in t["holes"]) or "-"
        W.append(
            f"| {n} | ({c[0]:.1f}, {c[2]:.1f}) | {t['inst']:#x} {t['key']} | {t['how']} | {f['name']} {f['node']:#x}"
            f" tier {t['tier']} | ({', '.join(f'{v:.1f}' for v in f['lo'])}) / "
            f"({', '.join(f'{v:.1f}' for v in f['hi'])}) | {t['share']:.2f} | {len(t['contain'])}/"
            f"{len(t['here'] - S.res)} | {holes} |"
        )
    W.append("\n## Slot swaps\n")
    for n, ti, other, fo, F in swaps:
        W.append(
            f"- tile {n}: {ti:#x} -> {fb[F]['name']} ({F:#x}); parked {other:#x} -> {fb[fo]['name']} ({fo:#x})"
        )
    W.append(
        "\n## Prediction for installed bisection B1 (boxes +-100000, translation zeroed)\n"
    )
    order = [f for f in S.cand]
    first = order[0]

    def groups(f):
        out = set()
        for lay, g in f["pairs"]:
            if lay == "" and g in M.blink:
                out |= set(M.blink[g])
        return out

    g1 = groups(first)
    sizes = collections.Counter(len(groups(f)) for f in order)
    inR = [p for p in SAMPLES if math.dist(p, (0, 0, 0)) <= R]
    spawn = (33.0, 42.6, 150.0)
    W.append(
        f"With every box at +-100000 each point is inside every box (distance 0) and all priorities are 0, so the "
        f"best folder falls to the centre tie-break, which is the same origin for all 230; the winner is decided by "
        f"registration order, not geometry. If it is the first registered non-resident folder ({first['name']}), its "
        f"empty-layout pairs name {len(g1)} folders. Across all 229 possible winners the named-folder count "
        f"ranges {min(sizes)}..{max(sizes)}. Those blocks pass the radius gate only where the shared box centre "
        f"(local origin) lies within R = {R:g} of the point: {len(inR)} of {len(SAMPLES)} sample points "
        f"({sum(p[1] == 0 for p in inR)} at Y 0, {sum(p[1] == 20 for p in inR)} at Y 20), i.e. the disc of "
        f"radius ~{R:g} around the origin; every other sample point gets only the resident folder. The B1 spawn "
        f"{spawn} is {math.dist(spawn, (0, 0, 0)):.1f} from the origin, beyond R, so under this rule B1 creates no "
        f"non-resident block at spawn (also none with R = 112). At the older spawn (2, 0.2, 30) "
        f"(distance {math.dist((2, 0.2, 30), (0, 0, 0)):.1f}, inside R) the rule creates exactly the winner's named "
        f"folders; the capture of 10 created folders there matches winners naming 9-11 folders "
        f"({sum(v for k, v in sizes.items() if 9 <= k <= 11)} of 229 folders do). Named-folder count per possible "
        f"winner: {dict(sorted(sizes.items()))}.\n"
    )
    W.append(
        "## Renamed keys\n\n"
        + f"{len(renames)} keys, scheme `P` + 5-digit base36 counter + original key[6:], all absent "
        f"from the input and from {stats['sea_s0_low copies checked']} sea_s0_low copies. First: "
        + ", ".join(f"{a}->{b}" for a, b in list(sorted(renames.items()))[:5])
        + "\n"
    )
    W.append(
        "## Check categories\n\n"
        + "\n".join(f"- {k}: {v}" for k, v in sorted(chk["categories"].items()))
        + f"\n- unexplained changed bytes: {chk['other']}\n"
    )
    open(path, "w", encoding="utf-8", newline="\n").write("\n".join(W) + "\n")


def write_report_resident(
    path,
    src,
    out,
    M,
    tiles,
    swaps,
    stats,
    ident,
    chk,
    h,
    world_err,
    rows_in,
    rows_out,
    nren,
):
    def fv(v):
        return "(" + ", ".join(f"{x:.1f}" for x in v) + ")"

    zt = chk["zone"]
    t0 = tgt_pos(tgt_ids()[0]) if SINGLE else cell(24)
    W = [
        f"# layout authoring report (--resident{' --compact' if 'compact' in h else ''}"
        f"{' --single' if SINGLE else ''})\n\nInput `{src}`, output `{out}` ({chk['size'][1]} bytes = input "
        f"{chk['size'][0]} + {rows_out - rows_in} rows x 32; rows {rows_in} -> {rows_out}).\n",
        f"All positions in this report are layout space. The LaySettings zone translation {zt} is kept, so world = "
        f"layout + {zt}: the {'carrier' if SINGLE else 'tile 24'} at layout {t0} appears at world "
        f"{tuple(round(a + b, 4) for a, b in zip(t0, zt))}.\n",
        f"Carriers use {CARRIER_CLASS} leaves only. Keys renamed: {nren}. Rows appended: "
        f"{chk['categories'].get('rows appended', 0)}.\n",
        "## Host folder\n",
        f"- host {h['name']} at payload {h['node']:#x}; box {fv(h['lo'])} .. {fv(h['hi'])}; {h['members']} members "
        f"in the input; named in {h['links']} BlockLink lists"
        + (
            " (no folder had a harmless member: swapped with any member)"
            if h.get("any_member")
            else ""
        ),
        f"- byte +0x62: {h['old62']:#04x} -> {h['new62']:#04x} (bit 0 = resident: created at manager build, never "
        f"released, not in the streaming AABB vector)",
        (
            "- --compact: host member count {} -> {} (carrier slots first in its own array); every other folder count "
            "-> 0; arrays and slot nodes otherwise untouched; check: {} folders with reduced counts, {} counted members, "
            "{} non-carrier\n".format(
                h["compact"][0],
                h["compact"][1],
                chk["reduced"],
                chk["counted"],
                chk["nontile"],
            )
            if "compact" in h
            else "- other folders untouched\n"
        ),
        (
            "Hosts used (fill order): "
            + "; ".join(
                f"{x['node']:#x} {x['name']} {x['carriers']} carriers{' + %d kept env' % x['kept'] if x.get('kept') else ''} of {x['members']} members, +0x62 "
                f"{x['old62']:#04x}->{x['new62']:#04x}, {x['links']} BlockLink lists"
                for x in h.get("hosts", [])
            )
            + "\n"
        ),
        "Candidates considered (non-resident, enough harmless members; fewest BlockLink lists"
        + (
            ")"
            if SINGLE
            else ", off the city footprint, then farthest box centre from the city centre (-20, 20))"
        )
        + ":\n",
        "| offset | name | members | harmless members | box | BlockLink lists | centre distance |",
        "|---|---|---|---|---|---|---|",
    ]
    for f, nm, mem, harm, lo, hi, nl, dist in h["cands"]:
        W.append(
            f"| {f:#x}{' (pick)' if f == h['node'] else ''} | {nm} | {mem} | {harm} | {fv(lo)} .. {fv(hi)} | {nl} | {dist:.0f} |"
        )
    W.append("\n## lyb identity\n")
    if SINGLE or NONE:
        W.append("Not applicable (single / none mode).")
    else:
        W.append(
            "Payload byte-identical to the pristine layout."
            if ident.get("identical")
            else f"Payload NOT byte-identical to `{ident['orig']}` ({ident['len']} vs {ident['len_orig']} bytes): "
            f"differences only in {ident.get('diff_owners')}; extent moved {ident.get('extents_changed')}. Slot "
            f"offsets valid (slots_valid={ident['slots_valid']})."
        )
    W.append(
        f"\n## Carriers ({len(tiles)} homes)\n\n| tile | carrier | key | how | home | bhp | placement | BG +0x38/+0x34 before |\n|---|---|---|---|---|---|---|---|"
    )
    for n in tgt_ids():
        t = tiles[n]
        ct = chk["tiles"].get(n, dict(homes=[], bhp=[], world=(float("nan"),) * 3))
        W.append(
            f"| {tgt_name(n)} (brt {tgt_model(n):#x}) | {t['inst']:#x} | {t['key']} | {t['how']} | {', '.join(M.fname[f] for f in ct['homes'])} | "
            f"{', '.join(hex(v) for v in ct['bhp']) or 'none'} | {fv(ct['world'])} | "
            f"{'%#x / %g' % t['bg_before'] if 'bg_before' in t else '-'} |"
        )
    W.append(
        f"\nPlacement error vs target (float32): {world_err:g}. Slot swaps: {len(swaps)}.\n"
    )
    W.append(
        "## Edits\n\n" + "\n".join(f"- {k}: {v}" for k, v in sorted(stats.items()))
    )
    W.append(
        "\n## Check\n\n"
        + ("OK" if chk["ok"] else "FAILED: " + "; ".join(chk["errors"]))
        + "\n\n"
        + "\n".join(f"- {k}: {v}" for k, v in sorted(chk["categories"].items()))
        + f"\n- unexplained changed bytes: {chk['other']}\n- census fails {chk['census']['fails_out']} "
        f"(input baseline {chk['census']['fails_in']}); SEDB size match {chk['census']['sedb']}; extent disjoint "
        f"pass/fail {chk['census']['disjoint']}; zone translation {chk['zone']}; boxes equal {chk['boxes_equal']}\n"
        f"- carriers whose single home is created at their target: "
        f"{sum(t['ok'] for t in chk['tiles'].values())}/{len(chk['tiles'])}\n"
        + "".join(f"- note: {w}\n" for w in chk["warnings"])
    )
    open(path, "w", encoding="utf-8", newline="\n").write("\n".join(W) + "\n")


def run(argv, z):
    """argv as the original script took it (argv[0] = program name); z = zone settings dict."""
    configure(argv, z)
    if "--check" in ARGV:
        i = ARGV.index("--check")
        a, b = ARGV[i + 1], ARGV[i + 2]
        R = float(arg("--radius", "144"))
        r = check(open(a, "rb").read(), open(b, "rb").read(), R)
        return 0 if r["ok"] else 1
    author()
    return 0
