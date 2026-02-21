# Collections & Sizing

TejX provides built-in collection types via `import * from "std:collections"`. All collections live on a global heap as `TaggedValue` variants, referenced by `i64` IDs.

## Collection Types

### Array

```tejx
let arr: int[] = [1, 2, 3];
arr.push(4);
print(len(arr));        // 4
print(arr.length);      // 4 (property access)
```

- **Storage**: `Vec<i64>` — dynamic, grows automatically
- **Element size**: 8 bytes each (all values boxed as `i64`)
- **Sizing**: `len(arr)` or `arr.length`
- **Methods**: `push`, `pop`, `shift`, `unshift`, `slice`, `concat`, `indexOf`, `join`, `map`, `filter`, `reduce`, `find`, `findIndex`, `reverse`, `sort`, `forEach`, `splice`, `flat`, `includes`, `fill`

### Map

```tejx
import * from "std:collections";

let m = new Map<string, int>();
m.set("x", 10);
print(m.get("x"));     // 10
print(m.size());        // 1
m.clear();
print(m.size());        // 0
```

- **Storage**: `HashMap<String, i64>` — keys are always stringified
- **Sizing**: `.size()` or `.isEmpty()`
- **Methods**: `set`/`put`, `get`/`at`, `has`, `delete`/`remove`, `clear`, `keys`, `values`, `size`, `isEmpty`

### Set

```tejx
import * from "std:collections";

let s = new Set<int>();
s.add(1);
s.add(2);
s.add(1);               // duplicate ignored
print(s.size());         // 2
print(s.has(1));         // true
s.delete(1);
print(s.has(1));         // false
```

- **Storage**: `HashSet<String>` — values stringified before insertion
- **Sizing**: `.size()` or `.isEmpty()`
- **Methods**: `add`, `has`, `delete`/`remove`, `clear`, `values`, `size`, `isEmpty`

### Stack (LIFO)

```tejx
import * from "std:collections";

let st = new Stack<int>();
st.push(10);
st.push(20);
print(st.peek());       // 20
print(st.pop());        // 20
print(st.size());       // 1
```

- **Storage**: `Vec<i64>` (same as Array internally)
- **Sizing**: `.size()` or `.isEmpty()`
- **Methods**: `push`, `pop`, `peek`, `size`, `isEmpty`

### Queue (FIFO)

```tejx
import * from "std:collections";

let q = new Queue<int>();
q.enqueue(100);
q.enqueue(200);
print(q.dequeue());     // 100
print(q.size());        // 1
```

- **Storage**: `Vec<i64>` (same as Array internally)
- **Sizing**: `.size()` or `.isEmpty()`
- **Methods**: `enqueue`, `dequeue`, `size`, `isEmpty`
- **Note**: `dequeue` is O(n) — elements shift after removal

### PriorityQueue / MinHeap

```tejx
import * from "std:collections";

let pq = new PriorityQueue<int>();
pq.insert(30);
pq.insert(10);
pq.insert(20);
print(pq.extractMin()); // 10
```

- **Storage**: `Vec<i64>` — maintains min-heap ordering
- **Sizing**: `.size()` or `.isEmpty()`
- **Methods**: `insert`, `extractMin`, `size`, `isEmpty`

### MaxHeap

```tejx
import * from "std:collections";

let h = new MaxHeap<int>();
h.insertMax(10);
h.insertMax(50);
h.insertMax(100);
print(h.extractMax());  // 100
```

- **Storage**: `Vec<i64>` — maintains max-heap ordering
- **Methods**: `insertMax`, `extractMax`, `size`, `isEmpty`

## Sizing API Reference

| API          | Works On                                              | Returns | Notes           |
| ------------ | ----------------------------------------------------- | ------- | --------------- |
| `len(x)`     | Array, String                                         | `int`   | Global function |
| `.length`    | Array, String                                         | `int`   | Property access |
| `.size()`    | Map, Set, Stack, Queue, Heaps, OrderedMap, OrderedSet | `int`   | Method call     |
| `.isEmpty()` | All collections                                       | `bool`  | Method call     |

## Internal Memory Implementation

TejX uses a global, centralized heap to manage all collection lifecycles. This ensures memory safety and consistency across modules.

### Global Heap Architecture

The heap is a flat `Vec<Option<TaggedValue>>` protected by a `Mutex`.

- **Heap Offset**: All object IDs start at `200,000,000` (`HEAP_OFFSET`).
- **Addressing**: To get the internal vector index of an object, use `index = id - 200,000,000`.
- **Storage**: Objects are stored as `TaggedValue` variants, which wrap native Rust collections (e.g., `Vec`, `HashMap`).

### Value Representation (The `i64` Box)

TejX represents all values—whether numbers, booleans, or heap references—as a 64-bit integer (`i64`). This allows collections to be homogeneous in memory while remaining heterogeneous in content.

| Value Type     | Bit Pattern / Range              | Notes                                       |
| :------------- | :------------------------------- | :------------------------------------------ |
| **Small Ints** | `0` to `199,999,999`             | Literal integers stored directly.           |
| **Heap IDs**   | `200,000,000` to `2,000,000,000` | References to objects in the global heap.   |
| **Doubles**    | Mixed / Bitcasted `f64`          | Standard IEEE-754 bit patterns.             |
| **Pointers**   | `> 0x100000000`                  | Fallback for C-style string literals / FFI. |

### Collection-Specific Layouts

| Type      | Internal Rust Storage  | Memory Footprint                                       |
| :-------- | :--------------------- | :----------------------------------------------------- |
| **Array** | `Vec<i64>`             | `8 bytes * capacity` + Vector overhead.                |
| **Map**   | `HashMap<String, i64>` | Dynamic bucket-based storage. Keys are always strings. |
| **Set**   | `HashSet<String>`      | Keys are stringified before insertion.                 |
| **Trie**  | `HashMap<char, i64>`   | Recursive node structure per character.                |

### Memory Management & ARC

TejX uses a **Strict Ownership** model with Automatic Reference Counting (ARC).

> [!WARNING]
> **Current Status**: While the compiler generates `rt_inc_ref` and `rt_free` calls, the runtime `rt_free` is currently a **no-op**. This is an intentional stability choice to prevent use-after-free corruption while the borrow checker's ownership analysis is being perfected. Memory currently "leaks" but the execution remains stable.

### Memory Layout Visualization

```
Variable         Global Heap Slot (TaggedValue)
────────         ────────────────────────────────────
arr (200000001) → [ 1, 10.5 (bitcasted), 200000005 ]  (Array holding int, float, and Map ID)
m   (200000005) → { "key": 42 }                        (Map holding string key and int)
```
