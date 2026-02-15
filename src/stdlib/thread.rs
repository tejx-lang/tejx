use std::collections::HashSet;

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("Thread".to_string());
    s.insert("Mutex".to_string());
    s.insert("Atomic".to_string());
    s.insert("Condition".to_string());
    s.insert("spawn".to_string());
    s.insert("sleep".to_string());
    s
}
