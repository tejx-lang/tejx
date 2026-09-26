use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_Object_keys(obj: i64) -> i64 {
    if !rt_is_object(obj) {
        return rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_FIXED | ARRAY_FLAG_PTR);
    }
    rt_object_keys_array(obj)
}
#[no_mangle]
pub unsafe extern "C" fn rt_Object_values(obj: i64) -> i64 {
    if !rt_is_object(obj) {
        return rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_FIXED | ARRAY_FLAG_PTR);
    }
    rt_object_values_array(obj)
}
#[no_mangle]
pub unsafe extern "C" fn rt_Object_entries(obj: i64) -> i64 {
    let mut owner = obj;
    if !rt_is_object(owner) {
        return rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_FIXED | ARRAY_FLAG_PTR);
    }
    let mut keys = rt_object_keys_array(owner);
    let mut values = rt_object_values_array(owner);
    let len = rt_len(keys);
    let mut result = rt_Array_constructor_v2(0, len, 8, ARRAY_FLAG_FIXED | ARRAY_FLAG_PTR);
    let mut i = 0;
    while i < len {
        let mut pair = rt_Array_constructor_v2(0, 2, 8, ARRAY_FLAG_FIXED | ARRAY_FLAG_PTR);
        rt_array_set_fast(pair, 0, rt_array_get_fast(keys, i));
        rt_array_set_fast(pair, 1, rt_array_get_fast(values, i));
        rt_array_set_fast(result, i, pair);
        i += 1;
    }
    result
}
#[no_mangle]
pub unsafe extern "C" fn rt_Object_assign(target: i64, source: i64) -> i64 {
    let mut target = target;
    let mut source = source;
    if !rt_is_object(target) || !rt_is_object(source) {
        return target;
    }
    let mut keys = rt_object_keys_array(source);
    let mut values = rt_object_values_array(source);
    let len = rt_len(keys);
    let mut i = 0;
    while i < len {
        rt_set_property(
            target,
            rt_array_get_fast(keys, i),
            rt_array_get_fast(values, i),
        );
        i += 1;
    }
    target
}
#[no_mangle]
pub unsafe extern "C" fn rt_Object_freeze(obj: i64) -> i64 {
    obj
}
