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

                let is_number = match ty {
                    TejxType::Primitive(name) => name == "number" || name == "int" || name == "float",
                    TejxType::Any => {
                         // Check if value parses as number
                         value.parse::<f64>().is_ok()
                    },
                    _ => false,
                };
                let is_bool = match ty {
                    TejxType::Primitive(name) => name == "boolean",
                     _ => false,
                };

                if is_bool {
                     if value == "true" || value == "1" { return "1".to_string(); }
                     return "0".to_string();
                }

                if is_number {
                    // Always convert to f64 bits to ensure runtime consistency (fadd/fsub expect bits)
                    if let Ok(d) = value.parse::<f64>() {
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
                    // Unknown variable or temporary?
                    // If it's a temporary like %t0, %call1 etc, they are values, not pointers?
                    // Wait, temporaries are usually in value_map? NO.
                    // MIRLowering new_temp returns name like "t0".
                    // CodeGen doesn't put "t0" in value_map?
                    // In gen_instruction_v2, dst is put in value_map only if it is Move/Binary/Call dest?
                    // Actually, gen_function_v2 iterates instructions to create allocas for dsts.
                    // So temporaries ARE allocas in current CodeGen strategy!
                    // except if it missed something.
                    // But __arg0 is NOT a temporary that is written to (it's read from).
                    // So it is NOT in value_map.
                    
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

    fn resolve_ptr(&self, name: &str) -> String {
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
        for f in functions {
            self.declared_functions.insert(f.name.clone());
            self.function_param_counts.insert(f.name.clone(), f.params.len());
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

        // Declarations are now handled on-demand during instruction generation
        // to support LTO-style linking (only declared if used).
        
        self.global_buffer.push_str("@.fmt_d = private unnamed_addr constant [5 x i8] c\"%lld\\00\"\n");
        self.global_buffer.push_str("@.fmt_f = private unnamed_addr constant [3 x i8] c\"%f\\00\"\n");
        self.global_buffer.push_str("@.fmt_s = private unnamed_addr constant [3 x i8] c\"%s\\00\"\n");
        self.global_buffer.push_str("@.fmt_nl = private unnamed_addr constant [2 x i8] c\"\\0A\\00\"\n");
        self.global_buffer.push_str("@.fmt_sp = private unnamed_addr constant [2 x i8] c\" \\00\"\n");

        for func in functions {
            self.gen_function_v2(func);
        }

        format!("{}{}", self.global_buffer, self.buffer)
    }

    fn gen_function_v2(&mut self, func: &MIRFunction) {
        self.value_map.clear();
        self.temp_counter = 0;

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
        
        // Allocas for all other variables
        for bb in &func.blocks {
            for inst in &bb.instructions {
                let dest_var = match inst {
                    MIRInstruction::Move { dst, .. } => Some(dst.clone()),
                    MIRInstruction::BinaryOp { dst, .. } => Some(dst.clone()),
                    MIRInstruction::Call { dst, .. } => Some(dst.clone()),
                    MIRInstruction::ObjectLiteral { dst, .. } => Some(dst.clone()),
                    MIRInstruction::ArrayLiteral { dst, .. } => Some(dst.clone()),
                    MIRInstruction::LoadMember { dst, .. } => Some(dst.clone()),
                    MIRInstruction::LoadIndex { dst, .. } => Some(dst.clone()),
                    MIRInstruction::IndirectCall { dst, .. } => Some(dst.clone()),
                    _ => None,
                };
                if let Some(name) = dest_var {
                    if !name.starts_with("g_") && !self.value_map.contains_key(&name) {
                        let reg_name = format!("%{}_ptr", name);
                        self.emit_line(&format!("{} = alloca i64", reg_name));
                        self.value_map.insert(name, reg_name);
                    }
                }
            }
        }

        // Branch to first block
        if !func.blocks.is_empty() {
            self.emit_line(&format!("br label %{}", func.blocks[0].name));
        } else {
            self.emit_line("ret i64 0");
        }

        // Generate blocks with block name resolution
        for bb in &func.blocks {
            self.emit(&format!("{}:\n", bb.name));
            for inst in &bb.instructions {
                self.gen_instruction_v2(inst, func);
            }
        }

        self.emit("}\n\n");
    }

    fn gen_instruction_v2(&mut self, inst: &MIRInstruction, func: &MIRFunction) {
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
                let is_string_op = matches!(l_ty, TejxType::Primitive(n) if n == "string") || 
                                matches!(r_ty, TejxType::Primitive(n) if n == "string");
                
                let is_any_op = matches!(l_ty, TejxType::Any) || matches!(r_ty, TejxType::Any);
                let is_float_op = matches!(l_ty, TejxType::Primitive(n) if n == "number" || n == "float") || 
                               matches!(r_ty, TejxType::Primitive(n) if n == "number" || n == "float");

                if is_any_op {
                    let runtime_func = match op {
                        TokenType::Plus => Some("rt_add"),
                        TokenType::Minus => Some("rt_sub"),
                        TokenType::Star => Some("rt_mul"),
                        TokenType::Slash => Some("rt_div"),
                        _ => None
                    };

                    if let Some(func_name) = runtime_func {
                         if !self.declared_functions.contains(func_name) {
                            self.global_buffer.push_str(&format!("declare i64 @{}(i64, i64)\n", func_name));
                            self.declared_functions.insert(func_name.to_string());
                        }
                        
                        let l_is_num = matches!(l_ty, TejxType::Primitive(n) if n == "number" || n == "float");
                        let r_is_num = matches!(r_ty, TejxType::Primitive(n) if n == "number" || n == "float");
                        
                        let l_val = if l_is_num {
                            if !self.declared_functions.contains("rt_box_number") {
                                self.global_buffer.push_str("declare i64 @rt_box_number(i64)\n");
                                self.declared_functions.insert("rt_box_number".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_box_number(i64 {})", boxed, l));
                            boxed
                        } else {
                            l.to_string()
                        };

                        let r_val = if r_is_num {
                            if !self.declared_functions.contains("rt_box_number") {
                                self.global_buffer.push_str("declare i64 @rt_box_number(i64)\n");
                                self.declared_functions.insert("rt_box_number".to_string());
                            }
                            self.temp_counter += 1;
                            let boxed = format!("%boxed{}", self.temp_counter);
                            self.emit_line(&format!("{} = call i64 @rt_box_number(i64 {})", boxed, r));
                            boxed
                        } else {
                            r.to_string()
                        };

                        self.emit_line(&format!("{} = call i64 @{}(i64 {}, i64 {})", tmp, func_name, l_val, r_val));
                    } else {
                         let (is_cmp, llvm_op, pred) = match op {
                             TokenType::EqualEqual => (true, "", "eq"),
                             TokenType::BangEqual => (true, "", "ne"),
                             TokenType::Less => (true, "", "slt"),
                             TokenType::Greater => (true, "", "sgt"),
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
                    }
                } else if is_string_op {
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
                         self.temp_counter += 1;
                         let eq_tmp = format!("%eq{}", self.temp_counter);
                         self.emit_line(&format!("{} = call i64 @rt_str_equals(i64 {}, i64 {})", eq_tmp, l, r));
                         self.emit_line(&format!("{} = call i64 @rt_not(i64 {})", tmp, eq_tmp));
                    } else {
                         // Fallback?
                         self.emit_line(&format!("{} = add i64 {}, {}", tmp, l, r));
                    }
                } else if is_float_op {
                     // Convert inputs to double
                     self.temp_counter += 1;
                     let l_dbl = format!("%l_dbl{}", self.temp_counter);
                     self.emit_line(&format!("{} = bitcast i64 {} to double", l_dbl, l));
                     
                     self.temp_counter += 1;
                     let r_dbl = format!("%r_dbl{}", self.temp_counter);
                     self.emit_line(&format!("{} = bitcast i64 {} to double", r_dbl, r));

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
                        _ => (false, "fadd", "")
                     };

                     if is_cmp {
                         self.temp_counter += 1;
                         let cmp_res = format!("%cmp_res{}", self.temp_counter);
                         self.emit_line(&format!("{} = fcmp {} double {}, {}", cmp_res, pred, l_dbl, r_dbl));
                         self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_res));
                     } else {
                         self.temp_counter += 1;
                         let res_dbl = format!("%res_dbl{}", self.temp_counter);
                         self.emit_line(&format!("{} = {} double {}, {}", res_dbl, llvm_op, l_dbl, r_dbl));
                         self.emit_line(&format!("{} = bitcast double {} to i64", tmp, res_dbl));
                     }
                } else {
                    // Integer / Default
                    let (is_cmp, llvm_op, pred) = match op {
                        TokenType::Plus => (false, "add", ""),
                        TokenType::Minus => (false, "sub", ""),
                        TokenType::Star => (false, "mul", ""),
                        TokenType::Slash => (false, "sdiv", ""),
                        TokenType::Modulo => (false, "srem", ""),
                        TokenType::Less => (true, "", "slt"),
                        TokenType::Greater => (true, "", "sgt"),
                        TokenType::EqualEqual => (true, "", "eq"),
                        TokenType::BangEqual => (true, "", "ne"),
                        TokenType::LessEqual => (true, "", "sle"),
                        TokenType::GreaterEqual => (true, "", "sge"),
                        _ => (false, "add", "")
                    };

                    if is_cmp {
                        self.temp_counter += 1;
                        let cmp_tmp = format!("%cmp{}", self.temp_counter);
                        self.emit_line(&format!("{} = icmp {} i64 {}, {}", cmp_tmp, pred, l, r));
                        self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_tmp));
                    } else {
                        self.emit_line(&format!("{} = {} i64 {}, {}", tmp, llvm_op, l, r));
                    }
                }
                
                self.emit_line(&format!("store i64 {}, i64* {}", tmp, self.resolve_ptr(dst)));
            }


            MIRInstruction::Jump { target } => {
                if *target < func.blocks.len() {
                    self.emit_line(&format!("br label %{}", func.blocks[*target].name));
                }
            }
            MIRInstruction::Branch { condition, true_target, false_target } => {
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
                self.emit_line(&format!("br i1 {}, label %{}, label %{}", cmp, true_name, false_name));
            }
            MIRInstruction::Return { value } => {
                if let Some(val) = value {
                    let v = self.resolve_value(val);
                    self.emit_line(&format!("ret i64 {}", v));
                } else {
                    self.emit_line("ret i64 0");
                }
            }
            MIRInstruction::Call { dst, callee, args } => {
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

                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.emit_line("call void @print_space()");
                        }
                        let arg_val = self.resolve_value(arg);
                        self.emit_line(&format!("call void @print_raw(i64 {})", arg_val));
                    }
                    self.emit_line("call void @print_newline()");
                    
                    if !dst.is_empty() {
                         self.emit_line(&format!("store i64 0, i64* {}", self.resolve_ptr(dst)));
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

                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.emit_line("call void @eprint_space()");
                        }
                        let arg_val = self.resolve_value(arg);
                        self.emit_line(&format!("call void @eprint_raw(i64 {})", arg_val));
                    }
                    self.emit_line("call void @eprint_newline()");

                    if let Some(ptr) = self.value_map.get(dst) {
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
                            self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, self.resolve_ptr(dst)));
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
                        self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, self.resolve_ptr(dst)));
                    }
                }
            }
            MIRInstruction::IndirectCall { dst, callee, args } => {
                let callee_val = self.resolve_value(callee);
                
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
                    self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, self.resolve_ptr(dst)));
                }
            }
            MIRInstruction::ObjectLiteral { dst, entries } => {
                self.temp_counter += 1;
                let obj_tmp = format!("%obj{}", self.temp_counter);
                if !self.declared_functions.contains("m_new") {
                    self.global_buffer.push_str("declare i64 @m_new()\n");
                    self.declared_functions.insert("m_new".to_string());
                }
                self.emit_line(&format!("{} = call i64 @m_new()", obj_tmp));
                for (k, v) in entries {
                    let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", k), ty: TejxType::Primitive("string".to_string()) });
                    let v_val = self.resolve_value(v);
                    if !self.declared_functions.contains("m_set") {
                        self.global_buffer.push_str("declare i64 @m_set(i64, i64, i64)\n");
                        self.declared_functions.insert("m_set".to_string());
                    }
                    self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_tmp, k_val, v_val));
                }
                self.emit_line(&format!("store i64 {}, i64* {}", obj_tmp, self.resolve_ptr(dst)));
            }
            MIRInstruction::ArrayLiteral { dst, elements } => {
                self.temp_counter += 1;
                let arr_tmp = format!("%arr{}", self.temp_counter);
                if !self.declared_functions.contains("a_new") {
                    self.global_buffer.push_str("declare i64 @a_new()\n");
                    self.declared_functions.insert("a_new".to_string());
                }
                self.emit_line(&format!("{} = call i64 @a_new()", arr_tmp));
                for v in elements {
                    let v_val = self.resolve_value(v);
                    if !self.declared_functions.contains("Array_push") {
                        self.global_buffer.push_str("declare i64 @Array_push(i64, i64)\n");
                        self.declared_functions.insert("Array_push".to_string());
                    }
                    self.emit_line(&format!("call i64 @Array_push(i64 {}, i64 {})", arr_tmp, v_val));
                }
                self.emit_line(&format!("store i64 {}, i64* {}", arr_tmp, self.resolve_ptr(dst)));
            }
            MIRInstruction::LoadMember { dst, obj, member } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", member), ty: TejxType::Primitive("string".to_string()) });
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);
                if !self.declared_functions.contains("m_get") {
                    self.global_buffer.push_str("declare i64 @m_get(i64, i64)\n");
                    self.declared_functions.insert("m_get".to_string());
                }
                self.emit_line(&format!("{} = call i64 @m_get(i64 {}, i64 {})", res_tmp, obj_val, k_val));
                self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, self.resolve_ptr(dst)));
            }
            MIRInstruction::StoreMember { obj, member, src } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", member), ty: TejxType::Primitive("string".to_string()) });
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
                if !self.declared_functions.contains("m_get") {
                    self.global_buffer.push_str("declare i64 @m_get(i64, i64)\n");
                    self.declared_functions.insert("m_get".to_string());
                }
                self.emit_line(&format!("{} = call i64 @m_get(i64 {}, i64 {})", res_tmp, obj_val, idx_val));
                self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, self.resolve_ptr(dst)));
            }
            MIRInstruction::StoreIndex { obj, index, src } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                let v_val = self.resolve_value(src);
                if !self.declared_functions.contains("m_set") {
                    self.global_buffer.push_str("declare i64 @m_set(i64, i64, i64)\n");
                    self.declared_functions.insert("m_set".to_string());
                }
                self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_val, idx_val, v_val));
            }
        }
    }
}
