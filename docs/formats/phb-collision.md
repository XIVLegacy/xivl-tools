# Experimental PHB collision authoring

[Zone geometry](zone-geometry.md) | [Format evidence](../format-evidence.md)

The `xivl_formats::phb_author` library accepts local, Y-up triangular
collision geometry and an explicit native PHB template. The default profile
retains an allocation with one `PHB.TBC`, one `PHB.TBD`, one tree leaf, and no
optional GBD tail tables. The separate branching profile reallocates the
resource into binary trees. Client collision acceptance is untested and is a
separate requirement from successful serialization or `parse_phb` decoding.

The experimental [zone-project builder](../zone-project.md) uses this writer
with an explicitly declared PHB template and compares its placed triangles
with the authored render model before publishing offline output.

PHB authoring carries no support-matrix status. It has no public conformance
case, and the bounded PHB read used for collision export belongs to the
`zone-geometry` row.

The [branching writer specification](#branching-writer-specification) below
defines the implemented experimental larger profile. Its inferred partition
choices do not reproduce a recovered retail construction algorithm.

## Input contract

`author_phb(template, &CollisionGeometry)` takes finite `f32` positions and
`CollisionTriangle` records with three zero-based `u16` indices and an explicit
raw `u16` surface. Triangle order and winding are retained. Empty geometry,
out-of-range indices, and zero-area triangles are rejected. The accepted
allocation holds at most ten vertices and four triangles.

The writer changes the used vertex and triangle records, counts, surface
aggregates, triangle-reference list, leaf count, terminal count, and three
float boxes. Each box encloses the geometry with the template's per-axis TBC
margin, rounded outward where needed. Non-finite box width, center sum, or
reciprocal width is rejected. Declared bounds can therefore exceed decoded
vertex bounds. The geometry offsets, complete file length, fourth float lanes,
prefix tables, unused allocation bytes, gaps, and trailer are retained.

`author_phb_branching(template, &CollisionGeometry)` uses the same geometry
contract and retained single-leaf template input profile. It supports up to
65536 global vertices and 65536 global triangles, with at most 16384 references
per TBD and one TBD per TBC. Each triangle is referenced exactly once. The
dimension-one grid lists every TBC, with a maximum of 512 TBC IDs. The global
triangle limit bounds this writer to four TBCs. Empty optional tails are
emitted. Non-finite or unrepresentable float and index arithmetic is rejected.

Two environment variables change the branching profile. `XIVL_PHB_CAPACITY`
(1..16384, default 16384) sets the references per TBD, so a smaller value
writes more, smaller TBCs; the 512 TBC ID limit still applies.
`XIVL_PHB_GRID=N` (1..16, default 1) writes an N x N x N bucket grid over the
GBD box. Cell `x + N * (y + N * z)` lists, 0xFFFF-terminated, every TBC whose
box overlaps the cell within 0.001 world units. Outputs with either setting
remain client-unverified.

Consecutive groups retain original triangle IDs. Each group uses stable median
splits on the widest vertex-centroid axis, with X, Y, Z axis ties and original
triangle indices for equal centroids. Centroids use the arithmetic mean of the
three positions in `f64`. Leaves hold one to four references. Bounds round
outward using separate `f32` subtraction, multiplication by 32768, division by
the extent, and addition of 16384. TBC geometry margins come from the template.
An `f64` endpoint check prevents an `f32` rounding step from contracting a bound.
The SEDB prefix, GBD profile word, reserved header words, fourth float lanes,
and trailer are retained while tables, chunks, and geometry are reallocated.

`xivl_formats::phb_tree::validate_phb_tree(bytes)` independently reads the
finished resource and returns `PhbTreeReport`. It checks nested allocations,
binary topology, references, aggregates, bounds, buckets, and terminal records.
It also accepts retail grid and tail layouts, allowing wider surface ranges
and the measured 0.001 world-unit float-box excursion. Integer node coverage
uses the client query biases of 16383 and 16385 with clamping to 0..65535.5
before truncation. `PhbTreeReport::candidates(min, max)` returns conservative
triangle IDs in traversal order without surface filtering. It visits all
registered buckets, so it can return extra candidates beyond grid selection.

The explicit-path example accepts a native input, a geometry JSON input, and a
new output:

```powershell
cargo run --locked -p xivl-formats --example phb_collision_author -- <native-phb-input> <geometry-json-input> <new-output>
cargo run --locked -p xivl-formats --example phb_collision_author -- --profile branching <native-phb-input> <geometry-json-input> <new-output>
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
`--profile single-leaf|branching` selects the profile and defaults to
`single-leaf`. Branching output also passes the tree validator before writing.
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
| `+0x10` | Grid dimension `1` on 3796, `8` on 1407; the writer profile uses `1` |
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
| `0x00A7BEA0` | Converts a query box to TBD tree coordinates, with separate lower and upper biases |
| `0x00A7D5D0` | Converts query boxes to GBD bucket coordinates, using a separate base and scale |
| `0x00B0C6AF` through `0x00B0C77E` | Resolves a `u16` triangle reference, reads the surface at eight-byte triangle record `+0x06`, applies query masks, and calls the vertex consumer |
| `0x00A7CB20` | Reads three `u16` vertex indices and resolves three-float positions |

The factory identity is also recorded in
`xivl-decomp:config/ffxivgame.rtti.json`, entry `PhysicsResourceNodeFactory`.
The unsigned comparisons, node stride, reference packing, and surface-mask
consumer support the retained single-leaf profile. The tracked finding
`xivl-decomp:docs/resource/phb-branching-tree.md` owns the
[branching traversal and quantization observations](https://github.com/XIVLegacy/xivl-decomp/blob/main/docs/resource/phb-branching-tree.md),
including the distinction between tree and bucket coordinates. Static
inspection does not establish layout activation or movement-query surface masks.

## Retail branching-tree census

Evidence tier: verified from retail file bytes by
`tools/research/census_phb_tree.py`. The explicit-root scan identified all
5203 PHBs without structural or I/O anomalies. Their identity-list SHA-256 is
`a09e7b91eeccd1d780754e65bd776e7211d6fc22b4e992815b820523afdcc0f1`.
The digest input is sorted ASCII lines `0xID SIZE SHA256\n`. The script emits
counts, shapes, measurements and input identities, never a retail byte dump.

```text
python tools/research/census_phb_tree.py --client-root <install>
python tools/research/test_phb_tree.py
```

### Nested header and terminal layout

Offsets in this table are relative to the named chunk. Integer fields are
little-endian `u32` unless specified. The two TBC tables have one entry per TBD.

| Chunk and offset | Layout |
|---|---|
| TBC `+0x00` | Eight-byte `PHB.TBC\0` tag |
| TBC `+0x08` | Observed word `0x100`; semantic name unresolved |
| TBC `+0x0C` | Complete TBC byte extent |
| TBC `+0x10/+0x20` | Minimum/maximum float boxes, four floats each |
| TBC `+0x30` | TBD count |
| TBC `+0x34/+0x38` | TBC-relative offset-table and extent-table offsets |
| TBC `+0x3C` | Packed low/high `u16` surface aggregate |
| TBC `+0x40..+0x4F` | Zero throughout the census; semantic names unresolved |
| TBD `+0x00` | Eight-byte `PHB.TBD\0` tag |
| TBD `+0x08` | Complete TBD byte extent, including padding |
| TBD `+0x0C/+0x10` | Ordinary node count and TBD-relative node-array offset |
| TBD `+0x14/+0x18` | Reference count and TBD-relative reference-list offset |
| TBD `+0x1C` | Zero throughout the census; semantic name unresolved |
| TBD `+0x20/+0x30` | Minimum/maximum float boxes, four floats each |
| TBD `+0x40` | Packed low/high `u16` surface aggregate |
| TBD `+0x44..+0x4F` | Zero throughout the census; semantic names unresolved |

All 236758 TBCs contained exactly one TBD. Their offset and extent tables
begin at `+0x50` and `+0x60`; the TBD begins at `+0x70`. The client also loops
over a larger TBD count, but multiple TBDs in one TBC were not observed here.
Every TBD node array begins at `+0x50`. For `N` ordinary nodes and `R`
references, its reference list begins at `0x50 + 16*(N+1)`, and its extent is
`align16(referenceOffset + 2*R)`. The TBC's extent includes its TBD.
TBC/TBD bases, extents and node arrays are 16-byte aligned. Both float-box
fourth lanes are `1.0` throughout the census.

The extra record at ordinary index `N` is the terminal, excluded from the
header's node count. Its eight `u16` lanes are
`65535,65535,65535,N+R,0,0,0,65535`. This shape held throughout the corpus.
The client stops on the end pointer, not on the terminal's `N+R` value. That
value's use outside the surveyed walker remains unknown. The terminal is
nevertheless loaded and must remain readable and aligned.

### Topology, references and construction hypotheses

| Measurement | Verified observation |
|---|---|
| Ordinary nodes | 2055636 |
| Branches | 909439, all with two consecutive child subtrees |
| Leaves | 1146197 |
| References and triangles | 3494118 of each; each resource's references are a permutation of its triangle IDs |
| Largest TBD | 1023 nodes; maximum reference count 1451; maximum reference start 1448 |
| Maximum depth | 9 edges below the root |
| Leaf reference counts | 1: 26885 leaves; 2: 215010; 3: 579995; 4: 324307 |
| Skip layout | Preorder; leaf skip `i+1`, branch skip immediately after its subtree, root skip `N` |
| Reference spans | Contiguous in leaf traversal order, each reference-list slot used exactly once |
| Branch integer bounds | Exact union of the two child boxes on all 909439 branches |
| Child triangle counts | Equal on 411140 branches; differed by one on 498299 |

More precisely, the first child always receives `floor(R/2)` references and
the second `ceil(R/2)`. This establishes count ordering, not spatial axis
ordering. Reference IDs within a leaf were ascending on only 374611 leaves
and were not ascending on 771586. Whole-TBD lists were ascending on 91850
trees and were not ascending on 144908. The client follows the explicit
references; neither a triangle-ID sort nor a vertex-ID sort is required.

The bounds are not necessarily strict enclosing float AABBs under the nominal
inverse coordinate formula. The maximum inward excursion was
about `0.999609375` quantization units. The independently measured maximum world
distance was about `0.0115662`. Under that nominal formula every node enclosed
its referenced geometry within one quantization unit. A fixed world tolerance
such as `0.001` would incorrectly
reject some of these trees. The client query's separate lower/upper biases
must be included when assessing candidate coverage.

TBC float boxes enclosed their referenced geometry on all 236758 chunks;
the largest measured padding was `0.010009765625`. TBD float boxes enclosed
their geometry strictly on 106478 chunks. All TBD excursions were within
`0.001` world units, with a maximum of `0.000244140625`. Coordinates outside
the nominal `0x4000..0xC000` interval occurred on 260742 nodes, so that interval
is not a retail validity rule; extrema reached 10 units below `0x4000` and
13 above `0xC000`. TBD float padding reached about `2.56` world units, and
GBD padding about `0.0256`. Original padding and rounding choices are not
fully recovered. Simple truncation of the nominal formula disagreed on
77067 lower and 197202 upper coordinate components out of 6166908 each.
These measurements use the nominal coordinate formula, not a bit-exact
simulation of all client float operations or queries.

Evidence tier: inferred or unknown for the original construction algorithm.
The file stores no split-axis field. Binary topology and exact child unions
establish a bounding-volume hierarchy, but do not identify the split cost,
axis tie rules, reference permutation or the original geometry precision.
Triangle AABB-center separation on some axis matched 881408 branches and
failed on 28031. Separation on a longest geometry axis matched only 435359.
Those hypotheses do not explain the complete corpus and do not establish
that the builder used vertex centroids. Balanced triangle counts support a
median partition, but no deterministic retail reconstruction algorithm is
established.

### GBD grid and optional tails

Evidence tier: verified static consumers establish that GBD `+0x10` is the
grid dimension `n`, rather than the number of cells. Dimension eight therefore
has 512 cells. GBD `+0x14` addresses `n^3` `u32` bucket offsets; each offset is
in `u16` units into the list at GBD `+0x1C`. A list contains TBC IDs followed by
`0xFFFF`. The selected consumer uses a 64-byte visited bitset, so IDs must be
below both the TBC count and 512.

The table at GBD `+0x24` contains 16-byte TBC descriptors. Their first two
dwords are the TBC offset relative to the payload at GBD `+0x2C` and the TBC
extent. Updating only the chunk chain would leave this consumer route stale.
The `+0x14/+0x18`, `+0x1C/+0x20` and `+0x24/+0x28` pairs describe the three
prefix-table allocations, separately from the TBC payload extent.
All descriptor final two dwords were zero. Across all 5203 resources, the
grid-offset allocation was `align128(4*n^3)`, the descriptor allocation was
`align128(16*tbcCount)`, and the list allocation was the used list extent
rounded up to 128 bytes. All 724180 cells resolved to bounded terminated
lists with valid IDs, and their union covered every resource's TBC IDs.

Surface ranges enclosed their referenced triangle surfaces throughout the
corpus. GBD aggregates were exact on 5013 resources and wider on 190;
TBC and TBD aggregates were each exact on 232002 chunks and wider on 4756.
Wider aggregates are not evidence of missing triangles or invalid references.

The existing install census establishes 1002 resources with empty optional
tails and 4201 with nonempty tails, correlated exactly with GBD `+0x6C`
values zero and one. Empty tails are not restricted to one-leaf trees: the
largest no-tail resource has 15256 triangles and 9664 nodes across its trees
(`0x72E702E4`). The largest no-tail tree has 1023 nodes and depth 9
(`0x72E702F5`). In contrast, the dimension-one/no-tail subset contains one
TBC per resource, with at most 504 triangles.

| Grid dimension | Empty tails | Nonempty tails |
|---|---|---|
| 1 | 961 resources | 2835 resources |
| 8 | 41 resources | 1366 resources |

Selected private-input identities for those bounds:

- `0x72E702E4`, 451664 bytes, SHA-256
  `e020d8153cb1f9c47432eae27cd65744d86fd9e87ff05b736c08d2202d16681d`.
- `0x72E702F5`, 455632 bytes, SHA-256
  `04e800b9ee2a7f6a99b3a3c14784ba992411155c6c5f8b927558e0be8c2522fe`.
- `0x899102D2`, the dimension-one/no-tail 504-triangle example, 14800 bytes,
  SHA-256 `beaeecdea96de545e2ad1c1cb4ab362ea39cd3215c14faa29f6f0944d6ca246d`.

The [surveyed query](https://github.com/XIVLegacy/xivl-decomp/blob/main/docs/resource/phb-branching-tree.md#grid-buckets-and-triangle-references)
reads the core grid/TBC/TBD route without reading GBD `+0x6C` or the three
optional-tail offsets. Evidence tier: inferred that these tails are not a
prerequisite for a larger tree in that query. The exact generation criterion,
full tail semantics and requirements in other consumers remain unknown.

## Branching writer specification

This is the experimental branching profile's contract, not a write-support claim or a
client acceptance result. Verified byte layouts and static consumers constrain
the output. Tree partition choices below are inferences from those consumers,
not a recovered retail builder algorithm.

### Tree and reference limits

Each ordinary node is eight little-endian `u16` lanes:

| Lane | Meaning |
|---|---|
| `0..2` | Quantized minimum X, Y, Z |
| `3` | Absolute next-node index from the node-array base |
| `4..6` | Quantized maximum X, Y, Z |
| `7` | `0xFFFF` for a branch; otherwise a packed leaf span |

For a leaf word `w`, `start = w & 0x3FFF` and
`count = (w >> 14) + 1`. The span indexes a separate `u16` reference list.
Each reference indexes an eight-byte GBD triangle record. It is not a vertex
index or a byte offset. The `0xFFFF` word is reserved: a four-reference leaf
starting at `16383` would be misread as a branch. The representable start and
the total list length are distinct limits. A final four-reference leaf
starting at `16382` can end at exclusive index `16386`.

Use at most `16384` references per TBD in the experimental writer. This is a
conservative writer limit, not an observed header limit. With each triangle
referenced once, a mesh of about 21000 triangles needs at least two TBDs.
Keep the global vertex and triangle indices representable as `u16`, checking
the highest used index rather than casting a larger count. Partitioning TBDs
does not enlarge those global index fields.

### Construction and serialization

1. Validate finite Y-up positions, nonzero triangle area, explicit raw surface
   words and valid indices. Preserve winding and triangle identity. Build a
   global vertex array and triangle array, with a separate reference order.
2. Use one TBD per TBC, matching the retail census. Partition triangles into
   bounded groups inside one PHB resource. Within each group, make a
   binary bounding-volume hierarchy with one to four references per leaf.
   Split counts into `floor(R/2)` and `ceil(R/2)`, matching retail count
   ordering, and stop at four references. A stable median split on the widest
   triangle-centroid axis is a reasonable writer choice. Use X, Y, Z for axis
   ties and the original triangle index for equal centroids so repeated writes
   agree. This axis heuristic and immediate stop rule are not established as
   the retail algorithm.
3. Emit each tree in preorder. A branch is followed by its first child.
   Emit both child subtrees consecutively. Set every node's lane 3 to the
   index immediately after its subtree. A leaf therefore has `next = i + 1`,
   and the root has `next = nodeCount`. The client does not enforce binary
   arity; binary topology is the experimental writer's stricter invariant.
4. Store leaf references consecutively in traversal order. Check every packed
   start/count before narrowing, reject the branch collision, and check each
   reference against the GBD triangle count. Require every input triangle
   exactly once across the experimental profile's trees. Duplication is not needed
   for a bounding-volume hierarchy.
5. Choose a finite, positive-width float box per TBD. Pad flat axes outward
   before division. Check that the `f32` width and intermediate coordinate
   calculations remain finite. With `a` and `b` its endpoints, the nominal node coordinate
   is `16384 + 32768 * (position - a) / (b - a)`. Round lower bounds down and
   upper bounds up, checking representability and float rounding. Form branch
   boxes from the integer union of their children. Conservative outward
   bounds are a writer choice; retail bounds need not be exact float AABBs.
   Enclose each TBD in its TBC float box and all TBC boxes in the GBD box,
   retaining the template's positive TBC geometry margins. Check the GBD
   grid's `f32` widths and scale independently of the TBD arithmetic.
6. Allocate the aligned node array plus a readable 16-byte terminal record.
   The iterator performs a SIMD load at `nodes + 16 * nodeCount` even after
   traversal reaches its end. Do not place references directly over that
   record. Give the terminal lane 7 value `0xFFFF`. Keep all ordinary-node
   skip targets in `0..nodeCount`, with strict forward progress. The terminal
   skip value is not the traversal termination condition. Emit the retail
   terminal shape with lane 3 equal to `nodeCount + referenceCount`; check
   both that sum and `nodeCount` before narrowing to `u16`.
7. Use dimension one and put every TBC ID in its sole bucket list, terminated
   by `0xFFFF`. Limit the TBC count to 512. This one-cell/multiple-TBC
   combination is supported by the inspected query, but was not observed in
   the dimension-one/no-tail retail subset. It is an inferred minimal writer
   profile requiring client qualification. It avoids a spatial bucket-assignment
   heuristic and keeps the resource internally partitioned into real trees.
8. Derive every count, extent, table offset, aggregate and containing box from
   the completed structure, including the GBD bucket-offset, bucket-list and
   TBC-descriptor tables and both TBC TBD tables. Use checked arithmetic and
   re-read the finished file with a tree validator. `parse_phb` only extracts
   geometry and cannot validate these tree obligations.
9. Retain the explicit template's SEDB prefix, GBD profile word, reserved
   header words and 16-byte trailer. Rebuild the three prefix allocations,
   adjacent TBC chunks, geometry allocations and their offsets without
   overlap. For one TBD per TBC, keep its tables at `+0x50/+0x60` and TBD at
   `+0x70`, setting the TBC extent to `0x70 + tbdExtent`. Set empty-tail
   offsets to the final GBD `payloadEnd` and the file length to
   `0xC0 + payloadEnd + 0x10`. Do not carry an old descriptor extent or old
   geometry offset into a reallocated file.

The TBC contains tree metadata rather than separate vertex/triangle arrays.
For each generated TBD and TBC, compute the aggregate from its referenced
triangles as `minSurface | (maxSurface << 16)`. Compute the GBD aggregate
over all triangles. Retail aggregates need not be tight; the validator should
require enclosure rather than exact equality for retail inputs. New output
can use exact extrema without assigning material names to raw surface words.
No additional surface field occurs in an ordinary 16-byte node or leaf.

### Validator and known-answer tests

The validator must independently bound all nested slices, distinguish their
offset bases, check node alignment and the terminal allocation, and validate
each tree from its root. For the binary writer profile, the first child is
`i + 1`, the second child is the first child's skip target, and the second
child's skip target must equal the parent's. Reject cycles, backward links,
targets past the declared end, unreachable records, inverted boxes, malformed
reference spans, invalid triangle/vertex indices and non-finite boxes. Check
child containment and coverage against the source geometry with the actual
query quantizer, including its lower/upper bias and clamping.

Use these literal synthetic oracles when implementing the writer:

- Two leaves: nodes `0,1,2` have skip links `3,2,3` and words
  `0xFFFF,0x4000,0x4002`. References are `0,1,2,3`. Give the leaves disjoint X
  interiors and a shared X boundary. Left/right interior queries return
  `0,1` / `2,3`, a boundary-touching query returns all four, and a query
  outside the root returns none. Reordering references must change returned
  triangle identities without changing triangle storage.
- Deep unbalanced binary tree: alternate a branch's first leaf and its second
  branch, ending with two leaves. Check the literal skip map, complete-query
  reference order, queries that reject a whole deep subtree, and termination
  on the extra record. The walker must not depend on recursion or a balanced
  shape.
- Packing: exercise leaf counts one and four, starts `0`, `16382`, `16383`,
  the valid last spans, and the reserved `0xFFFF` collision. Keep the
  conservative writer capacity separate from the decoder's representable
  spans.
- Quantization: exact endpoints, a flat-axis padded box, a query touching a
  triangle extremum, and boxes outside both ends of the quantized domain.
  Reproduce the client's `f32` operation order: subtract the minimum,
  multiply by `32768`, divide by the extent, then add the endpoint bias.
  Every exhaustive triangle-AABB overlap must appear in the accelerated
  candidates. Extra leaf candidates are allowed.
- Mutations: backward/past-end skip, missing terminal allocation, missing
  reference, invalid global triangle/vertex index, inverted bounds, overflowing
  extent and misaligned node base must produce bounded validator errors.
- Multiple TBDs: place triangles in both groups and prove that iteration
  visits both, including a query whose only candidates are in the second.
  For the experimental one-TBD-per-TBC profile, use two TBC descriptors and the
  literal sole-bucket list `0,1,0xFFFF`. Check its offset in `u16` units,
  global triangle IDs and complete candidate coverage through both trees.

Re-reading an existing retail tree can verify its topology, references and
query candidate coverage. Reconstructing an equivalent valid tree from its
triangles is a different check. Byte-for-byte reconstruction or matching the
retail split structure requires a recovered deterministic construction rule.
Do not treat a different valid partition as a failed round trip or claim the
original builder was reproduced without that evidence.

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

The retained single-leaf and branching writer profiles exclude
dimension-eight GBD output grids and optional tail tables. Layout activation and
full-city walking acceptance remain separate requirements. The writer does
not modify a layout, installed client, or prototype package.

Remaining format questions include the exact retail split heuristic, the
generation criterion and full semantics of optional tails, the meaning of
GBD `+0x6C`, and why 190 GBD surface aggregates are wider than their triangle
minimum/maximum. Remaining client questions are PHB
binding and placement, movement surface masks and winding, and the floor,
wall, and stair behavior listed above. Retaining unknown template bytes
reduces the format scope; it does not answer those client questions.
