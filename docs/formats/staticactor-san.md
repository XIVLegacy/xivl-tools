# The static-actor SAN record table

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

Promoted comparison references:

```text
xivl-client-data:manifests/retail_inputs.json, sha256 fec8c7932b4c6c733ad5ad4afd4228d855581cf57f2c40ddd84d5ecd29c5930c
xivl-client-data:manifests/staticactor_class_paths.json, sha256 d612438827e5997422ab6f64a807e567ddf1b953c532e8a319d67b93c53c9db0
xivl-client-data:tools/extract_staticactor_san.py, sha256 fd1da86e30bb28279fa51dd5cd1cea6d29346c68ccb972ece7a5d14ee2cdb808
```

The sanctioned retail 1.23b input is 108911 bytes with SHA-256
`bb7306461b1728493242016a16d9dd5257d7512c60e423b017de5ec7aced3d14`.
It begins with plain ASCII `sane`. XOR 0x73 applies to every byte from offset
4 through the end. After decoding, offsets 4 through 8 are a five-byte unknown
span and offsets 9 through 12 are a big-endian record count. The body begins at
offset 13 and repeats this framing exactly that many times:

```text
u32 big-endian value
zero-terminated byte string
```

The terminator is therefore byte 0x73 in the encoded input. The retail count
is 2812. Parsing exactly 2812 records consumes the complete file at offset
108911, with no partial or trailing bytes. The four-byte values are unique,
strictly increasing in file order, and range from 12002 through 330002. Every
decoded string is ASCII, begins with `/`, and is 17 through 55 bytes before
its terminator. There are 826 distinct decoded strings; the second lexical
segment occurs as `Command` 1661 times, `Quest` 734 times, and `Status` 398
times. It occurs as `Judge` 19 times. These are byte-string census labels, not
semantic actor categories. The decoded records agree exactly with the existing
client-data
inventory. That agreement is a comparison, not authority for the unknown
five-byte header or either record member's meaning.

The reader preserves the five unknown header bytes by span and encoded and
decoded digest. It reports each record's complete span and four-byte value. It
also reports the string's encoded span, terminator, decoded length, and digest.
It reports whether the decoded bytes are ASCII and start with `/`; it does not
reject a record when either observation is false. This keeps the structural
read lossless by reference to the caller's input without publishing payload
strings in an inspection report.

The fixed public record budget is 100000. The parser does not reserve from the
untrusted count and refuses a larger declaration at offset 9. It rejects a
truncated header, a missing or unterminated declared record, a terminal fragment
shorter than the minimum five-byte record, and complete bytes after the
declared record count. Generated public fixtures cover each boundary, and the
deterministic truncation and byte-mutation sweep runs the reader over every
staticactor fixture.

This moves `staticactor-san` read from `planned` to `partial`. It does not
establish:

- what the five unknown header bytes mean;
- whether the four-byte value is signed, what it identifies, or whether
  uniqueness and increasing order are format requirements;
- whether the string names a class, a resource, or another object, or whether
  ASCII and a leading slash are requirements rather than properties of this
  one retail input;
- whether another SAN header or record variant exists;
- writing or lossless JSON export. Inspection is a redacted structural report,
  so export remains `planned` and write remains `none`.
