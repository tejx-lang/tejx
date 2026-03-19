use serde_json::Value;

pub struct WasmCodeGen {
    config: Value,
    buffer: String,
    current_func: String,
    generated_ids: std::collections::HashSet<String>,
    signatures: std::collections::HashMap<String, usize>,
    table_indices: std::collections::HashMap<String, usize>,
    string_constants: Vec<String>,
}

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn tejx_compiler_log(ptr: *const u8, len: usize);
}

fn mangle(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push_str("m_");
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => out.push(c),
            ':' => out.push_str("_NS_"),
            '.' => out.push_str("_MB_"),
            '-' => out.push_str("_HY_"),
            '$' => out.push_str("_DO_"),
            _ => out.push_str(&format!("_X{:x}_", c as u32)),
        }
    }
    out
}

fn log_internal(s: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        tejx_compiler_log(s.as_ptr(), s.len());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        eprintln!("[WasmCodegen] {}", s);
    }
}

const F_PREFIX: &str = "f_";
const G_PREFIX: &str = "g_";
const L_PREFIX: &str = "l_";

impl WasmCodeGen {
    fn normalize_mir_name(name: &str) -> String {
        let mut clean = name;
        if clean.starts_with("f_rt_") { clean = &clean[5..]; }
        else if clean.starts_with("rt_") { clean = &clean[3..]; }
        else if clean.starts_with("f_") { clean = &clean[2..]; }
        
        if clean.starts_with("rt_") { clean = &clean[3..]; }

        if name.starts_with("rt_") || name.starts_with("f_rt_") {
            format!("rt_{}", clean)
        } else {
            clean.to_string()
        }
    }

    fn wasm_func_id(&self, mir_name: &str) -> String {
        if mir_name.is_empty() {
            log_internal("WARNING: wasm_func_id called with empty mir_name");
        }
        format!("{}{}", F_PREFIX, mangle(&Self::normalize_mir_name(mir_name)))
    }

    fn wasm_global_id(&self, mir_name: &str) -> String {
        format!("{}{}", G_PREFIX, mangle(mir_name))
    }

    fn wasm_local_id(&self, mir_name: &str) -> String {
        format!("{}{}", L_PREFIX, mangle(mir_name))
    }

    fn function_table_index(&self, mir_name: &str) -> Option<usize> {
        self.table_indices
            .get(mir_name)
            .copied()
            .or_else(|| self.table_indices.get(&Self::normalize_mir_name(mir_name)).copied())
    }

    fn string_offset(&self, s: &str) -> usize {
        let idx = self.string_constants.iter().position(|x| x == s).unwrap_or(0);
        1024 + self.string_constants[..idx].iter().map(|item| item.len() + 1).sum::<usize>()
    }

    fn push_string_constant(&mut self, s: &str) {
        if !self.string_constants.iter().any(|item| item == s) {
            self.string_constants.push(s.to_string());
        }
    }

    fn value_type_name(value: &Value) -> Option<&str> {
        value.as_object()?.get("ty")?.as_str()
    }

    fn cast_descriptor(src_ty: &str, dst_ty: &str) -> String {
        format!("{}->{}", src_ty, dst_ty)
    }

    fn needs_any_box(src_ty: &str) -> bool {
        matches!(src_ty, "Bool" | "Int" | "Int32" | "Int64" | "Float" | "Float32" | "Float64")
    }

    fn any_box_descriptor(value: &Value, element_ty: &str) -> Option<String> {
        if element_ty != "Any" {
            return None;
        }
        let src_ty = Self::value_type_name(value)?;
        if !Self::needs_any_box(src_ty) {
            return None;
        }
        Some(Self::cast_descriptor(src_ty, "Any"))
    }

    pub fn new(config: Value) -> Self {
        Self {
            config,
            buffer: String::new(),
            current_func: String::new(),
            generated_ids: std::collections::HashSet::new(),
            signatures: std::collections::HashMap::new(),
            table_indices: std::collections::HashMap::new(),
            string_constants: Vec::new(),
        }
    }

    /// Generic MIR-to-JSON Adapter (Zero-Change Trick)
    /// Converts a Rust Debug string into a generic JSON Value.
    pub fn debug_to_json(s: &str) -> serde_json::Value {
        let s = s.trim();
        if s == "None" {
            return serde_json::Value::Null;
        }
        if s.starts_with("Some(") && s.ends_with(')') {
            return Self::debug_to_json(&s[5..s.len() - 1]);
        }
        if s.starts_with('{') && s.ends_with('}') {
            serde_json::Value::String(s.to_string())
        } else if s.starts_with('[') && s.ends_with(']') {
            let content = &s[1..s.len() - 1];
            let mut items = Vec::new();
            let mut depth = 0;
            let mut start = 0;
            for (i, c) in content.char_indices() {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    ',' if depth == 0 => {
                        items.push(Self::debug_to_json(&content[start..i]));
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            if !content[start..].trim().is_empty() {
                items.push(Self::debug_to_json(&content[start..]));
            }
            serde_json::Value::Array(items)
        } else if s.contains('{') {
            if let Some((name, body)) = s.split_once('{') {
                let body = body.trim_end_matches('}');
                let mut map = serde_json::Map::new();
                map.insert("_type".to_string(), serde_json::Value::String(name.trim().to_string()));
                let mut depth = 0;
                let mut start = 0;
                for (i, c) in body.char_indices() {
                    match c {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' => depth -= 1,
                        ',' if depth == 0 => {
                            if let Some((k, v)) = body[start..i].split_once(':') {
                                map.insert(k.trim().to_string(), Self::debug_to_json(v));
                            }
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                if let Some((k, v)) = body[start..].split_once(':') {
                    map.insert(k.trim().to_string(), Self::debug_to_json(v));
                }
                serde_json::Value::Object(map)
            } else {
                serde_json::Value::String(s.to_string())
            }
        } else if s.contains('(') {
            if let Some((name, body)) = s.split_once('(') {
                let body = body.trim_end_matches(')');
                let mut map = serde_json::Map::new();
                map.insert("_type".to_string(), serde_json::Value::String(name.trim().to_string()));
                let mut items = Vec::new();
                let mut depth = 0;
                let mut start = 0;
                for (i, c) in body.char_indices() {
                    match c {
                        '(' | '[' | '{' => depth += 1,
                        ')' | ']' | '}' => depth -= 1,
                        ',' if depth == 0 => {
                            items.push(Self::debug_to_json(&body[start..i]));
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                if !body[start..].trim().is_empty() {
                    items.push(Self::debug_to_json(&body[start..]));
                }
                map.insert("_args".to_string(), serde_json::Value::Array(items));
                serde_json::Value::Object(map)
            } else {
                serde_json::Value::String(s.to_string())
            }
        } else {
            let s = s.trim().trim_matches('"');
            if let Ok(n) = s.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(b) = s.parse::<bool>() {
                serde_json::Value::Bool(b)
            } else {
                serde_json::Value::String(s.to_string())
            }
        }
    }

    fn emit(&mut self, s: &str) {
        self.buffer.push_str(s);
    }

    fn emit_line(&mut self, s: &str) {
        self.buffer.push_str("    ");
        self.buffer.push_str(s);
        self.buffer.push('\n');
    }

    pub fn generate_wat_generic(&mut self, mir_functions: Value) -> String {
        self.buffer.clear();
        self.string_constants.clear();
        self.generated_ids.clear();
        self.table_indices.clear();

        // 0. Pre-populate function sets to distinguish between local and FFI
        let mut extern_calls = std::collections::HashMap::new();
        let mut local_names = std::collections::HashSet::new();
        if let Some(funcs) = mir_functions.as_array() {
            for func in funcs {
                if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                    let is_extern = func.get("is_extern").and_then(|v| v.as_bool()).unwrap_or(false);
                    if is_extern {
                        let nargs = func.get("params").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                        extern_calls.insert(name.to_string(), nargs);
                    } else {
                        local_names.insert(name.to_string());
                        self.generated_ids.insert(format!("func:{}", name));
                    }
                }
            }
        }

        self.emit("(module\n");
        self.emit_line("(import \"env\" \"memory\" (memory 1))");

        // 1. Mangled Imports & Discovery
        if let Some(funcs) = mir_functions.as_array() {
            // Add discovered calls from local function bodies
            for func in funcs {
                if !func.get("is_extern").and_then(|v| v.as_bool()).unwrap_or(false) {
                    self.collect_extern_calls(func, &local_names, &mut extern_calls);
                }
            }

            // Add hardcoded runtime helpers used by the assembler
            // Note: These must use the SAME naming convention as is_extern functions in prelude.
            let helpers = [
                ("rt_alloc", 1), ("rt_free", 1), ("rt_print", 1), ("rt_panic", 1),
                ("rt_load_member", 2), ("rt_store_member", 3), ("rt_call_member", 3),
                ("rt_load_index", 2), ("rt_store_index", 3),
                ("rt_box_string", 1), ("rt_throw", 1), ("rt_cast", 2),
                ("rt_to_number_internal", 1), ("rt_box_number_internal", 1),
                ("rt_str_equals", 2), ("rt_op_equalequal", 2),
            ];
            for (h, a) in helpers {
                // Runtime helpers in MIR are usually named with f_ prefix (e.g. f_rt_print)
                // but the assembler refers to them as rt_print.
                // We'll normalize to the f_ prefixed version in extern_calls to match MIR.
                let mir_name = if h.starts_with("rt_") && !h.starts_with("f_") { format!("f_{}", h) } else { h.to_string() };
                extern_calls.insert(mir_name, a);
            }

            // 2. Builtin Types
                for n in 0..11 {
                    let mut params = String::new();
                    for _ in 0..n { params.push_str(" i64"); }
                    let p_spec = if n > 0 { format!("(param {})", params.trim()) } else { String::new() };
                    self.emit_line(&format!("    (type $t_func_{} (func {} (result i64)))", 
                        n, p_spec
                    ));
                }
            let mut params_counts = std::collections::HashMap::new();
            for (callee, nargs) in extern_calls.iter() {
                let wasm_id = self.wasm_func_id(callee);
                if self.generated_ids.contains(&wasm_id) { continue; }
                self.generated_ids.insert(wasm_id.clone());
                
                self.signatures.insert(wasm_id.clone(), *nargs);
                params_counts.insert(wasm_id.clone(), *nargs);

                let mut params_str = String::new();
                for _ in 0..*nargs { params_str.push_str(" i64"); }
                
                // Normalization is now centralized in wasm_func_id
                let env_name = Self::normalize_mir_name(callee);
                let wasm_id = self.wasm_func_id(callee);
                
                self.emit(&format!("    (import \"env\" \"{}\" (func ${} (type $t_func_{})))\n", 
                    env_name, wasm_id, nargs));
            }

            // Build dynamic elem table from all symbols we actually emitted
            let mut elem_items = Vec::new();
            
            // 1. Add all local functions
            if let Some(funcs) = mir_functions.as_array() {
                for func in funcs {
                    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                        if !func.get("is_extern").and_then(|v| v.as_bool()).unwrap_or(false) {
                            let table_index = elem_items.len();
                            self.table_indices.insert(name.to_string(), table_index);
                            self.table_indices
                                .entry(Self::normalize_mir_name(name))
                                .or_insert(table_index);
                            elem_items.push(format!("${}", self.wasm_func_id(name)));
                        }
                    }
                }
            }
            // 2. Add all imported functions (must be in the table to support indirect calls)
            for (callee, _) in extern_calls.iter() {
                let table_index = elem_items.len();
                self.table_indices
                    .entry(callee.to_string())
                    .or_insert(table_index);
                self.table_indices
                    .entry(Self::normalize_mir_name(callee))
                    .or_insert(table_index);
                let wasm_id = self.wasm_func_id(callee);
                elem_items.push(format!("${}", wasm_id));
            }

            if !elem_items.is_empty() {
                self.emit_line(&format!("(table {} funcref)", elem_items.len()));
                self.emit(&format!("(elem (i32.const 0) {})\n", elem_items.join(" ")));
            }
        }

        // 3. Globals
        let mut declared_globals = std::collections::HashSet::new();
        if let Some(funcs) = mir_functions.as_array() {
            for func in funcs {
                if let Some(vars) = func.get("variables").and_then(|v| v.as_object()) {
                    for var_name in vars.keys() {
                        if var_name.starts_with("g_") {
                            if declared_globals.contains(var_name) { continue; }
                            declared_globals.insert(var_name.clone());
                            self.emit_line(&format!("(global ${}{} (mut i64) (i64.const 0))", G_PREFIX, mangle(var_name)));
                        }
                    }
                }
            }
        }
        let mut globals_from_inst = std::collections::HashSet::new();
        if let Some(funcs) = mir_functions.as_array() {
            for func in funcs {
                self.collect_globals(func, &mut globals_from_inst);
            }
        }        // 2. Element/Table section handled above dynamically based on generated symbols

        for global in globals_from_inst {
            if !declared_globals.contains(&global) {
                declared_globals.insert(global.clone());
                self.emit_line(&format!("(global $g_{} (mut i64) (i64.const 0))", mangle(&global)));
            }
        }


        // 4. String Section
        if let Some(funcs) = mir_functions.as_array() {
            for func in funcs {
                self.collect_strings(func);
            }
        }
        let mut offset = 1024;
        for s in self.string_constants.clone().iter() {
            let escaped: String = s
                .as_bytes()
                .iter()
                .map(|&b| format!("\\{:02x}", b))
                .collect();
            self.emit_line(&format!(
                "(data (i32.const {}) \"{}\\00\")",
                offset, escaped
            ));
            offset += s.len() + 1;
        }

        // 5. Function Generation
        if let Some(funcs) = mir_functions.as_array() {
            for func in funcs {
                self.gen_func(func);
            }
        }

        // 6. Exports: Dynamically find main or tejx_main (only one!)
        if let Some(funcs) = mir_functions.as_array() {
            let mut exported = false;
            // Priority: tejx_main, then main
            for priority_name in &["tejx_main", "main", "f_tejx_main", "f_main"] {
                if exported { break; }
                for func in funcs {
                    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                        if name == *priority_name {
                            self.emit_line(&format!("(export \"main\" (func ${}))", self.wasm_func_id(name)));
                            exported = true;
                            break;
                        }
                    }
                }
            }
        }
        self.emit(")\n");
        log_internal("WAT generation complete in assembler.");
        std::mem::take(&mut self.buffer)
    }

    fn collect_from_value(&self, val: &Value, params: &std::collections::HashSet<String>, out: &mut std::collections::HashSet<String>) {
        if let Some(obj) = val.as_object() {
            if let Some(ty) = obj.get("_type").and_then(|v| v.as_str()) {
                if ty == "Variable" {
                    if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                        if !name.starts_with("g_") && !params.contains(name) {
                            out.insert(name.to_string());
                        }
                    }
                }
            }
            for v in obj.values() {
                self.collect_from_value(v, params, out);
            }
        } else if let Some(arr) = val.as_array() {
            for v in arr {
                self.collect_from_value(v, params, out);
            }
        }
    }

    fn collect_extern_calls(
        &self,
        val: &Value,
        local_names: &std::collections::HashSet<String>,
        extern_calls: &mut std::collections::HashMap<String, usize>,
    ) {
        if let Some(obj) = val.as_object() {
            if let Some(ty) = obj.get("_type").and_then(|v| v.as_str()) {
                match ty {
                    "Call" => {
                        if let Some(callee) = obj.get("callee").and_then(|v| v.as_str()) {
                            if !local_names.contains(callee) {
                                let nargs = obj
                                    .get("args")
                                    .and_then(|v| v.as_array())
                                    .map(|a| a.len())
                                    .unwrap_or(0);
                                // Unified normalization: MIR names are used directly
                                extern_calls.insert(callee.to_string(), nargs);
                            }
                        }
                    }
                    "BinaryOp" => {
                        let op = obj.get("op").and_then(|v| v.as_str()).unwrap_or("");
                        if !self.config.get("ops").and_then(|ops| ops.get(op)).is_some() {
                            let op_name = if op == "EqualEqual" { "rt_op_equalequal".to_string() }
                                            else { format!("rt_op_{}", op.to_lowercase()) };
                            extern_calls.insert(op_name, 2);
                        }
                    }
                    "LoadMember" => { extern_calls.insert("rt_load_member".to_string(), 2); }
                    "StoreMember" => { extern_calls.insert("rt_store_member".to_string(), 3); }
                    "LoadIndex" => { extern_calls.insert("rt_load_index".to_string(), 2); }
                    "StoreIndex" => { extern_calls.insert("rt_store_index".to_string(), 3); }
                    "Throw" => { extern_calls.insert("rt_throw".to_string(), 1); }
                    "Cast" => { extern_calls.insert("rt_cast".to_string(), 2); }
                    "Class" => { extern_calls.insert("rt_class_new".to_string(), 1); }
                    "Constant" => {
                        if let Some(s) = obj.get("value").and_then(|v| v.as_str()) {
                            if s.parse::<i64>().is_err() && s != "true" && s != "false" && s != "null" && s != "None" {
                                extern_calls.insert("rt_box_string".to_string(), 1);
                            }
                        }
                    }
                    "Print" => { extern_calls.insert("rt_print".to_string(), 1); }
                    "Len" => { extern_calls.insert("rt_len".to_string(), 1); }
                    _ => {}
                }
            }
            for v in obj.values() {
                self.collect_extern_calls(v, local_names, extern_calls);
            }
        } else if let Some(arr) = val.as_array() {
            for v in arr {
                self.collect_extern_calls(v, local_names, extern_calls);
            }
        }
    }

    fn collect_globals(&self, val: &Value, globals: &mut std::collections::HashSet<String>) {
        if let Some(obj) = val.as_object() {
            if let Some(ty) = obj.get("_type").and_then(|v| v.as_str()) {
                if ty == "Variable" {
                    if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
                        if name.starts_with("g_") { globals.insert(name.to_string()); }
                    }
                }
                if let Some(dst) = obj.get("dst").and_then(|v| v.as_str()) {
                    if dst.starts_with("g_") { globals.insert(dst.to_string()); }
                }
            }
            if let Some(vars) = obj.get("variables").and_then(|v| v.as_object()) {
                for name in vars.keys() {
                    if name.starts_with("g_") { globals.insert(name.clone()); }
                }
            }
            for v in obj.values() {
                self.collect_globals(v, globals);
            }
        } else if let Some(arr) = val.as_array() {
            for v in arr {
                self.collect_globals(v, globals);
            }
        }
    }

    fn collect_strings(&mut self, val: &Value) {
        if let Some(obj) = val.as_object() {
            if let Some(ty) = obj.get("_type").and_then(|v| v.as_str()) {
                if ty == "Constant" {
                    if let Some(s) = obj.get("value").and_then(|v| v.as_str()) {
                        // Heuristic: if it's not a pure number, it's likely a string
                        if s.parse::<i64>().is_err() && s != "true" && s != "false" && s != "null" && s != "None" {
                            if !self.string_constants.contains(&s.to_string()) {
                                self.string_constants.push(s.to_string());
                            }
                        }
                    }
                } else if ty == "LoadMember" || ty == "StoreMember" {
                    if let Some(member) = obj.get("member").and_then(|v| v.as_str()) {
                         self.push_string_constant(member);
                    }
                } else if ty == "Cast" {
                    if let Some(src) = obj.get("src") {
                        if let Some(src_ty) = Self::value_type_name(src) {
                            if let Some(dst_ty) = obj.get("ty").and_then(|v| v.as_str()) {
                                self.push_string_constant(&Self::cast_descriptor(src_ty, dst_ty));
                            }
                        }
                    }
                } else if ty == "StoreIndex" {
                    if let Some(src) = obj.get("src") {
                        let element_ty = obj.get("element_ty").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(desc) = Self::any_box_descriptor(src, element_ty) {
                            self.push_string_constant(&desc);
                        }
                    }
                }
            }
            for v in obj.values() {
                self.collect_strings(v);
            }
        } else if let Some(arr) = val.as_array() {
            for v in arr {
                self.collect_strings(v);
            }
        }
    }

    fn gen_func(&mut self, func: &Value) {
        let name = func
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        self.current_func = name.to_string();
        if func.get("is_extern").and_then(|v| v.as_bool()).unwrap_or(false) {
            return;
        }
        let wasm_id = self.wasm_func_id(name);
        if self.generated_ids.contains(&wasm_id) {
            return;
        }
        self.generated_ids.insert(wasm_id.clone());
        let params_len = func.get("params").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        self.signatures.insert(wasm_id.clone(), params_len);

        self.emit(&format!("    (func ${}\n", wasm_id));

        let mut locals = std::collections::HashSet::new();
        let mut params_set = std::collections::HashSet::new();
        if let Some(p_arr) = func.get("params").and_then(|v| v.as_array()) {
            for p in p_arr {
                if let Some(s) = p.as_str() {
                    params_set.insert(s.to_string());
                    let p_wasm_id = format!("{}{}", L_PREFIX, mangle(s));
                    log_internal(&format!("Emitting param: ${} for {}", p_wasm_id, s));
                    self.emit(&format!(" (param ${} i64)", p_wasm_id));
                }
            }
        }
        self.emit(" (result i64)\n");

        if let Some(vars) = func.get("variables").and_then(|v| v.as_object()) {
            for var_name in vars.keys() {
                if !var_name.starts_with("g_") && !params_set.contains(var_name) {
                    locals.insert(var_name.clone());
                }
            }
        }

        if let Some(blocks) = func.get("blocks").and_then(|v| v.as_array()) {
            for block in blocks {
                if let Some(insts) = block.get("instructions").and_then(|v| v.as_array()) {
                    for inst in insts {
                        // Collect from dst
                        if let Some(dst) = inst.get("dst").and_then(|v| v.as_str()) {
                            if !dst.is_empty() && !dst.starts_with("g_") && !params_set.contains(dst) {
                                locals.insert(dst.to_string());
                            }
                        }
                        // Broad-spectrum collection from all fields
                        if let Some(obj) = inst.as_object() {
                            for v in obj.values() {
                                self.collect_from_value(v, &params_set, &mut locals);
                            }
                        }
                    }
                }
            }
        }

        for local in locals {
            let l_wasm_id = self.wasm_local_id(&local);
            log_internal(&format!("Emitting local: ${} for {}", l_wasm_id, local));
            self.emit_line(&format!("(local ${} i64)", l_wasm_id));
        }
        self.emit_line("(local $__pc i32)");
        let body_start = self.buffer.len();
        if let Some(blocks) = func.get("blocks").and_then(|v| v.as_array()) {
            self.emit_line("(loop $__loop");
            for (i, block) in blocks.iter().enumerate() {
                self.emit_line("(local.get $__pc)");
                self.emit_line(&format!("i32.const {}", i));
                self.emit_line("i32.eq");
                self.emit_line("if");
                self.gen_block(block, i);
                self.emit_line("end");
            }
            self.emit_line("i64.const 0");
            self.emit_line("return");
            self.emit_line(")");
        }
        if name == "_stringify_for_print" {
            log_internal(&format!("WAT for {}:\n{}", name, &self.buffer[body_start..]));
        }
        self.emit_line("i64.const 0");
        self.emit_line(")");
    }

    fn gen_block(&mut self, block: &Value, i: usize) {
        if let Some(insts) = block.get("instructions").and_then(|v| v.as_array()) {
            for inst in insts {
                self.gen_inst(inst);
            }
        }
        // Fallthrough to next block by default
        self.emit_line(&format!("i32.const {}", i + 1));
        self.emit_line("local.set $__pc");
        self.emit_line("br $__loop");
    }

    fn gen_expr(&mut self, val: &Value) {
        if let Some(name) = val.as_str() {
            if let Ok(i) = name.parse::<i64>() {
                self.emit_line(&format!("i64.const {}", i));
            } else if name == "true" {
                self.emit_line("i64.const 1");
            } else if name == "false" || name == "null" || name == "None" {
                self.emit_line("i64.const 0");
            } else if name.starts_with("g_") {
                self.emit_line(&format!("global.get ${}", self.wasm_global_id(name)));
            } else {
                self.emit_line(&format!("local.get ${}", self.wasm_local_id(name)));
            }
            return;
        }
        if let Some(i) = val.as_i64() {
            self.emit_line(&format!("i64.const {}", i));
            return;
        }
        if let Some(u) = val.as_u64() {
            self.emit_line(&format!("i64.const {}", u));
            return;
        }
        if let Some(b) = val.as_bool() {
            self.emit_line(if b { "i64.const 1" } else { "i64.const 0" });
            return;
        }
        if val.is_null() {
            self.emit_line("i64.const 0");
            return;
        }

        let obj = match val.as_object() {
            Some(o) => o,
            None => { self.emit_line("i64.const 0"); return; }
        };

        let ty = obj.get("_type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "Call" => {
                let name = obj.get("callee").and_then(|v| v.as_str()).unwrap_or("");
                if name.is_empty() {
                    log_internal(&format!("ERROR: Call instruction missing callee: {:?}", obj));
                }
                let is_indirect = obj.get("is_indirect").and_then(|v| v.as_bool()).unwrap_or(false);
                let args = obj.get("args").and_then(|v| v.as_array());
                let nargs = args.map(|a| a.len()).unwrap_or(0);
                if let Some(args) = args {
                    for arg in args { self.gen_expr(arg); }
                }
                if is_indirect {
                    self.emit_line(&format!("call_indirect (type $t_func_{})", nargs));
                } else {
                    self.emit_line(&format!("call ${}", self.wasm_func_id(name)));
                }
            }
            "IndirectCall" => {
                let args = obj.get("args").and_then(|v| v.as_array());
                let nargs = args.map(|a| a.len()).unwrap_or(0);
                if let Some(args) = args {
                    for arg in args { self.gen_expr(arg); }
                }
                let callee = obj.get("callee").unwrap_or(&Value::Null);
                self.gen_expr(callee);
                self.emit_line("i32.wrap_i64");
                self.emit_line(&format!("call_indirect (type $t_func_{})", nargs));
            }
            "BinaryOp" => {
                let left = obj.get("left").unwrap_or(&Value::Null);
                let right = obj.get("right").unwrap_or(&Value::Null);
                let op = obj.get("op").and_then(|v| v.as_str()).unwrap_or("");
                let op_width = obj.get("op_width").and_then(|v| v.as_str()).unwrap_or("");
                self.gen_expr(left);
                self.gen_expr(right);
                if op == "Plus" && op_width == "String" {
                    self.emit_line(&format!("call ${}", self.wasm_func_id("rt_str_concat_v2")));
                    return;
                }
                let wasm_op = self.config.get("ops").and_then(|ops| ops.get(op)).and_then(|v| v.as_str()).map(|s| s.to_string());
                if let Some(op_code) = wasm_op {
                    self.emit_line(&op_code);
                    if op_code.contains(".eq") || op_code.contains(".ne") || op_code.contains(".lt") || op_code.contains(".gt") || op_code.contains(".le") || op_code.contains(".ge") {
                        self.emit_line("i64.extend_i32_u");
                    }
                } else {
                    let op_name = if op == "EqualEqual" { "rt_op_equalequal".to_string() }
                                    else { format!("rt_op_{}", op.to_lowercase()) };
                    self.emit_line(&format!("call ${}", self.wasm_func_id(&op_name)));
                }
            }
            "UnaryOp" => {
                let obj_val = obj.get("value").unwrap_or(&Value::Null);
                let op = obj.get("op").and_then(|v| v.as_str()).unwrap_or("");
                self.gen_expr(obj_val);
                if op == "Not" {
                    self.emit_line(&format!("call ${}", self.wasm_func_id("rt_not")));
                } else if op == "Minus" {
                    self.emit_line("i64.const -1");
                    self.emit_line("i64.mul");
                }
            }
            "LoadMember" => {
                let obj_val = obj.get("obj").unwrap_or(&Value::Null);
                let member = obj.get("member").and_then(|v| v.as_str()).unwrap_or("");
                self.gen_expr(obj_val);
                let offset = self.string_offset(member);
                self.emit_line(&format!("i64.const {}", offset));
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_box_string")));
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_load_member")));
            }
            "StoreMember" => {
                let obj_val = obj.get("obj").unwrap_or(&Value::Null);
                let member = obj.get("member").and_then(|v| v.as_str()).unwrap_or("");
                let val = obj.get("src").unwrap_or(&Value::Null); // Align with Rust: src
                self.gen_expr(obj_val);
                let offset = self.string_offset(member);
                self.emit_line(&format!("i64.const {}", offset));
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_box_string")));
                self.gen_expr(val);
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_store_member")));
            }
            "LoadIndex" => {
                let obj_val = obj.get("obj").unwrap_or(&Value::Null);
                let index = obj.get("index").unwrap_or(&Value::Null);
                self.gen_expr(obj_val);
                self.gen_expr(index);
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_load_index")));
            }
            "StoreIndex" => {
                let obj_val = obj.get("obj").unwrap_or(&Value::Null);
                let index = obj.get("index").unwrap_or(&Value::Null);
                let val = obj.get("src").unwrap_or(&Value::Null); // Align with Rust: src
                let element_ty = obj.get("element_ty").and_then(|v| v.as_str()).unwrap_or("");
                self.gen_expr(obj_val);
                self.gen_expr(index);
                self.gen_expr(val);
                if let Some(desc) = Self::any_box_descriptor(val, element_ty) {
                    let offset = self.string_offset(&desc);
                    self.emit_line(&format!("i64.const {}", offset));
                    self.emit_line(&format!("call ${}", self.wasm_func_id("rt_box_string")));
                    self.emit_line(&format!("call ${}", self.wasm_func_id("rt_cast")));
                }
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_store_index")));
            }
            "Cast" => {
                let val = obj.get("src").unwrap_or(&Value::Null); // Align with Rust: src
                let src_ty = Self::value_type_name(val);
                let dst_ty = obj.get("ty").and_then(|v| v.as_str());
                self.gen_expr(val);
                if let (Some(src_ty), Some(dst_ty)) = (src_ty, dst_ty) {
                    let desc = Self::cast_descriptor(src_ty, dst_ty);
                    let offset = self.string_offset(&desc);
                    self.emit_line(&format!("i64.const {}", offset));
                    self.emit_line(&format!("call ${}", self.wasm_func_id("rt_box_string")));
                } else {
                    self.emit_line("i64.const 0");
                }
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_cast")));
            }
            "Class" => {
                let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let offset = self.string_offset(name);
                self.emit_line(&format!("i64.const {}", offset));
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_class_new")));
            }
            "Constant" => {
                if let Some(value) = obj.get("value") {
                    let ty_name = obj.get("ty").and_then(|v| v.as_str()).unwrap_or("");
                    if matches!(ty_name, "Float" | "Float32" | "Float64") {
                        if let Some(s) = value.as_str() {
                            if let Ok(f) = s.parse::<f64>() {
                                self.emit_line(&format!("i64.const {}", f.to_bits() as i64));
                                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_box_number_internal")));
                            } else {
                                self.emit_line("i64.const 0");
                            }
                        } else {
                            self.emit_line("i64.const 0");
                        }
                    } else if let Some(i) = value.as_i64() {
                        self.emit_line(&format!("i64.const {}", i));
                    } else if let Some(u) = value.as_u64() {
                        self.emit_line(&format!("i64.const {}", u));
                    } else if let Some(b) = value.as_bool() {
                        self.emit_line(if b { "i64.const 1" } else { "i64.const 0" });
                    } else if value.is_null() {
                        self.emit_line("i64.const 0");
                    } else if let Some(s) = value.as_str() {
                        if let Ok(i) = s.parse::<i64>() {
                            self.emit_line(&format!("i64.const {}", i));
                        } else if s == "true" {
                            self.emit_line("i64.const 1");
                        } else if s == "false" || s == "null" || s == "None" {
                            self.emit_line("i64.const 0");
                        } else {
                            let offset = self.string_offset(s);
                            self.emit_line(&format!("i64.const {}", offset));
                            self.emit_line(&format!("call ${}", self.wasm_func_id("rt_box_string")));
                        }
                    } else {
                        self.emit_line("i64.const 0");
                    }
                } else {
                    self.emit_line("i64.const 0");
                }
            }
            "Variable" => {
                let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let ty_name = obj.get("ty").and_then(|v| v.as_str()).unwrap_or("");
                if ty_name.starts_with("Function") {
                    if let Some(table_index) = self.function_table_index(name) {
                        self.emit_line(&format!("i64.const {}", table_index));
                        return;
                    }
                }
                if name.starts_with("g_") {
                    self.emit_line(&format!("global.get ${}", self.wasm_global_id(name)));
                } else {
                    self.emit_line(&format!("local.get ${}", self.wasm_local_id(name)));
                }
            }
            "Move" => {
                let value = obj.get("src").unwrap_or(&Value::Null); // Align with Rust: src
                self.gen_expr(value);
            }
            _ => {
                self.emit_line("i64.const 0");
            }
        }
    }

    fn gen_inst(&mut self, val: &Value) {
        let obj = match val.as_object() {
            Some(o) => o,
            None => { self.gen_expr(val); self.emit_line("drop"); return; }
        };

        let ty = obj.get("_type").and_then(|v| v.as_str()).unwrap_or("");
        log_internal(&format!("Processing instruction: {}", ty));
        let dst = obj.get("dst").and_then(|v| v.as_str()).unwrap_or("");

        match ty {
            "Return" => {
                if let Some(v) = obj.get("value") {
                    if v != &Value::Null { self.gen_expr(v); }
                    else { self.emit_line("i64.const 0"); }
                } else {
                    self.emit_line("i64.const 0");
                }
                self.emit_line("return");
                return;
            }
            "Print" => {
                let args = obj.get("args").and_then(|v| v.as_array());
                if let Some(args) = args {
                    for arg in args {
                        self.gen_expr(arg);
                        self.emit_line(&format!("call ${}", self.wasm_func_id("rt_print")));
                    }
                }
                return;
            }
            "Len" => {
                let obj_val = obj.get("obj").unwrap_or(&Value::Null);
                self.gen_expr(obj_val);
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_len")));
                // Result is already on stack, generic handler will set dst
            }
            "Branch" => {
                let cond = obj.get("condition").unwrap_or(&Value::Null);
                self.gen_expr(cond);
                self.emit_line("i32.wrap_i64");
                let t = obj.get("true_target").map(|v| v.to_string()).unwrap_or("0".to_string());
                let f = obj.get("false_target").map(|v| v.to_string()).unwrap_or("0".to_string());
                self.emit_line(&format!("if (result i32) i32.const {} else i32.const {} end", t, f));
                self.emit_line("local.set $__pc");
                self.emit_line("br $__loop");
                return;
            }
            "Jump" => {
                let t = obj.get("target").map(|v| v.to_string()).unwrap_or("0".to_string());
                self.emit_line(&format!("i32.const {}", t));
                self.emit_line("local.set $__pc");
                self.emit_line("br $__loop");
                return;
            }
            "Throw" => {
                let val = obj.get("value").unwrap_or(&Value::Null);
                self.gen_expr(val);
                self.emit_line(&format!("call ${}", self.wasm_func_id("rt_throw")));
                self.emit_line("i64.const 0"); // Balance stack for Wasm
                self.emit_line("return");
                return;
            }
            _ => {}
        }

        // Generic instruction: evaluation and destination handling
        self.gen_expr(val);
        if !dst.is_empty() {
            if dst.starts_with("g_") {
                self.emit_line(&format!("global.set ${}", self.wasm_global_id(dst)));
            } else {
                self.emit_line(&format!("local.set ${}", self.wasm_local_id(dst)));
            }
        } else {
            self.emit_line("drop");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_generic_assembler_logic() {
        let config = json!({
            "ops": {
                "Plus": "i64.add"
            }
        });
        let mut gen = WasmCodeGen::new(config);

        let mir = json!([{
            "name": "tejx_main",
            "is_extern": false,
            "params": [],
            "variables": { "x": "i64" },
            "blocks": [{
                "name": "entry",
                "instructions": [
                    {
                        "_type": "Move",
                        "dst": "x",
                        "src": { "_type": "Constant", "value": "42" }
                    },
                    {
                        "_type": "BinaryOp",
                        "op": "Plus",
                        "left": { "_type": "Variable", "name": "x" },
                        "right": { "_type": "Constant", "value": "1" },
                        "dst": "x"
                    },
                    {
                        "_type": "Return",
                        "value": { "_type": "Variable", "name": "x" }
                    }
                ]
            }]
        }]);

        let wat = gen.generate_wat_generic(mir);

        assert!(wat.contains("(module"), "Should contain module");
        assert!(wat.contains("(func $f_m_tejx_main"), "Should contain function");
        assert!(wat.contains("i64.const 42"), "Should contain constant 42");
        assert!(wat.contains("i64.add"), "Should have mapped Plus to i64.add via config");
        assert!(wat.contains("local.set $l_m_x"), "Should have namespaced local variable x");
    }

    #[test]
    fn test_complex_features() {
        let config = json!({});
        let mut gen = WasmCodeGen::new(config);

        let mir = json!([{
            "name": "complex_test",
            "is_extern": false,
            "params": ["p1"],
            "variables": { "g_counter": "i64", "obj": "object" },
            "blocks": [
                {
                    "name": "b0",
                    "instructions": [
                        {
                            "_type": "Move",
                            "dst": "g_counter",
                            "src": { "_type": "Variable", "name": "p1" }
                        },
                        {
                            "_type": "LoadMember",
                            "obj": { "_type": "Variable", "name": "obj" },
                            "member": "data",
                            "dst": "p1"
                        },
                        {
                            "_type": "Branch",
                            "condition": { "_type": "Variable", "name": "p1" },
                            "true_target": 1,
                            "false_target": 0
                        }
                    ]
                }
            ]
        }]);

        let wat = gen.generate_wat_generic(mir);
        
        assert!(wat.contains("(global $g_m_g_counter (mut i64) (i64.const 0))"), "Should define global");
        assert!(wat.contains("global.set $g_m_g_counter"), "Should set global");
        assert!(wat.contains("call $f_m_rt_load_member"), "Should call runtime for member access");
        assert!(wat.contains("if (result i32) i32.const 1 else i32.const 0 end"), "Should handle branching");
    }
}
