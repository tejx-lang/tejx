use crate::runtime::{
    __resolve_promise, ACTIVE_ASYNC_OPS, HEAP, Promise_new, TaggedValue, tejx_enqueue_task,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static TIMERS: LazyLock<Mutex<HashMap<i64, Arc<AtomicBool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_TIMER_ID: AtomicI64 = AtomicI64::new(1);

// --- Exports ---

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("sleep".to_string());
    s.insert("delay".to_string());
    s.insert("now".to_string());
    s.insert("setTimeout".to_string());
    s.insert("setInterval".to_string());
    s.insert("clearTimeout".to_string());
    s.insert("clearInterval".to_string());
    s
}

/// Extract the f64 milliseconds from a heap-boxed number ID
unsafe fn unbox_ms(ms_id: i64) -> u64 {
    let heap = HEAP.lock().unwrap();
    match heap.get(ms_id) {
        Some(TaggedValue::Number(n)) => *n as u64,
        _ => ms_id as u64, // fallback: treat as raw integer
    }
}

// --- Sync APIs ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_sleep(ms_id: i64) -> i64 {
    let ms = unsafe { unbox_ms(ms_id) };
    thread::sleep(Duration::from_millis(ms));
    0 // void return
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_now() -> i64 {
    let start = SystemTime::now();
    let since_the_epoch = start
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let millis = since_the_epoch.as_millis() as f64;
    millis.to_bits() as i64
}

// --- Async APIs ---

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_delay(ms_id: i64) -> i64 {
    let ms = unsafe { unbox_ms(ms_id) };
    let pid = { Promise_new(0) };

    // Synchronous sleep + resolve on the same thread.
    // Since async TejX functions already run on worker threads via tejx_enqueue_task,
    // spawning another thread would cause thread-local HEAP isolation issues.
    thread::sleep(Duration::from_millis(ms));
    {
        __resolve_promise(pid, 0)
    };

    pid
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_setTimeout(callback: i64, ms_id: i64) -> i64 {
    let ms = unsafe { unbox_ms(ms_id) };
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));

    {
        let mut timers = TIMERS.lock().unwrap();
        timers.insert(timer_id, cancelled.clone());
    }

    ACTIVE_ASYNC_OPS.fetch_add(1, Ordering::SeqCst);

    thread::spawn(move || {
        thread::sleep(Duration::from_millis(ms));

        if !cancelled.load(Ordering::SeqCst) {
            unsafe { tejx_enqueue_task(callback, 0) }; // 0 as dummy arg for now

            // Clean up from map
            let mut timers = TIMERS.lock().unwrap();
            timers.remove(&timer_id);
        }

        ACTIVE_ASYNC_OPS.fetch_sub(1, Ordering::SeqCst);
    });

    timer_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_setInterval(callback: i64, ms_id: i64) -> i64 {
    let ms = unsafe { unbox_ms(ms_id) };
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));

    {
        let mut timers = TIMERS.lock().unwrap();
        timers.insert(timer_id, cancelled.clone());
    }

    ACTIVE_ASYNC_OPS.fetch_add(1, Ordering::SeqCst);

    thread::spawn(move || {
        while !cancelled.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(ms));
            if !cancelled.load(Ordering::SeqCst) {
                unsafe { tejx_enqueue_task(callback, 0) };
            }
        }
        ACTIVE_ASYNC_OPS.fetch_sub(1, Ordering::SeqCst);
    });

    timer_id
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_clearTimeout(id: i64) -> i64 {
    let mut timers = TIMERS.lock().unwrap();
    if let Some(cancelled) = timers.remove(&id) {
        cancelled.store(true, Ordering::SeqCst);
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_time_clearInterval(id: i64) -> i64 {
    unsafe { std_time_clearTimeout(id) }
}
