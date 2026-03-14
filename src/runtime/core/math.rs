use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_math_sqrt(x: i64) -> i64 {
    let x_f = f64::from_bits(x as u64);
    let res = x_f.sqrt();
    res.to_bits() as i64
}
#[no_mangle]
pub unsafe extern "C" fn rt_math_pow(base: i64, exp: i64) -> i64 {
    let base_f = f64::from_bits(base as u64);
    let exp_f = f64::from_bits(exp as u64);
    let res = base_f.powf(exp_f);
    res.to_bits() as i64
}
#[no_mangle]
pub unsafe extern "C" fn rt_math_random() -> f64 {
    if RNG_STATE == 0 {
        use std::time::{SystemTime, UNIX_EPOCH};
        RNG_STATE = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
    }
    // xorshift64
    RNG_STATE ^= RNG_STATE << 13;
    RNG_STATE ^= RNG_STATE >> 7;
    RNG_STATE ^= RNG_STATE << 17;
    (RNG_STATE as f64) / (u64::MAX as f64)
}
