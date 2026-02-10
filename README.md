# TejX Programming Language

TejX is a high-performance, strictly typed language that compiles to C++. It combines the ergonomics of modern TypeScript/Swift with the raw power and control of C++.

## Philosophy: "Best of All Worlds"

TejX is designed to be the ultimate sweet spot between productivity and performance.

## Key Features

- **Strict Contracts**: Protocols ensure strict interface adherence.
- **Native Performance**: Compiles to optimized C++.
- **Modern Syntax**: Familiar to JS/TS developers but without the runtime overhead.

## Usage

Compile a TejX file:

```bash
./build/tejxc tests/hello.tx
```

## Examples

Check out `tests/` for more `.tx` files.

### 🌓 The Hybrid Strategy: Total Evolution

TejX is built on the belief that code should be as expressive as JavaScript but as robust and predictable as a systems language.

| Aspect          | JavaScript Baggage (Removed)     | TejX Evolution (Borrowed Excellence)        | Inspired By    |
| :-------------- | :------------------------------- | :------------------------------------------ | :------------- |
| **Memory**      | Garbage Collection (Pauses)      | **Deterministic ARC (Automatic Ref Count)** | Swift / Obj-C  |
| **Nullability** | `null` AND `undefined` confusion | **Exhaustive Option<T> / None Type**        | Rust / Haskell |
| **Errors**      | Untracked Exceptions             | **Result<T, E> & Type-Safe Handling**       | Rust / Zig     |
| **Concurrency** | Single-threaded Event Loop       | **Structured Concurrency (Actor Model)**    | Swift / Go     |
| **Logic**       | Fragile Switch / Truthy Coercion | **Exhaustive Match & Strict Booleans**      | Rust / Kotlin  |
| **Types**       | Loose Structural Refs            | **Memory-Stable Static Structs**            | C++ / Swift    |
| **Inheritance** | Prototype Pollutions             | **Protocols & Type Extensions**             | Swift / Scala  |
| **Identity**    | Type Coercion (`==` / `!=`)      | **Strict Typed Equality (Native Identity)** | Rust / C++     |

## 🚀 Quick Start

### Build & Run Single File

```bash
./build.sh tests/logic.tejx
```

### Build & Run ALL Examples

```bash
./test_all.sh
```

---

## 📚 Comprehensive Feature Matrix

### 1. Variables & Data Types

| Feature          | Granularity          | Status | Notes                                        |
| :--------------- | :------------------- | :----: | :------------------------------------------- |
| **Declarations** | `let` (Mutable)      |   ✅   | Block-scoped, stack-allocated by default.    |
|                  | `const` (Immutable)  |   ✅   | Compile-time enforcement of immutability.    |
|                  | `var`                |   ⛔   | **Removed** to prevent hoisting bugs.        |
| **Primitives**   | `int` / `bigInt`     |   ✅   | Explicit 32/64-bit integers.                 |
|                  | `float` / `bigfloat` |   ✅   | Explicit 32/128-bit(quad) floats.            |
|                  | `number` (f64)       |   ✅   | Standard double-precision floating point.    |
|                  | `boolean`            |   ✅   | Strict true/false. No truthy/falsy coercion. |
|                  | `string`             |   ✅   | Immutable, UTF-8 aware.                      |
|                  | `void`               |   ✅   | Represents absence of return value.          |
| **Composite**    | `Option<T>`          |   ✅   | Type-safe null handling (replaces null).     |
|                  | `Result<T,E>`        |   🔮   | Planned explicit error wrapping type.        |
| **Inference**    | Local Type Inference |   ✅   | `let x = 10` automatically infers `number`.  |

### 2. Operators & Expressions

| Feature        | Granularity             | Status | Notes                                 |
| :------------- | :---------------------- | :----: | :------------------------------------ | ------------------------- |
| **Arithmetic** | `+`, `-`, `*`, `/`      |   ✅   | Standard arithmetic.                  |
|                | `%` (Modulo)            |   ✅   | Supports floating point modulo.       |
|                | `**` (Exponentiation)   |   🔮   | Planned syntax sugar for `pow()`.     |
| **Comparison** | `==`, `!=`              |   ✅   | Strict structural equality.           |
|                | `===`                   |   ⛔   | Unnecessary due to strict typing.     |
|                | `<`, `>`, `<=`, `>=`    |   ✅   | Numeric comparison.                   |
| **Logical**    | `&&` (AND), `\|\|` (OR) |   ✅   | Short-circuiting evaluation.          |
|                | `!` (NOT)               |   ✅   | Boolean negation only.                |
|                | `??` (Nullish Coalesce) |   ✅   | Fallback for null/undefined values.   |
| **Assignment** | `=`, `+=`, `-=`, etc.   |   ✅   | Standard compound assignments.        |
| **Access**     | `.` (Dot)               |   ✅   | Direct member access.                 |
|                | `?.` (Optional Chain)   |   ✅   | Safe navigation for nullable objects. |
|                | `[]` (Index)            |   ✅   | Array/Map access.                     |
| **Other**      | `typeof`                |   ✅   | Runtime type inspection string.       |
|                | `instanceof`            |   ✅   | Inheritance-aware runtime type check. |
|                | Ternary `? :`           |   ✅   | Conditional expression.               |
|                | Spread `...`            |   ✅   | Array/Object expansion.               |
|                | Pipeline `              |   >`   | 🔮                                    | Function chaining syntax. |

### 3. Control Flow

| Feature          | Granularity                 | Status | Notes                                     |
| :--------------- | :-------------------------- | :----: | :---------------------------------------- |
| **Conditionals** | `if` / `else` / `else if`   |   ✅   | Standard branching.                       |
|                  | `match`                     |   ✅   | Exhaustive pattern matching (Rust-style). |
| **Loops**        | `while`                     |   ✅   | Standard while loop.                      |
|                  | `do-while`                  |   🔮   | Post-condition loop.                      |
|                  | `for` (C-style)             |   ✅   | Classic loop logic.                       |
|                  | `for-of`                    |   🔮   | Iterator-based collection traversal.      |
|                  | `for-in`                    |   ⛔   | Discouraged (use `Object.keys`).          |
| **Jumps**        | `break`                     |   ✅   | Exit loop.                                |
|                  | `continue`                  |   ✅   | Skip iteration.                           |
|                  | `return`                    |   ✅   | Return from function.                     |
|                  | `try` / `catch` / `finally` |   ✅   | Structured Exception Handling.            |

### 4. Functions

| Feature         | Granularity           | Status | Notes                                 |
| :-------------- | :-------------------- | :----: | :------------------------------------ |
| **Definitions** | Function Declarations |   ✅   | Hoisted, named functions.             |
|                 | Arrow Functions       |   ✅   | Concise syntax `() => {}`.            |
|                 | Anonymous Functions   |   ✅   | Lambdas.                              |
| **Parameters**  | Typed Parameters      |   ✅   | Enforced at compile time.             |
|                 | Default Values        |   ✅   | `func(a = 10)`.                       |
|                 | Rest Parameters       |   ✅   | `func(...args)` as vector.            |
|                 | Named Parameters      |   🔮   | Call site clarity `func(x: 10)`.      |
| **Features**    | Recursion             |   ✅   | Stack-safe within limits.             |
|                 | Closures              |   ✅   | Capture-by-value semantics default.   |
|                 | Generators            |   🔮   | `function*` with `yield`.             |
|                 | Async/Await           |   ✅   | Native `std::future` integration.     |
|                 | Overloading           |   🔮   | Multiple signatures for one function. |

### 5. Object Oriented Programming

| Feature           | Granularity            | Status | Notes                                            |
| :---------------- | :--------------------- | :----: | :----------------------------------------------- |
| **Classes**       | Definition             |   ✅   | `class Name { ... }`.                            |
|                   | Constructors           |   ✅   | `constructor()`.                                 |
| Feature           | Granularity            | Status | Notes                                            |
| :---------------- | :--------------------- | :----: | :----------------------------------------------- |
| **Classes**       | Definition             |   ✅   | `class Name { ... }`.                            |
|                   | Constructors           |   ✅   | `constructor()`.                                 |
|                   | Properties             |   ✅   | Typed instance variables.                        |
|                   | Methods                |   ✅   | Instance capabilities.                           |
| **Encapsulation** | `public`               |   ✅   | Default visibility.                              |
|                   | `private`              |   ✅   | Module/Class restricted.                         |
|                   | `protected`            |   ✅   | Subclass restricted.                             |
|                   | `readonly`             |   🔮   | Immutable properties.                            |
| **Inheritance**   | `extends`              |   ✅   | Single inheritance chain.                        |
|                   | `super`                |   ✅   | Call parent methods.                             |
|                   | `abstract`             |   ✅   | Abstract base classes.                           |
| **Polymorphism**  | Extensions             |   ✅   | Swift-like `extension Class {}`.                 |
|                   | Protocols              |   ✅   | Contract-based polymorphism.                     |
|                   | Generic Classes        |   🔮   | Blocked on full generic parser.                  |
|                   | Mixins                 |   🔮   | Composition over inheritance.                    |

### 6. Data Structures

| Feature         | Granularity    | Status | Notes                                            |
| :-------------- | :------------- | :----: | :----------------------------------------------- |
| **Collections** | `Array<T>`     |   ✅   | backed by `std::vector`. dynamic size.           |
|                 | `Map<K,V>`     |   ✅   | backed by `std::map`.                            |
|                 | `Set<T>`       |   ✅   | backed by `std::set`.                            |
|                 | `LinkedList`   |   🔮   | Standard library addition.                       |
| **Structs**     | `struct`       |   ✅   | Value-type, stack allocated lightweight objects. |
| **JSON**        | Native Parsing |   ✅   | `JSON.parse` / `stringify`.                      |

### 7. Advanced Type System

| Feature       | Granularity         | Status | Notes                       |
| :------------ | :------------------ | :----: | :-------------------------- |
| **Generics**  | Generic Functions   |   🔮   | `func<T>(arg: T)`.          |
|               | Generic Constraints |   🔮   | `func<T: Number>(arg: T)`.  |
| **Algebraic** | Union Types         |   🔮   | `string \| number`.         |
|               | Intersection Types  |   🔮   | `Named & Identifiable`.     |
|               | Type Aliases        |   🔮   | `type ID = string`.         |
| **Safety**    | Null Safety         |   ✅   | Strict null checks enabled. |
|               | Casts               |   🔮   | `as Type` safe casting.     |

### 8. System & Runtime

| Feature         | Granularity        | Status | Notes                                    |
| :-------------- | :----------------- | :----: | :--------------------------------------- |
| **Memory**      | ARC                |   ✅   | Deterministic cleanup. No GC pauses.     |
|                 | Manual Management  |   ⛔   | Safe by default. `unsafe` block planned. |
| **Concurrency** | OS Threads         |   ✅   | `Thread.spawn`. True parallelism.        |
|                 | Atomic Types       |   🔮   | Thread-safe primitives.                  |
|                 | Mutex / Locks      |   ✅   | `Mutex` for synchronization.             |
|                 | Channels / Actors  |   🔮   | Message passing concurrency.             |
| **IO**          | File System        |   ✅   | Read/Write/Exists/Delete.                |
|                 | Network (TCP/HTTP) |   🔮   | Native networking stack.                 |
| **Interop**     | FFI (C/C++)        |   🔮   | Zero-cost calls to native libs.          |

### 9. Tooling & Ecosystem

| Feature       | Granularity           | Status | Notes                      |
| :------------ | :-------------------- | :----: | :------------------------- |
| **Compiler**  | Optimization Levels   |   🔮   | `-O1`, `-O2`, `-O3`.       |
|               | Incremental Build     |   🔮   | Fast recompilation.        |
| **Dev Tools** | Formatter             |   🔮   | `tejx fmt`.                |
|               | Linter                |   🔮   | Static analysis.           |
|               | LSP (Language Server) |   🔮   | IDE Autocomplete & Errors. |
|               | REPL                  |   🔮   | Interactive shell.         |
|               | Debugger Support      |   🔮   | DWARF symbol generation.   |
| **Package**   | Package Manager       |   🔮   | `tejx install`.            |
|               | Versioning            |   🔮   | SemVer enforcement.        |

---

<div align="center">
    Built with ❤️ by the TejX Team
</div>
