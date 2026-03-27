use super::*;

use super::gc::{ThreadContext, MY_CONTEXT};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

// --- Async Task Queue ---
// TejX user callbacks still resume on a single event-loop thread, but async helpers
// may complete from different worker threads. Keep the queue/handle table global so
// wakeups and GC-visible handles stay correct across those boundaries.
type TejxTask = (i64, i64);

static TASK_QUEUE: LazyLock<Mutex<VecDeque<TejxTask>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));
static GLOBAL_HANDLES: LazyLock<Mutex<HashMap<usize, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

// Notify to wake up the blocked event loop when a task is enqueued
pub static TASK_NOTIFY: LazyLock<tokio::sync::Notify> =
    LazyLock::new(|| tokio::sync::Notify::const_new());

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn task_queue_is_empty() -> bool {
    lock_unpoisoned(&TASK_QUEUE).is_empty()
}

fn pop_ready_task() -> Option<TejxTask> {
    lock_unpoisoned(&TASK_QUEUE).pop_front()
}

unsafe fn run_task((worker, args): TejxTask) {
    if worker != 0 {
        let worker_fn: unsafe extern "C" fn(i64) = std::mem::transmute(worker);
        worker_fn(args);
    }
}

unsafe fn run_ready_tasks() -> usize {
    let mut ran = 0;
    while let Some(task) = pop_ready_task() {
        run_task(task);
        ran += 1;
    }
    ran
}

#[no_mangle]
pub unsafe extern "C" fn tejx_enqueue_task(worker: i64, args: i64) {
    lock_unpoisoned(&TASK_QUEUE).push_back((worker, args));
    TASK_NOTIFY.notify_one();
}

#[no_mangle]
pub unsafe extern "C" fn tejx_inc_async_ops() {
    ASYNC_OPS.fetch_add(1, Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn tejx_dec_async_ops() {
    let prev = ASYNC_OPS.fetch_sub(1, Ordering::SeqCst);
    if prev <= 1 {
        TASK_NOTIFY.notify_one();
    }
}

#[no_mangle]
pub unsafe extern "C" fn tejx_run_event_loop_step() -> bool {
    if run_ready_tasks() > 0 {
        return true;
    }

    if ASYNC_OPS.load(Ordering::SeqCst) <= 0 {
        return false;
    }

    // Process Tokio tasks to pull next events (I/O, timers, etc) into the Tejx queue.
    // Instead of busy-waiting with sleep, we block until a callback is enqueued or the
    // last outstanding async operation completes/cancels.
    TOKIO_RT.block_on(async {
        // Yield to the Tokio scheduler so any ready tasks execute immediately
        tokio::task::yield_now().await;

        // Only park if we are still waiting on outstanding async work after the yield.
        if task_queue_is_empty() && ASYNC_OPS.load(Ordering::SeqCst) > 0 {
            TASK_NOTIFY.notified().await;
        }
    });

    run_ready_tasks() > 0 || ASYNC_OPS.load(Ordering::SeqCst) > 0
}

#[no_mangle]
pub unsafe extern "C" fn tejx_run_event_loop() {
    while tejx_run_event_loop_step() {}
}

#[no_mangle]
pub unsafe extern "C" fn tejx_create_global_handle(ptr: i64) -> usize {
    let id = GLOBAL_HANDLE_NEXT_ID.fetch_add(1, Ordering::SeqCst);
    lock_unpoisoned(&GLOBAL_HANDLES).insert(id, ptr);
    id
}

#[no_mangle]
pub unsafe extern "C" fn tejx_get_global_handle(id: usize) -> i64 {
    *lock_unpoisoned(&GLOBAL_HANDLES).get(&id).unwrap_or(&0)
}

#[no_mangle]
pub unsafe extern "C" fn tejx_drop_global_handle(id: usize) {
    lock_unpoisoned(&GLOBAL_HANDLES).remove(&id);
}

pub unsafe fn rt_gc_scan_tasks() {
    for (_, ref mut args) in lock_unpoisoned(&TASK_QUEUE).iter_mut() {
        crate::gc::copy_object(args as *mut i64);
    }
    for val in lock_unpoisoned(&GLOBAL_HANDLES).values_mut() {
        crate::gc::copy_object(val as *mut i64);
    }
}

pub unsafe fn rt_gc_mark_tasks() {
    for (_, ref mut args) in lock_unpoisoned(&TASK_QUEUE).iter_mut() {
        crate::gc::mark_object(args as *mut i64);
    }
    for val in lock_unpoisoned(&GLOBAL_HANDLES).values_mut() {
        crate::gc::mark_object(val as *mut i64);
    }
}

pub unsafe fn rt_gc_update_tasks() {
    for (_, ref mut args) in lock_unpoisoned(&TASK_QUEUE).iter_mut() {
        crate::gc::rt_update_ptr(args as *mut i64);
    }
    for val in lock_unpoisoned(&GLOBAL_HANDLES).values_mut() {
        crate::gc::rt_update_ptr(val as *mut i64);
    }
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
