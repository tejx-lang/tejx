/// Borrow Checker for MIR, mirroring C++ BorrowChecker.cpp
/// Performs simple ownership analysis: tracks Live/Moved state for class-type variables.

use crate::mir::*;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
enum VarState {
    // Uninitialized, // unused
    Live,
    Moved,
}

pub struct BorrowChecker {
    pub errors: Vec<String>,
    block_visited: Vec<bool>,
}

impl BorrowChecker {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            block_visited: Vec::new(),
        }
    }

    fn error(&mut self, msg: String) {
        self.errors.push(msg);
    }

    pub fn check(&mut self, func: &MIRFunction) {
        self.errors.clear();
        self.block_visited = vec![false; func.blocks.len()];

        let entry_state: HashMap<String, VarState> = HashMap::new();
        self.check_block(func, func.entry_block, entry_state);
    }

    fn check_block(
        &mut self,
        func: &MIRFunction,
        block_idx: usize,
        mut current_state: HashMap<String, VarState>,
    ) {
        if block_idx >= func.blocks.len() {
            return;
        }

        // Simple cycle detection (not full fix-point)
        if self.block_visited[block_idx] {
            return;
        }
        self.block_visited[block_idx] = true;

        let block = &func.blocks[block_idx];

        for inst in &block.instructions {
            match inst {
                MIRInstruction::Move { dst, src } => {
                    // Check source
                    if let MIRValue::Variable { name, ty } = src {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!(
                                "Borrow Error: Use of moved variable '{}'", name
                            ));
                        }
                        // Only move class types (primitives are Copy)
                        if ty.is_class() {
                            current_state.insert(name.clone(), VarState::Moved);
                        }
                    }
                    // Define destination
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::BinaryOp { dst, left, right, .. } => {
                    // Check operands
                    if let MIRValue::Variable { name, .. } = left {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!(
                                "Borrow Error: Use of moved variable '{}'", name
                            ));
                        }
                    }
                    if let MIRValue::Variable { name, .. } = right {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!(
                                "Borrow Error: Use of moved variable '{}'", name
                            ));
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::Branch { condition, true_target, false_target } => {
                    // Check condition
                    if let MIRValue::Variable { name, .. } = condition {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!(
                                "Borrow Error: Use of moved variable '{}'", name
                            ));
                        }
                    }
                    self.check_block(func, *true_target, current_state.clone());
                    self.check_block(func, *false_target, current_state);
                    return;
                }
                MIRInstruction::Jump { target } => {
                    self.check_block(func, *target, current_state);
                    return;
                }
                MIRInstruction::Return { value } => {
                    if let Some(MIRValue::Variable { name, .. }) = value {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!(
                                "Borrow Error: Return of moved variable '{}'", name
                            ));
                        }
                    }
                    return;
                }
                MIRInstruction::Call { dst, args, .. } => {
                    for arg in args {
                        if let MIRValue::Variable { name, .. } = arg {
                            if current_state.get(name) == Some(&VarState::Moved) {
                                self.error(format!(
                                    "Borrow Error: Use of moved variable '{}'", name
                                ));
                            }
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::IndirectCall { dst, callee, args } => {
                    if let MIRValue::Variable { name, .. } = callee {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                        }
                    }
                    for arg in args {
                        if let MIRValue::Variable { name, .. } = arg {
                            if current_state.get(name) == Some(&VarState::Moved) {
                                self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                            }
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::ObjectLiteral { dst, entries, .. } => {
                    for (_, v) in entries {
                        if let MIRValue::Variable { name, .. } = v {
                            if current_state.get(name) == Some(&VarState::Moved) {
                                self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                            }
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::ArrayLiteral { dst, elements, .. } => {
                    for v in elements {
                        if let MIRValue::Variable { name, .. } = v {
                            if current_state.get(name) == Some(&VarState::Moved) {
                                self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                            }
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::LoadMember { dst, obj, .. } => {
                    if let MIRValue::Variable { name, .. } = obj {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::StoreMember { obj, src, .. } => {
                    if let MIRValue::Variable { name, .. } = obj {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                        }
                    }
                    if let MIRValue::Variable { name, .. } = src {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                        }
                    }
                }
                MIRInstruction::LoadIndex { dst, obj, index } => {
                    if let MIRValue::Variable { name, .. } = obj {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                        }
                    }
                    if let MIRValue::Variable { name, .. } = index {
                        if current_state.get(name) == Some(&VarState::Moved) {
                            self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                        }
                    }
                    current_state.insert(dst.clone(), VarState::Live);
                }
                MIRInstruction::StoreIndex { obj, index, src } => {
                     for v in [obj, index, src] {
                        if let MIRValue::Variable { name, .. } = v {
                            if current_state.get(name) == Some(&VarState::Moved) {
                                self.error(format!("Borrow Error: Use of moved variable '{}'", name));
                            }
                        }
                    }
                }
            }
        }
    }
}
