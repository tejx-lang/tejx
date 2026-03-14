use crate::middle::mir::{MIRFunction, MIRInstruction, MIRValue};
use crate::common::types::TejxType;

pub struct MIROptimizer {}

impl MIROptimizer {
    pub fn new() -> Self {
        Self {}
    }

    pub fn optimize(&self, func: &mut MIRFunction) {
        let mut changed = true;
        while changed {
            changed = false;
            changed |= self.fold_constants(func);
            changed |= self.eliminate_dead_code(func);
            changed |= self.remove_unused_variables(func);
        }
    }

    fn fold_constants(&self, func: &mut MIRFunction) -> bool {
        let mut changed = false;
        
        for block in &mut func.blocks {
            for i in 0..block.instructions.len() {
                if let MIRInstruction::BinaryOp { dst, left, op, right, op_width: _, line } = &block.instructions[i] {
                    if let (MIRValue::Constant { value: l_val, ty: l_ty }, MIRValue::Constant { value: r_val, .. }) = (left, right) {
                        if l_ty.is_numeric() {
                            if let (Ok(l_num), Ok(r_num)) = (l_val.parse::<i64>(), r_val.parse::<i64>()) {
                                let result = match op {
                                    crate::frontend::token::TokenType::Plus => Some(l_num + r_num),
                                    crate::frontend::token::TokenType::Minus => Some(l_num - r_num),
                                    crate::frontend::token::TokenType::Star => Some(l_num * r_num),
                                    crate::frontend::token::TokenType::Slash if r_num != 0 => Some(l_num / r_num),
                                    crate::frontend::token::TokenType::Modulo if r_num != 0 => Some(l_num % r_num),
                                    _ => None,
                                };
                                
                                if let Some(res) = result {
                                    block.instructions[i] = MIRInstruction::Move {
                                        dst: dst.clone(),
                                        src: MIRValue::Constant {
                                            value: res.to_string(),
                                            ty: l_ty.clone(),
                                        },
                                        line: *line,
                                    };
                                    changed = true;
                                }
                            } else if let (Ok(l_num), Ok(r_num)) = (l_val.parse::<f64>(), r_val.parse::<f64>()) {
                                let result = match op {
                                    crate::frontend::token::TokenType::Plus => Some(l_num + r_num),
                                    crate::frontend::token::TokenType::Minus => Some(l_num - r_num),
                                    crate::frontend::token::TokenType::Star => Some(l_num * r_num),
                                    crate::frontend::token::TokenType::Slash if r_num != 0.0 => Some(l_num / r_num),
                                    _ => None,
                                };
                                
                                if let Some(res) = result {
                                    block.instructions[i] = MIRInstruction::Move {
                                        dst: dst.clone(),
                                        src: MIRValue::Constant {
                                            value: res.to_string(),
                                            ty: l_ty.clone(),
                                        },
                                        line: *line,
                                    };
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            }
        }
        
        changed
    }

    fn eliminate_dead_code(&self, func: &mut MIRFunction) -> bool {
        let mut changed = false;
        
        // Remove instructions after a terminator (Return, Jump, Branch, Throw) within the same block
        for block in &mut func.blocks {
            let mut terminator_idx = None;
            for (i, instr) in block.instructions.iter().enumerate() {
                if matches!(
                    instr,
                    MIRInstruction::Return { .. }
                        | MIRInstruction::Jump { .. }
                        | MIRInstruction::Branch { .. }
                        | MIRInstruction::Throw { .. }
                ) {
                    terminator_idx = Some(i);
                    break;
                }
            }
            
            if let Some(idx) = terminator_idx {
                if idx + 1 < block.instructions.len() {
                    block.instructions.truncate(idx + 1);
                    changed = true;
                }
            }
        }
        
        changed
    }
    
    fn remove_unused_variables(&self, func: &mut MIRFunction) -> bool {
        let mut changed = false;
        
        // 1. Collect all variable usages (reads and terminator usages)
        let mut used_vars = std::collections::HashSet::new();
        for block in &func.blocks {
            for instr in &block.instructions {
                match instr {
                    MIRInstruction::Move { src, .. } => {
                        if let MIRValue::Variable { name, .. } = src {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::BinaryOp { left, right, op_width: _, .. } => {
                        if let MIRValue::Variable { name, .. } = left {
                            used_vars.insert(name.clone());
                        }
                        if let MIRValue::Variable { name, .. } = right {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::Branch { condition, .. } => {
                        if let MIRValue::Variable { name, .. } = condition {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::Return { value: Some(val), .. } => {
                        if let MIRValue::Variable { name, .. } = val {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::Call { args, .. } | MIRInstruction::IndirectCall { args, .. } => {
                        for arg in args {
                            if let MIRValue::Variable { name, .. } = arg {
                                used_vars.insert(name.clone());
                            }
                        }
                        if let MIRInstruction::IndirectCall { callee: MIRValue::Variable { name, .. }, .. } = instr {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::LoadMember { obj, .. } => {
                        if let MIRValue::Variable { name, .. } = obj {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::StoreMember { obj, src, .. } => {
                        if let MIRValue::Variable { name, .. } = obj {
                            used_vars.insert(name.clone());
                        }
                        if let MIRValue::Variable { name, .. } = src {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::LoadIndex { obj, index, element_ty: _, .. } => {
                        if let MIRValue::Variable { name, .. } = obj {
                            used_vars.insert(name.clone());
                        }
                        if let MIRValue::Variable { name, .. } = index {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::StoreIndex { obj, index, src, element_ty: _, .. } => {
                        if let MIRValue::Variable { name, .. } = obj {
                            used_vars.insert(name.clone());
                        }
                        if let MIRValue::Variable { name, .. } = index {
                            used_vars.insert(name.clone());
                        }
                        if let MIRValue::Variable { name, .. } = src {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::Throw { value, .. } => {
                        if let MIRValue::Variable { name, .. } = value {
                            used_vars.insert(name.clone());
                        }
                    }
                    MIRInstruction::Cast { src, .. } => {
                        if let MIRValue::Variable { name, .. } = src {
                            used_vars.insert(name.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        
        // 2. Remove instructions that store to unused variables (except for instructions with side-effects like Call)
        for block in &mut func.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                let mut remove = false;
                match &block.instructions[i] {
                    MIRInstruction::Move { dst, .. }
                    | MIRInstruction::BinaryOp { dst, .. }
                    | MIRInstruction::LoadMember { dst, .. }
                    | MIRInstruction::LoadIndex { dst, .. }
                    | MIRInstruction::Cast { dst, .. } => {
                        if !used_vars.contains(dst) && !func.variables.contains_key(dst) && !func.params.contains(dst) {
                            // Be careful: variables mapping might track source maps, but temporaries usually start with % or _t
                            // If it's pure logic and not in used_vars, we can eliminate it unless it has side effects.
                            // Only safely remove temporaries
                            if dst.starts_with("%") || dst.starts_with("_t") {
                                remove = true;
                            }
                        }
                    }
                    MIRInstruction::Call { dst: _, .. } | MIRInstruction::IndirectCall { dst: _, .. } => {
                        // We CANNOT remove calls because they might have side effects, 
                        // even if the result `dst` is unused. Just let it be.
                    }
                    _ => {}
                }
                
                if remove {
                    block.instructions.remove(i);
                    changed = true;
                } else {
                    i += 1;
                }
            }
        }
        
        changed
    }
}
