# Project scope

XIVLegacy Tools owns clean-room tooling for Final Fantasy XIV 1.23b client
file formats. The target is frozen. The [support matrix](support-matrix.md)
defines the status vocabulary and promotion rules.

## In scope

- reusable libraries for client file and container formats;
- the `xivl` command line for inspection, validation, and extraction;
- lossless export views and structural reports that do not publish private
  client data;
- legacy patch parsing, verification, creation, and a self-hosted service
  component;
- an optional resource explorer backed by the format libraries.

## Out of scope

- server runtime, database, and operations;
- mesh, collision, and navigation compilation;
- ownership of protocol, client ABI, decoded static data, and retail
  observation evidence;
- process attachment, injection, and hooking.

## Workspace

```text
src/formats/       format library crate
apps/cli/          xivl command line
apps/conformance/  conformance runner
data/              machine-readable support matrix
docs/              public contracts and evidence
schemas/           JSON Schemas for contract data
tests/             conformance cases and fixtures
tools/             contract checks and maintenance scripts
.github/           cross-platform contract and Rust CI
```

Patch and export library crates and further front ends are outside the current
scope.
The [source and data policy](source-and-data-policy.md) defines the clean-room
and self-containment boundaries.

See the [documentation index](README.md) for the rest of the public contract.
