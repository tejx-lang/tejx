# TejX Language Reference

TejX is a high-performance, strictly typed programming language that combines the ergonomics of modern TypeScript/Swift with the raw power of native compilation via LLVM.

## 🚀 Key Philosophy

TejX is designed to be the "sweet spot" between productivity and performance.

- **Strictly Typed**: No hidden coercion or fragile `any` types.
- **Memory Safe**: Ownership-based memory management (Deterministic ARC).
- **Native Performance**: Compiles AOT (Ahead-of-Time) to machine code.

---

## 📚 Language Specs

### 1. Variables & Data Types

TejX uses `let` for mutable variables and `const` for immutables.

- **Primitives**: `int`, `float`, `bool`, `string`, `char`.
- **Booleans**: Strict `true`/`false`. No truthy/falsy logic.
- **Null Safety**: Uses `Option<T>` for safe null handling.

### 2. Control Flow

Standard branching and looping with enhanced safety.

- `if / else if / else`
- `while` & `for` (C-style)
- `match`: Exhaustive pattern matching (Rust-style).
- `try / catch / finally`: Structured exception handling.

### 3. Object-Oriented Programming

TejX features a robust class-based system.

- **Classes**: Supports inheritance (`extends`), `super`, and member initialization.
- **Visibility**: `public` (default), `private`, and `protected`.
- **Protocols & Extensions**: Define shared behavior (Protocols) and add methods to existing types (Extensions).

### 4. Functions & Closures

- **Lambdas**: Arrow functions `() => {}` with closure support.
- **Async Functions**: Native `async/await` support ([See Concurrency Guide](CONCURRENCY.md)).
- **Parameters**: Typed parameters with optional default values and rest params (`...args`).

---

## 📚 Standard Library (`std:`)

TejX includes a modular standard library:

- **`std:math`**: Constants and high-performance math functions.
- **`std:fs`**: File system operations.
- **`std:json`**: High-speed JSON serialization.
- **`std:thread`**: Low-level threading primitives.
- **`std:collections`**: Advanced data structures (Map, Set, Stack, Queue).

---

## 📊 Feature Matrix

| Feature             | Status | Notes                                         |
| :------------------ | :----: | :-------------------------------------------- |
| **Generics**        |   ✅   | Native support for Arrays, Options, and more. |
| **Async/Await**     |   ✅   | Cooperative multitasking event loop.          |
| **Multi-Threading** |   ✅   | Native OS-level parallelism.                  |
| **FFI**             |   🔮   | C/C++ Interop (Planned).                      |
| **Result Type**     |   🔮   | Explicit error wrapping (Planned).            |
