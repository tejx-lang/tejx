# Memory Management: Ownership & RAII in NovaJs

This document explains the shift from the legacy leaking memory model to the new Rust-like ownership model.

## 1. The Legacy Approach (Leaking)

Previously, NovaJs used a **Global Persistent Heap** without cleanup.

- **Allocation**: `new Class()` created an entry in a global array.
- **Assignment**: `let b = a` copied the reference ID. Both `a` and `b` pointed to the same object (Aliasing).
- **Deallocation**: **None**. Objects remained until program exit.

### Example (Legacy)

```javascript
class Node { id: int; }

function create() {
    let a = new Node(); // Allocates ID 100
    let b = a;          // Copies ID 100
    // Function ends. Object 100 LEAKS.
}
```

## 2. The New Approach (Ownership)

We are introducing **Ownership** and **Deterministic Destruction** (RAII).

- **Ownership**: Every object is owned by exactly **one** variable.
- **Move Semantics**: `let b = a` **moves** ownership to `b`. `a` becomes invalid.
- **Scope-based Drop**: When a variable goes out of scope, the memory is automatically freed.

### Example (New)

```javascript
class Node { id: int; }

function main() {
    let a = new Node(); // 'a' OWNS the object
    let b = a;          // MOVED to 'b'. 'a' is now dead.

    // print(a.id);     // COMPILE ERROR: Use of moved variable
    print(b.id);        // OK

} // 'b' goes out of scope -> Object is FREED.
```

## Implicit Moves vs Copy

- **Primitives** (`int`, `bool`, `float`): Implement **Copy**. They can be used multiple times.
- **Complex Types** (`Class`, `Array`, `Map`, `string`): Implement **Move**. Ownership is transferred.

## Function Calls

Passing a complex type to a function transfers ownership to that function.

```javascript
function use(n: Node) {
    print(n.id);
} // 'n' is freed here

function main() {
    let x = new Node();
    use(x); // 'x' is moved to 'use'
    // 'x' is invalid here
}
```
