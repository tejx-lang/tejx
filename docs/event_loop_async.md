# TejX Event Loop & Asynchronous Architecture

## 1. Overview

TejX implements a **Single-Threaded Reactor Pattern** for concurrency. While the underlying runtime is multi-threaded (Rust's Tokio), the TejX execution environment remains strictly single-threaded to guarantee memory safety and eliminate the need for complex synchronization in user scripts.

---

## 2. The Core: Current-Thread Tokio

TejX embeds a specialized instance of the Tokio runtime configured in `CurrentThread` mode.

### Why Single-Threaded?

- **Deterministic Heap:** Since only one thread modifies the TejX heap at any given time, we avoid the performance overhead of Mutexes and Atomic Pointers for every object access.
- **Zero-Cost Promises:** Resolving a promise is as simple as pushing a pointer to a thread-local queue. There is no cross-thread signaling or thread-safety overhead.

---

## 3. The Lifecycle: Tick & Poll

The event loop follows a strictly defined cycle to balance execution and I/O responsiveness.

### Step 1: Microtask Queue (The Task Stack)

- **Priority:** Draining the microtask queue is the highest priority.
- **Content:** These are immediately resolved promise callbacks and `await` continuations.
- **Reasoning:** Microtasks must complete before the next I/O poll to ensure that logical chains (e.g., `Promise.resolve().then(...)`) execute with minimal latency and consistent state.

### Step 2: Reactor Polling (Tokio `block_on`)

- **Action:** If the microtask queue is empty but asynchronous operations are in flight, the runtime invokes `tokio::runtime::Runtime::block_on`.
- **Reasoning:** This yields the CPU back to the OS, allowing the kernel to awaken the thread only when I/O (sockets, files, timers) is ready.

---

## 4. Async Intrinsics: Bridging to Native I/O

TejX provides native "intrinsics" that wrap top-tier Rust asynchronous libraries.

### Network Stack (`tokio::net`)

- When you call `socket.read()`, the compiler emits a call to a native intrinsic.
- This intrinsic returns a **Promise** immediately and registers a "Wakeup" closure with the OS.
- This ensures the main TejX thread never blocks on network latency.

### HTTP Stack (`reqwest`)

- High-level requests use `reqwest` in the background.
- Because `reqwest` operations can involve complex TLS handshaking, they are offloaded to background worker threads.
- Upon completion, they use the **Global Handle** system to safely signal the main thread from a background context.

---

## 5. Async/Await: Structural Lowering

The compiler transforms `async` functions into state machines.

- **Suspension:** When an `await` is encountered, the current function's state (local variables, instruction pointer) is captured in a **Closure** (Environment).
- **Resumption:** Once the awaited promise resolves, the event loop picks up this closure from the microtask queue and restores execution exactly where it left off.
- **Reasoning:** This "Syntactic Sugar" allows developers to write sequential-looking code that is internally executing as a high-efficiency async state machine.
