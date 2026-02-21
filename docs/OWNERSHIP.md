# Memory Ownership in TejX

TejX uses a **single-ownership** memory model to guarantee memory safety at compile time — no garbage collector, no manual `free`, no runtime overhead. Every heap-allocated value has exactly one owner at any given time. When that owner goes out of scope, the value is automatically freed.

This guide covers how ownership works, when values are moved vs. copied, and the rules you need to follow when writing TejX code.

---

## 1. Declaring Variables

TejX provides two keywords for variable declarations:

| Keyword | Mutability | Reassignable? |
| :------ | :--------- | :------------ |
| `let`   | Mutable    | ✅ Yes        |
| `const` | Immutable  | ❌ No         |

```ts
function main() {
  let x = 10; // mutable — can be reassigned
  x = 20; // ✅ OK

  const PI = 3.14; // immutable — cannot be reassigned
  PI = 3.0; // ❌ COMPILE ERROR: cannot assign to constant
}
```

Both `let` and `const` variables follow the same ownership rules described below. The difference is purely about reassignment — `const` prevents you from rebinding the name to a new value.

---

## 2. Ownership Rules (The Big Three)

1. **Every value has exactly one owner.**
2. **When the owner goes out of scope, the value is automatically dropped (freed).**
3. **Assigning or passing an owned value transfers ownership (a "move").**

### Automatic Drop

When a variable goes out of scope, TejX inserts a `Free` instruction automatically. You never call `free` yourself:

```ts
function main() {
  let r = new Resource(1);
  print("Using", r.id);
  // ← 'r' is automatically freed here at end of scope
}
```

Scoped blocks also trigger drops:

```ts
function main() {
  {
    let temp = new Resource(2);
    print(temp.id);
  }
  // ← 'temp' is freed here, not at end of main
}
```

---

## 3. Move Semantics (Heap Types)

For **class instances**, **arrays**, and **strings**, assignment transfers ownership. The original variable becomes **invalid** and cannot be used again.

### 3.1 Move on Assignment

```ts
class Box {
  value: int;
}

function main() {
  let a = new Box();
  a.value = 10;

  let b = a; // ← ownership MOVES from 'a' to 'b'

  print(b.value); // ✅ OK — 'b' owns the Box
  print(a.value); // ❌ COMPILE ERROR: use of moved variable 'a'
}
```

After `let b = a`, the variable `a` is in a **Moved** state. The compiler will reject any further use of `a`.

### 3.2 Move into Functions

Passing a heap value to a function also transfers ownership:

```ts
class Item {
  name: string;
  constructor(n: string) {
    this.name = n;
  }
}

function consume(item: Item) {
  print("Consumed:", item.name);
  // 'item' is freed at end of function
}

function main() {
  let item = new Item("Apple");

  consume(item); // ← ownership moves into 'consume'

  print(item.name); // ❌ COMPILE ERROR: use of moved variable 'item'
}
```

### 3.3 Returning Ownership

A function can give ownership back to the caller by returning the value:

```ts
function passThrough(c: Container): Container {
  print("Passing through:", c.id);
  return c; // ← ownership transfers to the caller
}

function main() {
  let c1 = new Container(100);
  let c2 = passThrough(c1); // c1 → moved, c2 → live

  print(c2.id); // ✅ OK — 'c2' owns the Container
}
```

### 3.4 Reinitializing After a Move

A moved variable can be **reinitialized** by assigning a new value to it:

```ts
function main() {
  let a = new Box();
  let b = a; // 'a' is moved

  a = new Box(); // ← 'a' is reinitialized with a new value
  print(a.value); // ✅ OK — 'a' is live again
}
```

---

## 4. Copy Semantics (Primitives)

**Primitives** (`int32`, `float64`, `bool`) use **copy semantics**. They are duplicated on assignment or when passed to functions. The original remains fully valid.

```ts
function takePrimitive(val: int32) {
  print("Received:", val);
}

function main() {
  let num = 42;
  takePrimitive(num); // 'num' is COPIED, not moved

  print(num); // ✅ OK — primitives are always valid
}
```

**Summary:**

| Type                       | Semantics | After Assignment / Pass |
| :------------------------- | :-------- | :---------------------- |
| `int32`, `float64`, `bool` | **Copy**  | Original stays valid    |
| Class instances            | **Move**  | Original is invalidated |
| Arrays                     | **Move**  | Original is invalidated |
| Strings                    | **Move**  | Original is invalidated |

---

## 5. Borrowing (Method Calls)

When you call a **method** on an object, TejX **borrows** the receiver (`this`) instead of moving it. This means the object stays valid after the method call.

```ts
class BankAccount {
  private balance: int32 = 0;

  constructor(initial: int32) {
    this.balance = initial;
  }

  deposit(amount: int32): void {
    this.balance += amount;
  }

  getBalance(): int32 {
    return this.balance;
  }
}

function main() {
  let acc = new BankAccount(100);

  acc.deposit(50); // ← borrows 'acc', does NOT move it
  acc.deposit(25); // ← still valid, can call again!

  print(acc.getBalance()); // ✅ prints 175
}
```

### Borrowing Rules Summary

| Call Type          | `this` / receiver | Other arguments |
| :----------------- | :---------------- | :-------------- |
| Method call        | **Borrowed**      | **Moved**       |
| Global function    | N/A               | **Moved**       |
| Constructor        | **Borrowed**      | **Moved**       |
| Stdlib / Intrinsic | **Borrowed**      | **Borrowed**    |

> **Key insight:** Accessing a field like `obj.value` is a borrow — the object remains live. But passing `obj` to a global function moves it.

---

## 6. Ownership in Control Flow

The compiler tracks ownership through branches and loops. If a variable is moved in only _some_ paths, it enters a **MaybeMoved** state and cannot be used afterward.

### 6.1 Conditional Moves

```ts
function main() {
  let a = new Data();

  if (someCondition) {
    let b = a; // moved in this branch only
  }

  print(a.value); // ❌ COMPILE ERROR: variable 'a' maybe moved
}
```

Even though `a` might not have been moved (the `else` branch doesn't touch it), the compiler conservatively rejects the use because it _could_ have been moved.

### 6.2 Loop Moves

Moving inside a loop is always an error, because the second iteration would try to move an already-moved value:

```ts
function main() {
  let a = new Box();

  while (true) {
    let b = a; // ❌ COMPILE ERROR: use of moved variable 'a'
    // (moved on the first iteration)
  }
}
```

---

## 7. Ownership with Collections

When you insert an object into a collection (e.g., `Array.push`), ownership transfers into the collection. When you remove it (e.g., `Array.pop`), ownership transfers back out.

```ts
class Resource {
  id: int32;
  constructor(id: int32) {
    this.id = id;
  }
}

class Pool {
  items: Resource[];

  constructor() {
    this.items = [];
  }

  add(r: Resource) {
    this.items.push(r); // ← 'r' is moved into the array
  }

  take(): Resource {
    return this.items.pop(); // ← ownership moves out to caller
  }
}

function main() {
  let pool = new Pool();

  let r1 = new Resource(1);
  pool.add(r1); // r1 is moved into pool
  // r1 is now invalid

  let r2 = pool.take(); // ownership comes back out
  print(r2.id); // ✅ OK
}
```

---

## 8. Scoping and Shadowing

Variables are block-scoped. An inner block can shadow an outer variable without affecting it:

```ts
function main() {
  let x = 10;

  {
    let x = 20; // shadows outer 'x'
    print(x); // prints 20
  }

  print(x); // prints 10 — outer 'x' is unchanged
}
```

Variables declared inside a block are not accessible outside it:

```ts
function main() {
  if (true) {
    let y = 30;
  }
  print(y); // ❌ COMPILE ERROR: 'y' not in scope
}
```

---

## 9. Summary Cheat Sheet

```
┌─────────────────────────────────────────────────────┐
│            TejX Ownership Quick Reference           │
├─────────────────────────────────────────────────────┤
│                                                     │
│  let x = value;     → mutable, can be reassigned    │
│  const x = value;   → immutable, cannot reassign    │
│                                                     │
│  let b = a;          → MOVE (heap) or COPY (prim)   │
│  foo(a);             → MOVE into function            │
│  return a;           → MOVE out to caller            │
│                                                     │
│  obj.method();       → BORROW (obj stays valid)      │
│  obj.field           → BORROW (obj stays valid)      │
│                                                     │
│  arr.push(item);     → MOVE item into collection     │
│  arr.pop();          → MOVE item out of collection   │
│                                                     │
│  Moved variable?     → Reassign to make it live      │
│  End of scope?       → Automatic free (no GC!)       │
│                                                     │
│  Conditional move?   → "MaybeMoved" = compile error  │
│  Loop move?          → Always a compile error         │
│                                                     │
└─────────────────────────────────────────────────────┘
```
