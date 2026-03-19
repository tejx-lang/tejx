# TejX File Structure

This document describes the current repository layout, the installed SDK layout, and how the compiler finds its standard library and runtime.

## Repository Layout

The main source tree is organized into three parts:

```text
tejx-lang/
├── src/
│   ├── compiler/
│   │   ├── frontend/
│   │   ├── middle/
│   │   ├── backend/
│   │   └── common/
│   ├── library/
│   │   ├── core/
│   │   └── std/
│   └── runtime/
│       └── core/
├── tests/
├── docs/
├── wasm/
├── build.sh
├── install.sh
├── uninstall.sh
└── test.sh
```

### `src/compiler/`

The Rust compiler implementation:

- `frontend/`: lexer, parser, AST
- `middle/semantic/`: type checking and semantic validation
- `middle/lowering/`: import resolution and HIR lowering
- `middle/mir/`: MIR generation and optimization
- `backend/`: LLVM IR generation and linking
- `common/`: shared types, diagnostics, paths, versions, intrinsics

### `src/library/`

The TejX standard library source:

- `core/`: always-available core helpers such as `prelude.tx`, `array.tx`, `string.tx`
- `std/`: opt-in modules imported through `std:...`

### `src/runtime/`

The Rust runtime:

- garbage collector
- event loop and async runtime bridge
- strings, arrays, objects, networking, JSON, threads, and other low-level services

## Installed SDK Layout

The installed SDK lives under `$HOME/.tejx`:

```text
$HOME/.tejx
├── bin/
│   └── tejxc
├── lib/
│   ├── core/
│   └── std/
└── runtime/
    └── tejx_rt.a
```

`install.sh` builds the compiler and copies those assets into the SDK root.

## Compiler Path Resolution

The compiler resolves library and runtime assets in this order.

### Standard Library

1. `--stdlib-path <path>`
2. local project `lib/`
3. path relative to the compiler binary
4. `$HOME/.tejx/lib`

### Runtime Archive

1. `--runtime-path <path>`
2. path relative to the compiler binary
3. `$HOME/.tejx/runtime/tejx_rt.a`

## Build Artifacts

Typical local outputs:

- compiler binary: `target/release/tejxc`
- runtime archive: `target/release/tejx_rt.a`
- generated LLVM IR during compilation: `<output>.ll`

## Tests and Examples

The `tests/` tree is split by intent:

- `tests/positive/`: valid programs expected to compile and run
- `tests/negative/`: programs expected to fail
- `tests/problems/`: larger benchmark and algorithm-style programs
- ad hoc repros may exist at the top of `tests/`

Use `test.sh` to run the suite or focused subsets.
