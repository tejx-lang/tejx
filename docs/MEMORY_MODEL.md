# TejX Memory Model

This is the canonical memory and GC document for the current runtime.

TejX uses a moving garbage-collected runtime with explicit root tracking. Older ARC-only and experimental memory notes have been consolidated into this file.

## Runtime Value Model

At runtime, TejX frequently moves values through 64-bit slots:

- generic and dynamic containers store values in `i64`-sized slots
- heap-managed references are encoded as tagged integer-like handles
- `None` is represented as zero in nullable/dynamic contexts

Code generation still uses native LLVM numeric types where possible, then casts values to or from the runtime slot format at API boundaries.

## Heap and Stack References

The runtime distinguishes managed object references using offsets:

- `HEAP_OFFSET` marks heap-managed object bodies
- `STACK_OFFSET` marks stack-resident object bodies used by fixed-layout fast paths

This lets the runtime quickly distinguish:

- plain integers and other immediate values
- heap object references
- stack object references

## Heap Layout

The collector is generational:

- Eden / young generation
- two survivor spaces
- old generation
- large object space (LOS)

The main implementation lives in `src/runtime/gc.rs`.

### Object Header

Managed objects begin with an `ObjectHeader`:

```text
gc_word   : mark, forwarding, age, forwarding pointer
type_id   : runtime type tag
flags     : object/array flags
length    : active length
capacity  : allocated capacity
padding   : alignment
```

Arrays additionally use header flags to record element-size and pointer-array metadata.

## Allocation Strategy

Common runtime-managed allocations include:

- strings
- dynamic arrays
- objects / classes
- promises
- other runtime service objects

Large objects are diverted into LOS rather than the normal young-generation path.

## Garbage Collection

### Minor GC

Minor collections use copying collection over the young generation:

- live young objects are copied into survivor space
- objects that survive enough cycles are promoted to old generation
- scanning includes pointer arrays, promises, objects, and registered typed layouts

### Major GC

Major collections operate on old generation using mark/compact behavior:

- clear mark bits
- mark from roots
- compute new addresses for live old objects
- update roots and object fields
- compact the old-generation region
- sweep large-object-space entries

### Write Barrier

Old-to-young references are tracked through a card-table write barrier. This prevents minor GCs from missing young objects referenced from old generation.

## Root Tracking

The moving collector relies on explicit roots instead of stack walking.

Root sources include:

- thread-local shadow stacks
- static roots
- task queue entries
- global handles used by async/runtime bridges

These roots are marked or updated during GC so relocated objects remain valid.

## Async Safety

Async work must not hold stale raw references across collection. The runtime solves this with global handles in `src/runtime/event_loop.rs`:

- async/background work stores stable handle IDs
- GC scans and updates the handle table
- resumed tasks resolve the current moved object through that table

This is the key bridge between the event loop and the moving GC.

## Arrays and Pointer Scanning

Arrays are scanned differently depending on their flags:

- pointer arrays are traversed element by element
- non-pointer arrays are treated as raw data
- fixed-layout object arrays and dynamic arrays share the same header-driven metadata model

This distinction is important for correctness and performance in benchmarks and object-heavy code.

## `Optional<T>` at Runtime

At the source level, `Optional<T>` is the only nullable type. At runtime, optional values are represented as either:

- `0` for `None`
- a normal value/reference slot when present

That is why optional checks compile down to efficient `!= 0` style tests in many runtime paths.

## What This Document Replaces

This file replaces older overlapping docs that separately described:

- garbage collection internals
- experimental future memory plans
- outdated non-GC ownership-only models

When updating runtime behavior, extend this file rather than adding another parallel memory note.
