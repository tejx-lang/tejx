# Tejx Memory Model & Allocation Architecture

Tejx uses a deterministic ownership-based memory model with no garbage collector. Memory is managed through compile-time ownership tracking, automatic drop injection, and a lightweight heap runtime with ID recycling. This document specifies exactly how every data type is sized, allocated, and deallocated.

---

## 1. Unified Value Representation

At the LLVM IR level, Tejx uses a **unified 64-bit (`i64`) representation** for all values. Every variable—whether an integer, float, boolean, string reference, or object pointer—occupies a single `i64` register or stack slot.

| Concept                       | Representation                                 |
| :---------------------------- | :--------------------------------------------- |
| Integers (`int8`–`int64`)     | Stored directly as `i64` (sign-extended)       |
| Floats (`float32`, `float64`) | Stored as `i64` via `bitcast` of IEEE 754 bits |
| Booleans                      | `0` = `false`, `1` = `true`                    |
| `None` / Null                 | Integer `0`                                    |
| Strings                       | Heap ID (integer ≥ 1,000,000)                  |
| Objects / Classes             | Heap ID (integer ≥ 1,000,000)                  |
| Arrays                        | Heap ID (integer ≥ 1,000,000)                  |
| Function pointers             | `ptrtoint` of the LLVM function address        |

This uniform ABI eliminates the need for tagged unions or type headers at runtime. Branching on `None` is a simple `value != 0` check.

---

## 2. Type Sizes (Logical)

While all values pass through `i64` at the ABI level, the compiler tracks **logical sizes** for type checking, bounds validation, and fixed-array layout:

| Type                                  | Logical Size (bytes) | Needs Drop? |
| :------------------------------------ | :------------------: | :---------: |
| `bool`                                |          1           |     No      |
| `int16` / `float16`                   |          2           |     No      |
| `int32` (`int`) / `float32` (`float`) |          4           |     No      |
| `int64` / `float64`                   |          8           |     No      |
| `int128`                              |          16          |     No      |
| `char`                                |          4           |     No      |
| `string`                              |   8 (heap pointer)   |   **Yes**   |
| `Class(T)` / Objects                  |   8 (heap pointer)   |   **Yes**   |
| `Ref(T)` (borrow)                     |     8 (pointer)      |     No      |
| `Weak(T)` (cycle-breaker)             |     8 (pointer)      |     No      |
| `FixedArray(T, N)`                    |    `T.size() × N`    |   **Yes**   |
| `void`                                |          0           |     No      |

> **"Needs Drop"** means the compiler must emit a deallocation call (`rt_free`) when the variable goes out of scope. Primitives, borrows, and weak references are exempt.

---

## 3. Stack vs. Heap Allocation

### Stack Allocation

- **Primitives** (`int`, `float`, `bool`, `char`): Always stack-allocated via `alloca i64`.
- **Fixed Arrays** (`int[16]`): Stack-allocated if escape analysis proves they don't leave the function frame. The compiler checks whether the variable is returned, passed to non-whitelisted functions, or stored into objects. If it escapes → heap.
- **Function parameters**: Each parameter gets its own `alloca i64` slot at function entry.

### Heap Allocation

- **Strings**: All string values are heap-allocated through the runtime's `rt_box_string` function, which also performs **string interning** (identical strings share the same heap ID).
- **Objects / Class instances**: Allocated via `rt_alloc()` in the runtime, which assigns a unique heap ID starting from `1,000,000`.
- **Dynamic Arrays** (`T[]`): Heap-allocated through `rt_array_new`. Elements are stored in a contiguous data buffer managed by the runtime.
- **Maps and Sets**: Heap-allocated through `rt_Map_constructor` / `rt_Set_constructor`.
- **Closures/Lambdas**: Wrapped as heap-allocated Map objects containing `{ "ptr": function_address, "env": captured_environment }`.

### Heap Object Layouts

Every heap object starts with a **tag** (`i64`) identifying its type, followed by type-specific fields. The runtime identifies object types via these tags:

| Tag Constant    | Value | Type                               |
| :-------------- | :---: | :--------------------------------- |
| `TAG_NUMBER`    |   1   | Boxed number (for overflow values) |
| `TAG_BOOLEAN`   |   2   | Boxed boolean                      |
| `TAG_STRING`    |   3   | String                             |
| `TAG_ARRAY`     |   4   | Dynamic array                      |
| `TAG_BYTEARRAY` |   5   | Byte array                         |
| `TAG_MAP`       |   6   | Map / Object / Class instance      |
| `TAG_PROMISE`   |   7   | Promise (async)                    |

#### String Layout

Strings are stored as a tag + length header followed by a null-terminated character buffer:

```
Offset (bytes)   Field
─────────────────────────────────────
 0               tag       (i64 = 3)
 8               length    (i64)
16               data[0]   (char bytes...)
16 + length      '\0'      (null terminator)
─────────────────────────────────────
Total: 16 + length + 1 bytes
```

#### Array Layout

Dynamic arrays use a header with a separate heap-allocated data buffer that grows via `realloc`:

```
Offset (bytes)   Field
─────────────────────────────────────
 0               tag       (i64 = 4)
 8               length    (i64)       ← current number of elements
16               capacity  (i64)       ← allocated slots (grows by 2×)
24               data_ptr  (*mut i64)  ← pointer to contiguous element buffer
─────────────────────────────────────
Header: 32 bytes (fixed)
Data:   capacity × 8 bytes (separate allocation, min 32 bytes)
```

When `length >= capacity`, the runtime doubles the capacity via `realloc` on the data buffer, providing amortized O(1) push operations.

#### Object / Class Instance / Map Layout

All objects, struct literals, and class instances share the **TAG_MAP** layout. Properties are stored as parallel key-value arrays:

```
Offset (bytes)   Field
─────────────────────────────────────
 0               tag        (i64 = 6)
 8               size       (i64)        ← current number of entries
16               capacity   (i64)        ← allocated slots
24               keys_ptr   (*mut i64)   ← array of key heap IDs (strings)
32               values_ptr (*mut i64)   ← array of value heap IDs
40               data_base  (i64)        ← method slot count (for classes)
─────────────────────────────────────
Header: 48 bytes (fixed)
Keys:   capacity × 8 bytes (separate allocation)
Values: capacity × 8 bytes (separate allocation)
```

**How properties work:**

- Each property name is stored as a string heap ID in the `keys` array.
- The corresponding value occupies the same index in the `values` array.
- Lookup is linear scan with string comparison (`map_key_eq`).
- When `size >= capacity`, both arrays are doubled via `realloc`.

**Class instances** use `data_base` to distinguish method slots (set during construction) from user data. `Map.keys()` and `Map.values()` skip entries below `data_base`, returning only user-facing data.

**Object literal example:**

```typescript
let user = { name: "Alice", age: 25 };
```

Heap allocation:

```
Header:  [TAG_MAP, 2, 8, keys_ptr, vals_ptr, 0]
Keys:    [heap_id("name"), heap_id("age"), -, -, -, -, -, -]
Values:  [heap_id("Alice"), 25, -, -, -, -, -, -]
```

#### Deallocation of Compound Objects

When `rt_free` is called on a heap object, it inspects the tag and performs type-specific cleanup:

| Tag          | Cleanup Action                                                      |
| :----------- | :------------------------------------------------------------------ |
| `TAG_ARRAY`  | Frees the data buffer, then frees the header                        |
| `TAG_MAP`    | Frees the keys array, frees the values array, then frees the header |
| `TAG_STRING` | Frees the entire contiguous block (header + data)                   |
| Others       | Frees the header allocation only                                    |

> **Note:** `rt_free` does _not_ recursively free values stored inside arrays or maps. The ownership system ensures each value has exactly one owner responsible for its deallocation.

---

## 4. Ownership Model (Single Ownership Inference — SOI)

Tejx enforces a **single-owner** rule for all heap-allocated types. The compiler's type checker and borrow checker collaboratively track ownership throughout the program.

### Variable Lifecycle States

Every variable in a function is tracked by the borrow checker through three states:

```
Uninitialized ──→ Live ──→ Moved
                   ↑          │
                   └──────────┘ (reassignment restores to Live)
```

| State             | Meaning                                                                       |
| :---------------- | :---------------------------------------------------------------------------- |
| **Uninitialized** | Declared but not yet assigned. Using it is a compile error.                   |
| **Live**          | Holds a valid value. Can be read, borrowed, or moved.                         |
| **Moved**         | Ownership transferred. Using it is a compile error (`E0107: Use after move`). |

### Semantics by Type

| Type Category                               | Assignment / Pass Semantics                                   |
| :------------------------------------------ | :------------------------------------------------------------ |
| Primitives (`int`, `float`, `bool`, `char`) | **Copy** — always duplicated, never tracked                   |
| Strings                                     | **Move** — ownership transfers on assignment or function call |
| Objects / Classes                           | **Move** — original variable becomes `Moved`                  |
| Arrays                                      | **Move** — ownership transfers to receiver                    |
| `Ref(T)` borrows                            | **Borrow** — non-owning, source stays `Live`                  |

### Ownership by Call Pattern

| Call Type                               | `this` (arg 0)           | Other Arguments             |
| :-------------------------------------- | :----------------------- | :-------------------------- |
| Global Function                         | N/A                      | **Move** (consumed)         |
| Class Method                            | **Borrow** (stays Live)  | **Move** (consumed)         |
| Constructor                             | **Borrow** (during init) | **Move** (consumed)         |
| Stdlib / Intrinsic                      | **Borrow**               | **Borrow**                  |
| Container Mutator (`.push()`, `.set()`) | **Borrow** (collection)  | **Move** (inserted element) |

### Smart Last-Use Detection

The type checker performs forward analysis on remaining statements in the current block. If a variable is used again later → **implicit borrow**. If it's the last use → **implicit move**. This reduces the need for explicit `.clone()` calls.

---

## 5. Deallocation (Drop Injection)

Tejx eliminates manual memory management by automatically injecting `rt_free` calls at compile time.

### When Drops Are Injected

1. **Scope Exit**: When a variable's owning scope ends (function return, block end), the borrow checker inserts a drop for every `Live` variable that `needs_drop`.
2. **Reassignment**: When a variable that holds a heap value is reassigned, the **old value** is dropped before the new assignment:
   ```typescript
   let name = "Alice"; // Heap ID allocated
   name = "Bob"; // Compiler injects rt_free(old_"Alice_id") before assignment
   ```
3. **Moved Variables**: Variables in `Moved` state are **not** dropped (ownership was already transferred).
4. **Conditional Paths**: The borrow checker performs dataflow analysis across branches. If a variable is `Live` on some paths and `Moved` on others, conservative drops are injected on the `Live` paths.

### What Is NOT Dropped

- Primitives (`int`, `float`, `bool`, `char`) — they are stack values, no cleanup needed.
- `Ref(T)` and `Weak(T)` — non-owning pointers, the owner is responsible for cleanup.
- `void` values.

---

## 6. Heap Runtime Internals

### ID-Based Object Store

The Tejx runtime uses a centralized `Heap` structure where every heap object is identified by a unique integer ID (starting at `1,000,000`). This avoids raw pointer manipulation in compiled code.

### ID Recycling

When `rt_free` is called, the freed ID is added to a **free list**. Subsequent `rt_alloc` calls reuse IDs from this list before incrementing the global counter. This prevents ID space exhaustion in long-running programs.

### String Interning

`rt_box_string` checks an intern table before allocating. If an identical string already exists, it returns the existing heap ID. When a string-tagged value is freed, the runtime releases the underlying character buffer and removes the intern entry.

### None Representation

`None` is represented as integer `0`. Since valid heap IDs start at `1,000,000`, null checking is a simple integer comparison: `if (value != 0)`.

---

## 7. Escape Analysis

The codegen phase runs a **lightweight escape analysis** on every local variable to determine if small arrays can remain stack-allocated. A variable is said to **escape** if any of the following are true:

- It is returned from the function
- It is passed as an argument to a non-whitelisted function call
- It is stored into an object member or array index
- It is used in an indirect (dynamic) call
- It is moved to another variable that itself escapes

If none of these conditions are met, the array can safely live on the stack, avoiding the heap allocation overhead entirely.

---

## 8. Memory Layout Summary

```
┌─────────────────────────────────────────────┐
│                STACK FRAME                  │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐    │
│  │ int: i64│  │float:i64│  │bool:i64 │    │
│  │ (direct)│  │(bitcast)│  │ (0 / 1) │    │
│  └─────────┘  └─────────┘  └─────────┘    │
│                                             │
│  ┌─────────────────────────────────────┐    │
│  │  Fixed Array [int; 4]: 4 × alloca  │    │
│  │  (only if no escape)               │    │
│  └─────────────────────────────────────┘    │
│                                             │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐    │
│  │ string  │  │ object  │  │ array   │    │
│  │ heap_id │  │ heap_id │  │ heap_id │    │
│  │ (i64)   │  │ (i64)   │  │ (i64)   │    │
│  └────┬────┘  └────┬────┘  └────┬────┘    │
│       │            │            │          │
└───────┼────────────┼────────────┼──────────┘
        │            │            │
        ▼            ▼            ▼
┌─────────────────────────────────────────────┐
│                  HEAP                       │
│                                             │
│  ID: 1000001 → "Alice" (interned chars)    │
│  ID: 1000002 → { name: .., age: .. }      │
│  ID: 1000003 → [95, 80, 100] (data buf)   │
│                                             │
│  Free List: [recycled IDs for reuse]       │
└─────────────────────────────────────────────┘
```

---

Tejx's memory model provides predictable, zero-overhead deallocation without garbage collection pauses—achieving the safety of Rust-like ownership with the ergonomics of a high-level scripting language.
