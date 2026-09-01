# The SSD document stack

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b), observed 2026-08-01 over all 140180 resource files under
`data/`. Reproduce with:

```bash
python tools/research/census_sheet_stack.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints counts rather than
bytes or text.

Nothing in this section is promoted from another repository. Every
statement below was established first-hand against the install named
above.

## The documents

Seven resources in the install begin with an XML document, and all seven
open with a UTF-8 byte order mark. The sweep does not require the mark, so
a document without a mark would have been counted. None exists.

Every one of the seven is a `<ssd version="0.1">` root holding `<sheet>`
elements. Which of the two documents a file is follows from its sheets, not
from its root:

- a **master** sheet is a reference. It is self-closing and carries only
  `name` and `infofile`, where `infofile` is a **decimal** resource id: the
  master at 0x27950000 names `infofile="664076291"`, which is 0x27950003.
  The install has one master, naming four sheets.
- a **schema** sheet is a definition. It carries `mode`, `column_max`,
  `column_count`, `cache`, `type`, an empty `infofile`, and optionally
  `lang`, over `<type>`, `<index>`, and `<block>` children. The install has
  six schema documents holding twelve sheets: the four the master names,
  each repeated once per language (ja, en, de, fr) where the sheet is
  translated, plus two untranslated ones and two standalone test documents
  no master references.

A sheet's rows live in blocks, and the block states its own resources
rather than relying on adjacency:

```xml
<block count="4">
  <file begin="10000" count="10011" offset="664076320" enable="664076319">664076318</file>
</block>
```

- `begin` is the identifier of the first row slot and `count` the number of
  slots, which is also the length of the row-offset array;
- the element text is the data resource, `enable` the enable file, and
  `offset` the row-offset array, all decimal resource ids;
- the three are consecutive in the observed documents, but the ids are
  written out, so nothing here infers a resource from its neighbour.

The `mode`, `cache`, `type`, `column_max`, `column_count`, and `index`
values are carried verbatim and are not interpreted here.

## The SSD document subset

The XML reader in `src/formats/src/xml.rs` accepts exactly the grammar
these documents use and refuses the rest with
`unsupported-xml-construct`. Across all seven documents:

```text
elements     ssd, sheet, type, param, index, block, file      7 names
attributes   version, encoding, name, mode, column_max,
             column_count, cache, type, count, begin,
             offset, enable, infofile, lang                  14 names
absent       entity references, comments, CDATA sections,
             doctype declarations, namespaces, and
             single-quoted attribute values                   0 uses
```

Accepting more than the format uses would mean claiming behavior for
constructs this project has no evidence for, so the reader implements only
this subset and preserves the typed-offset error contract.

## The enable file and the row-offset array

An enable file is pairs of `(u32 firstRow, u32 count)`: the row identifiers
that carry data, run-length encoded. The one at 0x2795001F is 64 bytes,
eight ranges beginning `(10000, 7)` and `(11000, 39)`.

A row-offset array is `u32` entries, one per slot the block declares. Entry
`i` is the **end** offset of row `i` in the data file, so row `i` spans
`offsets[i - 1] .. offsets[i]` and row 0 starts at zero. An empty slot
repeats its predecessor, which is how a block declares 10011 slots and
stores seven rows.

Over all 37 blocks in the install:

```text
offsets file length == 4 * count                     37 of 37
last offset == data file length                      37 of 37
enable ranges == the slots whose entry has a span    37 of 37
```

533 enable ranges name 7866 rows. The two readings are independent - one
comes from the enable file, the other from the offset array - and they
agree everywhere, which is what fixes the meaning of both.

## Rows and columns

A row is its columns back to back, in `<type>` order. Five column types
appear in the install and no others:

```text
str     framed string, self-delimiting     18 columns
u8      1 byte                              3 columns
bool    1 byte                              5 columns
s32     4 bytes, little-endian              4 columns
float   4 bytes, little-endian              2 columns
```

Because a string column is self-delimiting and the rest are fixed width,
the column list alone determines where a row ends. Two independent checks
over the install:

```text
rows consuming their span exactly                7866 of 7866
blocks where a sequential decode from offset 0
reproduces the row-offset array                    37 of 37
```

That second line is what makes reading a data file on its own sound, and
it is why `xivl inspect --as sheet-data --columns <list>` needs no second
file.

## The sheet string and its obfuscation

A string value is `u16` little-endian length, then a body of that many
bytes including its terminator. If `body[0]` is 0xFF the rest is
obfuscated; otherwise the body is plain UTF-8 ending in NUL. The marker is
unambiguous because 0xFF is never a valid UTF-8 lead byte.

The cipher, and how it was derived rather than assumed:

1. Every one of the 261193 obfuscated bodies in the install ends in 0x73,
   and not one of them contains 0x73 anywhere else. A byte that appears
   exactly once per body, always last, and never interior is the image of
   the NUL terminator under a fixed byte substitution, which gives
   `0x00 -> 0x73`.
2. A fixed substitution mapping 0 to 0x73 is XOR 0x73 if it is XOR at all.
   Applying it to the corpus is the test, and the corpus answers: all
   310497 strings in the install decode as clean UTF-8 outside their
   control tokens, across the four languages the schema documents tag
   (ja, en, de, fr) and the wider corpus the sweep reaches.

The totals across the install:

```text
resources framing as a string stream            3751
  by resource-id prefix    03A2 3150, 0B45 549, 2795 31, 0103 21
string values                                 310497
  obfuscated                                  261193
  plain                                        49304
obfuscated bodies terminated by 0x73          261193
obfuscated bodies with an interior 0x73             0
strings decoding as UTF-8 outside their tokens 310497
strings failing to decode                           0
```

"Frames as a string stream" is a structural test, not a claim that a
resource is a sheet: it means the whole file tiles as length-prefixed,
correctly terminated bodies. It is used for counting and never for a
support claim. The 37 blocks the documents name are the authoritative
corpus. The sweep is the wider count.

## Rich-string control tokens

A decoded string carries control tokens framed
`0x02 <code> <length> <payload> 0x03`. The length is not a plain byte:

```text
lead < 0xF0    the length is lead - 1               222777 tokens
lead == 0xF0   the length is the next byte             166 tokens
lead == 0xF1   the length is the next byte times 256     1 token
lead == 0xF2   the length is the next two bytes,
               big-endian                              569 tokens
```

The encoding was established by requiring the frame to close: under this
reading all 223513 tokens in the install end on a 0x03, and no token in
any string fails to frame. The 0xF1 form rests on one occurrence, where it
is the only one of the candidate readings that closes; a lead byte in the
escape range that is not one of these three is an error, not a guess.

A payload may contain its own `0x02 .. 0x03` pair. This project does not
descend into one: the payload is kept whole, so the representation stays
lossless without asserting a nesting rule.

26 distinct codes appear:

```text
code 0x10 70238   0x16 28252   0x1d 21796   0x14 18439   0x08 15938
     0x13 11852   0x09 11655   0x28 10089   0x1a  8452   0x20  6512
     0x2c  5033   0x29  4095   0x1f  2647   0x2b  2506   0x33  1459
     0x31  1337   0x32  1307   0x12  1020   0x22   710   0x19    48
     0x24    44   0x2d    25   0x25    23   0x07    20   0x2f    12
     0x11     4
```

The codes use the same SeString macro vocabulary retained by the modern
Lumina reader. The comparison was made against Lumina commit
`02934e4a077de6118fca5f8e0d7baad7048596bc`; its `MacroCode.cs` hashed to
`78156B8963555937DD183125110CCFDFC661B54A1D9A65ACC12EBB6B944A2883`.
All 26 retail codes have one name:

```text
07 set-time             08 if               09 switch
10 newline              11 wait             12 icon
13 color                14 edge-color       16 soft-hyphen
19 bold                 1A italic           1D non-breaking-space
1F hyphen               20 number           22 kilo
24 seconds              25 time             28 sheet
29 string               2B head             2C split
2D head-all             2F lower            31 english-noun
32 german-noun          33 french-noun
```

Payload expressions use a recursive prefix grammar: compact integers,
parameter references, comparison operators, and framed strings. An
independent pass parsed every payload of all 223513 retail tokens with zero
unconsumed bytes and zero failures. Names and parsed expressions supplement
the raw token bytes; they never replace them.

Losslessness is a checked property, not a design intention. Rebuilding each
string from its text runs and tokens reproduces the input exactly for all
310497 strings in the install, and the same round trip is asserted in
`src/formats/tests/malformed_inputs.rs` on every truncation and byte
mutation of every committed fixture.

## Parser behavior over the install

The counts above come from the research command. The parser in
`src/formats` was run over the same corpus separately, through the command
line, to check that the two agree:

```text
xivl inspect <file>                       7 of 7 xml documents parsed
xivl inspect <file> --as sheet-data    3751 of 3751 string streams parsed
```

0 errors and 0 panics in both passes.

## Negative result: this cipher is not the scrambled XML scheme

`client/sqwt/` holds 1524 files, of which 1155 begin with `SQEX`: 620
`.form`, 482 `.tpl`, 20 `.skin`, 20 `.style`, 12 `.sml`, and one `.xml`. Past the
first 16 bytes they carry 7.87 bits of Shannon entropy per byte at the
median, against 7.96 at the maximum, so the body is either compressed or
encrypted rather than substituted. A fixed byte substitution cannot
produce that from text, so whatever `SQEX` wraps, it is not the sheet
string cipher. The remaining 369 files are 207 `GTEX`, 160 `SPKL`, and two
plain CSS documents with a byte order mark.

## Why two formats have no private conformance case

`ssd-master` and the schema half of `ssd-sheet` have public cases and no
private one, deliberately. A faithful structural report of a schema
document is the document: its content is its sheet names, attribute
values, and resource ids, so a committed expected output for a retail one
would republish a client file. `docs/source-and-data-policy.md` forbids
that, and no amount of ignored pointers makes it a case worth having.

Retail parity for those two is established instead by the research
command, which reads all seven documents on the owner's machine and prints
counts. That is evidence, not a conformance case, and the matrix says so:
`ssd-master` stops at `supported`.

The binary half of the stack has no such problem. Spans, counts, digests, and
token codes are structure, not content, so the enable file, the row-offset
array, the string stream, and the typed rows all carry private
cases whose expected outputs hold no decoded text and no column values.

## The scrambled XML container

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b), observed 2026-08-01 over all 140180 resource files under
`data/`. Reproduce with:

```bash
python tools/research/census_sheet_stack.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints counts rather than
bytes or text.

### What the container is

801 resources in the install end in the byte 0xF1 without being SEDB
containers or plaintext documents. 799 of them decode to a well-formed
UTF-8 XML document with an `<ssd>` root. The other two decode to nothing
that opens on a byte order mark, and they are refused rather than read.
That is why recognition here is the whole decode and not the trailer byte:
a one-byte signature would have claimed two resources this project cannot
read.

The layout, in decode order:

```text
0x00              the encoded body, encodedLength bytes
encodedLength     u8   0xF1, the trailer; not part of the document
```

`encodedLength` is the file length minus one, and it is the only length the
format states. The decoded document is exactly `encodedLength` bytes.

### The decode

Two reversible steps.

1. **A partial byte reversal.** The byte at 0 trades places with the byte
   at `encodedLength - 1`, the byte at 2 with the byte at
   `encodedLength - 3`, and so on, each side stepping inward by two, until
   the two positions meet or cross. Every byte the two walks skip stays
   where it is. The step is its own inverse, so an encoder and a decoder
   run the same walk.

2. **A two-key word cipher.** Each four-byte group is two little-endian
   16-bit words. The word at group offset 0 is exclusive-ored with `keyA`
   and the word at group offset 2 with `keyB`. Both passes stop before the
   final byte of the body, so a body whose length is 1 modulo 4 ends on a
   byte neither pass covered; that byte alone is exclusive-ored with the
   low halves of both keys.

```text
keyA  =  (encodedLength * 7) & 0xFFFF
keyB  =  the little-endian word at offset 6 of the unscrambled body,
         exclusive-ored with 0x6C6D
```

`keyA` is derived from the length. `keyB` is not stored anywhere: it is
recovered from known plaintext, because bytes 6 and 7 of every document are
`ml`, from `<?xml` behind the byte order mark, and `0x6C6D` is that pair as
a little-endian word.

### How it was derived

The reading was established from the retail corpus in two stages. The first
stage isolated the parity structure and length-derived key. The second tested
the partial-reversal interpretation directly against every matching resource.

Established first-hand, before any external material was read:

1. 799 resources end in 0xF1 and nothing else in the install shares that
   ending outside two false positives, which is what identified the family.
2. Assuming the plaintext `EF BB BF <?xml version="1.0" encoding="utf-8"?>`
   at the head of a candidate, the recovered keystream is **constant at the
   odd byte positions** of every one of the 799, taking one value at
   positions 1 modulo 4 and another at positions 3 modulo 4. Under that key
   alone the odd half of every document reads as every other character of a
   coherent `<ssd>` document, tail included. That is what identified the
   4-byte grouping and gave the first readable content.
3. The value at positions 1 modulo 4 equals `((encodedLength * 7) >> 8)`
   for **799 of 799** documents, which fixed the length-derived key and the
   fact that the trailer is not part of the encoded body.
4. The even byte positions were not a keystream at all. Ruled out by test
   rather than by hunch: a fixed byte substitution, because a bijection
   cannot turn a fifty-symbol alphabet into the 136 distinct byte values one
   1664-byte document carries; a repeating exclusive-or key of every period
   from 1 to 48; a whole-file bit rotation; a byte transposition, because
   the exclusive-or of two ASCII bytes cannot set the high bit and 17 of 20
   successive stream differences do; a memoryless dependence on the
   plaintext at any fixed distance from -60 to 60; and mirrors of both the
   plaintext and the ciphertext. What did hold: over 4000 document pairs,
   the first even-position divergence sits 39 plus or minus 2 bytes before
   the first plaintext difference, so the even positions carry content from
   about forty bytes further on.

The partial-reversal interpretation was tested directly against retail data.
It moves the tail of the document into the even positions of the head, and the
last 40 bytes of these documents are a common
`</file>\r\n\t\t</block>\r\n\t</sheet>\r\n</ssd>\r\n` tail, which is why the
divergence begins where it does.

The first candidate applied the final-byte correction whenever the encoded
length was odd. Against the install that is wrong for lengths congruent to 3
modulo 4:

```text
encodedLength mod 4   final byte needs the correction   documents
0                     no                                     65
1                     yes                                    51
2                     no                                     51
3                     no                                    632
```

Under that candidate rule 632 documents decode with a wrong final byte, and
615 of them stop being valid UTF-8. Under the rule above all 799 decode.
The correction belongs to one residue, not to odd lengths.

### Corpus statistics

```text
resources scanned                                 140180
plaintext XML documents                                7
resources ending in the trailer                      801
  decode to a byte order mark and a declaration      799
  decode as UTF-8                                    799
  parse as well-formed XML                           799
  refused as not a document                            2
document roots                                  ssd, 799
keyA == (7 * encodedLength) & 0xFFFF, checked
  independently against the mark's second byte  799 of 799
final decoded byte                              0x0a, 799
```

The parser in `src/formats` was run over the same 801 resources separately,
through the command line, to check that the two agree:

```text
xivl inspect <file> --as scrambled-xml   799 decoded, 2 refused as bad-magic
xivl inspect <file> --as ssd             799 documents parsed
```

0 panics in both passes.

### What the documents say

The 799 decoded documents plus the 7 plaintext ones are 806 documents
holding 3820 sheets and 5384 file blocks. Two of them are masters and the
rest are schemas; every one of the 140 `infofile` references resolves to a
document in that set.

```text
languages   ja 720, en 720, de 720, fr 720, chs 717, untagged 223
modes       client 3680, untagged 140
```

`chs` is a fifth language the plaintext documents never showed: the one
master reachable without the decode names four.

### The answer to the unreachable string streams

The format census left 3720 of the install's 3751 string-stream
resources named by nothing this repository read. They are named by
these documents:

```text
resources framing as a string stream               3751
  named as a block's data file                     3731
  named by nothing                                   20
    of those, named as a block's enable file         10
```

The remaining 10 sit in the same three-resource stride the blocks use and
nothing references them; they are recorded here as unnamed rather than
explained. The 10 named as enable files are the structural test's false
positives: an enable file of the right byte values also tiles as string
values, which is exactly why "frames as a string stream" is used for
counting and never for a support claim.

### The column vocabulary is wider than the plaintext documents showed

The plaintext-only reading established five column types because the
seven plaintext documents declare five. The 806 documents declare ten and no others:

```text
str   3957    s32   750    f16   279    s8    162    bool  132
s16     99    float  47     u8    34     u16   20    u32    15
```

The widths are established by the same row-width test, run over every block
whose data file is present:

```text
blocks with a readable column list                 5189
  data file tiles exactly as rows                  5185
  data file does not tile                             4
rows consuming their span exactly                443165
blocks naming a type with no established width        0
blocks whose data/enable/offset triple is absent    195
```

Per type, counting blocks that tile against blocks that do not: `s8` 476/0,
`s16` 190/0, `f16` 564/0, `u16` 39/0, `u32` 10/0. One byte for `s8`, two for
`s16`, `u16`, and `f16`, four for `u32`. `f16` is IEEE-754 binary16 in
little-endian byte order. For example, retail resource `0x03A70820` stores
`D0 63`, which decodes to 1000.0 and agrees with the independent CSV
corpus. Public tests cover normal, subnormal, signed zero, infinity, and
NaN patterns; NaN export retains its raw halfword.

The four blocks that did not tile are all Chinese `xtx/quest` definitions.
Every one of their 639 rows contains `s32` plus ten strings while the schema
declares `s32` plus eleven strings. Row offsets bound every record exactly. No
non-trailing column is missing. The CSV view therefore represents the
last declared value as an explicit `[@missing]` cell instead of inventing
bytes or rejecting the other ten values.

### Static-sheet CSV export

`xivl extract <game-directory> --output <directory>` scans the install for
SSD definition documents and follows each block's explicit data, enable, and
row-offset resource ids. It does not use a master list, a sibling checkout, or
the third-party extractor. The frozen client produced:

```text
definition documents and CSV files       803
distinct output names                     803
rows                                    183678
declared block triples absent              195
explicit missing trailing values          639
conflicting duplicate logical values       57
```

All 195 absent blocks are `chs` definitions spanning 187 sheets. A direct
catalog of the install's 140180 resource paths found none of their 585 data,
enable, or row-offset ids, and the install contains no SqPack index files
that could name an alternate container. Twelve samples across both observed
identifier regions:

```text
sheet                       data        enable      offsets     result
InstanceRaidHamletDefense   03A227E1    03A227E2    03A227E3    all absent
brd0j5                      03A220A5    03A220A6    03A220A7    all absent
etc200                      03A222E5    03A222E6    03A222E7    all absent
etc5g6                      03A223E5    03A223E6    03A223E7    all absent
etc5u3                      03A224F5    03A224F6    03A224F7    all absent
gcl103                      03A21F85    03A21F86    03A21F87    all absent
gcu702                      03A22931    03A22932    03A22933    all absent
mnk1j0                      03A22195    03A22196    03A22197    all absent
populaceBountyPresenter     03A21F51    03A21F52    03A21F53    all absent
populaceYukata              03A22997    03A22998    03A22999    all absent
war0j8                      03A22035    03A22036    03A22037    all absent
xtx/title                   0B450BD8    0B450BD9    0B450BDA    all absent
```

The comparison corpus does not contradict that result. Across the row ranges
of all 195 declarations, it has zero non-empty values in the 9445 cells whose
columns are unique to those Chinese definitions. The corpus therefore did not
recover, synthesize, or bundle the absent strings. The declarations name an
unshipped optional language payload, not a resource-resolution fallback the
exporter missed.
The retail resource-resolution result is complete, but public malformed-input
conformance is still required to move export beyond `partial`.

The output filename set, two header rows, and per-file row counts match the
803-file comparison corpus.
After excluding rich-token rendering and normalizing booleans and literal
escapes, 282054 differing non-empty cells were `f16` values for which the
comparison corpus rounded to six decimals and this exporter retained the
exact binary16 value. Six non-empty, non-`f16` values genuinely disagree:
rows 2115 and 2133 of `xtx__fixedphrase.csv` differ in both their `u32` and
string values, and rows 295 and 299 of `xtx__textcommand.csv` carry different
command strings. In each case, the row is enabled in the Chinese resource and
disabled in all four other language resources. The comparison corpus fills
the missing common columns from the preceding enabled row: 2114 into 2115,
2122 into 2133, 294 into 295, and 298 into 299. The bytes for all four rows
are present in their declared resources. These are sparse-row carry-forward
defects from another source, not unresolved links and not defects in this exporter.
The first-party output keeps the values attached to their actual enabled row.

The CSV representation is UTF-8 with LF endings. Literal backslashes and
opening brackets in text are escaped. A rich token is
`[@name:<exact-token-hex>]`, preserving its framing and payload exactly.
Finite floats use a decimal that round-trips to the same value. NaNs keep
their raw bits. When two definitions supply different values for the same
logical cell, the first remains readable and each alternate is appended as
`[@duplicate:<UTF-8-hex>]`. The exporter reports the conflict count.
