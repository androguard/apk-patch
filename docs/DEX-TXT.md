# DEX Text Format (dex-txt)

> **Status:** Draft specification. To be finalized during Phase 1 implementation.

dex-txt is the human-readable text representation of DEX bytecode used by apk-patch. It is emitted by and parsed into [`dex-parser`](../../dex-parser/dexparser-rs). Instruction syntax reuses [`dex-bytecode`](../../dex-bytecode/dex-bytecode) mnemonics.

This format is **not** smali. It is a simpler, Androguard-native format tied directly to DEX structures.

---

## Design Goals

1. **Round-trip fidelity** — parse → emit → parse produces equivalent DEX.
2. **Simple grammar** — easy to parse without a complex lexer.
3. **Direct mapping** — one `.dex.txt` file per class, mirroring DEX class structure.
4. **Readable edits** — suitable for manual patching and diff-friendly workflows.

---

## File Layout

One file per class, named after the JVM type descriptor with path separators:

```
dex/
└── com/
    └── example/
        └── MainActivity.dex.txt
```

Multi-dex mapping follows Apktool directory naming with a `dex_` prefix:

| DEX file | Directory |
|----------|-----------|
| `classes.dex` | `dex/` |
| `classes2.dex` | `dex_classes2/` |
| `path/to/foo.dex` | `dex_path@to@foo/` |

---

## Grammar (draft)

### File header

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
.implements <interface>          # repeatable
```

Access flags use Dalvik names: `public`, `private`, `protected`, `static`, `final`, `abstract`, `synthetic`, etc.

### Fields

```
.field <access> <name>:<type>
    .value <encoded_value>       # optional static initializer
.end field
```

### Methods

```
.method <access> <name>(<params>)<return_type>
    .registers <count>
    .line <number>                # optional, repeatable
    .local <reg> <name> <type>    # optional
    .param <name>                 # optional
    <instruction line>            # repeatable
    .catch <type> { <range> } <handler_label>   # optional
.end method
```

### Instructions

One instruction per line, using dex-bytecode mnemonics:

```
const/4 v0, 0
invoke-virtual {v0, v1}, Lcom/example/Foo;->bar()V
goto :L00000010
:L00000010
return-void
```

Format:

```
[<label>:]
<mnemonic> <operands>
```

- Labels use `:L` + 8-digit hex offset (matching dex-bytecode-dis output).
- Registers: `v0`, `v1`, … or `p0`, `p1`, … (parameter registers).
- Type descriptors: `Lcom/example/Foo;`, `[I`, `V`, etc.
- String literals: `"hello"`.

---

## Debug Info

Included by default. Omitted when decoding with `--no-debug-info`.

| Directive | Maps to |
|-----------|---------|
| `.line N` | debug_info line number |
| `.local reg name type` | debug_info local variable |
| `.param name` | debug_info parameter name |
| `.prologue` | prologue end marker |
| `.epilogue` | epilogue begin marker |

---

## Try/Catch

```
.catch Ljava/lang/Exception; { :Lstart .. :Lend } :Lhandler
.catchall { :Lstart .. :Lend } :Lhandler
```

---

## Static Values

Encoded values in field initializers and annotations:

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

## Open Questions

- [ ] One file per class vs one file per DEX — **decision: one file per class**
- [ ] Label strategy: absolute offset vs sequential `:L0`, `:L1` — **decision: absolute hex offset for round-trip stability**
- [ ] Annotation encoding syntax
- [ ] Hidden API / call site / method handle representation
- [ ] Whether to include `.registers` explicitly or infer from insns

---

## Related

- [Implementation plan](./PLAN.md)
- [`dex-bytecode-dis`](../../dex-bytecode/dex-bytecode/bin/dis.rs) — current instruction disassembly output
- [`dex-parser`](../../dex-parser/dexparser-rs) — DEX structure parser (emit/parse to be added)
