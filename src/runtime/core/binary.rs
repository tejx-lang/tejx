use super::*;

pub unsafe fn int_array_from_bytes(bytes: &[u8]) -> i64 {
    let len = bytes.len() as i64;
    let arr = rt_Array_new(len, 4);
    if len == 0 {
        return arr;
    }

    let body = (arr - HEAP_OFFSET) as *mut u8;
    let data = body as *mut i32;
    for i in 0..bytes.len() {
        *data.add(i) = bytes[i] as i32;
    }
    rt_update_array_cache(arr, body, len, 4);
    arr
}

pub unsafe fn bytes_from_int_array(arr: i64) -> Option<Vec<u8>> {
    if arr < HEAP_OFFSET {
        return None;
    }

    let len = rt_len(arr);
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let value = rt_array_get_fast(arr, i);
        if !(0..=255).contains(&value) {
            return None;
        }
        out.push(value as u8);
    }
    Some(out)
}

#[no_mangle]
pub unsafe extern "C" fn rt_bytes_from_string(s: i64) -> i64 {
    if let Some((data, len)) = get_str_parts(s) {
        let bytes = std::slice::from_raw_parts(data, len as usize);
        return int_array_from_bytes(bytes);
    }
    rt_Array_new(0, 4)
}

#[no_mangle]
pub unsafe extern "C" fn rt_bytes_to_string(arr: i64) -> i64 {
    if let Some(bytes) = bytes_from_int_array(arr) {
        return new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64);
    }
    rt_throw_runtime_error("RuntimeError: Invalid byte array");
}
