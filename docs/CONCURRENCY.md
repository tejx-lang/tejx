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

- `TASK_QUEUE`: pending TejX callbacks waiting to run
- `ASYNC_OPS`: count of in-flight async operations
- `TASK_NOTIFY`: wakeup signal for the event loop
- `TOKIO_RT`: current-thread Tokio runtime used to poll async work

### Event Loop Behavior

Each event-loop step follows this pattern:

1. pop a task from the TejX task queue
2. run it immediately if one exists
3. if no tasks remain and `ASYNC_OPS == 0`, stop
4. otherwise let Tokio poll background async work
5. wake again when a task is enqueued back into the TejX queue

This keeps TejX user code single-threaded from the async point of view while still allowing non-blocking integration with timers, sockets, and HTTP.

### Important Properties

- async user callbacks resume on the TejX event loop
- background I/O may involve Tokio or worker-side activity, but completion is funneled back through the task queue
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
