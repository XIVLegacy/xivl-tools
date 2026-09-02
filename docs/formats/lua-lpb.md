# Promoted evidence: Lua paths and LPB wrappers

[Documentation index](../README.md) | [Format evidence index](../format-evidence.md)

Promoted references:

- [LPB wrapper format](https://github.com/XIVLegacy/xivl-decomp/blob/c55640fe9ad024b2163a563d30d6b7673aff2e13/docs/script/lpb-format.md)
- [Lua bytecode format](https://github.com/XIVLegacy/xivl-decomp/blob/c55640fe9ad024b2163a563d30d6b7673aff2e13/docs/script/lua-bytecode-format.md)
- [LPB decoder](https://github.com/XIVLegacy/xivl-decomp/blob/a8815924361889e5f61a2fed047e7785110ad898/tools/decode_lpb.py)

Lua resource paths use a character-wise involution after ASCII case folding:
`a` through `j` pair with `9` through `0`, `k` through `z` pair so their
letter positions sum to 37. Digits `0` through `9` pair with `j` through `a`.
Other ASCII bytes pass through. The client corpus paths are ASCII, so the
public API rejects non-ASCII input rather than extending the evidence with a
locale or Unicode case rule. Unit coverage exhausts all 128 ASCII bytes and
public conformance covers a mixed path plus the rejected non-ASCII boundary.

LPB has two evidenced wrappers around compiled Lua 5.1 chunks:

```text
rlu 0B: 8-byte header, then an unmodified chunk beginning 1B 4C 75 61 51
rle 0C: 16-byte header; bytes 13 onward XOR 73 decode to that same signature
```

For `rlu`, bytes 4 through 7 are preserved as uninterpreted header bytes. For
`rle`, bytes 4 through 7 and byte 12 are preserved the same way; bytes 8
through 11 are reported as a little-endian advisory size but are not enforced,
because the evidence records both offsets from decoded size and one outlier.
Bytes 13 through 15 are the encoded prefix of the Lua signature and bytes 16
onward are the remaining encoded payload. Inspection reports every span and a
digest for uninterpreted bytes, and extraction returns the complete decoded
chunk. Public cases cover both wrappers, a truncated header, and a payload
whose decoded signature is not Lua 5.1.

The LPB statuses remain `partial`: the wrapper reader does not assign meaning
to the advisory size or unknown header bytes, claim that no additional wrapper
variant exists, or write LPB. The binary export is the compiled chunk only;
retaining the parsed `LpbFile` alongside it is what keeps the original
wrapper's unknown bytes available to callers.

## The bounded Lua 5.1 structure view

The decoded target header is exactly the 12-byte official Lua 5.1 header
recorded above: format 0, little-endian, 4-byte `int`, 4-byte `size_t`, 4-byte
instruction, 8-byte floating `lua_Number`. This agrees with the official Lua
5.1.5 loader's header construction and load order in
[lundump.c](https://www.lua.org/source/5.1/lundump.c.html) and the fixed header
constants in [lundump.h](https://www.lua.org/source/5.1/lundump.h.html). Another
width, byte order, number representation, version, or format is refused as
`unsupported-lua-header`; it is not interpreted using the host platform.

After the header, the official loader reads one root function prototype. Each
prototype holds an optional `size_t`-prefixed, zero-terminated source string;
two line integers; four shape bytes; an instruction vector; constants; nested
prototypes; and the line, local, and upvalue-name debug tables. The constant
tags accepted by the official loader are nil (0), boolean (1), number (3), and
string (4). The public model retains exact string and number bytes, while the
normalized report publishes only type, span, length where applicable, and
digest.

The instruction layout and opcode metadata follow the official Lua 5.1.5
[lopcodes.h](https://www.lua.org/source/5.1/lopcodes.h.html) and
[lopcodes.c](https://www.lua.org/source/5.1/lopcodes.c.html). A 32-bit word has
the 6-bit opcode at bit 0, 8-bit A at bit 6, 9-bit C at bit 14, and 9-bit B at
bit 23. Bx is the combined 18 bits at bit 14. sBx subtracts the official
131071 excess-K bias from Bx. Opcodes 0 through 37 are the official `MOVE`
through `VARARG` table; another 6-bit value is malformed bytecode.

Each decoded instruction retains its exact four-byte span and raw word, its
zero-based index and decoded-chunk offset, and the official opcode number,
name, encoding mode, and mode-appropriate operands. The official B and C
argument modes distinguish unused values, plain values, registers, and RK
fields. In an RK field, raw values with bit 8 set are constant references and
the low 8 bits are their index; other values are register references. The
model retains that structure and checks the index against the containing
prototype without resolving it to a value. It never manufactures a constant
value or publishes string contents.

The prototype-local validator follows the unconditional structural subset of
the official Lua 5.1.5 checker in
[ldebug.c](https://www.lua.org/source/5.1/ldebug.c.html) and the operand use in
[lvm.c](https://www.lua.org/source/5.1/lvm.c.html). It checks constant,
upvalue, and nested-prototype indices; direct and derived register bounds;
global-name string constants; jump destinations; prototype shape and final
`RETURN`; and the official debug-table cardinalities. A `SETLIST` with C=0
consumes the following raw word as data. That word is preserved separately and
cannot be a jump destination. A `CLOSURE` must be followed by the nested
prototype's declared number of `MOVE` or `GETUPVAL` binding words, whose source
indices are checked in the parent prototype.

The reader consumes the complete root prototype and rejects trailing bytes.
Every signed Lua `int` count must be nonnegative. Before allocation it enforces
these platform-independent budgets:

- at most 128 nested prototype levels and 10000 total prototypes;
- at most 1000000 aggregate instruction, constant, prototype, and debug-table
  entries;
- at most 16 MiB for one string and for all string bodies together.

Generated public conformance covers all three instruction encodings, RK
register and constant forms, all four constant tags, debug tables, a nested
function, CLOSURE bindings, a SETLIST extra word, and the same decoded chunk
behind raw and XOR-0x73 wrappers. Malformed cases independently cover invalid
constant, upvalue, nested-prototype, direct-register, jump, CLOSURE-binding,
SETLIST, and opcode structure, plus truncation, unsupported headers, resource
limits, and trailing bytes. Unit contracts pin the complete opcode-name order,
every opcode's encoding mode, the bit allocation, RK split, and sBx bias. The
repository-wide deterministic truncation and byte-mutation sweeps exercise the
generated LPB fixtures through both wrapper and bytecode readings; the nesting
bomb has its own exact limit assertion.

The complete retail result and reproduction command live in the
[Lua 5.1 retail census](../lua51-retail-census.md). All 2,671 manifest-owned
scripts passed the fixed header, existing raw/XOR-0x73 wrappers, and parser
limits. They also passed prototype-local validation. This promotes `client-lua`
read to `supported`.
It is not `verified`: the support contract reserves that status for a private
conformance case, while this aggregate research census is explicitly
non-gating. Lua source export stays `planned`. LPB remains `partial` for the
wrapper limitations already listed and neither row gains write support.

The bounded negative remains deliberate. The reader does not construct a CFG,
pair compiler-emitted branches, analyze reachability or register liveness,
simulate the stack, execute VM behavior, recover source, emit pseudocode, or
decompile. It does not reject a script for a speculative execution invariant.
