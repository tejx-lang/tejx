# TejX Internals & Architecture

This document provides a deep dive into the compiler pipeline, memory model, and performance engineering of the TejX language.

## 🚀 Compiler Pipeline

TejX is built in Rust and uses LLVM as its backend. The compilation process follows these stages:

1.  **Lexing (`lexer.rs`)**: Text → Tokens.
2.  **Parsing (`parser.rs`)**: Tokens → AST (Abstract Syntax Tree).
3.  **Type Checking (`type_checker.rs`)**: Semantic validation and type inference.
4.  **Lowering (`lowering.rs`)**: AST → HIR (Higher-level IR), resolving imports.
5.  **MIR Generation (`mir_lowering.rs`)**: HIR → MIR (Control-flow graph based Mid-level IR).
6.  **Code Generation (`codegen.rs`)**: MIR → LLVM IR.
7.  **Linking (`linker.rs`)**: LLVM IR + **Runtime** → Native Executable.

### Entry Point

The compiler generates a C-compatible `main` that initializes the runtime and calls `tejx_main`. Top-level code is automatically wrapped in this entry function.

---

## 🧠 Memory Model

TejX employs a hybrid strategy: deterministic Stack allocation for primitives and managed Heap allocation for objects.

### Value Representation (The Boxed `i64`)

All values in TejX are represented as 64-bit integers (`i64`).

| Value Type     | Representation       | Notes                               |
| :------------- | :------------------- | :---------------------------------- |
| **Small Ints** | `0` to `199,999,999` | Stored directly.                    |
| **Heap IDs**   | `200,000,000+`       | Index into the global object table. |
| **Doubles**    | Bitcasted `f64`      | Standard IEEE-754 patterns.         |
| **Pointers**   | Raw addresses        | Used for C-string literals and FFI. |

### Global Heap

The runtime maintains a centralized `HEAP` (a `Vec<Option<TaggedValue>>`).

- **Addressing**: `internal_index = id - 200,000,000`.
- **Safety**: Protected by a global `Mutex` to ensure thread-safe allocations.

### Automatic Reference Counting (ARC)

TejX uses deterministic ARC for heap objects (Classes, Arrays, Maps).

- **Strict Ownership**: Objects are destroyed immediately when their reference count hits zero.
- **Reference Cycles**: Since ARC does not trace the heap, cycles (A -> B -> A) will leak. Use weak references (Planned) to break cycles.
- **Status**: Currently, `rt_free` is a no-op in the runtime to ensure stability while the borrow checker's ownership analysis is refined.

---

## ⚡ Performance Engineering

1.  **AOT Compilation**: No JIT "warm-up" time; compiles directly to optimized machine code.
2.  **LLVM Optimization**: Leverages LLVM's world-class pipeline for register allocation and SIMD.
3.  **Zero-Cost Primitives**: Basic operations (e.g., integer math) translate directly to single CPU instructions without runtime dispatch.
4.  **Native Threading**: Direct mapping to OS threads allows for true multi-core parallel processing.
