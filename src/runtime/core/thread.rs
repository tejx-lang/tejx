use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_constructor(this: i64, cb: i64, args: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    // field 0 = runtime data pointer (non-GC)
    // field 1 = callback (GC-managed)
    // field 2 = args array (GC-managed)
    *ptr.offset(1) = cb;
    *ptr.offset(2) = args;
    let data = Box::new(ThreadData {
        handle: None,
        started: false,
    });
    *ptr.offset(0) = Box::into_raw(data) as i64;
}
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_start(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    let data_ptr = *ptr.offset(0) as *mut ThreadData;
    if data_ptr.is_null() {
        return;
    }
    if (*data_ptr).started {
        return;
    }
    (*data_ptr).started = true;
    let cb = *ptr.offset(1);
    let args = *ptr.offset(2);
    let handle = std::thread::spawn(move || {
        rt_register_thread();
        let _ = rt_call_closure(cb, args);
        rt_unregister_thread();
    });
    (*data_ptr).handle = Some(handle);
}
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_join(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    let data_ptr = *ptr.offset(0) as *mut ThreadData;
    if data_ptr.is_null() {
        return;
    }
    if !(*data_ptr).started {
        rt_Thread_start(this);
    }
    if let Some(handle) = (*data_ptr).handle.take() {
        let _ = handle.join();
    }
}
