use super::*;
use crate::vthread;
use std::sync::Arc;

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
pub(crate) unsafe fn register_thread_data(ptr: *mut Arc<ThreadData>) -> i64 {
    ptr as i64
}

pub(crate) unsafe fn unregister_thread_data(addr: usize) -> bool {
    addr != 0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_constructor(this: i64, cb: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_thread_object_finalizer);
    rt_store_ref_slot(this, ptr.offset(1), cb);
    let slot_live = std::sync::Arc::new(AtomicBool::new(true));
    let cb_released = std::sync::Arc::new(AtomicBool::new(false));
    let cb_slot = rt_add_static_root(cb);
    let data = Box::new(Arc::new(ThreadData {
        handle: None,
        started: AtomicBool::new(false),
        cb_slot,
        slot_live,
        cb_released,
    }));
    *ptr.offset(0) = register_thread_data(Box::into_raw(data));
}

#[no_mangle]
pub unsafe extern "C" fn rt_Thread_start(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    let arc_ptr = *ptr.offset(0) as *mut Arc<ThreadData>;
    if arc_ptr.is_null() { return; }
    let data = (*arc_ptr).clone();
    if data.started.swap(true, Ordering::SeqCst) { return; }

    let cb_slot     = data.cb_slot;
    let slot_live   = data.slot_live.clone();
    let cb_released = data.cb_released.clone();

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
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    let arc_ptr = *ptr.offset(0) as *mut Arc<ThreadData>;
    if arc_ptr.is_null() { return; }
    let data = (*arc_ptr).clone();

    if !data.started.load(Ordering::Acquire) {
        rt_Thread_start(this);
    }

    let slot_live = data.slot_live.clone();
    let cb_slot = data.cb_slot;
    let cb_released = data.cb_released.clone();

    // Push `this` as root across `vt_join` so GC doesn't invalidate or relocate `this`
    rt_push_root(&mut this);
    vthread::vt_join(&slot_live);
    rt_pop_roots(1);

    rt_release_thread_cb_slot(cb_slot, &cb_released);

    // Re-resolve ptr after GC safepoints in vt_join!
    let cur_ptr = rt_obj_ptr(this);
    if !cur_ptr.is_null() {
        let atomic_slot = cur_ptr.offset(0) as *const AtomicI64;
        let reclaimed = (*atomic_slot).swap(0, Ordering::AcqRel);
        if reclaimed != 0 && unregister_thread_data(reclaimed as usize) {
            *cur_ptr.offset(1) = 0;
            let _ = Box::from_raw(reclaimed as *mut Arc<ThreadData>);
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
