/// MIR → LLVM IR Code Generator, mirroring C++ MIRCodeGen.cpp
/// Generates textual LLVM IR from MIR basic blocks.
use crate::mir::*;
use crate::token::TokenType;
use crate::types::TejxType;
use std::collections::{HashMap, HashSet};

pub struct CodeGen {
    buffer: String,
    global_buffer: String,
    value_map: HashMap<String, String>, // MIR var name → LLVM alloca ptr name
    temp_counter: usize,
    label_counter: usize,
    declared_functions: HashSet<String>,
    function_param_counts: HashMap<String, usize>,
    declared_globals: HashSet<String>,
    current_function_params: HashSet<String>,
    local_vars: HashSet<String>,

    captured_vars: HashSet<String>,
    current_env: Option<String>,
    alloca_buffer: String,
    stack_arrays: HashSet<String>,
    heap_array_ptrs: HashMap<String, (String, i64)>, // var_name -> (data_ptr_alloca, elem_size)
    pub unsafe_arrays: bool,
    float_ssa_vars: HashMap<String, String>, // var_name -> LLVM double SSA variable
}

impl CodeGen {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            global_buffer: String::new(),
            value_map: HashMap::new(),
            temp_counter: 0,
            label_counter: 0,
            declared_functions: HashSet::new(),
            function_param_counts: HashMap::new(),
            declared_globals: HashSet::new(),
            current_function_params: HashSet::new(),
            local_vars: HashSet::new(),

            captured_vars: HashSet::new(),
            current_env: None,
            alloca_buffer: String::new(),
            stack_arrays: HashSet::new(),
            heap_array_ptrs: HashMap::new(),
            unsafe_arrays: false,
            float_ssa_vars: HashMap::new(),
        }
    }

    fn emit(&mut self, code: &str) {
        self.buffer.push_str(code);
    }

    fn get_captured_key(&self, name: &str) -> Option<String> {
        if self.captured_vars.contains(name) {
            return Some(name.to_string());
        }
        // Handle MIR mangling suffixes like _123
        for cap in &self.captured_vars {
            if name.starts_with(cap)
                && (name.len() == cap.len() || name[cap.len()..].starts_with('_'))
            {
                return Some(cap.clone());
            }
        }
        None
    }

    fn is_captured(&self, name: &str) -> bool {
        self.get_captured_key(name).is_some()
    }

    fn emit_line(&mut self, code: &str) {
        self.buffer.push_str("  ");
        self.buffer.push_str(code);
        self.buffer.push('\n');
    }

    fn does_escape(&self, func: &MIRFunction, var_name: &str) -> bool {
        // Simple escape analysis: If the reference escapes via return, pass as argument,
        // or is stored inside an object/array, it escapes the stack frame.
        // It recursively checks variables if it is moved to another variable.
        let mut check_vars = vec![var_name.to_string()];
        let mut i = 0;

        while i < check_vars.len() {
            let current_var = check_vars[i].clone();
            for block in &func.blocks {
                for instr in &block.instructions {
                    match instr {
                        MIRInstruction::Call { callee, args, .. } => {
                            // Whitelist: array-safe runtime calls where args[0] is 'this'
                            // and the pointer is only borrowed, not escaped.
                            // NOTE: Array_push/pop/shift/unshift/splice are NOT whitelisted
                            // because they resize the array, which breaks stack allocation.
                            let is_array_safe = matches!(
                                callee.as_str(),
                                "f_Array_constructor"
                                    | "f_Array_fill"
                                    | "Array_fill"
                                    | "Array_sort"
                                    | "Array_reverse"
                                    | "Array_indexOf"
                                    | "rt_array_get_fast"
                                    | "rt_array_set_fast"
                                    | "rt_array_length"
                                    | "rt_free_array"
                                    | "rt_array_get_data_ptr"
                                    | "rt_array_get_data_ptr_nocache"
                            );

                            for (i, arg) in args.iter().enumerate() {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name == &current_var {
                                        // If this is args[0] (the 'this'/array pointer)
                                        // and the callee is a known array-safe function,
                                        // it doesn't escape — it's just borrowed.
                                        if i == 0 && is_array_safe {
                                            continue;
                                        }
                                        return true;
                                    }
                                }
                            }
                        }
                        MIRInstruction::IndirectCall { args, .. } => {
                            for arg in args {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name == &current_var {
                                        return true;
                                    }
                                }
                            }
                        }
                        MIRInstruction::Return {
                            value: Some(val), ..
                        } => {
                            if let MIRValue::Variable { name, .. } = val {
                                if name == &current_var {
                                    return true;
                                }
                            }
                        }
                        MIRInstruction::StoreIndex { src, .. }
                        | MIRInstruction::StoreMember { src, .. } => {
                            if let MIRValue::Variable { name, .. } = src {
                                if name == &current_var {
                                    return true;
                                }
                            }
                        }
                        MIRInstruction::ArrayLiteral { elements, .. } => {
                            for element in elements {
                                if let MIRValue::Variable { name, .. } = element {
                                    if name == &current_var {
                                        return true;
                                    }
                                }
                            }
                        }
                        MIRInstruction::ObjectLiteral { entries, .. } => {
                            for (_, element) in entries {
                                if let MIRValue::Variable { name, .. } = element {
                                    if name == &current_var {
                                        return true;
                                    }
                                }
                            }
                        }
                        MIRInstruction::Move { dst, src, .. } => {
                            if let MIRValue::Variable { name, .. } = src {
                                if name == &current_var {
                                    if !check_vars.contains(dst) {
                                        check_vars.push(dst.clone());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            i += 1;
        }
        false
    }

    fn resolve_float_value(&mut self, val: &MIRValue) -> String {
        if let MIRValue::Variable { name, ty } = val {
            if ty.is_float() {
                if let Some(ssa_var) = self.float_ssa_vars.get(name) {
                    return ssa_var.clone(); // Found direct double representation
                }
            }
        }

        // Fallback: resolve normal (i64) and convert based on type
        let normal_val = self.resolve_value(val);
        let ty = val.get_type();

        self.temp_counter += 1;
        let float_val = format!("%float_conv_{}", self.temp_counter);

        if ty.is_float() {
            self.emit_line(&format!(
                "{} = bitcast i64 {} to double",
                float_val, normal_val
            ));
            return float_val;
        } else if matches!(ty, TejxType::Any) {
            if !self.declared_functions.contains("rt_to_number_v2") {
                self.global_buffer
                    .push_str("declare i64 @rt_to_number_v2(i64) readonly\n");
                self.declared_functions
                    .insert("rt_to_number_v2".to_string());
            }
            self.temp_counter += 1;
            let bits_tmp = format!("%any_bits_{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call i64 @rt_to_number_v2(i64 {})",
                bits_tmp, normal_val
            ));
            self.emit_line(&format!(
                "{} = bitcast i64 {} to double",
                float_val, bits_tmp
            ));
            return float_val;
        } else {
            self.emit_line(&format!(
                "{} = sitofp i64 {} to double",
                float_val, normal_val
            ));
            return float_val;
        }
    }

    fn resolve_value(&mut self, val: &MIRValue) -> String {
        match val {
            MIRValue::Constant { value, ty } => {
                // Handle "new Class" hack
                if value.starts_with("@") {
                    let name = &value[1..];
                    let count = self.function_param_counts.get(name).cloned().unwrap_or(1); // Default to 1 for workers
                    let args = vec!["i64"; count].join(", ");
                    return format!("ptrtoint (i64 ({})* @{} to i64)", args, name);
                }
                if value.starts_with("lambda_") {
                    let count = self.function_param_counts.get(value).cloned().unwrap_or(1);
                    let args = vec!["i64"; count].join(", ");
                    let fn_ptr = format!("ptrtoint (i64 ({})* @{} to i64)", args, value);

                    let has_func_captures = !self.captured_vars.is_empty();

                    if has_func_captures || self.current_env.is_some() {
                        // Box as closure Map { "ptr": fn_ptr, "env": env }
                        if !self.declared_functions.contains("rt_map_new") {
                            self.global_buffer
                                .push_str("declare i64 @rt_map_new() nounwind\n");
                            self.declared_functions.insert("rt_map_new".to_string());
                        }
                        if !self.declared_functions.contains("rt_Map_set") {
                            self.global_buffer
                                .push_str("declare i64 @rt_Map_set(i64, i64, i64) nounwind\n");
                            self.declared_functions.insert("rt_Map_set".to_string());
                        }
                        if !self.declared_functions.contains("rt_box_string") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_string(i64)\n");
                            self.declared_functions.insert("rt_box_string".to_string());
                        }

                        self.temp_counter += 1;
                        let closure_id = format!("%closure{}", self.temp_counter);
                        self.emit_line(&format!("{} = call i64 @rt_map_new()", closure_id));

                        // Set "ptr"
                        let ptr_key = "@str_key_ptr";
                        if !self.declared_globals.contains(ptr_key) {
                            self.global_buffer.push_str("@str_key_ptr = private unnamed_addr constant [4 x i8] c\"ptr\\00\"\n");
                            self.declared_globals.insert(ptr_key.to_string());
                        }
                        self.temp_counter += 1;
                        let ptr_key_id = format!("%ptr_key{}", self.temp_counter);
                        self.emit_line(&format!("{} = call i64 @rt_box_string(i64 ptrtoint ([4 x i8]* @str_key_ptr to i64))", ptr_key_id));
                        self.emit_line(&format!(
                            "call i64 @rt_Map_set(i64 {}, i64 {}, i64 {})",
                            closure_id, ptr_key_id, fn_ptr
                        ));

                        // Set "env"
                        let env_to_pass = if let Some(env) = self.current_env.clone() {
                            env
                        } else {
                            // Create a fresh empty environment if the parent doesn't have one
                            self.temp_counter += 1;
                            let fresh_env = format!("%fresh_env{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_map_new()", fresh_env));
                            fresh_env
                        };

                        let env_key = "@str_key_env";
                        if !self.declared_globals.contains(env_key) {
                            self.global_buffer.push_str("@str_key_env = private unnamed_addr constant [4 x i8] c\"env\\00\"\n");
                            self.declared_globals.insert(env_key.to_string());
                        }
                        self.temp_counter += 1;
                        let env_key_id = format!("%env_key{}", self.temp_counter);
                        self.emit_line(&format!("{} = call i64 @rt_box_string(i64 ptrtoint ([4 x i8]* @str_key_env to i64))", env_key_id));
                        self.emit_line(&format!(
                            "call i64 @rt_Map_set(i64 {}, i64 {}, i64 {})",
                            closure_id, env_key_id, env_to_pass
                        ));

                        return closure_id;
                    }
                    return fn_ptr;
                }
                if value.starts_with("new ") {
                    return "0".to_string();
                }

                let is_integer_type = ty.is_numeric() && !ty.is_float();
                let is_float_type = ty.is_float();
                let is_bool_type = matches!(ty, TejxType::Bool);
                let is_any_type = matches!(ty, TejxType::Any);

                if is_bool_type {
                    if value == "true" || value == "1" {
                        return "1".to_string();
                    }
                    return "0".to_string();
                }

                if is_integer_type {
                    if let Ok(i) = value.parse::<i64>() {
                        return format!("{}", i);
                    }
                }

                if is_float_type || is_any_type && value.parse::<f64>().is_ok() {
                    if let Ok(d) = value.parse::<f64>() {
                        // Variables of type Any/Number ALWAYS store bitcasted doubles
                        return format!("{}", d.to_bits());
                    }
                    return "0".to_string();
                }

                // String literal → global constant
                self.label_counter += 1;
                let str_lbl = format!("@.str{}", self.label_counter);

                let raw_content = value.clone();
                let content = if raw_content.len() >= 2
                    && raw_content.starts_with('"')
                    && raw_content.ends_with('"')
                {
                    &raw_content[1..raw_content.len() - 1]
                } else {
                    &raw_content
                };

                let _len = content.len() + 1;

                let mut escaped = String::new();
                for b in content.as_bytes() {
                    match *b {
                        b'\\' => escaped.push_str("\\\\"),
                        b'\"' => escaped.push_str("\\22"),
                        b'\n' => escaped.push_str("\\0A"),
                        b'\r' => escaped.push_str("\\0D"),
                        b'\t' => escaped.push_str("\\09"),
                        32..=126 => escaped.push(*b as char), // Printable ASCII
                        _ => escaped.push_str(&format!("\\{:02X}", b)),
                    }
                }

                let byte_len = content.as_bytes().len() + 1;

                self.global_buffer.push_str(&format!(
                    "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"\n",
                    str_lbl, byte_len, escaped
                ));

                // Return ptrtoint cast to i64
                format!("ptrtoint ([{} x i8]* {} to i64)", byte_len, str_lbl)
            }
            MIRValue::Variable { name, ty } => {
                if name.starts_with("g_") {
                    self.temp_counter += 1;
                    let tmp = format!("%t{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @{}", tmp, name));
                    return tmp;
                }
                if name == "__env" {
                    if let Some(env) = &self.current_env {
                        return env.clone();
                    }
                    return "0".to_string();
                }

                if name.starts_with("%") || name.starts_with("@") {
                    return name.to_string();
                }

                if let Some(cap_key) = self.get_captured_key(name) {
                    if let Some(env) = self.current_env.clone() {
                        if !self.declared_functions.contains("rt_Map_get") {
                            self.global_buffer
                                .push_str("declare i64 @rt_Map_get_ref(i64, i64)\n");
                            self.declared_functions.insert("rt_Map_get".to_string());
                        }
                        if !self.declared_functions.contains("rt_box_string") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_string(i64)\n");
                            self.declared_functions.insert("rt_box_string".to_string());
                        }

                        // Get/Create key string - use base captured name for consistent keys
                        let key_global = format!("@str_key_{}", cap_key.replace("$", "_"));
                        if !self.declared_globals.contains(&key_global) {
                            self.global_buffer.push_str(&format!(
                                "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"\n",
                                key_global,
                                cap_key.len() + 1,
                                cap_key
                            ));
                            self.declared_globals.insert(key_global.clone());
                        }

                        self.temp_counter += 1;
                        let key_id = format!("%key_id{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_string(i64 ptrtoint ([{} x i8]* {} to i64))",
                            key_id,
                            cap_key.len() + 1,
                            key_global
                        ));

                        self.temp_counter += 1;
                        let val_reg = format!("%cap_val{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_Map_get_ref(i64 {}, i64 {})",
                            val_reg, env, key_id
                        ));

                        // Unbox if necessary - retrieved values from Map are TaggedValues
                        if ty.is_numeric() || matches!(ty, TejxType::Bool) {
                            if !self.declared_functions.contains("rt_to_number") {
                                self.global_buffer
                                    .push_str("declare double @rt_to_number(i64)\n");
                                self.declared_functions.insert("rt_to_number".to_string());
                            }
                            self.temp_counter += 1;
                            let d_tmp = format!("%d_tmp{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call double @rt_to_number(i64 {})",
                                d_tmp, val_reg
                            ));

                            if ty.is_float() {
                                self.temp_counter += 1;
                                let i_tmp = format!("%bit_tmp{}", self.temp_counter);
                                self.emit_line(&format!(
                                    "{} = bitcast double {} to i64",
                                    i_tmp, d_tmp
                                ));
                                return i_tmp;
                            } else {
                                // int or bool
                                self.temp_counter += 1;
                                let i_tmp = format!("%i_tmp{}", self.temp_counter);
                                self.emit_line(&format!(
                                    "{} = fptosi double {} to i64",
                                    i_tmp, d_tmp
                                ));
                                return i_tmp;
                            }
                        }

                        return val_reg;
                    }
                }

                if let Some(reg_ref) = self.value_map.get(name) {
                    let reg = reg_ref.clone();
                    // Intercept and load from alloca
                    self.temp_counter += 1;
                    let val_reg = format!("%val_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", val_reg, reg));
                    return val_reg;
                }

                // Check for function parameter (if not mapped to alloca yet? - should be in value_map)
                // Fallback for globals
                if self.declared_functions.contains(name) || name == "tejx_main" {
                    // Function pointer logic (same as before)
                    let count = self.function_param_counts.get(name).cloned().unwrap_or(0);
                    let args_sig = vec!["i64"; count].join(", ");
                    return format!("ptrtoint (i64 ({})* @{} to i64)", args_sig, name);
                }

                // Should check globals here properly
                if name.starts_with("g_") || self.declared_globals.contains(name) {
                    self.temp_counter += 1;
                    let val_reg = format!("%gval_{}", self.temp_counter);
                    let g_name = if name.starts_with("g_") {
                        name.to_string()
                    } else {
                        format!("g_{}", name)
                    };
                    if !self.declared_globals.contains(&g_name) {
                        self.global_buffer
                            .push_str(&format!("@{} = global i64 0\n", g_name));
                        self.declared_globals.insert(g_name.clone());
                    }
                    self.emit_line(&format!("{} = load i64, i64* @{}", val_reg, g_name));
                    return val_reg;
                }

                // Treat as valid global reference if we can't find it locally?
                // Or partial match?

                // Existing logic for function names used as values
                let mut target = name.clone();
                if name.starts_with("f_") {
                    let real_name = &name[2..]; // strip f_
                    if self.declared_functions.contains(real_name) {
                        target = real_name.to_string();
                    }
                }

                if self.declared_functions.contains(&target) {
                    let count = self
                        .function_param_counts
                        .get(&target)
                        .cloned()
                        .unwrap_or(0);
                    let args_sig = vec!["i64"; count].join(", ");
                    return format!("ptrtoint (i64 ({})* @{} to i64)", args_sig, target);
                }

                // Last resort: 0
                "0".to_string()
            }
        }
    }

    fn resolve_ptr(&mut self, name: &str) -> String {
        if name.starts_with("%") {
            return name.to_string();
        }

        if name.starts_with("g_") {
            if !self.declared_globals.contains(name) {
                self.global_buffer
                    .push_str(&format!("@{} = global i64 0\n", name));
                self.declared_globals.insert(name.to_string());
            }
            return format!("@{}", name);
        }

        // If it's a global variable (unmangled and not in this function's locals or params)
        if !name.contains("$")
            && !self.local_vars.contains(name)
            && !self.current_function_params.contains(name)
        {
            let g_name = format!("g_{}", name);
            if !self.declared_globals.contains(&g_name) {
                self.global_buffer
                    .push_str(&format!("@{} = global i64 0\n", g_name));
                self.declared_globals.insert(g_name.clone());
            }
            return format!("@{}", g_name);
        }

        self.value_map
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("%{}_ptr", name))
    }
}

/// Second pass: fix Jump and Branch instructions to use block names
impl CodeGen {
    pub fn generate_with_blocks(
        &mut self,
        functions: &[MIRFunction],
        captured_vars: HashSet<String>,
    ) -> String {
        self.captured_vars = captured_vars;
        self.buffer.clear();
        self.global_buffer.clear();
        self.declared_functions.clear();
        self.declared_globals.clear();

        // Register defined functions and their param counts
        let mut has_tejx_main = false;
        for f in functions {
            self.declared_functions.insert(f.name.clone());
            self.function_param_counts
                .insert(f.name.clone(), f.params.len());
            if f.name == "tejx_main" {
                has_tejx_main = true;
            }
        }

        // Collect and declare global variables
        let mut globals = HashSet::new();
        for func in functions {
            for bb in &func.blocks {
                for inst in &bb.instructions {
                    match inst {
                        MIRInstruction::Move { dst, src, .. } => {
                            if dst.starts_with("g_") {
                                globals.insert(dst.clone());
                            }
                            if let MIRValue::Variable { name, .. } = src {
                                if name.starts_with("g_") {
                                    globals.insert(name.clone());
                                }
                            }
                        }
                        MIRInstruction::BinaryOp {
                            dst, left, right, ..
                        } => {
                            if dst.starts_with("g_") {
                                globals.insert(dst.clone());
                            }
                            if let MIRValue::Variable { name, .. } = left {
                                if name.starts_with("g_") {
                                    globals.insert(name.clone());
                                }
                            }
                            if let MIRValue::Variable { name, .. } = right {
                                if name.starts_with("g_") {
                                    globals.insert(name.clone());
                                }
                            }
                        }
                        MIRInstruction::Call { dst, args, .. } => {
                            if dst.starts_with("g_") {
                                globals.insert(dst.clone());
                            }
                            for arg in args {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name.starts_with("g_") {
                                        globals.insert(name.clone());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for g in globals {
            if !self.declared_globals.contains(&g) {
                self.global_buffer
                    .push_str(&format!("@{} = global i64 0\n", g));
                self.declared_globals.insert(g);
            }
        }

        self.global_buffer
            .push_str("@.fmt_d = private unnamed_addr constant [5 x i8] c\"%lld\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_f = private unnamed_addr constant [3 x i8] c\"%f\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_s = private unnamed_addr constant [3 x i8] c\"%s\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_nl = private unnamed_addr constant [2 x i8] c\"\\0A\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_sp = private unnamed_addr constant [2 x i8] c\" \\00\"\n");

        for func in functions {
            self.gen_function_v2(func);
        }

        // Exception handling runtime functions
        self.global_buffer
            .push_str("declare i32 @_setjmp(i8*) returns_twice\n");
        self.global_buffer
            .push_str("declare void @tejx_push_handler(i8*)\n");
        self.global_buffer
            .push_str("declare void @tejx_pop_handler()\n");
        if !self.declared_functions.contains("tejx_throw") {
            self.global_buffer
                .push_str("declare void @tejx_throw(i64)\n");
        }
        if !self.declared_functions.contains("tejx_get_exception") {
            self.global_buffer
                .push_str("declare i64 @tejx_get_exception()\n");
        }
        if !self.declared_functions.contains("rt_box_string") {
            self.global_buffer
                .push_str("declare i64 @rt_box_string(i64)\n");
        }

        // Generate main wrapper if tejx_main exists
        if has_tejx_main {
            self.buffer.push_str("\n");
            self.buffer
                .push_str("declare i32 @tejx_runtime_main(i32, i8**)\n");
            self.buffer
                .push_str("define i32 @main(i32 %argc, i8** %argv) {\n");
            self.buffer.push_str("entry:\n");
            self.buffer
                .push_str("  %call = call i32 @tejx_runtime_main(i32 %argc, i8** %argv)\n");
            self.buffer.push_str("  ret i32 %call\n");
            self.buffer.push_str("}\n");
        }

        format!("{}{}", self.global_buffer, self.buffer)
    }

    fn gen_function_v2(&mut self, func: &MIRFunction) {
        self.value_map.clear();
        self.stack_arrays.clear();
        self.heap_array_ptrs.clear();
        self.float_ssa_vars.clear();
        self.temp_counter = 0;
        self.current_function_params.clear();
        self.local_vars.clear();
        self.current_env = None;

        for p in &func.params {
            self.current_function_params.insert(p.clone());
        }

        // Function signature with parameters
        let params_str = if func.params.is_empty() {
            String::new()
        } else {
            func.params
                .iter()
                .map(|p| format!("i64 %{}", p))
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.emit(&format!("define i64 @{}({}) {{\n", func.name, params_str));

        // Entry: allocas for all variables used in the function
        self.emit("entry:\n");

        // Scan for all local variables
        for bb in &func.blocks {
            for inst in &bb.instructions {
                let dest_var = match inst {
                    MIRInstruction::Move { dst, .. }
                    | MIRInstruction::BinaryOp { dst, .. }
                    | MIRInstruction::Call { dst, .. }
                    | MIRInstruction::IndirectCall { dst, .. }
                    | MIRInstruction::LoadMember { dst, .. }
                    | MIRInstruction::LoadIndex { dst, .. }
                    | MIRInstruction::ObjectLiteral { dst, .. }
                    | MIRInstruction::ArrayLiteral { dst, .. }
                    | MIRInstruction::Cast { dst, .. } => Some(dst.clone()),
                    _ => None,
                };
                if let Some(name) = dest_var {
                    if !name.starts_with("g_") && !self.current_function_params.contains(&name) {
                        self.local_vars.insert(name);
                    }
                }
            }
        }

        // Create environment if needed
        let has_captures = self.local_vars.iter().any(|v| self.is_captured(v))
            || func.params.iter().any(|p| self.is_captured(p));

        if func.name.starts_with("lambda_") {
            if !func.params.is_empty() {
                self.current_env = Some(format!("%{}", func.params[0]));
            }
        } else if has_captures {
            if !self.declared_functions.contains("rt_map_new") {
                self.global_buffer
                    .push_str("declare i64 @rt_map_new() nounwind\n");
                self.declared_functions.insert("rt_map_new".to_string());
            }
            self.temp_counter += 1;
            let env_reg = format!("%env_id{}", self.temp_counter);
            self.emit_line(&format!("{} = call i64 @rt_map_new()", env_reg));
            self.current_env = Some(env_reg);
        }

        // Allocas for parameters first
        for p in &func.params {
            if self.is_captured(p) {
                continue;
            } // Skip alloca for captured params
            if !self.value_map.contains_key(p) {
                let reg_name = format!("%{}_ptr", p);
                self.emit_line(&format!("{} = alloca i64", reg_name));
                self.value_map.insert(p.clone(), reg_name.clone());

                // CRITICAL: Store the incoming argument into the alloca
                self.emit_line(&format!("store i64 %{}, i64* {}", p, reg_name));
            }
        }

        // Allocas for all local variables
        let locals: Vec<String> = self.local_vars.iter().cloned().collect();
        for name in &locals {
            if self.is_captured(name) {
                continue;
            } // Skip alloca for captured locals
            if !self.value_map.contains_key(name) {
                let reg_name = format!("%{}_ptr", name);
                self.emit_line(&format!("{} = alloca i64", reg_name));
                self.value_map.insert(name.clone(), reg_name);
            }
        }

        // Sync parameters to environment if captured
        for p in &func.params {
            if let Some(cap_key) = self.get_captured_key(p) {
                if let Some(env) = self.current_env.clone() {
                    if !self.declared_functions.contains("rt_Map_set") {
                        self.global_buffer
                            .push_str("declare i64 @rt_Map_set(i64, i64, i64) nounwind\n");
                        self.declared_functions.insert("rt_Map_set".to_string());
                    }
                    if !self.declared_functions.contains("rt_box_string") {
                        self.global_buffer
                            .push_str("declare i64 @rt_box_string(i64)\n");
                        self.declared_functions.insert("rt_box_string".to_string());
                    }

                    // Create key string - use base captured name
                    let key_global = format!("@str_key_{}", cap_key.replace("$", "_"));
                    if !self.declared_globals.contains(&key_global) {
                        self.global_buffer.push_str(&format!(
                            "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"\n",
                            key_global,
                            cap_key.len() + 1,
                            cap_key
                        ));
                        self.declared_globals.insert(key_global.clone());
                    }

                    self.temp_counter += 1;
                    let key_id = format!("%key_id{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_box_string(i64 ptrtoint ([{} x i8]* {} to i64))",
                        key_id,
                        cap_key.len() + 1,
                        key_global
                    ));

                    self.emit_line(&format!(
                        "call i64 @rt_Map_set(i64 {}, i64 {}, i64 %{})",
                        env, key_id, p
                    ));
                }
            }
        }

        // We record the position to inject alloca instructions later
        let entry_marker = self.buffer.len();

        // Branch to first block
        if !func.blocks.is_empty() {
            self.emit_line(&format!("br label %{}", func.blocks[0].name));
        } else {
            self.emit_line("ret i64 0");
        }

        // Generate blocks with block name resolution
        for (_i, bb) in func.blocks.iter().enumerate() {
            self.emit(&format!("{}:\n", bb.name));

            let mut has_handler = false;
            if let Some(handler_idx) = bb.exception_handler {
                if handler_idx < func.blocks.len() {
                    has_handler = true;
                    let handler_name = &func.blocks[handler_idx].name;
                    // Allocate jmp_buf on THIS function's stack frame
                    self.temp_counter += 1;
                    let jmpbuf = format!("%jmpbuf{}", self.temp_counter);
                    self.emit_line(&format!("{} = alloca [37 x i64]", jmpbuf));
                    self.temp_counter += 1;
                    let jmpbuf_ptr = format!("%jmpbuf_ptr{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast [37 x i64]* {} to i8*",
                        jmpbuf_ptr, jmpbuf
                    ));
                    // Call setjmp inline — this is the critical part
                    self.temp_counter += 1;
                    let handler_res = format!("%handler_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i32 @_setjmp(i8* {}) returns_twice",
                        handler_res, jmpbuf_ptr
                    ));
                    // If setjmp returned 0, register the handler and continue
                    self.temp_counter += 1;
                    let is_exception = format!("%is_exception{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ne i32 {}, 0",
                        is_exception, handler_res
                    ));
                    let body_label = format!("{}_body", bb.name);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        is_exception, handler_name, body_label
                    ));
                    self.emit(&format!("{}:\n", body_label));
                    // Push handler AFTER setjmp returned 0 (normal path)
                    self.emit_line(&format!("call void @tejx_push_handler(i8* {})", jmpbuf_ptr));
                }
            }

            for inst in &bb.instructions {
                self.gen_instruction_v2(inst, func, &bb.name);
            }

            if has_handler {
                // If the block is not terminated, we need to call tejx_try_end before the terminator
                // But MIR instructions usually end with a terminator (Jump/Branch/Return).
                // We should inject tejx_try_end BEFORE the terminator.
                // Wait, gen_instruction_v2 handles terminators.
                // I need to modify gen_instruction_v2 or handle it here if I see a terminator.
            }
        }

        self.emit("}\n\n");

        if !self.alloca_buffer.is_empty() {
            self.buffer.insert_str(entry_marker, &self.alloca_buffer);
            self.alloca_buffer.clear();
        }
    }

    fn gen_instruction_v2(&mut self, inst: &MIRInstruction, func: &MIRFunction, _bb_name: &str) {
        match inst {
            MIRInstruction::Move { dst, src, .. } => {
                let val = self.resolve_value(src);

                if let Some(cap_key) = self.get_captured_key(dst) {
                    if let Some(env) = self.current_env.clone() {
                        if !self.declared_functions.contains("rt_Map_set") {
                            self.global_buffer
                                .push_str("declare i64 @rt_Map_set(i64, i64, i64) nounwind\n");
                            self.declared_functions.insert("rt_Map_set".to_string());
                        }
                        if !self.declared_functions.contains("rt_box_string") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_string(i64)\n");
                            self.declared_functions.insert("rt_box_string".to_string());
                        }

                        // Get/Create key string - use base captured name
                        let key_global = format!("@str_key_{}", cap_key.replace("$", "_"));
                        if !self.declared_globals.contains(&key_global) {
                            self.global_buffer.push_str(&format!(
                                "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"\n",
                                key_global,
                                cap_key.len() + 1,
                                cap_key
                            ));
                            self.declared_globals.insert(key_global.clone());
                        }

                        self.temp_counter += 1;
                        let key_id = format!("%key_id{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_string(i64 ptrtoint ([{} x i8]* {} to i64))",
                            key_id,
                            cap_key.len() + 1,
                            key_global
                        ));

                        self.emit_line(&format!(
                            "call i64 @rt_Map_set(i64 {}, i64 {}, i64 {})",
                            env, key_id, val
                        ));
                        return;
                    }
                }

                // Store new value (reassignment drops are handled statically by BorrowChecker)
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", val, ptr));

                // Propagate array data pointer tracking across variable copies
                if let MIRValue::Variable { name: src_name, .. } = src {
                    if let Some(info) = self.heap_array_ptrs.get(src_name).cloned() {
                        self.heap_array_ptrs.insert(dst.to_string(), info);
                    }
                    if self.stack_arrays.contains(src_name.as_str()) {
                        self.stack_arrays.insert(dst.to_string());
                    }
                }
            }
            MIRInstruction::BinaryOp {
                dst,
                left,
                op,
                right,
                ..
            } => {
                let l = self.resolve_value(left);
                let r = self.resolve_value(right);
                self.temp_counter += 1;
                let tmp = format!("%tmp{}", self.temp_counter);

                let l_ty = match left {
                    MIRValue::Constant { ty, .. } => ty,
                    MIRValue::Variable { ty, .. } => ty,
                };
                let r_ty = match right {
                    MIRValue::Constant { ty, .. } => ty,
                    MIRValue::Variable { ty, .. } => ty,
                };

                // Check types
                let is_string_op =
                    matches!(l_ty, TejxType::String) || matches!(r_ty, TejxType::String);
                let is_float_op = l_ty.is_float() || r_ty.is_float();
                let is_any_op = matches!(l_ty, TejxType::Any) || matches!(r_ty, TejxType::Any);

                let is_numeric_op = !is_string_op
                    && (is_float_op
                        || is_any_op
                        || l_ty.is_numeric()
                        || r_ty.is_numeric()
                        || matches!(l_ty, TejxType::Bool)
                        || matches!(r_ty, TejxType::Bool));

                if is_string_op {
                    if matches!(op, TokenType::Plus) {
                        if !self.declared_functions.contains("rt_str_concat_v2") {
                            self.global_buffer
                                .push_str("declare i64 @rt_str_concat_v2(i64, i64) nounwind\n");
                            self.declared_functions
                                .insert("rt_str_concat_v2".to_string());
                        }

                        let l_val = if l_ty.is_numeric() {
                            if !self.declared_functions.contains("rt_box_number") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_number(double) readnone\n");
                                self.declared_functions.insert("rt_box_number".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_l{}", self.temp_counter);
                            let val_as_double = if !l_ty.is_float() {
                                self.temp_counter += 1;
                                let d = format!("%d_l{}", self.temp_counter);
                                self.emit_line(&format!("{} = sitofp i64 {} to double", d, l));
                                d
                            } else {
                                self.temp_counter += 1;
                                let d = format!("%d_l{}", self.temp_counter);
                                self.emit_line(&format!("{} = bitcast i64 {} to double", d, l));
                                d
                            };
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_number(double {})",
                                boxed, val_as_double
                            ));
                            boxed
                        } else if matches!(l_ty, TejxType::Bool) {
                            if !self.declared_functions.contains("rt_box_boolean") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_boolean(i64)\n");
                                self.declared_functions.insert("rt_box_boolean".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_l{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_boolean(i64 {})",
                                boxed, l
                            ));
                            boxed
                        } else if matches!(l_ty, TejxType::String) && l.starts_with("ptrtoint") {
                            if !self.declared_functions.contains("rt_box_string") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_string(i64)\n");
                                self.declared_functions.insert("rt_box_string".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_l{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_string(i64 {})",
                                boxed, l
                            ));
                            boxed
                        } else if matches!(l_ty, TejxType::Any) {
                            // Box Any-typed values with rt_box_int to prevent raw 0 → null
                            if !self.declared_functions.contains("rt_box_int") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_int(i64)\n");
                                self.declared_functions.insert("rt_box_int".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_l{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_box_int(i64 {})", boxed, l));
                            boxed
                        } else {
                            l.to_string()
                        };

                        let r_val = if r_ty.is_numeric() {
                            if !self.declared_functions.contains("rt_box_number") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_number(double) readnone\n");
                                self.declared_functions.insert("rt_box_number".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_r{}", self.temp_counter);
                            let val_as_double = if !r_ty.is_float() {
                                self.temp_counter += 1;
                                let d = format!("%d_r{}", self.temp_counter);
                                self.emit_line(&format!("{} = sitofp i64 {} to double", d, r));
                                d
                            } else {
                                self.temp_counter += 1;
                                let d = format!("%d_r{}", self.temp_counter);
                                self.emit_line(&format!("{} = bitcast i64 {} to double", d, r));
                                d
                            };
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_number(double {})",
                                boxed, val_as_double
                            ));
                            boxed
                        } else if matches!(r_ty, TejxType::Bool) {
                            if !self.declared_functions.contains("rt_box_boolean") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_boolean(i64)\n");
                                self.declared_functions.insert("rt_box_boolean".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_r{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_boolean(i64 {})",
                                boxed, r
                            ));
                            boxed
                        } else if matches!(r_ty, TejxType::String) && r.starts_with("ptrtoint") {
                            if !self.declared_functions.contains("rt_box_string") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_string(i64)\n");
                                self.declared_functions.insert("rt_box_string".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_r{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_string(i64 {})",
                                boxed, r
                            ));
                            boxed
                        } else if matches!(r_ty, TejxType::Any) {
                            if !self.declared_functions.contains("rt_box_int") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_int(i64)\n");
                                self.declared_functions.insert("rt_box_int".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed_r{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_box_int(i64 {})", boxed, r));
                            boxed
                        } else {
                            r.to_string()
                        };

                        self.emit_line(&format!(
                            "{} = call i64 @rt_str_concat_v2(i64 {}, i64 {})",
                            tmp, l_val, r_val
                        ));
                    } else if matches!(op, TokenType::EqualEqual) {
                        if !self.declared_functions.contains("rt_str_equals") {
                            self.global_buffer
                                .push_str("declare i64 @rt_str_equals(i64, i64)\n");
                            self.declared_functions.insert("rt_str_equals".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i64 @rt_str_equals(i64 {}, i64 {})",
                            tmp, l, r
                        ));
                    } else if matches!(op, TokenType::BangEqual) {
                        if !self.declared_functions.contains("rt_str_equals") {
                            self.global_buffer
                                .push_str("declare i64 @rt_str_equals(i64, i64)\n");
                            self.declared_functions.insert("rt_str_equals".to_string());
                        }
                        if !self.declared_functions.contains("rt_not") {
                            self.global_buffer.push_str("declare i64 @rt_not(i64)\n");
                            self.declared_functions.insert("rt_not".to_string());
                        }
                        let eq_tmp = format!("%eq{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_str_equals(i64 {}, i64 {})",
                            eq_tmp, l, r
                        ));
                        self.temp_counter += 1;
                        self.emit_line(&format!("{} = call i64 @rt_not(i64 {})", tmp, eq_tmp));
                    } else {
                        self.emit_line(&format!("{} = add i64 {}, {}", tmp, l, r));
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
                } else if is_numeric_op {
                    let l_is_raw = l_ty.is_numeric() && !l_ty.is_float();
                    let r_is_raw = r_ty.is_numeric() && !r_ty.is_float();

                    if l_is_raw && r_is_raw {
                        // Raw Integer path (Fast path)
                        let (is_cmp, llvm_op, pred) = match op {
                            TokenType::Plus => (false, "add", ""),
                            TokenType::Minus => (false, "sub", ""),
                            TokenType::Star => (false, "mul", ""),
                            TokenType::Slash => (false, "sdiv", ""),
                            TokenType::Less => (true, "", "slt"),
                            TokenType::Greater => (true, "", "sgt"),
                            TokenType::EqualEqual => (true, "", "eq"),
                            TokenType::BangEqual => (true, "", "ne"),
                            TokenType::LessEqual => (true, "", "sle"),
                            TokenType::GreaterEqual => (true, "", "sge"),
                            TokenType::Modulo => (false, "srem", ""),
                            TokenType::Ampersand => (false, "and", ""),
                            TokenType::Pipe => (false, "or", ""),
                            TokenType::Caret => (false, "xor", ""),
                            TokenType::LessLess => (false, "shl", ""),
                            TokenType::GreaterGreater => (false, "ashr", ""),
                            _ => (false, "add", ""),
                        };
                        if is_cmp {
                            self.temp_counter += 1;
                            let cmp_res = format!("%cmp_res{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = icmp {} i64 {}, {}",
                                cmp_res, pred, l, r
                            ));
                            self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_res));
                        } else {
                            if llvm_op == "sdiv" || llvm_op == "srem" {
                                if !self.declared_functions.contains("rt_div_zero_error") {
                                    self.global_buffer
                                        .push_str("declare void @rt_div_zero_error()\n");
                                    self.declared_functions
                                        .insert("rt_div_zero_error".to_string());
                                }
                                let label_id = self.temp_counter;
                                self.temp_counter += 1;
                                let is_zero = format!("%is_zero{}", self.temp_counter);
                                self.emit_line(&format!("{} = icmp eq i64 {}, 0", is_zero, r));

                                let div_error = format!("div_zero_err{}", label_id);
                                let div_norm = format!("div_norm{}", label_id);
                                self.emit_line(&format!(
                                    "br i1 {}, label %{}, label %{}",
                                    is_zero, div_error, div_norm
                                ));

                                self.emit_line(&format!("{}:", div_error));
                                self.emit_line("call void @rt_div_zero_error()");
                                self.emit_line("unreachable");

                                self.emit_line(&format!("{}:", div_norm));
                            }
                            self.emit_line(&format!("{} = {} i64 {}, {}", tmp, llvm_op, l, r));
                        }
                    } else {
                        // Double precision path (Promotion)
                        let l_f = self.resolve_float_value(left);
                        let r_f = self.resolve_float_value(right);

                        let (is_cmp, llvm_op, pred) = match op {
                            TokenType::Plus => (false, "fadd", ""),
                            TokenType::Minus => (false, "fsub", ""),
                            TokenType::Star => (false, "fmul", ""),
                            TokenType::Slash => (false, "fdiv", ""),
                            TokenType::Less => (true, "", "olt"),
                            TokenType::Greater => (true, "", "ogt"),
                            TokenType::EqualEqual => (true, "", "oeq"),
                            TokenType::EqualEqualEqual => (true, "", "oeq"),
                            TokenType::BangEqual => (true, "", "one"),
                            TokenType::BangEqualEqual => (true, "", "one"),
                            TokenType::LessEqual => (true, "", "ole"),
                            TokenType::GreaterEqual => (true, "", "oge"),
                            TokenType::Modulo => (false, "frem", ""),
                            _ => (false, "fadd", ""),
                        };

                        // Specialized path for Any equality to use value comparison
                        let is_equality = matches!(
                            op,
                            TokenType::EqualEqual
                                | TokenType::BangEqual
                                | TokenType::EqualEqualEqual
                                | TokenType::BangEqualEqual
                        );

                        if is_any_op && is_equality {
                            let l_eval = self.resolve_value(left);
                            let r_eval = self.resolve_value(right);
                            let func = match op {
                                TokenType::EqualEqual | TokenType::BangEqual => "rt_eq",
                                _ => "rt_strict_equal",
                            };
                            if !self.declared_functions.contains(func) {
                                self.global_buffer
                                    .push_str(&format!("declare i64 @{}(i64, i64)\n", func));
                                self.declared_functions.insert(func.to_string());
                            }
                            let eq_res = format!("%eq_res{}", self.temp_counter);
                            self.temp_counter += 1;
                            self.emit_line(&format!(
                                "{} = call i64 @{}(i64 {}, i64 {})",
                                eq_res, func, l_eval, r_eval
                            ));

                            if matches!(op, TokenType::BangEqual | TokenType::BangEqualEqual) {
                                if !self.declared_functions.contains("rt_not") {
                                    self.global_buffer.push_str("declare i64 @rt_not(i64)\n");
                                    self.declared_functions.insert("rt_not".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_not(i64 {})",
                                    tmp, eq_res
                                ));
                            } else {
                                self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, eq_res));
                            }
                        } else if is_cmp {
                            self.temp_counter += 1;
                            let cmp_res = format!("%cmp_res{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = fcmp {} double {}, {}",
                                cmp_res, pred, l_f, r_f
                            ));
                            self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_res));
                        } else {
                            self.temp_counter += 1;
                            let res_f = format!("%res_f_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = {} double {}, {}",
                                res_f, llvm_op, l_f, r_f
                            ));

                            // Record float SSA variable for potential reuse
                            self.float_ssa_vars.insert(dst.clone(), res_f.clone());

                            // Does the destination expect a raw integer or a bitcasted double?
                            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);
                            if dst_ty.is_numeric() && !dst_ty.is_float() {
                                self.emit_line(&format!(
                                    "{} = fptosi double {} to i64",
                                    tmp, res_f
                                ));
                            } else {
                                self.emit_line(&format!(
                                    "{} = bitcast double {} to i64",
                                    tmp, res_f
                                ));
                            }
                        }
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
                } else {
                    // Integer / DefaultFallback
                    let (is_cmp, llvm_op, pred) = match op {
                        TokenType::Plus => (false, "add", ""),
                        TokenType::Minus => (false, "sub", ""),
                        TokenType::Star => (false, "mul", ""),
                        TokenType::Slash => (false, "sdiv", ""),
                        TokenType::Modulo => (false, "srem", ""),
                        TokenType::Less => (true, "", "slt"),
                        TokenType::Greater => (true, "", "sgt"),
                        TokenType::LessEqual => (true, "", "sle"),
                        TokenType::GreaterEqual => (true, "", "sge"),
                        TokenType::EqualEqual => (true, "", "eq"),
                        TokenType::BangEqual => (true, "", "ne"),
                        _ => (false, "add", ""),
                    };
                    if is_cmp {
                        self.temp_counter += 1;
                        let cmp_res = format!("%cmp_res{}", self.temp_counter);
                        self.emit_line(&format!("{} = icmp {} i64 {}, {}", cmp_res, pred, l, r));
                        self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_res));
                    } else {
                        self.temp_counter += 1;
                        let res_i = format!("%res_i{}", self.temp_counter);
                        self.emit_line(&format!("{} = {} i64 {}, {}", res_i, llvm_op, l, r));

                        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);
                        if matches!(dst_ty, TejxType::Any) {
                            // Inline: convert int result to bitcasted double representation
                            self.temp_counter += 1;
                            let res_f = format!("%res_f{}", self.temp_counter);
                            self.emit_line(&format!("{} = sitofp i64 {} to double", res_f, res_i));
                            self.emit_line(&format!("{} = bitcast double {} to i64", tmp, res_f));
                        } else {
                            self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, res_i));
                        }
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
                }
            }

            MIRInstruction::Jump { target, .. } => {
                let has_handler = func
                    .blocks
                    .iter()
                    .any(|b| b.name == _bb_name && b.exception_handler.is_some());
                if has_handler {
                    self.emit_line("call void @tejx_pop_handler()");
                }

                if *target < func.blocks.len() {
                    self.emit_line(&format!("br label %{}", func.blocks[*target].name));
                }
            }
            MIRInstruction::TrySetup { try_target, .. } => {
                if *try_target < func.blocks.len() {
                    self.emit_line(&format!("br label %{}", func.blocks[*try_target].name));
                }
            }
            MIRInstruction::PopHandler { .. } => {
                self.emit_line("call void @tejx_pop_handler()");
            }
            MIRInstruction::Branch {
                condition,
                true_target,
                false_target,
                ..
            } => {
                let has_handler = func
                    .blocks
                    .iter()
                    .any(|b| b.name == _bb_name && b.exception_handler.is_some());
                if has_handler {
                    self.emit_line("call void @tejx_pop_handler()");
                }

                let cond_val = self.resolve_value(condition);
                let ty = match condition {
                    MIRValue::Constant { ty, .. } => ty,
                    MIRValue::Variable { ty, .. } => ty,
                };

                let cond = if matches!(ty, TejxType::Any) {
                    if !self.declared_functions.contains("rt_to_boolean") {
                        self.global_buffer
                            .push_str("declare i64 @rt_to_boolean(i64)\n");
                        self.declared_functions.insert("rt_to_boolean".to_string());
                    }
                    self.temp_counter += 1;
                    let bool_val = format!("%bool_val{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_boolean(i64 {})",
                        bool_val, cond_val
                    ));
                    bool_val
                } else {
                    cond_val
                };

                self.temp_counter += 1;
                let cmp = format!("%cmp{}", self.temp_counter);
                self.emit_line(&format!("{} = icmp ne i64 {}, 0", cmp, cond));

                let true_name = if *true_target < func.blocks.len() {
                    func.blocks[*true_target].name.clone()
                } else {
                    "unknown".to_string()
                };
                let false_name = if *false_target < func.blocks.len() {
                    func.blocks[*false_target].name.clone()
                } else {
                    "unknown".to_string()
                };
                self.emit_line(&format!(
                    "br i1 {}, label %{}, label %{}",
                    cmp, true_name, false_name
                ));
            }
            MIRInstruction::Return { value, .. } => {
                let has_handler = func
                    .blocks
                    .iter()
                    .any(|b| b.name == _bb_name && b.exception_handler.is_some());
                if has_handler {
                    self.emit_line("call void @tejx_pop_handler()");
                }

                let _ret_var_name = if let Some(MIRValue::Variable { name, .. }) = value {
                    Some(name.clone())
                } else {
                    None
                };

                if let Some(val) = value {
                    let v = self.resolve_value(val);
                    self.emit_line(&format!("ret i64 {}", v));
                } else {
                    self.emit_line("ret i64 0");
                }
            }
            MIRInstruction::Call {
                dst, callee, args, ..
            } => {
                if callee == "rt_box_number" {
                    let float_val = self.resolve_float_value(&args[0]);

                    if !self.declared_functions.contains("rt_box_number") {
                        self.global_buffer
                            .push_str("declare i64 @rt_box_number(double) readnone\n");
                        self.declared_functions.insert("rt_box_number".to_string());
                    }

                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_box_number(double {})",
                        result_tmp, float_val
                    ));
                    if !dst.is_empty() {
                        self.float_ssa_vars.insert(dst.clone(), float_val);
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                    }
                    return;
                }

                if callee == "rt_to_number" {
                    let float_val = self.resolve_float_value(&args[0]);

                    if !dst.is_empty() {
                        self.float_ssa_vars.insert(dst.clone(), float_val.clone());
                        self.temp_counter += 1;
                        let bits_tmp = format!("%bits{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            bits_tmp, float_val
                        ));
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", bits_tmp, ptr));
                    }
                    return;
                }

                if callee == "rt_box_int" {
                    let arg_val = self.resolve_value(&args[0]);

                    if !self.declared_functions.contains("rt_box_int") {
                        self.global_buffer
                            .push_str("declare i64 @rt_box_int(i64)\n");
                        self.declared_functions.insert("rt_box_int".to_string());
                    }

                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_box_int(i64 {})",
                        result_tmp, arg_val
                    ));

                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                    }
                    return;
                }

                // Handle print/eprint specifically for variadic support (like console.log)
                if callee == "print" {
                    if !self.declared_functions.contains("print_raw") {
                        self.global_buffer
                            .push_str("declare void @print_raw(i64)\n");
                        self.declared_functions.insert("print_raw".to_string());
                    }
                    if !self.declared_functions.contains("print_space") {
                        self.global_buffer.push_str("declare void @print_space()\n");
                        self.declared_functions.insert("print_space".to_string());
                    }
                    if !self.declared_functions.contains("print_newline") {
                        self.global_buffer
                            .push_str("declare void @print_newline()\n");
                        self.declared_functions.insert("print_newline".to_string());
                    }
                    if !self.declared_functions.contains("rt_box_boolean") {
                        self.global_buffer
                            .push_str("declare i64 @rt_box_boolean(i64)\n");
                        self.declared_functions.insert("rt_box_boolean".to_string());
                    }

                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.emit_line("call void @print_space()");
                        }
                        let mut arg_val = self.resolve_value(arg);
                        let arg_ty = arg.get_type();

                        // Use direct print functions for known numeric types (avoids boxing overhead and 0→null bug)
                        if arg_ty.is_float() {
                            if !self.declared_functions.contains("print_float") {
                                self.global_buffer
                                    .push_str("declare void @print_float(i64)\n");
                                self.declared_functions.insert("print_float".to_string());
                            }
                            self.emit_line(&format!("call void @print_float(i64 {})", arg_val));
                        } else if arg_ty.is_numeric() {
                            if !self.declared_functions.contains("print_int") {
                                self.global_buffer
                                    .push_str("declare void @print_int(i64)\n");
                                self.declared_functions.insert("print_int".to_string());
                            }
                            self.emit_line(&format!("call void @print_int(i64 {})", arg_val));
                        } else {
                            // For non-numeric types, box if needed then use print_raw
                            let boxfunc = match arg_ty {
                                TejxType::Bool => Some("rt_box_boolean"),
                                TejxType::String if matches!(arg, MIRValue::Constant { .. }) => {
                                    Some("rt_box_string")
                                }
                                _ => None,
                            };

                            if let Some(f) = boxfunc {
                                if !self.declared_functions.contains(f) {
                                    self.global_buffer
                                        .push_str(&format!("declare i64 @{}(i64)\n", f));
                                    self.declared_functions.insert(f.to_string());
                                }

                                let temp = format!("%t_box_print_{}_{}", i, self.temp_counter);
                                self.temp_counter += 1;
                                self.emit_line(&format!(
                                    "{} = call i64 @{}(i64 {})",
                                    temp, f, arg_val
                                ));
                                arg_val = temp;
                            }
                            self.emit_line(&format!("call void @print_raw(i64 {})", arg_val));
                        }
                    }
                    self.emit_line("call void @print_newline()");

                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 0, i64* {}", ptr));
                    }
                } else if callee == "eprint" {
                    if !self.declared_functions.contains("eprint_raw") {
                        self.global_buffer
                            .push_str("declare void @eprint_raw(i64)\n");
                        self.declared_functions.insert("eprint_raw".to_string());
                    }
                    if !self.declared_functions.contains("eprint_space") {
                        self.global_buffer
                            .push_str("declare void @eprint_space()\n");
                        self.declared_functions.insert("eprint_space".to_string());
                    }
                    if !self.declared_functions.contains("eprint_newline") {
                        self.global_buffer
                            .push_str("declare void @eprint_newline()\n");
                        self.declared_functions.insert("eprint_newline".to_string());
                    }
                    if !self.declared_functions.contains("rt_box_boolean") {
                        self.global_buffer
                            .push_str("declare i64 @rt_box_boolean(i64)\n");
                        self.declared_functions.insert("rt_box_boolean".to_string());
                    }

                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.emit_line("call void @eprint_space()");
                        }
                        let mut arg_val = self.resolve_value(arg);
                        if matches!(arg.get_type(), TejxType::Bool) {
                            let temp = format!("%te_box_{}_{}", i, self.temp_counter);
                            self.temp_counter += 1;
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_boolean(i64 {})",
                                temp, arg_val
                            ));
                            arg_val = temp;
                        }
                        self.emit_line(&format!("call void @eprint_raw(i64 {})", arg_val));
                    }
                    self.emit_line("call void @eprint_newline()");

                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 0, i64* {}", ptr));
                    }
                } else if callee.starts_with("std_math_") {
                    // Emit LLVM intrinsics directly for math functions
                    // This avoids the runtime call overhead (i64→f64 unbox → math → f64→i64 box)
                    let intrinsic = match callee.as_str() {
                        "std_math_sqrt" => Some(("llvm.sqrt.f64", 1)),
                        "std_math_sin" => Some(("llvm.sin.f64", 1)),
                        "std_math_cos" => Some(("llvm.cos.f64", 1)),
                        "std_math_pow" => Some(("llvm.pow.f64", 2)),
                        "std_math_floor" => Some(("llvm.floor.f64", 1)),
                        "std_math_ceil" => Some(("llvm.ceil.f64", 1)),
                        "std_math_abs" => Some(("llvm.fabs.f64", 1)),
                        "std_math_round" => Some(("llvm.round.f64", 1)),
                        _ => None,
                    };

                    if let Some((intrinsic_name, param_count)) = intrinsic {
                        // Declare the intrinsic
                        if !self.declared_functions.contains(intrinsic_name) {
                            if param_count == 1 {
                                self.global_buffer.push_str(&format!(
                                    "declare double @{}(double)\n",
                                    intrinsic_name
                                ));
                            } else {
                                self.global_buffer.push_str(&format!(
                                    "declare double @{}(double, double)\n",
                                    intrinsic_name
                                ));
                            }
                            self.declared_functions.insert(intrinsic_name.to_string());
                        }

                        // Convert arg(s) from i64 to double using optimal SSA path
                        let arg1_f = self.resolve_float_value(&args[0]);

                        let result_f;
                        if param_count == 2 && args.len() >= 2 {
                            let arg2_f = self.resolve_float_value(&args[1]);
                            self.temp_counter += 1;
                            result_f = format!("%intrinsic_res_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call double @{}(double {}, double {})",
                                result_f, intrinsic_name, arg1_f, arg2_f
                            ));
                        } else {
                            self.temp_counter += 1;
                            result_f = format!("%intrinsic_res_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = call double @{}(double {})",
                                result_f, intrinsic_name, arg1_f
                            ));
                        }

                        // Record float SSA variable for potential reuse
                        if !dst.is_empty() {
                            self.float_ssa_vars.insert(dst.clone(), result_f.clone());
                        }

                        // Convert result back to i64 (bitcast double to i64)
                        self.temp_counter += 1;
                        let result_i = format!("%intrinsic_bits_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            result_i, result_f
                        ));

                        if !dst.is_empty() {
                            let ptr = self.resolve_ptr(dst);
                            self.emit_line(&format!("store i64 {}, i64* {}", result_i, ptr));
                        }
                    } else {
                        // Fallback to runtime call for unsupported math functions (random, min, max)
                        let mut arg_vals = Vec::new();
                        for arg in args {
                            let arg_val = self.resolve_value(arg);
                            arg_vals.push(format!("i64 {}", arg_val));
                        }
                        let args_str = arg_vals.join(", ");
                        if !self.declared_functions.contains(callee) {
                            let decl_args = vec!["i64"; arg_vals.len()].join(", ");
                            self.global_buffer.push_str(&format!(
                                "declare i64 @{}({}) readnone\n",
                                callee, decl_args
                            ));
                            self.declared_functions.insert(callee.clone());
                        }
                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @{}({})",
                            result_tmp, callee, args_str
                        ));
                        if !dst.is_empty() {
                            let ptr = self.resolve_ptr(dst);
                            self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                        }
                    }
                } else {
                    let mut arg_vals = Vec::new();
                    for arg in args {
                        let arg_val = self.resolve_value(arg);
                        arg_vals.push(format!("i64 {}", arg_val));
                    }

                    let mut final_callee = callee.clone();
                    let mut is_instance_call = false;
                    let mut instance_var = String::new();

                    if callee.contains('.') {
                        let parts: Vec<&str> = callee.split('.').collect();
                        if parts.len() == 2 {
                            let base = parts[0];
                            let method = parts[1];
                            if self.value_map.contains_key(base) {
                                is_instance_call = true;
                                instance_var = base.to_string();
                                if method == "join" {
                                    final_callee = "Thread_join".to_string();
                                } else if method == "push" {
                                    final_callee = "Array_push".to_string();
                                } else if method == "pop" {
                                    final_callee = "Array_pop".to_string();
                                } else if method == "fill" {
                                    final_callee = "Array_fill".to_string();
                                } else if method == "concat" {
                                    final_callee = "arrUtil_concat".to_string();
                                } else if method == "forEach" {
                                    final_callee = "Array_forEach".to_string();
                                } else if method == "map" {
                                    final_callee = "Array_map".to_string();
                                } else if method == "filter" {
                                    final_callee = "Array_filter".to_string();
                                } else if method == "indexOf" {
                                    final_callee = "arrUtil_indexOf".to_string();
                                } else if method == "shift" {
                                    final_callee = "arrUtil_shift".to_string();
                                } else if method == "unshift" {
                                    final_callee = "arrUtil_unshift".to_string();
                                } else if method == "slice" {
                                    final_callee = "Array_slice".to_string();
                                } else if method == "reduce" {
                                    final_callee = "Array_reduce".to_string();
                                } else if method == "find" {
                                    final_callee = "Array_find".to_string();
                                } else if method == "findIndex" {
                                    final_callee = "Array_findIndex".to_string();
                                } else if method == "reverse" {
                                    final_callee = "Array_reverse".to_string();
                                } else if method == "splice" {
                                    final_callee = "Array_splice".to_string();
                                } else if method == "clone" {
                                    final_callee = "Array_clone".to_string();
                                } else if method == "lock" {
                                    final_callee = "m_lock".to_string();
                                } else if method == "unlock" {
                                    final_callee = "m_unlock".to_string();
                                } else if method == "padStart" {
                                    final_callee = "trimmed_padStart".to_string();
                                } else if method == "padEnd" {
                                    final_callee = "trimmed_padEnd".to_string();
                                } else if method == "repeat" {
                                    final_callee = "trimmed_repeat".to_string();
                                } else if method == "keys" {
                                    final_callee = "Object_keys".to_string();
                                } else if method == "values" {
                                    final_callee = "Object_values".to_string();
                                } else if method == "entries" {
                                    final_callee = "Object_entries".to_string();
                                } else if method == "size" {
                                    final_callee = "Collection_size".to_string();
                                } else if method == "add" {
                                    final_callee = "Collection_add".to_string();
                                } else if method == "delete" {
                                    final_callee = "Collection_delete".to_string();
                                } else if method == "clear" {
                                    final_callee = "Collection_clear".to_string();
                                } else if method == "has" {
                                    final_callee = "Collection_has".to_string();
                                } else if method == "put" {
                                    final_callee = "Map_put".to_string();
                                }
                                // Specific to Map usually, or Collection_put?
                                else if method == "get" {
                                    final_callee = "Map_get".to_string();
                                }
                                // Specific to Map
                                else {
                                    final_callee = format!("{}_{}", base, method);
                                }
                            } else {
                                final_callee = format!("{}_{}", base, method);
                            }
                        }
                    } else if let Some(ptr) = self.value_map.get(callee) {
                        self.temp_counter += 1;
                        let func_val_tmp = format!("%func_val_{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i64, i64* {}", func_val_tmp, ptr));
                        self.temp_counter += 1;
                        let func_ptr_tmp = format!("%func_ptr_{}", self.temp_counter);
                        let ptr_args = vec!["i64"; args.len()].join(", ");
                        self.emit_line(&format!(
                            "{} = inttoptr i64 {} to i64 ({})*",
                            func_ptr_tmp, func_val_tmp, ptr_args
                        ));
                        let mut call_arg_vals = Vec::new();
                        for arg in args {
                            let arg_val = self.resolve_value(arg);
                            call_arg_vals.push(format!("i64 {}", arg_val));
                        }
                        let args_str = call_arg_vals.join(", ");
                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 {}({})",
                            result_tmp, func_ptr_tmp, args_str
                        ));
                        if !dst.is_empty() {
                            let ptr = self.resolve_ptr(dst);
                            self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                        }
                        return;
                    }

                    if is_instance_call {
                        if let Some(ptr) = self.value_map.get(&instance_var) {
                            self.temp_counter += 1;
                            let tmp = format!("%inst{}", self.temp_counter);
                            self.emit_line(&format!("{} = load i64, i64* {}", tmp, ptr));
                            arg_vals.insert(0, format!("i64 {}", tmp));
                        }
                    }

                    let args_str = arg_vals.join(", ");
                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    if !self.declared_functions.contains(&final_callee) {
                        let decl_args = vec!["i64"; arg_vals.len()].join(", ");
                        let pure_attrs = if final_callee.starts_with("std_math_") {
                            " readnone"
                        } else {
                            ""
                        };
                        self.global_buffer.push_str(&format!(
                            "declare i64 @{}({}){}\n",
                            final_callee, decl_args, pure_attrs
                        ));
                        self.declared_functions.insert(final_callee.clone());
                    }
                    self.emit_line(&format!(
                        "{} = call i64 @{}({})",
                        result_tmp, final_callee, args_str
                    ));

                    // --- INVALDIATION: Remove cached data pointers if array could reallocate ---
                    let mutating_methods = [
                        "Array_push",
                        "Array_pop",
                        "arrUtil_shift",
                        "arrUtil_unshift",
                        "Array_splice",
                    ];
                    if mutating_methods.contains(&final_callee.as_str()) {
                        if is_instance_call {
                            self.heap_array_ptrs.remove(&instance_var);
                        } else if !args.is_empty() {
                            if let MIRValue::Variable { name, .. } = &args[0] {
                                self.heap_array_ptrs.remove(name);
                            }
                        }
                    }

                    // --- FILL PROPAGATION: f_Array_fill returns the same array ---
                    // Propagate cached data pointer from args[0] to dst so that
                    // `let A = new Array(N).fill(1.0)` chains keep the fast path.
                    if (final_callee == "f_Array_fill" || final_callee == "Array_fill")
                        && !dst.is_empty()
                    {
                        if let Some(MIRValue::Variable { name, .. }) = args.first() {
                            if let Some(info) = self.heap_array_ptrs.get(name).cloned() {
                                self.heap_array_ptrs.insert(dst.to_string(), info);
                            }
                        }
                    }

                    // --- Ownership Transfer: Mark consumed arguments as Moved ---
                    let is_stdlib = final_callee.starts_with("Array_")
                        || final_callee.starts_with("Map_")
                        || final_callee.starts_with("String_")
                        || final_callee.starts_with("Math_")
                        || final_callee.starts_with("Collection_")
                        || final_callee.starts_with("Promise_")
                        || final_callee.starts_with("Thread_")
                        || final_callee.starts_with("tejx_")
                        || final_callee.starts_with("http_")
                        || final_callee.starts_with("d_")
                        || final_callee.starts_with("e_")
                        || final_callee.starts_with("n_")
                        || final_callee.starts_with("s_")
                        || final_callee == "__await"
                        || final_callee == "__resolve_promise"
                        || final_callee == "__reject_promise"
                        || final_callee == "print"
                        || final_callee == "len"
                        || final_callee == "assert"
                        || final_callee == "__join"
                        || final_callee.starts_with("rt_");

                    let is_method = final_callee.starts_with("m_");
                    let is_constructor = final_callee.ends_with("_constructor");
                    let is_container_mutator =
                        ["rt_Map_set", "rt_Map_put", "rt_Set_add"].contains(&final_callee.as_str());

                    for (i, _arg) in args.iter().enumerate() {
                        let mut should_consume = !is_stdlib;

                        if (is_constructor || is_method) && i == 0 {
                            should_consume = false; // 'this' is borrowed
                        }

                        if is_container_mutator && i > 0 {
                            should_consume = true;
                        }

                        // Fix: The worker task must NOT free the promise ID (Arg 0) after resolving.
                        // Consider it consumed by the resolve call (ownership transfer to runtime/void).
                        if (final_callee == "__resolve_promise"
                            || final_callee == "__reject_promise")
                            && (i == 0 || i == 1)
                        {
                            should_consume = true;
                        }

                        // Fix: The arguments bundle passed to a task MUST be consumed (moved to the task queue).
                        if final_callee == "tejx_enqueue_task" && i == 1 {
                            should_consume = true;
                        }

                        // Fix: Array_push must consume the value to take ownership (it's storing it).
                        if final_callee == "Array_push" && i == 1 {
                            should_consume = true;
                        }

                        if should_consume {}
                    }
                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                    }

                    // --- ARRAY CONSTRUCTOR: Cache data pointer for direct access ---
                    // Uses nocache variant to avoid clobbering LAST_ID cache
                    if final_callee == "f_Array_constructor" && args.len() >= 3 {
                        if let MIRValue::Variable {
                            name: this_name, ..
                        } = &args[0]
                        {
                            let is_escaped = self.does_escape(func, this_name);
                            if !is_escaped {
                                let this_val = self.resolve_value(&args[0]);
                                let elem_size_val =
                                    if let MIRValue::Constant { value, .. } = &args[2] {
                                        value.parse::<i64>().unwrap_or(8)
                                    } else {
                                        8
                                    };

                                if !self
                                    .declared_functions
                                    .contains("rt_array_get_data_ptr_nocache")
                                {
                                    self.global_buffer.push_str(
                                        "declare i64 @rt_array_get_data_ptr_nocache(i64) nounwind\n",
                                    );
                                    self.declared_functions
                                        .insert("rt_array_get_data_ptr_nocache".to_string());
                                }

                                self.temp_counter += 1;
                                let dp_val = format!("%ctor_dp_{}", self.temp_counter);
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_array_get_data_ptr_nocache(i64 {})",
                                    dp_val, this_val
                                ));

                                let alloca_name = format!(
                                    "%ctor_dp_alloca_{}",
                                    this_name.replace(".", "_").replace("#", "_")
                                );
                                self.alloca_buffer.push_str(&format!(
                                    "  {} = alloca i64, align 8\n",
                                    alloca_name
                                ));
                                self.emit_line(&format!(
                                    "store i64 {}, i64* {}",
                                    dp_val, alloca_name
                                ));

                                self.heap_array_ptrs
                                    .insert(this_name.clone(), (alloca_name, elem_size_val));
                            }
                        }
                    }
                }
            }
            MIRInstruction::IndirectCall {
                dst, callee, args, ..
            } => {
                let callee_val = self.resolve_value(callee);

                // Add null check for indirect call
                self.temp_counter += 1;
                let is_null = format!("%is_null{}", self.temp_counter);
                self.emit_line(&format!("{} = icmp eq i64 {}, 0", is_null, callee_val));
                self.temp_counter += 1;
                let fail_label = format!("call_fail_{}", self.temp_counter);
                let ok_label = format!("call_ok_{}", self.temp_counter);
                self.emit_line(&format!(
                    "br i1 {}, label %{}, label %{}",
                    is_null, fail_label, ok_label
                ));

                self.emit(&format!("{}:\n", fail_label));
                let err_msg = self.resolve_value(&MIRValue::Constant {
                    value: "\"Undefined function\"".to_string(),
                    ty: TejxType::String,
                });
                let err_obj = format!("%err_obj{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_box_string(i64 {})",
                    err_obj, err_msg
                ));
                self.emit_line(&format!("call void @tejx_throw(i64 {})", err_obj));
                self.emit_line("unreachable");

                self.emit(&format!("{}:\n", ok_label));

                if !self.declared_functions.contains("rt_get_closure_ptr") {
                    self.global_buffer
                        .push_str("declare i64 @rt_get_closure_ptr(i64)\n");
                    self.declared_functions
                        .insert("rt_get_closure_ptr".to_string());
                }
                if !self.declared_functions.contains("rt_get_closure_env") {
                    self.global_buffer
                        .push_str("declare i64 @rt_get_closure_env(i64)\n");
                    self.declared_functions
                        .insert("rt_get_closure_env".to_string());
                }

                self.temp_counter += 1;
                let ptr_reg = format!("%cb_ptr{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_get_closure_ptr(i64 {})",
                    ptr_reg, callee_val
                ));

                self.temp_counter += 1;
                let env_reg = format!("%cb_env{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_get_closure_env(i64 {})",
                    env_reg, callee_val
                ));

                self.temp_counter += 1;
                let func_ptr_tmp = format!("%func_ptr_{}", self.temp_counter);
                let mut arg_types = vec!["i64"]; // First arg is always env
                for _ in 0..args.len() {
                    arg_types.push("i64");
                }
                let ptr_args = arg_types.join(", ");
                self.emit_line(&format!(
                    "{} = inttoptr i64 {} to i64 ({})*",
                    func_ptr_tmp, ptr_reg, ptr_args
                ));

                let mut arg_vals = vec![format!("i64 {}", env_reg)];
                for arg in args {
                    let val = self.resolve_value(arg);
                    arg_vals.push(format!("i64 {}", val));
                }
                let args_str = arg_vals.join(", ");

                self.temp_counter += 1;
                let result_tmp = format!("%call{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 {}({})",
                    result_tmp, func_ptr_tmp, args_str
                ));

                if !dst.is_empty() {
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                }
            }
            MIRInstruction::ObjectLiteral { dst, entries, .. } => {
                self.temp_counter += 1;
                let obj_tmp = format!("%obj{}", self.temp_counter);

                // --- OPTIMIZATION: Batched object creation for 1-3 properties ---
                let n_entries = entries.len();
                if n_entries >= 1 && n_entries <= 3 {
                    // Box all values first
                    let mut kv_args: Vec<(String, String)> = Vec::new();
                    for (k, v) in entries {
                        let k_val = self.resolve_value(&MIRValue::Constant {
                            value: format!("\"{}\"", k),
                            ty: TejxType::String,
                        });
                        let mut v_val = self.resolve_value(v);

                        // Box primitives
                        let v_ty = v.get_type();
                        if v_ty.is_numeric() || matches!(v_ty, TejxType::Bool | TejxType::Char) {
                            self.temp_counter += 1;
                            let boxed_reg = format!("%boxed_obj_{}", self.temp_counter);
                            if v_ty.is_float() {
                                self.temp_counter += 1;
                                let d_val = format!("%d_val_obj_{}", self.temp_counter);
                                self.emit_line(&format!(
                                    "{} = bitcast i64 {} to double",
                                    d_val, v_val
                                ));
                                if !self.declared_functions.contains("rt_box_number") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_number(double) readnone\n");
                                    self.declared_functions.insert("rt_box_number".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_number(double {})",
                                    boxed_reg, d_val
                                ));
                            } else if matches!(v_ty, TejxType::Bool) {
                                if !self.declared_functions.contains("rt_box_boolean") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_boolean(i64)\n");
                                    self.declared_functions.insert("rt_box_boolean".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_boolean(i64 {})",
                                    boxed_reg, v_val
                                ));
                            } else {
                                if !self.declared_functions.contains("rt_box_int") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_int(i64)\n");
                                    self.declared_functions.insert("rt_box_int".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_int(i64 {})",
                                    boxed_reg, v_val
                                ));
                            }
                            v_val = boxed_reg;
                        }
                        kv_args.push((k_val, v_val));
                    }

                    // Emit batched call
                    let fn_name = format!("m_new_{}", n_entries);
                    let args_str: Vec<String> = kv_args
                        .iter()
                        .flat_map(|(k, v)| vec![format!("i64 {}", k), format!("i64 {}", v)])
                        .collect();
                    let arg_types: Vec<&str> = (0..n_entries * 2).map(|_| "i64").collect();

                    if !self.declared_functions.contains(&fn_name) {
                        self.global_buffer.push_str(&format!(
                            "declare i64 @{}({})\n",
                            fn_name,
                            arg_types.join(", ")
                        ));
                        self.declared_functions.insert(fn_name.clone());
                    }
                    self.emit_line(&format!(
                        "{} = call i64 @{}({})",
                        obj_tmp,
                        fn_name,
                        args_str.join(", ")
                    ));
                } else {
                    // Fallback for 0 or 4+ properties
                    if !self.declared_functions.contains("m_new") {
                        self.global_buffer.push_str("declare i64 @m_new()\n");
                        self.declared_functions.insert("m_new".to_string());
                    }
                    self.emit_line(&format!("{} = call i64 @m_new()", obj_tmp));

                    for (k, v) in entries {
                        let k_val = self.resolve_value(&MIRValue::Constant {
                            value: format!("\"{}\"", k),
                            ty: TejxType::String,
                        });
                        let mut v_val = self.resolve_value(v);

                        let v_ty = v.get_type();
                        if v_ty.is_numeric() || matches!(v_ty, TejxType::Bool | TejxType::Char) {
                            self.temp_counter += 1;
                            let boxed_reg = format!("%boxed_obj_{}", self.temp_counter);
                            if v_ty.is_float() {
                                self.temp_counter += 1;
                                let d_val = format!("%d_val_obj_{}", self.temp_counter);
                                self.emit_line(&format!(
                                    "{} = bitcast i64 {} to double",
                                    d_val, v_val
                                ));
                                if !self.declared_functions.contains("rt_box_number") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_number(double) readnone\n");
                                    self.declared_functions.insert("rt_box_number".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_number(double {})",
                                    boxed_reg, d_val
                                ));
                            } else if matches!(v_ty, TejxType::Bool) {
                                if !self.declared_functions.contains("rt_box_boolean") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_boolean(i64)\n");
                                    self.declared_functions.insert("rt_box_boolean".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_boolean(i64 {})",
                                    boxed_reg, v_val
                                ));
                            } else {
                                if !self.declared_functions.contains("rt_box_int") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_int(i64)\n");
                                    self.declared_functions.insert("rt_box_int".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_int(i64 {})",
                                    boxed_reg, v_val
                                ));
                            }
                            v_val = boxed_reg;
                        }

                        if !self.declared_functions.contains("rt_map_set_fast") {
                            self.global_buffer
                                .push_str("declare i64 @rt_map_set_fast(i64, i64, i64)\n");
                            self.declared_functions
                                .insert("rt_map_set_fast".to_string());
                        }
                        self.emit_line(&format!(
                            "call i64 @rt_map_set_fast(i64 {}, i64 {}, i64 {})",
                            obj_tmp, k_val, v_val
                        ));
                    }
                }
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", obj_tmp, ptr));
            }
            MIRInstruction::ArrayLiteral {
                dst, elements, ty, ..
            } => {
                self.temp_counter += 1;
                let arr_tmp = format!("%arr{}", self.temp_counter);

                let mut size = elements.len() as i64;
                let mut use_fixed = false;
                if let Some(TejxType::FixedArray(_, sz)) = ty {
                    size = *sz as i64;
                    use_fixed = true;
                }

                // --- STACK ALLOCATION FOR NON-ESCAPING ARRAYS ---
                let is_escaped = self.does_escape(func, dst);
                let can_stack_alloc = !is_escaped && size > 0 && use_fixed;

                let mut arr_elem_ty = TejxType::Any;
                if let Some(t) = &ty {
                    arr_elem_ty = t.get_array_element_type();
                }
                let needs_boxing =
                    !use_fixed && matches!(arr_elem_ty, TejxType::Any | TejxType::Class(_));

                if can_stack_alloc {
                    // Create stack allocation in the entry block
                    // Header: { length: i64, elem_size: i64 }
                    // Followed by elements: [size x i64]
                    let elem_size = if let Some(TejxType::FixedArray(inner, _)) = &ty {
                        if matches!(**inner, TejxType::Bool) {
                            1
                        } else {
                            8
                        }
                    } else {
                        8
                    };

                    self.temp_counter += 1;
                    let stack_ptr = format!("%stack_arr_{}", self.temp_counter);
                    let array_bytes = 16 + (size * elem_size); // 16 bytes for header + element data

                    self.alloca_buffer.push_str(&format!(
                        "  {} = alloca [{} x i8], align 8\n",
                        stack_ptr, array_bytes
                    ));

                    self.temp_counter += 1;
                    let cast_ptr = format!("%cast_arr_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast [{} x i8]* {} to i64*",
                        cast_ptr, array_bytes, stack_ptr
                    ));

                    // Store length at index 0
                    self.emit_line(&format!("store i64 {}, i64* {}", size, cast_ptr));
                    // Store elem_size at index 1
                    self.temp_counter += 1;
                    let elem_size_ptr = format!("%elem_sz_ptr_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr inbounds i64, i64* {}, i64 1",
                        elem_size_ptr, cast_ptr
                    ));
                    self.emit_line(&format!("store i64 {}, i64* {}", elem_size, elem_size_ptr));

                    // arr_tmp will hold the integer value of the pointer (ID)
                    self.temp_counter += 1;
                    let ptr_to_int = format!("%ptr_to_int_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = ptrtoint i64* {} to i64",
                        ptr_to_int, cast_ptr
                    ));
                    self.emit_line(&format!("{} = add i64 {}, 0", arr_tmp, ptr_to_int));
                } else {
                    if use_fixed {
                        if !self.declared_functions.contains("a_new_fixed") {
                            self.global_buffer
                                .push_str("declare i64 @a_new_fixed(i64, i64) nounwind\n");
                            self.declared_functions.insert("a_new_fixed".to_string());
                        }
                        let elem_size = if let Some(TejxType::FixedArray(inner, _)) = &ty {
                            if matches!(**inner, TejxType::Bool) {
                                1
                            } else {
                                8
                            }
                        } else {
                            8
                        };
                        self.emit_line(&format!(
                            "{} = call i64 @a_new_fixed(i64 {}, i64 {})",
                            arr_tmp, size, elem_size
                        ));
                    } else {
                        if !self.declared_functions.contains("a_new") {
                            self.global_buffer
                                .push_str("declare i64 @a_new() nounwind\n");
                            self.declared_functions.insert("a_new".to_string());
                        }
                        self.emit_line(&format!("{} = call i64 @a_new()", arr_tmp));
                    }
                }

                let mut idx = 0;
                for v in elements {
                    let mut v_val = self.resolve_value(v);

                    // Box primitives if storing into generic array
                    let v_ty = v.get_type();
                    if needs_boxing
                        && (v_ty.is_numeric()
                            || matches!(v_ty, TejxType::Bool | TejxType::Char | TejxType::String))
                    {
                        self.temp_counter += 1;
                        let boxed_reg = format!("%boxed_{}", self.temp_counter);

                        if v_ty.is_float() {
                            // v_val is i64 (bitcasted float). rt_box_number expects double.
                            self.temp_counter += 1;
                            let d_val = format!("%d_val_{}", self.temp_counter);
                            self.emit_line(&format!("{} = bitcast i64 {} to double", d_val, v_val));

                            if !self.declared_functions.contains("rt_box_number") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_number(double) readnone\n");
                                self.declared_functions.insert("rt_box_number".to_string());
                            }
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_number(double {})",
                                boxed_reg, d_val
                            ));
                        } else if matches!(v.get_type(), TejxType::Bool) {
                            if !self.declared_functions.contains("rt_box_boolean") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_boolean(i64)\n");
                                self.declared_functions.insert("rt_box_boolean".to_string());
                            }
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_boolean(i64 {})",
                                boxed_reg, v_val
                            ));
                        } else if matches!(v.get_type(), TejxType::String) {
                            if v_val.starts_with("ptrtoint") {
                                if !self.declared_functions.contains("rt_box_string") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_string(i64)\n");
                                    self.declared_functions.insert("rt_box_string".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_string(i64 {})",
                                    boxed_reg, v_val
                                ));
                            } else {
                                // Already an ID (variable), just move
                                self.emit_line(&format!("{} = add i64 {}, 0", boxed_reg, v_val));
                            }
                        } else {
                            // Int / Char (Int32, Int16 etc)
                            if !self.declared_functions.contains("rt_box_int") {
                                self.global_buffer
                                    .push_str("declare i64 @rt_box_int(i64)\n");
                                self.declared_functions.insert("rt_box_int".to_string());
                            }
                            self.emit_line(&format!(
                                "{} = call i64 @rt_box_int(i64 {})",
                                boxed_reg, v_val
                            ));
                        }
                        v_val = boxed_reg;
                    }

                    if can_stack_alloc {
                        // Write directly to stack array buffer (skip a_set)
                        // Data part starts at byte offset 16 (or i64 offset 2)
                        // Note: elem_size will be 1 or 8. If 1, we write i8, if 8, we write i64.
                        let elem_size = if let Some(TejxType::FixedArray(inner, _)) = &ty {
                            if matches!(**inner, TejxType::Bool) {
                                1
                            } else {
                                8
                            }
                        } else {
                            8
                        };

                        if elem_size == 1 {
                            self.temp_counter += 1;
                            let base_ptr8 = format!("%base_ptr8_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = inttoptr i64 {} to i8*",
                                base_ptr8, arr_tmp
                            ));
                            self.temp_counter += 1;
                            let elem_ptr = format!("%elem_ptr_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                                elem_ptr,
                                base_ptr8,
                                16 + idx
                            ));

                            // Convert value to i8
                            self.temp_counter += 1;
                            let v_byte = format!("%byte_val_{}", self.temp_counter);
                            if !self.declared_functions.contains("rt_i64_to_i8") {
                                self.global_buffer
                                    .push_str("declare i8 @rt_i64_to_i8(i64)\n");
                                self.declared_functions.insert("rt_i64_to_i8".to_string());
                            }
                            self.emit_line(&format!(
                                "{} = call i8 @rt_i64_to_i8(i64 {})",
                                v_byte, v_val
                            ));
                            self.emit_line(&format!("store i8 {}, i8* {}", v_byte, elem_ptr));
                        } else {
                            self.temp_counter += 1;
                            let base_ptr64 = format!("%base_ptr64_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = inttoptr i64 {} to i64*",
                                base_ptr64, arr_tmp
                            ));
                            self.temp_counter += 1;
                            let elem_ptr = format!("%elem_ptr_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = getelementptr inbounds i64, i64* {}, i64 {}",
                                elem_ptr,
                                base_ptr64,
                                2 + idx
                            ));
                            self.emit_line(&format!("store i64 {}, i64* {}", v_val, elem_ptr));
                        }
                    } else if use_fixed {
                        if !self.declared_functions.contains("rt_array_set_fast") {
                            self.global_buffer.push_str(
                                "declare i64 @rt_array_set_fast(i64, i64, i64) nounwind\n",
                            );
                            self.declared_functions
                                .insert("rt_array_set_fast".to_string());
                        }
                        let k_idx = self.resolve_value(&MIRValue::Constant {
                            value: idx.to_string(),
                            ty: TejxType::Int32,
                        });
                        self.emit_line(&format!(
                            "call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                            arr_tmp, k_idx, v_val
                        ));
                    } else {
                        if !self.declared_functions.contains("Array_push") {
                            self.global_buffer
                                .push_str("declare i64 @Array_push(i64, i64) nounwind\n");
                            self.declared_functions.insert("Array_push".to_string());
                        }
                        self.emit_line(&format!(
                            "call i64 @Array_push(i64 {}, i64 {})",
                            arr_tmp, v_val
                        ));
                    }

                    idx += 1;
                }
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", arr_tmp, ptr));

                // Track stack-allocated arrays for direct access in LoadIndex/StoreIndex
                if can_stack_alloc {
                    self.stack_arrays.insert(dst.to_string());
                }

                // Track heap-allocated FixedArrays with cached data pointers
                // Only for non-escaping arrays where the pointer remains valid
                let is_escaped = self.does_escape(func, dst);
                if use_fixed && !can_stack_alloc && size > 0 && !is_escaped {
                    // Declare rt_array_get_data_ptr if needed
                    if !self.declared_functions.contains("rt_array_get_data_ptr") {
                        self.global_buffer
                            .push_str("declare i64 @rt_array_get_data_ptr(i64) nounwind\n");
                        self.declared_functions
                            .insert("rt_array_get_data_ptr".to_string());
                    }

                    // Call rt_array_get_data_ptr to get raw data pointer
                    self.temp_counter += 1;
                    let data_ptr_val = format!("%heap_dp_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_array_get_data_ptr(i64 {})",
                        data_ptr_val, arr_tmp
                    ));

                    // Store in a local alloca
                    let alloca_name = format!(
                        "%heap_dp_alloca_{}",
                        dst.replace(".", "_").replace("#", "_")
                    );
                    self.alloca_buffer
                        .push_str(&format!("  {} = alloca i64, align 8\n", alloca_name));
                    self.emit_line(&format!("store i64 {}, i64* {}", data_ptr_val, alloca_name));

                    // Determine element size
                    let elem_size: i64 = if matches!(arr_elem_ty, TejxType::Bool) {
                        1
                    } else {
                        8
                    };

                    self.heap_array_ptrs
                        .insert(dst.to_string(), (alloca_name, elem_size));
                }
            }
            MIRInstruction::LoadMember {
                dst, obj, member, ..
            } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant {
                    value: format!("\"{}\"", member),
                    ty: TejxType::String,
                });
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);
                if !self.declared_functions.contains("rt_map_get_fast") {
                    self.global_buffer
                        .push_str("declare i64 @rt_map_get_fast(i64, i64)\n");
                    self.declared_functions
                        .insert("rt_map_get_fast".to_string());
                }
                self.emit_line(&format!(
                    "{} = call i64 @rt_map_get_fast(i64 {}, i64 {})",
                    res_tmp, obj_val, k_val
                ));

                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);
                let final_res = if dst_ty.is_numeric() && !dst_ty.is_float() {
                    // Expecting Int32/Int64: Unbox Any -> Double -> Int
                    if !self.declared_functions.contains("rt_to_number") {
                        self.global_buffer
                            .push_str("declare double @rt_to_number(i64)\n");
                        self.declared_functions.insert("rt_to_number".to_string());
                    }
                    self.temp_counter += 1;
                    let f_val = format!("%f_val_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call double @rt_to_number(i64 {})",
                        f_val, res_tmp
                    ));

                    self.temp_counter += 1;
                    let i_val = format!("%i_val_{}", self.temp_counter);
                    self.emit_line(&format!("{} = fptosi double {} to i64", i_val, f_val));
                    i_val
                } else if dst_ty.is_float() {
                    // Expecting Float: Unbox Any -> Double -> Bitcast to i64 (storage)
                    if !self.declared_functions.contains("rt_to_number") {
                        self.global_buffer
                            .push_str("declare double @rt_to_number(i64)\n");
                        self.declared_functions.insert("rt_to_number".to_string());
                    }
                    self.temp_counter += 1;
                    let f_val = format!("%f_val_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call double @rt_to_number(i64 {})",
                        f_val, res_tmp
                    ));

                    self.temp_counter += 1;
                    let bc_val = format!("%bc_val_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast double {} to i64", bc_val, f_val));
                    bc_val
                } else if matches!(dst_ty, TejxType::Bool) {
                    if !self.declared_functions.contains("rt_to_boolean") {
                        self.global_buffer
                            .push_str("declare i64 @rt_to_boolean(i64)\n");
                        self.declared_functions.insert("rt_to_boolean".to_string());
                    }
                    self.temp_counter += 1;
                    let b_val = format!("%b_val_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_boolean(i64 {})",
                        b_val, res_tmp
                    ));
                    b_val
                } else {
                    res_tmp
                };

                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
            }
            MIRInstruction::StoreMember {
                obj, member, src, ..
            } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant {
                    value: format!("\"{}\"", member),
                    ty: TejxType::String,
                });
                let mut v_val = self.resolve_value(src);

                // Box primitives if stored in object property (always 'Any' slot)
                let v_ty = src.get_type();
                if v_ty.is_numeric() || matches!(v_ty, TejxType::Bool | TejxType::Char) {
                    self.temp_counter += 1;
                    let boxed_reg = format!("%boxed_set_{}", self.temp_counter);

                    if v_ty.is_float() {
                        self.temp_counter += 1;
                        let d_val = format!("%d_val_set_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i64 {} to double", d_val, v_val));

                        if !self.declared_functions.contains("rt_box_number") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_number(double) readnone\n");
                            self.declared_functions.insert("rt_box_number".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_number(double {})",
                            boxed_reg, d_val
                        ));
                    } else if matches!(v_ty, TejxType::Bool) {
                        if !self.declared_functions.contains("rt_box_boolean") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_boolean(i64)\n");
                            self.declared_functions.insert("rt_box_boolean".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_boolean(i64 {})",
                            boxed_reg, v_val
                        ));
                    } else {
                        if !self.declared_functions.contains("rt_box_int") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_int(i64)\n");
                            self.declared_functions.insert("rt_box_int".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_int(i64 {})",
                            boxed_reg, v_val
                        ));
                    }
                    v_val = boxed_reg;
                }

                if !self.declared_functions.contains("rt_map_set_fast") {
                    self.global_buffer
                        .push_str("declare i64 @rt_map_set_fast(i64, i64, i64)\n");
                    self.declared_functions
                        .insert("rt_map_set_fast".to_string());
                }
                self.emit_line(&format!(
                    "call i64 @rt_map_set_fast(i64 {}, i64 {}, i64 {})",
                    obj_val, k_val, v_val
                ));
            }
            MIRInstruction::LoadIndex {
                dst, obj, index, ..
            } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);

                // --- STACK ARRAY DIRECT ACCESS (bypasses all cache checks) ---
                let obj_name = match obj {
                    MIRValue::Variable { name, .. } => Some(name.as_str()),
                    _ => None,
                };
                let is_stack_array = obj_name
                    .map(|n| self.stack_arrays.contains(n))
                    .unwrap_or(false);

                if is_stack_array {
                    // Stack arrays: base ptr IS the i64 value, data starts at offset 16 bytes
                    // Layout: [i64 length][i64 elem_size][data...]
                    let elem_type = obj.get_type().get_array_element_type();
                    let is_byte_elem = matches!(elem_type, TejxType::Bool);

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;
                    let sa_fast = format!("sa_fast_{}", label_id);
                    let sa_slow = format!("sa_slow_{}", label_id);
                    let sa_done = format!("sa_done_{}", label_id);

                    // Load length from stack header offset 0
                    self.temp_counter += 1;
                    let base_ptr = format!("%sa_base_{}", self.temp_counter);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i64*", base_ptr, obj_val));
                    self.temp_counter += 1;
                    let len_val = format!("%sa_len_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", len_val, base_ptr));

                    if self.unsafe_arrays {
                        // Unsafe mode: no bounds check, jump directly to fast path
                        self.emit_line(&format!("br label %{}", sa_fast));
                    } else {
                        // Bounds check: idx < length
                        self.temp_counter += 1;
                        let in_bounds = format!("%sa_inb_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = icmp ult i64 {}, {}",
                            in_bounds, idx_val, len_val
                        ));
                        self.emit_line(&format!(
                            "br i1 {}, label %{}, label %{}",
                            in_bounds, sa_fast, sa_slow
                        ));
                    }

                    // Fast path: direct GEP access
                    self.emit_line(&format!("{}:", sa_fast));
                    self.temp_counter += 1;
                    let base_i8 = format!("%sa_base8_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i64* {} to i8*", base_i8, base_ptr));
                    self.temp_counter += 1;
                    let data_ptr = format!("%sa_data_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr inbounds i8, i8* {}, i64 16",
                        data_ptr, base_i8
                    ));

                    if is_byte_elem {
                        // Byte element: i8 GEP
                        self.temp_counter += 1;
                        let elem_ptr = format!("%sa_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                            elem_ptr, data_ptr, idx_val
                        ));
                        self.temp_counter += 1;
                        let byte_val = format!("%sa_byte_{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i8, i8* {}", byte_val, elem_ptr));
                        self.emit_line(&format!("{} = zext i8 {} to i64", res_tmp, byte_val));
                    } else {
                        // i64 element: cast to i64* and GEP
                        self.temp_counter += 1;
                        let typed_ptr = format!("%sa_typed_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to i64*",
                            typed_ptr, data_ptr
                        ));
                        self.temp_counter += 1;
                        let elem_ptr = format!("%sa_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i64, i64* {}, i64 {}",
                            elem_ptr, typed_ptr, idx_val
                        ));
                        self.emit_line(&format!("{} = load i64, i64* {}", res_tmp, elem_ptr));
                    }

                    // Fast path result — branch to done
                    self.emit_line(&format!("br label %{}", sa_done));

                    // Slow path: call rt_array_get_fast (handles OOB error)
                    self.emit_line(&format!("{}:", sa_slow));
                    if !self.unsafe_arrays {
                        if !self.declared_functions.contains("rt_array_get_fast") {
                            self.global_buffer
                                .push_str("declare i64 @rt_array_get_fast(i64, i64) nounwind\n");
                            self.declared_functions
                                .insert("rt_array_get_fast".to_string());
                        }
                        self.temp_counter += 1;
                        let slow_res = format!("%sa_slow_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
                            slow_res, obj_val, idx_val
                        ));
                        self.emit_line(&format!("br label %{}", sa_done));

                        // Done: phi merge
                        self.emit_line(&format!("{}:", sa_done));
                        self.temp_counter += 1;
                        let final_res = format!("%sa_final_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = phi i64 [ {}, %{} ], [ {}, %{} ]",
                            final_res, res_tmp, sa_fast, slow_res, sa_slow
                        ));
                        // Store final result
                        let _dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
                    } else {
                        // If unsafe, slow path shouldn't be reached, so just unreachable
                        self.emit_line("unreachable");
                        self.emit_line(&format!("{}:", sa_done));
                        let _dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
                    }

                    return;
                }

                // --- HEAP ARRAY DIRECT ACCESS (cached data pointer from rt_array_get_data_ptr) ---
                let heap_info = obj_name.and_then(|n| self.heap_array_ptrs.get(n).cloned());
                if let Some((data_ptr_alloca, elem_size)) = heap_info {
                    // Load data pointer from alloca
                    self.temp_counter += 1;
                    let dp_raw = format!("%ha_dp_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", dp_raw, data_ptr_alloca));
                    self.temp_counter += 1;
                    let dp_ptr = format!("%ha_ptr_{}", self.temp_counter);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i8*", dp_ptr, dp_raw));

                    if elem_size == 1 {
                        // Byte element: i8 GEP
                        self.temp_counter += 1;
                        let elem_ptr = format!("%ha_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                            elem_ptr, dp_ptr, idx_val
                        ));
                        self.temp_counter += 1;
                        let byte_val = format!("%ha_byte_{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i8, i8* {}", byte_val, elem_ptr));
                        self.emit_line(&format!("{} = zext i8 {} to i64", res_tmp, byte_val));
                    } else {
                        // i64 element: cast to i64* and GEP
                        self.temp_counter += 1;
                        let typed_ptr = format!("%ha_typed_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i8* {} to i64*", typed_ptr, dp_ptr));
                        self.temp_counter += 1;
                        let elem_ptr = format!("%ha_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i64, i64* {}, i64 {}",
                            elem_ptr, typed_ptr, idx_val
                        ));
                        self.emit_line(&format!("{} = load i64, i64* {}", res_tmp, elem_ptr));
                    }

                    // Store result
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);
                    let ptr = self.resolve_ptr(dst);

                    if dst_ty.is_float() && elem_size != 1 {
                        self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
                    } else if dst_ty.is_float() && elem_size == 1 {
                        self.temp_counter += 1;
                        let f_val = format!("%ha_f_{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", f_val, res_tmp));
                        self.temp_counter += 1;
                        let f_bc = format!("%ha_fbc_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast double {} to i64", f_bc, f_val));
                        self.emit_line(&format!("store i64 {}, i64* {}", f_bc, ptr));
                    } else {
                        self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
                    }

                    return;
                }

                let known_byte_array = if let TejxType::Class(name) = obj.get_type() {
                    name == "ByteArray"
                } else {
                    false
                };

                if known_byte_array {
                    // ===== SPECIALIZED BYTE ARRAY FAST PATH =====
                    // Unconditional direct byte GEP via cached @LAST_PTR.
                    // Safe because fill() populates the cache before any loop access.
                    if !self.declared_functions.contains("LAST_ID") {
                        self.global_buffer
                            .push_str("@LAST_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@LAST_LEN = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_ELEM_SIZE = external global i64\n");
                        self.declared_functions.insert("LAST_ID".to_string());
                    }

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;

                    let idx_norm = idx_val.clone();

                    // --- Cache check ---
                    self.temp_counter += 1;
                    let last_id = format!("%ba_last_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ID", last_id));
                    self.temp_counter += 1;
                    let id_match = format!("%ba_id_match{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        id_match, last_id, obj_val
                    ));

                    let fast_path = format!("ba_get_fast{}", label_id);
                    let slow_path = format!("ba_get_slow{}", label_id);
                    let done_path = format!("ba_get_done{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        id_match, fast_path, slow_path
                    ));

                    // --- Fast path: direct byte GEP ---
                    self.emit_line(&format!("{}:", fast_path));
                    self.temp_counter += 1;
                    let ptr = format!("%ba_ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @LAST_PTR", ptr));
                    self.temp_counter += 1;
                    let gep = format!("%ba_gep{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                        gep, ptr, idx_norm
                    ));
                    self.temp_counter += 1;
                    let byte_val = format!("%ba_byte{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8, i8* {}", byte_val, gep));
                    self.temp_counter += 1;
                    let fast_res = format!("%ba_fast_res{}", self.temp_counter);
                    self.emit_line(&format!("{} = zext i8 {} to i64", fast_res, byte_val));
                    self.emit_line(&format!("br label %{}", done_path));

                    // --- Slow path: runtime call (populates cache) ---
                    self.emit_line(&format!("{}:", slow_path));
                    if !self.declared_functions.contains("rt_array_get_fast") {
                        self.global_buffer
                            .push_str("declare i64 @rt_array_get_fast(i64, i64) nounwind\n");
                        self.declared_functions
                            .insert("rt_array_get_fast".to_string());
                    }
                    self.temp_counter += 1;
                    let slow_res = format!("%ba_slow_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
                        slow_res, obj_val, idx_val
                    ));
                    self.emit_line(&format!("br label %{}", done_path));

                    // --- Merge ---
                    self.emit_line(&format!("{}:", done_path));
                    self.emit_line(&format!(
                        "{} = phi i64 [ {}, %{} ], [ {}, %{} ]",
                        res_tmp, fast_res, fast_path, slow_res, slow_path
                    ));

                    // --- Store based on destination type ---
                    let ptr = self.resolve_ptr(dst);
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);

                    if matches!(dst_ty, TejxType::Any) {
                        if !self.declared_functions.contains("rt_box_boolean") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_boolean(i64)\n");
                            self.declared_functions.insert("rt_box_boolean".to_string());
                        }
                        self.temp_counter += 1;
                        let boxed = format!("%ba_boxed{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_boolean(i64 {})",
                            boxed, res_tmp
                        ));
                        self.emit_line(&format!("store i64 {}, i64* {}", boxed, ptr));
                    } else if dst_ty.is_float() {
                        self.temp_counter += 1;
                        let f_val = format!("%ba_f{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", f_val, res_tmp));
                        self.temp_counter += 1;
                        let f_bc = format!("%ba_fbc{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast double {} to i64", f_bc, f_val));
                        self.emit_line(&format!("store i64 {}, i64* {}", f_bc, ptr));
                    } else {
                        // Int or Bool — store raw 0/1
                        self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
                    }
                } else if obj.get_type().is_array() {
                    // --- GENERIC ARRAY OPTIMIZATION: INLINED CACHE CHECK ---
                    if !self.declared_functions.contains("LAST_ID") {
                        self.global_buffer
                            .push_str("@LAST_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@LAST_LEN = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_ELEM_SIZE = external global i64\n");
                        self.declared_functions.insert("LAST_ID".to_string());
                    }
                    if !self.declared_functions.contains("PREV_ID") {
                        self.global_buffer
                            .push_str("@PREV_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@PREV_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@PREV_LEN = external global i64\n");
                        self.declared_functions.insert("PREV_ID".to_string());
                    }
                    if !self.declared_functions.contains("PREV2_ID") {
                        self.global_buffer
                            .push_str("@PREV2_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@PREV2_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@PREV2_LEN = external global i64\n");
                        self.declared_functions.insert("PREV2_ID".to_string());
                    }
                    if !self.declared_functions.contains("rt_array_get_fast") {
                        self.global_buffer
                            .push_str("declare i64 @rt_array_get_fast(i64, i64) nounwind\n");
                        self.declared_functions
                            .insert("rt_array_get_fast".to_string());
                    }

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;

                    self.temp_counter += 1;
                    let last_id = format!("%last_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ID", last_id));

                    self.temp_counter += 1;
                    let id_match = format!("%id_match{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        id_match, last_id, obj_val
                    ));

                    let fast_path = format!("array_get_fast{}", label_id);
                    let prev_check = format!("array_get_prev{}", label_id);
                    let slow_path = format!("array_get_slow{}", label_id);
                    let done_path = format!("array_get_done{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        id_match, fast_path, prev_check
                    ));

                    self.emit_line(&format!("{}:", fast_path));
                    self.temp_counter += 1;
                    let last_len = format!("%last_len{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_LEN", last_len));
                    self.temp_counter += 1;
                    let in_bounds = format!("%in_bounds{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ult i64 {}, {}",
                        in_bounds, idx_val, last_len
                    ));

                    let fast_access = format!("array_get_access{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        in_bounds, fast_access, slow_path
                    ));

                    self.emit_line(&format!("{}:", fast_access));
                    self.temp_counter += 1;
                    let ptr = format!("%ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @LAST_PTR", ptr));

                    let elem_type = obj.get_type().get_array_element_type();
                    let is_known_qword = elem_type.is_numeric()
                        || elem_type.is_float()
                        || matches!(
                            elem_type,
                            TejxType::Any | TejxType::String | TejxType::Bool | TejxType::Char
                        );

                    let get_qword = format!("array_get_qword{}", label_id);
                    let get_byte = format!("array_get_byte{}", label_id);
                    // Sentinel values for byte-path registers (only valid when !is_known_qword)
                    let mut res8 = String::new();
                    let mut res8_boxed = String::new();
                    let mut res8_bc = String::new();

                    if is_known_qword {
                        // Skip LAST_ELEM_SIZE check — we know this is always 8-byte elements
                        self.emit_line(&format!("br label %{}", get_qword));
                    } else {
                        self.temp_counter += 1;
                        let elem_size = format!("%elem_size{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i64, i64* @LAST_ELEM_SIZE", elem_size));
                        self.temp_counter += 1;
                        let is_byte = format!("%is_byte{}", self.temp_counter);
                        self.emit_line(&format!("{} = icmp eq i64 {}, 1", is_byte, elem_size));

                        self.emit_line(&format!(
                            "br i1 {}, label %{}, label %{}",
                            is_byte, get_byte, get_qword
                        ));

                        self.emit_line(&format!("{}:", get_byte));
                        self.temp_counter += 1;
                        let gep8 = format!("%gep8_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i8, i8* {}, i64 {}",
                            gep8, ptr, idx_val
                        ));
                        self.temp_counter += 1;
                        let val8 = format!("%val8_{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i8, i8* {}", val8, gep8));
                        self.temp_counter += 1;
                        res8 = format!("%res8_{}", self.temp_counter);
                        self.emit_line(&format!("{} = zext i8 {} to i64", res8, val8));

                        if !self.declared_functions.contains("rt_box_boolean") {
                            self.global_buffer
                                .push_str("declare i64 @rt_box_boolean(i64)\n");
                            self.declared_functions.insert("rt_box_boolean".to_string());
                        }
                        self.temp_counter += 1;
                        res8_boxed = format!("%res8_boxed{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_box_boolean(i64 {})",
                            res8_boxed, res8
                        ));

                        self.temp_counter += 1;
                        let res8_f = format!("%res8_f{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", res8_f, res8));
                        self.temp_counter += 1;
                        res8_bc = format!("%res8_bc{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast double {} to i64", res8_bc, res8_f));

                        self.emit_line(&format!("br label %{}", done_path));
                    }

                    self.emit_line(&format!("{}:", get_qword));
                    self.temp_counter += 1;
                    let ptr64 = format!("%ptr64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i8* {} to i64*", ptr64, ptr));
                    self.temp_counter += 1;
                    let gep64 = format!("%gep64_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr i64, i64* {}, i64 {}",
                        gep64, ptr64, idx_val
                    ));
                    self.temp_counter += 1;
                    let res64 = format!("%res64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", res64, gep64));

                    self.temp_counter += 1;
                    let elem_type = obj.get_type().get_array_element_type();
                    let is_numeric_elem = elem_type.is_numeric()
                        || matches!(elem_type, TejxType::Bool | TejxType::Char);
                    let is_int_elem = is_numeric_elem && !elem_type.is_float();

                    // For integer types: use raw i64 directly (skip sitofp/fptosi round-trip)
                    // For float types: bitcast i64 to double
                    // For Any/String: call rt_to_number
                    let res64_f = format!("%res64_f{}", self.temp_counter);
                    let mut res64_raw = res64.clone(); // Default for int types
                    let res64_f_bc;

                    if is_int_elem {
                        // Integer/bool/char: raw i64 IS the value — no conversion needed
                        // Still need a dummy res64_f_bc for Any-destination phi paths
                        self.temp_counter += 1;
                        res64_f_bc = format!("%res64_f_bc{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", res64_f, res64));
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            res64_f_bc, res64_f
                        ));
                    } else if elem_type.is_float() {
                        self.emit_line(&format!("{} = bitcast i64 {} to double", res64_f, res64));
                        self.temp_counter += 1;
                        res64_raw = format!("%res64_raw{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = fptosi double {} to i64",
                            res64_raw, res64_f
                        ));
                        self.temp_counter += 1;
                        res64_f_bc = format!("%res64_f_bc{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            res64_f_bc, res64_f
                        ));
                    } else {
                        res64_f_bc = res64.clone();
                    }

                    self.emit_line(&format!("br label %{}", done_path));

                    // --- PREV cache check (inline second slot) ---
                    self.emit_line(&format!("{}:", prev_check));
                    self.temp_counter += 1;
                    let prev_id = format!("%prev_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV_ID", prev_id));
                    self.temp_counter += 1;
                    let prev_match = format!("%prev_match{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        prev_match, prev_id, obj_val
                    ));
                    let prev_fast = format!("array_get_prev_fast{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev_match, prev_fast, slow_path
                    ));

                    self.emit_line(&format!("{}:", prev_fast));
                    self.temp_counter += 1;
                    let prev_len = format!("%prev_len{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV_LEN", prev_len));
                    self.temp_counter += 1;
                    let prev_bounds = format!("%prev_bounds{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ult i64 {}, {}",
                        prev_bounds, idx_val, prev_len
                    ));
                    let prev_access = format!("array_get_prev_access{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev_bounds, prev_access, slow_path
                    ));

                    self.emit_line(&format!("{}:", prev_access));
                    self.temp_counter += 1;
                    let prev_ptr = format!("%prev_ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @PREV_PTR", prev_ptr));
                    self.temp_counter += 1;
                    let prev_ptr64 = format!("%prev_ptr64_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast i8* {} to i64*",
                        prev_ptr64, prev_ptr
                    ));
                    self.temp_counter += 1;
                    let prev_gep = format!("%prev_gep_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr i64, i64* {}, i64 {}",
                        prev_gep, prev_ptr64, idx_val
                    ));
                    self.temp_counter += 1;
                    let prev_val = format!("%prev_val_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", prev_val, prev_gep));

                    let mut prev_raw = prev_val.clone(); // Default for int types
                    let prev_f_bc;
                    if is_int_elem {
                        self.temp_counter += 1;
                        let prev_f = format!("%prev_f_{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", prev_f, prev_val));
                        self.temp_counter += 1;
                        prev_f_bc = format!("%prev_f_bc_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            prev_f_bc, prev_f
                        ));
                    } else if elem_type.is_float() {
                        self.temp_counter += 1;
                        let prev_f = format!("%prev_f_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i64 {} to double", prev_f, prev_val));
                        self.temp_counter += 1;
                        prev_raw = format!("%prev_raw_{}", self.temp_counter);
                        self.emit_line(&format!("{} = fptosi double {} to i64", prev_raw, prev_f));
                        self.temp_counter += 1;
                        prev_f_bc = format!("%prev_f_bc_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            prev_f_bc, prev_f
                        ));
                    } else {
                        prev_f_bc = prev_val.clone();
                    }
                    self.emit_line(&format!("br label %{}", done_path));

                    // --- PREV2 cache check ---
                    let prev2_check = format!("array_get_prev2{}", label_id);
                    self.emit_line(&format!("{}:", prev2_check));
                    self.temp_counter += 1;
                    let prev2_id = format!("%prev2_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV2_ID", prev2_id));

                    self.temp_counter += 1;
                    let prev2_match = format!("%prev2_match{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        prev2_match, prev2_id, obj_val
                    ));

                    let prev2_fast = format!("array_get_prev2_fast{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev2_match, prev2_fast, slow_path
                    ));

                    self.emit_line(&format!("{}:", prev2_fast));
                    self.temp_counter += 1;
                    let prev2_len = format!("%prev2_len{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV2_LEN", prev2_len));

                    self.temp_counter += 1;
                    let prev2_in_bounds = format!("%prev2_bounds{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ult i64 {}, {}",
                        prev2_in_bounds, idx_val, prev2_len
                    ));

                    let prev2_access = format!("array_get_prev2_access{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev2_in_bounds, prev2_access, slow_path
                    ));

                    self.emit_line(&format!("{}:", prev2_access));
                    self.temp_counter += 1;
                    let prev2_ptr = format!("%prev2_ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @PREV2_PTR", prev2_ptr));

                    let prev2_val;
                    let mut prev2_raw = String::new();
                    let prev2_f_bc;

                    let mut _prev2_val_raw = String::new();
                    let mut prev2_byte = String::new();

                    if is_known_qword {
                        self.temp_counter += 1;
                        let prev2_ptr64 = format!("%prev2_ptr64{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to i64*",
                            prev2_ptr64, prev2_ptr
                        ));
                        self.temp_counter += 1;
                        let prev2_gep = format!("%prev2_gep{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i64, i64* {}, i64 {}",
                            prev2_gep, prev2_ptr64, idx_val
                        ));
                        self.temp_counter += 1;
                        prev2_val = format!("%prev2_val{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i64, i64* {}", prev2_val, prev2_gep));
                    } else {
                        let prev2_val_raw;
                        self.temp_counter += 1;
                        let prev2_gep8 = format!("%prev2_gep8{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i8, i8* {}, i64 {}",
                            prev2_gep8, prev2_ptr, idx_val
                        ));
                        self.temp_counter += 1;
                        let loaded2_byte = format!("%prev2_loaded8{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i8, i8* {}", loaded2_byte, prev2_gep8));

                        self.temp_counter += 1;
                        let zext2 = format!("%prev2_zext{}", self.temp_counter);
                        self.emit_line(&format!("{} = zext i8 {} to i64", zext2, loaded2_byte));
                        prev2_byte = zext2.clone();

                        // For non-known, we read i64 via bitcast check same as LAST/PREV
                        self.temp_counter += 1;
                        let prev2_ptr64 = format!("%prev2_ptr64_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to i64*",
                            prev2_ptr64, prev2_ptr
                        ));
                        self.temp_counter += 1;
                        let prev2_gep64 = format!("%prev2_gep64_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i64, i64* {}, i64 {}",
                            prev2_gep64, prev2_ptr64, idx_val
                        ));
                        self.temp_counter += 1;
                        prev2_val_raw = format!("%prev2_val_raw{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = load i64, i64* {}",
                            prev2_val_raw, prev2_gep64
                        ));
                        prev2_val = prev2_val_raw;
                    }

                    if is_int_elem {
                        self.temp_counter += 1;
                        let prev2_f = format!("%prev2_f_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = sitofp i64 {} to double",
                            prev2_f, prev2_val
                        ));
                        prev2_raw = prev2_val.clone();
                        self.temp_counter += 1;
                        prev2_f_bc = format!("%prev2_f_bc_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            prev2_f_bc, prev2_f
                        ));
                    } else if elem_type.is_float() {
                        self.temp_counter += 1;
                        let prev2_f = format!("%prev2_f_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i64 {} to double",
                            prev2_f, prev2_val
                        ));
                        self.temp_counter += 1;
                        prev2_raw = format!("%prev2_raw_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = fptosi double {} to i64",
                            prev2_raw, prev2_f
                        ));
                        self.temp_counter += 1;
                        prev2_f_bc = format!("%prev2_f_bc_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            prev2_f_bc, prev2_f
                        ));
                    } else {
                        prev2_f_bc = prev2_val.clone();
                    }
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", slow_path));
                    self.temp_counter += 1;
                    let slow_res = format!("%slow_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
                        slow_res, obj_val, idx_val
                    ));

                    // Same optimization for slow path
                    self.temp_counter += 1;
                    let slow_f = format!("%slow_f{}", self.temp_counter);
                    let mut slow_raw = slow_res.clone(); // Default: use raw for int types
                    let slow_f_bc;

                    if is_int_elem {
                        // Integer: raw i64 from rt_array_get_fast IS the value
                        self.emit_line(&format!("{} = sitofp i64 {} to double", slow_f, slow_res));
                        self.temp_counter += 1;
                        slow_f_bc = format!("%slow_f_bc{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            slow_f_bc, slow_f
                        ));
                    } else if elem_type.is_float() {
                        self.emit_line(&format!("{} = bitcast i64 {} to double", slow_f, slow_res));
                        self.temp_counter += 1;
                        slow_raw = format!("%slow_raw{}", self.temp_counter);
                        self.emit_line(&format!("{} = fptosi double {} to i64", slow_raw, slow_f));
                        self.temp_counter += 1;
                        slow_f_bc = format!("%slow_f_bc{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            slow_f_bc, slow_f
                        ));
                    } else {
                        slow_f_bc = slow_res.clone();
                    }

                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", done_path));

                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Any);

                    // Raw integer or Boolean destination
                    if (dst_ty.is_numeric() && !dst_ty.is_float())
                        || matches!(dst_ty, TejxType::Bool)
                    {
                        self.temp_counter += 1;
                        let final_res = format!("%final_res{}", self.temp_counter);
                        if is_known_qword {
                            self.emit_line(&format!(
                                "{} = phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]",
                                final_res,
                                res64_raw,
                                get_qword,
                                prev_raw,
                                prev_access,
                                prev2_raw,
                                prev2_access,
                                slow_raw,
                                slow_path
                            ));
                        } else {
                            let get_byte = format!("array_get_byte{}", label_id);
                            self.emit_line(&format!("{} = phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]", 
                                 final_res, res8, get_byte, res64_raw, get_qword, prev_raw, prev_access, prev2_raw, prev2_access, slow_raw, slow_path));
                        }
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
                    } else {
                        // Destination is Any, Float, or Object.
                        let elem_type = obj.get_type().get_array_element_type();
                        // Treat ByteArray as numeric (0/1) for conversion purposes
                        let is_byte_array = if let TejxType::Class(name) = obj.get_type() {
                            name == "ByteArray"
                        } else {
                            false
                        };
                        let is_numeric_elem = elem_type.is_numeric() || is_byte_array;

                        if is_known_qword {
                            // No byte path exists — 4-way phi (LAST + PREV + PREV2 + slow)
                            let (final_qword, final_prev, final_prev2, final_slow) =
                                if is_numeric_elem {
                                    (
                                        res64_f_bc.clone(),
                                        prev_f_bc.clone(),
                                        prev2_f_bc.clone(),
                                        slow_f_bc.clone(),
                                    )
                                } else {
                                    (
                                        res64.clone(),
                                        prev_val.clone(),
                                        prev2_val.clone(),
                                        slow_res.clone(),
                                    )
                                };

                            self.temp_counter += 1;
                            let final_res = format!("%final_res{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]",
                                final_res,
                                final_qword,
                                get_qword,
                                final_prev,
                                prev_access,
                                final_prev2,
                                prev2_access,
                                final_slow,
                                slow_path
                            ));
                            let ptr = self.resolve_ptr(dst);
                            self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
                        } else {
                            let get_byte = format!("array_get_byte{}", label_id);
                            let (final_qword, final_byte, final_prev, final_prev2, final_slow) =
                                if is_numeric_elem {
                                    if is_byte_array && matches!(dst_ty, TejxType::Any) {
                                        self.temp_counter += 1;
                                        let slow_boxed =
                                            format!("%slow_boxed{}", self.temp_counter);
                                        self.emit_line(&format!(
                                            "{} = call i64 @rt_box_boolean(i64 {})",
                                            slow_boxed, slow_res
                                        ));
                                        self.temp_counter += 1;
                                        let prev2_boxed =
                                            format!("%prev2_boxed{}", self.temp_counter);
                                        self.emit_line(&format!(
                                            "{} = call i64 @rt_box_boolean(i64 {})",
                                            prev2_boxed, prev2_byte
                                        ));
                                        (
                                            res64.clone(),
                                            res8_boxed,
                                            prev_val.clone(),
                                            prev2_boxed,
                                            slow_boxed,
                                        )
                                    } else if is_byte_array {
                                        self.temp_counter += 1;
                                        let slow_f2 = format!("%slow_f2_{}", self.temp_counter);
                                        self.emit_line(&format!(
                                            "{} = sitofp i64 {} to double",
                                            slow_f2, slow_res
                                        ));
                                        self.temp_counter += 1;
                                        let slow_f_bc2 =
                                            format!("%slow_f_bc2_{}", self.temp_counter);
                                        self.emit_line(&format!(
                                            "{} = bitcast double {} to i64",
                                            slow_f_bc2, slow_f2
                                        ));

                                        self.temp_counter += 1;
                                        let prev2_f2 = format!("%prev2_f2_{}", self.temp_counter);
                                        self.emit_line(&format!(
                                            "{} = sitofp i64 {} to double",
                                            prev2_f2, prev2_byte
                                        ));
                                        self.temp_counter += 1;
                                        let prev2_f_bc2 =
                                            format!("%prev2_f_bc2_{}", self.temp_counter);
                                        self.emit_line(&format!(
                                            "{} = bitcast double {} to i64",
                                            prev2_f_bc2, prev2_f2
                                        ));

                                        (
                                            res64_f_bc.clone(),
                                            res8_bc,
                                            prev_f_bc.clone(),
                                            prev2_f_bc2,
                                            slow_f_bc2,
                                        )
                                    } else {
                                        (
                                            res64_f_bc.clone(),
                                            res8_bc.clone(),
                                            prev_f_bc.clone(),
                                            prev2_f_bc.clone(),
                                            slow_f_bc.clone(),
                                        )
                                    }
                                } else {
                                    (
                                        res64.clone(),
                                        res8_boxed.clone(),
                                        prev_val.clone(),
                                        prev2_val.clone(),
                                        slow_res.clone(),
                                    )
                                };

                            self.temp_counter += 1;
                            let final_res = format!("%final_res{}", self.temp_counter);
                            self.emit_line(&format!("{} = phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]", 
                                 final_res, final_byte, get_byte, final_qword, get_qword, final_prev, prev_access, final_prev2, prev2_access, final_slow, slow_path));
                            let ptr = self.resolve_ptr(dst);
                            self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
                        }
                    }
                } else {
                    // When obj type is Any but index is numeric, it's likely an array
                    // accessed through a class field (LoadMember returns Any).
                    // Use rt_array_get_fast which handles arrays correctly at runtime.
                    let idx_ty = index.get_type();
                    if idx_ty.is_numeric() && !idx_ty.is_float() {
                        if !self.declared_functions.contains("rt_array_get_fast") {
                            self.global_buffer
                                .push_str("declare i64 @rt_array_get_fast(i64, i64) nounwind\n");
                            self.declared_functions
                                .insert("rt_array_get_fast".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
                            res_tmp, obj_val, idx_val
                        ));
                    } else {
                        if !self.declared_functions.contains("rt_map_get_fast") {
                            self.global_buffer
                                .push_str("declare i64 @rt_map_get_fast(i64, i64)\n");
                            self.declared_functions
                                .insert("rt_map_get_fast".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i64 @rt_map_get_fast(i64 {}, i64 {})",
                            res_tmp, obj_val, idx_val
                        ));
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
                }
            }
            MIRInstruction::StoreIndex {
                obj, index, src, ..
            } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                let v_val = self.resolve_value(src);

                // --- STACK ARRAY DIRECT STORE (bypasses all cache checks) ---
                let obj_name = match obj {
                    MIRValue::Variable { name, .. } => Some(name.as_str()),
                    _ => None,
                };
                let is_stack_array = obj_name
                    .map(|n| self.stack_arrays.contains(n))
                    .unwrap_or(false);

                if is_stack_array {
                    let elem_type = obj.get_type().get_array_element_type();
                    let is_byte_elem = matches!(elem_type, TejxType::Bool);

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;
                    let ss_fast = format!("ss_fast_{}", label_id);
                    let ss_slow = format!("ss_slow_{}", label_id);
                    let ss_done = format!("ss_done_{}", label_id);

                    // Load length from stack header offset 0
                    self.temp_counter += 1;
                    let base_ptr = format!("%ss_base_{}", self.temp_counter);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i64*", base_ptr, obj_val));
                    self.temp_counter += 1;
                    let len_val = format!("%ss_len_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", len_val, base_ptr));

                    if self.unsafe_arrays {
                        // Unsafe mode: no bounds check, jump directly to fast path
                        self.emit_line(&format!("br label %{}", ss_fast));
                    } else {
                        // Bounds check
                        self.temp_counter += 1;
                        let in_bounds = format!("%ss_inb_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = icmp ult i64 {}, {}",
                            in_bounds, idx_val, len_val
                        ));
                        self.emit_line(&format!(
                            "br i1 {}, label %{}, label %{}",
                            in_bounds, ss_fast, ss_slow
                        ));
                    }

                    // Fast path
                    self.emit_line(&format!("{}:", ss_fast));
                    self.temp_counter += 1;
                    let base_i8 = format!("%ss_base8_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i64* {} to i8*", base_i8, base_ptr));
                    self.temp_counter += 1;
                    let data_ptr = format!("%ss_data_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr inbounds i8, i8* {}, i64 16",
                        data_ptr, base_i8
                    ));

                    if is_byte_elem {
                        self.temp_counter += 1;
                        let elem_ptr = format!("%ss_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                            elem_ptr, data_ptr, idx_val
                        ));
                        self.temp_counter += 1;
                        let v_byte = format!("%ss_byte_{}", self.temp_counter);
                        self.emit_line(&format!("{} = trunc i64 {} to i8", v_byte, v_val));
                        self.emit_line(&format!("store i8 {}, i8* {}", v_byte, elem_ptr));
                    } else {
                        self.temp_counter += 1;
                        let typed_ptr = format!("%ss_typed_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to i64*",
                            typed_ptr, data_ptr
                        ));
                        self.temp_counter += 1;
                        let elem_ptr = format!("%ss_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i64, i64* {}, i64 {}",
                            elem_ptr, typed_ptr, idx_val
                        ));
                        self.emit_line(&format!("store i64 {}, i64* {}", v_val, elem_ptr));
                    }
                    self.emit_line(&format!("br label %{}", ss_done));

                    // Slow path: call rt_array_set_fast (handles OOB error)
                    self.emit_line(&format!("{}:", ss_slow));
                    if !self.unsafe_arrays {
                        if !self.declared_functions.contains("rt_array_set_fast") {
                            self.global_buffer.push_str(
                                "declare i64 @rt_array_set_fast(i64, i64, i64) nounwind\n",
                            );
                            self.declared_functions
                                .insert("rt_array_set_fast".to_string());
                        }
                        self.emit_line(&format!(
                            "call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                            obj_val, idx_val, v_val
                        ));
                        self.emit_line(&format!("br label %{}", ss_done));

                        // Done
                        self.emit_line(&format!("{}:", ss_done));
                    } else {
                        // If unsafe, slow path shouldn't be reached
                        self.emit_line("unreachable");
                        self.emit_line(&format!("{}:", ss_done));
                    }

                    return;
                }

                // --- HEAP ARRAY DIRECT STORE (cached data pointer) ---
                let heap_info = obj_name.and_then(|n| self.heap_array_ptrs.get(n).cloned());
                if let Some((data_ptr_alloca, elem_size)) = heap_info {
                    self.temp_counter += 1;
                    let dp_raw = format!("%hs_dp_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", dp_raw, data_ptr_alloca));
                    self.temp_counter += 1;
                    let dp_ptr = format!("%hs_ptr_{}", self.temp_counter);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i8*", dp_ptr, dp_raw));

                    if elem_size == 1 {
                        self.temp_counter += 1;
                        let elem_ptr = format!("%hs_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                            elem_ptr, dp_ptr, idx_val
                        ));
                        self.temp_counter += 1;
                        let v_byte = format!("%hs_byte_{}", self.temp_counter);
                        self.emit_line(&format!("{} = trunc i64 {} to i8", v_byte, v_val));
                        self.emit_line(&format!("store i8 {}, i8* {}", v_byte, elem_ptr));
                    } else {
                        self.temp_counter += 1;
                        let typed_ptr = format!("%hs_typed_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i8* {} to i64*", typed_ptr, dp_ptr));
                        self.temp_counter += 1;
                        let elem_ptr = format!("%hs_elem_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr inbounds i64, i64* {}, i64 {}",
                            elem_ptr, typed_ptr, idx_val
                        ));
                        self.emit_line(&format!("store i64 {}, i64* {}", v_val, elem_ptr));
                    }
                    return;
                }

                let known_byte_array = if let TejxType::Class(name) = obj.get_type() {
                    name == "ByteArray"
                } else {
                    false
                };

                if known_byte_array {
                    // ===== SPECIALIZED BYTE ARRAY STORE =====
                    if !self.declared_functions.contains("LAST_ID") {
                        self.global_buffer
                            .push_str("@LAST_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@LAST_LEN = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_ELEM_SIZE = external global i64\n");
                        self.declared_functions.insert("LAST_ID".to_string());
                    }
                    if !self.declared_functions.contains("rt_array_set_fast") {
                        self.global_buffer
                            .push_str("declare i64 @rt_array_set_fast(i64, i64, i64) nounwind\n");
                        self.declared_functions
                            .insert("rt_array_set_fast".to_string());
                    }

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;

                    let idx_norm = idx_val.clone();

                    // --- Cache check ---
                    self.temp_counter += 1;
                    let last_id = format!("%ba_s_last_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ID", last_id));
                    self.temp_counter += 1;
                    let id_match = format!("%ba_s_id_match{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        id_match, last_id, obj_val
                    ));

                    let fast_path = format!("ba_set_fast{}", label_id);
                    let slow_path = format!("ba_set_slow{}", label_id);
                    let done_path = format!("ba_set_done{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        id_match, fast_path, slow_path
                    ));

                    // --- Fast path: direct byte store ---
                    self.emit_line(&format!("{}:", fast_path));
                    self.temp_counter += 1;
                    let ptr = format!("%ba_s_ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @LAST_PTR", ptr));
                    self.temp_counter += 1;
                    let gep = format!("%ba_s_gep{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr inbounds i8, i8* {}, i64 {}",
                        gep, ptr, idx_norm
                    ));

                    // Value conversion to i8
                    let src_ty = src.get_type();
                    self.temp_counter += 1;
                    let v_byte = format!("%ba_s_byte{}", self.temp_counter);
                    if matches!(src_ty, TejxType::Bool)
                        || (src_ty.is_numeric() && !src_ty.is_float())
                    {
                        if !self.declared_functions.contains("rt_i64_to_i8") {
                            self.global_buffer
                                .push_str("declare i8 @rt_i64_to_i8(i64)\n");
                            self.declared_functions.insert("rt_i64_to_i8".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i8 @rt_i64_to_i8(i64 {})",
                            v_byte, v_val
                        ));
                    } else if src_ty.is_float() || matches!(src_ty, TejxType::Any) {
                        self.temp_counter += 1;
                        let v_f = format!("%ba_s_vf{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i64 {} to double", v_f, v_val));
                        if !self.declared_functions.contains("rt_f64_to_i8") {
                            self.global_buffer
                                .push_str("declare i8 @rt_f64_to_i8(double)\n");
                            self.declared_functions.insert("rt_f64_to_i8".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i8 @rt_f64_to_i8(double {})",
                            v_byte, v_f
                        ));
                    } else {
                        if !self.declared_functions.contains("rt_i64_to_i8") {
                            self.global_buffer
                                .push_str("declare i8 @rt_i64_to_i8(i64)\n");
                            self.declared_functions.insert("rt_i64_to_i8".to_string());
                        }
                        self.emit_line(&format!(
                            "{} = call i8 @rt_i64_to_i8(i64 {})",
                            v_byte, v_val
                        ));
                    }

                    self.emit_line(&format!("store i8 {}, i8* {}", v_byte, gep));
                    self.emit_line(&format!("br label %{}", done_path));

                    // --- Slow path ---
                    self.emit_line(&format!("{}:", slow_path));
                    if !self.declared_functions.contains("rt_array_set_fast") {
                        self.global_buffer
                            .push_str("declare i64 @rt_array_set_fast(i64, i64, i64) nounwind\n");
                        self.declared_functions
                            .insert("rt_array_set_fast".to_string());
                    }
                    self.temp_counter += 1;
                    let unused_res = format!("%ba_s_slow_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                        unused_res, obj_val, idx_val, v_val
                    ));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", done_path));
                } else if obj.get_type().is_array() {
                    // --- GENERIC ARRAY OPTIMIZATION: INLINED CACHE CHECK ---
                    if !self.declared_functions.contains("LAST_ID") {
                        self.global_buffer
                            .push_str("@LAST_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@LAST_LEN = external global i64\n");
                        self.global_buffer
                            .push_str("@LAST_ELEM_SIZE = external global i64\n");
                        self.declared_functions.insert("LAST_ID".to_string());
                    }
                    if !self.declared_functions.contains("rt_array_set_fast") {
                        self.global_buffer
                            .push_str("declare i64 @rt_array_set_fast(i64, i64, i64) nounwind\n");
                        self.declared_functions
                            .insert("rt_array_set_fast".to_string());
                    }
                    if !self.declared_functions.contains("PREV_ID") {
                        self.global_buffer
                            .push_str("@PREV_ID = external global i64\n");
                        self.global_buffer
                            .push_str("@PREV_PTR = external global i8*\n");
                        self.global_buffer
                            .push_str("@PREV_LEN = external global i64\n");
                        self.declared_functions.insert("PREV_ID".to_string());
                    }

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;
                    let done_path = format!("array_set_done{}", label_id);

                    let idx_norm = idx_val.clone();

                    self.temp_counter += 1;
                    let last_id = format!("%last_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ID", last_id));

                    self.temp_counter += 1;
                    let id_match = format!("%id_match{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        id_match, last_id, obj_val
                    ));

                    let _fast_path = format!("array_set_fast{}", label_id);
                    let slow_path = format!("array_set_slow{}", label_id);
                    let check_len = format!("array_set_check_len{}", label_id);
                    let prev_set_check = format!("array_set_prev{}", label_id);

                    // Branch to check_len if ID matches, else prev check
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        id_match, check_len, prev_set_check
                    ));

                    self.emit_line(&format!("{}:", check_len));
                    self.temp_counter += 1;
                    let last_len = format!("%last_len{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_LEN", last_len));

                    self.temp_counter += 1;
                    let in_bounds = format!("%in_bounds{}", self.temp_counter);
                    // Check bounds normally
                    self.emit_line(&format!(
                        "{} = icmp ult i64 {}, {}",
                        in_bounds, idx_norm, last_len
                    ));

                    let fast_access = format!("array_set_access{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        in_bounds, fast_access, slow_path
                    ));

                    self.emit_line(&format!("{}:", fast_access));
                    self.temp_counter += 1;
                    let ptr = format!("%ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @LAST_PTR", ptr));

                    let elem_type_set = obj.get_type().get_array_element_type();
                    let is_known_qword_set = elem_type_set.is_numeric()
                        || elem_type_set.is_float()
                        || matches!(
                            elem_type_set,
                            TejxType::Any | TejxType::String | TejxType::Bool | TejxType::Char
                        );

                    if is_known_qword_set {
                        // Skip LAST_ELEM_SIZE check — direct i64 store
                        let set_qword = format!("array_set_qword{}", label_id);
                        self.emit_line(&format!("br label %{}", set_qword));
                    } else {
                        self.temp_counter += 1;
                        let elem_size = format!("%elem_size{}", self.temp_counter);
                        self.emit_line(&format!("{} = load i64, i64* @LAST_ELEM_SIZE", elem_size));
                        self.temp_counter += 1;
                        let is_byte = format!("%is_byte{}", self.temp_counter);
                        self.emit_line(&format!("{} = icmp eq i64 {}, 1", is_byte, elem_size));

                        let set_byte = format!("array_set_byte{}", label_id);
                        let set_qword = format!("array_set_qword{}", label_id);
                        self.emit_line(&format!(
                            "br i1 {}, label %{}, label %{}",
                            is_byte, set_byte, set_qword
                        ));

                        self.emit_line(&format!("{}:", set_byte));
                        self.temp_counter += 1;
                        let gep8 = format!("%gep8_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i8, i8* {}, i64 {}",
                            gep8, ptr, idx_norm
                        ));

                        // Value conversion for byte array
                        let src_ty = src.get_type();
                        let v_to_store = if matches!(src_ty, TejxType::Bool)
                            || (src_ty.is_numeric() && !src_ty.is_float())
                        {
                            self.temp_counter += 1;
                            let v_i = format!("%v_i{}", self.temp_counter);
                            if !self.declared_functions.contains("rt_i64_to_i8") {
                                self.global_buffer
                                    .push_str("declare i8 @rt_i64_to_i8(i64)\n");
                                self.declared_functions.insert("rt_i64_to_i8".to_string());
                            }
                            self.emit_line(&format!(
                                "{} = call i8 @rt_i64_to_i8(i64 {})",
                                v_i, v_val
                            ));
                            v_i
                        } else if src_ty.is_float() || matches!(src_ty, TejxType::Any) {
                            self.temp_counter += 1;
                            let v_f = format!("%v_f{}", self.temp_counter);
                            self.emit_line(&format!("{} = bitcast i64 {} to double", v_f, v_val));
                            self.temp_counter += 1;
                            let v_i = format!("%v_i{}", self.temp_counter);
                            if !self.declared_functions.contains("rt_f64_to_i8") {
                                self.global_buffer
                                    .push_str("declare i8 @rt_f64_to_i8(double)\n");
                                self.declared_functions.insert("rt_f64_to_i8".to_string());
                            }
                            self.emit_line(&format!(
                                "{} = call i8 @rt_f64_to_i8(double {})",
                                v_i, v_f
                            ));
                            v_i
                        } else {
                            "0".to_string()
                        };

                        self.emit_line(&format!("store i8 {}, i8* {}", v_to_store, gep8));
                        self.emit_line(&format!("br label %{}", done_path));
                    }

                    let set_qword = format!("array_set_qword{}", label_id);
                    self.emit_line(&format!("{}:", set_qword));
                    self.temp_counter += 1;
                    let ptr64 = format!("%ptr64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i8* {} to i64*", ptr64, ptr));
                    self.temp_counter += 1;
                    let gep64 = format!("%gep64_{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr i64, i64* {}, i64 {}",
                        gep64, ptr64, idx_norm
                    ));
                    self.emit_line(&format!("store i64 {}, i64* {}", v_val, gep64));
                    self.emit_line(&format!("br label %{}", done_path));

                    // --- PREV cache check for StoreIndex ---
                    self.emit_line(&format!("{}:", prev_set_check));
                    self.temp_counter += 1;
                    let prev_id_s = format!("%prev_id_s{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV_ID", prev_id_s));
                    self.temp_counter += 1;
                    let prev_match_s = format!("%prev_match_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        prev_match_s, prev_id_s, obj_val
                    ));
                    let prev_set_fast = format!("array_set_prev_fast{}", label_id);
                    let prev2_set_check = format!("array_set_prev2{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev_match_s, prev_set_fast, prev2_set_check
                    ));

                    self.emit_line(&format!("{}:", prev_set_fast));
                    self.temp_counter += 1;
                    let prev_len_s = format!("%prev_len_s{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV_LEN", prev_len_s));
                    self.temp_counter += 1;
                    let prev_bounds_s = format!("%prev_bounds_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ult i64 {}, {}",
                        prev_bounds_s, idx_norm, prev_len_s
                    ));
                    let prev_set_access = format!("array_set_prev_access{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev_bounds_s, prev_set_access, prev2_set_check
                    ));

                    self.emit_line(&format!("{}:", prev_set_access));
                    self.temp_counter += 1;
                    let prev_ptr_s = format!("%prev_ptr_s{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @PREV_PTR", prev_ptr_s));
                    self.temp_counter += 1;
                    let prev_ptr64_s = format!("%prev_ptr64_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast i8* {} to i64*",
                        prev_ptr64_s, prev_ptr_s
                    ));
                    self.temp_counter += 1;
                    let prev_gep_s = format!("%prev_gep_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr i64, i64* {}, i64 {}",
                        prev_gep_s, prev_ptr64_s, idx_norm
                    ));
                    self.emit_line(&format!("store i64 {}, i64* {}", v_val, prev_gep_s));
                    self.emit_line(&format!("br label %{}", done_path));

                    // --- PREV2 cache check for StoreIndex ---
                    self.emit_line(&format!("{}:", prev2_set_check));
                    self.temp_counter += 1;
                    let prev2_id_s = format!("%prev2_id_s{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV2_ID", prev2_id_s));
                    self.temp_counter += 1;
                    let prev2_match_s = format!("%prev2_match_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp eq i64 {}, {}",
                        prev2_match_s, prev2_id_s, obj_val
                    ));
                    let prev2_set_fast = format!("array_set_prev2_fast{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev2_match_s, prev2_set_fast, slow_path
                    ));

                    self.emit_line(&format!("{}:", prev2_set_fast));
                    self.temp_counter += 1;
                    let prev2_len_s = format!("%prev2_len_s{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @PREV2_LEN", prev2_len_s));
                    self.temp_counter += 1;
                    let prev2_bounds_s = format!("%prev2_bounds_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ult i64 {}, {}",
                        prev2_bounds_s, idx_norm, prev2_len_s
                    ));
                    let prev2_set_access = format!("array_set_prev2_access{}", label_id);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        prev2_bounds_s, prev2_set_access, slow_path
                    ));

                    self.emit_line(&format!("{}:", prev2_set_access));
                    self.temp_counter += 1;
                    let prev2_ptr_s = format!("%prev2_ptr_s{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @PREV2_PTR", prev2_ptr_s));
                    self.temp_counter += 1;
                    let prev2_ptr64_s = format!("%prev2_ptr64_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast i8* {} to i64*",
                        prev2_ptr64_s, prev2_ptr_s
                    ));
                    self.temp_counter += 1;
                    let prev2_gep_s = format!("%prev2_gep_s{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = getelementptr i64, i64* {}, i64 {}",
                        prev2_gep_s, prev2_ptr64_s, idx_norm
                    ));
                    self.emit_line(&format!("store i64 {}, i64* {}", v_val, prev2_gep_s));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", slow_path));
                    self.temp_counter += 1;
                    let unused_res = format!("%array_set_slow_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                        unused_res, obj_val, idx_val, v_val
                    ));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", done_path));
                } else {
                    // When obj type is Any but index is numeric, it's likely an array
                    // accessed through a class field (LoadMember returns Any).
                    // Use rt_array_set_fast which handles arrays correctly at runtime.
                    let idx_ty = index.get_type();
                    if idx_ty.is_numeric() && !idx_ty.is_float() {
                        if !self.declared_functions.contains("rt_array_set_fast") {
                            self.global_buffer.push_str(
                                "declare i64 @rt_array_set_fast(i64, i64, i64) nounwind\n",
                            );
                            self.declared_functions
                                .insert("rt_array_set_fast".to_string());
                        }
                        self.temp_counter += 1;
                        let unused_res = format!("%any_set_res_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                            unused_res, obj_val, idx_val, v_val
                        ));
                    } else {
                        if !self.declared_functions.contains("m_set") {
                            self.global_buffer
                                .push_str("declare i64 @m_set(i64, i64, i64)\n");
                            self.declared_functions.insert("m_set".to_string());
                        }
                        // Box primitives for index access fallback (always 'Any' slot)
                        let v_ty = src.get_type();
                        let mut final_v_val = v_val;
                        if v_ty.is_numeric() || matches!(v_ty, TejxType::Bool | TejxType::Char) {
                            self.temp_counter += 1;
                            let boxed_reg = format!("%boxed_idx_set_{}", self.temp_counter);

                            if v_ty.is_float() {
                                self.temp_counter += 1;
                                let d_val = format!("%d_val_idx_set_{}", self.temp_counter);
                                self.emit_line(&format!(
                                    "{} = bitcast i64 {} to double",
                                    d_val, final_v_val
                                ));

                                if !self.declared_functions.contains("rt_box_number") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_number(double) readnone\n");
                                    self.declared_functions.insert("rt_box_number".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_number(double {})",
                                    boxed_reg, d_val
                                ));
                            } else if matches!(v_ty, TejxType::Bool) {
                                if !self.declared_functions.contains("rt_box_boolean") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_boolean(i64)\n");
                                    self.declared_functions.insert("rt_box_boolean".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_boolean(i64 {})",
                                    boxed_reg, final_v_val
                                ));
                            } else {
                                if !self.declared_functions.contains("rt_box_int") {
                                    self.global_buffer
                                        .push_str("declare i64 @rt_box_int(i64)\n");
                                    self.declared_functions.insert("rt_box_int".to_string());
                                }
                                self.emit_line(&format!(
                                    "{} = call i64 @rt_box_int(i64 {})",
                                    boxed_reg, final_v_val
                                ));
                            }
                            final_v_val = boxed_reg;
                        }

                        if !self.declared_functions.contains("rt_map_set_fast") {
                            self.global_buffer
                                .push_str("declare i64 @rt_map_set_fast(i64, i64, i64)\n");
                            self.declared_functions
                                .insert("rt_map_set_fast".to_string());
                        }
                        self.emit_line(&format!(
                            "call i64 @rt_map_set_fast(i64 {}, i64 {}, i64 {})",
                            obj_val, idx_val, final_v_val
                        ));
                    }
                }
            }
            MIRInstruction::Throw { value, .. } => {
                let val = self.resolve_value(value);
                self.emit_line(&format!("call void @tejx_throw(i64 {})", val));
                self.emit_line("unreachable");
            }
            MIRInstruction::Cast { dst, src, ty, .. } => {
                let s = self.resolve_value(src);
                let src_ty = src.get_type();

                self.temp_counter += 1;
                let tmp = format!("%cast{}", self.temp_counter);

                // SOI: Handle Reference Bit-Flipping for Runtime Memory Safety
                if matches!(ty, TejxType::Ref(_) | TejxType::Weak(_)) {
                    // Set BORROW_FLAG (1 << 63)
                    self.emit_line(&format!("{} = or i64 {}, -9223372036854775808", tmp, s));
                } else if matches!(src_ty, TejxType::Ref(_) | TejxType::Weak(_)) {
                    // Clear BORROW_FLAG to recover raw pointer if needed
                    self.emit_line(&format!("{} = and i64 {}, 9223372036854775807", tmp, s));
                } else if src_ty.is_numeric() && ty.is_numeric() {
                    if src_ty.is_float() && !ty.is_float() {
                        // bits(double) -> int
                        self.temp_counter += 1;
                        let f_val = format!("%f_val{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i64 {} to double", f_val, s));
                        self.emit_line(&format!("{} = fptosi double {} to i64", tmp, f_val));
                    } else if !src_ty.is_float() && ty.is_float() {
                        // int -> bits(double)
                        self.temp_counter += 1;
                        let f_res = format!("%f_res{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", f_res, s));
                        self.emit_line(&format!("{} = bitcast double {} to i64", tmp, f_res));
                    } else {
                        // Same kind or same bit-width (i64 vs i64)
                        self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, s));
                    }
                } else {
                    // Generic bitcast for other types
                    self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, s));
                }
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
            }
            MIRInstruction::Free { value, .. } => {
                let v = self.resolve_value(value);
                if !self.declared_functions.contains("rt_free") {
                    self.global_buffer.push_str("declare void @rt_free(i64)\n");
                    self.declared_functions.insert("rt_free".to_string());
                }
                self.emit_line(&format!("call void @rt_free(i64 {})", v));
            }
        }
    }
}
