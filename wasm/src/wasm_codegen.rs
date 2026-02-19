use crate::mir::*;
use crate::types::TejxType;
use crate::token::TokenType;
use std::collections::HashSet;

pub struct WasmCodeGen {
    buffer: String,
    label_counter: usize,
    string_constants: Vec<String>,
}

impl WasmCodeGen {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            label_counter: 0,
            string_constants: Vec::new(),
        }
    }

    fn emit(&mut self, code: &str) {
        self.buffer.push_str(code);
    }

    fn emit_line(&mut self, code: &str) {
        self.buffer.push_str("    ");
        self.buffer.push_str(code);
        self.buffer.push('\n');
    }

    pub fn generate_wat(&mut self, functions: &[MIRFunction]) -> String {
        self.buffer.clear();
        self.string_constants.clear();

        self.emit("(module\n");
        
        // Imports (ENV matches NovaJs runtime)
        self.emit_line("(import \"env\" \"rt_box_int\" (func $rt_box_int (param i64) (result i64)))");
        self.emit_line("(import \"env\" \"rt_box_string\" (func $rt_box_string (param i64) (result i64)))");
        self.emit_line("(import \"env\" \"rt_box_boolean\" (func $rt_box_boolean (param i64) (result i64)))");
        self.emit_line("(import \"env\" \"rt_box_number\" (func $rt_box_number (param f64) (result i64)))");
        self.emit_line("(import \"env\" \"rt_to_number\" (func $rt_to_number (param i64) (result f64)))");
        self.emit_line("(import \"env\" \"rt_to_boolean\" (func $rt_to_boolean (param i64) (result i64)))");
        self.emit_line("(import \"env\" \"m_new\" (func $m_new (result i64)))");
        self.emit_line("(import \"env\" \"m_set\" (func $m_set (param i64 i64 i64) (result i64)))");
        self.emit_line("(import \"env\" \"m_get\" (func $m_get (param i64 i64) (result i64)))");
        self.emit_line("(import \"env\" \"a_new\" (func $a_new (result i64)))");
        self.emit_line("(import \"env\" \"Array_push\" (func $Array_push (param i64 i64) (result i64)))");
        self.emit_line("(import \"env\" \"rt_array_get_fast\" (func $rt_array_get_fast (param i64 i64) (result i64)))");
        self.emit_line("(import \"env\" \"rt_array_set_fast\" (func $rt_array_set_fast (param i64 i64 i64) (result i64)))");
        self.emit_line("(import \"env\" \"print_raw\" (func $print_raw (param i64)))");
        self.emit_line("(import \"env\" \"print_space\" (func $print_space))");
        self.emit_line("(import \"env\" \"print_newline\" (func $print_newline))");
        self.emit_line("(import \"env\" \"rt_free\" (func $rt_free (param i64)))");
        self.emit_line("(import \"env\" \"memory\" (memory 1))");

        // Types for Indirect Calls (Support up to 10 args for now)
        for n in 0..11 {
            let mut params = String::new();
            for _ in 0..n {
                params.push_str(" i64");
            }
            self.emit_line(&format!("(type $t_func_{} (func (param {}) (result i64)))", n, params));
        }

        // Scan for strings first to build data section
        for func in functions {
            for block in &func.blocks {
                for inst in &block.instructions {
                    self.collect_strings(inst);
                }
            }
        }

        // Emit Data section
        let mut offset = 1024; // Start strings at 1KB offset
        let string_constants = self.string_constants.clone();
        for (i, s) in string_constants.iter().enumerate() {
            let bytes = s.as_bytes();
            let escaped: String = bytes.iter().map(|&b| format!("\\{:02x}", b)).collect();
            self.emit_line(&format!("(data (i32.const {}) \"{}\\00\") ;; str{}", offset, escaped, i));
            offset += bytes.len() + 1; 
        }

        for func in functions.iter() {
            self.generate_function(func);
        }

        // Export main
        let has_tejx_main = functions.iter().any(|f| f.name == "tejx_main");
        let has_main = functions.iter().any(|f| f.name == "main");
        
        if has_tejx_main {
            self.emit_line("(export \"main\" (func $f_tejx_main))");
        } else if has_main {
            self.emit_line("(export \"main\" (func $f_main))");
        } else if !functions.is_empty() {
             let first_name = format!("$f_{}", functions[0].name.replace(".", "_"));
             self.emit_line(&format!("(export \"main\" (func {}))", first_name));
        }
        
        // Table for indirect calls
        let mut func_names = Vec::new();
        for func in functions {
            func_names.push(format!("$f_{}", func.name.replace(".", "_")));
        }
        if !func_names.is_empty() {
            self.emit_line(&format!("(table (export \"table\") {} funcref)", func_names.len()));
            self.emit_line(&format!("(elem (i32.const 0) {})", func_names.join(" ")));
        }

        self.emit(")\n");

        self.buffer.clone()
    }

    fn collect_strings(&mut self, inst: &MIRInstruction) {
        match inst {
            MIRInstruction::Move { src, .. } => self.add_string_if_const(src),
            MIRInstruction::BinaryOp { left, right, .. } => {
                self.add_string_if_const(left);
                self.add_string_if_const(right);
            }
            MIRInstruction::Call { args, .. } => {
                for arg in args {
                    self.add_string_if_const(arg);
                }
            }
            MIRInstruction::Return { value, .. } => {
                if let Some(val) = value {
                    self.add_string_if_const(val);
                }
            }
            MIRInstruction::ObjectLiteral { entries, .. } => {
                for (name, val) in entries {
                    if !self.string_constants.contains(name) {
                        self.string_constants.push(name.clone());
                    }
                    self.add_string_if_const(val);
                }
            }
            MIRInstruction::ArrayLiteral { elements, .. } => {
                for el in elements {
                    self.add_string_if_const(el);
                }
            }
            MIRInstruction::LoadMember { member, obj, .. } => {
                if !self.string_constants.contains(member) {
                    self.string_constants.push(member.clone());
                }
                self.add_string_if_const(obj);
            }
            MIRInstruction::StoreMember { member, obj, src, .. } => {
                if !self.string_constants.contains(member) {
                    self.string_constants.push(member.clone());
                }
                self.add_string_if_const(obj);
                self.add_string_if_const(src);
            }
            MIRInstruction::StoreIndex { obj, index, src, .. } => {
                self.add_string_if_const(obj);
                self.add_string_if_const(index);
                self.add_string_if_const(src);
            }
            MIRInstruction::IndirectCall { callee, args, .. } => {
                self.add_string_if_const(callee);
                for arg in args {
                    self.add_string_if_const(arg);
                }
            }
            MIRInstruction::Cast { src, .. } => self.add_string_if_const(src),
            MIRInstruction::Throw { value, .. } => self.add_string_if_const(value),
            MIRInstruction::Free { value, .. } => self.add_string_if_const(value),
            _ => {}
        }
    }

    fn add_string_if_const(&mut self, val: &MIRValue) {
        if let MIRValue::Constant { value, ty } = val {
            if matches!(ty, TejxType::String) {
                if !self.string_constants.contains(value) {
                    self.string_constants.push(value.clone());
                }
            }
        }
    }

    fn generate_function(&mut self, func: &MIRFunction) {
        let wasm_name = format!("$f_{}", func.name.replace(".", "_"));
        self.emit(&format!("  (func {} ", wasm_name));
        
        // Parameters
        for param in &func.params {
            self.emit(&format!("(param ${} i64) ", param));
        }
        
        // Return type
        self.emit("(result i64)\n");

        // Local variables
        for (var_name, _ty) in &func.variables {
            if !func.params.contains(var_name) {
                self.emit_line(&format!("(local ${} i64)", var_name));
            }
        }
        
        // State for dispatch loop
        self.emit_line("(local $state i32)");
        
        // Dispatch Loop (Handles arbitrary jumps)
        self.emit_line("(block $exit");
        self.emit_line("  (loop $loop");
        
        // Block Dispatcher
        self.emit_line("    (block $b_last");
        for (i, _) in func.blocks.iter().enumerate().rev() {
            self.emit_line(&format!("      (block $b_{}", i));
        }
        
        // Br_table for dispatch
        self.emit_line("        (local.get $state)");
        let mut table_indices = String::new();
        for (i, _) in func.blocks.iter().enumerate() {
            table_indices.push_str(&format!("{} ", i));
        }
        self.emit_line(&format!("        (br_table {} $b_last)", table_indices));
        
        // Close blocks and generate body for each basic block
        for (i, block) in func.blocks.iter().enumerate() {
            self.emit_line(&format!("      ) ;; end $b_{}", i));
            self.generate_block(block, func);
            self.emit_line("      (br $loop)");
        }
        
        self.emit_line("    ) ;; end $b_last");
        self.emit_line("  ) ;; end $loop");
        self.emit_line(") ;; end $exit");
        
        self.emit_line("i64.const 0");
        self.emit_line(")\n");
    }

    fn generate_block(&mut self, block: &BasicBlock, func: &MIRFunction) {
        for inst in &block.instructions {
            self.generate_instruction(inst, func);
        }
    }

    fn generate_instruction(&mut self, inst: &MIRInstruction, _func: &MIRFunction) {
        match inst {
            MIRInstruction::Move { dst, src, .. } => {
                self.push_boxed(src);
                self.emit_line(&format!("local.set ${}", dst));
            }
            MIRInstruction::BinaryOp { dst, left, op, right, .. } => {
                let is_float = left.get_type().is_float() || right.get_type().is_float();
                
                if is_float {
                    self.push_raw_float(left);
                    self.push_raw_float(right);
                    match op {
                        TokenType::Plus => self.emit_line("f64.add"),
                        TokenType::Minus => self.emit_line("f64.sub"),
                        TokenType::Star => self.emit_line("f64.mul"),
                        TokenType::Slash => self.emit_line("f64.div"),
                        TokenType::EqualEqual => {
                            self.emit_line("f64.eq");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::BangEqual => {
                            self.emit_line("f64.ne");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::Less => {
                            self.emit_line("f64.lt");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::LessEqual => {
                            self.emit_line("f64.le");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::Greater => {
                            self.emit_line("f64.gt");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::GreaterEqual => {
                            self.emit_line("f64.ge");
                            self.emit_line("i64.extend_i32_u");
                        }
                        _ => self.emit_line("f64.const 0.0 ;; Unknown Float Op"),
                    }
                    if !matches!(op, TokenType::EqualEqual | TokenType::BangEqual | TokenType::Less | TokenType::LessEqual | TokenType::Greater | TokenType::GreaterEqual) {
                        self.emit_line("call $rt_box_number");
                    }
                } else {
                    self.push_raw_int(left);
                    self.push_raw_int(right);
                    match op {
                        TokenType::Plus => self.emit_line("i64.add"),
                        TokenType::Minus => self.emit_line("i64.sub"),
                        TokenType::Star => self.emit_line("i64.mul"),
                        TokenType::Slash => self.emit_line("i64.div_s"),
                        TokenType::Ampersand => self.emit_line("i64.and"),
                        TokenType::Pipe => self.emit_line("i64.or"),
                        TokenType::Caret => self.emit_line("i64.xor"),
                        TokenType::EqualEqual => {
                            self.emit_line("i64.eq");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::BangEqual => {
                            self.emit_line("i64.ne");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::Less => {
                            self.emit_line("i64.lt_s");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::LessEqual => {
                            self.emit_line("i64.le_s");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::Greater => {
                            self.emit_line("i64.gt_s");
                            self.emit_line("i64.extend_i32_u");
                        }
                        TokenType::GreaterEqual => {
                            self.emit_line("i64.ge_s");
                            self.emit_line("i64.extend_i32_u");
                        }
                        _ => self.emit_line("i64.const 0 ;; Unknown Int Op"),
                    }
                    if !matches!(op, TokenType::EqualEqual | TokenType::BangEqual | TokenType::Less | TokenType::LessEqual | TokenType::Greater | TokenType::GreaterEqual) {
                        self.emit_line("call $rt_box_int");
                    }
                }
                self.emit_line(&format!("local.set ${}", dst));
            }
            MIRInstruction::Call { dst, callee, args, .. } => {
                if callee == "print" {
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.emit_line("call $print_space");
                        }
                        self.push_boxed(arg);
                        self.emit_line("call $print_raw");
                    }
                    self.emit_line("call $print_newline");
                    if dst != "" {
                        self.emit_line("i64.const 0");
                        self.emit_line(&format!("local.set ${}", dst));
                    }
                } else if callee == "rt_box_number" {
                    self.push_raw_float(&args[0]);
                    self.emit_line("call $rt_box_number");
                    if dst != "" {
                        self.emit_line(&format!("local.set ${}", dst));
                    } else {
                        self.emit_line("drop");
                    }
                } else if callee == "rt_box_int" {
                    self.push_raw_int(&args[0]);
                    self.emit_line("call $rt_box_int");
                    if dst != "" {
                        self.emit_line(&format!("local.set ${}", dst));
                    } else {
                        self.emit_line("drop");
                    }
                } else if callee == "rt_box_string" {
                    if let MIRValue::Constant { value, ty } = &args[0] {
                        if matches!(ty, TejxType::String) {
                            self.push_string_const_ptr(value);
                        } else {
                            self.push_boxed(&args[0]);
                        }
                    } else {
                        self.push_boxed(&args[0]);
                    }
                    if dst != "" {
                        self.emit_line(&format!("local.set ${}", dst));
                    } else {
                        self.emit_line("drop");
                    }
                } else if callee == "rt_box_boolean" {
                    // Constant handling for booleans
                    if let MIRValue::Constant { value, .. } = &args[0] {
                       self.emit_line(&format!("i64.const {}", if value == "true" || value == "1" { "1" } else { "0" }));
                    } else {
                       self.push_boxed(&args[0]);
                       self.emit_line("call $rt_to_boolean");
                    }
                    self.emit_line("call $rt_box_boolean");
                    if dst != "" {
                        self.emit_line(&format!("local.set ${}", dst));
                    } else {
                        self.emit_line("drop");
                    }
                } else if callee == "rt_to_number" {
                    self.push_boxed(&args[0]);
                    self.emit_line("call $rt_to_number");
                    if dst != "" {
                        // We must box it back to i64 because our locals are all i64
                        self.emit_line("call $rt_box_number");
                        self.emit_line(&format!("local.set ${}", dst));
                    } else {
                        self.emit_line("drop");
                    }
                } else {
                    for arg in args {
                        self.push_boxed(arg);
                    }
                    if callee == "print_raw" || callee == "print_space" || callee == "print_newline" ||
                       callee.starts_with("rt_") || callee.starts_with("m_") || callee == "a_new" || callee == "Array_push" {
                        self.emit_line(&format!("call ${}", callee));
                    } else {
                        self.emit_line(&format!("call $f_{}", callee.replace(".", "_")));
                    }
                    
                    if dst != "" {
                        self.emit_line(&format!("local.set ${}", dst));
                    } else {
                        self.emit_line("drop");
                    }
                }
            }
            MIRInstruction::IndirectCall { dst, callee, args, .. } => {
                for arg in args {
                    self.push_boxed(arg);
                }
                self.push_boxed(callee);
                // In NovaJs/Wasm, we treat the i64 value as an index into the WASM table
                self.emit_line("i32.wrap_i64");
                
                let mut params = String::new();
                for _ in args {
                    params.push_str(" i64");
                }
                self.emit_line(&format!("call_indirect (func (param {}) (result i64))", params));
                
                if dst != "" {
                    self.emit_line(&format!("local.set ${}", dst));
                } else {
                    self.emit_line("drop");
                }
            }
            MIRInstruction::Return { value, .. } => {
                if let Some(val) = value {
                    self.push_boxed(val);
                } else {
                    self.emit_line("i64.const 0");
                }
                self.emit_line("return");
            }
            MIRInstruction::Jump { target, .. } => {
                self.emit_line(&format!("i32.const {}", target));
                self.emit_line("local.set $state");
            }
            MIRInstruction::Branch { condition, true_target, false_target, .. } => {
                self.push_boxed(condition);
                // rt_to_boolean would be safer here, but for now we assume it's already a truthy number as expected by NovaJs
                self.emit_line("call $rt_to_boolean"); 
                self.emit_line("i64.const 0");
                self.emit_line("i64.ne");
                self.emit_line("if");
                self.emit_line(&format!("  i32.const {}", true_target));
                self.emit_line("  local.set $state");
                self.emit_line("else");
                self.emit_line(&format!("  i32.const {}", false_target));
                self.emit_line("  local.set $state");
                self.emit_line("end");
            }
            MIRInstruction::ObjectLiteral { dst, entries, .. } => {
                self.emit_line("call $m_new");
                self.emit_line(&format!("local.set ${}", dst));
                for (name, val) in entries {
                    self.emit_line(&format!("local.get ${}", dst));
                    self.push_string_const_ptr(name);
                    self.push_boxed(val);
                    self.emit_line("call $m_set");
                    self.emit_line("drop");
                }
            }
            MIRInstruction::ArrayLiteral { dst, elements, .. } => {
                self.emit_line("call $a_new");
                self.emit_line(&format!("local.set ${}", dst));
                for val in elements {
                    self.emit_line(&format!("local.get ${}", dst));
                    self.push_boxed(val);
                    self.emit_line("call $Array_push");
                    self.emit_line("drop");
                }
            }
            MIRInstruction::LoadMember { dst, obj, member, .. } => {
                self.push_boxed(obj);
                self.push_string_const_ptr(member);
                self.emit_line("call $m_get");
                self.emit_line(&format!("local.set ${}", dst));
            }
            MIRInstruction::StoreMember { obj, member, src, .. } => {
                self.push_boxed(obj);
                self.push_string_const_ptr(member);
                self.push_boxed(src);
                self.emit_line("call $m_set");
                self.emit_line("drop");
            }
            MIRInstruction::LoadIndex { dst, obj, index, .. } => {
                self.push_boxed(obj);
                // In NovaJs, indexes are often also numbers or strings. rt_array_get_fast expects i64 index.
                // We assume indexing is numeric for now.
                self.push_boxed(index);
                self.emit_line("call $rt_to_number");
                self.emit_line("i64.trunc_f64_s");
                self.emit_line("call $rt_array_get_fast");
                self.emit_line(&format!("local.set ${}", dst));
            }
            MIRInstruction::StoreIndex { obj, index, src, .. } => {
                self.push_boxed(obj);
                self.push_boxed(index);
                self.emit_line("call $rt_to_number");
                self.emit_line("i64.trunc_f64_s");
                self.push_boxed(src);
                self.emit_line("call $rt_array_set_fast");
                self.emit_line("drop");
            }
            MIRInstruction::Cast { dst, src, ty, .. } => {
                self.push_boxed(src);
                if matches!(ty, TejxType::Any) {
                    // Do nothing, already a boxed/tagged value usually
                } else if ty.is_numeric() {
                    // In Wasm, we might need to unbox if src is Any
                    if matches!(src.get_type(), TejxType::Any) {
                        self.emit_line("call $rt_to_number");
                        self.emit_line("i64.trunc_f64_s");
                    }
                }
                self.emit_line(&format!("local.set ${}", dst));
            }
            MIRInstruction::Free { value, .. } => {
                self.push_boxed(value);
                self.emit_line("call $rt_free");
            }
            _ => {
                self.emit_line(&format!(";; Unsupported instruction: {:?}", inst));
            }
        }
    }

    fn push_boxed(&mut self, val: &MIRValue) {
        match val {
            MIRValue::Variable { name, .. } => {
                self.emit_line(&format!("local.get ${}", name));
            }
            MIRValue::Constant { value, ty } => {
                if ty.is_float() {
                    self.emit_line(&format!("f64.const {}", value));
                    self.emit_line("call $rt_box_number");
                } else if ty.is_numeric() {
                    self.emit_line(&format!("i64.const {}", value));
                    self.emit_line("call $rt_box_int");
                } else if matches!(ty, TejxType::Bool) {
                    self.emit_line(&format!("i64.const {}", if value == "true" || value == "1" { "1" } else { "0" }));
                    self.emit_line("call $rt_box_boolean");
                } else if matches!(ty, TejxType::String) {
                    self.push_string_const_ptr(value);
                } else {
                    self.emit_line("i64.const 0");
                }
            }
        }
    }

    fn push_raw_int(&mut self, val: &MIRValue) {
        match val {
            MIRValue::Variable { name, .. } => {
                self.emit_line(&format!("local.get ${}", name));
                self.emit_line("call $rt_to_number");
                self.emit_line("i64.trunc_f64_s");
            }
            MIRValue::Constant { value, ty } => {
                if ty.is_numeric() {
                    if ty.is_float() {
                         self.emit_line(&format!("f64.const {}", value));
                         self.emit_line("i64.trunc_f64_s");
                    } else {
                         self.emit_line(&format!("i64.const {}", value));
                    }
                } else {
                    self.emit_line("i64.const 0");
                }
            }
        }
    }

    fn push_raw_float(&mut self, val: &MIRValue) {
        match val {
            MIRValue::Variable { name, .. } => {
                self.emit_line(&format!("local.get ${}", name));
                self.emit_line("call $rt_to_number");
            }
            MIRValue::Constant { value, ty } => {
                if ty.is_float() {
                    self.emit_line(&format!("f64.const {}", value));
                } else {
                    self.emit_line(&format!("i64.const {}", value));
                    self.emit_line("f64.convert_i64_s");
                }
            }
        }
    }



    fn push_string_const_ptr(&mut self, value: &str) {
        let idx = self.string_constants.iter().position(|r| r == value).unwrap_or(0);
        let mut offset = 1024;
        for i in 0..idx {
            offset += self.string_constants[i].len() + 1;
        }
        self.emit_line(&format!("i64.const {}", offset));
        self.emit_line("call $rt_box_string");
    }
}
