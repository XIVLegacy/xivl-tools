# Evidence and claims

Repository code, tests, and documentation establish this project's
implementation contracts. They do not by themselves establish retail
behavior. A claim about a frozen client format needs the evidence and
conformance coverage recorded by the repository's technical documents.

Agent output, summaries, search snippets, and unattributed statements are
leads. Inspect the underlying artifact before promoting a fact. Make the
narrowest claim the evidence supports and state uncertainty when a value,
name, version, or interpretation is unresolved.

## Numbers in prose

Every figure in authored prose has to carry its sentence's claim. Ask of each
one: is this number the finding, or is it scene-setting?

Figures that are themselves claims stay verbatim. Row counts, coverage ratios, per-file byte
sizes and hashes, offsets, and extraction diffs are the claim itself - the
sentence exists to state them. Removing one destroys evidence.

Incidental figures go and the claim stays. When the sentence is about
something else, a count is unnecessary context: it tells the reader nothing they
can act on, and it invites doubt when their own run differs by one. Keep what
was found. Drop the size of the haystack.

Use an exact value when the source establishes one. Preserve approximate
observations and bounded estimates when they carry the claim, and state the
measurement method, resolution, or unresolved boundary. Do not turn uncertain
evidence into a precise value or remove it merely because it is approximate.
Where the number is incidental, cite the source instead of repeating it.

This governs prose the repository authors. A figure inside a quoted or
transcribed source is source content and stays verbatim, hedge included.

## Immutable citations

A fact promoted from another repository uses this form:

```text
repository-name:path/to/file
```

Add a row, symbol, or section locator when useful. Record an immutable revision
or digest when the claim depends on exact version or byte identity, using the
relevant provenance or checksum record when one exists. Ordinary documentation
citation does not require a hash. Commit hashes and date pins are not default
citation requirements. Branch names, working-tree paths, and sibling
paths are not citations. The promoted fact becomes locally owned and receives
local tests when an implementation behavior is claimed.

## Self-containment

A supported command never discovers a client install, fixture root, or sibling
checkout from workspace layout. A path such as `../consumer` is a forbidden
default. An external research input must be supplied explicitly, must remain
non-gating, and must not become a support claim.

External source code is not copied, translated line by line, or vendored. See
the [source and data policy](../source-and-data-policy.md) for rules on
proposed reuse.

## Data boundary

Public fixtures are authored synthetic bytes. Retail fixtures and user-written
files stay outside the checkout and are represented by identity and derived
structure only. An expected output may carry lengths, spans, counts, and
digests, but never recoverable client or owner values.

Related contracts and navigation:

- [source and data policy](../source-and-data-policy.md): the full data boundary.
- [format evidence](../format-evidence.md): byte-layout claims.
- [documentation index](../README.md): this page's public home.
- [policy index](README.md): the canonical shelf for policy pages.
