# Format evidence

Byte-layout evidence behind the parsers in `src/formats`, with the retail
citation for every claim. A layout with no page here is not a claim this
repository makes.

[Documentation index](README.md)

## Evidence pages

- [SEDB, RES, and resource paths](formats/sedb-res.md)
- [SSD documents, sheets, and scrambled XML](formats/ssd-sheet.md)
- [SQEX containers](formats/sqex.md)
- [Configuration files](formats/configuration.md)
- [Static-actor SAN records](formats/staticactor-san.md)
- [Lua paths, LPB wrappers, and Lua 5.1](formats/lua-lpb.md)
- [GTEX fields and PWIB segments](formats/gtex-pwib.md)

## How a fact gets here

- A layout is established against retail Final Fantasy XIV 1.23b data. A
  published layout or field name from an external project is only a lead; it
  becomes a claim only when retail evidence establishes it
  (`docs/source-and-data-policy.md`).
- A fact established in another repository is promoted with a
  `repository:path, sha256 <digest>` citation naming the exact source file
  by its byte content, plus a local test. This repository owns the promoted
  copy. There is no automated freshness promise.
- Observation dates in a citation are reference data and are kept verbatim.
  They identify the client revision the reading was taken from.
- A field this project has not resolved is named as unresolved and its
  bytes are preserved, never dropped.

## What this repository does not claim

The `partial` read statuses for `sedb` and `res` in
`data/support-matrix.json` cover container enumeration and nothing else.
Specifically not handled:


- payload interpretation for any subtype. A non-composite container's
  payload is one opaque span with a digest;
- the meaning of `unknownA`, `flags`, the RES `unknownB`, and the
  directory `kind` values. The values are carried into the report;
- non-SEDB layouts remain outside the SEDB/RES claim. GTEX has its own
  loader-backed reader and PWIB reports two loader-bounded segments without
  treating its SEDB prefix as a standalone child; MapLayout, zero-filled, and
  unrecognized files still fail with `bad-magic`;
- the `wrb` chunk tree, model, mesh, skeleton, and texture layouts, which are
  outside this support claim;
- extraction and manifesting of subresource bytes, which are also outside this
  support claim. The `inspect` document is a structural report, not an export: it
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
  refused by signature; neither is covered by this support claim;
- writing. `write` stays `none`: the encoder exists so the decode can be
  round-tripped in this crate's own tests, and a write claim needs
  round-trip fixtures of its own;
- a container whose base name is not the one it is stored under. The key
  is the name, so a renamed file is unreadable, and the reading reports the
  name it used rather than guessing at another.

The `partial` read statuses for `config-sys`, `config-pad`, `config-lng`, and `config-rgn` cover the shape and nothing above it:

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

The `supported` write statuses for the same four rows cover the round trip and nothing above it:

- a file this tool wrote after a value was changed has never been through
  the client. `verified` waits on that;
- nothing validates a value. A word the client would reject is written
  back exactly as a caller set it, because this project does not know
  which values it would reject.

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

See the [documentation index](README.md) for the support-matrix page that
records these status claims.
