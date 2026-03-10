/// MIR → LLVM IR Code Generator, mirroring C++ MIRCodeGen.cpp
/// Generates textual LLVM IR from MIR basic blocks.
use crate::intrinsics::*;
use crate::mir::*;
use crate::token::TokenType;
use crate::types::TejxType;
use std::collections::{HashMap, HashSet};

pub struct CodeGen {
    buffer: String,
    global_buffer: String,
    value_map: HashMap<String, String>,
    ptr_types: HashMap<String, String>, // MIR var name → LLVM alloca ptr name
    temp_counter: usize,
    label_counter: usize,
    declared_functions: HashSet<String>,
    defined_functions: HashSet<String>,
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
    num_roots: usize,
    pub class_fields: HashMap<String, Vec<(String, TejxType)>>,
    pub class_methods: HashMap<String, Vec<String>>,
    pub type_id_map: HashMap<String, u32>,
    current_arena: Option<String>,
}

impl CodeGen {
    fn get_aligned_offset(current: usize, ty: &TejxType) -> usize {
        let size = ty.size();
        let align = if size <= 1 {
            1
        } else if size <= 2 {
            2
        } else if size <= 4 {
            4
        } else {
            8
        };
        (current + align - 1) & !(align - 1)
    }

    fn get_llvm_type(ty: &TejxType) -> &str {
        match ty {
            // We use i64 for almost everything to maintain ABI consistency
            // with our bitcasting and boxing strategy.
            // Specialized types like Float32/Int32 are also stored in i64 registers/allocas
            // for uniform handling in function calls and collections.
            _ => "i64",
        }
    }

    fn is_gc_managed(ty: &TejxType) -> bool {
        match ty {
            TejxType::Class(_, _)
            | TejxType::FixedArray(_, _)
            | TejxType::DynamicArray(_)
            | TejxType::Function(_, _)
            | TejxType::String => true,
            _ => false,
        }
    }

    fn emit_strip_heap_offset(&mut self, val: &str) -> String {
        // Handle management offsets (HEAP_OFFSET or STACK_OFFSET)
        self.temp_counter += 1;
        let is_heap = format!("%is_heap_{}", self.temp_counter);
        self.temp_counter += 1;
        let is_stack = format!("%is_stack_{}", self.temp_counter);

        if !self.declared_functions.contains("HEAP_OFFSET_GLOBAL") {
            self.global_buffer
                .push_str("@HEAP_OFFSET = external global i64\n");
            self.declared_functions
                .insert("HEAP_OFFSET_GLOBAL".to_string());
        }
        if !self.declared_functions.contains("STACK_OFFSET_GLOBAL") {
            self.global_buffer
                .push_str("@STACK_OFFSET = external global i64\n");
            self.declared_functions
                .insert("STACK_OFFSET_GLOBAL".to_string());
        }

        self.temp_counter += 1;
        let h_offset = format!("%h_offset_{}", self.temp_counter);
        self.emit_line(&format!("{} = load i64, i64* @HEAP_OFFSET", h_offset));

        self.temp_counter += 1;
        let s_offset = format!("%s_offset_{}", self.temp_counter);
        self.emit_line(&format!("{} = load i64, i64* @STACK_OFFSET", s_offset));

        self.emit_line(&format!("{} = icmp uge i64 {}, {}", is_heap, val, h_offset));
        self.emit_line(&format!(
            "{} = icmp uge i64 {}, {}",
            is_stack, val, s_offset
        ));

        self.temp_counter += 1;
        let sub_val = format!("%sub_val_{}", self.temp_counter);
        // Prioritize HEAP_OFFSET if both are true (since HEAP_OFFSET > STACK_OFFSET)
        self.emit_line(&format!(
            "{} = select i1 {}, i64 {}, i64 {}",
            sub_val, is_heap, h_offset, s_offset
        ));

        self.temp_counter += 1;
        let effectively_obj = format!("%eff_obj_{}", self.temp_counter);
        self.emit_line(&format!(
            "{} = or i1 {}, {}",
            effectively_obj, is_heap, is_stack
        ));

        self.temp_counter += 1;
        let real_sub = format!("%real_sub_{}", self.temp_counter);
        self.emit_line(&format!(
            "{} = select i1 {}, i64 {}, i64 0",
            real_sub, effectively_obj, sub_val
        ));

        self.temp_counter += 1;
        let stripped = format!("%stripped_{}", self.temp_counter);
        self.emit_line(&format!("{} = sub i64 {}, {}", stripped, val, real_sub));

        stripped
    }

    fn store_ptr(&mut self, ptr: &str, src_val: &str) {
        let llvm_ty = self.ptr_types.get(ptr).map(|s| s.as_str()).unwrap_or("i64");
        let mut final_src = src_val.to_string();
        if llvm_ty != "i64" {
            self.temp_counter += 1;
            let cast_reg = format!("%cast_{}", self.temp_counter);
            if llvm_ty == "float" {
                // i64 -> i32 -> float
                let trunc_reg = format!("%trunc_to_i32_{}", self.temp_counter);
                self.buffer.push_str(&format!(
                    "  {} = trunc i64 {} to i32\n",
                    trunc_reg, final_src
                ));
                self.buffer.push_str(&format!(
                    "  {} = bitcast i32 {} to float\n",
                    cast_reg, trunc_reg
                ));
            } else if llvm_ty == "double" {
                self.buffer.push_str(&format!(
                    "  {} = bitcast i64 {} to double\n",
                    cast_reg, final_src
                ));
            } else {
                self.buffer.push_str(&format!(
                    "  {} = trunc i64 {} to {}\n",
                    cast_reg, final_src, llvm_ty
                ));
            }
            final_src = cast_reg;
        }
        self.buffer.push_str(&format!(
            "  store {} {}, {}* {}\n",
            llvm_ty, final_src, llvm_ty, ptr
        ));
    }

    fn load_ptr(&mut self, ptr: &str, dest_reg: &str) {
        let llvm_ty = self.ptr_types.get(ptr).map(|s| s.as_str()).unwrap_or("i64");
        if llvm_ty != "i64" {
            self.temp_counter += 1;
            let val_reg = format!("%ld_{}", self.temp_counter);
            self.buffer.push_str(&format!(
                "  {} = load {}, {}* {}\n",
                val_reg, llvm_ty, llvm_ty, ptr
            ));
            // Extend back to i64
            let cast_code = if llvm_ty == "float" {
                self.temp_counter += 1;
                let i32_reg = format!("%bits_i32_{}", self.temp_counter);
                self.buffer.push_str(&format!(
                    "  {} = bitcast float {} to i32\n",
                    i32_reg, val_reg
                ));
                format!("zext i32 {} to i64", i32_reg)
            } else if llvm_ty == "double" {
                format!("bitcast double {} to i64", val_reg)
            } else {
                format!("sext {} {} to i64", llvm_ty, val_reg)
            };
            self.buffer
                .push_str(&format!("  {} = {}\n", dest_reg, cast_code));
        } else {
            self.buffer
                .push_str(&format!("  {} = load i64, i64* {}\n", dest_reg, ptr));
        }
    }

    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            global_buffer: String::new(),
            value_map: HashMap::new(),
            ptr_types: HashMap::new(),
            temp_counter: 0,
            label_counter: 0,
            declared_functions: HashSet::new(),
            defined_functions: HashSet::new(),
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
            num_roots: 0,
            class_fields: HashMap::new(),
            class_methods: HashMap::new(),
            type_id_map: HashMap::new(),
            current_arena: None,
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
        // More robust escape analysis: If the reference escapes via return, pass as argument,
        // or is stored inside an object/array, it escapes the stack frame.
        let mut check_vars = vec![var_name.to_string()];
        let mut i = 0;

        while i < check_vars.len() {
            let current_var = check_vars[i].clone();
            for block in &func.blocks {
                for instr in &block.instructions {
                    match instr {
                        MIRInstruction::Call { callee, args, .. } => {
                            // Whitelist: common safe runtime calls where args[0] is 'this'
                            // and the pointer is only borrowed, not escaped.
                            let is_safe = matches!(
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
                                    | "rt_array_get_data_ptr"
                                    | "rt_to_string"
                                    | "rt_print"
                                    | "rt_get_type"
                                    | "rt_is_array"
                                    | "rt_len"
                            );

                            for (i, arg) in args.iter().enumerate() {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name == &current_var {
                                        if i == 0 && is_safe {
                                            continue;
                                        }
                                        return true; // Escapes as argument
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

    fn needs_arena(&self, func: &MIRFunction) -> bool {
        for bb in &func.blocks {
            for inst in &bb.instructions {
                if let MIRInstruction::Call {
                    callee, args, dst, ..
                } = inst
                {
                    if callee == RT_CLASS_NEW && !dst.is_empty() && !self.does_escape(func, dst) {
                        let class_name = match args.get(0) {
                            Some(MIRValue::Constant { value, .. }) => {
                                value.trim_matches('"').to_string()
                            }
                            _ => continue,
                        };
                        let body_size = self
                            .class_fields
                            .get(&class_name)
                            .map(|fields| fields.iter().map(|(_, ty)| ty.size()).sum::<usize>())
                            .unwrap_or(0);
                        if body_size > 1024 {
                            return true;
                        }
                    }
                }
            }
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

                    // Always Box as closure Map { "ptr": fn_ptr, "env": env }
                    if !self.declared_functions.contains("rt_closure_new") {
                        self.global_buffer
                            .push_str("declare i64 @rt_closure_new(i64) nounwind\n");
                        self.declared_functions.insert("rt_closure_new".to_string());
                    }
                    if !self.declared_functions.contains("rt_string_from_c_str") {
                        self.global_buffer
                            .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                        self.declared_functions
                            .insert("rt_string_from_c_str".to_string());
                    }

                    self.temp_counter += 1;
                    let closure_id = format!("%closure{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @rt_closure_new(i64 0)", closure_id));

                    // Set "ptr"
                    let ptr_key = "@str_key_ptr";
                    if !self.declared_globals.contains(ptr_key) {
                        self.global_buffer.push_str(
                            "@str_key_ptr = private unnamed_addr constant [4 x i8] c\"ptr\\00\"\n",
                        );
                        self.declared_globals.insert(ptr_key.to_string());
                    }
                    self.temp_counter += 1;
                    let ptr_key_id = format!("%ptr_key{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @rt_string_from_c_str(i64 ptrtoint ([4 x i8]* @str_key_ptr to i64))", ptr_key_id));
                    self.emit_line(&format!(
                        "call void @rt_Map_set(i64 {}, i64 {}, i64 {})",
                        closure_id, ptr_key_id, fn_ptr
                    ));

                    // Set "env"
                    let env_to_pass = if let Some(env) = self.current_env.clone() {
                        env
                    } else {
                        // Create a fresh empty environment if the parent doesn't have one
                        self.temp_counter += 1;
                        let fresh_env = format!("%fresh_env{}", self.temp_counter);
                        self.emit_line(&format!("{} = call i64 @rt_map_new(i64 0)", fresh_env));
                        fresh_env
                    };

                    let env_key = "@str_key_env";
                    if !self.declared_globals.contains(env_key) {
                        self.global_buffer.push_str(
                            "@str_key_env = private unnamed_addr constant [4 x i8] c\"env\\00\"\n",
                        );
                        self.declared_globals.insert(env_key.to_string());
                    }
                    self.temp_counter += 1;
                    let env_key_id = format!("%env_key{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @rt_string_from_c_str(i64 ptrtoint ([4 x i8]* @str_key_env to i64))", env_key_id));
                    self.emit_line(&format!(
                        "call void @rt_Map_set(i64 {}, i64 {}, i64 {})",
                        closure_id, env_key_id, env_to_pass
                    ));

                    return closure_id;
                }
                if value.starts_with("new ") {
                    return "0".to_string();
                }

                let is_integer_type = ty.is_numeric() && !ty.is_float();
                let is_float_type = ty.is_float();
                let is_bool_type = matches!(ty, TejxType::Bool);

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

                if is_float_type {
                    if let Ok(d) = value.parse::<f64>() {
                        // Variables of type Number ALWAYS store bitcasted doubles
                        return format!("{}", d.to_bits());
                    }
                    return "0".to_string();
                }

                if value == "null" || (value == "0" && Self::is_gc_managed(ty)) {
                    return "0".to_string();
                }

                if matches!(ty, TejxType::Void) && value == "0" {
                    return "0".to_string();
                }

                let raw_ptr = self.emit_string_constant(value);
                if !self.declared_functions.contains("rt_string_from_c_str") {
                    self.global_buffer
                        .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                    self.declared_functions
                        .insert("rt_string_from_c_str".to_string());
                }
                self.emit_box_string(&raw_ptr)
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
                                .push_str("declare i64 @rt_Map_get(i64, i64)\n");
                            self.declared_functions.insert("rt_Map_get".to_string());
                        }
                        if !self.declared_functions.contains("rt_string_from_c_str") {
                            self.global_buffer
                                .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                            self.declared_functions
                                .insert("rt_string_from_c_str".to_string());
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
                            "{} = call i64 @rt_string_from_c_str(i64 ptrtoint ([{} x i8]* {} to i64))",
                            key_id,
                            cap_key.len() + 1,
                            key_global
                        ));

                        self.temp_counter += 1;
                        let val_reg = format!("%cap_val{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_Map_get(i64 {}, i64 {})",
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
                    self.load_ptr(&reg, &val_reg);
                    return val_reg;
                }

                // Check for function pointer using type info
                if let TejxType::Function(params, _) = ty {
                    let args_sig = vec!["i64"; params.len()].join(", ");
                    return format!("ptrtoint (i64 ({})* @{} to i64)", args_sig, name);
                }

                // Check for function parameter (if not mapped to alloca yet? - should be in value_map)
                // Fallback for globals
                if self.declared_functions.contains(name) || name == TEJX_MAIN {
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

    fn emit_store_variable(&mut self, name: &str, val: &str, ty: &TejxType) {
        if let Some(cap_key) = self.get_captured_key(name) {
            if let Some(env) = self.current_env.clone() {
                if !self.declared_functions.contains("rt_Map_set") {
                    self.global_buffer
                        .push_str("declare void @rt_Map_set(i64, i64, i64) nounwind\n");
                    self.declared_functions.insert("rt_Map_set".to_string());
                }
                if !self.declared_functions.contains("rt_string_from_c_str") {
                    self.global_buffer
                        .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                    self.declared_functions
                        .insert("rt_string_from_c_str".to_string());
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
                    "{} = call i64 @rt_string_from_c_str(i64 ptrtoint ([{} x i8]* {} to i64))",
                    key_id,
                    cap_key.len() + 1,
                    key_global
                ));

                let val_to_store = if ty.is_float()
                    || ty.is_numeric()
                    || *ty == TejxType::Bool
                    || *ty == TejxType::Char
                {
                    // Primitives are now bitcasted directly into i64 slots (generic slots)
                    val.to_string()
                } else {
                    val.to_string()
                };

                self.emit_line(&format!(
                    "call void @rt_Map_set(i64 {}, i64 {}, i64 {})",
                    env, key_id, val_to_store
                ));
            }
        } else {
            // Only store to local ptr if it's NOT a captured variable
            let ptr = self.resolve_ptr(name);
            self.store_ptr(&ptr, val);
        }
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

        // Module headers
        self.global_buffer.push_str("; ModuleID = 'tejx_module'\n");
        self.global_buffer
            .push_str("source_filename = \"tejx_module\"\n");
        self.global_buffer
            .push_str("target datalayout = \"e-m:o-i64:64-i128:128-n32:64-S128-Fn32\"\n");
        self.global_buffer
            .push_str("target triple = \"arm64-apple-macosx15.0.0\"\n");

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

        self.global_buffer
            .push_str("%struct.ObjectHeader = type { i64, i16, i16, i32, i32, i32 }\n");

        // --- Type Registration ---
        let mut type_id = 100; // Start user types from 100
        let mut init_type_buffer = String::new();
        init_type_buffer.push_str("define void @rt_init_types() {\n");

        for (class_name, fields) in &self.class_fields {
            let id = type_id;
            type_id += 1;
            self.type_id_map.insert(class_name.clone(), id);

            let mut ptr_offsets = Vec::new();
            let mut current_offset = 0;
            for (_name, ty) in fields {
                current_offset = Self::get_aligned_offset(current_offset, ty);
                if matches!(ty, TejxType::Class(_, _) | TejxType::String) {
                    ptr_offsets.push(current_offset);
                }
                current_offset += ty.size();
            }
            let size = (current_offset + 7) & !7;

            // Create global array for offsets
            let offset_arr_name = format!("@type_{}_offsets", id);
            if !ptr_offsets.is_empty() {
                let offsets_str: Vec<String> =
                    ptr_offsets.iter().map(|o| format!("i64 {}", o)).collect();
                self.global_buffer.push_str(&format!(
                    "{} = private constant [{} x i64] [{}]\n",
                    offset_arr_name,
                    ptr_offsets.len(),
                    offsets_str.join(", ")
                ));
            }

            // Register call: rt_register_type(id, size, ptr_count, offsets_ptr, finalizer)
            let offsets_ptr = if ptr_offsets.is_empty() {
                "null".to_string()
            } else {
                format!(
                    "bitcast ([{} x i64]* {} to i64*)",
                    ptr_offsets.len(),
                    offset_arr_name
                )
            };

            // Check for finalizer
            let mut finalizer_ptr = "null".to_string();
            if let Some(methods) = self.class_methods.get(class_name) {
                if methods.contains(&"finalize".to_string())
                    || methods.contains(&"~destructor".to_string())
                {
                    let method_name = if methods.contains(&"finalize".to_string()) {
                        "finalize"
                    } else {
                        "~destructor"
                    };
                    let wrapper_name = format!("@finalizer_wrapper_{}", id);
                    let real_method = format!("f_{}_{}", class_name, method_name);

                    self.global_buffer.push_str(&format!(
                        "define void {}(i64 %body) nounwind {{\n  call void @{}(i64 %body)\n  ret void\n}}\n",
                        wrapper_name, real_method
                    ));
                    finalizer_ptr = format!("bitcast (void (i64)* {} to i8*)", wrapper_name);
                }
            }

            init_type_buffer.push_str(&format!(
                "  call void @rt_register_type(i32 {}, i64 {}, i64 {}, i64* {}, i8* {})\n",
                id,
                size,
                ptr_offsets.len(),
                offsets_ptr,
                finalizer_ptr
            ));
        }
        init_type_buffer.push_str("  ret void\n}\n");
        self.buffer.push_str(&init_type_buffer);

        if !self.declared_functions.contains("rt_register_type") {
            self.global_buffer
                .push_str("declare void @rt_register_type(i32, i64, i64, i64*, i8*)\n");
            self.declared_functions
                .insert("rt_register_type".to_string());
        }

        // Filter functions to remove duplicates, prioritizing non-empty tejx_main
        let mut unique_functions = Vec::new();
        let mut seen = HashSet::new();
        let mut tejx_main_to_keep = None;

        for (i, func) in functions.iter().enumerate() {
            if func.name == "tejx_main" {
                if tejx_main_to_keep.is_none() || func.blocks[0].instructions.len() > 1 {
                    tejx_main_to_keep = Some(i);
                }
                continue;
            }
            if !seen.contains(&func.name) {
                seen.insert(func.name.clone());
                unique_functions.push(func);
            }
        }
        if let Some(idx) = tejx_main_to_keep {
            unique_functions.push(&functions[idx]);
        }

        for func in unique_functions {
            self.gen_function_v2(func);
        }

        // Exception handling runtime functions
        self.global_buffer
            .push_str("declare i32 @_setjmp(i8*) returns_twice\n");
        self.global_buffer
            .push_str(&format!("declare void @{}(i8*)\n", TEJX_PUSH_HANDLER));
        self.global_buffer
            .push_str(&format!("declare void @{}()\n", TEJX_POP_HANDLER));
        if !self.declared_functions.contains(TEJX_THROW) {
            self.global_buffer
                .push_str(&format!("declare void @{}(i64)\n", TEJX_THROW));
        }
        if !self.declared_functions.contains(TEJX_GET_EXCEPTION) {
            self.global_buffer
                .push_str(&format!("declare i64 @{}()\n", TEJX_GET_EXCEPTION));
        }
        if !self.declared_functions.contains("rt_string_from_c_str") {
            self.global_buffer
                .push_str("declare i64 @rt_string_from_c_str(i64)\n");
        }

        // Generate main wrapper if tejx_main exists
        if has_tejx_main {
            self.buffer.push_str("\n");
            self.buffer
                .push_str(&format!("declare i32 @{}(i32, i8**)\n", TEJX_RUNTIME_MAIN));
            self.buffer
                .push_str("define i32 @main(i32 %argc, i8** %argv) {\n");
            self.buffer.push_str("entry:\n");
            self.buffer.push_str(&format!(
                "  %call = call i32 @{}(i32 %argc, i8** %argv)\n",
                TEJX_RUNTIME_MAIN
            ));
            self.buffer.push_str("  ret i32 %call\n");
            self.buffer.push_str("}\n");
        }

        format!("{}{}", self.global_buffer, self.buffer)
    }

    fn gen_function_v2(&mut self, func: &MIRFunction) {
        self.value_map.clear();
        self.ptr_types.clear();
        self.stack_arrays.clear();
        self.heap_array_ptrs.clear();
        self.float_ssa_vars.clear();
        self.temp_counter = 0;
        self.current_function_params.clear();
        self.local_vars.clear();
        self.current_env = None;
        self.current_arena = None;
        self.num_roots = 0;

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

        if func.is_extern {
            if !self.declared_functions.contains(&func.name) {
                let decl_params = if func.params.is_empty() {
                    String::new()
                } else {
                    func.params
                        .iter()
                        .map(|_| "i64".to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                self.global_buffer.push_str(&format!(
                    "declare i64 @\"{}\"({})\n",
                    func.name, decl_params
                ));
                self.declared_functions.insert(func.name.clone());
            }
            return;
        }

        // Skip if function was already defined (prevents duplicate definitions from prelude)
        if self.defined_functions.contains(&func.name) {
            return;
        }
        self.defined_functions.insert(func.name.clone());

        self.emit(&format!(
            "define i64 @\"{}\"({}) {{\n",
            func.name, params_str
        ));

        // Entry: allocas for all variables used in the function
        self.emit("entry:\n");
        let entry_marker = self.buffer.len();

        if !self.declared_functions.contains("rt_safepoint_poll") {
            self.global_buffer
                .push_str("declare void @rt_safepoint_poll() nounwind\n");
            self.declared_functions
                .insert("rt_safepoint_poll".to_string());
        }

        // 1. Scan for all local variables
        for _bb in &func.blocks {
            if !self.declared_functions.contains(RT_ARENA_CREATE) {
                self.global_buffer
                    .push_str(&format!("declare i64 @{}(i64) nounwind\n", RT_ARENA_CREATE));
                self.declared_functions.insert(RT_ARENA_CREATE.to_string());
            }
            if !self.declared_functions.contains(RT_ARENA_DESTROY) {
                self.global_buffer.push_str(&format!(
                    "declare void @{}(i64) nounwind\n",
                    RT_ARENA_DESTROY
                ));
                self.declared_functions.insert(RT_ARENA_DESTROY.to_string());
            }
            self.temp_counter += 1;
            let arena_reg = format!("%arena_{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call i64 @{}(i64 0)",
                arena_reg, RT_ARENA_CREATE
            ));
            self.current_arena = Some(arena_reg);
        }

        // 1. Scan for all local variables
        for bb in &func.blocks {
            for inst in &bb.instructions {
                let dest_var = match inst {
                    MIRInstruction::Move { dst, .. }
                    | MIRInstruction::BinaryOp { dst, .. }
                    | MIRInstruction::Call { dst, .. }
                    | MIRInstruction::IndirectCall { dst, .. }
                    | MIRInstruction::LoadMember { dst, .. }
                    | MIRInstruction::LoadIndex { dst, .. }
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

        // 2. Deterministic IR: Collect and sort ALL variables that need an alloca (params + locals)
        let mut sorted_alloca_vars: Vec<String> = Vec::new();
        for p in &func.params {
            if !self.is_captured(p) {
                sorted_alloca_vars.push(p.clone());
            }
        }
        for name in &self.local_vars {
            if !self.is_captured(name) {
                sorted_alloca_vars.push(name.clone());
            }
        }
        sorted_alloca_vars.sort();
        sorted_alloca_vars.dedup();

        // 3. Emit allocas deterministically
        for name in &sorted_alloca_vars {
            let reg_name = format!("%{}_ptr", name.replace('$', "_"));
            let ty = func.variables.get(name).unwrap_or(&TejxType::Void);
            let llvm_ty = Self::get_llvm_type(ty);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca {}\n", reg_name, llvm_ty));
            self.ptr_types.insert(reg_name.clone(), llvm_ty.to_string());
            self.value_map.insert(name.clone(), reg_name.clone());
        }

        // Create environment if needed
        let has_captures = self.local_vars.iter().any(|v| self.is_captured(v))
            || func.params.iter().any(|p| self.is_captured(p));

        if func.name.starts_with("lambda_") {
            if !func.params.is_empty() {
                // Create a NEW environment map for this lambda call
                // and COPY all keys from the passed environment (%__env)
                if !self.declared_functions.contains("rt_map_new") {
                    self.global_buffer
                        .push_str("declare i64 @rt_map_new(i64) nounwind\n");
                    self.declared_functions.insert("rt_map_new".to_string());
                }
                if !self.declared_functions.contains("rt_Map_merge") {
                    self.global_buffer
                        .push_str("declare void @rt_Map_merge(i64, i64) nounwind\n");
                    self.declared_functions.insert("rt_Map_merge".to_string());
                }

                self.temp_counter += 1;
                let env_alloca = format!("%env_alloca_{}", self.temp_counter);
                self.alloca_buffer
                    .push_str(&format!("  {} = alloca i64\n", env_alloca));

                let new_env = format!("%new_env_{}", self.temp_counter);
                let passed_env = format!("%{}", func.params[0]);

                self.emit_line(&format!("{} = call i64 @rt_map_new(i64 0)", new_env));
                self.emit_line(&format!("store i64 {}, i64* {}", new_env, env_alloca));
                self.emit_line(&format!("call void @rt_push_root(i64* {})", env_alloca));
                self.num_roots += 1;

                self.emit_line(&format!(
                    "call void @rt_Map_merge(i64 {}, i64 {})",
                    new_env, passed_env
                ));

                self.current_env = Some(new_env);
            }
        } else if has_captures {
            if !self.declared_functions.contains("rt_map_new") {
                self.global_buffer
                    .push_str("declare i64 @rt_map_new(i64) nounwind\n");
                self.declared_functions.insert("rt_map_new".to_string());
            }

            self.temp_counter += 1;
            let env_alloca = format!("%env_alloca_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", env_alloca));

            let env_reg = format!("%env_id{}", self.temp_counter);
            self.emit_line(&format!("{} = call i64 @rt_map_new(i64 0)", env_reg));
            self.emit_line(&format!("store i64 {}, i64* {}", env_reg, env_alloca));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", env_alloca));
            self.num_roots += 1;
            self.current_env = Some(env_reg);
        }

        // 4. Store parameters into their allocas
        for p in &func.params {
            if let Some(reg_name) = self.value_map.get(p).cloned() {
                self.store_ptr(&reg_name, &format!("%{}", p));
            }
        }

        // 5. GC Root Registration: Emit managed variables deterministically
        let mut sorted_managed_vars: Vec<String> = sorted_alloca_vars
            .iter()
            .filter(|name| {
                let ty = func.variables.get(*name).unwrap_or_else(|| {
                    panic!(
                        "Variable '{}' not found in variables map of function '{}'",
                        name, func.name
                    );
                });
                Self::is_gc_managed(ty)
            })
            .cloned()
            .collect();
        sorted_managed_vars.sort();

        for name in sorted_managed_vars {
            if let Some(ptr_name) = self.value_map.get(&name).cloned() {
                if !self.declared_functions.contains("rt_push_root") {
                    self.global_buffer
                        .push_str("declare void @rt_push_root(i64*) nounwind\n");
                    self.declared_functions.insert("rt_push_root".to_string());
                }
                self.emit_line(&format!("call void @rt_push_root(i64* {})", ptr_name));
                self.num_roots += 1;
            }
        }

        // Sync parameters to environment if captured
        for p in &func.params {
            if let Some(cap_key) = self.get_captured_key(p) {
                if let Some(env) = self.current_env.clone() {
                    if !self.declared_functions.contains("rt_Map_set") {
                        self.global_buffer
                            .push_str("declare void @rt_Map_set(i64, i64, i64) nounwind\n");
                        self.declared_functions.insert("rt_Map_set".to_string());
                    }
                    if !self.declared_functions.contains("rt_string_from_c_str") {
                        self.global_buffer
                            .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                        self.declared_functions
                            .insert("rt_string_from_c_str".to_string());
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
                        "{} = call i64 @rt_string_from_c_str(i64 ptrtoint ([{} x i8]* {} to i64))",
                        key_id,
                        cap_key.len() + 1,
                        key_global
                    ));

                    self.emit_line(&format!(
                        "call void @rt_Map_set(i64 {}, i64 {}, i64 %{})",
                        env, key_id, p
                    ));
                }
            }
        }

        // Branch to first block
        if !func.blocks.is_empty() {
            self.emit_line("call void @rt_safepoint_poll()");
            self.emit_line(&format!("br label %{}", func.blocks[0].name));
        } else {
            self.emit_line("ret i64 0");
        }

        // Generate blocks with block name resolution
        for (i, bb) in func.blocks.iter().enumerate() {
            self.emit(&format!("{}:\n", bb.name));

            let mut has_handler = false;
            if let Some(handler_idx) = bb.exception_handler {
                if handler_idx < func.blocks.len() {
                    has_handler = true;
                    let handler_name = &func.blocks[handler_idx].name;
                    // Allocate jmp_buf on THIS function's stack frame
                    self.temp_counter += 1;
                    let jmpbuf = format!("%jmpbuf{}", self.temp_counter);
                    self.alloca_buffer
                        .push_str(&format!("  {} = alloca [37 x i64]\n", jmpbuf));
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
                    self.emit_line(&format!(
                        "call void @{}(i8* {})",
                        TEJX_PUSH_HANDLER, jmpbuf_ptr
                    ));
                }
            }

            for inst in &bb.instructions {
                self.gen_instruction_v2(inst, func, &bb.name, i);
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

    fn gen_instruction_v2(
        &mut self,
        inst: &MIRInstruction,
        func: &MIRFunction,
        _bb_name: &str,
        current_bb: usize,
    ) {
        match inst {
            MIRInstruction::Move { dst, src, .. } => {
                let val = self.resolve_value(src);
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &val, dst_ty);

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

                let unwrap_ty = |ty: &TejxType| -> TejxType { ty.clone() };

                // Check types
                let is_string_op = matches!(unwrap_ty(l_ty), TejxType::String)
                    || matches!(unwrap_ty(r_ty), TejxType::String);
                let is_float_op = unwrap_ty(l_ty).is_float() || unwrap_ty(r_ty).is_float();
                let is_any_op = false || false;

                let is_numeric_op = !is_string_op
                    && (is_float_op
                        || is_any_op
                        || l_ty.is_numeric()
                        || r_ty.is_numeric()
                        || matches!(l_ty, TejxType::Bool)
                        || matches!(r_ty, TejxType::Bool));

                if is_string_op {
                    let l_val = if l_ty.is_numeric() {
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
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_l{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            boxed, val_as_double
                        ));
                        boxed
                    } else if matches!(l_ty, TejxType::Bool) {
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_l{}", self.temp_counter);
                        self.emit_line(&format!("{} = or i64 0, {}", boxed, l));
                        boxed
                    } else if matches!(l_ty, TejxType::String) && l.starts_with("ptrtoint") {
                        if !self.declared_functions.contains("rt_string_from_c_str") {
                            self.global_buffer
                                .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                            self.declared_functions
                                .insert("rt_string_from_c_str".to_string());
                        }
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_l{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_string_from_c_str(i64 {})",
                            boxed, l
                        ));
                        boxed
                    } else {
                        l.to_string()
                    };

                    let r_val = if r_ty.is_numeric() {
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
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_r{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            boxed, val_as_double
                        ));
                        boxed
                    } else if matches!(r_ty, TejxType::Bool) {
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_r{}", self.temp_counter);
                        self.emit_line(&format!("{} = or i64 0, {}", boxed, r));
                        boxed
                    } else if matches!(r_ty, TejxType::String) && r.starts_with("ptrtoint") {
                        if !self.declared_functions.contains("rt_string_from_c_str") {
                            self.global_buffer
                                .push_str("declare i64 @rt_string_from_c_str(i64)\n");
                            self.declared_functions
                                .insert("rt_string_from_c_str".to_string());
                        }
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_r{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_string_from_c_str(i64 {})",
                            boxed, r
                        ));
                        boxed
                    } else if matches!(r_ty, TejxType::Int32 | TejxType::Int64) {
                        self.temp_counter += 1;
                        let boxed = format!("%boxed_r{}", self.temp_counter);
                        self.emit_line(&format!("{} = or i64 0, {}", boxed, r));
                        boxed
                    } else {
                        r.to_string()
                    };

                    if matches!(op, TokenType::Plus) {
                        if !self.declared_functions.contains("rt_str_concat_v2") {
                            self.global_buffer
                                .push_str("declare i64 @rt_str_concat_v2(i64, i64) nounwind\n");
                            self.declared_functions
                                .insert("rt_str_concat_v2".to_string());
                        }
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
                            tmp, l_val, r_val
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
                            eq_tmp, l_val, r_val
                        ));
                        self.temp_counter += 1;
                        self.emit_line(&format!("{} = call i64 @rt_not(i64 {})", tmp, eq_tmp));
                    } else {
                        self.emit_line(&format!("{} = add i64 {}, {}", tmp, l_val, r_val));
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.store_ptr(&ptr, &tmp);
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
                                        .push_str("declare void @rt_div_zero_error() nounwind\n");
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
                            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
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
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                    self.emit_store_variable(dst, &tmp, dst_ty);
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

                        if false {
                            // Inline: convert int result to bitcasted double representation
                            self.temp_counter += 1;
                            let res_f = format!("%res_f{}", self.temp_counter);
                            self.emit_line(&format!("{} = sitofp i64 {} to double", res_f, res_i));
                            self.emit_line(&format!("{} = bitcast double {} to i64", tmp, res_f));
                        } else {
                            self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, res_i));
                        }
                    }
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                    self.emit_store_variable(dst, &tmp, dst_ty);
                }
            }

            MIRInstruction::Jump { target, .. } => {
                let has_handler = func
                    .blocks
                    .iter()
                    .any(|b| b.name == _bb_name && b.exception_handler.is_some());
                if has_handler {
                    self.emit_line(&format!("call void @{}()", TEJX_POP_HANDLER));
                }

                // Safepoint poll for backward jumps (loops)
                if *target <= current_bb {
                    self.emit_line("call void @rt_safepoint_poll()");
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
                    self.emit_line(&format!("call void @{}()", TEJX_POP_HANDLER));
                }

                let cond_val = self.resolve_value(condition);

                let cond = if true {
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

                // Safepoint poll for backward branches (loops)
                if *true_target <= current_bb || *false_target <= current_bb {
                    self.emit_line("call void @rt_safepoint_poll()");
                }

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
                    self.emit_line(&format!("call void @{}()", TEJX_POP_HANDLER));
                }

                if self.num_roots > 0 {
                    if !self.declared_functions.contains("rt_pop_roots") {
                        self.global_buffer
                            .push_str("declare void @rt_pop_roots(i64) nounwind\n");
                        self.declared_functions.insert("rt_pop_roots".to_string());
                    }
                    self.emit_line(&format!("call void @rt_pop_roots(i64 {})", self.num_roots));
                }

                if let Some(arena) = &self.current_arena {
                    self.emit_line(&format!("call void @{}(i64 {})", RT_ARENA_DESTROY, arena));
                }

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

                    self.temp_counter += 1;
                    let result_tmp = format!("%boxed_num{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast double {} to i64",
                        result_tmp, float_val
                    ));
                    if !dst.is_empty() {
                        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                        self.emit_store_variable(dst, &result_tmp, dst_ty);
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
                        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                        self.emit_store_variable(dst, &bits_tmp, dst_ty);
                    }
                    return;
                }

                if callee == RT_CLASS_NEW {
                    let class_name = match &args[0] {
                        MIRValue::Constant { value, .. } => value.trim_matches('"').to_string(),
                        _ => "any".to_string(),
                    };

                    let type_id = self.type_id_map.get(&class_name).cloned().unwrap_or(2);
                    let body_size = self
                        .class_fields
                        .get(&class_name)
                        .map(|fields| {
                            let mut offset = 0;
                            for (_, ty) in fields {
                                offset = Self::get_aligned_offset(offset, ty);
                                offset += ty.size();
                            }
                            (offset + 7) & !7
                        })
                        .unwrap_or(0);

                    let is_escaped = !dst.is_empty() && self.does_escape(func, dst);

                    if !is_escaped
                        && !dst.is_empty()
                        && body_size > 1024
                        && self.current_arena.is_some()
                    {
                        // Arena Allocation
                        let arena = self.current_arena.clone().unwrap();
                        if !self.declared_functions.contains(RT_ARENA_ALLOC) {
                            self.global_buffer.push_str(&format!(
                                "declare i64 @{}(i64, i32, i64) nounwind\n",
                                RT_ARENA_ALLOC
                            ));
                            self.declared_functions.insert(RT_ARENA_ALLOC.to_string());
                        }

                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @{}(i64 {}, i32 {}, i64 {})",
                            result_tmp, RT_ARENA_ALLOC, arena, type_id, body_size as i64
                        ));

                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);
                        return;
                    }

                    if !is_escaped && !dst.is_empty() {
                        // Stack Allocation (24 bytes header + body_size)
                        let total_size = body_size + 24;
                        let obj_alloca = format!("%stack_class_{}", dst.replace(".", "_"));
                        self.alloca_buffer.push_str(&format!(
                            "  {} = alloca i8, i32 {}, align 16\n",
                            obj_alloca, total_size
                        ));

                        // Initialize Header
                        self.temp_counter += 1;
                        let header_ptr = format!("%header_ptr_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to %struct.ObjectHeader*",
                            header_ptr, obj_alloca
                        ));

                        self.temp_counter += 1;
                        let tid_ptr = format!("%tid_ptr_{}", self.temp_counter);
                        self.emit_line(&format!("{} = getelementptr inbounds %struct.ObjectHeader, %struct.ObjectHeader* {}, i32 0, i32 1", tid_ptr, header_ptr));
                        self.emit_line(&format!("store i16 {}, i16* {}", type_id, tid_ptr));

                        // Body pointer (header + 24)
                        self.temp_counter += 1;
                        let body_ptr_i8 = format!("%body_ptr_i8_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i8, i8* {}, i32 24",
                            body_ptr_i8, obj_alloca
                        ));

                        // Zero-initialize entire object (Header + Body) to be GC-safe
                        if !self.declared_functions.contains("llvm.memset.p0i8.i64") {
                            self.global_buffer.push_str(
                                "declare void @llvm.memset.p0i8.i64(i8*, i8, i64, i1 immarg)\n",
                            );
                            self.declared_functions
                                .insert("llvm.memset.p0i8.i64".to_string());
                        }
                        self.emit_line(&format!(
                            "call void @llvm.memset.p0i8.i64(i8* {}, i8 0, i64 {}, i1 0)",
                            obj_alloca, total_size
                        ));

                        self.temp_counter += 1;
                        let body_ptr = format!("%body_ptr_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = ptrtoint i8* {} to i64",
                            body_ptr, body_ptr_i8
                        ));

                        // Call runtime to finalize setup (age, etc.)
                        if !self.declared_functions.contains(RT_CLASS_NEW) {
                            self.global_buffer.push_str(&format!(
                                "declare i64 @{}(i32, i64, i64, i64*, i64) nounwind\n",
                                RT_CLASS_NEW
                            ));
                            self.declared_functions.insert(RT_CLASS_NEW.to_string());
                        }

                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        // call rt_class_new(type_id, body_size, ptr_count, offsets_ptr, stack_ptr)
                        self.emit_line(&format!(
                            "{} = call i64 @{}(i32 {}, i64 {}, i64 0, i64* null, i64 {})",
                            result_tmp, RT_CLASS_NEW, type_id, body_size as i64, body_ptr
                        ));

                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);

                        // Register as GC root
                        self.temp_counter += 1;
                        let cast_root = format!("%cast_root_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to i64*",
                            cast_root, obj_alloca
                        ));
                        self.emit_line(&format!("call void @rt_push_root(i64* {})", cast_root));
                        return;
                    } else {
                        // Heap Allocation
                        if !self.declared_functions.contains(RT_CLASS_NEW) {
                            self.global_buffer.push_str(&format!(
                                "declare i64 @{}(i32, i64, i64, i64*, i64) nounwind\n",
                                RT_CLASS_NEW
                            ));
                            self.declared_functions.insert(RT_CLASS_NEW.to_string());
                        }

                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @{}(i32 {}, i64 {}, i64 0, i64* null, i64 0)",
                            result_tmp, RT_CLASS_NEW, type_id, body_size as i64
                        ));

                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);
                        return;
                    }
                }

                if callee == RT_MAP_NEW {
                    let is_escaped = !dst.is_empty() && self.does_escape(func, dst);

                    if !is_escaped && !dst.is_empty() {
                        // Stack Allocate Map (48 bytes for members + 24 bytes for ObjectHeader)
                        // Total 72 bytes
                        let total_size = 48 + 24;
                        let obj_alloca = format!("%stack_obj_{}", dst.replace(".", "_"));
                        self.alloca_buffer.push_str(&format!(
                            "  {} = alloca i8, i32 {}, align 16\n",
                            obj_alloca, total_size
                        ));

                        // Initialize Header (type_id = TAG_OBJECT, mark_bit = 0)
                        self.temp_counter += 1;
                        let header_ptr = format!("%header_ptr_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to %struct.ObjectHeader*",
                            header_ptr, obj_alloca
                        ));

                        // Zero-initialize entire object (Header + Body)
                        if !self.declared_functions.contains("llvm.memset.p0i8.i64") {
                            self.global_buffer.push_str(
                                "declare void @llvm.memset.p0i8.i64(i8*, i8, i64, i1 immarg)\n",
                            );
                            self.declared_functions
                                .insert("llvm.memset.p0i8.i64".to_string());
                        }
                        self.emit_line(&format!(
                            "call void @llvm.memset.p0i8.i64(i8* {}, i8 0, i64 {}, i1 0)",
                            obj_alloca, total_size
                        ));

                        // Set type_id = TAG_OBJECT (7)
                        self.temp_counter += 1;
                        let tid_ptr = format!("%tid_ptr_{}", self.temp_counter);
                        self.emit_line(&format!("{} = getelementptr inbounds %struct.ObjectHeader, %struct.ObjectHeader* {}, i32 0, i32 1", tid_ptr, header_ptr));
                        self.emit_line(&format!("store i16 7, i16* {}", tid_ptr));

                        self.temp_counter += 1;
                        let body_ptr_i8 = format!("%body_ptr_i8_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = getelementptr i8, i8* {}, i32 24",
                            body_ptr_i8, obj_alloca
                        ));

                        self.temp_counter += 1;
                        let body_ptr = format!("%body_ptr_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = ptrtoint i8* {} to i64",
                            body_ptr, body_ptr_i8
                        ));

                        // Call constructor with stack address
                        if !self.declared_functions.contains(RT_MAP_NEW) {
                            self.global_buffer
                                .push_str(&format!("declare i64 @{}(i64) nounwind\n", RT_MAP_NEW));
                            self.declared_functions.insert(RT_MAP_NEW.to_string());
                        }

                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @{}(i64 {})",
                            result_tmp, RT_MAP_NEW, body_ptr
                        ));

                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);

                        // Register as GC root (important if it contains heap pointers)
                        self.temp_counter += 1;
                        let cast_root = format!("%cast_root_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to i64*",
                            cast_root, obj_alloca
                        ));
                        self.emit_line(&format!("call void @rt_push_root(i64* {})", cast_root));
                        return;
                    }

                    // Heap Allocation
                    if !self.declared_functions.contains(RT_MAP_NEW) {
                        self.global_buffer
                            .push_str(&format!("declare i64 @{}(i64) nounwind\n", RT_MAP_NEW));
                        self.declared_functions.insert(RT_MAP_NEW.to_string());
                    }

                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @{}(i64 0)", result_tmp, RT_MAP_NEW));

                    let ptr = self.resolve_ptr(dst);
                    self.store_ptr(&ptr, &result_tmp);
                    return;
                }

                if callee == "rt_box_int" || callee == "rt_box_boolean" || callee == "rt_box_char" {
                    let arg_val = self.resolve_value(&args[0]);
                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    // Primitives are now bitcasted directly into i64 slots (generic slots)
                    self.emit_line(&format!("{} = or i64 0, {}", result_tmp, arg_val));
                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);
                    }
                    return;
                }

                if callee == "rt_box_number" {
                    let arg_val = self.resolve_value(&args[0]);
                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    // Bitcast double -> i64 to store in generic slot
                    self.emit_line(&format!(
                        "{} = bitcast double {} to i64",
                        result_tmp, arg_val
                    ));
                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);
                    }
                    return;
                }

                if callee == "rt_mem_get_i64" {
                    let ptr_val = self.resolve_value(&args[0]);
                    let offset_val = self.resolve_value(&args[1]);
                    self.temp_counter += 1;
                    let addr_val = format!("%addr{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = add i64 {}, {}",
                        addr_val, ptr_val, offset_val
                    ));
                    self.temp_counter += 1;
                    let ptr_cast = format!("%ptr_cast{}", self.temp_counter);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i64*", ptr_cast, addr_val));
                    self.temp_counter += 1;
                    let res_val = format!("%res{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", res_val, ptr_cast));
                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &res_val);
                    }
                    return;
                }

                if callee == "rt_mem_set_i64" {
                    let ptr_val = self.resolve_value(&args[0]);
                    let offset_val = self.resolve_value(&args[1]);
                    let src_val = self.resolve_value(&args[2]);
                    self.temp_counter += 1;
                    let addr_val = format!("%addr{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = add i64 {}, {}",
                        addr_val, ptr_val, offset_val
                    ));
                    self.temp_counter += 1;
                    let ptr_cast = format!("%ptr_cast{}", self.temp_counter);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i64*", ptr_cast, addr_val));
                    self.emit_line(&format!("store i64 {}, i64* {}", src_val, ptr_cast));
                    return;
                }

                if callee == "rt_mem_get_f64" {
                    let ptr_val = self.resolve_value(&args[0]);
                    let offset_val = self.resolve_value(&args[1]);
                    self.temp_counter += 1;
                    let addr_val = format!("%addr{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = add i64 {}, {}",
                        addr_val, ptr_val, offset_val
                    ));
                    self.temp_counter += 1;
                    let ptr_cast = format!("%ptr_cast{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = inttoptr i64 {} to double*",
                        ptr_cast, addr_val
                    ));
                    self.temp_counter += 1;
                    let res_val = format!("%res{}", self.temp_counter);
                    self.emit_line(&format!("{} = load double, double* {}", res_val, ptr_cast));
                    if !dst.is_empty() {
                        self.float_ssa_vars.insert(dst.clone(), res_val.clone());
                        self.temp_counter += 1;
                        let bits_tmp = format!("%bits{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            bits_tmp, res_val
                        ));
                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &bits_tmp);
                    }
                    return;
                }

                if callee == "rt_mem_set_f64" {
                    let ptr_val = self.resolve_value(&args[0]);
                    let offset_val = self.resolve_value(&args[1]);
                    let src_float = self.resolve_float_value(&args[2]);

                    self.temp_counter += 1;
                    let addr_val = format!("%addr{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = add i64 {}, {}",
                        addr_val, ptr_val, offset_val
                    ));
                    self.temp_counter += 1;
                    let ptr_cast = format!("%ptr_cast{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = inttoptr i64 {} to double*",
                        ptr_cast, addr_val
                    ));
                    self.emit_line(&format!("store double {}, double* {}", src_float, ptr_cast));
                    return;
                }

                if callee.starts_with("std_math_") {
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
                            self.store_ptr(&ptr, &result_i);
                        }
                    } else {
                        // Fallback to runtime call for unsupported math functions (random, min, max)
                        let mut arg_vals = Vec::new();
                        for arg in args {
                            let arg_val = self.resolve_value(arg);
                            arg_vals.push(format!(
                                "{} {}",
                                Self::get_llvm_type(arg.get_type()),
                                arg_val
                            ));
                        }
                        let args_str = arg_vals.join(", ");

                        let ret_ty = if !dst.is_empty() {
                            func.variables.get(dst).cloned().unwrap_or(TejxType::Void)
                        } else {
                            TejxType::Void
                        };
                        let llvm_ret = Self::get_llvm_type(&ret_ty);

                        if !self.declared_functions.contains(callee) {
                            let decl_args = args
                                .iter()
                                .map(|a| Self::get_llvm_type(a.get_type()))
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.global_buffer.push_str(&format!(
                                "declare {} @{}({})\n",
                                llvm_ret, callee, decl_args
                            ));
                            self.declared_functions.insert(callee.clone());
                        }
                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call {} @{}({})",
                            result_tmp, llvm_ret, callee, args_str
                        ));

                        let final_val = if llvm_ret == "double" || llvm_ret == "float" {
                            self.temp_counter += 1;
                            let bitcast_tmp = format!("%bitcast_res{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = bitcast {} {} to i64",
                                bitcast_tmp,
                                llvm_ret,
                                result_tmp.clone()
                            ));
                            bitcast_tmp
                        } else if llvm_ret != "i64"
                            && llvm_ret != "void"
                            && !llvm_ret.ends_with('*')
                        {
                            self.temp_counter += 1;
                            let zext_tmp = format!("%zext_res{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = zext {} {} to i64",
                                zext_tmp,
                                llvm_ret,
                                result_tmp.clone()
                            ));
                            zext_tmp
                        } else {
                            result_tmp.clone()
                        };

                        if !dst.is_empty() {
                            if ret_ty.is_float() {
                                self.float_ssa_vars.insert(dst.clone(), result_tmp.clone());
                            }
                            let ptr = self.resolve_ptr(dst);
                            self.store_ptr(&ptr, &final_val);
                        }
                    }
                } else {
                    let mut call_args_info: Vec<(MIRValue, String)> = Vec::new();
                    for arg in args {
                        let arg_val = self.resolve_value(arg);
                        call_args_info.push((arg.clone(), arg_val));
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
                                if method == "join" && func.variables.get(base).map(|t| matches!(t, TejxType::Class(n, _) if n == "Thread" || n.starts_with("Thread<"))).unwrap_or(false) {
                                    final_callee = "f_Thread_join".to_string();
                                } else {
                                    // Resolve instance type to get class name dynamically
                                    let mut class_name = base.to_string();
                                    if let Some(ty) = func.variables.get(base) {
                                        match ty {
                                            TejxType::Class(name, _) => {
                                                if name.starts_with("Array<")
                                                    || name.ends_with("[]")
                                                {
                                                    class_name = "Array".to_string();
                                                } else if name.contains('<') {
                                                    // Generic class like Map<string, CacheNode>
                                                    // Extract base class name before '<'
                                                    class_name = name.split('<').next().unwrap_or(name).to_string();
                                                } else {
                                                    class_name = name.clone();
                                                }
                                            }
                                            TejxType::String => class_name = "String".to_string(),
                                            _ => {
                                                if ty.is_array() {
                                                    class_name = "Array".to_string();
                                                }
                                            }
                                        }
                                    }
                                    // Uniform method dispatch for all types: f_{class}_{method}
                                    // Methods are defined in prelude.tx, so new methods added there
                                    // are automatically available without compiler changes.
                                    if class_name == "Array" || class_name == "String" {
                                        final_callee = method.to_string();
                                    } else {
                                        final_callee = format!("f_{}_{}", class_name, method);
                                    }
                                }
                            } else {
                                final_callee = format!("f_{}_{}", base, method);
                            }
                        }
                    } else if let Some(ptr) = self.value_map.get(callee) {
                        let ptr_clone = ptr.clone();
                        self.temp_counter += 1;
                        let func_val_tmp = format!("%func_val_{}", self.temp_counter);
                        self.load_ptr(&ptr_clone, &func_val_tmp);
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
                        let _ = call_arg_vals.join(", ");
                    }

                    let mut call_args_info = Vec::new();
                    if is_instance_call {
                        if let Some(ptr) = self.value_map.get(&instance_var) {
                            let ptr_clone = ptr.clone();
                            self.temp_counter += 1;
                            let tmp = format!("%inst{}", self.temp_counter);
                            self.load_ptr(&ptr_clone, &tmp);
                            call_args_info.push((
                                MIRValue::Variable {
                                    name: instance_var.clone(),
                                    ty: TejxType::Class("any".to_string(), vec![]),
                                },
                                tmp,
                            ));
                        }
                    }

                    for arg in args {
                        let reg = if final_callee == "rt_string_from_c_str" {
                            if let MIRValue::Constant {
                                value,
                                ty: TejxType::String,
                            } = arg
                            {
                                self.emit_string_constant(value)
                            } else {
                                self.resolve_value(arg)
                            }
                        } else {
                            self.resolve_value(arg)
                        };
                        call_args_info.push((arg.clone(), reg));
                    }

                    let callee_expects_double_args = final_callee.starts_with("std_math_");
                    let callee_returns_double = callee_expects_double_args
                        || final_callee == "rt_to_number"
                        || final_callee == "rt_math_random";

                    let mut llvm_args = Vec::new();
                    let mut llvm_decl_args = Vec::new();

                    for (arg_mir, reg) in call_args_info {
                        let arg_ty = arg_mir.get_type();
                        let mut final_reg = reg;

                        // Ensure string literals are boxed when passed to functions (unless the function is rt_string_from_c_str)
                        if matches!(arg_ty, TejxType::String)
                            && final_reg.starts_with("ptrtoint")
                            && final_callee != "rt_string_from_c_str"
                        {
                            final_reg = self.emit_box_string(&final_reg);
                        }

                        if callee_expects_double_args {
                            self.temp_counter += 1;
                            let f_reg = format!("%f_arg_{}", self.temp_counter);
                            if arg_ty.is_float() {
                                self.emit_line(&format!(
                                    "{} = bitcast i64 {} to double",
                                    f_reg, final_reg
                                ));
                            } else {
                                self.emit_line(&format!(
                                    "{} = sitofp i64 {} to double",
                                    f_reg, final_reg
                                ));
                            }
                            llvm_args.push(format!("double {}", f_reg));
                            llvm_decl_args.push("double");
                        } else {
                            llvm_args.push(format!("i64 {}", final_reg));
                            llvm_decl_args.push("i64");
                        }
                    }

                    let args_str = llvm_args.join(", ");
                    let decl_args_str = llvm_decl_args.join(", ");
                    let ret_ty = if callee_returns_double {
                        "double"
                    } else {
                        "i64"
                    };

                    let use_quotes = !(final_callee.starts_with("rt_")
                        || final_callee.starts_with("tejx_")
                        || final_callee.starts_with("std_")
                        || final_callee == "printf"
                        || final_callee == "malloc"
                        || final_callee == "free");

                    let callee_symbol = if use_quotes
                        || final_callee.starts_with("f_")
                        || final_callee.starts_with("m_")
                    {
                        format!("@\"{}\"", final_callee)
                    } else {
                        format!("@{}", final_callee)
                    };

                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    if !self.declared_functions.contains(&final_callee) {
                        let pure_attrs = "";
                        let mut decl_ret = ret_ty;

                        // Fix: rt_promise_resolve/reject and others are void in runtime
                        if final_callee == "rt_promise_resolve"
                            || final_callee == "rt_promise_reject"
                            || final_callee == "rt_promise_await_resume"
                            || final_callee == "rt_Map_set"
                            || final_callee == "rt_Map_merge"
                        {
                            decl_ret = "void";
                        }

                        self.global_buffer.push_str(&format!(
                            "declare {} {}({}){}\n",
                            decl_ret, callee_symbol, decl_args_str, pure_attrs
                        ));
                        self.declared_functions.insert(final_callee.clone());
                    }

                    if final_callee == "rt_promise_resolve"
                        || final_callee == "rt_promise_reject"
                        || final_callee == "rt_promise_await_resume"
                        || final_callee == "rt_Map_set"
                        || final_callee == "rt_Map_merge"
                    {
                        self.emit_line(&format!("call void {}({})", callee_symbol, args_str));
                    } else {
                        self.emit_line(&format!(
                            "{} = call {} {}({})",
                            result_tmp, ret_ty, callee_symbol, args_str
                        ));
                    }

                    let mut final_result = result_tmp.clone();
                    // If it was a void call, we still might have a 'dst' that expects a value.
                    // Store 0 as a dummy if needed, though well-formed MIR shouldn't do this.
                    if final_callee == "rt_promise_resolve"
                        || final_callee == "rt_promise_reject"
                        || final_callee == "rt_promise_await_resume"
                        || final_callee == "rt_Map_set"
                        || final_callee == "rt_Map_merge"
                    {
                        final_result = "0".to_string();
                    }
                    if callee_returns_double {
                        self.temp_counter += 1;
                        let i_res = format!("%i_res_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = bitcast double {} to i64",
                            i_res, result_tmp
                        ));
                        final_result = i_res;
                    }
                    let result_tmp = final_result;

                    // --- INVALDIATION: Remove cached data pointers if array could reallocate ---
                    // To remain generic without hardcoded arrays of methods, we conservatively
                    // invalidate the pointer if it's passed to any method call.
                    if is_instance_call {
                        self.heap_array_ptrs.remove(&instance_var);
                    } else if !args.is_empty() {
                        if let MIRValue::Variable { name, .. } = &args[0] {
                            self.heap_array_ptrs.remove(name);
                        }
                    }

                    // --- FILL PROPAGATION: f_Array_fill returns the same array ---
                    // Propagate cached data pointer from args[0] to dst so that
                    // `let A = new Array(N).fill(1.0)` chains keep the fast path.
                    if (final_callee.ends_with("_fill")) && !dst.is_empty() {
                        if let Some(MIRValue::Variable { name, .. }) = args.first() {
                            if let Some(info) = self.heap_array_ptrs.get(name).cloned() {
                                self.heap_array_ptrs.insert(dst.to_string(), info);
                            }
                        }
                    }

                    // --- Ownership Transfer: Mark consumed arguments as Moved ---
                    // Dynamic stdlib detection: functions from prelude/runtime don't consume args
                    let is_stdlib = final_callee.starts_with("f_")
                        || final_callee.starts_with("rt_")
                        || final_callee.starts_with("tejx_")
                        || final_callee.starts_with("m_");

                    let is_method = final_callee.starts_with("m_");
                    let is_constructor = final_callee.ends_with("_constructor");
                    // Generic container mutator detection: any method that stores a value
                    let is_container_mutator = final_callee.ends_with("_push")
                        || final_callee.ends_with("_set")
                        || final_callee.ends_with("_add")
                        || final_callee.ends_with("_enqueue")
                        || final_callee.ends_with("_unshift");

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
                        if (final_callee == RT_PROMISE_RESOLVE || final_callee == RT_PROMISE_REJECT)
                            && (i == 0 || i == 1)
                        {
                            should_consume = true;
                        }

                        // Fix: The arguments bundle passed to a task MUST be consumed (moved to the task queue).
                        if final_callee == TEJX_ENQUEUE_TASK && i == 1 {
                            should_consume = true;
                        }

                        // Container mutators consume value args (not the container itself at arg[0])
                        if is_container_mutator && i > 0 {
                            should_consume = true;
                        }

                        if should_consume {}
                    }
                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.store_ptr(&ptr, &result_tmp);
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
                    "{} = call i64 @{}(i64 {})",
                    err_obj, RT_STRING_FROM_C_STR, err_msg
                ));
                self.emit_line(&format!("call void @{}(i64 {})", TEJX_THROW, err_obj));
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
                // Pad to at least 5 total arguments (env + 4)
                while arg_types.len() < 5 {
                    arg_types.push("i64");
                }
                let ptr_args = arg_types.join(", ");
                self.emit_line(&format!(
                    "{} = inttoptr i64 {} to i64 ({})*",
                    func_ptr_tmp, ptr_reg, ptr_args
                ));

                let mut arg_vals = vec![format!("i64 {}", env_reg)];
                for arg in args {
                    let mut val = self.resolve_value(arg);
                    let arg_ty = arg.get_type();
                    if matches!(arg_ty, TejxType::String) && val.starts_with("ptrtoint") {
                        val = self.emit_box_string(&val);
                    }
                    arg_vals.push(format!("i64 {}", val));
                }
                // Pad with zeros to reach minimum 5 arguments
                while arg_vals.len() < 5 {
                    arg_vals.push("i64 0".to_string());
                }
                let args_str = arg_vals.join(", ");

                self.temp_counter += 1;
                let result_tmp = format!("%call{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 {}({})",
                    result_tmp, func_ptr_tmp, args_str
                ));

                if !dst.is_empty() {
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                    self.emit_store_variable(dst, &result_tmp, dst_ty);
                }
            }
            MIRInstruction::LoadMember {
                dst, obj, member, ..
            } => {
                let obj_val = self.resolve_value(obj);
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);

                let mut used_fast = false;
                if member == "length" {
                    // Use rt_len for .length access
                    if !self.declared_functions.contains("rt_len") {
                        self.global_buffer.push_str("declare i64 @rt_len(i64)\n");
                        self.declared_functions.insert("rt_len".to_string());
                    }
                    self.emit_line(&format!("{} = call i64 @rt_len(i64 {})", res_tmp, obj_val));
                    used_fast = true;
                } else {
                    if let TejxType::Class(class_name, _) = obj.get_type() {
                        let lookup_name = if class_name.contains('<') {
                            class_name.split('<').next().unwrap()
                        } else {
                            class_name
                        };
                        let field_info = self.class_fields.get(lookup_name).and_then(|fields| {
                            fields.iter().position(|(f, _)| f == member).map(|pos| {
                                let mut offset = 0;
                                for (_, ty) in &fields[..pos] {
                                    offset = Self::get_aligned_offset(offset, ty);
                                    offset += ty.size();
                                }
                                let field_ty = fields[pos].1.clone();
                                offset = Self::get_aligned_offset(offset, &field_ty);
                                (offset, field_ty)
                            })
                        });

                        if let Some((offset, field_ty)) = field_info {
                            let llvm_ty = match field_ty {
                                TejxType::Bool => "i8",
                                TejxType::Int16 | TejxType::Float16 => "i16",
                                TejxType::Int32 | TejxType::Float32 => "i32",
                                _ => "i64",
                            };

                            let ptr_reg = format!("%ptr_{}", self.temp_counter);
                            self.temp_counter += 1;
                            let raw_obj = self.emit_strip_heap_offset(&obj_val);
                            self.emit_line(&format!(
                                "{} = inttoptr i64 {} to i8*",
                                ptr_reg, raw_obj
                            ));

                            let field_ptr = format!("%field_ptr_{}", self.temp_counter);
                            self.temp_counter += 1;
                            self.emit_line(&format!(
                                "{} = getelementptr i8, i8* {}, i32 {}",
                                field_ptr, ptr_reg, offset
                            ));

                            let typed_field_ptr = format!("%typed_field_ptr_{}", self.temp_counter);
                            self.temp_counter += 1;
                            self.emit_line(&format!(
                                "{} = bitcast i8* {} to {}*",
                                typed_field_ptr, field_ptr, llvm_ty
                            ));

                            let loaded_val = format!("%loaded_val_{}", self.temp_counter);
                            self.temp_counter += 1;
                            self.emit_line(&format!(
                                "{} = load {}, {}* {}",
                                loaded_val, llvm_ty, llvm_ty, typed_field_ptr
                            ));

                            // Extend to i64 for the general register system
                            if llvm_ty == "i64" {
                                self.emit_line(&format!(
                                    "{} = bitcast i64 {} to i64",
                                    res_tmp, loaded_val
                                ));
                            } else {
                                self.emit_line(&format!(
                                    "{} = zext {} {} to i64",
                                    res_tmp, llvm_ty, loaded_val
                                ));
                            }

                            used_fast = true;
                        }
                    }

                    if !used_fast {
                        let k_val = self.resolve_value(&MIRValue::Constant {
                            value: format!("\"{}\"", member),
                            ty: TejxType::String,
                        });
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
                    }
                }

                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                let mut needs_unboxing = !used_fast;

                if !used_fast {
                    if let TejxType::Class(class_name, _) = obj.get_type() {
                        let lookup_name = if class_name.contains('<') {
                            class_name.split('<').next().unwrap()
                        } else {
                            class_name
                        };
                        if let Some(fields) = self.class_fields.get(lookup_name) {
                            if let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == member) {
                                if (field_ty.is_numeric()
                                    || matches!(field_ty, TejxType::Bool | TejxType::Char))
                                    && field_ty == dst_ty
                                {
                                    needs_unboxing = false;
                                }
                            }
                        }
                    }
                }

                let final_res = if needs_unboxing && dst_ty.is_numeric() && !dst_ty.is_float() {
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
                } else if needs_unboxing && dst_ty.is_float() {
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
                } else if needs_unboxing && matches!(dst_ty, TejxType::Bool) {
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

                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &final_res, dst_ty);
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
                let v_ty = src.get_type();
                if matches!(v_ty, TejxType::String) && v_val.starts_with("ptrtoint") {
                    v_val = self.emit_box_string(&v_val);
                }

                // Only box if target is not a known primitive field of a class
                let mut needs_boxing = true;
                let mut used_fast_store = false;
                if let TejxType::Class(class_name, _) = obj.get_type() {
                    let lookup_name = if class_name.contains('<') {
                        class_name.split('<').next().unwrap()
                    } else {
                        class_name
                    };
                    let field_info = self.class_fields.get(lookup_name).and_then(|fields| {
                        fields.iter().position(|(f, _)| f == member).map(|pos| {
                            let mut offset = 0;
                            for (_, ty) in &fields[..pos] {
                                offset = Self::get_aligned_offset(offset, ty);
                                offset += ty.size();
                            }
                            let field_ty = fields[pos].1.clone();
                            offset = Self::get_aligned_offset(offset, &field_ty);

                            (field_ty, offset)
                        })
                    });

                    if let Some((field_ty, offset)) = field_info {
                        needs_boxing = false;

                        let llvm_ty = match field_ty {
                            TejxType::Bool => "i8",
                            TejxType::Int16 | TejxType::Float16 => "i16",
                            TejxType::Int32 | TejxType::Float32 => "i32",
                            _ => "i64",
                        };

                        let ptr_reg = format!("%ptr_store_{}", self.temp_counter);
                        self.temp_counter += 1;
                        let raw_obj = self.emit_strip_heap_offset(&obj_val);
                        self.emit_line(&format!("{} = inttoptr i64 {} to i8*", ptr_reg, raw_obj));

                        let field_ptr = format!("%field_ptr_store_{}", self.temp_counter);
                        self.temp_counter += 1;
                        self.emit_line(&format!(
                            "{} = getelementptr i8, i8* {}, i32 {}",
                            field_ptr, ptr_reg, offset
                        ));

                        let typed_field_ptr =
                            format!("%typed_field_ptr_store_{}", self.temp_counter);
                        self.temp_counter += 1;
                        self.emit_line(&format!(
                            "{} = bitcast i8* {} to {}*",
                            typed_field_ptr, field_ptr, llvm_ty
                        ));

                        let truncated_val = format!("%trunc_val_store_{}", self.temp_counter);
                        self.temp_counter += 1;
                        if llvm_ty == "i64" {
                            self.emit_line(&format!(
                                "{} = bitcast i64 {} to i64",
                                truncated_val, v_val
                            ));
                        } else {
                            self.emit_line(&format!(
                                "{} = trunc i64 {} to {}",
                                truncated_val, v_val, llvm_ty
                            ));
                        }
                        self.emit_line(&format!(
                            "store {} {}, {}* {}",
                            llvm_ty, truncated_val, llvm_ty, typed_field_ptr
                        ));
                        used_fast_store = true;
                    }
                }

                // If not a static class field, check if it's an array index
                if !used_fast_store && obj.get_type().is_array() {
                    if let Ok(idx) = member.parse::<i64>() {
                        if !self.declared_functions.contains("rt_array_set_fast") {
                            self.global_buffer.push_str(
                                "declare void @rt_array_set_fast(i64, i64, i64) nounwind\n",
                            );
                            self.declared_functions
                                .insert("rt_array_set_fast".to_string());
                        }
                        self.emit_line(&format!(
                            "call void @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                            obj_val, idx, v_val
                        ));
                        used_fast_store = true;
                    }
                }

                if !used_fast_store
                    && needs_boxing
                    && (v_ty.is_numeric() || matches!(v_ty, TejxType::Bool | TejxType::Char))
                {
                    self.temp_counter += 1;
                    let boxed_reg = format!("%boxed_set_{}", self.temp_counter);

                    // Primitives are now bitcasted directly into i64 slots (generic slots)
                    if v_ty.is_float() {
                        self.emit_line(&format!("{} = bitcast double {} to i64", boxed_reg, v_val));
                    } else {
                        self.emit_line(&format!("{} = or i64 0, {}", boxed_reg, v_val));
                    }
                    v_val = boxed_reg;
                }

                if !used_fast_store {
                    if !self.declared_functions.contains("rt_Map_set") {
                        self.global_buffer
                            .push_str("declare void @rt_Map_set(i64, i64, i64) nounwind\n");
                        self.declared_functions.insert("rt_Map_set".to_string());
                    }
                    self.emit_line(&format!(
                        "call void @rt_Map_set(i64 {}, i64 {}, i64 {})",
                        obj_val, k_val, v_val
                    ));
                }
            }
            MIRInstruction::LoadIndex {
                dst, obj, index, ..
            } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);

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
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &res_tmp, dst_ty);
            }
            MIRInstruction::StoreIndex {
                obj, index, src, ..
            } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                let v_val = self.resolve_value(src);

                if !self.declared_functions.contains("rt_array_set_fast") {
                    self.global_buffer
                        .push_str("declare void @rt_array_set_fast(i64, i64, i64) nounwind\n");
                    self.declared_functions
                        .insert("rt_array_set_fast".to_string());
                }
                self.emit_line(&format!(
                    "call void @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                    obj_val, idx_val, v_val
                ));
            }
            MIRInstruction::Throw { value, .. } => {
                if self.num_roots > 0 {
                    if !self.declared_functions.contains("rt_pop_roots") {
                        self.global_buffer
                            .push_str("declare void @rt_pop_roots(i64) nounwind\n");
                        self.declared_functions.insert("rt_pop_roots".to_string());
                    }
                    self.emit_line(&format!("call void @rt_pop_roots(i64 {})", self.num_roots));
                }
                let val = self.resolve_value(value);
                self.emit_line(&format!("call void @tejx_throw(i64 {})", val));
                self.emit_line("unreachable");
            }
            MIRInstruction::Cast { dst, src, ty, .. } => {
                let s = self.resolve_value(src);
                let src_ty = src.get_type();

                self.temp_counter += 1;
                let tmp = format!("%cast{}", self.temp_counter);

                if src_ty.is_numeric() && ty.is_numeric() {
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
                        self.emit_line(&format!("{} = add i64 {}, 0", tmp, s));
                    }
                } else if ty.is_numeric() {
                    if !self.declared_functions.contains("rt_to_number_v2") {
                        self.global_buffer
                            .push_str("declare i64 @rt_to_number_v2(i64)\n");
                        self.declared_functions
                            .insert("rt_to_number_v2".to_string());
                    }
                    self.temp_counter += 1;
                    let num_val = format!("%num_val{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_number_v2(i64 {})",
                        num_val, s
                    ));

                    if ty.is_float() {
                        self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, num_val));
                    } else {
                        self.temp_counter += 1;
                        let f_val = format!("%f_val{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i64 {} to double", f_val, num_val));
                        self.emit_line(&format!("{} = fptosi double {} to i64", tmp, f_val));
                    }
                } else if matches!(ty, TejxType::Bool) {
                    if !self.declared_functions.contains("rt_to_boolean") {
                        self.global_buffer
                            .push_str("declare i64 @rt_to_boolean(i64)\n");
                        self.declared_functions.insert("rt_to_boolean".to_string());
                    }
                    self.emit_line(&format!("{} = call i64 @rt_to_boolean(i64 {})", tmp, s));
                } else if matches!(ty, TejxType::String)
                    && !src_ty.is_numeric()
                    && !matches!(src_ty, TejxType::Bool)
                {
                    if !self.declared_functions.contains("rt_to_string") {
                        self.global_buffer
                            .push_str("declare i64 @rt_to_string(i64)\n");
                        self.declared_functions.insert("rt_to_string".to_string());
                    }
                    self.emit_line(&format!("{} = call i64 @rt_to_string(i64 {})", tmp, s));
                } else {
                    // Generic bitcast for other types
                    self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, s));
                }
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &tmp, dst_ty);
            }
        }
    }

    fn emit_string_constant(&mut self, value: &str) -> String {
        self.label_counter += 1;
        let str_lbl = format!("@.str{}", self.label_counter);

        let raw_content = value.to_string();
        let content =
            if raw_content.len() >= 2 && raw_content.starts_with('"') && raw_content.ends_with('"')
            {
                &raw_content[1..raw_content.len() - 1]
            } else {
                &raw_content
            };

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

        format!("ptrtoint ([{} x i8]* {} to i64)", byte_len, str_lbl)
    }

    fn emit_box_string(&mut self, raw_ptr: &str) -> String {
        if !self.declared_functions.contains("rt_string_from_c_str") {
            self.global_buffer
                .push_str("declare i64 @rt_string_from_c_str(i64)\n");
            self.declared_functions
                .insert("rt_string_from_c_str".to_string());
        }
        self.temp_counter += 1;
        let boxed = format!("%boxed_str{}", self.temp_counter);
        self.emit_line(&format!(
            "{} = call i64 @rt_string_from_c_str(i64 {})",
            boxed, raw_ptr
        ));
        boxed
    }
}
