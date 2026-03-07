pub mod gc;
pub use gc::{
    GC_ROOTS_TOP, ObjectHeader, gc_allocate, rt_get_header, rt_init_gc, rt_is_gc_ptr, rt_pop_roots,
    rt_push_root,
};

#[no_mangle]
pub static HEAP_OFFSET: i64 = 1i64 << 50;
#[no_mangle]
pub static STACK_OFFSET: i64 = 1i64 << 48;

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

// --- Object Layout Constants ---
// String layout: [data: len+1 bytes]
const STRING_HEADER_SIZE: isize = 24;
// Array layout: [tag:i64] [data: len * elem_size bytes]
const ARRAY_LEN_OFFSET: isize = 12; // byte offset in header
const ARRAY_CAP_OFFSET: isize = 16; // byte offset in header
const ARRAY_FLAGS_OFFSET: isize = 10; // byte offset in header (flags & 0xFF = elem_size)
const ARRAY_HEADER_SIZE: isize = 24;

const ARRAY_FLAG_FIXED: i64 = 0x01;
const ARRAY_FLAG_CONSTANT: i64 = 0x02;
// Boolean sentinels (below HEAP_OFFSET, above normal number range)
#[no_mangle]
pub static BOOL_FALSE: i64 = 0;
#[no_mangle]
pub static BOOL_TRUE: i64 = 1;

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

    PREV_ID = LAST_ID;
    PREV_PTR = LAST_PTR;
    PREV_LEN = LAST_LEN;

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
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    let ptr = body as *const i64;
    if tag == TAG_FLOAT {
        return *(ptr as *const f64);
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
    0.0
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_get_data_ptr(id: i64) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    // With contiguous layout, id (pointer to body) IS the start of the data
    id
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_get_data_ptr_nocache(id: i64) -> i64 {
    rt_array_get_data_ptr(id)
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
    let body_ptr = gc_allocate(total_len as usize + 1);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_STRING as u16;
    (*header).length = total_len as u32;

    let out = body_ptr;
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
    let res = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(res, body_ptr, total_len, 1);
    res
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

    let ptr = if arr >= HEAP_OFFSET {
        (arr - HEAP_OFFSET) as *mut i64
    } else {
        std::ptr::null_mut()
    };
    let elem_size = if !ptr.is_null() {
        let body = (arr - HEAP_OFFSET) as *mut u8;
        let header = rt_get_header(body);
        ((*header).flags & 0xFF) as i64
    } else {
        8
    };

    if s >= e {
        return rt_Array_constructor(0, 0, elem_size);
    }

    let new_len = e - s;
    let result = rt_Array_constructor(0, new_len, elem_size);
    if !ptr.is_null() {
        let src_data = (arr - HEAP_OFFSET) as *const i8;
        let dst_data = (result - HEAP_OFFSET) as *mut i8;
        memcpy(
            dst_data as *mut _,
            src_data.offset((s * elem_size) as isize) as *const _,
            (new_len * elem_size) as usize,
        );
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_reverse(arr: i64) -> i64 {
    if arr < HEAP_OFFSET {
        return arr;
    }
    let body = (arr - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & (ARRAY_FLAG_CONSTANT as u16)) != 0 {
        printf("RuntimeError: Cannot reverse a constant array.\n\0".as_ptr() as *const _);
        exit(1);
    }

    let len = (*header).length as i64;
    let elem_size = (flags & 0xFF) as i64;
    if len <= 1 {
        return arr;
    }

    let data = body as *mut i8;
    let mut lo = 0i64;
    let mut hi = len - 1;
    while lo < hi {
        let p_lo = data.offset((lo * elem_size) as isize);
        let p_hi = data.offset((hi * elem_size) as isize);

        match elem_size {
            1 => {
                let tmp = *(p_lo as *mut i8);
                *(p_lo as *mut i8) = *(p_hi as *mut i8);
                *(p_hi as *mut i8) = tmp;
            }
            2 => {
                let tmp = *(p_lo as *mut i16);
                *(p_lo as *mut i16) = *(p_hi as *mut i16);
                *(p_hi as *mut i16) = tmp;
            }
            4 => {
                let tmp = *(p_lo as *mut i32);
                *(p_lo as *mut i32) = *(p_hi as *mut i32);
                *(p_hi as *mut i32) = tmp;
            }
            _ => {
                let tmp = *(p_lo as *mut i64);
                *(p_lo as *mut i64) = *(p_hi as *mut i64);
                *(p_hi as *mut i64) = tmp;
            }
        }
        lo += 1;
        hi -= 1;
    }
    arr
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_sort(arr: i64) -> i64 {
    if arr < HEAP_OFFSET {
        return arr;
    }
    let body = (arr - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & (ARRAY_FLAG_CONSTANT as u16)) != 0 {
        printf("RuntimeError: Cannot sort a constant array.\n\0".as_ptr() as *const _);
        exit(1);
    }

    let len = (*header).length as i64;
    let elem_size = (flags & 0xFF) as i64;
    if len <= 1 {
        return arr;
    }
    let data = body as *mut i64; // Still i64 for sort?
    // WARNING: sort_unstable only works for i64 slices here.
    // If we have Array<int32>, this might be wrong if it's treating it as i64.
    // However, the standard sort is usually for Any[].
    if elem_size == 8 {
        let slice = std::slice::from_raw_parts_mut(data, len as usize);
        slice.sort_unstable();
    } else if elem_size == 4 {
        let slice = std::slice::from_raw_parts_mut(data as *mut i32, len as usize);
        slice.sort_unstable();
    }
    arr
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_fill(arr: i64, val: i64) -> i64 {
    if arr < HEAP_OFFSET {
        return arr;
    }
    let body = (arr - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & (ARRAY_FLAG_CONSTANT as u16)) != 0 {
        printf("RuntimeError: Cannot fill a constant array.\n\0".as_ptr() as *const _);
        exit(1);
    }

    let len = (*header).length as i64;
    let elem_size = (flags & 0xFF) as i64;
    let data = body as *mut i8;
    for i in 0..len {
        let p = data.offset((i * elem_size) as isize);
        match elem_size {
            1 => *(p as *mut i8) = val as i8,
            2 => *(p as *mut i16) = val as i16,
            4 => *(p as *mut i32) = val as i32,
            _ => *(p as *mut i64) = val,
        }
    }
    arr
}
// --- Memory Management ---

#[no_mangle]
pub unsafe extern "C" fn rt_malloc(size: usize) -> *mut u8 {
    let header_size = std::mem::size_of::<ObjectHeader>();
    let p = calloc(1, size + header_size);
    if p.is_null() {
        printf("FATAL: Out of memory\n\0".as_ptr() as *const _);
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
        let _cap = *ptr.offset(2); // Renamed 'cap' to '_cap'
        let keys = *ptr.offset(3) as *mut i64;
        let values = *ptr.offset(4) as *mut i64;
        if !keys.is_null() {
            for i in 0.._cap {
                // Use _cap here
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
pub unsafe extern "C" fn rt_box_number(n: f64) -> i64 {
    let body_ptr = gc_allocate(8);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_FLOAT as u16;
    (*header).length = 0;

    let ptr = body_ptr as *mut f64;
    *ptr = n;

    (body_ptr as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_int(n: i64) -> i64 {
    let body_ptr = gc_allocate(8);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_INT as u16;
    (*header).length = 0;

    let ptr = body_ptr as *mut i64;
    *ptr = n;

    (body_ptr as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_char(c: i64) -> i64 {
    let body_ptr = gc_allocate(8);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_CHAR as u16;
    (*header).length = 0;

    let ptr = body_ptr as *mut i64;
    *ptr = c;

    (body_ptr as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_boolean(b: i64) -> i64 {
    let body_ptr = gc_allocate(8);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_BOOLEAN as u16;
    (*header).length = 0;

    let ptr = body_ptr as *mut i64;
    *ptr = if b != 0 { 1 } else { 0 };

    (body_ptr as i64) + HEAP_OFFSET
}

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
pub unsafe extern "C" fn rt_box_string(s: *const std::ffi::c_char) -> i64 {
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
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string(val: i64) -> i64 {
    let mut v = val;
    let mut res_id = 0i64;
    rt_push_root(&mut v);
    rt_push_root(&mut res_id);

    let is_gc = if v >= HEAP_OFFSET {
        rt_is_gc_ptr((v - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };

    if !is_gc {
        let mut buf = [0u8; 64];
        sprintf(buf.as_mut_ptr() as *mut _, "%lld\0".as_ptr() as *const _, v);
        res_id = rt_box_string(buf.as_ptr() as *const _);
    } else {
        let body_ptr = (v - HEAP_OFFSET) as *mut u8;
        let header = rt_get_header(body_ptr);
        let tag = (*header).type_id as i64;
        let ptr = body_ptr as *mut i64;

        if tag == TAG_STRING {
            res_id = v;
        } else if tag == TAG_FLOAT {
            let mut buf = [0u8; 64];
            let n = *(body_ptr as *const f64);
            sprintf(buf.as_mut_ptr() as *mut _, "%g\0".as_ptr() as *const _, n);
            res_id = rt_box_string(buf.as_ptr() as *const _);
        } else if tag == TAG_INT {
            let mut buf = [0u8; 64];
            let n = *ptr;
            sprintf(buf.as_mut_ptr() as *mut _, "%lld\0".as_ptr() as *const _, n);
            res_id = rt_box_string(buf.as_ptr() as *const _);
        } else if tag == TAG_CHAR {
            let mut buf = [0u8; 2];
            buf[0] = *body_ptr as u8;
            buf[1] = 0;
            res_id = rt_box_string(buf.as_ptr() as *const _);
        } else if tag == TAG_BOOLEAN {
            let b = *body_ptr;
            let s = if b == 0 { "false\0" } else { "true\0" };
            res_id = rt_box_string(s.as_ptr() as *const _);
        } else if tag == TAG_ARRAY {
            res_id = rt_box_string("[\0".as_ptr() as *const _);
            let len = rt_len(v);
            for i in 0..len {
                let mut item = rt_array_get_fast(v, i);
                rt_push_root(&mut item);
                let mut item_str = rt_to_string(item);
                rt_push_root(&mut item_str);
                res_id = rt_str_concat_v2(res_id, item_str);
                rt_pop_roots(1); // item_str
                rt_pop_roots(1); // item

                if i < len - 1 {
                    let mut comma = rt_box_string(", \0".as_ptr() as *const _);
                    rt_push_root(&mut comma);
                    res_id = rt_str_concat_v2(res_id, comma);
                    rt_pop_roots(1);
                }
            }
            let mut bracket = rt_box_string("]\0".as_ptr() as *const _);
            rt_push_root(&mut bracket);
            res_id = rt_str_concat_v2(res_id, bracket);
            rt_pop_roots(1);
        } else if tag == TAG_OBJECT {
            let mut brace_open = rt_box_string("{\0".as_ptr() as *const _);
            rt_push_root(&mut brace_open);
            res_id = rt_str_concat_v2(res_id, brace_open);
            rt_pop_roots(1);

            // We must re-resolve everything inside the loop as GC can happen
            let mut i = 0i64;
            loop {
                let current_ptr = (v - HEAP_OFFSET) as *mut i64;
                let size = *current_ptr.offset(0);
                if i >= size {
                    break;
                }

                let current_keys = *current_ptr.offset(2) as *const i64;
                let current_vals = *current_ptr.offset(3) as *const i64;

                let mut k = *current_keys.offset(i as isize);
                let mut v_val = *current_vals.offset(i as isize);
                rt_push_root(&mut k);
                rt_push_root(&mut v_val);

                let k_str = rt_to_string(k);
                res_id = rt_str_concat_v2(res_id, k_str);

                let mut colon = rt_box_string(": \0".as_ptr() as *const _);
                rt_push_root(&mut colon);
                res_id = rt_str_concat_v2(res_id, colon);
                rt_pop_roots(1);

                let v_str = rt_to_string(v_val);
                res_id = rt_str_concat_v2(res_id, v_str);

                // Re-read size inside loop in case something grew (unlikely here but safe)
                let latest_ptr = (v - HEAP_OFFSET) as *mut i64;
                let latest_size = *latest_ptr.offset(0);

                if i < latest_size - 1 {
                    let mut comma = rt_box_string(", \0".as_ptr() as *const _);
                    rt_push_root(&mut comma);
                    res_id = rt_str_concat_v2(res_id, comma);
                    rt_pop_roots(1);
                }

                rt_pop_roots(2); // k, v_val
                i += 1;
            }
            let mut brace_close = rt_box_string("}\0".as_ptr() as *const _);
            rt_push_root(&mut brace_close);
            res_id = rt_str_concat_v2(res_id, brace_close);
            rt_pop_roots(1);
        } else if tag == TAG_FUNCTION {
            res_id = rt_box_string("[function]\0".as_ptr() as *const _);
        } else {
            res_id = rt_box_string("[object]\0".as_ptr() as *const _);
        }
    }

    rt_pop_roots(2); // pop v, res_id
    res_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_panic(msg_id: i64) {
    if let Some(s) = i64_to_rust_str(msg_id) {
        let c_str = std::ffi::CString::new(s).unwrap_or_default();
        printf("PANIC: %s\n\0".as_ptr() as *const _, c_str.as_ptr());
    } else {
        printf("PANIC: (invalid string object)\n\0".as_ptr() as *const _);
    }
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
    let header = rt_get_header(obj as *mut u8);
    (*header).type_id = type_id as u16;
    // *obj = TAG_OBJECT; // Removed, type_id is in header
    (obj as i64) + HEAP_OFFSET
}

// --- Array Primitives ---

#[no_mangle]
pub unsafe extern "C" fn rt_Array_constructor(_this: i64, size: i64, elem_size: i64) -> i64 {
    rt_Array_constructor_v2(_this, size, elem_size, 0)
}

#[no_mangle]
pub unsafe extern "C" fn rt_Array_constructor_v2(
    _this: i64,
    size: i64,
    elem_size: i64,
    flags: i64,
) -> i64 {
    let cap = if size == 0 { 4 } else { size };
    let mut elem_size = elem_size;
    if elem_size == 0 {
        elem_size = 8; // Default to 8 if unknown
    }
    let total_size = (cap * elem_size) as usize;

    // Allocate space for data only
    let body_ptr = gc_allocate(total_size);
    let header = rt_get_header(body_ptr);

    (*header).type_id = TAG_ARRAY as u16;
    (*header).length = size as u32;
    (*header).capacity = cap as u32;
    // Store elem_size in lower 8 bits of flags
    (*header).flags = (flags as u16 & 0xFF00) | (elem_size as u16 & 0x00FF);

    let id = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(id, body_ptr, size, elem_size);
    id
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_ensure_capacity(id: i64, required: i64) -> i64 {
    let mut current_id = id;
    if (current_id as u64) < (HEAP_OFFSET as u64) {
        return current_id;
    }
    let body = (current_id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let cap = (*header).capacity as i64;
    let flags = (*header).flags;

    if required <= cap {
        return current_id;
    }

    if (flags & (ARRAY_FLAG_FIXED as u16)) != 0 {
        printf("RuntimeError: Cannot resize a fixed-size array.\n\0".as_ptr() as *const _);
        exit(1);
    }

    let elem_size = (flags & 0xFF) as i64;
    let mut new_cap = if cap == 0 { 4 } else { cap * 2 };
    if new_cap < required {
        new_cap = required;
    }

    rt_push_root(&mut current_id);
    let new_body = gc_allocate((new_cap * elem_size) as usize);
    // RE-RESOLVE after potential GC
    let body_res = (current_id - HEAP_OFFSET) as *mut u8;
    let header_res = rt_get_header(body_res);

    let new_header = rt_get_header(new_body);

    // Copy header
    *new_header = *header_res;
    (*new_header).capacity = new_cap as u32;

    // Copy data (direct copy)
    memcpy(
        new_body as *mut _,
        body_res as *const _,
        (cap * elem_size) as usize,
    );
    let res = (new_body as i64) + HEAP_OFFSET;
    rt_pop_roots(1);
    rt_update_array_cache(res, new_body, (*new_header).length as i64, elem_size);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_Array_new(len: i64, elem_size: i64) -> i64 {
    let data_size = len as usize * elem_size as usize;
    let body_ptr = gc_allocate(data_size); // No internal tag, just data
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_ARRAY as u16;
    (*header).length = len as u32;
    (*header).capacity = len as u32;
    (*header).flags = elem_size as u16; // Store elem_size in flags

    // let ptr = body_ptr as *mut i64; // No longer needed
    // *ptr = TAG_ARRAY; // No longer needed

    let new_id = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(new_id, body_ptr, len, elem_size); // Pass body_ptr directly
    new_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_new(len: i64, elem_size: i64) -> i64 {
    rt_Array_new(len, elem_size)
}

#[no_mangle]
pub unsafe extern "C" fn rt_Array_new_fixed(len: i64, elem_size: i64) -> i64 {
    rt_Array_new(len, elem_size)
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_push(id: i64, val: i64) -> i64 {
    let mut current_id = id;
    let mut current_val = val;
    if (current_id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    rt_push_root(&mut current_id);
    rt_push_root(&mut current_val);

    let body = (current_id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & (ARRAY_FLAG_CONSTANT as u16)) != 0 {
        printf("RuntimeError: Cannot push to a constant array.\n\0".as_ptr() as *const _);
        exit(1);
    }

    let len = (*header).length as i64;
    current_id = rt_array_ensure_capacity(current_id, len + 1);

    let new_body = (current_id - HEAP_OFFSET) as *mut u8;
    let new_header = rt_get_header(new_body);
    let elem_size = ((*new_header).flags & 0xFF) as i64;

    // Store val based on elem_size, direct access
    let data = new_body as *mut i8; // Data starts directly at body_ptr
    match elem_size {
        1 => *(data.offset((len * elem_size) as isize) as *mut i8) = current_val as i8,
        2 => *(data.offset((len * elem_size) as isize) as *mut i16) = current_val as i16,
        4 => *(data.offset((len * elem_size) as isize) as *mut i32) = current_val as i32,
        _ => *(data.offset((len * elem_size) as isize) as *mut i64) = current_val,
    }

    (*new_header).length = (len + 1) as u32;
    rt_update_array_cache(current_id, new_body, len + 1, elem_size);

    rt_pop_roots(2);
    current_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_pop(id: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & ((ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16)) != 0 {
        printf(
            "RuntimeError: Cannot pop from a fixed-size or constant array.\n\0".as_ptr()
                as *const _,
        );
        exit(1);
    }

    let len = (*header).length as i64;
    if len <= 0 {
        return 0;
    }

    let elem_size = (flags & 0xFF) as i64;
    let data = body as *mut i8; // Data starts directly at body_ptr

    let last_idx = len - 1;
    let val = match elem_size {
        1 => *(data.offset(last_idx as isize) as *mut i8) as i64,
        2 => *(data.offset((last_idx * 2) as isize) as *mut i16) as i64,
        4 => *(data.offset((last_idx * 4) as isize) as *mut i32) as i64,
        _ => *(data.offset((last_idx * 8) as isize) as *mut i64),
    };

    (*header).length = last_idx as u32;
    rt_update_array_cache(id, body, last_idx, elem_size); // Pass body_ptr directly
    val
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_set_fast(id: i64, index: i64, val: i64) {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & (ARRAY_FLAG_CONSTANT as u16)) != 0 {
        printf("RuntimeError: Cannot set element in a constant array.\n\0".as_ptr() as *const _);
        exit(1);
    }

    let len = (*header).length as i64;
    if index < 0 || index >= len {
        return;
    }
    let data = body as *mut i8; // Data starts directly at body_ptr
    let elem_size = (flags & 0xFF) as i64;
    match elem_size {
        1 => *(data.offset(index as isize) as *mut i8) = val as i8,
        2 => *(data.offset((index * 2) as isize) as *mut i16) = val as i16,
        4 => *(data.offset((index * 4) as isize) as *mut i32) = val as i32,
        _ => *(data.offset((index * 8) as isize) as *mut i64) = val,
    }
    rt_update_array_cache(id, body, len, elem_size);
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_shift(id: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & ((ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16)) != 0 {
        printf(
            "RuntimeError: Cannot shift a fixed-size or constant array.\n\0".as_ptr() as *const _,
        );
        exit(1);
    }

    let len = (*header).length as i64;
    if len <= 0 {
        return 0;
    }

    let elem_size = (flags & 0xFF) as i64;
    let data = body as *mut i8; // Data starts directly at body_ptr

    let val = match elem_size {
        1 => *(data as *const i8) as i64,
        2 => *(data as *const i16) as i64,
        4 => *(data as *const i32) as i64,
        _ => *(data as *const i64),
    };

    if len > 1 {
        memcpy(
            data as *mut _,
            data.offset(elem_size as isize) as *const _,
            ((len - 1) * elem_size) as usize,
        );
    }

    (*header).length = (len - 1) as u32;
    rt_update_array_cache(id, body, (len - 1) as i64, elem_size); // Pass body_ptr directly
    val
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_unshift(id: i64, val: i64) -> i64 {
    let mut current_id = id;
    if (current_id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (current_id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & ((ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16)) != 0 {
        printf(
            "RuntimeError: Cannot unshift a fixed-size or constant array.\n\0".as_ptr() as *const _,
        );
        exit(1);
    }

    let len = (*header).length as i64;
    current_id = rt_array_ensure_capacity(current_id, len + 1);

    let new_body = (current_id - HEAP_OFFSET) as *mut u8;
    let new_header = rt_get_header(new_body);
    let elem_size = ((*new_header).flags & 0xFF) as i64;

    let data = new_body as *mut i8; // Data starts directly at body_ptr
    if len > 0 {
        memmove(
            data.offset(elem_size as isize) as *mut _,
            data as *const _,
            (len * elem_size) as usize,
        );
    }

    match elem_size {
        1 => *(data as *mut i8) = val as i8,
        2 => *(data as *mut i16) = val as i16,
        4 => *(data as *mut i32) = val as i32,
        _ => *(data as *mut i64) = val,
    }

    (*new_header).length = (len + 1) as u32;
    rt_update_array_cache(current_id, new_body, len + 1, elem_size);
    current_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_splice(
    arr: i64,
    start: i64,
    delete_count: i64,
    items_arr: i64,
) -> i64 {
    let mut current_arr = arr;
    if (current_arr as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (current_arr - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & (ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16) != 0 {
        printf(
            "RuntimeError: Cannot splice a fixed-size or constant array.\n\0".as_ptr() as *const _,
        );
        exit(1);
    }

    let len = (*header).length as i64;
    let elem_size = (flags & 0xFF) as i64;

    let actual_start = if start < 0 {
        (len + start).max(0)
    } else {
        start.min(len)
    };
    let actual_delete = if delete_count < 0 {
        0
    } else {
        delete_count.min(len - actual_start)
    };

    let items_len = if (items_arr as u64) >= (HEAP_OFFSET as u64) {
        rt_len(items_arr)
    } else {
        0
    };
    let delta = items_len - actual_delete;

    // Create deleted array
    let _deleted_arr = rt_array_slice(current_arr, actual_start, actual_start + actual_delete);

    if delta > 0 {
        current_arr = rt_array_ensure_capacity(current_arr, len + delta);
    }

    // Refresh data pointer after potential realloc
    let new_body = (current_arr - HEAP_OFFSET) as *mut u8;
    let data = new_body as *mut i8; // Data starts directly at body_ptr

    if delta != 0 && actual_start + actual_delete < len {
        let src = data.offset(((actual_start + actual_delete) * elem_size) as isize);
        let dst = data.offset(((actual_start + items_len) * elem_size) as isize);
        let move_len = (len - (actual_start + actual_delete)) * elem_size;
        memmove(dst as *mut _, src as *const _, move_len as usize);
    }

    if items_len > 0 {
        let items_data = (items_arr - HEAP_OFFSET) as *mut u8 as *const i8; // Data starts directly at body_ptr
        let dst = data.offset((actual_start * elem_size) as isize);
        memcpy(
            dst as *mut _,
            items_data as *const _,
            (items_len * elem_size) as usize,
        );
    }

    let header = rt_get_header(new_body);
    (*header).length = (len + delta) as u32;
    rt_update_array_cache(current_arr, new_body, len + delta, elem_size);

    current_arr
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_indexOf(id: i64, val: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return -1;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let len = (*header).length as i64;
    let elem_size = ((*header).flags & 0xFF) as i64;
    let data = body as *const i8; // Data starts directly at body_ptr
    for i in 0..len {
        let p = data.offset((i * elem_size) as isize);
        let current_val = if elem_size == 1 {
            *(p as *const i8) as i64
        } else if elem_size == 4 {
            *(p as *const i32) as i64
        } else {
            *(p as *const i64)
        };
        if current_val == val {
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

    let elem_size = if (id1 as u64) >= (HEAP_OFFSET as u64) {
        let header = rt_get_header((id1 - HEAP_OFFSET) as *mut u8);
        ((*header).flags & 0xFF) as i64
    } else {
        8
    };

    let obj = rt_Array_constructor(0, new_len, elem_size);
    let data = ((obj - HEAP_OFFSET) as *mut u8) as *mut i8; // Data starts directly at body_ptr

    if len1 > 0 {
        let d1 = ((id1 - HEAP_OFFSET) as *mut u8) as *const i8; // Data starts directly at body_ptr
        memcpy(data as *mut _, d1 as *const _, (len1 * elem_size) as usize);
    }
    if len2 > 0 {
        let d2 = ((id2 - HEAP_OFFSET) as *mut u8) as *const i8; // Data starts directly at body_ptr
        memcpy(
            data.offset((len1 * elem_size) as isize) as *mut _,
            d2 as *const _,
            (len2 * elem_size) as usize,
        );
    }
    return obj;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Array_keys(id: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return rt_Array_new_fixed(0, 8);
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    if tag == TAG_OBJECT {
        return rt_Map_keys(id);
    }
    rt_Array_new_fixed(0, 8)
}

// Map layout: [size, capacity, keys_ptr, values_ptr, data_base] (after ObjectHeader)
// keys and values are parallel arrays of i64
// data_base: number of method-slot entries (set before user data)

#[no_mangle]
pub unsafe extern "C" fn rt_closure_new(capacity: i64) -> i64 {
    let raw = rt_Map_constructor(0);
    let body = (raw - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    (*header).type_id = TAG_FUNCTION as u16;
    raw
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_constructor(_this: i64) -> i64 {
    // If _this is already a TAG_OBJECT (called from class constructor), record data_base
    if (_this as u64) >= (HEAP_OFFSET as u64) {
        let body = (_this - HEAP_OFFSET) as *mut u8;
        let header = rt_get_header(body);
        if (*header).type_id == TAG_OBJECT as u16 {
            let ptr = body as *mut i64;
            // Record current size as data_base (method slots count)
            // Layout: [size, capacity, keys_ptr, values_ptr, data_base]
            let current_size = *ptr;
            *ptr.offset(4) = current_size; // data_base is at offset 4
            return _this;
        }
    }
    let body_ptr = gc_allocate(40); // 5 * 8 bytes for [size, capacity, keys_ptr, values_ptr, data_base]
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_OBJECT as u16;
    (*header).length = 0;

    let ptr = body_ptr as *mut i64;
    *ptr.offset(0) = 0; // size at offset 0
    let cap: i64 = 8;
    *ptr.offset(1) = cap; // capacity at offset 1
    let keys = calloc(cap as usize, 8) as *mut i64;
    let vals = calloc(cap as usize, 8) as *mut i64;

    *ptr.offset(2) = keys as i64; // keys_ptr at offset 2
    *ptr.offset(3) = vals as i64; // values_ptr at offset 3
    *ptr.offset(4) = 0; // data_base at offset 4

    (body_ptr as i64) + HEAP_OFFSET
}

unsafe fn map_key_eq(a: i64, b: i64) -> bool {
    if a == b {
        return true;
    }
    // Compare strings by value
    if (a as u64) >= (HEAP_OFFSET as u64) && (b as u64) >= (HEAP_OFFSET as u64) {
        let body_a = (a - HEAP_OFFSET) as *const u8;
        let body_b = (b - HEAP_OFFSET) as *const u8;
        let header_a = rt_get_header(body_a as *mut u8);
        let header_b = rt_get_header(body_b as *mut u8);
        let ta = (*header_a).type_id as i64;
        let tb = (*header_b).type_id as i64;

        // If one is a number and they aren't bitwise equal (handled above), they aren't equal
        if ta == TAG_FLOAT || tb == TAG_FLOAT {
            return false;
        }

        if ta == TAG_STRING && tb == TAG_STRING {
            return rt_str_equals(a, b) != 0;
        }
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_set(this: i64, key: i64, val: i64) {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return;
    }

    let mut v_this = this;
    let mut v_key = key;
    let mut v_val = val;
    rt_push_root(&mut v_this);
    rt_push_root(&mut v_key);
    rt_push_root(&mut v_val);

    // Clone key and val - these can trigger GC
    let key_owned = rt_clone(v_key);
    let mut v_key_owned = key_owned;
    rt_push_root(&mut v_key_owned);

    let val_owned = rt_clone(v_val);
    let mut v_val_owned = val_owned;
    rt_push_root(&mut v_val_owned);

    let body_ptr = (v_this - HEAP_OFFSET) as *mut u8;
    let ptr = body_ptr as *mut i64;
    let header = rt_get_header(body_ptr);

    if (*header).type_id != TAG_OBJECT as u16 && (*header).type_id != TAG_FUNCTION as u16 {
        rt_pop_roots(5);
        return;
    }

    let size = *ptr.offset(0);
    let cap = *ptr.offset(1);
    let mut keys = *ptr.offset(2) as *mut i64;
    let mut vals = *ptr.offset(3) as *mut i64;

    // Check if key exists
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), v_key_owned) {
            // Free old value before replacing it
            rt_free(*vals.offset(i as isize));
            *vals.offset(i as isize) = v_val_owned;
            rt_pop_roots(5);
            return;
        }
    }

    // Need to grow?
    if size >= cap {
        let new_cap = cap * 2;
        let new_keys = realloc(keys as *mut _, (new_cap * 8) as usize) as *mut i64;
        let new_vals = realloc(vals as *mut _, (new_cap * 8) as usize) as *mut i64;

        // Zero out the newly allocated capacity
        for i in cap..new_cap {
            *new_keys.offset(i as isize) = 0;
            *new_vals.offset(i as isize) = 0;
        }

        let current_ptr = (v_this - HEAP_OFFSET) as *mut i64;
        *current_ptr.offset(1) = new_cap;
        *current_ptr.offset(2) = new_keys as i64;
        *current_ptr.offset(3) = new_vals as i64;

        *new_keys.offset(size as isize) = v_key_owned;
        *new_vals.offset(size as isize) = v_val_owned;
    } else {
        *keys.offset(size as isize) = v_key_owned;
        *vals.offset(size as isize) = v_val_owned;
    }
    let current_ptr = (v_this - HEAP_OFFSET) as *mut i64;
    *current_ptr.offset(0) = size + 1;
    rt_pop_roots(5);
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_get(this: i64, key: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let size = *ptr;
    let keys = *ptr.offset(2) as *const i64;
    let vals = *ptr.offset(3) as *const i64;
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            return *vals.offset(i as isize);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_has(this: i64, key: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let size = *ptr;
    let keys = *ptr.offset(2) as *const i64;
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_delete(this: i64, key: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let size = *ptr;
    let keys = *ptr.offset(2) as *mut i64;
    let vals = *ptr.offset(3) as *mut i64;
    for i in 0..size {
        if map_key_eq(*keys.offset(i as isize), key) {
            // Shift remaining elements
            for j in i..(size - 1) {
                *keys.offset(j as isize) = *keys.offset((j + 1) as isize);
                *vals.offset(j as isize) = *vals.offset((j + 1) as isize);
            }
            *ptr = size - 1;
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_keys(this: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return rt_Array_new_fixed(0, 8);
    }
    let mut v_this = this;
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut v_this);
    rt_push_root(&mut result);

    let ptr = (v_this - HEAP_OFFSET) as *const i64;
    let size = *ptr;
    let data_base = *ptr.offset(4);
    let keys = *ptr.offset(2) as *const i64;
    for i in data_base..size {
        result = rt_array_push(result, *keys.offset(i as isize));
    }
    rt_pop_roots(2);
    result
}

// Map layout: [TAG_OBJECT, size, capacity, keys_ptr, values_ptr, data_base]
// keys and values are parallel arrays of i64
// data_base: number of method-slot entries (set before user data)

#[no_mangle]
pub unsafe extern "C" fn rt_Map_clear(this: i64) {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *mut i64;
    let data_base = *ptr.offset(4);
    *ptr = data_base; // Reset size to data_base (keep method slots)
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_size(this: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    let total = *ptr;
    let data_base = *ptr.offset(4);
    total - data_base
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_size(this: i64) -> i64 {
    rt_Map_size(this)
}

// Set uses the same parallel-array approach as Map, with dummy values

#[no_mangle]
pub unsafe extern "C" fn rt_Set_constructor(_this: i64) {
    // Record data_base for size tracking (same as Map constructor logic)
    if (_this as u64) >= (HEAP_OFFSET as u64) {
        let ptr = (_this - HEAP_OFFSET) as *mut i64;
        let current_size = *ptr;
        *ptr.offset(4) = current_size;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_add(this: i64, val: i64) {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    // If it has a TAG_OBJECT layout, use map operations
    if *ptr == TAG_OBJECT {
        rt_Map_set(this, val, 1); // value doesn't matter for Set
        return;
    }
    // Fallback: treat as inline array of values at offset 3
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_has(this: i64, val: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    if *ptr == TAG_OBJECT {
        return rt_Map_has(this, val);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_delete(this: i64, val: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let ptr = (this - HEAP_OFFSET) as *const i64;
    if *ptr == TAG_OBJECT {
        return rt_Map_delete(this, val);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Set_values(this: i64) -> i64 {
    if (this as u64) >= (HEAP_OFFSET as u64) {
        let ptr = (this - HEAP_OFFSET) as *const i64;
        if *ptr == TAG_OBJECT {
            return rt_Map_keys(this); // Set stores values as keys
        }
    }
    rt_Array_new_fixed(0, 8)
}

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
    let mut v_path = path;
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut v_path);
    rt_push_root(&mut result);

    if let Some(p) = i64_to_rust_str(v_path) {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    let mut name_id = rt_box_string(name.as_ptr() as *const _);
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
pub unsafe extern "C" fn rt_Map_values(this: i64) -> i64 {
    if (this as u64) < (HEAP_OFFSET as u64) {
        return rt_Array_new_fixed(0, 8);
    }
    let mut v_this = this;
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut v_this);
    rt_push_root(&mut result);

    let ptr = (v_this - HEAP_OFFSET) as *const i64;
    let size = *ptr;
    let data_base = *ptr.offset(4);
    let vals = *ptr.offset(3) as *const i64;
    for i in data_base..size {
        result = rt_array_push(result, *vals.offset(i as isize));
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
        let mut arg_id = rt_box_string(arg.as_ptr() as *const _);
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

#[no_mangle]
pub unsafe extern "C" fn rt_String_toUpperCase(arg_s: i64) -> i64 {
    let mut s = arg_s;
    rt_push_root(&mut s);

    let res = if let Some((data, len)) = get_str_parts(s) {
        let body_ptr = gc_allocate(len as usize + 1);
        let header = rt_get_header(body_ptr);
        (*header).type_id = TAG_STRING as u16;
        (*header).length = len as u32;

        let dst = body_ptr as *mut u8;
        for i in 0..len {
            let ch = *data.offset(i as isize);
            *dst.offset(i as isize) = if ch >= b'a' && ch <= b'z' {
                ch - 32
            } else {
                ch
            };
        }
        *dst.offset(len as isize) = 0;
        (body_ptr as i64) + HEAP_OFFSET
    } else {
        s
    };

    rt_pop_roots(1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_toLowerCase(arg_s: i64) -> i64 {
    let mut s = arg_s;
    rt_push_root(&mut s);

    let res = if let Some((data, len)) = get_str_parts(s) {
        let body_ptr = gc_allocate(len as usize + 1);
        let header = rt_get_header(body_ptr);
        (*header).type_id = TAG_STRING as u16;
        (*header).length = len as u32;

        let dst = body_ptr as *mut u8;
        for i in 0..len {
            let ch = *data.offset(i as isize);
            *dst.offset(i as isize) = if ch >= b'A' && ch <= b'Z' {
                ch + 32
            } else {
                ch
            };
        }
        *dst.offset(len as isize) = 0;
        (body_ptr as i64) + HEAP_OFFSET
    } else {
        s
    };

    rt_pop_roots(1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_trim(s_id: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s_id) {
        let mut start = 0;
        while start < len && (*data.add(start as usize) as char).is_whitespace() {
            start += 1;
        }
        let mut end = len;
        while end > start && (*data.add((end - 1) as usize) as char).is_whitespace() {
            end -= 1;
        }
        return rt_String_substring(s_id, start, end);
    }
    s_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_trimStart(s_id: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s_id) {
        let mut start = 0;
        while start < len && (*data.add(start as usize) as char).is_whitespace() {
            start += 1;
        }
        return rt_String_substring(s_id, start, len);
    }
    s_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_trimEnd(s_id: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s_id) {
        let mut end = len;
        while end > 0 && (*data.add((end - 1) as usize) as char).is_whitespace() {
            end -= 1;
        }
        return rt_String_substring(s_id, 0, end);
    }
    s_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_substring(arg_s: i64, start: i64, end: i64) -> i64 {
    let mut s = arg_s;
    rt_push_root(&mut s);

    let res = if let Some((_, len)) = get_str_parts(s) {
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
        let (real_start, real_end) = if s0 > e0 { (e0, s0) } else { (s0, e0) };
        new_string_from_parts(s, real_start, real_end - real_start)
    } else {
        s
    };

    rt_pop_roots(1);
    res
}

// Internal helper to create a string from parts of another, safe for GC moves
unsafe fn new_string_from_parts(source_s: i64, offset: i64, len: i64) -> i64 {
    if len <= 0 {
        return rt_box_string("\0".as_ptr() as *const _);
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
pub unsafe extern "C" fn rt_String_split(s: i64, sep: i64) -> i64 {
    let mut v_s = s;
    let mut v_sep = sep;
    let mut result = rt_Array_new_fixed(0, 8);

    rt_push_root(&mut v_s);
    rt_push_root(&mut v_sep);
    rt_push_root(&mut result);

    let s_len = rt_len(v_s);
    let sep_len = rt_len(v_sep);

    if result != 0 && s_len >= 0 && sep_len >= 0 {
        if sep_len == 0 {
            for i in 0..s_len {
                let part = new_string_from_parts(v_s, i, 1);
                result = rt_array_push(result, part);
            }
        } else {
            let mut last = 0i64;
            let mut i = 0i64;
            while i <= s_len - sep_len {
                let mut matched = true;
                // Re-resolve data pointers as GC can move strings
                if let (Some((s_data, _)), Some((sep_data, _))) =
                    (get_str_parts(v_s), get_str_parts(v_sep))
                {
                    for j in 0..sep_len {
                        if *s_data.offset((i + j) as isize) != *sep_data.offset(j as isize) {
                            matched = false;
                            break;
                        }
                    }
                } else {
                    matched = false;
                }

                if matched {
                    let part = new_string_from_parts(v_s, last, i - last);
                    result = rt_array_push(result, part);
                    last = i + sep_len;
                    i = last;
                } else {
                    i += 1;
                }
            }
            let part = new_string_from_parts(v_s, last, s_len - last);
            result = rt_array_push(result, part);
        }
    }

    rt_pop_roots(3);
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
pub unsafe extern "C" fn rt_String_includes(s: i64, search: i64) -> i64 {
    if rt_String_indexOf(s, search) >= 0 {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_startsWith(s: i64, prefix: i64) -> i64 {
    if let (Some((s_data, s_len)), Some((p_data, p_len))) =
        (get_str_parts(s), get_str_parts(prefix))
    {
        if p_len > s_len {
            return BOOL_FALSE;
        }
        for i in 0..p_len {
            if *s_data.offset(i as isize) != *p_data.offset(i as isize) {
                return BOOL_FALSE;
            }
        }
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_endsWith(s: i64, suffix: i64) -> i64 {
    if let (Some((s_data, s_len)), Some((sf_data, sf_len))) =
        (get_str_parts(s), get_str_parts(suffix))
    {
        if sf_len > s_len {
            return BOOL_FALSE;
        }
        let offset = s_len - sf_len;
        for i in 0..sf_len {
            if *s_data.offset((offset + i) as isize) != *sf_data.offset(i as isize) {
                return BOOL_FALSE;
            }
        }
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_padStart(s: i64, len: i64, pad: i64) -> i64 {
    if let (Some((s_data, s_len)), Some((pad_data, pad_len))) =
        (get_str_parts(s), get_str_parts(pad))
    {
        if s_len >= len || pad_len == 0 {
            return s; // no padding needed
        }
        let diff = (len - s_len) as usize;
        let mut new_str = Vec::with_capacity(len as usize + 1);
        let mut p_idx = 0;
        for _ in 0..diff {
            new_str.push(*pad_data.offset(p_idx as isize));
            p_idx = (p_idx + 1) % (pad_len as usize);
        }
        for i in 0..s_len {
            new_str.push(*s_data.offset(i as isize));
        }
        new_str.push(0);
        rt_box_string(new_str.as_ptr() as *const _)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_padEnd(s: i64, len: i64, pad: i64) -> i64 {
    if let (Some((s_data, s_len)), Some((pad_data, pad_len))) =
        (get_str_parts(s), get_str_parts(pad))
    {
        if s_len >= len || pad_len == 0 {
            return s;
        }
        let diff = (len - s_len) as usize;
        let mut new_str = Vec::with_capacity(len as usize + 1);
        for i in 0..s_len {
            new_str.push(*s_data.offset(i as isize));
        }
        let mut p_idx = 0;
        for _ in 0..diff {
            new_str.push(*pad_data.offset(p_idx as isize));
            p_idx = (p_idx + 1) % (pad_len as usize);
        }
        new_str.push(0);
        rt_box_string(new_str.as_ptr() as *const _)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_repeat(s: i64, count: i64) -> i64 {
    if count <= 0 {
        return rt_box_string("\0".as_ptr() as *const _);
    }
    if let Some((s_data, s_len)) = get_str_parts(s) {
        if s_len == 0 {
            return s;
        }
        let total_len = (s_len * count) as usize;
        let mut new_str = Vec::with_capacity(total_len + 1);
        for _ in 0..count {
            for i in 0..s_len {
                new_str.push(*s_data.offset(i as isize));
            }
        }
        new_str.push(0);
        rt_box_string(new_str.as_ptr() as *const _)
    } else {
        s
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_String_replace(s: i64, search: i64, replace: i64) -> i64 {
    if let (Some((s_data, s_len)), Some((sh_data, sh_len)), Some((r_data, r_len))) = (
        get_str_parts(s),
        get_str_parts(search),
        get_str_parts(replace),
    ) {
        if sh_len == 0 {
            return s;
        }
        let s_slice = std::slice::from_raw_parts(s_data, s_len as usize);
        let sh_slice = std::slice::from_raw_parts(sh_data, sh_len as usize);
        if let Some(pos) = s_slice.windows(sh_len as usize).position(|w| w == sh_slice) {
            let mut new_str = Vec::new();
            new_str.extend_from_slice(&s_slice[..pos]);
            new_str.extend_from_slice(std::slice::from_raw_parts(r_data, r_len as usize));
            new_str.extend_from_slice(&s_slice[pos + sh_len as usize..]);
            new_str.push(0);
            rt_box_string(new_str.as_ptr() as *const _)
        } else {
            s
        }
    } else {
        s
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

use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicI64, Ordering};

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
pub unsafe extern "C" fn rt_math_sqrt(x: i64) -> i64 {
    let x_f = f64::from_bits(x as u64);
    let res = x_f.sqrt();
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_math_pow(base: i64, exp: i64) -> i64 {
    let base_f = f64::from_bits(base as u64);
    let exp_f = f64::from_bits(exp as u64);
    let res = base_f.powf(exp_f);
    res.to_bits() as i64
}

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
pub unsafe extern "C" fn rt_to_number_v2(v: i64) -> i64 {
    // Unbox Any, convert it to f64 using standard rules, then return raw bits
    // instead of boxing it back in TAG_FLOAT. This allows LLVM to `bitcast` it directly to `double`.
    let f = rt_to_number(v); // Returns a f64
    f.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_array_get_fast(id: i64, index: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    let len = (*header).length as i64;
    let elem_size = ((*header).flags & 0xFF) as usize;

    if index < 0 || index >= len {
        printf(
            "FATAL: Array index out of bounds: %lld / %lld\n\0".as_ptr() as *const _,
            index,
            len,
        );
        exit(1);
    }

    let res = if tag == TAG_STRING {
        let ch = *body.offset(index as isize);
        let buf = [ch, 0];
        rt_box_string(buf.as_ptr() as *const _)
    } else {
        match elem_size {
            1 => *body.offset(index as isize) as i64,
            2 => *(body.offset((index * 2) as isize) as *const i16) as i64,
            4 => *(body.offset((index * 4) as isize) as *const i32) as i64,
            _ => *(body.offset((index * 8) as isize) as *const i64),
        }
    };
    res
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
            let val = rt_array_get_fast(this, idx);
            return val;
        }
    }
    let val = rt_Map_get(this, _key);
    return val;
}

extern "C" {
    fn tejx_main();
    fn rt_init_types();
}

#[no_mangle]
pub unsafe extern "C" fn tejx_runtime_main(_argc: i32, _argv: *mut *mut u8) -> i32 {
    rt_init_gc();
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
    let mut c = closure;
    rt_push_root(&mut c);
    let key = rt_box_string("ptr\0".as_ptr() as *const _);
    let mut res = rt_Map_get(c, key);
    if res >= HEAP_OFFSET {
        let body = (res - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            res = *(body as *mut i64);
        }
    }
    if res == 0 {
        printf("Runtime Error: Closure function pointer is NULL\n\0".as_ptr() as *const _);
    }
    rt_pop_roots(1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_env(closure: i64) -> i64 {
    let mut c = closure;
    rt_push_root(&mut c);
    let key = rt_box_string("env\0".as_ptr() as *const _);
    let res = rt_Map_get(c, key);
    // env can also be boxed? usually not, but let's be safe if it's an object ID
    rt_pop_roots(1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure(closure: i64, arg: i64) -> i64 {
    let mut c = closure;
    let mut a = arg;
    rt_push_root(&mut c);
    rt_push_root(&mut a);

    let mut ptr_val = 0;
    let mut env = 0;

    if (c as u64) >= (HEAP_OFFSET as u64) {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
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
    // Expected signature: fn(env: i64, arg: i64) -> i64
    let func: unsafe extern "C" fn(i64, i64) -> i64 = std::mem::transmute::<
        *const (),
        unsafe extern "C" fn(i64, i64) -> i64,
    >(raw_func_ptr as *const ());

    let result = func(env, a);

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

    let mut ptr_val = 0;
    let mut env = 0;
    let is_raw_ptr = (c as u64) < (HEAP_OFFSET as u64) && c != 0;

    if !is_raw_ptr {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
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
        rt_pop_roots(1);
        return 0;
    }

    let result = if is_raw_ptr {
        // Raw function pointers explicitly have NO env argument.
        let func: unsafe extern "C" fn() -> i64 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn() -> i64,
        >(raw_func_ptr as *const ());
        func()
    } else {
        // Heap closures expect at least an env argument.
        let func: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(i64) -> i64,
        >(raw_func_ptr as *const ());
        func(env)
    };

    rt_pop_roots(1);
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
pub unsafe extern "C" fn rt_to_boolean(val: i64) -> i64 {
    if val == 0 || val == BOOL_FALSE { 0 } else { 1 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_str_equals(a: i64, b: i64) -> i32 {
    if a == b {
        return 1;
    }
    let parts_a = get_str_parts(a);
    let parts_b = get_str_parts(b);

    if let (Some((d1, l1)), Some((d2, l2))) = (parts_a, parts_b) {
        if l1 != l2 {
            return 0;
        }
        if l1 == 0 {
            return 1;
        }
        if memcmp(d1 as *const _, d2 as *const _, l1 as usize) == 0 {
            1
        } else {
            0
        }
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_str_concat_v2(a_id: i64, b_id: i64) -> i64 {
    let mut val_a = a_id;
    let mut val_b = b_id;
    rt_push_root(&mut val_a);
    rt_push_root(&mut val_b);

    let parts_a = get_str_parts(val_a);
    let parts_b = get_str_parts(val_b);

    let res = if let (Some((_d1, l1)), Some((_d2, l2))) = (parts_a, parts_b) {
        let total_len = l1 + l2;
        let body_ptr = gc_allocate(total_len as usize + 1);

        // RE-RESOLVE after potential GC
        if let (Some((d1_new, _)), Some((d2_new, _))) = (get_str_parts(val_a), get_str_parts(val_b))
        {
            let header = rt_get_header(body_ptr);
            (*header).type_id = TAG_STRING as u16;
            (*header).length = total_len as u32;

            memcpy(body_ptr as *mut _, d1_new as *const _, l1 as usize);
            memcpy(
                body_ptr.add(l1 as usize) as *mut _,
                d2_new as *const _,
                l2 as usize,
            );
            *body_ptr.add(total_len as usize) = 0;

            (body_ptr as i64) + HEAP_OFFSET
        } else {
            0
        }
    } else if parts_a.is_some() {
        rt_clone(val_a)
    } else if parts_b.is_some() {
        rt_clone(val_b)
    } else {
        rt_box_string("\0".as_ptr() as *const _)
    };

    rt_pop_roots(2);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_Map_get_ref(this: i64, key: i64) -> i64 {
    rt_Map_get(this, key)
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

    let mut unboxed_val = val;
    if (val as u64) >= (HEAP_OFFSET as u64) {
        let v_ptr = (val - HEAP_OFFSET) as *mut i64;
        if *v_ptr == TAG_FLOAT {
            let f = *(v_ptr.offset(1) as *const f64);
            if f.fract() == 0.0 && f >= 0.0 && f < (HEAP_OFFSET as f64) {
                unboxed_val = f as i64;
            }
        }
    }

    if this >= HEAP_OFFSET {
        let tag = *((this - HEAP_OFFSET) as *mut i64);
        if tag == TAG_ARRAY {
            let idx = if _key >= 0 && _key < HEAP_OFFSET {
                _key
            } else {
                return;
            };
            rt_array_set_fast(this, idx, unboxed_val);
            return;
        }
    }
    rt_Map_set(this, _key, unboxed_val);
}

// --- Atomic Operations ---
// Atomic objects store an AtomicI64 at offset 1 (as a boxed pointer)

// Redundant imports removed

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
    let result = (obj as i64) + HEAP_OFFSET;
    printf(
        "DEBUG: rt_atomic_new val=%lld ptr=%p result=%lld\n\0".as_ptr() as *const _,
        val,
        obj,
        result,
    );
    result
}

// --- Mutex Operations ---
// Mutex objects store a Box<std::sync::Mutex<()>> pointer at offset 1

// Redundant StdMutex import removed

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
    *obj = TAG_OBJECT;
    *obj.offset(1) = 0; // State: Pending
    *obj.offset(2) = 0; // Value
    *obj.offset(3) = rt_Array_new_fixed(0, 8); // Callbacks array (empty)
    (obj as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_resolve(p: i64, val: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let ptr = (p - HEAP_OFFSET) as *mut i64;
    if *ptr != TAG_OBJECT {
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
    if *ptr != TAG_OBJECT {
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
    if *ptr != TAG_OBJECT {
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
        let body = (id - HEAP_OFFSET) as *mut u8;
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
pub unsafe extern "C" fn rt_instanceof(obj: i64, _class_name: i64) -> i32 {
    if obj < HEAP_OFFSET {
        return 0;
    }
    // Simple mock for now: Check if it's an object/map or array based on tag
    let ptr = (obj - HEAP_OFFSET) as *const i64;
    let tag = *ptr;
    if tag == TAG_OBJECT || tag == TAG_ARRAY {
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
pub unsafe extern "C" fn rt_eq(a: i64, b: i64) -> i64 {
    if a == b {
        return BOOL_TRUE;
    }
    if a >= HEAP_OFFSET && b >= HEAP_OFFSET {
        let ptr_a = (a - HEAP_OFFSET) as *const i64;
        let ptr_b = (b - HEAP_OFFSET) as *const i64;
        let tag_a = *ptr_a;
        let tag_b = *ptr_b;
        if tag_a == TAG_STRING && tag_b == TAG_STRING {
            if rt_str_equals(a, b) != 0 {
                return BOOL_TRUE;
            } else {
                return BOOL_FALSE;
            }
        }
        if tag_a == TAG_FLOAT && tag_b == TAG_FLOAT {
            let na = *(ptr_a.offset(1) as *const f64);
            let nb = *(ptr_b.offset(1) as *const f64);
            return if na == nb { BOOL_TRUE } else { BOOL_FALSE };
        }
    }
    printf(
        "DEBUG: rt_eq fallback to BOOL_FALSE for tags %d and %d\n\0".as_ptr() as *const _,
        if a >= HEAP_OFFSET {
            *((a - HEAP_OFFSET) as *const i64)
        } else {
            -1
        },
        if b >= HEAP_OFFSET {
            *((b - HEAP_OFFSET) as *const i64)
        } else {
            -1
        },
    );
    BOOL_FALSE
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_equal(a: i64, b: i64) -> i64 {
    if a == b { 1 } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_ne(a: i64, b: i64) -> i64 {
    if a != b { 1 } else { 0 }
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
    let is_gc = if val >= HEAP_OFFSET {
        rt_is_gc_ptr((val - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };

    if !is_gc {
        if val >= BOOL_FALSE && val <= BOOL_TRUE {
            return rt_box_string("bool\0".as_ptr() as *const _);
        } else {
            return rt_box_string("int\0".as_ptr() as *const _);
        }
    } else {
        let body = (val - HEAP_OFFSET) as *mut u8;
        let header = rt_get_header(body);
        let tag = (*header).type_id as i64;
        if tag == TAG_STRING {
            return rt_box_string("string\0".as_ptr() as *const _);
        } else if tag == TAG_FUNCTION {
            return rt_box_string("function\0".as_ptr() as *const _);
        } else if tag == TAG_ARRAY {
            return rt_box_string("array\0".as_ptr() as *const _);
        } else if tag == TAG_OBJECT {
            return rt_box_string("object\0".as_ptr() as *const _);
        } else if tag == TAG_BOOLEAN {
            return rt_box_string("bool\0".as_ptr() as *const _);
        } else if tag == TAG_FLOAT {
            return rt_box_string("float\0".as_ptr() as *const _);
        } else if tag == TAG_INT {
            return rt_box_string("int\0".as_ptr() as *const _);
        } else if tag == TAG_CHAR {
            return rt_box_string("char\0".as_ptr() as *const _);
        } else {
            return rt_box_string("object\0".as_ptr() as *const _);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_sizeof(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return 8; // Non-GC primitive
    }
    let body_ptr = (val - HEAP_OFFSET) as *mut u8;
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
    if p < HEAP_OFFSET {
        return p;
    }
    let ptr = (p - HEAP_OFFSET) as *mut i64;
    if *ptr != TAG_OBJECT {
        return p;
    }

    // Pump the event loop until the promise is resolved/rejected
    while *ptr.offset(1) == 0 {
        event_loop::tejx_run_event_loop_step();
    }

    // Return the resolved value (or error, which should ideally be thrown, but we just return it for now)
    return *ptr.offset(2);
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

pub mod event_loop;
pub use event_loop::*;
