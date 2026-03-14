### Part 1: The Anatomy of the Tejx Ecosystem

To build a language, you are building three distinct pillars that work together:

1.  **The Compiler (`tejxc`)**: Written in **Rust**. It is the brain. It parses `.tx` files, checks for type errors, and translates Tejx code into raw Machine Code.
2.  **The Runtime (`libtejx_rt`)**: Written in **C or Rust**. It is the engine. It handles low-level hardware tasks that Tejx cannot do safely on its own, like memory allocation (`malloc`/`free`), Garbage Collection, and CPU-specific SIMD instructions.
3.  **The Standard Library (`stdlib`)**: Written in **Tejx**. It is the toolbox. It provides the developer with high-level APIs like Arrays, Strings, Math, and File I/O.

---

### Part 2: The Directory Structures

#### A. The Architect's Workspace (Your GitHub Repository)

This is where you write the language. It is a Rust Cargo workspace.

```
/tejx-lang
├── Cargo.toml <-- Manages the Rust workspace
├── /src <-- RUST: The Brain
│ ├── lexer.rs <-- Converts text to tokens
│ ├── parser.rs <-- Builds the Abstract Syntax Tree (AST)
│ ├── semantic.rs <-- The Symbol Table (Type Checker)
│ └── codegen.rs <-- Emits machine code
├── /runtime <-- C/RUST: The Engine
│ ├── memory.c <-- Memory allocator (prevents Segfaults)
│ └── simd_array.c <-- High-speed CPU instructions
└── /stdlib <-- TEJX: The Toolbox
├── core/prelude.tx <-- Auto-loaded base types (String, Array)
├── core/array.tx <-- Array methods (filter, map)
└── std/math.tx <-- Optional libraries
```

#### B. The Developer's SDK (The Installation)

When a developer downloads Tejx (e.g., to `/usr/local/tejx`), they get a compiled, streamlined version.

```
/usr/local/tejx
├── /bin/tejxc <-- Your compiled Rust compiler executable
├── /runtime/tejx_rt.a <-- Your pre-compiled C/Rust runtime
└── /lib <-- The `.tx` Standard Library files
    ├── core/array.tx
    └── std/math.tx
```

#### C. The End-User's Device

Because Tejx uses Ahead-of-Time (AOT) compilation, the end-user needs **nothing**. The compiler fuses the developer's code, the standard library, and the runtime into a single file: `app.exe`.

---

### Part 3: High-Level vs. Low-Level Methods

- **Pure Tejx (`arr.filter`)**: Written entirely in `/stdlib/core/array.tx`. It uses standard loops and closures. The compiler reads this file and compiles the logic directly into the app.
- **Hardware-Optimized (`arr.find`)**: Written in C using SIMD instructions inside `/runtime/simd_array.c`.
  - To connect it, you use the **Foreign Function Interface (FFI)**.
  - In `array.tx`, you write: `extern function c_fast_find(...)`. This tells the compiler to leave a blank space in the machine code, which the Linker will later fill with the compiled C code.

---

### Part 4: The Symbol Table (The Compiler's Memory)

To enforce types and resolve methods, your Rust compiler uses a **Symbol Table**.

- **What it is**: A stack of Hash Maps (`Vec<HashMap<String, Symbol>>`) that tracks every variable, its type, and its scope.
- **Scope Management**: When the compiler hits a `{`, it pushes a new Hash Map to the stack. When it hits `}`, it pops it, destroying local variables.
- **Tracking `extern`**: The Symbol Table tracks if a method is `is_extern: true`. If true, the Code Generator knows to emit a system call to the C runtime.
- **Method Resolution**: When the compiler sees `my_arr.filter()`, it asks the Symbol Table: _"Is `my_arr` an Array? Does Array have a `filter` method?"_ If yes, the build continues. If no, it throws a `TypeError`.

---

### Part 5: File Discovery (How the compiler finds `array.tx`)

1.  **Dynamic Base Path**: The Rust compiler checks the OS environment (e.g., `$TEJX_HOME`) to find where the SDK is installed.
2.  **Hardcoded Core Files (The Prelude)**: Inside your Rust source code (`main.rs`), you hardcode a list of files that _must_ be loaded for the language to work. The compiler invisibly parses these files into the Symbol Table before it ever looks at the developer's code.
3.  **Dynamic Imports**: For non-core files (like `math.tx`), the compiler only searches the `$TEJX_HOME/src/std/` directory if it explicitly sees `import math;` in the developer's code.

---

### Part 6: The Full Build Lifecycle

When a developer runs `tejxc build app.tx`:

1.  **Boot & Locate**: The Rust compiler wakes up and finds `$TEJX_HOME`.
2.  **Prelude Injection**: It invisibly parses `prelude.tx` and `array.tx`, filling the Global Symbol Table with base types and methods.
3.  **User Parsing**: It parses `app.tx` into an Abstract Syntax Tree (AST).
4.  **Semantic Analysis**: It walks the AST, checking the Symbol Table to ensure every variable exists, every type matches, and every method call is valid.
5.  **Code Generation**: The Rust backend converts the validated AST into raw Machine Code. When it sees an object creation, it emits a call to your C runtime's memory allocator (`tejx_malloc`).
6.  **Linking**: The compiler calls the system Linker to stitch the generated machine code together with `/lib/libtejx_rt.a`.
7.  **Output**: A blazing-fast, standalone `app.exe` is born.
