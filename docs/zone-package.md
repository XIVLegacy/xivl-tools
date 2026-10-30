# Additive offline zone packages

`validate-zone-package` and `build-zone-package` chain the existing
[zone-project](zone-project.md), [allocation inventory](allocation-inventory.md)
and [region writer](formats/region.md#writing) in process. The
`additive-floor-v1` profile produces an extended registration catalog and the
same layout, model and collision bytes as the floor project. The package is
incomplete and client-unverified. Installation and launching are out of scope.
These experimental commands do not change the [support matrix](support-matrix.md).

## Package file

[The package schema](../schemas/zone-package.schema.json) defines version 1.
[The authored example](zone-package.example.json) shows the structure with a
reference to a saved floor project. Replace its identity with the exact size
and SHA-256 of your own project before use.

```json
{
  "schemaVersion": 1,
  "profile": "additive-floor-v1",
  "project": {
    "path": "floor/project.json",
    "size": 1,
    "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "layoutKey": "0xF1000000",
  "region": {
    "like": "0x0000006F",
    "likeChild": null,
    "rootKey": "0x0000F000",
    "rootName": "new-root",
    "childKey": "0xF0000001",
    "childName": "new-child",
    "childType": null
  }
}
```

Unknown and duplicate members, sequence-shaped objects and missing members
are refused, including the nullable `likeChild` and `childType` members.
Keys require `0x` and eight uppercase hexadecimal digits. `likeChild` is a
zero-based local child index or null to select the first child with type
other than 0 or 1. `childType` is a u32 or null to retain the selected type.
The root, child, name and template constraints are those of the region writer.

`project` is an identity-checked file reference relative to the package
file's directory, limited to 1 MiB. Its paths and input root protections follow
the zone-project contract unchanged. Geometry paths are relative to that saved
project's directory; client resource paths remain relative to `--client-root`.
Package files are also limited to 1 MiB. Keep all inputs unchanged during a run.

The layout key becomes the new child's resource key. Model and collision keys
come from the saved project's `binding.outputModelId` and `outputCollisionId`.
The auxiliary key is derived by `region::auxiliary_key` on the appended catalog.
The layout's low 16 bits cannot be `0xFFFF`.

All four allocations must be nonzero and pairwise distinct. Each is checked
against scanned numeric DAT keys, existing uppercase numeric paths, catalog
child resource keys, root +0x08 words, explicit root +0x20 requests and derived
auxiliary keys. Refusals identify the colliding role and key. The extra +0x08
check is a package allocation rule; it does not reinterpret that raw word in
the inventory report. Numeric formatter mode remains an assumption, as
documented by the allocation inventory's evidence and blockers.

## Overlay roots

Both commands accept repeatable `--overlay <dir>` options. Supply overlay roots
in the order the loader applies DAT overlays. A root contains a `data/` subtree
mirroring client-relative paths. The first overlay containing a relative path
wins; otherwise the client root supplies it. The catalog to extend uses this
rule. Occupancy includes numeric keys and unmapped DAT paths scanned from the
client and every overlay, including hidden files. Scans refuse symbolic links
and reparse points.

Overlay roots must exist, be directories with `data/`, and must not be symbolic
links or reparse points. No input root may equal, contain or be inside another.
The output directory must be outside the client root and every overlay.

Floor preparation reads its pinned inputs from the client root. An overlay
containing the layout, any declared direct resource (including retained and
unplaced resources), or the collision template causes exit 2 naming the path
and zero-based overlay index. These identities were pinned against install
bytes; a shadowed input would make the loaded scene differ. Project-local OBJ
and collision JSON inputs keep their existing project-relative rules.

## Commands and output

```text
xivl validate-zone-package <package.json> --client-root <root> [--overlay <dir>]...
xivl build-zone-package <package.json> --client-root <root> [--overlay <dir>]... --output <new-directory>
```

Both commands prepare and verify every resource in memory before writing.
Validation prints the canonical manifest with status `offline-validated`
without creating files. Build success is silent on stdout and writes:

| Output | Contents |
|---|---|
| `data/03/C0/00/00.DAT` | Catalog with one appended root and child. |
| Layout, model and collision numeric paths | The floor project's exact prepared resource bytes. |
| `package-manifest.json` | Canonical manifest with status `offline-built`. |

In numeric mode, `0xAABBCCDD` maps to `data/AA/BB/CC/DD.DAT`. These output
paths mirror client paths but are not installed. Retained direct dependencies
are recorded and are not copied. No auxiliary payload is emitted.

The destination must be absent, its parent must exist, and its resolved
parent must be outside the client root. Existing directories and aliases are
refused. Outputs use exclusive file creation. Publication writes catalog,
layout, model and collision in that order, reads them back for exact byte
comparison, then writes `package-manifest.json.pending` and renames it last.
Catalog verification preserves the old prefix outside +0x20..+0x28 and checks
the appended rows, names, type, counts, size word and derived auxiliary key.
All resource outputs are reparsed during preparation.

Publication is not atomic across files. Failure can leave partial files or a
pending manifest. The command does not remove or repair that directory. Correct
the cause and retry with a new destination. Exit codes follow zone projects:
0 for success, 1 for usage or I/O, and 2 for invalid input or a profile refusal.
Diagnostics identify the role and failure kind; format errors retain offsets.

## Manifest contract

[The manifest schema](../schemas/zone-package-manifest.schema.json) defines
version 1. [The synthetic manifest example](zone-package-manifest.example.json)
records a catalog supplied by an overlay. The manifest hashes the four resource
outputs, not itself. It has no absolute paths, timestamps or registration name
strings. Unchanged inputs and the same executable produce byte-identical repeat builds.

| Field | Contents |
|---|---|
| `schemaVersion`, `profile`, `toolVersion` | Version 1, `additive-floor-v1`, and the CLI package version. |
| `status`, `clientVerification`, `complete` | Offline outcome, always `unverified`, and always false. |
| `inputs.package`, `inputs.project`, `inputs.catalog` | Relative paths, exact sizes and SHA-256 identities. The package path is its basename; the project path is relative to the package. The catalog path is relative to its input root, with `source: "client"` or `source: {"overlay": index}`. |
| `inputs.overlays` | Zero-based index and `datFileCount` for each overlay, in priority order. Always present, empty without overlays. Absolute roots are omitted. |
| `inputs.zoneProject`, `inputs.directResources` | The floor report's inputs and complete direct resource rows, including retained flags. Their paths retain the saved project's root rules. |
| `registration` | Root and child keys, resolved child type, template root key and child index, name byte lengths, before/after root counts and size words, appended row offsets and the changed header span. |
| `allocations` | Layout, model, collision and auxiliary, in that order, with key, numeric path and every occupancy check performed. |
| `outputs` | Catalog, layout, model and collision, in that order, with role, package-relative path, size, SHA-256 and `new: true`. |
| `dependencies` | The direct resource rows with retained flags, plus unresolved scene and transitive coverage. |
| `blockers` | The four allocation inventory blockers verbatim, then the three package blockers below. |

Blockers record facts and do not refuse preparation. `auxiliary-payload-absent`
records the reserved key without a payload. `scene-transitive-dependencies`
records unresolved scene and transitive coverage. `client-acceptance` records
the lack of client verification. The package cannot establish a complete
dependency closure, authored auxiliary acceptance, the zone/script join or
runtime acceptance. The allocation inventory owns the evidence citations
for these boundaries.
