pub mod array;
pub use array::*;
pub mod atomic;
pub use atomic::*;
pub mod condition;
pub use condition::*;
pub mod internal_http;
pub use internal_http::*;
pub mod math;
pub use math::*;
pub mod mutex;
pub use mutex::*;
pub mod net;
pub use net::*;
pub mod object;
pub use object::*;
pub mod promise;
pub use promise::*;
pub mod queue;
pub use queue::*;
pub mod string;
pub use string::*;
pub mod thread;
pub use thread::*;

#[path = "../gc.rs"]
pub mod gc;
pub use gc::{
    gc_allocate, rt_get_header, rt_init_gc, rt_is_gc_ptr, rt_pop_roots, rt_push_root,
    rt_register_thread, rt_unregister_thread, rt_write_barrier, ObjectHeader,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

pub unsafe fn rt_throw_runtime_error(msg: &str) -> ! {
    let cstr = std::ffi::CString::new(msg).unwrap_or_else(|_| {
        std::ffi::CString::new("RuntimeError").expect("CString for RuntimeError")
    });
    let msg_id = rt_string_from_c_str(cstr.as_ptr());
    crate::event_loop::tejx_throw(msg_id);
    std::hint::unreachable_unchecked();
}

#[no_mangle]
pub static HEAP_OFFSET: i64 = 1i64 << 50;
#[no_mangle]
pub static STACK_OFFSET: i64 = 1i64 << 48;

#[inline]
pub unsafe fn rt_obj_ptr(val: i64) -> *mut i64 {
    if val >= HEAP_OFFSET {
        (val - HEAP_OFFSET) as *mut i64
    } else if val >= STACK_OFFSET {
        (val - STACK_OFFSET) as *mut i64
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub static TAG_BOOLEAN: i64 = 1;
#[no_mangle]
pub static TAG_STRING: i64 = 2;
#[no_mangle]
pub static TAG_ARRAY: i64 = 3;
#[no_mangle]
pub static TAG_CHAR: i64 = 4;
#[no_mangle]
pub static TAG_INT: i64 = 5;
#[no_mangle]
pub static TAG_FLOAT: i64 = 6;
#[no_mangle]
pub static TAG_OBJECT: i64 = 7;
#[no_mangle]
pub static TAG_FUNCTION: i64 = 8;
#[no_mangle]
pub static TAG_PROMISE: i64 = 10;
#[no_mangle]
pub static TAG_RAW_DATA: i64 = 11;
#[no_mangle]
pub static TAG_MAP: i64 = 12;

// --- Object Layout Constants ---
pub const OBJECT_SIZE_OFFSET: isize = 0;
pub const OBJECT_CAP_OFFSET: isize = 8;
pub const OBJECT_KEYS_OFFSET: isize = 16;
pub const OBJECT_VALUES_OFFSET: isize = 24;

pub const MAP_SIZE_OFFSET: isize = 0;
pub const MAP_CAP_OFFSET: isize = 8;
pub const MAP_KEYS_OFFSET: isize = 16;
pub const MAP_VALUES_OFFSET: isize = 24;
pub const MAP_STATES_OFFSET: isize = 32;

pub const ARRAY_FLAG_FIXED: i64 = 0x0100;
pub const ARRAY_FLAG_CONSTANT: i64 = 0x0200;
// Boolean sentinels (below HEAP_OFFSET, above normal number range)
#[no_mangle]
pub static BOOL_FALSE: i64 = 0;
#[no_mangle]
pub static BOOL_TRUE: i64 = 1;

#[no_mangle]
pub static mut LAST_ID: i64 = 0;
#[no_mangle]
pub static mut LAST_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut LAST_LEN: i64 = 0;
#[no_mangle]
pub static mut LAST_ELEM_SIZE: i64 = 0;

#[no_mangle]
pub static mut PREV_ID: i64 = 0;
#[no_mangle]
pub static mut PREV_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut PREV_LEN: i64 = 0;
#[no_mangle]
pub static mut PREV_ELEM_SIZE: i64 = 0;

#[no_mangle]
pub unsafe extern "C" fn rt_invalidate_array_cache(id: i64) {
    if LAST_ID == id {
        LAST_ID = 0;
    }
    if PREV_ID == id {
        PREV_ID = 0;
    }
    if PREV2_ID == id {
        PREV2_ID = 0;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_update_array_cache(id: i64, data: *mut u8, len: i64, elem_size: i64) {
    if LAST_ID == id {
        LAST_PTR = data;
        LAST_LEN = len;
        LAST_ELEM_SIZE = elem_size;
        return;
    }
    PREV2_ID = PREV_ID;
    PREV2_PTR = PREV_PTR;
    PREV2_LEN = PREV_LEN;
    PREV2_ELEM_SIZE = PREV_ELEM_SIZE;

    PREV_ID = LAST_ID;
    PREV_PTR = LAST_PTR;
    PREV_LEN = LAST_LEN;
    PREV_ELEM_SIZE = LAST_ELEM_SIZE;

    LAST_ID = id;
    LAST_PTR = data;
    LAST_LEN = len;
    LAST_ELEM_SIZE = elem_size;
}

#[no_mangle]
pub static mut PREV2_ID: i64 = 0;
#[no_mangle]
pub static mut PREV2_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut PREV2_LEN: i64 = 0;
#[no_mangle]
pub static mut PREV2_ELEM_SIZE: i64 = 0;

static ARRAY_FORWARD: LazyLock<Mutex<HashMap<i64, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ARRAY_FORWARD_ACTIVE: AtomicBool = AtomicBool::new(false);

#[inline]
pub unsafe fn rt_resolve_array_id(mut id: i64) -> i64 {
    if id < HEAP_OFFSET {
        return id;
    }
    if !ARRAY_FORWARD_ACTIVE.load(Ordering::Acquire) {
        return id;
    }
    let map = ARRAY_FORWARD.lock().unwrap();
    while let Some(next) = map.get(&id) {
        if *next == id {
            break;
        }
        id = *next;
    }
    id
}

extern "C" {
    pub fn malloc(size: usize) -> *mut std::ffi::c_void;
    pub fn calloc(num: usize, size: usize) -> *mut std::ffi::c_void;
    pub fn free(p: *mut std::ffi::c_void);
    pub fn realloc(ptr: *mut std::ffi::c_void, size: usize) -> *mut std::ffi::c_void;
    pub fn strlen(s: *const std::ffi::c_char) -> usize;
    pub fn memcpy(
        dest: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
        n: usize,
    ) -> *mut std::ffi::c_void;
    pub fn memcmp(s1: *const std::ffi::c_void, s2: *const std::ffi::c_void, n: usize) -> i32;
    pub fn printf(fmt: *const std::ffi::c_char, ...) -> i32;
    pub fn sprintf(str: *mut std::ffi::c_char, fmt: *const std::ffi::c_char, ...) -> i32;
    pub fn atof(s: *const std::ffi::c_char) -> f64;
    pub fn exit(code: i32) -> !;
    pub fn write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
    pub fn mmap(
        addr: *mut std::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: isize,
    ) -> *mut std::ffi::c_void;
    pub fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
    pub fn memset(ptr: *mut std::ffi::c_void, value: i32, num: usize) -> *mut std::ffi::c_void;
    pub fn fflush(stream: *mut std::ffi::c_void) -> i32;
    pub fn memmove(
        dest: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
        n: usize,
    ) -> *mut std::ffi::c_void;
}

pub const PROT_READ: i32 = 0x1;
pub const PROT_WRITE: i32 = 0x2;
pub const MAP_PRIVATE: i32 = 0x02;
pub const MAP_ANON: i32 = 0x1000;

#[repr(C)]
pub struct Slice {
    pub ptr: i64,
    pub len: i64,
}

#[no_mangle]
pub unsafe fn rt_get_ptr(val: i64) -> *mut i64 {
    if val < HEAP_OFFSET {
        return std::ptr::null_mut();
    }
    (val - HEAP_OFFSET) as *mut i64
}

// --- Conversions ---

#[no_mangle]
pub extern "C" fn rt_i64_to_i8(n: i64) -> i8 {
    n as i8
}
#[no_mangle]
pub extern "C" fn rt_f64_to_i8(n: f64) -> i8 {
    n as i8
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_number(val: i64) -> f64 {
    if val < HEAP_OFFSET {
        return val as f64;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body) {
        return f64::from_bits(val as u64);
    }
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    let ptr = body as *const i64;
    if tag == TAG_FLOAT {
        return *(body as *const f64);
    }
    if tag == TAG_INT {
        return *ptr as f64;
    }
    if tag == TAG_CHAR {
        return *ptr as f64;
    }
    if tag == TAG_BOOLEAN {
        return *ptr as f64;
    }
    if tag == TAG_STRING {
        return atof(body as *const _);
    }

    // Fallback for other objects: return pointer bits as a double
    f64::from_bits(val as u64)
}







// --- Memory Management ---

#[no_mangle]
pub unsafe extern "C" fn rt_malloc(size: usize) -> *mut u8 {
    let header_size = std::mem::size_of::<ObjectHeader>();
    let p = calloc(1, size + header_size);
    if p.is_null() {
        //printf("FATAL: Out of memory\n\0".as_ptr() as *const _);
        std::process::exit(1);
    }
    let header = p as *mut ObjectHeader;
    (*header).gc_word = 0;
    (*header).type_id = 0;
    (*header).flags = 0;
    (*header).length = 0;
    (*header).capacity = 0;

    (p as *mut u8).add(header_size)
}

#[no_mangle]
pub unsafe extern "C" fn rt_free_raw(p: *mut std::ffi::c_void) {
    if !p.is_null() {
        if rt_is_gc_ptr(p as *mut u8) {
            return;
        }
        let orig_p = (p as *mut u8).sub(24); // ObjectHeader size is 24
        free(orig_p as *mut std::ffi::c_void);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_free(val: i64) {
    if val < HEAP_OFFSET {
        return;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    if rt_is_gc_ptr(body) {
        // GC objects are managed by the GC, don't free them manually
        return;
    }

    let header = rt_get_header(body);
    let type_id = (*header).type_id as i64;
    let ptr = body as *mut i64;

    if type_id == TAG_ARRAY {
        // Old layout support for arrays if any still exist using rt_malloc
        let len = *ptr.offset(1); // Old layout: tag, len, cap, ...
        let elem_size = *ptr.offset(4); // Old layout: ... flags, elem_size
        let data = *ptr.offset(3) as *mut i64;
        if !data.is_null() {
            if elem_size == 8 {
                for i in 0..len {
                    rt_free(*data.offset(i as isize));
                }
            }
            rt_free_raw(data as *mut std::ffi::c_void);
        }
    } else if type_id == TAG_OBJECT {
        let _cap = *ptr.offset(1);
        let keys = *ptr.offset(2) as *mut i64;
        let values = *ptr.offset(3) as *mut i64;
        if !keys.is_null() {
            for i in 0.._cap {
                let k = *keys.offset(i as isize);
                if k != 0 {
                    rt_free(k);
                    rt_free(*values.offset(i as isize));
                }
            }
            free(keys as *mut std::ffi::c_void);
        }
        if !values.is_null() {
            free(values as *mut std::ffi::c_void);
        }
    }

    rt_free_raw(ptr as *mut std::ffi::c_void);
}

// --- Tagging Primitives ---

#[no_mangle]
pub unsafe extern "C" fn rt_clone(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return val;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;

    if tag == TAG_STRING {
        let (data, len) = get_str_parts(val).expect("val must be string in rt_clone");
        return new_string_from_bytes(data, len);
    } else if tag == TAG_ARRAY {
        let len = (*header).length as i64;
        let elem_size = ((*header).flags & 0xFF) as i64;
        let data = body as *const i8;

        // Create new array with same elem_size
        let new_arr_val = rt_Array_new(len, elem_size);
        let new_body = (new_arr_val - HEAP_OFFSET) as *mut u8;
        let new_data = new_body as *mut i8;

        if len > 0 {
            if elem_size == 8 {
                let d_src = data as *const i64;
                let d_dst = new_data as *mut i64;
                for i in 0..len {
                    *d_dst.offset(i as isize) = rt_clone(*d_src.offset(i as isize));
                }
            } else {
                memcpy(
                    new_data as *mut _,
                    data as *const _,
                    (len * elem_size) as usize,
                );
            }
        }
        new_arr_val
    } else {
        // For other types (Objects, Char, Int, Float, Boolean, etc.), we can do a shallow copy for now,
        // but for Map/Object we might want to eventually do a deeper clone if they have nested objects.
        // However, for anagram.tx, this should be enough.
        val
    }
}


#[no_mangle]
pub unsafe extern "C" fn rt_string_from_c_str(s: *const std::ffi::c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    let len = strlen(s);
    // Body now only contains characters + null terminator. No more 16-byte [TAG, len] in body!
    let body_ptr = gc_allocate(len + 1);
    let header = rt_get_header(body_ptr);

    (*header).type_id = TAG_STRING as u16;
    (*header).length = len as u32;

    std::ptr::copy_nonoverlapping(s as *const u8, body_ptr, len);
    *(body_ptr.add(len)) = 0;

    let res = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(res, body_ptr, len as i64, 1);
    res
}

// --- IO Primitives ---

#[no_mangle]
pub unsafe extern "C" fn tejx_libc_write(fd: i64, s_ptr: i64) -> i64 {
    if s_ptr >= HEAP_OFFSET {
        let body = (s_ptr - HEAP_OFFSET) as *const u8;
        let header = rt_get_header(body as *mut u8);
        if (*header).type_id == TAG_STRING as u16 {
            let len = (*header).length as usize;
            return write(fd as i32, body as *const _, len) as i64;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn tejx_libc_puts(s_ptr: i64) -> i64 {
    if s_ptr >= HEAP_OFFSET {
        let body = (s_ptr - HEAP_OFFSET) as *const u8;
        let header = rt_get_header(body as *mut u8);
        if (*header).type_id == TAG_STRING as u16 {
            printf("%s\n\0".as_ptr() as *const _, body as *const _);
            fflush(std::ptr::null_mut());
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string_int(v: i64) -> i64 {
    let mut buf = [0u8; 32];
    sprintf(buf.as_mut_ptr() as *mut _, "%lld\0".as_ptr() as *const _, v);
    rt_string_from_c_str(buf.as_ptr() as *const _)
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string_float(v: f64) -> i64 {
    let mut buf = [0u8; 64];
    sprintf(buf.as_mut_ptr() as *mut _, "%g\0".as_ptr() as *const _, v);
    rt_string_from_c_str(buf.as_ptr() as *const _)
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string_boolean(v: i64) -> i64 {
    if v != 0 {
        rt_string_from_c_str("true\0".as_ptr() as *const _)
    } else {
        rt_string_from_c_str("false\0".as_ptr() as *const _)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string(val: i64) -> i64 {
    let mut v = val;
    let mut res_id = 0i64;
    rt_push_root(&mut v);
    rt_push_root(&mut res_id);

    // Robust pointer check: within the HEAP_OFFSET range AND verified by GC.
    let body_ptr_probe = (v - HEAP_OFFSET) as *mut u8;
    let in_offset_range = v >= HEAP_OFFSET;
    let is_gc = in_offset_range && rt_is_gc_ptr(body_ptr_probe);
    let is_stack = v >= STACK_OFFSET && v < HEAP_OFFSET;
    let is_ptr = is_gc || is_stack;

    if is_ptr {
        let body_ptr = if is_gc {
            (v - HEAP_OFFSET) as *mut u8
        } else {
            (v - STACK_OFFSET) as *mut u8
        };
        let header = rt_get_header(body_ptr);
        let tag = (*header).type_id as i64;

        if tag == TAG_STRING {
            res_id = v;
        } else if tag == TAG_FLOAT {
            let n = *(body_ptr as *const f64);
            res_id = rt_to_string_float(n);
        } else if tag == TAG_INT {
            let n = *(body_ptr as *const i64);
            res_id = rt_to_string_int(n);
        } else if tag == TAG_CHAR {
            let mut buf = [0u8; 2];
            buf[0] = *(body_ptr) as u8;
            buf[1] = 0;
            res_id = rt_string_from_c_str(buf.as_ptr() as *const _);
        } else if tag == TAG_BOOLEAN {
            let b = *(body_ptr as *const i64);
            res_id = rt_to_string_boolean(b as i64);
        } else if tag == TAG_ARRAY {
            res_id = rt_string_from_c_str("[\0".as_ptr() as *const _);
            let len = rt_len(v);
            for i in 0..len {
                let mut item = rt_array_get_fast(v, i);
                rt_push_root(&mut item);
                let mut item_str = rt_to_string(item);
                rt_push_root(&mut item_str);
                res_id = rt_str_concat_v2(res_id, item_str);
                rt_pop_roots(2); // item_str, item

                if i < len - 1 {
                    let mut comma = rt_string_from_c_str(", \0".as_ptr() as *const _);
                    rt_push_root(&mut comma);
                    res_id = rt_str_concat_v2(res_id, comma);
                    rt_pop_roots(1);
                }
            }
            let mut bracket = rt_string_from_c_str("]\0".as_ptr() as *const _);
            rt_push_root(&mut bracket);
            res_id = rt_str_concat_v2(res_id, bracket);
            rt_pop_roots(1);
        } else {
            res_id = rt_string_from_c_str("[object Object]\0".as_ptr() as *const _);
        }
    } else {
        // Handle unboxed primitives (direct i64 representation)
        if in_offset_range {
            // Large value, not a GC pointer -> Assume it's a bitcasted f64
            let n = f64::from_bits(v as u64);
            res_id = rt_to_string_float(n);
        } else {
            // Normal small integer
            res_id = rt_to_string_int(v);
        }
    }

    rt_pop_roots(2);
    res_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_panic(msg_id: i64) {
    let msg = if let Some(s) = i64_to_rust_str(msg_id) {
        format!("PANIC: {}", s)
    } else {
        "PANIC: (invalid string object)".to_string()
    };
    rt_throw_runtime_error(&msg);
}

#[no_mangle]
pub unsafe extern "C" fn rt_div_zero_error() {
    rt_throw_runtime_error("RuntimeError: Division by zero");
}

// --- Metadata ---

#[no_mangle]
pub unsafe extern "C" fn rt_len(val: i64) -> i64 {
    let resolved = if val >= HEAP_OFFSET {
        rt_resolve_array_id(val)
    } else {
        val
    };
    let is_gc = if resolved >= HEAP_OFFSET {
        rt_is_gc_ptr((resolved - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };
    if !is_gc {
        return 0;
    }
    let body = (resolved - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    if (*header).type_id == TAG_STRING as u16 || (*header).type_id == TAG_ARRAY as u16 {
        return (*header).length as i64;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_array(val: i64) -> i64 {
    let is_gc = if val >= HEAP_OFFSET {
        rt_is_gc_ptr((val - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };
    if !is_gc {
        return 0;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    if (*header).type_id == TAG_ARRAY as u16 {
        1
    } else {
        0
    }
}

// --- Raw Memory Access (Intrinsics) ---

#[no_mangle]
pub unsafe extern "C" fn rt_mem_set_i64(val_ptr: i64, offset: i64, val: i64) {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *mut u8
    } else {
        val_ptr as *mut u8
    };
    *(ptr.offset(offset as isize) as *mut i64) = val;
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_get_i64(val_ptr: i64, offset: i64) -> i64 {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *const u8
    } else {
        val_ptr as *const u8
    };
    *(ptr.offset(offset as isize) as *const i64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_set_f64(val_ptr: i64, offset: i64, val: f64) {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *mut u8
    } else {
        val_ptr as *mut u8
    };
    *(ptr.offset(offset as isize) as *mut f64) = val;
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_get_f64(val_ptr: i64, offset: i64) -> f64 {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *const u8
    } else {
        val_ptr as *const u8
    };
    *(ptr.offset(offset as isize) as *const f64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_class_new(
    type_id: i32,
    body_size: i64,
    _ptr_count: i64,
    _offsets_ptr: *const i64,
    stack_ptr: i64,
) -> i64 {
    if stack_ptr != 0 {
        // Tag it with STACK_OFFSET so runtime knows it's on stack
        return stack_ptr + STACK_OFFSET;
    }
    let size = (body_size) as usize; // Body size is now just for data, no internal tag
    let obj = gc_allocate(size) as *mut i64;

    // Primitives and fields now start at offset 0.

    let header = rt_get_header(obj as *mut u8);
    (*header).type_id = type_id as u16;
    // *obj = TAG_OBJECT; // Removed, type_id is in header
    (obj as i64) + HEAP_OFFSET
}

// --- Array Primitives ---
















// Map layout: [size, capacity, keys_ptr, values_ptr, data_base] (after ObjectHeader)
// keys and values are parallel arrays of i64
// data_base: number of method-slot entries (set before user data)

#[no_mangle]
pub unsafe extern "C" fn rt_closure_new(_capacity: i64) -> i64 {
    let raw = rt_Array_new_fixed(2, 8);
    let body = (raw - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    (*header).type_id = TAG_FUNCTION as u16;
    raw
}

// rt_get_map_body removed.

#[no_mangle]
pub unsafe extern "C" fn rt_closure_from_ptr(ptr: i64) -> i64 {
    let closure = rt_closure_new(0);
    // index 0: fn_ptr, index 1: env_ptr (null)
    rt_array_set_fast(closure, 0, ptr);
    rt_array_set_fast(closure, 1, 0);
    closure
}

// Legacy Map and Set implementations removed.

unsafe fn i64_to_rust_str(val: i64) -> Option<String> {
    if (val as u64) < (HEAP_OFFSET as u64) {
        return None;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    if (*header).type_id != TAG_STRING as u16 {
        return None;
    }
    let len = (*header).length;
    let slice = std::slice::from_raw_parts(body, len as usize);
    Some(String::from_utf8_lossy(slice).to_string())
}

// Legacy Map and Set implementations removed.

#[no_mangle]
pub unsafe extern "C" fn rt_fs_read_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if let Ok(content) = std::fs::read_to_string(&p) {
            let c_str = std::ffi::CString::new(content).unwrap_or_default();
            return rt_string_from_c_str(c_str.as_ptr());
        }
    }
    rt_string_from_c_str("\0".as_ptr() as *const _)
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_write_sync(path: i64, content: i64) -> i64 {
    if let (Some(p), Some(c)) = (i64_to_rust_str(path), i64_to_rust_str(content)) {
        if std::fs::write(&p, c).is_ok() {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_append_sync(path: i64, content: i64) -> i64 {
    if let (Some(p), Some(c)) = (i64_to_rust_str(path), i64_to_rust_str(content)) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&p)
        {
            if f.write_all(c.as_bytes()).is_ok() {
                return 1;
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_exists(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if std::path::Path::new(&p).exists() {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_unlink_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        let path = std::path::Path::new(&p);
        if path.is_dir() {
            if std::fs::remove_dir_all(path).is_ok() {
                return 1;
            }
        } else {
            if std::fs::remove_file(path).is_ok() {
                return 1;
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_mkdir_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if std::fs::create_dir_all(&p).is_ok() {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_readdir_sync(path: i64) -> i64 {
    let mut v_path = path;
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut v_path);
    rt_push_root(&mut result);

    if let Some(p) = i64_to_rust_str(v_path) {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    let mut name_id = rt_string_from_c_str(name.as_ptr() as *const _);
                    rt_push_root(&mut name_id);
                    result = rt_array_push(result, name_id);
                    rt_pop_roots(1);
                }
            }
        }
    }
    rt_pop_roots(2);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_args() -> i64 {
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut result);
    let args: Vec<String> = std::env::args().collect();
    for arg in args {
        let mut arg_id = rt_string_from_c_str(arg.as_ptr() as *const _);
        rt_push_root(&mut arg_id);
        result = rt_array_push(result, arg_id);
        rt_pop_roots(1);
    }
    rt_pop_roots(1);
    result
}

// --- String Helpers ---

unsafe fn get_str_parts(s: i64) -> Option<(*const u8, i64)> {
    if s < HEAP_OFFSET {
        return None;
    }
    let body = (s - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body) {
        return None;
    }
    let header = rt_get_header(body);
    if (*header).type_id != TAG_STRING as u16 {
        return None;
    }
    Some((body as *const u8, (*header).length as i64))
}

unsafe fn new_string_from_bytes(data: *const u8, len: i64) -> i64 {
    let body_ptr = gc_allocate(len as usize + 1);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_STRING as u16;
    (*header).length = len as u32;

    memcpy(body_ptr as *mut _, data as *const _, len as usize);
    *(body_ptr.add(len as usize)) = 0;

    (body_ptr as i64) + HEAP_OFFSET
}

// --- String Operations ---







// Internal helper to create a string from parts of another, safe for GC moves
unsafe fn new_string_from_parts(source_s: i64, offset: i64, len: i64) -> i64 {
    if len <= 0 {
        return rt_string_from_c_str("\0".as_ptr() as *const _);
    }

    let mut s = source_s;
    rt_push_root(&mut s);

    let body_ptr = gc_allocate(len as usize + 1);

    // RE-RESOLVE after potential GC
    if let Some((data, _)) = get_str_parts(s) {
        let header = rt_get_header(body_ptr);
        (*header).type_id = TAG_STRING as u16;
        (*header).length = len as u32;

        std::ptr::copy_nonoverlapping(data.offset(offset as isize), body_ptr, len as usize);
        *(body_ptr.add(len as usize)) = 0;

        let res = (body_ptr as i64) + HEAP_OFFSET;
        rt_update_array_cache(res, body_ptr, len as i64, 1);
        rt_pop_roots(1);
        res
    } else {
        rt_pop_roots(1);
        0
    }
}










#[no_mangle]
pub unsafe extern "C" fn rt_exit(code: i64) {
    exit(code as i32)
}
#[no_mangle]
pub unsafe extern "C" fn rt_getenv(key: i64) -> i64 {
    if let Some(k) = i64_to_rust_str(key) {
        if let Ok(val) = std::env::var(&k) {
            let c_str = std::ffi::CString::new(val).unwrap_or_default();
            return rt_string_from_c_str(c_str.as_ptr());
        }
    }
    rt_string_from_c_str("\0".as_ptr() as *const _)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_free_memory() -> i64 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_time_now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[no_mangle]
pub unsafe extern "C" fn rt_sleep(ms: i64) {
    let actual_ms = rt_to_number(ms) as i64;
    if actual_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(actual_ms as u64));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_time_now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// --- Timer Management ---
use std::sync::atomic::AtomicI64;
use std::time::Duration;

static NEXT_TIMER_ID: AtomicI64 = AtomicI64::new(1);

thread_local! {
    static CANCELLED_TIMERS: std::cell::RefCell<std::collections::HashSet<i64>> = std::cell::RefCell::new(std::collections::HashSet::new());
}

#[no_mangle]
pub unsafe extern "C" fn rt_timer_worker(closure_id: i64) {
    rt_call_closure_no_args(closure_id);
}


#[no_mangle]
pub unsafe extern "C" fn rt_setTimeout(callback: i64, ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let handle = unsafe { crate::event_loop::tejx_create_global_handle(callback) };
    crate::event_loop::TOKIO_RT.spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms as u64)).await;

        let cancelled = CANCELLED_TIMERS.with(|c| c.borrow().contains(&timer_id));
        if !cancelled {
            unsafe {
                let cb = crate::event_loop::tejx_get_global_handle(handle);
                crate::event_loop::tejx_enqueue_task(rt_timer_worker as *const () as i64, cb);
            }
        } else {
            CANCELLED_TIMERS.with(|c| c.borrow_mut().remove(&timer_id));
        }
        unsafe { crate::event_loop::tejx_drop_global_handle(handle) };
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    timer_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_setInterval(callback: i64, ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let handle = unsafe { crate::event_loop::tejx_create_global_handle(callback) };
    crate::event_loop::TOKIO_RT.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(ms as u64));
        interval.tick().await; // first tick returns immediately

        loop {
            interval.tick().await;

            let cancelled = CANCELLED_TIMERS.with(|c| c.borrow().contains(&timer_id));
            if cancelled {
                CANCELLED_TIMERS.with(|c| c.borrow_mut().remove(&timer_id));
                unsafe { crate::event_loop::tejx_drop_global_handle(handle) };
                unsafe { crate::event_loop::tejx_dec_async_ops() };
                break;
            }

            unsafe {
                let cb = crate::event_loop::tejx_get_global_handle(handle);
                crate::event_loop::tejx_enqueue_task(rt_timer_worker as *const () as i64, cb);
            }
        }
    });

    timer_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_clearTimeout(id: i64) -> i64 {
    CANCELLED_TIMERS.with(|c| c.borrow_mut().insert(id));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_clearInterval(id: i64) -> i64 {
    CANCELLED_TIMERS.with(|c| c.borrow_mut().insert(id));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_delay(ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    let actual_ms = rt_to_number(ms) as u64;
    let pid = rt_promise_new();

    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let handle = unsafe { crate::event_loop::tejx_create_global_handle(pid) };
    crate::event_loop::TOKIO_RT.spawn(async move {
        tokio::time::sleep(Duration::from_millis(actual_ms)).await;

        let cancelled = CANCELLED_TIMERS.with(|c| c.borrow().contains(&timer_id));
        if !cancelled {
            unsafe {
                let actual_pid = crate::event_loop::tejx_get_global_handle(handle);
                let task_args = rt_Array_new_fixed(2, 8);
                rt_array_set_fast(task_args, 0, actual_pid);
                rt_array_set_fast(task_args, 1, 0);
                crate::event_loop::tejx_enqueue_task(rt_promise_resolver_worker as *const () as i64, task_args);
            }
        } else {
            CANCELLED_TIMERS.with(|c| c.borrow_mut().remove(&timer_id));
        }
        unsafe { crate::event_loop::tejx_drop_global_handle(handle) };
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    pid
}









static mut RNG_STATE: u64 = 0;




// --- Fast Path Helpers (for Codegen) ---

#[no_mangle]
pub unsafe extern "C" fn rt_to_number_v2(v: i64) -> i64 {
    // Unbox Any, convert it to f64 using standard rules, then return raw bits
    // instead of boxing it back in TAG_FLOAT. This allows LLVM to `bitcast` it directly to `double`.
    let f = rt_to_number(v); // Returns a f64
    f.to_bits() as i64
}


#[no_mangle]
pub unsafe extern "C" fn rt_map_get_fast(this: i64, key: i64) -> i64 {
    let mut _key = key;
    if (key as u64) >= (HEAP_OFFSET as u64) {
        let k_ptr = (key - HEAP_OFFSET) as *mut i64;
        if *k_ptr == TAG_FLOAT {
            let f = *(k_ptr.offset(1) as *const f64);
            if f.fract() == 0.0 && f >= 0.0 && f < (HEAP_OFFSET as f64) {
                _key = f as i64;
            }
        }
    }

    if (this as u64) >= (HEAP_OFFSET as u64) {
        let tag = *((this - HEAP_OFFSET) as *mut i64);
        if tag == TAG_ARRAY {
            let idx = if _key >= 0 && (_key as u64) < (HEAP_OFFSET as u64) {
                _key
            } else {
                return 0;
            };
            return rt_array_get_fast(this, idx);
        }
    }
    0 // Dynamic property access on objects via string keys no longer supported without Map
}

extern "C" {
    fn tejx_main();
    fn rt_init_types();
}

#[no_mangle]
pub unsafe extern "C" fn tejx_runtime_main(_argc: i32, _argv: *mut *mut u8) -> i32 {
    rt_init_gc();
    rt_register_thread();
    rt_init_types();
    tejx_main();
    tejx_run_event_loop();
    0
}

#[no_mangle]
pub unsafe extern "C" fn a_new() -> i64 {
    rt_Array_new_fixed(0, 8)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_ptr(closure: i64) -> i64 {
    // 1. Check if it's a GC pointer (a Closure object)
    let body_ptr_probe = (closure - HEAP_OFFSET) as *mut u8;
    if (closure >= HEAP_OFFSET) && rt_is_gc_ptr(body_ptr_probe) {
        let h = rt_get_header(body_ptr_probe);
        if (*h).type_id == TAG_FUNCTION as u16 || (*h).type_id == TAG_ARRAY as u16 {
            // It's a standard Closure (stored as an array where elem 0 is the func ptr)
            let mut res = rt_array_get_fast(closure, 0);
            if res >= HEAP_OFFSET {
                let body = (res - HEAP_OFFSET) as *mut u8;
                let h_inner = rt_get_header(body);
                if (*h_inner).type_id == TAG_INT as u16 {
                    res = *(body as *mut i64);
                }
            }
            return res;
        }
    }

    // 2. Otherwise treatment as a raw function address (as stored in Any)
    closure
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_env(closure: i64) -> i64 {
    // If this is not a GC-managed closure object, treat env as null.
    if (closure as u64) >= (HEAP_OFFSET as u64) {
        let body_ptr_probe = (closure - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body_ptr_probe) {
            let h = rt_get_header(body_ptr_probe);
            if (*h).type_id == TAG_FUNCTION as u16 || (*h).type_id == TAG_ARRAY as u16 {
                return rt_array_get_fast(closure, 1);
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure(closure: i64, arg: i64) -> i64 {
    let mut c = closure;
    let mut a = arg;
    rt_push_root(&mut c);
    rt_push_root(&mut a);

    let ptr_val;
    let env;

    if (c as u64) >= (HEAP_OFFSET as u64) {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
        env = 0;
    }

    // Ensure ptr_val is unboxed if it's a heap object
    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(2);
        return 0;
    }
    let result = if env == 0 {
        // Non-closure function: no env parameter
        let func: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(a, 0, 0, 0)
    } else {
        // Closure: first argument is env
        let func: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(env, a, 0, 0, 0)
    };

    rt_pop_roots(2);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure_argv(closure: i64, args: i64) -> i64 {
    let mut c = closure;
    let mut a = args;
    rt_push_root(&mut c);
    rt_push_root(&mut a);

    let ptr_val;
    let env;

    if (c as u64) >= (HEAP_OFFSET as u64) {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
        env = 0;
    }

    // Ensure ptr_val is unboxed if it's a heap object
    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(2);
        return 0;
    }

    let mut a0 = 0;
    let mut a1 = 0;
    let mut a2 = 0;
    let mut a3 = 0;

    if a >= HEAP_OFFSET {
        let body = (a - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body) {
            let h = rt_get_header(body);
            if (*h).type_id == TAG_ARRAY as u16 {
                let len = (*h).length as i64;
                if len > 0 {
                    a0 = rt_array_get_fast(a, 0);
                }
                if len > 1 {
                    a1 = rt_array_get_fast(a, 1);
                }
                if len > 2 {
                    a2 = rt_array_get_fast(a, 2);
                }
                if len > 3 {
                    a3 = rt_array_get_fast(a, 3);
                }
            }
        }
    }

    let result = if env == 0 {
        // Non-closure function: no env parameter
        let func: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(a0, a1, a2, a3)
    } else {
        // Closure: first argument is env
        let func: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(env, a0, a1, a2, a3)
    };
    rt_pop_roots(2);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_test_invoke(func: i64) {
    let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(func as *const ());
    f();
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure_no_args(closure: i64) -> i64 {
    let mut c = closure;
    rt_push_root(&mut c);

    let is_raw_ptr = (c as u64) < (HEAP_OFFSET as u64) && c != 0;
    let ptr_val = if !is_raw_ptr {
        rt_get_closure_ptr(c)
    } else {
        c
    };
    let env = if !is_raw_ptr {
        rt_get_closure_env(c)
    } else {
        0
    };

    // Ensure ptr_val is unboxed if it's a heap object
    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(1);
        return 0;
    }

    let result = if is_raw_ptr {
        // Raw function pointers explicitly have NO env argument.
        let func: unsafe extern "C" fn() -> i64 = std::mem::transmute::<
            *const (), unsafe extern "C" fn() -> i64,
        >(raw_func_ptr as *const ());
        func()
    } else if env == 0 {
        // Closure created from raw pointer: no env parameter
        let func: unsafe extern "C" fn() -> i64 = std::mem::transmute::<
            *const (), unsafe extern "C" fn() -> i64,
        >(raw_func_ptr as *const ());
        func()
    } else {
        // Heap closures expect at least an env argument.
        let func: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute::<
            *const (), unsafe extern "C" fn(i64) -> i64,
        >(raw_func_ptr as *const ());
        func(env)
    };

    rt_pop_roots(1);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_not(val: i64) -> i64 {
    if rt_to_boolean(val) != 0 {
        BOOL_FALSE
    } else {
        BOOL_TRUE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_boolean(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return if val != 0 { 1 } else { 0 };
    }
    let body = (val - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    let tag = (*header).type_id as i64;
    if tag == TAG_BOOLEAN {
        return if *body != 0 { 1 } else { 0 };
    }
    1 // Other objects are truthy
}




#[no_mangle]
pub unsafe extern "C" fn rt_map_set_fast(this: i64, key: i64, val: i64) {
    let mut _key = key;
    if (key as u64) >= (HEAP_OFFSET as u64) {
        let k_ptr = (key - HEAP_OFFSET) as *mut i64;
        if *k_ptr == TAG_FLOAT {
            let f = *(k_ptr.offset(1) as *const f64);
            if f.fract() == 0.0 && f >= 0.0 && f < (HEAP_OFFSET as f64) {
                _key = f as i64;
            }
        }
    }

    let unboxed_val = val;
    if (this as u64) >= (HEAP_OFFSET as u64) {
        let tag = *((this - HEAP_OFFSET) as *mut i64);
        if tag == TAG_ARRAY {
            let idx = if _key >= 0 && (_key as u64) < (HEAP_OFFSET as u64) {
                _key
            } else {
                return;
            };
            rt_array_set_fast(this, idx, unboxed_val);
            return;
        }
    }
    // Dynamic property access on objects via string keys no longer supported without Map
}

// --- Atomic Operations ---
// Atomic objects store an AtomicI64 at offset 0 (as a boxed pointer)

unsafe fn get_atomic(this: i64) -> Option<&'static AtomicI64> {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return None;
    }
    let atom_ptr = *ptr.offset(0) as *const AtomicI64;
    if atom_ptr.is_null() {
        return None;
    }
    Some(&*atom_ptr)
}









#[no_mangle]
pub unsafe extern "C" fn rt_atomic_new(val: i64) -> i64 {
    let atom = Box::new(AtomicI64::new(val));
    let atom_ptr = Box::into_raw(atom);
    // Create a simple object: [0, atom_ptr]
    let obj = malloc(16) as *mut i64;
    *obj = 0; // No tag needed
    *obj.offset(0) = atom_ptr as i64;
    let result = (obj as i64) + HEAP_OFFSET;

    result
}

// --- Mutex Operations ---
// Mutex objects store a Box<std::sync::Mutex<()>> pointer at offset 0

thread_local! {
    static HELD_MUTEX_GUARDS: std::cell::RefCell<std::collections::HashMap<usize, std::sync::MutexGuard<'static, ()>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}





#[no_mangle]
pub unsafe extern "C" fn f_any_lock(m: i64) {
    rt_Mutex_acquire(m);
}
#[no_mangle]
pub unsafe extern "C" fn f_any_unlock(m: i64) {
    rt_Mutex_release(m);
}

// --- Thread Operations ---

struct ThreadData {
    handle: Option<std::thread::JoinHandle<()>>,
    started: bool,
}




// --- SharedQueue Operations ---





// Map aliases removed.

// --- Condition Variables ---

use std::sync::Condvar;

struct ConditionData {
    condvar: Condvar,
}





// --- Promises ---








#[no_mangle]
pub unsafe extern "C" fn rt_move_member(id: i64, index: i32) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    if tag == TAG_ARRAY {
        let header = rt_get_header(body);
        let len = (*header).length as i64;
        let data = body as *mut i64;
        if index >= 0 && (index as i64) < len {
            let val = *data.offset(index as isize);
            *data.offset(index as isize) = 0; // Move out
            return val;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_instanceof(obj: i64, _class_name: i64) -> i64 {
    let is_stack = obj >= STACK_OFFSET && obj < HEAP_OFFSET;
    let is_heap = obj >= HEAP_OFFSET && rt_is_gc_ptr((obj - HEAP_OFFSET) as *mut u8);
    if !is_heap && !is_stack {
        return 0;
    }
    let body = if is_heap {
        (obj - HEAP_OFFSET) as *mut u8
    } else {
        (obj - STACK_OFFSET) as *mut u8
    };
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    // Return true for objects, arrays, and user-defined classes (tag >= 12)
    if tag == TAG_OBJECT || tag == TAG_ARRAY || tag >= 12 {
        return 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_eq(a: i64, b: i64) -> i64 {
    if a == b {
        return BOOL_TRUE;
    }
    let a_is_gc = if a >= HEAP_OFFSET {
        rt_is_gc_ptr((a - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };
    let b_is_gc = if b >= HEAP_OFFSET {
        rt_is_gc_ptr((b - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };

    if a_is_gc && b_is_gc {
        let body_a = (a - HEAP_OFFSET) as *mut u8;
        let body_b = (b - HEAP_OFFSET) as *mut u8;
        let header_a = rt_get_header(body_a);
        let header_b = rt_get_header(body_b);
        let tag_a = (*header_a).type_id as i64;
        let tag_b = (*header_b).type_id as i64;

        if tag_a == TAG_STRING && tag_b == TAG_STRING {
            return if rt_str_equals(a, b) != 0 {
                BOOL_TRUE
            } else {
                BOOL_FALSE
            };
        }

        let a_num =
            tag_a == TAG_FLOAT || tag_a == TAG_INT || tag_a == TAG_CHAR || tag_a == TAG_BOOLEAN;
        let b_num =
            tag_b == TAG_FLOAT || tag_b == TAG_INT || tag_b == TAG_CHAR || tag_b == TAG_BOOLEAN;
        if a_num && b_num {
            return if rt_to_number(a) == rt_to_number(b) {
                BOOL_TRUE
            } else {
                BOOL_FALSE
            };
        }

        return BOOL_FALSE;
    }

    if a_is_gc {
        let tag = (*rt_get_header((a - HEAP_OFFSET) as *mut u8)).type_id as i64;
        if tag == TAG_STRING {
            return BOOL_FALSE;
        }
    }
    if b_is_gc {
        let tag = (*rt_get_header((b - HEAP_OFFSET) as *mut u8)).type_id as i64;
        if tag == TAG_STRING {
            return BOOL_FALSE;
        }
    }

    if rt_to_number(a) == rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_equal(a: i64, b: i64) -> i64 {
    if a == b {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_ne(a: i64, b: i64) -> i64 {
    if a != b {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_print(val: i64) {
    let mut v = val;
    let mut s_id = 0i64;
    rt_push_root(&mut v);
    rt_push_root(&mut s_id);

    s_id = rt_to_string(v);

    if let Some((data, len)) = get_str_parts(s_id) {
        let mut bytes_written = 0;
        while bytes_written < len {
            let chunk = (len - bytes_written).min(1024) as usize;
            let s = std::slice::from_raw_parts(data.add(bytes_written as usize), chunk);
            let out = std::io::stdout();
            let mut handle = out.lock();
            use std::io::Write;
            let _ = handle.write_all(s);
            bytes_written += chunk as i64;
        }
    }
    println!();

    rt_pop_roots(2);
}

#[no_mangle]
pub unsafe extern "C" fn rt_typeof(val: i64) -> i64 {
    let is_stack = val >= STACK_OFFSET && val < HEAP_OFFSET;
    let is_heap = if val >= HEAP_OFFSET {
        rt_is_gc_ptr((val - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };

    if !is_heap && !is_stack {
        if val >= BOOL_FALSE && val <= BOOL_TRUE {
            return rt_string_from_c_str("bool\0".as_ptr() as *const _);
        } else {
            return rt_string_from_c_str("int\0".as_ptr() as *const _);
        }
    } else {
        let body = if is_heap {
            (val - HEAP_OFFSET) as *mut u8
        } else {
            (val - STACK_OFFSET) as *mut u8
        };
        let header = rt_get_header(body);
        let tag = (*header).type_id as i64;
        if tag == TAG_STRING {
            return rt_string_from_c_str("string\0".as_ptr() as *const _);
        } else if tag == TAG_FUNCTION {
            return rt_string_from_c_str("function\0".as_ptr() as *const _);
        } else if tag == TAG_ARRAY {
            return rt_string_from_c_str("array\0".as_ptr() as *const _);
        } else if tag == TAG_OBJECT {
            return rt_string_from_c_str("object\0".as_ptr() as *const _);
        } else if tag == TAG_BOOLEAN {
            return rt_string_from_c_str("bool\0".as_ptr() as *const _);
        } else if tag == TAG_FLOAT {
            return rt_string_from_c_str("float\0".as_ptr() as *const _);
        } else if tag == TAG_INT {
            return rt_string_from_c_str("int\0".as_ptr() as *const _);
        } else if tag == TAG_CHAR {
            return rt_string_from_c_str("char\0".as_ptr() as *const _);
        } else if tag == TAG_PROMISE {
            return rt_string_from_c_str("object\0".as_ptr() as *const _);
        } else {
            return rt_string_from_c_str("object\0".as_ptr() as *const _);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_sizeof(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return 8; // Non-GC primitive
    }
    let body_ptr = (val - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body_ptr) {
        return 8; // Likely a 64-bit float or other non-pointer
    }
    let header = rt_get_header(body_ptr);
    let type_id = (*header).type_id;

    let header_size = 24i64;
    let mut body_size = 8i64; // Default boxed size

    if type_id == TAG_STRING as u16 {
        body_size = ((*header).length as i64 + 1 + 7) & !7;
    } else if type_id == TAG_ARRAY as u16 {
        let elem_size = ((*header).flags & 0xFF) as i64;
        body_size = ((*header).capacity as i64 * elem_size + 7) & !7;
    } else if type_id == TAG_OBJECT as u16 {
        // Map layout: fixed body 40 + [keys_ptr, values_ptr] arrays
        let cap = *(body_ptr.add(8) as *const i64);
        body_size = 40 + (cap * 16); // 2 arrays of 'cap' 8-byte elements
    } else if type_id == TAG_FUNCTION as u16 {
        body_size = 32; // rt_closure_new allocates 32
    } else if type_id == TAG_PROMISE as u16 {
        body_size = 48; // from gc.rs get_object_size
    }

    header_size + body_size
}

#[no_mangle]
pub unsafe extern "C" fn rt_await(p: i64) -> i64 {
    if p < HEAP_OFFSET {
        return p;
    }
    let mut v_p = p;
    rt_push_root(&mut v_p);

    let mut body = (v_p - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(1);
        return v_p;
    }

    // Pump the event loop until the promise is resolved/rejected
    while *body.offset(0) == 0 {
        let has_more = event_loop::tejx_run_event_loop_step();
        // Re-resolve body in case p moved during GC
        body = (v_p - HEAP_OFFSET) as *mut i64;

        if !has_more && *body.offset(0) == 0 {
            rt_throw_runtime_error(
                "RuntimeError: Deadlock detected. Awaited Promise will never resolve.",
            );
        }
    }

    // Return the resolved value
    let res = *body.offset(1);
    rt_pop_roots(1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_slice(val: i64) -> Slice {
    if val < HEAP_OFFSET {
        return Slice { ptr: 0, len: 0 };
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let len = (*header).length as i64;

    // For contiguous arrays and strings, the data starts directly at the body
    Slice {
        ptr: body as i64,
        len,
    }
}

#[path = "../event_loop.rs"]
pub mod event_loop;
pub use event_loop::*;

#[no_mangle]
pub unsafe extern "C" fn rt_box_boolean(b: i64) -> i64 {
    let body = gc_allocate(8);
    let header = rt_get_header(body);
    (*header).type_id = TAG_BOOLEAN as u16;
    (*header).length = 0;
    *(body as *mut i64) = if b != 0 { 1 } else { 0 };
    (body as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_float(f: f64) -> i64 {
    let body = gc_allocate(16);
    let header = rt_get_header(body);
    (*header).type_id = TAG_FLOAT as u16;
    (*header).length = 0;
    *(body as *mut f64) = f;
    (body as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_add(a: i64, b: i64) -> i64 {
    let a_is_str = if a >= HEAP_OFFSET {
        let body = (a - HEAP_OFFSET) as *mut u8;
        rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_STRING as u16
    } else {
        false
    };

    let b_is_str = if b >= HEAP_OFFSET {
        let body = (b - HEAP_OFFSET) as *mut u8;
        rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_STRING as u16
    } else {
        false
    };

    if a_is_str || b_is_str {
        return rt_str_concat_v2(a, b);
    }
    let res = rt_to_number(a) + rt_to_number(b);
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_sub(a: i64, b: i64) -> i64 {
    let res = rt_to_number(a) - rt_to_number(b);
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_mul(a: i64, b: i64) -> i64 {
    let res = rt_to_number(a) * rt_to_number(b);
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_div(a: i64, b: i64) -> i64 {
    let fb = rt_to_number(b);
    if fb == 0.0 {
        return 0; // Or panic
    }
    let res = rt_to_number(a) / fb;
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_lt(a: i64, b: i64) -> i64 {
    if rt_to_number(a) < rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_gt(a: i64, b: i64) -> i64 {
    if rt_to_number(a) > rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_le(a: i64, b: i64) -> i64 {
    if rt_to_number(a) <= rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_ge(a: i64, b: i64) -> i64 {
    if rt_to_number(a) >= rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_ne(a: i64, b: i64) -> i64 {
    if rt_eq(a, b) == BOOL_TRUE {
        BOOL_FALSE
    } else {
        BOOL_TRUE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_object_new() -> i64 {
    let body_ptr = gc_allocate(32);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_OBJECT as u16;

    let mut obj_id = (body_ptr as i64) + HEAP_OFFSET;
    rt_push_root(&mut obj_id);

    let mut keys = rt_Array_constructor_v2(0, 0, 8, 0);
    rt_push_root(&mut keys);
    let mut values = rt_Array_constructor_v2(0, 0, 8, 0);
    rt_push_root(&mut values);

    let body_ptr = (obj_id - HEAP_OFFSET) as *mut u8;

    *(body_ptr.offset(OBJECT_SIZE_OFFSET) as *mut i64) = 0;
    *(body_ptr.offset(OBJECT_CAP_OFFSET) as *mut i64) = 0;
    *(body_ptr.offset(OBJECT_KEYS_OFFSET) as *mut i64) = keys;
    *(body_ptr.offset(OBJECT_VALUES_OFFSET) as *mut i64) = values;

    rt_pop_roots(3);
    obj_id
}

pub unsafe fn rt_is_object(val: i64) -> bool {
    if val < HEAP_OFFSET {
        return false;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_OBJECT as u16
}

pub unsafe fn rt_object_keys_array(obj: i64) -> i64 {
    let body = (obj - HEAP_OFFSET) as *mut u8;
    *(body.offset(OBJECT_KEYS_OFFSET) as *const i64)
}

pub unsafe fn rt_object_values_array(obj: i64) -> i64 {
    let body = (obj - HEAP_OFFSET) as *mut u8;
    *(body.offset(OBJECT_VALUES_OFFSET) as *const i64)
}

pub unsafe fn rt_object_set_arrays(obj: i64, keys: i64, values: i64) {
    let body = (obj - HEAP_OFFSET) as *mut u8;
    *(body.offset(OBJECT_KEYS_OFFSET) as *mut i64) = keys;
    *(body.offset(OBJECT_VALUES_OFFSET) as *mut i64) = values;
}

pub unsafe fn rt_object_refresh_meta(obj: i64) {
    let body = (obj - HEAP_OFFSET) as *mut u8;
    let keys = rt_object_keys_array(obj);
    let len = rt_len(keys);
    *(body.offset(OBJECT_SIZE_OFFSET) as *mut i64) = len;
    *(body.offset(OBJECT_CAP_OFFSET) as *mut i64) = len;
}

pub unsafe fn rt_object_find_key_index(obj: i64, key: i64) -> i64 {
    let keys = rt_object_keys_array(obj);
    let len = rt_len(keys);
    let mut i = 0;
    while i < len {
        let existing = rt_array_get_fast(keys, i);
        if existing == key || rt_str_equals(existing, key) != 0 {
            return i;
        }
        i += 1;
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_property(obj: i64, key: i64) -> i64 {
    if (obj as u64) < (HEAP_OFFSET as u64) {
        let msg = rt_string_from_c_str(
            "RuntimeError: Null pointer dereference in property access\0".as_ptr() as *const _,
        );
        crate::event_loop::tejx_throw(msg);
    }
    if !rt_is_object(obj) {
        return 0;
    }
    let idx = rt_object_find_key_index(obj, key);
    if idx < 0 {
        return 0;
    }
    rt_array_get_fast(rt_object_values_array(obj), idx)
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_property(obj: i64, key: i64, val: i64) {
    if (obj as u64) < (HEAP_OFFSET as u64) {
        let msg = rt_string_from_c_str(
            "RuntimeError: Null pointer dereference in property assignment\0".as_ptr() as *const _,
        );
        crate::event_loop::tejx_throw(msg);
    }
    if !rt_is_object(obj) {
        return;
    }

    let mut obj = obj;
    let mut key = key;
    let mut val = val;
    rt_push_root(&mut obj);
    rt_push_root(&mut key);
    rt_push_root(&mut val);

    let mut keys = rt_object_keys_array(obj);
    let mut values = rt_object_values_array(obj);
    rt_push_root(&mut keys);
    rt_push_root(&mut values);

    let idx = rt_object_find_key_index(obj, key);
    if idx >= 0 {
        rt_array_set_fast(values, idx, val);
    } else {
        keys = rt_array_push(keys, key);
        values = rt_array_push(values, val);
        rt_object_set_arrays(obj, keys, values);
        rt_object_refresh_meta(obj);
    }

    rt_pop_roots(5);
}


#[no_mangle]
pub unsafe extern "C" fn rt_obj_keys(obj: i64) -> i64 {
    rt_Object_keys(obj)
}



pub unsafe fn rt_is_map(val: i64) -> bool {
    if val < HEAP_OFFSET {
        return false;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_MAP as u16
}

pub unsafe fn rt_map_keys_array(map: i64) -> i64 {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_KEYS_OFFSET) as *const i64)
}

pub unsafe fn rt_map_values_array(map: i64) -> i64 {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_VALUES_OFFSET) as *const i64)
}

pub unsafe fn rt_map_states_array(map: i64) -> i64 {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_STATES_OFFSET) as *const i64)
}

pub unsafe fn rt_map_set_arrays(map: i64, keys: i64, values: i64, states: i64) {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_KEYS_OFFSET) as *mut i64) = keys;
    *(body.offset(MAP_VALUES_OFFSET) as *mut i64) = values;
    *(body.offset(MAP_STATES_OFFSET) as *mut i64) = states;
}

pub unsafe fn rt_map_get_size(map: i64) -> i64 {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_SIZE_OFFSET) as *const i64)
}

pub unsafe fn rt_map_set_size(map: i64, size: i64) {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_SIZE_OFFSET) as *mut i64) = size;
}

pub unsafe fn rt_map_get_cap(map: i64) -> i64 {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_CAP_OFFSET) as *const i64)
}

pub unsafe fn rt_map_set_cap(map: i64, cap: i64) {
    let body = (map - HEAP_OFFSET) as *mut u8;
    *(body.offset(MAP_CAP_OFFSET) as *mut i64) = cap;
}

pub unsafe fn rt_tag_of(val: i64) -> i64 {
    if val >= HEAP_OFFSET {
        let body = (val - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body) {
            return (*rt_get_header(body)).type_id as i64;
        }
    }
    -1
}

fn rt_mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

pub unsafe fn rt_hash_bytes(data: *const u8, len: i64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < len {
        h ^= *data.add(i as usize) as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    rt_mix64(h)
}

pub unsafe fn rt_map_hash_key(key: i64) -> u64 {
    let tag = rt_tag_of(key);
    if tag == TAG_STRING {
        if let Some((data, len)) = get_str_parts(key) {
            return rt_hash_bytes(data, len);
        }
    }
    let is_num = key < HEAP_OFFSET
        || tag == TAG_FLOAT
        || tag == TAG_INT
        || tag == TAG_CHAR
        || tag == TAG_BOOLEAN;
    if is_num {
        let n = rt_to_number(key);
        return rt_mix64(n.to_bits());
    }
    rt_mix64(key as u64)
}

pub unsafe fn rt_map_keys_equal(a: i64, b: i64) -> bool {
    if a == b {
        return true;
    }
    let tag_a = rt_tag_of(a);
    let tag_b = rt_tag_of(b);
    if tag_a == TAG_STRING && tag_b == TAG_STRING {
        return rt_str_equals(a, b) != 0;
    }
    let a_num = a < HEAP_OFFSET
        || tag_a == TAG_FLOAT
        || tag_a == TAG_INT
        || tag_a == TAG_CHAR
        || tag_a == TAG_BOOLEAN;
    let b_num = b < HEAP_OFFSET
        || tag_b == TAG_FLOAT
        || tag_b == TAG_INT
        || tag_b == TAG_CHAR
        || tag_b == TAG_BOOLEAN;
    if a_num && b_num {
        return rt_to_number(a) == rt_to_number(b);
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_new() -> i64 {
    let cap = 16i64;
    let body_ptr = gc_allocate(40);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_MAP as u16;

    let keys = rt_Array_constructor_v2(0, cap, 8, ARRAY_FLAG_FIXED);
    let values = rt_Array_constructor_v2(0, cap, 8, ARRAY_FLAG_FIXED);
    let states = rt_Array_constructor_v2(0, cap, 1, ARRAY_FLAG_FIXED);

    *(body_ptr.offset(MAP_SIZE_OFFSET) as *mut i64) = 0;
    *(body_ptr.offset(MAP_CAP_OFFSET) as *mut i64) = cap;
    *(body_ptr.offset(MAP_KEYS_OFFSET) as *mut i64) = keys;
    *(body_ptr.offset(MAP_VALUES_OFFSET) as *mut i64) = values;
    *(body_ptr.offset(MAP_STATES_OFFSET) as *mut i64) = states;

    (body_ptr as i64) + HEAP_OFFSET
}

pub unsafe fn rt_map_resize(map: i64, new_cap: i64) {
    let keys_old = rt_map_keys_array(map);
    let values_old = rt_map_values_array(map);
    let states_old = rt_map_states_array(map);
    let cap_old = rt_map_get_cap(map);

    let keys_new = rt_Array_constructor_v2(0, new_cap, 8, ARRAY_FLAG_FIXED);
    let values_new = rt_Array_constructor_v2(0, new_cap, 8, ARRAY_FLAG_FIXED);
    let states_new = rt_Array_constructor_v2(0, new_cap, 1, ARRAY_FLAG_FIXED);

    let mut i = 0;
    while i < cap_old {
        let state = rt_array_get_fast(states_old, i);
        if state == 1 {
            let key = rt_array_get_fast(keys_old, i);
            let val = rt_array_get_fast(values_old, i);
            let mut idx = (rt_map_hash_key(key) & (new_cap as u64 - 1)) as i64;
            loop {
                let s = rt_array_get_fast(states_new, idx);
                if s == 0 {
                    rt_array_set_fast(keys_new, idx, key);
                    rt_array_set_fast(values_new, idx, val);
                    rt_array_set_fast(states_new, idx, 1);
                    break;
                }
                idx = (idx + 1) & (new_cap - 1);
            }
        }
        i += 1;
    }

    rt_map_set_arrays(map, keys_new, values_new, states_new);
    rt_map_set_cap(map, new_cap);
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_size(map: i64) -> i64 {
    if !rt_is_map(map) {
        return 0;
    }
    rt_map_get_size(map)
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_has(map: i64, key: i64) -> i64 {
    if !rt_is_map(map) {
        return BOOL_FALSE;
    }
    let cap = rt_map_get_cap(map);
    if cap <= 0 {
        return BOOL_FALSE;
    }
    let keys = rt_map_keys_array(map);
    let states = rt_map_states_array(map);
    let mut idx = (rt_map_hash_key(key) & (cap as u64 - 1)) as i64;
    loop {
        let state = rt_array_get_fast(states, idx);
        if state == 0 {
            return BOOL_FALSE;
        }
        if state == 1 {
            let existing = rt_array_get_fast(keys, idx);
            if rt_map_keys_equal(existing, key) {
                return BOOL_TRUE;
            }
        }
        idx = (idx + 1) & (cap - 1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_get(map: i64, key: i64) -> i64 {
    if !rt_is_map(map) {
        return 0;
    }
    let cap = rt_map_get_cap(map);
    if cap <= 0 {
        return 0;
    }
    let keys = rt_map_keys_array(map);
    let values = rt_map_values_array(map);
    let states = rt_map_states_array(map);
    let mut idx = (rt_map_hash_key(key) & (cap as u64 - 1)) as i64;
    loop {
        let state = rt_array_get_fast(states, idx);
        if state == 0 {
            return 0;
        }
        if state == 1 {
            let existing = rt_array_get_fast(keys, idx);
            if rt_map_keys_equal(existing, key) {
                return rt_array_get_fast(values, idx);
            }
        }
        idx = (idx + 1) & (cap - 1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_set(map: i64, key: i64, val: i64) {
    if !rt_is_map(map) {
        return;
    }
    let mut cap = rt_map_get_cap(map);
    let mut size = rt_map_get_size(map);
    if cap <= 0 {
        return;
    }
    if (size + 1) * 10 >= cap * 7 {
        rt_map_resize(map, cap * 2);
        cap = rt_map_get_cap(map);
    }

    let keys = rt_map_keys_array(map);
    let values = rt_map_values_array(map);
    let states = rt_map_states_array(map);

    let mut idx = (rt_map_hash_key(key) & (cap as u64 - 1)) as i64;
    let mut tombstone = -1;
    loop {
        let state = rt_array_get_fast(states, idx);
        if state == 0 {
            let insert_idx = if tombstone >= 0 { tombstone } else { idx };
            rt_array_set_fast(keys, insert_idx, key);
            rt_array_set_fast(values, insert_idx, val);
            rt_array_set_fast(states, insert_idx, 1);
            size += 1;
            rt_map_set_size(map, size);
            return;
        }
        if state == 2 {
            if tombstone < 0 {
                tombstone = idx;
            }
        } else {
            let existing = rt_array_get_fast(keys, idx);
            if rt_map_keys_equal(existing, key) {
                rt_array_set_fast(values, idx, val);
                return;
            }
        }
        idx = (idx + 1) & (cap - 1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_delete(map: i64, key: i64) -> i64 {
    if !rt_is_map(map) {
        return BOOL_FALSE;
    }
    let cap = rt_map_get_cap(map);
    if cap <= 0 {
        return BOOL_FALSE;
    }
    let keys = rt_map_keys_array(map);
    let values = rt_map_values_array(map);
    let states = rt_map_states_array(map);
    let mut idx = (rt_map_hash_key(key) & (cap as u64 - 1)) as i64;
    loop {
        let state = rt_array_get_fast(states, idx);
        if state == 0 {
            return BOOL_FALSE;
        }
        if state == 1 {
            let existing = rt_array_get_fast(keys, idx);
            if rt_map_keys_equal(existing, key) {
                rt_array_set_fast(states, idx, 2);
                rt_array_set_fast(keys, idx, 0);
                rt_array_set_fast(values, idx, 0);
                let size = rt_map_get_size(map);
                rt_map_set_size(map, if size > 0 { size - 1 } else { 0 });
                return BOOL_TRUE;
            }
        }
        idx = (idx + 1) & (cap - 1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_keys(map: i64) -> i64 {
    if !rt_is_map(map) {
        return rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_FIXED);
    }
    let size = rt_map_get_size(map);
    let keys = rt_map_keys_array(map);
    let states = rt_map_states_array(map);
    let cap = rt_map_get_cap(map);
    let result = rt_Array_constructor_v2(0, size, 8, ARRAY_FLAG_FIXED);
    let mut out = 0;
    let mut i = 0;
    while i < cap {
        if rt_array_get_fast(states, i) == 1 {
            rt_array_set_fast(result, out, rt_array_get_fast(keys, i));
            out += 1;
        }
        i += 1;
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_values(map: i64) -> i64 {
    if !rt_is_map(map) {
        return rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_FIXED);
    }
    let size = rt_map_get_size(map);
    let values = rt_map_values_array(map);
    let states = rt_map_states_array(map);
    let cap = rt_map_get_cap(map);
    let result = rt_Array_constructor_v2(0, size, 8, ARRAY_FLAG_FIXED);
    let mut out = 0;
    let mut i = 0;
    while i < cap {
        if rt_array_get_fast(states, i) == 1 {
            rt_array_set_fast(result, out, rt_array_get_fast(values, i));
            out += 1;
        }
        i += 1;
    }
    result
}



#[no_mangle]
pub unsafe extern "C" fn rt_object_merge(obj: i64, other: i64) -> i64 {
    rt_Object_assign(obj, other)
}

#[no_mangle]
pub unsafe extern "C" fn rt_optional_chain(obj: i64, _op: *const u8) -> i64 {
    if obj < HEAP_OFFSET {
        return 0;
    }
    obj
}
