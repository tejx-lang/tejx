use crate::common::intrinsics::*;
use crate::common::types::TejxType;
use crate::frontend::token::TokenType;
/// HIR → MIR Lowering pass, mirroring C++ MIRLowering.cpp
/// Converts high-level typed IR into basic blocks with low-level instructions.
use crate::middle::hir::*;
use crate::middle::mir::*;
use std::collections::HashMap;

const ARRAY_FLAG_FIXED: i64 = 0x0100;
const ARRAY_FLAG_PTR: i64 = 0x0400;
const ARRAY_FLAG_KIND_UNSIGNED: i64 = 0x1000;
const ARRAY_FLAG_KIND_FLOAT: i64 = 0x2000;
const ARRAY_FLAG_KIND_BOOL: i64 = 0x3000;
const ARRAY_FLAG_KIND_CHAR: i64 = 0x4000;

#[derive(Clone)]
struct LoopContext {
    continue_target: usize,
    break_target: usize,
}

#[derive(Clone)]
struct ThrowFinallyContext {
    id: usize,
    is_unwinding_var: String,
    saved_ex_var: String,
    finally_body_idx: usize,
}

#[derive(Clone)]
struct ControlFinallyContext {
    id: usize,
    finally_stmt: HIRStatement,
}

#[derive(Clone)]
enum PendingControlTransfer {
    Return(Option<MIRValue>),
    Jump(usize),
}

pub struct MIRLowering {
    current_function: MIRFunction,
    current_block: usize, // index into current_function.blocks
    temp_counter: usize,
    block_counter: usize,
    finally_counter: usize,
    loop_stack: Vec<LoopContext>,
    exception_handler_stack: Vec<usize>,
    expected_ty: Option<TejxType>,
    signatures: HashMap<String, Vec<TejxType>>,
    current_return_type: TejxType,
    current_line: usize,
    scopes: Vec<HashMap<String, String>>, // Stack of scopes: original_name -> unique_mir_name
    var_counter: usize,
    class_fields: HashMap<String, Vec<(String, TejxType)>>,
    control_finally_stack: Vec<ControlFinallyContext>,
    throw_finally_stack: Vec<ThrowFinallyContext>,
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
            finally_counter: 0,
            loop_stack: Vec::new(),
            exception_handler_stack: Vec::new(),
            expected_ty: None,
            signatures,
            current_return_type: TejxType::Void,
            current_line: 0,
            scopes: vec![HashMap::new()], // Global/Function scope
            var_counter: 0,
            class_fields,
            control_finally_stack: Vec::new(),
            throw_finally_stack: Vec::new(),
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
                ..
            } => (
                name.clone(),
                params.clone(),
                body.as_ref(),
                _return_type.clone(),
                *is_extern,
            ),
            _ => (
                crate::common::intrinsics::TEJX_MAIN.to_string(),
                vec![],
                hir_func,
                TejxType::Void,
                false,
            ),
        };

        let (name, params, body, ret_ty, is_extern) = info;
        self.lower_function(name, params, ret_ty, body, is_extern)
    }

    pub fn lower_function(
        &mut self,
        name: String,
        params: Vec<(String, TejxType)>,
        return_type: TejxType,
        body: &HIRStatement,
        is_extern: bool,
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

        self.pre_collect_variables(body);

        let entry = self.new_block("entry");
        self.current_function.entry_block = entry;
        self.current_block = entry;

        self.lower_statement(body);

        // Ensure current block is terminated
        let cb = self.current_block;
        if !self.current_function.blocks[cb].is_terminated() {
            self.emit(MIRInstruction::Return {
                line: 0,
                value: None,
            });
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



    fn new_finally_id(&mut self) -> usize {
        let id = self.finally_counter;
        self.finally_counter += 1;
        id
    }

    fn pop_control_finally_if(&mut self, id: usize) {
        if self.control_finally_stack.last().map(|ctx| ctx.id) == Some(id) {
            self.control_finally_stack.pop();
        }
    }

    fn pop_throw_finally_if(&mut self, id: usize) {
        if self.throw_finally_stack.last().map(|ctx| ctx.id) == Some(id) {
            self.throw_finally_stack.pop();
        }
    }

    fn emit_return_statement(&mut self, mut val: Option<MIRValue>) {
        if let Some(ret_val) = val {
            val = Some(self.auto_box(ret_val, &self.current_return_type.clone()));
        }

        self.emit(MIRInstruction::Return {
            line: 0,
            value: val,
        });
    }

    fn emit_control_transfer(&mut self, transfer: PendingControlTransfer) {
        if self.control_finally_stack.last().is_some() {
            self.lower_control_transfer_through_finally(transfer);
            return;
        }

        match transfer {
            PendingControlTransfer::Return(val) => self.emit_return_statement(val),
            PendingControlTransfer::Jump(target) => {
                self.emit(MIRInstruction::Jump { line: 0, target });
            }
        }
    }

    fn lower_control_transfer_through_finally(&mut self, transfer: PendingControlTransfer) {
        let Some(ctx) = self.control_finally_stack.pop() else {
            self.emit_control_transfer(transfer);
            return;
        };

        self.pop_throw_finally_if(ctx.id);

        if self.current_function.blocks[self.current_block]
            .exception_handler
            .is_some()
        {
            self.emit(MIRInstruction::PopHandler { line: 0 });
        }

        self.lower_statement(&ctx.finally_stmt);

        if !self.current_function.blocks[self.current_block].is_terminated() {
            self.emit_control_transfer(transfer);
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
            let is_primitive_storage_cast = matches!(target_ty, TejxType::Int64 | TejxType::Any)
                && (src_ty.is_numeric() || matches!(src_ty, TejxType::Bool | TejxType::Char));
            let is_downcast = (src_ty == &TejxType::Int64)
                && (target_ty.is_numeric()
                    || matches!(target_ty, TejxType::Bool | TejxType::String));
            if is_primitive_cast || is_primitive_storage_cast || is_downcast {
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

    fn emit_array_receiver_storeback(&mut self, receiver: &HIRExpression, updated_array: MIRValue) {
        match receiver {
            HIRExpression::Variable { name, .. } => {
                self.emit(MIRInstruction::Move {
                    line: 0,
                    dst: self.resolve_variable(name),
                    src: updated_array,
                });
            }
            HIRExpression::MemberAccess { target, member, .. } => {
                let obj_val = self.lower_expression(target);
                self.emit(MIRInstruction::StoreMember {
                    obj: obj_val,
                    member: member.clone(),
                    src: updated_array,
                    line: 0,
                });
            }
            _ => {}
        }
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
                    self.emit_control_transfer(PendingControlTransfer::Jump(ctx.break_target));
                }
            }
            HIRStatement::Continue { .. } => {
                if let Some(ctx) = self.loop_stack.last() {
                    self.emit_control_transfer(PendingControlTransfer::Jump(ctx.continue_target));
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
                let val = value.as_ref().map(|e| self.lower_expression(e));
                self.emit_control_transfer(PendingControlTransfer::Return(val));
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
                let finally_id = finally_block.as_ref().map(|_| self.new_finally_id());

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
                if let (Some(id), Some(f_stmt)) = (finally_id, finally_block.as_deref()) {
                    self.control_finally_stack.push(ControlFinallyContext {
                        id,
                        finally_stmt: f_stmt.clone(),
                    });
                }

                self.lower_statement(try_block);
                if let Some(id) = finally_id {
                    self.pop_control_finally_if(id);
                }

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
                if let (Some(fb_idx), Some(id), Some(f_stmt)) =
                    (finally_body_idx, finally_id, finally_block.as_deref())
                {
                    self.control_finally_stack.push(ControlFinallyContext {
                        id,
                        finally_stmt: f_stmt.clone(),
                    });
                    self.throw_finally_stack.push(ThrowFinallyContext {
                        id,
                        is_unwinding_var: is_unwinding_var.clone(),
                        saved_ex_var: saved_ex_var.clone(),
                        finally_body_idx: fb_idx,
                    });
                }
                self.lower_statement(catch_block);
                if let Some(id) = finally_id {
                    self.pop_throw_finally_if(id);
                    self.pop_control_finally_if(id);
                }

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

                        if self.current_function.blocks[cb].exception_handler.is_some() {
                            self.emit(MIRInstruction::PopHandler { line: 0 });
                        }

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
                if let Some(ctx) = self.throw_finally_stack.last().cloned() {
                    if self.current_function.blocks[self.current_block]
                        .exception_handler
                        .is_some()
                    {
                        self.emit(MIRInstruction::PopHandler { line: 0 });
                    }
                    self.emit(MIRInstruction::Move {
                        line: 0,
                        dst: ctx.is_unwinding_var,
                        src: MIRValue::Constant {
                            value: "true".to_string(),
                            ty: TejxType::Bool,
                        },
                    });
                    self.emit(MIRInstruction::Move {
                        line: 0,
                        dst: ctx.saved_ex_var,
                        src: val,
                    });
                    self.emit(MIRInstruction::Jump {
                        line: 0,
                        target: ctx.finally_body_idx,
                    });
                } else {
                    self.emit(MIRInstruction::Throw {
                        line: 0,
                        value: val,
                    });
                }
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

    fn is_heap_ref_type(&self, ty: &TejxType) -> bool {
        match ty {
            TejxType::String | TejxType::DynamicArray(_) | TejxType::Object(_) | TejxType::Any => {
                true
            }
            TejxType::Class(name, _) => {
                let lookup_name = if name.contains('<') {
                    name.split('<').next().unwrap()
                } else {
                    name.as_str()
                };
                !lookup_name.starts_with('{')
            }
            _ => false,
        }
    }

    fn array_flags_for_type(&self, array_ty: &TejxType) -> i64 {
        let mut flags = 0;
        if matches!(array_ty, TejxType::FixedArray(_, _)) {
            flags |= ARRAY_FLAG_FIXED;
        }
        let elem_ty = array_ty.get_array_element_type();
        if self.is_heap_ref_type(&elem_ty) {
            flags |= ARRAY_FLAG_PTR;
        }
        flags |= match elem_ty {
            TejxType::UInt8
            | TejxType::UInt16
            | TejxType::UInt32
            | TejxType::UInt64
            | TejxType::UInt128 => ARRAY_FLAG_KIND_UNSIGNED,
            TejxType::Float16 | TejxType::Float32 | TejxType::Float64 => ARRAY_FLAG_KIND_FLOAT,
            TejxType::Bool => ARRAY_FLAG_KIND_BOOL,
            TejxType::Char => ARRAY_FLAG_KIND_CHAR,
            _ => 0,
        };
        flags
    }

    fn lower_expression(&mut self, expr: &HIRExpression) -> MIRValue {
        self.current_line = expr.get_line();
        match expr {
            HIRExpression::Cast { expr, ty, .. } => {
                let src_val = self.lower_expression(expr);
                let dst_temp = self.new_temp(ty.clone());
                self.emit(MIRInstruction::Cast {
                    line: 0,
                    dst: dst_temp.clone(),
                    src: src_val,
                    ty: ty.clone(),
                });
                MIRValue::Variable {
                    name: dst_temp,
                    ty: ty.clone(),
                }
            }
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
                if matches!(&ty, TejxType::Class(name, _) if name == "Promise") {
                    let temp = self.new_temp(ty.clone());
                    let executor = _args
                        .first()
                        .map(|arg| self.lower_expression(arg))
                        .unwrap_or(MIRValue::Constant {
                            value: "0".to_string(),
                            ty: TejxType::Int64,
                        });
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        callee: "rt_promise_from_executor".to_string(),
                        args: vec![executor],
                        dst: temp.clone(),
                    });
                    return MIRValue::Variable {
                        name: temp,
                        ty: ty.clone(),
                    };
                }

                let is_raw_array = class_name.ends_with("[]") || class_name.contains("[");
                let is_any_array = is_raw_array;

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
                let constructor_sig = self.signatures.get(&constructor_name).cloned();
                for (i, arg) in _args.iter().enumerate() {
                    let arg_val = self.lower_expression(arg);
                    if let Some(params) = &constructor_sig {
                        // Account for 'this' (param 0)
                        if i + 1 < params.len() {
                            let target_ty = &params[i + 1];
                            constructor_args.push(self.auto_box(arg_val, target_ty));
                        } else {
                            constructor_args.push(arg_val);
                        }
                    } else {
                        constructor_args.push(arg_val);
                    }
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
                        let flags = self.array_flags_for_type(ty);
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

                        self.lower_expression(right)
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
                            op: *op,
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
                        let obj_val = if let HIRExpression::MemberAccess {
                            target,
                            member,
                            ty: member_ty,
                            ..
                        } = obj_expr.as_ref()
                        {
                            let base_obj = self.lower_expression(target);
                            let loaded_arr = self.new_temp(member_ty.clone());
                            self.emit(MIRInstruction::LoadMember {
                                line: 0,
                                dst: loaded_arr.clone(),
                                obj: base_obj.clone(),
                                member: member.clone(),
                            });
                            MIRValue::Variable {
                                name: loaded_arr,
                                ty: member_ty.clone(),
                            }
                        } else {
                            self.lower_expression(obj_expr)
                        };
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
                            obj: obj_val.clone(),
                            index: idx_val,
                            src: val.clone(),
                            element_ty: ty.clone(),
                        });

                        if matches!(obj_expr.as_ref(), HIRExpression::MemberAccess { ty, .. } if ty.is_array())
                        {
                            self.emit_array_receiver_storeback(obj_expr, obj_val);
                        }
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
                let _is_ufcs = final_callee.starts_with("rt_array_push")
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
                        if !args.is_empty() {
                            if let Some(arr_ty) = args.first().map(|a| a.get_type()) {
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

                let mut raw_temp = self.new_temp(ty.clone());
                // For strings coming back from some runtime calls (e.g. property access or array pop/shift),
                // they might be raw ptrs, but the user expects boxed strings if the variable is typed 'string'.
                if (final_callee == "rt_get_property"
                    || final_callee == "rt_array_pop"
                    || final_callee == "rt_array_shift")
                    && ty == &TejxType::String
                {
                    raw_temp = self.new_temp(TejxType::Int64);
                }

                let is_array_mut = matches!(
                    final_callee.as_str(),
                    "rt_array_push"
                        | "rt_array_unshift"
                        | "rt_array_splice"
                        | "rt_array_reverse"
                        | "rt_array_fill"
                        | "rt_array_sort"
                );
                let returns_length =
                    final_callee == "rt_array_push" || final_callee == "rt_array_unshift";

                if is_array_mut {
                    let arr_val = mir_args.first().cloned();
                    if let Some(arr_val) = arr_val {
                        let arr_ty = arr_val.get_type();
                        let new_arr_tmp = self.new_temp(arr_ty.clone());
                        self.emit(MIRInstruction::Call {
                            line: 0,
                            dst: new_arr_tmp.clone(),
                            callee: final_callee.clone(),
                            args: mir_args,
                        });

                        if let Some(first_arg) = args.first() {
                            let updated_arr = MIRValue::Variable {
                                name: new_arr_tmp.clone(),
                                ty: arr_ty.clone(),
                            };
                            self.emit_array_receiver_storeback(first_arg, updated_arr.clone());

                            if !matches!(
                                first_arg,
                                HIRExpression::Variable { .. } | HIRExpression::MemberAccess { .. }
                            ) {
                                if let MIRValue::Variable { name, .. } = &arr_val {
                                    self.emit(MIRInstruction::Move {
                                        line: 0,
                                        dst: name.clone(),
                                        src: updated_arr,
                                    });
                                }
                            }
                        }

                        if returns_length && *ty == TejxType::Int32 {
                            let len_tmp = self.new_temp(TejxType::Int32);
                            self.emit(MIRInstruction::Call {
                                line: 0,
                                dst: len_tmp.clone(),
                                callee: "rt_len".to_string(),
                                args: vec![MIRValue::Variable {
                                    name: new_arr_tmp.clone(),
                                    ty: arr_ty.clone(),
                                }],
                            });
                            return MIRValue::Variable {
                                name: len_tmp,
                                ty: TejxType::Int32,
                            };
                        }

                        return MIRValue::Variable {
                            name: new_arr_tmp,
                            ty: arr_ty.clone(),
                        };
                    }
                }

                self.emit(MIRInstruction::Call {
                    line: 0,
                    dst: raw_temp.clone(),
                    callee: final_callee.clone(),
                    args: mir_args,
                });

                MIRValue::Variable {
                    name: raw_temp.clone(),
                    ty: ty.clone(),
                }
            }
            HIRExpression::IndirectCall {
                callee, args, ty, ..
            } => {
                let mir_callee = self.lower_expression(callee);
                let mir_args: Vec<MIRValue> =
                    args.iter().map(|a| self.lower_expression(a)).collect();
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

                if obj_ty.is_object() {
                    let temp = self.new_temp(TejxType::Any);
                    self.emit(MIRInstruction::Call {
                        line: 0,
                        dst: temp.clone(),
                        callee: "rt_get_property".to_string(), // Use generic property getter
                        args: vec![obj, idx],
                    });
                    let val_any = MIRValue::Variable {
                        name: temp,
                        ty: TejxType::Any,
                    };
                    if ty != &TejxType::Any {
                        let cast_temp = self.new_temp(ty.clone());
                        self.emit(MIRInstruction::Cast {
                            line: 0,
                            dst: cast_temp.clone(),
                            src: val_any,
                            ty: ty.clone(),
                        });
                        return MIRValue::Variable {
                            name: cast_temp,
                            ty: ty.clone(),
                        };
                    }
                    return val_any;
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
                if member == "length"
                    && (matches!(obj_ty, TejxType::String)
                        || obj_ty.is_array()
                        || obj_ty.is_slice())
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
                let map_temp = self.new_temp(ty.clone());
                self.emit(MIRInstruction::Call {
                    line: 0,
                    callee: "rt_object_new".to_string(),
                    args: vec![],
                    dst: map_temp.clone(),
                });

                for (key, value) in entries {
                    let value = self.lower_expression(value);
                    self.emit(MIRInstruction::StoreMember {
                        obj: MIRValue::Variable {
                            name: map_temp.clone(),
                            ty: ty.clone(),
                        },
                        member: key.clone(),
                        src: value,
                        line: 0,
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
                    self.lower_expression(size_hir)
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

                let flags = self.array_flags_for_type(ty);

                let args = vec![
                    array_obj,
                    initial_size,
                    MIRValue::Constant {
                        value: elem_size_bytes.to_string(),
                        ty: TejxType::Int64,
                    },
                    MIRValue::Constant {
                        value: flags.to_string(),
                        ty: TejxType::Int64,
                    },
                ];

                self.emit(MIRInstruction::Call {
                    dst: arr_temp.clone(),
                    callee: "rt_Array_constructor_v2".to_string(),
                    args,
                    line: *expr_line,
                });

                for (i, e) in elements.iter().enumerate() {
                    let elem_ty = ty.get_array_element_type();
                    let mut val = self.lower_expression(e);
                    val = self.auto_box(val, &elem_ty);

                    self.emit(MIRInstruction::StoreIndex {
                        line: 0,
                        obj: MIRValue::Variable {
                            name: arr_temp.clone(),
                            ty: ty.clone(),
                        },
                        index: MIRValue::Constant {
                            value: i.to_string(),
                            ty: TejxType::Int64,
                        },
                        src: val,
                        element_ty: elem_ty,
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
                ty: TejxType::Void,
            },
            HIRExpression::SomeExpr { value, .. } => self.lower_expression(value),
        }
    }
}
