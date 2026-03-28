use super::*;
use crate::event_loop::tejx_enqueue_microtask;
use std::sync::atomic::{AtomicUsize, Ordering};

extern "C" {
    fn _setjmp(env: *mut i8) -> i32;
}

const PROMISE_STATE_PENDING: i64 = 0;
const PROMISE_STATE_RESOLVED: i64 = 1;
const PROMISE_STATE_REJECTED: i64 = 2;
const PROMISE_FLAG_OBSERVED: i64 = 1;

static UNHANDLED_PROMISE_REPORTS: AtomicUsize = AtomicUsize::new(0);

#[inline]
unsafe fn promise_state_ptr(body: *mut i64) -> *mut i64 {
    body.offset(0)
}

#[inline]
unsafe fn promise_value_ptr(body: *mut i64) -> *mut i64 {
    body.offset(1)
}

#[inline]
unsafe fn promise_callbacks_ptr(body: *mut i64) -> *mut i64 {
    body.offset(2)
}

#[inline]
unsafe fn promise_flags_ptr(body: *mut i64) -> *mut i64 {
    body.offset(3)
}

#[inline]
unsafe fn promise_owner_id(body: *mut i64) -> i64 {
    (body as i64) + HEAP_OFFSET
}

#[inline]
unsafe fn promise_store_ref(body: *mut i64, slot: *mut i64, value: i64) {
    rt_store_ref_slot(promise_owner_id(body), slot, value);
}

#[inline]
unsafe fn promise_mark_observed(body: *mut i64) {
    *promise_flags_ptr(body) |= PROMISE_FLAG_OBSERVED;
}

#[inline]
unsafe fn promise_is_observed(body: *mut i64) -> bool {
    (*promise_flags_ptr(body) & PROMISE_FLAG_OBSERVED) != 0
}

#[cfg(test)]
fn take_unhandled_promise_reports() -> usize {
    UNHANDLED_PROMISE_REPORTS.swap(0, Ordering::SeqCst)
}

unsafe fn rt_is_promise_value(value: i64) -> bool {
    if value < HEAP_OFFSET {
        return false;
    }
    let body = (value - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body) {
        return false;
    }
    let header = rt_get_header(body);
    (*header).type_id == TAG_PROMISE as u16
}

unsafe fn rt_promise_self_resolution_error() -> i64 {
    rt_string_from_c_str_const(b"TypeError: Promise cannot resolve to itself.\0".as_ptr() as *const _)
}

unsafe fn rt_promise_adopt(target_p: i64, source_p: i64) {
    let mut v_target = target_p;
    let mut v_source = source_p;
    rt_push_root(&mut v_target);
    rt_push_root(&mut v_source);

    if v_target == v_source {
        let mut err = rt_promise_self_resolution_error();
        rt_push_root(&mut err);
        rt_promise_reject(v_target, err);
        rt_pop_roots(3);
        return;
    }

    let body = (v_source - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(2);
        rt_promise_resolve(v_target, v_source);
        return;
    }

    promise_mark_observed(body);

    match *body.offset(0) {
        0 => {
            let mut callbacks_arr = *body.offset(2);
            rt_push_root(&mut callbacks_arr);
            callbacks_arr = rt_array_push(callbacks_arr, 0);
            callbacks_arr = rt_array_push(callbacks_arr, 0);
            callbacks_arr = rt_array_push(callbacks_arr, v_target);
            promise_store_ref(body, body.offset(2), callbacks_arr);
            rt_pop_roots(1);
        }
        1 => {
            let mut value = *body.offset(1);
            rt_push_root(&mut value);
            rt_promise_resolve(v_target, value);
            rt_pop_roots(1);
        }
        2 => {
            let mut err = *body.offset(1);
            rt_push_root(&mut err);
            rt_promise_reject(v_target, err);
            rt_pop_roots(1);
        }
        _ => {}
    }

    rt_pop_roots(2);
}

unsafe fn rt_call_closure_catch_exception(closure: i64, arg: i64) -> Result<i64, i64> {
    let mut jmpbuf = [0i64; 37];
    let jmp_ptr = jmpbuf.as_mut_ptr() as *mut i8;
    if _setjmp(jmp_ptr) == 0 {
        crate::event_loop::tejx_push_handler(jmp_ptr as *mut u8);
        let res = rt_call_closure(closure, arg);
        crate::event_loop::tejx_pop_handler();
        Ok(res)
    } else {
        Err(crate::event_loop::tejx_get_exception())
    }
}

unsafe fn rt_call_closure_argv_catch_exception(closure: i64, args: i64) -> Result<i64, i64> {
    let mut jmpbuf = [0i64; 37];
    let jmp_ptr = jmpbuf.as_mut_ptr() as *mut i8;
    if _setjmp(jmp_ptr) == 0 {
        crate::event_loop::tejx_push_handler(jmp_ptr as *mut u8);
        let res = rt_call_closure_argv(closure, args);
        crate::event_loop::tejx_pop_handler();
        Ok(res)
    } else {
        Err(crate::event_loop::tejx_get_exception())
    }
}

unsafe fn rt_schedule_unhandled_rejection_check(promise: i64) {
    let handle = crate::event_loop::tejx_create_global_handle(promise);
    crate::event_loop::tejx_enqueue_task(
        rt_promise_unhandled_rejection_worker as *const () as i64,
        handle as i64,
    );
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_resolve_callback(
    env: i64,
    value: i64,
    _a2: i64,
    _a3: i64,
    _a4: i64,
) -> i64 {
    rt_promise_resolve(env, value);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_reject_callback(
    env: i64,
    err: i64,
    _a2: i64,
    _a3: i64,
    _a4: i64,
) -> i64 {
    rt_promise_reject(env, err);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_from_executor(executor: i64) -> i64 {
    let mut promise = rt_promise_new();
    let mut exec = executor;
    rt_push_root(&mut promise);
    rt_push_root(&mut exec);

    let mut resolve = rt_closure_from_ptr(rt_promise_resolve_callback as *const () as i64);
    let mut reject = rt_closure_from_ptr(rt_promise_reject_callback as *const () as i64);
    rt_push_root(&mut resolve);
    rt_push_root(&mut reject);
    rt_array_set_fast(resolve, 1, promise);
    rt_array_set_fast(reject, 1, promise);

    let mut args = rt_Array_new_fixed(2, 8);
    rt_push_root(&mut args);
    rt_array_set_fast(args, 0, resolve);
    rt_array_set_fast(args, 1, reject);

    if let Err(err) = rt_call_closure_argv_catch_exception(exec, args) {
        rt_promise_reject(promise, err);
    }

    rt_pop_roots(5);
    promise
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_resolver_worker(args: i64) {
    let pid = rt_array_get_fast(args, 0);
    let val = rt_array_get_fast(args, 1);
    rt_promise_resolve(pid, val);
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_new() -> i64 {
    let mut callbacks = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut callbacks);

    let body = gc_allocate(PROMISE_BODY_SIZE);
    let obj = body as *mut i64;
    let header = rt_get_header(body);
    (*header).type_id = TAG_PROMISE as u16;

    *obj.offset(0) = 0; // State: Pending
    *obj.offset(1) = 0; // Value
    promise_store_ref(obj, obj.offset(2), callbacks); // Callbacks array
    *obj.offset(3) = 0; // flags
    *obj.offset(4) = 0;

    rt_pop_roots(1);
    (body as i64) + HEAP_OFFSET
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_resolve(p: i64, v_val: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let mut v_p = p;
    let mut v_v = v_val;
    rt_push_root(&mut v_p);
    rt_push_root(&mut v_v);

    let body = (v_p - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(2);
        return;
    }
    if *promise_state_ptr(body) != PROMISE_STATE_PENDING {
        rt_pop_roots(2);
        return;
    } // Already settled

    if rt_is_promise_value(v_v) {
        rt_promise_adopt(v_p, v_v);
        rt_pop_roots(2);
        return;
    }

    *promise_state_ptr(body) = PROMISE_STATE_RESOLVED;
    promise_store_ref(body, promise_value_ptr(body), v_v);

    // Execute callbacks asynchronously
    let callbacks_arr = *promise_callbacks_ptr(body);
    *promise_callbacks_ptr(body) = 0;
    if callbacks_arr >= HEAP_OFFSET {
        let mut v_callbacks = callbacks_arr;
        rt_push_root(&mut v_callbacks);
        let n = rt_len(v_callbacks);

        for i in (0..n).step_by(3) {
            let mut cb_resolve = rt_array_get_fast(v_callbacks, i as i64);
            let mut cb_reject = rt_array_get_fast(v_callbacks, (i + 1) as i64);
            let mut next_p = rt_array_get_fast(v_callbacks, (i + 2) as i64);
            rt_push_root(&mut cb_resolve);
            rt_push_root(&mut cb_reject);
            rt_push_root(&mut next_p);

            if next_p == -2 {
                // State machine resume: cb_resolve is worker, cb_reject is ctx
                tejx_enqueue_microtask(cb_resolve, cb_reject);
            } else {
                // Standard .then() callback: [callback, value, next_promise]
                let mut task_args = rt_Array_new_fixed(3, 8);
                rt_push_root(&mut task_args);
                rt_array_set_fast(task_args, 0, cb_resolve);
                rt_array_set_fast(task_args, 1, v_v);
                rt_array_set_fast(task_args, 2, next_p);
                tejx_enqueue_microtask(rt_promise_callback_worker as *const () as i64, task_args);
                rt_pop_roots(1); // task_args
            }

            rt_pop_roots(3); // next_p, cb_reject, cb_resolve
        }
        rt_pop_roots(1); // v_callbacks
    }
    rt_pop_roots(2); // v_p, v_val
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_callback_worker(args: i64) {
    if args < HEAP_OFFSET {
        return;
    }
    let mut v_args = args;
    rt_push_root(&mut v_args);

    let mut cb = rt_array_get_fast(v_args, 0);
    let mut val = rt_array_get_fast(v_args, 1);
    let mut next_p = rt_array_get_fast(v_args, 2);
    rt_push_root(&mut cb);
    rt_push_root(&mut val);
    rt_push_root(&mut next_p);

    if cb >= HEAP_OFFSET {
        // Call callback and resolve next promise in chain with result
        match rt_call_closure_catch_exception(cb, val) {
            Ok(res) => rt_promise_resolve(next_p, res),
            Err(err) => rt_promise_reject(next_p, err),
        }
    } else {
        // No handler: propagate resolution
        rt_promise_resolve(next_p, val);
    }

    rt_pop_roots(4); // next_p, val, cb, v_args
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_reject(p: i64, v_err: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let mut v_p = p;
    let mut v_e = v_err;
    rt_push_root(&mut v_p);
    rt_push_root(&mut v_e);

    let body = (v_p - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(2);
        return;
    }
    if *promise_state_ptr(body) != PROMISE_STATE_PENDING {
        rt_pop_roots(2);
        return;
    } // Already settled
    *promise_state_ptr(body) = PROMISE_STATE_REJECTED;
    promise_store_ref(body, promise_value_ptr(body), v_e);

    // Propagate rejection asynchronously
    let callbacks_arr = *promise_callbacks_ptr(body);
    *promise_callbacks_ptr(body) = 0;
    if callbacks_arr >= HEAP_OFFSET {
        let mut v_callbacks = callbacks_arr;
        rt_push_root(&mut v_callbacks);
        let n = rt_len(v_callbacks);

        for i in (0..n).step_by(3) {
            let mut cb_resolve = rt_array_get_fast(v_callbacks, i as i64);
            let mut cb_reject = rt_array_get_fast(v_callbacks, (i + 1) as i64);
            let mut next_p = rt_array_get_fast(v_callbacks, (i + 2) as i64);
            rt_push_root(&mut cb_resolve);
            rt_push_root(&mut cb_reject);
            rt_push_root(&mut next_p);

            if next_p == -2 {
                // State machine resume: worker(ctx)
                tejx_enqueue_microtask(cb_resolve, cb_reject);
            } else {
                let mut task_args = rt_Array_new_fixed(3, 8);
                rt_push_root(&mut task_args);
                rt_array_set_fast(task_args, 0, cb_reject);
                rt_array_set_fast(task_args, 1, v_e);
                rt_array_set_fast(task_args, 2, next_p);
                tejx_enqueue_microtask(rt_promise_rejection_worker as *const () as i64, task_args);
                rt_pop_roots(1); // task_args
            }

            rt_pop_roots(3); // next_p, cb_reject, cb_resolve
        }
        rt_pop_roots(1); // v_callbacks
    }
    if !promise_is_observed(body) {
        rt_schedule_unhandled_rejection_check(v_p);
    }
    rt_pop_roots(2);
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_rejection_worker(args: i64) {
    if args < HEAP_OFFSET {
        return;
    }
    let mut v_args = args;
    rt_push_root(&mut v_args);

    let mut cb_reject = rt_array_get_fast(v_args, 0);
    let mut err = rt_array_get_fast(v_args, 1);
    let mut next_p = rt_array_get_fast(v_args, 2);
    rt_push_root(&mut cb_reject);
    rt_push_root(&mut err);
    rt_push_root(&mut next_p);

    if cb_reject >= HEAP_OFFSET {
        // Call rejection handler and resolve NEXT promise with result
        match rt_call_closure_catch_exception(cb_reject, err) {
            Ok(res) => rt_promise_resolve(next_p, res),
            Err(thrown) => rt_promise_reject(next_p, thrown),
        }
    } else {
        // No handler: propagate rejection to NEXT promise
        rt_promise_reject(next_p, err);
    }

    rt_pop_roots(4); // next_p, err, cb_reject, v_args
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_unhandled_rejection_worker(handle_id: i64) {
    let handle = handle_id as usize;
    let promise = crate::event_loop::tejx_get_global_handle(handle);
    crate::event_loop::tejx_drop_global_handle(handle);
    if promise < HEAP_OFFSET {
        return;
    }
    let body = (promise - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        return;
    }
    if *promise_state_ptr(body) == PROMISE_STATE_REJECTED && !promise_is_observed(body) {
        UNHANDLED_PROMISE_REPORTS.fetch_add(1, Ordering::SeqCst);
        crate::event_loop::log_exception("UnhandledPromiseRejection", *promise_value_ptr(body));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_resolved(value: i64) -> i64 {
    let promise = rt_promise_new();
    rt_promise_resolve(promise, value);
    promise
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_rejected(err: i64) -> i64 {
    let promise = rt_promise_new();
    rt_promise_reject(promise, err);
    promise
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_then(p: i64, cb_resolve: i64, cb_reject: i64) -> i64 {
    if p < HEAP_OFFSET {
        return rt_promise_new();
    }
    let mut v_p = p;
    let mut v_cb_res = cb_resolve;
    let mut v_cb_rej = cb_reject;
    rt_push_root(&mut v_p);
    rt_push_root(&mut v_cb_res);
    rt_push_root(&mut v_cb_rej);

    let body = (v_p - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(3);
        return rt_promise_new();
    }

    promise_mark_observed(body);
    let state = *promise_state_ptr(body);
    let new_p = rt_promise_new();
    let mut v_new_p = new_p;
    rt_push_root(&mut v_new_p);

    if state == PROMISE_STATE_PENDING {
        // Pending: Store (cb_res, cb_rej, new_p)
        let mut callbacks_arr = *promise_callbacks_ptr(body);
        rt_push_root(&mut callbacks_arr);
        callbacks_arr = rt_array_push(callbacks_arr, v_cb_res);
        callbacks_arr = rt_array_push(callbacks_arr, v_cb_rej);
        callbacks_arr = rt_array_push(callbacks_arr, v_new_p);
        promise_store_ref(body, promise_callbacks_ptr(body), callbacks_arr);
        rt_pop_roots(1);
    } else if state == PROMISE_STATE_RESOLVED {
        let mut val = *promise_value_ptr(body);
        rt_push_root(&mut val);
        let mut task_args = rt_Array_new_fixed(3, 8);
        rt_push_root(&mut task_args);
        rt_array_set_fast(task_args, 0, v_cb_res);
        rt_array_set_fast(task_args, 1, val);
        rt_array_set_fast(task_args, 2, v_new_p);
        tejx_enqueue_microtask(rt_promise_callback_worker as *const () as i64, task_args);
        rt_pop_roots(2); // task_args, val
    } else if state == PROMISE_STATE_REJECTED {
        // Rejected: Enqueue rejection microtask
        let mut err = *promise_value_ptr(body);
        rt_push_root(&mut err);
        let mut task_args = rt_Array_new_fixed(3, 8);
        rt_push_root(&mut task_args);
        rt_array_set_fast(task_args, 0, v_cb_rej);
        rt_array_set_fast(task_args, 1, err);
        rt_array_set_fast(task_args, 2, v_new_p);
        tejx_enqueue_microtask(rt_promise_rejection_worker as *const () as i64, task_args);
        rt_pop_roots(2); // task_args, err
    }

    rt_pop_roots(4);
    new_p
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_observe(p: i64, cb_resolve: i64, cb_reject: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let mut v_p = p;
    let mut v_cb_res = cb_resolve;
    let mut v_cb_rej = cb_reject;
    rt_push_root(&mut v_p);
    rt_push_root(&mut v_cb_res);
    rt_push_root(&mut v_cb_rej);

    let body = (v_p - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(3);
        return;
    }

    promise_mark_observed(body);
    let state = *promise_state_ptr(body);

    if state == PROMISE_STATE_PENDING {
        let mut callbacks_arr = *promise_callbacks_ptr(body);
        rt_push_root(&mut callbacks_arr);
        callbacks_arr = rt_array_push(callbacks_arr, v_cb_res);
        callbacks_arr = rt_array_push(callbacks_arr, v_cb_rej);
        callbacks_arr = rt_array_push(callbacks_arr, 0);
        promise_store_ref(body, promise_callbacks_ptr(body), callbacks_arr);
        rt_pop_roots(1);
    } else if state == PROMISE_STATE_RESOLVED {
        let mut val = *promise_value_ptr(body);
        rt_push_root(&mut val);
        let mut task_args = rt_Array_new_fixed(3, 8);
        rt_push_root(&mut task_args);
        rt_array_set_fast(task_args, 0, v_cb_res);
        rt_array_set_fast(task_args, 1, val);
        rt_array_set_fast(task_args, 2, 0);
        tejx_enqueue_microtask(rt_promise_callback_worker as *const () as i64, task_args);
        rt_pop_roots(2);
    } else if state == PROMISE_STATE_REJECTED {
        let mut err = *promise_value_ptr(body);
        rt_push_root(&mut err);
        let mut task_args = rt_Array_new_fixed(3, 8);
        rt_push_root(&mut task_args);
        rt_array_set_fast(task_args, 0, v_cb_rej);
        rt_array_set_fast(task_args, 1, err);
        rt_array_set_fast(task_args, 2, 0);
        tejx_enqueue_microtask(rt_promise_rejection_worker as *const () as i64, task_args);
        rt_pop_roots(2);
    }

    rt_pop_roots(3);
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_catch(p: i64, cb_reject: i64) -> i64 {
    rt_promise_then(p, 0, cb_reject)
}

#[no_mangle]
pub unsafe extern "C" fn rt_promise_await_resume(p: i64, worker: i64, ctx: i64) {
    if p < HEAP_OFFSET {
        return;
    }
    let mut v_p = p;
    let mut v_worker = worker;
    let mut v_ctx = ctx;
    rt_push_root(&mut v_p);
    rt_push_root(&mut v_worker);
    rt_push_root(&mut v_ctx);

    let body = (v_p - HEAP_OFFSET) as *mut i64;
    promise_mark_observed(body);
    let state = *promise_state_ptr(body);

    if state == PROMISE_STATE_PENDING {
        // Pending: Store (worker, ctx, -2) in callbacks
        let mut callbacks_arr = *promise_callbacks_ptr(body);
        rt_push_root(&mut callbacks_arr);
        callbacks_arr = rt_array_push(callbacks_arr, v_worker);
        callbacks_arr = rt_array_push(callbacks_arr, v_ctx);
        callbacks_arr = rt_array_push(callbacks_arr, -2); // Marker for state machine resume
        promise_store_ref(body, promise_callbacks_ptr(body), callbacks_arr);
        rt_pop_roots(1);
    } else {
        // Already settled: Enqueue microtask immediately
        tejx_enqueue_microtask(v_worker, v_ctx);
    }

    rt_pop_roots(3);
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_get_value(p: i64) -> i64 {
    if p < HEAP_OFFSET {
        return 0;
    }
    let body = (p - HEAP_OFFSET) as *mut i64;
    let state = *promise_state_ptr(body);
    let val = *promise_value_ptr(body);
    if state == PROMISE_STATE_REJECTED {
        crate::event_loop::tejx_throw(val);
        std::hint::unreachable_unchecked();
    }
    val
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_clone(p: i64) -> i64 {
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn drain_event_loop() {
        while crate::event_loop::tejx_run_event_loop_step() {}
    }

    unsafe extern "C" fn swallow_rejection(
        _env: i64,
        _err: i64,
        _a2: i64,
        _a3: i64,
        _a4: i64,
    ) -> i64 {
        0
    }

    unsafe extern "C" fn attach_rejection_handler_worker(args: i64) {
        let promise = rt_array_get_fast(args, 0);
        let handler = rt_array_get_fast(args, 1);
        let _ = rt_promise_then(promise, 0, handler);
    }

    #[test]
    fn reports_unhandled_rejections_after_microtask_turn() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            drain_event_loop();
            take_unhandled_promise_reports();

            let promise = rt_promise_new();
            let err = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            rt_promise_reject(promise, err);

            assert_eq!(take_unhandled_promise_reports(), 0);
            assert!(crate::event_loop::tejx_run_event_loop_step());
            assert_eq!(take_unhandled_promise_reports(), 1);
        }
    }

    #[test]
    fn observed_rejections_are_not_reported() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            drain_event_loop();
            take_unhandled_promise_reports();

            let promise = rt_promise_new();
            let handler = rt_closure_from_ptr(swallow_rejection as *const () as i64);
            let _ = rt_promise_then(promise, 0, handler);
            let err = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            rt_promise_reject(promise, err);

            while crate::event_loop::tejx_run_event_loop_step() {}
            assert_eq!(take_unhandled_promise_reports(), 0);
        }
    }

    #[test]
    fn same_turn_rejection_handler_suppresses_report() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            drain_event_loop();
            take_unhandled_promise_reports();

            let promise = rt_promise_new();
            let err = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            rt_promise_reject(promise, err);

            let handler = rt_closure_from_ptr(swallow_rejection as *const () as i64);
            let _ = rt_promise_then(promise, 0, handler);

            while crate::event_loop::tejx_run_event_loop_step() {}
            assert_eq!(take_unhandled_promise_reports(), 0);
        }
    }

    #[test]
    fn adopted_rejections_are_not_reported_as_unhandled() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            drain_event_loop();
            take_unhandled_promise_reports();

            let source = rt_promise_new();
            let adopted = rt_promise_new();
            let handler = rt_closure_from_ptr(swallow_rejection as *const () as i64);
            let _ = rt_promise_then(adopted, 0, handler);
            let err = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            rt_promise_reject(source, err);
            rt_promise_resolve(adopted, source);

            while crate::event_loop::tejx_run_event_loop_step() {}
            assert_eq!(take_unhandled_promise_reports(), 0);
        }
    }

    #[test]
    fn later_microtask_handler_suppresses_unhandled_report() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            drain_event_loop();
            take_unhandled_promise_reports();

            let mut promise = rt_promise_new();
            let mut handler = rt_closure_from_ptr(swallow_rejection as *const () as i64);
            let mut args = rt_Array_new_fixed(2, 8);
            rt_push_root(&mut promise);
            rt_push_root(&mut handler);
            rt_push_root(&mut args);
            rt_array_set_fast(args, 0, promise);
            rt_array_set_fast(args, 1, handler);

            let err = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            rt_promise_reject(promise, err);
            tejx_enqueue_microtask(attach_rejection_handler_worker as *const () as i64, args);

            rt_pop_roots(3);

            while crate::event_loop::tejx_run_event_loop_step() {}
            assert_eq!(take_unhandled_promise_reports(), 0);
        }
    }

    #[test]
    fn old_promises_dirty_cards_for_young_callbacks_and_values() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            drain_event_loop();

            let mut promise = rt_promise_new();
            rt_push_root(&mut promise);
            let mut pad = rt_Array_new(256, 8);
            rt_push_root(&mut pad);

            gc::minor_gc();
            gc::minor_gc();

            let promise_body = (promise - HEAP_OFFSET) as *mut u8;
            assert!(promise_body >= gc::OLD_START && promise_body < gc::OLD_TOP);

            let card_idx = (promise_body as usize - gc::OLD_START as usize) >> gc::CARD_SHIFT;
            *gc::CARD_TABLE.add(card_idx) = 0;

            let mut chained = rt_promise_then(promise, 0, 0);
            rt_push_root(&mut chained);

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old promises must dirty their card when then() swaps in a young callback array"
            );

            *gc::CARD_TABLE.add(card_idx) = 0;

            let mut value = rt_box_int(42);
            rt_push_root(&mut value);
            rt_promise_resolve(promise, value);

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old promises must dirty their card when resolving to a young heap value"
            );

            rt_pop_roots(4);
            drain_event_loop();
        }
    }
}
