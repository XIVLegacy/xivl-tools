# Public fixtures

Authored synthetic bytes, committed and freely redistributable under this
repository's license.

A fixture here is written by this project to exercise a parser, including
deliberately malformed input. It never copies, slices, transforms, or re-encodes
retail data. See `docs/source-and-data-policy.md`.

Layout is one directory per format id from the support matrix. Every byte
here is written by `tools/make_public_fixtures.py`, which describes what
each fixture represents and why it exists:

```bash
python tools/make_public_fixtures.py --check
```

`--check` fails if a committed fixture no longer matches the generator, so
a fixture nobody can regenerate cannot survive in the tree.

The contract check's ASCII rule exempts `.bin` files in this directory. They
are bytes, not text: a parser fixture that could not contain 0x80 and above
could not exercise the parser.

Retail fixtures live outside this checkout and are referenced by hash in
`tests/fixtures/private-manifest.json`.
