use std::collections::HashSet;
use std::env;
use std::ffi::CString;
use crate::runtime::{rt_box_string, stringify_value};

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("args".to_string());
    s.insert("exit".to_string());
    s.insert("env".to_string());
    s
}

#[unsafe(no_mangle)]
pub extern "C" fn std_os_exit(code: i64) -> i64 {
    std::process::exit(code as i32);
}

#[unsafe(no_mangle)]
pub extern "C" fn std_os_args() -> i64 {
    // Stub implementation returning count
    (std::env::args().len() as f64).to_bits() as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_os_env(key_id: i64) -> i64 {
    let key = stringify_value(key_id);
    if let Ok(val) = env::var(&key) {
         let c_str = CString::new(val).unwrap();
         c_str.into_raw() as i64
    } else {
        0 // undefined/null
    }
}
