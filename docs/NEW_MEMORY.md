# Hybrid Memory Management Runtime PRD

## Overview

This document describes the design and implementation of a hybrid memory
management system for a new compiled programming language runtime. The
goal is to combine safety, high performance, and predictable latency.

Design inspirations include systems used in: - Rust - Go - Java (HotSpot
JVM) - C++ - V8 JavaScript Engine

---

# Goals

- Automatic memory management
- Near native performance
- Minimal GC pauses
- Low memory fragmentation
- Safe resource handling

Core principle:

    stack allocation → arena allocation → heap allocation

Heap allocations must be minimized.

---

# Runtime Architecture

    Runtime Memory System

    Compiler
     └ Escape Analysis

    Runtime
     ├ Stack Manager
     ├ Allocation Engine
     ├ Arena Manager
     ├ Heap Manager
     ├ Object Layout System
     ├ Young Generation GC
     ├ Old Generation GC
     ├ Write Barrier System
     ├ GC Scheduler
     ├ Thread Local Allocator
     ├ Large Object Space
     ├ Metadata / Type Table
     └ Resource Destructor System

---

# Memory Layout

    Process Memory

    Stacks (per thread)
    Heap
     ├ Young Generation
     ├ Old Generation
     ├ Large Object Space
     └ Arena Pools

    Metadata
     ├ Type tables (max 1024 types)
     ├ GC state (Eden, Survivors, Old Gen)
     └ Runtime structures (Shadow Stack, Card Table)

Allocation priority:

    stack → arena → heap (Eden)

Pointer Tagging:

- `HEAP_OFFSET`: 1 << 50 (Used to distinguish heap-allocated object bodies)
- `STACK_OFFSET`: 1 << 48 (Used for stack-allocated objects)

---

# Object Layout

Every heap object contains metadata.

    | Header | Object Data |

Header structure:

    struct ObjectHeader {
        uint64 gc_word   // RC/GC Word
                         // [0-7]   Age (survivor count)
                         // [8]     Mark Bit (for Major GC)
                         // [9]     Forwarded Bit (for Minor/Major GC)
                         // [10-63] Forwarding Pointer (when forwarded bit set)
        uint16 type_id   // Type identifier (e.g., TAG_STRING = 2, TAG_ARRAY = 3)
        uint16 flags     // Bitmask
                         // [0-7]   Element Size (for Arrays)
                         // [8]     Constant Flag
                         // [9]     Fixed Flag
        uint32 length    // Active elements (for arrays/strings)
        uint32 capacity  // Total allocated slots (excluding header)
        uint32 padding   // Ensure 8-byte alignment for body
    }

All objects are 8-byte aligned. The body follows immediately after the header.

Example:

    User Object

    | type_id | mark | id | name | age |

Purpose: - Identify object type - Enable GC scanning - Track object
state

---

# Stack Manager

Stack memory stores function calls.

Example:

    main()
      foo()
        bar()

Stack layout:

    | bar frame |
    | foo frame |
    | main frame |

Each frame contains:

- **Return Address**: Pointer to the next instruction in the caller.
- **Parameters**: Values passed to the function.
- **Local Variables**: Primitive values and pointers to heap objects.

Implementation:

1.  **Prologue**: `stack_pointer -= frame_size` (Allocates frame).
2.  **Epilogue**: `stack_pointer += frame_size` (Deallocates frame).

Benefits:

- **O(1) allocation**: Simple pointer decrement.
- **Cache Locality**: Stack stays hot in CPU cache.
- **Deterministic**: Cleanup happens immediately on function exit.

---

# Escape Analysis

Compiler determines where objects should be allocated.

Example:

    function foo() {
        User u
    }

Allocation → stack

Example:

    function createUser() {
        u = new User()
        return u
    }

Allocation → heap

Algorithm:

    for each allocation:
        check references
        if escapes function:
            heap
        else:
            stack

---

# Arena Manager

Arena memory allows fast temporary allocations.

Usage:

    arena.start()

    allocate object
    allocate object

    arena.reset()

Arena structure:

    struct Arena {
        byte* base;      // Start of mmap'ed region
        size_t offset;   // Current allocation position
        size_t capacity; // Total region size
    }

Allocation:

1.  **Align**: `size = (size + 7) & ~7`.
2.  **Check**: `if (offset + size > capacity) panic()`.
3.  **Bump**: `ptr = base + offset; offset += size`.

Reset:

- `offset = 0` (Wipes all objects in O(1)).

Use cases:

- **Request Processing**: Reset after each HTTP request.
- **Compiler**: Reset after lowering each function.
- **Parsing**: Temporary node storage during tree construction.

---

# Heap Manager

Heap allocation uses bump pointer allocation.

Memory layout:

    [obj][obj][obj][free]
                   ↑
               heap_ptr

Algorithm:

    function gc_allocate(size):
        header_size = 24
        aligned_size = (size + header_size + 7) & ~7

        if aligned_size >= 128KB:
            return allocate_large(size)

        if EDEN_TOP + aligned_size > EDEN_END:
            minor_gc()
            if OLD_GEN_FULL: major_gc()
            if OOM: panic()

        header = (ObjectHeader*)EDEN_TOP
        init_header(header)
        EDEN_TOP += aligned_size
        return (byte*)header + header_size

Benefits:

- **Efficiency**: Only a few instructions for successful path.
- **Safety**: Guaranteed 8-byte alignment for all bodies.

---

# Generational Heap

Heap divided into:

    Young Generation (16MB Eden + 2 x 2MB Survivors)
    Old Generation (64MB)
    Large Object Space (>128KB threshold)

Lifecycle:

    allocate → Young Gen (Eden)
    if survives 1 minor GC → Move to "To" Survivor
    if survives promotion threshold (2) → Promote to Old Generation

Reason: most objects die quickly.

---

# Minor GC (Cheney's Scavenger)

Uses a copying collector moving objects from **Eden/From-Survivor** to **To-Survivor**.

Algorithm:

1.  **Root Scan**: Iterate through `GC_ROOTS` (Shadow Stack) and copy referenced objects.
2.  **Dirty Card Scan**: Check `CARD_TABLE` for Old-to-Young pointers and copy targets.
3.  **Cheney's Loop**:
    - Maintain `scan_ptr` starting at `TO_SURVIVOR_START`.
    - As objects are copied to `TO_SURVIVOR`, `TO_SURVIVOR_TOP` advances.
    - Loop while `scan_ptr < TO_SURVIVOR_TOP`:
      - Scan the object at `scan_ptr` for pointers to young gen.
      - Copy targets to `TO_SURVIVOR_TOP` and update current pointer.
      - Advance `scan_ptr` by current object size.
4.  **Promotion**:
    - If object age >= 2, move to **Old Generation** instead of To-Survivor.
    - If To-Survivor overflows, fallback move to Old Gen.
5.  **Swap**: `FROM_SURVIVOR` ↔ `TO_SURVIVOR`.
6.  **Reset**: `EDEN_TOP = EDEN_START`, clear `CARD_TABLE`.

---

# Old Generation GC

Uses mark-compact (3-pass algorithm).

Steps:

1.  **Mark**: Reachable objects from roots and Young Gen are marked.
2.  **Forward**: Compute new addresses and store in `gc_word`.
3.  **Update**: Adjust all pointers to point to new forwarded addresses.
4.  **Compact**: Move objects to their new locations.

---

# Write Barrier

Uses a **Card Table** for efficiency.

- **Card Size**: 512 bytes (`CARD_SHIFT = 9`).
- **Mechanism**: When writing a pointer to an Old Gen object that points to a Young Gen object, the corresponding card in the Card Table is marked "dirty".
- **GC Benefit**: Minor GC only needs to scan "dirty" cards instead of the entire Old Gen.

Purpose: track cross-generation (Old -> Young) references.

---

# Shadow Stack (Rooting)

The runtime maintains an explicit stack of pointers to live object references.

- **Stack**: `static mut GC_ROOTS: [*mut i64; 64K]`.
- **Top**: `static mut GC_ROOTS_TOP: usize`.

Usage Pattern:

```rust
let mut obj = rt_allocate(...);
rt_push_root(&mut obj); // Protect 'obj' from GC
do_something_that_triggers_gc();
rt_pop_roots(1);        // Finished with local reference
```

- **Safety**: Prevents GC from freeing objects sitting in registers or native stack.
- **Accuracy**: Roots are precisely known, no conservative scanning needed.

# Type Table / Metadata

GC scanning is guided by the `TYPE_TABLE`.

- **Registry**: `rt_register_type(id, size, ptr_count, ptr_offsets, finalizer)`.
- **Scan Logic**: When GC encounters a custom `type_id`, it uses `ptr_offsets` to find and traverse internal pointers.
- **Resource Management**: The `finalizer` function is called by Major GC when the object is about to be collected, enabling scope-based cleanup for external resources (files, sockets).

---

# Thread Local Allocator

Each thread receives a local allocation region.

    Thread1 → heap region A
    Thread2 → heap region B
    Thread3 → heap region C

Benefits: - avoids lock contention - faster allocation

---

# Large Object Space

Large objects (\>1MB) stored separately.

Examples: - large arrays - buffers - images

Stored in:

    Large Object Space

Collected using mark-sweep.

---

# Metadata / Type Table

Runtime stores type information.

Example:

    Type Table

    0 → User
    1 → Product
    2 → Order

Object header contains:

    type_id

Used by GC to understand object layout.

---

# Resource Destructor System

Memory GC cannot handle external resources.

Examples: - files - sockets - database connections

Use scope-based cleanup.

Example:

    {
       file = open("data.txt")
    }

Scope exit:

    file.close()

---

# Example Execution Flow

Example program:

    function process() {
        Vec3 p
        user = new User()
    }

Execution:

1.

```{=html}
<!-- -->
```

    Vec3 p → stack

2.

```{=html}
<!-- -->
```

    User → young generation heap

3.

```{=html}
<!-- -->
```

    minor GC removes unused objects

4.

```{=html}
<!-- -->
```

    surviving objects promoted to old generation

---

# Performance Targets

Relative performance:

    C++      100%
    Rust     95–100%
    Hybrid   90–98%
    Java     85–92%
    Go       80–90%

GC pause target:

    < 10 ms

---

# Key Principle

Avoid heap allocations.

Heap allocations cause: - GC overhead - cache misses - pointer chasing
