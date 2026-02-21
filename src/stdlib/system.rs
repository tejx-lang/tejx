use std::collections::HashSet;
use std::env;
use std::ffi::CString;
use crate::runtime::stringify_value;

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("args".to_string());
    s.insert("exit".to_string());
    s.insert("env".to_string());
    s.insert("argv".to_string());
    s.insert("os".to_string());
    s.insert("system".to_string());
    s
}

#[unsafe(no_mangle)]
pub extern "C" fn std_system_exit(code: i64) -> i64 {
    std::process::exit(code as i32);
}

#[unsafe(no_mangle)]
pub extern "C" fn std_system_args() -> i64 {
    let args: Vec<String> = std::env::args().collect();
    let mut heap = crate::runtime::HEAP.lock().unwrap();
    let mut arr_ids = Vec::new();
    
    for arg in args {
        if let Some(&id) = heap.strings.get(&arg) {
            arr_ids.push(id);
        } else {
             let id = heap.next_id;
             heap.next_id += 1;
             heap.insert(id, crate::runtime::TaggedValue::String(arg.clone()));
             heap.strings.insert(arg, id);
             arr_ids.push(id);
        }
    }
    
    let arr_id = heap.next_id;
    heap.next_id += 1;
    heap.insert(arr_id, crate::runtime::TaggedValue::Array(arr_ids));
    arr_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_system_env(key_id: i64) -> i64 {
    let key = stringify_value(key_id);
    if let Ok(val) = env::var(&key) {
         let c_str = CString::new(val).unwrap();
         c_str.into_raw() as i64
    } else {
        0
    }
}

// Legacy aliases (previously in os.rs)
#[unsafe(no_mangle)]
pub extern "C" fn std_os_exit(code: i64) -> i64 {
    std_system_exit(code)
}

#[unsafe(no_mangle)]
pub extern "C" fn std_os_args() -> i64 {
    std_system_args()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_os_env(key_id: i64) -> i64 {
    std_system_env(key_id)
}
