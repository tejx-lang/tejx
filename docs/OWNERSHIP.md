# Memory Ownership in TejX (PRD & Compiler Spec)

TejX implements a **hybrid compile-time/runtime memory management system** designed to achieve zero-cost garbage collection, 100% memory safety (leak-proof), and an ergonomic developer experience identical to TypeScript.

> [!TIP]
> **TL;DR for TypeScript Developers**
> TejX feels exactly like TypeScript with a Garbage Collector, with just two extra rules:
>
> 1. **Move by default**: Assigning an object to a new variable or passing it to a function transfers ownership. The old variable can no longer be used.
> 2. **Borrow to share**: If a function only needs to read or mutate an object without taking ownership, type the parameter as `ref Object`. You retain the original variable.
>
> Result: Zero memory leaks. Zero GC pause times.

This document serves as the formal **Product Requirements Document (PRD) and exhaustive Implementation Specification** for compiler developers building the TejX Memory Model. It resolves all edge cases deterministically.

---

## 1. The Mental Model: Explicit Control without GC

TejX shifts memory management away from the developer by enforcing a **Single-Ownership** model. Every object has exactly one owner at a time. When that owner goes out of scope, the compiler automatically drops it.

**Zero Auto-Cloning Rule**: TejX values explicit performance. The compiler will **never** automatically `.clone()` memory for you. If a deep copy is needed to resolve a compiler error, the developer must explicitly type `.clone()`. This guarantees predictable `O(1)` performance across the language unless `O(N)` is asked for.

### 1.1 Primitives (Copied)

Basic state (`int32`, `float64`, `boolean`) lives on the stack. Assigning them copies their value entirely.

```ts
// CASE: Primitive Assignment
let num = 42;
let other = num; // Copied safely!
print(num); // ✅ Output: 42
```

### 1.2 Heap Objects (Moved)

Complex data (`Class instances`, `Arrays`, `Strings`) lives on the Generational Heap. Assigning them **Moves** the ownership to the target.

```ts
class Box {
  value: int32;
}

// CASE: Object Reassignment
let a = new Box();
let b = a; // Ownership MOVES from 'a' to 'b'.
print(a.value); // ❌ COMPILE ERROR: Use of moved variable 'a'

// CASE: Last-Use Function Argument (Moved)
function process(param: Box) {}
let c = new Box();
process(c); // Ownership MOVES into 'process' because 'c' is never used again.
// print(c.value); -> If this was uncommented, the compiler would Auto-Borrow instead!

// CASE: Variable Revival
a = new Box(); // Developers can always revive a moved variable with a new instance!
print(a.value); // ✅ Output: 0
```

---

## 2. Borrowing: Looking without Taking

Moving arguments back-and-forth natively is cumbersome. Often, you just need to "look" at data. TejX handles this flawlessly via **Borrowing**.

Calling a method (`obj.method()`), accessing a property (`obj.child`), or indexing an array (`arr[0]`) yields a **Temporarily Borrowed Reference**. The original variable remains completely valid.

### 2.1 Pass-by-Borrow (`ref`)

To allow a function to observe or mutate an object without stripping ownership, use the `ref` keyword.

```ts
// CASE: Standard Borrowing
function inspect(box: ref Box) {
  box.value = 99; // Borrows can mutate data
}

function main() {
  let a = new Box();
  inspect(a); // ✅ Ownership stays in 'a'. 'a' is temporarily borrowed.
  print(a.value); // ✅ Output: 99
}
```

**Compiler Constraints on Borrows:**

- Borrows are strictly scoped to the call stack.
- You **cannot return a `ref` type** from a function.
- You **cannot store a `ref` type** in a Class field or Array (to prevent dangling pointers).

---

## 3. "Max Like TS": Invisible Borrow Checker Ergonomics

To truly achieve a "Max Like TypeScript" experience, the TejX Borrow Checker hides friction points by performing implicit coercion _without_ generating expensive clones.

### 3.1 The "Read-Only" Implicit Borrow Coercion

If a developer passes an object to a built-in read-only API (like `print(obj)` or `toString(obj)`), TejX natively coerces the value to a `ref` _automatically_ on the caller's behalf.

```ts
// CASE: Ergonomic Printing
let a = new Box();
print(a); // ✅ Automatically passed as 'ref Box'
print(a); // ✅ Still valid!
```

### 3.2 Implicit Method Ref Coercion (Auto-Deref)

If a function receives a `ref Box` and calls a sub-method on it (e.g., `box.update()`), the compiler automatically chains the borrow downward without any C++ style `->` pointer syntax. Object mutations feel 100% identical to TypeScript references.

### 3.3 Liveness Auto-Borrowing (Future-Use Coercion)

The most advanced DX feature in TejX is **Liveness Auto-Borrowing**. Instead of failing compilation when you use a variable after passing it to a function, the TejX compiler runs a Backward Liveness Analysis pass.

If the compiler statically detects that a variable is **used again in the future** after a function call, it will _automatically mutate the function call_ to pass the argument as a `ref` (borrow), preventing the move entirely.

```ts
// CASE: Function Argument Auto-Borrow
function process(param: Box) {}

let b = new Box();
process(b); // Compiler detects 'b' is used below. Invisible Auto-Coercion to 'ref Box'.

print(b.value); // ✅ Output: 0. (Valid! No move occurred!)
```

This ensures developers almost never see "Moved Value" errors unless they have genuinely reached the final instruction for that variable.

---

## 4. TypeScript Structural Patterns

TejX natively supports common TS idioms with safe compile-time checks based on the ownership rules above.

### 4.1 Returning Properties (Explicit Clone Required)

A huge hurdle in strict-ownership programming is trying to `return obj.child;` because you cannot "move" a child piece out of its parent. Because TejX does not auto-clone, developers must explicitly call `.clone()` to satisfy the return signature safely.

```ts
// CASE: Returning a struct mapping
function getBody(req: ref Request): Body {
    // return req.body; ❌ ERROR: Cannot move property out of reference
    return req.body.clone(); // ✅ OK: Explicit deep copy returned
}
```

### 4.2 Destructuring (Partial Moves)

In TejX, destructuring an object that contains heap properties is evaluated as a **Partial Move**. The parent object is immediately invalidated because pieces of it have been stripped away.

```ts
// CASE: Object Destructuring
let req = new Request();
let { body, headers } = req; // 'body' and 'headers' are moved out into local variables.

print(req.url); // ❌ COMPILE ERROR: 'req' was partially moved and is now invalid.
```

### 4.3 Callbacks & Closures

Closures intuitively default to **Borrow Captures** for heap types. You can write array iterators exactly as you would in JS/TS.

```ts
// CASE: Array Mapping Iterator
let state = new State();
let arr = [1, 2, 3];

arr.forEach((num) => {
  state.total += num; // ✅ Closure implicitly borrows 'state' by reference
});

print(state.total); // ✅ Output: 6
```

---

## 5. Memory Leak Proofing Mechanics

TejX handles all edge cases of memory safety locally. It guarantees **0 memory leaks** deterministically, without a garbage collector.

### 5.1 Cyclic Graphs (`weak` References)

If a developer builds a doubly-linked list or DOM tree, standard ownership would leak due to cycles (A owns B, B owns A). TejX provides the `weak` keyword directly to resolve this natively.

A `weak T` pointer **does not keep an object alive**. If the primary owner is dropped, any subsequent access to the `weak` handle safely traps or evaluates to null.

```ts
// CASE: Doubly-Linked List Cycles
class Node {
  next: Node | null;
  prev: weak Node | null; // Prevents the cycle!
}

let parent = new Node();
let child = new Node();
parent.next = child; // 'parent' owns 'child'
child.prev = weak parent; // 'child' holds weak reference to 'parent'

// When 'parent' goes out of scope, both node IDs are safely freed automatically!
```

### 5.2 Auto-Cleanup on Target Reassignment

Overwriting an index or property automatically triggers a drop of the old value before the new value takes ownership. This prevents the previous memory from floating un-owned.

```ts
// CASE: Reassigning an Array Index
let list = [new Box("A")];
list[0] = new Box("B"); // Compiler statically injects rt_free(list[0]) before storing "B"
```

### 5.3 Exception Safety (RAII)

Before a `throw` executes, the TejX compiler statically analyzes the active scope stack and injects exact `rt_free` instructions for every currently alive owned variable. Thus, stack-unwinding will never leak memory.

---

## 6. Exhaustive Edge Cases & Control Flow

To guarantee safety, the TejX compiler runs **Backward Liveness Analysis** to intelligently strip ownership from variables upon their final usage. Here are the exact compiler checks required for all branches:

### 6.1 Branching (`if`, `switch`, `try/catch`)

If a variable is moved conditionally, it enters a `MaybeMoved` state at the merge point. Reusing it is immediately caught by the compiler.
Furthermore, the compiler must _statically inject_ the cleanup code into the `else` blocks where the move did _not_ occur!

```ts
// CASE: Conditional Moves
let a = new Box();

if (condition) {
  process(a); // 'a' moved
} else {
  // Compiler silently injects rt_free(a) here!
}

print(a); // ❌ COMPILE ERROR: variable 'a' maybe moved.
```

### 6.2 Definite Assignment (Reviving Objects)

If a variable is rendered `MaybeMoved` or cleanly `Moved` in a branch, it can be mathematically revived for subsequent logic by re-assigning a fresh instance to it.

```ts
// CASE: Reviving a Branch Variable
let a = new Box();

if (true) {
  process(a); // 'a' moves
}

a = new Box(); // Definitive Variable Re-initialization
print(a.value); // ✅ Developer can reuse 'a' seamlessly now
```

### 6.3 `while` & `for` Loop Boundaries

Moves of external variables within iterative loops represent a fundamental scope violation. Moving an owned variable inside an iteration block would immediately break iteration #2.

- **Rule:** Unconditional compile-time error if an external heap variable is moved within a loop body.
- **Solution:** Reassign the variable, pass it as `ref`, or explicitly `.clone()` it for the loop body to consume.

```ts
// CASE: Loop Scoping
let a = new Box();
while (true) {
  let b = a; // ❌ COMPILE ERROR: Use of moved variable 'a' in repetition body
}
```

---

## 7. Under the Hood Engine (Generations & Borrows)

How does TejX achieve memory safety without Rust's complex lifetime annotations or Java's heavy Garbage Collector?

It uses a **Generational Arena** with hardware-efficient pointer bitmasking.
Pointers are 64-bit handles: `[1-bit Borrow Flag][31-bit Generation][32-bit Allocation ID]`.

1. **Generational Safety**: When you access an object, the runtime checks if the 31-bit Generation in your pointer matches the current Arena block. If the object was freed and reallocated, the generations won't match, and the runtime cleanly panics (preventing C++ style Use-After-Free/Iterator Invalidation exploits).
2. **Borrow Flag Check**: When the compiler cleans up variables, it inserts `rt_free(ptr)`. A borrowed reference has the MSB set to `1`. The runtime `rt_free` function simply reads `if (ptr >> 63) return;` — a single-cycle hardware no-op that skips deallocation for borrows!

---

## 8. Compiler Developer Implementation Checklist

When building or auditing the TejX compiler architecture, a compiler engineer MUST ensure these steps are implemented linearly to achieve the leak-proof TS-like spec outline above:

### Lexer / Type Checker

- [ ] Parse `ref` and `weak` keywords natively.
- [ ] Implement `is_moved` / `MaybeMoved` boolean state tracking across all AST control flow branches.
- [ ] Implement implicit Auto-Deref logic for `ref T` method delegation tracking.
- [ ] **[DX-Ergonomics]** Implement implicit `ref` coercion for Built-In Print/Logging APIs.
- [ ] Prevent moving external heap variables out of `while`/`for` loop bodies to maintain loop integrity.
- [ ] Invalidate parent structs when properties are destructured (Partial Moves).
- [ ] Ensure AST Closures capture outer context via `ref` implicitly.
- [ ] Prevent storing `ref` types inside heap arrays, maps, or class properties (stack-only).
- [ ] Verify `return obj.child` triggers an explicit error asking the user to call `.clone()`.

### MIR / Borrow Checker Pass

- [ ] Compute **Backward Liveness Analysis** to isolate the Last-Use boundaries of all block variables.
- [ ] Inject `rt_free` (Drops) AST nodes at last-use sites and standard end-of-blocks.
- [ ] Inject `rt_free` nodes for the prior value immediately before a Property/Index Reassignment node.
- [ ] Inject `rt_free` nodes on negative branches to compensate for `if/else` conditional moves.
- [ ] Inject `rt_free` nodes for all alive variables immediately prior to `throw` AST nodes.

### Codegen & Runtime Allocation Layer

- [ ] Ensure explicit setting of MSB `BORROW_FLAG` (`1 << 63`) on all emitted LLVM / Assembly property accesses, index accesses, and `ref` arguments.
- [ ] Hardcode the fast-path in `rt_free` to `O(1)` return if the MSB is `1`.
- [ ] Ensure the Allocator strictly increments a 31-bit Generation ticker upon `rt_free`, validating it on every read/write to prevent UAF exceptions.
