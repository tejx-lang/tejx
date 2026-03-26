# TejX Compiler Internals

This document is the compiler walkthrough for TejX. It explains what happens at each phase, which Rust modules do the work, what gets rewritten, and how the generated program eventually reaches LLVM and the runtime.

Use it together with:

- `TYPE_SYSTEM.md` for the language rules that the semantic pass enforces
- `MODULE_SYSTEM.md` for import/export behavior
- `MEMORY_MODEL.md` for the runtime value layout that codegen targets

## One-Screen Pipeline

The entry point is `src/compiler/main.rs`. The handoff is:

```text
.tx source
  -> Lexer::tokenize
  -> Parser::parse_program
  -> Lowering::resolve_imports
  -> TypeChecker::check
  -> Lowering::lower          (AST -> HIR)
  -> MIRLowering::lower       (HIR -> MIR)
  -> MIROptimizer::optimize
  -> CodeGen::generate_with_blocks
  -> Linker::link
  -> native executable
```

The important consequence is that TejX does not go straight from AST to LLVM. Most of the language semantics live in the middle of the pipeline:

- import injection and module flattening
- type checking and narrowing
- generic instantiation discovery
- AST-to-HIR desugaring
- HIR-to-MIR control-flow lowering
- async state-machine lowering
- runtime-aware ABI casts and GC-root management during LLVM emission

## Running Example

This small program is enough to show most of the pipeline:

```tx
function add(a: int, b: int): int {
    return a + b;
}

function main() {
    let [x, y] = [1, 2];
    let sum = add(x, y);
    print(sum);
}
```

Later sections will also use focused examples for generics, optional chaining, lambdas, and async.

## Where The Compiler Logic Lives

| Area | Main files |
| --- | --- |
| CLI orchestration | `src/compiler/main.rs` |
| tokens, lexer, parser, AST | `src/compiler/frontend/token.rs`, `lexer.rs`, `parser.rs`, `ast.rs` |
| import resolution and AST lowering | `src/compiler/middle/lowering/mod.rs`, `imports.rs`, `stmt.rs`, `expr.rs`, `func.rs`, `class.rs`, `patterns.rs`, `async_desugar.rs` |
| semantic analysis | `src/compiler/middle/semantic/mod.rs` and the files under `semantic/` |
| HIR | `src/compiler/middle/hir/mod.rs` |
| MIR + optimization | `src/compiler/middle/mir/mod.rs`, `lowering.rs`, `opt.rs` |
| LLVM IR generation | `src/compiler/backend/codegen/mod.rs`, `func.rs`, `inst.rs`, `memory.rs`, `utils.rs` |
| final link step | `src/compiler/backend/linker.rs` |
| runtime intrinsics used by compiler | `src/compiler/common/intrinsics.rs` |

## 1. CLI Orchestration In `main.rs`

`src/compiler/main.rs` is a coordinator, not the place where language semantics live.

It is responsible for:

- parsing flags like `--emit-mir`, `--emit-llvm`, `--stdlib-path`, and `--runtime-path`
- reading the entry file from disk
- running each phase in order
- stopping on the first phase that reports diagnostics
- deduplicating repeated diagnostics with `unique_diagnostics`
- writing the generated LLVM IR to `<output>.ll`
- invoking the platform linker through `backend/linker.rs`

The compiler resolves toolchain paths through `src/compiler/common/paths.rs`:

- stdlib path resolution: explicit flag -> local `lib/` -> installed layout -> `$HOME/.tejx/lib`
- runtime archive resolution: explicit flag -> installed layout -> `$HOME/.tejx/runtime/tejx_rt.a`

That path logic matters because `resolve_imports` depends on a real stdlib root, and the linker depends on a real `tejx_rt.a`.

## 2. Lexing: Characters Become Tokens

Files:

- `src/compiler/frontend/lexer.rs`
- `src/compiler/frontend/token.rs`

`Lexer::new` builds the keyword table. That table is already language policy, not just mechanics. For example:

- `bool` is a real keyword
- `Option` and `Optional` are both recognized, so the parser can reject legacy spellings precisely
- async, class, import, export, `Some`, `None`, `instanceof`, and the rest of the syntax surface are all registered here

`Lexer::tokenize` then walks the source one character at a time and produces `Token` values with:

- `token_type`
- raw `value`
- `line`
- `column`

From the running example:

```tx
function add(a: int, b: int): int { return a + b; }
```

the lexer emits a stream conceptually like:

```text
Function Identifier(add) OpenParen Identifier(a) Colon TypeInt Comma
Identifier(b) Colon TypeInt CloseParen Colon TypeInt OpenBrace
Return Identifier(a) Plus Identifier(b) Semicolon CloseBrace EndOfFile
```

The lexer also emits diagnostics directly. If lexing fails, `main.rs` never constructs a parser.

## 3. Parsing: Tokens Become The Source AST

Files:

- `src/compiler/frontend/parser.rs`
- `src/compiler/frontend/ast.rs`

TejX uses a hand-written recursive-descent parser.

The top-level flow is:

- `Parser::parse_program`
- `parse_declaration`
- specific declaration parsers such as `parse_function_declaration`, `parse_class_declaration`, `parse_import_statement`
- expression parsers organized by precedence, ending in `parse_primary`

The AST types live in `ast.rs`. The important split is:

- `Program { statements }`
- `Statement`
- `Expression`
- `TypeNode`

For the example above, the parser builds roughly:

```text
Program
  FunctionDeclaration "add"
    params: a: int, b: int
    return: int
    body:
      ReturnStmt
        BinaryExpr(a, +, b)

  FunctionDeclaration "main"
    body:
      VarDeclaration pattern=[x, y], initializer=[1, 2]
      VarDeclaration pattern=sum, initializer=CallExpr(add, [x, y])
      ExpressionStmt CallExpr(print, [sum])
```

### Parser-Level Language Rules

Some language rules are enforced before semantic analysis:

- `parse_type_annotation` rejects general union types
- `T | None` is diagnosed and normalized toward `Optional<T>`
- `Option<T>` is diagnosed in favor of `Optional<T>`
- nested generic closing tokens like `>>` are repaired for parsing nested generics
- `OptionalCallExpr`, `OptionalMemberAccessExpr`, `OptionalArrayAccessExpr`, `NullishCoalescingExpr`, lambda expressions, destructuring patterns, and `for..of` are all represented explicitly in the AST

This is important because later passes do not have to recover syntax intent from raw tokens.

## 4. Import Resolution: The AST Is Expanded Before Type Checking

Files:

- `src/compiler/middle/lowering/imports.rs`
- `src/compiler/common/paths.rs`

TejX resolves imports before semantic analysis. `Lowering::resolve_imports` takes the parsed AST and recursively produces a merged statement list.

This pass does more than "open imported files":

1. It injects core layers.

Every file gets the core base layer, and user files also get the prelude. Non-core files additionally get `array.tx` and `string.tx`.

2. It resolves import paths.

- `std:time` becomes `<stdlib>/std/time.tx`
- relative imports are resolved from the importing file
- missing `.tx` is appended automatically

3. It recursively lexes and parses imported modules.

Import resolution literally re-enters the frontend for every imported file.

4. It prevents cycles.

`import_stack` is used to report circular dependencies.

5. It validates exports and aliases.

When the source says:

```tx
import { foo as bar } from "./util";
```

the imported module is merged, then exported declarations are renamed in-place so later passes just see `bar`.

The output of this stage is still AST, but it is now a single, expanded AST.

## 5. Semantic Analysis: The Compiler Decides Whether The Program Is Legal

Files:

- `src/compiler/middle/semantic/mod.rs`
- `src/compiler/middle/semantic/class.rs`
- `src/compiler/middle/semantic/stmt.rs`
- `src/compiler/middle/semantic/expr.rs`
- `src/compiler/middle/semantic/compat.rs`
- `src/compiler/middle/semantic/generics.rs`

`TypeChecker::check` is a two-pass walk over the merged AST.

### Pass 1: Collect Declarations

`collect_declarations` hoists the names and type shape of:

- functions
- classes
- interfaces
- type aliases
- members and constructors

The symbol table is `Vec<HashMap<String, Symbol>>`, one map per scope.

This is where the compiler learns things like:

- class inheritance links
- interface member contracts
- generic parameter lists and bounds
- method signatures
- aliased types

### Pass 2: Check Statements And Expressions

`check_statement` and `check_expression` enforce the real language rules:

- variable declaration legality
- type compatibility
- const reassignment errors
- callable vs non-callable values
- member existence and access control
- constructor and override rules
- async/await legality
- `break` / `continue` placement
- top-level executable statement rejection
- optional access rules
- object literal compatibility
- generic argument counts and bounds

### Example: Destructuring Is Checked Structurally

For:

```tx
let [x, y] = [1, 2];
```

the type checker verifies:

- the initializer is an array-like value
- the element type is valid for each binding
- the pattern is legal

The lowering pass later turns it into ordinary assignments, but the type checker sees the original structure.

### Example: Narrowing

If the source contains:

```tx
let maybeName: Optional<string> = getName();
if (maybeName != None) {
    print(maybeName.length());
}
```

`get_narrowing_from_condition` records that the then-branch narrows `maybeName` from `Optional<string>` to `string`. That narrowing is reintroduced in lowering so HIR and later stages preserve the type knowledge.

### Example: Generic Instantiations Are Recorded Here

For:

```tx
function id<T>(x: T): T { return x; }

let a = id(1);
let b = id("hi");
```

the type checker records which concrete type arguments were actually used. Lowering consumes those instantiation sets and creates monomorphized declarations.

### Important Design Detail

The semantic pass still operates on source-level constructs. It does not build MIR. Its job is to answer "is this program legal, and what types flow through it?" The lowering pass is where the language is rewritten into compiler-friendly forms.

## 6. AST -> HIR Lowering: Source Syntax Becomes A Typed, More Explicit Tree

Files:

- `src/compiler/middle/lowering/mod.rs`
- `src/compiler/middle/hir/mod.rs`
- `src/compiler/middle/lowering/stmt.rs`
- `src/compiler/middle/lowering/expr.rs`
- `src/compiler/middle/lowering/func.rs`
- `src/compiler/middle/lowering/class.rs`
- `src/compiler/middle/lowering/patterns.rs`
- `src/compiler/middle/lowering/async_desugar.rs`
- `src/compiler/frontend/ast_transformer.rs`

HIR is still tree-shaped, but it is much closer to the backend than the source AST.

`HIRStatement` and `HIRExpression` make several things explicit:

- every expression already carries a `TejxType`
- loops are normalized around `HIRStatement::Loop`
- function names are mangled
- destructuring is desugared
- object and array literals are lowered to runtime-aware HIR nodes
- optional and nullish constructs are turned into explicit conditional logic

### Lowering Passes In `Lowering::lower`

`Lowering::lower` is not one sweep. It performs several sub-passes:

1. Scan for variadic functions and detect whether user `main` is async.
2. Register top-level functions, classes, and variables into lowering-side symbol tables.
3. Monomorphize generic declarations to a fixed point.
4. Lower declarations and statements into HIR.
5. Append generated lambdas and nested functions.
6. Synthesize `tejx_main`, the real entry function.

### Name Mangling

The lowering pass gives backend-stable names:

- user functions become `f_<name>`
- class methods become `f_<Class>_<method>`
- globals use `g_<name>`
- local bindings receive unique suffixes to avoid collisions

From the running example:

- `add` becomes `f_add`
- `main` becomes `f_main`

### Destructuring Example

`lower_binding_pattern` rewrites:

```tx
let [x, y] = [1, 2];
```

into the HIR equivalent of:

```text
let destructure_tmp = [1, 2]
let x = destructure_tmp[0]
let y = destructure_tmp[1]
```

Object destructuring is handled the same way, except it becomes member reads instead of index reads.

### Nullish Coalescing Example

This source:

```tx
let len = maybeName?.length() ?? 0;
```

is not preserved as a special backend instruction. Lowering turns it into explicit HIR conditionals.

There are two patterns in the current compiler:

- `?.member` and `?.[index]` are lowered into `HIRExpression::If` that checks for non-`None`
- `?.()` is lowered into `HIRExpression::OptionalChain`, which the MIR pass later maps to a runtime helper

### Generic Monomorphization

Generic lowering is concrete, not erased by default.

`monomorphize_to_fixed_point` repeatedly:

- looks at instantiations discovered by the semantic pass
- clones the original AST declaration
- substitutes generic `TypeNode`s with `TypeSubstitutor`
- emits specialized declarations with mangled names

This is why the backend mostly sees concrete function and class shapes rather than open type parameters.

### Classes

`register_class` collects:

- instance fields
- static fields
- methods
- constructor signatures
- inheritance links
- generic parameter names

`lower_class_declaration` then emits ordinary functions:

- `f_Class_constructor`
- `f_Class_method`

Constructor lowering also injects instance field initialization into the constructor body, after `super()` if needed.

### Lambdas And Captures

For:

```tx
let offset = 10;
let plus = (x: int) => x + offset;
```

lowering does all of the following:

- assigns a synthetic name like `lambda_0`
- prepends an implicit `__env` parameter
- records captured names in `captured_vars_by_owner`
- remembers which owner environment each lambda should read from
- appends the generated lambda function to the final HIR function list

That capture metadata is later passed into codegen so closure environments can be materialized.

### Async Functions

Async lowering is one of the most important transformations in the compiler.

For:

```tx
async function compute(x: int): Promise<int> {
    let y = await slow(x);
    return y + 1;
}
```

`lower_async_function` and `lower_async_function_impl` generate:

- a wrapper function, still exposed as `f_compute`
- a worker function, `f_compute_worker`
- a context array that stores promise id, state, parameters, and padded local slots
- a try/catch wrapper that resolves or rejects the promise

At the HIR level, async is already leaving the source model and moving toward a resumable state machine.

### The Real Entry Function

The last step of `Lowering::lower` synthesizes `tejx_main`.

That function:

- runs lowered top-level initialization statements
- calls user `f_main` if it exists
- awaits `f_main` first if the user wrote `async function main()`

This is why the runtime entry point is not the user's `main` directly.

## 7. HIR -> MIR Lowering: Trees Become Basic Blocks

Files:

- `src/compiler/middle/mir/mod.rs`
- `src/compiler/middle/mir/lowering.rs`

MIR is the compiler's control-flow representation.

Its core types are:

- `MIRFunction`
- `BasicBlock`
- `MIRInstruction`
- `MIRValue`

The instruction set is intentionally small:

- `Move`
- `BinaryOp`
- `Branch`
- `Jump`
- `Return`
- `Call`
- `IndirectCall`
- `LoadMember` / `StoreMember`
- `LoadIndex` / `StoreIndex`
- `Throw`
- `Cast`
- `TrySetup`
- `PopHandler`

### What `MIRLowering::lower_function` Does

For each HIR function, MIR lowering:

- resets function-local state
- records parameter variables and types
- pre-collects locals
- creates the entry block
- lowers HIR statements and expressions into block-based instructions
- terminates unterminated blocks with a `Return`

### Running Example In MIR Terms

The running example becomes the shape below. This is schematic MIR, not a verbatim dump:

```text
function f_add(a, b):
  entry:
    t0 = BinaryOp a + b
    Return t0

function f_main():
  entry:
    arr = Call rt_Array_constructor_v2(...)
    StoreIndex arr[0] = 1
    StoreIndex arr[1] = 2
    destructure_tmp = arr
    x = LoadIndex destructure_tmp[0]
    y = LoadIndex destructure_tmp[1]
    sum = Call f_add(x, y)
    _ = Call print(sum)
    Return

function tejx_main():
  entry:
    _ = Call f_main()
    Return
```

The exact temp names differ, but the shape is right: no AST nesting, explicit calls, explicit loads/stores, explicit blocks.

### Control Flow

Source control flow becomes branches and blocks.

Example:

```tx
let z = cond ? a : b;
```

becomes a result temp plus:

- `Branch` to then/else blocks
- `Move` of the chosen value into the temp
- `Jump` to a shared exit block

Loops are similarly lowered into condition/body/exit blocks.

### Objects, Arrays, Members, And Indexes

MIR is where runtime-aware data operations become explicit:

- object literals call `rt_object_new`
- array literals call `rt_Array_constructor_v2`
- `obj.x` becomes `LoadMember`
- `obj.x = y` becomes `StoreMember`
- `arr[i]` becomes `LoadIndex`
- `arr[i] = y` becomes `StoreIndex`

### Async In MIR

Async workers get the most specialized MIR path.

When MIR lowering sees `HIRExpression::Await` inside an async worker:

1. it stores the next state id into `ctx[1]`
2. it calls `rt_promise_await_resume(promise, worker_ptr, ctx)`
3. it returns to the event loop
4. it creates a continuation block for the resume point
5. that continuation block reads the fulfilled value with `rt_promise_get_value`

That is the real async state machine. It is not an abstract concept in TejX; it is encoded directly in MIR blocks and runtime calls.

### Exceptions

`TrySetup`, `Throw`, and `PopHandler` carry exception structure into the backend. MIR blocks also track an `exception_handler` so codegen knows when handler-aware cleanup is needed.

## 8. MIR Optimization: Small, Local, Practical

File:

- `src/compiler/middle/mir/opt.rs`

`MIROptimizer::optimize` repeatedly applies a small set of local optimizations:

- promote eligible dynamic arrays to fixed arrays
- rewrite local string appends
- constant fold simple operations
- eliminate dead code
- remove unused variables

This is not a giant global optimizer. The design assumes LLVM will do the heavy backend optimization later. TejX's own MIR optimizer mostly cleans obvious compiler-generated noise before IR generation.

## 9. LLVM IR Generation: MIR Becomes Runtime-Aware LLVM

Files:

- `src/compiler/backend/codegen/mod.rs`
- `src/compiler/backend/codegen/func.rs`
- `src/compiler/backend/codegen/inst.rs`
- `src/compiler/backend/codegen/memory.rs`
- `src/compiler/backend/codegen/utils.rs`

`CodeGen::generate_with_blocks` is the top-level backend entry.

It performs several jobs before a single instruction is emitted:

- clears per-module backend state
- records captured-variable lists per function
- discovers object shapes for fixed-layout fast paths
- writes the LLVM module header, datalayout, and target triple
- records function signatures and global types

### Function Emission

`gen_function_v2` emits each `MIRFunction`.

The function-level work includes:

- deterministic `alloca` creation for params and locals
- storing parameters into allocas
- registering GC-managed locals as runtime roots
- creating closure environments when a function captures outer values
- building or reusing lambda environments

That last point is important: TejX codegen is not emitting "plain LLVM locals only". It is coordinating with the GC and closure runtime at function entry.

### ABI Casting And Boxing

A major part of the backend is `emit_abi_cast` in `utils.rs`.

Why it exists:

- LLVM wants native integers, floats, and booleans for arithmetic
- the TejX runtime often wants `i64` handles or boxed values
- strings, arrays, objects, closures, and optionals are GC-managed runtime values

So codegen constantly translates between:

- native LLVM scalar values
- boxed runtime values
- generic `any`-style `i64` slots

Examples:

- numeric ops stay native where possible
- passing a primitive into a runtime helper may require boxing
- loading a numeric field from a generic object may require unboxing
- strings may need conversion through runtime helpers before concatenation or property access

### Instruction Emission

`gen_instruction_v2` dispatches every MIR instruction.

Typical mappings:

- `BinaryOp` -> native LLVM arithmetic or comparison, plus runtime helpers for dynamic cases
- `Call` -> direct call with argument casting and root management
- `IndirectCall` -> closure-pointer extraction and adapter invocation
- `LoadMember` / `StoreMember` -> fixed-layout fast path when safe, otherwise runtime property helpers
- `LoadIndex` / `StoreIndex` -> array/string access helpers plus bounds-sensitive logic

### Fixed-Layout Fast Paths

The backend does escape and shape analysis to decide whether some object accesses can skip dynamic property helpers.

That logic lives mostly in `func.rs` and `memory.rs`.

If the compiler can prove:

- the object shape is known
- the value does not escape in incompatible ways
- the field layout is stable

then it can emit a direct layout-based access pattern instead of falling back to `rt_get_property` / `rt_set_property`.

### Closures

Closure codegen uses the capture info from lowering:

- captured values are stored in an environment array
- lambdas reuse or receive that environment via the implicit `__env`
- function pointers are wrapped with `rt_closure_from_ptr`
- `ensure_closure_adapter` builds adapter functions so the runtime closure calling convention matches the compiled function signature

That is the bridge between source lambdas and callable runtime objects.

### GC Root Discipline

The backend is tightly coupled to the runtime's memory model.

For GC-managed locals and temporaries, codegen emits calls such as:

- `rt_push_root`
- `rt_pop_roots`

and in some stores:

- write barriers

This is why `MEMORY_MODEL.md` is required reading for backend work. The LLVM IR is only correct because it follows the runtime's root-scanning rules.

## 10. Linking: `.ll` To Native Binary

File:

- `src/compiler/backend/linker.rs`

The linker stage is intentionally simple and external-tool driven.

`Linker::link`:

1. finds a C/LLVM toolchain binary (`cc`, `clang`, or `gcc`)
2. converts `.ll` to assembly with `-S -O3`
3. assembles the `.s` file into `.o`
4. links all objects plus `tejx_rt.a`
5. adds platform libraries/frameworks

Platform-specific additions:

- Linux: `-lm -lpthread -ldl`
- macOS: `Security`, `CoreFoundation`, `SystemConfiguration`

So the compiler proper stops at LLVM text generation. Native object emission and final executable linking are delegated to the system toolchain.

## 11. Diagnostics: Every Phase Reports Through The Same Shape

Files:

- `src/compiler/common/diagnostics.rs`
- `src/compiler/main.rs`

Every phase emits `Diagnostic` values with:

- file
- line
- column
- error code
- hint
- optional inline label

`Diagnostic::report` prints a rustc-style error snippet with source context.

`main.rs` also deduplicates diagnostics with `unique_diagnostics` before printing, which matters because recursive import resolution and multi-pass checking can surface the same root problem more than once.

## 12. How To Inspect The Pipeline Yourself

Useful commands from the repo root:

```bash
./target/release/tejxc tests/test.tx --emit-mir
./target/release/tejxc tests/test.tx --emit-llvm
./target/release/tejxc tests/test.tx -c
```

What to use each for:

- `--emit-mir`: inspect control flow, async lowering, and runtime calls before LLVM
- `--emit-llvm`: inspect boxing, unboxing, root management, and direct vs runtime property access
- `-c`: stop after compilation artifacts instead of producing a final executable

When debugging a compiler change, the fastest path is usually:

1. verify the AST/parser behavior
2. inspect semantic diagnostics
3. inspect MIR shape
4. inspect LLVM IR only after MIR looks correct

If MIR is wrong, LLVM inspection is usually a waste of time.

## 13. Mental Model For Changing The Compiler

If you want to change a feature, start in the phase that should own the behavior.

| If you need to change... | Start here |
| --- | --- |
| syntax, precedence, or new tokens | `frontend/lexer.rs` and `frontend/parser.rs` |
| typing, narrowing, generics, access control | `middle/semantic/` |
| source desugaring, name mangling, class/lambda/async rewrites | `middle/lowering/` |
| block structure, state-machine shape, low-level calls | `middle/mir/lowering.rs` |
| instruction cleanup before backend | `middle/mir/opt.rs` |
| boxing, unboxing, LLVM types, closure ABI, GC roots | `backend/codegen/` |
| final native link behavior | `backend/linker.rs` |

The compiler is best understood as a series of increasingly explicit representations:

- AST preserves source meaning
- semantic analysis proves legality and types
- HIR rewrites source constructs into typed compiler forms
- MIR makes control flow and runtime operations explicit
- LLVM IR maps that explicit plan onto the runtime ABI

That is the real shape of the TejX compiler.
