use std::collections::HashSet;
use std::fs;
use std::path::Path;
use crate::runtime::{stringify_value, HEAP, TaggedValue, Promise_new, __resolve_promise, a_new, Array_push};

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    // Sync APIs
    s.insert("readFileSync".to_string());
    s.insert("writeFileSync".to_string());
    s.insert("appendFileSync".to_string());
    s.insert("existsSync".to_string());
    s.insert("unlinkSync".to_string());
    s.insert("mkdirSync".to_string());
    s.insert("readdirSync".to_string());
    
    // Async APIs
    s.insert("readFile".to_string());
    s.insert("writeFile".to_string());
    
    // Legacy/Compact aliases
    s.insert("exists".to_string()); // Maps to existsSync
    s.insert("write".to_string());  // Maps to writeFileSync
    s.insert("remove".to_string()); // Maps to unlinkSync
    
    s
}

// --- Synchronous Implementations ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_readFileSync(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    match fs::read_to_string(&path) {
        Ok(content) => {
             let mut heap = HEAP.lock().unwrap();
             heap.alloc(TaggedValue::String(content))
        }
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_writeFileSync(path_id: i64, content_id: i64) -> i64 {
    let path = stringify_value(path_id);
    let content = stringify_value(content_id);
    match fs::write(&path, &content) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_appendFileSync(path_id: i64, content_id: i64) -> i64 {
    let path = stringify_value(path_id);
    let content = stringify_value(content_id);
    
    use std::io::Write;
    let mut file = match fs::OpenOptions::new().write(true).append(true).create(true).open(&path) {
        Ok(f) => f,
        Err(_) => return 0,
    };
    
    match file.write_all(content.as_bytes()) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_existsSync(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    if Path::new(&path).exists() { 1 } else { 0 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_unlinkSync(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    let p = Path::new(&path);
    if p.is_dir() {
        match fs::remove_dir_all(p) { Ok(_) => 1, Err(_) => 0 }
    } else {
        match fs::remove_file(p) { Ok(_) => 1, Err(_) => 0 }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_mkdirSync(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    match fs::create_dir_all(&path) {
        Ok(_) => 1,
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_readdirSync(path_id: i64) -> i64 {
    let path = stringify_value(path_id);
    let arr_id = a_new();
    
    if let Ok(entries) = fs::read_dir(&path) {
        for entry in entries {
            if let Ok(entry) = entry {
                if let Ok(name) = entry.file_name().into_string() {
                    let mut heap = HEAP.lock().unwrap();
                    let name_id = heap.alloc(TaggedValue::String(name));
                    drop(heap);
                    Array_push(arr_id, name_id);
                }
            }
        }
    }
    arr_id
}

// --- Async Wrappers ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_readFile(path_id: i64) -> i64 {
    let pid = Promise_new(0);
    let path = stringify_value(path_id);
    
    std::thread::spawn(move || {
        match fs::read_to_string(&path) {
            Ok(content) => {
                 let mut heap = HEAP.lock().unwrap();
                 let val_id = heap.alloc(TaggedValue::String(content));
                 drop(heap);
                 __resolve_promise(pid, val_id);
            }
            Err(_) => {
                __resolve_promise(pid, 0); 
            }
        }
    });
    pid
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_writeFile(path_id: i64, content_id: i64) -> i64 {
    let pid = Promise_new(0);
    let path = stringify_value(path_id);
    let content = stringify_value(content_id);
    
    std::thread::spawn(move || {
        match fs::write(&path, &content) {
            Ok(_) => {
                 __resolve_promise(pid, 1); // Resolve with success (1) or undefined (0)? 1 for now.
            }
            Err(_) => {
                __resolve_promise(pid, 0); 
            }
        }
    });
    pid
}


// --- Legacy/Aliases ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_exists(path_id: i64) -> i64 {
    std_fs_existsSync(path_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_write(path_id: i64, content_id: i64) -> i64 {
    std_fs_writeFileSync(path_id, content_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_remove(path_id: i64) -> i64 {
    std_fs_unlinkSync(path_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_read_to_string(path_id: i64) -> i64 {
    std_fs_readFileSync(path_id)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_fs_mkdir(path_id: i64) -> i64 {
    std_fs_mkdirSync(path_id)
}
