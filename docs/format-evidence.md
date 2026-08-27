# Format evidence

Byte-layout evidence behind the parsers in `src/formats`, with the retail
citation for every claim. A layout with no entry here is not a claim this
repository makes.

## How a fact gets here

- A layout is established against retail Final Fantasy XIV 1.23b data. A
  published layout or field name from an external project is a lead until
  then, never a claim (`docs/source-and-data-policy.md`).
- A fact established in another repository is promoted with a
  `repository:path, sha256 <digest>` citation naming the exact source file
  by its byte content, plus a local test. This repository owns the promoted
  copy afterwards. There is no automated freshness promise and no checkout
  of the source repository.
- Observation dates in a citation are reference data and are kept verbatim.
  They identify the client revision the reading was taken from.
- A field this project has not resolved is named as unresolved and its
  bytes are preserved, never dropped.

## Promoted evidence: SEDB, RES, and resource paths

Citation:

```text
bahamut-navmesh:docs/world-layout-data.md, sha256 2fc79e8a131fedd52da5373e4b644347079e2e34ca6239617de5b4284f1d73da
bahamut-navmesh:docs/model-and-collision-data.md, sha256 6b5678670fccae4fb10f7283f3be46416bc6fceff744a55be45a5be83b3fb197
```

The source statements, quoted verbatim for the client revision they record:

> Retail world-layout evidence behind the MapLayout and scene parsers.
> Source: FFXIV 1.23b client install, `game.ver 2012.09.19.0001`.
> Evidence includes MapLayout 0x29D90001, its complete `lyb` scene
> container, resource-key bindings, unit-tree groups, placed-object
> classes, and transform composition checked across the source install's
> MapLayout resources.

> Retail geometry evidence behind the SEDB model and PHB collision
> parsers. Source: FFXIV 1.23b client install, `game.ver
> 2012.09.19.0001`. Evidence includes models 0x89940554, 0x899500BF,
> 0x897D03A9, 0x72E90455, and 0x72E50007; mesh chunk checks across the install;
> the `col_*` primitive hulls; and PHB collision resources read across the
> source install.

Additional resources in the evidence set are
0x7EB50002 (bhp), 0x89930214 (bxt texture) and 0x898C000D (sniv).

Only the three sections below are promoted. The model,
mesh, and chunk-tree layouts in the cited document belong to later work and
are not claims here.

The promotion is owned here, which means it is also corrected here. Read
back against the retail install the same day (see "First-hand retail
observation"), one promoted statement did not hold: the field at 0x10 is
not a container size for every subtype. That correction is recorded below
and the citation is left intact, because the provenance of a claim does
not stop being true when the claim is refined.

Local tests own each promoted fact from here on:
`src/formats/src/resource.rs` tests for the path mapping,
`tests/conformance/cases/` for the container layouts, and
`src/formats/tests/malformed_inputs.rs` for the failure behavior.

### Resource ID to DAT path

A 32-bit resource id `0xAABBCCDD` names `data/AA/BB/CC/DD.DAT` under the
client install root, uppercase hexadecimal. Established empirically across
the whole install and by resolving MapLayout table references to existing
files.

The mapping is total: every `u32` names a path, whether or not the file
exists. This repository never resolves a path against a client install on
its own. An install root is always supplied by the caller.

### SEDB container header

All integer fields are little-endian.

```text
0x00  char[4] magic      "SEDB"
0x04  char[4] subtype    "lyb", "PHB", "txb", "vins", "vmdl", "RES ",
                         "shd", "SKL ", "wrb", ...
0x08  u32     unknownA   varies per subtype (8 lyb, 0x120 PHB, 1 txb,
                         0 vins/vmdl, 0xFA0 RES, 0x1D shd, 0x17 wrb);
                         meaning unresolved
0x0C  u16     flags      0x0000, 0x0200, or 0x0400 observed; meaning
                         unresolved
0x0E  u16     headerSize payload begins at container start + headerSize
                         (0x30, 0x40, 0x48 observed)
0x10  u32     declaredSize  advisory; see "The 0x10 field" below. It is
                         the file size for most subtypes, the header size
                         for PHB, zero for mtb, and a genuine sub-file
                         extent for vins and leaf
...   padding to headerSize; subtype-specific extended fields may sit
      here, and RES uses 0x30..0x3F
```

`0x14` is therefore the smallest header a container can declare. The parser
rejects a smaller one, and rejects a `headerSize` or `declaredSize` that
runs past what the input holds, rather than reading what happens to follow.

### RES subresource directory

A `RES ` container is a composite with `headerSize` 0x40 and a subresource
directory. Extended header fields inside the header:

```text
0x30  u32     subresourceCount
0x34  u32     unknownB          points near the trailing name table,
                                relative to 0x40 (inferred, unresolved)
0x38  u32     subresourceCount  repeated; both must agree
0x3C  char[4] typeName          the resource type tag, for example "brt"
```

Directory at 0x40, one 16-byte entry per subresource:

```text
0x00  u32 index
0x04  u32 offset   relative to the payload base,
                   headerSize + 16 * subresourceCount
0x08  u32 size
0x0C  u32 kind     0, 2, and 4 observed; semantics unresolved
```

Observed subresource contents include nested SEDB containers (`shd`,
`wrb`, `SKL `) and non-SEDB trailing regions such as a name table.

Alignment slack is part of the format as observed, not a defect to reject:
the final directory entry of 0x89940554 declares an extent 0x16 bytes past
the end of the file (0x1BC40 against a 0x1BC2A file). The parser clamps
such an extent to the input and reports the clamp as an anomaly. It never
reads past the input and never drops the entry.

## First-hand retail observation

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b), observed 2026-08-01 over all 140180 resource files under
`data/`. Reproduce with:

```bash
python tools/research/census_sedb.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints counts rather than
bytes.

### The resource-path convention

All 140180 files under `data/` match `AA/BB/CC/DD.DAT`, confirming the
promoted convention across the whole install with no exception.

### The 0x10 field

The promoted statement, "container size including the header equals the
remaining file size for top-level containers", holds for 92303 of the
97749 SEDB resources and fails for 5446 of them:

```text
declaredSize == file size    92303   SSCF 77694, RES 9970, vmdl 1682,
                                     vtex 950, veff 920, txb 783,
                                     leaf 156, vins 148
declaredSize == headerSize    5203   PHB 5203
declaredSize <  headerSize     145   mtb 145 (the field reads zero)
declaredSize <  file size       98   vins 58, leaf 40
```

`vins` and `leaf` appear in two rows: 148 of 206 vins and 156 of 196 leaf
resources declare the file size and the rest a shorter extent. The split
is per file, not per subtype, so no subtype can be special-cased into
trusting the field.

So the field is advisory, not authoritative. The parser treats it that
way: a value below the header is not a malformed file but one with MTB-like
structure, so the container extent falls back to `headerSize`, a
`declared-size-below-header` anomaly is recorded, and the remainder is
reported as trailing bytes. Nothing is rejected and nothing is dropped.
Only a value past the end of the input is still an error, because bytes
that are not there cannot be accounted for.

Before this reading the parser rejected all 145 mtb resources outright
with `header-size-out-of-range`. That error kind now means only what its
name says: the header itself does not fit in the input.

### RES directory confirmations

Read directly from `data/89/94/05/54.DAT` (resource 0x89940554), the model
the cited document names:

- the payload base is `headerSize + 16 * subresourceCount` (0x40 + 0x70 =
  0xB0). Subresources 0 through 4 begin exactly on `SEDB` signatures at
  0xB0, 0x55E8, 0xA638, 0xEED8, and 0x1B9E0, which is what fixes the base;
- consecutive subresources are separated by up to three bytes of
  alignment padding, which this parser reports as unknown gaps rather than
  folding into either neighbour;
- the final entry declares 0x1BB20..0x1BC40 against a 0x1BC2A file: the
  0x16 bytes of slack the cited document records, reproduced exactly;
- the last two entries overlap (0x1BB20 begins inside 0x1BB04..0x1BBD0).
  Sampling 107 RES resources found this in all 107, so the overlapping
  trailing region is a property of the format, not a defect. Declared
  offsets and sizes are reported verbatim alongside the resolved spans, so
  the overlap is visible rather than silently resolved.

### Parser behavior over the install

A 1508-file sample, every 93rd resource, run through `xivl inspect`: 1056
SEDB resources parsed, 0 errors, 0 panics. The remaining 452 are not SEDB
at all and are reported as `bad-magic`: GTEX 226, zero-filled 59, PWIB 38,
and a tail of smaller signatures. None of those formats is handled here.

## The SSD document stack

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

### The documents

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

### The SSD document subset

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

### The enable file and the row-offset array

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

### Rows and columns

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

### The sheet string and its obfuscation

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

### Rich-string control tokens

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

### Parser behavior over the install

The counts above come from the research command. The parser in
`src/formats` was run over the same corpus separately, through the command
line, to check that the two agree:

```text
xivl inspect <file>                       7 of 7 xml documents parsed
xivl inspect <file> --as sheet-data    3751 of 3751 string streams parsed
```

0 errors and 0 panics in both passes.

### Negative result: this cipher is not the scrambled XML scheme

`client/sqwt/` holds 1524 files, of which 1155 begin with `SQEX`: 620
`.form`, 482 `.tpl`, 20 `.skin`, 20 `.style`, 12 `.sml`, and one `.xml`. Past the
first 16 bytes they carry 7.87 bits of Shannon entropy per byte at the
median, against 7.96 at the maximum, so the body is either compressed or
encrypted rather than substituted. A fixed byte substitution cannot
produce that from text, so whatever `SQEX` wraps, it is not the sheet
string cipher. The remaining 369 files are 207 `GTEX`, 160 `SPKL`, and two
plain CSS documents with a byte order mark.

### Why two formats have no private conformance case

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

The binary half of the stack has no such problem. Spans, counts, digests,
and token codes are structure, not content, so the enable file, the
row-offset array, the string stream, and the typed rows all carry private
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
seven plaintext documents declare five. The 806 documents declare ten
and no others:

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
declares `s32` plus eleven strings. Row offsets bound every record exactly,
and no non-trailing column is missing. The CSV view therefore represents the
last declared value as an explicit `[@missing]` cell instead of inventing
bytes or rejecting the other ten values.

### Static-sheet CSV export

`xivl extract <game-directory> --output <directory>` scans the install for
SSD definition documents and follows each block's explicit data, enable,
and row-offset resource ids. It does not use a master list, a sibling
checkout, or the third-party extractor. The frozen client produced:

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
This closes the retail resource-resolution question, but it does not add the
public malformed-input conformance required to move export beyond `partial`.

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

## The SQEX container

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b), observed 2026-08-01 over all 1524 files under
`client/sqwt/`. Reproduce with:

```bash
python tools/research/census_sqwt.py --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--client-root`, and prints counts rather than
bytes or text.

These files are not resources. They are named files under `client/sqwt/`,
not `data/AA/BB/CC/DD.DAT`, so the resource-path convention says nothing
about them and this section adds no path claim of its own: a caller supplies
the path, as it does everywhere else.

### What the container is

```text
0x00  char[4] magic       "SQEX"
0x04  u32     reserved    zero in all 1155 containers
0x08  ...     body        whole 8-byte blocks, enciphered
      ...     tail        the final (length - 8) mod 8 bytes, in the clear
```

The signature is all eight bytes, not the tag alone. Every one of the 1155
containers sets the word at 0x04 to zero, so a file carrying the tag and
something else there is not a container this project has read, and it is
refused rather than deciphered on a guess.

The body's whole blocks are enciphered under a key that is the file's own
base name. The final run shorter than a block is left in the clear, which
is why the last few bytes of these files read as ordinary markup:

```text
files under client/sqwt                                1524
  SQEX 1155, GTEX 207, SPKL 160, plain CSS 2
SQEX files by suffix   .form 620, .tpl 482, .skin 20,
                       .style 20, .sml 12, .xml 1
containers carrying the 8-byte signature       1155 of 1155
body length not a multiple of 8                        1000
  the trailing run is printable text           1000 of 1000
body length not a multiple of 16                       1078
  the trailing run is printable text            513 of 1078
```

That pair of lines is what fixes the block size at eight rather than
sixteen. The tails themselves are XML closing markup, which is also the
first evidence that the plaintext is a document rather than compressed
bytes: the most common are `</Window>`, `</Dictionary>`, and
`</Resources>` and their suffixes.

### The key is the file's base name

Established before the cipher was identified, and independent of it. Two
counts over the whole corpus:

```text
enciphered blocks                                   1059234
  distinct                                           459063
  shared by two files with different base names            0
distinct base names                                    1017
distinct first ciphertext blocks                       1017
  a first block belonging to two names                    0
  a name with two first blocks                            0
```

Blocks repeat freely inside a file - one block occurs 969 times in
`EquipWidget.form`, at arbitrary block-aligned positions and sometimes
consecutively - so the cipher maps a block to a block independently of
where it sits. Under a single key, two documents sharing an aligned
eight-byte run would then share a ciphertext block, and across 459063
distinct blocks not one pair of differently named files does. Files with
the same base name in different directories do share blocks. So the key is
a function of the base name, and the first-block bijection above is that
function tabulated: 1017 names, 1017 first blocks, no collision in either
direction, over files whose lengths differ by a factor of fifty.

### How it was derived

Established first-hand, before any external material was read: the
eight-byte signature, the block size and the plaintext tail, the
block-to-block behavior, the key being the base name, and these
eliminations by test:

- the body is not a positional byte substitution. Under one, each residue
  of the position modulo the period would carry only the image of the text
  alphabet; all 256 byte values occur at every residue modulo 1, 4, 8, and
  16;
- it is not stock Blowfish, DES, CAST-128, RC2, TEA, or XTEA under any of
  24 key forms derived from the name - as given, lower-cased, upper-cased,
  with and without the suffix, with and without a terminator, and as a
  CRC-32 of each. The test needs no known plaintext: with the key a
  function of the name, a correct guess must decipher some fixed set of
  first blocks consistently, and none did.

The retail corpus confirmed three facts: the cipher is Blowfish, the
passphrase is the file name with its suffix, and a trailing partial block is
left untouched.

The byte order of a block's two halves remained unresolved. Retail data
settles it: under the big-endian reading not one of the 1155 containers
deciphers to text, and under the little-endian reading all 1155 do. That is
why an off-the-shelf Blowfish is not a drop-in for this format.

The initialization tables are the hexadecimal expansion of pi's fractional
part, which is what the algorithm specifies them to be.
`src/formats/src/blowfish.rs` and `tools/blowfish.py` generate them from
that expansion rather than transcribing them from anywhere.

### The decode over the install

```text
containers decoded                                     1155
  body decodes as UTF-8                                1155
  re-enciphering reproduces the input byte for byte    1155
  opens on a byte order mark                             11
  opens on an XML declaration                             0
```

The round-trip line is what makes losslessness a checked property rather
than an intention, and it is asserted again in
`src/formats/tests/malformed_inputs.rs` on every truncation and byte
mutation of every committed SQEX fixture.

### What the widget documents are

The plaintext is XML in a widget vocabulary, with no declaration in any of
the 1155 documents:

```text
roots   Window 618, ResourceDictionary 482, SkinResources 20,
        StyleResources 20, root 13, DesktopWindow 2
distinct element names                                  117
distinct attribute names                                279
  of those, namespace qualified                           2
```

Against the reader's SSD subset the documents add exactly two constructs
and no others:

```text
documents using a comment                                12
documents using an ampersand                              5
documents using CDATA, a doctype, or another bang         0
documents using a processing instruction                  0
documents using a single-quoted attribute value           0
documents using a namespace-qualified element name        0
```

So `xml.rs` grew a second profile rather than a second parser. The widget
profile accepts comments, which it counts and does not keep, and carries
ampersand runs verbatim without expanding them; everything else the SSD
profile refuses, it still refuses. Two documents are not well-formed XML
by a strict reading and the client accepts them both: `stafflist.xml`
carries bare ampersands inside attribute values, and `Staffroll.form` uses
a namespace prefix on an attribute that nothing binds. Both are private
conformance cases here.

### Parser behavior over the corpus

The parser in `src/formats` was run over the same 1524 files separately,
through the command line, to check that it agrees with the research
command:

```text
xivl inspect <file> --as sqwt   1155 of 1155 containers read
                                 369 refused as bad-magic
```

The 369 are the 207 GTEX, the 160 SPKL, and the two plain CSS documents.
0 panics.

### Out of scope

- the GTEX and SPKL files stored beside these containers are out of
  scope here. GTEX is a row for later work with 21161 further resources
  under `data/`, and reading it here would open that resource coverage;
  SPKL has no matrix row at all. Both are refused by signature and
  counted;
- the cipher is implemented in house rather than taken from crates.io. It
  is a small amount of code, avoids another dependency, and
  the block-order departure above means a stock implementation would have
  needed wrapping anyway;
- `client/sqwt/` gets no path convention here. These files are named, the
  resource-path claim covers `data/` only, and nothing in the library
  needs one.

## The configuration files

source: retail FFXIV 1.23b client install (game.ver 2012.09.19.0001,
patch.ver 1.23b) and its configuration directory, observed 2026-08-01.
Reproduce with:

```bash
python tools/research/census_config.py --config-root <dir> --client-root <install>
```

That command is research only. It makes no support claim, never runs in
CI, requires an explicit `--config-root`, and prints counts, offsets, and
lengths rather than values.

Nothing in this section is promoted from another repository and no published
lead was consulted. Every statement below was established against the files
named above.

### Where they are, and why that matters here

The configuration files are not under the client install. They live under the
explicitly supplied configuration root, and the four are:

```text
config.sys   684 bytes
config.pad   328 bytes
config.lng     8 bytes
config.rgn     5 bytes
```

They are also not client assets. They are user-written settings, which is why
the reports below carry spans and counts rather than field values and why
`docs/source-and-data-policy.md` classifies them as user-written data.

### One sample is a structural claim, not a semantic one

There is exactly one of each file and no second install to diff against, so
this reading stops at structure. The parser carries every word as an unresolved
field with its offset and value, while public reports expose only shape and
round-trip facts. A differential experiment against `ffxivconfig.exe` could
establish field names, but nothing here depends on them.

The one exception is the leading word, and it is an exception because
evidence outside the file settles it. See "What the leading word is".

### The stamped word grid

`config.sys` and `config.pad` are a leading 32-bit word and then a grid of
little-endian 32-bit words, with no remainder and no length field
anywhere:

```text
0x00        u32     stamp
0x04 ...    u32[]   the grid, to the end of the file
```

```text
file         stamp        words   zero   non-zero
config.sys   0x20120419     170    120         50
config.pad   0x20100211      81     67         14
```

Which slots an install has written is structure and is reported. What it
wrote in them is not. Inside the grid sit fixed-width text fields, which
the run census below locates without the reader having to declare where
they are: `config.sys` carries one at 0x60 and one at 0xA0, each
NUL-terminated UTF-16LE fields padded with zeros out to a field boundary, and the
second is a path this project can confirm against the filesystem because
it names the `screenshots/` folder stored beside the file.

### What the leading word is

Both leading words read as dates - 2012-04-19 and 2010-02-11 - and neither
is this install's. The file was written in 2026 and the client is
game.ver 2012.09.19.0001, which is later than either, so the word is not a
save timestamp. What it is, is compiled in:

```text
executable          0x20120419   0x20100211
ffxivboot.exe                0            2
ffxivconfig.exe              2            3
ffxivgame.exe                2            2
ffxivlogin.exe               0            0
ffxivupdater.exe             0            0
```

Both values occur as immediate constants in three of the five client
executables, which is why this project calls the word a format stamp and
not a field. The parser still does not test it: one sample per file is not
a signature, so a value the client would refuse is not a value this
project refuses.

### The two files that carry no stamp

`config.lng` is eight bytes, two words, and no leading stamp - the census
finds neither of the stamped constants in it. `config.rgn` is five bytes,
all zero, and is not a word grid at all: five bytes is not a multiple of
four and nothing divides it. It is read as one opaque body carried whole,
which is the entire claim the `config-rgn` row makes.

### The printable-run census, and what it is not

The reader reports maximal runs of four or more printable units, scanned
independently under ASCII and UTF-16LE. It is a census, not a field list,
and its false positives are the reason it is called one:

```text
file         encoding   offset   units
config.sys   utf16le     0x0060      12
config.sys   utf16le     0x00A0      63
config.pad   utf16le     0x0000       9
config.pad   ascii       0x000E       4
config.pad   ascii       0x001E       6
config.pad   ascii       0x002C      12
config.pad   utf16le     0x002C      12
```

Two of `config.sys`'s runs are its text fields. Not one of `config.pad`'s
is text at all: the run at 0 is a sixteen-byte binary device identifier
whose halves happen to be printable UTF-16 units, the two short ASCII runs
are tags inside two such identifiers, and the twelve at 0x2C are a byte
mapping whose values fall in the printable range by coincidence. A rule
that called any of them a string field would be wrong four times out of
five, so the reader counts them and names none of them.

### The round trip

This is the repository's first write claim, and it is a checked property
rather than a design intention. The model holds every byte of the input in
exactly one place - the stamp, the grid, or the opaque body - and
`ConfigFile::encode` puts them back:

```text
xivl validate <file> --as config-sys    round trip, 684 bytes
xivl validate <file> --as config-pad    round trip, 328 bytes
xivl validate <file> --as config-lng    round trip,   8 bytes
xivl validate <file> --as config-rgn    round trip,   5 bytes
```

4 of the 4 configuration files on the owner's machine read, 0 do not, and
all 4 write back byte for byte. The four private fixtures are a frozen
snapshot of those files rather than the files themselves, because the
client rewrites them whenever a setting changes. See
`docs/conformance-tests.md`, "Fixture roots". The same round trip is asserted in
`src/formats/tests/malformed_inputs.rs` on every truncation and byte
mutation of every committed configuration fixture, under all four
readings, so it holds for inputs no client ever wrote as well as for the
four that one did.

What the round trip does not establish is that the client accepts a file
this tool wrote after a value was changed. Nothing here has been through
the game. That is why `write` stops at `supported`: `verified` for a write
should mean retail accepted it, and only the owner running the client can
say that.

### The four readings are not interchangeable

Nothing in the bytes distinguishes the four files - two stamps that are
not signatures, and two files with no leading word at all - so the caller
names the reading, as it does for an enable file and a row-offset array.
Reading one file as another fails rather than producing a plausible
answer: `config.rgn` read as `config-lng` is five bytes against a grid of
four-byte words and reports `trailing-partial-record`, which is a public
conformance case.

### Deliberately not read

The unexamined user-data subtree holds 66 files - 34 `.cmb`, 28
without a suffix, 4 `.log` - and the `.cmb` files are obfuscated macro
text at a near-fixed 998 or 1000 bytes. They remain out of scope because they
are user-written data and have no matrix row.

## What this repository does not claim

The `partial` read statuses for `sedb` and `res` in
`data/support-matrix.json` cover container enumeration and nothing else.
Specifically not handled, and reported rather than guessed:

- payload interpretation for any subtype. A non-composite container's
  payload is one opaque span with a digest;
- the meaning of `unknownA`, `flags`, the RES `unknownB`, and the
  directory `kind` values. The values are carried into the report;
- any non-SEDB resource. GTEX, PWIB-wrapped textures, MapLayout, and the
  zero-filled and unrecognized files fail with `bad-magic`. Together they
  are 452 of every 1508 resources sampled;
- the `wrb` chunk tree, model, mesh, skeleton, and texture layouts, which
  belong to later work;
- extraction and manifesting of subresource bytes, which is also later
  work. The `inspect` document is a structural report, not an export: it
  carries spans, counts, and digests, never payload bytes.

The `partial` read status for `ssd-sheet` covers the schema document, the
data stack, binary16 values, the bounded `xtx/quest` trailing omission, and
install-root linking. It remains partial because the new paths do not yet
have public conformance coverage for their full malformed-input space.
Specifically not handled:

- any column type beyond the ten the 1.23b documents declare. A type with
  no width established against retail data is refused with
  `unknown-column-type` rather than given an invented width. The
  vocabulary is closed against the install: no block in any of the 806
  documents names a type outside it;
- the meaning of `mode`, `cache`, and `type`. They remain verbatim
  attributes. The exporter does use `column_max`, `column_count`, and
  `index` to assemble the CSV view;
- preservation of absent block contents. The frozen install omits all three
  resources for 195 Chinese definitions, so the exporter reports and skips
  them. Direct cataloging and the comparison corpus both confirm the payloads
  are absent rather than reachable through another path;
- complete public conformance coverage of linking across a whole document, missing
  trailing values, duplicate logical cells, and malformed resource sets.

The `partial` read status for `rich-string` covers framing, the 26-code
vocabulary, and payload expressions. It remains partial because the full
retail vocabulary and malformed expression space are not yet represented by
public conformance cases:

- nested `0x02 .. 0x03` framing inside a payload is kept whole rather than
  replacing the raw payload, even when expressions are also decoded;
- the 0xF1 payload-length escape rests on one occurrence in the install.

The `verified` read status for `scrambled-xml` covers the container and
nothing above it. Specifically not handled:

- what the encoder derives `keyB` from. The decoder recovers it from known
  plaintext and needs no answer, so this is an open question rather than a
  gap in the read;
- a scrambled resource whose plaintext does not open on a byte order mark
  and `<?xml`. The key recovery has nothing to stand on there, and such an
  input is refused with `bad-magic` rather than decoded on a guess. Two
  resources in the install take that path;
- writing. `write` stays `none`: the encoder is exercised only inside this
  crate's own tests, and a write claim needs round-trip fixtures of its own;
- the document's meaning. The container hands plain bytes to `xml` and
  `ssd`, and every claim about what a document says belongs to those rows.

The `verified` read status for `sqwt` covers the container and the shape
of the document inside it. Specifically not handled:

- what the widget markup means. Not one of the 117 element names or 279
  attribute names is interpreted. They are counted and named in a census
  and nothing more. A widget document model, if it is ever wanted, is its
  own row;
- entity expansion. An ampersand run is carried verbatim, so `&amp;` stays
  five characters and the text a caller reads back is the text the file
  holds;
- what the reserved word at 0x04 is for. It is zero in all 1155 containers
  and is required to be zero, which is a recognition rule and not a
  reading;
- the `GTEX` and `SPKL` files stored in the same directory. They are
  refused by signature; GTEX belongs to later work and SPKL has no row;
- writing. `write` stays `none`: the encoder exists so the decode can be
  round-tripped in this crate's own tests, and a write claim needs
  round-trip fixtures of its own;
- a container whose base name is not the one it is stored under. The key
  is the name, so a renamed file is unreadable, and the reading reports the
  name it used rather than guessing at another.

The `partial` read statuses for `config-sys`, `config-pad`, `config-lng`,
and `config-rgn` cover the shape and nothing above it:

- no field's meaning. A word is carried with its offset and its value and
  nothing is named, because one sample cannot fix a meaning;
- the leading word is called a format stamp on the executable evidence
  above and is not otherwise interpreted, and the parser does not require
  it to be either of the two observed values;
- a printable run is counted, never treated as a string field. Four of the
  five runs the two grids carry are not text;
- `config.rgn` is five bytes carried whole, with no field at all. There is
  no malformed input for it, because every byte string is a valid body;
- what a file the client has never written looks like. Everything here
  rests on one sample of each, which is why these rows are `partial` and
  not `supported` however complete the round trip is.

The `supported` write statuses for the same four rows cover the round trip
and nothing above it:

- a file this tool wrote after a value was changed has never been through
  the client. `verified` waits on that;
- nothing validates a value. A word the client would reject is written
  back exactly as a caller set it, because this project does not know
  which values it would reject.

## Promoted evidence: Lua paths and LPB wrappers

Promoted references:

```text
xivl-decomp:docs/script/lpb-format.md, sha256 38cf3bbc0b27681a7eb89f10f88968ea8ae10695fe41bfb044c0f8b96d9e344e
xivl-decomp:docs/script/lua-bytecode-format.md, sha256 346b62a5b6c1732e3693b88c71c9383f02b0e700c5b6978800fd8987183ceb56
xivl-decomp:tools/decode_lpb.py, sha256 74994d6714a5acd161d241a40db8b3907b88871129e29ac9aed821191dd5020a
```

Lua resource paths use a character-wise involution after ASCII case folding:
`a` through `j` pair with `9` through `0`, `k` through `z` pair so their
letter positions sum to 37, digits `0` through `9` pair with `j` through `a`,
and other ASCII bytes pass through. The client corpus paths are ASCII, so the
public API rejects non-ASCII input rather than extending the evidence with a
locale or Unicode case rule. Unit coverage exhausts all 128 ASCII bytes and
public conformance covers a mixed path plus the rejected non-ASCII boundary.

LPB has two evidenced wrappers around compiled Lua 5.1 chunks:

```text
rlu 0B: 8-byte header, then an unmodified chunk beginning 1B 4C 75 61 51
rle 0C: 16-byte header; bytes 13 onward XOR 73 decode to that same signature
```

For `rlu`, bytes 4 through 7 are preserved as uninterpreted header bytes. For
`rle`, bytes 4 through 7 and byte 12 are preserved the same way; bytes 8
through 11 are reported as a little-endian advisory size but are not enforced,
because the evidence records both offsets from decoded size and one outlier.
Bytes 13 through 15 are the encoded prefix of the Lua signature and bytes 16
onward are the remaining encoded payload. Inspection reports every span and a
digest for uninterpreted bytes, and extraction returns the complete decoded
chunk. Public cases cover both wrappers, a truncated header, and a payload
whose decoded signature is not Lua 5.1.

The LPB statuses remain `partial`: the wrapper reader does not assign meaning
to the advisory size or unknown header bytes, claim that no additional wrapper
variant exists, or write LPB. The binary export is the compiled chunk only;
retaining the parsed `LpbFile` alongside it is what keeps the original
wrapper's unknown bytes available to callers.

### The bounded Lua 5.1 structure view

The decoded target header is exactly the 12-byte official Lua 5.1 header
recorded above: format 0, little-endian, 4-byte `int`, 4-byte `size_t`, 4-byte
instruction, 8-byte floating `lua_Number`. This agrees with the official Lua
5.1.5 loader's header construction and load order in
[lundump.c](https://www.lua.org/source/5.1/lundump.c.html) and the fixed header
constants in [lundump.h](https://www.lua.org/source/5.1/lundump.h.html). Another
width, byte order, number representation, version, or format is refused as
`unsupported-lua-header`; it is not interpreted using the host platform.

After the header, the official loader reads one root function prototype. Each
prototype holds an optional `size_t`-prefixed, zero-terminated source string;
two line integers; four shape bytes; an instruction vector; constants; nested
prototypes; and the line, local, and upvalue-name debug tables. The constant
tags accepted by the official loader are nil (0), boolean (1), number (3), and
string (4). The public model retains exact string and number bytes, while the
normalized report publishes only type, span, length where applicable, and
digest.

The instruction layout and opcode metadata follow the official Lua 5.1.5
[lopcodes.h](https://www.lua.org/source/5.1/lopcodes.h.html) and
[lopcodes.c](https://www.lua.org/source/5.1/lopcodes.c.html). A 32-bit word has
the 6-bit opcode at bit 0, 8-bit A at bit 6, 9-bit C at bit 14, and 9-bit B at
bit 23. Bx is the combined 18 bits at bit 14. sBx subtracts the official
131071 excess-K bias from Bx. Opcodes 0 through 37 are the official `MOVE`
through `VARARG` table; another 6-bit value is malformed bytecode.

Each decoded instruction retains its exact four-byte span and raw word, its
zero-based index and decoded-chunk offset, and the official opcode number,
name, encoding mode, and mode-appropriate operands. The official B and C
argument modes distinguish unused values, plain values, registers, and RK
fields. In an RK field, raw values with bit 8 set are constant references and
the low 8 bits are their index; other values are register references. The
model retains that structure even when the referenced constant table entry is
absent. It never manufactures a constant value or publishes string contents.

The reader consumes the complete root prototype and rejects trailing bytes.
Every signed Lua `int` count must be nonnegative. Before allocation it enforces
these platform-independent budgets:

- at most 128 nested prototype levels and 10000 total prototypes;
- at most 1000000 aggregate instruction, constant, prototype, and debug-table
  entries;
- at most 16 MiB for one string and for all string bodies together.

Generated public conformance covers all three instruction encodings, RK
register and constant forms, all four constant tags, debug tables, a nested
function, and the same decoded chunk behind raw and XOR-0x73 wrappers. The RK
constant case deliberately names an unavailable table index so the normalized
contract proves that no value is invented. Malformed cases cover an invalid
opcode, a truncated instruction word, an unsupported endian marker, a string
allocation bomb, a table-count bomb, a nesting bomb, and trailing bytes. Unit
contracts pin the complete opcode-name order, every opcode's encoding mode,
the bit allocation, RK split, and sBx bias. The repository-wide deterministic
truncation and byte-mutation sweeps exercise the generated LPB fixtures through
both wrapper and bytecode readings; the nesting bomb has its own exact limit
assertion.

The `client-lua` read status remains `partial`, not `supported`. This slice does
not validate referenced constants, prototypes, registers, jump destinations,
or stack behavior; build a control-flow graph; analyze reachability; simulate
the stack; execute the VM; recover source; emit pseudocode; or decompile. Other
serialized representations and retail fixture parity are also not claimed.
Lua source export stays `planned`. LPB remains `partial` for the wrapper
limitations already listed, and neither row gains write support.

## Open questions

- what any field of the configuration files means; a differential experiment
  against `ffxivconfig.exe` could establish that;
- whether the client accepts a configuration file this tool wrote after a
  value was changed;
- what the `user/` subtree's 34 obfuscated `.cmb` files hold, and under
  what scheme;
- what the SQEX reserved word at 0x04 is for;
- why the client enciphers UI markup under a per-file key at all, when the
  key is the file's own name and is therefore no secret;
- what the two documents that are not well-formed XML - bare ampersands in
  `stafflist.xml`, an unbound attribute prefix in `Staffroll.form` - imply
  about the client's own reader;

- what the encoder derives the scrambled container's second word key from;
- why the final-byte correction belongs to one encoded-length residue
  rather than to odd lengths, which is what the shape of the two word
  passes suggests it should;
- why the Chinese `xtx/quest` schema declares one trailing string that none
  of its 639 rows stores;
- what names the 10 string-stream resources no document references;
- why 57 duplicate logical cells disagree across language definitions;
- whether the 0xF1 length escape means anything other than the reading
  that closes the one token carrying it;
- what `mode`, `cache`, `type`, and the `index` list select.

The promoted SEDB and RES evidence leaves these questions open:

- the meaning of SEDB `unknownA` and `flags`;
- what PHB, mtb, vins, and leaf put in the 0x10 field instead of a
  container size, and what bounds their real payloads;
- what the overlapping trailing region of a RES directory describes;
- what the RES directory `kind` values distinguish;
- what `unknownB` points at, and whether it is always relative to 0x40;
- whether a top-level container ever legitimately leaves trailing bytes.
  The parser preserves them as trailing entries either way.

See the [documentation index](README.md) for the support-matrix page this
evidence promotes status claims into.
