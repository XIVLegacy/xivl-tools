<h1 align="center">XIVLegacy Tools</h1>

<p align="center">
Client format parsers, conformance tools, and exporters.<br>
Preservation and research tooling for Final Fantasy XIV 1.23b.
</p>

<p align="center">
<a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg" alt="License: AGPL-3.0-or-later"></a>
<a href="https://github.com/XIVLegacy/xivl-tools/actions/workflows/checks.yml"><img src="https://github.com/XIVLegacy/xivl-tools/actions/workflows/checks.yml/badge.svg" alt="Checks"></a>
</p>

## About

XIVLegacy Tools is a Rust workspace for bounded parsing, inspection,
conformance, and export. The [project scope](docs/project-scope.md) defines
the frozen target, supported work, and repository layout.

## Quick start

Build the release CLI, then run it against an authored public fixture:

```text
cargo build --release --locked -p xivl-cli
cargo run --release --locked -p xivl-cli -- inspect tests/fixtures/public/sedb/plain-container.bin
```

See the [CLI guide](docs/cli.md) for commands, selectors, reports, and exit
statuses.

## Documentation

- [Documentation home](docs/README.md)
- [CLI guide](docs/cli.md)
- [Project scope](docs/project-scope.md)
- [Support matrix](docs/support-matrix.md)
- [Conformance tests](docs/conformance-tests.md)
- [Format evidence](docs/format-evidence.md)
- [Source and data policy](docs/source-and-data-policy.md)
- [Tooling and regeneration](tools/README.md)

## Community

Join the [project Discord](https://discord.gg/PxK5RJYQjm) for questions and
community support. Use [Issues](https://github.com/XIVLegacy/xivl-tools/issues)
to report bugs and durable research findings.

## Contributing

Pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before you
open one.

## License

<a href="LICENSE"><img src="https://www.gnu.org/graphics/agplv3-155x51.png" alt="GNU AGPLv3 logo"></a>

Original work created by this project uses the
[GNU AGPL version 3 or later](LICENSE). Material from third parties is covered
by [NOTICE](NOTICE). This project is unaffiliated with and unendorsed by the
publisher. All trademarks belong to their respective owners.
