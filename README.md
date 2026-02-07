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
./build/tejxc examples/hello.tx
```

## Examples

Check out `examples/` for more `.tx` files.

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
./build.sh examples/logic.tejx
```

### Build & Run ALL Examples

```bash
./test_all.sh
```

---

## 📚 Comprehensive Feature Status

### 1. Variables & Types

- [x] **`number` Type**: Double-precision floating point (`let x: number = 42.5;`).
- [x] **`string` Type**: Native immutable strings (`let s: string = "Hello";`).
- [x] **`boolean` Type**: `true` / `false`.
- [x] **`void` Type**: For functions returning nothing.
- [x] **`let` Keyword**: Block-scoped mutable variable declaration.
- [x] **`const` Keyword**: Immutable variable declaration.
- [ ] **`var` Keyword**: **Intentionally Omitted** (Avoids hoisting and scope confusion).
- [x] **Type Inference**: Automatic type deduction for variables (`let x = 10;` -> number).
- [x] **Explicit Typing**: TypeScript-style syntax (`let x: number = 10;`).
- [x] **Destructuring**: Pattern matching assignment (`let {x, y} = p;` or `let [a, b] = list;`).

### 2. Operators

- [x] **Arithmetic**: `+`, `-`, `*`, `/`.
- [x] **Modulo**: `%` (Supports floating point modulo via `fmod`).
- [x] **Assignment**: `=`.
- [x] **Equality**: Strict-by-default behavior (Compiles to typed C++ comparisons). No hidden type-coercion bugs.
- [x] **Relational**: `<`, `>`, `<=`, `>=`.
- [x] **Logical AND**: `&&` (Short-circuiting).
- [x] **Logical OR**: `||` (Short-circuiting).
- [x] **Logical NOT**: `!`.
- [x] **String Concatenation**: `"A" + "B"` or `"Val: " + 123` or `"Active: " + true`.
- [x] **Compound Assignment**: `+=`, `-=`, `*=`, `/=`.
- [x] Increment/Decrement: `++`, `--` (Prefix Supported).
- [x] Ternary Operator: `cond ? a : b`.
- [x] **`typeof` Operator**: Returns native type as string.

### 3. Control Flow

- [x] **If / Else**: Standard conditional execution.
- [x] **While Loop**: Standard while loop.
- [x] **For Loop**: C-style `for (let i=0; i<10; i = i+1) { ... }`.
- [x] **Block Scoping**: Variables declared inside `{ ... }` are scoped to that block.
- [x] Return Statement: Returning values from functions.
- [x] Break Statement: Exit loops early.
- [x] Continue Statement: Skip loop iteration.
- [x] Switch Statement: Standard multi-way branch.
- [x] **Match Expression**: Rust-style exhaustive pattern matching with guards and rest patterns.
- [x] **Strict Boolean Evaluation**: Control flow (`if`, `while`, `for`) requires explicit `boolean` values.

### 4. Functions

- [x] **Function Declaration**: `function name(arg: type): returnType { ... }`.
- [x] **Arguments**: Typed arguments supported.
- [x] **Recursion**: Fully supported (stack-safe within system limits).
- [x] **Void Returns**: Supported.
- [x] Anonymous Functions / Lambdas: `(x) => ...`.
- [x] Default Arguments: `function f(x = 10)`.
- [x] Rest Arguments: `function sum(...nums: number[])`.
- [x] **Destructuring (Array & Object)**: Native desugaring into efficient local assignments.

### 5. Arrays

- [x] **Array Literals**: `[1, 2, 3]`.
- [x] **Typed Arrays**: `number[]`, `string[]`, etc.
- [x] **Index Access**: `arr[i]` (0-based).
- [x] **Index Assignment**: `arr[i] = newVal`.
- [x] **Dynamic Sizing**: Backed by `std::vector`.
- [x] **`.push(item)`**: Add elements to end.
- [x] **`.length`**: Get array size (returns double).
- [x] **Array Methods**: `.map`, `.filter`, `.forEach` (Implemented).

### 6. Object Literals (Dynamic)

- [x] **Literal Syntax**: `{ name: "TejX", age: 1 }`.
- [x] **Dynamic Key Access**: `obj["key"]` or `obj.key` (via Map).
- [x] **Heterogeneous Values**: Can hold mixed types (Number, String, Boolean).
- [x] **Printing**: `console.log(obj)` prints JSON-like string.
- [x] **Nested Dynamic Objects**: `{ inner: { key: value } }` (Implemented).

### 7. Structural Types (Static)

- [x] **Inline Type Definition**: `let p: { x: number, y: number };`.
- [x] **Auto-Struct Generation**: Compiler generates unique C++ structs (`Struct_hash...`).
- [x] **Member Access**: `p.x` (Direct field access, high performance).
- [x] **Initialization**: `let p = { x: 10, y: 20 };`.
- [x] **Nested Structural Types**: Supported via recursive Var type.

### 8. Object Oriented Programming (OOP)

- [x] **Class Declaration**: `class Person { ... }`.
- [x] **Member Variables**: `name: string;`.
- [x] **Constructor**: `constructor(n: string) { ... }`.
- [x] **Methods**: `sayHello(): void { ... }`.
- [x] **Instantiation**: `let p = new Person("TejX");`.
- [x] **`this` Keyword**: Access current instance.
- [x] **Member Access**: `p.name`, `p.sayHello()`.
- [x] **Inheritance**: `extends` (Implemented).
- [ ] **Interfaces**: `interface` (Planned).
- [x] **Access Modifiers**: `public`, `private` (Implemented).
- [x] **Static Members**: `static` (Implemented).
- [x] **Extensions**: Add methods to existing types (`extension User { ... }`).

### 9. Standard Library

- [x] **Console**: `console.log(...)` (Variadic, supports all native types).
- [x] **Math Library**:
  - `Math.sin(x)`, `Math.cos(x)`, `Math.tan(x)`
  - `Math.max(a, b)`, `Math.min(a, b)`
  - `Math.pow(base, exp)`, `Math.sqrt(x)`
  - `Math.floor(x)`, `Math.ceil(x)`, `Math.round(x)`
  - `Math.random()`
- [x] File I/O: `fs.readFile`, `fs.writeFile`.
- [x] Time/Date: `Date` class.
- [x] **Error Handling**: `try { } catch (e) { } finally { }`.
- [x] **Modern Syntax**:
  - [x] **Optional Chaining**: `obj?.prop`.
  - [x] **Nullish Coalescing**: `val ?? default`.
  - [x] **Enums**: `enum Direction { Up, Down }`.

### 10. Compiler & Architecture

- [x] Lexer: Custom high-performance tokenizer.
- [x] Parser: Hand-written recursive descent parser.
- [x] AST: Tree representation with Binding Patterns support.
- [x] Code Generator: Transpiles AST to native C++17.
- [x] Memory Management: Automatic Reference Counting (ARC) via `std::shared_ptr`.
- [x] **Unified Nullish**: `null` and `undefined` unified into a single internal state.
- [x] Compilation: Uses `clang++` to produce native binaries.
- [ ] **Error Handling**: Graceful error messages with line numbers (Partially Implemented).
- [ ] **Optimization Passes**: (Planned).

---

## 🛠 Project Structure

- `src/` - Source code for the compiler steps.
  - `lexer/` - Tokenization.
  - `parser/` - Syntax analysis.
  - `codegen/` - C++ generation.
- `include/` - Header files.
- `examples/` - Sample `.tejx` programs to test features.
- `tejxc` - The compiled compiler binary.

---

### 🔴 Phase 3: The "Native Soul" Core (Primary Focus)

The ultimate goal is to provide a safety model that prevents entire categories of bugs at compile-time.

- [x] **Exhaustive Match Expressions**: Replaces `switch` with powerful nested pattern matching, guards, and rest patterns.
- [x] **Native Option<T> & Result<T, E>**: Eliminates the "Billion Dollar Mistake" by making absence and errors explicit members of the type system.
- [x] **Protocols & Extensions**: A powerful system for adding behavior to existing types without inheritance chains.
- [ ] **Structured Concurrency (Actors)**: Isolated state management with high-performance native thread utilization. No data races.
- [ ] **Generics & Variance**: Type-safe reuse for advanced data structures.

### 🟠 Phase 4: Modern Ergonomics (Syntax Excellence)

Borrowing the most productive syntaxes from the modern world.

- [ ] **Spread/Rest for Objects & Arrays**: High-performance native implementation using stack-allocated fragments where possible.
- [ ] **Template Literals (Tagged)**: Efficient string interpolation with zero runtime overhead via comptime parsing.
- [ ] **Decorators & Macros**: metaprogramming capabilities inspired by Rust/Scala for high-level abstractions.
- [ ] **Tuple Types**: Fixed-size heterogeneous collections.
- [ ] **Iterators & Generators**: Native protocol-backed iteration.

### � Phase 4.5: Daily Driver Utilities (StdLib Expansion)

Essential tools for comfortable daily development.

- [x] **Rich String API**: `split`, `trim`, `replace`, `up/down case`, `search`.
- [x] **Array Power**: `reduce`, `find`, `slice`, `sort`, `reverse`, `forEach`, `map`, `filter`, `concat`, `join`, `indexOf`, `push`, `pop`, `shift`, `unshift`.
- [x] **Console Enhancements**: `console.log`, `console.error`, `console.warn`.
- [x] **Iteration**: `for...of` loop for typed arrays.
- [x] **JSON Integration**: Native `parse` and `stringify`.
- [x] **FileSystem++**: `exists`, `remove`, `mkdir`.

### �🟡 Phase 5: Ecosystem & Tooling (The Muscle)

A language is only as good as its tools.

- [ ] **TejX Package Manager (npm+)**: Secure, fast, and local-first dependency management.
- [ ] **Zero-Overhead FFI**: Call into C, C++, and Swift with native calling conventions. No "glue code" performance tax.
- [ ] **LLVM Backend**: Move from C++ transpilation to direct LLVM IR generation for maximum optimization.
- [ ] **WASM Support**: Compile the same "Native Soul" to the web at peak performance.
- [ ] **Integrated LSP**: Immediate, precise feedback in all major IDEs.
- [ ] **Built-in Test Runner**: Blazing fast testing suite inspired by Go/Rust.

### 🔵 Advanced Features

| Feature              | Description                                         | Status  |
| -------------------- | --------------------------------------------------- | ------- |
| **Modules**          | `import`/`export` for code organization             | Planned |
| **Abstract Classes** | `abstract class Shape { abstract area(): number }`  | Planned |
| **Decorators**       | `@deprecated`, `@readonly` annotations              | Planned |
| **Getter/Setter**    | `get name() { }`, `set name(v) { }`                 | Planned |
| **Symbol Type**      | Unique identifiers                                  | Planned |
| **Map/Set Types**    | `Map<K,V>`, `Set<T>` built-in types                 | Planned |
| **WeakMap/WeakRef**  | Weak references for GC                              | Planned |
| **Proxy/Reflect**    | Metaprogramming (Evaluating for performance/safety) | Planned |
| **Iterators**        | `for...of`, `Symbol.iterator`                       | Planned |
| **Generators**       | `function* gen() { yield 1; }`                      | Planned |

### 🟣 Concurrency & I/O

| Feature              | Description                         | Status          |
| -------------------- | ----------------------------------- | --------------- |
| **Async/Await**      | `async function`, `await promise`   | **Implemented** |
| **Promise Chaining** | `.then()`, `.catch()`, `.finally()` | Planned         |

| **Promise.all** | `await Promise.all([p1, p2])` | **Implemented** |
| **Promise.race** | First to resolve wins | **Implemented** |
| **HTTP Client** | `http.get()`, `http.post()`, `fetch()` | **Basic Impl** |
| **TCP/UDP Sockets** | Network programming | Planned |
| **WebSockets** | Real-time communication | Planned |
| **Threads** | True parallelism (`Thread`, `Mutex`) | **Implemented** |
| **Event Emitter** | Pub/sub pattern | Planned |

### ⚙️ Compiler & Tooling

| Feature                     | Description                                       | Status  |
| --------------------------- | ------------------------------------------------- | ------- |
| **Better Error Messages**   | Line numbers, column info, helpful hints          | Planned |
| **Source Maps**             | Debug compiled code                               | Planned |
| **Optimization Passes**     | Dead code elimination, inlining, constant folding | Planned |
| **REPL Mode**               | Interactive shell for quick testing               | Planned |
| **Type Checker**            | Static type checking before codegen               | Planned |
| **LSP Server**              | IDE integration (autocomplete, hover, go-to-def)  | Planned |
| **Package Manager**         | Dependency management (`tejx install pkg`)        | Planned |
| **Build System**            | Incremental compilation, caching                  | Planned |
| **Test Framework**          | Built-in test runner (`test("name", () => {})`)   | Planned |
| **Documentation Generator** | Generate docs from comments                       | Planned |

### 📦 Platform Features

| Feature               | Description                   | Status  |
| --------------------- | ----------------------------- | ------- |
| **FFI**               | Call native C/C++ libraries   | Planned |
| **WASM Target**       | Compile to WebAssembly        | Planned |
| **Cross-Compilation** | Build for different targets   | Planned |
| **Embedding API**     | Embed TejX in other apps      | Planned |
| **Debug Symbols**     | DWARF debug info for GDB/LLDB | Planned |
