/// Minimal runtime shim for the WASM compiler crate.
///
/// The core compiler's `lowering.rs` imports `crate::runtime::stdlib::StdLib`.
/// On native builds, `runtime.rs` is a 3800-line file with threads, networking,
/// longjmp, etc. For WASM, we only need the `StdLib` metadata struct.
/// This shim provides exactly that — pure data, no OS dependencies.

pub mod stdlib {
    use std::collections::{HashMap, HashSet};

    pub mod prelude {
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
    }

    pub struct StdLib {
        modules: HashMap<String, HashSet<String>>,
        prelude: HashSet<String>,
    }

    impl StdLib {
        pub fn new() -> Self {
            let mut modules = HashMap::new();

            // Math
            modules.insert("math".to_string(), HashSet::from([
                "sqrt".to_string(), "sin".to_string(), "cos".to_string(),
                "pow".to_string(), "abs".to_string(), "ceil".to_string(),
                "floor".to_string(), "round".to_string(), "random".to_string(),
                "min".to_string(), "max".to_string(),
            ]));

            // FS
            modules.insert("fs".to_string(), HashSet::from([
                "readFileSync".to_string(), "writeFileSync".to_string(),
                "appendFileSync".to_string(), "existsSync".to_string(),
                "unlinkSync".to_string(), "mkdirSync".to_string(),
                "readdirSync".to_string(), "readFile".to_string(),
                "writeFile".to_string(), "exists".to_string(),
                "write".to_string(), "remove".to_string(),
            ]));

            // System
            modules.insert("system".to_string(), HashSet::from([
                "args".to_string(), "exit".to_string(), "env".to_string(),
                "argv".to_string(), "os".to_string(), "system".to_string(),
            ]));

            // Time
            modules.insert("time".to_string(), HashSet::from([
                "sleep".to_string(), "delay".to_string(), "now".to_string(),
                "setTimeout".to_string(), "setInterval".to_string(),
                "clearTimeout".to_string(), "clearInterval".to_string(),
            ]));

            // JSON
            modules.insert("json".to_string(), HashSet::from([
                "stringify".to_string(), "parse".to_string(),
            ]));

            // Collections
            modules.insert("collections".to_string(), HashSet::from([
                "Stack".to_string(), "Queue".to_string(), "PriorityQueue".to_string(),
                "MinHeap".to_string(), "MaxHeap".to_string(), "Map".to_string(),
                "Set".to_string(), "OrderedMap".to_string(), "OrderedSet".to_string(),
                "BloomFilter".to_string(), "Trie".to_string(),
            ]));

            // Thread
            modules.insert("thread".to_string(), HashSet::from([
                "Thread".to_string(), "Mutex".to_string(), "Atomic".to_string(),
                "Condition".to_string(), "SharedQueue".to_string(),
                "spawn".to_string(), "sleep".to_string(),
            ]));

            // Net
            modules.insert("net".to_string(), HashSet::from([
                "connect".to_string(), "send".to_string(),
                "receive".to_string(), "close".to_string(),
            ]));

            // HTTP / HTTPS
            let http_methods: HashSet<String> = {
                let mut s = HashSet::new();
                for m in ["get", "post", "put", "delete", "patch", "head", "options"] {
                    s.insert(m.to_string());
                    s.insert(format!("{}Sync", m));
                }
                s
            };
            modules.insert("http".to_string(), http_methods.clone());
            modules.insert("https".to_string(), http_methods);

            // Collection methods
            if let Some(funcs) = modules.get_mut("collections") {
                let extra = [
                    "push", "pop", "peek", "enqueue", "dequeue", "insert", "extractMin",
                    "insertMax", "extractMax", "isEmpty", "size", "put", "at", "has",
                    "delete", "add", "clear", "contains", "find", "addPath"
                ];
                for f in extra { funcs.insert(f.to_string()); }
            }

            Self {
                modules,
                prelude: prelude::exports(),
            }
        }

        pub fn is_prelude_func(&self, name: &str) -> bool {
            self.prelude.contains(name)
        }

        pub fn is_std_func(&self, mod_name: &str, func_name: &str) -> bool {
            if let Some(funcs) = self.modules.get(mod_name) {
                funcs.contains(func_name)
            } else {
                false
            }
        }

        pub fn get_runtime_name(&self, mod_name: &str, func_name: &str) -> String {
            if mod_name == "thread" && func_name == "sleep" {
                return "std_time_sleep".to_string();
            }
            if mod_name == "thread" && func_name == "spawn" {
                return "Thread_new".to_string();
            }
            format!("std_{}_{}", mod_name, func_name)
        }

        pub fn resolve_runtime_func(&self, name: &str) -> Option<String> {
            if self.prelude.contains(name) {
                return Some(name.to_string());
            }

            if name.starts_with("std_") {
                for (mod_name, funcs) in &self.modules {
                    let prefix = format!("std_{}_", mod_name);
                    if name.starts_with(&prefix) {
                        let func_name = &name[prefix.len()..];
                        if funcs.contains(func_name) {
                            return Some(name.to_string());
                        }
                    }
                }
            }

            None
        }
    }
}
