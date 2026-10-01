use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Mutex;

// ── Mutex ─────────────────────────────────────────────────────────────────────
// VThread-aware Mutex:
// Works safely across virtual threads and OS worker threads without any thread-local
// guard maps or lifetime transmutes.

pub struct TejxMutex {
    state: AtomicBool,
    cvar: Condvar,
    wait_lock: StdMutex<()>,
}

impl TejxMutex {
    pub fn new() -> Self {
        Self {
            state: AtomicBool::new(false),
            cvar: Condvar::new(),
            wait_lock: StdMutex::new(()),
        }
    }

    pub fn lock(&self) {
        let mut spins = 0;
        loop {
            if self.state.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                return;
            }
            if crate::vthread::vt_is_vthread() {
                spins += 1;
                if spins < 8 {
                    std::hint::spin_loop();
                } else if spins < 16 {
                    crate::vthread::vt_yield();
                } else {
                    crate::vthread::vt_sleep(1);
                }
            } else {
                let _guard = crate::ThreadIoGuard::new();
                let mut guard = self.wait_lock.lock().unwrap();
                while self.state.load(Ordering::Acquire) {
                    guard = self.cvar.wait(guard).unwrap();
                }
            }
        }
    }

    pub fn unlock(&self) {
        self.state.store(false, Ordering::Release);
        self.cvar.notify_one();
    }
}

pub type TejxMutexArc = Arc<TejxMutex>;

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_new() -> i64 {
    Box::into_raw(Box::new(TejxMutex::new())) as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_free(mutex: i64) {
    if mutex <= 0 { return; }
    let _ = Box::from_raw(mutex as *mut TejxMutex);
}

static LIVE_MUTEX_DATA: Lazy<Mutex<HashSet<usize>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub(crate) unsafe fn register_mutex_data(ptr: *mut TejxMutexArc) -> i64 {
    let addr = ptr as usize;
    if let Ok(mut set) = LIVE_MUTEX_DATA.lock() {
        set.insert(addr);
    }
    addr as i64
}

pub(crate) unsafe fn unregister_mutex_data(addr: usize) -> bool {
    if let Ok(mut set) = LIVE_MUTEX_DATA.lock() {
        set.remove(&addr)
    } else {
        false
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_mutex_object_finalizer);
    let mutex: Box<TejxMutexArc> = Box::new(Arc::new(TejxMutex::new()));
    *ptr.offset(0) = register_mutex_data(Box::into_raw(mutex));
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_acquire(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() { return; }
    let arc = (*mutex_ptr).clone();
    arc.lock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_release(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() { return; }
    let arc = (*mutex_ptr).clone();
    arc.unlock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_lock(mutex: i64) {
    if mutex <= 0 { return; }
    let mutex_ptr = mutex as *const TejxMutex;
    if mutex_ptr.is_null() { return; }
    (*mutex_ptr).lock();
}
