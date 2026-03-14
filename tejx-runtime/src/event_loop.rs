use super::*;

use super::gc::{ThreadContext, MY_CONTEXT};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

// --- Async Task Queue ---
// Since we are running on a single-threaded Tokio runtime, we can use
// thread-local structures for our task queue, eliminating Mutexes.

thread_local! {
    static TASK_QUEUE: RefCell<VecDeque<(i64, i64)>> = RefCell::new(VecDeque::new());
    static GLOBAL_HANDLES: RefCell<HashMap<usize, i64>> = RefCell::new(HashMap::new());
}

static GLOBAL_HANDLE_NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// Global Tokio Runtime for background I/O tasks and timers
pub static TOKIO_RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
});

#[no_mangle]
pub static ASYNC_OPS: AtomicI64 = AtomicI64::new(0);

#[no_mangle]
pub unsafe extern "C" fn tejx_enqueue_task(worker: i64, args: i64) {
    TASK_QUEUE.with(|q| {
        q.borrow_mut().push_back((worker, args));
    });
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
    let task = TASK_QUEUE.with(|q| q.borrow_mut().pop_front());

    if let Some((worker, args)) = task {
        if worker != 0 {
            let worker_fn: unsafe extern "C" fn(i64) = std::mem::transmute(worker);
            worker_fn(args);
        }
        return true;
    }

    if ASYNC_OPS.load(Ordering::SeqCst) <= 0 {
        return false;
    }

    // Process Tokio tasks to pull next events (I/O, timers, etc) into the Tejx queue
    // We use `spawn` and `block_on` a short sleep to ensure the runtime's scheduler
    // gets a chance to poll other tasks, as `yield_now` within `block_on` might not
    // be sufficient to give CPU time to tasks spawned on the runtime's scheduler.
    TOKIO_RT.block_on(async {
        // Spawn a task that yields and then sleeps briefly.
        // This ensures the runtime's scheduler is actively polled.
        tokio::task::spawn(async {
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(1)).await;
        })
        .await
        .ok(); // Await the spawned task to ensure it runs
    });

    // Check again after Tokio polled
    let task = TASK_QUEUE.with(|q| q.borrow_mut().pop_front());
    if let Some((worker, args)) = task {
        if worker != 0 {
            let worker_fn: unsafe extern "C" fn(i64) = std::mem::transmute(worker);
            worker_fn(args);
        }
        return true;
    }

    true // Keep looping, as ASYNC_OPS > 0
}

#[no_mangle]
pub unsafe extern "C" fn tejx_run_event_loop() {
    while tejx_run_event_loop_step() {}
}

#[no_mangle]
pub unsafe extern "C" fn tejx_create_global_handle(ptr: i64) -> usize {
    let id = GLOBAL_HANDLE_NEXT_ID.fetch_add(1, Ordering::SeqCst);
    GLOBAL_HANDLES.with(|handles| {
        handles.borrow_mut().insert(id, ptr);
    });
    id
}

#[no_mangle]
pub unsafe extern "C" fn tejx_get_global_handle(id: usize) -> i64 {
    GLOBAL_HANDLES.with(|handles| *handles.borrow().get(&id).unwrap_or(&0))
}

#[no_mangle]
pub unsafe extern "C" fn tejx_drop_global_handle(id: usize) {
    GLOBAL_HANDLES.with(|handles| {
        handles.borrow_mut().remove(&id);
    });
}

pub unsafe fn rt_gc_scan_tasks() {
    TASK_QUEUE.with(|q| {
        for (_, ref mut args) in q.borrow_mut().iter_mut() {
            crate::gc::copy_object(args as *mut i64);
        }
    });
    GLOBAL_HANDLES.with(|handles| {
        for val in handles.borrow_mut().values_mut() {
            crate::gc::copy_object(val as *mut i64);
        }
    });
}

pub unsafe fn rt_gc_mark_tasks() {
    TASK_QUEUE.with(|q| {
        for (_, ref mut args) in q.borrow_mut().iter_mut() {
            crate::gc::mark_object(args as *mut i64);
        }
    });
    GLOBAL_HANDLES.with(|handles| {
        for val in handles.borrow_mut().values_mut() {
            crate::gc::mark_object(val as *mut i64);
        }
    });
}

pub unsafe fn rt_gc_update_tasks() {
    TASK_QUEUE.with(|q| {
        for (_, ref mut args) in q.borrow_mut().iter_mut() {
            crate::gc::rt_update_ptr(args as *mut i64);
        }
    });
    GLOBAL_HANDLES.with(|handles| {
        for val in handles.borrow_mut().values_mut() {
            crate::gc::rt_update_ptr(val as *mut i64);
        }
    });
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

unsafe fn log_exception(prefix: &str, exception: i64) {
    let mut v = exception;
    rt_push_root(&mut v);

    let mut s_id = rt_to_string(v);
    rt_push_root(&mut s_id);

    let mut t_id = rt_typeof(v);
    rt_push_root(&mut t_id);

    let _ = std::io::stderr().write_all(prefix.as_bytes());
    let _ = std::io::stderr().write_all(b" (");
    if let Some((data, len)) = get_str_parts(t_id) {
        let slice = std::slice::from_raw_parts(data, len as usize);
        let _ = std::io::stderr().write_all(slice);
    } else {
        let _ = std::io::stderr().write_all(b"unknown");
    }
    let _ = std::io::stderr().write_all(b"): ");

    if let Some((data, len)) = get_str_parts(s_id) {
        let slice = std::slice::from_raw_parts(data, len as usize);
        let _ = std::io::stderr().write_all(slice);
    } else {
        let _ = std::io::stderr().write_all(b"<unprintable>");
    }
    let _ = std::io::stderr().write_all(b"\n");

    rt_pop_roots(3);
}

#[no_mangle]
pub unsafe extern "C" fn tejx_get_exception() -> i64 {
    CURRENT_EXCEPTION
}

#[no_mangle]
pub unsafe extern "C" fn tejx_push_handler(jmpbuf: *mut u8) {
    let top = MY_CONTEXT.with(|ctx: &std::cell::UnsafeCell<Box<ThreadContext>>| {
        let ctx_ptr = (*ctx.get()).as_mut() as *mut ThreadContext;
        (*ctx_ptr).roots_top
    });
    if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.push(ExceptionHandler {
            jmpbuf: jmpbuf as i64,
            roots_top: top,
        });
    }
}

extern "C" {
    fn longjmp(env: *mut i8, val: i32);
}

#[no_mangle]
pub unsafe extern "C" fn tejx_pop_handler() {
    if let Ok(mut stack) = EXCEPTION_STACK.lock() {
        stack.pop();
    }
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
        log_exception("Throw", exception);
        MY_CONTEXT.with(|ctx: &std::cell::UnsafeCell<Box<ThreadContext>>| {
            let ctx_ptr = (*ctx.get()).as_mut() as *mut ThreadContext;
            (*ctx_ptr).roots_top = h.roots_top;
        });
        longjmp(h.jmpbuf as *mut i8, 1);
    } else {
        log_exception("UnhandledException", exception);
        exit(1);
    }
}
