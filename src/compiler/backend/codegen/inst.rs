use super::*;
use crate::common::builtins;
use crate::common::intrinsics::*;
use crate::middle::mir::*;
use crate::frontend::token::TokenType;
use crate::common::types::TejxType;

impl CodeGen {
    pub(crate) fn gen_instruction_v2(
        &mut self,
        inst: &MIRInstruction,
        func: &MIRFunction,
        _bb_name: &str,
        current_bb: usize,
    ) {
        match inst {
            MIRInstruction::Move { dst, src, .. } => {
                let val = self.resolve_value(src);
                let src_ty = src.get_type();
                self.emit_store_variable(dst, &val, src_ty);

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
                op_width,
                ..
            } => {
                self.emit_binary_op(func, dst, left, op, right, op_width);
            }

            MIRInstruction::Jump { target, .. } => {
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
                self.emit_line(&format!("call void @{}()", TEJX_POP_HANDLER));
            }
            MIRInstruction::Branch {
                condition,
                true_target,
                false_target,
                ..
            } => {
                let cond_val = self.resolve_value(condition);

                let cond = if condition.get_type() == &TejxType::Bool {
                    self.temp_counter += 1;
                    let cond_i1 = format!("%cond_i1{}", self.temp_counter);
                    self.emit_line(&format!("{} = trunc i8 {} to i1", cond_i1, cond_val));
                    cond_i1
                } else {
                    let casted_cond =
                        self.emit_abi_cast(&cond_val, condition.get_type(), &TejxType::Int64);
                    self.declare_runtime_fn("rt_to_boolean", "i64 @rt_to_boolean(i64)");
                    self.temp_counter += 1;
                    let bool_val = format!("%bool_val{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_boolean(i64 {})",
                        bool_val, casted_cond
                    ));
                    self.temp_counter += 1;
                    let cmp = format!("%cmp{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp ne i64 {}, 0", cmp, bool_val));
                    cmp
                };

                // Safepoint poll for backward branches (loops)
                if *true_target <= current_bb || *false_target <= current_bb {
                    self.emit_line("call void @rt_safepoint_poll()");
                }

                let true_name = if *true_target < func.blocks.len() {
                    func.blocks[*true_target].name.clone()
                } else {
                    "unreachable_block".to_string()
                };
                let false_name = if *false_target < func.blocks.len() {
                    func.blocks[*false_target].name.clone()
                } else {
                    "unreachable_block".to_string()
                };
                self.emit_line(&format!(
                    "br i1 {}, label %{}, label %{}",
                    cond, true_name, false_name
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
                    self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                    self.emit_line(&format!("call void @rt_pop_roots(i64 {})", self.num_roots));
                }

                if let Some(arena) = self.current_arena.clone() {
                    self.emit_line(&format!("call void @{}(i64 {})", RT_ARENA_DESTROY, arena));
                }

                let ret_llvm_ty = Self::get_llvm_type(&func.return_type);
                if let Some(v) = value {
                    let val_str = self.resolve_value(v);
                    let final_val = self.emit_abi_cast(&val_str, v.get_type(), &func.return_type);
                    self.emit_line(&format!("ret {} {}", ret_llvm_ty, final_val));
                } else if ret_llvm_ty == "void" {
                    self.emit_line("ret void");
                } else if ret_llvm_ty == "float" || ret_llvm_ty == "double" {
                    self.emit_line(&format!("ret {} 0.0", ret_llvm_ty));
                } else if ret_llvm_ty.ends_with('*') {
                    self.emit_line(&format!("ret {} null", ret_llvm_ty));
                } else {
                    self.emit_line(&format!("ret {} 0", ret_llvm_ty)); // fallback
                }
            }
            MIRInstruction::Call {
                dst, callee, args, ..
            } => {
                self.emit_call(func, dst, callee, args);
            }
            MIRInstruction::IndirectCall {
                dst, callee, args, ..
            } => {
                self.emit_indirect_call(func, dst, callee, args);
            }
            MIRInstruction::LoadMember {
                dst, obj, member, ..
            } => {
                self.emit_load_member(func, dst, obj, member);
            }
            MIRInstruction::StoreMember {
                obj, member, src, ..
            } => {
                self.emit_store_member(func, obj, member, src);
            }
            MIRInstruction::LoadIndex {
                dst,
                obj,
                index,
                element_ty,
                ..
            } => {
                self.emit_load_index(func, dst, obj, index, element_ty);
            }
            MIRInstruction::StoreIndex {
                obj,
                index,
                src,
                element_ty,
                ..
            } => {
                self.emit_store_index(func, obj, index, src, element_ty);
            }
            MIRInstruction::Throw { value, .. } => {
                self.emit_throw(value);
            }
            MIRInstruction::Cast { dst, src, ty, .. } => {
                self.emit_cast(func, dst, src, ty);
            }
        }
    }

    pub(crate) fn emit_binary_op(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        left: &MIRValue,
        op: &TokenType,
        right: &MIRValue,
        op_width: &TejxType,
    ) {
        let l_ty = match left {
            MIRValue::Constant { ty, .. } => ty,
            MIRValue::Variable { ty, .. } => ty,
        };
        let r_ty = match right {
            MIRValue::Constant { ty, .. } => ty,
            MIRValue::Variable { ty, .. } => ty,
        };

        let mut temp_root_count = 0;
        let l = self.resolve_value(left);
        if Self::is_gc_managed(l_ty)
            && !(matches!(l_ty, TejxType::String) && l.starts_with("ptrtoint"))
        {
            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let tmp_root = format!("%bin_l_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", tmp_root));
            self.emit_line(&format!("store i64 {}, i64* {}", l, tmp_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
            temp_root_count += 1;
        }

        let r = self.resolve_value(right);
        if Self::is_gc_managed(r_ty)
            && !(matches!(r_ty, TejxType::String) && r.starts_with("ptrtoint"))
        {
            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let tmp_root = format!("%bin_r_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", tmp_root));
            self.emit_line(&format!("store i64 {}, i64* {}", r, tmp_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
            temp_root_count += 1;
        }

        self.temp_counter += 1;
        let tmp = format!("%tmp{}", self.temp_counter);

        // Use op_width to determine the type of the operation
        let mut is_string_op = matches!(op_width, TejxType::String)
            || matches!(l_ty, TejxType::String)
            || matches!(r_ty, TejxType::String);
        let is_float_op = op_width.is_float();
        let is_any_op = matches!(op_width, TejxType::Any)
            || matches!(l_ty, TejxType::Any)
            || matches!(r_ty, TejxType::Any);

        // If it's a comparison and either side is String, it's a string op
        if matches!(
            op,
            TokenType::EqualEqual
                | TokenType::BangEqual
        )
            && (matches!(l_ty, TejxType::String) || matches!(r_ty, TejxType::String)) {
                is_string_op = true;
            }

        let is_numeric_op = !is_string_op
            && !is_any_op
            && (is_float_op || op_width.is_numeric() || matches!(op_width, TejxType::Bool));

        if is_string_op {
            let l_val = if l_ty.is_numeric() {
                if l_ty.is_float() {
                    self.declare_runtime_fn(
                        "rt_to_string_float",
                        "i64 @rt_to_string_float(double)",
                    );
                    let val_as_double = self.emit_abi_cast(&l, l_ty, &TejxType::Float64);
                    self.temp_counter += 1;
                    let l_str = format!("%l_str{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_string_float(double {})",
                        l_str, val_as_double
                    ));
                    l_str
                } else {
                    self.declare_runtime_fn("rt_to_string_int", "i64 @rt_to_string_int(i64)");
                    let val_as_int = self.emit_abi_cast(&l, l_ty, &TejxType::Int64);
                    self.temp_counter += 1;
                    let l_str = format!("%l_str{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_string_int(i64 {})",
                        l_str, val_as_int
                    ));
                    l_str
                }
            } else if matches!(l_ty, TejxType::Bool) {
                self.declare_runtime_fn("rt_to_string_boolean", "i64 @rt_to_string_boolean(i64)");
                let val_as_bool = self.emit_abi_cast(&l, l_ty, &TejxType::Int64);
                self.temp_counter += 1;
                let l_str = format!("%l_str{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_to_string_boolean(i64 {})",
                    l_str, val_as_bool
                ));
                l_str
            } else if matches!(l_ty, TejxType::String) && l.starts_with("ptrtoint") {
                self.declare_runtime_fn("rt_string_from_c_str", "i64 @rt_string_from_c_str(i64)");
                self.temp_counter += 1;
                let boxed = format!("%boxed_l{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_string_from_c_str(i64 {})",
                    boxed, l
                ));
                boxed
            } else {
                self.emit_abi_cast(&l, l_ty, &TejxType::Class("Any".to_string(), vec![]))
            };

            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let l_val_root = format!("%str_l_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", l_val_root));
            self.emit_line(&format!("store i64 {}, i64* {}", l_val, l_val_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", l_val_root));
            temp_root_count += 1;

            let r_val = if r_ty.is_numeric() {
                if r_ty.is_float() {
                    self.declare_runtime_fn(
                        "rt_to_string_float",
                        "i64 @rt_to_string_float(double)",
                    );
                    let val_as_double = self.emit_abi_cast(&r, r_ty, &TejxType::Float64);
                    self.temp_counter += 1;
                    let r_str = format!("%r_str{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_string_float(double {})",
                        r_str, val_as_double
                    ));
                    r_str
                } else {
                    self.declare_runtime_fn("rt_to_string_int", "i64 @rt_to_string_int(i64)");
                    let val_as_int = self.emit_abi_cast(&r, r_ty, &TejxType::Int64);
                    self.temp_counter += 1;
                    let r_str = format!("%r_str{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_string_int(i64 {})",
                        r_str, val_as_int
                    ));
                    r_str
                }
            } else if matches!(r_ty, TejxType::Bool) {
                self.declare_runtime_fn("rt_to_string_boolean", "i64 @rt_to_string_boolean(i64)");
                let val_as_bool = self.emit_abi_cast(&r, r_ty, &TejxType::Int64);
                self.temp_counter += 1;
                let r_str = format!("%r_str{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_to_string_boolean(i64 {})",
                    r_str, val_as_bool
                ));
                r_str
            } else if matches!(r_ty, TejxType::String) && r.starts_with("ptrtoint") {
                self.declare_runtime_fn("rt_string_from_c_str", "i64 @rt_string_from_c_str(i64)");
                self.temp_counter += 1;
                let boxed = format!("%boxed_r{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_string_from_c_str(i64 {})",
                    boxed, r
                ));
                boxed
            } else {
                self.emit_abi_cast(&r, r_ty, &TejxType::Class("Any".to_string(), vec![]))
            };

            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let r_val_root = format!("%str_r_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", r_val_root));
            self.emit_line(&format!("store i64 {}, i64* {}", r_val, r_val_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", r_val_root));
            temp_root_count += 1;

            if matches!(op, TokenType::Plus) {
                self.declare_runtime_fn(
                    "rt_str_concat_v2",
                    "i64 @rt_str_concat_v2(i64, i64) nounwind",
                );
                self.emit_line(&format!(
                    "{} = call i64 @rt_str_concat_v2(i64 {}, i64 {})",
                    tmp, l_val, r_val
                ));
            } else if matches!(op, TokenType::EqualEqual)
            {
                self.declare_runtime_fn("rt_str_equals", "i32 @rt_str_equals(i64, i64)");
                self.temp_counter += 1;
                let eq_res = format!("%eq_res{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i32 @rt_str_equals(i64 {}, i64 {})",
                    eq_res, l_val, r_val
                ));
                self.emit_line(&format!("{} = zext i32 {} to i64", tmp, eq_res));
            } else if matches!(op, TokenType::BangEqual)
            {
                self.declare_runtime_fn("rt_str_equals", "i32 @rt_str_equals(i64, i64)");
                self.declare_runtime_fn("rt_not", "i64 @rt_not(i64)");
                let eq_res_32 = format!("%eq_res32_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i32 @rt_str_equals(i64 {}, i64 {})",
                    eq_res_32, l_val, r_val
                ));
                let eq_tmp = format!("%eq{}", self.temp_counter);
                self.emit_line(&format!("{} = zext i32 {} to i64", eq_tmp, eq_res_32));
                self.temp_counter += 1;
                self.emit_line(&format!("{} = call i64 @rt_not(i64 {})", tmp, eq_tmp));
            } else {
                // Fallback numeric addition on strings
                self.emit_line(&format!("{} = add i64 {}, {}", tmp, l_val, r_val));
            }
            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
            let final_tmp =
                self.emit_abi_cast(&tmp, &TejxType::Class("Any".to_string(), vec![]), dst_ty);
            self.emit_store_variable(dst, &final_tmp, dst_ty);
        } else if is_any_op {
            let l_any = self.emit_abi_cast(&l, l_ty, &TejxType::Class("Any".to_string(), vec![]));

            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let l_any_root = format!("%any_l_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", l_any_root));
            self.emit_line(&format!("store i64 {}, i64* {}", l_any, l_any_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", l_any_root));
            temp_root_count += 1;

            let r_any = self.emit_abi_cast(&r, r_ty, &TejxType::Class("Any".to_string(), vec![]));

            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let r_any_root = format!("%any_r_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", r_any_root));
            self.emit_line(&format!("store i64 {}, i64* {}", r_any, r_any_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", r_any_root));
            temp_root_count += 1;

            let rt_fn = match op {
                TokenType::Plus => "rt_add",
                TokenType::Minus => "rt_sub",
                TokenType::Star => "rt_mul",
                TokenType::Slash => "rt_div",
                TokenType::Less => "rt_lt",
                TokenType::Greater => "rt_gt",
                TokenType::EqualEqual => "rt_eq",
                TokenType::BangEqual => "rt_ne",
                TokenType::LessEqual => "rt_le",
                TokenType::GreaterEqual => "rt_ge",
                _ => "rt_add",
            };

            self.declare_runtime_fn(rt_fn, &format!("i64 @{}(i64, i64)", rt_fn));
            self.emit_line(&format!(
                "{} = call i64 @{}(i64 {}, i64 {})",
                tmp, rt_fn, l_any, r_any
            ));

            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
            let final_tmp =
                self.emit_abi_cast(&tmp, &TejxType::Class("Any".to_string(), vec![]), dst_ty);
            self.emit_store_variable(dst, &final_tmp, dst_ty);
        } else if is_numeric_op {
            let l_is_raw = l_ty.is_numeric() && !l_ty.is_float();
            let r_is_raw = r_ty.is_numeric() && !r_ty.is_float();
            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);

            if l_is_raw && r_is_raw {
                let op_ty = if l_ty == r_ty { l_ty } else { &TejxType::Int64 };
                let op_llvm = Self::get_llvm_type(op_ty);
                let l_cast = self.emit_abi_cast(&l, l_ty, op_ty);
                let r_cast = self.emit_abi_cast(&r, r_ty, op_ty);

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
                        "{} = icmp {} {} {}, {}",
                        cmp_res, pred, op_llvm, l_cast, r_cast
                    ));
                    self.temp_counter += 1;
                    let cmp_bool = format!("%cmp_bool{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = zext i1 {} to i8",
                        cmp_bool, cmp_res
                    ));
                    let final_res = self.emit_abi_cast(&cmp_bool, &TejxType::Bool, dst_ty);
                    self.emit_store_variable(dst, &final_res, dst_ty);
                } else {
                    if llvm_op == "sdiv" || llvm_op == "srem" {
                        self.declare_runtime_fn(
                            "rt_div_zero_error",
                            "void @rt_div_zero_error() nounwind",
                        );
                        let label_id = self.temp_counter;
                        self.temp_counter += 1;
                        let is_zero = format!("%is_zero{}", self.temp_counter);
                        self.emit_line(&format!("{} = icmp eq {} {}, 0", is_zero, op_llvm, r_cast));
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
                    self.emit_line(&format!(
                        "{} = {} {} {}, {}",
                        tmp, llvm_op, op_llvm, l_cast, r_cast
                    ));
                    let final_res = self.emit_abi_cast(&tmp, op_ty, dst_ty);
                    self.emit_store_variable(dst, &final_res, dst_ty);
                }
            } else {
                // Double precision path (Promotion)
                let op_ty = &TejxType::Float64;
                let op_llvm = Self::get_llvm_type(op_ty);
                let l_cast = self.emit_abi_cast(&l, l_ty, op_ty);
                let r_cast = self.emit_abi_cast(&r, r_ty, op_ty);

                let (is_cmp, llvm_op, pred) = match op {
                    TokenType::Plus => (false, "fadd", ""),
                    TokenType::Minus => (false, "fsub", ""),
                    TokenType::Star => (false, "fmul", ""),
                    TokenType::Slash => (false, "fdiv", ""),
                    TokenType::Less => (true, "", "olt"),
                    TokenType::Greater => (true, "", "ogt"),
                    TokenType::EqualEqual => (true, "", "oeq"),
                    TokenType::BangEqual => (true, "", "one"),
                    TokenType::LessEqual => (true, "", "ole"),
                    TokenType::GreaterEqual => (true, "", "oge"),
                    TokenType::Modulo => (false, "frem", ""),
                    _ => (false, "fadd", ""),
                };

                let is_equality = matches!(
                    op,
                    TokenType::EqualEqual
                        | TokenType::BangEqual
                );

                if is_any_op && is_equality {
                    let l_eval =
                        self.emit_abi_cast(&l, l_ty, &TejxType::Class("Any".to_string(), vec![]));
                    let r_eval =
                        self.emit_abi_cast(&r, r_ty, &TejxType::Class("Any".to_string(), vec![]));
                    let func_name = match op {
                        TokenType::EqualEqual | TokenType::BangEqual => "rt_eq",
                        _ => "rt_strict_equal",
                    };
                    self.declare_runtime_fn(func_name, &format!("i64 @{}(i64, i64)", func_name));
                    let eq_res = format!("%eq_res{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!(
                        "{} = call i64 @{}(i64 {}, i64 {})",
                        eq_res, func_name, l_eval, r_eval
                    ));

                    if matches!(op, TokenType::BangEqual) {
                        self.declare_runtime_fn("rt_not", "i64 @rt_not(i64)");
                        self.emit_line(&format!("{} = call i64 @rt_not(i64 {})", tmp, eq_res));
                    } else {
                        self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, eq_res));
                    }
                    let final_res = self.emit_abi_cast(
                        &tmp,
                        &TejxType::Class("Any".to_string(), vec![]),
                        dst_ty,
                    );
                    self.emit_store_variable(dst, &final_res, dst_ty);
                } else if is_cmp {
                    self.temp_counter += 1;
                    let cmp_res = format!("%cmp_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = fcmp {} {} {}, {}",
                        cmp_res, pred, op_llvm, l_cast, r_cast
                    ));
                    self.temp_counter += 1;
                    let cmp_bool = format!("%cmp_bool{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = zext i1 {} to i8",
                        cmp_bool, cmp_res
                    ));
                    let final_res = self.emit_abi_cast(&cmp_bool, &TejxType::Bool, dst_ty);
                    self.emit_store_variable(dst, &final_res, dst_ty);
                } else {
                    self.emit_line(&format!(
                        "{} = {} {} {}, {}",
                        tmp, llvm_op, op_llvm, l_cast, r_cast
                    ));
                    let final_res = self.emit_abi_cast(&tmp, op_ty, dst_ty);
                    self.emit_store_variable(dst, &final_res, dst_ty);
                }
            }
        } else {
            // DefaultFallback (e.g. Object comparisons)
            let (is_cmp, llvm_op, pred) = match op {
                TokenType::Plus => (false, "add", ""),
                TokenType::Minus => (false, "sub", ""),
                TokenType::Star => (false, "mul", ""),
                TokenType::Slash => (false, "sdiv", ""),
                TokenType::Less => (true, "", "slt"),
                TokenType::Greater => (true, "", "sgt"),
                TokenType::LessEqual => (true, "", "sle"),
                TokenType::GreaterEqual => (true, "", "sge"),
                TokenType::EqualEqual => (true, "", "eq"),
                TokenType::BangEqual => (true, "", "ne"),
                _ => (false, "add", ""),
            };

            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
            let op_ty = if l_ty == r_ty { l_ty } else { &TejxType::Int64 };
            let op_llvm = Self::get_llvm_type(op_ty);

            let l_cast = self.emit_abi_cast(&l, l_ty, op_ty);
            let r_cast = self.emit_abi_cast(&r, r_ty, op_ty);

            if is_cmp {
                self.temp_counter += 1;
                let cmp_res = format!("%cmp_res{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = icmp {} {} {}, {}",
                    cmp_res, pred, op_llvm, l_cast, r_cast
                ));
                self.temp_counter += 1;
                let cmp_bool = format!("%cmp_bool{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = zext i1 {} to i8",
                    cmp_bool, cmp_res
                ));
                let final_res = self.emit_abi_cast(&cmp_bool, &TejxType::Bool, dst_ty);
                self.emit_store_variable(dst, &final_res, dst_ty);
            } else {
                self.emit_line(&format!(
                    "{} = {} {} {}, {}",
                    tmp, llvm_op, op_llvm, l_cast, r_cast
                ));
                let final_res = self.emit_abi_cast(&tmp, op_ty, dst_ty);
                self.emit_store_variable(dst, &final_res, dst_ty);
            }
        }

        if temp_root_count > 0 {
            self.emit_line(&format!(
                "call void @rt_pop_roots(i64 {})",
                temp_root_count
            ));
        }
    }

    pub(crate) fn emit_call(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        callee: &String,
        args: &[MIRValue],
    ) {
        if callee == "rt_box_number" {
            let float_val = self.resolve_float_value(&args[0]);

            if !dst.is_empty() {
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                let result_tmp = match dst_ty {
                    TejxType::Float32 => {
                        self.temp_counter += 1;
                        let tmp = format!("%boxed_num{}", self.temp_counter);
                        self.emit_line(&format!("{} = fptrunc double {} to float", tmp, float_val));
                        tmp
                    }
                    TejxType::Float64 => float_val,
                    _ => {
                        self.temp_counter += 1;
                        let tmp = format!("%boxed_num{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast double {} to i64", tmp, float_val));
                        tmp
                    }
                };
                self.emit_store_variable(dst, &result_tmp, dst_ty);
            }
            return;
        }

        if callee == "rt_to_number" {
            let arg_ty = args[0].get_type();
            let needs_runtime = matches!(arg_ty, TejxType::String | TejxType::Any)
                || matches!(arg_ty, TejxType::Class(name, _) if name == "Any");

            if needs_runtime {
                let arg_val = self.resolve_value(&args[0]);
                self.declare_runtime_fn("rt_to_number", "double @rt_to_number(i64)");
                self.temp_counter += 1;
                let num_val = format!("%num_val{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call double @rt_to_number(i64 {})",
                    num_val, arg_val
                ));

                if !dst.is_empty() {
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                    let store_val = match dst_ty {
                        TejxType::Float32 => {
                            self.temp_counter += 1;
                            let trunc = format!("%ftrunc{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = fptrunc double {} to float",
                                trunc, num_val
                            ));
                            trunc
                        }
                        TejxType::Float64 => num_val,
                        _ => {
                            self.temp_counter += 1;
                            let bits_tmp = format!("%bits{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = bitcast double {} to i64",
                                bits_tmp, num_val
                            ));
                            bits_tmp
                        }
                    };
                    self.emit_store_variable(dst, &store_val, dst_ty);
                }
            } else {
                let float_val = self.resolve_float_value(&args[0]);

                if !dst.is_empty() {
                    self.float_ssa_vars.insert(dst.clone(), float_val.clone());
                    let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);

                    let store_val = match dst_ty {
                        TejxType::Float32 => {
                            // fptrunc double -> float for Float32 destinations
                            self.temp_counter += 1;
                            let trunc = format!("%ftrunc{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = fptrunc double {} to float",
                                trunc, float_val
                            ));
                            trunc
                        }
                        TejxType::Float64 => {
                            // Store the double directly without bitcasting to i64
                            float_val
                        }
                        _ => {
                            // Default: bitcast double -> i64 for integer/general types
                            self.temp_counter += 1;
                            let bits_tmp = format!("%bits{}", self.temp_counter);
                            self.emit_line(&format!(
                                "{} = bitcast double {} to i64",
                                bits_tmp, float_val
                            ));
                            bits_tmp
                        }
                    };
                    self.emit_store_variable(dst, &store_val, dst_ty);
                }
            }
            return;
        }

        if callee == RT_CLASS_NEW {
            let class_name = match &args[0] {
                MIRValue::Constant { value, .. } => value.trim_matches('"').to_string(),
                _ => "UnknownClass".to_string(),
            };

            let type_id = self.type_id_map.get(&class_name).cloned().unwrap_or(2);
            let mut ptr_offsets = Vec::new();
            let body_size = self
                .class_fields
                .get(&class_name)
                .map(|fields| {
                    let mut offset = 0;
                    for (_, ty) in fields {
                        offset = Self::get_aligned_offset(offset, ty);
                        if Self::is_gc_managed(ty) {
                            ptr_offsets.push(offset);
                        }
                        offset += ty.size();
                    }
                    (offset + 7) & !7
                })
                .unwrap_or(0);

            let is_escaped = !dst.is_empty() && self.does_escape(func, dst);

            if !is_escaped && !dst.is_empty() && body_size > 64 && self.current_arena.is_some() {
                let arena = self.current_arena.clone().unwrap();
                self.declare_runtime_fn(
                    RT_ARENA_ALLOC,
                    &format!("i64 @{}(i64, i32, i64) nounwind", RT_ARENA_ALLOC),
                );

                self.temp_counter += 1;
                let result_tmp = format!("%call{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @{}(i64 {}, i32 {}, i64 {})",
                    result_tmp, RT_ARENA_ALLOC, arena, type_id, body_size as i64
                ));

                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &result_tmp, dst_ty);
                return;
            }

            let offsets_ptr = if ptr_offsets.is_empty() {
                "null".to_string()
            } else {
                format!(
                    "bitcast ([{} x i64]* @type_{}_offsets to i64*)",
                    ptr_offsets.len(),
                    type_id
                )
            };

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
                self.declare_runtime_fn(
                    "llvm.memset.p0i8.i64",
                    "void @llvm.memset.p0i8.i64(i8*, i8, i64, i1 immarg)",
                );
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
                self.declare_runtime_fn(
                    RT_CLASS_NEW,
                    &format!("i64 @{}(i32, i64, i64, i64*, i64) nounwind", RT_CLASS_NEW),
                );

                self.temp_counter += 1;
                let result_tmp = format!("%call{}", self.temp_counter);
                // call rt_class_new(type_id, body_size, ptr_count, offsets_ptr, stack_ptr)
                self.emit_line(&format!(
                    "{} = call i64 @{}(i32 {}, i64 {}, i64 {}, i64* {}, i64 {})",
                    result_tmp,
                    RT_CLASS_NEW,
                    type_id,
                    body_size as i64,
                    ptr_offsets.len(),
                    offsets_ptr,
                    body_ptr
                ));

                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &result_tmp, dst_ty);

                return;
            } else {
                // Heap Allocation
                self.declare_runtime_fn(
                    RT_CLASS_NEW,
                    &format!("i64 @{}(i32, i64, i64, i64*, i64) nounwind", RT_CLASS_NEW),
                );

                self.temp_counter += 1;
                let result_tmp = format!("%call{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @{}(i32 {}, i64 {}, i64 {}, i64* {}, i64 0)",
                    result_tmp,
                    RT_CLASS_NEW,
                    type_id,
                    body_size as i64,
                    ptr_offsets.len(),
                    offsets_ptr
                ));

                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &result_tmp, dst_ty);
                return;
            }
        }

        if callee == "rt_box_int" || callee == "rt_box_boolean" || callee == "rt_box_char" {
            let mut arg_val = self.resolve_value(&args[0]);
            arg_val = self.emit_abi_cast(&arg_val, args[0].get_type(), &TejxType::Int64);
            self.temp_counter += 1;
            let result_tmp = format!("%call{}", self.temp_counter);
            // Primitives are now bitcasted directly into i64 slots (generic slots)
            self.emit_line(&format!("{} = or i64 0, {}", result_tmp, arg_val));
            if !dst.is_empty() {
                let ptr = self.resolve_ptr(dst);
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Int64);
                self.store_ptr(&ptr, &result_tmp, Some(dst_ty));
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
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Int64);
                self.store_ptr(&ptr, &result_tmp, Some(dst_ty));
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
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &res_val, dst_ty);
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
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Float64);
                self.emit_store_variable(dst, &res_val, dst_ty);
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
                let sig = if param_count == 1 {
                    format!("double @{}(double)", intrinsic_name)
                } else {
                    format!("double @{}(double, double)", intrinsic_name)
                };
                self.declare_runtime_fn(intrinsic_name, &sig);

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

                if !dst.is_empty() {
                    let mut dst_ty = func.variables.get(dst).cloned().unwrap_or(TejxType::Void);
                    if matches!(dst_ty, TejxType::Void) {
                        if let Some(ptr_name) = self.value_map.get(dst) {
                            if let Some(ptr_llvm) = self.ptr_types.get(ptr_name) {
                                dst_ty = match ptr_llvm.as_str() {
                                    "i1" | "i8" => TejxType::Bool,
                                    "i16" => TejxType::Int16,
                                    "i32" => TejxType::Int32,
                                    "float" => TejxType::Float32,
                                    "double" => TejxType::Float64,
                                    _ => TejxType::Int64,
                                };
                            }
                        }
                    }
                    let result_val = self.emit_abi_cast(&result_f, &TejxType::Float64, &dst_ty);
                    self.float_ssa_vars.insert(dst.clone(), result_f.clone());
                    self.emit_store_variable(dst, &result_val, &dst_ty);
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

                let decl_args = args
                    .iter()
                    .map(|a| Self::get_llvm_type(a.get_type()))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.declare_runtime_fn(
                    callee,
                    &format!("{} @{}({})", llvm_ret, callee, decl_args),
                );
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
                } else if llvm_ret != "i64" && llvm_ret != "void" && !llvm_ret.ends_with('*') {
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
                    let mut store_ty = ret_ty.clone();
                    if matches!(store_ty, TejxType::Void) {
                        if let Some(ptr_name) = self.value_map.get(dst) {
                            if let Some(ptr_llvm) = self.ptr_types.get(ptr_name) {
                                store_ty = match ptr_llvm.as_str() {
                                    "i1" | "i8" => TejxType::Bool,
                                    "i16" => TejxType::Int16,
                                    "i32" => TejxType::Int32,
                                    "float" => TejxType::Float32,
                                    "double" => TejxType::Float64,
                                    _ => TejxType::Int64,
                                };
                            }
                        }
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.store_ptr(&ptr, &final_val, Some(&store_ty));
                }
            }
        } else {
            let mut call_args_info: Vec<(MIRValue, String)> = Vec::new();
            for arg in args {
                let arg_val = self.resolve_value(arg);
                call_args_info.push((arg.clone(), arg_val));
            }

            let mut final_callee = callee.clone();
            // m_set (Map.set) logic removed.
            let mut is_instance_call = false;
            let mut instance_var = String::new();

            if callee.contains('.') {
                let parts: Vec<&str> = callee.split('.').collect();
                if parts.len() == 2 {
                    let base = parts[0];
                    let method = parts[1];
                    if self.value_map.contains_key(base) || func.variables.contains_key(base) {
                        is_instance_call = true;
                        instance_var = base.to_string();

                        if method == "join"
                            && func
                                .variables
                                .get(base)
                                .map(|t| {
                                    matches!(
                                        t,
                                        TejxType::Class(n, _) if n == "Thread" || n.starts_with("Thread<")
                                    )
                                })
                                .unwrap_or(false)
                        {
                            final_callee = "f_Thread_join".to_string();
                        } else if let Some(ty) = func.variables.get(base) {
                            if let Some(builtin_callee) =
                                builtins::method_callee(ty, method)
                            {
                                final_callee = builtin_callee;
                            } else {
                                // Resolve instance type to get class name dynamically
                                let mut class_name = base.to_string();
                                match ty {
                                    TejxType::Class(name, _) => {
                                        if name.contains('<') {
                                            // Generic class like Map<string, CacheNode>
                                            // Extract base class name before '<'
                                            class_name = name
                                                .split('<')
                                                .next()
                                                .unwrap_or(name)
                                                .to_string();
                                        } else {
                                            class_name = name.clone();
                                        }
                                    }
                                    TejxType::Any => class_name = "Any".to_string(),
                                    _ => {}
                                }
                                if class_name == "Any" {
                                    // For performance, we could devirtualize, but for now we prioritize
                                    // correctness with dynamic dispatch for all class methods to support overriding.
                                    final_callee = format!("virtual_call_{}", method);
                                }
                            }
                        }
                    } else {
                        // Fallback to virtual call if base type is unknown
                        final_callee = format!("virtual_call_{}", method);
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

            let is_math_fn = final_callee.starts_with("std_math_");
            let is_runtime_fn = final_callee.starts_with("rt_")
                || final_callee.starts_with("tejx_")
                || final_callee == "printf"
                || final_callee == "malloc"
                || final_callee == "free";
            let is_runtime_any = is_runtime_fn && final_callee != "rt_str_equals";

            let mut call_args_info: Vec<(MIRValue, String)> = Vec::new();
            let mut llvm_args = Vec::new();
            let mut llvm_decl_args = Vec::new();
            let mut temp_root_count = 0;

            if is_instance_call {
                // Peek at the first argument to see if it's already the instance
                let already_has_this = args
                    .first()
                    .map(|arg| {
                        if let MIRValue::Variable { name, .. } = arg {
                            name == &instance_var
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);

                if !already_has_this {
                    if let Some(ptr) = self.value_map.get(&instance_var) {
                        let ptr_clone = ptr.clone();
                        self.temp_counter += 1;
                        let tmp = format!("%inst{}", self.temp_counter);
                        self.load_ptr(&ptr_clone, &tmp);
                        let instance_ty = func
                            .variables
                            .get(&instance_var)
                            .cloned()
                            .unwrap_or(TejxType::Int64);
                        let arg_mir = MIRValue::Variable {
                            name: instance_var.clone(),
                            ty: instance_ty,
                        };

                        let mut final_reg = tmp.clone();
                        if matches!(arg_mir.get_type(), TejxType::String)
                            && final_reg.starts_with("ptrtoint")
                            && final_callee != "rt_string_from_c_str"
                        {
                            final_reg = self.emit_box_string(&final_reg);
                        }

                        let target_llvm_ty = if is_math_fn {
                            "double".to_string()
                        } else if is_runtime_fn {
                            "i64".to_string()
                        } else {
                            Self::get_llvm_type(arg_mir.get_type()).to_string()
                        };

                        let casted = if is_math_fn {
                            self.emit_abi_cast(&final_reg, arg_mir.get_type(), &TejxType::Float64)
                        } else if is_runtime_any {
                            self.emit_abi_cast(
                                &final_reg,
                                arg_mir.get_type(),
                                &TejxType::Class("Any".to_string(), vec![]),
                            )
                        } else {
                            final_reg.clone()
                        };

                        if Self::is_gc_managed(arg_mir.get_type())
                            && !(final_callee == "rt_string_from_c_str"
                                && matches!(arg_mir.get_type(), TejxType::String)
                                && final_reg.starts_with("ptrtoint"))
                        {
                            self.declare_runtime_fn(
                                "rt_push_root",
                                "void @rt_push_root(i64*) nounwind",
                            );
                            self.declare_runtime_fn(
                                "rt_pop_roots",
                                "void @rt_pop_roots(i64) nounwind",
                            );
                            self.temp_counter += 1;
                            let tmp_root = format!("%arg_root_{}", self.temp_counter);
                            self.alloca_buffer
                                .push_str(&format!("  {} = alloca i64\n", tmp_root));
                            self.emit_line(&format!(
                                "store i64 {}, i64* {}",
                                casted, tmp_root
                            ));
                            self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                            temp_root_count += 1;
                        }

                        call_args_info.push((arg_mir, final_reg));
                        llvm_args.push(format!("{} {}", target_llvm_ty, casted));
                        llvm_decl_args.push(target_llvm_ty);
                    }
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

                let arg_ty = arg.get_type();
                let mut final_reg = reg.clone();

                // Ensure string literals are boxed when passed to runtime functions (except rt_string_from_c_str)
                if matches!(arg_ty, TejxType::String)
                    && final_reg.starts_with("ptrtoint")
                    && final_callee != "rt_string_from_c_str"
                {
                    final_reg = self.emit_box_string(&final_reg);
                }

                let target_llvm_ty = if is_math_fn {
                    "double".to_string()
                } else if is_runtime_fn {
                    "i64".to_string()
                } else {
                    Self::get_llvm_type(arg_ty).to_string()
                };

                let casted = if is_math_fn {
                    self.emit_abi_cast(&final_reg, arg_ty, &TejxType::Float64)
                } else if is_runtime_any {
                    self.emit_abi_cast(
                        &final_reg,
                        arg_ty,
                        &TejxType::Class("Any".to_string(), vec![]),
                    )
                } else {
                    final_reg.clone()
                };

                if Self::is_gc_managed(arg_ty)
                    && !(final_callee == "rt_string_from_c_str"
                        && matches!(arg_ty, TejxType::String)
                        && final_reg.starts_with("ptrtoint"))
                {
                    self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                    self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                    self.temp_counter += 1;
                    let tmp_root = format!("%arg_root_{}", self.temp_counter);
                    self.alloca_buffer
                        .push_str(&format!("  {} = alloca i64\n", tmp_root));
                    self.emit_line(&format!("store i64 {}, i64* {}", casted, tmp_root));
                    self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                    temp_root_count += 1;
                }

                call_args_info.push((arg.clone(), final_reg));
                llvm_args.push(format!("{} {}", target_llvm_ty, casted));
                llvm_decl_args.push(target_llvm_ty);
            }

            let args_str = llvm_args.join(", ");
            let decl_args_str = llvm_decl_args.join(", ");

            let ret_ty = if !dst.is_empty() {
                func.variables.get(dst).cloned().unwrap_or(TejxType::Void)
            } else {
                TejxType::Void
            };
            let mut store_ty = ret_ty.clone();
            if !dst.is_empty() && matches!(store_ty, TejxType::Void) {
                if let Some(ptr_name) = self.value_map.get(dst) {
                    if let Some(ptr_llvm) = self.ptr_types.get(ptr_name) {
                        store_ty = match ptr_llvm.as_str() {
                            "i1" | "i8" => TejxType::Bool,
                            "i16" => TejxType::Int16,
                            "i32" => TejxType::Int32,
                            "float" => TejxType::Float32,
                            "double" => TejxType::Float64,
                            _ => TejxType::Int64,
                        };
                    }
                }
            }

            let decl_ret =
                if is_math_fn || final_callee == "rt_to_number" || final_callee == "rt_math_random"
                {
                    "double".to_string()
                } else if is_runtime_fn {
                    if final_callee == "rt_promise_resolve"
                        || final_callee == "rt_promise_reject"
                        || final_callee == "rt_promise_await_resume"
                    {
                        "void".to_string()
                    } else {
                        "i64".to_string()
                    }
                } else {
                    Self::get_llvm_type(&ret_ty).to_string()
                };

            let use_quotes = !is_runtime_fn;
            let is_virtual = final_callee.starts_with("virtual_call_");

            let callee_symbol = if is_virtual {
                // Dynamic dispatch on 'any' types via Map is removed.
                // In a modern compiler, this would use a vtable or IC.
                // For now, it fails at runtime or returns null.
                self.declare_runtime_fn("rt_get_property", "i64 @rt_get_property(i64, i64)");

                let method_name = final_callee.replace("virtual_call_", "");
                let method_key_raw = self.emit_string_constant(&method_name);
                let this_ptr = if let Some((_, reg)) = call_args_info.first() {
                    reg.clone()
                } else {
                    "0".to_string()
                };

                self.temp_counter += 1;
                let func_ptr_i64 = format!("%vfunc_i64_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_get_property(i64 {}, i64 {})",
                    func_ptr_i64, this_ptr, method_key_raw
                ));

                self.temp_counter += 1;
                let func_ptr = format!("%vfunc_ptr_{}", self.temp_counter);
                let arg_tys = vec!["i64"; llvm_args.len()].join(", ");
                self.emit_line(&format!(
                    "{} = inttoptr i64 {} to {} ({})*",
                    func_ptr, func_ptr_i64, decl_ret, arg_tys
                ));
                func_ptr
            } else if use_quotes || final_callee.starts_with("f_") || final_callee.starts_with("m_")
            {
                format!("@\"{}\"", final_callee)
            } else {
                format!("@{}", final_callee)
            };

            self.temp_counter += 1;
            let result_tmp = format!("%call{}", self.temp_counter);

            if !is_virtual {
                let sig = format!("{} {}({})", decl_ret, callee_symbol, decl_args_str);
                self.declare_runtime_fn(&final_callee, &sig);
            }

            if decl_ret == "void" {
                self.emit_line(&format!("call void {}({})", callee_symbol, args_str));
            } else {
                self.emit_line(&format!(
                    "{} = call {} {}({})",
                    result_tmp, decl_ret, callee_symbol, args_str
                ));
            }

            if temp_root_count > 0 {
                self.emit_line(&format!(
                    "call void @rt_pop_roots(i64 {})",
                    temp_root_count
                ));
            }

            let mut final_val = result_tmp.clone();
            if !dst.is_empty() {
                if decl_ret == "double" {
                    final_val = self.emit_abi_cast(&result_tmp, &TejxType::Float64, &store_ty);
                } else if is_runtime_fn && decl_ret == "i64" {
                    final_val = self.emit_abi_cast(
                        &result_tmp,
                        &TejxType::Class("Any".to_string(), vec![]),
                        &store_ty,
                    );
                }
            }

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
            let is_stdlib = final_callee.starts_with("std_")
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
            if !dst.is_empty() && decl_ret != "void" {
                self.emit_store_variable(dst, &final_val, &store_ty);
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
                        let elem_size_val = if let MIRValue::Constant { value, .. } = &args[2] {
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
                        self.alloca_buffer
                            .push_str(&format!("  {} = alloca i64, align 8\n", alloca_name));
                        self.emit_line(&format!("store i64 {}, i64* {}", dp_val, alloca_name));

                        self.heap_array_ptrs
                            .insert(this_name.clone(), (alloca_name, elem_size_val));
                    }
                }
            }
        }
    }

    pub(crate) fn emit_indirect_call(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        callee: &MIRValue,
        args: &[MIRValue],
    ) {
        let callee_val = self.resolve_value(callee);
        let callee_ty = callee.get_type();

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

        let mut temp_root_count = 0;
        if Self::is_gc_managed(callee_ty) {
            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let tmp_root = format!("%callee_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", tmp_root));
            self.emit_line(&format!(
                "store i64 {}, i64* {}",
                callee_val, tmp_root
            ));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
            temp_root_count += 1;
        }

        self.declare_runtime_fn("rt_get_closure_ptr", "i64 @rt_get_closure_ptr(i64)");
        self.declare_runtime_fn("rt_get_closure_env", "i64 @rt_get_closure_env(i64)");

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
        let mut arg_types = vec!["i64".to_string()]; // First arg is always env
        let mut param_tys: Vec<TejxType> = Vec::new();
        let _ret_ty = if let TejxType::Function(params, ret) = &callee_ty {
            param_tys = params.clone();
            (**ret).clone()
        } else {
            TejxType::Int64
        };
        // Lambdas/closures use the Any/i64 ABI for return values.
        let call_ret_ty = TejxType::Any;
        for (idx, _) in args.iter().enumerate() {
            let ty = param_tys
                .get(idx)
                .cloned()
                .unwrap_or(TejxType::Int64);
            arg_types.push(Self::get_llvm_type(&ty).to_string());
        }
        while arg_types.len() < 5 {
            arg_types.push("i64".to_string());
        }
        let ptr_args = arg_types.join(", ");
        let ret_llvm = Self::get_llvm_type(&call_ret_ty);
        self.emit_line(&format!(
            "{} = inttoptr i64 {} to {} ({})*",
            func_ptr_tmp, ptr_reg, ret_llvm, ptr_args
        ));

        let mut arg_vals = vec![format!("i64 {}", env_reg)];
        for (idx, arg) in args.iter().enumerate() {
            let mut val = self.resolve_value(arg);
            let arg_ty = arg.get_type();
            let param_ty = param_tys
                .get(idx)
                .cloned()
                .unwrap_or(TejxType::Int64);

            if matches!(arg_ty, TejxType::String) && val.starts_with("ptrtoint") {
                val = self.emit_box_string(&val);
            }

            let casted = self.emit_abi_cast(&val, arg_ty, &param_ty);
            if Self::is_gc_managed(arg_ty) {
                self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                self.temp_counter += 1;
                let tmp_root = format!("%arg_root_{}", self.temp_counter);
                self.alloca_buffer
                    .push_str(&format!("  {} = alloca i64\n", tmp_root));
                self.emit_line(&format!("store i64 {}, i64* {}", casted, tmp_root));
                self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                temp_root_count += 1;
            }
            arg_vals.push(format!("{} {}", Self::get_llvm_type(&param_ty), casted));
        }
        while arg_vals.len() < 5 {
            arg_vals.push("i64 0".to_string());
        }
        let args_str = arg_vals.join(", ");

        let mut result_tmp = String::new();
        if ret_llvm == "void" {
            self.emit_line(&format!("call void {}({})", func_ptr_tmp, args_str));
        } else {
            self.temp_counter += 1;
            result_tmp = format!("%call{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call {} {}({})",
                result_tmp, ret_llvm, func_ptr_tmp, args_str
            ));
        }

        if temp_root_count > 0 {
            self.emit_line(&format!(
                "call void @rt_pop_roots(i64 {})",
                temp_root_count
            ));
        }

        if !dst.is_empty() && ret_llvm != "void" {
            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
            let final_val = self.emit_abi_cast(&result_tmp, &call_ret_ty, dst_ty);
            self.emit_store_variable(dst, &final_val, dst_ty);
        }
    }

    pub(crate) fn emit_throw(&mut self, value: &MIRValue) {
        if let Some(arena) = self.current_arena.clone() {
            self.declare_runtime_fn(
                RT_ARENA_DESTROY,
                &format!("void @{}(i64) nounwind", RT_ARENA_DESTROY),
            );
            self.emit_line(&format!("call void @{}(i64 {})", RT_ARENA_DESTROY, arena));
        }
        if self.num_roots > 0 {
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.emit_line(&format!("call void @rt_pop_roots(i64 {})", self.num_roots));
        }
        let val = self.resolve_value(value);
        self.emit_line(&format!("call void @tejx_throw(i64 {})", val));
        self.emit_line("unreachable");
    }

    pub(crate) fn emit_cast(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        src: &MIRValue,
        ty: &TejxType,
    ) {
        let s = self.resolve_value(src);
        let src_ty = src.get_type();

        self.temp_counter += 1;
        let mut tmp = format!("%cast{}", self.temp_counter);

        if src_ty.is_numeric() && ty.is_numeric() {
            let casted = self.emit_abi_cast(&s, src_ty, ty);
            self.temp_counter += 1;
            tmp = format!("%cast{}", self.temp_counter);
            let target_llvm = Self::get_llvm_type(ty);
            if target_llvm == "double" {
                self.emit_line(&format!("{} = fadd double {}, 0.0", tmp, casted));
            } else if target_llvm == "float" {
                self.emit_line(&format!("{} = fadd float {}, 0.0", tmp, casted));
            } else {
                self.emit_line(&format!("{} = add {} {}, 0", tmp, target_llvm, casted));
            }
        } else if ty.is_numeric() {
            if matches!(src_ty, TejxType::Bool | TejxType::Char) {
                let casted = self.emit_abi_cast(&s, src_ty, ty);
                self.temp_counter += 1;
                tmp = format!("%cast{}", self.temp_counter);
                let target_llvm = Self::get_llvm_type(ty);
                if target_llvm == "double" {
                    self.emit_line(&format!("{} = fadd double {}, 0.0", tmp, casted));
                } else if target_llvm == "float" {
                    self.emit_line(&format!("{} = fadd float {}, 0.0", tmp, casted));
                } else {
                    self.emit_line(&format!("{} = add {} {}, 0", tmp, target_llvm, casted));
                }
                let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
                self.emit_store_variable(dst, &tmp, dst_ty);
                return;
            }

            self.declare_runtime_fn("rt_to_number_v2", "i64 @rt_to_number_v2(i64)");
            self.temp_counter += 1;
            let num_val = format!("%num_val{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call i64 @rt_to_number_v2(i64 {})",
                num_val, s
            ));

            if ty.is_float() {
                let target_llvm = Self::get_llvm_type(ty);
                if target_llvm == "float" {
                    self.temp_counter += 1;
                    let d_val = format!("%d_val{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast i64 {} to double",
                        d_val, num_val
                    ));
                    self.emit_line(&format!(
                        "{} = fptrunc double {} to float",
                        tmp, d_val
                    ));
                } else {
                    self.emit_line(&format!(
                        "{} = bitcast i64 {} to {}",
                        tmp, num_val, target_llvm
                    ));
                }
            } else {
                let target_llvm = Self::get_llvm_type(ty);
                self.temp_counter += 1;
                let f_val = format!("%f_val{}", self.temp_counter);
                self.emit_line(&format!("{} = bitcast i64 {} to double", f_val, num_val));
                self.emit_line(&format!(
                    "{} = fptosi double {} to {}",
                    tmp, f_val, target_llvm
                ));
            }
        } else if matches!(ty, TejxType::Bool) {
            if matches!(src_ty, TejxType::Bool) {
                self.emit_line(&format!("{} = add i8 {}, 0", tmp, s));
            } else {
                let casted_s = self.emit_abi_cast(&s, src_ty, &TejxType::Int64);
                self.declare_runtime_fn("rt_to_boolean", "i64 @rt_to_boolean(i64)");
                self.temp_counter += 1;
                let bool_val = format!("%bool_val{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = call i64 @rt_to_boolean(i64 {})",
                    bool_val, casted_s
                ));
                self.temp_counter += 1;
                let bool_i1 = format!("%bool_i1{}", self.temp_counter);
                self.emit_line(&format!("{} = icmp ne i64 {}, 0", bool_i1, bool_val));
                self.emit_line(&format!("{} = zext i1 {} to i8", tmp, bool_i1));
            }
        } else if matches!(ty, TejxType::String) {
            if src_ty.is_numeric() {
                if src_ty.is_float() {
                    self.declare_runtime_fn(
                        "rt_to_string_float",
                        "i64 @rt_to_string_float(double)",
                    );
                    let val = self.emit_abi_cast(&s, src_ty, &TejxType::Float64);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_string_float(double {})",
                        tmp, val
                    ));
                } else {
                    self.declare_runtime_fn("rt_to_string_int", "i64 @rt_to_string_int(i64)");
                    let val = self.emit_abi_cast(&s, src_ty, &TejxType::Int64);
                    self.emit_line(&format!(
                        "{} = call i64 @rt_to_string_int(i64 {})",
                        tmp, val
                    ));
                }
            } else if matches!(src_ty, TejxType::Bool) {
                self.declare_runtime_fn("rt_to_string_boolean", "i64 @rt_to_string_boolean(i64)");
                let val = self.emit_abi_cast(&s, src_ty, &TejxType::Int64);
                self.emit_line(&format!(
                    "{} = call i64 @rt_to_string_boolean(i64 {})",
                    tmp, val
                ));
            } else if matches!(src_ty, TejxType::String) {
                self.emit_line(&format!("{} = add i64 {}, 0", tmp, s));
            } else {
                self.declare_runtime_fn("rt_to_string", "i64 @rt_to_string(i64)");
                // If it's not a primitive, it might need boxing to 'any' before rt_to_string
                let val = self.emit_abi_cast(&s, src_ty, &TejxType::Any);
                self.emit_line(&format!("{} = call i64 @rt_to_string(i64 {})", tmp, val));
            }
        } else {
            tmp = self.emit_abi_cast(&s, src_ty, ty);
        }
        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
        self.emit_store_variable(dst, &tmp, dst_ty);
    }
}
