use super::*;

/// Spawn a TejX virtual thread (may coroutine).
///
/// Safety properties:
///   - Guard page below each coroutine stack prevents silent stack overflow.
///   - GC roots are isolated per coroutine via thread_local! (may CLS).
///   - Panics in the coroutine are caught and do NOT crash the scheduler.
///   - The closure GC root (cb_slot) is held alive by ThreadRunGuard until
///     the coroutine exits, then atomically released.
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_constructor(this: i64, cb: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    rt_ensure_type_finalizer(this, rt_thread_object_finalizer);
    // field 0 = runtime data pointer (non-GC managed)
    // field 1 = callback closure (GC-managed, also pinned via static root)
    rt_store_ref_slot(this, ptr.offset(1), cb);
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

    // Spawn a may virtual thread (coroutine). Each coroutine starts with
    // VTHREAD_STACK_SIZE (32KB) and grows on demand via mmap. A guard page
    // sits below the stack \u2014 stack overflow is caught as a segfault rather
    // than silently corrupting memory.
    let handle = may::coroutine::Builder::new()
        .stack_size(VTHREAD_STACK_SIZE)
        .spawn(move || {
            // Register this coroutine with the GC. Each virtual thread gets
            // its own MY_CONTEXT (thread_local via may CLS) and MY_TLAB for
            // lock-free allocation within the coroutine.
            rt_register_thread();

            // ThreadRunGuard releases the static GC root and unregisters the
            // coroutine from the GC when it exits (even via panic).
            let _guard = ThreadRunGuard { cb_slot, slot_live };

            // Catch panics so one misbehaving coroutine cannot crash the
            // entire process or orphan other virtual threads.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                let mut cb_root = 0;
                rt_pin_static_root(cb_slot, &mut cb_root);
                rt_call_closure_no_args(cb_root);
            }));
        })
        .expect("TejX virtual thread spawn failed — may scheduler not initialized");

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
    // Auto-start if not yet started (matches prior OS-thread behaviour).
    if !(*data_ptr).started {
        rt_Thread_start(this);
    }
    // Block the calling virtual thread until the target coroutine finishes.
    // If the caller is itself a may coroutine, this yield allows the may
    // scheduler to run other coroutines while waiting \u2014 no OS thread is
    // wasted. If called from the main OS thread, it blocks normally.
    if let Some(handle) = (*data_ptr).handle.take() {
        let _ = handle.join();
    }
    rt_release_thread_cb_slot((*data_ptr).cb_slot, &(*data_ptr).slot_live);
    *ptr.offset(1) = 0;
    let _ = Box::from_raw(data_ptr);
    *ptr.offset(0) = 0;
}

/// Sleep the current virtual thread.
///
/// If running inside a may coroutine, this yields the coroutine so the
/// underlying OS thread is free to run other coroutines. The coroutine
/// wakes after `ms` milliseconds.
///
/// If called from the main OS thread (not a coroutine), falls back to a
/// standard blocking sleep.
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_sleep(ms: i64) {
    let dur = std::time::Duration::from_millis(ms as u64);
    if may::coroutine::is_coroutine() {
        // Cooperative sleep: releases the OS thread to other coroutines.
        may::coroutine::sleep(dur);
    } else {
        // Main thread or non-coroutine context: plain blocking sleep.
        std::thread::sleep(dur);
    }
}
