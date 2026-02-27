use crate::runtime::rt_to_number;
use std::collections::HashSet;

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("sqrt".to_string());
    s.insert("sin".to_string());
    s.insert("cos".to_string());
    s.insert("pow".to_string());
    s.insert("abs".to_string());
    s.insert("ceil".to_string());
    s.insert("floor".to_string());
    s.insert("round".to_string());
    s.insert("random".to_string());
    s.insert("min".to_string());
    s.insert("max".to_string());
    s
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_sqrt(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.sqrt().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_sin(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.sin().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_cos(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.cos().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_pow(base: i64, exp: i64) -> i64 {
    let b = rt_to_number(base);
    let e = rt_to_number(exp);
    b.powf(e).to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_abs(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.abs().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_ceil(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.ceil().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_floor(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.floor().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_round(x: i64) -> i64 {
    let v = rt_to_number(x);
    v.round().to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_random() -> i64 {
    // Stub
    let v: f64 = 0.42;
    v.to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_min(a: i64, b: i64) -> i64 {
    let v1 = rt_to_number(a);
    let v2 = rt_to_number(b);
    v1.min(v2).to_bits() as i64
}

#[unsafe(no_mangle)]
pub extern "C" fn std_math_max(a: i64, b: i64) -> i64 {
    let v1 = rt_to_number(a);
    let v2 = rt_to_number(b);
    v1.max(v2).to_bits() as i64
}
