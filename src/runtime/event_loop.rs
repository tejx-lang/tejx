use super::*;

use super::gc::{ThreadContext, MY_CONTEXT};
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Condvar, LazyLock, Mutex};

// --- Async Task Queue ---
// TejX user callbacks still resume on a single event-loop thread, but async helpers
// may complete from different worker threads. Keep the queue/handle table global so
// wakeups and GC-visible handles stay correct across those boundaries.


#[derive(Default)]
struct GlobalHandles {
    slots: Vec<Option<i64>>,
    free: Vec<usize>,
}

static GLOBAL_HANDLES: LazyLock<Mutex<GlobalHandles>> =
    LazyLock::new(|| Mutex::new(GlobalHandles::default()));





fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn tejx_create_global_handle(ptr: i64) -> usize {
    let mut handles = lock_unpoisoned(&GLOBAL_HANDLES);
    if handles.slots.is_empty() {
        handles.slots.push(None);
    }
    if let Some(id) = handles.free.pop() {
        handles.slots[id] = Some(ptr);
        return id;
    }
    handles.slots.push(Some(ptr));
    handles.slots.len() - 1
}

#[no_mangle]
pub unsafe extern "C" fn tejx_get_global_handle(id: usize) -> i64 {
    let handles = lock_unpoisoned(&GLOBAL_HANDLES);
    handles.slots.get(id).and_then(|slot| *slot).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn tejx_drop_global_handle(id: usize) {
    let mut handles = lock_unpoisoned(&GLOBAL_HANDLES);
    if let Some(slot) = handles.slots.get_mut(id) {
        if slot.take().is_some() {
            handles.free.push(id);
        }
    }
}

pub unsafe fn rt_gc_scan_tasks() {
    crate::gc::copy_object(std::ptr::addr_of_mut!(CURRENT_EXCEPTION));
    for val in lock_unpoisoned(&GLOBAL_HANDLES).slots.iter_mut().flatten() {
        crate::gc::copy_object(val as *mut i64);
    }
}

pub unsafe fn rt_gc_mark_tasks() {
    crate::gc::mark_object(std::ptr::addr_of_mut!(CURRENT_EXCEPTION));
    for val in lock_unpoisoned(&GLOBAL_HANDLES).slots.iter_mut().flatten() {
        crate::gc::mark_object(val as *mut i64);
    }
}

pub unsafe fn rt_gc_update_tasks() {
    crate::gc::rt_update_ptr(std::ptr::addr_of_mut!(CURRENT_EXCEPTION));
    for val in lock_unpoisoned(&GLOBAL_HANDLES).slots.iter_mut().flatten() {
        crate::gc::rt_update_ptr(val as *mut i64);
    }
}

// --- Exception Handling ---
// TejX uses setjmp/longjmp style exception handling.
// We maintain a stack of jump buffers for try/catch blocks.

struct ExceptionHandler {
    jmpbuf: i64,
    roots_top: usize,
    frame_depth: usize,
}

static EXCEPTION_STACK: LazyLock<Mutex<Vec<ExceptionHandler>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;
    use std::time::Duration;

    static GC_VISIBLE_BATCH_ARG_OK: AtomicBool = AtomicBool::new(false);
    static FAIRNESS_TASK_RAN: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn recursive_microtask_worker(remaining: i64) {
        if remaining > 0 {
            tejx_enqueue_microtask(
                recursive_microtask_worker as *const () as i64,
                remaining - 1,
            );
        }
    }

    unsafe extern "C" fn gc_visibility_batch_first_worker(_arg: i64) {
        crate::gc::minor_gc();
        let bytes = b"overwrite-after-gc";
        for _ in 0..4096 {
            let _ = crate::new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64);
        }
    }

    unsafe extern "C" fn gc_visibility_batch_second_worker(arg: i64) {
        GC_VISIBLE_BATCH_ARG_OK.store(
            crate::rt_strlen(arg) == b"queued-microtask-arg".len() as i64,
            Ordering::SeqCst,
        );
    }

    unsafe extern "C" fn fairness_task_worker(_arg: i64) {
        FAIRNESS_TASK_RAN.store(true, Ordering::SeqCst);
    }

    #[test]
    fn tokio_runtime_advances_background_tasks_without_block_on() {
        let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
        let (tx, rx) = mpsc::channel();

        TOKIO_RT.spawn(async move {
            let _ = tx.send(());
        });

        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_ok(),
            "background async work should progress without explicit block_on pumping"
        );
    }

    #[test]
    fn global_handle_ids_are_reused_after_drop() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            let first = tejx_create_global_handle(11);
            tejx_drop_global_handle(first);
            let second = tejx_create_global_handle(22);
            assert_eq!(first, second);
            assert_eq!(tejx_get_global_handle(second), 22);
            tejx_drop_global_handle(second);
        }
    }

    #[test]
    fn recursive_microtasks_do_not_starve_ready_tasks() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            FAIRNESS_TASK_RAN.store(false, Ordering::SeqCst);
            while tejx_run_event_loop_step() {}

            tejx_enqueue_microtask(recursive_microtask_worker as *const () as i64, 32);
            tejx_enqueue_task(fairness_task_worker as *const () as i64, 0);

            assert!(tejx_run_event_loop_step());
            assert!(
                FAIRNESS_TASK_RAN.load(Ordering::SeqCst),
                "ready tasks should make progress even when microtasks keep chaining"
            );

            while tejx_run_event_loop_step() {}
        }
    }

    #[test]
    fn drained_microtask_batches_remain_gc_visible() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            GC_VISIBLE_BATCH_ARG_OK.store(false, Ordering::SeqCst);
            while tejx_run_event_loop_step() {}

            let payload = b"queued-microtask-arg";
            let mut arg = crate::new_string_from_bytes(payload.as_ptr(), payload.len() as i64);
            crate::gc::rt_push_root(&mut arg);

            tejx_enqueue_microtask(gc_visibility_batch_first_worker as *const () as i64, 0);
            tejx_enqueue_microtask(gc_visibility_batch_second_worker as *const () as i64, arg);

            crate::gc::rt_pop_roots(1);

            assert!(tejx_run_event_loop_step());
            assert!(
                GC_VISIBLE_BATCH_ARG_OK.load(Ordering::SeqCst),
                "queued microtask args must stay visible to GC while earlier batch items run"
            );

            while tejx_run_event_loop_step() {}
        }
    }

    #[test]
    fn current_exception_stays_gc_visible() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let payload = b"exception-root-after-gc";
            let mut exception =
                crate::new_string_from_bytes(payload.as_ptr(), payload.len() as i64);
            crate::gc::rt_push_root(&mut exception);
            CURRENT_EXCEPTION = exception;
            crate::gc::rt_pop_roots(1);

            crate::gc::minor_gc();
            let bytes = b"overwrite-after-exception-gc";
            for _ in 0..4096 {
                let _ = crate::new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64);
            }

            assert_eq!(
                crate::rt_strlen(tejx_get_exception()),
                payload.len() as i64,
                "the active exception value must stay visible to GC"
            );

            CURRENT_EXCEPTION = 0;
        }
    }
}

static mut CURRENT_EXCEPTION: i64 = 0;

pub(crate) unsafe fn log_exception(prefix: &str, exception: i64) {
    let report = crate::render_runtime_exception_report(prefix, exception);
    let _ = std::io::stderr().write_all(report.as_bytes());
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
            frame_depth: crate::runtime_call_stack_depth(),
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
    let exception = crate::runtime_prepare_thrown_exception_value(exception);
    crate::remember_exception_trace(exception);
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
        crate::runtime_restore_call_stack(h.frame_depth);
        longjmp(h.jmpbuf as *mut i8, 1);
    } else {
        log_exception("UnhandledException", exception);
        exit(1);
    }
}
