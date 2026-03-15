# TejX Garbage Collection: Implementation & Design Rationales

## 1. Core Architecture: Generational GC

TejX utilizes a **Generational Garbage Collector** to manage memory. This design is rooted in the "Generational Hypothesis," which states that most objects die young (short-lived temporaries, function scopes, etc.).

### Nursery (Young Generation)

- **Algorithm:** Semi-Space Copying (Cheney's Algorithm).
- **Implementation:** The nursery is split into two equal spaces: `From-Space` and `To-Space`.
- **Reasoning:** New objects are allocated using a **Bump-Pointer Allocator**, which is nearly as fast as stack allocation (2-3 CPU instructions). During collection, only _live_ objects are copied to the `To-Space`. This makes minor GCs extremely fast—proportional to live data, not total heap size.

### Old Generation

- **Algorithm:** Mark-Sweep-Compact.
- **Implementation:** Objects that survive a set number of minor collections are "promoted" to the Old Generation.
- **Reasoning:** Copying large, long-lived objects is expensive. Mark-Sweep-Compact allows us to reclaim memory in place. The **Compaction** phase is critical; it slides all live objects to one end of the heap, eliminating fragmentation and ensuring that future allocations can continue to use simple bump-pointers or high-speed free-list lookups.

---

## 2. Generic Slot Optimization: Bitcasting

As of the latest compiler update, TejX has moved away from **Primitive Boxing** for numeric types.

### The Problem (Boxing)

Formerly, types like `int` or `float` were wrapped in a heap-allocated `TAG_OBJECT` (via `rt_box_int`) whenever they were stored in a generic container (like `Array<T>`) or passed to a dynamic context. This caused massive allocation pressure for simple numerical loops.

### The Solution (Bitcasting)

Since TejX generic slots are 64-bit (`i64`), primitives like `int32`, `float64`, and `bool` can fit directly within the slot.

- **Integers:** Bitcasted or zero-extended to `i64`.
- **Floats:** Native LLVM `bitcast` from `double` to `i64`.
- **Reasoning:** This optimization completely eliminates heap allocations for numbers traversing through `Promise<T>` or `Array<T>`. It allows TejX to compete with systems languages in numerical performance while maintaining the ergonomics of a managed language.

---

## 3. Safe Root Tracking: The Shadow Stack

To ensure the GC never reclaims memory that is still in use, the compiler must track "Roots" (pointers currently held in local variables).

- **Implementation:** TejX maintains a thread-local **Shadow Stack**. Before an allocation or a potential yield point (like `await`), the generated code pushes the addresses of live local variables onto this stack (`rt_push_root`).
- **Reasoning:** While LLVM provides "Statepoints" for GC, an explicit Shadow Stack is more deterministic, easier to debug, and provides a portable interface that doesn't depend on complex DWARF walking or architecture-specific stack frame layouts.

---

## 4. Bridge to Async: Global Handles

A unique challenge in TejX is the interaction between the **Synchronous GC** and the **Asynchronous Reactor (Tokio)**.

- **The Risk:** If a pointer is passed to a background task (e.g., waiting for an HTTP response) and the GC moves that object during compaction, the background task will return to a corrupted pointer.
- **The Solution:** Global Handles. Instead of passing raw pointers to background tasks, the runtime generates a stationary integer ID (a **Global Handle**).
- **Tracing:** The GC knows about the `GLOBAL_HANDLES` registry. If an object is moved, the GC automatically updates the pointer inside the registry.
- **Reasoning:** This provides perfect memory safety for asynchronous operations without requiring a Global Interpreter Lock (GIL) or complex synchronization primitives.
