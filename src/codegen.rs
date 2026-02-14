/// MIR → LLVM IR Code Generator, mirroring C++ MIRCodeGen.cpp
/// Generates textual LLVM IR from MIR basic blocks.

use crate::mir::*;
use crate::types::TejxType;
use crate::token::TokenType;
use std::collections::{HashMap, HashSet};

pub struct CodeGen {
    buffer: String,
    global_buffer: String,
    value_map: HashMap<String, String>,  // MIR var name → LLVM alloca ptr name
    temp_counter: usize,
    label_counter: usize,
    declared_functions: HashSet<String>,
    function_param_counts: HashMap<String, usize>,
    declared_globals: HashSet<String>,
    current_function_params: HashSet<String>,
    local_vars: HashSet<String>,
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
        }
    }

    fn emit(&mut self, code: &str) {
        self.buffer.push_str(code);
    }

    fn emit_line(&mut self, code: &str) {
        self.buffer.push_str("  ");
        self.buffer.push_str(code);
        self.buffer.push('\n');
    }

    fn resolve_value(&mut self, val: &MIRValue) -> String {
        match val {
            MIRValue::Constant { value, ty } => {
                // Handle "new Class" hack
                if value.starts_with("lambda_") {
                    let count = self.function_param_counts.get(value).cloned().unwrap_or(1);
                    let args = vec!["i64"; count].join(", ");
                    return format!("ptrtoint (i64 ({})* @{} to i64)", args, value);
                }
                if value.starts_with("new ") {
                    return "0".to_string();
                }

                let is_integer_type = ty.is_numeric() && !ty.is_float();
                let is_float_type = ty.is_float();
                let is_bool_type = matches!(ty, TejxType::Bool);
                let is_any_type = matches!(ty, TejxType::Any);

                if is_bool_type {
                     if value == "true" || value == "1" { return "1".to_string(); }
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
                let content = if raw_content.len() >= 2 && raw_content.starts_with('"') && raw_content.ends_with('"') {
                    &raw_content[1..raw_content.len()-1]
                } else {
                    &raw_content
                };
                
                let len = content.len() + 1;

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
            MIRValue::Variable { name, .. } => {
                if name.starts_with("g_") {
                    self.temp_counter += 1;
                    let tmp = format!("%t{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @{}", tmp, name));
                    return tmp;
                }
                if let Some(ptr) = self.value_map.get(name).cloned() {
                    // It's a pointer (alloca), load it
                    self.temp_counter += 1;
                    let tmp = format!("%t{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", tmp, ptr));
                    tmp
                } else if name.starts_with("__arg") {
                    // It's a function argument (value, not pointer)
                    format!("%{}", name)
                } else {
                    if name.starts_with("%") {
                         name.clone()
                    } else {
                         // Check if it's a known function (global or user defined)
                         let mut target = name.clone();
                         let f_name = format!("f_{}", name);
                         if self.declared_functions.contains(&f_name) {
                             target = f_name;
                         }
                         
                         if self.declared_functions.contains(&target) {
                             // Known function: Cast function pointer to i64
                             {
                               let count = self.function_param_counts.get(&target).cloned().unwrap_or(1);
                               let args_sig = vec!["i64"; count].join(", ");
                               format!("ptrtoint (i64 ({})* @{} to i64)", args_sig, target)
                           }
                         } else {
                             // Unknown global (e.g. Class name used as value) -> 0
                             "0".to_string()
                         }
                    }
                }
            }
        }
    }

    fn resolve_ptr(&mut self, name: &str) -> String {
        if name.starts_with("%") {
            return name.to_string();
        }
        
        // If it's a global variable (unmangled and not in this function's locals or params)
        // This handles top-level variables that are not explicitly `g_` prefixed in MIR,
        // but are treated as globals because they are not local to the current function.
        if !name.contains("$") && !self.local_vars.contains(name) && !self.current_function_params.contains(name) {
            if !self.declared_globals.contains(name) {
                self.global_buffer.push_str(&format!("@g_{} = global i64 0\n", name));
                self.declared_globals.insert(name.to_string());
            }
            return format!("@g_{}", name);
        }

        // Fallback to existing logic for MIR-prefixed globals (g_...) or local variables
        if name.starts_with("g_") {
            format!("@{}", name)
        } else {
            self.value_map.get(name).cloned().unwrap_or_else(|| format!("%{}_ptr", name))
        }
    }
}

/// Second pass: fix Jump and Branch instructions to use block names
impl CodeGen {
    pub fn generate_with_blocks(&mut self, functions: &[MIRFunction]) -> String {
        self.buffer.clear();
        self.global_buffer.clear();
        self.declared_functions.clear();
        
        // Register defined functions and their param counts
        let mut has_tejx_main = false;
        for f in functions {
            self.declared_functions.insert(f.name.clone());
            self.function_param_counts.insert(f.name.clone(), f.params.len());
            if f.name == "tejx_main" { has_tejx_main = true; }
        }

        // Collect and declare global variables
        let mut globals = HashSet::new();
        for func in functions {
            for bb in &func.blocks {
                for inst in &bb.instructions {
                    match inst {
                        MIRInstruction::Move { dst, src } => {
                            if dst.starts_with("g_") { globals.insert(dst.clone()); }
                            if let MIRValue::Variable { name, .. } = src { if name.starts_with("g_") { globals.insert(name.clone()); } }
                        }
                        MIRInstruction::BinaryOp { dst, left, right, .. } => {
                            if dst.starts_with("g_") { globals.insert(dst.clone()); }
                            if let MIRValue::Variable { name, .. } = left { if name.starts_with("g_") { globals.insert(name.clone()); } }
                            if let MIRValue::Variable { name, .. } = right { if name.starts_with("g_") { globals.insert(name.clone()); } }
                        }
                        MIRInstruction::Call { dst, args, .. } => {
                            if dst.starts_with("g_") { globals.insert(dst.clone()); }
                            for arg in args { if let MIRValue::Variable { name, .. } = arg { if name.starts_with("g_") { globals.insert(name.clone()); } } }
                        }
                        _ => {}
                    }
                }
            }
        }
        for g in globals {
            self.global_buffer.push_str(&format!("@{} = global i64 0\n", g));
        }

        self.global_buffer.push_str("@.fmt_d = private unnamed_addr constant [5 x i8] c\"%lld\\00\"\n");
        self.global_buffer.push_str("@.fmt_f = private unnamed_addr constant [3 x i8] c\"%f\\00\"\n");
        self.global_buffer.push_str("@.fmt_s = private unnamed_addr constant [3 x i8] c\"%s\\00\"\n");
        self.global_buffer.push_str("@.fmt_nl = private unnamed_addr constant [2 x i8] c\"\\0A\\00\"\n");
        self.global_buffer.push_str("@.fmt_sp = private unnamed_addr constant [2 x i8] c\" \\00\"\n");

        for func in functions {
            self.gen_function_v2(func);
        }

        // Exception handling runtime functions
        self.global_buffer.push_str("declare i32 @_setjmp(i8*)\n");
        self.global_buffer.push_str("declare void @tejx_push_handler(i8*)\n");
        self.global_buffer.push_str("declare void @tejx_pop_handler()\n");
        if !self.declared_functions.contains("tejx_throw") {
            self.global_buffer.push_str("declare void @tejx_throw(i64)\n");
        }
        if !self.declared_functions.contains("tejx_get_exception") {
            self.global_buffer.push_str("declare i64 @tejx_get_exception()\n");
        }
        if !self.declared_functions.contains("rt_box_string") {
            self.global_buffer.push_str("declare i64 @rt_box_string(i64)\n");
        }

        // Generate main wrapper if tejx_main exists
        if has_tejx_main {
            self.buffer.push_str("\n");
            self.buffer.push_str("declare i32 @tejx_runtime_main(i32, i8**)\n");
            self.buffer.push_str("define i32 @main(i32 %argc, i8** %argv) {\n");
            self.buffer.push_str("entry:\n");
            self.buffer.push_str("  %call = call i32 @tejx_runtime_main(i32 %argc, i8** %argv)\n");
            self.buffer.push_str("  ret i32 %call\n");
            self.buffer.push_str("}\n");
        }

        format!("{}{}", self.global_buffer, self.buffer)
    }

    fn gen_function_v2(&mut self, func: &MIRFunction) {
        self.value_map.clear();
        self.temp_counter = 0;
        self.current_function_params.clear();
        self.local_vars.clear();

        for p in &func.params {
            self.current_function_params.insert(p.clone());
        }

        // Function signature with parameters
        let params_str = if func.params.is_empty() {
            String::new()
        } else {
            func.params.iter()
                .enumerate()
                .map(|(i, _)| format!("i64 %__arg{}", i))
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
                    MIRInstruction::Move { dst, .. } => Some(dst.clone()),
                    MIRInstruction::BinaryOp { dst, .. } => Some(dst.clone()),
                    MIRInstruction::Call { dst, .. } => Some(dst.clone()),
                    MIRInstruction::ObjectLiteral { dst, .. } |
                    MIRInstruction::ArrayLiteral { dst, .. } => Some(dst.clone()),
                    MIRInstruction::LoadMember { dst, .. } => Some(dst.clone()),
                    MIRInstruction::LoadIndex { dst, .. } => Some(dst.clone()),
                    MIRInstruction::IndirectCall { dst, .. } => Some(dst.clone()),
                    MIRInstruction::Cast { dst, .. } => Some(dst.clone()),
                    _ => None,
                };
                if let Some(name) = dest_var {
                    if !name.starts_with("g_") && !self.current_function_params.contains(&name) {
                        self.local_vars.insert(name);
                    }
                }
            }
        }

        // Allocas for parameters first
        for (i, p) in func.params.iter().enumerate() {
            if !self.value_map.contains_key(p) {
                let reg_name = format!("%{}_ptr", p);
                self.emit_line(&format!("{} = alloca i64", reg_name));
                self.value_map.insert(p.clone(), reg_name.clone());
                
                // CRITICAL: Store the incoming argument into the alloca
                self.emit_line(&format!("store i64 %__arg{}, i64* {}", i, reg_name));
            }
        }
        
        // Allocas for all local variables
        let locals: Vec<String> = self.local_vars.iter().cloned().collect();
        for name in locals {
            if !self.value_map.contains_key(&name) {
                let reg_name = format!("%{}_ptr", name);
                self.emit_line(&format!("{} = alloca i64", reg_name));
                self.value_map.insert(name, reg_name);
            }
        }

        // Branch to first block
        if !func.blocks.is_empty() {
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
                    self.emit_line(&format!("{} = alloca [37 x i64]", jmpbuf));
                    self.temp_counter += 1;
                    let jmpbuf_ptr = format!("%jmpbuf_ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast [37 x i64]* {} to i8*", jmpbuf_ptr, jmpbuf));
                    // Call setjmp inline — this is the critical part
                    self.temp_counter += 1;
                    let handler_res = format!("%handler_res{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i32 @_setjmp(i8* {})", handler_res, jmpbuf_ptr));
                    // If setjmp returned 0, register the handler and continue
                    self.temp_counter += 1;
                    let is_exception = format!("%is_exception{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp ne i32 {}, 0", is_exception, handler_res));
                    let body_label = format!("{}_body", bb.name);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", is_exception, handler_name, body_label));
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
    }

    fn gen_instruction_v2(&mut self, inst: &MIRInstruction, _func: &MIRFunction, bb_name: &str) {
        match inst {
            MIRInstruction::Move { dst, src } => {
                let val = self.resolve_value(src);
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", val, ptr));
            }
            MIRInstruction::BinaryOp { dst, left, op, right } => {
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
                let is_string_op = matches!(l_ty, TejxType::String) || matches!(r_ty, TejxType::String);
                let is_any_op = matches!(l_ty, TejxType::Any) || matches!(r_ty, TejxType::Any);
                let is_float_op = l_ty.is_float() || r_ty.is_float();

                let is_numeric_op = (l_ty.is_numeric() && r_ty.is_numeric()) || is_float_op || is_any_op;

                if is_string_op {
                    if matches!(op, TokenType::Plus) {
                        if !self.declared_functions.contains("rt_str_concat_v2") {
                            self.global_buffer.push_str("declare i64 @rt_str_concat_v2(i64, i64)\n");
                            self.declared_functions.insert("rt_str_concat_v2".to_string());
                        }
                        self.emit_line(&format!("{} = call i64 @rt_str_concat_v2(i64 {}, i64 {})", tmp, l, r));
                    } else if matches!(op, TokenType::EqualEqual) {
                        if !self.declared_functions.contains("rt_str_equals") {
                            self.global_buffer.push_str("declare i64 @rt_str_equals(i64, i64)\n");
                            self.declared_functions.insert("rt_str_equals".to_string());
                        }
                        self.emit_line(&format!("{} = call i64 @rt_str_equals(i64 {}, i64 {})", tmp, l, r));
                    } else if matches!(op, TokenType::BangEqual) {
                         if !self.declared_functions.contains("rt_str_equals") {
                            self.global_buffer.push_str("declare i64 @rt_str_equals(i64, i64)\n");
                            self.declared_functions.insert("rt_str_equals".to_string());
                        }
                         if !self.declared_functions.contains("rt_not") {
                            self.global_buffer.push_str("declare i64 @rt_not(i64)\n");
                            self.declared_functions.insert("rt_not".to_string());
                        }
                         let eq_tmp = format!("%eq{}", self.temp_counter);
                         self.emit_line(&format!("{} = call i64 @rt_str_equals(i64 {}, i64 {})", eq_tmp, l, r));
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
                            _ => (false, "add", "")
                        };
                        if is_cmp {
                            self.temp_counter += 1;
                            let cmp_res = format!("%cmp_res{}", self.temp_counter);
                            self.emit_line(&format!("{} = icmp {} i64 {}, {}", cmp_res, pred, l, r));
                            self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_res));
                        } else {
                            self.emit_line(&format!("{} = {} i64 {}, {}", tmp, llvm_op, l, r));
                        }
                    } else {
                        // Double precision path (Promotion)
                        self.temp_counter += 1;
                        let l_f = format!("%l_f{}", self.temp_counter);
                        if l_is_raw {
                            self.emit_line(&format!("{} = sitofp i64 {} to double", l_f, l));
                        } else if matches!(l_ty, TejxType::Any) {
                            if !self.declared_functions.contains("rt_to_number_v2") {
                                self.global_buffer.push_str("declare i64 @rt_to_number_v2(i64)\n");
                                self.declared_functions.insert("rt_to_number_v2".to_string());
                            }
                            let bits_tmp = format!("%l_bits{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_to_number_v2(i64 {})", bits_tmp, l));
                            self.emit_line(&format!("{} = bitcast i64 {} to double", l_f, bits_tmp));
                        } else {
                            self.emit_line(&format!("{} = bitcast i64 {} to double", l_f, l));
                        }
                        
                        self.temp_counter += 1;
                        let r_f = format!("%r_f{}", self.temp_counter);
                        if r_is_raw {
                            self.emit_line(&format!("{} = sitofp i64 {} to double", r_f, r));
                        } else if matches!(r_ty, TejxType::Any) {
                            if !self.declared_functions.contains("rt_to_number_v2") {
                                self.global_buffer.push_str("declare i64 @rt_to_number_v2(i64)\n");
                                self.declared_functions.insert("rt_to_number_v2".to_string());
                            }
                            let bits_tmp = format!("%r_bits{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_to_number_v2(i64 {})", bits_tmp, r));
                            self.emit_line(&format!("{} = bitcast i64 {} to double", r_f, bits_tmp));
                        } else {
                            self.emit_line(&format!("{} = bitcast i64 {} to double", r_f, r));
                        }

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
                            _ => (false, "fadd", "")
                        };

                        if is_cmp {
                            self.temp_counter += 1;
                            let cmp_res = format!("%cmp_res{}", self.temp_counter);
                            self.emit_line(&format!("{} = fcmp {} double {}, {}", cmp_res, pred, l_f, r_f));
                            self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_res));
                        } else {
                            self.temp_counter += 1;
                            let res_f = format!("%res_f{}", self.temp_counter);
                            self.emit_line(&format!("{} = {} double {}, {}", res_f, llvm_op, l_f, r_f));
                            
                            // Does the destination expect a raw integer or a bitcasted double?
                            let dst_ty = _func.variables.get(dst).unwrap_or(&TejxType::Any);
                            if dst_ty.is_numeric() && !dst_ty.is_float() {
                                self.emit_line(&format!("{} = fptosi double {} to i64", tmp, res_f));
                            } else if matches!(dst_ty, TejxType::Any) {
                                if !self.declared_functions.contains("rt_box_number") {
                                    self.global_buffer.push_str("declare i64 @rt_box_number(double)\n");
                                    self.declared_functions.insert("rt_box_number".to_string());
                                }
                                self.emit_line(&format!("{} = call i64 @rt_box_number(double {})", tmp, res_f));
                            } else {
                                self.emit_line(&format!("{} = bitcast double {} to i64", tmp, res_f));
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
                        _ => (false, "add", "")
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
                        
                        let dst_ty = _func.variables.get(dst).unwrap_or(&TejxType::Any);
                        if matches!(dst_ty, TejxType::Any) {
                             if !self.declared_functions.contains("rt_box_number") {
                                 self.global_buffer.push_str("declare i64 @rt_box_number(double)\n");
                                 self.declared_functions.insert("rt_box_number".to_string());
                             }
                             self.temp_counter += 1;
                             let res_f = format!("%res_f{}", self.temp_counter);
                             self.emit_line(&format!("{} = sitofp i64 {} to double", res_f, res_i));
                             self.emit_line(&format!("{} = call i64 @rt_box_number(double {})", tmp, res_f));
                        } else {
                             self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, res_i));
                        }
                    }
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
                }
            }

            MIRInstruction::Jump { target } => {
                let current_bb_idx = self.find_block_idx(_func, inst);
                if let Some(idx) = current_bb_idx {
                    if _func.blocks[idx].exception_handler.is_some() {
                        self.emit_line("call void @tejx_pop_handler()");
                    }
                }

                if *target < _func.blocks.len() {
                    self.emit_line(&format!("br label %{}", _func.blocks[*target].name));
                }
            }
            MIRInstruction::Branch { condition, true_target, false_target } => {
                let current_bb_idx = self.find_block_idx(_func, inst);
                if let Some(idx) = current_bb_idx {
                    if _func.blocks[idx].exception_handler.is_some() {
                        self.emit_line("call void @tejx_pop_handler()");
                    }
                }
                
                let cond_val = self.resolve_value(condition);
                let ty = match condition {
                    MIRValue::Constant { ty, .. } => ty,
                    MIRValue::Variable { ty, .. } => ty,
                };

                let cond = if matches!(ty, TejxType::Any) {
                    if !self.declared_functions.contains("rt_to_boolean") {
                        self.global_buffer.push_str("declare i64 @rt_to_boolean(i64)\n");
                        self.declared_functions.insert("rt_to_boolean".to_string());
                    }
                    self.temp_counter += 1;
                    let bool_val = format!("%bool_val{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @rt_to_boolean(i64 {})", bool_val, cond_val));
                    bool_val
                } else {
                    cond_val
                };

                self.temp_counter += 1;
                let cmp = format!("%cmp{}", self.temp_counter);
                self.emit_line(&format!("{} = icmp ne i64 {}, 0", cmp, cond));

                let true_name = if *true_target < _func.blocks.len() {
                    _func.blocks[*true_target].name.clone()
                } else {
                    "unknown".to_string()
                };
                let false_name = if *false_target < _func.blocks.len() {
                    _func.blocks[*false_target].name.clone()
                } else {
                    "unknown".to_string()
                };
                self.emit_line(&format!("br i1 {}, label %{}, label %{}", cmp, true_name, false_name));
            }
            MIRInstruction::Return { value } => {
                let current_bb_idx = self.find_block_idx(_func, inst);
                if let Some(idx) = current_bb_idx {
                    if _func.blocks[idx].exception_handler.is_some() {
                        self.emit_line("call void @tejx_pop_handler()");
                    }
                }

                if let Some(val) = value {
                    let v = self.resolve_value(val);
                    self.emit_line(&format!("ret i64 {}", v));
                } else {
                    self.emit_line("ret i64 0");
                }
            }
            MIRInstruction::Call { dst, callee, args } => {
                if callee == "rt_box_number" {
                    let mut arg_val = self.resolve_value(&args[0]);
                    let arg_ty = args[0].get_type();
                    
                    if !self.declared_functions.contains("rt_box_number") {
                        self.global_buffer.push_str("declare i64 @rt_box_number(double)\n");
                        self.declared_functions.insert("rt_box_number".to_string());
                    }

                    if !arg_ty.is_float() {
                        self.temp_counter += 1;
                        let f_tmp = format!("%f_tmp{}", self.temp_counter);
                        self.emit_line(&format!("{} = sitofp i64 {} to double", f_tmp, arg_val));
                        arg_val = f_tmp;
                    } else {
                        self.temp_counter += 1;
                        let f_tmp = format!("%f_tmp{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast i64 {} to double", f_tmp, arg_val));
                        arg_val = f_tmp;
                    }

                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @rt_box_number(double {})", result_tmp, arg_val));
                    if !dst.is_empty() {
                         let ptr = self.resolve_ptr(dst);
                         self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                    }
                    return;
                }

                if callee == "rt_to_number" {
                    let arg_val = self.resolve_value(&args[0]);
                    
                    if !self.declared_functions.contains("rt_to_number") {
                        self.global_buffer.push_str("declare double @rt_to_number(i64)\n");
                        self.declared_functions.insert("rt_to_number".to_string());
                    }

                    self.temp_counter += 1;
                    let result_tmp = format!("%call{}", self.temp_counter);
                    self.emit_line(&format!("{} = call double @rt_to_number(i64 {})", result_tmp, arg_val));
                    
                    if !dst.is_empty() {
                         self.temp_counter += 1;
                         let bits_tmp = format!("%bits{}", self.temp_counter);
                         self.emit_line(&format!("{} = bitcast double {} to i64", bits_tmp, result_tmp));
                         let ptr = self.resolve_ptr(dst);
                         self.emit_line(&format!("store i64 {}, i64* {}", bits_tmp, ptr));
                    }
                    return;
                }

                // Handle print/eprint specifically for variadic support (like console.log)
                if callee == "print" {
                    if !self.declared_functions.contains("print_raw") {
                        self.global_buffer.push_str("declare void @print_raw(i64)\n");
                        self.declared_functions.insert("print_raw".to_string());
                    }
                    if !self.declared_functions.contains("print_space") {
                        self.global_buffer.push_str("declare void @print_space()\n");
                        self.declared_functions.insert("print_space".to_string());
                    }
                    if !self.declared_functions.contains("print_newline") {
                        self.global_buffer.push_str("declare void @print_newline()\n");
                        self.declared_functions.insert("print_newline".to_string());
                    }
                    if !self.declared_functions.contains("rt_box_boolean") {
                        self.global_buffer.push_str("declare i64 @rt_box_boolean(i64)\n");
                        self.declared_functions.insert("rt_box_boolean".to_string());
                    }

                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.emit_line("call void @print_space()");
                        }
                        let mut arg_val = self.resolve_value(arg);
                        let arg_ty = arg.get_type();
                        
                        let box_func = match arg_ty {
                            t if t.is_numeric() => Some("rt_box_number"),
                            TejxType::Bool => Some("rt_box_boolean"),
                            TejxType::String if matches!(arg, MIRValue::Constant { .. }) => Some("rt_box_string"),
                            _ => None
                        };

                        if let Some(f) = box_func {
                            if !self.declared_functions.contains(f) {
                                if f == "rt_box_number" {
                                    self.global_buffer.push_str("declare i64 @rt_box_number(double)\n");
                                } else {
                                    self.global_buffer.push_str(&format!("declare i64 @{}(i64)\n", f));
                                }
                                self.declared_functions.insert(f.to_string());
                            }
                            
                            let temp = format!("%t_box_print_{}_{}", i, self.temp_counter);
                            self.temp_counter += 1;
                            
                            if f == "rt_box_number" {
                                // Extract double bits if needed
                                let res_f = format!("%f_arg_{}", self.temp_counter);
                                self.temp_counter += 1;
                                self.emit_line(&format!("{} = bitcast i64 {} to double", res_f, arg_val));
                                self.emit_line(&format!("{} = call i64 @rt_box_number(double {})", temp, res_f));
                            } else {
                                self.emit_line(&format!("{} = call i64 @{}(i64 {})", temp, f, arg_val));
                            }
                            arg_val = temp;
                        }
                        self.emit_line(&format!("call void @print_raw(i64 {})", arg_val));
                    }
                    self.emit_line("call void @print_newline()");
                    
                    if !dst.is_empty() {
                         let ptr = self.resolve_ptr(dst);
                         self.emit_line(&format!("store i64 0, i64* {}", ptr));
                    }
                } else if callee == "eprint" {
                    if !self.declared_functions.contains("eprint_raw") {
                        self.global_buffer.push_str("declare void @eprint_raw(i64)\n");
                        self.declared_functions.insert("eprint_raw".to_string());
                    }
                    if !self.declared_functions.contains("eprint_space") {
                        self.global_buffer.push_str("declare void @eprint_space()\n");
                        self.declared_functions.insert("eprint_space".to_string());
                    }
                    if !self.declared_functions.contains("eprint_newline") {
                        self.global_buffer.push_str("declare void @eprint_newline()\n");
                        self.declared_functions.insert("eprint_newline".to_string());
                    }
                    if !self.declared_functions.contains("rt_box_boolean") {
                        self.global_buffer.push_str("declare i64 @rt_box_boolean(i64)\n");
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
                            self.emit_line(&format!("{} = call i64 @rt_box_boolean(i64 {})", temp, arg_val));
                            arg_val = temp;
                        }
                        self.emit_line(&format!("call void @eprint_raw(i64 {})", arg_val));
                    }
                    self.emit_line("call void @eprint_newline()");

                    if !dst.is_empty() {
                         let ptr = self.resolve_ptr(dst);
                         self.emit_line(&format!("store i64 0, i64* {}", ptr));
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
                                if method == "join" { final_callee = "Thread_join".to_string(); }
                                else if method == "push" { final_callee = "Array_push".to_string(); }
                                else if method == "pop" { final_callee = "Array_pop".to_string(); }
                                else if method == "fill" { final_callee = "Array_fill".to_string(); }
                                else if method == "concat" { final_callee = "arrUtil_concat".to_string(); }
                                else if method == "forEach" { final_callee = "Array_forEach".to_string(); }
                                else if method == "map" { final_callee = "Array_map".to_string(); }
                                else if method == "filter" { final_callee = "Array_filter".to_string(); }
                                else if method == "indexOf" { final_callee = "arrUtil_indexOf".to_string(); }
                                else if method == "shift" { final_callee = "arrUtil_shift".to_string(); }
                                else if method == "unshift" { final_callee = "arrUtil_unshift".to_string(); }
                                else if method == "lock" { final_callee = "m_lock".to_string(); }
                                else if method == "unlock" { final_callee = "m_unlock".to_string(); }
                                else { final_callee = format!("{}_{}", base, method); }
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
                        self.emit_line(&format!("{} = inttoptr i64 {} to i64 ({})*", func_ptr_tmp, func_val_tmp, ptr_args));
                        let mut call_arg_vals = Vec::new();
                        for arg in args {
                            let arg_val = self.resolve_value(arg);
                            call_arg_vals.push(format!("i64 {}", arg_val));
                        }
                        let args_str = call_arg_vals.join(", ");
                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!("{} = call i64 {}({})", result_tmp, func_ptr_tmp, args_str));
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
                            self.global_buffer.push_str(&format!("declare i64 @{}({})\n", final_callee, decl_args));
                        self.declared_functions.insert(final_callee.clone());
                    }
                    self.emit_line(&format!("{} = call i64 @{}({})", result_tmp, final_callee, args_str));
                    if !dst.is_empty() {
                        let ptr = self.resolve_ptr(dst);
                        self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                    }
                }
            }
            MIRInstruction::IndirectCall { dst, callee, args } => {
                let callee_val = self.resolve_value(callee);
                
                // Add null check for indirect call
                self.temp_counter += 1;
                let is_null = format!("%is_null{}", self.temp_counter);
                self.emit_line(&format!("{} = icmp eq i64 {}, 0", is_null, callee_val));
                self.temp_counter += 1;
                let fail_label = format!("call_fail_{}", self.temp_counter);
                let ok_label = format!("call_ok_{}", self.temp_counter);
                self.emit_line(&format!("br i1 {}, label %{}, label %{}", is_null, fail_label, ok_label));
                
                self.emit(&format!("{}:\n", fail_label));
                let err_msg = self.resolve_value(&MIRValue::Constant { value: "\"Undefined function\"".to_string(), ty: TejxType::String });
                let err_obj = format!("%err_obj{}", self.temp_counter);
                self.emit_line(&format!("{} = call i64 @rt_box_string(i64 {})", err_obj, err_msg));
                self.emit_line(&format!("call void @tejx_throw(i64 {})", err_obj));
                self.emit_line("unreachable");
                
                self.emit(&format!("{}:\n", ok_label));

                self.temp_counter += 1;
                let func_ptr_tmp = format!("%func_ptr_{}", self.temp_counter);
                let ptr_args = vec!["i64"; args.len()].join(", ");
                self.emit_line(&format!("{} = inttoptr i64 {} to i64 ({})*", func_ptr_tmp, callee_val, ptr_args));
                
                let mut arg_vals = Vec::new();
                for arg in args {
                    let val = self.resolve_value(arg);
                    arg_vals.push(format!("i64 {}", val));
                }
                let args_str = arg_vals.join(", ");
                
                self.temp_counter += 1;
                let result_tmp = format!("%call{}", self.temp_counter);
                self.emit_line(&format!("{} = call i64 {}({})", result_tmp, func_ptr_tmp, args_str));
                
                if !dst.is_empty() {
                    let ptr = self.resolve_ptr(dst);
                    self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                }
            }
            MIRInstruction::ObjectLiteral { dst, entries, .. } => {
                self.temp_counter += 1;
                let obj_tmp = format!("%obj{}", self.temp_counter);
                if !self.declared_functions.contains("m_new") {
                    self.global_buffer.push_str("declare i64 @m_new()\n");
                    self.declared_functions.insert("m_new".to_string());
                }
                self.emit_line(&format!("{} = call i64 @m_new()", obj_tmp));
                for (k, v) in entries {
                    let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", k), ty: TejxType::String });
                    let v_val = self.resolve_value(v);
                    if !self.declared_functions.contains("m_set") {
                        self.global_buffer.push_str("declare i64 @m_set(i64, i64, i64)\n");
                        self.declared_functions.insert("m_set".to_string());
                    }
                    self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_tmp, k_val, v_val));
                }
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", obj_tmp, ptr));
            }
            MIRInstruction::ArrayLiteral { dst, elements, ty } => {
                self.temp_counter += 1;
                let arr_tmp = format!("%arr{}", self.temp_counter);
                
                let mut size = elements.len() as i64;
                let mut use_fixed = false;
                if let Some(TejxType::FixedArray(_, sz)) = ty {
                    size = *sz as i64;
                    use_fixed = true;
                }

                if use_fixed {
                    if !self.declared_functions.contains("a_new_fixed") {
                        self.global_buffer.push_str("declare i64 @a_new_fixed(i64, i64)\n");
                        self.declared_functions.insert("a_new_fixed".to_string());
                    }
                    let elem_size = if let Some(TejxType::FixedArray(inner, _)) = &ty {
                        if matches!(**inner, TejxType::Bool) { 1 } else { 8 }
                    } else { 8 };
                    self.emit_line(&format!("{} = call i64 @a_new_fixed(i64 {}, i64 {})", arr_tmp, size, elem_size));
                } else {
                    if !self.declared_functions.contains("a_new") {
                        self.global_buffer.push_str("declare i64 @a_new()\n");
                        self.declared_functions.insert("a_new".to_string());
                    }
                    self.emit_line(&format!("{} = call i64 @a_new()", arr_tmp));
                }

                let mut idx = 0;
                for v in elements {
                    let v_val = self.resolve_value(v);
                    if use_fixed {
                         if !self.declared_functions.contains("a_set") {
                            self.global_buffer.push_str("declare i64 @a_set(i64, i64, i64)\n");
                            self.declared_functions.insert("a_set".to_string());
                        }
                        let k_idx = self.resolve_value(&MIRValue::Constant { value: idx.to_string(), ty: TejxType::Int32 });
                        self.emit_line(&format!("call i64 @a_set(i64 {}, i64 {}, i64 {})", arr_tmp, k_idx, v_val));
                    } else {
                        if !self.declared_functions.contains("Array_push") {
                             self.global_buffer.push_str("declare i64 @Array_push(i64, i64)\n");
                             self.declared_functions.insert("Array_push".to_string());
                        }
                        self.emit_line(&format!("call i64 @Array_push(i64 {}, i64 {})", arr_tmp, v_val));
                    }
                    idx += 1;
                }
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", arr_tmp, ptr));
            }
            MIRInstruction::LoadMember { dst, obj, member } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", member), ty: TejxType::String });
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);
                if !self.declared_functions.contains("m_get") {
                    self.global_buffer.push_str("declare i64 @m_get(i64, i64)\n");
                    self.declared_functions.insert("m_get".to_string());
                }
                self.emit_line(&format!("{} = call i64 @m_get(i64 {}, i64 {})", res_tmp, obj_val, k_val));
                let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
            }
            MIRInstruction::StoreMember { obj, member, src } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", member), ty: TejxType::String });
                let v_val = self.resolve_value(src);
                if !self.declared_functions.contains("m_set") {
                    self.global_buffer.push_str("declare i64 @m_set(i64, i64, i64)\n");
                    self.declared_functions.insert("m_set".to_string());
                }
                self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_val, k_val, v_val));
            }
            MIRInstruction::LoadIndex { dst, obj, index } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);
                
                if obj.get_type().is_array() {
                    // --- ULTIMATE OPTIMIZATION: INLINED CACHE CHECK ---
                    if !self.declared_functions.contains("LAST_ID") {
                        self.global_buffer.push_str("@LAST_ID = external global i64\n");
                        self.global_buffer.push_str("@LAST_PTR = external global i8*\n");
                        self.global_buffer.push_str("@LAST_LEN = external global i64\n");
                        self.global_buffer.push_str("@LAST_ELEM_SIZE = external global i64\n");
                        self.declared_functions.insert("LAST_ID".to_string());
                    }
                    if !self.declared_functions.contains("rt_array_get_fast") {
                        self.global_buffer.push_str("declare i64 @rt_array_get_fast(i64, i64)\n");
                        self.declared_functions.insert("rt_array_get_fast".to_string());
                    }

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;
                    
                    self.temp_counter += 1;
                    let last_id = format!("%last_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ID", last_id));

                    self.temp_counter += 1;
                    let id_match = format!("%id_match{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp eq i64 {}, {}", id_match, last_id, obj_val));

                    let fast_path = format!("array_get_fast{}", label_id);
                    let slow_path = format!("array_get_slow{}", label_id);
                    let done_path = format!("array_get_done{}", label_id);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", id_match, fast_path, slow_path));

                    self.emit_line(&format!("{}:", fast_path));
                    self.temp_counter += 1;
                    let last_len = format!("%last_len{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_LEN", last_len));
                    self.temp_counter += 1;
                    let in_bounds = format!("%in_bounds{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp ult i64 {}, {}", in_bounds, idx_val, last_len));
                    
                    let fast_access = format!("array_get_access{}", label_id);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", in_bounds, fast_access, slow_path));

                    self.emit_line(&format!("{}:", fast_access));
                    self.temp_counter += 1;
                    let ptr = format!("%ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @LAST_PTR", ptr));
                    self.temp_counter += 1;
                    let elem_size = format!("%elem_size{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ELEM_SIZE", elem_size));
                    self.temp_counter += 1;
                    let is_byte = format!("%is_byte{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp eq i64 {}, 1", is_byte, elem_size));
                    
                    let get_byte = format!("array_get_byte{}", label_id);
                    let get_qword = format!("array_get_qword{}", label_id);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", is_byte, get_byte, get_qword));

                    self.emit_line(&format!("{}:", get_byte));
                    self.temp_counter += 1;
                    let gep8 = format!("%gep8_{}", self.temp_counter);
                    self.emit_line(&format!("{} = getelementptr i8, i8* {}, i64 {}", gep8, ptr, idx_val));
                    self.temp_counter += 1;
                    let val8 = format!("%val8_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8, i8* {}", val8, gep8));
                    self.temp_counter += 1;
                    let res8 = format!("%res8_{}", self.temp_counter);
                    self.emit_line(&format!("{} = zext i8 {} to i64", res8, val8));
                    
                    self.temp_counter += 1;
                    let res8_f = format!("%res8_f{}", self.temp_counter);
                    self.emit_line(&format!("{} = sitofp i64 {} to double", res8_f, res8));
                    self.temp_counter += 1;
                    let res8_bc = format!("%res8_bc{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast double {} to i64", res8_bc, res8_f));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", get_qword));
                    self.temp_counter += 1;
                    let ptr64 = format!("%ptr64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i8* {} to i64*", ptr64, ptr));
                    self.temp_counter += 1;
                    let gep64 = format!("%gep64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = getelementptr i64, i64* {}, i64 {}", gep64, ptr64, idx_val));
                    self.temp_counter += 1;
                    let res64 = format!("%res64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* {}", res64, gep64));
                    
                    self.temp_counter += 1;
                    let elem_type = obj.get_type().get_array_element_type();
                    let is_numeric_elem = elem_type.is_numeric();
                    
                    let res64_f = format!("%res64_f{}", self.temp_counter);
                    if is_numeric_elem {
                        self.emit_line(&format!("{} = sitofp i64 {} to double", res64_f, res64));
                    } else {
                        self.emit_line(&format!("{} = bitcast i64 {} to double", res64_f, res64));
                    }
                    
                    self.temp_counter += 1;
                    let res64_raw = format!("%res64_raw{}", self.temp_counter);
                    self.emit_line(&format!("{} = fptosi double {} to i64", res64_raw, res64_f));
                    
                    self.temp_counter += 1;
                    let res64_f_bc = format!("%res64_f_bc{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast double {} to i64", res64_f_bc, res64_f));

                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", slow_path));
                    self.temp_counter += 1;
                    let slow_res = format!("%slow_res{}", self.temp_counter);
                    self.emit_line(&format!("{} = call i64 @rt_array_get_fast(i64 {}, i64 {})", slow_res, obj_val, idx_val));
                    
                    self.temp_counter += 1;
                    let slow_f = format!("%slow_f{}", self.temp_counter);
                    if is_numeric_elem {
                         self.emit_line(&format!("{} = sitofp i64 {} to double", slow_f, slow_res));
                    } else {
                         self.emit_line(&format!("{} = bitcast i64 {} to double", slow_f, slow_res));
                    }
                    
                    self.temp_counter += 1;
                    let slow_raw = format!("%slow_raw{}", self.temp_counter);
                    self.emit_line(&format!("{} = fptosi double {} to i64", slow_raw, slow_f));
                    
                    self.temp_counter += 1;
                    let slow_f_bc = format!("%slow_f_bc{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast double {} to i64", slow_f_bc, slow_f));

                    self.emit_line(&format!("br label %{}", done_path));
 
                    self.emit_line(&format!("{}:", done_path));
                    
                    let dst_ty = _func.variables.get(dst).unwrap_or(&TejxType::Any);
                    if dst_ty.is_numeric() && !dst_ty.is_float() {
                         self.temp_counter += 1;
                         let final_res = format!("%final_res{}", self.temp_counter);
                         self.emit_line(&format!("{} = phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]", 
                             final_res, res8, get_byte, res64_raw, get_qword, slow_raw, slow_path));
                         let ptr = self.resolve_ptr(dst);
                         self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
                    } else {
                         // Destination is Any.
                         // If we are loading from a typed numeric array, we MUST bitcast the raw value to double.
                         let elem_type = obj.get_type().get_array_element_type();
                         let is_numeric_elem = elem_type.is_numeric();
                         
                         let (final_qword, final_slow) = if is_numeric_elem {
                             (res64_f_bc.clone(), slow_f_bc.clone())
                         } else {
                             (res64.clone(), slow_res.clone())
                         };

                         self.temp_counter += 1;
                         let final_res = format!("%final_res{}", self.temp_counter);
                         self.emit_line(&format!("{} = phi i64 [ {}, %{} ], [ {}, %{} ], [ {}, %{} ]", 
                             final_res, res8_bc, get_byte, final_qword, get_qword, final_slow, slow_path));
                         let ptr = self.resolve_ptr(dst);
                         self.emit_line(&format!("store i64 {}, i64* {}", final_res, ptr));
                    }
                } else {
                    if !self.declared_functions.contains("m_get") {
                        self.global_buffer.push_str("declare i64 @m_get(i64, i64)\n");
                        self.declared_functions.insert("m_get".to_string());
                    }
                    self.emit_line(&format!("{} = call i64 @m_get(i64 {}, i64 {})", res_tmp, obj_val, idx_val));
                    let ptr = self.resolve_ptr(dst);
                self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
                }
            }
            MIRInstruction::StoreIndex { obj, index, src } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                let v_val = self.resolve_value(src);
                
                if obj.get_type().is_array() {
                    // --- ULTIMATE OPTIMIZATION: INLINED CACHE CHECK ---
                    if !self.declared_functions.contains("LAST_ID") {
                        self.global_buffer.push_str("@LAST_ID = external global i64\n");
                        self.global_buffer.push_str("@LAST_PTR = external global i8*\n");
                        self.global_buffer.push_str("@LAST_LEN = external global i64\n");
                        self.global_buffer.push_str("@LAST_ELEM_SIZE = external global i64\n");
                        self.declared_functions.insert("LAST_ID".to_string());
                    }
                    if !self.declared_functions.contains("rt_array_set_fast") {
                        self.global_buffer.push_str("declare i64 @rt_array_set_fast(i64, i64, i64)\n");
                        self.declared_functions.insert("rt_array_set_fast".to_string());
                    }

                    let label_id = self.temp_counter;
                    self.temp_counter += 1;
                    let done_path = format!("array_set_done{}", label_id);

                    let entry = bb_name;
                    self.temp_counter += 1;
                    let idx_i = format!("%idx_i{}", self.temp_counter);
                    // Safe range for raw integers: 0 to 200M. Bitcasted doubles are usually huge.
                    self.emit_line(&format!("{} = icmp slt i64 {}, 200000000", idx_i, idx_val));
                    
                    let get_idx_f = format!("get_idx_f{}", label_id);
                    let check_idx_fast = format!("check_idx_fast{}", label_id);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", idx_i, get_idx_f, get_idx_f));
                    
                    self.emit_line(&format!("{}:", get_idx_f));
                    self.temp_counter += 1;
                    let idx_f = format!("%idx_f{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i64 {} to double", idx_f, idx_val));
                    self.temp_counter += 1;
                    let idx_from_f = format!("%idx_from_f{}", self.temp_counter);
                    self.emit_line(&format!("{} = fptosi double {} to i64", idx_from_f, idx_f));
                    self.emit_line(&format!("br label %{}", check_idx_fast));
                    
                    self.emit_line(&format!("{}:", check_idx_fast));
                    self.temp_counter += 1;
                    let idx_norm = format!("%idx_norm{}", self.temp_counter);
                    self.emit_line(&format!("{} = phi i64 [ {}, %{} ], [ {}, %{} ]", idx_norm, idx_val, entry, idx_from_f, get_idx_f));

                    self.temp_counter += 1;
                    let last_id = format!("%last_id{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ID", last_id));

                    self.temp_counter += 1;
                    let id_match = format!("%id_match{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp eq i64 {}, {}", id_match, last_id, obj_val));

                    let fast_path = format!("array_set_fast{}", label_id);
                    let slow_path = format!("array_set_slow{}", label_id);
                    let check_len = format!("array_set_check_len{}", label_id);

                    // Branch to check_len if ID matches, else slow path
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", id_match, check_len, slow_path));

                    self.emit_line(&format!("{}:", check_len));
                    self.temp_counter += 1;
                    let last_len = format!("%last_len{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_LEN", last_len));

                    self.temp_counter += 1;
                    let in_bounds = format!("%in_bounds{}", self.temp_counter);
                    // Check bounds normally
                    self.emit_line(&format!("{} = icmp ult i64 {}, {}", in_bounds, idx_norm, last_len));
                    
                    let fast_access = format!("array_set_access{}", label_id);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", in_bounds, fast_access, slow_path));

                    self.emit_line(&format!("{}:", fast_access));
                    self.temp_counter += 1;
                    let ptr = format!("%ptr{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i8*, i8** @LAST_PTR", ptr));
                    self.temp_counter += 1;
                    let elem_size = format!("%elem_size{}", self.temp_counter);
                    self.emit_line(&format!("{} = load i64, i64* @LAST_ELEM_SIZE", elem_size));
                    self.temp_counter += 1;
                    let is_byte = format!("%is_byte{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp eq i64 {}, 1", is_byte, elem_size));
                    
                    let set_byte = format!("array_set_byte{}", label_id);
                    let set_qword = format!("array_set_qword{}", label_id);
                    self.emit_line(&format!("br i1 {}, label %{}, label %{}", is_byte, set_byte, set_qword));

                    self.emit_line(&format!("{}:", set_byte));
                    self.temp_counter += 1;
                    let gep8 = format!("%gep8_{}", self.temp_counter);
                    self.emit_line(&format!("{} = getelementptr i8, i8* {}, i64 {}", gep8, ptr, idx_norm));
                    
                    // Value conversion for byte array
                    let src_ty = src.get_type();
                    let v_to_store = if !src_ty.is_numeric() || src_ty.is_float() {
                         self.temp_counter += 1;
                         let v_f = format!("%v_f{}", self.temp_counter);
                         self.emit_line(&format!("{} = bitcast i64 {} to double", v_f, v_val));
                         self.temp_counter += 1;
                         let v_i = format!("%v_i{}", self.temp_counter);
                         self.emit_line(&format!("{} = fptosi double {} to i8", v_i, v_f));
                         v_i
                    } else {
                         self.temp_counter += 1;
                         let v_i = format!("%v_i{}", self.temp_counter);
                         self.emit_line(&format!("{} = trunc i64 {} to i8", v_i, v_val));
                         v_i
                    };
                    
                    self.emit_line(&format!("store i8 {}, i8* {}", v_to_store, gep8));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", set_qword));
                    self.temp_counter += 1;
                    let ptr64 = format!("%ptr64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = bitcast i8* {} to i64*", ptr64, ptr));
                    self.temp_counter += 1;
                    let gep64 = format!("%gep64_{}", self.temp_counter);
                    self.emit_line(&format!("{} = getelementptr i64, i64* {}, i64 {}", gep64, ptr64, idx_norm));
                    self.emit_line(&format!("store i64 {}, i64* {}", v_val, gep64));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", slow_path));
                    self.emit_line(&format!("call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})", obj_val, idx_val, v_val));
                    self.emit_line(&format!("br label %{}", done_path));

                    self.emit_line(&format!("{}:", done_path));
                } else {
                    if !self.declared_functions.contains("m_set") {
                        self.global_buffer.push_str("declare i64 @m_set(i64, i64, i64)\n");
                        self.declared_functions.insert("m_set".to_string());
                    }
                    self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_val, idx_val, v_val));
                }
            }
            MIRInstruction::Throw { value } => {
                let val = self.resolve_value(value);
                self.emit_line(&format!("call void @tejx_throw(i64 {})", val));
                self.emit_line("unreachable");
            }
            MIRInstruction::Cast { dst, src, ty } => {
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
                           self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, s));
                      }
                 } else {
                      // Generic bitcast for other types
                      self.emit_line(&format!("{} = bitcast i64 {} to i64", tmp, s));
                 }
                 let ptr = self.resolve_ptr(dst);
                 self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
            }
        }
    }

    fn find_block_idx(&self, func: &MIRFunction, inst: &MIRInstruction) -> Option<usize> {
        for (i, bb) in func.blocks.iter().enumerate() {
            for bi in &bb.instructions {
                if std::ptr::eq(bi, inst) {
                    return Some(i);
                }
            }
        }
        None
    }
}
