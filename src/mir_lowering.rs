/// HIR → MIR Lowering pass, mirroring C++ MIRLowering.cpp
/// Converts high-level typed IR into basic blocks with low-level instructions.
use crate::hir::*;
use crate::intrinsics::*;
use crate::mir::*;
use crate::token::TokenType;
use crate::types::TejxType;
use std::collections::HashMap;

#[derive(Clone)]
struct LoopContext {
    continue_target: usize,
    break_target: usize,
}

pub struct MIRLowering {
    current_function: MIRFunction,
    current_block: usize, // index into current_function.blocks
    temp_counter: usize,
    block_counter: usize,
    loop_stack: Vec<LoopContext>,
    exception_handler_stack: Vec<usize>,
    expected_ty: Option<TejxType>,
    signatures: HashMap<String, Vec<TejxType>>,
    current_return_type: TejxType,
    current_line: usize,
    scopes: Vec<HashMap<String, String>>, // Stack of scopes: original_name -> unique_mir_name
    var_counter: usize,
    class_fields: HashMap<String, Vec<(String, TejxType)>>,
    async_state_counter: usize,
    pub async_continuations: Vec<(usize, usize)>, // (state, target_block_idx)
    pub current_async_params: Option<Vec<(String, TejxType)>>,
    pub current_promise_id: Option<String>,
    pub async_locals: Vec<String>,
}

impl MIRLowering {
    pub fn new(
        signatures: HashMap<String, Vec<TejxType>>,
        class_fields: HashMap<String, Vec<(String, TejxType)>>,
    ) -> Self {
        Self {
            current_function: MIRFunction::new("".to_string(), TejxType::Void),
            current_block: 0,
            temp_counter: 0,
            block_counter: 0,
            loop_stack: Vec::new(),
            exception_handler_stack: Vec::new(),
            expected_ty: None,
            signatures,
            current_return_type: TejxType::Void,
            current_line: 0,
            scopes: vec![HashMap::new()], // Global/Function scope
            var_counter: 0,
            class_fields,
            async_state_counter: 1,
            async_continuations: Vec::new(),
            current_async_params: None,
            current_promise_id: None,
            async_locals: Vec::new(),
        }
    }

    fn new_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) -> Vec<String> {
        let scope = self.scopes.pop().expect("Scope stack underflow");
        // Return variables declared in this scope safely (values of the map)
        scope.values().cloned().collect()
    }

    fn declare_variable(&mut self, name: &str) -> String {
        if name.starts_with("g_") {
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(name.to_string(), name.to_string());
            }
            return name.to_string();
        }
        let unique_name = if name.contains('$') || self.scopes.len() == 1 {
            // Global/Top-level or already mangled: preserve name
            name.to_string()
        } else {
            format!("{}_{}", name, self.var_counter)
        };
        if !name.contains('$') {
            self.var_counter += 1;
        }

        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), unique_name.clone());
        }
        unique_name
    }

    fn resolve_variable(&self, name: &str) -> String {
        // Search from inner to outer
        for scope in self.scopes.iter().rev() {
            if let Some(unique) = scope.get(name) {
                return unique.clone();
            }
        }
        // If not found, assume global or parameter (if parameters aren't in scope yet)
        name.to_string()
    }

    pub fn lower(&mut self, hir_func: &HIRStatement) -> MIRFunction {
        // Extract function info
        let info = match hir_func {
            HIRStatement::Function {
                name,
                params,
                body,
                _return_type,
                is_extern,
                async_params,
                ..
            } => (
                name.clone(),
                params.clone(),
                body.as_ref(),
                _return_type.clone(),
                *is_extern,
                async_params.clone(),
            ),
            _ => (
                crate::intrinsics::TEJX_MAIN.to_string(),
                vec![],
                hir_func,
                TejxType::Void,
                false,
                None,
            ),
        };

        let (name, params, body, ret_ty, is_extern, async_params) = info;
        self.lower_function(name, params, ret_ty, body, is_extern, async_params)
    }

    pub fn lower_function(
        &mut self,
        name: String,
        params: Vec<(String, TejxType)>,
        return_type: TejxType,
        body: &HIRStatement,
        is_extern: bool,
        async_params: Option<Vec<(String, TejxType)>>,
    ) -> MIRFunction {
        self.current_function = MIRFunction::new(name.clone(), return_type.clone());
        self.current_return_type = return_type.clone();
        self.temp_counter = 0;
        self.block_counter = 0;
        self.var_counter = 0;
        self.loop_stack.clear();
        self.exception_handler_stack.clear();
        self.scopes.clear();
        self.scopes.push(HashMap::new());

        // Reset async state
        self.async_state_counter = 1;
        self.async_continuations.clear();
        self.current_async_params = None;
        self.current_promise_id = None;
        self.async_locals.clear();

        // Reset async state
        self.async_state_counter = 1;
        self.async_continuations.clear();
        self.current_async_params = None;
        self.current_promise_id = None;
        self.async_locals.clear();

        self.current_function.params = params.iter().map(|(n, _)| n.clone()).collect();
        self.current_function.is_extern = is_extern;

        for (pname, pty) in params.iter() {
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(pname.clone(), pname.clone());
            }
            self.current_function
                .variables
                .insert(pname.clone(), pty.clone());
        }

        // --- NEW: Pre-define original async parameters for machines ---
        // This ensures they are available in variable_map for collect_async_locals
        if let Some(aps) = &async_params {
            for (pname, pty) in aps {
                self.declare_variable(pname);
                // Also ensure the type is recorded
                self.current_function
                    .variables
                    .insert(pname.clone(), pty.clone());
            }
        }

        let is_async_worker =
            name.ends_with("_worker") && async_params.is_some() && params.len() == 1;
        if is_async_worker {
            if let Some(ref ap) = async_params {
                for (pname, pty) in ap.iter() {
                    self.current_function
                        .variables
                        .insert(pname.clone(), pty.clone());
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(pname.clone(), pname.clone());
                    }
                }
            }
        }

        self.pre_collect_variables(body);

        if is_async_worker {
            // Find mangled promise_id_local
            for var_name in self.current_function.variables.keys() {
                if var_name.starts_with("promise_id_local") {
                    self.current_promise_id = Some(var_name.clone());
                    break;
                }
            }
            self.current_async_params = async_params.clone();
            self.async_locals.clear();
            let aps = self.current_async_params.clone();
            self.collect_async_locals(&aps, &params);
        }

        let switch_block = if is_async_worker {
            Some(self.new_block("async_switch"))
        } else {
            None
        };

        if let Some(sb) = switch_block {
            self.current_function.entry_block = sb;
            let entry = self.new_block("entry");
            self.async_continuations.push((0, entry));
            self.current_block = entry;
        } else {
            let entry = self.new_block("entry");
            self.current_function.entry_block = entry;
            self.current_block = entry;
        }

        self.lower_statement(body);

        if is_async_worker {
            let aps = self.current_async_params.clone();
            self.collect_async_locals(&aps, &params);
        }

        // Ensure current block is terminated
        let cb = self.current_block;
        if !self.current_function.blocks[cb].is_terminated() {
            if is_async_worker {
                let ctx_val = MIRValue::Variable {
                    name: "ctx".to_string(),
                    ty: TejxType::Class("Int64[]".to_string(), vec![]),
                };
                let promise_id =
                    self.new_async_temp(TejxType::Class("Promise".to_string(), vec![]));
                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: promise_id.clone(),
                    callee: "rt_array_get_fast".to_string(),
                    args: vec![
                        ctx_val.clone(),
                        MIRValue::Constant {
                            value: "0".to_string(),
                            ty: TejxType::Int32,
                        },
                    ],
                });

                let unused = self.new_async_temp(TejxType::Void);
                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: unused.clone(),
                    callee: "rt_promise_resolve".to_string(),
                    args: vec![
                        MIRValue::Variable {
                            name: promise_id,
                            ty: TejxType::Class("Promise".to_string(), vec![]),
                        },
                        MIRValue::Constant {
                            value: "0".to_string(),
                            ty: TejxType::Int32,
                        },
                    ],
                });

                let unused2 = self.new_async_temp(TejxType::Void);
                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: unused2,
                    callee: "tejx_dec_async_ops".to_string(),
                    args: vec![],
                });

                self.emit(MIRInstruction::Return {
                    line: 0,
                    value: None,
                });
            } else {
                self.emit(MIRInstruction::Return {
                    line: 0,
                    value: None,
                });
            }
        }

        if is_async_worker {
            // Restore async state from ctx
            let ctx_val = MIRValue::Variable {
                name: "ctx".to_string(),
                ty: TejxType::Class("Int64[]".to_string(), vec![]),
            };

            let mut restoration_instrs = Vec::new();

            // 1. Initial State: Zero-initialize all temporaries and locals to be safe
            for (name, ty) in self.current_function.variables.iter() {
                if name == "ctx" || name.starts_with("promise_id_local") {
                    continue;
                }
                restoration_instrs.push(MIRInstruction::Move {
                    line: 0,
                    dst: name.clone(),
                    src: MIRValue::Constant {
                        value: "0".to_string(),
                        ty: ty.clone(),
                    },
                });
            }

            // 2. Restore Params & Locals from ctx
            // They are stored at ctx[2...] as unified in collect_async_locals
            for (i, name) in self.async_locals.iter().enumerate() {
                let ix = 2 + i;
                restoration_instrs.push(MIRInstruction::Call {
                    line: 0,
                    dst: name.clone(),
                    callee: "rt_array_get_fast".to_string(),
                    args: vec![
                        ctx_val.clone(),
                        MIRValue::Constant {
                            value: ix.to_string(),
                            ty: TejxType::Int32,
                        },
                    ],
                });
            }

            // Restore promise_id_local at index 0
            if let Some(ref p_var) = self.current_promise_id {
                restoration_instrs.push(MIRInstruction::Call {
                    line: 0,
                    dst: p_var.clone(),
                    callee: "rt_array_get_fast".to_string(),
                    args: vec![
                        ctx_val.clone(),
                        MIRValue::Constant {
                            value: "0".to_string(),
                            ty: TejxType::Int32,
                        },
                    ],
                });
            }

            // Extract state and build switch
            let state_temp = self.new_async_temp(TejxType::Int32);
            restoration_instrs.push(MIRInstruction::Call {
                line: 0,
                dst: state_temp.clone(),
                callee: "rt_array_get_fast".to_string(),
                args: vec![
                    ctx_val.clone(),
                    MIRValue::Constant {
                        value: "1".to_string(),
                        ty: TejxType::Int32,
                    },
                ],
            });

            let state_val = MIRValue::Variable {
                name: state_temp,
                ty: TejxType::Int32,
            };

            let entry_idx = self.current_function.entry_block;

            // 3. Build switch chain using multiple blocks
            let conts = self.async_continuations.clone();
            let mut prev_block = entry_idx;

            for i in 0..conts.len() {
                let (state_id, target_block) = conts[i];
                self.current_block = prev_block;

                if i == conts.len() - 1 {
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: target_block,
                    });
                } else {
                    let next_check = self.new_block("async_check_next");
                    let cmp_res = format!("_cmp_state_{}", i);
                    self.current_function
                        .variables
                        .insert(cmp_res.clone(), TejxType::Bool);

                    self.emit(MIRInstruction::BinaryOp {
                        line: 0,
                        dst: cmp_res.clone(),
                        left: state_val.clone(),
                        op: TokenType::EqualEqual,
                        right: MIRValue::Constant {
                            value: state_id.to_string(),
                            ty: TejxType::Int32,
                        },
                        op_width: TejxType::Int32,
                    });

                    self.emit(MIRInstruction::Branch {
                        line: 0,
                        condition: MIRValue::Variable {
                            name: cmp_res,
                            ty: TejxType::Bool,
                        },
                        true_target: target_block,
                        false_target: next_check,
                    });

                    prev_block = next_check;
                }
            }

            // Prepend only restoration instructions (excluding switch) to the entry block
            self.current_function.blocks[entry_idx]
                .instructions
                .splice(0..0, restoration_instrs);
        }

        self.current_function.clone()
    }

    fn new_block(&mut self, prefix: &str) -> usize {
        let name = format!("{}_{}", prefix, self.block_counter);
        self.block_counter += 1;
        let mut bb = BasicBlock::new(name);
        bb.exception_handler = self.exception_handler_stack.last().cloned();
        self.current_function.blocks.push(bb);
        self.current_function.blocks.len() - 1
    }

    fn pre_collect_variables(&mut self, stmt: &HIRStatement) {
        match stmt {
            HIRStatement::Block { statements, .. } => {
                for s in statements {
                    self.pre_collect_variables(s);
                }
            }
            HIRStatement::VarDecl { name, ty, .. } => {
                self.current_function
                    .variables
                    .insert(name.clone(), ty.clone());
            }
            HIRStatement::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.pre_collect_variables_expr(condition);
                self.pre_collect_variables(then_branch);
                if let Some(eb) = else_branch {
                    self.pre_collect_variables(eb);
                }
            }
            HIRStatement::Loop { body, .. } => {
                self.pre_collect_variables(body);
            }
            HIRStatement::Try {
                try_block,
                catch_var,
                catch_block,
                finally_block,
                ..
            } => {
                if let Some(var) = catch_var {
                    self.current_function
                        .variables
                        .insert(var.clone(), TejxType::Class("Error".to_string(), vec![]));
                }
                self.pre_collect_variables(try_block);
                self.pre_collect_variables(catch_block);
                if let Some(fb) = finally_block {
                    self.pre_collect_variables(fb);
                }
            }
            HIRStatement::ExpressionStmt { expr, .. } => {
                self.pre_collect_variables_expr(expr);
            }
            HIRStatement::Sequence { statements, .. } => {
                for s in statements {
                    self.pre_collect_variables(s);
                }
            }
            HIRStatement::Return { value, .. } => {
                if let Some(v) = value {
                    self.pre_collect_variables_expr(v);
                }
            }
            HIRStatement::Throw { value, .. } => {
                self.pre_collect_variables_expr(value);
            }
            _ => {}
        }
    }

    fn pre_collect_variables_expr(&mut self, expr: &HIRExpression) {
        match expr {
            HIRExpression::BinaryExpr { left, right, .. } => {
                self.pre_collect_variables_expr(left);
                self.pre_collect_variables_expr(right);
            }
            HIRExpression::Call { args, .. } => {
                for arg in args {
                    self.pre_collect_variables_expr(arg);
                }
            }
            HIRExpression::IndirectCall { callee, args, .. } => {
                self.pre_collect_variables_expr(callee);
                for arg in args {
                    self.pre_collect_variables_expr(arg);
                }
            }
            HIRExpression::MemberAccess { target, .. } => {
                self.pre_collect_variables_expr(target);
            }
            HIRExpression::IndexAccess { target, index, .. } => {
                self.pre_collect_variables_expr(target);
                self.pre_collect_variables_expr(index);
            }
            HIRExpression::Assignment { target, value, .. } => {
                self.pre_collect_variables_expr(target);
                self.pre_collect_variables_expr(value);
            }
            HIRExpression::Await { expr, .. } => {
                self.pre_collect_variables_expr(expr);
            }
            HIRExpression::ObjectLiteral { entries, .. } => {
                for (_, v) in entries {
                    self.pre_collect_variables_expr(v);
                }
            }
            HIRExpression::ArrayLiteral { elements, .. } => {
                for e in elements {
                    self.pre_collect_variables_expr(e);
                }
            }
            HIRExpression::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.pre_collect_variables_expr(condition);
                self.pre_collect_variables_expr(then_branch);
                self.pre_collect_variables_expr(else_branch);
            }
            HIRExpression::Sequence { expressions, .. } => {
                for e in expressions {
                    self.pre_collect_variables_expr(e);
                }
            }
            HIRExpression::SomeExpr { value, .. } => {
                self.pre_collect_variables_expr(value);
            }
            _ => {}
        }
    }
    fn new_temp(&mut self, ty: TejxType) -> String {
        let name = format!("_t{}", self.temp_counter);
        self.temp_counter += 1;
        self.current_function
            .variables
            .insert(name.clone(), ty.clone());
        name
    }

    fn new_async_temp(&mut self, ty: TejxType) -> String {
        let name = format!("_atmp{}", self.temp_counter);
        self.temp_counter += 1;
        self.current_function
            .variables
            .insert(name.clone(), ty.clone());
        name
    }

    fn collect_async_locals(
        &mut self,
        async_params: &Option<Vec<(String, TejxType)>>,
        _params: &[(String, TejxType)],
    ) {
        // Collect existing names into a loopup set for additive behavior
        let mut added: std::collections::HashSet<String> =
            self.async_locals.iter().cloned().collect();

        // 1. Original parameters MUST come first (after promise and state)
        // because lowering.rs puts them there: [promise, state, params..., locals...]
        if let Some(aps) = async_params {
            for (pname, _pty) in aps {
                let mangled = self.resolve_variable(pname);
                if !mangled.is_empty() {
                    if !added.contains(&mangled) {
                        self.async_locals.push(mangled.clone());
                        added.insert(mangled.clone());
                    }
                }
            }
        }

        // 2. All other variables, sorted for determinism
        let mut keys: Vec<_> = self.current_function.variables.keys().cloned().collect();
        keys.sort();
        for name in keys {
            if name == "ctx"
                || name.starts_with("__ctx_")
                || name.starts_with("__p_")
                || name.starts_with("promise_id_local")
                || name.starts_with("_atmp")
                || name.starts_with("_restore_local_")
                || name.starts_with("_state")
                || name.starts_with("_cmp_state_")
            {
                continue;
            }
            if !added.contains(&name) {
                self.async_locals.push(name.clone());
                added.insert(name.clone());
            }
        }
    }

    fn emit(&mut self, mut inst: MIRInstruction) {
        inst.set_line(self.current_line);
        let cb = self.current_block;
        if cb < self.current_function.blocks.len()
            && self.current_function.blocks[cb].is_terminated()
        {
            return;
        }
        self.current_function.blocks[cb].add_instruction(inst);
    }

    fn auto_box(&mut self, val: MIRValue, target_ty: &TejxType) -> MIRValue {
        let src_ty = val.get_type();
        // Explicit boxing to any is no longer supported nor performed.

        // Implicit Casting for Primitives or downcasting from any/Object
        if *src_ty != *target_ty {
            let is_primitive_cast = src_ty.is_numeric() && target_ty.is_numeric();
            let is_downcast = (src_ty == &TejxType::Int64)
                && (target_ty.is_numeric()
                    || matches!(target_ty, TejxType::Bool | TejxType::String));

            if is_primitive_cast || is_downcast {
                let cast_temp = self.new_temp(target_ty.clone());
                self.emit(MIRInstruction::Cast {
                    line: 0,
                    dst: cast_temp.clone(),
                    src: val.clone(),
                    ty: target_ty.clone(),
                });
                return MIRValue::Variable {
                    name: cast_temp,
                    ty: target_ty.clone(),
                };
            }
        }

        // Slice Coercion: T[] -> Slice<T>, T[N] -> Slice<T>
        if let TejxType::Slice(_inner_target) = target_ty {
            if src_ty.is_array() {
                // Lowering a fat pointer {ptr, len}
                // We'll need a MIR instruction for this or a special call
                // For now, let's use a dummy temporary and we'll refine the instruction set if needed
                let slice_temp = self.new_temp(target_ty.clone());
                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: slice_temp.clone(),
                    callee: "rt_to_slice".to_string(), // Runtime helper to build fat pointer
                    args: vec![val.clone()],
                });
                return MIRValue::Variable {
                    name: slice_temp,
                    ty: target_ty.clone(),
                };
            }
        }

        val
    }

    fn lower_statement(&mut self, stmt: &HIRStatement) {
        self.current_line = stmt.get_line();
        match stmt {
            HIRStatement::Block { statements, .. } => {
                self.new_scope();
                for s in statements {
                    self.lower_statement(s);
                }
                // End of block: Scope management
                let _vars_to_drop = self.pop_scope();
                // Do NOT inject scope-based Free instructions here!
            }
            HIRStatement::Sequence { statements, .. } => {
                // Sequence is a block without a scope.
                for s in statements {
                    self.lower_statement(s);
                }
            }
            HIRStatement::VarDecl {
                name,
                initializer,
                ty,
                ..
            } => {
                let unique_name = self.declare_variable(name);
                self.current_function
                    .variables
                    .insert(unique_name.clone(), ty.clone());

                if let Some(init) = initializer {
                    self.expected_ty = Some(ty.clone());
                    let mut src = self.lower_expression(init);
                    self.expected_ty = None;

                    src = self.auto_box(src, ty);

                    self.emit(MIRInstruction::Move {
                        line: 0,
                        dst: unique_name,
                        src,
                    });
                }
            }
            HIRStatement::Loop {
                condition,
                body,
                increment,
                ..
            } => {
                let loop_header = self.new_block("loop_header");
                let loop_body = self.new_block("loop_body");
                let loop_latch = if increment.is_some() {
                    self.new_block("loop_latch") // For 'continue' in for-loop (increment)
                } else {
                    loop_header // For 'continue' in while-loop (jump to condition)
                };
                let loop_exit = self.new_block("loop_exit");

                self.loop_stack.push(LoopContext {
                    continue_target: loop_latch,
                    break_target: loop_exit,
                });

                self.emit(MIRInstruction::Jump {
                    line: 0,
                    target: loop_header,
                });

                // Header: check condition
                self.current_block = loop_header;
                let cond_val = self.lower_expression(condition);
                self.emit(MIRInstruction::Branch {
                    line: 0,
                    condition: cond_val,
                    true_target: loop_body,
                    false_target: loop_exit,
                });

                // Body
                self.current_block = loop_body;
                self.lower_statement(body);
                // Fallthrough to latch (or header if no latch)
                let cb = self.current_block;
                if !self.current_function.blocks[cb].is_terminated() {
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: loop_latch,
                    });
                }

                // Latch (Increment)
                if let Some(inc) = increment {
                    self.current_block = loop_latch;
                    self.lower_statement(inc);
                    let cb = self.current_block;
                    if !self.current_function.blocks[cb].is_terminated() {
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: loop_header,
                        });
                    }
                }

                self.loop_stack.pop();
                self.current_block = loop_exit;
            }
            HIRStatement::Break { .. } => {
                if let Some(ctx) = self.loop_stack.last() {
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: ctx.break_target,
                    });
                }
            }
            HIRStatement::Continue { .. } => {
                if let Some(ctx) = self.loop_stack.last() {
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: ctx.continue_target,
                    });
                }
            }
            HIRStatement::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let then_block = self.new_block("if_then");
                let else_block = self.new_block("if_else");
                let merge_block = self.new_block("if_merge");

                let cond_val = self.lower_expression(condition);
                self.emit(MIRInstruction::Branch {
                    line: 0,
                    condition: cond_val,
                    true_target: then_block,
                    false_target: else_block,
                });

                // Then
                self.current_block = then_block;
                self.lower_statement(then_branch);
                let cb = self.current_block;
                if !self.current_function.blocks[cb].is_terminated() {
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: merge_block,
                    });
                }

                // Else
                self.current_block = else_block;
                if let Some(else_stmt) = else_branch {
                    self.lower_statement(else_stmt);
                }
                let cb = self.current_block;
                if !self.current_function.blocks[cb].is_terminated() {
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: merge_block,
                    });
                }

                self.current_block = merge_block;
            }
            HIRStatement::Return { value, .. } => {
                let mut val = value.as_ref().map(|e| self.lower_expression(e));

                if let Some(ret_val) = val {
                    val = Some(self.auto_box(ret_val, &self.current_return_type.clone()));
                }

                let is_async_worker = self.current_function.name.ends_with("_worker");
                if is_async_worker {
                    let ctx_val = MIRValue::Variable {
                        name: "ctx".to_string(),
                        ty: TejxType::Class("Int64[]".to_string(), vec![]),
                    };
                    let promise_id = self.new_async_temp(TejxType::Int64);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: promise_id.clone(),
                        callee: "rt_array_get_fast".to_string(),
                        args: vec![
                            ctx_val.clone(),
                            MIRValue::Constant {
                                value: "0".to_string(),
                                ty: TejxType::Int32,
                            },
                        ],
                    });

                    let resolve_val = val.clone().unwrap_or(MIRValue::Constant {
                        value: "0".to_string(),
                        ty: TejxType::Int32,
                    });

                    let unused = self.new_async_temp(TejxType::Void);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: unused.clone(),
                        callee: "rt_promise_resolve".to_string(),
                        args: vec![
                            MIRValue::Variable {
                                name: promise_id,
                                ty: TejxType::Int64,
                            },
                            resolve_val,
                        ],
                    });

                    let unused2 = self.new_async_temp(TejxType::Void);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: unused2,
                        callee: "tejx_dec_async_ops".to_string(),
                        args: vec![],
                    });

                    self.emit(MIRInstruction::Return {
                        line: 0,
                        value: None,
                    });
                } else {
                    self.emit(MIRInstruction::Return {
                        line: 0,
                        value: val,
                    });
                }
            }
            HIRStatement::ExpressionStmt { expr, .. } => {
                self.lower_expression(expr);
            }
            HIRStatement::Function { body, .. } => {
                self.lower_statement(body);
            }
            HIRStatement::Switch {
                condition, cases, ..
            } => {
                let switch_val = self.lower_expression(condition);
                let switch_exit = self.new_block("switch_exit");

                // Push switch exit as break target (continue target is none/invalid? or enclosing loop?)
                // If we use loop_stack, we need to handle "continue" carefully.
                // Continue in switch refers to enclosing loop. Break refers to switch.
                // So we need to copy previous continue target.
                let prev_continue = self
                    .loop_stack
                    .last()
                    .map(|c| c.continue_target)
                    .unwrap_or(switch_exit);
                self.loop_stack.push(LoopContext {
                    continue_target: prev_continue,
                    break_target: switch_exit,
                });

                // Chain of comparisons
                // case 1: check -> body -> exit
                // case 2: check -> ...
                // default: body -> exit

                let mut next_check_block = self.new_block("case_check");
                self.emit(MIRInstruction::Jump {
                    line: 0,
                    target: next_check_block,
                });

                for case in cases {
                    self.current_block = next_check_block;

                    if let Some(val) = &case.value {
                        let case_val = self.lower_expression(val);
                        let body_block = self.new_block("case_body");
                        let next_c = self.new_block("next_case");

                        // Compare
                        // Compare: switch_val == case_val
                        let cmp_res = self.new_temp(TejxType::Bool);
                        self.emit(MIRInstruction::BinaryOp {
                            line: 0,
                            dst: cmp_res.clone(),
                            left: switch_val.clone(),
                            op: TokenType::EqualEqual,
                            right: case_val.clone(),
                            op_width: switch_val.get_type().clone(),
                        });
                        self.emit(MIRInstruction::Branch {
                            line: 0,
                            condition: MIRValue::Variable {
                                name: cmp_res,
                                ty: TejxType::Bool,
                            },
                            true_target: body_block,
                            false_target: next_c,
                        });

                        // Body
                        self.current_block = body_block;
                        self.lower_statement(&case.body);
                        let cb = self.current_block;
                        if !self.current_function.blocks[cb].is_terminated() {
                            self.emit(MIRInstruction::Jump {
                                line: 0,
                                target: switch_exit,
                            });
                        }

                        next_check_block = next_c;
                    } else {
                        // Default case - unconditional
                        let default_block = self.new_block("default_case");
                        // We are at next_check_block (which was previous Loop's false_target).
                        // wait, logic above sets current_block to next_check_block at start of loop.
                        // So here we are at 'next_check_block'.
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: default_block,
                        });

                        self.current_block = default_block;
                        self.lower_statement(&case.body);
                        let cb = self.current_block;
                        if !self.current_function.blocks[cb].is_terminated() {
                            self.emit(MIRInstruction::Jump {
                                line: 0,
                                target: switch_exit,
                            });
                        }

                        // Default should be last in HIR usually?
                        // If not, we just continue emitting checks?
                        // But default captures everything.
                        // Let's assume it's last or acts as catch-all.
                        // We update next_check_block to a dead block or exit?
                        // Actually if default is not last, unreachable code follows.
                        next_check_block = self.new_block("after_default"); // Unreachable
                    }
                }

                // If fall through all cases (no default or default didn't match?), jump to exit
                self.current_block = next_check_block;
                self.emit(MIRInstruction::Jump {
                    line: 0,
                    target: switch_exit,
                });

                self.loop_stack.pop();
                self.current_block = switch_exit;
            }
            HIRStatement::Try {
                try_block,
                catch_var,
                catch_block,
                finally_block,
                ..
            } => {
                let exit_block_idx = self.new_block("try_exit");

                // Variables to track unwinding state across finally block
                let is_unwinding_var = self.new_temp(TejxType::Bool);
                let saved_ex_var = self.new_temp(TejxType::Int64);

                let finally_handler_idx = if finally_block.is_some() {
                    Some(self.new_block("finally_unwind"))
                } else {
                    None
                };
                let finally_body_idx = if finally_block.is_some() {
                    Some(self.new_block("finally_body"))
                } else {
                    None
                };

                // 2. Setup catch block entry with finally handler if needed
                if let Some(fh_idx) = finally_handler_idx {
                    self.exception_handler_stack.push(fh_idx);
                }
                let catch_block_idx = self.new_block("catch");
                if finally_handler_idx.is_some() {
                    self.exception_handler_stack.pop();
                }

                // 1. Lower Try Block
                // Handler: Catch
                self.exception_handler_stack.push(catch_block_idx);

                let try_start_idx = self.new_block("try_start");
                // self.emit(MIRInstruction::Jump { line: 0,  target: try_start_idx  });
                // Replaced by TrySetup which branches to try or catch
                self.emit(MIRInstruction::TrySetup {
                    line: 0,
                    try_target: try_start_idx,
                    _catch_target: catch_block_idx,
                });

                self.current_block = try_start_idx;

                self.lower_statement(try_block);

                // Try success path
                let cb = self.current_block;
                if !self.current_function.blocks[cb].is_terminated() {
                    // Successful execution of try block: Pop the handler
                    self.emit(MIRInstruction::PopHandler { line: 0 });

                    if let Some(fb_idx) = finally_body_idx {
                        // Normal execution: flow to finally
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: is_unwinding_var.clone(),
                            src: MIRValue::Constant {
                                value: "false".to_string(),
                                ty: TejxType::Bool,
                            },
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: fb_idx,
                        });
                    } else {
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: exit_block_idx,
                        });
                    }
                }
                self.exception_handler_stack.pop();

                // 2. Lower Catch Block
                // Handler: Finally Unwind (if exists)
                if let Some(fh_idx) = finally_handler_idx {
                    self.exception_handler_stack.push(fh_idx);
                }

                self.current_block = catch_block_idx;
                if let Some(var) = catch_var {
                    // Extract exception into variable
                    let temp = self.new_temp(TejxType::Int64);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: temp.clone(),
                        callee: TEJX_GET_EXCEPTION.to_string(),
                        args: vec![],
                    });
                    self.emit(MIRInstruction::Move {
                        line: 0,
                        dst: var.clone(),
                        src: MIRValue::Variable {
                            name: temp,
                            ty: TejxType::Int64,
                        },
                    });
                }
                self.lower_statement(catch_block);

                // Catch success path
                let cb = self.current_block;
                if !self.current_function.blocks[cb].is_terminated() {
                    if finally_handler_idx.is_some() {
                        self.emit(MIRInstruction::PopHandler { line: 0 });
                    }
                    if let Some(fb_idx) = finally_body_idx {
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: is_unwinding_var.clone(),
                            src: MIRValue::Constant {
                                value: "false".to_string(),
                                ty: TejxType::Bool,
                            },
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: fb_idx,
                        });
                    } else {
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: exit_block_idx,
                        });
                    }
                }

                if finally_handler_idx.is_some() {
                    self.exception_handler_stack.pop();
                }

                // 3. Lower Finally Unwind Handler
                if let Some(fh_idx) = finally_handler_idx {
                    self.current_block = fh_idx;
                    self.emit(MIRInstruction::Move {
                        line: 0,
                        dst: is_unwinding_var.clone(),
                        src: MIRValue::Constant {
                            value: "true".to_string(),
                            ty: TejxType::Bool,
                        },
                    });

                    // Save exception
                    let temp = self.new_temp(TejxType::Int64);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: temp.clone(),
                        callee: TEJX_GET_EXCEPTION.to_string(),
                        args: vec![],
                    });
                    self.emit(MIRInstruction::Move {
                        line: 0,
                        dst: saved_ex_var.clone(),
                        src: MIRValue::Variable {
                            name: temp,
                            ty: TejxType::Int64,
                        },
                    });

                    if let Some(fb_idx) = finally_body_idx {
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: fb_idx,
                        });
                    }
                }

                // 4. Lower Finally Body
                if let Some(fb_idx) = finally_body_idx {
                    self.current_block = fb_idx;
                    if let Some(f_stmt) = finally_block {
                        self.lower_statement(f_stmt);
                    }

                    let cb = self.current_block;
                    if !self.current_function.blocks[cb].is_terminated() {
                        let rethrow_idx = self.new_block("finally_rethrow");

                        self.emit(MIRInstruction::Branch {
                            line: 0,
                            condition: MIRValue::Variable {
                                name: is_unwinding_var.clone(),
                                ty: TejxType::Bool,
                            },
                            true_target: rethrow_idx,
                            false_target: exit_block_idx,
                        });

                        self.current_block = rethrow_idx;
                        self.emit(MIRInstruction::Throw {
                            line: 0,
                            value: MIRValue::Variable {
                                name: saved_ex_var,
                                ty: TejxType::Int64,
                            },
                        });
                    }
                }

                self.current_block = exit_block_idx;
            }
            HIRStatement::Throw { value, .. } => {
                let val = self.lower_expression(value);
                self.emit(MIRInstruction::Throw {
                    line: 0,
                    value: val,
                });
            }
        }
    }

    fn get_type_size(&self, ty: &TejxType) -> usize {
        match ty {
            TejxType::Class(name, _) => {
                let lookup_name = if name.contains('<') {
                    name.split('<').next().unwrap()
                } else {
                    name
                };
                // Anonymous record types (starts with {) are value types in arrays
                if lookup_name.starts_with('{') {
                    if let Some(fields) = self.class_fields.get(lookup_name) {
                        let mut size = 0;
                        for (_, fty) in fields {
                            size += fty.size();
                        }
                        return size;
                    }
                }
                // Named classes are pointers (8 bytes) in arrays
                8
            }
            _ => ty.size(),
        }
    }

    fn lower_expression(&mut self, expr: &HIRExpression) -> MIRValue {
        self.current_line = expr.get_line();
        match expr {
            HIRExpression::Literal { value, ty, .. } => MIRValue::Constant {
                value: value.clone(),
                ty: ty.clone(),
            },
            HIRExpression::Variable { name, ty, .. } => {
                let unique_name = self.resolve_variable(name);
                if let TejxType::Function(_, _) = ty {
                    if name.starts_with("f_") {
                        let temp = self.new_temp(TejxType::Int64);
                        let raw_ptr = MIRValue::Variable {
                            name: unique_name,
                            ty: ty.clone(),
                        };
                        self.emit(MIRInstruction::Call {
                            line: 0,
                            dst: temp.clone(),
                            callee: "rt_closure_from_ptr".to_string(),
                            args: vec![raw_ptr],
                        });
                        return MIRValue::Variable {
                            name: temp,
                            ty: TejxType::Int64,
                        };
                    }
                }
                MIRValue::Variable {
                    name: unique_name,
                    ty: ty.clone(),
                }
            }
            HIRExpression::NewExpr {
                class_name,
                _args,
                ty,
                ..
            } => {
                let is_raw_array = class_name.ends_with("[]") || class_name.contains("[");
                let is_array_wrapper = class_name.starts_with("Array<") || class_name == "Array";
                let is_any_array = is_raw_array || is_array_wrapper;

                // Create fixed-layout object if it's a class (unless it's a raw array which is just a header)
                let temp = self.new_temp(if is_raw_array {
                    ty.clone()
                } else {
                    TejxType::Class(class_name.clone(), vec![])
                });
                if !is_raw_array {
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        callee: RT_CLASS_NEW.to_string(),
                        args: vec![MIRValue::Constant {
                            value: format!("\"{}\"", class_name),
                            ty: TejxType::String,
                        }],
                        dst: temp.clone(),
                    });
                }

                let constructor_name = if is_raw_array {
                    "rt_Array_constructor_v2".to_string()
                } else {
                    format!("f_{}_constructor", class_name)
                };

                let mut constructor_args = vec![if is_raw_array {
                    MIRValue::Constant {
                        value: "0".to_string(),
                        ty: TejxType::Int64,
                    }
                } else {
                    MIRValue::Variable {
                        name: temp.clone(),
                        ty: TejxType::Class(class_name.clone(), vec![]),
                    }
                }];
                for arg in _args {
                    constructor_args.push(self.lower_expression(arg));
                }

                if is_any_array {
                    if _args.is_empty() {
                        // Push default 0 for sizeOrArr
                        constructor_args.push(MIRValue::Constant {
                            value: "0".to_string(),
                            ty: TejxType::Int64,
                        });
                    }
                    let elem_ty = ty.get_array_element_type();
                    let elem_size = self.get_type_size(&elem_ty);
                    constructor_args.push(MIRValue::Constant {
                        value: elem_size.to_string(),
                        ty: TejxType::Int64,
                    });

                    if is_raw_array {
                        // Pass flags
                        let flags = if matches!(ty, TejxType::FixedArray(_, _)) {
                            1
                        } else {
                            0
                        };
                        constructor_args.push(MIRValue::Constant {
                            value: flags.to_string(),
                            ty: TejxType::Int64,
                        });
                    }
                }

                let call_dst = if is_raw_array {
                    temp.clone()
                } else {
                    self.new_temp(TejxType::Void)
                };

                self.emit(MIRInstruction::Call {
                    line: 0,
                    callee: constructor_name,
                    args: constructor_args,
                    dst: call_dst,
                });

                MIRValue::Variable {
                    name: temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::BinaryExpr {
                left,
                op,
                right,
                ty,
                ..
            } => {
                match op {
                    TokenType::QuestionQuestion => {
                        // Nullish Coalescing: left ?? right
                        // if !rt_is_nullish(left) then left else right
                        let l_val = self.lower_expression(left);
                        let result_temp = self.new_temp(ty.clone());

                        let _nullish_check_block = self.new_block("nullish_check");
                        let not_null_block = self.new_block("not_null");
                        let null_block = self.new_block("is_null");
                        let merge_block = self.new_block("nullish_merge");

                        // Emit check: rt_is_nullish(l_val)
                        // Note: l_val might be Any or specific type. rt_is_nullish takes i64 (Any).
                        let is_null = self.new_temp(TejxType::Int64);
                        self.emit(MIRInstruction::Call {
                            line: 0,
                            dst: is_null.clone(),
                            callee: "rt_is_nullish".to_string(),
                            args: vec![l_val.clone()],
                        });

                        // Convert i64/bool to i1 for Branch? CodeGen expects i64 for condition?
                        // Branch instruction expects MIRValue::Variable (which is i64 usually).
                        // wait, Branch implementation in CodeGen:
                        // "stmt: Branch { condition, ... }"
                        // "val = resolve_value(condition)" -> returns string (register name)
                        // "emit: br i1 val..."
                        // BUT `resolve_value` returns i64 string?
                        // `resolve_value` returns register/const string.
                        // `codegen.rs`: "if is_bool_type { ... return "1" }"
                        // It seems CodeGen expects the condition value to be boolean-ish i1?
                        // Wait, `rt_is_nullish` returns 1 or 0 (i64).
                        // If we pass this i64 to Branch, LLVM verify might fail if it expects i1.
                        // CodeGen: "br i1 {}, ..."
                        // We need to Compare with 0?
                        // `MIRInstruction::Branch` takes a `condition` MIRValue.
                        // In `If` lowering: `cond_val = self.lower_expression(condition)`.
                        // If `condition` expr was `BinaryExpr` (e.g. `==`), it returns `Bool`.
                        // In CodeGen `BinaryOp` for comparators: `zext i1 %cmp to i64`. It returns i64!
                        // In CodeGen `Branch`:
                        // `let cond_str = resolve_value(condition);`
                        // `emit("trunc i64 {} to i1", cond_str)` ??
                        // I need to check CodeGen `Branch` implementation.
                        // I don't have CodeGen file open right now.
                        // But looking at `Loop` lowering `Branch`:
                        // `cond_val = self.lower_expression(condition);`
                        // If checking CodeGen from memory/previous reads:
                        // Usually `Branch` instruction handling in CodeGen takes `i64` and truncates or compares ne 0.
                        // Let's assume `rt_is_nullish` returns 1/0 (i64).
                        // We can use it directly? or compare `ne 0`?
                        // Let's create a comparison instr to be safe and cleaner.
                        let is_null_bool = self.new_temp(TejxType::Bool);
                        self.emit(MIRInstruction::BinaryOp {
                            line: 0,
                            dst: is_null_bool.clone(),
                            left: MIRValue::Variable {
                                name: is_null,
                                ty: TejxType::Int64,
                            },
                            op: TokenType::BangEqual, // != 0?
                            // wait, rt_is_nullish returns 1 if NULL.
                            // So if (is_null == 1) -> Go to null_block (evaluate right).
                            // if (is_null == 0) -> Go to not_null_block (return left).
                            // Let's check: is_null != 0
                            right: MIRValue::Constant {
                                value: "0".to_string(),
                                ty: TejxType::Int64,
                            },
                            op_width: TejxType::Int64,
                        });
                        // is_null_bool is True if is_null != 0 (i.e. is null).

                        self.emit(MIRInstruction::Branch {
                            line: 0,
                            condition: MIRValue::Variable {
                                name: is_null_bool,
                                ty: TejxType::Bool,
                            },
                            true_target: null_block,
                            false_target: not_null_block,
                        });

                        self.current_block = not_null_block;
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: result_temp.clone(),
                            src: l_val,
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: merge_block,
                        });

                        self.current_block = null_block;
                        let r_val = self.lower_expression(right);
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: result_temp.clone(),
                            src: r_val,
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: merge_block,
                        });

                        self.current_block = merge_block;
                        MIRValue::Variable {
                            name: result_temp,
                            ty: ty.clone(),
                        }
                    }
                    TokenType::AmpersandAmpersand => {
                        // Short-circuit AND: left && right
                        // if left then evaluate right else left
                        let l_val = self.lower_expression(left);
                        let result_temp = self.new_temp(ty.clone());

                        let right_block = self.new_block("and_right");
                        let false_block = self.new_block("and_false");
                        let merge_block = self.new_block("and_merge");

                        self.emit(MIRInstruction::Branch {
                            line: 0,
                            condition: l_val.clone(),
                            true_target: right_block,
                            false_target: false_block,
                        });

                        self.current_block = right_block;
                        let r_val = self.lower_expression(right);
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: result_temp.clone(),
                            src: r_val,
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: merge_block,
                        });

                        self.current_block = false_block;
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: result_temp.clone(),
                            src: l_val,
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: merge_block,
                        });

                        self.current_block = merge_block;
                        MIRValue::Variable {
                            name: result_temp,
                            ty: ty.clone(),
                        }
                    }
                    TokenType::Comma => {
                        let _l = self.lower_expression(left);
                        let r = self.lower_expression(right);
                        r
                    }
                    TokenType::PipePipe => {
                        // Short-circuit OR: left || right
                        // if left then left else evaluate right
                        let l_val = self.lower_expression(left);
                        let result_temp = self.new_temp(ty.clone());

                        let true_block = self.new_block("or_truthy");
                        let right_block = self.new_block("or_falsy");
                        let merge_block = self.new_block("or_merge");

                        self.emit(MIRInstruction::Branch {
                            line: 0,
                            condition: l_val.clone(),
                            true_target: true_block,
                            false_target: right_block,
                        });

                        self.current_block = true_block;
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: result_temp.clone(),
                            src: l_val,
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: merge_block,
                        });

                        self.current_block = right_block;
                        let r_val = self.lower_expression(right);
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: result_temp.clone(),
                            src: r_val,
                        });
                        self.emit(MIRInstruction::Jump {
                            line: 0,
                            target: merge_block,
                        });

                        self.current_block = merge_block;
                        MIRValue::Variable {
                            name: result_temp,
                            ty: ty.clone(),
                        }
                    }
                    _ => {
                        let l = self.lower_expression(left);
                        let r = self.lower_expression(right);
                        let temp = self.new_temp(ty.clone());
                        self.emit(MIRInstruction::BinaryOp {
                            line: 0,
                            dst: temp.clone(),
                            left: l.clone(),
                            op: op.clone(),
                            right: r,
                            op_width: ty.clone(),
                        });
                        MIRValue::Variable {
                            name: temp,
                            ty: ty.clone(),
                        }
                    }
                }
            }
            HIRExpression::Assignment { target, value, .. } => {
                let mut val = self.lower_expression(value);

                match target.as_ref() {
                    HIRExpression::Variable { name, ty, .. } => {
                        let unique_name = self.resolve_variable(name);
                        val = self.auto_box(val, ty);
                        self.emit(MIRInstruction::Move {
                            line: 0,
                            dst: unique_name,
                            src: val.clone(),
                        });
                    }
                    HIRExpression::MemberAccess {
                        target: obj_expr,
                        member,
                        ty,
                        ..
                    } => {
                        let obj_val = self.lower_expression(obj_expr);
                        val = self.auto_box(val, ty);
                        self.emit(MIRInstruction::StoreMember {
                            line: 0,
                            obj: obj_val,
                            member: member.clone(),
                            src: val.clone(),
                        });
                    }
                    HIRExpression::IndexAccess {
                        target: obj_expr,
                        index: idx_expr,
                        ty,
                        ..
                    } => {
                        let obj_val = self.lower_expression(obj_expr);
                        let mut idx_val = self.lower_expression(idx_expr);

                        if idx_val.get_type().is_float() {
                            let temp_idx = self.new_temp(TejxType::Int32);
                            self.emit(MIRInstruction::Cast {
                                line: 0,
                                dst: temp_idx.clone(),
                                src: idx_val,
                                ty: TejxType::Int32,
                            });
                            idx_val = MIRValue::Variable {
                                name: temp_idx,
                                ty: TejxType::Int32,
                            };
                        }

                        val = self.auto_box(val, ty);
                        self.emit(MIRInstruction::StoreIndex {
                            line: 0,
                            obj: obj_val,
                            index: idx_val,
                            src: val.clone(),
                            element_ty: ty.clone(),
                        });
                    }
                    _ => {}
                }
                val
            }
            HIRExpression::Call {
                callee, args, ty, ..
            } => {
                let mut final_callee = callee.clone();
                if callee.contains('.') {
                    let parts: Vec<&str> = callee.split('.').collect();
                    if parts.len() == 2 {
                        let base = parts[0];
                        let method = parts[1];
                        let resolved_base = self.resolve_variable(base);
                        final_callee = format!("{}.{}", resolved_base, method);
                    }
                }

                // Check if this is a UFCS/runtime array mutation that might reallocate
                let is_ufcs = final_callee.starts_with("rt_array_push")
                    || final_callee.starts_with("rt_array_unshift")
                    || final_callee.starts_with("rt_array_splice")
                    || final_callee.contains(".push")
                    || final_callee.contains(".unshift");

                let maybe_sig = self.signatures.get(&final_callee).cloned();
                let mir_args: Vec<MIRValue> = args
                    .iter()
                    .enumerate()
                    .map(|(i, a)| {
                        let val = self.lower_expression(a);
                        let mut target_ty = maybe_sig
                            .as_ref()
                            .and_then(|sig| sig.get(i))
                            .unwrap_or(&TejxType::Void)
                            .clone();

                        // Fix: prevent primitive boxing for typed arrays by overriding target type
                        if args.len() >= 1 {
                            if let Some(arr_ty) = args.get(0).map(|a| a.get_type()) {
                                if arr_ty.is_array() {
                                    if (final_callee.ends_with("_push")
                                        || final_callee.ends_with("_unshift")
                                        || final_callee.ends_with("_indexOf")
                                        || final_callee.ends_with("_includes"))
                                        && i == 1
                                    {
                                        target_ty = arr_ty.get_array_element_type();
                                    } else if final_callee.ends_with("_fill") {
                                        if i == 1 {
                                            target_ty = arr_ty.get_array_element_type();
                                        } else if i >= 2 {
                                            target_ty = TejxType::Int64;
                                        }
                                    } else if final_callee.ends_with("_splice") {
                                        if i == 1 || i == 2 {
                                            target_ty = TejxType::Int64;
                                        } else if i >= 3 {
                                            target_ty = arr_ty.get_array_element_type();
                                        }
                                    }
                                }
                            }
                        }

                        // Fix: prevent primitive boxing for math intrinsics
                        if final_callee == "std_math_sin"
                            || final_callee == "std_math_cos"
                            || final_callee == "std_math_tan"
                            || final_callee == "std_math_asin"
                            || final_callee == "std_math_acos"
                            || final_callee == "std_math_atan"
                            || final_callee == "std_math_sqrt"
                            || final_callee == "std_math_log"
                            || final_callee == "std_math_exp"
                            || final_callee == "std_math_round"
                            || final_callee == "std_math_floor"
                            || final_callee == "std_math_ceil"
                            || final_callee == "std_math_abs"
                            || final_callee == "std_math_pow"
                            || final_callee == "std_math_min"
                            || final_callee == "std_math_max"
                        {
                            target_ty = TejxType::Float64;
                        }

                        self.auto_box(val, &target_ty)
                    })
                    .collect();

                if final_callee.contains("Element_constructor") {
                    println!("DEBUG mir: new_temp for {} with ty {:?}", final_callee, ty);
                }
                let mut raw_temp = self.new_temp(ty.clone());
                // For strings coming back from Map/Set intrinsics, they might be raw ptrs,
                // and the user expects them as boxed strings if the variable is typed 'string'.
                // If it's a known runtime method that returns raw generic i64 representing a string ptr
                if (final_callee == "rt_Map_get"
                    || final_callee == "rt_array_pop"
                    || final_callee == "rt_array_shift")
                    && ty == &TejxType::String
                {
                    raw_temp = self.new_temp(TejxType::Int64);
                }

                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: raw_temp.clone(),
                    callee: final_callee.clone(),
                    args: mir_args,
                });

                let result_val = MIRValue::Variable {
                    name: raw_temp.clone(),
                    ty: ty.clone(),
                };

                result_val
            }
            HIRExpression::IndirectCall {
                callee, args, ty, ..
            } => {
                let mir_callee = self.lower_expression(callee);
                let mir_args: Vec<MIRValue> = args
                    .iter()
                    .map(|a| {
                        let mut val = self.lower_expression(a);
                        // For indirect calls, we assume boxed Any is expected (especially for lambdas)
                        let src_ty = val.get_type();
                        let is_primitive = src_ty.is_numeric()
                            || matches!(src_ty, TejxType::Bool | TejxType::String);
                        if is_primitive {
                            let box_func = match src_ty {
                                t if t.is_float() => Some("rt_box_number"),
                                t if t.is_numeric() => Some("rt_box_int"),
                                TejxType::Bool => Some("rt_box_boolean"),
                                _ => None,
                            };
                            if let Some(f) = box_func {
                                let temp = self.new_temp(TejxType::Int64);
                                self.emit(MIRInstruction::Call {
                                    line: 0,
                                    dst: temp.clone(),
                                    callee: f.to_string(),
                                    args: vec![val],
                                });
                                val = MIRValue::Variable {
                                    name: temp,
                                    ty: TejxType::Int64,
                                };
                            }
                        }
                        val
                    })
                    .collect();
                let temp = self.new_temp(ty.clone());
                self.emit(MIRInstruction::IndirectCall {
                    line: 0,
                    dst: temp.clone(),
                    callee: mir_callee,
                    args: mir_args,
                });
                MIRValue::Variable {
                    name: temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::Await { expr, ty, line } => {
                let val = self.lower_expression(expr);
                let is_async_worker = self.current_function.name.ends_with("_worker");

                if is_async_worker {
                    // 1. Evaluate the promise (val)
                    // 2. Schedule continuation: rt_promise_then(val, worker_ptr, ctx, next_state)
                    let next_state_id = self.async_state_counter;
                    self.async_state_counter += 1;

                    let worker_ptr = MIRValue::Constant {
                        value: format!("@{}", self.current_function.name),
                        ty: TejxType::Int64,
                    };

                    let ctx_val = MIRValue::Variable {
                        name: "ctx".to_string(),
                        ty: TejxType::Class("Int64[]".to_string(), vec![]),
                    };

                    // --- SAVE LOCALS TO CTX ---
                    let aps = self.current_async_params.clone();
                    self.collect_async_locals(&aps, &[]);
                    let promise_id_var = self.current_promise_id.clone();
                    let mut other_vars = self.async_locals.clone();

                    let mut ap_len = 0;
                    let local_ap = self.current_async_params.clone();
                    if let Some(ref ap) = local_ap {
                        let ap_names: Vec<String> = ap.iter().map(|(n, _)| n.clone()).collect();
                        other_vars.retain(|n: &String| !ap_names.contains(n));
                        ap_len = ap.len();

                        for (i, (name, ty)) in ap.iter().enumerate() {
                            let src_val = MIRValue::Variable {
                                name: name.clone(),
                                ty: ty.clone(),
                            };

                            let unused = self.new_async_temp(TejxType::Void);
                            self.emit(MIRInstruction::Call {
                                line: *line,
                                dst: unused,
                                callee: "rt_array_set_fast".to_string(),
                                args: vec![
                                    ctx_val.clone(),
                                    MIRValue::Constant {
                                        value: (i + 2).to_string(),
                                        ty: TejxType::Int32,
                                    },
                                    src_val,
                                ],
                            });
                        }
                    }

                    if let Some(ref p_var) = promise_id_var {
                        let ty = self.current_function.variables.get(p_var).unwrap().clone();
                        let src_val = MIRValue::Variable {
                            name: p_var.clone(),
                            ty: ty.clone(),
                        };

                        let unused = self.new_async_temp(TejxType::Void);
                        self.emit(MIRInstruction::Call {
                            line: *line,
                            dst: unused,
                            callee: "rt_array_set_fast".to_string(),
                            args: vec![
                                ctx_val.clone(),
                                MIRValue::Constant {
                                    value: "0".to_string(),
                                    ty: TejxType::Int32,
                                },
                                src_val,
                            ],
                        });
                    }

                    for (i, name) in other_vars.iter().enumerate() {
                        let ix = 2 + ap_len + i;
                        let ty = self.current_function.variables.get(name).unwrap().clone();
                        let src_val = MIRValue::Variable {
                            name: name.clone(),
                            ty: ty.clone(),
                        };

                        let unused = self.new_async_temp(TejxType::Void);
                        self.emit(MIRInstruction::Call {
                            line: *line,
                            dst: unused,
                            callee: "rt_array_set_fast".to_string(),
                            args: vec![
                                ctx_val.clone(),
                                MIRValue::Constant {
                                    value: ix.to_string(),
                                    ty: TejxType::Int32,
                                },
                                src_val,
                            ],
                        });
                    }

                    let next_state_any = MIRValue::Constant {
                        value: next_state_id.to_string(),
                        ty: TejxType::Int32,
                    };

                    let unused = self.new_temp(TejxType::Void);
                    self.emit(MIRInstruction::Call {
                        line: *line,
                        dst: unused,
                        callee: "rt_array_set_fast".to_string(),
                        args: vec![
                            ctx_val.clone(),
                            MIRValue::Constant {
                                value: "1".to_string(),
                                ty: TejxType::Int32,
                            },
                            next_state_any,
                        ],
                    });

                    // Call rt_promise_await_resume
                    let dummy_dst = self.new_temp(TejxType::Void);
                    self.emit(MIRInstruction::Call {
                        line: *line,
                        dst: dummy_dst,
                        callee: "rt_promise_await_resume".to_string(),
                        args: vec![val.clone(), worker_ptr, ctx_val.clone()],
                    });

                    // Return to event loop
                    self.emit(MIRInstruction::Return {
                        line: *line,
                        value: None,
                    });

                    // --- Split Block ---
                    let continuation_block =
                        self.new_block(&format!("await_cont_{}", next_state_id));
                    self.async_continuations
                        .push((next_state_id, continuation_block));
                    self.current_block = continuation_block;

                    let result_temp = self.new_temp(ty.clone());
                    self.emit(MIRInstruction::Call {
                        line: *line,
                        dst: result_temp.clone(),
                        callee: "rt_promise_get_value".to_string(),
                        args: vec![val],
                    });

                    MIRValue::Variable {
                        name: result_temp,
                        ty: ty.clone(),
                    }
                } else {
                    // Synchronous blocking wrapper fallback (for top-level await if allowed, or non-async functions)
                    let temp = self.new_temp(ty.clone());
                    self.emit(MIRInstruction::Call {
                        line: *line,
                        dst: temp.clone(),
                        callee: "rt_await".to_string(),
                        args: vec![val],
                    });
                    MIRValue::Variable {
                        name: temp,
                        ty: ty.clone(),
                    }
                }
            }
            HIRExpression::OptionalChain {
                target,
                operation,
                ty,
                ..
            } => {
                // Lower to runtime call: __optional_chain(target, "operation")
                let val = self.lower_expression(target);
                let op_str = MIRValue::Constant {
                    value: format!("\"{}\"", operation), // Quote string
                    ty: TejxType::String,
                };
                let temp = self.new_temp(ty.clone());
                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: temp.clone(),
                    callee: "rt_optional_chain".to_string(),
                    args: vec![val, op_str],
                });
                MIRValue::Variable {
                    name: temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::IndexAccess {
                target, index, ty, ..
            } => {
                let obj = self.lower_expression(target);
                let mut idx = self.lower_expression(index);

                if idx.get_type().is_float() {
                    let temp_idx = self.new_temp(TejxType::Int32);
                    self.emit(MIRInstruction::Cast {
                        line: 0,
                        dst: temp_idx.clone(),
                        src: idx,
                        ty: TejxType::Int32,
                    });
                    idx = MIRValue::Variable {
                        name: temp_idx,
                        ty: TejxType::Int32,
                    };
                }

                let obj_ty = obj.get_type();
                if matches!(obj_ty, TejxType::String) {
                    let temp = self.new_temp(TejxType::String);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: temp.clone(),
                        callee: "rt_str_at".to_string(),
                        args: vec![obj, idx],
                    });
                    return MIRValue::Variable {
                        name: temp,
                        ty: TejxType::String,
                    };
                }

                let elem_ty = obj_ty.get_array_element_type();

                // Only load as 'any' if the array actually stores tagged values.
                // Otherwise use the actual element type (int, float, etc.)
                let load_ty = if matches!(elem_ty, TejxType::Int64) {
                    TejxType::Int64
                } else {
                    elem_ty.clone()
                };

                let temp = self.new_temp(load_ty.clone());
                self.emit(MIRInstruction::LoadIndex {
                    line: 0,
                    dst: temp.clone(),
                    obj: obj.clone(),
                    index: idx.clone(),
                    element_ty: load_ty.clone(),
                });

                let val = MIRValue::Variable {
                    name: temp,
                    ty: load_ty,
                };

                // Auto-unbox only if we actually loaded a tagged value but the target expects a primitive
                self.auto_box(val, ty)
            }
            HIRExpression::MemberAccess {
                target, member, ty, ..
            } => {
                let obj = self.lower_expression(target);
                let obj_ty = obj.get_type();

                // Special handling for 'length' on arrays, strings, and slices
                if member == "length" {
                    if matches!(obj_ty, TejxType::String) || obj_ty.is_array() || obj_ty.is_slice()
                    {
                        let temp = self.new_temp(TejxType::Int32);
                        self.emit(MIRInstruction::Call {
                            line: 0,
                            dst: temp.clone(),
                            callee: "rt_len".to_string(),
                            args: vec![obj],
                        });
                        return MIRValue::Variable {
                            name: temp,
                            ty: TejxType::Int32,
                        };
                    }
                }

                let temp = self.new_temp(ty.clone());
                self.emit(MIRInstruction::LoadMember {
                    line: 0,
                    dst: temp.clone(),
                    obj,
                    member: member.clone(),
                });
                MIRValue::Variable {
                    name: temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::ObjectLiteral { entries, ty, .. } => {
                // Default: create a generic object (Map)
                let map_temp = self.new_temp(ty.clone());
                self.emit(MIRInstruction::Call {
                    line: 0,
                    callee: "rt_Map_constructor".to_string(),
                    args: vec![],
                    dst: map_temp.clone(),
                });

                for (k, v) in entries {
                    let mut val = self.lower_expression(v);
                    let is_primitive = val.get_type().is_numeric()
                        || matches!(val.get_type(), TejxType::Bool | TejxType::String);
                    if is_primitive {
                        let src_ty = val.get_type();
                        let box_func = match src_ty {
                            t if t.is_float() => Some("rt_box_number"),
                            t if t.is_numeric() => Some("rt_box_int"),
                            TejxType::Bool => Some("rt_box_boolean"),
                            _ => None,
                        };
                        if let Some(f) = box_func {
                            let temp = self.new_temp(TejxType::Int64);
                            self.emit(MIRInstruction::Call {
                                line: 0,
                                dst: temp.clone(),
                                callee: f.to_string(),
                                args: vec![val],
                            });
                            val = MIRValue::Variable {
                                name: temp,
                                ty: TejxType::Int64,
                            };
                        }
                    }

                    // For the key, we need to box the string literal
                    let key_boxed = self.new_temp(TejxType::String);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: key_boxed.clone(),
                        callee: RT_STRING_FROM_C_STR.to_string(),
                        args: vec![MIRValue::Constant {
                            value: k.clone(),
                            ty: TejxType::String,
                        }],
                    });

                    let void_temp = self.new_temp(TejxType::Void);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        callee: RT_MAP_SET.to_string(),
                        args: vec![
                            MIRValue::Variable {
                                name: map_temp.clone(),
                                ty: ty.clone(),
                            },
                            MIRValue::Variable {
                                name: key_boxed,
                                ty: TejxType::String,
                            },
                            val,
                        ],
                        dst: void_temp,
                    });
                }

                MIRValue::Variable {
                    name: map_temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::ArrayLiteral {
                elements,
                sized_allocation,
                ty,
                line: expr_line,
            } => {
                let arr_temp = self.new_temp(ty.clone());

                let initial_size = if let Some(size_hir) = sized_allocation {
                    self.lower_expression(&*size_hir)
                } else {
                    MIRValue::Constant {
                        value: elements.len().to_string(),
                        ty: TejxType::Int64,
                    }
                };
                let array_obj = MIRValue::Constant {
                    value: "0".to_string(),
                    ty: TejxType::Int64,
                };

                // Call constructor: rt_Array_constructor_v2(this, sizeOrArr, elem_size, flags)
                let inner_type = ty.get_array_element_type();
                let elem_size_bytes = self.get_type_size(&inner_type);

                let args = vec![
                    array_obj,
                    initial_size,
                    MIRValue::Constant {
                        value: elem_size_bytes.to_string(),
                        ty: TejxType::Int64,
                    },
                    MIRValue::Constant {
                        value: "0".to_string(), // Literals are growable by default
                        ty: TejxType::Int64,
                    },
                ];

                self.emit(MIRInstruction::Call {
                    dst: arr_temp.clone(),
                    callee: "rt_Array_constructor_v2".to_string(),
                    args,
                    line: *expr_line,
                });

                let unused = self.new_temp(TejxType::Void);
                for (i, e) in elements.into_iter().enumerate() {
                    let mut val = self.lower_expression(e);
                    let elem_ty = ty.get_array_element_type();
                    let should_box = (val.get_type().is_numeric()
                        || matches!(val.get_type(), TejxType::Bool | TejxType::Char))
                        && (elem_ty == TejxType::Int64);

                    if should_box {
                        let src_ty = val.get_type();
                        let box_func = match src_ty {
                            t if t.is_float() => Some("rt_box_number"),
                            t if t.is_numeric() => Some("rt_box_int"),
                            TejxType::Bool => Some("rt_box_boolean"),
                            TejxType::Char => Some("rt_box_char"),
                            _ => None,
                        };
                        if let Some(f) = box_func {
                            let temp = self.new_temp(TejxType::Int64);
                            self.emit(MIRInstruction::Call {
                                line: 0,
                                dst: temp.clone(),
                                callee: f.to_string(),
                                args: vec![val],
                            });
                            val = MIRValue::Variable {
                                name: temp,
                                ty: TejxType::Int64,
                            };
                        }
                    }

                    self.emit(MIRInstruction::Call {
                        line: 0,
                        callee: "rt_array_set_fast".to_string(),
                        args: vec![
                            MIRValue::Variable {
                                name: arr_temp.clone(),
                                ty: ty.clone(),
                            },
                            MIRValue::Constant {
                                value: i.to_string(),
                                ty: TejxType::Int64,
                            },
                            val,
                        ],
                        dst: unused.clone(),
                    });
                }

                // Final results already in arr_temp

                MIRValue::Variable {
                    name: arr_temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::If {
                condition,
                then_branch,
                else_branch,
                ty,
                ..
            } => {
                let cond_val = self.lower_expression(condition);
                let result_temp = self.new_temp(ty.clone());

                let then_block = self.new_block("ternary_then");
                let else_block = self.new_block("ternary_else");
                let exit_block = self.new_block("ternary_exit");

                self.emit(MIRInstruction::Branch {
                    line: 0,
                    condition: cond_val,
                    true_target: then_block,
                    false_target: else_block,
                });

                // Then
                self.current_block = then_block;
                let then_val = self.lower_expression(then_branch);
                self.emit(MIRInstruction::Move {
                    line: 0,
                    dst: result_temp.clone(),
                    src: then_val,
                });
                self.emit(MIRInstruction::Jump {
                    line: 0,
                    target: exit_block,
                });

                // Else
                self.current_block = else_block;
                let else_val = self.lower_expression(else_branch);
                self.emit(MIRInstruction::Move {
                    line: 0,
                    dst: result_temp.clone(),
                    src: else_val,
                });
                self.emit(MIRInstruction::Jump {
                    line: 0,
                    target: exit_block,
                });

                self.current_block = exit_block;
                MIRValue::Variable {
                    name: result_temp,
                    ty: ty.clone(),
                }
            }
            HIRExpression::Sequence { expressions, .. } => {
                let mut last_val = MIRValue::Constant {
                    value: "0".to_string(),
                    ty: TejxType::Int32,
                };
                for e in expressions {
                    last_val = self.lower_expression(e);
                }
                last_val
            }
            HIRExpression::NoneLiteral { .. } => MIRValue::Constant {
                value: "0".to_string(),
                ty: TejxType::Int64,
            },
            HIRExpression::SomeExpr { value, .. } => self.lower_expression(value),
        }
    }
}
