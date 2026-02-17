# Async/Await in NovaJs

NovaJs employs a **single-threaded non-blocking event loop** model, similar to JavaScript (Node.js/Browsers). This allows you to perform I/O operations (file handling, networking, timers) without blocking the main execution thread.

## 1. Usage Guide

### Basic Syntax

Use the `async` keyword to define functions that perform asynchronous operations, and `await` to pause execution until a result is ready.

```typescript
import { readFile } from "fs";

// standard async function
async function readConfig(): string {
  print("Reading file...");
  let content = await readFile("config.json");
  return content;
}

function main() {
  // Top-level async calls are supported
  readConfig();
}
```

### Supported APIs

NovaJs provides a growing set of standard library modules that support async operations:

- **Timers**:
  - `setTimeout(callback, ms)`: Run a function after a delay.
  - `setInterval(callback, ms)`: Run a function repeatedly.
  - `delay(ms)`: A promisified sleep function (in `std:time`).

- **File System (`fs`)**:
  - `fs.readFile(path)`: Asynchronously read a file.
  - `fs.writeFile(path, content)`: Asynchronously write to a file.

- **Networking**:
  - `fetch(url)`: Perform HTTP requests.
  - `net` module: TCP client/server operations.

## 2. Constraints & Best Practices

### ⚠️ Single Threaded Nature

Async functions in NovaJs run on the **same thread** as your synchronous code.

- **Do not** perform heavy CPU computations (e.g., matrix multiplication, image processing) inside an `async` function. It will block the Event Loop and freeze the entire application (including timers and network requests).
- **Use Threads** for CPU-intensive tasks (see [THREADING.md](THREADING.md)).

### ⚠️ Top-Level Execution

- The Event Loop starts automatically after your `main()` or top-level code finishes.
- If you have detached async tasks (promises defined but not awaited), the process will keep running until all of them complete.

## 3. Internal Architecture (Advanced)

_This section details the internal mechanics of the runtime._

### Runtime Architecture

NovaJs utilizes a central **Event Queue** to manage asynchronous tasks. All `async` function execution and promise resolutions are coordinated through this loop on the main thread.

#### Event Loop Components (`src/runtime.rs`)

- **`Task`**: A simple structure representing a deferred function call.
- **`EVENT_QUEUE`**: A global `VecDeque<Task>` protected by a `Mutex`.
- **`ACTIVE_ASYNC_OPS`**: An atomic counter (`AtomicI64`) that tracks pending background operations (e.g., I/O, timers). The event loop continues running as long as this counter is greater than zero.

#### `await` Lowering

The `await` keyword is lowered to a call to the `__await` runtime intrinsic. Unlike a purely blocking model, the new `__await` is **Event Loop Aware**:
It allows the program to continue processing other queued tasks while waiting for a specific promise to resolve, preventing deadlocks and maintaining responsiveness.
