# Fixed-control diagnostics

`xivl decode-fixed` and `xivl_formats::richstring::fixed::decode` decode one
selector-0x2E control from explicitly supplied bytes at a byte offset. They
return the exact raw control span, selector and two unsigned numeric fields.
The [CLI guide](../cli.md#decode-one-fixed-control) gives invocations.

## Authenticated format

The source is
`xivl-client-structs:manifests/autotranslate_wire_format_study.json`,
`fixedMacroControl` AT-COMP-007 and `controlParserContract` AT-PARSE-001,
at revision `67aa4c96ab5ac6b57a4573206f1d30765937885d`.
AT-CHAT-002 establishes conditional retention of admitted controls in the
native chat message source.
The authenticated 1.23b executable SHA-256 is
`9341f2b4567440b310a4d494f5cc5599ca334ba51c8042247317ff466492f2e9`.

The successful ordinary literal-numeric producer emits:

```text
02 2E E(payload byte count) E(first number) E(second number) 03
```

FUN_00CB2210's `E` writes values below 0xCF as one byte `value + 1`.
For larger uint32 values, let b0..b3 be their least-to-most-significant bytes.
The mask has bit i set when bi is nonzero. The prefix is `0xEF + mask`,
followed by the selected nonzero bytes in descending significance, b3 to b0.
The payload length uses this same encoding through FUN_00CB6510.

The diagnostic accepts short leads 01..D7, decoding them as lead minus one,
because AT-CHAT-002's FUN_00C99AF0 admits that wider range. It accepts extended
leads F0..FE using the authenticated producer codec. Zero, D8..EF and FF
are invalid numeric leads. A selected zero byte in an extended form is
rejected as outside the producer encoding. This does not establish retail
rejection of noncanonical encodings or complete FUN_00CB2020 behavior.

## Read and error contract

Reads are bounded by the supplied slice and then by the declared payload.
The two numeric widths must exhaust that payload, and its following byte must
be 03. Returned spans use offsets relative to the full supplied input.
Bytes following a decoded control are left unread. No plain-text scan,
NUL handling, escape rules, packet parsing or recursive expressions are inferred.

Other selectors return `unsupported`, with no numeric fields. Their raw span
covers the opaque input remainder at the requested offset, not a claimed
control boundary. Their length and terminator are not interpreted.

Missing bytes return `unexpected-end-of-input`; invalid starts, numeric leads,
selected zero bytes, terminators or extra payload bytes return
`malformed-rich-string-token`. A numeric read that reaches the payload boundary
reports that boundary even if a terminator or further input exists.
The CLI reports errors as JSON with an error offset and the available raw
remainder, then exits 2. These are diagnostic tool decisions. The retained
native parser has no explicit input-length argument, so this bounded behavior
does not claim retail behavior on malformed or truncated input.

## Conditional fixedPhrase source lookup

The optional CLI lookup accepts an explicitly supplied merged
`xtx__fixedPhrase.csv` and locale. It applies only when the caller has associated
the control with the ordinary fixedPhrase producer and admitted backing/cache
conditions below. A matching source row is a conditional data lookup, not proof
that an arbitrary control belongs to that family.

`xivl-client-structs:docs/autotranslate-selection-contract.md`, Native selected
record join and Native block slots and conditional CSV keys, at revision
`67aa4c96ab5ac6b57a4573206f1d30765937885d`, owns AT-COMP-010/011/016. For a
successfully constructed authenticated fixedPhrase locale row, a ready CacheAll
pair `(100,N)` and `blockSlot < N`, the mapped native key equals CSV row key
`100 + blockSlot`. D550 supplies row field 0 minus one as the separate category;
the ordinary packed record and fixed formatter preserve category then key.
Actual delivered resources, retained cache state and historical selection remain
qualifications supplied by that evidence, not facts recovered by this CLI.

`xivl-client-data:docs/autotranslate-name-crosswalk.md`, Records and source fields
and Backing-row identities, and `manifests/fixed_phrase_backing_rows.json`, at
revision `750eb59885752e1a4f6e69c00e87697fce6a2580`, own the named table, block
begin 100/count 2926, field-0 equality and locale columns. The authenticated CSV
is 82,373 bytes with SHA-256
`A9CE38B0B4CA33E1C03BF22BC854E054BB46DB1CEF9EDFB4E9C55C5D0516A16C`.
The lookup reports the supplied file's digest; it does not authenticate an
arbitrary supplied file against that pin.

The bounded CSV reader requires the two header rows, columns 0 through 11,
types `u32` then eleven `str` fields, equal row widths, unique decimal keys in
`[100,3026)` and unsigned decimal field 0. Quoted fields must close; embedded
quotes must be doubled, and a closing quote must precede a delimiter or EOF.
It validates the entire table before returning a match. Invalid tables report
`invalid-attribute-value` with a byte offset in the CSV, separate from offsets
in the control input.

For the requested row, field 0 must be 1 through 64, its value minus one must
equal the first number, and descriptor field 1 must be empty. This lookup accepts
only nonnegative signed seven-bit categories 0 through 63. Negative categories,
special entries and descriptor rows are outside this case. Missing rows and
refused associations expose no phrase fields. No iteration ordinal or
enable-range base substitutes for a key.

The returned source fields are Japanese columns 2-3, English 4-5, German 6-7 or
French 8-9. Values retain the SSD lossless CSV escaping and duplicate/missing
annotations unchanged. Both fields, including empty cells, are returned
separately. Selecting a display field, fallback or rich-control evaluation is
outside this diagnostic. Chinese lookup remains outside this bounded case.

The conditional pair `[8,974]`, encoded as `02 2E 05 09 F2 03 CE 03`, matches
row 974's field 0 = 9 and empty descriptor. With the pinned CSV and `en`, it
returns column 4 = `Map` and empty column 5. This is an independently checked
source-cell result for a constructed ordinary-producer example, not captured
control provenance or client rendering.

## Semantic limit and coverage

The numeric fields carry no universal category, CSV row, item or zone identity.
General phrase resolution still requires a verified association from the
particular control's two numeric fields to a family, selected locale/table and
backing lookup key. The optional fixedPhrase lookup uses explicit family and
locale context with the conditional producer association above. AT-COMP-016/018
do not select that association for arbitrary supplied controls. Item/place
resolution and final client rendering remain qualified.

Independent literal tests check AT-COMP-007's examples, every extended mask,
the short-range distinction, extended lengths, offsets, malformed fields and
truncated reads. The two `rich-string-public-fixed` conformance cases call the
production decoder and retain structural outcomes without raw bytes or decoded
values. Synthetic coverage establishes executable diagnostics, not retail
message or rendering parity.

Authored lookup tests check independent keys, all four locale field pairs,
quoted CSV cells and newlines, malformed quoting, missing keys, category
mismatches, descriptor refusal, duplicate keys and malformed rows after a
candidate match. Native CLI tests check explicit file/locale dispatch, lookup
exit statuses and opaque unknown controls. This source-cell diagnostic changes
no format support-matrix status.
