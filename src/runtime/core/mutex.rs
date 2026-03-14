use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_new() -> i64 {
    let boxed = Box::new(std::sync::Mutex::new(()));
    Box::into_raw(boxed) as i64
}
#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    let mutex = Box::new(std::sync::Mutex::new(()));
    *ptr.offset(0) = Box::into_raw(mutex) as i64;
}
#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_acquire(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return;
    }
    let mutex_ptr = *ptr.offset(0) as *const std::sync::Mutex<()>;
    if mutex_ptr.is_null() {
        return;
    }
    let guard = (*mutex_ptr)
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<std::sync::MutexGuard<'_, ()>>| e.into_inner());
    let static_guard: std::sync::MutexGuard<'static, ()> = std::mem::transmute::<
        std::sync::MutexGuard<'_, ()>,
        std::sync::MutexGuard<'static, ()>,
    >(guard);
    HELD_MUTEX_GUARDS.with(|held| {
        held.borrow_mut().insert(mutex_ptr as usize, static_guard);
    });
}
#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_release(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return;
    }
    let mutex_ptr = *ptr.offset(0) as *const std::sync::Mutex<()>;
    if mutex_ptr.is_null() {
        return;
    }
    HELD_MUTEX_GUARDS.with(|held| {
        held.borrow_mut().remove(&(mutex_ptr as usize));
    });
}
#[no_mangle]
pub unsafe extern "C" fn rt_Mutex_lock(mutex: i64) {
    if mutex <= 0 {
        return;
    }
    let mutex_ptr = mutex as *const std::sync::Mutex<()>;
    if mutex_ptr.is_null() {
        return;
    }
    let guard = (*mutex_ptr)
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<std::sync::MutexGuard<'_, ()>>| e.into_inner());
    let static_guard: std::sync::MutexGuard<'static, ()> = std::mem::transmute::<
        std::sync::MutexGuard<'_, ()>,
        std::sync::MutexGuard<'static, ()>,
    >(guard);
    HELD_MUTEX_GUARDS.with(|held| {
        held.borrow_mut().insert(mutex_ptr as usize, static_guard);
    });
}
