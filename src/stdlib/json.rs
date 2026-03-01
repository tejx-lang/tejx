use crate::runtime::{HEAP, TaggedValue, new_fast_map, rt_box_string};

use std::ffi::CString;

fn stringify_json_recursive(id: i64) -> String {
    let heap = HEAP.lock().unwrap();
    if let Some(obj) = heap.get(id) {
        match obj {
            TaggedValue::Map(map) => {
                let entries: Vec<(String, i64)> = map
                    .iter_entries()
                    .into_iter()
                    .filter(|(k, _v)| *k != "toString" && *k != "__proto__" && *k != "constructor")
                    .collect();
                drop(heap);
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("\"{}\":{}", k, stringify_json_recursive(*v)))
                    .collect();
                format!("{{{}}}", parts.join(","))
            }
            TaggedValue::Array(arr) => {
                let elements = arr.clone();
                drop(heap);
                let parts: Vec<String> = elements
                    .iter()
                    .map(|v| stringify_json_recursive(*v))
                    .collect();
                format!("[{}]", parts.join(","))
            }
            TaggedValue::String(s) => format!("\"{}\"", s),
            TaggedValue::Number(n) => {
                if n.fract() == 0.0 {
                    format!("{:.0}", n)
                } else {
                    format!("{}", n)
                }
            }
            TaggedValue::Boolean(b) => b.to_string(),
            _ => "null".to_string(),
        }
    } else {
        drop(heap);
        if id == 0 {
            return "null".to_string();
        }
        // Primitive literal fallback
        if id > -1000 && id < 1000 {
            return id.to_string();
        }
        // Bitcasted double fallback
        let f = f64::from_bits(id as u64);
        if !f.is_nan() && !f.is_infinite() {
            if f.fract() == 0.0 {
                format!("{:.0}", f)
            } else {
                format!("{}", f)
            }
        } else {
            "null".to_string()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_json_stringify(val: i64) -> i64 {
    let s = stringify_json_recursive(val);
    let c_str = CString::new(s).unwrap();
    unsafe { rt_box_string(c_str.into_raw() as i64) }
}

fn parse_val(chars: &mut std::iter::Peekable<std::str::Chars>) -> i64 {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        break;
    }

    let first = if let Some(&c) = chars.peek() {
        c
    } else {
        return 0;
    };

    match first {
        '{' => {
            chars.next();
            let mut map = new_fast_map();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                    continue;
                }
                if c == '}' {
                    chars.next();
                    break;
                }

                // Key
                let key_ptr = parse_val(chars);
                let key = if let Some(TaggedValue::String(s)) =
                    crate::runtime::HEAP.lock().unwrap().get(key_ptr)
                {
                    s.clone()
                } else {
                    String::new()
                };

                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == ':' {
                        chars.next();
                        continue;
                    }
                    break;
                }

                // Value
                let val = parse_val(chars);
                if !key.is_empty() {
                    map.insert(key, val);
                }

                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == ',' {
                        chars.next();
                        continue;
                    }
                    break;
                }
            }
            let mut heap = crate::runtime::HEAP.lock().unwrap();
            heap.alloc(TaggedValue::Map(map))
        }
        '[' => {
            chars.next();
            let mut arr = Vec::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    chars.next();
                    continue;
                }
                if c == ']' {
                    chars.next();
                    break;
                }

                arr.push(parse_val(chars));

                while let Some(&c) = chars.peek() {
                    if c.is_whitespace() || c == ',' {
                        chars.next();
                        continue;
                    }
                    break;
                }
            }
            let mut heap = crate::runtime::HEAP.lock().unwrap();
            heap.alloc(TaggedValue::Array(arr))
        }
        '"' => {
            chars.next();
            let mut s = String::new();
            while let Some(c) = chars.next() {
                if c == '"' {
                    break;
                }
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        match next {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            'r' => s.push('\r'),
                            _ => s.push(next),
                        }
                    }
                } else {
                    s.push(c);
                }
            }
            let c_str = CString::new(s).unwrap();
            unsafe { rt_box_string(c_str.into_raw() as i64) }
        }
        't' => {
            for _ in 0..4 {
                chars.next();
            }
            crate::runtime::rt_box_boolean(1)
        }
        'f' => {
            for _ in 0..5 {
                chars.next();
            }
            crate::runtime::rt_box_boolean(0)
        }
        'n' => {
            for _ in 0..4 {
                chars.next();
            }
            0
        }
        _ if first.is_digit(10) || first == '-' => {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_digit(10) || c == '.' || c == '-' || c == 'e' || c == 'E' || c == '+' {
                    s.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            if let Ok(n) = s.parse::<f64>() {
                crate::runtime::rt_box_number(n)
            } else {
                0
            }
        }
        _ => {
            chars.next();
            0
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_json_parse(str_ptr: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::String(s)) = heap.get(str_ptr) {
        let s = s.clone();
        drop(heap);
        let mut chars = s.chars().peekable();
        return parse_val(&mut chars);
    }
    0
}
