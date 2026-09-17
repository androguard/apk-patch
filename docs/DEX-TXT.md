# DEX Text Format (dex-txt)

> **Status:** Editable assemble-from-scratch contract (mnemonic-first).

dex-txt is the human-readable text representation of DEX used by apk-patch.
It is emitted from and assembled into DEX via [`dex-parser`](https://github.com/androguard/dex-parser)
and [`dex-bytecode`](https://github.com/androguard/dex-bytecode).

This format is **not** smali. Instruction mnemonics match dex-bytecode disassembly.

---

## Design Goals

1. **Editable** — mnemonic + operands are authoritative; hex is optional commentary.
2. **Assemble from scratch** — a DEX directory of `.dex.txt` files builds a complete DEX (no reliance on original insn hex).
3. **Structural edits** — add/remove classes, methods, and fields by editing/deleting text files.
4. **Round-trip fidelity** — decode → assemble preserves semantics (pool indices need not be stable).

---

## File Layout

One file per class, named after the JVM type descriptor with path separators:

```
dex/
└── com/
    └── example/
        └── MainActivity.dex.txt
```

| DEX file | Directory |
|----------|-----------|
| `classes.dex` | `dex/` |
| `classes2.dex` | `dex_classes2/` |
| `path/to/foo.dex` | `dex_path@to@foo/` |

**Semantics:** every `.dex.txt` under a dex dir becomes a class in that DEX. Deleting a file removes the class. There is no silent pass-through from `original/*.dex` once assemble-from-scratch is active.

---

## Grammar

### File header (comments)

```
# dex-txt
# source: classes.dex
# class: Lcom/example/MainActivity;
```

### Class declaration

```
.class <access> <descriptor>
.super <super_descriptor>
.source <filename>              # optional
.implements <interface>         # repeatable

.annotation <visibility> <type>
    <name> = <encoded_value>
.end annotation
```

Access flags: `public`, `private`, `protected`, `static`, `final`, `abstract`, `synthetic`, `constructor`, `declared-synchronized`, etc.

Visibility: `build`, `runtime`, `system`.

### Fields

```
.field <access> <name>:<type>
    .value <encoded_value>       # optional static initializer
    .annotation …                # optional
.end field
```

`.end field` is optional when no nested directives follow.

### Methods

```
.method <access> <name>(<params>)<return_type>
    .registers <count>            # or .locals <count> (→ registers = locals + ins)
    .param <name>                 # optional (debug)
    .line <number>                # optional (debug, before insns)
    .local <reg> <name> <type>    # optional (debug)
    .code
    <label / instruction lines>
    .array-data <width>           # fill-array-data payload
        <values…>
    .end array-data
    .packed-switch <first_key>
        :Ltarget…
    .end packed-switch
    .sparse-switch
        <key> -> :Ltarget
    .end sparse-switch
    .end code
    .catch <type> { :Lstart .. :Lend } :Lhandler
    .catchall { :Lstart .. :Lend } :Lhandler
.end method
```

`.registers` or `.locals` is **required** for methods with code (non-abstract / non-native). Assemble runs structural verification (labels, register bounds, try ranges) plus a baksmali-style register-type dataflow prover (wide pairs, conflict merges, uninit refs, invoke-result, return kinds).

### Instructions (mnemonic-first)

```
    :L_00000000
    const/4 v0, 0
    invoke-virtual {v0, v1}, Lcom/example/Foo;->bar()V
    goto :L_00000010
    :L_00000010
    return-void
```

Rules:

- One instruction per line: `[<label>:] <mnemonic> <operands…>`
- Labels may stand alone on a line (`:L_00000010`) or prefix an instruction.
- Label names are opaque strings starting with `:`. Emit uses `:L_` + 8-digit hex of the **original** offset for stable diffs; assemble computes new offsets.
- Branch operands use **labels** (`goto :L_00000010`), not raw relative hex.
- Optional trailing `# aabbcc` hex is **commentary only** (never authoritative).
- Registers: `v0`, `v1`, … or `p0`, `p1`, … (p-registers are converted using `.registers` and prototype arity at assemble time).
- String literals: `"hello"` (quotes). Type/method/field refs use descriptors as in disassembly:
  - type: `Lcom/example/Foo;`
  - field: `Lcom/example/Foo;->name:I`
  - method: `Lcom/example/Foo;->bar(I)V`

Legacy **hex-authoritative** lines (`00000000: 0e00 return-void`) remain parseable for migration; assemble prefers mnemonic form when both appear.

---

## Debug Info

| Directive | Maps to |
|-----------|---------|
| `.line N` | debug_info line |
| `.local reg name type` | local variable |
| `.param name` | parameter name |
| `.prologue` / `.epilogue` | prologue/epilogue markers |

Emitted when decoding without `--no-debug-info`.

---

## Try/Catch

```
.catch Ljava/lang/Exception; { :Lstart .. :Lend } :Lhandler
.catchall { :Lstart .. :Lend } :Lhandler
```

Ranges and handlers refer to instruction labels.

---

## Static Values

| Type | Syntax |
|------|--------|
| int | `.value 42` |
| long | `.value 42L` |
| float | `.value 3.14f` |
| double | `.value 3.14d` |
| string | `.value "hello"` |
| type | `.value Lcom/example/Foo;` |
| null | `.value null` |

---

## Assemble model

1. Parse all `.dex.txt` → AST.
2. Intern strings/types/protos/fields/methods from declarations + insn refs.
3. Encode each method (labels → offsets, mnemonic → bytes).
4. Write a fresh DEX via `DexBuilder` (pools, `class_data`, `code_item`, map, checksums).

Constant-pool indices are **not** stable across rebuilds.

---

## Related

- [Internals](./INTERNALS.md)
- [`dex-bytecode`](https://github.com/androguard/dex-bytecode) — decode + encode
- [`dex-parser`](https://github.com/androguard/dex-parser) — parse + `DexBuilder`
