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
do not select that association for arbitrary supplied controls. The initial
registry option supplies the separate declared context below. Other target-key
associations and final client rendering remain qualified.

The native selector-0x2E TextModule handler queries an installed FixedSheetInterface
with the first number, then passes its returned descriptor and the second number
to the guarded TextModule query or noun path. The category byte map and descriptor
vector supply the family context; the numbers alone do not. The conditional
binding, exact dispatch cells and state requirements are recorded in
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Fixed-control
registry lookup, at revision `143ff251b5e2d8ffc464207ccb097476cf17ad73`,
against the authenticated executable identity above. Automatic classification
requires a matching registry and provider/locale context.

## Conditional initial registry lookup

`--initial-registry <client-root> --locale ja|en|de|fr` reconstructs a fresh
registry from the authenticated locale-specific DAT resources. It accepts an
explicit preserved install root, reads only named resources under `data/`, and
verifies every resource's pinned size and SHA-256 before parsing. It never uses
merged CSV field 1 as native descriptor input. This option and the explicit
CSV lookup are mutually exclusive.

The model assumes fresh Completion vectors and counter, ready backing resources,
successful allocations, descriptor parsing and target preparation, and an
uninterrupted full D550 traversal. It reconstructs category-map growth, category
object ordinals and descriptor indices separately. Empty descriptor cells do not
add categories; `@` headers select root descriptor zero; ordinary descriptors
append a table descriptor; leading `#` categories retain an invalid initial
descriptor index. Map holes and repeated codes follow C9D0D0's growth-only
assignment. The native rule and conditions are owned by
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Fixed-control
registry lookup and Conditional initial registry.

`registryLookup` reports the category source key, object ordinal, descriptor
index, original metadata descriptor, named sheet and source identities. The
root descriptor has no metadata descriptor string. Its context is
`conditional-initial-1.23b-full-load`. The pinned four locales yield 41 category
objects and 24 descriptors. For English noun descriptors, the model declares
a stable type-1 provider, reports that assumption in `providerContext`, and
selects `dispatch = query`. C9C0A0 enables noun mode only for type 2 together
with the descriptor noun flag. Other locales retain `provider-dependent-noun`.
The numeric control fields do not establish a historical provider type.

For `xtx/_fixedPhrase`, the second number selects a backing row under the
existing AT-COMP-016 key conditions. The tool requires matching field 0 minus
one and empty field 1, then reports its two locale source cells. For
`xtx/placeName`, AT-COMP-017/018 supply conditional target-key equality with
block `(0,8002)`. Its reported source columns are ja0, en1-3, de4-5 or fr6-7;
the English u8 field stays numeric. Source strings use SSD lossless CSV text,
including opaque controls. These source fields remain separate from the
selected default text below.

For `xtx/itemName`, the tool selects the authenticated block containing the
second number with `begin <= key < begin + count`, verifies its three resource
identities and matches the nonempty backing span by key. It reports all
declared locale fields, including numeric s8 values and opaque controls. It
does not filter keys through the descriptor's UI candidate ranges. The generic
ready-cache mapping makes this a conditional source-key association; it does
not establish that an item was selected or admitted in a historical client.
The native application is owned by
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Conditional
item-name source keys.

Constructed `[22,1000002]` selects category ordinal 22, descriptor 7 and
metadata key 2106, whose mode-zero descriptor names `xtx/itemName`. The source
key belongs to block `(1000000,204)` at slot 2. Its English first source cell
is `Crystal`. Keys 10009506 and 12000016 both supply English `Bat Wing` from
different blocks; neither equal text nor an enabled ordinal substitutes for
the key. Source fields remain separate from TextModule query or noun output.

Constructed controls `[8,974]` and `[20,1051]` select the fixedPhrase and
placeName tables automatically in this context. English source cells are `Map`
and empty for the first; the second supplies `Limsa Lominsa`, `Limsa Lominsa`
and u8 value 1. Code 31 and codes 41..48 report `unmapped-category`; code 49
reports `dynamic-category`. These are static checks, without captured control
provenance or observed rendering.

Beyond these families and the bounded English displayName query below,
other named sheets report
`sourceLookup.status = unsupported-key-association`
while preserving the successful table selection. Missing rows, unmapped codes,
dynamic categories and refused ordinary phrases exit 2. Resource identity and
parse failures exit 2 with a diagnostic on stderr; file-read failures exit 1.
Unknown controls do not load resources. Later dynamic additions and locale or
provider changes are outside this initial-state model.

### Conditional command-name lookup

Initial categories 24..30 and 32..35 select `xtx/command` without noun mode
in ja/en/de/fr. The second decoded number selects a backing key in one of
23 authenticated blocks per locale. All 1662 enabled rows per locale are
available to this diagnostic; descriptor UI ranges do not filter supplied
query keys. Source lookup returns every declared locale field, preserving
signed 32-bit values as numbers and controls as lossless source text.

The direct lookup verifies the pinned game master and command definition,
reported in `configurationSources`, and the selected block's three resources
in `targetSources`. Missing rows and keys outside the blocks exit 2.
The selected default name fields are ja1/en2/de8/fr16. Token-free fields
return conditional default text; supported sheet references and copied controls
use the existing evaluator below. Other expressions defer the whole result.

The native key/default association is owned by
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Conditional
command-name source keys, at revision
`2e20277889df7bfdd56281b185ce6d85388bbce2`. It applies the completed CacheAll mapping strictly
inside matching ready block ranges and the constructor's first-index/SHEET
DEF rules. Runtime state, selection production and rendering remain qualified.
Initial registry inputs below owns the exact configuration and block identities.

### Conditional status-name lookup

Initial category 23 selects `xtx/status` without noun mode in ja/en/de/fr.
All 399 enabled rows per locale are available through three authenticated
blocks. The lookup verifies the master and status definition in
`configurationSources` and the selected block's data/enable/offset resources
in `targetSources`. Missing rows and keys outside the blocks exit 2.
Source fields preserve signed 32-bit numbers and lossless control text.

The selected default name columns are ja1/en3/de8/fr15. English's explicit
SHEET DEF setting overrides first index 2 with column 3. Token-free names
return conditional default text; supported copied controls use the existing
evaluator. Other expressions defer the whole result.

The definition declares `cache=-1,type=none`. `nativeConditions` states the
assumed completed local cache state zero with matching ready CacheAll pairs.
The diagnostic reads backing files; it does not observe or change native cache
state. The key/default association is owned by
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Conditional
status-name source keys, at revision
`6a840194cf73a14f23cd636915ad3fead9ac9310`. Runtime state, selection production and rendering
remain qualified. Initial registry inputs owns resource identities.

### Conditional plain text

A matched placeName, itemName, command or status row also reports
`registryLookup.textResolution`.
The promoted constructor defaults select placeName fields ja0/en1/de4/fr6 and
itemName fields ja4/en5/de11/fr19. These are global sheet column indexes,
mapped through the authenticated locale layout to the corresponding row value.
The itemName SHEET DEF settings select those indexes; its NOUN settings are
separate. The placeName first-index defaults retain AT-COMP-021's existing
constructor-local result, including English field 1 rather than noun DEF 3.
Command and status defaults and their native associations are described above.

For a mode-zero descriptor and a selected field containing no control tokens,
the result has `status: "resolved"`, `association: "conditional-default-field"`,
the selected `column`, `text` and `textEncoding: "utf-8"`. Literal brackets and
backslashes retain their decoded values rather than CSV escaping. An empty
literal returns empty text; it does not trigger a fallback. The source fields
remain available in their SSD lossless CSV representation.

A selected field with supported literal sheet references or copied controls
uses the evaluation below. Other controls report `requires-evaluation` with
reason `embedded-controls`, its column and no text. Controls in other fields do
not prevent a plain result. Provider-dependent noun mode is deferred with reason
`provider-dependent-noun`. A missing or non-string default field reports
`unsupported-default-field`. These diagnostic statuses do not change the
existing source-lookup exit codes or claim retail fallback behavior.

The association applies AT-COMP-020/021 to definitionLocales in
`xivl-client-data:manifests/place_name_backing_rows.json` at revision
`750eb59885752e1a4f6e69c00e87697fce6a2580` and
`xivl-client-data:manifests/item_name_backing_rows.json` at revision
`06f6687b4e0adc24e7f38f3105ae9fc237de3eda`. The native application is owned by
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Conditional
default fields. Matching pinned definition/index/parameter inputs, successful
construction and unchanged selected-field state are additional conditions.
The CLI uses promoted defaults; it does not inspect a retained native sheet.
This resolves the conditional default text source, without claiming
all TextModule query/conversion paths, arbitrary expression or noun evaluation,
Completion admission, output wrappers or client rendering.

### Literal sheet references

For a mode-zero default field, the CLI evaluates selector `0x28` only when its
payload contains exactly a token-free string, integer row key and integer global
column index. The supported target is `xtx/command`. A column in `[0,255)` must
be declared as `str` in the selected locale layout. Selector 255 uses the
promoted command default: ja1/en2/de8/fr16. Other targets, numeric or missing
columns, parameters, negative selectors and additional arguments are deferred.

Each requested command block is checked by size and SHA-256 before the existing
bounded sheet parser verifies offsets, enable keys and row spans. The requested
key must be an enabled, nonempty slot strictly within the block. String fields
are evaluated recursively under the same restricted rule. Literal text is
appended in order, preserving brackets, backslashes and empty strings.

A complete token-free result has `association: "conditional-sheet-reference"`,
UTF-8 `text` and `references`. An evaluated field retaining supported controls
uses the control-stream result below. Each reference records sheet, row key,
requested and selected columns, block begin/count, resource identities and the
original lossless source cell. The outer source fields are unchanged. Any unsupported
control or reference prevents text output for the whole selected field; partial
reference evidence may remain. Reasons are `embedded-controls`,
`unsupported-sheet-expression`, `unsupported-sheet-target`,
`unsupported-sheet-column`, `sheet-row-not-found` or `sheet-expansion-limit`.
The diagnostic permits at most 16 nested references, 128 reference occurrences
and 1 MiB of expanded output bytes, including copied controls. These are tool
budgets. Source-lookup exit codes stay unchanged; resource identity, parse and
IO failures retain their existing explicit failures.

The native argument and recursive string association is promoted from
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Literal sheet
references, at revision `ea118f61c6d4f2a581ea4a47cdda701b15a388e4`.
It requires the pinned executable, retained dispatch/provider,
ready admitted rows and matching named resources. It does not establish all
expression, numeric conversion or noun paths, Completion admission, output
wrappers or rendering.

For the constructed control `[22,13000001]`, English itemName field 5 contains
a literal reference to `xtx/command`, row 26679, field 2. With the pinned inputs
it returns `Pummel`. This is a verified static lookup, not a captured selection.

### Copied control streams

The conditional evaluator preserves these direct-length forms byte for byte:

| Control | Selector | Envelope |
|---|---|---|
| Newline | `0x10` | `02 10 01 03` |
| Soft-hyphen | `0x16` | `02 16 01 03` |
| Italic | `0x1A` | `02 1A 02 01 03` or `02 1A 02 02 03` |
| Non-breaking-space | `0x1D` | `02 1D 01 03` |
| Hyphen | `0x1F` | `02 1F 01 03` |

It applies the constructor-local null callback rule in
`xivl-client-structs:docs/autotranslate-selection-contract.md`, AT-COMP-020's
selected-field and finite callback findings at revision
`bbb04aca4d57e6369fa025d099bd9008ae474f54`. Those slots remain null after
construction, and CB2760 copies the valid control envelope. The native contract
pins the complete constructor and separate CB25A0 helper spans against the
authenticated executable. Retained callback state is a condition;
the CLI does not inspect a live callback vector.

If the whole selected field can be evaluated and copied, the result reports
`status: "resolved-control-stream"`, `association: "conditional-parser-output"`,
the selected `column`, `outputHex`, `outputEncoding: "rich-string-control-bytes"`,
`preservedControls` and `references`. `outputHex` contains literal UTF-8 bytes
and exact copied control envelopes in output order. Literal sheet references
are expanded before producing this stream, including controls in a selected
command cell. There is no plain `text` field or partial byte output. The
source cells remain unchanged in their lossless representation.

Other control types, payloads and length forms remain deferred.
The existing output-byte budget includes each copied envelope. Unknown nested
controls or unsupported references defer the whole field with no `outputHex`.
This is a conditional parser-output diagnostic. It does not assign a display
character to a control or establish output wrappers or client rendering.

The constructed control `[20,1502]` selects English placeName field 1.
Its output contains `The Astalicia`, unchanged bytes `02 10 01 03`, then
`(Marauders' Guild)`. It returns `resolved-control-stream` without plain text.

### Automatic English displayName query

With `--initial-registry` and `--locale en`, category 21 selects the
displayName descriptor under the declared initial registry context. The
stable English provider has type 1, so the fixed-control caller selects
mode-zero query despite the descriptor's noun flag. Default selector 0xFF
selects column 1: the pinned definition starts with that English index and
has no SHEET override. No noun input words or attributive rows are required.
The query preserves the singular field without applying noun flags or
count replacement.

The bounded target is block `(1000000,601)`, with 418 enabled rows. The CLI
authenticates the master, displayName definition and three target resources.
It returns all five fields with `sourceLookup.association` equal to
`conditional-ready-map-key`, and token-free column 1 as conditional default
text. Missing enabled rows and keys outside this block exit 2. Other
displayName locales remain outside this source lookup.

The native owners are English displayName fixed-control query and English
displayName map admission in
`xivl-client-structs:docs/autotranslate-selection-contract.md` at revision
`2f72fbe09c78c6692b46fb9e7a677fd72964eb5d`. The source key is conditional
on a matching native kind-1 local map backend, populated
exact-key association and ready row. Provider/locale state must remain
unchanged. The tool returns a conditional result from authenticated inputs;
it does not observe runtime cache state, wrappers or rendering.

### Conditional English noun identity

`--noun-inputs <config>:<count>:<attributive-key>:<retained-word>` supplies
explicit metadata for a fresh type-1 NounEn evaluation. It requires
`--initial-registry` and `--locale en`. The initial registry must select the
noun descriptor for `xtx/displayName`. The second decoded number supplies
the target key, and `nounContext.sourceWords` reports the complete five-word
input. The tool does not infer these words or the provider type from a control.

The bounded case is `0:1:5:0`, target block `(1000000,601)`, configuration
`NOUN|SGL;1/PLR;2/VOW;3/CNT;4/DEF;5`, and target fields VOW=0, CNT=0, DEF=1.
Both target and attributive key 5 must exist in the authenticated source
blocks. Under successful English construction and admission of those named
rows, the nonzero DEF branch copies SGL column 1. With no `[n]` placeholder,
the later count replacement leaves that string unchanged. Token-free output
returns `textResolution.status = resolved` and association
`conditional-explicit-NounEn`. All five target fields remain in `sourceLookup`.

The native rule is owned by
`xivl-client-structs:docs/autotranslate-selection-contract.md`, Conditional
English noun identity, at revision `96583a1a35426f19adab1b903879353eb72496ee`.
NounEn construction, valid retained configuration,
initialized needle and ready rows remain explicit conditions. The static
target key uses the conditional map relation described above; attributive
named-row admission remains assumed. The declared displayName
`cache=100,type=map` is distinct from CacheAll, so the completed CacheAll
inverse is not applied. The automatic English fixed-control path uses
a sheet query and supplies no NounEn metadata; this option evaluates a
separate explicit source context. Runtime row-cache admission is not observed.

Other metadata words, row flags, controls or count placeholders return
`requires-evaluation` without a `text` value. An unavailable source row, an
unsupported target block or an inapplicable descriptor exits 2. These safe
diagnostic refusals do not claim native handling of invalid configuration.
Without this option, the English descriptor uses the default-field query.

`apps/cli/src/english_noun_inputs.json` retains only block coordinates and nine
resource identities. The pinned master names displayName definition
`0x0B450000` (16987 bytes, SHA-256
`7C9B77F9E5989C1C85BBD8152162160CC423F1A6CA17B730F51B5DDC5E4FF866`).
Its English columns 1..5 have types `str,str,s8,s8,s8`. The selected block's
418 enabled rows and all 2090 fields match authenticated `xtx_displayName.csv`
(838262 bytes, SHA-256
`36C5DFBA312695D9F4EE090DBA123DFD7792FEBA55D88B0062C070D98ABC0505`),
identified by `xivl-client-data:manifests/manifest.json`, its table entry at
revision `06f6687b4e0adc24e7f38f3105ae9fc237de3eda`.
The English attributive block and definition identities come from
`xivl-client-data:manifests/attributive_backing_rows.json`, en and
sourceDefinition. All 245 English fields were compared to its authenticated
CSV. The explicit context verifies the master, both definitions and each
block's data/enable/offset resources before using their pinned layouts. The
automatic query uses only the master, target definition and target block.

### Initial registry inputs

`apps/cli/src/initial_registry_inputs.rs` owns the immutable resource IDs,
sizes and SHA-256 pins. They are promoted from
`xivl-client-data:manifests/fixed_phrase_backing_rows.json`, locales/sources,
and `xivl-client-data:manifests/place_name_backing_rows.json`,
definitionLocales and locales/blocks/sources, at revision
`750eb59885752e1a4f6e69c00e87697fce6a2580`. The former owns block `(100,2926)`
and four-field locale rows; the latter owns block `(0,8002)` and each locale's
source column types. The executable identity above pins the native registry
algorithm. No retail row text or resource payload is embedded in the tool.

`apps/cli/src/item_name_inputs.json` retains only the four locale layouts,
block begin/count pairs and data/enable/offset identities from
`xivl-client-data:manifests/item_name_backing_rows.json`, definitionLocales
and locales/blocks/sources. The report has 386556 bytes and SHA-256
`D13CD40E4BBF4AC7BADECB262A185E3432F85AE787C48B978AC8854D897B5835`.
Each supported locale has 50 disjoint blocks;
offset counts match their declared counts. The shipped chs definition is
documented by the source report and is outside the CLI's locale choices.

`apps/cli/src/command_inputs.json` owns only four command locale layouts,
selected defaults and 23 disjoint blocks per locale, with 276 resource pins.
`COMMAND_CONFIGURATION` in `initial_registry_inputs.rs` pins the master and
definition used to authenticate direct command lookup.
The source is the preserved 1.23b install's game master `0x01030000`
(7044 bytes, SHA-256
`860023878F9AD1E98FD1BAA3E3380F971F86BF1F7747043CF0C3C7459CEE2EB4`),
which names `xtx/command` definition `0x0B450017` (12993 bytes, SHA-256
`99762442C38144D8AE82C83B24265C4B1AFE3050AAB769DC7AE2CADDEB12B3E5`).
The definition's client/all locale indexes, types, SHEET DEF parameters and
explicit data/enable/offset IDs supply this metadata. The same constructor
rules select its defaults. Each supported locale has 1662 enabled rows; all
41550 declared cells were compared to the authenticated merged
`xtx_command.csv` (987740 bytes, SHA-256
`2AA0B20822DF0B2270918D9AC5525C86A34A941D7C66D17A106ADD1B2D6567D3`),
identified by `xivl-client-data:manifests/manifest.json`, its table entry at
revision `06f6687b4e0adc24e7f38f3105ae9fc237de3eda`.
No decoded row text is embedded. chs remains outside the CLI's locale choices.

`apps/cli/src/status_inputs.json` owns four status locale layouts, explicit
defaults and three blocks per locale, with 36 resource pins.
`STATUS_CONFIGURATION` pins the same master and status definition `0x0B450073`
(4056 bytes, SHA-256
`2F8768776C61E7F62627E43245AD0E255584C50DBB83F5F1D6C8AB7166995D56`).
The definition's client/none sheets supply the indexes, types, SHEET DEF
settings and block identities. Each supported locale has 399 enabled rows.
All 10773 declared backing cells matched the authenticated merged
`xtx_status.csv` (331062 bytes, SHA-256
`60ECA3B76D1D9799589D9059ADEB1E7C3D142E07609A1AD0A053C5E75D9B96EE`),
identified by the same manifest revision above. Forty-eight column-24 cells
matched one of the merged CSV's retained locale variants; the CLI reports
the actual selected locale's backing value. Default name columns have no
such duplicate annotations. No row text is embedded; chs is outside the
CLI's locale choices.

## Verification

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

Initial-registry tests cover sparse backing keys, row-span disagreement,
out-of-block enable ranges, distinct map/ordinal/descriptor coordinates,
growth-only map assignment, noun flags and dynamic descriptor refusal.
Item lookup checks exclusive block bounds, gaps, locale layouts and sparse
keys across multiple blocks; private CLI checks compare selected cells in
every supported block with the authenticated extraction.
Plain-text tests check column mapping, literal escaping, empty fields,
control deferral, noun deferral and refusal of an unavailable string default.
Sheet-reference tests cover exact literal arguments, explicit/default column
mapping, unsupported targets and numeric columns, missing keys, nested controls,
whole-field refusal, cycles, reference counts and expansion budgets.
Control-stream tests check exact copied envelopes, both italic payloads,
literal escaping, output order through nested sheet references, refusal of
unsupported controls and forms, and exact output-byte budget boundaries.
English noun tests check explicit metadata parsing and locale requirements,
singular identity, literal escaping, empty output, required attributive presence,
other inputs and flags, count placeholders and whole-field control deferral.
English query tests check that a noun flag does not select noun mode and that
column 1 preserves literal count placeholders regardless of noun row flags.
Command source tests check signed 32-bit fields and default-name selection.
Private manual checks compare source cells with the authenticated extraction
and exercise identity rejection; they are not required public CI inputs.
