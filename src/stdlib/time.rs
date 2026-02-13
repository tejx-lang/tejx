use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use std::thread;
use crate::runtime::rt_to_number;

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("now".to_string());
    s.insert("sleep".to_string());
    s
}

#[unsafe(no_mangle)]
pub extern "C" fn std_time_now(_this: i64) -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    (duration.as_millis() as f64).to_bits() as i64
}

#[unsafe(no_mangle)] 
pub extern "C" fn std_time_sleep(ms: i64) -> i64 {
    let duration = Duration::from_millis(rt_to_number(ms) as u64);
    thread::sleep(duration);
    0
}
