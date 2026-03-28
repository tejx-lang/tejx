use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_constructor(this: i64) {
    let mut owner = this;
    rt_push_root(&mut owner);
    let ptr = rt_obj_ptr(owner);
    if ptr.is_null() {
        rt_pop_roots(1);
        return;
    }
    let arr = rt_array_new(0, 8);
    let ptr = rt_obj_ptr(owner);
    if !ptr.is_null() {
        rt_store_ref_slot(owner, ptr.offset(0), arr);
    }
    rt_pop_roots(1);
}
#[no_mangle]
pub unsafe extern "C" fn rt_SharedQueue_enqueue(this: i64, val: i64) {
    let mut owner = this;
    let mut value = val;
    rt_push_root(&mut owner);
    rt_push_root(&mut value);

    let ptr = rt_obj_ptr(owner) as *const i64;
    if ptr.is_null() {
        rt_pop_roots(2);
        return;
    }
    let mut arr = *ptr.offset(0);
    rt_push_root(&mut arr);
    if arr == 0 {
        arr = rt_array_new(0, 8);
        let ptr = rt_obj_ptr(owner) as *mut i64;
        if !ptr.is_null() {
            rt_store_ref_slot(owner, ptr.offset(0), arr);
        }
    }
    let new_arr = rt_array_push(arr, value);
    if new_arr != arr && new_arr != 0 {
        let ptr = rt_obj_ptr(owner) as *mut i64;
        if !ptr.is_null() {
            rt_store_ref_slot(owner, ptr.offset(0), new_arr);
        }
    }
    rt_pop_roots(3);
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
