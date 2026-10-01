# Documentation

Choose a guide by the job you are trying to do. Start with the [repository
README](../README.md), then use the CLI or DAT workflow guide. The format pages
are detailed references.

## Start here

- [CLI guide](cli.md) - build the command, inspect a file, catalog a DAT tree,
  extract selected resources, and understand the reports.
- [DAT catalog and resource extraction](resource-extraction.md) - follow the
  complete catalog -> select -> extract -> verify workflow.
- [Support matrix](support-matrix.md) - check whether a format is readable,
  writable, or exportable and how strong that claim is.

## Common tasks

| Task | Guide |
|---|---|
| Read one file and get a structural report | [CLI: inspect and validate](cli.md#inspect-one-file) |
| Find resources in a game or resource directory | [CLI: catalog](cli.md#catalog-a-dat-tree) |
| Extract one or more selected resources | [DAT extraction](resource-extraction.md) |
| Verify an extraction without changing it | [CLI: verify-extraction](cli.md#verify-an-extraction) |
| Export sheet definitions as CSV | [CLI: export sheets](cli.md#export-sheets) |
| Export collision geometry | [Zone geometry](formats/zone-geometry.md) |
| Read Lua paths, LPB wrappers, or bytecode structure | [Lua and LPB](formats/lua-lpb.md) |
| Query command metadata and observed loadout writes | [CLI: advanced reports](cli.md#advanced-reports) |

## Format reference

These pages describe the supported file formats.

- [SEDB, RES, and resource paths](formats/sedb-res.md)
- [SSD sheets and scrambled XML](formats/ssd-sheet.md)
- [SQEX containers](formats/sqex.md)
- [Configuration files](formats/configuration.md)
- [Static-actor SAN records](formats/staticactor-san.md)
- [RegionResourceData root and child rows](formats/region.md)
- [GTEX fields and PWIB segments](formats/gtex-pwib.md)
- [Zone geometry export](formats/zone-geometry.md)

The [format evidence index](format-evidence.md) links the evidence and
coverage behind the support matrix. The [Lua 5.1 retail census](lua51-retail-census.md)
records aggregate structural validation for the retained Lua corpus.

## Resource and output contracts

- [DAT catalog and resource extraction](resource-extraction.md) defines the
  catalog, extraction, payload, and verification manifests.
- [Conformance tests](conformance-tests.md) explains public fixtures, private
  fixture identities, normalized output, and the runner.
- [Source and data policy](source-and-data-policy.md) defines what may be
  committed and what must stay outside the repository.

## Contributing and maintenance

- [Style guide](style-guide.md) covers authored Rust, Python, schemas, and
  documentation.
- [AI-assisted contributions](ai_agents/README.md) describes the tracked
  contribution contract.
- [Comments and prose](ai_agents/comments-and-prose.md) is the policy for
  public prose and source comments.
- [Evidence and claims](ai_agents/evidence-and-claims.md) defines citations,
  uncertainty, and data boundaries.
- [Maintenance tools](../tools/README.md) documents contract checks, fixture
  generation, and optional research commands.

The repository contract checks every local link listed in this index.
