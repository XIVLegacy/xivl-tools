# Comments and prose

Use these rules to decide which Rust comments and public prose belong in the
repository. Delete explanatory comments unless they meet a keep criterion.

Deletion is the default. Keep a comment only when it records one of these:

- a current invariant;
- a client or format quirk;
- a wire or file-layout fact;
- a source or evidence citation;
- a safety constraint;
- an API contract not inferable from types and names.

Keep source and evidence identifiers verbatim, including their dates. Compress
other survivors to about one line at the use site. Move a longer contract to a
public declaration or a documentation page and leave a short pointer.

Generated comments are generated output. Preserve them exactly or change the
owning generator and regenerate. JSON Schema descriptions are validation
metadata, not a place for branch history or agent narration.

Command help and error text are public behavior. They are not comments and
must not be removed or rewritten during a comment pass.

When a comment is arguable, keep one concise line and flag the decision in
review notes. Do not silently delete a format fact or a policy rationale that
prevents a reader from violating the data boundary.

Reference project names stay out of code comments and generated artifacts.
Attribution lives in `NOTICE`.

## Authored public prose

Public prose uses a plain, direct register.

- Avoid over-hyphenation and invented compound modifiers. Established
  technical terms keep their hyphens.
- Use semicolons sparingly, preferring periods, commas, or short lists.

Internal working docs are outside this public policy tier.

The [documentation index](../README.md) lists this page, and the
[policy index](README.md) is its canonical shelf.
