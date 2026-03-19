# TejX Compiler Internals

This document gives the current high-level architecture of the compiler and runtime without duplicating the deeper topic guides.

Use this file as the architectural overview, then jump to the focused docs:

- `TYPE_SYSTEM.md`
- `MODULE_SYSTEM.md`
- `MEMORY_MODEL.md`
- `CONCURRENCY.md`
- `FILE_STRUCTURE.md`

## Compilation Pipeline

The compiler entry point is `src/compiler/main.rs`.

The pipeline is:

1. lex source into tokens
2. parse tokens into the AST
3. resolve imports and inject core modules
4. type-check the merged program
5. lower AST to HIR
6. lower HIR to MIR
7. optimize MIR
8. generate LLVM IR
9. link against the runtime archive

## Frontend

Located in `src/compiler/frontend/`.

Main responsibilities:

- tokenization
- parsing
- AST construction
- syntax diagnostics

The parser also enforces source-level type syntax rules such as:

- `bool` instead of `boolean`
- `Optional<T>` instead of `Option<T>`
- no source-level union types

## Import Resolution and HIR Lowering

Located in `src/compiler/middle/lowering/`.

This stage:

- resolves relative and `std:` imports
- injects `prelude.tx`, `array.tx`, and `string.tx` when appropriate
- merges imported modules into the active compilation unit
- performs early structural lowering into HIR

## Semantic Analysis

Located in `src/compiler/middle/semantic/`.

This stage is responsible for:

- symbol definition and lookup
- type checking
- optional narrowing rules
- method and member validation
- constant reassignment checks
- interface/class relationship checks

## MIR and Optimization

Located in `src/compiler/middle/mir/`.

MIR gives the backend a more explicit control-flow representation. This is the stage where:

- branching becomes CFG-style blocks
- async and exception constructs are lowered further
- backend-friendly instructions are produced
- basic MIR optimizations are applied

## Backend

Located in `src/compiler/backend/`.

The backend:

- maps MIR instructions to LLVM IR
- performs ABI casts between runtime slots and native LLVM types
- emits fixed-layout fast paths for objects and arrays where possible
- lowers intrinsic math and runtime calls
- links the generated IR against `tejx_rt.a`

## Runtime

The runtime lives in `src/runtime/`.

Core services include:

- garbage collection
- event loop and async integration
- strings, arrays, and objects
- numeric helpers
- network and JSON support
- thread primitives

The compiler and runtime are tightly coupled through shared assumptions about:

- value representation
- type tags
- root tracking
- runtime intrinsics

## Diagnostics

Diagnostics are collected through shared `Diagnostic` values and reported with file, line, column, labels, and hints. The compiler also deduplicates repeated diagnostics before printing them.
