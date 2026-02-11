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
                    return format!("ptrtoint (i64 (...)* @{} to i64)", value);
                }
                if value.starts_with("new ") {
                    return "0".to_string();
                }

                let is_number = match ty {
                    TejxType::Primitive(name) => name == "number" || name == "int" || name == "float",
                    _ => false,
                };

                if is_number {
                    if value.contains('.') {
                        // Float → truncate to int
                        if let Ok(d) = value.parse::<f64>() {
                            return format!("{}", d as i64);
                        }
                        return "0".to_string();
                    }
                    return value.clone();
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

                self.global_buffer.push_str(&format!(
                    "{} = private unnamed_addr constant [{} x i8] c\"{}\\00\"\n",
                    str_lbl, len, content
                ));

                // Return ptrtoint cast to i64
                format!("ptrtoint ([{} x i8]* {} to i64)", len, str_lbl)
            }
            MIRValue::Variable { name, .. } => {
                if let Some(ptr) = self.value_map.get(name) {
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
                         // Unknown or global symbol (Status, Dog, etc.)
                         // Return "0" to keep LLVM happy; these are stubs for now.
                         "0".to_string()
                    }
                }
            }
        }
    }
}

/// Second pass: fix Jump and Branch instructions to use block names
impl CodeGen {
    pub fn generate_with_blocks(&mut self, functions: &[MIRFunction]) -> String {
        self.buffer.clear();
        self.global_buffer.clear();
        self.declared_functions.clear();
        
        // Register defined functions to avoid auto-declaring them
        for f in functions {
            self.declared_functions.insert(f.name.clone());
        }

        // Standard declarations
        self.global_buffer.push_str("declare i32 @printf(i8*, ...)\n");
        self.declared_functions.insert("printf".to_string());
        


        self.global_buffer.push_str("@.fmt_d = private unnamed_addr constant [5 x i8] c\"%lld\\00\"\n");
        self.global_buffer.push_str("@.fmt_s = private unnamed_addr constant [3 x i8] c\"%s\\00\"\n");
        self.global_buffer.push_str("@.fmt_nl = private unnamed_addr constant [2 x i8] c\"\\0A\\00\"\n");
        self.global_buffer.push_str("@.fmt_sp = private unnamed_addr constant [2 x i8] c\" \\00\"\n");

        let runtime_funcs = vec![
            ("Math_pow", "i64, i64"), ("fs_exists", "i64"), ("Array_push", "i64, i64"), 
            ("Array_pop", "i64"), ("arrUtil_concat", "i64, i64"), ("Thread_join", "i64"), 
            ("__await", "i64"), ("__optional_chain", "i64, i64"), 
            ("add", "i64, i64"), ("multiply", "i64, i64"), 
            ("Calculator_add", "i64, i64"), ("Calculator_getValue", "i64"),
            ("calc_add", "i64, i64"), ("calc_getValue", "i64"), 
            ("hello", "i64"), ("__callee___area", ""),
            ("arrUtil_indexOf", "i64, i64"), ("arrUtil_shift", "i64"), 
            ("arrUtil_unshift", "i64, i64"), ("Array_forEach", "i64, i64"), 
            ("Array_map", "i64, i64"), ("Array_filter", "i64, i64"),
            ("Date_now", ""), ("fs_mkdir", "i64"), ("fs_readFile", "i64"), 
            ("fs_writeFile", "i64, i64"), ("fs_remove", "i64"), ("Promise_all", "i64"), 
            ("delay", "i64"), ("http_get", "i64"),
            ("Math_abs", "i64"), ("Math_ceil", "i64"), ("Math_floor", "i64"), 
            ("Math_round", "i64"), ("Math_sqrt", "i64"), ("Math_sin", "i64"), 
            ("Math_cos", "i64"), ("Math_random", ""), ("Math_min", "i64, i64"), 
            ("Math_max", "i64, i64"), ("parseInt", "i64"), ("parseFloat", "i64"), 
            ("JSON_stringify", "i64"), ("JSON_parse", "i64"),
            ("console_error", "i64"), ("console_warn", "i64"), 
            ("d_getTime", "i64"), ("d_toISOString", "i64"),
            ("m_new", ""), ("a_new", ""),
            ("m_set", "i64, i64, i64"), ("m_get", "i64, i64"), ("m_has", "i64, i64"), 
            ("m_del", "i64, i64"), ("m_size", "i64"),
            ("s_add", "i64, i64"), ("s_has", "i64, i64"), ("s_size", "i64"),
            ("strVal_trim", "i64"), ("trimmed_startsWith", "i64, i64"), 
            ("trimmed_endsWith", "i64, i64"), ("trimmed_replace", "i64, i64, i64"), 
            ("trimmed_toLowerCase", "i64"), ("trimmed_toUpperCase", "i64"),
            ("m_lock", "i64"), ("m_unlock", "i64")
        ];

        for (name, args) in runtime_funcs {
            if !self.declared_functions.contains(name) {
                self.global_buffer.push_str(&format!("declare i64 @{}({})\n", name, args));
                self.declared_functions.insert(name.to_string());
            }
        }

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
        for p in &func.params {
            if !self.value_map.contains_key(p) {
                let reg_name = format!("%{}_ptr", p);
                self.emit_line(&format!("{} = alloca i64", reg_name));
                self.value_map.insert(p.clone(), reg_name);
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
                    _ => None,
                };
                if let Some(name) = dest_var {
                    if !self.value_map.contains_key(&name) {
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
                let ptr = self.value_map.get(dst).cloned().unwrap_or_else(|| format!("%{}_ptr", dst));
                self.emit_line(&format!("store i64 {}, i64* {}", val, ptr));
            }
            MIRInstruction::BinaryOp { dst, left, op, right } => {
                let l = self.resolve_value(left);
                let r = self.resolve_value(right);
                self.temp_counter += 1;
                let tmp = format!("%tmp{}", self.temp_counter);

                let (is_compare, llvm_op, cmp_pred) = match op {
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
                    _ => (false, "add", ""),
                };

                if is_compare {
                    self.temp_counter += 1;
                    let cmp_tmp = format!("%cmp{}", self.temp_counter);
                    self.emit_line(&format!("{} = icmp {} i64 {}, {}", cmp_tmp, cmp_pred, l, r));
                    self.emit_line(&format!("{} = zext i1 {} to i64", tmp, cmp_tmp));
                } else {
                    self.emit_line(&format!("{} = {} i64 {}, {}", tmp, llvm_op, l, r));
                }

                let ptr = self.value_map.get(dst).cloned().unwrap_or_else(|| format!("%{}_ptr", dst));
                self.emit_line(&format!("store i64 {}, i64* {}", tmp, ptr));
            }
            MIRInstruction::Jump { target } => {
                if *target < func.blocks.len() {
                    self.emit_line(&format!("br label %{}", func.blocks[*target].name));
                }
            }
            MIRInstruction::Branch { condition, true_target, false_target } => {
                let cond = self.resolve_value(condition);
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
                if callee == "console.log" || callee == "printf" {
                    for arg in args {
                        let arg_val = self.resolve_value(arg);
                        let is_str = matches!(arg.get_type(), TejxType::Primitive(n) if n == "string");

                        if is_str {
                            self.temp_counter += 1;
                            let cast_tmp = format!("%str{}", self.temp_counter);
                            self.emit_line(&format!("{} = inttoptr i64 {} to i8*", cast_tmp, arg_val));
                            self.temp_counter += 1;
                            let fmt_tmp = format!("%fmt{}", self.temp_counter);
                            self.emit_line(&format!("{} = getelementptr inbounds [3 x i8], [3 x i8]* @.fmt_s, i64 0, i64 0", fmt_tmp));
                            self.emit_line(&format!("call i32 (i8*, ...) @printf(i8* {}, i8* {})", fmt_tmp, cast_tmp));
                        } else {
                            self.temp_counter += 1;
                            let fmt_tmp = format!("%fmt{}", self.temp_counter);
                            self.emit_line(&format!("{} = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_d, i64 0, i64 0", fmt_tmp));
                            self.emit_line(&format!("call i32 (i8*, ...) @printf(i8* {}, i64 {})", fmt_tmp, arg_val));
                        }
                        
                        // Print space between args
                        self.temp_counter += 1;
                        let sp_tmp = format!("%sp{}", self.temp_counter);
                        self.emit_line(&format!("{} = getelementptr inbounds [2 x i8], [2 x i8]* @.fmt_sp, i64 0, i64 0", sp_tmp));
                        self.emit_line(&format!("call i32 (i8*, ...) @printf(i8* {})", sp_tmp));
                    }
                    self.temp_counter += 1;
                    let nl_tmp = format!("%nl{}", self.temp_counter);
                    self.emit_line(&format!("{} = getelementptr inbounds [2 x i8], [2 x i8]* @.fmt_nl, i64 0, i64 0", nl_tmp));
                    self.emit_line(&format!("call i32 (i8*, ...) @printf(i8* {})", nl_tmp));
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
                        self.emit_line(&format!("{} = inttoptr i64 {} to i64 (...)*", func_ptr_tmp, func_val_tmp));
                        let mut call_arg_vals = Vec::new();
                        for arg in args {
                            let arg_val = self.resolve_value(arg);
                            call_arg_vals.push(format!("i64 {}", arg_val));
                        }
                        let args_str = call_arg_vals.join(", ");
                        self.temp_counter += 1;
                        let result_tmp = format!("%call{}", self.temp_counter);
                        self.emit_line(&format!("{} = call i64 {}({})", result_tmp, func_ptr_tmp, args_str));
                        if let Some(p) = self.value_map.get(dst) {
                            self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, p));
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
                        self.global_buffer.push_str(&format!("declare i64 @{}(...)\n", final_callee));
                        self.declared_functions.insert(final_callee.clone());
                    }
                    self.emit_line(&format!("{} = call i64 @{}({})", result_tmp, final_callee, args_str));
                    if let Some(ptr) = self.value_map.get(dst) {
                        self.emit_line(&format!("store i64 {}, i64* {}", result_tmp, ptr));
                    }
                }
            }
            MIRInstruction::ObjectLiteral { dst, entries } => {
                self.temp_counter += 1;
                let obj_tmp = format!("%obj{}", self.temp_counter);
                self.emit_line(&format!("{} = call i64 @m_new()", obj_tmp));
                for (k, v) in entries {
                    let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", k), ty: TejxType::Primitive("string".to_string()) });
                    let v_val = self.resolve_value(v);
                    self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_tmp, k_val, v_val));
                }
                let ptr = self.value_map.get(dst).cloned().unwrap_or_else(|| format!("%{}_ptr", dst));
                self.emit_line(&format!("store i64 {}, i64* {}", obj_tmp, ptr));
            }
            MIRInstruction::ArrayLiteral { dst, elements } => {
                self.temp_counter += 1;
                let arr_tmp = format!("%arr{}", self.temp_counter);
                self.emit_line(&format!("{} = call i64 @a_new()", arr_tmp));
                for v in elements {
                    let v_val = self.resolve_value(v);
                    self.emit_line(&format!("call i64 @Array_push(i64 {}, i64 {})", arr_tmp, v_val));
                }
                let ptr = self.value_map.get(dst).cloned().unwrap_or_else(|| format!("%{}_ptr", dst));
                self.emit_line(&format!("store i64 {}, i64* {}", arr_tmp, ptr));
            }
            MIRInstruction::LoadMember { dst, obj, member } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", member), ty: TejxType::Primitive("string".to_string()) });
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);
                self.emit_line(&format!("{} = call i64 @m_get(i64 {}, i64 {})", res_tmp, obj_val, k_val));
                let ptr = self.value_map.get(dst).cloned().unwrap_or_else(|| format!("%{}_ptr", dst));
                self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
            }
            MIRInstruction::StoreMember { obj, member, src } => {
                let obj_val = self.resolve_value(obj);
                let k_val = self.resolve_value(&MIRValue::Constant { value: format!("\"{}\"", member), ty: TejxType::Primitive("string".to_string()) });
                let v_val = self.resolve_value(src);
                self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_val, k_val, v_val));
            }
            MIRInstruction::LoadIndex { dst, obj, index } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                self.temp_counter += 1;
                let res_tmp = format!("%val{}", self.temp_counter);
                self.emit_line(&format!("{} = call i64 @m_get(i64 {}, i64 {})", res_tmp, obj_val, idx_val));
                let ptr = self.value_map.get(dst).cloned().unwrap_or_else(|| format!("%{}_ptr", dst));
                self.emit_line(&format!("store i64 {}, i64* {}", res_tmp, ptr));
            }
            MIRInstruction::StoreIndex { obj, index, src } => {
                let obj_val = self.resolve_value(obj);
                let idx_val = self.resolve_value(index);
                let v_val = self.resolve_value(src);
                self.emit_line(&format!("call i64 @m_set(i64 {}, i64 {}, i64 {})", obj_val, idx_val, v_val));
            }
        }
    }
}
