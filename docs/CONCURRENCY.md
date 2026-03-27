# TejX Concurrency

TejX currently supports two different concurrency models:

- `async` / `await` for non-blocking workflows
- native OS threads through `std:thread` for parallel work

These models serve different purposes and should not be treated as interchangeable.

## Async / Await

Async functions are designed for timers, networking, file I/O, and other latency-bound work:

```tx
async function fetchData(): string {
    await delay(100);
    return "done";
}
```

### Runtime Model

The async runtime is implemented in `src/runtime/event_loop.rs`.

Important runtime pieces:

- `MICROTASK_QUEUE`: promise continuations and `await` resumes
- `TASK_QUEUE`: pending TejX callbacks waiting to run
- `ASYNC_OPS`: count of in-flight async operations
- `TASK_NOTIFY`: wakeup signal for the event loop
- `TOKIO_RT`: multithreaded Tokio runtime for timers, sockets, and other background async work

### Event Loop Behavior

Each event-loop step follows this pattern:

1. drain promise/`await` microtasks first
2. run at most one queued timer/I/O callback
3. drain any microtasks created by that callback
4. if no tasks remain and `ASYNC_OPS == 0`, stop
5. otherwise park until background async work or a queued callback wakes the loop

This keeps TejX user callbacks single-threaded while allowing timers and I/O futures to keep progressing on Tokio worker threads even when the main loop is not actively inside `block_on`.

### Important Properties

- async user callbacks resume on the TejX event loop
- background I/O and timers may advance on Tokio worker threads, but completion is funneled back through the task queue
- GC-visible values that outlive the current turn are protected through global handles so moving GC can update them safely

## Native Threads

For CPU-bound parallel work, TejX provides thread primitives in `std:thread`.

Common primitives include:

- `Thread<T>`
- `Mutex`
- `Atomic`
- `Condition`
- `SharedQueue<T>`

Typical usage:

```tx
import { Thread } from "std:thread";

function worker(n: int): void {
    print(n);
}

let t = new Thread<int>(worker, 42);
t.start();
t.join();
```

Use native threads when you want real parallel execution across cores.

## Choosing Between Async and Threads

Use async when:

- you are waiting on I/O
- you need timers
- you want sequential-looking non-blocking code

Use threads when:

- you need true parallel CPU work
- a task would otherwise block the event loop
- shared-state synchronization is acceptable

## Best Practices

- do not put heavy CPU loops on the async event loop
- prefer async for networking and timers
- prefer threads for compute-heavy loops
- protect shared mutable data in threaded code with `Mutex`, `Atomic`, or message passing
- narrow `Optional<T>` values before dereferencing them inside long-running async or threaded logic
