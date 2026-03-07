# Tejx Concurrency Model

Tejx provides two orthogonal concurrency primitives: **Async/Await** for cooperative, non-blocking I/O workloads and **Native Threads** for true parallel computation. Both models integrate cleanly with the ownership system to prevent data races at compile time.

---

## 1. Async/Await (Cooperative Concurrency)

Async functions enable non-blocking execution on a single thread, ideal for I/O-bound operations like networking, file access, and timers.

### Declaring Async Functions

```typescript
async function fetchUser(id: int): string {
  let response = await httpGet("/api/users/" + id);
  return response;
}
```

- The `async` keyword marks a function as asynchronous.
- `await` suspends execution until the awaited operation completes, yielding control back to the event loop without blocking the thread.

### The Event Loop

Tejx implements a lightweight cooperative scheduler backed by a task queue:

```
┌──────────────────────────────────────────┐
│              EVENT LOOP                  │
│                                          │
│  ┌────────────────────────────────────┐  │
│  │         TASK_QUEUE (FIFO)          │  │
│  │  (worker_fn, args) → dequeue      │  │
│  └────────────────────────────────────┘  │
│                                          │
│  Loop:                                   │
│    1. Pop task from queue                │
│    2. Execute worker(args)               │
│    3. If queue empty & no pending ops:   │
│       → exit loop                        │
│    4. Otherwise: yield + sleep(10ms)     │
│       → goto 1                           │
└──────────────────────────────────────────┘
```

**Key Properties:**

- **Single-threaded**: All async tasks execute on the main thread — no data races.
- **Non-blocking**: `await` pauses only the calling function, not the entire program.
- **Pending counter**: An atomic `ASYNC_OPS` counter tracks in-flight operations. The loop exits only when both the queue is empty and no operations are pending.

### Built-in Async Utilities

| Function         | Description                                |
| :--------------- | :----------------------------------------- |
| `delay(ms: int)` | Suspends execution for `ms` milliseconds   |
| `await expr`     | Pauses until the async expression resolves |

---

## 2. Native Threads (True Parallelism)

For CPU-intensive workloads, Tejx provides OS-level threads through the `std:thread` module. Each thread runs independently on its own OS thread with full parallel execution.

```typescript
import { Thread, Mutex, Atomic } from "std:thread";
```

### Thread\<T\>

Spawns a new OS thread with a typed callback and argument.

```typescript
function compute(n: int): void {
  let result = fibonacci(n);
  console.log("Result: " + result);
}

let worker = new Thread<int>(compute, 40);
worker.start(); // Launches the OS thread
worker.join(); // Blocks until the thread completes
```

| Method        | Signature                        | Description                                   |
| :------------ | :------------------------------- | :-------------------------------------------- |
| `constructor` | `(cb: (arg: T) => void, arg: T)` | Creates a thread with a callback and argument |
| `start()`     | `(): void`                       | Launches the OS thread                        |
| `join()`      | `(): void`                       | Blocks caller until the thread finishes       |

### Mutex

Provides mutual exclusion for critical sections. Protects shared state from concurrent access.

```typescript
let lock = new Mutex();
let counter = 0;

// Inside thread callback:
lock.lock();
counter = counter + 1;
lock.unlock();
```

| Method     | Signature  | Description                         |
| :--------- | :--------- | :---------------------------------- |
| `lock()`   | `(): void` | Acquires the mutex (blocks if held) |
| `unlock()` | `(): void` | Releases the mutex                  |

### Atomic

Lock-free atomic integer operations for high-performance counters and flags. All operations use sequential consistency ordering.

```typescript
let hits = new Atomic(0);

// From any thread — no lock needed:
hits.add(1);
let current = hits.load();
```

| Method                               | Signature                            | Description                                                    |
| :----------------------------------- | :----------------------------------- | :------------------------------------------------------------- |
| `add(val)`                           | `(val: int): int`                    | Atomically adds and returns previous value                     |
| `sub(val)`                           | `(val: int): int`                    | Atomically subtracts and returns previous value                |
| `load()`                             | `(): int`                            | Atomically reads the current value                             |
| `store(val)`                         | `(val: int): void`                   | Atomically writes a new value                                  |
| `exchange(val)`                      | `(val: int): int`                    | Atomically swaps and returns old value                         |
| `compareExchange(expected, desired)` | `(expected: int, desired: int): int` | CAS operation — sets to `desired` if current equals `expected` |

### Condition

Condition variables for thread signaling, always used with a `Mutex`.

```typescript
let lock = new Mutex();
let ready = new Condition();

// Consumer thread:
lock.lock();
ready.wait(lock); // Atomically releases lock and waits
// ... woken up, lock re-acquired ...
lock.unlock();

// Producer thread:
lock.lock();
// ... prepare data ...
ready.notify(); // Wake one waiting thread
lock.unlock();
```

| Method        | Signature              | Description                              |
| :------------ | :--------------------- | :--------------------------------------- |
| `wait(mutex)` | `(mutex: Mutex): void` | Releases mutex and blocks until notified |
| `notify()`    | `(): void`             | Wakes one waiting thread                 |
| `notifyAll()` | `(): void`             | Wakes all waiting threads                |

### SharedQueue\<T\>

A thread-safe FIFO queue for producer-consumer communication patterns.

```typescript
let queue = new SharedQueue<string>();

// Producer thread:
queue.enqueue("task-1");
queue.enqueue("task-2");

// Consumer thread:
let task = queue.dequeue();
```

| Method         | Signature        | Description                |
| :------------- | :--------------- | :------------------------- |
| `enqueue(val)` | `(val: T): void` | Thread-safe push to back   |
| `dequeue()`    | `(): T`          | Thread-safe pop from front |
| `size()`       | `(): int`        | Current number of items    |
| `isEmpty()`    | `(): bool`       | Whether the queue is empty |

---

## 3. Choosing the Right Model

|                  | Async/Await                                   | Native Threads                                          |
| :--------------- | :-------------------------------------------- | :------------------------------------------------------ |
| **Best for**     | I/O, timers, networking, sequential workflows | CPU-heavy computation, true parallelism                 |
| **Execution**    | Cooperative (single-threaded event loop)      | Preemptive (OS-scheduled threads)                       |
| **Data sharing** | Implicit — single thread, no races            | Explicit — requires `Mutex`, `Atomic`, or `SharedQueue` |
| **Overhead**     | Minimal (queue push/pop)                      | Significant (OS thread creation)                        |
| **Complexity**   | Low (reads like sequential code)              | Higher (synchronization required)                       |

---

## 4. Safety Rules & Best Practices

### Ownership Across Threads

- When passing data to a thread via `Thread<T>`, the argument is **moved**. The calling thread loses access to it.
- Primitives (`int`, `float`, `bool`) are copied — they can be freely shared.
- Heap objects (`string`, `Array`, `Map`) are moved into the thread's ownership.

### Avoiding Deadlocks

- Always acquire multiple mutexes in the **same order** across all threads.
- Prefer `SharedQueue<T>` for message passing over shared mutable state.
- Use `Atomic` for simple counters instead of wrapping them with a `Mutex`.

### Async Constraints

- **Never block the event loop**: Heavy computation inside `async` functions will freeze all pending tasks. Use `Thread` for CPU work.
- **No thread primitives in async context**: `Mutex.lock()` inside an async function will deadlock the single-threaded loop.
