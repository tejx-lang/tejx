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
pub unsafe extern "C" fn tejx_run_event_loop() {
    loop {
        // Process all queued tasks
        let task = if let Ok(mut queue) = TASK_QUEUE.lock() {
            queue.pop_front()
        } else {
            None
        };

        if let Some((worker, args)) = task {
            let worker_fn: unsafe extern "C" fn(i64) = std::mem::transmute(worker);
            worker_fn(args);
        } else if ASYNC_OPS.load(Ordering::SeqCst) <= 0 {
            break;
        } else {
            std::thread::yield_now();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

// --- Exception Handling ---
// TejX uses setjmp/longjmp style exception handling.
// We maintain a stack of jump buffers for try/catch blocks.

static EXCEPTION_STACK: LazyLock<Mutex<Vec<i64>>> = LazyLock::new(|| Mutex::new(Vec::new()));

static mut CURRENT_EXCEPTION: i64 = 0;

#[no_mangle]
pub unsafe extern "C" fn tejx_get_exception() -> i64 {
    CURRENT_EXCEPTION
}

#[no_mangle]
pub unsafe extern "C" fn tejx_push_handler(jmpbuf: *mut u8) {
    if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.push(jmpbuf as i64);
    }
}

#[no_mangle]
pub unsafe extern "C" fn tejx_pop_handler() {
    if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.pop();
    }
}
