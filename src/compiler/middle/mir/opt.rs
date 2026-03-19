use crate::common::types::TejxType;
use crate::frontend::token::TokenType;
use crate::middle::mir::{MIRFunction, MIRInstruction, MIRValue};
use std::collections::{HashMap, HashSet};

pub struct MIROptimizer {}

impl Default for MIROptimizer {
    fn default() -> Self {
        Self::new()
    }
}

impl MIROptimizer {
    pub fn new() -> Self {
        Self {}
    }

    pub fn optimize(&self, func: &mut MIRFunction) {
        let mut changed = true;
        while changed {
            changed = false;
            changed |= self.promote_local_constant_arrays(func);
            changed |= self.rewrite_local_string_appends(func);
            changed |= self.fold_constants(func);
            changed |= self.eliminate_dead_code(func);
            changed |= self.remove_unused_variables(func);
        }
    }

    fn promote_local_constant_arrays(&self, func: &mut MIRFunction) -> bool {
        let mut promotions: Vec<(HashSet<String>, TejxType)> = Vec::new();

        for block in &func.blocks {
            for inst in &block.instructions {
                let (dst, len) = match inst {
                    MIRInstruction::Call { dst, callee, args, .. }
                        if callee == "rt_Array_constructor_v2" && args.len() >= 2 =>
                    {
                        let len = match &args[1] {
                            MIRValue::Constant { value, .. } => value.parse::<usize>().ok(),
                            _ => None,
                        };
                        match len {
                            Some(len) if len > 0 => (dst.clone(), len),
                            _ => continue,
                        }
                    }
                    _ => continue,
                };

                let inner_ty = match func.variables.get(&dst) {
                    Some(TejxType::DynamicArray(inner)) => (**inner).clone(),
                    _ => continue,
                };

                if let Some(aliases) = self.collect_array_aliases(func, &dst, len) {
                    let new_ty = TejxType::FixedArray(Box::new(inner_ty), len);
                    promotions.push((aliases, new_ty));
                }
            }
        }

        if promotions.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut renamed_types = HashMap::new();
        for (aliases, new_ty) in &promotions {
            for name in aliases {
                if func.variables.get(name) != Some(new_ty) {
                    func.variables.insert(name.clone(), new_ty.clone());
                    renamed_types.insert(name.clone(), new_ty.clone());
                    changed = true;
                }
            }
        }

        if !changed {
            return false;
        }

        for block in &mut func.blocks {
            for inst in &mut block.instructions {
                self.rewrite_instruction_types(inst, &renamed_types);
            }
        }

        true
    }

    fn collect_array_aliases(
        &self,
        func: &MIRFunction,
        root: &str,
        len: usize,
    ) -> Option<HashSet<String>> {
        let mut aliases = HashSet::from([root.to_string()]);
        let mut changed = true;
        while changed {
            changed = false;
            for block in &func.blocks {
                for inst in &block.instructions {
                    if let MIRInstruction::Move { dst, src: MIRValue::Variable { name, .. }, .. } = inst {
                        if aliases.contains(name) && !aliases.contains(dst) {
                            aliases.insert(dst.clone());
                            changed = true;
                        }
                    }
                }
            }
        }

        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    MIRInstruction::Move { dst, src: MIRValue::Variable { name, .. }, .. } => {
                        if aliases.contains(name) {
                            let dst_ty = func.variables.get(dst);
                            if matches!(dst_ty, Some(TejxType::DynamicArray(_)) | Some(TejxType::FixedArray(_, _))) {
                                continue;
                            }
                            return None;
                        }
                        if aliases.contains(dst) && !aliases.contains(name) {
                            return None;
                        }
                    }
                    MIRInstruction::LoadIndex { obj: MIRValue::Variable { name, .. }, index, .. }
                    | MIRInstruction::StoreIndex { obj: MIRValue::Variable { name, .. }, index, .. }
                        if aliases.contains(name) =>
                    {
                        if let MIRValue::Constant { value, .. } = index {
                            if let Ok(idx) = value.parse::<i64>() {
                                if idx < 0 || idx as usize >= len {
                                    return None;
                                }
                            }
                        }
                    }
                    MIRInstruction::Call { args, .. } | MIRInstruction::IndirectCall { args, .. } => {
                        for arg in args {
                            if let MIRValue::Variable { name, .. } = arg {
                                if aliases.contains(name) {
                                    return None;
                                }
                            }
                        }
                    }
                    MIRInstruction::Return { value: Some(MIRValue::Variable { name, .. }), .. }
                    | MIRInstruction::LoadMember { obj: MIRValue::Variable { name, .. }, .. }
                    | MIRInstruction::StoreMember { obj: MIRValue::Variable { name, .. }, .. }
                    | MIRInstruction::StoreMember { src: MIRValue::Variable { name, .. }, .. }
                    | MIRInstruction::Cast { src: MIRValue::Variable { name, .. }, .. }
                    | MIRInstruction::Branch { condition: MIRValue::Variable { name, .. }, .. }
                        if aliases.contains(name) =>
                    {
                        return None;
                    }
                    MIRInstruction::BinaryOp { left, right, .. } => {
                        if matches!(left, MIRValue::Variable { name, .. } if aliases.contains(name))
                            || matches!(right, MIRValue::Variable { name, .. } if aliases.contains(name))
                        {
                            return None;
                        }
                    }
                    _ => {}
                }
            }
        }

        Some(aliases)
    }

    fn rewrite_value_types(&self, value: &mut MIRValue, renamed_types: &HashMap<String, TejxType>) {
        if let MIRValue::Variable { name, ty } = value {
            if let Some(new_ty) = renamed_types.get(name) {
                *ty = new_ty.clone();
            }
        }
    }

    fn rewrite_instruction_types(
        &self,
        inst: &mut MIRInstruction,
        renamed_types: &HashMap<String, TejxType>,
    ) {
        match inst {
            MIRInstruction::Move { src, .. }
            | MIRInstruction::Return { value: Some(src), .. }
            | MIRInstruction::Throw { value: src, .. }
            | MIRInstruction::Branch { condition: src, .. }
            | MIRInstruction::Cast { src, .. } => self.rewrite_value_types(src, renamed_types),
            MIRInstruction::BinaryOp { left, right, .. } => {
                self.rewrite_value_types(left, renamed_types);
                self.rewrite_value_types(right, renamed_types);
            }
            MIRInstruction::Call { args, .. } | MIRInstruction::IndirectCall { args, .. } => {
                for arg in args {
                    self.rewrite_value_types(arg, renamed_types);
                }
            }
            MIRInstruction::LoadMember { obj, .. } => self.rewrite_value_types(obj, renamed_types),
            MIRInstruction::StoreMember { obj, src, .. } => {
                self.rewrite_value_types(obj, renamed_types);
                self.rewrite_value_types(src, renamed_types);
            }
            MIRInstruction::LoadIndex { obj, index, .. } => {
                self.rewrite_value_types(obj, renamed_types);
                self.rewrite_value_types(index, renamed_types);
            }
            MIRInstruction::StoreIndex { obj, index, src, .. } => {
                self.rewrite_value_types(obj, renamed_types);
                self.rewrite_value_types(index, renamed_types);
                self.rewrite_value_types(src, renamed_types);
            }
            MIRInstruction::Return { value: None, .. }
            | MIRInstruction::Jump { .. }
            | MIRInstruction::PopHandler { .. }
            | MIRInstruction::TrySetup { .. } => {}
        }
    }

    fn rewrite_local_string_appends(&self, func: &mut MIRFunction) -> bool {
        let mut candidates = HashSet::new();
        for (name, ty) in &func.variables {
            if matches!(ty, TejxType::String) {
                candidates.insert(name.clone());
            }
        }

        candidates.retain(|name| self.can_use_local_string_append(func, name));
        if candidates.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut assigned_so_far = HashSet::new();
        for block in &mut func.blocks {
            let mut i = 0;
            while i < block.instructions.len() {
                if let MIRInstruction::Move {
                    dst,
                    src: MIRValue::Constant { value, ty: TejxType::String },
                    line,
                } = &block.instructions[i]
                {
                    if value.is_empty()
                        && candidates.contains(dst)
                        && assigned_so_far.contains(dst)
                    {
                        block.instructions[i] = MIRInstruction::Call {
                            dst: dst.clone(),
                            callee: "rt_str_clear_local".to_string(),
                            args: vec![MIRValue::Variable {
                                name: dst.clone(),
                                ty: TejxType::String,
                            }],
                            line: *line,
                        };
                        changed = true;
                        i += 1;
                        continue;
                    }
                }

                match &block.instructions[i] {
                    MIRInstruction::Move { dst, .. }
                    | MIRInstruction::BinaryOp { dst, .. }
                    | MIRInstruction::Call { dst, .. }
                    | MIRInstruction::IndirectCall { dst, .. }
                    | MIRInstruction::LoadMember { dst, .. }
                    | MIRInstruction::LoadIndex { dst, .. }
                    | MIRInstruction::Cast { dst, .. } => {
                        if !dst.is_empty() {
                            assigned_so_far.insert(dst.clone());
                        }
                    }
                    _ => {}
                }

                if i + 1 >= block.instructions.len() {
                    break;
                }

                let replacement = match (&block.instructions[i], &block.instructions[i + 1]) {
                    (
                        MIRInstruction::BinaryOp {
                            dst,
                            left: MIRValue::Variable { name: left_name, ty: left_ty },
                            op,
                            right,
                            op_width,
                            line,
                        },
                        MIRInstruction::Move {
                            dst: move_dst,
                            src: MIRValue::Variable { name: src_name, .. },
                            ..
                        },
                    ) if dst == src_name
                        && move_dst == left_name
                        && matches!(left_ty, TejxType::String)
                        && matches!(right.get_type(), TejxType::String)
                        && matches!(op_width, TejxType::String)
                        && matches!(op, TokenType::Plus)
                        && candidates.contains(left_name) =>
                    {
                        Some(MIRInstruction::Call {
                            dst: dst.clone(),
                            callee: "rt_str_append_local".to_string(),
                            args: vec![
                                MIRValue::Variable {
                                    name: left_name.clone(),
                                    ty: TejxType::String,
                                },
                                right.clone(),
                            ],
                            line: *line,
                        })
                    }
                    _ => None,
                };

                if let Some(new_inst) = replacement {
                    block.instructions[i] = new_inst;
                    changed = true;
                }
                i += 1;
            }
        }

        changed
    }

    fn can_use_local_string_append(&self, func: &MIRFunction, name: &str) -> bool {
        for block in &func.blocks {
            for inst in &block.instructions {
                match inst {
                    MIRInstruction::Move { dst, src, .. } => {
                        if let MIRValue::Variable { name: src_name, .. } = src {
                            if src_name == name && dst != name {
                                return false;
                            }
                        }
                    }
                    MIRInstruction::Return { value: Some(MIRValue::Variable { name: ret_name, .. }), .. }
                        if ret_name == name =>
                    {
                        return false;
                    }
                    MIRInstruction::Call { callee, args, .. } => {
                        for arg in args {
                            if let MIRValue::Variable { name: arg_name, .. } = arg {
                                if arg_name == name
                                    && callee != "rt_len"
                                    && callee != "rt_strlen"
                                {
                                    return false;
                                }
                            }
                        }
                    }
                    MIRInstruction::IndirectCall { args, .. } => {
                        for arg in args {
                            if let MIRValue::Variable { name: arg_name, .. } = arg {
                                if arg_name == name {
                                    return false;
                                }
                            }
                        }
                    }
                    MIRInstruction::StoreMember { src: MIRValue::Variable { name: src_name, .. }, .. }
                    | MIRInstruction::StoreIndex { src: MIRValue::Variable { name: src_name, .. }, .. }
                        if src_name == name =>
                    {
                        return false;
                    }
                    MIRInstruction::LoadMember { obj: MIRValue::Variable { name: obj_name, .. }, .. }
                    | MIRInstruction::LoadIndex { obj: MIRValue::Variable { name: obj_name, .. }, .. }
                    | MIRInstruction::StoreIndex { obj: MIRValue::Variable { name: obj_name, .. }, .. }
                        if obj_name == name =>
                    {
                        return false;
                    }
                    MIRInstruction::Cast { src: MIRValue::Variable { name: src_name, .. }, .. }
                        if src_name == name =>
                    {
                        return false;
                    }
                    _ => {}
                }
            }
        }
        true
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
