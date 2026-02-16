# Async/Await Implementation in NovaJs

This document details the implementation of `async` and `await` in NovaJs. It explains the runtime primitives, how the compiler transforms async code, and provides examples of usage.

## 1. Runtime Architecture

NovaJs implements asynchronous programming using a **blocking thread model** backed by native OS threads. This is different from the event-loop model found in JavaScript (Node.js/Browser) but allows for true parallelism.

### The `Promise` Primitive

The core primitive is the `Promise`, defined in `src/runtime.rs`. It is a thread-safe wrapper around a Mutex and a Condition Variable.

```rust
// src/runtime.rs
pub enum PromiseState {
    Pending,
    Resolved(i64),   // Value ID
    Rejected(i64),   // Error Value ID
}

pub enum TaggedValue {
    // ...
    Promise(Arc<(Mutex<PromiseState>, Condvar)>),
}
```

### Runtime Functions

The runtime exposes the following C-compatible functions for the compiler and standard library:

- **`Promise_new(callback: i64)`**: Creates a new pending Promise.
- **`Promise_resolve(val: i64)`**: Creates a Promise resolved with `val`.
- **`Promise_reject(reason: i64)`**: Creates a Promise rejected with `reason`.
- **`__await(val: i64)`**: The implementation of the `await` keyword.
- **`Promise_all(args_id: i64)`**: Creates a Promise that resolves when all input promises resolve.

## 2. Implementation Logic

### `async` Functions

When you declare a function as `async`, the compiler transforms it into a function that returns a `Promise`.

**Source:**

```typescript
async function heavyComputation(x) {
  return x * 2;
}
```

**Conceptual Lowering:**
Since `await` is blocking, strict async functions in NovaJs are often wrapper functions that spawn a thread to execute their body, returning a Promise immediately.

```rust
// Conceptual runtime logic for an async function call
fn heavyComputation_async(x: i64) -> i64 { // returns Promise ID
    let p = Promise_new(0);
    thread::spawn(move || {
        let result = x * 2;
        __resolve_promise(p, result);
    });
    return p;
}
```

### `await` Expression

The `await` keyword is lowered to a call to the `__await` runtime intrinsic.

**Source:**

```typescript
let result = await promise;
```

**Implementation (`src/runtime.rs`):**
The `__await` function blocks the **current OS thread** until the promise state changes to `Resolved` or `Rejected`.

```rust
fn await_impl(val: i64) -> Result<i64, i64> {
    // 1. Get the Promise from the heap
    // 2. Lock the mutex
    // 3. Loop while Pending:
    //      state = cvar.wait(state)
    // 4. Return value or error
}
```

### `Promise.all`

`Promise.all` is implemented by spawning a coordinator thread that invokes `__await` on each promise in sequence (or simply correctly waits for them). In the current implementation, it spawns a thread that iterates and waits.

## 3. Example Usage

Here is an example of valid NovaJs code utilizing the async/await features (once enabled in the parser).

```typescript
// Define an async function (simulated)
// In reality, async keyword handles the wrapping.
function performTask(id) {
  print("Starting task " + id);
  // Simulate work or return a value
  return id * 10;
}

async function main() {
  print("Main started");

  // 1. Create a promise (async execution)
  // Note: In strict NovaJs, 'async' keyword does the thread spawning.
  // Use Thread.spawn explicitly if you need manual control.
  let p1 = Thread.spawn(performTask, 1);
  let p2 = Thread.spawn(performTask, 2);

  print("Tasks spawned, waiting...");

  // 2. Await the results (Blocks main thread)
  let r1 = await p1;
  let r2 = await p2;

  print("Result 1: " + r1);
  print("Result 2: " + r2);
}

main();
```

## 4. Comparisons

| Feature | NovaJs | Node.js |
|BC|---|---|
| **Model** | Thread-based (Blocking) | Event Loop (Non-blocking) |
| **Concurrency** | True Parallelism | Single Threaded |
| **Await** | Blocks OS Thread | Suspends Coroutine |
| **Overhead** | High (Stack per task) | Low (State machine) |

## 5. Current Limitation

The `async/await` syntax is currently disabled in `src/parser.rs` with the error `async/await is disabled`. To enable it:

1.  Remove the error check in `Parser::parse_function_declaration`.
2.  Ensure `MIRLowering` correctly handles `Expression::Await`.
