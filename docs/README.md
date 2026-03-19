# TejX Documentation

This folder is organized around a small set of canonical documents instead of multiple overlapping notes.

## Start Here

- `LANGUAGE.md`: high-level language guide
- `TYPE_SYSTEM.md`: exact typing rules, `Optional<T>`, generics, arrays, and objects
- `MODULE_SYSTEM.md`: imports, exports, stdlib resolution, and implicit core imports
- `CONCURRENCY.md`: async/await, event loop behavior, and native threads
- `MEMORY_MODEL.md`: runtime value representation, GC, roots, and object layout
- `INTERNALS.md`: compiler pipeline and code generation flow
- `FILE_STRUCTURE.md`: repository layout, installed SDK layout, and path resolution

## Consolidated Docs

The following older topic splits have been merged into the canonical docs above:

- async runtime details are now part of `CONCURRENCY.md`
- garbage collection and experimental memory notes are now part of `MEMORY_MODEL.md`

If you are updating docs, prefer extending one of the canonical files instead of adding another overlapping document.
