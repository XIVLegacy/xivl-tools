# Experimental PHB collision authoring

[Zone geometry](zone-geometry.md) | [Format evidence](../format-evidence.md)

The `xivl_formats::phb_author` library accepts local, Y-up triangular
collision geometry and an explicit native PHB template. The writer's accepted
profile is a retained allocation with one `PHB.TBC`, one `PHB.TBD`, one tree
leaf, and no optional GBD tail tables. Client collision acceptance is a
separate requirement from successful serialization or `parse_phb` decoding.

## Input contract

`author_phb(template, &CollisionGeometry)` takes finite `f32` positions and
`CollisionTriangle` records with three zero-based `u16` indices and an explicit
raw `u16` surface. Triangle order and winding are retained. Empty geometry,
out-of-range indices, and zero-area triangles are rejected. The accepted
allocation holds at most ten vertices and four triangles. This is a small
collision primitive profile; a complete city collision mesh needs additional
evidence for larger trees or another supported allocation profile.

The writer changes the used vertex and triangle records, counts, surface
aggregates, triangle-reference list, leaf count, terminal count, and three
float boxes. Each box encloses the geometry with the template's per-axis TBC
margin, rounded outward where needed. Non-finite box width, center sum, or
reciprocal width is rejected. Declared bounds can therefore exceed decoded
vertex bounds. The geometry offsets, complete file length, fourth float lanes,
prefix tables, unused allocation bytes, gaps, and trailer are retained.

The explicit-path example accepts a native input, a geometry JSON input, and a
new output:

```powershell
cargo run --locked -p xivl-formats --example phb_collision_author -- <native-phb-input> <geometry-json-input> <new-output>
```

The JSON contract is:

```json
{
  "schemaVersion": 1,
  "vertices": [[0, 0, 0], [1, 0, 0], [0, 0, 1]],
  "triangles": [{"indices": [0, 1, 2], "surface": 0}]
}
```

Keys and array dimensions are exact; duplicate and unknown object members
are rejected. Indices, surfaces, and `schemaVersion`
must be unsigned JSON integers. Coordinates are converted to `f32`, with
non-finite results rejected. No OBJ, unit conversion, automatic winding
repair, material naming, or surface default is inferred. The example limits
native input to 64 MiB and JSON to 16 MiB, refuses existing outputs and input
overwrites, and uses exclusive creation after authoring and parser checks.
Retail templates and generated DATs belong outside tracked trees.

## Retail layout evidence

The FFXIV 1.23b resource census inspected `SEDBPHB\0` files in the explicit
retail `data/` tree using little-endian field reads. All 5203 PHBs had GBD
field `+0x0C == 0x212`. Of the 64 resources with one TBC and at most four
triangles, all had one TBD node. Twenty-six had no optional GBD tail tables
and retained the allocation below. The other 38 had nonempty optional tails
and are outside this writer profile.

The representative `data/7E/B5/00/10.DAT` is 1232 bytes, SHA-256
`e62b997c8d0a0822402848ad7c2a045d31b630813cade05d32f0d127fbe1b291`.
It has four vertices and two triangles. The four-triangle representative
`data/7E/B5/00/18.DAT` has the same byte length and allocation, SHA-256
`d0b863c5fa4401a1e72a2135871a848648f7a90115e9c916d9a82876ddc60ed7`.
These are private-input identities, not distributed fixtures.

Offsets in this table are relative to the GBD chunk at file `0xC0`:

| Offset | Accepted field or range |
|---|---|
| `+0x08` | Payload end `0x400`, followed by the retained 16-byte trailer |
| `+0x10` | Observed profile word `1` |
| `+0x14/+0x18`, `+0x1C/+0x20`, `+0x24/+0x28` | Retained offset/size pairs `0x80/0x80`, `0x100/0x80`, `0x180/0x80` |
| `+0x2C/+0x30/+0x34` | TBC offset `0x200`, extent `0xF0`, count `1` |
| `+0x38/+0x3C` | Vertex offset `0x380` and count |
| `+0x40/+0x50` | Minimum/maximum bounds, three little-endian floats each |
| `+0x60/+0x64` | Triangle offset `0x300` and count |
| `+0x68` | Exact lowest/highest surfaces in the low/high halfwords for this profile |
| `+0x6C` | Zero in the accepted profile |
| `+0x70/+0x74/+0x78` | Empty tail offsets, each `0x400` |
| `+0x300` | Eight-byte triangle records: three `u16` indices and one `u16` surface |
| `+0x380` | Twelve-byte vertices: three little-endian floats |

The mixed-surface retail resource `data/89/A7/00/21.DAT` has triangle surface
words `5, 5, 4, 4` and aggregate field `0x00050004`. The census also includes
one aggregate value `0x00780078`. These establish raw storage, without
establishing material names or movement behavior for every surface value.
The mixed-surface input SHA-256 is
`8d6bdf960e825e0b0ecf354f5c6bb04b94c73cf2204ef478a78b27530c95fbd3`.

The nested TBC begins at GBD `+0x200`. Its TBD begins at TBC `+0x70`.
The accepted TBD has an 80-byte header, one 16-byte node, one 16-byte terminal
record, and a `u16` triangle-reference list at TBD `+0x70`, inside its retained
128-byte allocation. Its bounds are at TBD `+0x20/+0x30`; TBC bounds are at
TBC `+0x10/+0x20`. The fourth float lanes are retained.

TBC `+0x3C` and TBD `+0x40` carry their own packed minimum/maximum surface
aggregates. Comparing the identical geometry in `data/7E/B5/00/10.DAT` and
`data/7E/B5/00/11.DAT` isolates only these three aggregate fields and the two
triangle surface words as changed binary fields. The latter input SHA-256 is
`86af62d162333ce7cd1e2da6df7e6162e89bc5972ce0ab8eb5b8869ee61a4d10`.
The writer updates all three aggregates together.

The node's first three `u16` lanes are minimum coordinates and its three
lanes at `+0x08` are maximum coordinates. The retail cylinder
`data/7E/B5/00/08.DAT` provides non-endpoint coordinates for comparison with
its referenced triangle vertices. For a TBD float box `[min, max]`, the
observed coordinate relation is:

```text
coordinate = min + (raw_u16 - 16384) / 32768 * (max - min)
```

`0x4000` maps to the minimum, `0xC000` to the maximum, and `0x8000` to the
center. The writer gives its one leaf the full float box on every axis.
The node halfword at `+0x06` is the next node index, retained as `1`.
At `+0x0E`, the high two bits encode reference count minus one and the low
14 bits encode the reference-list start, emitted as zero. The terminal
record's `+0x06` is updated to one plus the triangle count. Its other bytes
are retained. The generated reference list enumerates each triangle once.

The cylinder input SHA-256 is
`0373b2d1673a9e8e9518f8b1336a20a93d162f441958ec96256697d5c15d5ffd`.
Synthetic tests independently decode this node relation, check candidate
coverage for every floor, wall, and stair vertex, verify exact geometry and
surfaces through `parse_phb`, and compare every byte outside the owned ranges.
The authoring example also replayed all 26 accepted retail templates with
synthetic floor geometry. No generated resource was installed or launched.

## Install census

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001),
all 5203 PHB resources under `data/`. Reproduce with:

```bash
python tools/research/census_phb.py --client-root <install>
```

This command is research only. It requires an explicit install root, makes no
support claim, never runs in CI, and prints measurements and resource identities
rather than client bytes.

All 5203 resources had SEDB `headerSize == 0x48` and the word at file `0x10`
equal to `0x48`. `PHB.GBD` began at file `0xC0`, and file size equaled
`0xC0 + payloadEnd + 0x10` on all resources. Offsets below are GBD-relative.

| Field or relation | Install observation |
|---|---|
| `+0x0C` | `0x212` on all 5203 |
| `+0x10` | `1` on 3796, `8` on 1407; meaning unresolved; the writer profile uses `1` |
| `+0x2C/+0x30/+0x34` | TBC offset, extent, and count; every chain consumed exactly its extent |
| TBC extent and count | Extents ranged from 240 to 347952 bytes; counts ranged from 1 to 372 |
| Region after TBC | Began at the TBC end rounded up to `0x80` on all 5203; exact adjacency on 670 |
| `+0x68` | Low/high halfwords enclosed every triangle surface on all 5203; exact minimum/maximum on 5013, wider on 190 |
| `+0x6C` | `0` on 1002, `1` on 4201; meaning unresolved |
| `+0x70/+0x74/+0x78` | All three equaled `payloadEnd`, giving empty tails, on exactly the 1002 resources with `+0x6C == 0`; on the 4201 others at least one differed |
| `+0x7C` | Zero on all 5203 |
| Declared minimum/maximum boxes | Fourth float lane `1.0` in both on all 5203 |

Vertex and triangle arrays were non-empty and inside `payloadEnd` on all 5203.
Every triangle index was below `vertexCount`, and the highest index equaled
`vertexCount - 1` on every resource. The declared box contained every decoded
vertex within an absolute per-axis tolerance of `0.001` on all 5203, with strict
containment on 1767. The largest excursion was `0.000244140625` on
`0x89800414`. The largest padding on an exactly flat axis was approximately
`0.0064` per side: the measured value was `0.00640000216663` on the Y axis of
`0x899103E4`.

Those two resource identities are:

- `data/89/80/04/14.DAT` (`0x89800414`), 1872 bytes, SHA-256
  `9e85127a75822eb44a651228f1779627bfca08b1e59d147b3f7626cf7d5c1d1a`.
- `data/89/91/03/E4.DAT` (`0x899103E4`), 6224 bytes, SHA-256
  `8456307e290cf00ee760e9834de300fac3eb6745d21b4e05c9f0032650f59428`.

### Raw triangle surfaces

The census counted 3494118 triangles. Surface values are stored as `u16`;
their meaning is unresolved.

| Raw surface | Triangles |
|---|---|
| 0 | 2326169 |
| 1 | 86084 |
| 2 | 496761 |
| 3 | 16600 |
| 4 | 300190 |
| 5 | 185721 |
| 6 | 68939 |
| 7 | 13145 |
| 120 | 509 |

Value `120` occurred only in `data/89/80/02/F3.DAT` (`0x898002F3`), on all
509 of its triangles, consistent with the recorded aggregate `0x00780078`.
The resource is 15440 bytes, SHA-256
`0cedf3ef92ee69849c900eeb60ea0608eac6f9578ddc14daa1c1ba8d3baed1a0`.

### Primitive family

`data/7E/B5/00` held exactly 32 PHBs, `0x7EB50000` through `0x7EB5001F`,
in four groups of eight resources each. The shape names below are labels
assigned from vertex and triangle counts. Within each group, the surface
value ran `0, 2, 1, 3, 4, 6, 5, 7` in resource order.

| Shape | Resource suffixes | Vertices | Triangles |
|---|---|---|---|
| Cube | `00..07` | 8 | 12 |
| Cylinder | `08..0F` | 16 | 28 |
| Single plane | `10..17` | 4 | 2 |
| Double-sided plane | `18..1F` | 8 | 4 |

Cube and cylinder bounds are approximately minimum `(-0.5, 0, -0.5)` and
maximum `(0.5, 1, 0.5)`. The exact decoded and declared bounds printed by
the census were identical within each of those families:

```text
cube decoded = declared
  min (-0.4999999701976776, 0.0, -0.4999999701976776)
  max (0.4999999701976776, 0.9999999403953552, 0.4999999701976776)
cylinder decoded = declared
  min (-0.49999991059303284, 0.0, -0.49999988079071045)
  max (0.49999991059303284, 0.9999999403953552, 0.49999988079071045)
single plane decoded
  min (-0.4999999701976776, 0.0, 0.0)
  max (0.4999999701976776, 0.9999999403953552, 0.0)
double-sided plane decoded
  min (-0.4999999701976776, 0.0, -6.123233601181349e-17)
  max (0.4999999701976776, 0.9999999403953552, 6.123233601181349e-17)
both plane families declared
  min (-0.4999999701976776, 0.0, -4.999999509891495e-05)
  max (0.4999999701976776, 0.9999999403953552, 4.999999509891495e-05)
```

Single planes decode at `z = 0`; double-sided planes decode approximately at
`z = 0`. Both declare approximately `z = +/-0.00005` to enclose the flat axis.
Representative identities complement the cylinder and plane inputs above:

- `data/7E/B5/00/00.DAT` (`0x7EB50000`), 1360 bytes, SHA-256
  `059b9c14aac056bad6f273f481db45df0b5d1de09842f928144bc67264c378eb`.
- `data/7E/B5/00/09.DAT` (`0x7EB50009`), 1744 bytes, SHA-256
  `554445567dcae5406be9ddc871c02de8d924f4f821bfe102d6efbda940513a7a`.

## Client consumer evidence

The FFXIV 1.23b executable used for consumer inspection has SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`
and image base `0x00400000`. Locations below are virtual addresses; subtract
the image base for RVAs. The disassembly producer was
`xivl-decomp:tools/ghidra_scripts/DumpFunctions.java`, SHA-256
`8b2ab0188a1ae563f2f7018b58095b85652377d7e40e0afa346d49a0430ed4bc`.
These are promoted binary observations, with no external implementation
copied into the writer.

| Consumer location | Observed behavior |
|---|---|
| `0x00A63D10`, registration at `0x00A63DFE` | Stores the literal registry key bytes `bhp\0` alongside the `PhysicsResourceNodeFactory` vtable |
| `0x00A7C180`, comparisons at `0x00A7C218/0x00A7C224` | Traverses 16-byte nodes using unsigned saturating halfword subtraction for box overlap |
| `0x00A7C240/0x00A7C251` | Reads lane 7 as the leaf/branch word and lane 3 as the next node index, scaled by 16 |
| `0x00A7B560` | Packs reference start into the low 14 bits and count minus one into the high two bits; emits `0xFFFF` for a branch |
| `0x00A7C550` | Iterates references from the low 14 bits and derives leaf count from the high two bits plus one |
| `0x00A7D5D0` | Converts query float boxes into clamped integer coordinates after subtracting a base and multiplying by a scale |
| `0x00B0C6AF` through `0x00B0C77E` | Resolves a `u16` triangle reference, reads the surface at eight-byte triangle record `+0x06`, applies query masks, and calls the vertex consumer |
| `0x00A7CB20` | Reads three `u16` vertex indices and resolves three-float positions |

The factory identity is also recorded in
`xivl-decomp:config/ffxivgame.rtti.json`, entry `PhysicsResourceNodeFactory`.
The unsigned comparisons, node stride, reference packing, and surface-mask
consumer support the retained single-leaf profile. The exact static setup
that establishes the on-disk `0x4000` coordinate bias was not recovered;
that relation is supported by the retail cylinder comparison above. This
trace does not establish how the registry key is normalized, layout
activation, or movement-query surface masks.

## Client acceptance requirements

Owner-operated client tests must establish that the resource loads in its
intended layout placement and that collision queries use the authored
geometry. Parser acceptance and decoded coordinates do not establish either.

1. Confirm the PHB resource binding and complete placement transform against
   the intended render blockout. The existing render importer is independent.
2. Test standing on and walking across the synthetic floor, approaching the
   wall from both sides, and ascending and descending the stair treads.
3. Test triangle edges, resource boundaries, spawn height, slope and step
   handling, and collision after reloading the placement.
4. Establish which raw surface values and winding rules the movement consumer
   accepts. Serialization retains these inputs without assigning semantics.

Larger branching trees, eight-bucket GBD profiles, optional tail tables,
layout activation, and full-city walking acceptance remain outside this
profile. The writer does not modify a layout, installed client, or prototype
package.

Remaining format questions are the static coordinate-bias setup, the
acceptance rules for retained prefix metadata, registry-key normalization,
the construction rules for branching trees and optional tails, the meanings
of GBD `+0x10` and `+0x6C`, and why 190 GBD surface aggregates are wider than
their triangle minimum/maximum. Remaining client questions are PHB
binding and placement, movement surface masks and winding, and the floor,
wall, and stair behavior listed above. Retaining unknown template bytes
reduces the format scope; it does not answer those client questions.
