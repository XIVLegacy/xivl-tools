# The SQEX container

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

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

## What the container is

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

## The key is the file's base name

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

## How it was derived

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

## The decode over the install

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

## What the widget documents are

The plaintext is XML in a widget vocabulary, with no declaration in any of
the 1155 documents:

```text
roots   Window 618, ResourceDictionary 482, SkinResources 20,
        StyleResources 20, root 13, DesktopWindow 2
distinct element names                                  117
distinct attribute names                                279
  of those, namespace qualified                           2
```

Against the reader's SSD subset the documents add exactly two constructs and no others:

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

## Parser behavior over the corpus

The parser in `src/formats` was run over the same 1524 files separately,
through the command line, to check that it agrees with the research
command:

```text
xivl inspect <file> --as sqwt   1155 of 1155 containers read
                                 369 refused as bad-magic
```

The 369 are the 207 GTEX, the 160 SPKL, and the two plain CSS documents.
0 panics.

## Out of scope

- the GTEX and SPKL files stored beside these containers are out of
  scope here. Neither format is covered by this page. Both are refused by
  signature and counted;
- the cipher is implemented in house rather than taken from crates.io. It
  is a small amount of code, avoids another dependency, and
  the block-order departure above means a stock implementation would have
  needed wrapping anyway;
- `client/sqwt/` gets no path convention here. These files are named, the
  resource-path claim covers `data/` only, and nothing in the library
  needs one.
