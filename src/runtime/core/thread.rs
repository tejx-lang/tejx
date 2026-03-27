use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_constructor(this: i64, cb: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    rt_ensure_type_finalizer(this, rt_thread_object_finalizer);
    // field 0 = runtime data pointer (non-GC)
    // field 1 = callback closure (GC-managed)
    *ptr.offset(1) = cb;
    let slot_live = std::sync::Arc::new(AtomicBool::new(true));
    let data = Box::new(ThreadData {
        handle: None,
        started: false,
        cb_slot: rt_add_static_root(cb),
        slot_live,
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
    let cb_slot = (*data_ptr).cb_slot;
    let slot_live = (*data_ptr).slot_live.clone();
    let handle = std::thread::spawn(move || {
        rt_register_thread();
        let _guard = ThreadRunGuard { cb_slot, slot_live };
        let mut cb_root = 0;
        rt_pin_static_root(cb_slot, &mut cb_root);
        rt_call_closure_no_args(cb_root);
        rt_pop_roots(1);
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
    rt_release_thread_cb_slot((*data_ptr).cb_slot, &(*data_ptr).slot_live);
    *ptr.offset(1) = 0;
    let _ = Box::from_raw(data_ptr);
    *ptr.offset(0) = 0;
}
