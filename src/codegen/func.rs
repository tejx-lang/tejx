use super::*;
use crate::intrinsics::*;
use crate::mir::*;
use crate::types::TejxType;
use std::collections::HashSet;

impl CodeGen {
    pub(crate) fn does_escape(&self, func: &MIRFunction, var_name: &str) -> bool {
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

    pub(crate) fn needs_arena(&self, func: &MIRFunction) -> bool {
        for bb in &func.blocks {
            for inst in &bb.instructions {
                if let MIRInstruction::Call {
                    callee, args, dst, ..
                } = inst
                {
                    if callee == RT_MAP_NEW
                        || callee == "f_Array_constructor"
                        || callee == "rt_Array_new_fixed"
                        || callee == "f_Function_constructor"
                    {
                        return true;
                    }
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
                        if body_size > 64 {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    pub(crate) fn get_captured_index(&self, name: &str) -> Option<usize> {
        if let Some(pos) = self.captured_vars.iter().position(|c| c == name) {
            return Some(pos);
        }
        // Handle MIR mangling suffixes like _123
        for (i, cap) in self.captured_vars.iter().enumerate() {
            if name.starts_with(cap)
                && (name.len() == cap.len() || name[cap.len()..].starts_with('_'))
            {
                return Some(i);
            }
        }
        None
    }

    pub(crate) fn is_captured(&self, name: &str) -> bool {
        self.get_captured_index(name).is_some()
    }

    pub fn generate_with_blocks(
        &mut self,
        functions: &[MIRFunction],
        captured_vars: HashSet<String>,
    ) -> String {
        let mut sorted_captured: Vec<String> = captured_vars.into_iter().collect();
        sorted_captured.sort();
        self.captured_vars = sorted_captured;
        self.buffer.clear();
        self.global_buffer.clear();
        self.declared_functions.clear();
        self.declared_globals.clear();

        // Module headers
        self.global_buffer.push_str("; ModuleID = 'tejx_module'\n");
        self.global_buffer
            .push_str("source_filename = \"tejx_module\"\n");
        let (datalayout, triple) = Self::get_target_info();
        self.global_buffer
            .push_str(&format!("target datalayout = \"{}\"\n", datalayout));
        self.global_buffer
            .push_str(&format!("target triple = \"{}\"\n", triple));

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

        self.declare_runtime_fn(
            "rt_register_type",
            "void @rt_register_type(i32, i64, i64, i64*, i8*)",
        );

        // Pre-declare commonly used runtime functions
        self.declare_runtime_fn("rt_class_new", "i64 @rt_class_new(i8*)");
        self.declare_runtime_fn("rt_len", "i64 @rt_len(i64)");
        self.declare_runtime_fn("rt_typeof", "i64 @rt_typeof(i64)");
        self.declare_runtime_fn("rt_to_string", "i64 @rt_to_string(i64)");
        self.declare_runtime_fn("rt_str_concat_v2", "i64 @rt_str_concat_v2(i64, i64)");
        self.declare_runtime_fn("rt_str_equals", "i64 @rt_str_equals(i64, i64)");
        self.declare_runtime_fn("rt_box_int", "i64 @rt_box_int(i64)");
        self.declare_runtime_fn("rt_box_number", "i64 @rt_box_number(double)");
        self.declare_runtime_fn("rt_unbox_int", "i64 @rt_unbox_int(i64)");
        self.declare_runtime_fn("rt_unbox_number", "double @rt_unbox_number(i64)");
        self.declare_runtime_fn("rt_array_push", "i64 @rt_array_push(i64, i64)");
        self.declare_runtime_fn("rt_array_pop", "i64 @rt_array_pop(i64)");
        self.declare_runtime_fn("rt_array_get_fast", "i64 @rt_array_get_fast(i64, i64)");
        self.declare_runtime_fn(
            "rt_array_set_fast",
            "void @rt_array_set_fast(i64, i64, i64)",
        );
        self.declare_runtime_fn("rt_object_get", "i64 @rt_object_get(i64, i8*)");
        self.declare_runtime_fn("rt_object_set", "void @rt_object_set(i64, i8*, i64)");
        self.declare_runtime_fn("rt_is_nullish", "i64 @rt_is_nullish(i64)");
        self.declare_runtime_fn("rt_not", "i64 @rt_not(i64)");
        self.declare_runtime_fn("rt_panic", "void @rt_panic(i64)");
        self.declare_runtime_fn("rt_Arena_create", "i64 @rt_Arena_create()");
        self.declare_runtime_fn("rt_Arena_destroy", "void @rt_Arena_destroy(i64)");
        self.declare_runtime_fn("rt_closure_from_ptr", "i64 @rt_closure_from_ptr(i64)");

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
        self.declare_runtime_fn("_setjmp", "i32 @_setjmp(i8*) returns_twice");
        self.declare_runtime_fn(
            TEJX_PUSH_HANDLER,
            &format!("void @{}(i8*)", TEJX_PUSH_HANDLER),
        );
        self.declare_runtime_fn(TEJX_POP_HANDLER, &format!("void @{}()", TEJX_POP_HANDLER));
        self.declare_runtime_fn(TEJX_THROW, &format!("void @{}(i64)", TEJX_THROW));
        self.declare_runtime_fn(
            TEJX_GET_EXCEPTION,
            &format!("i64 @{}()", TEJX_GET_EXCEPTION),
        );
        self.declare_runtime_fn("rt_string_from_c_str", "i64 @rt_string_from_c_str(i64)");

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

    pub(crate) fn gen_function_v2(&mut self, func: &MIRFunction) {
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

        let ret_llvm_ty = Self::get_llvm_type(&func.return_type);

        if func.is_extern {
            let decl_params = if func.params.is_empty() {
                String::new()
            } else {
                func.params
                    .iter()
                    .map(|p| {
                        let ty = func.variables.get(p).unwrap_or(&TejxType::Void);
                        Self::get_llvm_type(ty).to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            self.declare_runtime_fn(
                &func.name,
                &format!("{} @\"{}\"({})", ret_llvm_ty, func.name, decl_params),
            );
            return;
        }

        // Skip if function was already defined (prevents duplicate definitions from prelude)
        if self.defined_functions.contains(&func.name) {
            return;
        }
        self.defined_functions.insert(func.name.clone());

        let params_str = if func.params.is_empty() {
            String::new()
        } else {
            func.params
                .iter()
                .map(|p| {
                    let ty = func.variables.get(p).unwrap_or(&TejxType::Void);
                    format!("{} %{}", Self::get_llvm_type(ty), p)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        // Track function parameter counts for Call/IndirectCall logic
        self.function_param_counts
            .insert(func.name.clone(), func.params.len());
        self.current_function_params = func.params.iter().cloned().collect();

        self.emit(&format!(
            "define {} @\"{}\"({}) {{\n",
            ret_llvm_ty, func.name, params_str
        ));

        // Entry: allocas for all variables used in the function
        self.emit("entry:\n");
        let entry_marker = self.buffer.len();

        self.declare_runtime_fn("rt_safepoint_poll", "void @rt_safepoint_poll() nounwind");

        // 1. Scan for all local variables
        // Create one arena per function (not per basic block)
        if self.needs_arena(func) {
            self.declare_runtime_fn(
                RT_ARENA_CREATE,
                &format!("i64 @{}(i64) nounwind", RT_ARENA_CREATE),
            );
            self.declare_runtime_fn(
                RT_ARENA_DESTROY,
                &format!("void @{}(i64) nounwind", RT_ARENA_DESTROY),
            );
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
            let ty = func.variables.get(name).unwrap_or(&TejxType::Void);
            // Void-typed temps (e.g. result of void-returning calls or unresolved types)
            // still need an alloca as they may be used as call destinations. Default to i64.
            let llvm_ty = if matches!(ty, TejxType::Void) {
                "i64"
            } else {
                Self::get_llvm_type(ty)
            };
            let reg_name = format!("%{}_ptr", name.replace('$', "_"));
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
                self.declare_runtime_fn("rt_map_new", "i64 @rt_map_new(i64) nounwind");
                self.declare_runtime_fn("rt_Map_merge", "void @rt_Map_merge(i64, i64) nounwind");

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
            self.declare_runtime_fn("rt_map_new", "i64 @rt_map_new(i64) nounwind");

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
                self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                self.emit_line(&format!("call void @rt_push_root(i64* {})", ptr_name));
                self.num_roots += 1;
            }
        }

        // Sync parameters to environment if captured
        for p in &func.params {
            if let Some(cap_idx) = self.get_captured_index(p) {
                if let Some(env) = self.current_env.clone() {
                    self.declare_runtime_fn(
                        "rt_array_set_fast",
                        "void @rt_array_set_fast(i64, i64, i64)",
                    );

                    let ty = func.variables.get(p).unwrap();
                    let val_to_store = if ty.is_float() {
                        self.temp_counter += 1;
                        let bits = format!("%fbits_param_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast double %{} to i64", bits, p));
                        bits
                    } else if ty.is_numeric() || *ty == TejxType::Bool || *ty == TejxType::Char {
                        let casted = self.emit_abi_cast(&format!("%{}", p), ty, &TejxType::Int64);
                        self.temp_counter += 1;
                        let tmp = format!("%zext_param_{}", self.temp_counter);
                        self.emit_line(&format!("{} = or i64 0, {}", tmp, casted));
                        tmp
                    } else {
                        format!("%{}", p)
                    };

                    self.emit_line(&format!(
                        "call void @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                        env, cap_idx, val_to_store
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
}
