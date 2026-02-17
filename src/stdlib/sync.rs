use std::collections::HashSet;
use std::sync::{Arc, Mutex, Condvar};
use crate::runtime::{HEAP, TaggedValue};
use super::collections::{rt_Stack_push, rt_Queue_dequeue, rt_collections_isEmpty};

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("Mutex".to_string());
    s.insert("SharedQueue".to_string());
    s
}

// --- Mutex ---
#[unsafe(no_mangle)] pub extern "C" fn rt_Mutex_constructor(this: i64) -> i64 {
    let m = Arc::new((Mutex::new(false), Condvar::new()));
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Mutex(m));
    this
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Mutex_acquire(this: i64) -> i64 {
    let pair = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Mutex(pair)) = heap.get(this) {
            pair.clone()
        } else {
            return 0;
        }
    };

    let (lock, cvar) = &*pair;
    let mut started = lock.lock().unwrap();
    while *started {
        started = cvar.wait(started).unwrap();
    }
    *started = true;
    1
}

#[unsafe(no_mangle)] pub extern "C" fn rt_Mutex_release(this: i64) -> i64 {
    let pair = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Mutex(pair)) = heap.get(this) {
            pair.clone()
        } else {
            return 0;
        }
    };

    let (lock, cvar) = &*pair;
    let mut started = lock.lock().unwrap();
    *started = false;
    cvar.notify_one();
    1
}

// --- SharedQueue ---
// Behaves like a regular Queue (backed by Array) but intended for concurrent use
// (User is expected to lock externally as per producer_consumer.tx)

#[unsafe(no_mangle)] pub extern "C" fn rt_SharedQueue_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    heap.insert(this, TaggedValue::Array(Vec::new()));
    this
}

#[unsafe(no_mangle)] pub extern "C" fn rt_SharedQueue_enqueue(this: i64, val: i64) -> i64 {
    rt_Stack_push(this, val) 
}

#[unsafe(no_mangle)] pub extern "C" fn rt_SharedQueue_dequeue(this: i64) -> i64 {
    rt_Queue_dequeue(this)
}

#[unsafe(no_mangle)] pub extern "C" fn rt_SharedQueue_isEmpty(this: i64) -> i64 {
    rt_collections_isEmpty(this)
}

#[unsafe(no_mangle)] pub extern "C" fn rt_SharedQueue_size(this: i64) -> i64 {
    super::collections::rt_collections_size(this)
}
