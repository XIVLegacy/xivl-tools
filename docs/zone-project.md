# Experimental offline zone projects

`validate-zone-project` and `build-zone-project` prepare a reproducible
model/PHB/layout prototype from explicit, identity-checked inputs. They do not
register a zone, install files, allocate IDs, or establish client acceptance.
The output remains client-unverified. These experimental commands do not
change the [support matrix](support-matrix.md).

## Synthetic reproduction

Install Rust 1.88 or newer and run PowerShell from the repository root.
The [public project](../tests/fixtures/public/zone-project/project.json),
OBJ, collision JSON and synthetic client files are committed under
`tests/fixtures/public/zone-project/`. No retail installation, fixture
generation, Python, or setup script is needed.

Choose an existing writable output parent. Both named child directories
must be absent. The parent must be outside the fixture's `client` directory.

```powershell
$outputRoot = Read-Host "Existing parent directory for new synthetic outputs"
cargo build --locked -p xivl-cli -j 2
```

Run these commands with that same binary and unchanged inputs:

```powershell
target/debug/xivl validate-zone-project tests/fixtures/public/zone-project/project.json --client-root tests/fixtures/public/zone-project/client
target/debug/xivl build-zone-project tests/fixtures/public/zone-project/project.json --client-root tests/fixtures/public/zone-project/client --output "$outputRoot/zone-project-first"
target/debug/xivl build-zone-project tests/fixtures/public/zone-project/project.json --client-root tests/fixtures/public/zone-project/client --output "$outputRoot/zone-project-second"
```

Each command exits 0. Validation prints its report without creating files.
Each build creates the four files listed below and prints no success report
to stdout. The two builds produce byte-identical files, including the report.
The public CLI test runs these documented arguments against the committed
fixtures and compares every output byte.

The fixture repeats a four-vertex, two-triangle tile at four retained
placements. Its checks report `placementCount: 4`, `flattenedVertexCount: 16`,
`triangleCount: 8` and `worldBounds: [[-32,0,-32],[32,0,32]]`.
The output model ID is `0xFFFF0001` and collision ID is `0xFFFF0002`.
These are synthetic build identities, not reserved client IDs.

## Profile and limits

`floor-blockout-v1` accepts one shared model/collision pair at exactly four
existing placements. It refuses other parsed placements and collision boxes.
Placement identities and composed transforms are assertions, not edits.

The model template must satisfy the existing importer: one RES resource with
one WRB, one MDL, one MESH and one COMP, one direct AABB branch with one
leaf under MDL, and one HEAD and one AABB leaf under MESH. The
[WRB model page](formats/wrb-model.md) owns that layout. Reading a model
successfully does not prove it is a writable template. The importer retains
materials, shaders and opaque chunks while replacing supported geometry and
bounds. It accepts UTF-8 OBJ triangular `v`/`f` geometry. It ignores `l`, `o`
and `s` records and rejects other record types. Face position indices may be
positive or negative; slash suffixes do not supply normals or UVs.

Collision uses the [single-leaf PHB profile](formats/phb-collision.md#input-contract)
and its strict JSON geometry contract. Its retained allocation accepts at
most ten vertices and four triangles. Vertices are local, Y-up finite `f32`
positions. Triangle order, winding and explicit raw surfaces are retained.
Empty, out-of-range or zero-area geometry fails. Render and collision must
have equal vertex counts per placement and equal world bounds. Their ordered
triangle coordinates must have exact finite `f32` bit equality after the
world transform. Matching only counts or bounds is insufficient. There is
no tolerance or automatic winding repair.

The layout editor writes only the selected rows' four-byte little-endian
resource IDs at row start plus `0x14`. It derives these positions from parsed
rows. It reparses the output and checks that every other byte, including
scene and environment bytes, remains unchanged. The build also checks model
RES directory bytes and model/PHB container allocations and file lengths.
It does not serialize a general layout or write RegionResourceData.

| Input | Maximum bytes |
|---|---:|
| Project JSON, before parsing | 1048576 (1 MiB) |
| Layout | 67108864 (64 MiB) |
| Each direct resource, including the model template | 268435456 (256 MiB) |
| Separate PHB template | 67108864 (64 MiB) |
| Render OBJ | 67108864 (64 MiB) |
| Collision JSON | 16777216 (16 MiB) |
| Sum of declared file references, excluding project JSON | 536870912 (512 MiB) |

Each file reference must declare at least one byte. Repeated references count
again toward the total. The model importer also caps parsed OBJ vertices and
triangles at 1000000 each, written counts at 65535 each, and model output at
256 MiB. Retained template allocations and the PHB profile impose tighter
limits on a project build.

## Saved project

[The schema](../schemas/zone-project.schema.json) describes the JSON shape.
All fields below are required. The runtime also rejects duplicate and unknown
members, arrays in object slots, malformed identities and non-integer values
in integer slots. Schema validation alone does not establish these checks or
template compatibility.

| Field | Meaning |
|---|---|
| `schemaVersion` | Unsigned JSON integer `1`. Other versions fail. |
| `profile` | Exact string `floor-blockout-v1`. |
| `layout` | File reference for the source MapLayout under `--client-root`. |
| `resources` | One declaration for every nonzero source resource-table row, including rows with no parsed placement. Zero-ID marker rows remain in the layout but have no declaration or opened file. |
| `resources[].row` | Unique zero-based source table row, as a `u32` integer. Declaration order is immaterial. |
| `resources[].id` | Nonzero source ID, matching that parsed row. |
| `resources[].tag` | Exact parsed row tag, at most four printable ASCII bytes. |
| `resources[].file` | File reference under `--client-root`. The selected model row supplies the render template. The old collision row is also required and identity-checked. |
| `collisionTemplate` | Separate supported PHB template file reference under `--client-root`. It is never inferred from the old collision binding. |
| `geometry` | Object containing the two authored geometry file references below. |
| `geometry.renderObj` | OBJ file reference relative to the saved project's parent directory. |
| `geometry.collisionJson` | Strict collision JSON file reference relative to the saved project's parent directory. |
| `binding` | Object selecting the shared source rows and explicit output IDs below. |
| `binding.resourceKey` | Shared parsed row name, from 1 through 16 printable ASCII bytes. Exactly one `brt` row and one `bhp` row must match. |
| `binding.modelRow` | Zero-based `u32` row selector for that `brt` row. |
| `binding.collisionRow` | Distinct zero-based `u32` row selector for that `bhp` row. Source IDs and tags must agree with `resources` and the layout. |
| `binding.outputModelId` | Explicit nonzero output model ID. |
| `binding.outputCollisionId` | Explicit nonzero output collision ID, distinct from the output model ID. |
| `expectedPlacements` | Exactly four assertions. Order is immaterial, but each must match one source placement. |
| `expectedPlacements[].sourceNode` | `u32` source node reference, relative to the embedded lyb payload. |
| `expectedPlacements[].sourceBlock` | `u32` start of the source node's declared node-type extent, relative to the embedded lyb payload. |
| `expectedPlacements[].worldMatrix` | Four rows of four finite `f32` values, matching the composed world matrix. Translation occupies the last column. Repeated source references are valid when the complete placement identities match. |

Each file reference has these fields:

| Field | Meaning |
|---|---|
| `path` | Relative slash-separated path under its selected root. |
| `size` | Exact byte length as a positive unsigned JSON integer, within the role's limit. |
| `sha256` | Exact SHA-256, encoded as 64 lowercase hexadecimal digits. |

IDs use `0x` followed by exactly eight uppercase hexadecimal digits. Output
IDs must be distinct and absent from unmodified source rows. This checks only
consistency inside the offline build. It does not prove that an ID or client
path is unused, reserve either, or persist an allocation elsewhere.

Paths contain 1 through 512 printable ASCII bytes. They cannot be absolute,
empty, or contain empty, `.` or `..` components. Backslashes, colons,
`<`, `>`, double quotes, `|`, `?` and `*` are refused. Components cannot end
in a dot or space. Windows device names, including names with extensions,
are refused case-insensitively: `CON`, `PRN`, `AUX`, `NUL`, `COM1` through
`COM9` and `LPT1` through `LPT9`.

Selected roots must be directories. Input components beneath them cannot be
symbolic links or reparse points, and final inputs must be regular files that
remain within their selected root. A root itself cannot be a link or reparse
point. Sizes and hashes are checked against the exact bounded bytes used by
preparation. There is no install discovery, sibling lookup, newest-file
selection, or fallback to an earlier output.

## Commands and publication

The command forms are:

```text
xivl validate-zone-project <project.json> --client-root <root>
xivl build-zone-project <project.json> --client-root <root> --output <new-directory>
```

Both use the same in-memory preparation, authoring, reparse and comparison
path. Validation writes canonical JSON to stdout only. Build prepares first,
then writes into a new directory outside the client input tree. Its parent
must already exist. Existing destinations, even empty directories or source
aliases, are refused. Inputs remain unchanged.

| Output | Contents |
|---|---|
| `layout.dat` | Source layout with the two selected binding fields replaced. |
| `model.dat` | Authored render model in its retained RES allocation. |
| `collision.dat` | Authored collision in the separate PHB template's allocation. |
| `build-report.json` | Canonical JSON report with status `offline-built`. |

These are local artifact names, not installation paths. Retained direct
dependencies are inventoried, not copied into the output directory. The tool
does not install or launch any output.

| Exit code | Meaning |
|---|---|
| `0` | Validation or publication succeeded. |
| `1` | Usage or I/O failure, including a missing input, a missing or non-directory client root, an output inside the client input tree, an existing destination or an output write failure. |
| `2` | Invalid project, path, size limit, identity or inventory, or a format/profile refusal. A client root or project directory that is a link or reparse point is an invalid path. |

Diagnostics go to stderr and identify the input role and failure kind.
Underlying format errors retain their kind and byte offset. Invalid JSON also
reports its parser location. Failure does not publish a success report.
Output files use exclusive creation and are written in the order
`layout.dat`, `model.dat`, `collision.dat`. The report is then written as
`build-report.json.pending` and renamed only after all resource writes finish.
A write failure can leave partial files or a pending report in the new
directory. Publication is not atomic across files. The tool does not delete
or repair that directory. Correct the cause and retry with a new destination.

## Report contract

Validation reports `status: "offline-validated"`. A published build reports
`status: "offline-built"`. Both retain `clientVerification: "unverified"`,
which means client-unverified; there is no client acceptance result.
The report hashes the three resources, not itself.

| Field | Contents |
|---|---|
| `schemaVersion`, `profile`, `toolVersion` | Report version `1`, selected profile and CLI package version. |
| `status`, `clientVerification` | Offline outcome and the separate client verification boundary above. |
| `inputs` | `layout`, `collisionTemplate`, `renderObj` and `collisionJson`, each with relative `path`, `size` and `sha256`. |
| `resources` | All nonzero direct rows sorted by row. Each has `row`, source `id`, `tag`, an `input` file identity and `retained`. The latter is false for the two replaced rows and true for other inventoried rows. |
| `outputs` | `layout`, `model` and `collision` with output `path`, `size` and `sha256`. Model and collision also have their output `id`. Validation reports the same prepared bytes without publishing them. |
| `binding.resourceKey` | Selected shared key. |
| `binding.selectedRows` | Row-sorted selections with `row`, full source row `span`, four-byte `changedSpan`, `sourceId` and `outputId`. Spans use file-relative byte `offset` and `length`. |
| `coverage.directTable` | `status: "complete"` and the inventoried `nonzeroRows` count. |
| `coverage.sceneDependencies`, `coverage.transitiveDependencies` | Both `"unresolved"`. Hashing direct dependencies does not establish scene or transitive closure. |
| `checks.placementCount`, `checks.flattenedVertexCount`, `checks.triangleCount` | Reparsed, flattened geometry counts. |
| `checks.worldBounds` | World-space minimum and maximum coordinate triples. |
| `checks.orderedTriangles` | `"exact-finite-equality"` after placement, winding and coordinate comparison. |
| `checks.placements` | Complete placement identities, sorted by the canonical JSON text of each identity, with `sourceNode`, `sourceBlock` and `worldMatrix`. |
| `checks.modelAllocation`, `checks.collisionAllocation` | `status: "retained"`, `fileSize`, container `declaredSize`, `resolvedSpan`, `directoryStart` and ordered `entries` with `span` and `kind`. |
| `checks.layoutOutsideBindingFields` | `"byte-identical"`. |

Reports contain no absolute paths, timestamps, durations or random IDs.
Declaration-derived arrays are sorted as described above. Use the same
executable and unchanged pinned inputs for repeatability. Between validation
and build, only `status` changes in the report.

## Explicit private inputs

This optional local procedure requires your own saved project, authored
geometry, supported templates and complete direct-row inventory. Keep private
inputs, the saved project and generated artifacts outside tracked trees.
Supply the client root explicitly and treat it as read-only. This procedure
is not a claim of public retail replay or client acceptance.

1. Save a schema-1 project using the fields above. Pin every input's size and
   SHA-256. Record the four source placements and matrices from your selected
   layout. Declare the supported PHB template separately from the old binding.
2. Build the CLI with the synthetic reproduction's build command. In PowerShell
   at the repository root, supply these paths. Choose an existing output parent
   outside the client root with neither named child directory present.

```powershell
$project = Read-Host "Saved local project JSON"
$clientRoot = Read-Host "Explicit client input root"
$outputRoot = Read-Host "Existing parent directory for new private outputs"
target/debug/xivl validate-zone-project "$project" --client-root "$clientRoot"
target/debug/xivl build-zone-project "$project" --client-root "$clientRoot" --output "$outputRoot/zone-project-first"
target/debug/xivl build-zone-project "$project" --client-root "$clientRoot" --output "$outputRoot/zone-project-second"
```

3. Require exit 0, compare every byte of all four output files between builds,
   and compare the three resource sizes and hashes with your independently
   retained expected identities. Recheck input identities after both builds.
   A mismatch needs investigation of the selected writer or binding. Changing
   expected hashes does not establish parity.
4. Keep client loading, zone registration and movement acceptance as separate
   work. Offline success proves the checks in the report only.
