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

### `std:os`

Operating system integration:

- `args()`: Returns command-line arguments.
- `env()`: Access environment variables.

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
./target/release/tejxr input.tx
./input # Run the generated binary
```
