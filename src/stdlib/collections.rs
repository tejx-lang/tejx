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
    s
}

// --- Stack ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Stack_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Array(Vec::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Stack_push(this: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) { 
        arr.push(val); 
    }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Stack_pop(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) { 
        return arr.pop().unwrap_or(0); 
    }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Stack_peek(this: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(this) { return arr.last().cloned().unwrap_or(0); }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Stack_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_Stack_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- Queue ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Queue_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Array(Vec::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Queue_enqueue(this: i64, val: i64) -> i64 {
    rt_Stack_push(this, val)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Queue_dequeue(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(this) {
        if !arr.is_empty() { return arr.remove(0); }
    }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Queue_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_Queue_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- MinHeap / PriorityQueue ---
#[unsafe(no_mangle)] pub extern "C" fn rt_MinHeap_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Array(Vec::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_PriorityQueue_constructor(this: i64) -> i64 {
    rt_MinHeap_constructor(this)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_MinHeap_insert(this: i64, val: i64) -> i64 {
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
#[unsafe(no_mangle)] pub extern "C" fn rt_PriorityQueue_insert(this: i64, val: i64) -> i64 {
    rt_MinHeap_insert(this, val)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_MinHeap_extractMin(this: i64) -> i64 {
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
#[unsafe(no_mangle)] pub extern "C" fn rt_PriorityQueue_extractMin(this: i64) -> i64 {
    rt_MinHeap_extractMin(this)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_MinHeap_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_MinHeap_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_PriorityQueue_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_PriorityQueue_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- MaxHeap ---
#[unsafe(no_mangle)] pub extern "C" fn rt_MaxHeap_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Array(Vec::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_MaxHeap_insertMax(this: i64, val: i64) -> i64 {
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
#[unsafe(no_mangle)] pub extern "C" fn rt_MaxHeap_extractMax(this: i64) -> i64 {
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
#[unsafe(no_mangle)] pub extern "C" fn rt_MaxHeap_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_MaxHeap_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- Map ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Map(HashMap::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_set(this: i64, key: i64, val: i64) -> i64 {
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get_mut(this) { map.insert(k_str, val); }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_put(this: i64, key: i64, val: i64) -> i64 {
    rt_Map_set(this, key, val)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_get(this: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get(this) { return map.get(&k_str).cloned().unwrap_or(0); }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_at(this: i64, key: i64) -> i64 {
    rt_Map_get(this, key)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_delete(this: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get_mut(this) { map.remove(&k_str); }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_remove(this: i64, key: i64) -> i64 {
    rt_Map_delete(this, key)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_has(this: i64, val: i64) -> i64 {
    let k_str = stringify_value(val);
    let result = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Map(map)) = heap.get(this) { map.contains_key(&k_str) }
        else { false }
    };
    rt_box_boolean(if result { 1 } else { 0 })
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_keys(this: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get(this) {
        let keys_str: Vec<String> = map.keys().cloned().collect();
        drop(heap); 
        let mut boxed_keys = Vec::new();
        for k in keys_str {
            let c_str = CString::new(k).unwrap();
            unsafe { boxed_keys.push(rt_box_string(c_str.as_ptr() as i64)); }
        }
        let mut heap = HEAP.lock().unwrap();
        // Use alloc loop or insert
        let new_id = heap.next_id; heap.next_id += 1;
        heap.insert(new_id, TaggedValue::Array(boxed_keys));
        return new_id;
    }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_values(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get(this) {
        let vals: Vec<i64> = map.values().cloned().collect();
        // Use alloc loop or insert
        let new_id = heap.next_id; heap.next_id += 1;
        heap.insert(new_id, TaggedValue::Array(vals));
        return new_id;
    }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_Map_clear(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get_mut(this) { map.clear(); }
    this
}

// --- Set ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Set(HashSet::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_add(this: i64, val: i64) -> i64 {
    let key = stringify_value(val);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(set)) = heap.get_mut(this) { set.insert(key); }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_has(this: i64, val: i64) -> i64 {
    let key = stringify_value(val);
    let result = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Set(set)) = heap.get(this) { set.contains(&key) }
        else { false }
    };
    rt_box_boolean(if result { 1 } else { 0 })
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_delete(this: i64, val: i64) -> i64 {
    let key = stringify_value(val);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(set)) = heap.get_mut(this) { set.remove(&key); }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_remove(this: i64, val: i64) -> i64 {
    rt_Set_delete(this, val)
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_values(this: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(set)) = heap.get(this) {
        let vals: Vec<String> = set.iter().cloned().collect();
        drop(heap);
        let mut boxed_vals = Vec::new();
        for v in vals {
            let c_str = CString::new(v).unwrap();
            unsafe { boxed_vals.push(rt_box_string(c_str.as_ptr() as i64)); }
        }
        let mut heap = HEAP.lock().unwrap();
        let new_id = heap.next_id; heap.next_id += 1;
        heap.insert(new_id, TaggedValue::Array(boxed_vals));
        return new_id;
    }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_Set_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- OrderedMap ---
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedMap_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::OrderedMap(Vec::new(), HashMap::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedMap_put(this: i64, key: i64, val: i64) -> i64 {
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::OrderedMap(keys, map)) = heap.get_mut(this) {
        if !map.contains_key(&k_str) { keys.push(k_str.clone()); }
        map.insert(k_str, val);
    }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedMap_at(this: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::OrderedMap(_, map)) = heap.get(this) { return map.get(&k_str).cloned().unwrap_or(0); }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedMap_has(this: i64, val: i64) -> i64 {
    let k_str = stringify_value(val);
    let result = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::OrderedMap(_, map)) = heap.get(this) { map.contains_key(&k_str) }
        else { false }
    };
    rt_box_boolean(if result { 1 } else { 0 })
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedMap_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedMap_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- OrderedSet ---
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedSet_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::OrderedSet(Vec::new(), HashSet::new()));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedSet_add(this: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::OrderedSet(elements, set)) = heap.get_mut(this) {
        if !set.contains(&val) { elements.push(val); set.insert(val); }
    }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedSet_has(this: i64, val: i64) -> i64 {
    let result = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::OrderedSet(_, set)) = heap.get(this) { set.contains(&val) }
        else { false }
    };
    rt_box_boolean(if result { 1 } else { 0 })
}
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedSet_size(this: i64) -> i64 { rt_collections_size(this) }
#[unsafe(no_mangle)] pub extern "C" fn rt_OrderedSet_isEmpty(this: i64) -> i64 { rt_collections_isEmpty(this) }

// --- Generic Collection Methods ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_keys(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let keys = if let Some(TaggedValue::Map(m)) = heap.get(id) {
        m.keys().filter(|k| *k != "toString" && *k != "constructor").cloned().collect::<Vec<String>>()
    } else if let Some(TaggedValue::OrderedMap(order, _)) = heap.get(id) {
        order.iter().filter(|k| *k != "toString" && *k != "constructor").cloned().collect::<Vec<String>>()
    } else {
        Vec::new()
    };
    drop(heap);
    
    let mut boxed_keys = Vec::new();
    for k in keys {
        let c_str = CString::new(k).unwrap();
        unsafe { boxed_keys.push(rt_box_string(c_str.as_ptr() as i64)); }
    }
    
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id; heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(boxed_keys));
    arr_id
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_values(id: i64) -> i64 {
    // Handle Set separately since it stores strings now
    {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Set(s)) = heap.get(id) {
            let str_vals: Vec<String> = s.iter().cloned().collect();
            drop(heap);
            let mut boxed_vals = Vec::new();
            for v in str_vals {
                let c_str = CString::new(v).unwrap();
                unsafe { boxed_vals.push(rt_box_string(c_str.as_ptr() as i64)); }
            }
            let mut heap = HEAP.lock().unwrap();
            let arr_id = heap.next_id; heap.next_id += 1;
            heap.insert(arr_id, TaggedValue::Array(boxed_vals));
            return arr_id;
        }
    }

    let heap = HEAP.lock().unwrap();
    let values = if let Some(TaggedValue::Map(m)) = heap.get(id) {
        m.iter().filter(|(k, _)| *k != "toString" && *k != "constructor")
            .map(|(_, v)| *v).collect::<Vec<i64>>()
    } else if let Some(TaggedValue::OrderedMap(order, m)) = heap.get(id) {
        order.iter().filter(|k| *k != "toString" && *k != "constructor")
            .map(|k| m.get(k).cloned().unwrap_or(0)).collect::<Vec<i64>>()
    } else {
        Vec::new()
    };
    drop(heap);
    
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id; heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(values));
    arr_id
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_entries(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let entries = if let Some(TaggedValue::Map(m)) = heap.get(id) {
        m.iter().filter(|(k, _)| *k != "toString" && *k != "constructor")
            .map(|(k, v)| (k.clone(), *v)).collect::<Vec<(String, i64)>>()
    } else if let Some(TaggedValue::OrderedMap(order, m)) = heap.get(id) {
        order.iter().filter(|k| *k != "toString" && *k != "constructor")
            .map(|k| (k.clone(), m.get(k).cloned().unwrap_or(0))).collect::<Vec<(String, i64)>>()
    } else {
        Vec::new()
    };
    drop(heap);
    
    // Entries returns Array of [key, value] arrays? Or Array of objects?
    // Usually Array of [key, value].
    // Original runtime creates Array of 2-element Arrays?
    // Let's create proper structure.
    // .. original runtime used loop to create inner arrays.
    
    let heap = HEAP.lock().unwrap();
    // Logic:
    // Need to alloc string key and array, so we must drop lock before boxing.

    // Re-implementation simplified:
    drop(heap); // Release lock
    
    let mut final_entries = Vec::new();
    for (k, v) in entries {
        let c_str = CString::new(k).unwrap();
        let k_id = unsafe { rt_box_string(c_str.as_ptr() as i64) };
        
        let mut heap = HEAP.lock().unwrap();
        let inner_arr_id = heap.next_id; heap.next_id += 1;
        heap.insert(inner_arr_id, TaggedValue::Array(vec![k_id, v]));
        drop(heap);
        final_entries.push(inner_arr_id);
    }
    
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id; heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(final_entries));
    arr_id
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_has(id: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get(id) {
        return if m.contains_key(&k_str) { 1 } else { 0 };
    }
    if let Some(TaggedValue::Set(s)) = heap.get(id) {
        return if s.contains(&k_str) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_delete(id: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) {
        return if m.remove(&k_str).is_some() { 1 } else { 0 };
    }
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) {
        return if s.remove(&k_str) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_clear(id: i64) {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) { m.clear(); }
    else if let Some(TaggedValue::Set(s)) = heap.get_mut(id) { s.clear(); }
    else if let Some(TaggedValue::Array(a)) = heap.get_mut(id) { a.clear(); }
    else if let Some(TaggedValue::OrderedMap(_, m)) = heap.get_mut(id) { m.clear(); }
    else if let Some(TaggedValue::OrderedSet(_, s)) = heap.get_mut(id) { s.clear(); }
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Collection_add(id: i64, val: i64) {
    let key = stringify_value(val);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) { s.insert(key); }
}

// --- Prefix-less aliases for runtime method dispatch ---
// The lowering can emit callee names without the `rt_` prefix (e.g. `Collection_keys`),
// but the runtime functions are prefixed with `rt_`. These wrappers bridge the gap.
#[unsafe(no_mangle)] pub extern "C" fn Collection_keys(id: i64) -> i64 { rt_Collection_keys(id) }
#[unsafe(no_mangle)] pub extern "C" fn Collection_values(id: i64) -> i64 { rt_Collection_values(id) }
#[unsafe(no_mangle)] pub extern "C" fn Collection_entries(id: i64) -> i64 { rt_Collection_entries(id) }
#[unsafe(no_mangle)] pub extern "C" fn Collection_has(id: i64, key: i64) -> i64 { rt_Collection_has(id, key) }
#[unsafe(no_mangle)] pub extern "C" fn Collection_delete(id: i64, key: i64) -> i64 { rt_Collection_delete(id, key) }
#[unsafe(no_mangle)] pub extern "C" fn Collection_clear(id: i64) { rt_Collection_clear(id) }
#[unsafe(no_mangle)] pub extern "C" fn Collection_add(id: i64, val: i64) { rt_Collection_add(id, val) }

#[unsafe(no_mangle)] pub extern "C" fn rt_BloomFilter_constructor(this: i64, size_bits: i64, k_hashes: i64) -> i64 {
    let bits = to_f64(size_bits) as usize;
    let k = to_f64(k_hashes) as usize;
    let byte_size = (bits + 7) / 8;
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::BloomFilter(vec![0; byte_size], k));
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_BloomFilter_add(this: i64, val: i64) -> i64 {
    let val_str = stringify_value(val);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::BloomFilter(bits, k)) = heap.get_mut(this) {
        let n_bits = bits.len() * 8;
        let k_val = *k;
        for i in 0..k_val {
            let h = djb2_hash(&val_str, i) % n_bits;
            bits[h / 8] |= 1 << (h % 8);
        }
    }
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_BloomFilter_contains(this: i64, val: i64) -> i64 {
    let s = stringify_value(val);
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
    };
    rt_box_boolean(if result { 1 } else { 0 })
}

// --- Trie ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Trie_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::TrieNode { children: HashMap::new(), is_end: false, value: 0 });
    this
}
#[unsafe(no_mangle)] pub extern "C" fn rt_Trie_addPath(this: i64, path: i64, val: i64) -> i64 {
    let s = stringify_value(path);
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
#[unsafe(no_mangle)] pub extern "C" fn rt_Trie_find(this: i64, path: i64) -> i64 {
    let s = stringify_value(path);
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

// --- Internal Helpers & Common ---
fn djb2_hash(s: &str, seed: usize) -> usize {
    let mut hash: usize = 5381 + seed;
    for c in s.chars() { hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as usize); }
    hash
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_collections_size(this: i64) -> i64 {
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
    };
    rt_box_number(sz)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_collections_isEmpty(this: i64) -> i64 {
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
    };
    rt_box_boolean(if empty { 1 } else { 0 })
}
