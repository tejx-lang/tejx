#[no_mangle]
pub static HEAP_OFFSET: i64 = 1000000000000;

#[no_mangle]
pub static TAG_NUMBER: i64 = 1;
#[no_mangle]
pub static TAG_BOOLEAN: i64 = 2;
#[no_mangle]
pub static TAG_STRING: i64 = 3;
#[no_mangle]
pub static TAG_ARRAY: i64 = 4;
#[no_mangle]
pub static TAG_BYTEARRAY: i64 = 5;
#[no_mangle]
pub static TAG_MAP: i64 = 6;
#[no_mangle]
pub static TAG_PROMISE: i64 = 7;

// --- Object Layout Constants ---
// String layout: [tag:i64, len:i64] [data: len+1 bytes]
const STRING_HEADER_SIZE: isize = 16; // sizeof(tag) + sizeof(len) = 2 * 8
// Array layout: [tag, len, cap, data_ptr]
const ARRAY_LEN_OFFSET: isize = 1;
const ARRAY_CAP_OFFSET: isize = 2;
const ARRAY_DATA_OFFSET: isize = 3;
// Boolean sentinels (below HEAP_OFFSET, above normal number range)
#[no_mangle]
pub static BOOL_FALSE: i64 = 1000000;
#[no_mangle]
pub static BOOL_TRUE: i64 = 1000001;

// --- Globals used by Codegen Cache ---
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
pub static mut PREV2_ID: i64 = 0;
#[no_mangle]
pub static mut PREV2_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut PREV2_LEN: i64 = 0;

extern "C" {
    pub fn malloc(size: usize) -> *mut std::ffi::c_void;
    pub fn free(ptr: *mut std::ffi::c_void);
    pub fn realloc(ptr: *mut std::ffi::c_void, size: usize) -> *mut std::ffi::c_void;
    pub fn strlen(s: *const std::ffi::c_char) -> usize;
    pub fn memcpy(
        dest: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
        n: usize,
    ) -> *mut std::ffi::c_void;
    pub fn printf(fmt: *const std::ffi::c_char, ...) -> i32;
    pub fn sprintf(str: *mut std::ffi::c_char, fmt: *const std::ffi::c_char, ...) -> i32;
    pub fn atof(s: *const std::ffi::c_char) -> f64;
    pub fn exit(code: i32) -> !;
    pub fn write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
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
    let ptr = (val - HEAP_OFFSET) as *const i64;
    let tag = *ptr;
    if tag == TAG_NUMBER {
        return *(ptr.offset(1) as *const f64);
    }
    if tag == TAG_STRING {
        return atof((ptr as *const u8).offset(STRING_HEADER_SIZE) as *const _);
    }
    0.0
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_get_data_ptr_nocache(id: i64) -> *mut i64 {
    if id < HEAP_OFFSET {
        return std::ptr::null_mut();
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    *ptr.offset(3) as *mut i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_join(arr: i64, sep: i64) -> i64 {
    let arr_len = rt_len(arr);
    if arr_len == 0 {
        return rt_box_string("\0".as_ptr() as *const _);
    }
    // Get separator string
    let (sep_data, sep_len) = get_str_parts(sep).unwrap_or(("\0".as_ptr(), 0));
    // First pass: calculate total length
    let mut total_len: i64 = 0;
    for i in 0..arr_len {
        let elem = rt_array_get_fast(arr, i);
        let s = rt_to_string(elem);
        total_len += rt_len(s);
        if i < arr_len - 1 {
            total_len += sep_len;
        }
    }
    // Allocate result
    let obj = malloc(STRING_HEADER_SIZE as usize + total_len as usize + 1) as *mut i64;
    *obj = TAG_STRING;
    *obj.offset(1) = total_len;
    let out = (obj as *mut u8).offset(STRING_HEADER_SIZE);
    let mut pos: i64 = 0;
    for i in 0..arr_len {
        let elem = rt_array_get_fast(arr, i);
        let s = rt_to_string(elem);
        if let Some((data, len)) = get_str_parts(s) {
            memcpy(
                out.offset(pos as isize) as *mut _,
                data as *const _,
                len as usize,
            );
            pos += len;
        }
        if i < arr_len - 1 && sep_len > 0 {
            memcpy(
                out.offset(pos as isize) as *mut _,
                sep_data as *const _,
                sep_len as usize,
            );
            pos += sep_len;
        }
    }
    *out.offset(total_len as isize) = 0;
    (obj as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_slice(arr: i64, start: i64, end: i64) -> i64 {
    let arr_len = rt_len(arr);
    let s = if start < 0 {
        let v = arr_len + start;
        if v < 0 { 0 } else { v }
    } else if start > arr_len {
        arr_len
    } else {
        start
    };
    let e = if end < 0 {
        let v = arr_len + end;
        if v < 0 { 0 } else { v }
    } else if end > arr_len {
        arr_len
    } else {
        end
    };
    if s >= e {
        return a_new_fixed(0, 8);
    }
    let new_len = e - s;
    let result = a_new_fixed(new_len, 8);
    let src_data = *((arr - HEAP_OFFSET) as *const i64).offset(3) as *const i64;
    let dst_data = *((result - HEAP_OFFSET) as *const i64).offset(3) as *mut i64;
    memcpy(
        dst_data as *mut _,
        src_data.offset(s as isize) as *const _,
        (new_len * 8) as usize,
    );
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_reverse(arr: i64) -> i64 {
    if arr < HEAP_OFFSET {
        return arr;
    }
    let ptr = (arr - HEAP_OFFSET) as *mut i64;
    let len = *ptr.offset(1);
    if len <= 1 {
        return arr;
    }
    let data = *ptr.offset(3) as *mut i64;
    let mut lo = 0i64;
    let mut hi = len - 1;
    while lo < hi {
        let tmp = *data.offset(lo as isize);
        *data.offset(lo as isize) = *data.offset(hi as isize);
        *data.offset(hi as isize) = tmp;
        lo += 1;
        hi -= 1;
    }
    arr
}
// --- Memory Management ---

#[no_mangle]
pub unsafe extern "C" fn rt_malloc(size: usize) -> *mut u8 {
    malloc(size) as *mut u8
}

#[no_mangle]
pub unsafe extern "C" fn rt_free(val: i64) {
    if val >= HEAP_OFFSET {
        let ptr = (val - HEAP_OFFSET) as *mut i64;
        let tag = *ptr;
        if tag >= 1 && tag <= 7 {
            if tag == TAG_ARRAY {
                let data_ptr = *ptr.offset(3) as *mut i64;
                if !data_ptr.is_null() {
                    free(data_ptr as *mut std::ffi::c_void);
                }
            } else if tag == TAG_MAP {
                let keys = *ptr.offset(1) as *mut i64;
                if !keys.is_null() {
                    free(keys as *mut std::ffi::c_void);
                }
                let values = *ptr.offset(2) as *mut i64;
                if !values.is_null() {
                    free(values as *mut std::ffi::c_void);
                }
            }
            free(ptr as *mut std::ffi::c_void);
        }
    }
}

// --- Tagging Primitives ---

#[no_mangle]
pub unsafe extern "C" fn rt_box_number(n: f64) -> i64 {
    if n >= 0.0 && n < HEAP_OFFSET as f64 {
        return n as i64;
    }
    let ptr = malloc(16) as *mut i64;
    *ptr = TAG_NUMBER;
    *(ptr.offset(1) as *mut f64) = n;
    (ptr as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_int(n: i64) -> i64 {
    rt_box_number(n as f64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_boolean(b: bool) -> i64 {
    if b { BOOL_TRUE } else { BOOL_FALSE }
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_string(s: *const std::ffi::c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    let len = strlen(s);
    let obj = malloc(STRING_HEADER_SIZE as usize + len + 1) as *mut i64;
    *obj = TAG_STRING;
    *obj.offset(1) = len as i64;
    memcpy(
        (obj as *mut u8).offset(STRING_HEADER_SIZE) as *mut std::ffi::c_void,
        s as *const std::ffi::c_void,
        len + 1,
    );
    (obj as i64) + HEAP_OFFSET
}

// --- IO Primitives ---

#[no_mangle]
pub unsafe extern "C" fn tejx_libc_write(fd: i64, s_ptr: i64) -> i64 {
    if s_ptr >= HEAP_OFFSET {
        let ptr = (s_ptr - HEAP_OFFSET) as *const i64;
        let tag = *ptr;
        if tag == TAG_STRING {
            let len = *ptr.offset(1) as usize;
            let c_str = (ptr as *const u8).offset(STRING_HEADER_SIZE) as *const std::ffi::c_void;
            return write(fd as i32, c_str, len) as i64;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn tejx_libc_puts(s_ptr: i64) -> i64 {
    if s_ptr >= HEAP_OFFSET {
        let ptr = (s_ptr - HEAP_OFFSET) as *const i64;
        let tag = *ptr;
        if tag == TAG_STRING {
            let c_str = (ptr as *const u8).offset(STRING_HEADER_SIZE) as *const std::ffi::c_char;
            if let Ok(s) = std::ffi::CStr::from_ptr(c_str).to_str() {
                println!("{}", s);
            }
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string(val: i64) -> i64 {
    let mut buf = [0u8; 64];
    if val < HEAP_OFFSET {
        if val >= BOOL_FALSE && val <= BOOL_TRUE {
            let s = if val == BOOL_FALSE {
                "false\0"
            } else {
                "true\0"
            };
            return rt_box_string(s.as_ptr() as *const _);
        }
        sprintf(
            buf.as_mut_ptr() as *mut _,
            "%g\0".as_ptr() as *const _,
            val as f64,
        );
        rt_box_string(buf.as_ptr() as *const _)
    } else {
        let ptr = (val - HEAP_OFFSET) as *const i64;
        let tag = *ptr;
        if tag == TAG_STRING {
            val
        } else {
            rt_box_string("[object]\0".as_ptr() as *const _)
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_panic(msg: *const std::ffi::c_char) {
    printf("PANIC: %s\n\0".as_ptr() as *const _, msg);
    exit(1);
}

#[no_mangle]
pub unsafe extern "C" fn rt_div_zero_error() {
    eprintln!("PANIC: Division by zero");
    exit(1);
}

// --- Metadata ---

#[no_mangle]
pub unsafe extern "C" fn rt_len(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return 0;
    }
    let ptr = (val - HEAP_OFFSET) as *const i64;
    *ptr.offset(1)
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_array(val: i64) -> bool {
    if val < HEAP_OFFSET {
        return false;
    }
    let ptr = (val - HEAP_OFFSET) as *const i64;
    *ptr == TAG_ARRAY
}

// --- Raw Memory Access (Intrinsics) ---

#[no_mangle]
pub unsafe extern "C" fn rt_mem_set_i64(val_ptr: i64, offset: i64, val: i64) {
    let ptr = if val_ptr >= HEAP_OFFSET {
        (val_ptr - HEAP_OFFSET) as *mut u8
    } else {
        val_ptr as *mut u8
    };
    *(ptr.offset(offset as isize) as *mut i64) = val;
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_get_i64(val_ptr: i64, offset: i64) -> i64 {
    let ptr = if val_ptr >= HEAP_OFFSET {
        (val_ptr - HEAP_OFFSET) as *const u8
    } else {
        val_ptr as *const u8
    };
    *(ptr.offset(offset as isize) as *const i64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_set_f64(val_ptr: i64, offset: i64, val: f64) {
    let ptr = if val_ptr >= HEAP_OFFSET {
        (val_ptr - HEAP_OFFSET) as *mut u8
    } else {
        val_ptr as *mut u8
    };
    *(ptr.offset(offset as isize) as *mut f64) = val;
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_get_f64(val_ptr: i64, offset: i64) -> f64 {
    let ptr = if val_ptr >= HEAP_OFFSET {
        (val_ptr - HEAP_OFFSET) as *const u8
    } else {
        val_ptr as *const u8
    };
    *(ptr.offset(offset as isize) as *const f64)
}

// --- Array Primitives ---

#[no_mangle]
pub unsafe extern "C" fn a_new_fixed(len: i64, elem_size: i64) -> i64 {
    let size = 32;
    let obj = malloc(size) as *mut i64;
    *obj = TAG_ARRAY;
    *obj.offset(ARRAY_LEN_OFFSET) = len;

    let cap = if len == 0 { 4 } else { len };
    *obj.offset(ARRAY_CAP_OFFSET) = cap;

    let data_size = (cap * elem_size) as usize;
    let data_size = if data_size < 32 { 32 } else { data_size };
    let data_ptr = malloc(data_size) as *mut i64;
    *obj.offset(ARRAY_DATA_OFFSET) = data_ptr as i64;

    (obj as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_push(id: i64, val: i64) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    let len = *ptr.offset(ARRAY_LEN_OFFSET);
    let cap = *ptr.offset(ARRAY_CAP_OFFSET);
    let mut data = *ptr.offset(ARRAY_DATA_OFFSET) as *mut i64;

    if len >= cap {
        let new_cap = if cap == 0 { 4 } else { cap * 2 };
        let new_data = realloc(data as *mut _, (new_cap * 8) as usize) as *mut i64;
        *ptr.offset(ARRAY_CAP_OFFSET) = new_cap;
        *ptr.offset(ARRAY_DATA_OFFSET) = new_data as i64;
        data = new_data;
    }

    *data.offset(len as isize) = val;
    *ptr.offset(ARRAY_LEN_OFFSET) = len + 1;
    len + 1
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_pop(id: i64) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    let len = *ptr.offset(ARRAY_LEN_OFFSET);
    if len <= 0 {
        return 0;
    }
    let data = *ptr.offset(ARRAY_DATA_OFFSET) as *mut i64;
    let val = *data.offset((len - 1) as isize);
    *ptr.offset(ARRAY_LEN_OFFSET) = len - 1;
    val
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_shift(id: i64) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    let len = *ptr.offset(1);
    if len <= 0 {
        return 0;
    }
    let data = *ptr.offset(3) as *mut i64;
    let val = *data.offset(0);
    memcpy(
        data as *mut _,
        data.offset(1) as *const _,
        ((len - 1) * 8) as usize,
    );
    *ptr.offset(1) = len - 1;
    val
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_unshift(id: i64, val: i64) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    let len = *ptr.offset(1);
    let cap = *ptr.offset(2);
    let mut data = *ptr.offset(3) as *mut i64;

    if len >= cap {
        let new_cap = if cap == 0 { 4 } else { cap * 2 };
        let new_data = realloc(data as *mut _, (new_cap * 8) as usize) as *mut i64;
        *ptr.offset(2) = new_cap;
        *ptr.offset(3) = new_data as i64;
        data = new_data;
    }

    memcpy(
        data.offset(1) as *mut _,
        data as *const _,
        (len * 8) as usize,
    );
    *data = val;
    *ptr.offset(1) = len + 1;
    len + 1
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_indexOf(id: i64, val: i64) -> i64 {
    if id < HEAP_OFFSET {
        return -1;
    }
    let ptr = (id - HEAP_OFFSET) as *const i64;
    let len = *ptr.offset(1);
    let data = *ptr.offset(3) as *const i64;
    for i in 0..len {
        if *data.offset(i as isize) == val {
            return i;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_concat(id1: i64, id2: i64) -> i64 {
    let len1 = rt_len(id1);
    let len2 = rt_len(id2);
    let new_len = len1 + len2;
    let obj = a_new_fixed(new_len, 8);
    let data = *((obj - HEAP_OFFSET) as *const i64).offset(3) as *mut i64;

    if len1 > 0 {
        let d1 = *((id1 - HEAP_OFFSET) as *const i64).offset(3) as *const i64;
        memcpy(data as *mut _, d1 as *const _, (len1 * 8) as usize);
    }
    if len2 > 0 {
        let d2 = *((id2 - HEAP_OFFSET) as *const i64).offset(3) as *const i64;
        memcpy(
            data.offset(len1 as isize) as *mut _,
            d2 as *const _,
            (len2 * 8) as usize,
        );
    }
    obj
}

#[no_mangle]
pub unsafe extern "C" fn rt_obj_keys(id: i64) -> i64 {
    // Delegate to Map_keys if it's a map
    if id >= HEAP_OFFSET {
        let ptr = (id - HEAP_OFFSET) as *const i64;
        if *ptr == TAG_MAP {
            return rt_Map_keys(id);
        }
    }
    a_new_fixed(0, 8)
}

// Map layout: [TAG_MAP, size, capacity, keys_ptr, values_ptr]
// keys and values are parallel arrays of i64

#[no_mangle]
pub unsafe extern "C" fn rt_Map_constructor(_this: i64) -> i64 {
    let ptr = rt_malloc(40) as *mut i64; // 5 * 8 bytes
    *ptr = TAG_MAP;
    *ptr.offset(1) = 0; // size
    let cap: i64 = 8;
    *ptr.offset(2) = cap; // capacity
    let keys = malloc((cap * 8) as usize) as *mut i64;
    let vals = malloc((cap * 8) as usize) as *mut i64;
    for i in 0..cap {
        *keys.offset(i as isize) = 0;
        *vals.offset(i as isize) = 0;
    }
    *ptr.offset(3) = keys as i64;
    *ptr.offset(4) = vals as i64;
    (ptr as i64) + HEAP_OFFSET
}

unsafe fn map_key_eq(a: i64, b: i64) -> bool {
    if a == b {
        return true;
    }
    // Compare strings by value
    if a >= HEAP_OFFSET && b >= HEAP_OFFSET {
        let pa = (a - HEAP_OFFSET) as *const i64;
        let pb = (b - HEAP_OFFSET) as *const i64;
        if *pa == TAG_STRING && *pb == TAG_STRING {
            let la = *pa.offset(1);
            let lb = *pb.offset(1);
            if la != lb {
                return false;
            }
            let sa = (pa as *const u8).offset(STRING_HEADER_SIZE);
            let sb = (pb as *const u8).offset(STRING_HEADER_SIZE);
            for i in 0..la {
                if *sa.offset(i as isize) != *sb.offset(i as isize) {
                    return false;
                }
            }
            return true;
        }
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_set(this: i64, key: i64, val: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let size = *ptr.offset(1);
    let cap = *ptr.offset(2);
    let keys = *ptr.offset(3) as *mut i64;
    let vals = *ptr.offset(4) as *mut i64;
    // Check if key exists
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            *vals.offset(i as isize) = val;
            return;
        }
    }
    // Need to grow?
    if size >= cap {
        let new_cap = cap * 2;
        let new_keys = realloc(keys as *mut _, (new_cap * 8) as usize) as *mut i64;
        let new_vals = realloc(vals as *mut _, (new_cap * 8) as usize) as *mut i64;
        *ptr.offset(2) = new_cap;
        *ptr.offset(3) = new_keys as i64;
        *ptr.offset(4) = new_vals as i64;
        *new_keys.offset(size as isize) = key;
        *new_vals.offset(size as isize) = val;
    } else {
        *keys.offset(size as isize) = key;
        *vals.offset(size as isize) = val;
    }
    *ptr.offset(1) = size + 1;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_get(this: i64, key: i64) -> i64 {
    if this < HEAP_OFFSET {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let size = *ptr.offset(1);
    let keys = *ptr.offset(3) as *const i64;
    let vals = *ptr.offset(4) as *const i64;
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            return *vals.offset(i as isize);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_has(this: i64, key: i64) -> bool {
    if this < HEAP_OFFSET {
        return false;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let size = *ptr.offset(1);
    let keys = *ptr.offset(3) as *const i64;
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            return true;
        }
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_delete(this: i64, key: i64) -> bool {
    if this < HEAP_OFFSET {
        return false;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let size = *ptr.offset(1);
    let keys = *ptr.offset(3) as *mut i64;
    let vals = *ptr.offset(4) as *mut i64;
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            // Shift remaining elements
            for j in i..(size - 1) {
                *keys.offset(j as isize) = *keys.offset((j + 1) as isize);
                *vals.offset(j as isize) = *vals.offset((j + 1) as isize);
            }
            *ptr.offset(1) = size - 1;
            return true;
        }
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_keys(this: i64) -> i64 {
    let result = a_new_fixed(0, 8);
    if this < HEAP_OFFSET {
        return result;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let size = *ptr.offset(1);
    let keys = *ptr.offset(3) as *const i64;
    for i in 0..size {
        rt_array_push(result, *keys.offset(i as isize));
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_clear(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    *ptr.offset(1) = 0; // Reset size
}

// Set uses the same parallel-array approach as Map, with dummy values

#[no_mangle]
pub unsafe extern "C" fn rt_Set_constructor(_this: i64) {
    // Set is implemented as a Map under the hood — the _this object
    // is managed by codegen as a class instance, so nothing extra needed here
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_add(this: i64, val: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    // If it has a TAG_MAP layout, use map operations
    if *ptr == TAG_MAP {
        rt_Map_set(this, val, 1); // value doesn't matter for Set
        return;
    }
    // Fallback: treat as inline array of values at offset 3
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_has(this: i64, val: i64) -> bool {
    if this < HEAP_OFFSET {
        return false;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    if *ptr == TAG_MAP {
        return rt_Map_has(this, val);
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_delete(this: i64, val: i64) -> bool {
    if this < HEAP_OFFSET {
        return false;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    if *ptr == TAG_MAP {
        return rt_Map_delete(this, val);
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_values(this: i64) -> i64 {
    if this >= HEAP_OFFSET {
        let ptr = (this - HEAP_OFFSET) as *const i64;
        if *ptr == TAG_MAP {
            return rt_Map_keys(this); // Set stores values as keys
        }
    }
    a_new_fixed(0, 8)
}

unsafe fn i64_to_rust_str(val: i64) -> Option<String> {
    if let Some((data, len)) = get_str_parts(val) {
        let slice = std::slice::from_raw_parts(data, len as usize);
        Some(String::from_utf8_lossy(slice).to_string())
    } else {
        None
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_read_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if let Ok(content) = std::fs::read_to_string(&p) {
            let c_str = std::ffi::CString::new(content).unwrap_or_default();
            return rt_box_string(c_str.as_ptr());
        }
    }
    rt_box_string("\0".as_ptr() as *const _)
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
    let result = a_new_fixed(0, 8);
    if let Some(p) = i64_to_rust_str(path) {
        if let Ok(entries) = std::fs::read_dir(&p) {
            for entry in entries {
                if let Ok(entry) = entry {
                    if let Ok(name) = entry.file_name().into_string() {
                        let c_str = std::ffi::CString::new(name).unwrap_or_default();
                        let name_id = rt_box_string(c_str.as_ptr());
                        rt_array_push(result, name_id);
                    }
                }
            }
        }
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_values(this: i64) -> i64 {
    let result = a_new_fixed(0, 8);
    if this < HEAP_OFFSET {
        return result;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let size = *ptr.offset(1);
    let vals = *ptr.offset(4) as *const i64;
    for i in 0..size {
        rt_array_push(result, *vals.offset(i as isize));
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_args() -> i64 {
    let result = a_new_fixed(0, 8);
    for arg in std::env::args() {
        let c_str = std::ffi::CString::new(arg).unwrap_or_default();
        let arg_id = rt_box_string(c_str.as_ptr());
        rt_array_push(result, arg_id);
    }
    result
}

// --- String Helpers ---

unsafe fn get_str_parts(s: i64) -> Option<(*const u8, i64)> {
    if s < HEAP_OFFSET {
        return None;
    }
    let ptr = (s - HEAP_OFFSET) as *const i64;
    if *ptr != TAG_STRING {
        return None;
    }
    let len = *ptr.offset(1);
    let data = (ptr as *const u8).offset(STRING_HEADER_SIZE);
    Some((data, len))
}

unsafe fn new_string_from_bytes(data: *const u8, len: i64) -> i64 {
    let obj = malloc(STRING_HEADER_SIZE as usize + len as usize + 1) as *mut i64;
    *obj = TAG_STRING;
    *obj.offset(1) = len;
    let out = (obj as *mut u8).offset(STRING_HEADER_SIZE);
    if len > 0 {
        memcpy(out as *mut _, data as *const _, len as usize);
    }
    *out.offset(len as isize) = 0;
    (obj as i64) + HEAP_OFFSET
}

// --- String Operations ---

#[no_mangle]
pub unsafe extern "C" fn rt_String_toUpperCase(s: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let obj = malloc(STRING_HEADER_SIZE as usize + len as usize + 1) as *mut i64;
        *obj = TAG_STRING;
        *obj.offset(1) = len;
        let out = (obj as *mut u8).offset(STRING_HEADER_SIZE);
        for i in 0..len {
            let ch = *data.offset(i as isize);
            *out.offset(i as isize) = if ch >= b'a' && ch <= b'z' {
                ch - 32
            } else {
                ch
            };
        }
        *out.offset(len as isize) = 0;
        (obj as i64) + HEAP_OFFSET
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_toLowerCase(s: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let obj = malloc(STRING_HEADER_SIZE as usize + len as usize + 1) as *mut i64;
        *obj = TAG_STRING;
        *obj.offset(1) = len;
        let out = (obj as *mut u8).offset(STRING_HEADER_SIZE);
        for i in 0..len {
            let ch = *data.offset(i as isize);
            *out.offset(i as isize) = if ch >= b'A' && ch <= b'Z' {
                ch + 32
            } else {
                ch
            };
        }
        *out.offset(len as isize) = 0;
        (obj as i64) + HEAP_OFFSET
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_trim(s: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let mut start = 0i64;
        while start < len && (*data.offset(start as isize) <= b' ') {
            start += 1;
        }
        let mut end = len;
        while end > start && (*data.offset((end - 1) as isize) <= b' ') {
            end -= 1;
        }
        new_string_from_bytes(data.offset(start as isize), end - start)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_trimStart(s: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let mut start = 0i64;
        while start < len && (*data.offset(start as isize) <= b' ') {
            start += 1;
        }
        new_string_from_bytes(data.offset(start as isize), len - start)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_trimEnd(s: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let mut end = len;
        while end > 0 && (*data.offset((end - 1) as isize) <= b' ') {
            end -= 1;
        }
        new_string_from_bytes(data, end)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_substring(s: i64, start: i64, end: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let s0 = if start < 0 {
            0
        } else if start > len {
            len
        } else {
            start
        };
        let e0 = if end < 0 {
            0
        } else if end > len {
            len
        } else {
            end
        };
        let (s0, e0) = if s0 > e0 { (e0, s0) } else { (s0, e0) };
        new_string_from_bytes(data.offset(s0 as isize), e0 - s0)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_split(s: i64, sep: i64) -> i64 {
    let result = a_new_fixed(0, 8);
    if let (Some((s_data, s_len)), Some((sep_data, sep_len))) =
        (get_str_parts(s), get_str_parts(sep))
    {
        if sep_len == 0 {
            for i in 0..s_len {
                let ch_str = new_string_from_bytes(s_data.offset(i as isize), 1);
                rt_array_push(result, ch_str);
            }
        } else {
            let mut last = 0i64;
            let mut i = 0i64;
            while i <= s_len - sep_len {
                let mut matched = true;
                for j in 0..sep_len {
                    if *s_data.offset((i + j) as isize) != *sep_data.offset(j as isize) {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    let part = new_string_from_bytes(s_data.offset(last as isize), i - last);
                    rt_array_push(result, part);
                    last = i + sep_len;
                    i = last;
                } else {
                    i += 1;
                }
            }
            let part = new_string_from_bytes(s_data.offset(last as isize), s_len - last);
            rt_array_push(result, part);
        }
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_indexOf(s: i64, search: i64) -> i64 {
    if let (Some((s_data, s_len)), Some((search_data, search_len))) =
        (get_str_parts(s), get_str_parts(search))
    {
        if search_len == 0 {
            return 0;
        }
        if search_len > s_len {
            return -1;
        }
        for i in 0..=(s_len - search_len) {
            let mut matched = true;
            for j in 0..search_len {
                if *s_data.offset((i + j) as isize) != *search_data.offset(j as isize) {
                    matched = false;
                    break;
                }
            }
            if matched {
                return i;
            }
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_startsWith(s: i64, prefix: i64) -> bool {
    if let (Some((s_data, s_len)), Some((p_data, p_len))) =
        (get_str_parts(s), get_str_parts(prefix))
    {
        if p_len > s_len {
            return false;
        }
        for i in 0..p_len {
            if *s_data.offset(i as isize) != *p_data.offset(i as isize) {
                return false;
            }
        }
        true
    } else {
        false
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_endsWith(s: i64, suffix: i64) -> bool {
    if let (Some((s_data, s_len)), Some((sf_data, sf_len))) =
        (get_str_parts(s), get_str_parts(suffix))
    {
        if sf_len > s_len {
            return false;
        }
        let offset = s_len - sf_len;
        for i in 0..sf_len {
            if *s_data.offset((offset + i) as isize) != *sf_data.offset(i as isize) {
                return false;
            }
        }
        true
    } else {
        false
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
            return rt_box_string(c_str.as_ptr());
        }
    }
    rt_box_string("\0".as_ptr() as *const _)
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

use std::sync::atomic::AtomicBool;

static NEXT_TIMER_ID: AtomicI64 = AtomicI64::new(1);

static TIMERS: std::sync::LazyLock<
    StdMutex<std::collections::HashMap<i64, std::sync::Arc<AtomicBool>>>,
> = std::sync::LazyLock::new(|| StdMutex::new(std::collections::HashMap::new()));

#[no_mangle]
pub unsafe extern "C" fn rt_timer_worker(closure_id: i64) {
    rt_call_closure_no_args(closure_id);
}

#[no_mangle]
pub unsafe extern "C" fn rt_setTimeout(callback: i64, ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    let cancelled = std::sync::Arc::new(AtomicBool::new(false));

    if let Ok(mut timers) = TIMERS.lock() {
        timers.insert(timer_id, cancelled.clone());
    }

    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let cancelled_clone = cancelled.clone();
    std::thread::spawn(move || {
        let mut elapsed = 0;
        while elapsed < ms {
            if cancelled_clone.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            elapsed += 10;
        }
        // println!("DEBUG: rt_setTimeout {} sleep finished", timer_id);
        if !cancelled_clone.load(Ordering::SeqCst) {
            // Execute callback on the main event loop
            unsafe {
                crate::event_loop::tejx_enqueue_task(rt_timer_worker as i64, callback);
            }
            // Clean up
            if let Ok(mut timers) = TIMERS.lock() {
                timers.remove(&timer_id);
            }
        }
        // println!("DEBUG: rt_setTimeout {} decrementing ASYNC_OPS", timer_id);
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    timer_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_setInterval(callback: i64, ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    let cancelled = std::sync::Arc::new(AtomicBool::new(false));

    if let Ok(mut timers) = TIMERS.lock() {
        timers.insert(timer_id, cancelled.clone());
    }

    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let cancelled_clone = cancelled.clone();
    std::thread::spawn(move || {
        while !cancelled_clone.load(Ordering::SeqCst) {
            let mut elapsed = 0;
            while elapsed < ms {
                if cancelled_clone.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
                elapsed += 10;
            }
            if !cancelled_clone.load(Ordering::SeqCst) {
                unsafe {
                    crate::event_loop::tejx_enqueue_task(rt_timer_worker as i64, callback);
                }
            }
        }
        // println!("DEBUG: rt_setInterval {} broken out, decrementing ASYNC_OPS", timer_id);
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    timer_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_clearTimeout(id: i64) -> i64 {
    if let Ok(mut timers) = TIMERS.lock() {
        if let Some(cancelled) = timers.remove(&id) {
            cancelled.store(true, Ordering::SeqCst);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_clearInterval(id: i64) -> i64 {
    rt_clearTimeout(id)
}

#[no_mangle]
pub unsafe extern "C" fn rt_delay(ms: i64) -> i64 {
    let actual_ms = rt_to_number(ms) as u64;
    let pid = rt_promise_new();
    unsafe { crate::event_loop::tejx_inc_async_ops() };

    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(actual_ms));
        rt_promise_resolve(pid, 0);
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    pid
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect(addr: i64) -> i64 {
    use std::net::TcpStream;
    if let Some(address) = i64_to_rust_str(addr) {
        if let Ok(stream) = TcpStream::connect(&address) {
            let boxed = Box::new(stream);
            return Box::into_raw(boxed) as i64;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_send(stream: i64, data: i64) -> i64 {
    use std::io::Write;
    if stream <= 0 {
        return -1;
    }
    let s = &mut *(stream as *mut std::net::TcpStream);
    if let Some((bytes, len)) = get_str_parts(data) {
        let slice = std::slice::from_raw_parts(bytes, len as usize);
        if s.write_all(slice).is_ok() {
            return len;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_receive(stream: i64, max_len: i64) -> i64 {
    use std::io::Read;
    if stream <= 0 {
        return rt_box_string("\0".as_ptr() as *const _);
    }
    let s = &mut *(stream as *mut std::net::TcpStream);
    let max = if max_len <= 0 { 4096 } else { max_len as usize };
    let mut buf = vec![0u8; max];
    match s.read(&mut buf) {
        Ok(n) => {
            buf.truncate(n);
            buf.push(0);
            rt_box_string(buf.as_ptr() as *const _)
        }
        Err(_) => rt_box_string("\0".as_ptr() as *const _),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close(stream: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }
    let _ = Box::from_raw(stream as *mut std::net::TcpStream); // drops & closes
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_http_request(url: i64, method: i64, body: i64) -> i64 {
    let url_str = match i64_to_rust_str(url) {
        Some(s) => s,
        None => return 0,
    };
    let method_str = i64_to_rust_str(method).unwrap_or_else(|| "GET".to_string());
    let body_str = i64_to_rust_str(body);

    use std::process::Command;
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.arg("-s").arg("-X").arg(&method_str).arg(&url_str);
    if let Some(b) = body_str {
        if !b.is_empty() {
            cmd.arg("-d").arg(&b);
        }
    }
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout);
                let c_str = std::ffi::CString::new(s.as_ref()).unwrap_or_default();
                rt_box_string(c_str.as_ptr())
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

static mut RNG_STATE: u64 = 0;

#[no_mangle]
pub unsafe extern "C" fn rt_math_random() -> f64 {
    if RNG_STATE == 0 {
        use std::time::{SystemTime, UNIX_EPOCH};
        RNG_STATE = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
    }
    // xorshift64
    RNG_STATE ^= RNG_STATE << 13;
    RNG_STATE ^= RNG_STATE >> 7;
    RNG_STATE ^= RNG_STATE << 17;
    (RNG_STATE as f64) / (u64::MAX as f64)
}

// --- Fast Path Helpers (for Codegen) ---

#[no_mangle]
pub unsafe extern "C" fn rt_array_get_fast(id: i64, index: i64) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let ptr = (id - HEAP_OFFSET) as *const i64;
    let len = *ptr.offset(1);
    if index < 0 || index >= len {
        exit(1);
    }
    let data = *ptr.offset(3) as *const i64;
    *data.offset(index as isize)
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_set_fast(id: i64, index: i64, val: i64) {
    if id < HEAP_OFFSET {
        return;
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    let len = *ptr.offset(1);
    if index < 0 || index >= len {
        exit(1);
    }
    let data = *ptr.offset(3) as *mut i64;
    *data.offset(index as isize) = val;
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_get_fast(_id: i64, _key: i64) -> i64 {
    0
}

extern "C" {
    fn tejx_main();
}

#[no_mangle]
pub unsafe extern "C" fn tejx_runtime_main(_argc: i32, _argv: *mut *mut u8) -> i32 {
    tejx_main();
    tejx_run_event_loop();
    0
}

#[no_mangle]
pub unsafe extern "C" fn a_new() -> i64 {
    a_new_fixed(0, 8)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_ptr(closure: i64) -> i64 {
    let key = rt_box_string("ptr\0".as_ptr() as *const _);
    rt_Map_get(closure, key)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_env(closure: i64) -> i64 {
    let key = rt_box_string("env\0".as_ptr() as *const _);
    rt_Map_get(closure, key)
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure(closure: i64, arg: i64) -> i64 {
    let mut ptr_val = closure;
    let mut env = 0;

    if closure >= HEAP_OFFSET {
        ptr_val = rt_get_closure_ptr(closure);
        env = rt_get_closure_env(closure);
    }

    if ptr_val == 0 {
        printf("Runtime: FATAL: closure ptr is null!\n\0".as_ptr() as *const _);
        return 0;
    }
    // Expected signature: fn(env: i64, arg: i64) -> i64
    let func: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(i64, i64) -> i64,
    >(ptr_val as *const ());

    let result = func(env, arg);

    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_test_invoke(func: i64) {
    let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(func as *const ());
    f();
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure_no_args(closure: i64) -> i64 {
    let mut ptr_val = closure;
    let mut env = 0;
    let is_raw_ptr = closure < HEAP_OFFSET && closure != 0;

    if !is_raw_ptr {
        ptr_val = rt_get_closure_ptr(closure);
        env = rt_get_closure_env(closure);
    }

    if ptr_val == 0 {
        printf("Runtime: FATAL: closure ptr is null!\n\0".as_ptr() as *const _);
        return 0;
    }

    let result = if is_raw_ptr {
        // Raw function pointers explicitly have NO env argument.
        let func: unsafe extern "C" fn() -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn() -> i64>(ptr_val as *const ());
        func()
    } else {
        // Heap closures expect at least an env argument.
        let func: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(i64) -> i64,
        >(ptr_val as *const ());
        func(env)
    };

    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_not(val: i64) -> i64 {
    if val == BOOL_TRUE {
        BOOL_FALSE
    } else if val == BOOL_FALSE {
        BOOL_TRUE
    } else if val == 0 {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_str_equals(a: i64, b: i64) -> i64 {
    if a == b {
        return rt_box_boolean(true);
    }
    if a < HEAP_OFFSET || b < HEAP_OFFSET {
        return rt_box_boolean(false);
    }
    let ptr_a = (a - HEAP_OFFSET) as *const i64;
    let ptr_b = (b - HEAP_OFFSET) as *const i64;
    if *ptr_a != TAG_STRING || *ptr_b != TAG_STRING {
        return rt_box_boolean(false);
    }
    let len_a = *ptr_a.offset(1);
    let len_b = *ptr_b.offset(1);
    if len_a != len_b {
        return rt_box_boolean(false);
    }
    let str_a = (ptr_a as *const u8).offset(STRING_HEADER_SIZE);
    let str_b = (ptr_b as *const u8).offset(STRING_HEADER_SIZE);
    for i in 0..len_a {
        if *str_a.offset(i as isize) != *str_b.offset(i as isize) {
            return rt_box_boolean(false);
        }
    }
    rt_box_boolean(true)
}

#[no_mangle]
pub unsafe extern "C" fn rt_str_concat_v2(a: i64, b: i64) -> i64 {
    let sa = rt_to_string(a);
    let sb = rt_to_string(b);
    let ptr_a = (sa - HEAP_OFFSET) as *const i64;
    let ptr_b = (sb - HEAP_OFFSET) as *const i64;
    let len_a = *ptr_a.offset(1);
    let len_b = *ptr_b.offset(1);

    let obj = malloc(STRING_HEADER_SIZE as usize + (len_a + len_b) as usize + 1) as *mut i64;
    *obj = TAG_STRING;
    *obj.offset(1) = len_a + len_b;
    let out_str = (obj as *mut u8).offset(STRING_HEADER_SIZE);

    memcpy(
        out_str as *mut _,
        (ptr_a as *const u8).offset(STRING_HEADER_SIZE) as *const _,
        len_a as usize,
    );
    memcpy(
        out_str.offset(len_a as isize) as *mut _,
        (ptr_b as *const u8).offset(STRING_HEADER_SIZE) as *const _,
        len_b as usize,
    );
    *out_str.offset((len_a + len_b) as isize) = 0;

    (obj as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn tejx_throw(err: i64) -> ! {
    let msg = rt_to_string(err);
    if msg >= HEAP_OFFSET {
        let ptr = (msg - HEAP_OFFSET) as *const i64;
        let c_str = (ptr as *const u8).offset(STRING_HEADER_SIZE) as *const std::ffi::c_char;
        printf("Uncaught Exception: %s\n\0".as_ptr() as *const _, c_str);
    } else {
        printf("Uncaught Exception\n\0".as_ptr() as *const _);
    }
    exit(1);
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_get_ref(this: i64, key: i64) -> i64 {
    rt_Map_get(this, key)
}

#[no_mangle]
pub unsafe extern "C" fn rt_map_set_fast(this: i64, key: i64, val: i64) {
    rt_Map_set(this, key, val);
}

// --- Atomic Operations ---
// Atomic objects store an AtomicI64 at offset 1 (as a boxed pointer)

use std::sync::atomic::{AtomicI64, Ordering};

unsafe fn get_atomic(this: i64) -> Option<&'static AtomicI64> {
    if this < HEAP_OFFSET {
        return None;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let atom_ptr = *ptr.offset(1) as *const AtomicI64;
    if atom_ptr.is_null() {
        return None;
    }
    Some(&*atom_ptr)
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_add(this: i64, val: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.fetch_add(val, Ordering::SeqCst)
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_sub(this: i64, val: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.fetch_sub(val, Ordering::SeqCst)
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_load(this: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.load(Ordering::SeqCst)
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_store(this: i64, val: i64) {
    if let Some(atom) = get_atomic(this) {
        atom.store(val, Ordering::SeqCst);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_exchange(this: i64, val: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.swap(val, Ordering::SeqCst)
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_compareExchange(this: i64, expected: i64, desired: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        match atom.compare_exchange(expected, desired, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(v) => v,
            Err(v) => v,
        }
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_atomic_new(val: i64) -> i64 {
    let atom = Box::new(AtomicI64::new(val));
    let atom_ptr = Box::into_raw(atom);
    // Create a simple object: [0, atom_ptr]
    let obj = malloc(16) as *mut i64;
    *obj = 0; // No tag needed
    *obj.offset(1) = atom_ptr as i64;
    (obj as i64) + HEAP_OFFSET
}

// --- Mutex Operations ---
// Mutex objects store a Box<std::sync::Mutex<()>> pointer at offset 1

use std::sync::Mutex as StdMutex;

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_constructor(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let mutex = Box::new(StdMutex::new(()));
    *ptr.offset(1) = Box::into_raw(mutex) as i64;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_acquire(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let mutex_ptr = *ptr.offset(1) as *const StdMutex<()>;
    if mutex_ptr.is_null() {
        return;
    }
    let _guard = (*mutex_ptr).lock().unwrap_or_else(|e| e.into_inner());
    std::mem::forget(_guard); // Keep locked until release
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_release(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let mutex_ptr = *ptr.offset(1) as *const StdMutex<()>;
    if mutex_ptr.is_null() {
        return;
    }
    // Rust's std::sync::Mutex doesn't support explicit unlock.
    // The MutexGuard was forgotten in acquire. We cannot cleanly release it.
    // For TejX threading, we use a simpler approach: just remember the guard.
    // This is a known limitation; for production use, switch to parking_lot::Mutex.
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
    cb: i64,
    arg: i64,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_constructor(this: i64, cb: i64, arg: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let data = Box::new(ThreadData {
        cb,
        arg,
        handle: None,
    });
    *ptr.offset(1) = Box::into_raw(data) as i64;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_start(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let data_ptr = *ptr.offset(1) as *mut ThreadData;
    if data_ptr.is_null() {
        return;
    }
    let cb = (*data_ptr).cb;
    let arg = (*data_ptr).arg;
    let handle = std::thread::spawn(move || {
        let func: unsafe extern "C" fn(i64) = std::mem::transmute(cb);
        func(arg);
    });
    (*data_ptr).handle = Some(handle);
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_join(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let data_ptr = *ptr.offset(1) as *mut ThreadData;
    if data_ptr.is_null() {
        return;
    }
    if let Some(handle) = (*data_ptr).handle.take() {
        let _ = handle.join();
    }
}

// --- SharedQueue Operations ---

use std::collections::VecDeque;

struct SharedQueueData {
    mutex: StdMutex<VecDeque<i64>>,
}

#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_constructor(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let data = Box::new(SharedQueueData {
        mutex: StdMutex::new(VecDeque::new()),
    });
    *ptr.offset(1) = Box::into_raw(data) as i64;
}

#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_enqueue(this: i64, val: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let data = *ptr.offset(1) as *const SharedQueueData;
    if data.is_null() {
        return;
    }
    if let Ok(mut q) = (*data).mutex.lock() {
        q.push_back(val);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_dequeue(this: i64) -> i64 {
    if this < HEAP_OFFSET {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let data = *ptr.offset(1) as *const SharedQueueData;
    if data.is_null() {
        return 0;
    }
    if let Ok(mut q) = (*data).mutex.lock() {
        q.pop_front().unwrap_or(0)
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_size(this: i64) -> i64 {
    if this < HEAP_OFFSET {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let data = *ptr.offset(1) as *const SharedQueueData;
    if data.is_null() {
        return 0;
    }
    if let Ok(q) = (*data).mutex.lock() {
        q.len() as i64
    } else {
        0
    }
}

// --- Map Aliases ---
#[no_mangle]
pub unsafe extern "C" fn rt_map_new() -> i64 {
    rt_Map_constructor(0)
}

#[no_mangle]
pub unsafe extern "C" fn m_new() -> i64 {
    rt_Map_constructor(0)
}

#[no_mangle]
pub unsafe extern "C" fn m_set(this: i64, key: i64, val: i64) {
    rt_Map_set(this, key, val);
}

// --- Condition Variables ---

use std::sync::Condvar;

struct ConditionData {
    condvar: Condvar,
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_constructor(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let data = Box::new(ConditionData {
        condvar: Condvar::new(),
    });
    *ptr.offset(1) = Box::into_raw(data) as i64;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_wait(this: i64, mutex: i64) {
    if this < HEAP_OFFSET || mutex < HEAP_OFFSET {
        return;
    }
    let cond_ptr = (this - HEAP_OFFSET) as *const i64;
    let cond_data = *cond_ptr.offset(1) as *const ConditionData;
    let mutex_obj = (mutex - HEAP_OFFSET) as *const i64;
    let mutex_ptr = *mutex_obj.offset(1) as *const StdMutex<()>;
    if cond_data.is_null() || mutex_ptr.is_null() {
        return;
    }
    let guard = (*mutex_ptr).lock().unwrap_or_else(|e| e.into_inner());
    drop(
        (*cond_data)
            .condvar
            .wait(guard)
            .unwrap_or_else(|e| e.into_inner()),
    );
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notify(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let data = *ptr.offset(1) as *const ConditionData;
    if data.is_null() {
        return;
    }
    (*data).condvar.notify_one();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notifyAll(this: i64) {
    if this < HEAP_OFFSET {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let data = *ptr.offset(1) as *const ConditionData;
    if data.is_null() {
        return;
    }
    (*data).condvar.notify_all();
}

// --- Promises ---
#[no_mangle]
pub unsafe extern "C" fn rt_promise_new() -> i64 {
    let obj = malloc(32) as *mut i64;
    *obj = TAG_PROMISE;
    *obj.offset(1) = 0; // State: Pending
    *obj.offset(2) = 0; // Value
    *obj.offset(3) = a_new_fixed(0, 8); // Callbacks array (empty)
    (obj as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_resolve(p: i64, val: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let ptr = (p - HEAP_OFFSET) as *mut i64;
    if *ptr != TAG_PROMISE {
        return;
    }
    if *ptr.offset(1) != 0 {
        return;
    } // Already resolved/rejected

    *ptr.offset(1) = 1; // Resolved
    *ptr.offset(2) = val; // Store value

    // Execute callbacks
    let callbacks_arr = *ptr.offset(3);
    let n = rt_len(callbacks_arr);

    for i in (0..n).step_by(2) {
        let cb = rt_array_get_fast(callbacks_arr, i as i64);
        let next_p = rt_array_get_fast(callbacks_arr, (i + 1) as i64);

        let res = rt_call_closure(cb, val);
        rt_promise_resolve(next_p, res);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_reject(p: i64, err: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let ptr = (p - HEAP_OFFSET) as *mut i64;
    if *ptr != TAG_PROMISE {
        return;
    }
    if *ptr.offset(1) != 0 {
        return;
    }

    *ptr.offset(1) = 2; // Rejected
    *ptr.offset(2) = err; // Store error
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_then(p: i64, cb: i64) -> i64 {
    if p < HEAP_OFFSET {
        return rt_promise_new();
    }
    let ptr = (p - HEAP_OFFSET) as *mut i64;
    if *ptr != TAG_PROMISE {
        return rt_promise_new();
    }

    let state = *ptr.offset(1);
    let new_p = rt_promise_new();

    if state == 0 {
        // Pending: Store (cb, new_p)
        let callbacks_arr = *ptr.offset(3);
        rt_array_push(callbacks_arr, cb);
        rt_array_push(callbacks_arr, new_p);
    } else if state == 1 {
        // Resolved: Execute immediately (or enqueue)
        let val = *ptr.offset(2);
        let res = rt_call_closure(cb, val);
        rt_promise_resolve(new_p, res);
    }
    new_p
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_clone(p: i64) -> i64 {
    p
}

#[no_mangle]
pub unsafe extern "C" fn rt_move_member(id: i64, index: i32) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let ptr = (id - HEAP_OFFSET) as *mut i64;
    let tag = *ptr;
    if tag == TAG_ARRAY {
        let len = *ptr.offset(ARRAY_LEN_OFFSET);
        let data_ptr = *ptr.offset(ARRAY_DATA_OFFSET) as *mut i64;
        if index >= 0 && (index as i64) < len {
            let val = *data_ptr.offset(index as isize);
            *data_ptr.offset(index as isize) = 0; // Move out
            return val;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_instanceof(obj: i64, _class_name: i64) -> i32 {
    if obj < HEAP_OFFSET {
        return 0;
    }
    // Simple mock for now: Check if it's an object/map or array based on tag
    let ptr = (obj - HEAP_OFFSET) as *const i64;
    let tag = *ptr;
    if tag == TAG_MAP || tag == TAG_ARRAY {
        return 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_object_merge(obj: i64, _other: i64) -> i64 {
    // Basic merge: return first for now or create new
    obj
}

#[no_mangle]
pub unsafe extern "C" fn rt_optional_chain(obj: i64, _op: *const u8) -> i64 {
    if obj < HEAP_OFFSET {
        return 0;
    }
    obj
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_equal(a: i64, b: i64) -> bool {
    a == b
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_ne(a: i64, b: i64) -> bool {
    a != b
}

#[no_mangle]
pub unsafe extern "C" fn rt_typeof(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return rt_box_string("number\0".as_ptr() as *const _);
    }
    let ptr = (val - HEAP_OFFSET) as *const i64;
    let tag = *ptr;
    if tag == TAG_BOOLEAN {
        return rt_box_string("boolean\0".as_ptr() as *const _);
    } else if tag == TAG_STRING {
        return rt_box_string("string\0".as_ptr() as *const _);
    } else if tag == TAG_ARRAY || tag == TAG_MAP {
        return rt_box_string("object\0".as_ptr() as *const _);
    }
    rt_box_string("object\0".as_ptr() as *const _)
}

#[no_mangle]
pub unsafe extern "C" fn m_new_1(k1: i64, v1: i64) -> i64 {
    let m = rt_Map_constructor(0);
    rt_Map_set(m, k1, v1);
    m
}

#[no_mangle]
pub unsafe extern "C" fn m_new_2(k1: i64, v1: i64, k2: i64, v2: i64) -> i64 {
    let m = rt_Map_constructor(0);
    rt_Map_set(m, k1, v1);
    rt_Map_set(m, k2, v2);
    m
}

#[no_mangle]
pub unsafe extern "C" fn m_new_3(k1: i64, v1: i64, k2: i64, v2: i64, k3: i64, v3: i64) -> i64 {
    let m = rt_Map_constructor(0);
    rt_Map_set(m, k1, v1);
    rt_Map_set(m, k2, v2);
    rt_Map_set(m, k3, v3);
    m
}

#[no_mangle]
pub unsafe extern "C" fn rt_await(p: i64) -> i64 {
    p
}

pub mod event_loop;
pub use event_loop::*;
