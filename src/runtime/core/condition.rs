use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use crate::mutex::TejxMutexArc;

// ── Condition Variable ────────────────────────────────────────────────────────

pub struct ConditionData {
    pub vt_waiters: crate::SpinMutex<Vec<usize>>,
    pub os_waiters: crate::SpinMutex<Vec<Arc<AtomicBool>>>,
}

impl ConditionData {
    pub fn new() -> Self {
        Self {
            vt_waiters: crate::SpinMutex::new(Vec::new()),
            os_waiters: crate::SpinMutex::new(Vec::new()),
        }
    }
}

pub(crate) unsafe fn register_condition_data(ptr: *mut Arc<ConditionData>) -> i64 {
    ptr as i64
}

pub(crate) unsafe fn unregister_condition_data(addr: usize) -> bool {
    addr != 0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_constructor(this: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return; }
    rt_ensure_type_finalizer(this, rt_condition_object_finalizer);
    let data = Box::new(Arc::new(ConditionData::new()));
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

    if crate::vthread::vt_is_vthread() {
        let token = crate::vthread::vt_next_park_token();
        {
            let mut waiters = cond_data.vt_waiters.lock();
            waiters.push(token);
        }
        mutex.unlock();
        crate::vthread::vt_park(token);
        mutex.lock();
    } else {
        let flag = Arc::new(AtomicBool::new(false));
        {
            let mut waiters = cond_data.os_waiters.lock();
            waiters.push(flag.clone());
        }
        mutex.unlock();
        while !flag.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        mutex.lock();
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notify(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let data = *ptr.offset(0) as *const Arc<ConditionData>;
    if data.is_null() { return; }
    let cond_data = &*data;

    let vt_waiter = {
        let mut waiters = cond_data.vt_waiters.lock();
        if !waiters.is_empty() {
            Some(waiters.remove(0))
        } else {
            None
        }
    };
    if let Some(token) = vt_waiter {
        crate::vthread::vt_unpark(token);
        return;
    }

    let os_waiter = {
        let mut waiters = cond_data.os_waiters.lock();
        if !waiters.is_empty() {
            Some(waiters.remove(0))
        } else {
            None
        }
    };
    if let Some(flag) = os_waiter {
        flag.store(true, Ordering::Release);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Condition_notifyAll(this: i64) {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() { return; }
    let data = *ptr.offset(0) as *const Arc<ConditionData>;
    if data.is_null() { return; }
    let cond_data = &*data;

    let vt_waiters: Vec<usize> = {
        let mut waiters = cond_data.vt_waiters.lock();
        waiters.drain(..).collect()
    };
    for token in vt_waiters {
        crate::vthread::vt_unpark(token);
    }

    let os_waiters: Vec<Arc<AtomicBool>> = {
        let mut waiters = cond_data.os_waiters.lock();
        waiters.drain(..).collect()
    };
    for flag in os_waiters {
        flag.store(true, Ordering::Release);
    }
}
