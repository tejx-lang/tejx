# Multi-Threading in NovaJs

NovaJs provides native support for **OS-level threads**, allowing you to execute code in parallel on multiple CPU cores. This is distinct from the single-threaded Async/Await model and is intended for CPU-intensive tasks.

## 1. Usage Guide

### Spawning Threads

The `Thread` class is used to create and manage threads.

```typescript
// 1. Define a worker function
function heavyTask(limit: int): int {
  let sum = 0;
  for (let i = 0; i < limit; i++) {
    sum += i;
  }
  return sum;
}

function main() {
  print("Main thread working...");

  // 2. Spawn a thread
  // Thread.spawn(function, argument)
  let t = Thread.spawn(heavyTask, 1000000);

  print("Thread spawned, continuing main work...");

  // 3. Wait for result (Join)
  let result = t.join();
  print("Result:", result);
}
```

### Supported APIs (`std:thread`)

- `Thread.spawn(func, arg)`: Starts a new thread. Returns a `Thread` handle.
- `thread.join()`: Blocks until the thread finishes and returns its result.
- `Thread.sleep(ms)`: Puts the _current_ thread to sleep (blocking).

## 2. Synchronization (`std:thread`)

When threads need to share data or coordinate, you **must** use synchronization primitives to avoid race conditions.

### Mutex (Mutual Exclusion)

Protects shared data by ensuring only one thread can access it at a time.

```typescript
import { Mutex } from "std:thread";

let lock = new Mutex();
// ... inside a thread ...
lock.acquire();
// Critical Section: Safe to modify shared variables
lock.release();
```

### SharedQueue

A thread-safe queue implementation, useful for Producer-Consumer patterns.

```typescript
import { SharedQueue, Mutex } from "std:thread";

let q = new SharedQueue<int>();
let lock = new Mutex();

// Check 'tests/problems/producer_consumer.tx' for a full example
```

## 3. Constraints & Memory Model

### ⚠️ Shared State Safety

- **Memory Isolation**: Threads share the same Heap but have their own Stacks.
- **Race Conditions**: Modifying standard collections (`Array`, `Map`) or objects from multiple threads simultaneously **without locks** leads to Undefined Behavior (crashes or corruption).
- **ARC Safety**: The Atomic Reference Counting in NovaJs is thread-safe, so passing objects between threads will not cause memory leaks, but the _contents_ of those objects are not automatically protected from race conditions.

### ⚠️ Usage Limits

- **Argument Passing**: Currently, `Thread.spawn` accepts a single argument. Wrap multiple arguments in an object or array (e.g., `Thread.spawn(worker, {start: 0, end: 100})`).
- **`this` Binding**: Passing an instance method to `Thread.spawn` disconnects it from its `this` context. Use an arrow function or static method if possible, or manually pass the instance `this` as the argument.

## 4. Internal Architecture (Advanced)

_This section details the internal mechanics of the runtime._

### Runtime Primitives (`src/runtime.rs`)

- **`Thread_new`**: Uses Rust's `std::thread::spawn`. It transmutes the function ID to a function pointer and executes it.
- **`Mutex`**: Wraps a `std::sync::Mutex` and `Condvar`.
- **Global Lock**: The runtime currently employs a global lock on the main `HEAP` map to prevent memory corruption during allocation/deallocation, providing a baseline of safety (similar to a GIL for allocation), but individual object properties are not locked.
