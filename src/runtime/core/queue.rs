use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    let arr = rt_array_new(0, 8);
    *ptr.offset(0) = arr;
}
#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_enqueue(this: i64, val: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return;
    }
    let mut arr = *ptr.offset(0);
    if arr == 0 {
        arr = rt_array_new(0, 8);
        *(ptr as *mut i64).offset(0) = arr;
    }
    let new_arr = rt_array_push(arr, val);
    if new_arr != arr && new_arr != 0 {
        *(ptr as *mut i64).offset(0) = new_arr;
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_dequeue(this: i64) -> i64 {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return 0;
    }
    let arr = *ptr.offset(0);
    if arr == 0 {
        return 0;
    }
    rt_array_shift(arr)
}
#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_size(this: i64) -> i64 {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return 0;
    }
    let arr = *ptr.offset(0);
    if arr == 0 {
        return 0;
    }
    rt_len(arr)
}
