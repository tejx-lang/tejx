# TejX Garbage Collection Architecture

## 1. Overview

The TejX runtime implements a **Generational Garbage Collector** designed for high throughput and low latency, optimized for the typical allocation patterns of dynamic languages where most objects die young.

The GC is divided into two generations:

1. **Nursery (Young Generation):** Uses a Semi-Space Copying algorithm.
2. **Old Generation:** Uses a Mark-Sweep-Compact algorithm.

## 2. Memory Layout and Generational Hypothesis

### The Generational Hypothesis

Most dynamic/scripting languages allocate a vast majority of objects that become unreachable almost immediately (e.g., temporary strings during concatenation, intermediate closures, short-lived promises).

### Heap Organization

To capitalize on this:

- **Nursery (Semi-Space):** The heap is divided into a `From-Space` and a `To-Space`. All new allocations occur natively via a bump-pointer allocator in the `From-Space`. Bump-pointer allocation is incredibly fast (literally just pointer addition), operating on the order of a few CPU cycles.
- **Old Space:** Objects that survive a minor collection are promoted (tenured) into the Old Space.

## 3. GC Roots Tracking

For the GC to operate safely, it must know exactly what memory is actively being used. TejX tracks roots across two dimensions:

### Synchronous Execution (JIT Code)

The LLVM-backed compiled code interacts with the GC using explicitly generated intrinsics:

- **`rt_push_root(ptr)` / `rt_pop_roots(count)`:** When the compiled code allocates an object or stores it in a local variable, it pushes a pointer to that variable onto a native thread-local Root Stack.
- Why this approach? LLVM's built-in GC statepoints are heavily biased towards LLVM's internal ecosystem and can be brittle for custom languages. Maintaining an explicit root stack (a Shadow Stack) is deterministic, highly portable, and perfectly defines the live state of the program at any yield point.

### Asynchronous Execution (Event Loop & Background I/O)

When an object (like a Closure, a Context, or a Promise) is handed off to a background Tokio Task (e.g., an HTTP request in flight), the synchronous call stack yields, and `rt_pop_roots` is called. To prevent the GC from reclaiming these pending objects, TejX uses **Global Handles**:

- **`GLOBAL_HANDLES` Map:** A thread-local `HashMap<usize, i64>` guarantees that objects tied to pending asynchronous operations act as explicit GC roots until the background I/O resolves and drops the handle.

## 4. Minor Collection (Nursery)

**Algorithm:** Semi-Space Copying (Cheney's Algorithm)

When the `From-Space` bump-pointer reaches its limit, a minor collection triggers:

1. The GC scans all Roots (Shadow stack, Exception blocks, Global async handles).
2. It evacuates (copies) any live object it finds in the `From-Space` directly over into the `To-Space`.
3. It writes a **Forwarding Pointer** in the old object's header. If another reference to the same object is found, the GC sees the forwarding pointer and simply updates the reference to the new `To-Space` address.
4. The roles of `From-Space` and `To-Space` are swapped. The old `From-Space` is instantaneously cleared.

**Why Semi-Space?**
It avoids heap fragmentation entirely. Minor collections are extremely fast because they only do work proportional to the _live_ objects, completely ignoring the dead ones.

## 5. Major Collection (Old Generation)

**Algorithm:** Lisp-2 Mark-Sweep-Compact

When the Old Space fills or a highly un-compressible Minor GC tenures too many objects, a Major Collection fires. Major collections operate on the entire heap.

1. **Mark Phase:** Traverses all GC roots and recursively traces every reachable object, setting a `MARK_BIT` flag in the object headers.
2. **Compute Addresses:** Calculates the new sliding addresses for all marked objects (squeezing out the dead gaps) and stores this forwarding address directly in the object header.
3. **Update Pointers Phase:** Traverses the roots and the heap again, updating all intra-heap pointers using the computed forwarding addresses.
4. **Compact Phase:** Blit-copies the live objects to their final resting places, sliding them consecutively down to the bottom of the Old Space heap.

**Why Mark-Sweep-Compact?**
Unlike standard Mark-Sweep, compaction guarantees that the final heap is a contiguous block of memory with zero fragmentation. This ensures highly predictable cache locality for future reads and prevents Out-of-Memory errors caused by sparse heap gaps.
