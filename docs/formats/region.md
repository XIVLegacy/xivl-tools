# RegionResourceData 1.1.0

[Documentation index](../README.md) | [Format evidence](../format-evidence.md)

The `region` reader recognizes the `RegionResourceData` table and version
`1.1.0`. It reads a bounded root/child structure, retains all rows in file
order, and supplies structural inspection and validation. Read support is
`partial`. Writing and resource export are outside this contract.

## Evidence and identity

Promoted facts come from
`xivl-decomp:docs/resource/region-weather-resource-rows.md`, especially
"Native loader boundary", at revision
`8b62e88ecada59ef069fa20fa9c21554e1957c88`.
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
`0x18`, with the observed size word at `0x20`. These byte positions and
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
| `0x20..0x24` | Observed little-endian size word, reported as metadata |
| `0x24..0x28` | Little-endian root count |
| `0x28..0x40` | Opaque header remainder |

| Row-relative span | Reader contract |
|---|---|
| `0x00..0x04` | Little-endian row ID |
| `0x04..0x08` | Opaque field |
| `0x08..0x0C` | Little-endian DAT key |
| `0x0C..0x10` on a root | Little-endian count of immediately following children |
| `0x0C..0x10` on a child | Opaque field, never a recursive count |
| `0x10..0x20` | Fixed 16-byte token span |
| `0x20..0x30` | Opaque row remainder |

The source records separate root and child constructors at `0x0079CD60`
and `0x0079A280`. Child objects are appended at parent `+0xBC`, roots at
loader `+0x08`. The child constructor receives child `+0x0C` too, but its
meaning is unresolved. The direct caller at `0x0062B27D` checks the loaded
root container and returns early when empty. This establishes a loader
handoff only.

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
The observed size word does not bound the walk or reject a mismatch: its
native enforcement rules are not established by the promoted source.

Automatic recognition and `--as region` select the same reader. For example:

```text
xivl inspect <input> --as region
xivl validate <input> --as region
```

Inspection reports row locations, root index and child membership, counts,
token spans, unknown boundaries, and digests. Repeated rows remain separate
entries with separate locations. Row ID and DAT key values are available in
the library model; inspection retains only their spans and digests. Tokens
are never decoded or printed. Header name and version are fixed recognition
metadata. Validation checks parsing and marks round-trip as not applicable.
These reports are structural views, not resource exports.

Authored public fixtures exercise duplicate children with nonzero opaque
`+0x0C`, multiple and empty roots, header field truncations, name and version
refusals, impossible root/child counts, partial rows, children consuming
remaining roots, opaque trailing bytes, and advisory size metadata. The
malformed-input sweep also mutates and truncates the region fixtures.

## Evidence limits

Unknown header and row fields, token character and terminator rules, other
table versions, native size enforcement, and the legality or meaning of
trailing bytes remain unresolved. The reader preserves their boundaries
without assigning semantics. One retained vector does not establish complete
frozen-target input coverage, so private parity does not promote read support
beyond `partial`.

Resource IDs do not establish wire zone or weather IDs, runtime weather
selection, travel routes, endpoint selection, fleet control, or rendering
behavior. No token or resource grouping is promoted into those claims.
