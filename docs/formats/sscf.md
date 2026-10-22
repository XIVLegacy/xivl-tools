# SSCF sound containers

[Documentation index](../README.md) | [Format evidence](../format-evidence.md)

An SSCF resource is a SEDB container whose subtype is `SSCF`. The reader in
`xivl_formats::sscf` resolves the SEDB prefix, the table header, three offset
tables, and every stream header, and it reports each stream's auxiliary chunks
and codec-specific head. It does not decode audio, resolve what the table
records mean, or write containers. SSCF is not a support-matrix format. This
page records observations, and the reader and census command make no support
claim.

## Evidence and identity

Observations come from a full census of the retail 1.23b install
(`game.ver` 2012.09.19.0001) that reads every `.DAT` file below `data/`. Run:

```powershell
cargo run --locked -p xivl-cli -- sscf-census "C:\path\to\FINAL FANTASY XIV"
```

The command takes an explicit client root, walks `data/` without following
symbolic links, reads only files that begin with `SEDBSSCF`, and prints one
canonical JSON report of counts, ranges, and distributions. The report carries
no file bytes. A failure lists at most five resource ids per error kind. A
file that cannot be read, or exceeds the input limit, is counted under the
`unreadable` failure kind and the walk continues.

Two retained pages agree with these observations.
`xivl-decomp:docs/resource/music-id-resources.md`, "Retained identities",
records `SEDBSSCF`, a u32 of 3 at `+0x08`, and the file size at `+0x10` for
resources `0x07FE000D` and `0x07FE0039`. Its "Bounded metadata check" records a
Vorbis comment packet with `LoopStart` and `LoopEnd` comments inside each
file. The SEDB prefix values are tabulated in
[SEDB, RES, and resource paths](sedb-res.md#sedb-container-header).

Tier labels used below:

| Tier | Meaning |
|---|---|
| observed | Read from the files. The census counts back it, or the reader enforces it with zero failures over the install. |
| inferred | Follows from observations but nothing in the files states it. |
| unknown | A field or table whose meaning is not established. The reader carries the value and does not interpret it. |

All integers are little-endian.

## Layout

### SEDB prefix

| Offset | Field | Tier | Observed |
|---|---|---|---|
| 0x00 | `SEDB` | observed | all files |
| 0x04 | `SSCF` | observed | all files |
| 0x08 | u32 revision | observed value, inferred name | 2 in 736 files, 3 in 76958 files |
| 0x0C | u16 flags | unknown meaning | `0x0400` in all files |
| 0x0E | u16 headerSize | observed | `0x30` in all files |
| 0x10 | u32 declared size | observed | equals the file size in all files |
| 0x14..0x30 | zero | observed | nonzero in 5 files, reported as the `prefix-field-nonzero` anomaly |

### Table header

The table header starts at `headerSize` (0x30) and is 0x30 bytes long. Table
offsets are absolute file offsets. Three u16 counts govern three arrays of u32
file offsets.

| Offset | Field | Tier |
|---|---|---|
| 0x30 | u16 count of table C | observed value (1 in all files), inferred pairing |
| 0x32 | u16 count of table A | inferred pairing |
| 0x34 | u16 count of the stream table | observed pairing |
| 0x36 | u16 unknown | unknown, 920 distinct values from 0 to 9999 |
| 0x38 | u32 table A offset | inferred pairing |
| 0x3C | u32 stream table offset | observed pairing |
| 0x40 | u32 table C offset | inferred pairing |
| 0x44 | u32 reserved | observed zero |
| 0x48 | u32 block D offset | unknown target, zero in 733 files |
| 0x4C | u32 reserved | observed zero |
| 0x50 | u32 block E offset | unknown target, nonzero in all files |
| 0x54..0x60 | reserved | observed zero |

The stream table pairing is observed because every entry resolves to a stream
header whose extents end inside the file in all 84010 streams. The pairing of
the A and C counts with their offsets is unresolved. Table A starts at 0x60 in
all 77694 files, but the distance from the table A offset to the stream table
offset equals four times the table A count in only 43228 files, and the same
test for the stream table and table C offsets holds in 43212 files. When all
three counts are 1 (11714 files), the three arrays sit 16 bytes apart, which
does not separate the assignments. The reader implements one assignment and
its tests pin that order.

Table A holds 0 to 10 entries. Its count equals the stream count in most files
and differs in some. The meaning of a table A
record, a table C record, block D, and block E is unknown. The reader keeps
their offsets and checks only that each lies inside the input.

### Stream header

Each stream-table entry is the absolute offset of a 0x20-byte stream header.

| Offset | Field | Tier |
|---|---|---|
| 0x00 | u32 codec data length | observed |
| 0x04 | u32 channels | observed |
| 0x08 | u32 sample rate | observed |
| 0x0C | u32 codec id | observed |
| 0x10 | u32 loop start | observed value, unit unresolved |
| 0x14 | u32 loop end | observed value, unit unresolved |
| 0x18 | u32 extra data length | observed |
| 0x1C | u16 auxiliary chunk count | observed |
| 0x1E | u16 unknown | observed zero in all streams |

Extra data follows the header and the codec data follows the extra data, so a
stream occupies `header + 0x20 + extraLength + dataLength`. The reader refuses
a stream whose end lies past the input. All 84010 streams pass.

Auxiliary chunks sit at the start of the extra data. A chunk is a four-byte
tag, a u32 length that includes those eight bytes, and a payload the reader does
not interpret. Only the tag `MARK` occurs, 286 times. The reader refuses a
chunk that is shorter than its own header or leaves the extra data. Bytes
after the chunks are the codec head.

### Codecs

| Codec id | Streams | Tier of the name | Evidence |
|---|---|---|---|
| 12 | 83251 | `wave-format` observed, ADPCM inferred | The codec head begins with a WAVEFORMATEX-shaped record. Its format tag is 2 in 83241 streams and its channel count and sample rate match the stream header in the same 83241. The other 10 streams have zero channels and fewer than eight bytes of extra data. |
| 6 | 755 | Vorbis observed | A plain Vorbis identification header marker (`0x01` followed by `vorbis`) sits 16 bytes into the extra data in 599 streams. |
| 1 | 4 | unknown | Extra data length is 0 in all four streams. |

For codec 6, the first u16 of the extra data is 0 in 599 streams and `0x2002`
in 156. All 599 streams with 0 carry the marker at 16 and none of the 156 does.
The marker is absent from 147 of the `0x2002` streams, which suggests that
value flags an obfuscated header (inferred). The other nine contain the byte
pattern at an offset other than 16 (152 in four streams, and 156, 668, 2520,
2656, and 8908 in one each), which this census does not explain. No decoder or
key is established here.

Loop fields compared with the codec data length:

| Codec id | Both zero | End equals length | End below length | End above length | Start above end |
|---|---|---|---|---|---|
| 12 | 82707 | 0 | 279 | 265 | 0 |
| 6 | 195 | 558 | 0 | 1 | 1 |
| 1 | 1 | 0 | 3 | 0 | 0 |

For codec 6 the loop end equals the codec data length in 558 streams, so the
unit is plausibly bytes of codec data (inferred). For codec 12 the end exceeds
the length in 265 streams, so it is not a byte offset into the codec data. Its
unit is unknown. The Vorbis comment loop values recorded by the retained page
are not the header values, so the header fields are not those comments.

## Retail census

The census scanned 140179 `.DAT` files (the count is `.DAT` files only and
specific to this install, so it differs from the file count of
[SEDB, RES, and resource paths](sedb-res.md)) and found 77694 that begin with
`SEDBSSCF`, all of which the reader accepts with no failure.

| Quantity | Observed |
|---|---|
| File size | 368 to 13456222 bytes, median 400, total 3116236639 |
| Files with zero streams | 43028, all in the 256 to 511 byte size bucket |
| Streams | 84010 |
| Stream count per file | 0 to 20, with 1 (11719 files), 2, 3 (19490 files), and 10 (674 files) the common cases |
| Table C count | 1 in all files |
| Codec ids | 12 (83251 streams), 6 (755), 1 (4) |
| Channels, codec 12 | 1 (81409), 2 (1832), 0 (10) |
| Channels, codec 6 | 1 (8), 2 (220), 4 (435), 6 (86), 8 (6) |
| Channels, codec 1 | 1 (3), 2 (1) |
| Sample rate, all streams | 746 distinct values from 0 to 49085. 44100 is the most common (3364 streams) |
| Sample rate, codec 6 | 44100 (747 streams) and 48000 (8) |
| Sample rate, codec 1 | 44100 (3 streams) and 48000 (1) |
| Anomalies | `prefix-field-nonzero` in 5 files, `loop-start-after-end` in 1 stream |

Codec 12 streams carry rates such as 44115 and 44081 that differ from a
standard rate. Their channel count and rate match the codec head, so the
header is self-consistent. Whether these values are authored pitch offsets is
unknown.

The report also carries the Vorbis head pairs (`vorbisHead`), the codec 12 head
tags, the auxiliary chunk tags, and the per-codec extra and codec data length
ranges. Run the command for those distributions.

## Reader behavior

`sscf::parse` takes the whole file. It returns a typed error and an offset for:

| Error kind | Condition |
|---|---|
| `bad-magic` | Not `SEDB`, or a subtype other than `SSCF` |
| `unexpected-end-of-input` | A fixed field runs past the input |
| `header-too-small` | `headerSize` is below 0x14 |
| `header-size-out-of-range` | The table header does not fit after `headerSize` |
| `declared-size-out-of-range` | The declared size exceeds the input |
| `table-out-of-range` | The stream table, or an entry it holds, lies past the input |
| `stream-out-of-range` | A stream header, or its extra data and codec data, ends past the input |
| `invalid-aux-chunk` | A chunk is shorter than eight bytes or leaves the extra data |

It records these as anomalies and continues: `header-size-unexpected`,
`declared-size-mismatch`, `prefix-field-nonzero`,
`reserved-header-word-nonzero`, `block-offset-out-of-range`,
`unknown-table-out-of-range` (table A or C, whose records are uninterpreted),
`wave-format-mismatch`, and `loop-start-after-end`. A table with a zero count
is not read, so its offset is not checked.

## Unknown

- The meaning of the SEDB flags, the revision, the u16 at 0x36, and the u16 at
  stream-header offset 0x1E.
- The record layout and role of table A, table C, block D, and block E, and
  how their records refer to streams.
- The unit of the loop fields for codecs 12 and 1.
- The codec behind id 1, and whether codec 12 is the ADPCM variant its format
  tag names.
- The encoding behind the `0x2002` Vorbis head, and any seek table or setup
  packet layout in the extra data.
- The `MARK` chunk payload.
