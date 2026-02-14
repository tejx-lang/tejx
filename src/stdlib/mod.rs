use std::collections::{HashMap, HashSet};

pub mod math;
pub mod fs;
pub mod os;
pub mod time;
pub mod json;
pub mod prelude;
pub mod collections;

pub struct StdLib {
    modules: HashMap<String, HashSet<String>>,
    prelude: HashSet<String>,
}

impl StdLib {
    pub fn new() -> Self {
        let mut modules = HashMap::new();
        
        modules.insert("math".to_string(), math::exports());
        modules.insert("fs".to_string(), fs::exports());
        modules.insert("os".to_string(), os::exports());
        modules.insert("time".to_string(), time::exports());
        modules.insert("json".to_string(), HashSet::from(["stringify".to_string(), "parse".to_string()]));
        modules.insert("collections".to_string(), collections::exports());
        
        // Add all methods to collections
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
        // Special case handling if needed, otherwise standard naming std_{mod}_{func}
        // or just return the runtime name if it differs.
        // For now, consistent naming:
        format!("std_{}_{}", mod_name, func_name)
    }

    /// Checks if a function name (like "std_math_sqrt" or "sqrt") is a known runtime function
    /// Returns the canonical runtime name if found.
    pub fn resolve_runtime_func(&self, name: &str) -> Option<String> {
        // Check prelude
        if self.prelude.contains(name) {
            return Some(name.to_string());
        }

        // Check explicit std_mod_func names
        if name.starts_with("std_") {
            // minimal validation? 
            // Better: iterate modules
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

        // Check specialized runtime names (legacy support or internal)
        // like "Math_pow" -> mapped to std:math:pow? 
        // No, we are moving to std:math:pow -> std_math_pow. 
        // But we have legacy "Math_pow" in runtime.rs?
        // We should probably update runtime.rs too eventually, but for now `lowering` handles the mapping.
        // Wait, `lowering` currently checks `is_runtime_func` against a hardcoded list that includes "Math_pow".
        // If we want to replace `is_runtime_func`, we need that list here or we need to rename runtime functions.
        // Renaming runtime functions is a bigger task.
        // For this refactor, let's keep the hardcoded "legacy" runtime list for non-std functions (like Array methods),
        // but delegate std module checks to this struct.
        None
    }
}
