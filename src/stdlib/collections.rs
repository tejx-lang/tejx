use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use crate::runtime::{HEAP, TaggedValue, rt_box_number, rt_box_boolean, rt_box_string, stringify_value};

/// Convert an i64 value to f64 WITHOUT locking the HEAP.
/// This is safe for numeric values (small ints and bitcasted doubles)
/// which is the expected use case for heap/priority queue elements.
fn to_f64(v: i64) -> f64 {
    if v > -1_000_000 && v < 1_000_000 {
        return v as f64;
    }
    f64::from_bits(v as u64)
}

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("Stack".to_string());
    s.insert("Queue".to_string());
    s.insert("PriorityQueue".to_string());
    s.insert("MinHeap".to_string());
    s.insert("MaxHeap".to_string());
    s.insert("Map".to_string());
    s.insert("Set".to_string());
    s.insert("OrderedMap".to_string());
    s.insert("OrderedSet".to_string());
    s.insert("BloomFilter".to_string());
    s.insert("Trie".to_string());
    
    let methods = [
        "push", "pop", "peek", "enqueue", "dequeue", "insert", "extractMin", "insertMax", "extractMax",
        "isEmpty", "size", "put", "at", "has", "delete", "add", "clear", "contains",
        "find", "addPath"
    ];
    for m in methods { s.insert(m.to_string()); }
    s
}

// --- Stack & Queue (Array-based) ---

#[unsafe(no_mangle)] pub extern "C" fn std_collections_Stack() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Array(Vec::new()))
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_push(this: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) { arr.push(val); }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_pop(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) { return arr.pop().unwrap_or(0); }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_peek(this: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(this) { return arr.last().cloned().unwrap_or(0); }
    0
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_Queue() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Array(Vec::new()))
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_enqueue(this: i64, val: i64) -> i64 {
    std_collections_push(this, val)
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_dequeue(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) {
        if !arr.is_empty() { return arr.remove(0); }
    }
    0
}

// --- Heaps ---

#[unsafe(no_mangle)] pub extern "C" fn std_collections_MinHeap() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Array(Vec::new()))
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_MaxHeap() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Array(Vec::new()))
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_insert(this: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) {
        arr.push(val);
        let mut idx = arr.len() - 1;
        while idx > 0 {
            let p = (idx - 1) / 2;
            if to_f64(arr[idx]) < to_f64(arr[p]) {
                arr.swap(idx, p);
                idx = p;
            } else { break; }
        }
    }
    this
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_extractMin(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) {
        if arr.is_empty() { return 0; }
        let res = arr[0];
        let last = arr.pop().unwrap();
        if !arr.is_empty() {
            arr[0] = last;
            let mut i = 0;
            while 2 * i + 1 < arr.len() {
                let mut s = 2 * i + 1;
                if s + 1 < arr.len() && to_f64(arr[s+1]) < to_f64(arr[s]) { s += 1; }
                if to_f64(arr[s]) < to_f64(arr[i]) {
                    arr.swap(i, s);
                    i = s;
                } else { break; }
            }
        }
        return res;
    }
    0
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_insertMax(this: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) {
        arr.push(val);
        let mut idx = arr.len() - 1;
        while idx > 0 {
            let p = (idx - 1) / 2;
            if to_f64(arr[idx]) > to_f64(arr[p]) {
                arr.swap(idx, p);
                idx = p;
            } else { break; }
        }
    }
    this
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_extractMax(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) {
        if arr.is_empty() { return 0; }
        let res = arr[0];
        let last = arr.pop().unwrap();
        if !arr.is_empty() {
            arr[0] = last;
            let mut i = 0;
            while 2 * i + 1 < arr.len() {
                let mut s = 2 * i + 1;
                if s + 1 < arr.len() && to_f64(arr[s+1]) > to_f64(arr[s]) { s += 1; }
                if to_f64(arr[s]) > to_f64(arr[i]) {
                    arr.swap(i, s);
                    i = s;
                } else { break; }
            }
        }
        return res;
    }
    0
}

// --- PriorityQueue (Alias for MinHeap) ---
#[unsafe(no_mangle)] pub extern "C" fn std_collections_PriorityQueue() -> i64 {
    std_collections_MinHeap()
}

// --- Map & Set ---

#[unsafe(no_mangle)] pub extern "C" fn std_collections_Map() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Map(HashMap::new()))
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_Set() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Set(HashSet::new()))
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_put(this: i64, key: i64, val: i64) -> i64 {
    let k_str = stringify_value(key); // locks and releases HEAP
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get_mut(this) { map.insert(k_str, val); }
    else if let Some(TaggedValue::OrderedMap(keys, map)) = heap.get_mut(this) {
        if !map.contains_key(&k_str) { keys.push(k_str.clone()); }
        map.insert(k_str, val);
    }
    this
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_at(this: i64, key: i64) -> i64 {
    let k_str = stringify_value(key); // locks and releases HEAP
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get(this) { return map.get(&k_str).cloned().unwrap_or(0); }
    if let Some(TaggedValue::OrderedMap(_, map)) = heap.get(this) { return map.get(&k_str).cloned().unwrap_or(0); }
    0
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_add(this: i64, val: i64) -> i64 {
    // For BloomFilter, we need stringify_value which locks HEAP, so do it first
    let val_str = stringify_value(val);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(set)) = heap.get_mut(this) { 
        set.insert(val); 
    }
    else if let Some(TaggedValue::OrderedSet(elements, set)) = heap.get_mut(this) {
        if !set.contains(&val) { elements.push(val); set.insert(val); }
    }
    else if let Some(TaggedValue::BloomFilter(bits, k)) = heap.get_mut(this) {
        let n_bits = bits.len() * 8;
        let k_val = *k;
        for i in 0..k_val {
            let h = djb2_hash(&val_str, i) % n_bits;
            bits[h / 8] |= 1 << (h % 8);
        }
    }
    this
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_has(this: i64, val: i64) -> i64 {
    let k_str = stringify_value(val); // locks and releases HEAP
    let result = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Map(map)) = heap.get(this) { Some(map.contains_key(&k_str)) }
        else if let Some(TaggedValue::Set(set)) = heap.get(this) { Some(set.contains(&val)) }
        else if let Some(TaggedValue::OrderedMap(_, map)) = heap.get(this) { Some(map.contains_key(&k_str)) }
        else if let Some(TaggedValue::OrderedSet(_, set)) = heap.get(this) { Some(set.contains(&val)) }
        else { Some(false) }
    }; // HEAP lock dropped here
    rt_box_boolean(if result.unwrap_or(false) { 1 } else { 0 })
}

// --- Ordered Collections ---

#[unsafe(no_mangle)] pub extern "C" fn std_collections_OrderedMap() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::OrderedMap(Vec::new(), HashMap::new()))
}
#[unsafe(no_mangle)] pub extern "C" fn std_collections_OrderedSet() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::OrderedSet(Vec::new(), HashSet::new()))
}

// --- Bloom Filter ---

#[unsafe(no_mangle)] pub extern "C" fn std_collections_BloomFilter(size_bits: i64, k_hashes: i64) -> i64 {
    let bits = to_f64(size_bits) as usize;
    let k = to_f64(k_hashes) as usize;
    let byte_size = (bits + 7) / 8;
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::BloomFilter(vec![0; byte_size], k))
}

fn djb2_hash(s: &str, seed: usize) -> usize {
    let mut hash: usize = 5381 + seed;
    for c in s.chars() { hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as usize); }
    hash
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_contains(this: i64, val: i64) -> i64 {
    let s = stringify_value(val); // locks and releases HEAP
    let result = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::BloomFilter(bits, k)) = heap.get(this) {
            let n_bits = bits.len() * 8;
            let mut found = true;
            for i in 0..*k {
                let h = djb2_hash(&s, i) % n_bits;
                if (bits[h / 8] & (1 << (h % 8))) == 0 { found = false; break; }
            }
            found
        } else { false }
    }; // HEAP lock dropped here
    rt_box_boolean(if result { 1 } else { 0 })
}

// --- Trie ---

#[unsafe(no_mangle)] pub extern "C" fn std_collections_Trie() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::TrieNode { children: HashMap::new(), is_end: false, value: 0 })
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_addPath(this: i64, path: i64, val: i64) -> i64 {
    let s = stringify_value(path); // locks and releases HEAP
    let mut curr = this;
    for c in s.chars() {
        let mut next = 0;
        {
            let heap = HEAP.lock().unwrap();
            if let Some(TaggedValue::TrieNode { children, .. }) = heap.get(curr) {
                if let Some(&child) = children.get(&c) { next = child; }
            }
        }
        if next == 0 {
            let mut heap = HEAP.lock().unwrap();
            next = heap.alloc(TaggedValue::TrieNode { children: HashMap::new(), is_end: false, value: 0 });
            if let Some(TaggedValue::TrieNode { children, .. }) = heap.get_mut(curr) { children.insert(c, next); }
        }
        curr = next;
    }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::TrieNode { is_end, value, .. }) = heap.get_mut(curr) {
        *is_end = true;
        *value = val;
    }
    this
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_find(this: i64, path: i64) -> i64 {
    let s = stringify_value(path); // locks and releases HEAP
    let mut curr = this;
    for c in s.chars() {
        let mut next = 0;
        {
            let heap = HEAP.lock().unwrap();
            if let Some(TaggedValue::TrieNode { children, .. }) = heap.get(curr) {
                if let Some(&child) = children.get(&c) { next = child; }
            }
        }
        if next == 0 { return 0; }
        curr = next;
    }
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::TrieNode { is_end, value, .. }) = heap.get(curr) {
        if *is_end { return *value; }
    }
    0
}

// --- Common ---
#[unsafe(no_mangle)] pub extern "C" fn std_collections_size(this: i64) -> i64 {
    let sz = {
        let heap = HEAP.lock().unwrap();
        match heap.get(this) {
            Some(TaggedValue::Array(a)) => a.len() as f64,
            Some(TaggedValue::Map(m)) => m.len() as f64,
            Some(TaggedValue::Set(s)) => s.len() as f64,
            Some(TaggedValue::OrderedMap(_, m)) => m.len() as f64,
            Some(TaggedValue::OrderedSet(_, s)) => s.len() as f64,
            _ => 0.0,
        }
    }; // HEAP lock dropped here
    rt_box_number(sz)
}

#[unsafe(no_mangle)] pub extern "C" fn std_collections_isEmpty(this: i64) -> i64 {
    let empty = {
        let heap = HEAP.lock().unwrap();
        match heap.get(this) {
            Some(TaggedValue::Array(a)) => a.is_empty(),
            Some(TaggedValue::Map(m)) => m.is_empty(),
            Some(TaggedValue::Set(s)) => s.is_empty(),
            Some(TaggedValue::OrderedMap(_, m)) => m.is_empty(),
            Some(TaggedValue::OrderedSet(_, s)) => s.is_empty(),
            _ => true,
        }
    }; // HEAP lock dropped here
    rt_box_boolean(if empty { 1 } else { 0 })
}
