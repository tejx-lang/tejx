use super::*;

// ── Mutex ─────────────────────────────────────────────────────────────────────
// Replaced may::sync::Mutex with std::sync::Mutex.
// The old may::sync version called __os_semaphore_wait inside coroutines,
// adding ~64 KB of OS frames to every coroutine stack under contention.
// std::sync::Mutex parks the OS worker thread directly — correct and safe
// because our VThread scheduler manages its own pool of OS threads.

type TejxMutex = std::sync::Mutex<()>;
type TejxMutexArc = std::sync::Arc<TejxMutex>;
type TejxGuard<'a> = std::sync::MutexGuard<'a, ()>;

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_new() -> i64 {
    Box::into_raw(Box::new(TejxMutex::new(()))) as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_free(mutex: i64) {
    if mutex <= 0 { return; }
    HELD_MUTEX_GUARDS.with(|held| { held.borrow_mut().remove(&(mutex as usize)); });
    let _ = Box::from_raw(mutex as *mut TejxMutex);
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_mutex_object_finalizer);
    let mutex: Box<TejxMutexArc> = Box::new(std::sync::Arc::new(TejxMutex::new(())));
    *ptr.offset(0) = Box::into_raw(mutex) as i64;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_acquire(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() { return; }
    let mutex = (*mutex_ptr).clone();
    let guard = if crate::vthread::vt_is_vthread() {
        let mut spins = 0;
        loop {
            match mutex.try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    spins += 1;
                    if spins < 4 {
                        std::hint::spin_loop();
                    } else {
                        crate::vthread::vt_yield();
                    }
                }
            }
        }
    } else {
        let _guard = crate::ThreadIoGuard::new();
        mutex.lock().unwrap_or_else(|e| e.into_inner())
    };
    let static_guard: TejxGuard<'static> = std::mem::transmute::<TejxGuard<'_>, TejxGuard<'static>>(guard);
    let key = std::sync::Arc::as_ptr(&mutex) as usize;
    HELD_MUTEX_GUARDS.with(|held| {
        held.borrow_mut().insert(key, HeldMutexGuard { guard: static_guard, _mutex: Some(mutex) });
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_release(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let mutex_ptr = *ptr.offset(0) as *const TejxMutexArc;
    if mutex_ptr.is_null() { return; }
    let key = std::sync::Arc::as_ptr(&*mutex_ptr) as usize;
    HELD_MUTEX_GUARDS.with(|held| { held.borrow_mut().remove(&key); });
}

#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_lock(mutex: i64) {
    if mutex <= 0 { return; }
    let mutex_ptr = mutex as *const TejxMutex;
    if mutex_ptr.is_null() { return; }
    let guard = if crate::vthread::vt_is_vthread() {
        let mut spins = 0;
        loop {
            match (*mutex_ptr).try_lock() {
                Ok(g) => break g,
                Err(std::sync::TryLockError::Poisoned(p)) => break p.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    spins += 1;
                    if spins < 4 {
                        std::hint::spin_loop();
                    } else {
                        crate::vthread::vt_yield();
                    }
                }
            }
        }
    } else {
        let _guard = crate::ThreadIoGuard::new();
        (*mutex_ptr).lock().unwrap_or_else(|e| e.into_inner())
    };
    let static_guard: TejxGuard<'static> = std::mem::transmute::<TejxGuard<'_>, TejxGuard<'static>>(guard);
    HELD_MUTEX_GUARDS.with(|held| {
        held.borrow_mut().insert(mutex_ptr as usize, HeldMutexGuard { guard: static_guard, _mutex: None });
    });
}
