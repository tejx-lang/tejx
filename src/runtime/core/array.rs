use super::*; // Extracted \n
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
    let mut a = arr;
    let mut s_id = sep;
    rt_push_root(&mut a);
    rt_push_root(&mut s_id);

    let arr_len = rt_len(a);
    if arr_len == 0 {
        let res = rt_string_from_c_str("\0".as_ptr() as *const _);
        rt_pop_roots(2);
        return res;
    }
    // Get separator string
    let (sep_data, sep_len) = get_str_parts(s_id).unwrap_or(("\0".as_ptr(), 0));
    // First pass: calculate total length
    let mut total_len: i64 = 0;
    for i in 0..arr_len {
        let elem = rt_array_get_fast(a, i);
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
        let elem = rt_array_get_fast(a, i);
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
    rt_pop_roots(2);
    res
}
#[no_mangle]
pub unsafe extern "C" fn rt_array_slice(arr: i64, start: i64, end: i64) -> i64 {
    let mut a = arr;
    rt_push_root(&mut a);

    let arr_len = rt_len(a);
    let s = if start < 0 {
        let v = arr_len + start;
        if v < 0 {
            0
        } else {
            v
        }
    } else if start > arr_len {
        arr_len
    } else {
        start
    };
    let e = if end < 0 {
        let v = arr_len + end;
        if v < 0 {
            0
        } else {
            v
        }
    } else if end > arr_len {
        arr_len
    } else {
        end
    };

    let ptr = if a >= HEAP_OFFSET {
        (a - HEAP_OFFSET) as *mut i64
    } else {
        std::ptr::null_mut()
    };
    let elem_size = if !ptr.is_null() {
        let body = (a - HEAP_OFFSET) as *mut u8;
        let header = rt_get_header(body);
        ((*header).flags & 0xFF) as i64
    } else {
        8
    };

    if s >= e {
        let res = rt_Array_constructor(0, 0, elem_size);
        rt_pop_roots(1);
        return res;
    }

    let new_len = e - s;
    let result = rt_Array_constructor(0, new_len, elem_size);
    if !ptr.is_null() {
        let src_data = (a - HEAP_OFFSET) as *const i8;
        let dst_data = (result - HEAP_OFFSET) as *mut i8;
        memcpy(
            dst_data as *mut _,
            src_data.offset((s * elem_size) as isize) as *const _,
            (new_len * elem_size) as usize,
        );
    }
    rt_pop_roots(1);
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
        rt_throw_runtime_error("RuntimeError: Cannot reverse a constant array.");
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
        rt_throw_runtime_error("RuntimeError: Cannot sort a constant array.");
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
        rt_throw_runtime_error("RuntimeError: Cannot fill a constant array.");
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
#[no_mangle]
pub unsafe extern "C" fn rt_array_ensure_capacity(id: i64, required: i64) -> i64 {
    let mut current_id = rt_resolve_array_id(id);
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
        rt_throw_runtime_error("RuntimeError: Cannot resize a fixed-size array.");
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
    ARRAY_FORWARD_ACTIVE.store(true, Ordering::Release);
    ARRAY_FORWARD.lock().unwrap().insert(current_id, res);
    res
}
#[no_mangle]
pub unsafe extern "C" fn rt_array_new(len: i64, elem_size: i64) -> i64 {
    rt_Array_new(len, elem_size)
}
#[no_mangle]
pub unsafe extern "C" fn rt_array_push(id: i64, val: i64) -> i64 {
    let mut current_id = rt_resolve_array_id(id);
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
        rt_throw_runtime_error("RuntimeError: Cannot push to a constant array.");
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
        _ => {
            *(data.offset((len * elem_size) as isize) as *mut i64) = current_val;
            rt_write_barrier(current_id, current_val);
        }
    }

    (*new_header).length = (len + 1) as u32;
    rt_update_array_cache(current_id, new_body, (len + 1) as i64, elem_size);

    rt_pop_roots(2);
    current_id
}
#[no_mangle]
pub unsafe extern "C" fn rt_array_pop(id: i64) -> i64 {
    let id = rt_resolve_array_id(id);
    if (id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & ((ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16)) != 0 {
        rt_throw_runtime_error("RuntimeError: Cannot pop from a fixed-size or constant array.");
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
pub unsafe extern "C" fn rt_array_set_fast(mut id: i64, index: i64, val: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        let msg = rt_string_from_c_str(
            "RuntimeError: Null pointer dereference in array assignment\0".as_ptr() as *const _,
        );
        crate::event_loop::tejx_throw(msg);
    }
    let mut id = id;
    let mut body: *mut u8;
    let mut header: *mut ObjectHeader;
    let mut flags: u16;

    if id == LAST_ID && !LAST_PTR.is_null() {
        body = LAST_PTR;
        header = rt_get_header(body);
        flags = (*header).flags;
    } else {
        id = rt_resolve_array_id(id);
        if (id as u64) < (HEAP_OFFSET as u64) {
            let msg = rt_string_from_c_str(
                "RuntimeError: Null pointer dereference in array assignment\0".as_ptr() as *const _,
            );
            crate::event_loop::tejx_throw(msg);
        }
        body = (id - HEAP_OFFSET) as *mut u8;
        header = rt_get_header(body);
        flags = (*header).flags;
    }
    if (flags & (ARRAY_FLAG_CONSTANT as u16)) != 0 {
        rt_throw_runtime_error("RuntimeError: Cannot set element in a constant array.");
    }

    let mut len = (*header).length as i64;
    if index < 0 || index >= len {
        if (flags & (ARRAY_FLAG_FIXED as u16)) != 0 {
            let msg_str = format!(
                "RuntimeError: Array index {} out of bounds (length {}) in assignment\0",
                index, len
            );
            let msg = rt_string_from_c_str(msg_str.as_ptr() as *const _);
            crate::event_loop::tejx_throw(msg);
        }
        let new_len = index + 1;
        id = rt_array_ensure_capacity(id, new_len);
        if (id as u64) < (HEAP_OFFSET as u64) {
            return id;
        }
        body = (id - HEAP_OFFSET) as *mut u8;
        header = rt_get_header(body);
        flags = (*header).flags;
        let elem_size = (flags & 0xFF) as i64;
        if new_len > len {
            let data = body as *mut u8;
            let byte_start = (len * elem_size) as isize;
            let byte_len = ((new_len - len) * elem_size) as usize;
            std::ptr::write_bytes(data.offset(byte_start), 0, byte_len);
        }
        (*header).length = new_len as u32;
        len = new_len;
        rt_update_array_cache(id, body, len, elem_size);
    }

    let data = body as *mut i8; // Data starts directly at body_ptr
    let elem_size = (flags & 0xFF) as i64;
    match elem_size {
        1 => *(data.offset(index as isize) as *mut i8) = val as i8,
        2 => *(data.offset((index * 2) as isize) as *mut i16) = val as i16,
        4 => *(data.offset((index * 4) as isize) as *mut i32) = val as i32,
        _ => {
            *(data.offset((index * 8) as isize) as *mut i64) = val;
            rt_write_barrier(id, val);
        }
    }
    rt_update_array_cache(id, body, len, elem_size);
    id
}
#[no_mangle]
pub unsafe extern "C" fn rt_array_shift(id: i64) -> i64 {
    let id = rt_resolve_array_id(id);
    if (id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & ((ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16)) != 0 {
        rt_throw_runtime_error("RuntimeError: Cannot shift a fixed-size or constant array.");
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
    let mut current_id = rt_resolve_array_id(id);
    if (current_id as u64) < (HEAP_OFFSET as u64) {
        return 0;
    }
    let body = (current_id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let flags = (*header).flags;
    if (flags & ((ARRAY_FLAG_FIXED | ARRAY_FLAG_CONSTANT) as u16)) != 0 {
        rt_throw_runtime_error("RuntimeError: Cannot unshift a fixed-size or constant array.");
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
        rt_throw_runtime_error("RuntimeError: Cannot splice a fixed-size or constant array.");
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
    let id = rt_resolve_array_id(id);
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
pub unsafe extern "C" fn rt_array_get_fast(id: i64, index: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        let msg = rt_string_from_c_str(
            "RuntimeError: Null pointer dereference in array access\0".as_ptr() as *const _,
        );
        crate::event_loop::tejx_throw(msg);
    }
    if id == LAST_ID && !LAST_PTR.is_null() {
        if index < 0 || index >= LAST_LEN {
            return 0;
        }
        let body = LAST_PTR;
        let elem_size = LAST_ELEM_SIZE;
        return if elem_size == 1 {
            *(body.offset(index as isize) as *mut i8) as i64
        } else if elem_size == 2 {
            *(body.offset((index * 2) as isize) as *mut i16) as i64
        } else if elem_size == 4 {
            *(body.offset((index * 4) as isize) as *mut i32) as i64
        } else {
            *(body.offset((index * 8) as isize) as *const i64)
        };
    }

    let id = rt_resolve_array_id(id);
    if (id as u64) < (HEAP_OFFSET as u64) {
        let msg = rt_string_from_c_str(
            "RuntimeError: Null pointer dereference in array access\0".as_ptr() as *const _,
        );
        crate::event_loop::tejx_throw(msg);
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let len = (*header).length as i64;
    let flags = (*header).flags;

    if index < 0 || index >= len {
        return 0;
    }

    let elem_size = (flags & 0xFF) as i64;
    let res = if elem_size == 1 {
        *(body.offset(index as isize) as *mut i8) as i64
    } else if elem_size == 2 {
        *(body.offset((index * 2) as isize) as *mut i16) as i64
    } else if elem_size == 4 {
        *(body.offset((index * 4) as isize) as *mut i32) as i64
    } else {
        *(body.offset((index * 8) as isize) as *const i64)
    };

    rt_update_array_cache(id, body, len, elem_size);
    res
}
#[no_mangle]
pub unsafe extern "C" fn rt_Array_constructor(_this: i64, size: i64, elem_size: i64) -> i64 {
    rt_Array_constructor_v2(_this, size, elem_size, 0)
}
#[no_mangle]
pub unsafe extern "C" fn rt_Array_constructor_v2(
    _this: i64,
    size_or_arr: i64,
    elem_size: i64,
    flags: i64,
) -> i64 {
    let size = if (size_or_arr as u64) >= (HEAP_OFFSET as u64) {
        rt_len(size_or_arr)
    } else {
        size_or_arr
    };

    let cap = if size == 0 { 4 } else { size };
    let mut actual_elem_size = elem_size;
    if actual_elem_size == 0 {
        actual_elem_size = 8; // Default to 8 if unknown
    }

    let total_size = (cap * actual_elem_size) as usize;
    let body_ptr = gc_allocate(total_size);
    let header = rt_get_header(body_ptr);

    (*header).type_id = TAG_ARRAY as u16;
    (*header).length = size as u32;
    (*header).capacity = cap as u32;
    // Store elem_size in lower 8 bits of flags
    (*header).flags = (flags as u16 & 0xFF00) | (actual_elem_size as u16 & 0x00FF);

    if (size_or_arr as u64) >= (HEAP_OFFSET as u64) {
        let src_body = (size_or_arr - HEAP_OFFSET) as *mut u8;
        let src_header = rt_get_header(src_body);
        let src_elem_size = ((*src_header).flags & 0xFF) as i64;

        if src_elem_size == actual_elem_size {
            let data_size = (size * actual_elem_size) as usize;
            memcpy(body_ptr as *mut _, src_body as *const _, data_size);
        } else {
            // Slower element-by-element copy if sizes don't match (todo if needed)
            std::ptr::write_bytes(body_ptr, 0, total_size);
        }
    } else {
        std::ptr::write_bytes(body_ptr, 0, total_size);
    }

    let id = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(id, body_ptr, size, actual_elem_size);
    id
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
pub unsafe extern "C" fn rt_Array_new_fixed(len: i64, elem_size: i64) -> i64 {
    rt_Array_new(len, elem_size)
}
#[no_mangle]
pub unsafe extern "C" fn rt_Array_keys(id: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return rt_Array_new_fixed(0, 8);
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    if tag == TAG_ARRAY {
        // ... handled elsewhere or just return empty for now
    }
    rt_Array_new_fixed(0, 8)
}
