# Async/Await Implementation in NovaJs

This document details the transition of NovaJs from a multi-threaded blocking model to a **single-threaded event loop** architecture, similar to modern execution environments like Node.js or browsers.

## 1. Runtime Architecture

NovaJs now utilizes a central **Event Queue** to manage asynchronous tasks. All `async` function execution and promise resolutions are coordinated through this loop on the main thread.

### Event Loop Components (`src/runtime.rs`)

- **`Task`**: A simple structure representing a deferred function call.
  ```rust
  struct Task {
      func: i64, // Function pointer
      arg: i64,  // Argument (usually a heap-boxed array)
  }
  ```
- **`EVENT_QUEUE`**: A global `VecDeque<Task>` protected by a `Mutex`.
- **`ACTIVE_ASYNC_OPS`**: An atomic counter (`AtomicI64`) that tracks pending background operations (e.g., I/O, timers). The event loop continues running as long as this counter is greater than zero.

### Core Runtime Functions

- **`tejx_enqueue_task(func, arg)`**: Pushes a new task to the queue.
- **`tejx_run_event_loop()`**: The main runner loop. It processes tasks from the queue sequentially. If the queue is empty but `ACTIVE_ASYNC_OPS > 0`, it waits (sleeps) for background operations to complete and potentially queue new tasks.
- **`tejx_inc_async_ops()` / `tejx_dec_async_ops()`**: Used by the standard library to signal the start and end of background work.

## 2. Implementation Logic

### `async` Functions

When the compiler lowers an `async` function, it generates a "worker" function and a "wrapper" function.

**Original Source:**

```typescript
async function fetchData(id) {
  return "Data_" + id;
}
```

**Lowering Transformation:**

1. **Worker**: Contains the original function body, wraps it in a try-catch, and ensures the promise is resolved or rejected. It also decrements `ACTIVE_ASYNC_OPS` upon completion.
2. **Wrapper**:
   - Creates a new `Promise`.
   - Increments `ACTIVE_ASYNC_OPS`.
   - Enqueues the worker function call using `tejx_enqueue_task`.
   - Returns the `Promise` immediately.

### `await` Expression

The `await` keyword is lowered to a call to the `__await` runtime intrinsic. Unlike a purely blocking model, the new `__await` is **Event Loop Aware**:

```rust
fn await_impl(val: i64) -> Result<i64, i64> {
    // ... setup ...
    loop {
        // 1. Check if the promise is resolved/rejected
        // 2. If Pending:
        //    a. If EVENT_QUEUE is empty, wait for a short timeout on the Promise's Condvar.
        //    b. Process ONE task from the EVENT_QUEUE.
    }
}
```

This allows the program to continue processing other queued tasks while waiting for a specific promise to resolve, preventing deadlocks and maintaining responsiveness.

## 3. Standard Library Integration

Background operations (I/O, timers) still use native threads for waiting, but their completion is signaled back to the event loop.

- **Networking (`net.rs`)**: Async requests increment the counter before spawning a background thread and decrement it when the response is received and the promise is resolved.
- **Timers (`time.rs`)**: `delay(ms)` increments the counter, sleeps in a background thread, and decrements the counter after resolving the promise.

## 4. Program Lifecycle

The event loop is automatically started at the end of the `tejx_main` function. This ensures that even if the top-level script finishes, the process stays alive until all queued tasks and background operations are completed.

## 5. Comparison

| Feature         | Old NovaJs Model            | New Event Loop Model          | Node.js                  |
| --------------- | --------------------------- | ----------------------------- | ------------------------ |
| **Concurrency** | One thread per `async` call | Single-threaded (Main)        | Single-threaded (Main)   |
| **Await**       | Blocks OS thread entirely   | Processes queue while waiting | Suspends (State machine) |
| **I/O**         | Native blocking             | Native async/threaded         | Native async (libuv)     |
| **Overhead**    | High (Native stacks)        | Low (Queue objects)           | Low (State machine)      |

## 6. Current Improvements

- **Reliability**: No more race conditions from multiple threads accessing the same local scopes (unless explicitly using `Thread.spawn`).
- **Standardization**: Behaves more like traditional JavaScript environments, making it easier to port code.
- **Scalability**: Can handle significantly more concurrent "tasks" as they are just pointers in a queue rather than full OS threads.
