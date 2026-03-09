# TejX Event Loop & Asynchronous Architecture

## 1. Overview

The TejX runtime leverages a strictly **Non-Blocking, Single-Threaded Reactor Architecture** modeled closely after V8, Node.js, and browser environments. The central goal is maximum concurrency for I/O bound tasks without introducing the complexity, synchronization bugs, or memory-safety violations associated with multi-threaded, parallel scripting code.

## 2. Core Architecture: The Tokio Reactor

Under the hood, TejX integrates the industry-standard Rust `tokio` asynchronous runtime.
However, instead of using a multi-threaded work-stealing scheduler, TejX explicitly initializes an embedded **Current-Thread Runtime** (`tokio::runtime::Builder::new_current_thread()`).

### Why Single-Threaded Tokio?

Dynamic languages that execute synchronously on shared heaps generally suffer when heavily parallelized (requiring massive global locks, like Python's GIL).
By employing a single-threaded runtime:

- **No Mutexes:** Core event queue structures, object properties, and closures can be accessed lock-free. Data races inside TejX code are impossible by design.
- **Microtask Efficiency:** The `thread_local! { static TASK_QUEUE: RefCell<VecDeque> }` allows virtually zero-overhead scheduling. Resolving a promise directly pushes to this thread-local queue memory.

## 3. The Event Loop Cycle

The event loop executes in a continuous tick process, defined physically by `tejx_run_event_loop_step`:

1. **Microtask Queue Processing:**
   The loop first actively drains the synchronous `TASK_QUEUE`. These are callbacks from immediately resolved promises, `await` continuations, and `.then()` chains.
2. **Reactor Polling (I/O & Timers):**
   If the microtask queue is empty but active asynchronous operations exist (`ASYNC_OPS > 0`), the runtime yields control down to the OS kernel (`epoll` on Linux, `kqueue` on macOS) via the `TOKIO_RT.block_on` cycle.
3. **Yielding control:**
   During this poll phase, background I/O operations (like socket reads/writes or HTTP streams) that are unblocked by the OS will push their completion payloads (the un-suspended closures) back onto the native TejX `TASK_QUEUE`.

## 4. Modernizing I/O Intrinsics

Standard blocking OS operations are disastrous for a single-threaded architecture. To maintain smoothness, TejX translates all classical blocking code into state-machine driven futures.

### Network Sockets (`tokio::net`)

When compiling `net.connect()`, the runtime invokes an intrinsic that spawns a non-blocking `TcpStream`.
Rather than blocking the CPU, the intrinsic registers the socket descriptor with the OS kernel. The thread is immediately released, and the TejX script receives a `Promise` representing the future connection.

### Asynchronous HTTP (`reqwest`)

High-level HTTP protocol handling operates over `reqwest`. When an HTTP request is fired, a new background Tokio task is spawned. The internal Rust implementation parses TLS and packet framing asynchronously, and upon completion, dynamically invokes `rt_promise_resolve`, pushing the final payload onto the TejX microtask queue.

## 5. Safely Crossing the Async / GC Boundary

The most dangerous aspect of mapping garbage-collected dynamic memory to background asynchronous kernel events is **object relocation** and **use-after-free**.

If a closure (or Promise pointer) is passed to a background HTTP task, and a GC cycle triggers while the HTTP request is in-flight, the Garbage Collector will move the Promise object in memory. If the background HTTP task attempts to resolve the old pointer address, a segmentation fault occurs.

### The Global Handle Resolution

To perfectly insulate background Tokio tasks from the TejX GC timeline:

- The runtime uses a `GLOBAL_HANDLES` registry.
- When an intrinsic pushes work to Tokio, it generates a unique, stationary integer ID (`tejx_create_global_handle`). The actual closure/Promise pointers never cross the thread boundary.
- The JIT compiler's GC scanner natively hooks into `GLOBAL_HANDLES`. It tracks exactly which handles are in flight, actively treating them as root pointers and accurately rewriting the pointers directly inside the registry when objects are compacted or evacuated.
- When the background Tokio task resolves, it returns the stationary integer ID to the main thread. The main thread dereferences the ID through the `GLOBAL_HANDLES` registry (which accurately points to the post-GC living memory address) and safely executes the callback.
