use super::*;
use crate::intrinsics::*;
use crate::mir::*;
use crate::types::TejxType;

impl CodeGen {
    pub(crate) fn emit_strip_heap_offset(&mut self, val: &str) -> String {
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

    pub(crate) fn store_ptr(&mut self, ptr: &str, src_val: &str) {
        let llvm_ty = self.ptr_types.get(ptr).map(|s| s.as_str()).unwrap_or("i64");
        self.buffer.push_str(&format!(
            "  store {} {}, {}* {}\n",
            llvm_ty, src_val, llvm_ty, ptr
        ));
    }

    pub(crate) fn load_ptr(&mut self, ptr: &str, dest_reg: &str) {
        let llvm_ty = self.ptr_types.get(ptr).map(|s| s.as_str()).unwrap_or("i64");
        self.buffer.push_str(&format!(
            "  {} = load {}, {}* {}\n",
            dest_reg, llvm_ty, llvm_ty, ptr
        ));
    }

    pub(crate) fn emit_abi_cast(
        &mut self,
        val_name: &str,
        src_ty: &TejxType,
        dst_ty: &TejxType,
    ) -> String {
        let src_llvm = Self::get_llvm_type(src_ty);
        let dst_llvm = Self::get_llvm_type(dst_ty);

        // Cannot cast to/from void — just pass through
        if src_llvm == "void" || dst_llvm == "void" {
            return val_name.to_string();
        }

        if src_llvm == dst_llvm {
            return val_name.to_string();
        }

        self.temp_counter += 1;
        let cast_reg = format!("%cast_{}", self.temp_counter);

        match (src_llvm, dst_llvm) {
            ("i64", "i32")
            | ("i64", "i16")
            | ("i64", "i1")
            | ("i32", "i16")
            | ("i32", "i1")
            | ("i16", "i1") => {
                self.emit_line(&format!(
                    "{} = trunc {} {} to {}",
                    cast_reg, src_llvm, val_name, dst_llvm
                ));
            }
            ("i1", "i16") | ("i1", "i32") | ("i1", "i64") => {
                self.emit_line(&format!(
                    "{} = zext {} {} to {}",
                    cast_reg, src_llvm, val_name, dst_llvm
                ));
            }
            ("i16", "i32") | ("i16", "i64") | ("i32", "i64") => {
                self.emit_line(&format!(
                    "{} = sext {} {} to {}",
                    cast_reg, src_llvm, val_name, dst_llvm
                ));
            }
            ("double", "i64") => {
                self.emit_line(&format!(
                    "{} = bitcast double {} to i64",
                    cast_reg, val_name
                ));
            }
            ("i64", "double") => {
                self.emit_line(&format!(
                    "{} = bitcast i64 {} to double",
                    cast_reg, val_name
                ));
            }
            ("float", "double") => {
                self.emit_line(&format!(
                    "{} = fpext float {} to double",
                    cast_reg, val_name
                ));
            }
            ("double", "float") => {
                self.emit_line(&format!(
                    "{} = fptrunc double {} to float",
                    cast_reg, val_name
                ));
            }
            ("i32", "double")
            | ("i16", "double")
            | ("i1", "double")
            | ("i32", "float")
            | ("i16", "float")
            | ("i1", "float") => {
                self.emit_line(&format!(
                    "{} = sitofp {} {} to {}",
                    cast_reg, src_llvm, val_name, dst_llvm
                ));
            }
            ("double", "i32")
            | ("double", "i16")
            | ("double", "i1")
            | ("float", "i32")
            | ("float", "i16")
            | ("float", "i1") => {
                self.emit_line(&format!(
                    "{} = fptosi {} {} to {}",
                    cast_reg, src_llvm, val_name, dst_llvm
                ));
            }
            _ => {
                if src_llvm.contains('*') && dst_llvm == "i64" {
                    self.emit_line(&format!(
                        "{} = ptrtoint {} {} to i64",
                        cast_reg, src_llvm, val_name
                    ));
                } else if src_llvm == "i64" && dst_llvm.contains('*') {
                    self.emit_line(&format!(
                        "{} = inttoptr i64 {} to {}",
                        cast_reg, val_name, dst_llvm
                    ));
                } else {
                    self.emit_line(&format!(
                        "{} = bitcast {} {} to {}",
                        cast_reg, src_llvm, val_name, dst_llvm
                    ));
                }
            }
        }
        cast_reg
    }

    pub(crate) fn resolve_float_value(&mut self, val: &MIRValue) -> String {
        if let MIRValue::Variable { name, ty } = val {
            if ty.is_float() {
                if let Some(ssa_var) = self.float_ssa_vars.get(name) {
                    return ssa_var.clone(); // Found direct double representation
                }
            }
        }

        // Fallback: resolve normal value and convert based on type
        let normal_val = self.resolve_value(val);
        let ty = val.get_type();

        self.temp_counter += 1;
        let float_val = format!("%float_conv_{}", self.temp_counter);

        match ty {
            TejxType::Float32 => {
                // Float32 -> double via fpext
                self.emit_line(&format!(
                    "{} = fpext float {} to double",
                    float_val, normal_val
                ));
                float_val
            }
            TejxType::Float64 => {
                // Float64 is already a double in LLVM, just return it
                normal_val
            }
            _ if ty.is_float() => {
                // Other floats? (Float16 etc) - Shouldn't happen, but fallback
                self.emit_line(&format!(
                    "{} = bitcast i64 {} to double",
                    float_val, normal_val
                ));
                float_val
            }
            _ => {
                // Integer -> double via sitofp
                self.emit_line(&format!(
                    "{} = sitofp i64 {} to double",
                    float_val, normal_val
                ));
                float_val
            }
        }
    }

    pub(crate) fn resolve_value(&mut self, val: &MIRValue) -> String {
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

                    // Wrap function pointer into a closure object using rt_closure_from_ptr
                    self.declare_runtime_fn(
                        "rt_closure_from_ptr",
                        "i64 @rt_closure_from_ptr(i64) nounwind",
                    );
                    self.declare_runtime_fn(
                        "rt_array_set_fast",
                        "void @rt_array_set_fast(i64, i64, i64)",
                    );

                    self.temp_counter += 1;
                    let closure_id = format!("%closure{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_closure_from_ptr(i64 {})",
                        closure_id, fn_ptr
                    ));

                    // Set env (slot 1) — rt_closure_from_ptr already sets fn_ptr at slot 0
                    let env_to_pass = if let Some(env) = self.current_env.clone() {
                        env
                    } else {
                        // Create a fresh empty environment (array) if the parent doesn't have one
                        self.declare_runtime_fn(
                            "rt_Array_new_fixed",
                            "i64 @rt_Array_new_fixed(i64, i64)",
                        );
                        self.temp_counter += 1;
                        let fresh_env = format!("%fresh_env{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_Array_new_fixed(i64 0, i64 8)",
                            fresh_env
                        ));
                        fresh_env
                    };

                    self.emit_line(&format!(
                        "call void @rt_array_set_fast(i64 {}, i64 1, i64 {})",
                        closure_id, env_to_pass
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
                        return format!("0x{:016X}", d.to_bits());
                    }
                    return "0.0".to_string();
                }

                if value == "null" || (value == "0" && Self::is_gc_managed(ty)) {
                    return "0".to_string();
                }

                if matches!(ty, TejxType::Void) && value == "0" {
                    return "0".to_string();
                }

                let raw_ptr = self.emit_string_constant(value);
                self.declare_runtime_fn("rt_string_from_c_str", "i64 @rt_string_from_c_str(i64)");
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
                if let Some(cap_idx) = self.get_captured_index(name) {
                    if let Some(env) = self.current_env.clone() {
                        self.declare_runtime_fn(
                            "rt_array_get_fast",
                            "i64 @rt_array_get_fast(i64, i64)",
                        );

                        self.temp_counter += 1;
                        let val_tmp = format!("%val_tmp_{}", self.temp_counter);
                        self.emit_line(&format!(
                            "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
                            val_tmp, env, cap_idx
                        ));

                        // Unbox float if needed
                        if ty.is_float() {
                            self.temp_counter += 1;
                            let f_tmp = format!("%f_tmp_{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = bitcast i64 {} to double",
                                f_tmp, val_tmp
                            ));
                            return f_tmp;
                        }

                        // Cast back to correct primitive type
                        if ty.is_numeric() || matches!(ty, TejxType::Bool | TejxType::Char) {
                            return self.emit_abi_cast(&val_tmp, &TejxType::Int64, ty);
                        }

                        return val_tmp;
                    }
                    return "0".to_string();
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

    pub(crate) fn resolve_ptr(&mut self, name: &str) -> String {
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

    pub(crate) fn emit_store_variable(&mut self, name: &str, val: &str, ty: &TejxType) {
        if let Some(cap_idx) = self.get_captured_index(name) {
            if let Some(env) = self.current_env.clone() {
                self.declare_runtime_fn(
                    "rt_array_set_fast",
                    "void @rt_array_set_fast(i64, i64, i64)",
                );

                let val_to_store = if ty.is_float() {
                    self.temp_counter += 1;
                    let bits = format!("%fbits_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast double {} to i64", bits, val));
                    bits
                } else if ty.is_numeric() || *ty == TejxType::Bool || *ty == TejxType::Char {
                    let casted = self.emit_abi_cast(val, ty, &TejxType::Int64);
                    self.temp_counter += 1;
                    let tmp = format!("%zext_{}", self.temp_counter);
                    self.emit_line(&format!("{} = or i64 0, {}", tmp, casted));
                    tmp
                } else {
                    val.to_string()
                };

                self.emit_line(&format!(
                    "call void @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                    env, cap_idx, val_to_store
                ));
                return;
            }
        }
        {
            // Only store to local ptr if it's NOT a captured variable
            let ptr = self.resolve_ptr(name);
            // Cast the value to match the alloca's LLVM type if they differ
            let ptr_llvm_ty = self
                .ptr_types
                .get(&ptr)
                .cloned()
                .unwrap_or_else(|| "i64".to_string());
            let val_llvm_ty = Self::get_llvm_type(ty);
            let final_val = if val_llvm_ty != ptr_llvm_ty && ptr_llvm_ty != "void" {
                // Need to cast from the value's type to the alloca's type
                let ptr_ty_enum = match ptr_llvm_ty.as_str() {
                    "i1" => TejxType::Bool,
                    "i16" => TejxType::Int16,
                    "i32" => TejxType::Int32,
                    "float" => TejxType::Float32,
                    "double" => TejxType::Float64,
                    _ => TejxType::Int64,
                };
                self.emit_abi_cast(val, ty, &ptr_ty_enum)
            } else {
                val.to_string()
            };
            self.store_ptr(&ptr, &final_val);
        }
    }

    pub(crate) fn emit_string_constant(&mut self, value: &str) -> String {
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

    pub(crate) fn emit_box_string(&mut self, raw_ptr: &str) -> String {
        self.declare_runtime_fn("rt_string_from_c_str", "i64 @rt_string_from_c_str(i64)");
        self.temp_counter += 1;
        let boxed = format!("%boxed_str{}", self.temp_counter);
        self.emit_line(&format!(
            "{} = call i64 @rt_string_from_c_str(i64 {})",
            boxed, raw_ptr
        ));
        boxed
    }
}
