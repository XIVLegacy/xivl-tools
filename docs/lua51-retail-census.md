# Lua 5.1 retail census

This page records the 2026-08-27 aggregate-only validation of the complete
FFXIV 1.23b compiled-script inventory owned by `XIVLegacy/xivl-client-scripts`.
The canonical exact result is `data/lua51-retail-census.json`. It contains no
resource path, script path, source or constant string, payload bytes, private
root, host name, timestamp, or per-file row.

The source inventory is identified by the
[retail Lua coverage census](https://github.com/XIVLegacy/xivl-client-scripts/blob/49957ae64471fecd5f705dcb196afb031d4eca7b/docs/retail-lua-coverage.md).
Its retained coverage manifest file hashed to
`33DFFF57BB2419C4B4778F73DB367726DABFA6532C73DF03A687CF31F5F50E31`, and its recorded retail inventory digest was
`C0BC21DE2626F619AD278C4E043F537A2B6C1CF3263D3110323CC31054B97CF2`.

## Result

All 2,671 manifest-owned LPBs passed their recorded file hash, wrapper class,
decoded length and hash, exact evidenced Lua 5.1 header, bounded parser, and
prototype-local structural validation. The accounting was exact: 2,670
XOR-0x73 wrappers plus one raw wrapper, 2,671 accepted payloads, and zero
wrapper, header, parser, or structural failures. Their decoded payloads totaled
5,509,805 bytes.

The payloads contained 16,821 prototypes: 2,671 roots, 14,143 at depth 1, and
7 at depth 2. One script held between 1 and 569 prototypes. The deepest root-
relative nesting depth was 2. Every prototype had empty line-info, local, and
upvalue-name debug tables.

The code vectors held 535,323 words, between 1 and 22,617 per script. All were
decoded opcode words: retail used no `SETLIST C=0` extra words. The mode totals
were 332,171 iABC, 156,211 iABx, and 46,941 iAsBx. RK operands split into
49,738 register forms and 136,385 constant forms. Eleven CLOSURE binding words
were present, all MOVE and none GETUPVAL.

Every one of the 38 official opcode slots is retained in the canonical JSON,
including the zero count for SETUPVAL. The observed counts were:

| Opcode | Count | Opcode | Count |
|---|---:|---|---:|
| MOVE | 59,938 | LOADK | 112,546 |
| LOADBOOL | 10,903 | LOADNIL | 2,780 |
| GETUPVAL | 12 | GETGLOBAL | 29,508 |
| GETTABLE | 18,865 | SETGLOBAL | 7 |
| SETUPVAL | 0 | SETTABLE | 18,792 |
| NEWTABLE | 3,131 | SELF | 73,236 |
| ADD | 1,623 | SUB | 1,165 |
| MUL | 351 | DIV | 142 |
| MOD | 28 | POW | 4 |
| UNM | 3 | NOT | 15 |
| LEN | 325 | CONCAT | 2,054 |
| JMP | 44,843 | EQ | 21,922 |
| LT | 1,728 | LE | 1,256 |
| TEST | 1,866 | TESTSET | 51 |
| CALL | 80,303 | TAILCALL | 987 |
| RETURN | 27,158 | FORLOOP | 1,049 |
| FORPREP | 1,049 | TFORLOOP | 5 |
| SETLIST | 2,917 | CLOSE | 1 |
| CLOSURE | 14,150 | VARARG | 610 |

## Reproduce

Use explicit read-only inputs. The command has no default, environment
fallback, sibling search, or output path. It verifies the retained manifest
identities before parsing and writes aggregate JSON to stdout only.

```powershell
$scriptsRoot = Resolve-Path <xivl-client-scripts-root>
$retailScripts = Resolve-Path <retail-client-script-root>
$scriptsCommit = git -C $scriptsRoot rev-parse HEAD
cargo run --locked -p xivl-formats --example lua51_census -- `
  --client-script-root $retailScripts `
  --coverage-manifest "$scriptsRoot/manifests/retail_lua_coverage.json" `
  --owner-commit $scriptsCommit `
  --check data/lua51-retail-census.json
```

The result proves complete static acceptance for that exact manifest-owned
inventory. It does not prove live execution or another client version.

## Validation boundary

The parser enforces rules that can be decided from one prototype without
execution: table indices, direct and derived register spans, jump bounds,
prototype/debug shape, final RETURN, SETLIST extra-word structure, and CLOSURE
binding structure. Official Lua 5.1.5
[`lopcodes.h`](https://www.lua.org/source/5.1/lopcodes.h.html),
[`lopcodes.c`](https://www.lua.org/source/5.1/lopcodes.c.html),
[`ldebug.c`](https://www.lua.org/source/5.1/ldebug.c.html), and
[`lvm.c`](https://www.lua.org/source/5.1/lvm.c.html) are the semantic
authority.

Compiler branch pairing, reachability, register liveness, CFG construction,
stack simulation, VM execution, source recovery, pseudocode, decompilation,
LPB writing, new wrappers, and normalized string contents remain outside the
claim. See the [format evidence](format-evidence.md) and
[documentation index](README.md).
