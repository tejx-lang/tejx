# TejX Language Guide

TejX is a statically typed language that compiles ahead of time to native code through LLVM. The language is strict by default: no implicit nullability, no hidden truthy/falsy coercions, and no source-level union types.

This guide is the high-level entry point. For the exact rules, see:

- `TYPE_SYSTEM.md` for type rules and `Optional<T>`
- `MODULE_SYSTEM.md` for imports and implicit core modules
- `CONCURRENCY.md` for async/await and native threads
- `MEMORY_MODEL.md` for runtime representation and GC

## Core Syntax

Variables use `let` for mutable bindings and `const` for immutable bindings:

```tx
let count: int = 10;
const pi: float64 = 3.141592653589793;
```

Functions, lambdas, classes, interfaces, and type aliases are all first-class language features:

```tx
type Point = { x: int; y: int };

function add(a: int, b: int): int {
    return a + b;
}

let square = (n: int): int => n * n;

interface Greeter {
    greet(name: string): string;
}

class Robot {
    model: string;
    constructor(model: string) {
        this.model = model;
    }
}
```

## Built-in Types

TejX supports:

- `int` (`int32`) and `int64`
- `float` (`float32`) and `float64`
- `bool`
- `char`
- `string`
- `any`
- arrays, objects, classes, interfaces, and generic types

`bool` is the boolean type. `boolean` is not valid source syntax.

## Nullability

TejX uses `Optional<T>` as the only nullable source-level type:

```tx
let name: Optional<string> = None;
let id: Optional<int>;
```

Rules:

- `Optional<T>` is the only supported nullable type.
- `Option<T>` is not supported.
- Union syntax such as `T | None` is not supported.
- `let x: Optional<T>;` defaults to `None`.
- Non-optional typed declarations must be initialized.

To use an optional value as a non-optional one, first prove it is not `None`:

```tx
let node: Optional<Node> = getNode();
if (node != None) {
    print(node.value);
}
```

Optional chaining is also available:

```tx
print(node?.value);
```

## Control Flow

TejX supports:

- `if`, `else if`, `else`
- `while`
- C-style `for`
- `try`, `catch`, `finally`

Example:

```tx
try {
    risky();
} catch (err) {
    print(err);
} finally {
    cleanup();
}
```

## Objects, Classes, and Interfaces

TejX supports both structural object types and nominal classes.

Structural object types:

```tx
type User = { id: int; name: string };
let u: User = { id: 1, name: "Alice" };
```

Class-based OOP:

```tx
class Animal {
    speak(): string { return "noise"; }
}

class Dog extends Animal {
    speak(): string { return "bark"; }
}
```

Useful runtime checks:

- `typeof(value)`
- `value instanceof ClassName`

For optional values, check `!= None` before using `instanceof`.

## Arrays

TejX supports both dynamic and fixed-size arrays:

```tx
let xs: int[] = [1, 2, 3];
let grid: int[16] = [];
```

Common array operations are provided by the core library through implicit imports.

## Functions and Generics

Typed functions and generics are built into the language:

```tx
function identity<T>(value: T): T {
    return value;
}

let n = identity(10);
let s = identity("tejx");
```

Generic classes and standard library containers follow the same pattern:

```tx
let map = new Map<string, int>();
map.set("a", 1);
```

## Async and Threads

TejX supports both:

- `async` / `await` for non-blocking workflows
- native threads through `std:thread` for parallel CPU work

Use async for I/O-style workflows and threads for CPU-bound parallelism. See `CONCURRENCY.md` for details.

## Imports

Imports are static and resolved at compile time:

```tx
import { Map } from "std:collections";
import helper from "./helper.tx";
```

Core library files such as `prelude.tx`, `array.tx`, and `string.tx` are injected automatically for normal modules. See `MODULE_SYSTEM.md`.
