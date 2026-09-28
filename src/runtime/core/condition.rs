use super::*;

// ── Condition Variable ────────────────────────────────────────────────────────
// Replaced may::sync::Condvar with std::sync::Condvar.
// Works correctly on our VThread OS worker threads.

type TejxMutexArc = std::sync::Arc<std::sync::Mutex<()>>;
type TejxGuard<'a> = std::sync::MutexGuard<'a, ()>;

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_condition_object_finalizer);
    let data = Box::new(std::sync::Arc::new(ConditionData {
        condvar: std::sync::Condvar::new(),
    }));
    *ptr.offset(0) = Box::into_raw(data) as i64;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_wait(this: i64, mutex_arg: i64) {
    let cond_ptr  = rt_obj_ptr(this)       as *const i64;
    let mutex_obj = rt_obj_ptr(mutex_arg)  as *const i64;
    if cond_ptr.is_null() || mutex_obj.is_null() { return; }

    let cond_data = *cond_ptr.offset(0)  as *const std::sync::Arc<ConditionData>;
    let mutex_ptr = *mutex_obj.offset(0) as *const TejxMutexArc;
    if cond_data.is_null() || mutex_ptr.is_null() { return; }

    let cond_data = (*cond_data).clone();
    let mutex     = (*mutex_ptr).clone();
    let key       = std::sync::Arc::as_ptr(&mutex) as usize;

    // Retrieve the held guard for this mutex.
    let guard_opt = HELD_MUTEX_GUARDS.with(|held| held.borrow_mut().remove(&key));
    let guard: TejxGuard<'_> = match guard_opt {
        Some(g) => std::mem::transmute::<TejxGuard<'static>, TejxGuard<'_>>(g.guard),
        None    => mutex.lock().unwrap_or_else(|e| e.into_inner()),
    };

    // Wait — the OS worker thread blocks here, releasing the mutex.
    let new_guard = {
        let _guard = crate::ThreadIoGuard::new();
        cond_data.condvar.wait(guard).unwrap_or_else(|e| e.into_inner())
    };

    let static_guard: TejxGuard<'static> =
        std::mem::transmute::<TejxGuard<'_>, TejxGuard<'static>>(new_guard);
    HELD_MUTEX_GUARDS.with(|held| {
        held.borrow_mut().insert(key, HeldMutexGuard { guard: static_guard, _mutex: Some(mutex) });
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notify(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let data = *ptr.offset(0) as *const std::sync::Arc<ConditionData>;
    if data.is_null() { return; }
    (&(*data)).condvar.notify_one();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notifyAll(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let data = *ptr.offset(0) as *const std::sync::Arc<ConditionData>;
    if data.is_null() { return; }
    (&(*data)).condvar.notify_all();
}
