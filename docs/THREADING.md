# Thread Implementation in NovaJs

NovaJs provides native multi-threading support through its runtime, leveraging Rust's `std::thread` and synchronization primitives. This allows for parallel execution of code, distinguishing it from the single-threaded event loop model of JavaScript.

## 1. Runtime Primitives

The threading support is implemented in `src/runtime.rs` and exposed via the `std:thread` module.

### Core Functions

- **`Thread.spawn(callback, arg)`**:
  - **Runtime Name**: `Thread_new`
  - **Description**: Spawns a new OS thread that executes `callback(arg)`.
  - **Returns**: A Thread ID (handle).
  - **Implementation**:
    ```rust
    pub extern "C" fn Thread_new(callback: i64, arg: i64) -> i64 {
        let handle = thread::spawn(move || {
            // Transmute callback ID to function pointer and execute
            let cb: unsafe extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(callback) };
            cb(arg)
        });
        // Store handle in Heap...
    }
    ```

- **`Thread.join(thread_id)`**:
  - **Runtime Name**: `Thread_join`
  - **Description**: Blocks the current thread until the specified thread terminates. Returns the result of the thread's function.

- **`Thread.sleep(ms)`**:
  - **Runtime Name**: `std_time_sleep`
  - **Description**: Puts the current thread to sleep for `ms` milliseconds.

### Synchronization

- **`Mutex`**:
  - **Usage**: `let m = Mutex.new();`
  - **Locking**: `m.lock()` blocks until acquired.
  - **Unlocking**: `m.unlock()` releases the lock.
  - **Condvar**: Associated with the mutex for `wait` and `notify`.

## 2. Example Usage

```typescript
// Define a worker function
// Note: Variable capture is not fully supported across threads yet; pass data via arguments.
function worker(id) {
  print("Worker " + id + " started");
  Thread.sleep(1000);
  print("Worker " + id + " finished");
  return id * 100;
}

function main() {
  print("Main started");

  // Spawn two threads
  let t1 = Thread.spawn(worker, 1);
  let t2 = Thread.spawn(worker, 2);

  // Wait for them to finish
  let r1 = Thread.join(t1);
  let r2 = Thread.join(t2);

  print("Results: " + r1 + ", " + r2);
}

main();
```

## 3. Memory Safety & Data Races

NovaJs is currently implementing a "Shared Heap" model protected by a global lock (GIL-like) or fine-grained locks on objects.

- **Heap Access**: The `HEAP` global in `runtime.rs` is a `Mutex<Heap>`. Every property access (`m_get`, `m_set`) acquires this lock.
- **Implication**: This ensures memory safety (no segfaults from concurrent implementation map modifications), but heavy contention can reduce parallelism benefits for shared object manipulation.
- **Best Practice**: Perform heavy computations using local variables (stack-based) or distinct objects before merging results to minimize lock contention.

## 4. Source Locations

- **`src/runtime.rs`**: implementation of `Thread_new`, `Thread_join`, `Mutex_new`.

## 5. Async/Await Implementation

Async/Await is built on top of the threading primitives but abstracts away manual thread management.

### Async Function Lowering

When an `async` function is defined, the compiler lowers it into two components:

1.  **Wrapper Function**: A function that creates a `Promise`, spawns a worker thread, and returns the Promise ID.
2.  **Worker Function**: A simulated state machine that executes the body of the async function.

### Argument Passing

Arguments to async functions are boxed and passed to the worker thread via an `any[]` array. The worker function automatically unboxes these arguments back to their primitive types (e.g., `int`, `float`) to ensure correct logic execution.

### Return Values

- **Wrapper**: Uses `TejxType::Any` as the return type to ensure the `Promise ID` is passed correctly without invalid unboxing by `codegen`.
- **Worker**: Resolves the promise using `__resolve_promise(id, val)` upon completion or `__reject_promise(id, error)` on exception.

### Runtime Support

- **`Promise`**: A synchronization primitive wrapping a `Mutex<PromiseState>` and `Condvar`.
- **`__await(promise_id)`**: Blocks the current thread (using `Condvar::wait`) until the target promise is resolved, then returns the value.
