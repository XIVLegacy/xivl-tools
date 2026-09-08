# Style guide

Repository style covers authored Rust, Python support tools, schemas,
fixtures, and documentation. It does not establish retail format semantics,
evidence strength, fixture provenance, or compatibility claims.

## General

- Prefer existing local patterns once they exist.
- Keep changes scoped to the format, command, or conformance boundary being
  changed.
- Use small, explicit modules before adding abstractions.
- Follow the [comment policy](ai_agents/comments-and-prose.md) for source
  comments.
- Do not reformat generated products, fixture bytes, or evidence records for
  style alone.

### Documentation

The public [documentation policy](ai_agents/README.md#public-documentation) is
canonical for authored documentation. The
[evidence policy](ai_agents/evidence-and-claims.md) owns claim wording,
citations, confidence, and provenance.

## Rust

- Use stable Rust and keep the workspace free of `unsafe` code.
- Run `cargo fmt` on changed Rust code.
- Use `snake_case` for modules, functions, and variables, `UpperCamelCase` for
  types and traits, and `UPPER_SNAKE_CASE` for constants.
- Prefer explicit domain types over unlabelled tuples or primitive parameters
  when width, offset, or ownership matters.
- Accept bounded byte slices, return `Result`, and retain the failing offset in
  malformed-input errors.
- Do not panic on input-controlled data.
- Keep CLI parsing and presentation in `apps/`; reusable format behavior
  belongs in the owning library module.
- Add focused tests at parser, writer, and command boundaries where behavior
  can regress.

## Python support tools

- Use 4 spaces for indentation and no tabs.
- Use `lower_snake_case` for modules, functions, and variables,
  `UpperCamelCase` for classes, and `UPPER_SNAKE_CASE` for constants.
- Prefer `pathlib.Path`, explicit text encodings, and specific failure paths.
- Keep support scripts orchestration-focused; format behavior belongs in Rust
  unless the tool contract explicitly says otherwise.

## Schemas and fixtures

- Preserve schema-defined names, field order, identifiers, and null behavior.
- Public binary fixtures are authored synthetic data and are not reformatted.
- Edit canonical inputs and generators, then regenerate owned output.
- Preserve the local indentation and ordering of hand-authored JSON and TOML.

## Verification

Use the owning commands in [conformance-tests.md](conformance-tests.md) and
[tools/README.md](../tools/README.md). Formatting is not a substitute for
tests, schemas, conformance, or evidence validation.
