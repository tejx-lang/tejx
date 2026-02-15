use std::collections::HashSet;
use std::ffi::CString;
use std::fs;
use std::path::Path;
use crate::runtime::stringify_value;

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("exists".to_string());
    s.insert("read_to_string".to_string());
    s.insert("write".to_string());
    s.insert("remove".to_string());
    s.insert("mkdir".to_string());
    s.insert("readFile".to_string()); // Legacy
    s.insert("writeFile".to_string()); // Legacy
    s
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_exists(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    if Path::new(&path).exists() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_read_to_string(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    match fs::read_to_string(&path) {
        Ok(content) => {
             let c_str = CString::new(content).unwrap();
             c_str.into_raw() as i64
        }
        Err(_) => 0,
    }
}

// Alias for legacy support if needed (or just implemented same way)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_readFile(path_id: i64) -> i64 {
    unsafe { std_fs_read_to_string(path_id) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_write(path_id: i64, content_id: i64) -> i64 {
    let path = stringify_value(path_id);
    let content = stringify_value(content_id);
    match fs::write(&path, &content) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_writeFile(path_id: i64, content_id: i64) -> i64 {
    unsafe { std_fs_write(path_id, content_id) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_remove(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    let p = Path::new(&path);
    if p.is_dir() {
        match fs::remove_dir_all(p) { Ok(_) => 0, Err(_) => -1 }
    } else {
        match fs::remove_file(p) { Ok(_) => 0, Err(_) => -1 }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_mkdir(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    match fs::create_dir_all(&path) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
