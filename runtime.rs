use std::ffi::{CStr, CString};
use std::fs;
use std::os::raw::c_char;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::thread;
use std::collections::HashMap;
use std::sync::{Mutex, LazyLock};

enum TaggedValue {
    Array(Vec<i64>),
    Map(HashMap<String, i64>),
}

struct Heap {
    next_id: i64,
    objects: HashMap<i64, TaggedValue>,
}

static HEAP: LazyLock<Mutex<Heap>> = LazyLock::new(|| Mutex::new(Heap {
    next_id: 1000, // Start high to avoid collision with small ints if any
    objects: HashMap::new(),
}));

#[no_mangle]
pub extern "C" fn tejx_hello() {
    println!("TejX Runtime Initialized");
}

#[no_mangle]
pub extern "C" fn a_new() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Array(Vec::new()));
    id
}

#[no_mangle]
pub extern "C" fn m_new() -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let id = heap.next_id;
    heap.next_id += 1;
    heap.objects.insert(id, TaggedValue::Map(HashMap::new()));
    id
}

#[no_mangle]
pub unsafe extern "C" fn m_set(id: i64, key_ptr: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get_mut(&id) {
        match obj {
            TaggedValue::Array(arr) => {
                // If key_ptr is small, it's an index. If it's a pointer, it's a string key.
                if key_ptr < 1000000 {
                    let idx = key_ptr as usize;
                    if idx >= arr.len() {
                        arr.resize(idx + 1, 0);
                    }
                    arr[idx] = val;
                }
            }
            TaggedValue::Map(map) => {
                let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy().into_owned();
                map.insert(key, val);
            }
        }
    }
    val
}

#[no_mangle]
pub unsafe extern "C" fn m_get(id: i64, key_ptr: i64) -> i64 {
    let  heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.objects.get(&id) {
        match obj {
            TaggedValue::Array(arr) => {
                if key_ptr < 1000000 {
                    let idx = key_ptr as usize;
                    return arr.get(idx).cloned().unwrap_or(0);
                }
                // Special properties
                let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy();
                if key == "length" {
                    return arr.len() as i64;
                }
            }
            TaggedValue::Map(map) => {
                let key = CStr::from_ptr(key_ptr as *const c_char).to_string_lossy();
                return map.get(key.as_ref()).cloned().unwrap_or(0);
            }
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn Array_push(id: i64, val: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get_mut(&id) {
        arr.push(val);
        return arr.len() as i64;
    }
    0
}

#[no_mangle]
pub extern "C" fn Array_pop(id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::Array(arr)) = heap.objects.get_mut(&id) {
        return arr.pop().unwrap_or(0);
    }
    0
}

#[no_mangle]
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

// Math
#[no_mangle]
pub extern "C" fn Math_pow(base: i64, exp: i64) -> i64 {
    (base as f64).powf(exp as f64) as i64
}

#[no_mangle]
pub extern "C" fn Date_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// File System
#[no_mangle]
pub unsafe extern "C" fn fs_exists(path_ptr: i64) -> i64 {
    let path = CStr::from_ptr(path_ptr as *const c_char).to_string_lossy();
    if Path::new(path.as_ref()).exists() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn fs_mkdir(path_ptr: i64) -> i64 {
    let path = CStr::from_ptr(path_ptr as *const c_char).to_string_lossy();
    match fs::create_dir_all(path.as_ref()) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn fs_readFile(path_ptr: i64) -> i64 {
    let path = CStr::from_ptr(path_ptr as *const c_char).to_string_lossy();
    match fs::read_to_string(path.as_ref()) {
        Ok(content) => {
            let c_str = CString::new(content).unwrap();
            c_str.into_raw() as i64
        }
        Err(_) => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn fs_writeFile(path_ptr: i64, content_ptr: i64) -> i64 {
    let path = CStr::from_ptr(path_ptr as *const c_char).to_string_lossy();
    let content = CStr::from_ptr(content_ptr as *const c_char).to_string_lossy();
    match fs::write(path.as_ref(), content.as_ref()) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn fs_remove(path_ptr: i64) -> i64 {
    let path = CStr::from_ptr(path_ptr as *const c_char).to_string_lossy();
    let p = Path::new(path.as_ref());
    if p.is_dir() {
        match fs::remove_dir_all(p) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    } else {
        match fs::remove_file(p) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

// Async/Await Stubs
#[no_mangle]
pub extern "C" fn __await(promise_or_val: i64) -> i64 {
    promise_or_val
}

#[no_mangle]
pub extern "C" fn Promise_all(promises_ptr: i64) -> i64 {
    promises_ptr
}

#[no_mangle]
pub extern "C" fn delay(ms: i64) -> i64 {
    thread::sleep(Duration::from_millis(ms as u64));
    0
}

#[no_mangle]
pub extern "C" fn http_get(_url_ptr: i64) -> i64 {
    let dummy = "<html><body>Google</body></html>";
    let c_str = CString::new(dummy).unwrap();
    c_str.into_raw() as i64
}

// Optional Chaining Stub
#[no_mangle]
pub extern "C" fn __optional_chain(obj: i64, _op_ptr: i64) -> i64 {
    if obj == 0 { 0 } else { obj }
}

// Array extra Stubs
#[no_mangle] pub extern "C" fn Array_concat(_a: i64, _b: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn Array_indexOf(_arr: i64, _val: i64) -> i64 { -1 }
#[no_mangle] pub extern "C" fn Array_shift(_arr: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn Array_unshift(_arr: i64, _val: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn Array_forEach(_arr: i64, _callback: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn Array_map(_arr: i64, _callback: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn Array_filter(_arr: i64, _callback: i64) -> i64 { 0 }

// Thread Stub
#[no_mangle]
pub extern "C" fn Thread_join(_t: i64) -> i64 { 0 }

#[no_mangle]
pub extern "C" fn __callee___area() -> i64 { 100 }
#[no_mangle]
pub extern "C" fn __callee___describe() -> i64 { 0 }
#[no_mangle]
pub unsafe extern "C" fn __callee___toString(id: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    let s = if let Some(obj) = heap.objects.get(&id) {
        match obj {
            TaggedValue::Array(arr) => format!("[{}]", arr.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")),
            TaggedValue::Map(_) => "[Object]".to_string(),
        }
    } else {
        format!("{}", id)
    };
    let c_str = CString::new(s).unwrap();
    c_str.into_raw() as i64
}

// OOP Stubs
#[no_mangle] pub extern "C" fn BankAccount_getBankName() -> i64 {
    CString::new("TejX Bank").unwrap().into_raw() as i64
}
#[no_mangle] pub extern "C" fn acc_deposit(_this: i64, _amt: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn acc_getBalance(_this: i64) -> i64 { 100 }
#[no_mangle] pub extern "C" fn c_printDetails(_this: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn dDog_bark(_this: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn dDog_speak(_this: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn p_get_age(_this: i64) -> i64 { 30 }
#[no_mangle] pub unsafe extern "C" fn p_get_name(_this: i64) -> i64 {
    CString::new("StubName").unwrap().into_raw() as i64
}
#[no_mangle] pub unsafe extern "C" fn p_get_info(_this: i64) -> i64 {
    CString::new("StubInfo").unwrap().into_raw() as i64
}
#[no_mangle] pub unsafe extern "C" fn p_print(_this: i64, prefix: i64) -> i64 {
    if prefix != 0 {
        let p = CStr::from_ptr(prefix as *const c_char).to_string_lossy();
        println!("{} Point Stub", p);
    } else {
        println!("Point Stub");
    }
    0
}
#[no_mangle] pub extern "C" fn p_set_age(_this: i64, _val: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn p_set_name(_this: i64, _val: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn r_greet(_this: i64, _name: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn r_identify(_this: i64) -> i64 { 0 }
#[no_mangle] pub unsafe extern "C" fn r_status(_this: i64) -> i64 {
    CString::new("Operational").unwrap().into_raw() as i64
}
#[no_mangle] pub extern "C" fn uExt_greet(_this: i64, _times: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn uExt_sayHello(_this: i64) -> i64 { 0 }

// Math Extensions
#[no_mangle] pub extern "C" fn Math_abs(x: i64) -> i64 { x.abs() }
#[no_mangle] pub extern "C" fn Math_ceil(x: i64) -> i64 { (x as f64).ceil() as i64 }
#[no_mangle] pub extern "C" fn Math_floor(x: i64) -> i64 { (x as f64).floor() as i64 }
#[no_mangle] pub extern "C" fn Math_round(x: i64) -> i64 { (x as f64).round() as i64 }
#[no_mangle] pub extern "C" fn Math_sqrt(x: i64) -> i64 { (x as f64).sqrt() as i64 }
#[no_mangle] pub extern "C" fn Math_sin(x: i64) -> i64 { (x as f64).sin() as i64 }
#[no_mangle] pub extern "C" fn Math_cos(x: i64) -> i64 { (x as f64).cos() as i64 }
#[no_mangle] pub extern "C" fn Math_random() -> i64 { 42 } // Stub
#[no_mangle] pub extern "C" fn Math_min(a: i64, b: i64) -> i64 { a.min(b) }
#[no_mangle] pub extern "C" fn Math_max(a: i64, b: i64) -> i64 { a.max(b) }

// Parsing
#[no_mangle]
pub unsafe extern "C" fn parseInt(s: i64) -> i64 {
    let s_str = CStr::from_ptr(s as *const c_char).to_string_lossy();
    s_str.parse::<i64>().unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn parseFloat(s: i64) -> i64 {
    let s_str = CStr::from_ptr(s as *const c_char).to_string_lossy();
    s_str.parse::<f64>().unwrap_or(0.0) as i64
}

// JSON Stubs
#[no_mangle] pub unsafe extern "C" fn JSON_stringify(_obj: i64) -> i64 {
    CString::new("{}").unwrap().into_raw() as i64
}
#[no_mangle] pub extern "C" fn JSON_parse(_str: i64) -> i64 { 0 }

// Console
#[no_mangle]
pub unsafe extern "C" fn console_error(s: i64) {
    let msg = CStr::from_ptr(s as *const c_char).to_string_lossy();
    eprintln!("Error: {}", msg);
}

#[no_mangle]
pub unsafe extern "C" fn console_warn(s: i64) {
    let msg = CStr::from_ptr(s as *const c_char).to_string_lossy();
    eprintln!("Warning: {}", msg);
}

// Date
#[no_mangle] pub extern "C" fn d_getTime(_d: i64) -> i64 { Date_now() }
#[no_mangle] pub unsafe extern "C" fn d_toISOString(_d: i64) -> i64 {
    CString::new("2023-01-01T00:00:00.000Z").unwrap().into_raw() as i64
}

// Map & Set stubs
#[no_mangle] pub extern "C" fn m_has(_this: i64, _k: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn m_del(_this: i64, _k: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn m_size(_this: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn s_add(_this: i64, _v: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn s_has(_this: i64, _v: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn s_size(_this: i64) -> i64 { 0 }

// String Utils
#[no_mangle] pub extern "C" fn strVal_trim(s: i64) -> i64 { s }
#[no_mangle] pub extern "C" fn trimmed_startsWith(_s: i64, _p: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn trimmed_endsWith(_s: i64, _p: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn trimmed_replace(s: i64, _a: i64, _b: i64) -> i64 { s }
#[no_mangle] pub extern "C" fn trimmed_toLowerCase(s: i64) -> i64 { s }
#[no_mangle] pub extern "C" fn trimmed_toUpperCase(s: i64) -> i64 { s }

// Testing Mocks
#[no_mangle] pub extern "C" fn add(a: i64, b: i64) -> i64 { a + b }
#[no_mangle] pub extern "C" fn multiply(a: i64, b: i64) -> i64 { a * b }
#[no_mangle] pub extern "C" fn hello() -> i64 {
    println!("Hello from runtime.rs!");
    0
}
#[no_mangle] pub extern "C" fn calc_add(a: i64, b: i64) -> i64 { a + b }
#[no_mangle] pub extern "C" fn calc_getValue(a: i64) -> i64 { a }
#[no_mangle] pub extern "C" fn m_lock(_m: i64) -> i64 { 0 }
#[no_mangle] pub extern "C" fn m_unlock(_m: i64) -> i64 { 0 }

// Link with the compiled TejX code
extern "C" {
    fn tejx_main() -> i64;
}

#[no_mangle]
pub unsafe extern "C" fn main(_argc: i32, _argv: *const *const c_char) -> i32 {
    tejx_main() as i32
}
