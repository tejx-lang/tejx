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

pub struct SpinMutex<T> {
    lock: AtomicBool,
    data: std::cell::UnsafeCell<T>,
}

unsafe impl<T: Send> Send for SpinMutex<T> {}
unsafe impl<T: Send> Sync for SpinMutex<T> {}

pub struct SpinMutexGuard<'a, T> {
    mutex: &'a SpinMutex<T>,
}

impl<T> SpinMutex<T> {
    pub const fn new(data: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            data: std::cell::UnsafeCell::new(data),
        }
    }

    #[inline(always)]
    pub fn lock(&self) -> SpinMutexGuard<'_, T> {
        let mut backoff = 1;
        while self.lock.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            while self.lock.load(Ordering::Relaxed) {
                for _ in 0..backoff {
                    std::hint::spin_loop();
                }
                backoff = (backoff << 1).min(64);
            }
        }
        SpinMutexGuard { mutex: self }
    }
}

impl<'a, T> std::ops::Deref for SpinMutexGuard<'a, T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> std::ops::DerefMut for SpinMutexGuard<'a, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for SpinMutexGuard<'a, T> {
    #[inline(always)]
    fn drop(&mut self) {
        self.mutex.lock.store(false, Ordering::Release);
    }
}

pub struct TejxMutex {
    state: AtomicBool,
    cvar: Condvar,
    os_lock: StdMutex<()>,
    wait_lock: SpinMutex<Vec<usize>>,
}

impl TejxMutex {
    pub fn new() -> Self {
        Self {
            state: AtomicBool::new(false),
            cvar: Condvar::new(),
            os_lock: StdMutex::new(()),
            wait_lock: SpinMutex::new(Vec::new()),
        }
    }

    pub fn lock(&self) {
        // Fast path: uncontended acquire
        if self.state.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return;
        }

        // Short adaptive spin loop (for low contention)
        for _ in 0..16 {
            std::hint::spin_loop();
            if !self.state.load(Ordering::Relaxed)
                && self.state.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok()
            {
                return;
            }
        }

        if crate::vthread::vt_is_vthread() {
            let token = crate::vthread::vt_next_park_token();
            loop {
                {
                    let mut waiters = self.wait_lock.lock();
                    if self.state.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                        return;
                    }
                    waiters.push(token);
                }
                crate::vthread::vt_park(token);
                if self.state.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return;
                }
            }
        } else {
            let _guard = crate::ThreadIoGuard::new();
            let mut guard = self.os_lock.lock().unwrap();
            while self.state.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
                guard = self.cvar.wait(guard).unwrap();
            }
        }
    }

    pub fn unlock(&self) {
        self.state.store(false, Ordering::Release);
        let waiter = {
            let mut waiters = self.wait_lock.lock();
            if !waiters.is_empty() {
                Some(waiters.remove(0))
            } else {
                None
            }
        };
        if let Some(token) = waiter {
            crate::vthread::vt_unpark(token);
        } else {
            let _guard = self.os_lock.lock();
            self.cvar.notify_one();
        }
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

static LIVE_MUTEX_DATA: Lazy<SpinMutex<HashSet<usize>>> = Lazy::new(|| SpinMutex::new(HashSet::new()));

pub(crate) unsafe fn register_mutex_data(ptr: *mut TejxMutexArc) -> i64 {
    let addr = ptr as usize;
    LIVE_MUTEX_DATA.lock().insert(addr);
    addr as i64
}

pub(crate) unsafe fn unregister_mutex_data(addr: usize) -> bool {
    LIVE_MUTEX_DATA.lock().remove(&addr)
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
    (&**mutex_ptr).lock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_release(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() { return; }
    (&**mutex_ptr).unlock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_lock(mutex: i64) {
    if mutex <= 0 { return; }
    let mutex_ptr = mutex as *const TejxMutex;
    if mutex_ptr.is_null() { return; }
    (*mutex_ptr).lock();
}
