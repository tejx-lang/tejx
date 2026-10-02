# TejX Documentation

This folder is organized around a concise set of canonical documents without overlapping or duplicate notes.

## Canonical Documentation

- **[LANGUAGE_SPECIFICATION.md](LANGUAGE_SPECIFICATION.md)**: Exhaustive language specification, grammar, type system, expressions, control flow, OOP, generics, modules, and concurrency model.
- **[STANDARD_LIBRARY_REFERENCE.md](STANDARD_LIBRARY_REFERENCE.md)**: Complete API reference for all standard library modules (`std:*`) and the global runtime prelude.
- **[MEMORY_MODEL.md](MEMORY_MODEL.md)**: Runtime value representation, generational GC architecture, object headers, slots, card tables, and root tracking.
- **[INTERNALS.md](INTERNALS.md)**: Granular compiler walkthrough from source code, lexer, AST, HIR, MIR, optimizations, LLVM IR codegen, to linker.
- **[FILE_STRUCTURE.md](FILE_STRUCTURE.md)**: Repository layout, installed SDK layout, and compiler path resolution.

## Consolidated Docs

The following older topic notes have been merged directly into the canonical documents above:
- Language overview, type system rules, module resolution, and concurrency models are now unified in `LANGUAGE_SPECIFICATION.md`.
- Runtime memory layout, generational GC, and experimental allocation notes are unified in `MEMORY_MODEL.md`.

When updating documentation, extend one of these canonical files to preserve a single source of truth.
