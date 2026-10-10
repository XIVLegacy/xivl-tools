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

## Semantic limit and coverage

The numeric fields carry no universal category, CSV row, item or zone identity.
Phrase resolution still requires a verified association from the particular
control's two numeric fields to a family, selected locale/table and backing
lookup key. AT-COMP-016/018 establish conditional fixedPhrase/placeName key
relationships only with their admitted resource, row and selection prerequisites.
They do not select that association for arbitrary supplied controls. Final
client rendering remains qualified.

Independent literal tests check AT-COMP-007's examples, every extended mask,
the short-range distinction, extended lengths, offsets, malformed fields and
truncated reads. The two `rich-string-public-fixed` conformance cases call the
production decoder and retain structural outcomes without raw bytes or decoded
values. Synthetic coverage establishes executable diagnostics, not retail
message or rendering parity.
