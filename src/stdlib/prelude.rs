use std::collections::HashSet;

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("len".to_string());
    s.insert("abs".to_string());
    s.insert("min".to_string());
    s.insert("max".to_string());
    s.insert("assert".to_string());
    s.insert("print".to_string());
    s.insert("eprint".to_string());
    s.insert("panic".to_string());
    s.insert("process_exit".to_string());
    s.insert("process_argv".to_string());
    s
}
