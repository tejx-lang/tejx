# TejX Language Specification & Syntax Manual

**TejX** is a statically typed, high-performance programming language compiling ahead-of-time (AOT) to native machine code via LLVM. It features an M:N work-stealing virtual thread scheduler (similar to Go goroutines), an automatic generational garbage collector, strict typing with explicit nullability via `Optional<T>`, and a rich standard library.

---

## Table of Contents

1. [Program Structure & Execution Model](#1-program-structure--execution-model)
2. [Lexical Structure & Tokens](#2-lexical-structure--tokens)
3. [Type System](#3-type-system)
4. [Variables & Declarations](#4-variables--declarations)
5. [Expressions & Operators](#5-expressions--operators)
6. [Control Flow](#6-control-flow)
7. [Functions & Closures](#7-functions--closures)
8. [Object-Oriented Programming](#8-object-oriented-programming)
9. [Error Handling & Exceptions](#9-error-handling--exceptions)
10. [Modules, Imports & Exports](#10-modules-imports--exports)
11. [Concurrency Model](#11-concurrency-model)
12. [Memory Model & Garbage Collection](#12-memory-model--garbage-collection)
13. [Foreign Function Interface (FFI)](#13-foreign-function-interface-ffi)

---

## 1. Program Structure & Execution Model

### 1.1 Source Files & Entry Point
TejX source files use the `.tx` file extension. The entry point of an executable TejX program is the `main` function:

```tx
function main(): void {
    print("Hello, TejX!");
}
```

The return type `: void` is optional on `main`.

### 1.2 Execution Lifecycle
1. **Compilation (`tejxc`)**: Source `.tx` files are parsed into an Abstract Syntax Tree (AST), validated by the type checker, optimized, and compiled to native object files via LLVM.
2. **Runtime Initialization (`tejx_runtime_main`)**:
   - Panic hooks and signal handlers are registered.
   - The file descriptor limit is raised (up to `65535` on Unix).
   - Heap memory limits (`-Xmx` / `--tejx-heap`) are parsed.
   - The M:N virtual thread runtime (`vthread`) is initialized, spawning worker threads matching CPU cores (`available_parallelism`).
   - The epoll/kqueue network I/O poller thread starts.
   - The generational Garbage Collector (`gc`) initializes its memory pools (Eden, Survivor, Old Generation, Large Object Space).
   - Type metadata and global constants are registered.
   - The initial virtual thread executes `tejx_main()`.
   - On completion, stdout/stderr are flushed and the process exits cleanly.

---

## 2. Lexical Structure & Tokens

### 2.1 Comments
TejX supports single-line and multi-line comments:

```tx
// Single-line comment

/*
 * Multi-line comment
 */
```

### 2.2 Identifiers
Identifiers begin with an ASCII letter (`a`-`z`, `A`-`Z`) or underscore (`_`), followed by letters, digits (`0`-`9`), or underscores:

```tx
let user_name = "Alice";
let _internal_id = 1001;
let totalCount2 = 50;
```

### 2.3 Reserved Keywords
| Category | Keywords |
|---|---|
| **Declarations** | `let`, `const`, `function`, `class`, `interface`, `enum`, `type`, `namespace`, `extern` |
| **Control Flow** | `if`, `else`, `while`, `for`, `of`, `to`, `break`, `continue`, `return`, `switch`, `case`, `default` |
| **OOP** | `new`, `this`, `super`, `constructor`, `extends`, `implements`, `abstract`, `public`, `private`, `protected`, `static`, `instanceof` |
| **Modules** | `import`, `export`, `from` |
| **Error Handling** | `try`, `catch`, `finally`, `throw` |
| **Nullability & State** | `Optional`, `Some`, `None`, `del` |
| **Primitives** | `int`, `int8`, `uint8`, `int16`, `uint16`, `uint`, `int64`, `uint64`, `int128`, `uint128`, `float`, `float16`, `float64`, `char`, `bool`, `string`, `void`, `any`, `as` |
| **Literals** | `true`, `false`, `None` |

### 2.4 Literals

#### Integer Literals
```tx
let dec = 42;
let neg = -15;
let zero = 0;
```

#### Floating-Point Literals
```tx
let f1 = 3.14159;
let f2 = 0.5;
let f3 = -10.25;
```

#### Boolean Literals
```tx
let isReady = true;
let isDone = false;
```

#### Character Literals
Characters represent a single Unicode code point enclosed in single quotes:
```tx
let ch: char = 'A';
let newline: char = '\n';
let tab: char = '\t';
```

#### String Literals
Strings are UTF-8 sequences enclosed in double quotes:
```tx
let greeting = "Hello, World!\n";
let path = "C:\\projects\\tejx";
let escaped = "Quotes: \"inside\" string";
```

#### Template Strings
Template strings use backticks (`` ` ``) and support variable/expression interpolation with `${...}`:
```tx
let name = "TejX";
let version = 1;
let message = `Welcome to ${name} version ${version}! Result: ${2 + 2}`;
```

#### The `None` Literal
`None` denotes the absence of a value for `Optional<T>` types:
```tx
let missing: Optional<string> = None;
```
> **Note**: TejX strictly disallows `null` and `undefined`. Attempting to use `null` or `undefined` triggers a compile-time error with a fix suggestion to use `None`.


---

## 3. Type System

TejX features a static, sound type system. Types are checked at compile time with zero hidden conversions or JavaScript-like truthy/falsy coercions.

### 3.1 Primitive Types

| Type | Bit Width | Description | Example |
|---|---|---|---|
| `int` / `int32` | 32-bit | Signed 32-bit integer (default integer) | `let a: int = 42;` |
| `int8` | 8-bit | Signed 8-bit integer | `let b: int8 = -128;` |
| `uint8` | 8-bit | Unsigned 8-bit integer / raw byte | `let c: uint8 = 255;` |
| `int16` | 16-bit | Signed 16-bit integer | `let d: int16 = 32767;` |
| `uint16` | 16-bit | Unsigned 16-bit integer | `let e: uint16 = 65535;` |
| `uint` / `uint32` | 32-bit | Unsigned 32-bit integer | `let f: uint = 4000000000;` |
| `int64` | 64-bit | Signed 64-bit integer | `let g: int64 = 9223372036854775807;` |
| `uint64` | 64-bit | Unsigned 64-bit integer | `let h: uint64 = 18446744073709551615;` |
| `int128` | 128-bit | Signed 128-bit integer | `let i: int128;` |
| `uint128` | 128-bit | Unsigned 128-bit integer | `let j: uint128;` |
| `float` / `float32` | 32-bit | IEEE 754 single-precision float | `let k: float = 3.14;` |
| `float16` | 16-bit | IEEE 754 half-precision float | `let l: float16;` |
| `float64` | 64-bit | IEEE 754 double-precision float | `let m: float64 = 2.718281828459;` |
| `bool` | 1-bit | Boolean (`true` or `false`) | `let ok: bool = true;` |
| `char` | 32-bit | Unicode scalar value | `let letter: char = 'Z';` |
| `string` | Dynamic | Immutable UTF-8 string | `let s: string = "tejx";` |
| `void` | 0-bit | Unit type, indicates no return value | `function log(): void {}` |
| `any` | Tagged | Dynamic boxed value (bypasses static checks) | `let x: any = 123;` |

### 3.2 Numeric Conversions (`class Number`)

TejX provides the built-in `Number` class in the prelude for converting strings and arbitrary values into specific numeric types:

```tx
// Floating-point conversions
let f: float = Number.Float("3.1415");
let f64: float64 = Number.Float64("2.718281828459");

// Integer conversions with explicit sizing
let n: int = Number.Int("42");
let n8: int8 = Number.Int8("127");
let n16: int16 = Number.Int16("32000");
let n64: int64 = Number.Int64("9223372036854775807");
let u32: uint32 = Number.UInt32("4000000000");
```

> **Note**: Legacy `parseFloat` and `parseInt` are replaced with `Number.Float()` and `Number.Int()`.

### 3.3 Array Types
TejX supports both dynamic and fixed-size arrays:

```tx
// Dynamic array (heap-managed, resizable)
let numbers: int[] = [1, 2, 3, 4, 5];

// Fixed-size array (exact capacity)
let board: int[64] = [];

// Multidimensional arrays
let matrix: int[][] = [[1, 0], [0, 1]];

// Sized byte buffer
let buffer: uint8[1024];
```

Array methods are available directly on arrays:
```tx
numbers.push(6);
let last = numbers.pop();
let count = numbers.length();
```

Rules:
- Empty array literals `[]` require an explicit target type annotation.
- Array element types are strictly enforced at compile time.
- Fixed-size arrays `T[N]` and dynamic arrays `T[]` are distinct types.

### 3.4 The `Optional<T>` Nullability Model
TejX rejects naked `null`, `nil`, or `undefined`. To represent a value that may be missing, use `Optional<T>`:

```tx
let user: Optional<string> = None;
user = "Alice";
```

#### Strict Rules:
- `Optional<T>` is the **only** nullable type in TejX.
- `Option<T>` is rejected; use `Optional<T>`.
- `null` and `undefined` are rejected at compile time; use `None`.
- Union syntax such as `int | None` is rejected.
- `let x: Optional<T>;` defaults to `None` automatically.
- Non-optional typed declarations **must** be initialized (`let x: int;` is a compile error).

#### Safe Unwrapping & Narrowing:
1. **Equality check with `None`**:
   ```tx
   let email: Optional<string> = getEmail();
   if (email != None) {
       // Inside this block, email is narrowed and safe to access
       print(email as string);
   }
   ```
2. **Member Access and Indexing**:
   - Member access on `Optional<T>` requires a prior `!= None` check, unless using optional chaining `?.`.
   - Indexing into `Optional<T>` requires a prior `!= None` check.
   - `instanceof` cannot be used directly on `Optional<T>`; narrow with `!= None` first.
3. **Nullish coalescing operator (`??`)**:
   ```tx
   let displayName: string = email ?? "guest@example.com";
   ```
4. **Optional chaining (`?.`)**:
   ```tx
   let street = user?.address?.street;
   ```

### 3.5 Function Types
Functions are first-class values with structural types:
```tx
type BinaryOp = (a: int, b: int) => int;
type Callback<T> = (result: T) => void;

function apply(op: (x: int, y: int) => int, a: int, b: int): int {
    return op(a, b);
}
```

### 3.6 Structural Object / Struct Types
Anonymous structural record types provide shape-checked data structures:
```tx
type UserProfile = {
    id: int;
    name: string;
    bio?: string; // Optional property
};

let user: UserProfile = {
    id: 1,
    name: "Praveen",
    bio: "TejX author"
};
```

Constraints:
- Variables of structural object types must be initialized at declaration.
- Object members must match the declared shape; extra unexpected properties are rejected.
- `object` and `Object` are **not valid types** in TejX. Use `any` for dynamic boxed values, or explicit structural records `{ key: type }`.

### 3.7 Intersection Types (`&`)
Combines multiple types:
```tx
type HasId = { id: int };
type HasName = { name: string };
type Entity = HasId & HasName;
```

### 3.8 Type Aliases
Declare readable names for complex types:
```tx
type StringMap<T> = Map<string, T>;
type Coordinate = { x: float; y: float };
```

### 3.9 Type Casting (`as`)
Explicit type conversion:
```tx
let boxed: any = 42;
let unboxed: int = boxed as int;

let piFlt: float = 3.14159;
let piInt: int = piFlt as int; // Truncates to 3
```

### 3.10 Type Introspection (`typeof` and `instanceof`)
- `typeof(v)` returns a string representation of the runtime type: `"int"`, `"float"`, `"string"`, `"bool"`, `"char"`, `"array"`, `"function"`, `"struct"`, `"None"`, or registered class name.
- `v instanceof ClassName` checks if an object is an instance of a given class:

```tx
let err: any = new RuntimeError("disk full");
if (err instanceof Error) {
    print("Caught an error: " + (err as Error).message);
}
```

---

## 4. Variables & Declarations

### 4.1 Mutable Variables (`let`)
`let` binds a mutable variable:
```tx
let counter: int = 0;
counter = counter + 1;

// Type inference is supported
let message = "hello"; // Inferred as string
```

### 4.2 Immutable Constants (`const`)
`const` declares an immutable binding that cannot be reassigned:
```tx
const MAX_RETRIES: int = 5;
const APP_NAME: string = "TejX Engine";
```

---

## 5. Expressions & Operators

### 5.1 Operator Precedence Table (High to Low)

| Precedence | Operators | Associativity | Description |
|---|---|---|---|
| **1 (Highest)** | `.` `?.` `[]` `?[]` `()` `?()` | Left-to-right | Member access, optional chaining, calls |
| **2** | `++` `--` (postfix) | Left-to-right | Postfix increment/decrement |
| **3** | `++` `--` (prefix) `+` `-` `!` `~` `typeof` | Right-to-left | Unary prefix operators |
| **4** | `as` | Left-to-right | Type cast |
| **5** | `*` `/` `%` | Left-to-right | Multiplicative |
| **6** | `+` `-` | Left-to-right | Additive, string concatenation |
| **7** | `<<` `>>` | Left-to-right | Bitwise shift |
| **8** | `<` `<=` `>` `>=` `instanceof` | Left-to-right | Relational / Comparison |
| **9** | `==` `!=` | Left-to-right | Equality |
| **10** | `&` | Left-to-right | Bitwise AND |
| **11** | `^` | Left-to-right | Bitwise XOR |
| **12** | `\|` | Left-to-right | Bitwise OR |
| **13** | `&&` | Left-to-right | Logical AND (short-circuit) |
| **14** | `\|\|` | Left-to-right | Logical OR (short-circuit) |
| **15** | `??` | Left-to-right | Nullish coalescing |
| **16** | `? :` | Right-to-left | Ternary conditional |
| **17** | `=` `+=` `-=` `*=` `/=` `%=` `&=` `\|=` `^=` `<<=` `>>=` | Right-to-left | Assignment |
| **18 (Lowest)**| `,` | Left-to-right | Comma sequence operator |

### 5.2 Operator Highlights

#### String Concatenation (`+`)
```tx
let full = "Hello, " + name + "!";
```

#### Increment and Decrement (`++`, `--`)
```tx
let i = 0;
i++; // Postfix
++i; // Prefix
```

#### Nullish Coalescing (`??`)
Returns the left-hand operand if it is not `None`; otherwise returns the right-hand operand:
```tx
let port = config.port ?? 8080;
```

#### Optional Chaining (`?.`)
Short-circuits to `None` if the target is `None`:
```tx
let zip = user?.address?.zipCode;
```

#### Delete Operator (`del`)
Removes a field from an object or deletes a collection element:
```tx
del user.temporaryField;
```

---

## 6. Control Flow

### 6.1 Conditional Statements (`if / else if / else`)
```tx
if (score >= 90) {
    print("Grade: A");
} else if (score >= 80) {
    print("Grade: B");
} else {
    print("Grade: C");
}
```

### 6.2 While Loops (`while`)
```tx
let i = 0;
while (i < 10) {
    print(i);
    i++;
}
```

### 6.3 C-Style For Loops (`for`)
```tx
for (let i = 0; i < 5; i++) {
    print(`Iteration ${i}`);
}
```

### 6.4 For-Of Loops (`for (let x of array)`)
Iterate over elements in an array or iterable collection:
```tx
let fruits = ["Apple", "Banana", "Cherry"];
for (let fruit of fruits) {
    print(fruit);
}
```

### 6.5 Switch Statements (`switch / case / default`)
Supports integer, character, and string matching with explicit `break`:
```tx
switch (command) {
    case "start":
        print("Starting engine...");
        break;
    case "stop":
        print("Stopping engine...");
        break;
    default:
        print("Unknown command: " + command);
        break;
}
```

### 6.6 Loop Control (`break` & `continue`)
- `break`: Immediately exits the enclosing loop or switch.
- `continue`: Skips to the next iteration of the loop.

---

## 7. Functions & Closures

### 7.1 Named Functions
```tx
function calculateArea(width: float, height: float): float {
    return width * height;
}
```

### 7.2 Default Parameter Values
```tx
function greet(name: string, title: string = "Mr."): string {
    return `Hello, ${title} ${name}`;
}
```

### 7.3 Rest Parameters
Accepts an arbitrary number of arguments as an array:
```tx
function sumAll(...numbers: int[]): int {
    let total = 0;
    for (let n of numbers) {
        total += n;
    }
    return total;
}
```

### 7.4 Generic Functions
Functions parameterized by type arguments:
```tx
function swap<T>(arr: T[], i: int, j: int): void {
    let temp: T = arr[i];
    arr[i] = arr[j];
    arr[j] = temp;
}
```

### 7.5 Arrow Functions (Lambdas) & Closures
Arrow functions capture their enclosing lexical scope:
```tx
let factor = 3;
let multiplier = (n: int): int => n * factor;

print(multiplier(10)); // 30
```

Multi-statement arrow functions:
```tx
let compute = (x: int, y: int): int => {
    let temp = x * 2;
    return temp + y;
};
```

---

## 8. Object-Oriented Programming

### 8.1 Class Declaration
```tx
class Account {
    public id: string;
    private balance: float;
    protected owner: string;

    constructor(id: string, initialBalance: float, owner: string) {
        this.id = id;
        this.balance = initialBalance;
        this.owner = owner;
    }

    deposit(amount: float): void {
        if (amount > 0.0) {
            this.balance += amount;
        }
    }

    getBalance(): float {
        return this.balance;
    }
}
```

### 8.2 Inheritance (`extends` & `super`)
```tx
class SavingsAccount extends Account {
    private interestRate: float;

    constructor(id: string, initial: float, owner: string, rate: float) {
        super(id, initial, owner);
        this.interestRate = rate;
    }

    applyInterest(): void {
        let earned = this.getBalance() * this.interestRate;
        this.deposit(earned);
    }
}
```

### 8.3 Getters and Setters
```tx
class Rectangle {
    private _width: float;
    private _height: float;

    constructor(w: float, h: float) {
        this._width = w;
        this._height = h;
    }

    get area(): float {
        return this._width * this._height;
    }

    set width(w: float) {
        if (w >= 0.0) this._width = w;
    }
}
```

### 8.4 Static Members
```tx
class MathUtils {
    static const EPSILON: float = 0.000001;

    static clamp(val: float, min: float, max: float): float {
        if (val < min) return min;
        if (val > max) return max;
        return val;
    }
}
```

### 8.5 Interfaces (`interface` & `implements`)
Interfaces define structural contracts for classes:
```tx
interface Serializable {
    serialize(): string;
}

interface Printable {
    printDetails(): void;
}

class Document implements Serializable, Printable {
    title: string;

    constructor(t: string) {
        this.title = t;
    }

    serialize(): string {
        return `{"title":"${this.title}"}`;
    }

    printDetails(): void {
        print(`Doc: ${this.title}`);
    }
}
```

### 8.6 Abstract Classes
```tx
abstract class Shape {
    abstract area(): float;

    describe(): void {
        print(`Shape with area: ${this.area()}`);
    }
}

class Circle extends Shape {
    radius: float;

    constructor(r: float) {
        this.radius = r;
    }

    area(): float {
        return 3.14159 * this.radius * this.radius;
    }
}
```

### 8.7 Enums
Enumerated sets of named constants:
```tx
enum Status {
    Pending,
    Active,
    Suspended,
    Closed
}

let currentStatus = Status.Active;
```

### 8.8 Namespaces
Namespaces bundle functions, types, and constants to prevent name collisions:
```tx
namespace Geometry {
    export function distance(x1: float, y1: float, x2: float, y2: float): float {
        let dx = x2 - x1;
        let dy = y2 - y1;
        return (dx * dx + dy * dy) as float;
    }
}

let d = Geometry.distance(0.0, 0.0, 3.0, 4.0);
```

---

## 9. Error Handling & Exceptions

### 9.1 Throwing Exceptions (`throw`)
Any object or value can be thrown, but deriving from `Error` provides rich stack traces:
```tx
if (divisor == 0) {
    throw new Error("Division by zero!");
}
```

### 9.2 Catching Exceptions (`try / catch / finally`)
```tx
try {
    let result = riskyOperation();
    print("Success: " + result);
} catch (e) {
    print("Failed with error: " + e.message);
} finally {
    cleanupResources();
}
```

### 9.3 Standard Exception Hierarchy
- **`Error`**: Base class (`message`, `stack`, `code`).
- **`RuntimeError`**: Runtime failure (e.g., failed assertions, out-of-bounds).
- **`PanicError`**: Unrecoverable panic condition.
- **`AggregateError`**: Wraps multiple errors (`errors: Error[]`).

---

## 10. Modules, Imports & Exports

TejX uses a static, compile-time module system. Imports are resolved during lowering, merged into the active compilation unit, and verified before code generation.

### 10.1 File Imports
Import relative files with or without the `.tx` extension:
```tx
import "./utils.tx";
import { helperFunction, HelperClass as HC } from "./helpers.tx";
import defaultThing from "./module.tx";
```

### 10.2 Standard Library Imports
Standard library modules use the `std:<module>` namespace:
```tx
import std:fs;
import std:net;
import std:http;
import std:json;
import std:math;
import std:time;
import std:system;
import std:collections;
import std:crypto;
import std:binary;
import std:thread;
import std:gc;
import std:runtime;
```

Selective standard library imports:
```tx
import { now } from "std:time";
import { sleep } from "std:thread";
import { Map, Set } from "std:collections";
import { fetch, HttpServer } from "std:http";
```

### 10.3 Resolution Rules & Implicit Core Imports
1. **Standard Library Resolution Order**:
   - Explicit CLI flag: `--stdlib-path <path>`
   - Local project directory: `./lib/`
   - Installed SDK location: `$HOME/.tejx/lib`
2. **Implicit Core Injection**:
   Normal source files automatically have fundamental prelude helpers injected during lowering without manual imports:
   - `core/prelude.tx`
   - `core/array.tx`
   - `core/string.tx`
3. **Circular Dependency Detection**:
   The compiler maintains an active import stack. Circular import cycles are detected during lowering and reported with descriptive diagnostic traces.

### 10.4 Exports
Export items from a module for consumption by other files:
```tx
export function computeHash(data: string): string { ... }
export class Engine { ... }
export const VERSION = "2.0.0";
export namespace Config { ... }
export default function mainEntry(): void { ... }
```

---

## 11. Concurrency Model

TejX provides two distinct, powerful concurrency mechanisms tailored for different workloads:
1. **`async` / `await` Event Loop**: Optimized for non-blocking I/O, timers, and network operations.
2. **M:N Virtual Thread Scheduler & OS Threads (`std:thread`)**: Optimized for high-throughput CPU-bound parallelism.

### 11.1 Async / Await Event Loop
Async functions execute on the single-threaded TejX event loop while non-blocking I/O and timers advance on background worker threads (powered by Tokio):
```tx
async function fetchData(): Promise<string> {
    let res = await fetch("https://example.com/api");
    return res.text();
}
```
- Promise microtasks are drained with priority before queued I/O callbacks.
- Values surviving across `await` suspensions are tracked via global GC handles to ensure moving GC safety.

### 11.2 Virtual Thread Spawning (M:N)
Thousands or millions of virtual threads can be spawned, multiplexed across OS worker threads with work-stealing:
```tx
Thread.spawn(() => {
    print("Running concurrently in a virtual thread!");
});
```

### 11.3 Thread Join & Sleep
```tx
import { sleep } from "std:thread";

let t = Thread.spawn(() => {
    doHeavyWork();
});

t.join(); // Blocks calling virtual thread until t finishes
sleep(100); // Parks calling thread for 100 milliseconds
```

### 11.4 Promises (`Promise<T>`)
High-level task management:
```tx
// Spawn background task returning a Promise
let p = Promise.spawn(() => {
    return computeValue();
});

// Run tasks in parallel and wait for all (throws on first error)
let results = Promise.all([
    () => fetch("https://api.one.com").text(),
    () => fetch("https://api.two.com").text()
]);

// Run tasks in parallel and capture outcomes (never throws)
let settled = Promise.settled([
    () => taskA(),
    () => taskB()
]);
// settled[0].status == "fulfilled" or "rejected"
```

### 11.4 Synchronization Primitives (`std:thread`)
- **`Atomic`**: Lock-free atomic integers (`add`, `sub`, `increment`, `decrement`, `load`, `store`, `exchange`, `compareExchange`).
- **`Mutex`**: Mutual exclusion lock (`lock()`, `unlock()`).
- **`Condition`**: Condition variable (`wait(mutex)`, `notify()`, `notifyAll()`).
- **`SharedQueue<T>`**: Thread-safe FIFO queue (`enqueue`, `dequeue`, `drain`, `size`).

---

## 12. Memory Model & Garbage Collection

### 12.1 Automatic Memory Management
TejX uses a high-throughput generational garbage collector:
- **Eden Space**: Fast bump-pointer allocation for young objects.
- **Survivor Space**: Objects surviving a minor collection are copied to Survivor.
- **Old Generation**: Tenured objects are collected via concurrent Mark-Sweep.
- **Large Object Space (LOS)**: Large allocations (> 64KB) bypass young generation.
- **Card Table Marking**: Tracks Old-to-Young references for fast minor collections.

### 12.2 Programmatic GC Control (`std:gc`)
```tx
import std:gc;

gc.collect();        // Trigger major GC
gc.collectYoung();   // Trigger minor GC
let stats = gc.getStats();
print(`Heap used: ${stats.heapUsed} bytes / ${stats.heapTotal} bytes`);
```

---

## 13. Foreign Function Interface (FFI)

Call external C ABI functions directly using `extern function`:

```tx
extern function puts(s: string): int;
extern function getpid(): int;

function main() {
    puts("Direct C puts call!");
    print("PID: " + getpid());
}
```
