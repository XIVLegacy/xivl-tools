# RegionResourceData 1.1.0

[Documentation index](../README.md) | [Format evidence](../format-evidence.md)

The `region` reader recognizes the `RegionResourceData` table and version
`1.1.0`. It reads a bounded root/child structure, retains all rows in file
order, and supplies structural inspection and validation. Read support is
`partial`. Writing supports the bounded append profile below and is `partial`.
Resource export is outside this contract.

## Evidence and identity

Promoted loader facts come from
`xivl-decomp:docs/resource/region-weather-resource-rows.md`, "Native loader
boundary" and "Size and counted-row boundary". Row words, effective names,
lookup widths and auxiliary derivation come from
`xivl-decomp:docs/resource/map-layout-selector.md`, "Root and child
construction", "Lookup widths and bounds" and "Normal construction and
name resolution". These findings were read at revision
`cb3639a9c03ad79724b52b710337a166d52de8ea`.
The source identifies retail 1.23b `ffxivgame.exe` by SHA-256
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`,
PE32 image base `0x00400000`, with mapping by pefile 2024.8.26 and x86-32
decoding by Capstone 5.0.7. Loader VA `0x0079D900` compares the table name
and version. The table-name pointer is pushed at `0x0079DA63`, with its
immediate beginning at `0x0079DA64`. Global VA `0x012C492C` contains resource
key `0x03C00000` for this table.

The source's `data/03/C0/00/00.DAT` is 52336 bytes, with SHA-256
`c04b0d998aea4c1b13ed322292a5aa5af45485c698da2315171c3c024bcb9a74`.
Its size word is `0xCC70`, root count is 61, and root/child walking consumes
1089 rows ending at EOF. The exact identity is declared as
`retail-region-03c00000` in `tests/fixtures/private-manifest.json`.

Independent fixed-stride accounting of that retained vector enumerates
1089 complete rows from `0x40`, then hops through those rows using only
root `+0x0C` counts. All 61 roots account for 1028 children, with the last
row ending at 52336. Private inspect and validate cases reproduce the
structural result against an explicitly supplied frozen external root.
No decompiled body is imported.

## Header and rows

The pinned DAT places the NUL-terminated name at `0x00` and version at
`0x18`, with the size word at `0x20`. These byte positions and
terminators are recognition rules for this bounded reader, not a claim
about every native string-comparison boundary. Header padding remains
opaque. The native loader establishes the root count at `+0x24`, first row
at `+0x40`, and row stride `0x30`.

| Header span | Reader contract |
|---|---|
| `0x00..0x13` | `RegionResourceData` and NUL |
| `0x13..0x18` | Opaque padding |
| `0x18..0x1E` | `1.1.0` and NUL |
| `0x1E..0x20` | Opaque padding |
| `0x20..0x24` | Little-endian size word; `sizeMatchesInput` reports equality with the input length |
| `0x24..0x28` | Little-endian root count |
| `0x28..0x40` | Opaque header remainder |

| Row-relative span | Reader contract |
|---|---|
| `0x00..0x04` | Little-endian row ID: root region lookup key or child key |
| `0x04..0x08` on a root | Stored dword, reported as `storedWord04` |
| `0x04..0x08` on a child | Full-dword `type` |
| `0x08..0x0C` on a root | Stored dword, retained under the report's `datKey` field |
| `0x08..0x0C` on a child | Numeric resource key, reported as `datKey` |
| `0x0C..0x10` on a root | Little-endian count of immediately following children |
| `0x0C..0x10` on a child | Stored kind dword, reported as `storedKind`, never a recursive count |
| `0x10..0x20` | Fixed 16-byte token span; first 15 bytes supply the bounded native name copy |
| `0x20..0x24` on a root | LayoutWorld construction argument, reported as `layoutWorldArgument` |
| `0x24..0x30` on a root | Opaque row remainder |
| `0x20..0x30` on a child | Opaque row remainder |

The source records separate root and child constructors at `0x0079CD60`
and `0x0079A280`. Child objects are appended at parent `+0xBC`, roots at
loader `+0x08`. Root `+0x04` and `+0x08` are stored at RegionInfo `+0xAC`
and `+0xB4`; no further meaning is assigned here. Root `+0x20` becomes
RegionInfo `+0xB8`. Child type, resource key and stored kind become
LayoutInfo `+0x5C`, `+0x60` and `+0x64`. The direct caller at `0x0062B27D` checks the loaded
root container and returns early when empty. This establishes a loader
handoff only.

Both constructors copy 15 bytes from token `+0x10` and append a NUL. The
effective name ends at the first NUL in that copy, with a maximum length
of 15 bytes. Raw byte `+0x1F` does not enter that copy. A second root-name
copy uses a NUL scan when global `0x00F67298` is `0xFFFFFFFF`, its pinned
image value. An unterminated raw root token is therefore a native safety
concern even though the bounded reader accepts it. `tokenTerminated`
records whether any of the 16 raw token bytes is NUL, without scanning
beyond the token.

Root and child resource lookups compare full dwords. Normal SetMap
transport separately packs a 16-bit region key. Normal construction skips
child types zero and one for layout submission; it does not remove them
from the auxiliary calculation. Manager name resolution can fall back to
matching child resource keys across all regions. These observations are
from the selector finding's "Lookup widths and bounds" and "Normal
construction and name resolution" sections, not allocation guarantees.

## Bounded reader and reports

`xivl_formats::region::parse` accepts a slice and returns typed errors with
absolute offsets. It checks the fixed header before reading rows and checks
counts against complete rows available in that slice before looping or
allocating. A root's children must leave enough complete rows for every
remaining root. No allocation is reserved from an unchecked declaration.

Truncated header fields produce `unexpected-end-of-input` at the start of
the requested field. A mismatched name or version produces `bad-magic` at
`0x00` or `0x18`. Root counts that cannot fit produce
`subresource-count-out-of-range` at `0x24`; child counts that cannot fit
produce the same error at root `+0x0C`. A partial declared row cannot satisfy
either count. Bytes after the counted walk remain one opaque trailing span.
The size word does not bound this reader's walk or reject a mismatch.
Header `sizeMatchesInput` compares that word with the entire input length,
including any trailing bytes; it matches the native check only when the
reported extent is that whole input. The native loader compares header `+0x20`
with the reported resource extent at `0x0079DA56`; a mismatch skips row
construction. Its normal loader control flow leaves an unconsumed suffix
unread and has no final cursor-versus-extent check. Size equality alone
does not prove that the counted rows fit. These native checks are established
by the loader finding's "Size and counted-row boundary" section.

Automatic recognition and `--as region` select the same reader. For example:

```text
xivl inspect <input> --as region
xivl validate <input> --as region
```

Inspection reports row locations, root index and child membership, counts,
token spans, unknown boundaries, and digests. Repeated rows remain separate
entries with separate locations. Row ID and `datKey` carry their source span
and digest. Root `storedWord04` and `layoutWorldArgument` and child `type`
and `storedKind` report only their span, because a digest of a small-range
4-byte word would reveal it. Numeric values stay in the library model and the
local `xivl allocation-inventory` report. The root's `datKey` label retains its raw
`+0x08` stored dword without claiming a resource-key meaning. The unknown
inventory excludes the separately reported words: roots retain only
`+0x24..+0x30`, children only `+0x20..+0x30`.
Tokens are never decoded or printed. Each row adds `effectiveName` with
`byteLength` and `sha256`, plus `tokenTerminated`. `effectiveName` describes
the bounded first native copy; `tokenTerminated` is the 16-byte precondition
for the root's second copy and is reported on child rows too, where no second
copy applies. Neither establishes runtime safety.
Header name and version are fixed recognition
metadata. Validation checks parsing and marks round-trip as not applicable.
These reports are structural views, not resource exports.

Each root adds `auxiliaryKey` from `region::auxiliary_key`. Status `derived`
means the key `(first child datKey & 0xFFFF0000) | (max child low16 + 1)` is
computable; `xivl allocation-inventory` reports its value. Every child
type contributes to that maximum, including zero and one. A nonzero root
`+0x20` reports `nonzero-layout-argument`. Zero `+0x20` with no children
reports `no-children`; any child low index `0xFFFF` reports
`low-index-overflow` rather than a derived value. These are metadata
statuses, never new parser refusals or proof that the native request occurs.

The selector finding's "Normal construction and name resolution" section
establishes the conditional auxiliary request. Numeric formatter mode makes
its predicate true without testing file existence. In numeric mode, resource
keys format as uppercase MSB-first byte groups in a DAT path; mode zero uses
catalog group/index lookup instead. These modes are established by
`xivl-decomp:docs/resource/resource-path-producer.md`, "Formatting contract"
and "Catalog mode", at the same decomp revision. This report does not infer
the active mode, resolve paths, or test file existence.

Authored public fixtures exercise duplicate children with nonzero stored
`+0x0C`, multiple and empty roots, header field truncations, name and version
refusals, impossible root/child counts, partial rows, children consuming
remaining roots, opaque trailing bytes, size mismatches, unterminated root
tokens, nonzero LayoutWorld arguments, low-index overflow, and type zero/one
children supplying the auxiliary maximum. The
malformed-input sweep also mutates and truncates the region fixtures.

## Writing

`xivl_formats::region_write::append_region_root` appends exactly one root
with one child to a copy of catalog `0x03C00000`. The input must parse,
have a size word equal to its length, and end at the counted row boundary.
Trailing bytes and size mismatches are refused. This no-tail profile is
stricter than the native loader: `xivl-decomp:docs/resource/region-weather-resource-rows.md`,
"Size and counted-row boundary", establishes size equality and count-based
iteration without a remaining-length or final cursor check.

The template root key must occur exactly once and its `+0x20` must be zero.
An explicit template child index is local and zero-based. Without one, the
writer selects the first child whose type is neither zero nor one. Missing
roots, missing eligible children and out-of-range indices are refused.
The new rows copy those template bytes, replacing only the root key,
root child count (one), root token, child key, child resource key, child
token, and child type when supplied. The resulting type must not be zero
or one. Root `+0x04/+0x08/+0x24..+0x30` and child
`+0x0C/+0x20..+0x30` remain exact template bytes.

Names must contain 1 to 15 printable ASCII bytes, excluding NUL. Each is
written with NUL fill through the 16-byte token. This authoring restriction
supplies termination for the root's second copy and prevents truncation in
the first copy. The native copies, row construction and conditional auxiliary
request are established by `xivl-decomp:docs/resource/map-layout-selector.md`,
"Root and child construction" and "Normal construction and name resolution".
Printable ASCII is a tool profile, not a native character rule.

The writer refuses these catalog collisions:

- A root key that is zero, above `0xFFFF`, or already a root key.
- A child key already used by any existing root or child.
- Either new effective name already present on any root or child, or the
  two new names matching each other.
- A zero child resource, low16 `0xFFFF`, or a child resource matching an
  existing child resource, raw root `+0x08` word or derived auxiliary key.
- A new derived auxiliary key matching any of those occupied values or
  the new child resource.

The root key bound follows the normal 16-bit scene transport profile even
though native root lookup compares full dwords. The derived key for one
child is `(child resource & 0xFFFF0000) | (child low16 + 1)`. Existing
auxiliary statuses that cannot derive a key supply no collision value.
These rules preserve raw root `+0x08` words without assigning them a
resource meaning. They do not claim a complete allocation policy.

The two rows are appended after the last existing row. Only header
`+0x20..+0x28` changes in the original prefix: size becomes the output
length and root count increases by one. Sizes above u32 are refused.
Appending to the pinned 52336-byte no-tail catalog produces 52432 bytes,
root count 62 and size word `0xCCD0`. A first-hand census of that pinned
catalog found no child sharing its own root's effective name and 59 of
61 roots with zero `+0x20`; these are retail observations, not native rules.

Synthetic Rust tests check exact bytes, template selection, name boundaries
and refusals. The conformance runner has no comparable append operation.
The [CLI command](../cli.md#append-a-region-root) re-reads and verifies its
new output and removes it on write or verification failure. Filesystem path
occupancy belongs to [allocation-inventory](../allocation-inventory.md).
Client acceptance, the auxiliary payload and zone initialization are not
established by these checks.

## Evidence limits

Unknown header and row fields, the further meaning of root stored dwords and
child kinds, token character rules, other table versions, and downstream
acceptance of trailing bytes remain unresolved. The native loader's size
equality and unread suffix do not establish downstream validity. The reader
preserves opaque boundaries without assigning semantics. One retained vector does not establish complete
frozen-target input coverage, so private parity does not promote read support
beyond `partial`.

`xivl-decomp:docs/resource/region-auxiliary-resource.md`, "Completion and
decoder boundary", "Catalogued consumer profiles" and "Selection readiness
and remaining evidence", at the decomp revision above, establishes generic
decoding and forwarding of the auxiliary resource, not a supported authored
payload or that failure is harmless. The report derives a key without
allocating it or claiming a successful request, open, decode or activation.

`xivl-client-structs:docs/map-layout-selector.md`, "Minimal candidate and
allocation constraints" and "Zone-master and script dependencies", at
revision `fff480ab502bfae6b3e555d62679b635479821de`, requires inventory of
occupied keys, effective names and path mappings before allocation. Its
ZoneMaster/script boundary remains separate from layout selection. These
reports do not establish a complete authored table profile or initialized
custom scene.

Resource IDs do not establish wire zone or weather IDs, runtime weather
selection, travel routes, endpoint selection, fleet control, or rendering
behavior. No token or resource grouping is promoted into those claims.
