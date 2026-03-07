use super::*;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};

// --- Async Task Queue ---
static TASK_QUEUE: LazyLock<Mutex<VecDeque<(i64, i64)>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

#[no_mangle]
pub static ASYNC_OPS: AtomicI64 = AtomicI64::new(0);

#[no_mangle]
pub unsafe extern "C" fn tejx_enqueue_task(worker: i64, args: i64) {
    if let Ok(mut queue) = TASK_QUEUE.lock() {
        queue.push_back((worker, args));
    }
}

#[no_mangle]
pub unsafe extern "C" fn tejx_inc_async_ops() {
    ASYNC_OPS.fetch_add(1, Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn tejx_dec_async_ops() {
    ASYNC_OPS.fetch_sub(1, Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn tejx_run_event_loop_step() -> bool {
    let task = if let Ok(mut queue) = TASK_QUEUE.lock() {
        queue.pop_front()
    } else {
        None
    };

    if let Some((worker, args)) = task {
        let worker_fn: unsafe extern "C" fn(i64) = std::mem::transmute(worker);
        worker_fn(args);
        true
    } else if ASYNC_OPS.load(Ordering::SeqCst) <= 0 {
        false
    } else {
        std::thread::yield_now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        true
    }
}

#[no_mangle]
pub unsafe extern "C" fn tejx_run_event_loop() {
    while tejx_run_event_loop_step() {}
}

// --- Exception Handling ---
// TejX uses setjmp/longjmp style exception handling.
// We maintain a stack of jump buffers for try/catch blocks.

struct ExceptionHandler {
    jmpbuf: i64,
    roots_top: usize,
}

static EXCEPTION_STACK: LazyLock<Mutex<Vec<ExceptionHandler>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

static mut CURRENT_EXCEPTION: i64 = 0;

#[no_mangle]
pub unsafe extern "C" fn tejx_get_exception() -> i64 {
    CURRENT_EXCEPTION
}

#[no_mangle]
pub unsafe extern "C" fn tejx_push_handler(jmpbuf: *mut u8) {
    if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.push(ExceptionHandler {
            jmpbuf: jmpbuf as i64,
            roots_top: GC_ROOTS_TOP,
        });
    }
}

#[no_mangle]
pub unsafe extern "C" fn tejx_pop_handler() {
    if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.pop();
    }
}

extern "C" {
    fn longjmp(env: *mut i8, val: i32);
}

#[no_mangle]
pub unsafe extern "C" fn tejx_throw(exception: i64) {
    CURRENT_EXCEPTION = exception;
    let handler = if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.pop()
    } else {
        None
    };

    if let Some(h) = handler {
        GC_ROOTS_TOP = h.roots_top;
        longjmp(h.jmpbuf as *mut i8, 1);
    } else {
        printf(
            "Uncaught exception: %lld\n\0".as_ptr() as *const _,
            exception,
        );
        exit(1);
    }
}
