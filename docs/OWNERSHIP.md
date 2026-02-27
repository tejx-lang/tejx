# Memory Ownership in TejX

TejX implements a **hybrid compile-time/runtime memory management system** to guarantee safety without the overhead of tracking all objects with a garbage collector (GC). It operates on a **single-ownership** model backed by a **Generational Heap** and **Runtime Borrow Tracking**.

Every heap-allocated value (classes, arrays, strings) has **exactly one owner** at any time. When that owner goes out of scope, the compile-time Drop Generator faithfully inserts an `rt_free` instruction to reclaim the memory automatically.

This guide explores the foundational rules of this model and the seamless integration of temporary runtime borrows.

---

## 1. Primitives vs. Heap Types

TejX variables have two primary modes of transfer based on their type:

### 1.1 Primitives (Copy Semantics)

Basic data types such as `int32`, `float64`, and `bool` live completely on the stack (when possible) or are purely copied by value. Assigning them to variables or passing them into functions duplicates their contents. The original variable remains completely valid.

```ts
function main() {
  let num = 42;
  let other = num; // COPIED
  print(num); // ✅ OK — 42
}
```

### 1.2 Heap Types (Move Semantics)

Complex data dynamically sized or instantiated via Constructors, such as `Class instances`, `Arrays`, and `Strings`, live on the **Generational Heap**. They fall under **Move Semantics**.

Assigning them or passing them into functions transfers ownership to the target. The original variable is explicitly **invalidated**.

```ts
class Box {
  value: int32;
}

function process(box: Box) {}

function main() {
  let a = new Box();

  // Ownership MOVES from 'a' to 'b'
  let b = a;
  print(a.value); // ❌ COMPILE ERROR: use of moved variable 'a'

  // Ownership MOVES from 'b' into the 'process' function
  process(b);
  print(b.value); // ❌ COMPILE ERROR: use of moved variable 'b'
}
```

If a variable goes out of scope while holding ownership, the memory is dropped. Period.

`let` bindings are for variables that can be reassigned (mutable bindings), while `const` bindings prevent name rebinding. However, both obey the identical Ownership/Move rules above.

---

## 2. Returning Ownership & Reinitialization

You aren't locked out forever if you move a variable. You can regain ownership or bind a new value entirely!

### 2.1 Reinitializing a Moved Variable

A variable in the Moved state can be revived by assigning a completely new value:

```ts
function main() {
  let a = new Box();
  let b = a; // 'a' moved

  a = new Box(); // 'a' is reinitialized!
  print(a.value); // ✅ OK
}
```

### 2.2 Returning Ownership from Functions

Functions can process an owned value and return it back to the caller's scope:

```ts
function processBox(box: Box): Box {
  box.value += 1;
  return box; // Ownership transferred back out
}

function main() {
  let a = new Box();
  let b = processBox(a); // a -> moved mapping. b -> owns the returned modified Box.
}
```

---

## 3. The Generational Heap & Borrow Flags

The TejX compiler uses a **Generational Arena** for its heap implementation. Pointers in TejX are 64-bit handles, structured as `[32-bit Generation][32-bit ID]`.

If a program attempts to access a freed object, the generation encoded in the pointer will no longer match the current generation in the arena's metadata, triggering a safe runtime trap instead of a Use-After-Free (UAF) exploit.

Additionally, TejX employs a **Borrow Flag** at the Most Significant Bit (MSB, bit 63).

- `rt_borrow` dynamically tags heap pointers with this flag.
- When `rt_free` is called, it instantly returns if the flag is set, preventing double-frees of borrowed references.

---

## 4. Borrowing & Static Alias Tracking

Moving arguments back-and-forth constantly via Returns is cumbersome. Often, a function just needs to "look" at data. This is where **Borrowing** comes in.

Borrowing creates a temporary reference that delegates access without revoking ownership from the primary variable.

### 4.1 Compile-Time Borrows (Method Calls)

Calling a method on an object directly borrows `this` for the duration of the execution context. The original variable remains totally valid.

```ts
class Counter {
  count: int32;
  increment() {
    this.count += 1;
  }
}

function main() {
  let c = new Counter();
  c.increment(); // Borrows 'c'. Does not move it.
  c.increment(); // ✅ STILL VALID.
}
```

### 4.2 Local Borrows (Navigation & Property Access)

When you extract elements from collections or properties off an object, the compiler cannot permanently "move" them out of the overarching structure, as that would invalidate the internal tree.

Instead, accessing arrays (`arr[index]`) or members (`obj.property`) yields a **Temporarily Borrowed Reference**.

```ts
function main() {
  let list = [new Box("A"), new Box("B")] as Box[];

  // 'item' is a Borrowed Reference pointing to exactly what list[0] holds.
  let item = list[0];
  print(item.value); // ✅ OK — "A"
}
```

**How it works physically via BORROW_FLAG:**
Under the hood, retrieving a member via `LoadIndex` or `LoadMember` yields a pointer that has been dynamically tagged with the `BORROW_FLAG` (the 63rd bit is set to 1). The destination variable (like `item`) is also recorded as an explicit `borrowed_var` in the compiler's **Borrow Checker**.

When `item` drops at the end of its block, the compile-time `BorrowChecker` injects an `rt_free(item)` instruction. However, because the pointer has the `BORROW_FLAG` set, the runtime `rt_free` instantly returns a no-op! The system defers actual deallocation until the true parent owner (the array itself) is freed.

This architecture seamlessly combines static analysis (preventing moves out of borrowed data) with zero-cost runtime safety.

### 4.3 Borrow Limitations (Dataflow Checks)

While borrows drop gracefully, **they cannot be freely mutated, cloned blindly, or relocated** like standalone data! You cannot move a borrowed reference to another scope or treat it structurally as an owned instance.

Because TejX employs a secondary Dataflow Liveness Checker, it will interrupt compilation if you assign a borrow elsewhere without explicitly calling `.clone()`:

```ts
function processData(b: Box) {}

function main() {
  let list = [new Box("A")] as Box[];
  let borrowedItem = list[0];

  let cloned = borrowedItem; // ❌ COMPILE ERROR: Cannot move a borrowed reference
  processData(borrowedItem); // ❌ COMPILE ERROR: Cannot move a borrowed reference
}
```

---

## 5. Last-Use Detection & Implicit Moves

Because TejX values performance, the compiler uses **Liveness Analysis** to find the absolute final time an owned variable is read.

At the variable's final usage site, the compiler implicitly **Upgrades the operation to a Move**, freeing up local resources early rather than waiting artificially for the enclosing scope loop to terminate. This enables heavy memory reuse throughout long methods.

### 5.1 Conditional Moves (MaybeMoved)

If an owned variable is moved inside one branch but ignored in another, the compiler evaluates it defensively as `MaybeMoved` at the merge point. You can no longer access it after the branch structure resolves.

```ts
function main() {
  let a = new Box();

  if (someCondition) {
    let b = a; // Moved on this branch
  }

  print(a.value); // ❌ COMPILE ERROR: variable 'a' maybe moved.
}
```

### 5.2 Loop Scopes

Ownership moves executed within an iterative loop are evaluated as unconditional compile-time errors. Moving an owned variable inside an iteration block would immediately break iteration #2, confusing the scope bounds.

```ts
function main() {
  let a = new Box();

  while (true) {
    let b = a; // ❌ COMPILE ERROR: Use of moved variable 'a' in repetition body
  }
}
```

---

## 6. End-to-End Walkthrough

Here is a visual map of how ownership propagates, scales, and is deterministically torn down statically:

```ts
function spawn(): Array<Node> {
  const root = new Node("A"); // 1. Root allocation ID [200]
  const list = new Array(); // 2. Collection allocation ID [201]

  list.push(root); // 3. 'root' -> MOVED inside to Array [200].
  //    'root' binding is now statically invalidated.

  return list; // 4. 'list' -> MOVES out to Caller block.
}

function process() {
  let nodes = spawn(); // 5. 'nodes' binds ID [201]. (which owns ID [200]).

  let pointer = nodes[0]; // 6. LoadIndex applies BORROW_FLAG to the returned handle.

  print(pointer.data); // 7. Borrows successfully.

  // 8. End of Context!
  // -> BorrowChecker injects rt_free(pointer). Runtime sees BORROW_FLAG=1 and no-ops!
  // -> BorrowChecker injects rt_free(nodes). Runtime sees BORROW_FLAG=0 and deallocates!
  // -> The Runtime Array teardown triggers recursively, executing rt_free on node [200]
  // -> Memory cleans up seamlessly with ZERO garbage collection!
}
```

**Quick Referencing Chart**

| Action                  | Result                           | Original Status After Evaluated    |
| ----------------------- | -------------------------------- | ---------------------------------- |
| `let a = b` (Primitive) | Copy allocated                   | Fully Valid                        |
| `let a = b` (Heap Type) | Ownership moves (Implicit)       | **Invalidated (Moved)**            |
| `func(b)` (Heap Type)   | Ownership passed inside limits   | **Invalidated (Moved)**            |
| `return a;` (Heap Type) | Ownership transfers up hierarchy | Transfer Complete                  |
| `obj.method()`          | Object execution                 | Borrowed temporarily (Fully Valid) |
| `let val = arr[index]`  | Emits Static Borrow Tracking     | `borrowed_vars` exclusion recorded |
| End of Scope (`{ }`)    | Drop Generator (`rt_free`)       | All underlying owned values flush  |
