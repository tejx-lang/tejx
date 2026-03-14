use super::*;
use crate::event_loop::tejx_enqueue_task; // Extracted \n
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

    let body = gc_allocate(40);
    let obj = body as *mut i64;
    let header = rt_get_header(body);
    (*header).type_id = TAG_PROMISE as u16;

    *obj.offset(0) = 0; // State: Pending
    *obj.offset(1) = 0; // Value
    *obj.offset(2) = callbacks; // Callbacks array
    *obj.offset(3) = 0; // data_base
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
    if *body.offset(0) != 0 {
        rt_pop_roots(2);
        return;
    } // Already settled

    *body.offset(0) = 1; // Resolved
    *body.offset(1) = v_v; // Store value

    // Execute callbacks asynchronously
    let callbacks_arr = *body.offset(2);
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
                tejx_enqueue_task(cb_resolve, cb_reject);
            } else {
                // Standard .then() callback: [callback, value, next_promise]
                let mut task_args = rt_Array_new_fixed(3, 8);
                rt_push_root(&mut task_args);
                rt_array_set_fast(task_args, 0, cb_resolve);
                rt_array_set_fast(task_args, 1, v_v);
                rt_array_set_fast(task_args, 2, next_p);
                tejx_enqueue_task(rt_promise_callback_worker as i64, task_args);
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
        let res = rt_call_closure(cb, val);
        rt_promise_resolve(next_p, res);
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
    if *body.offset(0) != 0 {
        rt_pop_roots(2);
        return;
    } // Already settled
    *body.offset(0) = 2; // Rejected
    *body.offset(1) = v_e;

    // Propagate rejection asynchronously
    let callbacks_arr = *body.offset(2);
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
                tejx_enqueue_task(cb_resolve, cb_reject);
            } else {
                let mut task_args = rt_Array_new_fixed(3, 8);
                rt_push_root(&mut task_args);
                rt_array_set_fast(task_args, 0, cb_reject);
                rt_array_set_fast(task_args, 1, v_e);
                rt_array_set_fast(task_args, 2, next_p);
                tejx_enqueue_task(rt_promise_rejection_worker as i64, task_args);
                rt_pop_roots(1); // task_args
            }

            rt_pop_roots(3); // next_p, cb_reject, cb_resolve
        }
        rt_pop_roots(1); // v_callbacks
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
        let res = rt_call_closure(cb_reject, err);
        rt_promise_resolve(next_p, res);
    } else {
        // No handler: propagate rejection to NEXT promise
        rt_promise_reject(next_p, err);
    }

    rt_pop_roots(4); // next_p, err, cb_reject, v_args
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

    let state = *body.offset(0);
    let new_p = rt_promise_new();
    let mut v_new_p = new_p;
    rt_push_root(&mut v_new_p);

    if state == 0 {
        // Pending: Store (cb_res, cb_rej, new_p)
        let mut callbacks_arr = *body.offset(2);
        rt_push_root(&mut callbacks_arr);
        callbacks_arr = rt_array_push(callbacks_arr, v_cb_res);
        callbacks_arr = rt_array_push(callbacks_arr, v_cb_rej);
        callbacks_arr = rt_array_push(callbacks_arr, v_new_p);
        *body.offset(2) = callbacks_arr;
        rt_pop_roots(1);
    } else if state == 1 {
        let mut val = *body.offset(1);
        rt_push_root(&mut val);
        let mut task_args = rt_Array_new_fixed(3, 8);
        rt_push_root(&mut task_args);
        rt_array_set_fast(task_args, 0, v_cb_res);
        rt_array_set_fast(task_args, 1, val);
        rt_array_set_fast(task_args, 2, v_new_p);
        tejx_enqueue_task(rt_promise_callback_worker as i64, task_args);
        rt_pop_roots(2); // task_args, val
    } else if state == 2 {
        // Rejected: Enqueue rejection microtask
        let mut err = *body.offset(1);
        rt_push_root(&mut err);
        let mut task_args = rt_Array_new_fixed(3, 8);
        rt_push_root(&mut task_args);
        rt_array_set_fast(task_args, 0, v_cb_rej);
        rt_array_set_fast(task_args, 1, err);
        rt_array_set_fast(task_args, 2, v_new_p);
        tejx_enqueue_task(rt_promise_rejection_worker as i64, task_args);
        rt_pop_roots(2); // task_args, err
    }

    rt_pop_roots(4);
    new_p
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
    let state = *body.offset(0);

    if state == 0 {
        // Pending: Store (worker, ctx, -2) in callbacks
        let mut callbacks_arr = *body.offset(2);
        rt_push_root(&mut callbacks_arr);
        callbacks_arr = rt_array_push(callbacks_arr, v_worker);
        callbacks_arr = rt_array_push(callbacks_arr, v_ctx);
        callbacks_arr = rt_array_push(callbacks_arr, -2); // Marker for state machine resume
        *body.offset(2) = callbacks_arr;
        rt_pop_roots(1);
    } else {
        // Already settled: Enqueue microtask immediately
        tejx_enqueue_task(v_worker, v_ctx);
    }

    rt_pop_roots(3);
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_get_value(p: i64) -> i64 {
    if p < HEAP_OFFSET {
        return 0;
    }
    let body = (p - HEAP_OFFSET) as *mut i64;
    // Return the resolved value or error (offset 1)
    *body.offset(1)
}
#[no_mangle]
pub unsafe extern "C" fn rt_promise_clone(p: i64) -> i64 {
    p
}
