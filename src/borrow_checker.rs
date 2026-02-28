use crate::diagnostics::Diagnostic;
use crate::mir::*;
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Copy)]

pub enum VarState {
    Uninitialized,
    Live,
    Moved,
}

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

    // Extracted helper
    fn get_def_use(inst: &MIRInstruction, func: &MIRFunction) -> (Vec<String>, Vec<String>) {
        let mut defs = Vec::new();
        let mut uses = Vec::new();

        let mut add_use = |val: &MIRValue| {
            if let MIRValue::Variable { name, ty } = val {
                if ty.needs_drop() && !name.starts_with("__") {
                    uses.push(name.clone());
                }
            }
        };

        let mut add_def = |dst: &String| {
            if let Some(ty) = func.variables.get(dst) {
                if ty.needs_drop() && !dst.starts_with("__") {
                    defs.push(dst.clone());
                }
            }
        };

        match inst {
            MIRInstruction::Move { dst, src, .. } => {
                add_use(src);
                add_def(dst);
            }
            MIRInstruction::BinaryOp {
                dst, left, right, ..
            } => {
                add_use(left);
                add_use(right);
                add_def(dst);
            }
            MIRInstruction::Branch { condition, .. } => {
                add_use(condition);
            }
            MIRInstruction::Return {
                value: Some(val), ..
            } => {
                add_use(val);
            }
            MIRInstruction::Call { dst, args, .. } => {
                for arg in args {
                    add_use(arg);
                }
                add_def(dst);
            }
            MIRInstruction::IndirectCall {
                dst, callee, args, ..
            } => {
                add_use(callee);
                for arg in args {
                    add_use(arg);
                }
                add_def(dst);
            }
            MIRInstruction::ObjectLiteral { dst, entries, .. } => {
                for (_, v) in entries {
                    add_use(v);
                }
                add_def(dst);
            }
            MIRInstruction::ArrayLiteral { dst, elements, .. } => {
                for v in elements {
                    add_use(v);
                }
                add_def(dst);
            }
            MIRInstruction::LoadMember { dst, obj, .. } => {
                add_use(obj);
                add_def(dst);
            }
            MIRInstruction::StoreMember { obj, src, .. } => {
                add_use(obj);
                add_use(src);
            }
            MIRInstruction::LoadIndex {
                dst, obj, index, ..
            } => {
                add_use(obj);
                add_use(index);
                add_def(dst);
            }
            MIRInstruction::StoreIndex {
                obj, index, src, ..
            } => {
                add_use(obj);
                add_use(index);
                add_use(src);
            }
            MIRInstruction::Throw { value, .. } => {
                add_use(value);
            }
            MIRInstruction::Cast { dst, src, .. } => {
                add_use(src);
                add_def(dst);
            }
            MIRInstruction::Free { value, .. } => {
                add_use(value);
            }
            _ => {}
        }

        (defs, uses)
    }

    pub fn check(
        &mut self,
        func: &MIRFunction,
        filename: &str,
    ) -> (
        HashMap<usize, Vec<String>>,
        Vec<ReassignDrop>,
        Vec<(usize, usize)>,
    ) {
        self.filename = filename.to_string();
        let mut drops: HashMap<usize, Vec<String>> = HashMap::new();
        let mut reassignment_drops: Vec<ReassignDrop> = Vec::new();
        let dead_frees = Vec::new(); // Unused in new model, left for signature compat
        self.errors.clear();

        let num_blocks = func.blocks.len();
        if num_blocks == 0 {
            return (drops, reassignment_drops, dead_frees);
        }

        // --- PHASE 1: Backward Liveness Analysis (Last-Use Detection) ---
        let mut block_succs: Vec<Vec<usize>> = vec![Vec::new(); num_blocks];
        for i in 0..num_blocks {
            block_succs[i] = self.get_successors(func, i);
        }

        let mut in_live: Vec<HashSet<String>> = vec![HashSet::new(); num_blocks];
        let mut out_live: Vec<HashSet<String>> = vec![HashSet::new(); num_blocks];

        let mut changed = true;
        while changed {
            changed = false;
            for i in (0..num_blocks).rev() {
                let old_in = in_live[i].clone();
                let mut cur_out: HashSet<String> = HashSet::new();
                for &succ in &block_succs[i] {
                    cur_out.extend(in_live[succ].iter().cloned());
                }
                out_live[i] = cur_out.clone();

                let mut cur_in = cur_out;
                for inst in func.blocks[i].instructions.iter().rev() {
                    let (defs, uses) = Self::get_def_use(inst, func);
                    for d in defs {
                        cur_in.remove(&d);
                    }
                    for u in uses {
                        cur_in.insert(u);
                    }
                }

                if cur_in != old_in {
                    in_live[i] = cur_in;
                    changed = true;
                }
            }
        }

        // Identify exact Last-Use instructions
        let mut last_uses = HashSet::new(); // (block_idx, inst_idx, var_name)
        for i in 0..num_blocks {
            let mut live_set = out_live[i].clone();
            for (inst_idx, inst) in func.blocks[i].instructions.iter().enumerate().rev() {
                let (defs, uses) = Self::get_def_use(inst, func);

                for d in &defs {
                    if !live_set.contains(d) && !func.params.contains(d) {
                        last_uses.insert((i, inst_idx, d.clone()));
                    }
                    live_set.remove(d);
                }

                for u in &uses {
                    if !live_set.contains(u) && !func.params.contains(u) {
                        last_uses.insert((i, inst_idx, u.clone()));
                        live_set.insert(u.clone());
                    }
                }
            }
        }

        // --- PHASE 2: Forward Pass & Drop Injection ---
        let predecessors = self.build_predecessors(func);
        let mut in_states: Vec<HashMap<String, VarState>> = vec![HashMap::new(); num_blocks];
        let mut out_states: Vec<HashMap<String, VarState>> = vec![HashMap::new(); num_blocks];

        let mut borrowed_vars = std::collections::HashSet::new();
        let mut changed = true;
        while changed {
            changed = false;
            for block in &func.blocks {
                for inst in &block.instructions {
                    match inst {
                        MIRInstruction::LoadMember { dst, borrow, .. } => {
                            if *borrow && borrowed_vars.insert(dst.clone()) {
                                changed = true;
                            }
                        }
                        MIRInstruction::LoadIndex { dst, borrow, .. } => {
                            if *borrow && borrowed_vars.insert(dst.clone()) {
                                changed = true;
                            }
                        }
                        MIRInstruction::Move { dst, src, .. } => {
                            if let MIRValue::Variable { name, .. } = src {
                                if borrowed_vars.contains(name) {
                                    if borrowed_vars.insert(dst.clone()) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let mut entry_state = HashMap::new();
        for param in &func.params {
            if let Some(ty) = func.variables.get(param) {
                if ty.is_class() {
                    entry_state.insert(param.clone(), VarState::Live);
                }
            }
        }
        for (var_name, ty) in &func.variables {
            if !entry_state.contains_key(var_name) && ty.needs_drop() {
                entry_state.insert(var_name.clone(), VarState::Uninitialized);
            }
        }
        in_states[func.entry_block] = entry_state.clone();

        let mut worklist = VecDeque::new();
        for i in 0..num_blocks {
            worklist.push_back(i);
        }

        while let Some(block_idx) = worklist.pop_front() {
            let current_in = if block_idx == func.entry_block {
                in_states[block_idx].clone()
            } else {
                if let Some(preds) = predecessors.get(&block_idx) {
                    if preds.is_empty() {
                        HashMap::new()
                    } else {
                        let mut joined = out_states[preds[0]].clone();
                        for &pred_idx in preds.iter().skip(1) {
                            joined = self.join_states(&joined, &out_states[pred_idx]);
                        }
                        joined
                    }
                } else {
                    HashMap::new()
                }
            };

            in_states[block_idx] = current_in.clone();
            let old_out = out_states[block_idx].clone();

            let mut state = current_in.clone();
            for (inst_idx, inst) in func.blocks[block_idx].instructions.iter().enumerate() {
                let (defs, uses) = Self::get_def_use(inst, func);
                for u in &uses {
                    if last_uses.contains(&(block_idx, inst_idx, u.clone())) {
                        state.insert(u.clone(), VarState::Moved);
                    }
                }
                for d in &defs {
                    if let Some(ty) = func.variables.get(d) {
                        if ty.needs_drop() {
                            state.insert(d.clone(), VarState::Live);
                        }
                    }
                }
                for def_var in &defs {
                    if last_uses.contains(&(block_idx, inst_idx, def_var.clone())) {
                        state.insert(def_var.clone(), VarState::Moved);
                    }
                }
            }

            let new_out = state;
            if new_out != old_out {
                out_states[block_idx] = new_out;
                for succ in self.get_successors(func, block_idx) {
                    worklist.push_back(succ);
                }
            }
        }

        for (block_idx, block_in) in in_states.iter().enumerate() {
            let mut state = block_in.clone();
            let block = &func.blocks[block_idx];

            for (inst_idx, inst) in block.instructions.iter().enumerate() {
                let (defs, uses) = Self::get_def_use(inst, func);

                // Reassignment Drops
                for dst_name in &defs {
                    if let Some(&VarState::Live) = state.get(dst_name) {
                        if !func.params.contains(dst_name)
                            && !dst_name.starts_with("__")
                            && !dst_name.starts_with("g_")
                            && !borrowed_vars.contains(dst_name)
                        {
                            reassignment_drops.push((block_idx, inst_idx, dst_name.clone()));
                        }
                    }
                    state.insert(dst_name.clone(), VarState::Live);
                }

                // Last-Use Drops (Implicit Moves)
                let is_terminator = matches!(
                    inst,
                    MIRInstruction::Return { .. }
                        | MIRInstruction::Throw { .. }
                        | MIRInstruction::Branch { .. }
                        | MIRInstruction::Jump { .. }
                );
                let is_returned = |var: &String| -> bool {
                    matches!(inst, MIRInstruction::Return { value: Some(MIRValue::Variable { name, .. }), .. } if name == var)
                        || matches!(inst, MIRInstruction::Throw { value: MIRValue::Variable { name, .. }, .. } if name == var)
                };
                let is_moved = |var: &String| -> bool {
                    match inst {
                        MIRInstruction::Move {
                            src: MIRValue::Variable { name, .. },
                            ..
                        } => name == var,
                        // Call args: ownership is transferred to the callee.
                        // The caller must NOT Free them — the callee manages their lifetime.
                        MIRInstruction::Call { args, .. } => args.iter().any(|v| {
                            if let MIRValue::Variable { name, .. } = v {
                                name == var
                            } else {
                                false
                            }
                        }),
                        MIRInstruction::IndirectCall { args, .. } => args.iter().any(|v| {
                            if let MIRValue::Variable { name, .. } = v {
                                name == var
                            } else {
                                false
                            }
                        }),
                        MIRInstruction::ArrayLiteral { elements, .. } => elements.iter().any(|v| {
                            if let MIRValue::Variable { name, .. } = v {
                                name == var
                            } else {
                                false
                            }
                        }),
                        MIRInstruction::ObjectLiteral { entries, .. } => {
                            entries.iter().any(|(_, v)| {
                                if let MIRValue::Variable { name, .. } = v {
                                    name == var
                                } else {
                                    false
                                }
                            })
                        }
                        MIRInstruction::StoreMember {
                            src: MIRValue::Variable { name, .. },
                            ..
                        } => name == var,
                        MIRInstruction::StoreIndex {
                            src: MIRValue::Variable { name, .. },
                            ..
                        } => name == var,
                        // LoadMember/LoadIndex obj: accessing a member/index borrows the parent.
                        // The parent must not be freed at this point — the loaded member
                        // may reference memory inside the parent's allocation.
                        MIRInstruction::LoadMember {
                            obj: MIRValue::Variable { name, .. },
                            ..
                        } => name == var,
                        MIRInstruction::LoadIndex {
                            obj: MIRValue::Variable { name, .. },
                            ..
                        } => name == var,
                        _ => false,
                    }
                };

                for used_var in &uses {
                    if last_uses.contains(&(block_idx, inst_idx, used_var.clone())) {
                        if let Some(&VarState::Live) = state.get(used_var) {
                            let drop_idx = if is_terminator {
                                inst_idx
                            } else {
                                inst_idx + 1
                            };
                            if !is_returned(used_var)
                                && !is_moved(used_var)
                                && !used_var.starts_with("g_")
                                && !borrowed_vars.contains(used_var)
                            {
                                reassignment_drops.push((block_idx, drop_idx, used_var.clone()));
                            }
                            state.insert(used_var.clone(), VarState::Moved);
                        }
                    }
                }

                // Dead Stores (Definition never used)
                for def_var in &defs {
                    if last_uses.contains(&(block_idx, inst_idx, def_var.clone())) {
                        let drop_idx = if is_terminator {
                            inst_idx
                        } else {
                            inst_idx + 1
                        };
                        if !borrowed_vars.contains(def_var) && !def_var.starts_with("g_") {
                            reassignment_drops.push((block_idx, drop_idx, def_var.clone()));
                        }
                        state.insert(def_var.clone(), VarState::Moved);
                    }
                }
            }
        }

        for i in 0..num_blocks {
            if self.get_successors(func, i).is_empty() {
                for (var_name, state) in &out_states[i] {
                    if *state == VarState::Live
                        && !func.params.contains(var_name)
                        && !var_name.starts_with("__")
                        && !var_name.starts_with("g_")
                        && !borrowed_vars.contains(var_name)
                    {
                        drops.entry(i).or_insert(Vec::new()).push(var_name.clone());
                    }
                }
            }
        }

        (drops, reassignment_drops, dead_frees)
    }

    fn build_predecessors(&self, func: &MIRFunction) -> HashMap<usize, Vec<usize>> {
        let mut preds = HashMap::new();
        for (i, _) in func.blocks.iter().enumerate() {
            for succ in self.get_successors(func, i) {
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
                MIRInstruction::Branch {
                    true_target,
                    false_target,
                    ..
                } => {
                    succs.push(*true_target);
                    succs.push(*false_target);
                }
                MIRInstruction::TrySetup {
                    try_target,
                    _catch_target,
                    ..
                } => {
                    succs.push(*try_target);
                    succs.push(*_catch_target);
                }
                MIRInstruction::Jump { target, .. } => succs.push(*target),
                _ => {}
            }
        }
        if let Some(handler) = block.exception_handler {
            succs.push(handler);
        }
        succs
    }

    fn join_states(
        &self,
        s1: &HashMap<String, VarState>,
        s2: &HashMap<String, VarState>,
    ) -> HashMap<String, VarState> {
        let mut res = HashMap::new();
        for (k, v1) in s1 {
            let v2 = s2.get(k).unwrap_or(&VarState::Uninitialized);
            res.insert(k.clone(), self.join_val(*v1, *v2));
        }
        res
    }

    fn join_val(&self, v1: VarState, v2: VarState) -> VarState {
        if v1 == VarState::Live && v2 == VarState::Live {
            return VarState::Live;
        }
        if v1 == VarState::Uninitialized && v2 == VarState::Uninitialized {
            return VarState::Uninitialized;
        }
        VarState::Moved // Default merge
    }
}
