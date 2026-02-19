use crate::mir::*;
use crate::diagnostics::Diagnostic;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
enum VarState {
    Uninitialized,
    Live,     // Owned, needs drop
    Borrowed, // Reference, does NOT need drop
    Moved,
    MaybeMoved, // For control flow merges where one path moves and another doesn't
    Error,
}

/// A reassignment drop: free the old value of a variable before it's overwritten.
/// (block_idx, instruction_idx_within_block, var_name)
pub type ReassignDrop = (usize, usize, String);

pub struct BorrowChecker {
    pub errors: Vec<Diagnostic>,
    pub filename: String,
}

impl BorrowChecker {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            filename: String::new(),
        }
    }

    pub fn check(&mut self, func: &MIRFunction, filename: &str) -> (HashMap<usize, Vec<String>>, Vec<ReassignDrop>) {
        self.filename = filename.to_string();
        let mut drops = HashMap::new();
        let mut reassignment_drops: Vec<ReassignDrop> = Vec::new();
        self.errors.clear();
        
        // 1. Initialize State Vectors for each block
        let num_blocks = func.blocks.len();
        if num_blocks == 0 { return (drops, reassignment_drops); }

        let mut in_states: Vec<HashMap<String, VarState>> = vec![HashMap::new(); num_blocks];
        let mut out_states: Vec<HashMap<String, VarState>> = vec![HashMap::new(); num_blocks];

        // Initialize entry block with arguments as Live
        let mut entry_state = HashMap::new();
        for param in &func.params {
             if let Some(ty) = func.variables.get(param) {
                 if ty.is_class() {
                      entry_state.insert(param.clone(), VarState::Live);
                 }
             }
        }
        // Initialize other local variables as Uninitialized
        for (var_name, ty) in &func.variables {
            if !entry_state.contains_key(var_name) && ty.needs_drop() {
                entry_state.insert(var_name.clone(), VarState::Uninitialized);
            }
        }
        in_states[func.entry_block] = entry_state.clone();

        // 2. Build Predecessors Map
        let predecessors = self.build_predecessors(func);

        // 3. Worklist Algorithm
        let mut worklist = VecDeque::new();
        for i in 0..num_blocks {
            worklist.push_back(i);
        }

        // To avoid infinite loops in case of malformed CFG or non-monotonic transfer (shouldn't happen with this lattice)
        // we can limit iterations or rely on monotonicity. The lattice height is small (4).
        
        let mut visit_count = vec![0; num_blocks];

        while let Some(block_idx) = worklist.pop_front() {
            visit_count[block_idx] += 1;
            if visit_count[block_idx] > num_blocks * 5 {
                // Safety break for cycles
                 continue; 
            }

            // Compute IN state: Join of predecessors' OUT states
            let current_in = if block_idx == func.entry_block {
                in_states[block_idx].clone() // Start with initialized entry state
            } else {
                 if let Some(preds) = predecessors.get(&block_idx) {
                     if preds.is_empty() {
                         HashMap::new()
                     } else {
                         // Start with the first predecessor's state (or default if unvisited)
                         // Note: We need to handle "Unvisited" effectively. 
                         // Standard allow: init to Bottom (Uninitialized/Error or specific Empty).
                         // Here we assume variables exist in map.
                         let mut joined = out_states[preds[0]].clone();
                         for &pred_idx in preds.iter().skip(1) {
                             joined = self.join_states(&joined, &out_states[pred_idx]);
                         }
                         joined
                     }
                 } else {
                     HashMap::new() // Unreachable block
                 }
            };
            
            // If IN state changed, we might need to process. But usually we check if OUT changes.
            // For the entry block, we merge with the initial entry state logic if we treated it as a predator loop? 
            // Actually, standard worklist:
            
            // Optimization: If current_in == in_states[block_idx] and we visited before, continue?
            // Only if out_states wouldn't change.
            // Let's just update IN.
            in_states[block_idx] = current_in.clone();

            // Compute OUT state by applying Transfer Function
            let old_out = out_states[block_idx].clone();
            let new_out = self.transfer_block(func, block_idx, &current_in);
            
            if new_out != old_out {
                out_states[block_idx] = new_out;
                // Add successors to worklist
                let successors = self.get_successors(func, block_idx);
                for succ in successors {
                    worklist.push_back(succ);
                }
            }
        }

        // 4. Final Pass: Report Errors & Collect Drops
        for (i, block_in) in in_states.iter().enumerate() {
             self.report_errors_in_block(func, i, block_in);
        }

        // Collect drops for leaf blocks
        for i in 0..num_blocks {
             let successors = self.get_successors(func, i);
             if successors.is_empty() {
                  // This is a leaf block (Return or Throw)
                  // Use variables from out_state
                   for (var_name, state) in &out_states[i] {
                        // EXEMPT ALL PARAMETERS: They are owned by the caller
                        // EXEMPT INTERNAL VARIABLES: Managed by compiler machinery
                        if *state == VarState::Live && !func.params.contains(var_name) {
                             if var_name.starts_with("__") {
                                 // Skipping internal variable
                             } else {
                                 drops.entry(i).or_insert(Vec::new()).push(var_name.clone());
                             }
                        }
                   }
             }
        }

        // 5. Collect reassignment drops: when a Live variable is overwritten,
        // the old value must be freed to prevent memory leaks.
        for (block_idx, block_in) in in_states.iter().enumerate() {
            self.collect_reassignment_drops(func, block_idx, block_in, &mut reassignment_drops);
        }

        (drops, reassignment_drops)
    }

    fn build_predecessors(&self, func: &MIRFunction) -> HashMap<usize, Vec<usize>> {
        let mut preds = HashMap::new();
        for (i, _) in func.blocks.iter().enumerate() {
            let succs = self.get_successors(func, i);
            for succ in succs {
                preds.entry(succ).or_insert(Vec::new()).push(i);
            }
        }
        preds
    }

    fn get_successors(&self, func: &MIRFunction, block_idx: usize) -> Vec<usize> {
        let block = &func.blocks[block_idx];
        let mut succs = Vec::new();
        if let Some(last) = block.instructions.last() {
            match last {
                MIRInstruction::Branch { true_target, false_target, .. } => {
                    succs.push(*true_target);
                    succs.push(*false_target);
                },
                MIRInstruction::Jump { target, .. } => {
                    succs.push(*target);
                },
                _ => {}
            }
        }
        // Exception handlers are implicit successors, simplified here.
        if let Some(handler) = block.exception_handler {
            succs.push(handler);
        }
        succs
    }

    fn join_states(&self, s1: &HashMap<String, VarState>, s2: &HashMap<String, VarState>) -> HashMap<String, VarState> {
        let mut result = HashMap::new();
        // Variables present in both
        for (k, v1) in s1 {
            if let Some(v2) = s2.get(k) {
                result.insert(k.clone(), self.join_val(*v1, *v2));
            } else {
                 // Present in 1 but not 2 (e.g. defined in one branch).
                 // Conservatively treated as MaybeMoved or Uninitialized depending on logic.
                 // If not in s2, it might be uninitialized there.
                 result.insert(k.clone(), VarState::Uninitialized);
            }
        }
        result
    }

    fn join_val(&self, v1: VarState, v2: VarState) -> VarState {
        match (v1, v2) {
            (VarState::Error, _) | (_, VarState::Error) => VarState::Error,
            (VarState::Live, VarState::Live) => VarState::Live,
            (VarState::Borrowed, VarState::Borrowed) => VarState::Borrowed,
            (VarState::Moved, VarState::Moved) => VarState::Moved,
            (VarState::Uninitialized, VarState::Uninitialized) => VarState::Uninitialized,
            
            (VarState::Live, VarState::Moved) | (VarState::Moved, VarState::Live) => VarState::MaybeMoved,
            (VarState::MaybeMoved, _) | (_, VarState::MaybeMoved) => VarState::MaybeMoved,

            (VarState::Live, VarState::Borrowed) | (VarState::Borrowed, VarState::Live) => VarState::Live, // Treat as Live for safety? Or maybe it's just Live.
            (VarState::Live, VarState::Uninitialized) | (VarState::Uninitialized, VarState::Live) => VarState::Uninitialized, // Or MaybeUninit
            (VarState::Moved, VarState::Uninitialized) | (VarState::Uninitialized, VarState::Moved) => VarState::MaybeMoved,
            _ => VarState::MaybeMoved,
        }
    }

    fn transfer_block(&mut self, func: &MIRFunction, block_idx: usize, in_state: &HashMap<String, VarState>) -> HashMap<String, VarState> {
        let mut state = in_state.clone();
        let block = &func.blocks[block_idx];

        for inst in &block.instructions {
            self.apply_instruction(func, inst, &mut state, false); // False = don't report errors
        }
        state
    }

    fn report_errors_in_block(&mut self, func: &MIRFunction, block_idx: usize, in_state: &HashMap<String, VarState>) {
        let mut state = in_state.clone();
        let block = &func.blocks[block_idx];
         for inst in &block.instructions {
            self.apply_instruction(func, inst, &mut state, true); // True = report errors
        }
    }

    /// Walks a block's instructions with the converged dataflow state,
    /// and detects when a `Live` variable is about to be overwritten.
    /// For each such case, records (block_idx, instruction_idx, var_name)
    /// so that a Free can be inserted before the overwriting instruction.
    fn collect_reassignment_drops(
        &self,
        func: &MIRFunction,
        block_idx: usize,
        in_state: &HashMap<String, VarState>,
        reassignment_drops: &mut Vec<ReassignDrop>,
    ) {
        let mut state = in_state.clone();
        let block = &func.blocks[block_idx];

        for (inst_idx, inst) in block.instructions.iter().enumerate() {
            // Check if this instruction writes to a dst that is currently Live
            let dst_name = match inst {
                MIRInstruction::Move { dst, .. } => Some(dst.clone()),
                MIRInstruction::Call { dst, .. } => Some(dst.clone()),
                MIRInstruction::IndirectCall { dst, .. } => Some(dst.clone()),
                MIRInstruction::ObjectLiteral { dst, .. } => Some(dst.clone()),
                MIRInstruction::ArrayLiteral { dst, .. } => Some(dst.clone()),
                MIRInstruction::BinaryOp { dst, .. } => Some(dst.clone()),
                MIRInstruction::Cast { dst, .. } => Some(dst.clone()),
                _ => None,
            };

            if let Some(ref name) = dst_name {
                // Only inject drop if:
                // 1. The variable is currently Live (holds an owned allocation)
                // 2. The variable is not a parameter (owned by caller)
                // 3. The variable is not an internal compiler variable
                // 4. The variable's type needs drop
                if let Some(&VarState::Live) = state.get(name) {
                    if !func.params.contains(name) && !name.starts_with("__") {
                        if let Some(ty) = func.variables.get(name) {
                            if ty.needs_drop() {
                                reassignment_drops.push((block_idx, inst_idx, name.clone()));
                            }
                        }
                    }
                }
            }

            // Apply the instruction to update state (same logic as transfer_block)
            // We clone enough to avoid double-borrow issues
            self.apply_instruction_readonly(func, inst, &mut state);
        }
    }

    /// Read-only version of apply_instruction that only updates state, no error reporting.
    fn apply_instruction_readonly(&self, func: &MIRFunction, inst: &MIRInstruction, state: &mut HashMap<String, VarState>) {
        match inst {
            MIRInstruction::Move { dst, src, .. } => {
                Self::mark_moved_static(src, state);
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::BinaryOp { dst, .. } => {
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::Call { dst, callee, args, .. } => {
                let is_constructor = callee.ends_with("_constructor");
                let is_method = callee.starts_with("f_") && callee.matches('_').count() >= 2;
                for (i, arg) in args.iter().enumerate() {
                    let is_receiver = i == 0 && (is_constructor || is_method);
                    if !is_receiver { Self::mark_moved_static(arg, state); }
                }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::Return { value, .. } => {
                if let Some(val) = value { Self::mark_moved_static(val, state); }
            },
            MIRInstruction::ObjectLiteral { dst, entries, .. } => {
                for (_, v) in entries { Self::mark_moved_static(v, state); }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::ArrayLiteral { dst, elements, .. } => {
                for v in elements { Self::mark_moved_static(v, state); }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::StoreMember { src, .. } => {
                Self::mark_moved_static(src, state);
            },
            MIRInstruction::StoreIndex { src, .. } => {
                Self::mark_moved_static(src, state);
            },
            MIRInstruction::Throw { value, .. } => {
                Self::mark_moved_static(value, state);
            },
            MIRInstruction::Cast { dst, src, .. } => {
                Self::mark_moved_static(src, state);
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::IndirectCall { dst, args, .. } => {
                for arg in args { Self::mark_moved_static(arg, state); }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Live); }
                }
            },
            MIRInstruction::LoadMember { dst, .. } => {
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Borrowed); }
                }
            },
            MIRInstruction::LoadIndex { dst, .. } => {
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() { state.insert(dst.clone(), VarState::Borrowed); }
                }
            },
            MIRInstruction::Free { value, .. } => {
                Self::mark_moved_static(value, state);
            },
            _ => {}
        }
    }

    /// Static version of mark_moved that doesn't need &mut self
    fn mark_moved_static(val: &MIRValue, state: &mut HashMap<String, VarState>) {
        if let MIRValue::Variable { name, .. } = val {
            if state.contains_key(name) {
                state.insert(name.clone(), VarState::Moved);
            }
        }
    }

    fn apply_instruction(&mut self, func: &MIRFunction, inst: &MIRInstruction, state: &mut HashMap<String, VarState>, report: bool) {
        match inst {
            MIRInstruction::Move { dst, src, line } => {
                self.check_use(src, state, report, *line);
                self.mark_moved(src, state);
                // Destination becomes Live if type needs drop
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                }
            },
            MIRInstruction::BinaryOp { dst, left, right, line, .. } => {
                self.check_use(left, state, report, *line);
                self.check_use(right, state, report, *line);
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                }
            },
            MIRInstruction::Call { dst, callee, args, line } => {
                let is_constructor = callee.ends_with("_constructor");
                let is_method = callee.starts_with("f_") && callee.matches('_').count() >= 2;
                
                for (i, arg) in args.iter().enumerate() {
                    self.check_use(arg, state, report, *line);
                    
                    // Argument consumption? 
                    // Constructors: 'this' (args[0]) is borrowed during init.
                    // Methods: 'this' (args[0]) is borrowed/shared.
                    // Regular Functions: All args are moved (if non-Copy, though we treat all as non-Copy for now).
                    let is_receiver = i == 0 && (is_constructor || is_method);
                    
                    if !is_receiver {
                        self.mark_moved(arg, state);
                    }
                }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                }
            },
            MIRInstruction::Return { value, line } => {
                 if let Some(val) = value {
                     self.check_use(val, state, report, *line);
                     self.mark_moved(val, state);
                 }
            },
            MIRInstruction::Branch { condition, line, .. } => {
                 self.check_use(condition, state, report, *line);
            },
            MIRInstruction::ObjectLiteral { dst, entries, line, .. } => {
                 for (_, v) in entries {
                      self.check_use(v, state, report, *line);
                      self.mark_moved(v, state);
                 }
                 if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                 }
            },
            MIRInstruction::ArrayLiteral { dst, elements, line, .. } => {
                for v in elements {
                    self.check_use(v, state, report, *line);
                    self.mark_moved(v, state);
                }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                }
            },
            MIRInstruction::LoadMember { obj, line, dst, .. } => {
                self.check_use(obj, state, report, *line);
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Borrowed);
                    }
                }
            },
            MIRInstruction::StoreMember { obj, src, line, .. } => {
                self.check_use(obj, state, report, *line);
                self.check_use(src, state, report, *line);
                self.mark_moved(src, state);
            },
            MIRInstruction::LoadIndex { obj, index, line, dst } => {
                self.check_use(obj, state, report, *line);
                self.check_use(index, state, report, *line);
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Borrowed);
                    }
                }
            },
            MIRInstruction::StoreIndex { obj, index, src, line } => {
                self.check_use(obj, state, report, *line);
                self.check_use(index, state, report, *line);
                self.check_use(src, state, report, *line);
                self.mark_moved(src, state);
            },
            MIRInstruction::Throw { value, line } => {
                self.check_use(value, state, report, *line);
                self.mark_moved(value, state);
            },
            MIRInstruction::Cast { dst, src, line, .. } => {
                self.check_use(src, state, report, *line);
                self.mark_moved(src, state);
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                }
            },
            MIRInstruction::IndirectCall { dst, callee, args, line } => {
                self.check_use(callee, state, report, *line);
                for arg in args {
                    self.check_use(arg, state, report, *line);
                    self.mark_moved(arg, state);
                }
                if let Some(ty) = func.variables.get(dst) {
                    if ty.needs_drop() {
                        state.insert(dst.clone(), VarState::Live);
                    }
                }
            },
            MIRInstruction::Jump { .. } => {},
            MIRInstruction::TrySetup { .. } => {},
            MIRInstruction::PopHandler { .. } => {},
            MIRInstruction::Free { value, line } => {
                self.check_use(value, state, report, *line);
                self.mark_moved(value, state);
            }
        }
    }

    fn check_use(&mut self, val: &MIRValue, state: &HashMap<String, VarState>, report: bool, line: usize) {
        if let MIRValue::Variable { name, .. } = val {
             if name.starts_with("__") { return; } // Skip internal compiler-generated variables
             if let Some(s) = state.get(name) {
                 match s {
                     VarState::Moved => {
                         if report {
                             self.errors.push(
                                 Diagnostic::new(format!("Use of moved variable '{}'", name), line, 1, self.filename.clone())
                                     .with_code("E0301")
                                     .with_hint("Value was moved to another variable; consider using `.clone()` or restructuring ownership")
                                     .with_label("value used after move")
                             );
                         }
                     },
                     VarState::MaybeMoved => {
                          if report {
                              self.errors.push(
                                  Diagnostic::new(format!("Use of possibly moved variable '{}'", name), line, 1, self.filename.clone())
                                      .with_code("E0302")
                                      .with_hint("Value may have been moved in a branch; ensure all control-flow paths keep the variable alive")
                                      .with_label("value may have been moved")
                              );
                          }
                     },
                     VarState::Uninitialized => {
                          // if report {
                          //     self.errors.push(
                          //         Diagnostic::new(format!("Use of uninitialized variable '{}'", name), line, 1, self.filename.clone())
                          //             .with_code("E0303")
                          //             .with_hint("Assign a value before use")
                          //     );
                          // }
                     },
                     _ => {}
                 }
             }
        }
    }

    fn mark_moved(&mut self, val: &MIRValue, state: &mut HashMap<String, VarState>) {
        if let MIRValue::Variable { name, ty } = val {
            if ty.needs_drop() && !ty.is_copyable() {
                state.insert(name.clone(), VarState::Moved);
            }
        }
    }
}
