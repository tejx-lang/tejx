use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── Mutex ─────────────────────────────────────────────────────────────────────
// VThread-aware Mutex:
// Works safely across virtual threads and OS worker threads without any thread-local
// guard maps or lifetime transmutes.

pub struct SpinMutex<T>(std::sync::Mutex<T>);

pub type SpinMutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;

impl<T> SpinMutex<T> {
    pub const fn new(data: T) -> Self {
        Self(std::sync::Mutex::new(data))
    }

    #[inline(always)]
    pub fn lock(&self) -> SpinMutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[inline(always)]
    pub fn try_lock(&self) -> Option<SpinMutexGuard<'_, T>> {
        self.0.try_lock().ok()
    }
}

pub struct TejxMutex {
    state: AtomicBool,
    wait_lock: SpinMutex<Vec<usize>>,
    granted_token: std::sync::atomic::AtomicUsize,
}

impl TejxMutex {
    pub fn new() -> Self {
        Self {
            state: AtomicBool::new(false),
            wait_lock: SpinMutex::new(Vec::new()),
            granted_token: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn lock(&self) {
        // Fast path: uncontended acquire when no handoff is pending
        if self.granted_token.load(Ordering::Relaxed) == 0
            && self
                .state
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        {
            return;
        }

        // Short adaptive spin loop (for low contention)
        for _ in 0..32 {
            std::hint::spin_loop();
            if self.granted_token.load(Ordering::Relaxed) == 0
                && !self.state.load(Ordering::Relaxed)
                && self
                    .state
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
            {
                return;
            }
        }

        if crate::vthread::vt_is_vthread() {
            loop {
                let token = crate::vthread::vt_next_park_token();
                {
                    let mut waiters = self.wait_lock.lock();
                    if self.granted_token.load(Ordering::Relaxed) == 0
                        && self
                            .state
                            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                            .is_ok()
                    {
                        return;
                    }
                    waiters.push(token);
                }

                crate::vthread::vt_park(token);

                // Check if lock was handed off to us
                if self.granted_token.load(Ordering::Acquire) == token {
                    self.granted_token.store(0, Ordering::Release);
                    return;
                }

                // Or if lock became free
                if self.granted_token.load(Ordering::Relaxed) == 0
                    && self
                        .state
                        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                        .is_ok()
                {
                    let mut waiters = self.wait_lock.lock();
                    waiters.retain(|&t| t != token);
                    return;
                }

                // If not acquired, clean up old token before getting a new one
                {
                    let mut waiters = self.wait_lock.lock();
                    waiters.retain(|&t| t != token);
                }
            }
        } else {
            let mut backoff = 1;
            while self.granted_token.load(Ordering::Relaxed) != 0
                || self
                    .state
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_err()
            {
                for _ in 0..backoff {
                    std::hint::spin_loop();
                }
                if backoff < 64 {
                    backoff <<= 1;
                } else {
                    std::thread::yield_now();
                }
            }
        }
    }

    pub fn unlock(&self) {
        let waiter = {
            let mut waiters = self.wait_lock.lock();
            if !waiters.is_empty() {
                Some(waiters.remove(0))
            } else {
                None
            }
        };
        if let Some(token) = waiter {
            // Direct handoff: keep state = true, grant ownership directly to token
            self.granted_token.store(token, Ordering::Release);
            crate::vthread::vt_unpark(token);
        } else {
            self.granted_token.store(0, Ordering::Relaxed);
            self.state.store(false, Ordering::Release);
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
    if mutex <= 0 {
        return;
    }
    let _ = Box::from_raw(mutex as *mut TejxMutex);
}

pub(crate) unsafe fn register_mutex_data(ptr: *mut TejxMutexArc) -> i64 {
    ptr as i64
}

pub(crate) unsafe fn unregister_mutex_data(addr: usize) -> bool {
    addr != 0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    rt_ensure_type_finalizer(this, rt_mutex_object_finalizer);
    let mutex: Box<TejxMutexArc> = Box::new(Arc::new(TejxMutex::new()));
    *ptr.offset(0) = register_mutex_data(Box::into_raw(mutex));
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_acquire(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return;
    }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() {
        return;
    }
    (&**mutex_ptr).lock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_release(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return;
    }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() {
        return;
    }
    (&**mutex_ptr).unlock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_lock(mutex: i64) {
    if mutex <= 0 {
        return;
    }
    let mutex_ptr = mutex as *const TejxMutex;
    if mutex_ptr.is_null() {
        return;
    }
    (*mutex_ptr).lock();
}
