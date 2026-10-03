use super::*; // Extracted \n
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Mutex;

static LIVE_ATOMICS: Lazy<Mutex<HashSet<usize>>> = Lazy::new(|| Mutex::new(HashSet::new()));

pub unsafe fn register_atomic(ptr: *mut AtomicI64) -> i64 {
    let addr = ptr as usize;
    if let Ok(mut set) = LIVE_ATOMICS.lock() {
        set.insert(addr);
    }
    addr as i64
}

pub unsafe fn unregister_atomic(addr: usize) -> bool {
    if let Ok(mut set) = LIVE_ATOMICS.lock() {
        set.remove(&addr)
    } else {
        false
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_constructor(this: i64, val: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    rt_ensure_type_finalizer(this, rt_atomic_object_finalizer);
    let atom = Box::new(AtomicI64::new(val));
    *ptr.offset(0) = register_atomic(Box::into_raw(atom));
}
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_add(this: i64, val: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.fetch_add(val, Ordering::SeqCst)
    } else {
        0
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_sub(this: i64, val: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.fetch_sub(val, Ordering::SeqCst)
    } else {
        0
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_load(this: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.load(Ordering::SeqCst)
    } else {
        0
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_store(this: i64, val: i64) {
    if let Some(atom) = get_atomic(this) {
        atom.store(val, Ordering::SeqCst);
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_exchange(this: i64, val: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        atom.swap(val, Ordering::SeqCst)
    } else {
        0
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_compareExchange(this: i64, expected: i64, desired: i64) -> i64 {
    if let Some(atom) = get_atomic(this) {
        match atom.compare_exchange(expected, desired, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(v) => v,
            Err(v) => v,
        }
    } else {
        0
    }
}
