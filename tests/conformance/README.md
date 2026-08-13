# Conformance cases and oracles

Use this directory to add conformance cases and external oracle records. The
[conformance interface](../../docs/conformance-tests.md) defines their behavior,
and validation contracts live under [`schemas/`](../../schemas/).

```text
cases/<id>/case.json    case manifest; directory name equals the case id
cases/<id>/*.json       expected normalized output
oracles/<id>.json       how to invoke an external implementation
```

The runner is `apps/conformance`; `cargo test` runs the same cases through
the same library code.

```bash
cargo run -p xivl-conformance --bin conformance -- run
```
