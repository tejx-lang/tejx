use super::*;
use crate::vthread;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Mutex;

/// Spawn a TejX Virtual Thread — Go/Java M:N model.
///
///  Property          │ Go goroutine   │ Java 21 VT   │ TejX VThread (this)
///  ──────────────────┼────────────────┼──────────────┼────────────────────
///  Concurrency model │ M:N            │ M:N          │ M:N
///  Stack start       │ 2–4 KB mmap    │ heap frames  │ 4 KB mmap pool
///  Stack growth      │ copy-on-overflow│ heap-extend │ copy-on-overflow
///  OS threads        │ GOMAXPROCS     │ ForkJoinPool │ num_cpus workers
///  I/O parking       │ netpoller      │ Selector     │ kqueue/epoll
///
/// Every spawn() allocates a 4 KB stack from the pool (or a fresh mmap if
/// the pool is empty).  The stack doubles on overflow — up to 64 MB — using
/// the copy-on-grow technique from Go's runtime.
/// No may, no generator, no fixed 2 MB overhead, no SIGSEGV crashes.
static LIVE_THREAD_DATA: Lazy<crate::SpinMutex<HashSet<usize>>> = Lazy::new(|| crate::SpinMutex::new(HashSet::new()));

pub(crate) unsafe fn register_thread_data(ptr: *mut ThreadData) -> i64 {
    let addr = ptr as usize;
    LIVE_THREAD_DATA.lock().insert(addr);
    addr as i64
}

pub(crate) unsafe fn unregister_thread_data(addr: usize) -> bool {
    LIVE_THREAD_DATA.lock().remove(&addr)
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_constructor(this: i64, cb: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_thread_object_finalizer);
    rt_store_ref_slot(this, ptr.offset(1), cb);
    let slot_live = std::sync::Arc::new(AtomicBool::new(true));
    let cb_released = std::sync::Arc::new(AtomicBool::new(false));
    let data = Box::new(ThreadData {
        handle: None,
        started: false,
        cb_slot: rt_add_static_root(cb),
        slot_live,
        cb_released,
    });
    *ptr.offset(0) = register_thread_data(Box::into_raw(data));
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_start(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    let data_ptr = *ptr.offset(0) as *mut ThreadData;
    if data_ptr.is_null() { return; }
    if (*data_ptr).started { return; }
    (*data_ptr).started = true;

    let cb_slot     = (*data_ptr).cb_slot;
    let slot_live   = (*data_ptr).slot_live.clone();
    let cb_released = (*data_ptr).cb_released.clone();

    // Spawn a VThread via the TejX M:N scheduler.
    // Initial stack: 2 KB from pool.
    vthread::vt_spawn_closure(
        move || {
            // Worker thread is already registered by worker_loop().
            // Just run the closure and release the GC cb_slot on exit exactly once.
            let _guard = ThreadRunGuard { cb_slot, cb_released };
            let closure = rt_get_static_root(cb_slot);
            if closure != 0 {
                rt_call_closure_no_args(closure);
            }
        },
        cb_slot,
        slot_live,
    );
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_join(mut this: i64) {
    let mut ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    let data_ptr = *ptr.offset(0) as *mut ThreadData;
    if data_ptr.is_null() { return; }
    if !(*data_ptr).started { rt_Thread_start(this); }

    let slot_live = (*data_ptr).slot_live.clone();
    let cb_slot = (*data_ptr).cb_slot;
    let cb_released = (*data_ptr).cb_released.clone();

    // Push `this` as root across `vt_join` so GC doesn't invalidate or relocate `this`
    rt_push_root(&mut this);
    vthread::vt_join(&slot_live);
    rt_pop_roots(1);

    // Re-resolve ptr after GC safepoints in vt_join!
    ptr = rt_obj_ptr(this);
    if !ptr.is_null() {
        let atomic_slot = ptr.offset(0) as *const AtomicI64;
        let reclaimed = (*atomic_slot).swap(0, Ordering::AcqRel);
        if reclaimed != 0 && unregister_thread_data(reclaimed as usize) {
            rt_release_thread_cb_slot(cb_slot, &cb_released);
            *ptr.offset(1) = 0;
            let _ = Box::from_raw(reclaimed as *mut ThreadData);
        }
    }
}

/// Sleep the current VThread.
/// The calling OS worker thread is freed immediately so it can run other
/// VThreads — identical to Go's `time.Sleep` parking the goroutine.
#[no_mangle]
pub unsafe extern "C" fn rt_Thread_sleep(ms: i64) {
    if ms > 0 {
        crate::vthread::vt_sleep(ms as u64);
    }
}
