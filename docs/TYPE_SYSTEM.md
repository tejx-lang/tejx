# Tejx Type System & Data Types Specification

Tejx is a structurally and nominally typed language designed to be extremely strict, explicit, and modern. The type system eliminates undefined behavior, implicitly degraded generic variables, and `any` fallbacks.

This document outlines the language's core types, generics mechanics, standard library structures, and strict type-checking behaviors.

---

## 1. Primitive Types

Tejx supports explicitly sized numeric primitives, boolean logic, and strings.

### Numeric Types

- **Integer Types**: `int8`, `uint8`, `int16`, `uint16`, `int32` (alias `int`), `uint32`, `int64`, `uint64`.
- **Floating-Point Types**: `float32` (alias `float`), `float64`.
- **Compile-Time Bounds Checking**: Numeric variables are strictly evaluated during assignment. Overflows throw compilation errors (`E0100`).

```typescript
let age: uint8 = 250; // OK
let height: int16 = 40000; // ERROR: Value 40000 out of bounds for int16 (-32768 to 32767)
```

### Boolean & Char

- **`bool`**: `true` and `false`.
- **`char`**: Single ASCII/UTF-8 byte representations.

### String

- **`string`**: Immutable UTF-8 text structure managed by the Tejx core runtime.
  Standard Library methods available globally on string prototypes (`rt_String_*`):
- `.toLowerCase()`, `.toUpperCase()`, `.trim()`, `.trimStart()`, `.trimEnd()`
- `.split(sep)`, `.replace(search, repl)`, `.substring(start, end)`
- `.indexOf(search)`, `.startsWith(prefix)`, `.endsWith(suffix)`
- `.padStart(len, char)`, `.padEnd(len, char)`, `.repeat(n)`

---

## 2. Array and Homogeneity

Arrays in Tejx are defined nominally via generics `Array<T>` or literal syntax `T[]`.

### Strict Literal Evaluation

Array literals are strictly homogenous. Tejx determines the type by evaluating the **common ancestor** of the inline arguments. If no exact match or parent match exists, a strict fallback error occurs rather than defaulting to `any`.

```typescript
let numbers: int[] = [1, 2, 3];
let bad_arr: int[] = [1, 2.5, "cat"]; // ERROR: Incompatible array elements `int32` and `string`
```

### Empty Array Initializers

Tejx prevents pseudo-types from polluting runtime allocations. You cannot assign an empty array without an explicit generic/literal target context.

```typescript
let items = []; // ERROR E0106: Cannot infer type for empty array.
let items: string[] = []; // OK
```

### Standard `Array<T>` API

Arrays support high-level data mutation and iteration through standard library generics:

- `.push(val: T): int`, `.pop(): T`, `.shift(): T`, `.unshift(val: T): int`
- `.indexOf(val: T): int`, `.includes(val: T): bool`
- `.slice(start, end): Array<T>`, `.concat(other: Array<T>): Array<T>`
- `.forEach(cb)`, `.map<U>(cb)`, `.filter(cb)`, `.reduce<U>(cb, init)`
- `.find(cb)`, `.findIndex(cb)`, `.some(cb)`, `.every(cb)`

---

## 3. Object Literals and Structs

Objects are mapped structurally. Tejx enforces **Exact Structural Consistency** on assignments.

### Structural Typing Checks

- **Shape Matching**: Only object literals perfectly matching the target prototype shape are accepted.
- **No Extra Properties**: Adding undefined keys internally or inline throws structural errors blocking assignment.

```typescript
type Point = { x: int; y: int };
let a: Point = { x: 10, y: 20 }; // OK
let b: Point = { x: 10, y: 20, z: 0 }; // ERROR: Extra property `z` not in `Point`
```

### Option<T> Key Assignment

If an object map defines a key as `Option<T>` (e.g., `id: Option<int>`), it represents an **optional property**. Missing keys assigned to `Option<T>` types will naturally be accepted during compile-time structural validation.

```typescript
type User = { name: string; handle: Option<string> };
let u: User = { name: "Alice" }; // OK - handle is safely optionally omitted
```

### Nullability (`None`, `T | None`, `Option<T>`)

By default, all variable types in Tejx are strictly non-nullable. You cannot assign `None` to an `int` or `string` directly.

Tejx provides explicit nullability through Union types and the `Option` generic wrapper:

- **`None` Literal**: The dedicated type representing an empty value reference.
- **Union Types (`T | None`)**: explicitly typing a variable as `string | None` or `int | None` allows it to accept either the typed value or the `None` literal.
- **`Option<T>` Wrapper**: Standard convention wrapper effectively identical to `T | None`, universally employed across Standard Library and Object prototypes to designate intentional optionality safely.

```typescript
let title: string = None; // ERROR: Type mismatch expected string, got None
let nickname: string | None = "Ace"; // OK
nickname = None; // OK
```

---

## 4. Functions and Parameters

Tejx strictly enforces function signatures, arity (parameter counts), and parameter types during compilation to eliminate runtime errors.

### Strict Parameter Count (Arity)

When invoking a function, the number of provided arguments must exactly match the number of declared parameters. If you provide too few or too many arguments, the compiler will throw an error (`E0109`).

```typescript
function greet(name: string, age: int): string {
  return "Hello";
}

greet("Alice"); // ERROR E0109: Function 'greet' expects 2 argument(s), but 1 were provided
greet("Alice", 25, true); // ERROR E0109: Function 'greet' expects 2 argument(s), but 3 were provided
```

### Optional Parameters (`Option<T>` and `T | None`)

To allow functions to accept fewer arguments without triggering an arity error, Tejx allows defining trailing parameters as explicitly optional using `Option<T>` or `T | None`.

If an argument is omitted during a function call, and the corresponding trailing parameter is typed as `Option<T>` or contains `| None`, Tejx automatically accepts the omission and implicitly binds the parameter's local variable to `None`. This seamlessly integrates with the type system's nullability checks.

```typescript
function register(username: string, referralCode: Option<string>): void {
  // referralCode is safely typed as `string | None`
}

register("Bob"); // OK: referralCode implicitly bounds to `None`
register("Charlie", "AliceCode"); // OK: referralCode bounds to `string`
```

_Note: Providing fewer arguments than required by the strictly non-optional parameters will still result in a strict arity compilation block._

### Parameter Type Resolution

Arguments passed into functions are strictly matched against their formal parameter definitions. Unlike dynamic languages, there is no silent type coercion. When interacting with Generic Functions (`<T>`), Tejx uses the parameter inputs to structurally resolve the generic parameters on the fly (e.g., calling `identity(42)` explicitly locks the return type of `identity<T>(val: T): T` to `int` without manual casting).

---

## 5. Generics and Type Tracking

Tejx evaluates `<T>` strictly at the compilation layer. There is no runtime type erasure or dynamic fallback to `any` internally.

- **Fixed Argument Constraints**: `class Box<T, U>` must be instantiated with exactly `<Type1, Type2>`. Providing missing generics halts compilation.
- **Exact Member Resolution**: Abstract Generic implementations (`T`) map locally inside their parent context. Attempting to trick generic tracking functions via partial generic overlaps strictly fails (`Array<int>` rejecting `string` insertion).

### Deep Generic Inference (Maps, Sets, Classes, Functions)

Tejx leverages structural typing to automatically resolve generic output types (`T`, `U`, `V`) without requiring explicit localized annotations when you retrieve or return values. This behavior spans across all natively supported structures and your own defined classes.

#### Data Structures (Maps and Sets)

When instantiating a bounded generic data structure, Tejx strictly refuses mismatched assignments via `.set()` or `.add()` and implicitly types the returned properties from `.get()` or `.values()` correctly.

```typescript
// Map<K, V> perfectly maps values without manual variable casting
let cache = new Map<string, int>();
cache.set("Alice", 95); // OK
cache.set("Bob", "100"); // ERROR: Expected `int` for `V`, got `string`
let aliceScore = cache.get("Alice"); // Automatically inferred as `int`

// Set<T> ensures unique constraints perfectly
let names = new Set<string>();
names.add("Alice"); // OK
names.add(95); // ERROR: Expected `string` for `T`, got `int32`
let allNames = names.values(); // Automatically inferred as `string[]`
```

#### Custom Classes and Functions

Instances of user-defined generic classes internally lock their generic scope across all exposed methods. Calls made to generic functions infer their required `T` properties based on the parameters passed during invocation.

```typescript
class Box<T> {
  value: T;
  constructor(val: T) {
    this.value = val;
  }
  unwrap(): T {
    return this.value;
  }
}

let intBox = new Box<int>(42);
intBox.value = "Hello"; // ERROR: Expected `int`
let payload = intBox.unwrap(); // Automatically inferred as `int`

// Standalone Generic Functions automatically assign types based on input parameters
function identity<T>(item: T): T {
  return item;
}
let textResult = identity<string>("Test"); // Output inferred as `string`
let intResult = identity(1234); // Output safely inferred as `int` from literal 1234
```

---

## 6. Classes and Inheritance

Classes follow an explicit prototype-based nominal structure.

- Support for `private`, `export`, and `static` visibility.
- `extends` forces parent structure inheritance overriding.
- `constructor()` initialization is dynamically type-checked.
- Read-only variable properties are safely guaranteed not to be overwritten.
- References to the containing scope are made strictly via `this`.

---

## 7. Maps, Sets, and Advanced Data Structures

Tejx provides a massive typed standard library targeting modern engineering structures under `stdlib/collections.tx`. All collections execute optimally against the native runtime layer (`rt_Map_*`, `rt_Set_*`).

### `Map<K, V>`

Explicit key-value dictionaries.

- **Methods**: `.set(key: K, val: V)`, `.get(key: K): V`, `.has(key: K): bool`, `.delete(key: K): bool`
- **Utilities**: `.keys(): K[]`, `.values(): V[]`, `.size()`, `.clear()`

**Generic Inference and Enforcement Example**:
When a Map is declared with explicit generics `Map<K, V>`, Tejx strictly enforces the insertion bindings preventing invalid data, and automatically infers the precise type of items retrieved.

```typescript
let scores = new Map<string, int>();

scores.set("Alice", 95); // OK: Matches K=string, V=int
scores.set("Bob", "100"); // ERROR: Expected `int` for value `V`, got `string`

// The return type of `.get()` is automatically and safely inferred as `int` without explicit variable annotation
let aliceScore = scores.get("Alice");
```

### `Set<T>`

Explicitly typed unique collections.

- **Methods**: `.add(val: T)`, `.has(val: T)`, `.delete(val: T)`, `.values(): T[]`, `.size()`

### Advanced Pre-Packaged Collections

Importing the collections module provides:

- **`Stack<T>`** and **`Queue<T>`**
- **`OrderedMap<K, V>`** and **`OrderedSet<T>`** (Insertion order preserved)
- **`MinHeap<T>`** and **`MaxHeap<T>`**
- **`PriorityQueue<T>`** (Built on MinHeap)
- **`BloomFilter`** (Probabilistic membership arrays)
- **`Trie`** (Flat array String-prefix lookup trees)

---

## 8. Automatic Type Inference (Data-Driven Assignment)

While explicit typing (`let x: int = 5;`) is rigorously checked, Tejx utilizes a powerful and safe intelligent type inference engine to automatically assign types based on initial data when annotations are omitted.

### Inference Rules

- **Variable Initialization**: If a variable is assigned immediately upon declaration without an explicit type, Tejx assumes exactly the type of the assigned data.
  ```typescript
  let explicit: string = "hello";
  let implicit = "world"; // Automatically inferred as `string`
  let float_val = 3.14; // Automatically inferred as `float`
  let bool_val = true; // Automatically inferred as `bool`
  ```
- **Uninitialized Variables**: Variables declared without initial data _must_ have an explicit type.
  ```typescript
  let name; // ERROR E0101: Type annotation required
  let active: bool; // OK - explicitly typed for later assignment
  ```

### Complex Structural Inference

- **Object Inference**: When an object literal is assigned to an untyped variable, Tejx automatically constructs an exact structural map representation of that object.
  ```typescript
  let user = { id: 1, name: "Alice" };
  // `user` is implicitly typed as { id: int, name: string }
  ```
- **Array Inference**: Arrays assigned without types automatically evaluate their lowest common ancestor element type.
  ```typescript
  let scores = [95, 80, 100]; // Inferred as `int[]`
  ```
  _Note: Because of array inference strictness, assigning an empty array `[]` without a target context will trigger an error `E0106`, because Tejx cannot safely determine what data the array is intended to hold._

* **Arrays of Objects**: When assigning variables with arrays containing object literals, Tejx recursively infers and requires structural integrity across all elements inside the array loop.
  ```typescript
  type Point = { x: int; y: int };
  // The explicit Array target forces all internal literal items to adhere to exactly the `Point` structure rules.
  let paths: Point[] = [
    { x: 1, y: 2 },
    { x: 3, y: 4 },
  ];
  ```

---

Tejx prioritizes stability, exact intent, and explicit boundaries over permissive silent casting—enabling highly durable, low-defect runtime binaries.
