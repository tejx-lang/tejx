use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_Atomic_constructor(this: i64, val: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        return;
    }
    let atom = Box::new(AtomicI64::new(val));
    *ptr.offset(0) = Box::into_raw(atom) as i64;
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
