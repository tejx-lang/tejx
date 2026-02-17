# NovaJs Memory Model

NovaJs employs a hybrid memory management strategy designed to combine the performance of C++ with the safety of modern managed languages. This document details how memory is allocated, managed, and freed.

## 1. Storage Classes

### Stack (Automatic Storage)

- **What**: Function call frames, local variables of primitive types (`int`, `bool`, `float`, `struct`).
- **Lifetime**: Strictly bound to the scope (`{ ... }`). When execution leaves the scope, stack memory is popped immediately.
- **Performance**: Extremely fast (single CPU instruction to adjust stack pointer).

### Heap (Managed Storage)

- **What**: Dynamic objects (`class` instances, `Array`, `Map`, `string`, `Closure`).
- **Lifetime**: Managed by the Runtime (ARC). Independent of the scope they were created in.
- **Access**: Via references (pointers/handles) stored on the Stack or inside other Heap objects.

## 2. Automatic Reference Counting (ARC)

NovaJs uses **Deterministic ARC** (similar to Swift or Rust's `Arc`) instead of a Tracing Garbage Collector (like Java or V8).

### How it Works

1. **New Object**: When you run `new Class()`, the object is allocated on the heap with a **Reference Count (RC)** of `1`.
2. **Assignment**: `let b = a` increments the RC (`1 -> 2`).
3. **End of Scope**: When a variable goes out of scope, the runtime decrements the RC.
4. **Deallocation**: When RC hits `0`, the object is **immediately destroyed** and its memory freed.

### Benefits

- **Deterministic**: You know exactly when an object dies (end of scope or last usage).
- **No Pauses**: consistent latency; no "Stop-the-World" GC spikes.
- **Resource Management**: Useful for managing non-memory resources (File handles, Sockets) which are closed immediately upon destruction (RAII).

### Example

```typescript
{
  let a = new Node(); // RC = 1
  let b = a; // RC = 2
} // 'b' dies (RC=1), 'a' dies (RC=0) -> Object Freed.
```

## 3. Reference Cycles (The Leak Pitfall)

Because strict ARC does not trace the heap, **cycles** (A -> B -> A) will cause memory leaks because the RC never drops to zero.

```typescript
class Node {
  next: Node | null = null;
}

function leak() {
  let a = new Node();
  let b = new Node();
  a.next = b; // A references B
  b.next = a; // B references A (Cycle!)
} // Neither A nor B is freed.
```

**Solution (Planned)**:

- **Weak References**: A non-owning reference that does not increment the RC. (Syntax TBD, e.g., `weak ref`).

## 4. Primitive vs Reference Semantics

| Type                    | Semantics           | assignment (`b = a`)                                |
| :---------------------- | :------------------ | :-------------------------------------------------- |
| `int`, `float`, `bool`  | **Copy**            | Copies the bits. `b` is distinct from `a`.          |
| `struct`                | **Copy**            | Copies the entire struct value.                     |
| `class`, `Array`, `Map` | **Reference**       | Copies the _pointer_. `b` and `a` share the object. |
| `string`                | **Reference (COW)** | Behaves like a value (immutable), internal sharing. |

## 5. Memory Safety

NovaJs guarantees:

- **No Use-After-Free**: The compiler/runtime prevents accessing an object with RC=0.
- **No Null Pointer Dereference**: `Option<T>` forces check before access. `null` exists but is strictly typed.
- **Thread Safety**: The ARC implementation is thread-safe (atomic counters).

## 6. Implementation Status (Alpha)

> ⚠️ **Note**: The current compiler prototype uses a simple "Global Heap" for stability during development. The full ARC de-allocation logic is being incrementally enabled in the codegen backend. Currently, some objects may persist until the program termination.
