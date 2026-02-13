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
}

#[path = "stdlib/mod.rs"]
pub mod stdlib;
use std::thread;
use std::collections::HashMap;
use std::sync::{Mutex, Arc, Condvar, LazyLock};

#[derive(Debug, Clone)]
pub enum TaggedValue {
    Array(Vec<i64>),
    Map(HashMap<String, i64>),
    Thread(Arc<Mutex<Option<thread::JoinHandle<i64>>>>),
    Mutex(Arc<(Mutex<bool>, Condvar)>),
    Number(f64),
    Boolean(bool),
    String(String),
}

pub struct Heap {
    pub next_id: i64,
    pub objects: HashMap<i64, TaggedValue>,
}

pub static HEAP: LazyLock<Mutex<Heap>> = LazyLock::new(|| Mutex::new(Heap {
    next_id: 1000, 
    objects: HashMap::new(),
}));

#[unsafe(no_mangle)]
pub extern "C" fn tejx_hello() {
    println!("TejX Runtime Initialized");
}

#[unsafe(no_mangle)]
pub extern "C" fn Thread_new(callback: i64, arg: i64) -> i64 {
    let handle = thread::spawn(move || {
        let cb: unsafe extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(callback) };
        unsafe { cb(arg) }
    });

    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Thread(Arc::new(Mutex::new(Some(handle)))));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn Thread_join(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Thread(handle_mutex)) = heap.objects.get(&id) {
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
    heap.objects.insert(id, TaggedValue::Mutex(m));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn m_lock(id: i64) -> i64 {
    let pair = {
        let heap = HEAP.lock().unwrap();
        if let Some(TaggedValue::Mutex(pair)) = heap.objects.get(&id) {
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
        if let Some(TaggedValue::Mutex(pair)) = heap.objects.get(&id) {
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
    heap.objects.insert(id, TaggedValue::Array(Vec::new()));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn m_new() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Map(HashMap::new()));
    id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_set(id: i64, key_ptr: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get_mut(&id) {
        match obj {
            TaggedValue::Array(arr) => {
                let idx = if key_ptr < 1000 {
                    key_ptr as usize
                } else if key_ptr > 0xFFFFFFFFFFFF {
                    f64::from_bits(key_ptr as u64) as usize
                } else {
                    usize::MAX // Invalid for array index set
                };

                if idx != usize::MAX {
                    if idx >= arr.len() {
                        if idx < arr.len() + 1000 { // Limit growth prevents OOM on huge index
                             arr.resize(idx + 1, 0);
                        }
                    }
                    if idx < arr.len() {
                        arr[idx] = val;
                    }
                }
            }
            TaggedValue::Map(map) => {
                let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy().into_owned();
                map.insert(key, val);
            }
            _ => {} // Ignore others
        }
    }
    val
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_get(id: i64, key_ptr: i64) -> i64 {
    let  heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get(&id) {
        match obj {
            TaggedValue::Array(arr) => {
                let idx = if key_ptr < 1000 {
                     Some(key_ptr as usize)
                } else if key_ptr > 0xFFFFFFFFFFFF {
                     Some(f64::from_bits(key_ptr as u64) as usize)
                } else if let Some(TaggedValue::Number(n)) = heap.objects.get(&key_ptr) {
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
                // Only treat as pointer if in valid range range
                if key_ptr >= 1000 && key_ptr <= 0xFFFFFFFFFFFF && !heap.objects.contains_key(&key_ptr) {
                     // Probably a raw string pointer
                     let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy();
                     if key == "length" {
                         return (arr.len() as f64).to_bits() as i64;
                     }
                } else if let Some(TaggedValue::String(key)) = heap.objects.get(&key_ptr) {
                     if key == "length" {
                         return (arr.len() as f64).to_bits() as i64;
                     }
                }
            }
            TaggedValue::Map(map) => {
                let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy().into_owned();
                
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
    if let Some(obj) = heap.objects.get(&id) {
        return match obj {
            TaggedValue::Array(arr) => {
                let ids = arr.clone();
                drop(heap);
                let mut parts = Vec::new();
                for val_id in ids {
                    parts.push(stringify_value(val_id));
                }
                format!("[{}]", parts.join(", "))
            }
            TaggedValue::Map(_) => "[Object]".to_string(),
            TaggedValue::Thread(_) => "[Thread]".to_string(),
            TaggedValue::Mutex(_) => "[Mutex]".to_string(),
            TaggedValue::Number(n) => {
                if n.fract() == 0.0 { format!("{:.0}", n) } else { format!("{}", n) }
            },
            TaggedValue::Boolean(b) => format!("{}", b),
            TaggedValue::String(s) => s.clone(),
        };
    }
    drop(heap);
    
    if id == 0 { return "0".to_string(); }
    
    // Heuristic: Small IDs are literals, large IDs are pointers or bitcasted doubles
    if id > -1000 && id < 1000 {
        return format!("{}", id);
    }
    
    // Pointer fallback: Only if it's in a known valid pointer range for macOS literal strings
    // AND it doesn't look like a typical bitcasted double (e.g. starting with 0x40...)
    if id > 0x100000000 && id < 0x7FFFFFFFFFFF && (id >> 60) == 0 {
        unsafe {
            let c_str = CStr::from_ptr(id as *const c_char);
            if let Ok(s) = c_str.to_str() {
                // Sanity check: is it printable?
                if s.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
                    return s.to_string();
                }
            }
        }
    }
    
    // Assume bitcasted double
    let f = f64::from_bits(id as u64);
    if f.is_nan() || f.is_infinite() {
        return format!("{}", id);
    }
    format!("{}", f)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __callee___toString(id: i64) -> i64 {
    let s = stringify_value(id);
    let c_str = CString::new(s).unwrap();
    c_str.into_raw() as i64
}

// ... OOP Stubs ...

// Testing Mocks
#[unsafe(no_mangle)] pub extern "C" fn add(a: i64, b: i64) -> i64 { a + b }
#[unsafe(no_mangle)] pub extern "C" fn multiply(a: i64, b: i64) -> i64 { a * b }
#[unsafe(no_mangle)] pub extern "C" fn hello() -> i64 {
    println!("Hello from runtime.rs!");
    0
}
#[unsafe(no_mangle)] pub extern "C" fn calc_add(a: i64, b: i64) -> i64 { a + b }
#[unsafe(no_mangle)]
pub extern "C" fn Array_push(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get_mut(&id) {
        arr.push(val);
        return arr.len() as i64;
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn Array_pop(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get_mut(&id) {
        return arr.pop().unwrap_or(0);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Array_join(id: i64, sep_ptr: i64) -> i64 {
    let  heap = HEAP.lock().unwrap();
    let sep = if sep_ptr != 0 {
        CStr::from_ptr(sep_ptr as *const c_char).to_string_lossy().into_owned()
    } else {
        ",".to_string()
    };
    
    if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) {
        let joined = arr.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&sep);
        return CString::new(joined).unwrap().into_raw() as i64;
    }
    0
}

// Date_now moved to stdlib/time.rs

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn d_getTime(d: i64) -> i64 { d }

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn d_toISOString(_d: i64) -> i64 {
    CString::new("2023-01-01T00:00:00.000Z").unwrap().into_raw() as i64
}

#[unsafe(export_name = "Some")]
pub extern "C" fn rt_Some(val: i64) -> i64 { val } // Stub: Just return value

#[unsafe(export_name = "None")]
pub extern "C" fn rt_None() -> i64 { 0 } // Stub: Null

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_typeof(val: i64) -> i64 {
    // Very simplified typeof
    if val == 0 {
        return CString::new("undefined").unwrap().into_raw() as i64;
    }
    if val < 1000000 {
        return CString::new("number").unwrap().into_raw() as i64;
    }
    CString::new("object").unwrap().into_raw() as i64
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
        matches!(heap.objects.get(&a), Some(TaggedValue::String(_))) || 
        matches!(heap.objects.get(&b), Some(TaggedValue::String(_))) ||
        (a > 4294967296) || (b > 4294967296) // Simple heuristic for raw string pointers
    };

    if is_str {
        return rt_str_concat_v2(a, b);
    }

    rt_box_number((l_f + r_f).to_bits() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_sub(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    rt_box_number((l_f - r_f).to_bits() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_mul(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    rt_box_number((l_f * r_f).to_bits() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_div(a: i64, b: i64) -> i64 {
    let l_f = rt_to_number(a);
    let r_f = rt_to_number(b);
    if r_f == 0.0 { return rt_box_number(f64::INFINITY.to_bits() as i64); }
    rt_box_number((l_f / r_f).to_bits() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_box_number(bits: i64) -> i64 {
    // If bits look like a small integer (not a float bit pattern), handle as literal
    let n = if bits >= 0 && bits < 1000 {
        bits as f64
    } else {
        f64::from_bits(bits as u64)
    };
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Number(n));
    id
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_box_boolean(b: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Boolean(b != 0));
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
    heap.objects.insert(id, TaggedValue::String(s));
    id
}

pub fn rt_to_number(id: i64) -> f64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get(&id) {
        match obj {
            TaggedValue::Number(n) => *n,
            TaggedValue::Boolean(b) => if *b { 1.0 } else { 0.0 },
            TaggedValue::String(s) => s.parse::<f64>().unwrap_or(0.0),
            _ => 0.0
        }
    } else {
        // literal bits?
        if id == 0 { return 0.0; }
        if id > -1000 && id < 1000 { 
            return id as f64; 
        }
        f64::from_bits(id as u64)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_str_concat_v2(a: i64, b: i64) -> i64 {
    let sa = __callee___toString(a);
    let sb = __callee___toString(b);
    let s_a = CStr::from_ptr(sa as *const c_char).to_string_lossy();
    let s_b = CStr::from_ptr(sb as *const c_char).to_string_lossy();
    let joined = format!("{}{}", s_a, s_b);
    CString::new(joined).unwrap().into_raw() as i64
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
pub unsafe extern "C" fn rt_to_boolean(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get(&id) {
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
pub unsafe extern "C" fn rt_eq(a: i64, b: i64) -> i64 {
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
        if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) { arr.clone() } else { vec![id] }
    };
    new_arr.extend(extract(id_a));
    new_arr.extend(extract(id_b));
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Array(new_arr));
    id
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_indexOf(id: i64, val: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) {
         for (i, &v) in arr.iter().enumerate() {
             if v == val { return i as i64; }
         }
    }
    -1
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_shift(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get_mut(&id) {
        if !arr.is_empty() { return arr.remove(0); }
    }
    0
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_unshift(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get_mut(&id) {
        arr.insert(0, val);
        return arr.len() as i64;
    }
    0
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_forEach(id: i64, callback: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) {
         let elements = arr.clone();
         drop(heap);
         let cb: extern "C" fn(i64, i64, i64) = std::mem::transmute(callback);
         for (i, &val) in elements.iter().enumerate() { cb(val, i as i64, id); }
         return 0;
    }
    0
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_map(id: i64, callback: i64) -> i64 {
    let mut new_arr = Vec::new();
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) {
         let elements = arr.clone();
         drop(heap);
         let cb: extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(callback);
         for (i, &val) in elements.iter().enumerate() { new_arr.push(cb(val, i as i64, id)); }
    } else { drop(heap); }
    let mut heap = HEAP.lock().unwrap();
    let new_id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(new_id, TaggedValue::Array(new_arr));
    new_id
}

#[unsafe(no_mangle)] 
pub unsafe extern "C" fn Array_filter(id: i64, callback: i64) -> i64 {
    let mut new_arr = Vec::new();
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) {
         let elements = arr.clone();
         drop(heap);
         let cb: extern "C" fn(i64, i64, i64) -> i64 = std::mem::transmute(callback);
         for (i, &val) in elements.iter().enumerate() { if cb(val, i as i64, id) != 0 { new_arr.push(val); } }
    } else { drop(heap); }
    let mut heap = HEAP.lock().unwrap();
    let new_id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(new_id, TaggedValue::Array(new_arr));
    new_id
}

#[unsafe(no_mangle)]
pub extern "C" fn Array_sliceRest(id: i64, start: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get(&id) {
        let len = arr.len() as i64;
        let s = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
        let new_arr = arr[s..].to_vec();
        let new_id = heap.next_id;
        heap.next_id += 1;
        heap.objects.insert(new_id, TaggedValue::Array(new_arr));
        return new_id;
    }
    let new_id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(new_id, TaggedValue::Array(Vec::new()));
    new_id
}

#[unsafe(no_mangle)] pub extern "C" fn calc_getValue(a: i64) -> i64 { a }

// Link with the compiled TejX code

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __join(obj: i64, arg: i64) -> i64 {
    // Check if obj is in heap as Array
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(_)) = heap.objects.get(&obj) {
        drop(heap); 
        return Array_join(obj, arg);
    }
    drop(heap);
    
    // Assume thread or other
    Thread_join(obj)
}

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
        println!("{} Point Stub", p);
    } else {
        println!("Point Stub");
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

// Parsing
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parseInt(s: i64) -> i64 {
    let s_str = CStr::from_ptr(s as *const c_char).to_string_lossy();
    s_str.parse::<i64>().unwrap_or(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn parseFloat(s: i64) -> i64 {
    let s_str = CStr::from_ptr(s as *const c_char).to_string_lossy();
    let f_val = s_str.parse::<f64>().unwrap_or(0.0);
    rt_box_number(f_val.to_bits() as i64)
}

// JSON Stubs
#[unsafe(no_mangle)]
pub unsafe extern "C" fn JSON_stringify(id: i64) -> i64 {
    let s = stringify_json_recursive(id);
    let c_str = CString::new(s).unwrap();
    c_str.into_raw() as i64
}

fn stringify_json_recursive(id: i64) -> String {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get(&id) {
         match obj {
             TaggedValue::Map(map) => {
                 let entries: Vec<(String, i64)> = map.iter().map(|(k, v)| {
                     (k.clone(), *v)
                 }).collect();
                 drop(heap);
                 
                 let parts: Vec<String> = entries.iter().map(|(k, v)| {
                     format!("\"{}\":{}", k, stringify_json_recursive(*v))
                 }).collect();
                 let res = format!("{{{}}}", parts.join(","));
                 res
             }
             TaggedValue::Array(arr) => {
                 let elements = arr.clone();
                 drop(heap);
                 let parts: Vec<String> = elements.iter().map(|v| stringify_json_recursive(*v)).collect();
                 format!("[{}]", parts.join(","))
             }
             TaggedValue::String(s) => format!("\"{}\"", s), // Quote strings
             TaggedValue::Number(n) => n.to_string(),
             TaggedValue::Boolean(b) => b.to_string(),
             _ => "null".to_string()
         }
    } else {
        drop(heap);
        if id == 0 { return "null".to_string(); }
        // Literal number/bool support if needed, but ID based system usually boxes everything for objects
         if id < 1000 && id > -1000 { return id.to_string(); } // Primitive/Lit
         // fallback
         stringify_value(id)
    }
}
#[unsafe(no_mangle)] pub extern "C" fn JSON_parse(_str: i64) -> i64 { 0 }

// Map & Set stubs -> Real Implementation
#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_has(id: i64, key_ptr: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.objects.get(&id) {
        let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy();
        if map.contains_key(&*key) { 1 } else { 0 }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_del(id: i64, key_ptr: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Map(map)) = heap.objects.get_mut(&id) {
        let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy();
        map.remove(&*key);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn m_size(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get(&id) {
        match obj {
            TaggedValue::Map(map) => map.len() as f64,
            TaggedValue::Array(arr) => arr.len() as f64,
            TaggedValue::String(s) => s.len() as f64,
            _ => 0.0,
        }.to_bits() as i64
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_add(_this: i64, _v: i64) -> i64 { 0 } // Set heap support missing in TaggedValue?
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_has(_this: i64, _v: i64) -> i64 { 0 }
#[unsafe(no_mangle)]
pub unsafe extern "C" fn s_size(_this: i64) -> i64 { 0 }

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
    let s = stringify_value(id);
    println!("{}", s);
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
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn print_newline() {
    println!();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint(id: i64) {
    let s = stringify_value(id);
    eprintln!("{}", s);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint_raw(id: i64) {
    let s = stringify_value(id);
    eprint!("{}", s);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint_space() {
    eprint!(" ");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn eprint_newline() {
    eprintln!();
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
    if let Some(obj) = heap.objects.get(&id) {
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
    rt_box_number(n.abs().to_bits() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn min(a: i64, b: i64) -> i64 {
    let na = rt_to_number(a);
    let nb = rt_to_number(b);
    rt_box_number(na.min(nb).to_bits() as i64)
}

#[unsafe(no_mangle)]
pub extern "C" fn max(a: i64, b: i64) -> i64 {
    let na = rt_to_number(a);
    let nb = rt_to_number(b);
    rt_box_number(na.max(nb).to_bits() as i64)
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

#[cfg(runtime_build)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn main(_argc: i32, _argv: *const *const c_char) -> i32 {
    tejx_main() as i32
}
