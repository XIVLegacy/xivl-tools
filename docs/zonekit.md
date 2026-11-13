# zonekit: experimental zone builder

`tools/zonekit` builds a new 1.23b zone from a modern-client zone: it extracts
the source scene, cuts it into a tile grid, converts the tiles to 1.23b
background models, authors a town layout around them, and builds walkable
collision and sounds. It is a Python orchestration tool. Every output is
client-unverified unless a page under `docs/formats/` says otherwise, and no
command changes the [support matrix](support-matrix.md).

zonekit is a candidate source for the Rust [zone project](zone-project.md)
commands: the stage table below names what each stage reads and writes so a
stage can move into `xivl` on its own.

## Requirements

- Python 3.12 with numpy.
- Blender 4.5 for `tiles build`, `tiles cut`, `tiles shade` and `convert tile`.
- An xiv1tt build with `model import --split-mdl`, `model export` and `info`
  for `convert tile`.
- The PHB author example of this repository for `collision phb`:
  `cargo build --locked -p xivl-formats --example phb_collision_author`.

Run every command as:

```powershell
python -I tools/zonekit/zonekit.py <group> <command> ...
```

## Settings

Machine-local settings live in `local/zonekit/settings.json` (ignored by Git)
or in the file named by `ZONEKIT_SETTINGS`. No program or folder has a default.

| Key | Environment override | Meaning |
|---|---|---|
| `blender` | `ZONEKIT_BLENDER` | Blender executable |
| `xiv1tt` | `ZONEKIT_XIV1TT` | xiv1tt executable |
| `phb_author` | `ZONEKIT_PHB_AUTHOR` | built `phb_collision_author` example |
| `zones` | `ZONEKIT_ZONES` | folder of `<name>/zone.json` zone settings |
| `read_only` | none | folders zonekit refuses to write under (client installs) |

Each zone has a settings file, `<zones>/<name>/zone.json`. The
[example zone file](zonekit-zone.example.json) lists every key. `zone show`
checks a file and prints its grid.

## Frames

| Frame | Rule |
|---|---|
| Layout | the frame of the authored layout and of every part JSON |
| In-game | layout + `frames.world_offset` |
| Collision file | layout - centre of the collision tile (`collision.tile`); for a 7 x 7 grid starting at (-200, -140) and tile 24 this is layout + `frames.collision_offset` |
| Modern source | layout + (0, `frames.source_y_shift`, 0) |
| Tile GLB | layout - centre of its grid cell, Y unchanged |

The grid has `nx` x `nz` cells of `width / nx` by `depth / nz` metres from
(`x0`, `z0`). Cells listed in `inactive` hold no tile; geometry that falls in
them goes to the nearest active cell.

## Stages

| Stage | Commands | Reads | Writes |
|---|---|---|---|
| Extract | `extract file`, `instances`, `sgb`, `props`, `model` | modern SqPack (read-only) | raw files, instances JSON, shared-group models, prop placements, GLB |
| Tiles | `tiles build`, `terrain`, `cut`, `shade` | scene FBX, terrain plates, prop placements | one GLB per tile, filled ground, pieces with tight boxes, baked shading |
| Convert | `convert tile` | tile GLB, piece manifest, donor model | 1.23b background model, `result.json` |
| Layout | `layout rows list`, `set`, `add`, `layout author` | retail layout, slot table, extras | town layout with tile carriers and resource rows |
| Collision | `collision objects`, `fix`, `patch`, `walls`, `merge`, `phb` | part JSONs, terrain plates, object `.pcb` | collision JSON, PHB |
| Audio | `audio bgm`, `se`, `info`, `preview` | modern `.scd`, stock 1.23b sound file | 1.23b sound file |

`python -I tools/zonekit/zonekit.py <group> <command> --help` documents every
option.

### Tiles

- `tiles build` cuts the scene FBX into the grid, one GLB per active tile and
  one mesh per source material. Faces whose material or object name matches
  `tiles.skip` are dropped. A `tiles.bgs` ground face is dropped when its
  centroid and three inner points all have other ground within `--cover`
  metres vertically. `--fix-ground-winding` turns down-facing ground faces
  (`tiles.ground`) up.
- `tiles terrain` adds back every terrain-plate triangle that has no matching
  tile triangle (every corner within 5 cm) and at least one uncovered sample
  point, plus every shared-group prop whose model matches `tiles.terrain_sgb`.
- `tiles cut` splits one tile into pieces whose boxes stay small, routes
  materials into donor slots with `tiles.slotmap`, and writes a manifest of
  piece boxes and triangle counts. Default options are `tiles.cut_args`.
- `tiles shade` bakes ambient occlusion into `COLOR_0` without changing
  geometry. The up to eight neighbouring tiles occlude too.

### Convert

`convert tile` imports a tile onto a donor model with one MDL per GLB node,
renames `w0d0_tub_ed1_h` to the given 14-character name, and sets MESH HEAD
bytes 4-7 from the donor's per-shader values or `--patch-value`. It then
checks MDL boxes against the manifest (2 cm), component nodes, triangle
count, round-trip area (0.1 %), and parses the output with
`tools/research/census_wrb.py`. Any failed check is listed in
`result.json` and fails the command. Convert a shaded tile again with
`--prefixed`.

### Layout

`layout rows` edits the resource row table: 32-byte rows from 0x40, row count
(including `DEF_BLK`) at 0x24, table end at 0x20. `set` accepts only existing
row offsets and `add` refuses a key that already has a row. Appending shifts
every later offset by 32 bytes per row and leaves the payload unchanged.

`layout author` turns a retail town layout into a tile layout: tile carriers
get the tile model and collision ids, every other background key is renamed
to an empty model, and the streaming structure is kept. `--check` compares an
authored file with its input. The [world layout page](formats/world-layout.md)
describes the structures it reads; `tools/research/census_layout.py` is its
parser.

### Collision

- `collision objects` places each instance's `.pcb` into layout space.
- `collision fix` drops old generated walls and lips, welds floor seams, and
  adds double-sided edge walls at real drops around the floor reachable from
  `collision.spawn_layout`. Output is in the collision tile's frame.
- `collision patch` fills floor holes from the terrain plates, then runs the
  `walls` pass, which removes generated walls with floor on both sides.
- `collision merge` appends layout-frame parts.
- `collision phb` refuses more than 65535 triangles or vertices and runs the
  PHB author with `--profile branching`. See the
  [PHB collision page](formats/phb-collision.md).

### Audio

`audio bgm` writes the Ogg Vorbis stream of a modern music or ambience file
onto a stock version-2 file. `audio se` writes one mono MS-ADPCM entry onto a
stock one-entry file.

## Output rules

- Every command writes new files only and stops when the target exists or is
  under a `read_only` folder. Folder outputs must be new or empty.
- Python stages are deterministic. Extract JSON is written with indent 0 and
  CRLF line ends so outputs compare by hash.
- `tiles build`, `tiles shade` and `convert tile` gave identical bytes on
  repeated runs. `tiles cut` does not always: its weld can flip a few vertex
  normals from run to run on the same input, while pieces, boxes, positions
  and triangle counts stay the same.

## Third-party behaviour

The modern-client readers translate behaviour from Physis (GPL-3.0,
https://github.com/redstrate/Physis): `zonekit/models.py` (model decoding),
`zonekit/extract.py` (type-3 model entry reassembly) and `zonekit/audio.py`
(version-3 `.scd` obfuscation). Each module names its source. The
[source and data policy](source-and-data-policy.md) requires owner approval
for this reuse.
