# Concurrency: Async & Threads

TejX provides two distinct models for concurrent execution: **Async/Await** for non-blocking I/O and **Multi-Threading** for parallel CPU tasks.

---

## 1. Async/Await (Event Loop)

TejX features a non-blocking event loop, similar to JavaScript. This is the preferred model for I/O-bound tasks like networking or file system operations.

### Usage

```typescript
async function fetchData(): string {
  let data = await fetch("https://api.example.com");
  return data;
}
```

### Key Concepts

- **Single Threaded**: All async tasks run on the main thread.
- **Non-Blocking**: `await` pauses the function without freezing the whole program.
- **Event Queue**: Managed by the runtime (`EVENT_QUEUE` in `src/runtime.rs`).
- **Global Function**: `delay(ms)` provides a promisified sleep.

---

## 2. Multi-Threading (Parallelism)

For CPU-intensive work, TejX provides native OS-level threads via the `Thread` class.

### Usage

```typescript
function heavyTask(n: int): int {
  // ... computation ...
  return n * 2;
}

let t = Thread.spawn(heavyTask, 100);
let result = t.join(); // Blocks until done
```

### Synchronization Primitives (`std:thread`)

When sharing data between threads, use these to avoid race conditions:

- **Mutex**: `lock.acquire()` and `lock.release()` for critical sections.
- **Atomic**: Lock-free integer operations.
- **SharedQueue**: Thread-safe communication between producers and consumers.

---

## 3. Choosing the Right Tool

| Feature        | Async/Await              | Multi-Threading                  |
| :------------- | :----------------------- | :------------------------------- |
| **Best For**   | I/O, Timers, Networking  | Compute-heavy tasks, Parallelism |
| **Execution**  | Cooperative (Event Loop) | Preemptive (OS Threads)          |
| **Complexity** | Low (Sequential code)    | High (Requires synchronization)  |
| **Overhead**   | Minimal                  | Significant (Thread creation)    |

---

## ⚠️ Constraints & Safety

1.  **Don't block the Loop**: Never perform heavy math inside `async` functions; use a `Thread` instead.
2.  **Shared State**: While ARC is thread-safe, the _contents_ of objects are not. Always protect shared `Array` or `Map` access with a `Mutex`.
3.  **`this` Context**: Passing class methods to `Thread.spawn` may lose `this` binding. Use arrow functions as wrappers.
