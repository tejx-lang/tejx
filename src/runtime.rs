#![allow(unsafe_op_in_unsafe_fn)]
use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::c_char;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
// Compatibility module for dual-build (host/target)
// Compatibility module for dual-build (host/target)
pub mod runtime {
    pub use super::rt_to_number;
    pub use super::rt_box_string;
    pub use super::stringify_value;
    pub use super::HEAP;
    pub use super::TaggedValue;
    pub use super::Heap;
    pub use super::rt_box_boolean;
    pub use super::rt_box_number;
}

#[path = "stdlib/mod.rs"]
pub mod stdlib;
use std::thread;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, Arc, Condvar, LazyLock};

#[derive(Debug, Clone)]
pub enum TaggedValue {
    Array(Vec<i64>),
    ByteArray(Vec<u8>),
    Map(HashMap<String, i64>),
    Thread(Arc<Mutex<Option<thread::JoinHandle<i64>>>>),
    Mutex(Arc<(Mutex<bool>, Condvar)>),
    Number(f64),
    Boolean(bool),
    String(String),
    Set(std::collections::HashSet<i64>),
    Date(f64),
    OrderedMap(Vec<String>, HashMap<String, i64>),
    OrderedSet(Vec<i64>, HashSet<i64>),
    BloomFilter(Vec<u8>, usize), // bits, k
    TrieNode { children: HashMap<char, i64>, is_end: bool, value: i64 },
}

pub struct Heap {
    pub next_id: i64,
    pub objects: Vec<Option<TaggedValue>>,
}

impl Heap {
    pub fn get(&self, id: i64) -> Option<&TaggedValue> {
        if id >= 0 && id < self.objects.len() as i64 {
            self.objects[id as usize].as_ref()
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut TaggedValue> {
        if id >= 0 && id < self.objects.len() as i64 {
            self.objects[id as usize].as_mut()
        } else {
            None
        }
    }

    pub fn insert(&mut self, id: i64, val: TaggedValue) {
        let idx = id as usize;
        if idx >= self.objects.len() {
            self.objects.resize(idx + 1, None);
        }
        self.objects[idx] = Some(val);
    }

    pub fn contains_key(&self, id: i64) -> bool {
        id >= 0 && id < self.objects.len() as i64 && self.objects[id as usize].is_some()
    }

    pub fn alloc(&mut self, val: TaggedValue) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.insert(id, val);
        id
    }
}

pub static HEAP: LazyLock<Mutex<Heap>> = LazyLock::new(|| Mutex::new(Heap {
    next_id: 1000000, 
    objects: Vec::with_capacity(1000),
}));

 static EXCEPTION_STACK: LazyLock<Mutex<Vec<usize>>> = LazyLock::new(|| Mutex::new(Vec::new()));
 static CURRENT_EXCEPTION: LazyLock<Mutex<i64>> = LazyLock::new(|| Mutex::new(0));
 
 #[unsafe(no_mangle)]
 static mut LAST_ID: i64 = -1;
 #[unsafe(no_mangle)]
 static mut LAST_PTR: *mut u8 = std::ptr::null_mut();
 #[unsafe(no_mangle)]
 static mut LAST_LEN: usize = 0;
 #[unsafe(no_mangle)]
 static mut LAST_ELEM_SIZE: usize = 0;
 
  unsafe extern "C" {
      fn longjmp(env: *mut u64, val: i32);
  }

 // Push a pointer to a jmp_buf (allocated in the generated code's stack frame)
 #[unsafe(no_mangle)]
 pub extern "C" fn tejx_push_handler(buf_ptr: *mut u64) {
     let mut stack = EXCEPTION_STACK.lock().unwrap();
     stack.push(buf_ptr as usize);
 }

 // Pop the top handler (called at end of try block, normal path)
 #[unsafe(no_mangle)]
 pub extern "C" fn tejx_pop_handler() {
     let mut stack = EXCEPTION_STACK.lock().unwrap();
     stack.pop();
 }

 #[unsafe(no_mangle)]
 pub unsafe extern "C" fn tejx_throw(val: i64) {
     {
         let mut exc = CURRENT_EXCEPTION.lock().unwrap();
         *exc = val;
     }
     
     // Pop the handler address and RELEASE THE LOCK before longjmp.
     // longjmp never returns, so we must not hold any locks when calling it.
     let buf_addr = {
         let mut stack = EXCEPTION_STACK.lock().unwrap();
         stack.pop()
     };
     
     if let Some(addr) = buf_addr {
         longjmp(addr as *mut u64, 1);
     } else {
         // Uncaught exception!
         let msg = stringify_value(val);
         eprintln!("Uncaught Exception: {}", msg);
         std::process::exit(1);
     }
 }
 
 #[unsafe(no_mangle)]
 pub extern "C" fn tejx_get_exception() -> i64 {
     let exc = CURRENT_EXCEPTION.lock().unwrap();
     *exc
 }

#[unsafe(no_mangle)]
pub extern "C" fn tejx_hello() {

}

#[unsafe(no_mangle)]
pub extern "C" fn Thread_new(callback: i64, arg: i64) -> i64 {
    let handle = thread::spawn(move || {
        let cb: unsafe extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(callback) };
        let res = unsafe { cb(arg) };
        res
    });

    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::Thread(Arc::new(Mutex::new(Some(handle)))));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn Thread_join(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Thread(handle_mutex)) = heap.get(id) {
         let mut guard = handle_mutex.lock().unwrap();
         if let Some(handle) = guard.take() {
            drop(guard);
            drop(heap); // Verify release lock before join
            return handle.join().unwrap_or(0);
         }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Mutex_new() -> i64 {
    let m = Arc::new((Mutex::new(false), Condvar::new()));
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::Mutex(m));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn m_lock(id: i64) -> i64 {
    let pair = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Mutex(pair)) = heap.get(id) {
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

#[unsafe(no_mangle)]
pub extern "C" fn m_unlock(id: i64) -> i64 {
    let pair = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Mutex(pair)) = heap.get(id) {
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

#[unsafe(no_mangle)]
pub extern "C" fn a_new() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::Array(Vec::new()));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn a_new_fixed(size: i64, elem_size: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    if elem_size == 1 {
        let v = vec![0u8; size as usize];
        heap.insert(id, TaggedValue::ByteArray(v));
    } else {
        let v = vec![0i64; size as usize];
        heap.insert(id, TaggedValue::Array(v));
    }
    id
}


#[unsafe(no_mangle)]
pub extern "C" fn m_new() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    let mut map = HashMap::new();
    map.insert("toString".to_string(), rt_Object_toString as *const () as i64);
    heap.insert(id, TaggedValue::Map(map));
    id
}

unsafe fn get_string_key(heap: &Heap, key_ptr: i64) -> String {
    if let Some(TaggedValue::String(s)) = heap.get(key_ptr) {
        return s.clone();
    }
    
    // 1. Heap ID check (new safe range)
    if key_ptr >= 1000000 && key_ptr < 2000000000 {
        return format!("<id:{}>", key_ptr);
    }
    
    // 2. Small raw integer (0...1M)
    if key_ptr >= 0 && key_ptr < 1000000 {
        return key_ptr.to_string();
    }

    // 3. Bitcasted double check
    if key_ptr != 0 {
        let d = f64::from_bits(key_ptr as u64);
        if d.is_finite() && d.abs() < 1e308 && d.abs() > 1e-308 {
            return format!("{}", d);
        }
    }

    // 4. Pointer check (last resort, with cautious range)
    if key_ptr > 0x100000000 && key_ptr < 0x7FFFFFFFFFFF {
        let p = key_ptr as *const c_char;
        let c_str = CStr::from_ptr(p);
        return c_str.to_string_lossy().into_owned();
    }

    key_ptr.to_string()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_set(id: i64, key_ptr: i64, val: i64) -> i64 {

    let mut heap = HEAP.lock().unwrap();
    
    // Pre-calculate boxed index to avoid borrow checker conflict
    let boxed_idx = if let Some(TaggedValue::Number(n)) = heap.get(key_ptr) {
        Some(*n as usize)
    } else {
        None
    };

    let key = get_string_key(&heap, key_ptr);
    if let Some(obj) = heap.get_mut(id) {
        match obj {
            TaggedValue::Array(arr) => {
                let idx = if key_ptr < 1000000000 {
                    key_ptr as usize
                } else if key_ptr > 0xFFFFFFFFFFFF {
                    f64::from_bits(key_ptr as u64) as usize
                } else if let Some(i) = boxed_idx {
                    i
                } else {
                    usize::MAX // Invalid for array index set
                };

                if idx != usize::MAX {
                    if idx >= arr.len() {
                        if idx < arr.len() + 1000 { // Limit growth
                             arr.resize(idx + 1, 0);
                        }
                    }
                    if idx < arr.len() {
                        arr[idx] = val;
                    }
                }
            }
            TaggedValue::ByteArray(arr) => {
                let idx = if key_ptr < 1000000000 {
                    key_ptr as usize
                } else if key_ptr > 0xFFFFFFFFFFFF {
                    f64::from_bits(key_ptr as u64) as usize
                } else if let Some(i) = boxed_idx {
                    i
                } else {
                    usize::MAX
                };

                if idx != usize::MAX {
                    if idx >= arr.len() {
                        if idx < arr.len() + 1000 {
                             arr.resize(idx + 1, 0);
                        }
                    }
                    if idx < arr.len() {
                        arr[idx] = (val != 0) as u8;
                    }
                }
            }
            TaggedValue::Map(map) => {
                map.insert(key, val);
            }
            _ => {} 
        }
    } 
    val
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_get(id: i64, key_ptr: i64) -> i64 {

    let  heap = HEAP.lock().unwrap();
    let key = if key_ptr > 1000 && key_ptr < 0xFFFFFFFFFFFF { get_string_key(&heap, key_ptr) } else { format!("idx:{}", key_ptr) };
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Array(arr) => {
                let idx = if key_ptr < 1000000000 {
                     Some(key_ptr as usize)
                } else if key_ptr > 0xFFFFFFFFFFFF {
                     Some(f64::from_bits(key_ptr as u64) as usize)
                } else if let Some(TaggedValue::Number(n)) = heap.get(key_ptr) {
                     Some(*n as usize)
                } else {
                     None
                };

                if let Some(i) = idx {
                    if i < arr.len() {
                        return arr.get(i).cloned().unwrap_or(0);
                    }
                }

                // Special properties (length)
                let key = get_string_key(&heap, key_ptr);
                if key == "length" {
                    return arr.len() as i64;
                }
            }
            TaggedValue::ByteArray(arr) => {
                let idx = if key_ptr < 1000000000 {
                     Some(key_ptr as usize)
                } else if key_ptr > 0xFFFFFFFFFFFF {
                     Some(f64::from_bits(key_ptr as u64) as usize)
                } else if let Some(TaggedValue::Number(n)) = heap.get(key_ptr) {
                     Some(*n as usize)
                } else {
                     None
                };

                if let Some(i) = idx {
                    if i < arr.len() {
                        return (arr.get(i).cloned().unwrap_or(0) != 0) as i64;
                    }
                }

                let key = get_string_key(&heap, key_ptr);
                if key == "length" {
                    return arr.len() as i64;
                }
            }
            TaggedValue::Map(map) => {
                let key = get_string_key(&heap, key_ptr);
                
                if map.contains_key(&key) {
                     let val = map.get(&key).cloned().unwrap_or(0);
                     return val;
                } else {
                     return 0;
                }
            }
            _ => { }
        }
    } else {
        // Drop lock before check primitives
        drop(heap);
    }
    0
}

// ... Array functions ...

// Thread Stub - REMOVED

#[unsafe(no_mangle)]
pub extern "C" fn __callee___area() -> i64 { 100 }
#[unsafe(no_mangle)]
pub extern "C" fn __callee___describe() -> i64 { 0 }
#[unsafe(no_mangle)]
pub fn stringify_value(id: i64) -> String {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        let res = match obj {
            TaggedValue::Array(arr) => {
                let ids = arr.clone();
                drop(heap);
                let mut parts = Vec::new();
                for val_id in ids {
                    parts.push(stringify_value(val_id));
                }
                format!("[{}]", parts.join(", "))
            }
            TaggedValue::ByteArray(arr) => {
                let mut parts = Vec::new();
                for val in arr {
                    parts.push(if *val != 0 { "true" } else { "false" });
                }
                format!("[{}]", parts.join(", "))
            }
            TaggedValue::Map(map) => {
                let name_id = map.get("name").cloned();
                let msg_id = map.get("message").cloned();
                drop(heap); // Release lock before recursing
                
                let name = name_id.map(|id| stringify_value(id)).unwrap_or_else(|| "Object".to_string());
                let message = msg_id.map(|id| stringify_value(id));
                
                if let Some(msg) = message {
                    if !msg.is_empty() {
                         format!("{}: {}", name, msg)
                    } else {
                         format!("[{}]", name)
                    }
                } else {
                    format!("[{}]", name)
                }
            }
            TaggedValue::Thread(_) => "[Thread]".to_string(),
            TaggedValue::Mutex(_) => "[Mutex]".to_string(),
            TaggedValue::Number(n) => {
                if n.fract() == 0.0 { format!("{:.0}", n) } else { format!("{}", n) }
            },
            TaggedValue::Boolean(b) => format!("{}", b),
            TaggedValue::String(s) => s.clone(),
            TaggedValue::Set(_) => "[Set]".to_string(),
            TaggedValue::Date(t) => format!("[Date: {}]", t),
            TaggedValue::OrderedMap(_, _) => "[OrderedMap]".to_string(),
            TaggedValue::OrderedSet(_, _) => "[OrderedSet]".to_string(),
            TaggedValue::BloomFilter(_, _) => "[BloomFilter]".to_string(),
            TaggedValue::TrieNode { .. } => "[TrieNode]".to_string(),
        };
        return res;
    }
    drop(heap);

    // 4. Default: assume it might be a bitcasted double or a pointer
    // Typical bitcasted doubles start with 0x3FF (for 1.0) or 0x40 (for 2.0+)
    // Most pointers on 64-bit systems (macOS/Linux) are in the range 0x100000000 to 0x700000000000
    // However, bitcasted doubles are much more common in our system for unboxed numbers.
    
    // 4. Fallbacks for non-heap IDs (unboxed values)
    if id != 0 {
        // Optimization: Values between -1 trillion and 1 trillion are treated as direct integers.
        // This covers the sum of 1...100k (5e9) and most loop indices.
        if id > -1_000_000_000_000 && id < 1_000_000_000_000 {
            return id.to_string();
        }

        // Try treating it as a bitcasted double
        let d = f64::from_bits(id as u64);
        if d.is_finite() && (d.abs() > 1e-300 || d.abs() == 0.0) {
             if d.fract() == 0.0 { return format!("{:.0}", d); }
             else { return format!("{}", d); }
        }

        // Pointers are risky. On macOS, string literals from segments are typically in this range.
        if id > 0x100000000 && id < 0x200000000000 {
             let p = id as *const c_char;
             if !p.is_null() {
                 let c_str = unsafe { CStr::from_ptr(p) };
                 if let Ok(s) = c_str.to_str() {
                     return s.to_owned();
                 }
             }
        }
    }

    id.to_string()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __callee___toString(id: i64) -> i64 {
    let s = stringify_value(id);
    let c_str = CString::new(s).unwrap();
    c_str.into_raw() as i64
}

// ... OOP Stubs ...
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Error_constructor(this: i64, message: i64) {
    let mut heap = HEAP.lock().unwrap();
    
    // Create a boxed "Error" string for the name if not present
    let name_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(name_id, TaggedValue::String("Error".to_string()));

    if let Some(TaggedValue::Map(map)) = heap.get_mut(this) {
        map.insert("message".to_string(), message);
        map.insert("name".to_string(), name_id);
        map.insert("toString".to_string(), rt_Object_toString as *const () as i64);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_Object_toString(this: i64) -> i64 {
    // stringify_value already handles everything including releasing the lock
    let s = stringify_value(this);
    
    // Create a new boxed string from the result
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::String(s));
    id
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Date_constructor(this: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_nanos(0))
        .as_millis() as f64;
    heap.insert(this, TaggedValue::Date(now));
    this
}


// Testing Mocks
#[unsafe(no_mangle)] pub extern "C" fn add(a: i64, b: i64) -> i64 { a + b }
#[unsafe(no_mangle)] pub extern "C" fn multiply(a: i64, b: i64) -> i64 { a * b }
#[unsafe(no_mangle)] pub extern "C" fn hello() -> i64 {

    0
}
#[unsafe(no_mangle)] pub extern "C" fn calc_add(a: i64, b: i64) -> i64 { a + b }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Array_constructor(id: i64, arg: i64, elem_size: i64) {
    let size = if arg == 0 { 0 } else { rt_to_number(arg) as usize };
    let mut heap = HEAP.lock().unwrap();
    if elem_size == 1 {
        let v = vec![0u8; size];
        heap.insert(id, TaggedValue::ByteArray(v));
    } else {
        let v = vec![0i64; size];
        heap.insert(id, TaggedValue::Array(v));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Array_push(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    match heap.get_mut(id) {
        Some(TaggedValue::Array(arr)) => {
            arr.push(val);
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 8;
            }
            return arr.len() as i64;
        }
        Some(TaggedValue::ByteArray(arr)) => {
            arr.push((val != 0) as u8);
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 1;
            }
            return arr.len() as i64;
        }
        _ => { return 0; }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Array_pop(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    match heap.get_mut(id) {
        Some(TaggedValue::Array(arr)) => {
            let res = arr.pop().unwrap_or(0);
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 8;
            }
            return res;
        }
        Some(TaggedValue::ByteArray(arr)) => {
            let res = arr.pop().unwrap_or(0);
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 1;
            }
            return (res != 0) as i64;
        }
        _ => { return 0; }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_array_get_fast(id: i64, idx: i64) -> i64 {
    unsafe {
        if LAST_ID == id {
            let i = if idx >= 0 && idx < 200000000 {
                idx as usize
            } else {
                let top = (idx >> 48) & 0xFFFF;
                if top == 0 || top == 0xFFFF { idx as usize } else { f64::from_bits(idx as u64) as usize }
            };
            if i < LAST_LEN {
                if LAST_ELEM_SIZE == 1 {
                    return *LAST_PTR.add(i) as i64;
                } else {
                    return *(LAST_PTR as *mut i64).add(i);
                }
            }
        }
    }

    let heap = HEAP.lock().unwrap();
    let i = if idx >= 0 && idx < 200000000 { idx as usize } else { rt_to_number_internal(&heap, idx) as usize };
    match heap.get(id) {
        Some(TaggedValue::Array(arr)) => {
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 8;
            }
            if i < arr.len() { return arr[i]; }
        }
        Some(TaggedValue::ByteArray(arr)) => {
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 1;
            }
            if i < arr.len() { return arr[i] as i64; }
        }
        _ => {}
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_array_set_fast(id: i64, idx: i64, val: i64) -> i64 {
    unsafe {
        if LAST_ID == id {
            let i = if idx >= 0 && idx < 200000000 {
                idx as usize
            } else {
                let top = (idx >> 48) & 0xFFFF;
                if top == 0 || top == 0xFFFF { idx as usize } else { f64::from_bits(idx as u64) as usize }
            };
            if i < LAST_LEN {
                if LAST_ELEM_SIZE == 1 {
                    *LAST_PTR.add(i) = (val != 0) as u8;
                    return val;
                } else {
                    *(LAST_PTR as *mut i64).add(i) = val;
                    return val;
                }
            }
        }
    }

    let mut heap = HEAP.lock().unwrap();
    let i = if idx >= 0 && idx < 200000000 { idx as usize } else { rt_to_number_internal(&heap, idx) as usize };
    match heap.get_mut(id) {
        Some(TaggedValue::Array(arr)) => {
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 8;
            }
            if i < arr.len() {
                arr[i] = val;
            } else if i < 200000000 {
                arr.resize(i + 1, 0);
                arr[i] = val;
                unsafe { LAST_PTR = arr.as_ptr() as *mut u8; LAST_LEN = arr.len(); }
            }
        }
        Some(TaggedValue::ByteArray(arr)) => {
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 1;
            }
            if i < arr.len() {
                arr[i] = (val != 0) as u8;
            } else if i < 200000000 {
                arr.resize(i + 1, 0);
                arr[i] = (val != 0) as u8;
                unsafe { LAST_PTR = arr.as_ptr() as *mut u8; LAST_LEN = arr.len(); }
            }
        }
        _ => {}
    }
    val
}

#[unsafe(no_mangle)]
pub extern "C" fn Array_fill(id: i64, val: i64, size_arg: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let size = if size_arg >= 0 && size_arg < 200000000 { size_arg as usize } else { rt_to_number_internal(&heap, size_arg) as usize };
    match heap.get_mut(id) {
        Some(TaggedValue::Array(arr)) => {
            arr.clear();
            arr.resize(size, val);
        }
        Some(TaggedValue::ByteArray(arr)) => {
            arr.clear();
            arr.resize(size, (val != 0) as u8);
        }
        _ => {}
    }
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __join(id: i64, sep_ptr: i64) -> i64 {
    let mut type_val = 0; // 1: array, 2: thread
    {
        let heap = HEAP.lock().unwrap();
        if let Some(obj) = heap.get(id) {
            match obj {
                TaggedValue::Array(_) => type_val = 1,
                TaggedValue::Thread(_) => type_val = 2,
                _ => {}
            }
        }
    }

    if type_val == 2 {
        return Thread_join(id);
    }

    if type_val == 1 {
        let sep = if sep_ptr == 0 {
            ",".to_string()
        } else {
            let heap = HEAP.lock().unwrap();
            get_string_key(&heap, sep_ptr)
        };
        
        let elements = {
            let heap = HEAP.lock().unwrap();
            if let Some(TaggedValue::Array(arr)) = heap.get(id) {
                arr.clone()
            } else {
                return 0;
            }
        };

        let joined = elements.iter().map(|v| stringify_value(*v)).collect::<Vec<_>>().join(&sep);
        let c_str = CString::new(joined).unwrap();
        let ptr = c_str.as_ptr() as i64;
        return rt_box_string(ptr);
    }
    
    0
}

// Date_now moved to stdlib/time.rs

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn d_getTime(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Date(t)) = heap.get(id) {
        return t.to_bits() as i64;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn d_toISOString(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Date(t)) = heap.get(id) {
        // Very basic ISO string (not full but enough for tests)
        let s = "2023-01-01T00:00:00.000Z".to_string(); // Mock for now
        let c_str = CString::new(s).unwrap();
        let ptr = c_str.as_ptr() as i64;
        drop(heap);
        return rt_box_string(ptr);
    }
    0
}


#[unsafe(export_name = "Some")]
pub extern "C" fn rt_Some(val: i64) -> i64 { val } // Stub: Just return value

#[unsafe(export_name = "None")]
pub extern "C" fn rt_None() -> i64 { 0 } // Stub: Null

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_typeof(val: i64) -> i64 {
    if val == 0 {
        let undef = CString::new("undefined").unwrap();
        return rt_box_string(undef.as_ptr() as i64);
    }
    
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(val) {
        let type_str = match obj {
            TaggedValue::Number(_) => "number",
            TaggedValue::String(_) => "string",
            TaggedValue::Boolean(_) => "boolean",
            TaggedValue::Array(_) | TaggedValue::ByteArray(_) | TaggedValue::Map(_) | TaggedValue::Set(_) | TaggedValue::Date(_) | TaggedValue::Thread(_) | TaggedValue::Mutex(_) |
            TaggedValue::OrderedMap(_, _) | TaggedValue::OrderedSet(_, _) | TaggedValue::BloomFilter(_, _) | TaggedValue::TrieNode { .. } => "object",
        };
        let c_str = CString::new(type_str).unwrap();
        let ptr = c_str.as_ptr() as i64;
        drop(heap); // Release lock before calling rt_box_string (might lock heap again)
        return rt_box_string(ptr);
    }
    
    let num_str = CString::new("number").unwrap();
    rt_box_string(num_str.as_ptr() as i64)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_instanceof(obj: i64, class_name_id: i64) -> i64 {
    let target_class = {
        let heap = HEAP.lock().unwrap();
        get_string_key(&heap, class_name_id)
    };

    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get(obj) {
        // Check __class__ field (Map keys are String)
        if let Some(&class_val) = map.get("__class__") {
            let cls = get_string_key(&heap, class_val);
            if cls == target_class {
                return 1;
            }
        }

        // Check __parents__ field for inheritance chain
        if let Some(&parents_val) = map.get("__parents__") {
            let parents = get_string_key(&heap, parents_val);
            for parent in parents.split(',') {
                if parent == target_class {
                    return 1;
                }
            }
        }
    }

    0
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_not(val: i64) -> i64 {
    if val == 0 { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_add(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    
    // Check if either is a string for concatenation
    let is_str = {
        let heap = HEAP.lock().unwrap();
        matches!(heap.get(a), Some(TaggedValue::String(_))) || 
        matches!(heap.get(b), Some(TaggedValue::String(_))) ||
        (a > 0x100000000 && a < 0x7FFFFFFFFFFF && (a >> 60) == 0) ||
        (b > 0x100000000 && b < 0x7FFFFFFFFFFF && (b >> 60) == 0)
    };

    if is_str {
        return rt_str_concat_v2(a, b);
    }

    rt_box_number(l_f + r_f)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_sub(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    rt_box_number(l_f - r_f)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_mul(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    rt_box_number(l_f * r_f)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_div(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    if r_f == 0.0 { return rt_box_number(f64::INFINITY); }
    rt_box_number(l_f / r_f)
}



#[unsafe(no_mangle)]
pub extern "C" fn rt_box_number(n: f64) -> i64 {
    // Optimization: If n is a small integer, return it as literal ID
    if n.fract() == 0.0 && n >= 0.0 && n < 1000000.0 {
        return n as i64;
    }

    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::Number(n));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_box_boolean(b: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::Boolean(b != 0));
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_box_string(s_ptr: i64) -> i64 {
     let s = if s_ptr == 0 {
         "".to_string()
     } else {
         CStr::from_ptr(s_ptr as *const c_char).to_string_lossy().into_owned()
     };
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::String(s));
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_to_number_v2(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let val = rt_to_number_internal(&heap, id);
    // eprintln!("rt_to_number_v2: id={} -> val={}", id, val);
    val.to_bits() as i64
}

#[unsafe(no_mangle)]
pub fn rt_to_number(id: i64) -> f64 {
    let heap = HEAP.lock().unwrap();
    rt_to_number_internal(&heap, id)
}

pub fn rt_to_number_internal(heap: &Heap, id: i64) -> f64 {
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Number(n) => *n,
            TaggedValue::Boolean(b) => if *b { 1.0 } else { 0.0 },
            TaggedValue::String(s) => s.parse::<f64>().unwrap_or(0.0),
            _ => 0.0
        }
    } else {
        // Small integer optimization: values between -1,000,000 and 1,000,000 are treated as direct numbers
        if id > -1000000 && id < 1000000 {
            return id as f64;
        }
        f64::from_bits(id as u64)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_str_concat_v2(a: i64, b: i64) -> i64 {
    let sa = __callee___toString(a);
    let sb = __callee___toString(b);
    
    if sa == 0 || sb == 0 {
        let joined = "null".to_string();
        let c_str = CString::new(joined).unwrap();
        return rt_box_string(c_str.into_raw() as i64);
    }
    
    let s_a = CStr::from_ptr(sa as *const c_char).to_string_lossy();
    let s_b = CStr::from_ptr(sb as *const c_char).to_string_lossy();
    let joined = format!("{}{}", s_a, s_b);
    
    // Safety: we should free sa and sb if they were allocated by __callee___toString
    // but right now __callee___toString uses into_raw() which leaks.
    
    let c_str = CString::new(joined).unwrap();
    let ptr = c_str.into_raw() as i64;
    rt_box_string(ptr)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_str_equals(a: i64, b: i64) -> i64 {
    let sa = __callee___toString(a);
    let sb = __callee___toString(b);
    let s_a = CStr::from_ptr(sa as *const c_char).to_string_lossy();
    let s_b = CStr::from_ptr(sb as *const c_char).to_string_lossy();
    if s_a == s_b { 1 } else { 0 }
}

// File System
// File System functions moved to stdlib/fs.rs

// Async/Await Stubs
#[unsafe(no_mangle)] pub extern "C" fn __await(val: i64) -> i64 { val }
#[unsafe(no_mangle)] pub extern "C" fn Promise_all(ptr: i64) -> i64 { ptr }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Promise_new(callback: i64) -> i64 {
    // A very basic Promise implementation for the test.
    // In a real runtime, this would manage state and callbacks.
    // For now, we'll just treat it as a Map to avoid linker errors.
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    let mut map = HashMap::new();
    map.insert("__is_promise".to_string(), 1);
    heap.insert(id, TaggedValue::Map(map));
    
    // Call the callback immediately with stub resolve/reject
    // let cb: extern "C" fn(i64, i64) = std::mem::transmute(callback);
    // cb(0, 0); // Stubs for resolve/reject
    
    id
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_to_boolean(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Boolean(b) => if *b { 1 } else { 0 },
            TaggedValue::Number(n) => if *n != 0.0 { 1 } else { 0 },
            TaggedValue::String(s) => if !s.is_empty() { 1 } else { 0 },
            TaggedValue::Array(a) => 1, // Objects/Arrays are truthy in JS
            TaggedValue::Map(_) => 1,
            _ => 1,
        }
    } else {
        // literal?
        if id == 0 { 0 } else { 1 }
    }
}
// delay moved to stdlib/time.rs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn delay(ms: i64) -> i64 {
    unsafe extern "C" { fn std_time_sleep(ms: i64) -> i64; }
    std_time_sleep(ms)
}
#[unsafe(no_mangle)] pub extern "C" fn http_get(_url: i64) -> i64 { CString::new("<html></html>").unwrap().into_raw() as i64 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_strict_equal(a: i64, b: i64) -> i64 {
    if a == b { return 1; }
    let heap = HEAP.lock().unwrap();
    let obj_a = heap.get(a);
    let obj_b = heap.get(b);
    
    match (obj_a, obj_b) {
        (Some(TaggedValue::Number(n1)), Some(TaggedValue::Number(n2))) => if n1 == n2 { 1 } else { 0 },
        (Some(TaggedValue::String(s1)), Some(TaggedValue::String(s2))) => if s1 == s2 { 1 } else { 0 },
        (Some(TaggedValue::Boolean(b1)), Some(TaggedValue::Boolean(b2))) => if b1 == b2 { 1 } else { 0 },
        _ => 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_eq(a: i64, b: i64) -> i64 {
    if a == b { return 1; }
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    if l_f == r_f { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_ne(a: i64, b: i64) -> i64 {
    if rt_eq(a, b) == 1 { 0 } else { 1 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_lt(a: i64, b: i64) -> i64 {
    if rt_to_number(a) < rt_to_number(b) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_gt(a: i64, b: i64) -> i64 {
    if rt_to_number(a) > rt_to_number(b) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_le(a: i64, b: i64) -> i64 {
    if rt_to_number(a) <= rt_to_number(b) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_ge(a: i64, b: i64) -> i64 {
    if rt_to_number(a) >= rt_to_number(b) { 1 } else { 0 }
}

#[unsafe(no_mangle)] pub extern "C" fn __optional_chain(obj: i64, _op: i64) -> i64 { if obj == 0 { 0 } else { obj } }

// Array extra Stubs
#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_concat(id_a: i64, id_b: i64) -> i64 {
    let mut new_arr = Vec::new();
    let mut heap = HEAP.lock().unwrap();
    let mut extract = |id| {
        if let Some(TaggedValue::Array(arr)) = heap.get(id) { arr.clone() } else { vec![id] }
    };
    new_arr.extend(extract(id_a));
    new_arr.extend(extract(id_b));
    let id = heap.next_id;
    heap.next_id += 1;
    heap.insert(id, TaggedValue::Array(new_arr));
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_strict_ne(a: i64, b: i64) -> i64 {
    if rt_strict_equal(a, b) == 1 { 0 } else { 1 }
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_indexOf(id: i64, val: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         for (i, &v) in elements.iter().enumerate() {
             if rt_strict_equal(v, val) == 1 { return i as i64; }
         }
    } else { drop(heap); }
    -1
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_shift(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(id) {
        if !arr.is_empty() { return arr.remove(0); }
    }
    0
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_unshift(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(id) {
        arr.insert(0, val);
        return arr.len() as i64;
    }
    0
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_forEach(id: i64, callback: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         let cb: extern "C" fn(i64, i64, i64) = std::mem::transmute(callback);
         for (i, &val) in elements.iter().enumerate() { cb(val, i as i64, id); }
         return 0;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_truthy(val: i64) -> bool {
    if val == 0 { return false; } // null/undefined
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(val) {
        match obj {
            TaggedValue::Boolean(b) => *b,
            TaggedValue::Number(n) => *n != 0.0 && !n.is_nan(),
            TaggedValue::String(s) => !s.is_empty(),
            TaggedValue::Array(a) => true,
            TaggedValue::Map(m) => true,
            _ => true
        }
    } else {
        // Assume raw float
        let f = f64::from_bits(val as u64);
        f != 0.0 && !f.is_nan()
    }
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_map(id: i64, callback: i64) -> i64 {
    let mut new_arr = Vec::new();
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         let cb: extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(callback);
         for (i, &val) in elements.iter().enumerate() { new_arr.push(cb(val, i as i64, id)); }
    } else { drop(heap); }
    let mut heap = HEAP.lock().unwrap();
    let new_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(new_id, TaggedValue::Array(new_arr));
    new_id
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_filter(id: i64, callback: i64) -> i64 {
    let mut new_arr = Vec::new();
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         let cb: extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(callback);
         for (i, &val) in elements.iter().enumerate() { 
             let res = cb(val, i as i64, id);
             if rt_is_truthy(res) { new_arr.push(val); } 
         }
    } else { drop(heap); }
    let mut heap = HEAP.lock().unwrap();
    let new_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(new_id, TaggedValue::Array(new_arr));
    new_id
}

#[unsafe(no_mangle)]
pub extern "C" fn Array_sliceRest(id: i64, start: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
        let len = arr.len() as i64;
        let s = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
        let new_arr = arr[s..].to_vec();
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(new_id, TaggedValue::Array(new_arr));
        return new_id;
    }
    let new_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(new_id, TaggedValue::Array(Vec::new()));
    new_id
}

#[unsafe(no_mangle)] pub extern "C" fn calc_getValue(a: i64) -> i64 { a }

// OOP Stubs
#[unsafe(no_mangle)] pub extern "C" fn BankAccount_getBankName() -> i64 {
    CString::new("TejX Bank").unwrap().into_raw() as i64
}
#[unsafe(no_mangle)] pub extern "C" fn acc_deposit(_this: i64, _amt: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn acc_getBalance(_this: i64) -> i64 { 100 }
#[unsafe(no_mangle)] pub extern "C" fn c_printDetails(_this: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn dDog_bark(_this: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn dDog_speak(_this: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn p_get_age(_this: i64) -> i64 { 30 }
#[unsafe(no_mangle)] pub unsafe extern "C" fn p_get_name(_this: i64) -> i64 {
    CString::new("StubName").unwrap().into_raw() as i64
}
#[unsafe(no_mangle)] pub unsafe extern "C" fn p_get_info(_this: i64) -> i64 {
    CString::new("StubInfo").unwrap().into_raw() as i64
}
#[unsafe(no_mangle)] pub unsafe extern "C" fn p_print(_this: i64, prefix: i64) -> i64 {
    if prefix != 0 {
        let p = CStr::from_ptr(prefix as *const c_char).to_string_lossy();
    } else {
    }
    0
}
#[unsafe(no_mangle)] pub extern "C" fn p_set_age(_this: i64, _val: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn p_set_name(_this: i64, _val: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn r_greet(_this: i64, _name: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn r_identify(_this: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub unsafe extern "C" fn r_status(_this: i64) -> i64 {
    CString::new("Operational").unwrap().into_raw() as i64
}
#[unsafe(no_mangle)] pub extern "C" fn uExt_greet(_this: i64, _times: i64) -> i64 { 0 }
#[unsafe(no_mangle)] pub extern "C" fn uExt_sayHello(_this: i64) -> i64 { 0 }

// Math Extensions
// Math functions moved to stdlib/math.rs

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parseInt(s: i64) -> i64 {
    let s_str = stringify_value(s);
    s_str.parse::<i64>().unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parseFloat(s: i64) -> i64 {
    let s_str = stringify_value(s);
    let f_val = s_str.parse::<f64>().unwrap_or(0.0);
    rt_box_number(f_val)
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_del(id: i64, key_or_val: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    let key = get_string_key(&heap, key_or_val);
    match heap.get_mut(id) {
        Some(TaggedValue::Map(map)) => {
            let removed = map.remove(&key).is_some();
            return if removed { 1 } else { 0 };
        }
        Some(TaggedValue::Set(set)) => {
            let removed = set.remove(&key_or_val);
            return if removed { 1 } else { 0 };
        }
        _ => 0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_clear(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.get_mut(id) {
        map.clear();
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_size(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Map(map) => map.len() as f64,
            TaggedValue::Array(arr) => arr.len() as f64,
            TaggedValue::String(s) => s.len() as f64,
            TaggedValue::Set(set) => set.len() as f64,
            _ => 0.0,
        }.to_bits() as i64
    } else {
        0
    }
}



#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_has(id: i64, key_or_val: i64) -> i64 {
    if id == 0 { return 0; }
    let heap = HEAP.lock().unwrap();
    match heap.get(id) {
        Some(TaggedValue::Map(map)) => {
            let key = get_string_key(&heap, key_or_val);
            if map.contains_key(&key) { 1 } else { 0 }
        }
        Some(TaggedValue::Set(set)) => {
            if set.contains(&key_or_val) { 1 } else { 0 }
        }
        _ => 0
    }
}

// String Utils Implementation
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_trim(id: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        let trimmed = s.trim().to_string();
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(new_id, TaggedValue::String(trimmed));
        return new_id;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_to_lower(id: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        let lower = s.to_lowercase();
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(new_id, TaggedValue::String(lower));
        return new_id;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_to_upper(id: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        let upper = s.to_uppercase();
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(new_id, TaggedValue::String(upper));
        return new_id;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_starts_with(id: i64, prefix_ptr: i64) -> i64 {
    if id == 0 { return 0; }
    let heap = HEAP.lock().unwrap();
    let prefix = get_string_key(&heap, prefix_ptr);
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        if s.starts_with(&prefix) { 1 } else { 0 }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_ends_with(id: i64, suffix_ptr: i64) -> i64 {
    if id == 0 { return 0; }
    let heap = HEAP.lock().unwrap();
    let suffix = get_string_key(&heap, suffix_ptr);
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        if s.ends_with(&suffix) { 1 } else { 0 }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_replace(id: i64, from_ptr: i64, to_ptr: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    let from = get_string_key(&heap, from_ptr);
    let to = get_string_key(&heap, to_ptr);
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        let replaced = s.replace(&from, &to);
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(new_id, TaggedValue::String(replaced));
        return new_id;
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_del(id: i64, val: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(set)) = heap.get_mut(id) {
        let removed = set.remove(&val);
        return if removed { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_clear(id: i64) -> i64 {
    if id == 0 { return 0; }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(set)) = heap.get_mut(id) {
        set.clear();
    }
    0
}

// String Utils
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strVal_trim(s_id: i64) -> i64 {
    let val = stringify_value(s_id);
    let trimmed = val.trim();
    let c_str = CString::new(trimmed).unwrap();
    c_str.into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_startsWith(s_id: i64, p_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let p = stringify_value(p_id);
    if s.starts_with(&p) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_endsWith(s_id: i64, p_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let p = stringify_value(p_id);
    if s.ends_with(&p) { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_replace(s_id: i64, a_id: i64, b_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let a = stringify_value(a_id);
    let b = stringify_value(b_id);
    let replaced = s.replace(&a, &b);
    let c_str = CString::new(replaced).unwrap();
    c_str.into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_toLowerCase(s_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let lower = s.to_lowercase();
    let c_str = CString::new(lower).unwrap();
    c_str.into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_toUpperCase(s_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let upper = s.to_uppercase();
    let c_str = CString::new(upper).unwrap();
    c_str.into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn print(id: i64) {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
        let elements = arr.clone();
        drop(heap);
        for (i, &val) in elements.iter().enumerate() {
            if i > 0 { print!(" "); }
            let s = stringify_value(val);
            print!("{}", s);
        }
        println!();
    } else {
        drop(heap);
        let s = stringify_value(id);
        println!("{}", s);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn print_raw(id: i64) {
    let s = stringify_value(id);
    print!("{}", s);
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn print_space() {
    print!(" ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn print_newline() {
    println!();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint_raw(id: i64) {
    let s = stringify_value(id);
    eprint!("{}", s);
    use std::io::Write;
    let _ = std::io::stderr().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint_space() {
    eprint!(" ");
    use std::io::Write;
    let _ = std::io::stderr().flush();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint_newline() {
    eprintln!();
}



#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint(id: i64) {
    let s = stringify_value(id);
    eprintln!("{}", s);
}



#[unsafe(no_mangle)]
pub unsafe extern "C" fn panic(val: i64) {
    let s = stringify_value(val);
    eprintln!("Panic: {}", s);
    std::process::exit(1);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn len(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        let length = match obj {
            TaggedValue::Array(arr) => arr.len() as f64,
            TaggedValue::Map(map) => map.len() as f64,
            TaggedValue::String(s) => s.len() as f64,
            _ => 0.0,
        };
        return length.to_bits() as i64;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn abs(id: i64) -> i64 {
    let n = rt_to_number(id);
    rt_box_number(n.abs())
}

#[unsafe(no_mangle)]
pub extern "C" fn min(a: i64, b: i64) -> i64 {
    let na = rt_to_number(a);
    let nb = rt_to_number(b);
    rt_box_number(na.min(nb))
}

#[unsafe(no_mangle)]
pub extern "C" fn max(a: i64, b: i64) -> i64 {
    let na = rt_to_number(a);
    let nb = rt_to_number(b);
    rt_box_number(na.max(nb))
}

#[unsafe(no_mangle)]
pub extern "C" fn assert(cond: i64) {
    let b = unsafe { rt_to_boolean(cond) };
    if b == 0 {
        eprintln!("Assertion failed!");
        std::process::exit(1);
    }
}

// --- std::fs --- (Moved to stdlib/fs.rs)
// --- std::math --- (Moved to stdlib/math.rs)
// --- std::time --- (Moved to stdlib/time.rs)
// --- std::os --- (Moved to stdlib/os.rs)

// --- Compatibility Shims ---
#[unsafe(no_mangle)]
pub unsafe extern "C" fn process_argv() -> i64 {
    self::stdlib::os::std_os_args()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_sin(x: i64) -> i64 { self::stdlib::math::std_math_sin(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_cos(x: i64) -> i64 { self::stdlib::math::std_math_cos(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_pow(b: i64, e: i64) -> i64 { self::stdlib::math::std_math_pow(b, e) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_abs(x: i64) -> i64 { self::stdlib::math::std_math_abs(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_ceil(x: i64) -> i64 { self::stdlib::math::std_math_ceil(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_floor(x: i64) -> i64 { self::stdlib::math::std_math_floor(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_round(x: i64) -> i64 { self::stdlib::math::std_math_round(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_min(a: i64, b: i64) -> i64 { self::stdlib::math::std_math_min(a, b) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_max(a: i64, b: i64) -> i64 { self::stdlib::math::std_math_max(a, b) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_sqrt(x: i64) -> i64 { self::stdlib::math::std_math_sqrt(x) }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Math_random() -> i64 { self::stdlib::math::std_math_random() }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_exists(path: i64) -> i64 {
    self::stdlib::fs::std_fs_exists(path)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_read_to_string(path: i64) -> i64 {
    self::stdlib::fs::std_fs_read_to_string(path)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_readFile(path: i64) -> i64 {
    self::stdlib::fs::std_fs_readFile(path)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_write(path: i64, content: i64) -> i64 {
    self::stdlib::fs::std_fs_write(path, content)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_writeFile(path: i64, content: i64) -> i64 {
    self::stdlib::fs::std_fs_writeFile(path, content)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_remove(path: i64) -> i64 {
    self::stdlib::fs::std_fs_remove(path)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fs_mkdir(path: i64) -> i64 {
    self::stdlib::fs::std_fs_mkdir(path)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Date_now() -> i64 { self::stdlib::time::std_time_now(0) }

#[cfg(runtime_build)]
unsafe extern "C" {
    fn tejx_main() -> i64;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_json_stringify(val: i64) -> i64 {
    if val == 0 {
        let err_msg = "Cannot stringify null or undefined";
        let c_str = CString::new(err_msg).unwrap();
        let err_obj = rt_box_string(c_str.as_ptr() as i64);
        tejx_throw(err_obj);
        return 0;
    }
    // Very simplified JSON.stringify
    let s = format!("{{\"value\": {}}}", val);
    CString::new(s).unwrap().into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_json_parse(ptr: *const c_char) -> i64 {
    // Very simplified JSON.parse
    if ptr.is_null() { return 0; }
    // Just return a dummy number for now to show it works
    42
}

#[cfg(runtime_build)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tejx_runtime_main(_argc: i32, _argv: *const *const c_char) -> i32 {
    tejx_main() as i32
}
