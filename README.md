<h1 align="center">XIVLegacy Tools</h1>

<p align="center">
Command line tools for inspecting FFXIV 1.23b DAT files<br>
and exporting sheet CSVs, selected resources, and map collision meshes.
</p>

<p align="center">
<a href="LICENSE"><img src="https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg" alt="License: AGPL-3.0-or-later"></a>
<a href="https://github.com/XIVLegacy/xivl-tools/actions/workflows/checks.yml"><img src="https://github.com/XIVLegacy/xivl-tools/actions/workflows/checks.yml/badge.svg" alt="Checks"></a>
</p>

## Build

Install Rust 1.88 or newer, then run the CLI from this checkout:

```powershell
cargo run --locked -p xivl-cli -- --help
cargo run --locked -p xivl-cli -- inspect tests/fixtures/public/sedb/plain-container.bin
cargo run --locked -p xivl-cli -- catalog "C:\path\to\FINAL FANTASY XIV" --output catalog
```

## Commands

`inspect` and `validate` print a report for one file. `catalog` lists DAT
resources; `extract-resource` and `extract-catalog` write selected outputs;
`verify-extraction` checks them later. `extract` writes sheet CSVs,
`export-zones` writes collision OBJ files, and the other commands handle Lua
wrappers and command data.

## Documentation

- [CLI guide](docs/cli.md)
- [DAT catalog and resource extraction](docs/resource-extraction.md)
- [Format reference, evidence, and support](docs/README.md)
- [Conformance and maintenance](docs/conformance-tests.md)

## License

<a href="LICENSE"><img src="https://www.gnu.org/graphics/agplv3-155x51.png" alt="GNU AGPLv3 logo"></a>
