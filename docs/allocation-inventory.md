# Allocation inventory

`xivl allocation-inventory --client-root <root> [--overlay <dir>]... [--output <new-file>]` reports
occupied registration namespaces from an explicitly supplied client tree.
It chooses no allocation or candidate. The command writes JSON to stdout when
`--output` is absent. With `--output`, it creates only that new file and leaves
stdout empty. The parent directory must already exist. Existing destinations,
including symbolic links, and destinations inside the resolved client root are
refused. Resolving the parent prevents an existing directory alias from bypassing
the client-root restriction. No directory or file is written under the client root.

The command walks `data/` using the catalog scanner, which refuses symbolic
links, and also refuses a symbolic link at `data` itself. It then reads
`data/03/C0/00/00.DAT` with the existing 256 MiB capped reader and the bounded
`xivl_formats::region::parse` reader. Other DAT payloads are not opened.
Keep the input tree unchanged during the walk and read: the report is a
filesystem inventory, not an atomic snapshot or client observation.

## JSON contract

[The schema](../schemas/allocation-inventory.schema.json) defines version 1.
[The authored example](allocation-inventory.example.json) describes an empty
synthetic catalog with only its own DAT present. CLI tests validate the example
and generated reports against the schema.

| Field | Meaning |
|---|---|
| `schemaVersion` | Report contract version. |
| `catalog` | Fixed numeric key/path, input size and SHA-256, root/row counts, raw size word, `sizeMatchesInput`, and count of unconsumed `trailingBytes`. No trailing payload bytes are emitted. |
| `pathInventory` | Case rule, unique occupied key count, sorted keys, and separately counted/sorted unmapped DAT paths. |
| `roots` | Roots in file order with zero-based `rowIndex` and `rootIndex`, key, separate `nonzero` and `representable16Bit` flags, raw `word04`, `word08`, `word20`, effective name, `tokenTerminated`, child count, auxiliary status and children. Zero is representable but not nonzero. |
| `roots[].children` | Children in file order with global `rowIndex`, local `childIndex`, key, type, resource key, kind, effective name, eligibility, numeric path and existence. Types zero and one are ineligible but remain in the inventory and auxiliary calculation. |
| `roots[].auxiliary` | Status `derived`, `nonzero-layout-argument`, `no-children` or `low-index-overflow`. Only `derived` carries a key, numeric path and existence boolean. Other statuses carry nulls in those fields. |
| `collisions` | Duplicate root keys, duplicate child keys, duplicate effective names across all roots and children, derived keys colliding with resource keys, and derived keys shared by roots. |
| `blockers` | Stable ID, statement and revision-pinned source citation for each evidence boundary below. |

Keys use `0x` plus eight uppercase hexadecimal digits. Raw words, type and
kind use unsigned decimal JSON numbers. Row references in collision groups
are zero-based indices across the complete counted row sequence, including
roots and ineligible children. Duplicate groups contain every matching row
once, rather than expanding pairs. `auxiliaryResourceKeys` includes derived
keys matching any scanned numeric DAT key, an existing uppercase numeric path,
or any child resource key or explicit nonzero root `word20` request even if
its file is absent. Each entry records the deriving `rootRows`, registered
`childRows`, requesting `explicitRootRows` and `numericPathExists` separately.
Root `word08` is retained as a raw word, not reclassified as a child resource.

Effective names use the first 15 token bytes, ending at the first NUL.
Printable ASCII is emitted literally except backslash, which becomes `\\`.
Other bytes become `\xHH` with uppercase hex digits. JSON then applies its
normal string escaping. This representation preserves byte distinctions and
can contain the user's local names. Such reports stay local rather than
becoming committed fixtures. `tokenTerminated` checks the full 16-byte raw
root token independently of the truncated effective name.

Only paths of exactly `data/XX/XX/XX/XX.DAT` map to numeric keys: each `XX`
is exactly two ASCII hex digits. Hex digits and the DAT extension are
case-insensitive for inventory mapping. The supplied directory is lowercase
`data`. Nonconforming DAT paths are listed, while non-DAT files are outside
this inventory. Numeric paths emitted for existence checks use uppercase hex
and `.DAT`, matching the numeric formatter. `numericPathExists` queries that
spelling on the host filesystem. A lowercase file can therefore occupy a key
while its uppercase numeric path is absent on a case-sensitive filesystem.

Objects use the CLI's canonical JSON field order. Key groups and occupied
keys sort numerically, name groups sort by escaped effective name, unmapped
paths sort lexically, and row references retain file order. Aggregation visits
each path and row a bounded number of times with hash-based grouping, followed
by the sorting needed for deterministic output. It does not compare every pair
of rows or choose a free key.

Usage, filesystem, output and capped-read failures exit 1. Malformed catalogs
exit 2 with the parser's typed error kind and input offset. Size-word mismatch,
trailing bytes, duplicate keys/names and auxiliary overflow are reported rather
than rejected. A completed inventory, including one with collisions or blockers,
exits 0. A failed output write removes the file this command created; it
never reuses or overwrites an existing path.

## Overlay roots

Repeat `--overlay <dir>` in the order the loader applies DAT overlays. Each
overlay root must contain a `data/` subtree mirroring client-relative paths.
For each relative path, the first overlay containing that path wins; otherwise
the client root supplies it. The catalog and `numericPathExists` use this rule.
Numeric keys and unmapped DAT paths are the union of scans of the client and
every overlay, including files hidden by a higher-priority overlay. Scans refuse
symbolic links and reparse points.

Roots must be directories, must not be symbolic links or reparse points, and
must not equal, contain or be inside any other input root. Output must be
outside all input roots. Keep input trees unchanged during the run.

The required `overlays` array records each zero-based index, `datFileCount`
and distinct `numericKeyCount`. `catalog.source` is `"client"` or
`{"overlay": index}`. Neither records absolute paths. Without overlays these
fields are `[]` and `"client"`; the other report fields retain their behavior.

Zone packages also refuse an overlay containing any pinned client input read
by project preparation: the layout, every declared direct resource, or the
collision template. Those identities describe install bytes, so shadowing one
would make the loaded scene differ. See [zone packages](zone-package.md#overlay-roots).

## Evidence and limits

The selector, size and formatter facts below were read at
`xivl-decomp` revision `cb3639a9c03ad79724b52b710337a166d52de8ea`.
The integration and zone/script findings were read at `xivl-client-structs`
revision `fff480ab502bfae6b3e555d62679b635479821de`.

- `xivl-decomp:docs/resource/map-layout-selector.md` "Root and child construction"
  establishes catalog key `0x03C00000`, row field uses and the 15-byte name
  copy. The root's additional NUL scan is why termination is reported separately.
- The same page, "Lookup widths and bounds", establishes full-dword root
  comparison and the normal scene path's nonzero 16-bit region transport.
  `representable16Bit` is not a claim about a native ID-indexed array.
- The same page, "Normal construction and name resolution", establishes
  eligibility, inclusion of all child types in auxiliary derivation, and
  lookup fallback across regions. Any duplicate effective name is reported.
  The auxiliary calculation takes the first child's high 16 bits and one plus
  the largest child low index. Overflow is a status, never a wrapped allocation.
- `xivl-decomp:docs/resource/region-weather-resource-rows.md` "Size and counted-row boundary"
  establishes the native size equality check and counted row walk. The native
  loader leaves unconsumed suffix bytes unread. Matching size alone does not
  prove row safety or downstream acceptance.
- `xivl-decomp:docs/resource/resource-path-producer.md` "Formatting contract"
  establishes uppercase MSB-first numeric byte groups, conditional on numeric
  mode. The case-insensitive inventory mapping is this tool's reporting rule.
- `xivl-decomp:docs/resource/region-auxiliary-resource.md` "Request and event identities"
  establishes that both explicit root arguments and derived keys reach the
  resource request. Nonzero root `word20` requests therefore also occupy
  resource references for derived-key collision reporting.

The report always records these blockers:

| ID | Evidence boundary and citation |
|---|---|
| `numeric-mode-assumed` | Numeric mode is assumed. Mode zero uses a separate group/index path table, which this command does not inventory. Numeric filenames cannot establish free identities outside numeric mode. Source: `xivl-decomp:docs/resource/resource-path-producer.md` "Catalog mode". |
| `auxiliary-payload-acceptance` | An existing file or derived key does not establish a valid authored payload or that its absence is acceptable. Source: `xivl-decomp:docs/resource/region-auxiliary-resource.md` "Selection readiness and remaining evidence". |
| `zone-script-join` | Numeric zone, ZoneMaster class and script loading remain unjoined; this command does not inventory those namespaces or establish a sufficient sheet/script set. Source: `xivl-client-structs:docs/map-layout-selector.md` "Zone-master and script dependencies". |
| `integration-zone-limit` | The zone range `0..511` is an integration identity limit, not a demonstrated native bound and not a free-identity proof. Source: `xivl-client-structs:docs/map-layout-selector.md` "Minimal candidate and allocation constraints". |

The inventory proves only the reported catalog and filesystem observations for
the supplied input. It does not prove client acceptance, rendering, collision,
walking, complete zone initialization or a safe additive package. An absent
collision is not permission to allocate: unsupported mappings, dependencies
and runtime acceptance remain separate evidence requirements.
