# TejX Programming Language

[Home Page](https://tejx-lang.github.io/) | [Getting Started](https://tejx-lang.github.io/docs/get-started)

TejX is a high-performance, strictly typed programming language that compiles to native code via LLVM. It combines the ergonomics of modern TypeScript and Swift with the raw power and deterministic performance of systems languages.

## 📖 Documentation Index

| Guide                                             | Description                                       |
| :------------------------------------------------ | :------------------------------------------------ |
| **[Language Reference](docs/LANGUAGE.md)**        | Syntax, types, and core language features.        |
| **[Concurrency Guide](docs/CONCURRENCY.md)**      | Async/Await and Multi-Threading models.           |
| **[Internals & Architecture](docs/INTERNALS.md)** | Compiler pipeline, memory model, and performance. |
| **[Standard Collections](docs/COLLECTIONS.md)**   | Usage and memory layout of Array, Map, Set, etc.  |

---

## 🚀 Getting Started

### Prerequisites

- **Rust**: The compiler is built in Rust. [Install Rust](https://rustup.rs/)
- **LLVM & Clang**: Used for code generation and linking.
  - macOS: `brew install llvm`
  - Linux: `sudo apt-get install llvm clang`

### 🛠 How to Build

#### Native Compiler

Build the compiler in release mode:

```bash
cargo build --release
```

#### WASM Component

To build the WASM compiler for the web:

```bash
cd wasm
cargo build --release --target wasm32-unknown-unknown
```

---

## 💻 How to Use

### Compile & Run a TejX File

To compile a `.tx` file into a native executable:

```bash
./target/release/tejxc tests/hello.tx
./tests/hello
```

### CLI Command Reference

| Command                 | Description                                    |
| :---------------------- | :--------------------------------------------- |
| `-h`, `--help`          | Show this help message                         |
| `-v`, `--version`       | Show version information                       |
| `-o`, `--output <file>` | Specify output file name                       |
| `-c`, `--compile`       | Compile only (generate `.o` file); do not link |
| `--disable-async`       | Disable async/await features                   |
| `--emit-mir`            | Print MIR to stderr                            |
| `--emit-llvm`           | Print LLVM IR to stderr                        |
| `--target <target>`     | Specify target (e.g., `wasm`)                  |

### Run the Test Suite

To verify the compiler against the comprehensive test suite:

```bash
./test.sh
```

You can also run specific subsets:

- **Positive Tests**: `./test.sh --positive`
- **Negative Tests**: `./test.sh --negative`
- **Problem Tests**: `./test.sh --problems`

---

<div align="center">
    Built with ❤️ by the TejX Team
</div>
