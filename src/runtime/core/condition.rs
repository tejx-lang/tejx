use super::*;
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use crate::mutex::TejxMutexArc;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Mutex;

// ── Condition Variable ────────────────────────────────────────────────────────

pub struct ConditionData {
    pub condvar: Condvar,
    pub lock: StdMutex<()>,
}

static LIVE_COND_DATA: Lazy<crate::SpinMutex<HashSet<usize>>> = Lazy::new(|| crate::SpinMutex::new(HashSet::new()));

pub(crate) unsafe fn register_condition_data(ptr: *mut Arc<ConditionData>) -> i64 {
    let addr = ptr as usize;
    LIVE_COND_DATA.lock().insert(addr);
    addr as i64
}

pub(crate) unsafe fn unregister_condition_data(addr: usize) -> bool {
    LIVE_COND_DATA.lock().remove(&addr)
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_condition_object_finalizer);
    let data = Box::new(Arc::new(ConditionData {
        condvar: Condvar::new(),
        lock: StdMutex::new(()),
    }));
    *ptr.offset(0) = register_condition_data(Box::into_raw(data));
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_wait(this: i64, mutex_arg: i64) {
    let cond_ptr  = rt_obj_ptr(this)       as *const i64;
    let mutex_obj = rt_obj_ptr(mutex_arg)  as *const i64;
    if cond_ptr.is_null() || mutex_obj.is_null() { return; }

    let cond_data = *cond_ptr.offset(0)  as *const Arc<ConditionData>;
    let mutex_ptr = *mutex_obj.offset(0) as *const TejxMutexArc;
    if cond_data.is_null() || mutex_ptr.is_null() { return; }

    let cond_data = (*cond_data).clone();
    let mutex     = (*mutex_ptr).clone();

    // Release the mutex
    mutex.unlock();

    // Wait on condition
    {
        let _guard = crate::ThreadIoGuard::new();
        let guard = cond_data.lock.lock().unwrap();
        drop(cond_data.condvar.wait(guard));
    }

    // Re-acquire mutex
    mutex.lock();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notify(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let data = *ptr.offset(0) as *const Arc<ConditionData>;
    if data.is_null() { return; }
    (&(*data)).condvar.notify_one();
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notifyAll(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let data = *ptr.offset(0) as *const Arc<ConditionData>;
    if data.is_null() { return; }
    (&(*data)).condvar.notify_all();
}
