use super::*; // Extracted \n
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
        rt_string_from_c_str(new_str.as_ptr() as *const _)
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
        rt_string_from_c_str(new_str.as_ptr() as *const _)
    } else {
        s
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_String_repeat(s: i64, count: i64) -> i64 {
    if count <= 0 {
        return rt_string_from_c_str("\0".as_ptr() as *const _);
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
        rt_string_from_c_str(new_str.as_ptr() as *const _)
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
            rt_string_from_c_str(new_str.as_ptr() as *const _)
        } else {
            s
        }
    } else {
        s
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_String_concat(a: i64, b: i64) -> i64 {
    rt_str_concat_v2(a, b)
}
#[no_mangle]
pub unsafe extern "C" fn rt_str_at(id: i64, index: i64) -> i64 {
    if (id as u64) < (HEAP_OFFSET as u64) {
        return rt_string_from_c_str("\0".as_ptr() as *const _);
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let len = (*header).length as i64;
    if index < 0 || index >= len {
        return rt_string_from_c_str("\0".as_ptr() as *const _);
    }
    let c = *body.offset(index as isize);
    let mut buf = [0u8; 2];
    buf[0] = c;
    buf[1] = 0;
    rt_string_from_c_str(buf.as_ptr() as *const _)
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

    // Ensure they are strings by converting if necessary
    if get_str_parts(val_a).is_none() {
        val_a = rt_to_string(val_a);
    }
    if get_str_parts(val_b).is_none() {
        val_b = rt_to_string(val_b);
    }

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
        rt_string_from_c_str("\0".as_ptr() as *const _)
    };

    rt_pop_roots(2);
    res
}
