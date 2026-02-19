# NovaJs Compiler: Comprehensive Guide

NovaJs is a high-performance, TypeScript-like language compiler built in Rust that targets native machine code via LLVM. It combines modern web development syntax with the efficiency of native execution.

## 🚀 Architecture & Pipeline

The NovaJs compiler follows a traditional multicore-capable pipeline:

1.  **Lexing (`lexer.rs`)**: Converts raw source text into a stream of tokens.
2.  **Parsing (`parser.rs`)**: Transforms tokens into an Abstract Syntax Tree (AST).
3.  **Type Checking (`type_checker.rs`)**: Performs two-pass validation (hoisting + semantic check) to ensure type safety and resolve declarations.
4.  **Lowering (`lowering.rs`)**: Translates AST into a Higher-level Intermediate Representation (HIR), resolving namespaces and imports.
5.  **MIR Generation (`mir_lowering.rs`)**: Converts HIR into a control-flow graph based Mid-level Intermediate Representation (MIR) optimized for code generation.
6.  **Code Generation (`codegen.rs`)**: Emits LLVM IR from MIR functions.
7.  **Linking (`linker.rs`)**: Links generated LLVM IR with the embedded **TejX Runtime** to produce the final native executable.

### Entry Point Generation

The compiler automatically generates a C-compatible `main` entry point that:

1.  Initializes the runtime.
2.  Calls `tejx_runtime_main`.
3.  Executes the user's top-level code (wrapped in `tejx_main`).
4.  User-defined `main` functions are renamed to `f_main` to avoid symbol conflicts.

---

## ✨ Language Features

### Type System

NovaJs supports a robust, static type system with advanced features:

- **Primitives**:
  - **Integers**: `int`, `int16`, `int32`, `int64`, `int128`
  - **Floats**: `float`, `float16`, `float32`, `float64`
  - **Others**: `string`, `bool`, `char`, `any`, `void`
- **Generics**: Fully supported for arrays and built-ins: `Array<T>`, `Promise<T>`, `Option<T>`, `Map<K, V>`, `Set<T>`.
- **Function Types**: `(param: Type) => ReturnType`.
- **Object Types**: `{ property: Type }`.
- **Validation**: Strict "Unknown data type" detection for all declarations.

### Object-Oriented Programming (OOP)

- **Classes**: Support for `extends`, `super()`, and member initialization.
- **Protocols (Interfaces)**: Define contracts for classes to implement.
- **Extensions**: Dynamically add methods to existing classes or implement protocols for them.
- **Visibility**: `public`, `private`, and `protected` modifiers.
- **Static Members**: `static` properties and methods.
- **Accessors**: Support for `get` and `set` computed properties.

### Functional & Modern Syntax

- **Lambdas**: Arrow functions with closure support.
- **Destructuring**: Array and Object destructuring in assignments and patterns.
- **Spread/Rest**: Support for `...` in arrays, objects, and function parameters.
- **Control Flow**: `if/else`, `while`, `for`, `for...of`, and `switch`.
- **Operators**: Optional chaining (`?.`), Nullish coalescing (`??`), and Ternary (`? :`).

### Async Programming

- **Async/Await**: Native support for asynchronous functions.
- **Promises**: Built-in Promise type and `Promise.all` support.

---

## 📚 Standard Library (`std:`)

NovaJs includes a modular standard library accessible via the `std:module` syntax.

### `std:math`

Provides mathematical constants and functions:

- `abs(x)`, `sqrt(x)`, `sin(x)`, `cos(x)`, `pow(b, e)`.
- `floor(x)`, `ceil(x)`, `round(x)`, `random()`.
- `min(a, b)`, `max(a, b)`.

### `std:fs`

Low-level filesystem operations:

- `readFile(path)`, `writeFile(path, content)`.
- `exists(path)`, `remove(path)`, `mkdir(path)`.

### `std:system`

Operating system and process integration:

- `system.argv`: Command-line arguments.
- `system.env`: Access environment variables.
- `system.os`: Current operating system name.
- `system.exit(code)`: Exit the process with a status code.

### `std:time`

Time-related utilities:

- `now()`: Current timestamp in milliseconds.
- `sleep(ms)`: Pause execution.

### `std:json`

JSON serialization and parsing:

- `stringify(val)`: Convert value to JSON string.
- `parse(str)`: Parse JSON string to value.

### Built-in Globals

- `print(...)`: Console output.
- `parseInt(s)`, `parseFloat(s)`.
- `typeof(expr)`: Returns type string (Function call syntax).
- `instanceof`: Runtime type checking.

---

---

## 🧠 Deep Dive: Memory Management

NovaJs employs a sophisticated heap-based memory management system designed for both performance and thread safety.

### The Tagged Value System

At the core of the runtime is the `TaggedValue` enum. Every value in NovaJs is either a native primitive or an ID pointing to a `TaggedValue` on the heap:

- **Primitives**: `f64` (Numbers) and `bool` are often boxed/unboxed dynamically.
- **Complex Objects**: Arrays, Maps, Threads, and Mutexes are stored as variants of `TaggedValue`.
- **Ownership**: For threading primitives (Mutex, Atomic, Condition), the runtime uses `Arc<T>` to provide thread-safe, shared ownership across multiple user threads.

### The Global Heap

The compiler utilizes a global, thread-safe `HEAP` (backed by `LazyLock<Mutex<Heap>>`).

- **Object Allocation**: Objects are assigned a unique 64-bit ID.
- **Reference Resolution**: Runtime functions resolve these IDs to their underlying Rust structures with minimal overhead.
- **Memory Safety**: By using Rust's ownership model internally, the runtime ensures that even complex synchronization primitives are handled without data races.

---

## 🧵 Deep Dive: Threading & Synchronization

NovaJs provides a first-class threading model that bridges JavaScript simplicity with native performance.

### Threading Primitives

Exposed via the `std:thread` module:

- **`Thread`**: Maps directly to native Rust `std::thread`, running callbacks in parallel at the OS level.
- **`Atomic`**: Uses `AtomicI64` with `Ordering::SeqCst` for consistent, lock-free operations.
- **`Mutex`**: Implements a logical lock using `Arc<(Mutex<bool>, Condvar)>`, ensuring that even if multiple threads wake up, only one can claim the resource.
- **`Condition`**: Exposes native condition variables for efficient producer-consumer patterns.

### Scalability

Since NovaJs threads are native OS threads, they are not limited by a single-threaded event loop. This allows for true multi-core parallel processing of analytical and computational tasks.

---

## ⚡ Performance Engineering

NovaJs is built for speed. Its performance is derived from several key architectural choices:

1.  **AOT Compilation**: Unlike JIT-based engines, NovaJs generates optimized machine code before execution, eliminating "warm-up" time and lowering latency.
2.  **LLVM Backend**: Leverages the world-class LLVM optimization pipeline for register allocation, loop unrolling, and SIMD vectorization.
3.  **Minimal Runtime Overhead**: Runtime functions are written in highly optimized Rust and linked directly into the binary. Many primitive operations (like integer arithmetic) translate directly to single CPU instructions with zero runtime dispatch.
4.  **MIR Optimization**: The Mid-level Intermediate Representation (MIR) allows the compiler to perform control-flow analysis and dead-code elimination before passing the IR to LLVM.

---

## ⏳ Async/Await Implementation

Async programming in NovaJs is handled through a combination of compiler lowering and runtime support:

1.  **Lowering**: The compiler transforms `async` functions into state machines or utilizes the `__await` runtime intrinsic.
2.  **Promises**: The `Promise` class in NovaJs serves as a handle for values that will be resolved in the future, with built-in support for `Promise.all` and `await`.
3.  **Concurrency**: While `await` pauses the logical execution flow, the underlying runtime can continue processing other threads or I/O operations, ensuring high throughput.

---

## ⚠️ Current Limitations

- **Ecosystem**: No support for external NPM packages; use `std:` or local modules.
- **Inference**: While powerful, some complex nested generic inferences may require explicit annotations.
- **Garbage Collection**: The current TejX Runtime uses a persistent object heap with basic cleanup; advanced generational GC is under development.
- **Web APIs**: DOM and other browser-specific APIs are NOT available as NovaJs targets native CLI and Server execution.

---

## 🛠 Usage & Development

### Building the Compiler

```bash
./build.sh
```

### Running Tests

```bash
./test_all.sh
```

### Compiling a File

```bash
./target/release/tejxc input.tx
./input # Run the generated binary
```
