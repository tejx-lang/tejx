# TejX Standard Library & Prelude API Reference

This document provides a comprehensive, exhaustive reference for all built-in global functions, classes, methods, and standard library modules available in **TejX**.

---

## Table of Contents

1. [Core & Prelude (Always Available Globally)](#1-core--prelude-always-available-globally)
   - [Global Functions](#11-global-functions)
   - [Error Hierarchy](#12-error-hierarchy)
   - [Object Utilities](#13-object-utilities)
   - [Thread & Virtual Concurrency](#14-thread--virtual-concurrency)
   - [Promises & Task Combinators](#15-promises--task-combinators)
   - [String Built-in Methods](#16-string-built-in-methods)
   - [Array Built-in Methods](#17-array-built-in-methods)
2. [`std:collections` (Data Structures)](#2-stdcollections-data-structures)
   - [`Map<K, V>`](#21-mapk-v)
   - [`Set<T>`](#22-sett)
   - [`Stack<T>`](#23-stackt)
   - [`Deque<T>`](#24-dequet)
   - [`Queue<T>`](#25-queuet)
   - [`MinHeap<T>` & `MaxHeap<T>`](#26-minheapt--maxheapt)
   - [`PriorityQueue<T>`](#27-priorityqueuet)
   - [`OrderedMap<K, V>` & `OrderedSet<T>`](#28-orderedmapk-v--orderedsett)
   - [`Trie`](#29-trie)
   - [`BloomFilter`](#210-bloomfilter)
3. [`std:fs` (File System)](#3-stdfs-file-system)
4. [`std:http` (HTTP Client & High-Performance Server)](#4-stdhttp-http-client--high-performance-server)
   - [HTTP Client (`fetch`, `Request`, `Response`, `Headers`)](#41-http-client)
   - [HTTP Server (`HttpServer`, `ServerRequest`, `ServerResponse`, `PathParams`)](#42-http-server)
5. [`std:net` (TCP Networking & TLS)](#5-stdnet-tcp-networking--tls)
   - [Network Functions & Namespaces](#51-network-functions--namespaces)
   - [`TcpStream`](#52-tcpstream)
   - [`TcpListener`](#53-tcplistener)
6. [`std:dns` (Domain Name Resolution)](#6-stddns-domain-name-resolution)
7. [`std:url` (URL Parsing)](#7-stdurl-url-parsing)
8. [`std:json` (JSON Serialization & Parsing)](#8-stdjson-json-serialization--parsing)
9. [`std:math` (Mathematical Functions & Constants)](#9-stdmath-mathematical-functions--constants)
10. [`std:time` (Time, Timers & Date)](#10-stdtime-time-timers--date)
11. [`std:system`, `os`, `process` (Operating System & Process Control)](#11-stdsystem-os-process-operating-system--process-control)
12. [`std:crypto` (Cryptography & Hashing)](#12-stdcrypto-cryptography--hashing)
13. [`std:binary` (Binary Encoding, ByteReader & ByteWriter)](#13-stdbinary-binary-encoding-bytereader--bytewriter)
14. [`std:thread` (Multi-Thread Synchronization Primitives)](#14-stdthread-multi-thread-synchronization-primitives)
15. [`std:gc` (Garbage Collector Introspection & Tuning)](#15-stdgc-garbage-collector-introspection--tuning)
16. [`std:runtime` (Engine & Virtual Thread Metrics)](#16-stdruntime-engine--virtual-thread-metrics)

---

## 1. Core & Prelude (Always Available Globally)

The following functions, classes, and prototypes are loaded into every TejX program automatically without any `import` statement.

### 1.1 Global Functions

#### `print(...args: any[]): void`
Formats and writes arguments to standard output followed by a newline.
- Supports primitive values, complex objects, errors with full stack frames, arrays, circular references, and stringification.
- Arguments are separated by a space.

```tx
print("Hello", 42, true, [1, 2, 3]);
```

#### `panic(msg: string): void`
Prints an error report to `stderr` and immediately terminates the process with a non-zero exit code.
```tx
panic("Fatal state encountered");
```

#### `sizeof<T>(val: T): int`
Returns the in-memory byte size of the given value or data structure.
```tx
let size = sizeof(100); // 8 bytes
```

#### `String(v: any): string`
Converts any value to its string representation.
```tx
let s: string = String(12345);
```

#### `Boolean(v: any): bool`
Converts any value to a boolean (`false` for `None`, otherwise `v as bool`).
```tx
let b = Boolean(true);
let empty = Boolean(None); // false
```

#### `None`
The single canonical representation for absent, null, or empty values. `null` and `undefined` are invalid in TejX.

---

### 1.2 Array Utilities (`class Array`)

The built-in `Array` namespace provides array reflection and checking utilities:

```tx
let isList = Array.isArray([1, 2, 3]); // true
let notList = Array.isArray("hello"); // false
```

---

### 1.2 Number Utilities (`class Number`)

The built-in global `Number` class provides numeric conversions from strings and arbitrary values with support for all integer and floating-point sizes:

| Method | Signature | Description |
|---|---|---|
| `Float` | `static Float(val: any): float` | Parses/converts value to default float (`float32`). |
| `Float32` | `static Float32(val: any): float32` | Parses/converts value to 32-bit float. |
| `Float64` | `static Float64(val: any): float64` | Parses/converts value to 64-bit double float. |
| `Int` | `static Int(val: any): int` | Parses/converts value to default integer (`int32`). |
| `Int8` | `static Int8(val: any): int8` | Parses/converts value to 8-bit signed integer. |
| `Int16` | `static Int16(val: any): int16` | Parses/converts value to 16-bit signed integer. |
| `Int32` | `static Int32(val: any): int32` | Parses/converts value to 32-bit signed integer. |
| `Int64` | `static Int64(val: any): int64` | Parses/converts value to 64-bit signed integer. |
| `UInt` | `static UInt(val: any): uint` | Parses/converts value to default unsigned integer (`uint32`). |
| `UInt8` | `static UInt8(val: any): uint8` | Parses/converts value to 8-bit unsigned integer. |
| `UInt16` | `static UInt16(val: any): uint16` | Parses/converts value to 16-bit unsigned integer. |
| `UInt32` | `static UInt32(val: any): uint32` | Parses/converts value to 32-bit unsigned integer. |
| `UInt64` | `static UInt64(val: any): uint64` | Parses/converts value to 64-bit unsigned integer. |

```tx
let pi = Number.Float64("3.1415926535");
let port = Number.Int("8080");
let byte = Number.UInt8("255");
```

---

### 1.2 Error Hierarchy

#### `class Error`
Base class for all exceptions in TejX.

```tx
class Error {
    message: string;
    stack: string;
    code: int32;

    constructor(msg: string, code: Optional<int> = None);
    getMessage(): string;
    getStack(): string;
    getStackFrames(): string[];
    getName(): string;
    toString(): string;
}
```

#### Derived Errors
- **`class RuntimeError extends Error`**: Indicates an unexpected runtime violation or failed assertion.
- **`class PanicError extends Error`**: Raised during unrecoverable engine panics.
- **`class AggregateError extends Error`**: Contains a list of underlying errors:
  - `errors: Error[]`

---

### 1.3 Object Utilities

#### `class Object`
Static helper methods for working with objects, maps, and structs:

| Method | Signature | Description |
|---|---|---|
| `keys` | `static keys<T>(obj: T): string[]` | Returns an array of an object's enumerable property names. |
| `values` | `static values<T>(obj: T): any[]` | Returns an array of an object's property values. |
| `entries` | `static entries<T>(obj: T): any[]` | Returns an array of key-value pairs `[key, value]`. |
| `assign` | `static assign<T, U>(target: T, source: U): T` | Copies all own properties from `source` into `target`. |
| `freeze` | `static freeze<T>(obj: T): T` | Freezes an object preventing further property mutation. |

```tx
let user = { name: "Alice", age: 30 };
let keys = Object.keys(user); // ["name", "age"]
```

---

### 1.4 Thread & Virtual Concurrency

#### `class Thread`
Manages lightweight M:N virtual threads (goroutines).

| Member | Signature | Description |
|---|---|---|
| `constructor` | `constructor(task: () => void)` | Creates a new thread with the given task and starts it. |
| `start` | `start(): void` | Begins execution of the task if not already started. |
| `join` | `join(): void` | Blocks the calling virtual thread until this thread completes. |
| `spawn` | `static spawn(task: () => void): Thread` | Factory method to instantiate and run a virtual thread. |
| `sleep` | `static sleep(ms: int): void` | Parks the current virtual thread for `ms` milliseconds without blocking the OS worker thread. |

```tx
let t = Thread.spawn(() => {
    Thread.sleep(50);
    print("Worker complete");
});
t.join();
```

---

### 1.5 Promises & Task Combinators

#### `class PromiseResult<T>`
Result object returned by `Promise.settled`:
- `status: string`: Either `"fulfilled"` or `"rejected"`.
- `value: Optional<T>`: Result value if fulfilled; `None` otherwise.
- `error: Optional<Error>`: Error thrown if rejected; `None` otherwise.

#### `class Promise<T>`
Provides asynchronous task execution and synchronization.

| Method | Signature | Description |
|---|---|---|
| `then` | `then<U>(onResolve: (v: T) => U, onReject?: (e: Error) => U): Promise<U>` | Chaining handler on resolution. |
| `catch` | `catch<U>(onReject: (e: Error) => U): Promise<U>` | Error handler on rejection. |
| `catchError`| `catchError<U>(onReject: (e: Error) => U): Promise<U>`| Alias for `catch`. |
| `spawn` | `static spawn<U>(task: () => U): Promise<U>` | Runs `task` in a background virtual thread, returning a `Promise`. |
| `all` | `static all(tasks: any[]): any[]` | Runs all task closures in parallel virtual threads. Blocks until all complete. Throws immediately on first error. |
| `settled` | `static settled(tasks: any[]): PromiseResult<any>[]` | Runs all task closures in parallel virtual threads. Never throws; returns `PromiseResult` for every task. |

```tx
let results = Promise.all([
    () => 10 + 20,
    () => 30 + 40
]);
print(results[0]); // 30
```

---

### 1.6 String Built-in Methods

Available on all `string` instances or via module exports:

| Method | Signature | Description |
|---|---|---|
| `length()` | `length(): int` | Returns the UTF-8 byte length of the string. |
| `toUpperCase()` | `toUpperCase(): string` | Returns a copy converted to uppercase. |
| `toLowerCase()` | `toLowerCase(): string` | Returns a copy converted to lowercase. |
| `trim()` | `trim(): string` | Strips leading and trailing whitespace. |
| `trimStart()` | `trimStart(): string` | Strips leading whitespace. |
| `trimEnd()` | `trimEnd(): string` | Strips trailing whitespace. |
| `substring()` | `substring(start: int, end: int): string` | Returns substring between `start` and `end` indices. |
| `split()` | `split(sep: string): string[]` | Splits string by delimiter `sep`. |
| `startsWith()` | `startsWith(prefix: string): bool` | Returns `true` if string starts with `prefix`. |
| `endsWith()` | `endsWith(suffix: string): bool` | Returns `true` if string ends with `suffix`. |
| `indexOf()` | `indexOf(search: string): int` | Returns first index of substring `search`, or `-1`. |
| `padStart()` | `padStart(len: int, pad: string): string` | Pads string on the left to length `len`. |
| `padEnd()` | `padEnd(len: int, pad: string): string` | Pads string on the right to length `len`. |
| `repeat()` | `repeat(count: int): string` | Repeats string `count` times. |
| `replace()` | `replace(search: string, rep: string): string` | Replaces occurrences of `search` with `rep`. |
| `includes()` | `includes(search: string): bool` | Returns `true` if `search` appears anywhere in the string. |

---

### 1.7 Array Built-in Methods

Available on all dynamic arrays (`T[]`):

| Method | Signature | Description |
|---|---|---|
| `length()` | `length(): int` | Returns the number of elements in the array. |
| `push()` | `push(val: T): int` | Appends `val` to the end of the array, returning new length. |
| `pop()` | `pop(): T` | Removes and returns the last element. |
| `shift()` | `shift(): T` | Removes and returns the first element. |
| `unshift()` | `unshift(val: T): int` | Prepends `val` to the front, returning new length. |
| `indexOf()` | `indexOf(val: T): int` | Returns first index of `val`, or `-1`. |
| `concat()` | `concat(other: T[]): T[]` | Returns new array combining this array and `other`. |
| `join()` | `join(sep: string): string` | Joins all elements with delimiter `sep`. |
| `slice()` | `slice(start: int, end: int): T[]` | Returns a slice between `start` and `end`. |
| `reverse()` | `reverse(): T[]` | Reverses elements in-place and returns array. |
| `fill()` | `fill(val: T): T[]` | Fills all array elements with `val`. |
| `sort()` | `sort(): void` | Sorts array elements in-place in ascending order. |
| `forEach()` | `forEach(cb: (val: T, idx: int) => void): void` | Executes `cb` for each element. |
| `map()` | `map<U>(cb: (val: T, idx: int) => U): U[]` | Transforms each element via `cb`. |
| `filter()` | `filter(predicate: (val: T, idx: int) => bool): T[]` | Returns elements satisfying predicate. |
| `reduce()` | `reduce<U>(cb: (acc: U, v: T, i: int) => U, init: U): U` | Accumulates elements into a single value. |
| `find()` | `find(cb: (v: T, i: int) => bool): T` | Returns first element matching predicate. |
| `findIndex()`| `findIndex(cb: (v: T, i: int) => bool): int` | Returns index of first element matching predicate. |
| `includes()` | `includes(val: T): bool` | Returns `true` if `val` is contained in array. |
| `every()` | `every(cb: (v: T, i: int) => bool): bool` | Returns `true` if every element matches predicate. |
| `some()` | `some(cb: (v: T, i: int) => bool): bool` | Returns `true` if at least one element matches predicate. |
| `flat()` | `flat(depth: int = 1): T[]` | Recursively flattens nested arrays up to `depth`. |
| `isArray()` | `isArray(val: any): bool` | Returns `true` if the given argument is an array. |

---

## 2. `std:collections` (Data Structures)

Import:
```tx
import std:collections;
// or
import { Map, Set, Stack, Queue, Deque, MinHeap, MaxHeap, PriorityQueue, OrderedMap, OrderedSet, Trie, BloomFilter } from "std:collections";
```

### 2.1 `Map<K, V>`
High-performance hash table using Robin Hood probing with backward shift deletion.

```tx
class Map<K, V> {
    constructor();
    set(key: K, val: V): void;
    put(key: K, val: V): void;               // Alias for set
    get(key: K): V;                          // Returns value or None
    at(key: K): V;                           // Alias for get
    getOrDefault(key: K, fallback: V): V;
    getOrSet(key: K, defaultValue: V): V;
    has(key: K): bool;
    delete(key: K): bool;                    // Returns true if removed
    remove(key: K): bool;                    // Alias for delete
    clear(): void;
    size(): int;
    isEmpty(): bool;
    keys(): K[];
    values(): V[];
    entries(): MapEntry<K, V>[];
    forEach(callback: (val: V, key: K, idx: int) => void): void;
    clone(): Map<K, V>;
    merge(other: Map<K, V>): void;
    update(other: Map<K, V>): void;          // Alias for merge
    toFlatArray(): any[];                    // [k1, v1, k2, v2, ...]
    reserve(entries: int): void;
    shrinkToFit(): void;
}
```

### 2.2 `Set<T>`
Hash set for unique elements.

```tx
class Set<T> {
    constructor();
    add(val: T): void;
    has(val: T): bool;
    contains(val: T): bool;                  // Alias for has
    delete(val: T): bool;
    remove(val: T): bool;                    // Alias for delete
    clear(): void;
    size(): int;
    isEmpty(): bool;
    values(): T[];
    keys(): T[];
    entries(): SetEntry<T>[];
    forEach(callback: (val: T, idx: int) => void): void;
    toArray(): T[];
    clone(): Set<T>;
    union(other: Set<T>): Set<T>;
    intersection(other: Set<T>): Set<T>;
    difference(other: Set<T>): Set<T>;
    equals(other: Set<T>): bool;
    isSubsetOf(other: Set<T>): bool;
    reserve(entries: int): void;
    shrinkToFit(): void;
}
```

### 2.3 `Stack<T>`
LIFO (Last-In-First-Out) stack.

```tx
class Stack<T> {
    constructor();
    push(val: T): void;
    pop(): T;                                // Returns None if empty
    peek(): T;
    size(): int;
    isEmpty(): bool;
    clear(): void;
    toArray(): T[];
}
```

### 2.4 `Deque<T>`
Double-ended queue backed by a dynamic circular buffer.

```tx
class Deque<T> {
    constructor();
    pushFront(val: T): void;
    pushBack(val: T): void;
    enqueue(val: T): void;                   // Alias for pushBack
    popFront(): T;                           // Returns None if empty
    popBack(): T;                            // Returns None if empty
    dequeue(): T;                            // Alias for popFront
    peekFront(): T;
    peekBack(): T;
    size(): int;
    isEmpty(): bool;
    clear(): void;
    toArray(): T[];
    reserve(capacity: int): void;
}
```

### 2.5 `Queue<T>`
FIFO (First-In-First-Out) queue.

```tx
class Queue<T> {
    constructor();
    enqueue(val: T): void;
    dequeue(): T;                            // Returns None if empty
    peek(): T;
    front(): T;
    back(): T;
    size(): int;
    isEmpty(): bool;
    clear(): void;
    toArray(): T[];
    reserve(capacity: int): void;
}
```

### 2.6 `MinHeap<T>` & `MaxHeap<T>`
Binary heap implementations.

```tx
class MinHeap<T> {
    constructor();
    insert(val: T): void;
    peek(): T;
    extractMin(): T;
    size(): int;
    isEmpty(): bool;
    clear(): void;
    toArray(): T[];
}

class MaxHeap<T> {
    constructor();
    insert(val: T): void;
    insertMax(val: T): void;
    peek(): T;
    extractMax(): T;
    size(): int;
    isEmpty(): bool;
    clear(): void;
    toArray(): T[];
}
```

### 2.7 `PriorityQueue<T>`
Priority queue ordered by minimum element first.

```tx
class PriorityQueue<T> {
    constructor();
    insert(val: T): void;
    enqueue(val: T): void;
    peek(): T;
    extractMin(): T;
    dequeue(): T;
    size(): int;
    isEmpty(): bool;
    clear(): void;
}
```

### 2.8 `OrderedMap<K, V>` & `OrderedSet<T>`
Maintains deterministic insertion order for iteration.
- `OrderedMap<K, V>` provides all methods of `Map<K, V>`.
- `OrderedSet<T>` provides all methods of `Set<T>`.

### 2.9 `Trie`
Prefix tree for efficient string search and autocomplete.

```tx
class Trie {
    constructor();
    set(key: string, val: int): void;
    addPath(key: string, val: int): void;
    get(key: string): int;                   // Returns 0 if not found
    find(key: string): int;
    has(key: string): bool;
    startsWith(prefix: string): bool;
    delete(key: string): bool;
    remove(key: string): bool;
}
```

### 2.10 `BloomFilter`
Space-efficient probabilistic membership testing.

```tx
class BloomFilter {
    constructor(size: int, numHashes: int);
    add(val: string): void;
    contains(val: string): bool;
    mightContain(val: string): bool;
    clear(): void;
}
```

---

## 3. `std:fs` (File System)

Import:
```tx
import std:fs;
// or
import { readFile, writeFile, exists, mkdir, readdir, remove } from "std:fs";
```

### File System Operations

| Function | Signature | Description |
|---|---|---|
| `exists` | `exists(path: string): bool` | Returns `true` if file or directory exists at `path`. |
| `readFile` | `readFile(path: string): string` | Reads entire file into a UTF-8 string. |
| `writeFile` | `writeFile(path: string, content: string): bool` | Writes string to file, truncating existing contents. |
| `appendFile` | `appendFile(path: string, content: string): bool` | Appends `content` to the end of file. |
| `remove` | `remove(path: string): bool` | Deletes a file or directory. Returns `true` on success. |
| `unlink` | `unlink(path: string): bool` | Deletes a file. Returns `true` on success. |
| `mkdir` | `mkdir(path: string): bool` | Creates a single directory. |
| `ensureDir` | `ensureDir(path: string): bool` | Creates directory recursively if it does not already exist. |
| `listDir` | `listDir(path: string): string[]` | Lists file and directory entries inside `path`. |
| `readdir` | `readdir(path: string): string[]` | Lists file and directory entries inside `path`. |

All operations are available both at the top-level module scope (`import { readFile, writeFile } from "std:fs"`) and under the `fs.*` namespace (`import std:fs; fs.readFile(...)`).

---

## 4. `std:http` (HTTP Client & High-Performance Server)

Import:
```tx
import std:http;
// or
import { fetch, Request, Response, Headers, HttpServer, ServerRequest, ServerResponse } from "std:http";
```

### 4.1 HTTP Client

#### `fetch(input: any, options?: any): Response`
Performs an HTTP/HTTPS request synchronously or within a virtual thread.
```tx
let res = fetch("https://api.github.com/zen");
print("Status: " + res.status);
print("Body: " + res.text());
```

With options:
```tx
let res = fetch("https://example.com/api/create", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: { name: "TejX" },
    timeoutMs: 5000
});
```

#### `class Headers`
```tx
class Headers {
    constructor(init: Optional<any> = None);
    set(name: string, value: string): void;
    get(name: string): Optional<string>;
    has(name: string): bool;
    delete(name: string): void;
    keys(): string[];
    copy(): Headers;
}
```

#### `class Response`
```tx
class Response {
    status: int;
    statusText: string;
    httpVersion: string;
    headers: Headers;
    body: string;

    ok(): bool;                              // status >= 200 && status < 300
    text(): string;                          // Returns raw body string
    json<T>(): T;                            // Deserializes body via json.parse
    getHeader(name: string): Optional<string>;
}
```

---

### 4.2 HTTP Server

High-performance, asynchronous web server built on TejX virtual threads and `epoll`/`kqueue` non-blocking sockets.

```tx
let app = new HttpServer();

app.get("/", (req: ServerRequest, res: ServerResponse) => {
    res.json({ message: "Welcome to TejX!" });
});

app.get("/user/:id", (req: ServerRequest, res: ServerResponse) => {
    let id = req.params.get("id");
    res.text(`User ID: ${id}`);
});

app.listen(8080, () => {
    print("Server running on http://127.0.0.1:8080");
});
```

#### `class HttpServer`

| Method | Signature | Description |
|---|---|---|
| `get` | `get(path: string, handler: (req, res) => void): void` | Registers a GET handler. |
| `post` | `post(path: string, handler: (req, res) => void): void` | Registers a POST handler. |
| `put` | `put(path: string, handler: (req, res) => void): void` | Registers a PUT handler. |
| `delete_` | `delete_(path: string, handler: (req, res) => void): void` | Registers a DELETE handler. |
| `patch` | `patch(path: string, handler: (req, res) => void): void` | Registers a PATCH handler. |
| `options` | `options(path: string, handler: (req, res) => void): void` | Registers an OPTIONS handler. |
| `head` | `head(path: string, handler: (req, res) => void): void` | Registers a HEAD handler. |
| `all` | `all(path: string, handler: (req, res) => void): void` | Registers a handler matching all HTTP verbs. |
| `route` | `route(methods: string[], path: string, handler): void`| Registers handler for specified methods. |
| `enableCors`| `enableCors(origin: string): void` | Enables CORS headers and auto-handles OPTIONS preflight. |
| `enableCorsAll`| `enableCorsAll(): void` | Enables CORS for all origins (`*`). |
| `listen` | `listen(port: int, callback: () => void): void` | Starts listening. Throws `EADDRINUSE` if port is occupied. |

#### `class ServerRequest`
- `method: string`: HTTP verb (e.g. `"GET"`, `"POST"`).
- `path: string`: Clean route path (excluding query string).
- `rawPath: string`: Full URI path including query string.
- `headers: Map<string, string>`: Request headers (keys lowercased).
- `body: string`: Request payload string.
- `params: PathParams`: Extracted path variables (`req.params.get("id")`).
- `query: Map<string, string>`: Parsed URL query parameters (`req.query.get("search")`).
- `getHeader(name: string): Optional<string>`: Case-insensitive header lookup.
- `json<T>(): T`: Deserializes request body as JSON.

#### `class ServerResponse`
- `status: int`: HTTP status code (defaults to `200`).
- `headers: Map<string, string>`: Response headers.
- `setStatus(code: int): ServerResponse`: Sets status code (chainable).
- `setHeader(name: string, value: string): ServerResponse`: Adds header (chainable).
- `cors(origin: string): ServerResponse`: Sets CORS headers for origin.
- `corsAll(): ServerResponse`: Sets `Access-Control-Allow-Origin: *`.
- `send(body: string): void`: Flushes response with string content.
- `json(data: any): void`: Serializes `data` to JSON, sets `Content-Type: application/json` and flushes.
- `text(body: string): void`: Sets `Content-Type: text/plain` and flushes.
- `html(body: string): void`: Sets `Content-Type: text/html` and flushes.
- `redirect(location: string): void`: Sends 302 Found redirect.
- `redirectWithCode(location: string, code: int): void`: Sends redirect with custom code (e.g. 301).

---

## 5. `std:net` (TCP Networking & TLS)

Import:
```tx
import std:net;
// or
import { TcpStream, TcpListener, connect, listen } from "std:net";
```

### 5.1 Network Functions & Namespaces

```tx
namespace net {
    connect(addr: string): Optional<TcpStream>;
    connectHost(host: string, port: int = 80): Optional<TcpStream>;
    connectTls(host: string, port: int = 443): Optional<TcpStream>;
    connectTlsInsecure(host: string, port: int = 443): Optional<TcpStream>;
    listen(addr: string): TcpListener;
    close(stream: Optional<any>): void;
}
```

### 5.2 `TcpStream`
Non-blocking TCP socket stream.

| Method | Signature | Description |
|---|---|---|
| `write` | `write(data: string): bool` | Sends UTF-8 string over socket. |
| `send` | `send(data: string): bool` | Alias for `write`. |
| `writeBytes` | `writeBytes(data: int[]): bool` | Sends raw byte array over socket. |
| `read` | `read(max_len: int): string` | Reads up to `max_len` bytes as string. |
| `receive` | `receive(max_len: int): string` | Alias for `read`. |
| `readBytes` | `readBytes(max_len: int): int[]` | Reads up to `max_len` raw bytes. |
| `readAll` | `readAll(): string` | Reads all data until connection closes as string. |
| `readAllBytes` | `readAllBytes(): int[]` | Reads all data until connection closes as bytes. |
| `readExactBytes`| `readExactBytes(expected: int): Optional<int[]>`| Reads precisely `expected` bytes or returns `None`. |
| `startTls` | `startTls(host: string): bool` | Upgrades stream to secure TLS with SNI verification. |
| `startTlsInsecure`| `startTlsInsecure(host: string): bool`| Upgrades stream to TLS ignoring invalid certificates. |
| `setTimeout` | `setTimeout(timeout_ms: int): bool` | Configures socket read/write timeouts. |
| `isOpen` | `isOpen(): bool` | Returns `true` if socket descriptor is open. |
| `close` | `close(): void` | Closes socket descriptor. Safe against double-close. |

### 5.3 `TcpListener`
TCP server listener socket.

| Method | Signature | Description |
|---|---|---|
| `bind` | `static bind(addr: string): TcpListener` | Binds to `IP:Port`. Throws `EADDRINUSE` if busy. |
| `tryAccept` | `tryAccept(): Optional<TcpStream>` | Non-blocking accept; returns `None` if no client pending. |
| `accept` | `accept(): TcpStream` | Parks virtual thread until client connects. Throws on error. |
| `isOpen` | `isOpen(): bool` | Returns `true` if listener is open. |
| `close` | `close(): void` | Closes listening socket. |

---

## 6. `std:dns` (Domain Name Resolution)

Import:
```tx
import std:dns;
```

| Function | Signature | Description |
|---|---|---|
| `dns.lookup` | `lookup(host: string): Optional<string>` | Resolves hostname to its first IPv4/IPv6 address. |
| `dns.lookupAll`| `lookupAll(host: string): string[]` | Resolves hostname to all available IP addresses. |

---

## 7. `std:url` (URL Parsing)

Import:
```tx
import std:url;
```

### `class URL`
Parses and breaks down standard RFC URLs.

```tx
let u = new URL("https://user:pass@api.example.com:8443/v1/resource?page=2#top");

print(u.scheme);     // "https"
print(u.host);       // "api.example.com"
print(u.port);       // "8443"
print(u.hostHeader); // "api.example.com:8443"
print(u.resource);   // "/v1/resource?page=2#top"
```

---

## 8. `std:json` (JSON Serialization & Parsing)

Import:
```tx
import std:json;
// or
import { parse, tryParse, stringify, format } from "std:json";
```

| Function | Signature | Description |
|---|---|---|
| `json.stringify`| `stringify(val: any, space?: any): string` | Serializes any value to a JSON string (with optional indentation `space`). |
| `json.parse` | `parse(s: string): any` | Parses JSON string into native object/array. Throws on invalid syntax. |
| `json.tryParse` | `tryParse(s: string): any` | Parses JSON string; returns `None` if syntax is invalid. |

---

## 9. `std:math` (Mathematical Functions & Constants)

Import:
```tx
import std:math;
// or
import { pi, e, abs, min, max, clamp, sqrt, pow, sin, cos, random, randomInt } from "std:math";
```

### Constants
- `pi(): float` $\rightarrow 3.141592653589793$
- `tau(): float` $\rightarrow 6.283185307179586$
- `e(): float` $\rightarrow 2.718281828459045$

### Basic Math

| Function | Signature | Description |
|---|---|---|
| `abs` | `abs(x: float): float` | Returns absolute value $|x|$. |
| `min` | `min(a: float, b: float): float` | Returns minimum of $a$ and $b$. |
| `max` | `max(a: float, b: float): float` | Returns maximum of $a$ and $b$. |
| `clamp` | `clamp(val: float, low: float, high: float): float` | Clamps `val` to range `[low, high]`. |
| `sign` | `sign(value: float): int` | Returns `1` if positive, `-1` if negative, `0` if zero. |
| `lerp` | `lerp(start: float, end: float, t: float): float` | Linear interpolation between `start` and `end`. |
| `trunc` | `trunc(x: float): float` | Truncates fractional digits. |
| `floor` | `floor(x: float): float` | Rounds down to largest integer $\le x$. |
| `ceil` | `ceil(x: float): float` | Rounds up to smallest integer $\ge x$. |
| `round` | `round(x: float): float` | Rounds to nearest integer. |
| `sqrt` | `sqrt(x: float): float` | Returns square root $\sqrt{x}$. |
| `exp` | `exp(x: float): float` | Returns exponential $e^x$. |
| `ln` | `ln(x: float): float` | Returns natural logarithm $\ln(x)$. |
| `pow` | `pow(base: float, exponent: float): float` | Returns $\text{base}^{\text{exponent}}$. |
| `powi` | `powi(base: float, exponent: int): float` | Integer exponentiation. |

### Trigonometry
- `sin(x: float): float`
- `cos(x: float): float`
- `tan(x: float): float`
- `asin(x: float): float`
- `acos(x: float): float`
- `atan(x: float): float`

### Random Numbers
- `seedRandom(seed: int64): void`: Seeds the pseudorandom generator.
- `random(): float`: Returns a random float in range $[0.0, 1.0)$.
- `randomInt(lower: int, upper: int): int`: Returns random integer in range $[\text{lower}, \text{upper}]$.

### `class Math` (Static Class Utilities)
When importing `std:math`, the `Math` class is available with static methods:
```tx
import std:math;

let pi = Math.PI();
let root = Math.sqrt(25.0);
let maximum = Math.max(10.0, 20.0);
let clamped = Math.clamp(15.0, 0.0, 10.0);
```

---

## 10. `std:time` (Time, Timers & Date)

Import:
```tx
import std:time;
// or
import { now, elapsed, setTimeout, setInterval, clearTimeout, clearInterval, Date, Timeout, Interval } from "std:time";
```

### Functions

| Function | Signature | Description |
|---|---|---|
| `now` | `now(): int64` | Current Unix timestamp in milliseconds. |
| `elapsed` | `elapsed(start: int64): int64` | Returns milliseconds elapsed since `start`. |
| `setTimeout` | `setTimeout(cb: () => void, ms: int64): int` | Schedules callback once after `ms`. Returns handle ID. |
| `setInterval` | `setInterval(cb: () => void, ms: int64): int`| Schedules callback periodically every `ms`. Returns handle ID. |
| `clearTimeout`| `clearTimeout(id: int): void` | Cancels a pending timeout by handle ID. |
| `clearInterval`| `clearInterval(id: int): void` | Cancels an active interval by handle ID. |
| `afterMs` | `afterMs(cb: () => void, ms: int64): Timeout`| Object-oriented timeout timer. |
| `everyMs` | `everyMs(cb: () => void, ms: int64): Interval`| Object-oriented interval timer. |

### Classes

#### `class Date`
```tx
class Date {
    constructor();                            // Initialized to now()
    static now(): int64;
    static fromTimestamp(ts: int64): Date;
    getTime(): int64;
    valueOf(): int64;
    toISOString(): string;                   // E.g. "2026-10-03T00:30:00Z"
    toString(): string;
    isBefore(other: Date): bool;
    isAfter(other: Date): bool;
}
```

#### `class Timeout` & `class Interval`
```tx
let timer = afterMs(() => { print("Fired"); }, 1000);
if (timer.isActive()) {
    timer.cancel();
}
```

---

## 11. `std:system`, `os`, `process` (Operating System & Process Control)

Import:
```tx
import std:system;
// Available under: system.*, os.*, and process.*
```

### Process Lifecycle & Information

| Function | Signature | Description |
|---|---|---|
| `exit` | `exit(code: int): void` | Terminates process with exit status code. |
| `pid` | `pid(): int` | Returns current Process ID. |
| `uptime` | `uptime(): float` | Returns process uptime in seconds (floating-point). |
| `args` | `args(): string[]` | Returns command-line arguments array. |
| `getCwd` | `getCwd(): Optional<string>` | Returns current working directory. |
| `getHomeDir`| `getHomeDir(): Optional<string>`| Returns user's home directory. |
| `getTempDir`| `getTempDir(): string` | Returns system temporary directory (e.g. `/tmp`). |
| `getHostname`| `getHostname(): Optional<string>`| Returns system machine hostname. |

### Environment Variables
- `getEnv(key: string): Optional<string>`: Reads environment variable.
- `getAllEnv(): Map<string, string>`: Returns all environment variables as a `Map`.

### Hardware & Architecture
- `getOSType(): string`: OS platform (`"macos"`, `"linux"`, `"windows"`).
- `getOSArch(): string`: CPU architecture (`"arm64"`, `"x86_64"`).
- `cpuCount(): int`: Number of online CPU logical cores.
- `totalMemory(): int64`: Total physical RAM in bytes.
- `freeMemory(): int64`: Available physical RAM in bytes.

### Command Execution & Child Processes

| Function | Signature | Description |
|---|---|---|
| `exec` | `exec(cmd: string): string` | Runs shell command synchronously; returns stdout (trimmed). |
| `execFull` | `execFull(cmd: string): any[]` | Runs command synchronously; returns `[exitCode: int, stdout: string, stderr: string]`. |
| `spawn` | `spawn(program: string, args: string[]): int` | Spawns background child process. Returns child PID or `-1`. |
| `kill` | `kill(pid: int, force: bool = false): bool` | Sends `SIGTERM` (`force=false`) or `SIGKILL` (`force=true`) to process. |
| `waitFor` | `waitFor(pid: int): int` | Blocks until child process exits; returns exit code. |

---

## 12. `std:crypto` (Cryptography & Hashing)

Import:
```tx
import std:crypto;
// or
import { sha256, hmacSha256, pbkdf2Sha256, randomBytes } from "std:crypto";
```

All functions accept and return raw byte arrays (`int[]` with values `0..255`):

| Function | Signature | Description |
|---|---|---|
| `sha256` | `sha256(data: int[]): int[]` | Computes 32-byte SHA-256 cryptographic digest. |
| `hmacSha256` | `hmacSha256(key: int[], data: int[]): int[]` | Computes HMAC-SHA256 message authentication code. |
| `pbkdf2Sha256` | `pbkdf2Sha256(password: int[], salt: int[], iterations: int, dkLen: int = 32): int[]` | Derives key using PBKDF2 with SHA-256 HMAC. |
| `randomBytes` | `randomBytes(len: int): int[]` | Cryptographically secure random byte generator (OS CSPRNG). |

```tx
import std:crypto;
import std:binary;

let bytes = binary.fromString("secret message");
let hash = crypto.sha256(bytes);
print("Digest length: " + hash.length()); // 32
```

---

## 13. `std:binary` (Binary Encoding, ByteReader & ByteWriter)

Import:
```tx
import std:binary;
// or
import { ByteReader, ByteWriter, fromString, toString, concat } from "std:binary";
```

### Utility Functions
- `fromString(data: string): int[]`: Encodes UTF-8 string to byte array.
- `toString(data: int[]): string`: Decodes byte array into UTF-8 string.
- `concat(parts: int[][]): int[]`: Concatenates multiple byte buffers into one.

### `class ByteReader`
High-speed binary stream reader supporting little-endian (LE) and big-endian (BE) operations.

```tx
class ByteReader {
    constructor(data: int[]);
    position(): int;
    remaining(): int;
    seek(position: int): void;
    readU8(): int;
    readBytes(count: int): int[];
    readString(count: int): string;
    readCString(): string;                   // Reads null-terminated string
    readU16LE(): int;
    readU16BE(): int;
    readU24LE(): int;
    readU32LE(): int64;
    readU32BE(): int64;
    readI32LE(): int;
}
```

### `class ByteWriter`
Dynamic binary buffer builder.

```tx
class ByteWriter {
    constructor();
    length(): int;
    writeU8(value: int64): void;
    writeBytes(data: int[]): void;
    writeString(data: string): void;
    writeCString(data: string): void;        // Appends null byte
    writeU16LE(value: int64): void;
    writeU16BE(value: int64): void;
    writeU24LE(value: int64): void;
    writeU32LE(value: int64): void;
    writeU32BE(value: int64): void;
    writeI32LE(value: int): void;
    toBytes(): int[];
}
```

---

## 14. `std:thread` (Multi-Thread Synchronization Primitives)

Import:
```tx
import std:thread;
// or
import { Atomic, Mutex, Condition, SharedQueue, spawn, joinAll, parallel } from "std:thread";
```

### `class Atomic`
Lock-free atomic 64-bit integer.

```tx
class Atomic {
    constructor(initialValue: int);
    add(val: int): int;                      // Returns old value
    sub(val: int): int;                      // Returns old value
    increment(): int;
    decrement(): int;
    load(): int;
    store(val: int): void;
    exchange(val: int): int;                 // Swaps and returns old value
    compareExchange(expected: int, desired: int): int;
}
```

### `class Mutex`
Reentrant cross-thread mutual exclusion lock.

```tx
class Mutex {
    constructor();
    lock(): void;
    unlock(): void;
}
```

### `class Condition`
Condition variable paired with a `Mutex`.

```tx
class Condition {
    constructor();
    wait(mutex: Mutex): void;
    notify(): void;
    notifyAll(): void;
}
```

### `class SharedQueue<T>`
Thread-safe concurrent FIFO queue.

```tx
class SharedQueue<T> {
    constructor();
    enqueue(val: T): void;
    dequeue(): T;
    drain(): T[];
    size(): int;
    isEmpty(): bool;
}
```

### Thread Functions
- `spawn(task: () => void): Thread`: Spawns a virtual thread.
- `joinAll(threads: Thread[]): void`: Blocks until every thread in `threads` exits.
- `sleep(ms: int): void`: Suspends execution of current thread for `ms` milliseconds.

---

## 15. `std:gc` (Garbage Collector Introspection & Tuning)

Import:
```tx
import std:gc;
// or
import { collect, collectYoung, getStats, getHeapUsed, getHeapTotal, enable, disable, isEnabled, setGrowthFactor, getGrowthFactor, GcStats } from "std:gc";
```

### `class GcStats`
- `heapUsed: int64`: Total bytes used across heap.
- `heapTotal: int64`: Virtual memory reserved for heap.
- `oldGenUsed: int64`: Bytes occupied in Old Generation.
- `losUsed: int64`: Bytes occupied in Large Object Space.
- `threshold: int64`: Byte threshold before triggering next major collection.

### Functions

| Function | Signature | Description |
|---|---|---|
| `collect` | `collect(): void` | Triggers a full concurrent Mark-Sweep major GC collection. |
| `collectYoung`| `collectYoung(): void` | Triggers a fast generational minor GC collection. |
| `getStats` | `getStats(): GcStats` | Returns complete memory snapshot. |
| `getHeapUsed` | `getHeapUsed(): int64` | Returns currently allocated heap bytes. |
| `getHeapTotal`| `getHeapTotal(): int64` | Returns total heap capacity bytes. |
| `enable` | `enable(): void` | Enables automatic garbage collection cycles. |
| `disable` | `disable(): void` | Disables automatic GC (useful for critical real-time blocks). |
| `isEnabled` | `isEnabled(): bool` | Returns `true` if automatic GC is currently active. |
| `setGrowthFactor`| `setGrowthFactor(pct: int): void`| Adjusts heap growth factor (e.g. `50` for 50% headroom). |
| `getGrowthFactor`| `getGrowthFactor(): float64` | Returns current growth multiplier (e.g. `1.5`). |

---

## 16. `std:runtime` (Engine & Virtual Thread Metrics)

Import:
```tx
import std:runtime;
// or
import { getVThreadStackSize, setVThreadStackSize, getVThreadCount, getRuntimeInfo, RuntimeInfo } from "std:runtime";
```

### `class RuntimeInfo`
- `vthreadStackSize: int`: Current initial stack size per virtual thread in bytes.
- `vthreadsSpawned: int64`: Cumulative virtual thread count spawned since startup.
- `cpuCount: int`: Number of active OS worker threads in scheduler pool.
- `uptime: float`: Engine uptime in seconds.

### Functions
- `getVThreadStackSize(): int`: Returns initial stack size allocated for virtual threads (default `2048` bytes).
- `setVThreadStackSize(sizeBytes: int): void`: Dynamically updates stack size for future spawned virtual threads.
- `getVThreadCount(): int64`: Returns total number of virtual threads created.
- `getRuntimeInfo(): RuntimeInfo`: Captures complete runtime performance telemetry.
