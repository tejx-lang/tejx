# TejX Type System

This document describes the source-level type rules that the compiler currently enforces.

## Primitive Types

TejX supports these built-in primitive types:

| Source Type | Meaning |
| :-- | :-- |
| `int` | 32-bit signed integer (`int32`) |
| `int16` | 16-bit signed integer |
| `int64` | 64-bit signed integer |
| `int128` | 128-bit signed integer |
| `float` | 32-bit floating point (`float32`) |
| `float16` | 16-bit floating point |
| `float64` | 64-bit floating point |
| `bool` | boolean |
| `char` | character value |
| `string` | heap-managed UTF-8 string |
| `any` | dynamic escape hatch |

Notes:

- `bool` is the valid boolean type name.
- `boolean` is not a valid type.
- `float` is an alias for `float32`.
- `int` is an alias for `int32`.

## Composite Types

TejX also supports:

- `Optional<T>`
- dynamic arrays `T[]`
- fixed-size arrays `T[N]`
- structural object types such as `{ x: int; y: string }`
- classes and interfaces
- generic functions and generic classes
- function types

## Optional Values

`Optional<T>` is the only nullable source-level type.

```tx
let a: Optional<int> = None;
let b: Optional<string> = "tejx";
```

Rules:

- `Optional<T>` is supported.
- `Option<T>` is rejected. Use `Optional<T>`.
- Union syntax such as `int | None` is rejected.
- `None` may only flow into `Optional<T>` or dynamic contexts such as `any`.

### Default Initialization

Optional declarations may be left uninitialized:

```tx
let maybeId: Optional<int>;
```

That declaration defaults to `None`.

By contrast, non-optional typed declarations must be initialized:

```tx
let id: int = 10;      // OK
let bad: int;          // compile error
let obj: {x: int};     // compile error without initializer
```

## Narrowing

TejX narrows optional values only after an explicit `None` check:

```tx
let node: Optional<Node> = getNode();
if (node != None) {
    print(node.value);
}
```

Current rules:

- member access on `Optional<T>` requires a prior `!= None` check, unless you use `?.`
- indexing into `Optional<T>` requires a prior `!= None` check
- `instanceof` cannot be used directly on `Optional<T>`; first narrow with `!= None`
- optional chaining `?.` preserves optionality in the result

## Arrays

Dynamic arrays:

```tx
let xs: int[] = [1, 2, 3];
```

Fixed-size arrays:

```tx
let board: int[64] = [];
```

Rules:

- empty array literals need a target type
- array element types are checked strictly
- fixed arrays and dynamic arrays are distinct source-level forms

## Structural Objects

Structural object types are shape-checked:

```tx
type Point = { x: int; y: int };
let p: Point = { x: 1, y: 2 };
```

Important constraints:

- object members must match the declared shape
- extra unexpected properties are rejected
- object-typed variables must be initialized at declaration

## Classes and Interfaces

Classes are nominal and support inheritance:

```tx
class Animal {
    speak(): string { return "noise"; }
}

class Dog extends Animal {
    speak(): string { return "bark"; }
}
```

Interfaces define required method shapes and class contracts.

## Functions

Functions are fully typed:

```tx
function add(a: int, b: int): int {
    return a + b;
}
```

Rules:

- argument count is checked at compile time
- argument types are checked at compile time
- return values must match the declared return type
- trailing optional parameters should be modeled with `Optional<T>`

## Generics

Generic functions and classes are supported:

```tx
function identity<T>(value: T): T {
    return value;
}

class Box<T> {
    value: T;
    constructor(value: T) {
        this.value = value;
    }
}
```

The compiler tracks generic instantiations during semantic analysis and lowering. There is no source-level union fallback.

## `typeof` and `instanceof`

- `typeof(value)` returns the runtime type name
- `instanceof` checks class relationships across inheritance chains

`typeof` is useful for debugging and dynamic code paths, but it does not replace static typing.

## Type Inference

TejX can infer types from initializers:

```tx
let n = 10;
let name = "tejx";
let xs = [1, 2, 3];
```

Inference is intentionally conservative:

- empty arrays still need an explicit target type
- optionality is not inferred from missing initializers for non-optional types
- structural object inference follows the literal shape

## `any`

`any` is supported for dynamic or runtime-heavy paths, but it bypasses many static guarantees. Prefer specific types or `Optional<T>` where possible.
