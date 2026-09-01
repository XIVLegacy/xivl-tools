# The configuration files

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

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

## Where they are, and why that matters here

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

## One sample is a structural claim, not a semantic one

There is exactly one of each file and no second install to diff against, so
this reading stops at structure. The parser carries every word as an unresolved
field with its offset and value, while public reports expose only shape and
round-trip facts. A differential experiment against `ffxivconfig.exe` could
establish field names, but nothing here depends on them.

The one exception is the leading word, and it is an exception because
evidence outside the file settles it. See "What the leading word is".

## The stamped word grid

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

## What the leading word is

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

## The two files that carry no stamp

`config.lng` is eight bytes, two words, and no leading stamp - the census
finds neither of the stamped constants in it. `config.rgn` is five bytes,
all zero, and is not a word grid at all: five bytes is not a multiple of
four and nothing divides it. It is read as one opaque body carried whole,
which is the entire claim the `config-rgn` row makes.

## The printable-run census, and what it is not

The reader reports maximal runs of four or more printable units, scanned
independently under ASCII and UTF-16LE. It is a census, not a field list, and its false positives are the reason it is called one:

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

## The round trip

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

## The four readings are not interchangeable

Nothing in the bytes distinguishes the four files - two stamps that are
not signatures, and two files with no leading word at all - so the caller
names the reading, as it does for an enable file and a row-offset array.
Reading one file as another fails rather than producing a plausible
answer: `config.rgn` read as `config-lng` is five bytes against a grid of
four-byte words and reports `trailing-partial-record`, which is a public
conformance case.

## Deliberately not read

The unexamined user-data subtree holds 66 files - 34 `.cmb`, 28
without a suffix, 4 `.log` - and the `.cmb` files are obfuscated macro
text at a near-fixed 998 or 1000 bytes. They remain out of scope because they
are user-written data and have no matrix row.
