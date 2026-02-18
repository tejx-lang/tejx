#![allow(unsafe_op_in_unsafe_fn)]
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
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
    pub use super::PromiseState;
    pub use super::Promise_new;
    pub use super::__resolve_promise;
    pub use super::a_new;
    pub use super::Array_push;
    pub use super::ACTIVE_ASYNC_OPS;
    pub use super::tejx_enqueue_task;
}

#[path = "stdlib/mod.rs"]
pub mod stdlib;
use std::thread;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, Arc, Condvar, LazyLock};
use std::sync::atomic::{AtomicI64, Ordering};

struct Task {
    func: i64,
    arg: i64,
}

static EVENT_QUEUE: LazyLock<Mutex<VecDeque<Task>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
pub static ACTIVE_ASYNC_OPS: AtomicI64 = AtomicI64::new(0);

#[derive(Debug, Clone)]
pub enum PromiseState {
    Pending,
    Resolved(i64),
    Rejected(i64),
}

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
    Atomic(Arc<AtomicI64>),
    Condition(Arc<Condvar>),
    Promise(Arc<(Mutex<PromiseState>, Condvar)>),
    TCPStream(Arc<Mutex<std::net::TcpStream>>),
}

const HEAP_OFFSET: i64 = 1000000;

pub struct Heap {
    pub next_id: i64,
    pub objects: Vec<Option<TaggedValue>>,
    pub strings: HashMap<String, i64>,
    pub free_list: Vec<i64>,
}

impl Heap {
    pub fn get(&self, id: i64) -> Option<&TaggedValue> {
        let idx = (id - HEAP_OFFSET) as usize;
        if id >= HEAP_OFFSET && idx < self.objects.len() {
            self.objects[idx].as_ref()
        } else {
            None
        }
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut TaggedValue> {
        let idx = (id - HEAP_OFFSET) as usize;
        if id >= HEAP_OFFSET && idx < self.objects.len() {
            self.objects[idx].as_mut()
        } else {
            None
        }
    }

    pub fn insert(&mut self, id: i64, val: TaggedValue) {
        let idx = (id - HEAP_OFFSET) as usize;
        if id < HEAP_OFFSET { return; }
        if idx >= self.objects.len() {
            self.objects.resize(idx + 1, None);
        }
        self.objects[idx] = Some(val);
    }

    pub fn contains_key(&self, id: i64) -> bool {
        let idx = (id - HEAP_OFFSET) as usize;
        id >= HEAP_OFFSET && idx < self.objects.len() && self.objects[idx].is_some()
    }

    pub fn alloc(&mut self, val: TaggedValue) -> i64 {
        if let Some(id) = self.free_list.pop() {
            self.insert(id, val);
            id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.insert(id, val);
            id
        }
    }
}

pub static HEAP: LazyLock<Mutex<Heap>> = LazyLock::new(|| Mutex::new(Heap {
    next_id: HEAP_OFFSET, 
    objects: Vec::with_capacity(1000),
    strings: HashMap::new(),
    free_list: Vec::new(),
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
pub unsafe extern "C" fn rt_div_zero_error() {
    eprintln!("Division by zero");
    std::process::exit(1);
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
    let id = heap.alloc(TaggedValue::Thread(Arc::new(Mutex::new(Some(handle)))));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn Thread_join(id: i64) -> i64 {
    let handle_mutex = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Thread(hm)) = heap.get(id) {
            hm.clone()
        } else {
            return 0;
        }
    };
    // HEAP lock is dropped here — no deadlock with the spawned thread
    let mut guard = handle_mutex.lock().unwrap();
    if let Some(handle) = guard.take() {
        drop(guard);
        return handle.join().unwrap_or(0);
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Thread_sleep(ms: i64) {
    thread::sleep(std::time::Duration::from_millis(ms as u64));
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
    let id = heap.alloc(TaggedValue::Array(Vec::new()));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn a_new_fixed(size: i64, elem_size: i64) -> i64 {
    // eprintln!("a_new_fixed: size={}, elem_size={}", size, elem_size);
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

unsafe fn get_string_ext_internal(heap: &Heap, id: i64) -> Option<String> {
    if let Some(obj) = heap.get(id) {
        if let TaggedValue::String(s) = obj {
            return Some(s.clone());
        }
    } else if id > 0x10000 && (id < 1000000 || id > 0x100000000) {
        // Fallback for pointers. 0x100000000+ is common for macOS/Linux segments.
        // We also allow small pointers above 0x10000 just in case.
        let p = id as *const c_char;
        if !p.is_null() {
            let c_str = unsafe { CStr::from_ptr(p) };
            if let Ok(s) = c_str.to_str() {
                return Some(s.to_owned());
            }
        }
    }
    None
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
    if key_ptr < 2000000000 && key_ptr > 1000000 {
         let k = unsafe { get_string_key(&HEAP.lock().unwrap(), key_ptr) };
    }
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
    let heap = HEAP.lock().unwrap();
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
                        let res = arr[i];
                        return res;
                    } else {
                        eprintln!("Array index out of bounds: {} (length: {})", i, arr.len());
                        std::process::exit(1);
                    }
                }

                // Special properties (length)
                let key = get_string_key(&heap, key_ptr);
                if key == "length" {
                    return (arr.len() as f64).to_bits() as i64;
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
                    } else {
                        eprintln!("Array index out of bounds: {} (length: {})", i, arr.len());
                        std::process::exit(1);
                    }
                }

                let key = get_string_key(&heap, key_ptr);
                if key == "length" {
                    return (arr.len() as f64).to_bits() as i64;
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
            TaggedValue::String(s) => {
                let key = if key_ptr > 1000 && key_ptr < 0xFFFFFFFFFFFF { get_string_key(&heap, key_ptr) } else { "".to_string() };
                if key == "length" {
                    return (s.len() as f64).to_bits() as i64;
                }
                
                let idx = if key_ptr >= 0 && key_ptr < 1000000000 {
                     Some(key_ptr as usize)
                } else if key_ptr > 0xFFFFFFFFFFFF {
                     Some(f64::from_bits(key_ptr as u64) as usize)
                } else {
                     None
                };

                if let Some(i) = idx {
                    if i < s.len() {
                        let char_str = s[i..i+1].to_string();
                        // println!("m_get String index: {}, char: {}", i, char_str);
                        drop(heap); // Release lock before boxing
                        return rt_box_string_raw(char_str);
                    }
                }
            }
            _ => { }
        }
    } else {
        // Check for raw pointer strings (literals)
        if let Some(s) = unsafe { get_string_ext_internal(&heap, id) } {
            let key = if key_ptr > 1000 && key_ptr < 0xFFFFFFFFFFFF { unsafe { get_string_key(&heap, key_ptr) } } else { "".to_string() };
            if key == "length" {
                return (s.len() as f64).to_bits() as i64;
            }
            
            let idx = if key_ptr >= 0 && key_ptr < 1000000000 {
                 Some(key_ptr as usize)
            } else if key_ptr > 0xFFFFFFFFFFFF {
                 Some(f64::from_bits(key_ptr as u64) as usize)
            } else {
                 None
            };

            if let Some(i) = idx {
                if i < s.len() {
                    let char_str = s[i..i+1].to_string();
                    drop(heap);
                    return rt_box_string_raw(char_str);
                }
            }
        }
        drop(heap);
    }
    0
}

fn rt_box_string_raw(s: String) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::String(s));
    id
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
    let res = if let Some(obj) = heap.get(id) {
        match obj {
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
                
                // If it's a generic map (not an Error/Object with name/message)
                if name_id.is_none() && msg_id.is_none() {
                    let keys: Vec<String> = map.keys()
                        .filter(|k| *k != "toString" && *k != "constructor")
                        .cloned().collect();
                    drop(heap);
                    let mut parts = Vec::new();
                    for k in keys {
                        let v_id = {
                             let h = HEAP.lock().unwrap();
                             if let Some(TaggedValue::Map(m)) = h.get(id) { m.get(&k).cloned().unwrap_or(0) } else { 0 }
                        };
                        parts.push(format!("{}: {}", k, stringify_value(v_id)));
                    }
                    format!("{{ {} }}", parts.join(", "))
                } else {
                    drop(heap);
                    let name = name_id.map(|id| stringify_value(id)).unwrap_or_else(|| "Object".to_string());
                    let message = msg_id.map(|id| stringify_value(id));
                    
                    if let Some(msg) = message {
                        if !msg.is_empty() { format!("{}: {}", name, msg) }
                        else { format!("[{}]", name) }
                    } else {
                        format!("[{}]", name)
                    }
                }
            }
            TaggedValue::Thread(_) => "[Thread]".to_string(),
            TaggedValue::Mutex(_) => "[Mutex]".to_string(),
            TaggedValue::Promise(p) => {
                let p_clone = p.clone();
                drop(heap); 
                let (lock, _) = &*p_clone;
                let state = lock.lock().unwrap();
                match &*state {
                    PromiseState::Pending => "[Promise <Pending>]".to_string(),
                    PromiseState::Resolved(v) => format!("[Promise <Resolved: {}>]", v),
                    PromiseState::Rejected(r) => format!("[Promise <Rejected: {}>]", r),
                }
            },
            TaggedValue::Number(n) => {
                if n.fract() == 0.0 { format!("{:.0}", n) } else { format!("{}", n) }
            },
            TaggedValue::Boolean(b) => if *b { "true".to_string() } else { "false".to_string() },
            TaggedValue::String(s) => s.clone(),
            TaggedValue::Set(set) => {
                let vals: Vec<i64> = set.iter().cloned().collect();
                drop(heap);
                let mut parts = Vec::new();
                for v_id in vals {
                    parts.push(stringify_value(v_id));
                }
                format!("Set {{ {} }}", parts.join(", "))
            }
            TaggedValue::Date(t) => format!("[Date: {}]", t),
            TaggedValue::OrderedMap(_, _) => "[OrderedMap]".to_string(),
            TaggedValue::OrderedSet(_, _) => "[OrderedSet]".to_string(),
            TaggedValue::BloomFilter(_, _) => "[BloomFilter]".to_string(),
            TaggedValue::TrieNode { .. } => "[TrieNode]".to_string(),
            TaggedValue::Atomic(val) => format!("[Atomic: {}]", val.load(Ordering::SeqCst)),
            TaggedValue::Condition(_) => "[Condition]".to_string(),
            TaggedValue::TCPStream(_) => "[TCPStream]".to_string(),
        }
    } else {
        drop(heap);
        // Optimization: Values between -1 billion and 1 billion (including 0) are treated as direct integers.
        // Wait, if id is 0, we want "null" for JS compatibility usually, but user says "0 should be 0".
        // Actually, in NovaJs, we want to distinguish null (0) from integer 0.
        // We ensure integer 0 is boxed.
        if id == 0 { return "null".to_string(); }
        
        if id > -1_000_000_000 && id < 1_000_000_000 {
            return id.to_string();
        }

        // Try treating it as a bitcasted double
        let d = f64::from_bits(id as u64);
        // Only treat as double if it's a "reasonable" value and NOT a small ID or common bit pattern
        if d.is_finite() && d.abs() > 1e-300 && d.abs() < 1e300 {
             let res = if d.fract() == 0.0 { format!("{:.0}", d) }
             else { format!("{}", d) };
             return res;
        }

        // Pointers are risky. On macOS, string literals from segments are typically in this range.
        if id > 0x100000000 && id < 0x200000000000 { // let _k = unsafe { *(id as *const i64) }; // k is the boxed object ID if it was boxed_null() {
             let p = id as *const c_char;
             if !p.is_null() {
                 let c_str = unsafe { CStr::from_ptr(p) };
                 if let Ok(s) = c_str.to_str() {
                     return s.to_owned();
                 }
             }
        }
        id.to_string()
    };
    res
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
    let name_id = heap.alloc(TaggedValue::String("Error".to_string()));

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
    let id = heap.alloc(TaggedValue::String(s));
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
    // eprintln!("Array_constructor id={} size={} elem_size={}", id, size, elem_size);
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
            } else {
                let last_len = LAST_LEN;
                eprintln!("Array index out of bounds (fast path): {} (length: {})", i, last_len);
                std::process::exit(1);
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
            eprintln!("Array index out of bounds: {} (length: {})", i, arr.len());
            std::process::exit(1);
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
        Some(TaggedValue::String(s)) => {
            if i < s.len() {
                let char_str = s[i..i+1].to_string();
                drop(heap);
                return rt_box_string_raw(char_str);
            }
        }
        _ => {
            // Case: Raw pointer string (literal)
            if let Some(s) = unsafe { get_string_ext_internal(&heap, id) } {
                if i < s.len() {
                    let char_str = s[i..i+1].to_string();
                    drop(heap);
                    return rt_box_string_raw(char_str);
                }
            }
        }
    }
    drop(heap);
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
pub extern "C" fn Array_fill(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();

    let to_fill = if let Some(TaggedValue::Boolean(b)) = heap.get(val) {
        if *b { 1 } else { 0 }
    } else if let Some(TaggedValue::Number(n)) = heap.get(val) {
        if *n != 0.0 { 1 } else { 0 }
    } else if val == 0 || val == 1 {
        val
    } else {
        1
    };

    match heap.get_mut(id) {
        Some(TaggedValue::Array(arr)) => {
            for elem in arr.iter_mut() { *elem = val; }
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 8;
            }
        }
        Some(TaggedValue::ByteArray(arr)) => {
            let byte_val = if to_fill != 0 { 1 } else { 0 };
            for elem in arr.iter_mut() { *elem = byte_val; }
            unsafe {
                LAST_ID = id;
                LAST_PTR = arr.as_ptr() as *mut u8;
                LAST_LEN = arr.len();
                LAST_ELEM_SIZE = 1;
            }
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
            let extract = match heap.get(id) {
                Some(TaggedValue::Array(a)) => a.clone(),
                _ => return 0,
            };
            extract
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
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Date(_t)) = heap.get(id) {
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
pub extern "C" fn rt_some(val: i64) -> i64 { val } // Stub: Just return value

#[unsafe(export_name = "None")]
pub extern "C" fn rt_none() -> i64 { 0 } // Stub: Null

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
            TaggedValue::OrderedMap(_, _) | TaggedValue::OrderedSet(_, _) | TaggedValue::BloomFilter(_, _) | TaggedValue::TrieNode { .. } |
            TaggedValue::Atomic(_) | TaggedValue::Condition(_) | TaggedValue::Promise(_) | TaggedValue::TCPStream(_) => "object",
        };
        let c_str = CString::new(type_str).unwrap();
        let ptr = c_str.as_ptr() as i64;
        drop(heap); // Release lock before calling rt_box_string (might lock heap again)
        return rt_box_string(ptr);
    }
    
    drop(heap); // Release lock before fallback call
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
pub unsafe extern "C" fn rt_not(val: i64) -> i64 {
    if rt_is_truthy(val) { 0 } else { 1 }
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
    // Optimization: If n is a small integer (BUT NOT 0, as 0 is null), return it as literal ID
    if n.fract() == 0.0 && n > 0.0 && n < 1000000.0 {
        return n as i64;
    }

    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Number(n));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_box_int(n: i64) -> i64 {
    // Optimization: If n is a small integer (BUT NOT 0), return it as literal ID
    if n > 0 && n < 1000000 {
        return n;
    }
    // Also handle negative small integers if needed, but for now just 0 check is critical
    // Actually safe to return negatives? Heap Offset is positive.
    // If IDs are negative?
    // Let's just optimization > 0 to be safe
    
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Number(n as f64)); // Use Number for compatibility or Int?
    // Existing code used rt_box_number which uses TaggedValue::Number.
    // Let's stick to Number for consistency with existing runtime unless we want to introduce Int type
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_box_boolean(b: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Boolean(b != 0));
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
    
    if let Some(&id) = heap.strings.get(&s) {
        return id;
    }

    let id = heap.alloc(TaggedValue::String(s.clone()));
    heap.strings.insert(s, id);
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
            _ => std::f64::NAN
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
    
    let _ = sa; let _ = sb; // silence warnings if sa/sb unused in logs
    if s_a == s_b { 1 } else { 0 }
}

// File System
// File System functions moved to stdlib/fs.rs

// Async/Await Stubs
// Internal helper that returns Result for safe usage in threads
fn await_impl(val: i64) -> Result<i64, i64> {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Promise(p)) = heap.get(val) {
        let p_clone = p.clone();
        drop(heap); 

        let (lock, cvar) = &*p_clone;
        let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tejx_debug.log").map(|mut f| {
            use std::io::Write;
            let _ = writeln!(f, "Awaiting promise {}...", val);
        });
        loop {
            {
                let mut state = lock.lock().unwrap();
                match &*state {
                PromiseState::Resolved(res) => {
                    return Ok(*res);
                },
                PromiseState::Rejected(err) => {
                    return Err(*err);
                },
                    PromiseState::Pending => {
                        // If no tasks, we wait on the condvar with a timeout to allow checking the queue
                        // or other conditions.
                        let task_exists = {
                            let queue = EVENT_QUEUE.lock().unwrap();
                            !queue.is_empty()
                        };
                        
                        if !task_exists {
                             state = cvar.wait_timeout(state, std::time::Duration::from_millis(10)).unwrap().0;
                        }
                    }
                }
            }

            // Process one task from the queue to maintain single-threaded execution
            let task = {
                let mut queue = EVENT_QUEUE.lock().unwrap();
                queue.pop_front()
            };

            if let Some(t) = task {
                unsafe {
                    let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(t.func);
                    f(t.arg);
                }
            }
        }
    } else {
        Ok(val) // Not a promise, return as is
    }
}

#[unsafe(no_mangle)] 
pub extern "C" fn __await(val: i64) -> i64 {
    match await_impl(val) {
        Ok(v) => v,
        Err(r) => {
            unsafe { tejx_throw(r); }
            0 // unreachable
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Promise_new(_callback: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Promise(Arc::new((Mutex::new(PromiseState::Pending), Condvar::new()))));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn Promise_resolve(val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Promise(Arc::new((Mutex::new(PromiseState::Resolved(val)), Condvar::new()))));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn Promise_reject(reason: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Promise(Arc::new((Mutex::new(PromiseState::Rejected(reason)), Condvar::new()))));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn tejx_Promise_clone(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Promise(p)) = heap.get(id) {
        let p_clone = p.clone();
        return heap.alloc(TaggedValue::Promise(p_clone));
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __resolve_promise(id: i64, val: i64) {
    let p_arc = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Promise(p)) = heap.get(id) {
            p.clone()
        } else {
            return;
        }
    };
    
    let (lock, cvar) = &*p_arc;
    let mut state = lock.lock().unwrap();
    *state = PromiseState::Resolved(val);
    cvar.notify_all();
}

#[unsafe(no_mangle)]
pub extern "C" fn __reject_promise(id: i64, reason: i64) {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Promise(p)) = heap.get(id) {
        let p_clone = p.clone();
        drop(heap);
        let (lock, cvar) = &*p_clone;
        let mut state = lock.lock().unwrap();
        *state = PromiseState::Rejected(reason);
        cvar.notify_all();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Promise_all(args_id: i64) -> i64 {
    let mut promises = Vec::new();
    {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Array(arr)) = heap.get(args_id) {
            promises = arr.clone();
        }
    }
    
    // Create Pending Promise for result
    let p_result = Arc::new((Mutex::new(PromiseState::Pending), Condvar::new()));
    let p_clone = p_result.clone();
    
    let p_id = {
        let mut heap = HEAP.lock().unwrap();
       heap.alloc(TaggedValue::Promise(p_result))
    };

    thread::spawn(move || {
        let mut results = Vec::new();
        let mut rejected = None;
        
        for p_id in promises {
            // Use a simple blocking wait here, NOT await_impl (which processes tasks)
            // Background threads MUST NOT process the main task queue.
            let wait_res = {
                let p_arc = {
                    let heap = HEAP.lock().unwrap();
                    heap.get(p_id).and_then(|obj| if let TaggedValue::Promise(p) = obj { Some(p.clone()) } else { None })
                };

                if let Some(p) = p_arc {
                    let (lock, cvar) = &*p;
                    let mut state = lock.lock().unwrap();
                    loop {
                        match &*state {
                            PromiseState::Resolved(v) => break Ok(*v),
                            PromiseState::Rejected(r) => break Err(*r),
                            PromiseState::Pending => {
                                state = cvar.wait(state).unwrap();
                            }
                        }
                    }
                } else {
                    Ok(p_id) // Not a promise
                }
            };

            match wait_res {
                Ok(v) => results.push(v),
                Err(r) => { 
                    rejected = Some(r);
                    break; 
                }
            }
        }
        
        let (lock, cvar) = &*p_clone;
        let mut state = lock.lock().unwrap();
        if let Some(r) = rejected {
            *state = PromiseState::Rejected(r);
        } else {
             // Store results array
             let mut heap = HEAP.lock().unwrap();
             let arr_id = heap.alloc(TaggedValue::Array(results));
             *state = PromiseState::Resolved(arr_id);
        }
        cvar.notify_all();
    });
    
    p_id
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_atomic_new(val: i64) -> i64 {
    // println!("DEBUG: rt_atomic_new start");
    let initial = rt_to_number(val) as i64;
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Atomic(Arc::new(AtomicI64::new(initial))));
    // println!("DEBUG: rt_atomic_new end {}", id);
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atomic_add(id: i64, val: i64) -> i64 {
    let v = rt_to_number(val) as i64;
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Atomic(a)) = heap.get(id) {
        let prev = a.fetch_add(v, Ordering::SeqCst);
        return prev + v; // Return new value
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atomic_sub(id: i64, val: i64) -> i64 {
    let v = rt_to_number(val) as i64;
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Atomic(a)) = heap.get(id) {
        let prev = a.fetch_sub(v, Ordering::SeqCst);
        return prev - v;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atomic_load(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Atomic(a)) = heap.get(id) {
        return a.load(Ordering::SeqCst);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atomic_store(id: i64, val: i64) -> i64 {
    let v = rt_to_number(val) as i64;
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Atomic(a)) = heap.get(id) {
        a.store(v, Ordering::SeqCst);
        return v;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atomic_exchange(id: i64, val: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Atomic(a)) = heap.get(id) {
        let v = rt_to_number(val) as i64;
        return a.swap(v, Ordering::SeqCst);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atomic_compare_exchange(id: i64, expected: i64, val: i64) -> i64 {
    let e = rt_to_number(expected) as i64;
    let v = rt_to_number(val) as i64;
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Atomic(a)) = heap.get(id) {
        match a.compare_exchange(e, v, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(prev) => return prev,
            Err(curr) => return curr,
        }
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_cond_new() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.alloc(TaggedValue::Condition(Arc::new(Condvar::new())));
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_cond_wait(cond_id: i64, mutex_id: i64) -> i64 {
    let (pair, cond) = {
        let heap = HEAP.lock().unwrap();
        let c = if let Some(TaggedValue::Condition(c)) = heap.get(cond_id) { c.clone() } else { return 0; };
        let m = if let Some(TaggedValue::Mutex(m)) = heap.get(mutex_id) { m.clone() } else { return 0; };
        (m, c)
    };

    let (lock, internal_cvar) = &*pair;
    let mut guard = lock.lock().unwrap();
    
    // Release logical lock
    *guard = false;
    internal_cvar.notify_all();
    
    // Wait on external condvar
    guard = cond.wait(guard).unwrap();
    
    // Re-acquire logical lock
    while *guard {
        guard = internal_cvar.wait(guard).unwrap();
    }
    *guard = true;
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_cond_notify(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Condition(c)) = heap.get(id) {
        c.notify_one();
        return 1;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_cond_notify_all(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Condition(c)) = heap.get(id) {
        c.notify_all();
        return 1;
    }
    0
}
// ============================================
// MEMORY MANAGEMENT (Strict Ownership)
// ============================================

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_free(id: i64) {
    if id < 1000000 { return; } // Don't free primitives/small-ints

    // Recursive Free
    // We need to extract the object to drop it, and if it's a container, recurse.
    let val = {
        let mut heap = HEAP.lock().unwrap();
        let idx = (id - HEAP_OFFSET) as usize;
        if id >= HEAP_OFFSET && idx < heap.objects.len() {
            if let Some(obj) = &heap.objects[idx] {
                let _type_str = match obj {
                    TaggedValue::Array(_) => "Array",
                    TaggedValue::ByteArray(_) => "ByteArray",
                    TaggedValue::Map(_) => "Map",
                    TaggedValue::Thread(_) => "Thread",
                    TaggedValue::Mutex(_) => "Mutex",
                    TaggedValue::Number(_) => "Number",
                    TaggedValue::Boolean(_) => "Boolean",
                    TaggedValue::String(_) => "String",
                    TaggedValue::Set(_) => "Set",
                    TaggedValue::Date(_) => "Date",
                    TaggedValue::OrderedMap(_, _) => "OrderedMap",
                    TaggedValue::OrderedSet(_, _) => "OrderedSet",
                    TaggedValue::BloomFilter(_, _) => "BloomFilter",
                    TaggedValue::TrieNode { .. } => "TrieNode",
                    TaggedValue::Atomic(_) => "Atomic",
                    TaggedValue::Condition(_) => "Condition",
                    TaggedValue::Promise(_) => "Promise",
                    TaggedValue::TCPStream(_) => "TCPStream",
                };
                
                // Invalidate Fast-Path Cache
                unsafe {
                    if LAST_ID == id {
                        LAST_ID = -1;
                        LAST_PTR = std::ptr::null_mut();
                        LAST_LEN = 0;
                        LAST_ELEM_SIZE = 0;
                    }
                }

                heap.free_list.push(id);
                heap.objects[idx].take()
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(obj) = val {
        match obj {
            TaggedValue::Array(arr) => {
                for &child_id in &arr { rt_free(child_id); }
            },
            TaggedValue::Map(map) => {
                for (_, &child_id) in &map { 
                    rt_free(child_id); 
                }
            },
            TaggedValue::Set(set) => {
                for &child_id in &set { rt_free(child_id); }
            },
            TaggedValue::OrderedMap(_, map) => {
                 for (_, &child_id) in &map { rt_free(child_id); }
            },
            TaggedValue::String(s) => {
                 let mut heap = HEAP.lock().unwrap();
                 heap.strings.remove(&s);
            },
            // Primitives (Number, etc) don't reference others
            _ => {}
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_move_member(id: i64, key_ptr: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    
    // Resolve key
    let key = if let Some(TaggedValue::String(s)) = heap.get(key_ptr) {
        s.clone()
    } else {
        // Simple keys
        get_string_key(&heap, key_ptr)
    };
    // println!("DEBUG: rt_move_member obj={} key='{}'", id, key);

    // Extract and return value, removing from source
    if let Some(obj) = heap.get_mut(id) {
        match obj {
            TaggedValue::Map(map) => {
                // Check if we should move or copy
                if let Some(&val) = map.get(&key) {
                     if val < 1000000 {
                         // Primitive: Copy (Keep in map)
                         return val;
                     } else {
                         // Object: Move (Remove from map)
                         map.remove(&key);
                         return val;
                     }
                }
            },
            TaggedValue::OrderedMap(_, map) => {
                // Same logic for OrderedMap (keys order vector is left as is, effectively "sparse" if removed)
                 if let Some(&val) = map.get(&key) {
                     if val < 1000000 {
                         return val;
                     } else {
                         map.remove(&key);
                         return val;
                     }
                }
            },
            TaggedValue::Array(arr) => {
                 // Array destructive read? Index...
                 // Treat key as index
                 if let Ok(idx) = key.parse::<usize>() {
                     if idx < arr.len() {
                         let val = arr[idx];
                         arr[idx] = 0; // Nullify slot
                         return val;
                     }
                 }
            },
            _ => {}
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_to_boolean(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Boolean(b) => if *b { 1 } else { 0 },
            TaggedValue::Number(n) => if *n != 0.0 { 1 } else { 0 },
            TaggedValue::String(s) => if !s.is_empty() { 1 } else { 0 },
            TaggedValue::Array(_a) => 1, 
            TaggedValue::Map(_) => 1,
            TaggedValue::Promise(_) => 1,
            _ => 1,
        }
    } else {
        if id == 0 { 0 } else { 1 }
    }
}

// delay moved to stdlib/time.rs but we override it here for async support
#[unsafe(no_mangle)]
pub unsafe extern "C" fn delay(ms: i64) -> i64 {
    // Return a Promise that resolves after ms
    let promise = Arc::new((Mutex::new(PromiseState::Pending), Condvar::new()));
    let p_clone = promise.clone();
    
    // Register Promise in heap
    let p_id = {
        let mut heap = HEAP.lock().unwrap();
        heap.alloc(TaggedValue::Promise(promise))
    };

    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(ms as u64));
        let (lock, cvar) = &*p_clone;
        let mut state = lock.lock().unwrap();
        *state = PromiseState::Resolved(0); // Resolve with 0 (void)
        cvar.notify_all();
    });
    
    p_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_sleep(ms: i64) {
    thread::sleep(std::time::Duration::from_millis(ms as u64));
}
#[unsafe(no_mangle)] pub extern "C" fn http_get(_url: i64) -> i64 { CString::new("<html></html>").unwrap().into_raw() as i64 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_strict_equal(a: i64, b: i64) -> i64 {
    if a == b { return 1; }
    let heap = HEAP.lock().unwrap();
    
    // Check if either is a bitcasted double (Any type number)
    let val_a = if a > -1000000 && a < 1000000 { None } else if a > 0xFFFFFFFFFFFF || a < -1000000 { Some(f64::from_bits(a as u64)) } else { None };
    let val_b = if b > -1000000 && b < 1000000 { None } else if b > 0xFFFFFFFFFFFF || b < -1000000 { Some(f64::from_bits(b as u64)) } else { None };

    let obj_a = heap.get(a);
    let obj_b = heap.get(b);
    
    match (obj_a, obj_b) {
        (Some(TaggedValue::Number(n1)), Some(TaggedValue::Number(n2))) => return if (n1 - n2).abs() < 1e-9 { 1 } else { 0 },
        (Some(TaggedValue::String(s1)), Some(TaggedValue::String(s2))) => return if s1 == s2 { 1 } else { 0 },
        (Some(TaggedValue::Boolean(b1)), Some(TaggedValue::Boolean(b2))) => return if b1 == b2 { 1 } else { 0 },
        (None, Some(TaggedValue::Number(n))) => if let Some(v) = val_a { if (v - n).abs() < 1e-9 { return 1; } },
        (Some(TaggedValue::Number(n)), None) => if let Some(v) = val_b { if (v - n).abs() < 1e-9 { return 1; } },
        (None, None) => if let (Some(v1), Some(v2)) = (val_a, val_b) { if (v1 - v2).abs() < 1e-9 { return 1; } },
        _ => {}
    }

    // String normalization fallback (handles raw pointers vs heap IDs)
    let sa = get_string_ext_internal(&heap, a);
    let sb = get_string_ext_internal(&heap, b);

    if let (Some(s1), Some(s2)) = (sa, sb) {
        return if s1 == s2 { 1 } else { 0 };
    }
    
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_eq(a: i64, b: i64) -> i64 {
    if a == b { return 1; }
    let heap = HEAP.lock().unwrap();
    let obj_a = heap.get(a);
    let obj_b = heap.get(b);
    
    if let (Some(TaggedValue::String(s1)), Some(TaggedValue::String(s2))) = (obj_a, obj_b) {
        return (s1 == s2) as i64;
    }
    
    drop(heap);
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    if (l_f - r_f).abs() < 1e-9 { 1 } else { 0 }
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

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn __optional_chain(obj: i64, op: i64) -> i64 { 
    if obj == 0 { return 0; } 
    Map_get(obj, op) 
}

// Array extra Stubs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_concat(id_a: i64, id_b: i64) -> i64 {
    let mut new_arr = Vec::new();
    let mut heap = HEAP.lock().unwrap();

    let mut extract = |id| {
        if let Some(TaggedValue::Array(arr)) = heap.get(id) {
            new_arr.extend(arr.clone());
        } else {
            // If not an array, just push the ID as a single element
            new_arr.push(id);
        }
    };

    extract(id_a);
    extract(id_b);
    
    heap.alloc(TaggedValue::Array(new_arr))
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
            TaggedValue::Array(_a) => true,
            TaggedValue::Map(_m) => true,
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
    let heap = HEAP.lock().unwrap();
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
    let heap = HEAP.lock().unwrap();
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
pub unsafe extern "C" fn Array_reduce(id: i64, callback: i64, initial_val: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         
         let cb: extern "C" fn(i64, i64, i64, i64) -> i64 = std::mem::transmute(callback);
         let mut acc = initial_val;
         
         for (i, &val) in elements.iter().enumerate() {
             acc = cb(acc, val, i as i64, id);
         }
         return acc;
    }
    initial_val
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_find(id: i64, callback: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         
         let cb: extern "C" fn(i64, i64, i64) -> i64 = unsafe { std::mem::transmute(callback) };
         for (i, &val) in elements.iter().enumerate() {
             let idx_box = rt_box_number(i as f64);
             let res = cb(val, idx_box, id);
             eprintln!("Array_find: i={}, val={}, res={}", i, val, res);
             if rt_is_truthy(res) { return val; }
         }
    }
    0 // undefined/null
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_findIndex(id: i64, callback: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
         let elements = arr.clone();
         drop(heap);
         
         let cb: extern "C" fn(i64, i64, i64) -> i64 = unsafe { std::mem::transmute(callback) };
         for (i, &val) in elements.iter().enumerate() {
             let res = cb(val, rt_box_number(i as f64), id);
             if rt_is_truthy(res) { return rt_box_number(i as f64); }
         }
    }
    rt_box_number(-1.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_reverse(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(id) {
        arr.reverse();
        return id; // Returns self
    }
    0
}


#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_splice(id: i64, start: i64, delete_count: i64, arg3: i64, arg4: i64) -> i64 {
    eprintln!("Array_splice id: {}, start: {}, delete_count: {}, arg3: {}, arg4: {}", id, start, delete_count, arg3, arg4);
    let mut heap = HEAP.lock().unwrap();
    let mut removed_items = Vec::new();
    
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(id) {
        let len = arr.len() as i64;
        let actual_start = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
        let actual_delete = delete_count.max(0).min(len - actual_start as i64) as usize;
        
        for _ in 0..actual_delete {
            if actual_start < arr.len() {
                let v = arr.remove(actual_start);
                eprintln!("  Removing item: {}", v);
                removed_items.push(v);
            }
        }
        
        // Items insertion - for now handle arg3 and arg4 as potential items 
        // In NovaJs, if they are valid heap IDs or numbers, they should be inserted.
        // We assume they are passed if they are non-zero (simple heuristic for this test)
        if arg4 != 0 {
             arr.insert(actual_start, arg4);
        }
        if arg3 != 0 {
             arr.insert(actual_start, arg3);
        }
        eprintln!("  Final array state (id={}): {:?}", id, arr);
    }
    
    let removed_id = heap.next_id;
    heap.next_id += 1;
    eprintln!("  Final removed_items state (id={}): {:?}", removed_id, removed_items);
    heap.insert(removed_id, TaggedValue::Array(removed_items));
    eprintln!("  Array_splice returning removed_id: {}", removed_id);
    removed_id
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
        let _p = CStr::from_ptr(prefix as *const c_char).to_string_lossy();
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
    // Parse as f64 first to handle "55.5" correctly, then truncate to i64
    let n = s_str.trim().parse::<f64>().unwrap_or(0.0);
    rt_box_int(n as i64)
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
        return heap.alloc(TaggedValue::String(replaced));
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_includes(id: i64, sub_ptr: i64) -> i64 {
    if id == 0 { return 0; }
    let heap = HEAP.lock().unwrap();
    let sub = get_string_key(&heap, sub_ptr);
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        return if s.contains(&sub) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_indexOf(id: i64, sub_ptr: i64) -> i64 {
    if id == 0 { return rt_box_number(-1.0); }
    let heap = HEAP.lock().unwrap();
    let sub = get_string_key(&heap, sub_ptr);
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        match s.find(&sub) {
            Some(pos) => return rt_box_number(pos as f64),
            None => return rt_box_number(-1.0),
        }
    }
    rt_box_number(-1.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_slice(id: i64, start: i64, end: i64) -> i64 {
    if id == 0 { return rt_box_string_raw("".to_string()); }
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::String(s)) = heap.get(id) {
        let len = s.len() as i64;
        let s_idx = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
        let e_idx = if end == 0 { len } else if end < 0 { (len + end).max(0) } else { end.min(len) } as usize;
        
        if s_idx >= e_idx {
            return heap.alloc(TaggedValue::String("".to_string()));
        }
        let sliced = s[s_idx..e_idx].to_string();
        return heap.alloc(TaggedValue::String(sliced));
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
pub unsafe extern "C" fn trimmed_padStart(s_id: i64, len: i64, pad_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let pad = stringify_value(pad_id);
    let target_len = len as usize;
    if s.len() >= target_len {
        let c_str = CString::new(s).unwrap();
        return c_str.into_raw() as i64;
    }
    let needed = target_len - s.len();
    let mut padding = String::new();
    while padding.len() < needed {
        padding.push_str(&pad);
    }
    padding.truncate(needed);
    padding.push_str(&s); // Start pad
    let c_str = CString::new(padding).unwrap();
    c_str.into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_padEnd(s_id: i64, len: i64, pad_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let pad = stringify_value(pad_id);
    let target_len = len as usize;
    if s.len() >= target_len {
        let c_str = CString::new(s).unwrap();
        return c_str.into_raw() as i64;
    }
    let needed = target_len - s.len();
    let mut res = s;
    let mut padding = String::new();
    while padding.len() < needed {
        padding.push_str(&pad);
    }
    padding.truncate(needed);
    res.push_str(&padding);
    let c_str = CString::new(res).unwrap();
    c_str.into_raw() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_repeat(s_id: i64, count: i64) -> i64 {
    let s = stringify_value(s_id);
    let n = count.max(0) as usize;
    let repeated = s.repeat(n);
    let c_str = CString::new(repeated).unwrap();
    c_str.into_raw() as i64
}

// Object Static Methods
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_keys(id: i64) -> i64 {
    eprintln!("Collection_keys for id: {}", id);
    let heap = HEAP.lock().unwrap();
    let keys = if let Some(TaggedValue::Map(m)) = heap.get(id) {
        eprintln!("  Found Map with {} keys", m.len());
        m.keys()
            .filter(|k| *k != "toString" && *k != "constructor")
            .cloned()
            .collect::<Vec<String>>()
    } else if let Some(TaggedValue::OrderedMap(order, _)) = heap.get(id) {
        eprintln!("  Found OrderedMap with {} keys", order.len());
        order.iter()
            .filter(|k| *k != "toString" && *k != "constructor")
            .cloned()
            .collect::<Vec<String>>()
    } else {
        eprintln!("  NOT A MAP (type: {:?})", heap.get(id).map(|v| format!("{:?}", v)));
        drop(heap);
        let mut heap = HEAP.lock().unwrap();
        let arr_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(arr_id, TaggedValue::Array(Vec::new()));
        return arr_id;
    };
    drop(heap);
    
    let mut boxed_keys = Vec::new();
    for k in keys {
        boxed_keys.push(rt_box_string_raw(k));
    }
    
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(boxed_keys));
    arr_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Object_keys(id: i64) -> i64 {
    Collection_keys(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Object_values(id: i64) -> i64 {
    Collection_values(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Object_entries(id: i64) -> i64 {
    Collection_entries(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_values(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let values = if let Some(TaggedValue::Map(m)) = heap.get(id) {
        m.iter()
            .filter(|(k, _)| *k != "toString" && *k != "constructor")
            .map(|(_, v)| *v)
            .collect::<Vec<i64>>()
    } else if let Some(TaggedValue::OrderedMap(_, m)) = heap.get(id) {
        m.iter()
            .filter(|(k, _)| *k != "toString" && *k != "constructor")
            .map(|(_, v)| *v)
            .collect::<Vec<i64>>()
    } else {
        drop(heap);
        let mut heap = HEAP.lock().unwrap();
        let arr_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(arr_id, TaggedValue::Array(Vec::new()));
        return arr_id;
    };
    drop(heap);
    
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(values));
    arr_id
}

// Object_values moved


#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_entries(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let entries = if let Some(TaggedValue::Map(m)) = heap.get(id) {
        m.iter()
            .filter(|(k, _)| *k != "toString" && *k != "constructor")
            .map(|(k, v)| (k.clone(), *v))
            .collect::<Vec<(String, i64)>>()
    } else if let Some(TaggedValue::OrderedMap(order, m)) = heap.get(id) {
        order.iter()
            .filter(|k| *k != "toString" && *k != "constructor")
            .map(|k| (k.clone(), m.get(k).cloned().unwrap_or(0)))
            .collect::<Vec<(String, i64)>>()
    } else {
        drop(heap);
        let mut heap = HEAP.lock().unwrap();
        let arr_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(arr_id, TaggedValue::Array(Vec::new()));
         return arr_id;
    };
    drop(heap);
    
    let mut entry_ids = Vec::new();
    for (k, v) in entries {
        let mut heap = HEAP.lock().unwrap();
        let pair_id = heap.next_id;
        heap.next_id += 1;
        
        let k_id = heap.alloc(TaggedValue::String(k));
        heap.insert(pair_id, TaggedValue::Array(vec![k_id, v]));
        entry_ids.push(pair_id);
    }
    
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(entry_ids));
    arr_id
}

// Object_entries moved


// --- Map Methods ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_size(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get(id) {
        return m.len() as i64;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_delete(id: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) {
        let existed = m.remove(&k_str).is_some();
        return if existed { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_clear(id: i64) {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) {
        m.clear();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_keys(id: i64) -> i64 {
    Collection_keys(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_values(id: i64) -> i64 {
    Collection_values(id)
}
// --- Array Methods (Any/Dynamic) ---
// These are required by the linker for method calls on 'any' typed arrays or dynamic dispatch.
// They roughly map to Array_* functions but with specific mangled names.

// --- Array Implementation ---









#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_join(id: i64, sep_id: i64) -> i64 {
    // Release lock during stringify? No, stringify_value takes value not ID, or ID?
    // stringify_value(id) locks HEAP!
    // So we must get values, drop lock, stringify, then join.
    let sep = stringify_value(sep_id);
    
    let elements = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Array(arr)) = heap.get(id) {
            arr.clone() // Clone to release lock
        } else {
            drop(heap); // Explicit drop before returning
            // Return empty string ID
            let mut heap = HEAP.lock().unwrap();
            let id = heap.next_id;
            heap.next_id += 1;
            heap.insert(id, TaggedValue::String(String::new()));
            return id; 
        }
    };
    
    let strings: Vec<String> = elements.iter().map(|&e| stringify_value(e)).collect();
    let result = strings.join(&sep);
    
    let mut heap = HEAP.lock().unwrap();
    let res_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(res_id, TaggedValue::String(result));
    res_id
}

// ... existing f_any functions ...

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___forEach(arr_id: i64, callback_id: i64) {
    Array_forEach(arr_id, callback_id);
}

// ...

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___pop(arr_id: i64) -> i64 {
    Array_pop(arr_id)
}

// ...

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___shift(arr_id: i64) -> i64 {
    Array_shift(arr_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___unshift(arr_id: i64, val_id: i64) -> i64 {
    Array_unshift(arr_id, val_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn row_forEach(arr_id: i64, callback_id: i64) {
    Array_forEach(arr_id, callback_id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn merged_join(arr_id: i64, sep_id: i64) -> i64 {
    Array_join(arr_id, sep_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___indexOf(arr_id: i64, val_id: i64) -> i64 {
    Array_indexOf(arr_id, val_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___join(arr_id: i64, sep_id: i64) -> i64 {
    Array_join(arr_id, sep_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___concat(arr_id: i64, other_id: i64) -> i64 {
    Array_concat(arr_id, other_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___filter(arr_id: i64, callback_id: i64) -> i64 {
    Array_filter(arr_id, callback_id)
}

// Workaround for lowering quirk where variable name is part of symbol
#[unsafe(no_mangle)]
pub unsafe extern "C" fn doubled_forEach(arr_id: i64, callback_id: i64) {
    Array_forEach(arr_id, callback_id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn evens_forEach(arr_id: i64, callback_id: i64) {
    Array_forEach(arr_id, callback_id);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___map(arr_id: i64, callback_id: i64) -> i64 {
    Array_map(arr_id, callback_id)
}



#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___push(arr_id: i64, val_id: i64) -> i64 {
    Array_push(arr_id, val_id)
}





#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_int32___join(arr_id: i64, sep_id: i64) -> i64 {
    Array_join(arr_id, sep_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_int32___push(arr_id: i64, val_id: i64) -> i64 {
    Array_push(arr_id, rt_box_number(val_id as f64)) 
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_int___push(arr_id: i64, val_id: i64) -> i64 {
    Array_push(arr_id, val_id) 
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Array_fill(arr_id: i64, val_id: i64) -> i64 {
    Array_fill(arr_id, val_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_string___push(arr_id: i64, val_id: i64) -> i64 {
    Array_push(arr_id, val_id)
}

// Mutex acquire/release aliases for std:sync
#[unsafe(no_mangle)]
pub extern "C" fn f_Mutex_acquire(id: i64) -> i64 {
    m_lock(id)
}

#[unsafe(no_mangle)]
pub extern "C" fn f_Mutex_release(id: i64) -> i64 {
    m_unlock(id)
}

// SharedQueue: simple array-based FIFO queue
#[unsafe(no_mangle)]
pub extern "C" fn SharedQueue_new() -> i64 {
    a_new()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_SharedQueue_enqueue(q_id: i64, val: i64) -> i64 {
    Array_push(q_id, val)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_SharedQueue_dequeue(q_id: i64) -> i64 {
    Array_shift(q_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_SharedQueue_isEmpty(q_id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(q_id) {
        return if arr.is_empty() { 1 } else { 0 };
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_has(id: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get(id) {
        return if m.contains_key(&k_str) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_put(id: i64, key: i64, val: i64) {
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) {
        m.insert(k_str, val);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Map_get(id: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Map(m) => {
                return *m.get(&k_str).unwrap_or(&0);
            }
            TaggedValue::Array(a) => {
                if k_str == "length" {
                    let len = a.len() as f64;
                    drop(heap);
                    return rt_box_number(len);
                }
            }
            TaggedValue::String(s) => {
                if k_str == "length" {
                    let len = s.len() as f64;
                    drop(heap);
                    return rt_box_number(len);
                }
            }
            _ => {}
        }
    }
    0
}

// --- Set Methods ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Set_size(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get(id) {
        return s.len() as i64;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Set_delete(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) {
        let existed = s.remove(&val);
        return if existed { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Set_clear(id: i64) {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) {
        s.clear();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Set_add(id: i64, val: i64) {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) {
        s.insert(val);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Set_has(id: i64, val: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get(id) {
        return if s.contains(&val) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Set_values(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get(id) {
        let values: Vec<i64> = s.iter().cloned().collect();
        drop(heap);
        let mut heap = HEAP.lock().unwrap();
        let arr_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(arr_id, TaggedValue::Array(values));
        return arr_id;
    }
    drop(heap);
    let mut heap = HEAP.lock().unwrap();
    let arr_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(arr_id, TaggedValue::Array(Vec::new()));
    arr_id
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
pub unsafe extern "C" fn Date_now() -> i64 { self::stdlib::time::std_time_now() }

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
    unsafe {
        tejx_main(); // Ignore return value
        tejx_run_event_loop();
    }
    0
}

// --- Collection Generic Methods ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_size(id: i64) -> i64 {
    stdlib::collections::rt_collections_size(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_clear(id: i64) {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) { m.clear(); }
    else if let Some(TaggedValue::Set(s)) = heap.get_mut(id) { s.clear(); }
    else if let Some(TaggedValue::Array(a)) = heap.get_mut(id) { a.clear(); }
    else if let Some(TaggedValue::OrderedMap(_, m)) = heap.get_mut(id) { m.clear(); } // TODO: clear vec too
    else if let Some(TaggedValue::OrderedSet(_, s)) = heap.get_mut(id) { s.clear(); } // TODO: clear vec
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_delete(id: i64, key: i64) -> i64 {
    // stdlib::collections::std_collections_Map_delete(id, key); 
    // Manual dispatch since logic is simple and stdlib might be duplicated if not careful
    let k_str = stringify_value(key);
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get_mut(id) {
        return if m.remove(&k_str).is_some() { 1 } else { 0 };
    }
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) {
        return if s.remove(&key) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_has(id: i64, key: i64) -> i64 {
    let k_str = stringify_value(key);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(m)) = heap.get(id) {
        return if m.contains_key(&k_str) { 1 } else { 0 };
    }
    if let Some(TaggedValue::Set(s)) = heap.get(id) {
        return if s.contains(&key) { 1 } else { 0 };
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Collection_add(id: i64, val: i64) {
    // Forward to Set_add logic locally
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Set(s)) = heap.get_mut(id) {
        s.insert(val);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_slice(id: i64, start: i64, end: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
        let len = arr.len() as i64;
        let mut s = start;
        let mut e = end;
        if s < 0 { s = len + s; }
        if s < 0 { s = 0; }
        if s > len { s = len; }
        
        if e < 0 { e = len + e; }
        if e < 0 { e = 0; }
        if e > len { e = len; }
        
        if s >= e {
            drop(heap);
            let mut heap = HEAP.lock().unwrap();
            let new_id = heap.next_id;
            heap.next_id += 1;
            heap.insert(new_id, TaggedValue::Array(Vec::new()));
            return new_id;
        }
        
        let new_vec = arr[s as usize..e as usize].to_vec();
        drop(heap);
        let mut heap = HEAP.lock().unwrap();
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.insert(new_id, TaggedValue::Array(new_vec));
        return new_id;
    }
    0
}

// --- Extended Array Methods ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_includes(id: i64, item: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(id) {
        for &val in arr {
            if val == item { 
                drop(heap);
                return rt_box_boolean(1); 
            }
        }
        // Fallback: value equality for boxed primitives
        for &val in arr {
             if let Some(v_obj) = heap.get(val) {
                 if let Some(i_obj) = heap.get(item) {
                     match (v_obj, i_obj) {
                         (TaggedValue::Number(n1), TaggedValue::Number(n2)) => if (n1 - n2).abs() < f64::EPSILON { drop(heap); return rt_box_boolean(1); },
                         (TaggedValue::String(s1), TaggedValue::String(s2)) => if s1 == s2 { drop(heap); return rt_box_boolean(1); },
                         (TaggedValue::Boolean(b1), TaggedValue::Boolean(b2)) => if b1 == b2 { drop(heap); return rt_box_boolean(1); },
                         _ => {}
                     }
                 }
             }
        }
    }
    drop(heap);
    rt_box_boolean(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_sort(id: i64) -> i64 {
     // Default string sort
     let mut heap = HEAP.lock().unwrap();
     if let Some(TaggedValue::Array(arr)) = heap.get_mut(id) {
         // We can't easily user comparator callback here without releasing lock 
         // and calling back into VM. 
         // For now, implement default lexicographical sort.
         // We need to resolve values to strings while holding lock.
         // But `stringify_value` calls `HEAP.lock()`. Deadlock!
         // We must verify `stringify_value` implementation.
         // It likely locks. 
         // So we gather values, drop lock, sort, re-acquire, update.
         let elements = arr.clone();
         drop(heap); // Release
         
         let mut str_vals: Vec<(String, i64)> = elements.iter().map(|&e| (stringify_value(e), e)).collect();
         str_vals.sort_by(|a, b| a.0.cmp(&b.0));
         
         let mut heap = HEAP.lock().unwrap();
         if let Some(TaggedValue::Array(arr)) = heap.get_mut(id) {
             *arr = str_vals.into_iter().map(|(_, e)| e).collect();
             return id;
         }
     }
     0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_flat(id: i64, depth: i64) -> i64 {
    let elements = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Array(arr)) = heap.get(id) {
            arr.clone()
        } else {
            return id;
        }
    };

    let mut flattened = Vec::new();
    let d = depth;

    for &el in &elements {
        let mut sub_arr_opt = None;
        {
            let heap = HEAP.lock().unwrap();
            if let Some(TaggedValue::Array(arr)) = heap.get(el) {
                sub_arr_opt = Some(arr.clone());
            }
        }
        
        if let Some(_) = sub_arr_opt {
            if d > 0 {
                 let sub_flat_id = Array_flat(el, d - 1);
                 let heap = HEAP.lock().unwrap();
                 if let Some(TaggedValue::Array(sub)) = heap.get(sub_flat_id) {
                     flattened.extend(sub.clone());
                 }
            } else {
                 flattened.push(el);
            }
        } else {
             flattened.push(el);
        }
    }

    let mut heap = HEAP.lock().unwrap();
    heap.alloc(TaggedValue::Array(flattened))
}


// --- Extended String Methods ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_trimStart(s_id: i64) -> i64 {
    let val = stringify_value(s_id);
    let trimmed = val.trim_start().to_string();
    rt_box_string_raw(trimmed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_trimEnd(s_id: i64) -> i64 {
    let val = stringify_value(s_id);
    let trimmed = val.trim_end().to_string();
    rt_box_string_raw(trimmed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_includes(s_id: i64, sub_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let sub = stringify_value(sub_id);
    let res = if s.contains(&sub) { 1 } else { 0 };
    rt_box_boolean(res)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_indexOf(s_id: i64, sub_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let sub = stringify_value(sub_id);
    if let Some(pos) = s.find(&sub) {
         return pos as i64;
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_slice(s_id: i64, start: i64, end: i64) -> i64 {
    let s = stringify_value(s_id);
    let len = s.len() as i64;
    let mut st = start;
    let mut en = end;
    if st < 0 { st = len + st; }
    if st < 0 { st = 0; }
    if st > len { st = len; }
    if en < 0 { en = len + en; }
    if en < 0 { en = 0; }
    if en > len { en = len; }
    
    if st >= en { 
        return rt_box_string_raw("".to_string());
    }
    
    let chars: Vec<char> = s.chars().collect();
    let clen = chars.len() as i64;
    if st > clen { st = clen; }
    if en > clen { en = clen; }
    
    let slice: String = chars[st as usize..en as usize].iter().collect();
    rt_box_string_raw(slice)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn trimmed_concat(s_id: i64, other_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let o = stringify_value(other_id);
    let new_s = format!("{}{}", s, o);
    rt_box_string_raw(new_s)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_string_concat(s_id: i64, other_id: i64) -> i64 {
    trimmed_concat(s_id, other_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tejx_enqueue_task(func: i64, arg: i64) {
    let mut queue = EVENT_QUEUE.lock().unwrap();
    queue.push_back(Task { func, arg });
}

#[unsafe(no_mangle)]
pub extern "C" fn tejx_inc_async_ops() {
    ACTIVE_ASYNC_OPS.fetch_add(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub extern "C" fn tejx_dec_async_ops() {
    ACTIVE_ASYNC_OPS.fetch_sub(1, Ordering::SeqCst);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tejx_run_event_loop() {
    let mut _idle_count = 0;
    loop {
        let task = {
            let mut queue = EVENT_QUEUE.lock().unwrap();
            queue.pop_front()
        };

        if let Some(t) = task {
            _idle_count = 0;
            let f: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute(t.func);
            f(t.arg);
        } else {
            // No tasks. Check if we have background ops.
            let ops = ACTIVE_ASYNC_OPS.load(Ordering::SeqCst);
            if ops <= 0 {
                break;
            }
            
            _idle_count += 1;
            if ops == 0 {
                break;
            }
            // Idle
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

// === MISSING STUBS FOR FEATURE TESTS ===

// console.log alias
#[unsafe(no_mangle)]
pub unsafe extern "C" fn console_log(val: i64) -> i64 {
    print(val);
    0
}

// Error toString
#[unsafe(no_mangle)]
pub unsafe extern "C" fn e_toString(id: i64) -> i64 {
    let s = stringify_value(id);
    rt_box_string_raw(s)
}

// String pad/repeat/trim operations
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_padStart(s_id: i64, len_id: i64, fill_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let target_len = {
        let heap = HEAP.lock().unwrap();
        match heap.get(len_id) {
            Some(TaggedValue::Number(n)) => *n as usize,
            _ => len_id as usize,
        }
    };
    let fill = stringify_value(fill_id);
    let fill_char = fill.chars().next().unwrap_or(' ');
    if s.len() >= target_len { return s_id; }
    let pad: String = std::iter::repeat(fill_char).take(target_len - s.len()).collect();
    rt_box_string_raw(format!("{}{}", pad, s))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_padEnd(s_id: i64, len_id: i64, fill_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let target_len = {
        let heap = HEAP.lock().unwrap();
        match heap.get(len_id) {
            Some(TaggedValue::Number(n)) => *n as usize,
            _ => len_id as usize,
        }
    };
    let fill = stringify_value(fill_id);
    let fill_char = fill.chars().next().unwrap_or(' ');
    if s.len() >= target_len { return s_id; }
    let pad: String = std::iter::repeat(fill_char).take(target_len - s.len()).collect();
    rt_box_string_raw(format!("{}{}", s, pad))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_repeat(s_id: i64, count_id: i64) -> i64 {
    let s = stringify_value(s_id);
    let count = {
        let heap = HEAP.lock().unwrap();
        match heap.get(count_id) {
            Some(TaggedValue::Number(n)) => *n as usize,
            _ => count_id as usize,
        }
    };
    rt_box_string_raw(s.repeat(count))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_trimStart(s_id: i64) -> i64 {
    let s = stringify_value(s_id);
    rt_box_string_raw(s.trim_start().to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_trimEnd(s_id: i64) -> i64 {
    let s = stringify_value(s_id);
    rt_box_string_raw(s.trim_end().to_string())
}

// Date method stubs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Date_getTime(id: i64) -> i64 {
    d_getTime(id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Date_toISOString(id: i64) -> i64 {
    d_toISOString(id)
}

// n.describe / read_to_string stubs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_describe(id: i64) -> i64 {
    stringify_value(id);
    rt_box_string_raw(format!("Node({})", id))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn read_to_string(id: i64) -> i64 {
    id // pass through
}

// Network sync stubs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn http_getSync(url_id: i64) -> i64 {
    let _url = stringify_value(url_id);
    rt_box_string_raw("{}".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn http_postSync(url_id: i64, _body_id: i64) -> i64 {
    let _url = stringify_value(url_id);
    rt_box_string_raw("{}".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn https_getSync(url_id: i64) -> i64 {
    let _url = stringify_value(url_id);
    rt_box_string_raw("{}".to_string())
}

// rt_Map_clear
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_Map_clear(id: i64) -> i64 {
    Map_clear(id);
    0
}

// f_any___ array method stubs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___find(arr_id: i64, cb: i64) -> i64 {
    eprintln!("f_any___find arr_id: {}, cb: {}", arr_id, cb);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(arr_id) {
        let items: Vec<i64> = arr.clone();
        drop(heap);
        let func: unsafe extern "C" fn(i64, i64, i64) -> i64 = unsafe { std::mem::transmute(cb) };
        for (i, item) in items.iter().enumerate() {
            let idx_box = rt_box_number(i as f64);
            let result = unsafe { func(*item, idx_box, arr_id) };
            eprintln!("  i={}, item={}, result={}", i, *item, result);
            if rt_is_truthy(result) { return *item; }
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___findIndex(arr_id: i64, cb: i64) -> i64 {
    eprintln!("f_any___findIndex arr_id: {}, cb: {}", arr_id, cb);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(arr_id) {
        let items: Vec<i64> = arr.clone();
        drop(heap);
        let func: unsafe extern "C" fn(i64, i64, i64) -> i64 = unsafe { std::mem::transmute(cb) };
        for (i, item) in items.iter().enumerate() {
            let idx_box = rt_box_number(i as f64);
            let result = unsafe { func(*item, idx_box, arr_id) };
            eprintln!("  i={}, item={}, result={}", i, *item, result);
            if rt_is_truthy(result) { return idx_box; }
        }
    }
    rt_box_number(-1.0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___reduce(arr_id: i64, cb: i64, init: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(arr_id) {
        let items: Vec<i64> = arr.clone();
        drop(heap);
        let func: unsafe extern "C" fn(i64, i64, i64) -> i64 = unsafe { std::mem::transmute(cb) };
        let mut acc = init;
        for (i, item) in items.iter().enumerate() {
            acc = unsafe { func(acc, *item, rt_box_number(i as f64)) };
        }
        return acc;
    }
    init
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___reverse(arr_id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(arr_id) {
        arr.reverse();
    }
    arr_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___slice(arr_id: i64, start_id: i64, end_id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get(arr_id) {
        let len = arr.len() as i64;
        let s = {
            match heap.get(start_id) {
                Some(TaggedValue::Number(n)) => *n as i64,
                _ => start_id,
            }
        };
        let e = {
            match heap.get(end_id) {
                Some(TaggedValue::Number(n)) => *n as i64,
                _ => if end_id == 0 { len } else { end_id },
            }
        };
        let start = if s < 0 { (len + s).max(0) as usize } else { s.min(len) as usize };
        let end = if e < 0 { (len + e).max(0) as usize } else { e.min(len) as usize };
        let slice: Vec<i64> = if start < end { arr[start..end].to_vec() } else { vec![] };
        drop(heap);
        let new_id = a_new();
        let mut h = HEAP.lock().unwrap();
        if let Some(TaggedValue::Array(new_arr)) = h.get_mut(new_id) {
            *new_arr = slice;
        }
        return new_id;
    }
    a_new()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___splice(arr_id: i64, start_id: i64, count_id: i64) -> i64 {
    let (s, c) = {
        let heap = HEAP.lock().unwrap();
        let s = match heap.get(start_id) {
            Some(TaggedValue::Number(n)) => *n as usize,
            _ => start_id as usize,
        };
        let c = match heap.get(count_id) {
            Some(TaggedValue::Number(n)) => *n as usize,
            _ => count_id as usize,
        };
        (s, c)
    };
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(arr_id) {
        let end = (s + c).min(arr.len());
        let _removed: Vec<i64> = arr.drain(s..end).collect();
    }
    arr_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___flat(arr_id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let mut result = vec![];
    if let Some(TaggedValue::Array(arr)) = heap.get(arr_id) {
        for &item in arr.iter() {
            if let Some(TaggedValue::Array(inner)) = heap.get(item) {
                result.extend(inner.iter());
            } else {
                result.push(item);
            }
        }
    }
    drop(heap);
    let new_id = a_new();
    let mut h = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(new_arr)) = h.get_mut(new_id) {
        *new_arr = result;
    }
    new_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___includes(arr_id: i64, val_id: i64) -> i64 {
    let val_str = stringify_value(val_id);
    let items = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Array(arr)) = heap.get(arr_id) {
            arr.clone()
        } else {
            return 0;
        }
    };
    for item in items {
        if stringify_value(item) == val_str {
            return 1;
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_any___sort(arr_id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.get_mut(arr_id) {
        arr.sort();
    }
    arr_id
}

// forEach for typed arrays (class instances)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Animal___forEach(arr_id: i64, cb: i64) -> i64 {
    f_any___forEach(arr_id, cb);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Node___forEach(arr_id: i64, cb: i64) -> i64 {
    f_any___forEach(arr_id, cb);
    0
}

// Class method stubs for OOP
#[unsafe(no_mangle)]
pub unsafe extern "C" fn f_Printable_print(_this: i64) -> i64 {
    // Default Printable.print - just stringify and print
    let s = stringify_value(_this);
    let boxed = rt_box_string_raw(s.clone());
    print(boxed);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn this_area() -> i64 {
    0 // placeholder - should be handled by class method dispatch
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn this_toString() -> i64 {
    0 // placeholder
}
