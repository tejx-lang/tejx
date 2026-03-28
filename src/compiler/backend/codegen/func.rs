use super::*;
use crate::common::intrinsics::*;
use crate::common::types::TejxType;
use crate::frontend::token::TokenType;
use crate::middle::mir::*;
use std::collections::{HashMap, HashSet};

impl CodeGen {
    fn function_tracks_runtime_location(function_name: &str) -> bool {
        function_name != "tejx_main"
            && function_name != "rt_main_async_worker"
            && function_name != "f_async_add"
    }

    pub(crate) fn known_non_throwing_call_target(callee: &str) -> bool {
        callee.starts_with("llvm.")
            || callee.starts_with("std_math_")
            || matches!(
                callee,
                "rt_time_now_ms"
                    | "rt_random"
                    | "rt_random_int"
                    | "rt_random_seed"
                    | "rt_len"
                    | "rt_strlen"
                    | "rt_is_array"
                    | "rt_string_from_c_str"
                    | "rt_string_from_c_str_const"
                    | "rt_str_concat_v2"
                    | "rt_str_append_local"
                    | "rt_str_clear_local"
            )
    }

    fn function_has_direct_runtime_exception_edge(
        func: &MIRFunction,
        known_functions: &HashSet<String>,
        extern_functions: &HashSet<String>,
    ) -> bool {
        func.blocks.iter().flat_map(|bb| bb.instructions.iter()).any(|inst| {
            match inst {
                MIRInstruction::Throw { .. }
                | MIRInstruction::IndirectCall { .. }
                | MIRInstruction::LoadMember { .. }
                | MIRInstruction::StoreMember { .. }
                | MIRInstruction::LoadIndex { .. }
                | MIRInstruction::StoreIndex { .. } => true,
                MIRInstruction::BinaryOp { op, .. } => {
                    matches!(op, TokenType::Slash | TokenType::Modulo)
                }
                MIRInstruction::Call { callee, .. } => {
                    !Self::known_non_throwing_call_target(callee)
                        && (extern_functions.contains(callee) || !known_functions.contains(callee))
                }
                MIRInstruction::Move { .. }
                | MIRInstruction::Branch { .. }
                | MIRInstruction::Jump { .. }
                | MIRInstruction::Return { .. }
                | MIRInstruction::Cast { .. }
                | MIRInstruction::TrySetup { .. }
                | MIRInstruction::PopHandler { .. } => false,
            }
        })
    }

    fn compute_tracked_runtime_functions(
        functions: &[&MIRFunction],
    ) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
        let known_functions: HashSet<String> =
            functions.iter().map(|func| func.name.clone()).collect();
        let extern_functions: HashSet<String> = functions
            .iter()
            .filter(|func| func.is_extern)
            .map(|func| func.name.clone())
            .collect();

        let mut tracked_functions = HashSet::new();
        for func in functions {
            if Self::function_has_direct_runtime_exception_edge(
                func,
                &known_functions,
                &extern_functions,
            ) {
                tracked_functions.insert(func.name.clone());
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            for func in functions {
                if func.is_extern || tracked_functions.contains(&func.name) {
                    continue;
                }

                let propagates_tracked_exception =
                    func.blocks.iter().flat_map(|bb| bb.instructions.iter()).any(|inst| {
                        match inst {
                            MIRInstruction::Call { callee, .. } => {
                                !Self::known_non_throwing_call_target(callee)
                                    && (tracked_functions.contains(callee)
                                    || extern_functions.contains(callee)
                                    || !known_functions.contains(callee))
                            }
                            MIRInstruction::IndirectCall { .. } => true,
                            _ => false,
                        }
                    });

                if propagates_tracked_exception {
                    tracked_functions.insert(func.name.clone());
                    changed = true;
                }
            }
        }

        (tracked_functions, known_functions, extern_functions)
    }

    fn stringify_field_kind(ty: &TejxType) -> u8 {
        match ty {
            TejxType::String
            | TejxType::Class(_, _)
            | TejxType::Optional(_)
            | TejxType::DynamicArray(_)
            | TejxType::Object(_)
            | TejxType::Any => 0,
            TejxType::Bool => 1,
            TejxType::Int16 => 2,
            TejxType::Int32 => 3,
            TejxType::Int64 => 4,
            TejxType::Float32 => 5,
            TejxType::Float64 => 6,
            TejxType::Char => 7,
            _ => 255,
        }
    }

    fn is_null_like_value(value: &MIRValue) -> bool {
        matches!(value, MIRValue::Constant { value, .. } if value == "0")
    }

    fn fixed_layout_field_key(ty: &TejxType) -> String {
        if Self::fixed_layout_object_type(ty).is_some() {
            return "__fixed_object_ref".to_string();
        }

        match ty {
            TejxType::FixedArray(inner, len) => {
                format!("[{};{}]", Self::fixed_layout_field_key(inner), len)
            }
            TejxType::DynamicArray(inner) => {
                format!("Array<{}>", Self::fixed_layout_field_key(inner))
            }
            TejxType::Slice(inner) => format!("slice<{}>", Self::fixed_layout_field_key(inner)),
            TejxType::Optional(inner) => {
                format!("Optional<{}>", Self::fixed_layout_field_key(inner))
            }
            TejxType::Function(params, ret) => {
                let params = params
                    .iter()
                    .map(Self::fixed_layout_field_key)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("fn({})->{}", params, Self::fixed_layout_field_key(ret))
            }
            _ => ty.to_name(),
        }
    }

    fn fixed_layout_shape_key(ty: &TejxType) -> Option<String> {
        match Self::fixed_layout_object_type(ty)? {
            TejxType::Object(props) => Some(
                props
                    .iter()
                    .map(|(name, _, field_ty)| {
                        format!("{}:{}", name, Self::fixed_layout_field_key(field_ty))
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            _ => None,
        }
    }

    fn array_element_fixed_layout_shape_key(ty: &TejxType) -> Option<String> {
        if !ty.is_array() {
            return None;
        }
        Self::fixed_layout_shape_key(&ty.get_array_element_type())
    }

    fn register_object_shape_type(&mut self, ty: &TejxType) {
        match ty {
            TejxType::Object(props) => {
                let signature = ty.to_name();
                if !self.object_shape_names.contains_key(&signature) {
                    let shape_name = format!("__objshape_{}", self.object_shape_names.len() + 1);
                    self.object_shape_names
                        .insert(signature.clone(), shape_name.clone());
                    self.class_fields.insert(
                        shape_name,
                        props
                            .iter()
                            .map(|(name, _, ty)| (name.clone(), ty.clone()))
                            .collect(),
                    );
                }
                for (_, _, field_ty) in props {
                    self.register_object_shape_type(field_ty);
                }
            }
            TejxType::FixedArray(inner, _)
            | TejxType::DynamicArray(inner)
            | TejxType::Slice(inner)
            | TejxType::Optional(inner) => self.register_object_shape_type(inner),
            TejxType::Function(params, ret) => {
                for param in params {
                    self.register_object_shape_type(param);
                }
                self.register_object_shape_type(ret);
            }
            TejxType::Class(_, generics) => {
                for generic in generics {
                    self.register_object_shape_type(generic);
                }
            }
            _ => {}
        }
    }

    fn collect_object_shapes(&mut self, functions: &[MIRFunction]) {
        for func in functions {
            self.register_object_shape_type(&func.return_type);
            for ty in func.variables.values() {
                self.register_object_shape_type(ty);
            }
        }
    }

    pub(crate) fn does_escape(&self, func: &MIRFunction, var_name: &str) -> bool {
        // More robust escape analysis: If the reference escapes via return, pass as argument,
        // or is stored inside an object/array, it escapes the stack frame.
        let mut check_vars = vec![var_name.to_string()];
        let mut i = 0;

        while i < check_vars.len() {
            let current_var = check_vars[i].clone();
            for block in &func.blocks {
                for instr in &block.instructions {
                    match instr {
                        MIRInstruction::Call { callee, args, .. } => {
                            // Whitelist: common safe runtime calls where args[0] is 'this'
                            // and the pointer is only borrowed, not escaped.
                            let is_safe = matches!(
                                callee.as_str(),
                                "f_Array_constructor"
                                    | "f_Array_fill"
                                    | "Array_fill"
                                    | "Array_sort"
                                    | "Array_reverse"
                                    | "Array_indexOf"
                                    | "rt_array_get_fast"
                                    | "rt_array_set_fast"
                                    | "rt_array_length"
                                    | "rt_array_get_data_ptr"
                                    | "rt_to_string"
                                    | "rt_print"
                                    | "rt_get_type"
                                    | "rt_is_array"
                                    | "rt_len"
                            );

                            for (i, arg) in args.iter().enumerate() {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name == &current_var {
                                        if i == 0 && is_safe {
                                            continue;
                                        }
                                        return true; // Escapes as argument
                                    }
                                }
                            }
                        }
                        MIRInstruction::IndirectCall { args, .. } => {
                            for arg in args {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name == &current_var {
                                        return true;
                                    }
                                }
                            }
                        }
                        MIRInstruction::Return {
                            value: Some(val), ..
                        } => {
                            if let MIRValue::Variable { name, .. } = val {
                                if name == &current_var {
                                    return true;
                                }
                            }
                        }
                        MIRInstruction::StoreIndex { obj, src, .. }
                        | MIRInstruction::StoreMember { obj, src, .. } => {
                            if let MIRValue::Variable { name, .. } = src {
                                if name == &current_var {
                                    if let MIRValue::Variable { name: obj_name, .. } = obj {
                                        if !check_vars.contains(obj_name) {
                                            check_vars.push(obj_name.clone());
                                        }
                                    } else {
                                        return true;
                                    }
                                }
                            }
                        }
                        MIRInstruction::Move { dst, src, .. } => {
                            if let MIRValue::Variable { name, .. } = src {
                                if name == &current_var && !check_vars.contains(dst) {
                                    check_vars.push(dst.clone());
                                }
                            }
                        }
                        MIRInstruction::Cast {
                            dst,
                            src: MIRValue::Variable { name, .. },
                            ..
                        } => {
                            if name == &current_var && !check_vars.contains(dst) {
                                check_vars.push(dst.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            i += 1;
        }
        false
    }

    fn value_uses_fixed_object_layout(
        &self,
        func: &MIRFunction,
        value: &MIRValue,
        _cache: &mut HashMap<String, bool>,
        visiting: &mut HashSet<String>,
    ) -> bool {
        matches!(value.get_type(), TejxType::Class(_, _))
            || matches!(
                    value,
                    MIRValue::Variable { name, ty, .. }
                        if Self::fixed_layout_object_type(ty).is_some()
                            && if visiting.contains(name) {
                                true
                            } else {
                                self.can_use_fixed_object_layout_for_ty(func, name, ty)
                            }
            )
    }

    fn can_use_fixed_object_layout_with_key(
        &self,
        func: &MIRFunction,
        var_name: &str,
        target_key: &str,
        cache: &mut HashMap<String, bool>,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if let Some(cached) = cache.get(var_name) {
            return *cached;
        }
        if !visiting.insert(var_name.to_string()) {
            return false;
        }

        let result = 'analysis: {
            if func.params.iter().any(|param| param == var_name) {
                break 'analysis false;
            }

            let params_are_opaque = func.params.iter().any(|param| {
                func.variables
                    .get(param)
                    .and_then(Self::fixed_layout_shape_key)
                    .as_ref()
                    == Some(&target_key.to_string())
                    || func
                        .variables
                        .get(param)
                        .map(|ty| {
                            Self::array_element_fixed_layout_shape_key(ty).as_ref()
                                == Some(&target_key.to_string())
                        })
                        .unwrap_or(false)
            });
            if params_are_opaque {
                break 'analysis false;
            }

            let mut defs_to_check = vec![var_name.to_string()];
            let mut seen_defs: HashSet<String> = HashSet::new();
            let mut has_trusted_source = false;

            while let Some(current_var) = defs_to_check.pop() {
                if !seen_defs.insert(current_var.clone()) {
                    continue;
                }

                if func.params.iter().any(|param| param == &current_var) {
                    break 'analysis false;
                }

                let mut found_definition = false;
                for block in &func.blocks {
                    for instr in &block.instructions {
                        match instr {
                            MIRInstruction::Call { dst, callee, .. } if dst == &current_var => {
                                found_definition = true;
                                if callee == "rt_object_new" {
                                    has_trusted_source = true;
                                } else {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::Move {
                                dst,
                                src: MIRValue::Variable { name, .. },
                                ..
                            } if dst == &current_var => {
                                found_definition = true;
                                defs_to_check.push(name.clone());
                            }
                            MIRInstruction::LoadMember {
                                dst, obj, member, ..
                            } if dst == &current_var => {
                                found_definition = true;
                                let parent_is_fixed_layout =
                                    self.value_uses_fixed_object_layout(func, obj, cache, visiting);
                                let field_matches_shape = match self
                                    .resolve_fixed_field_info(obj.get_type(), member)
                                {
                                    Some((_, field_ty)) => {
                                        Self::fixed_layout_shape_key(&field_ty).as_deref()
                                            == Some(target_key)
                                            || match &field_ty {
                                                TejxType::Optional(inner) => {
                                                    Self::fixed_layout_shape_key(inner).as_deref()
                                                        == Some(target_key)
                                                }
                                                _ => false,
                                            }
                                    }
                                    None => false,
                                };
                                if parent_is_fixed_layout && field_matches_shape {
                                    has_trusted_source = true;
                                } else {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::LoadIndex { dst, obj, .. } if dst == &current_var => {
                                found_definition = true;
                                if Self::array_element_fixed_layout_shape_key(obj.get_type())
                                    .as_deref()
                                    == Some(target_key)
                                {
                                    has_trusted_source = true;
                                } else {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::BinaryOp { dst, .. }
                            | MIRInstruction::IndirectCall { dst, .. }
                            | MIRInstruction::Cast { dst, .. }
                                if dst == &current_var =>
                            {
                                break 'analysis false;
                            }
                            _ => {}
                        }
                    }
                }

                if !found_definition {
                    break 'analysis false;
                }
            }

            let mut use_aliases = vec![var_name.to_string()];
            let mut seen_uses: HashSet<String> = HashSet::new();

            while let Some(current_var) = use_aliases.pop() {
                if !seen_uses.insert(current_var.clone()) {
                    continue;
                }

                for block in &func.blocks {
                    for instr in &block.instructions {
                        match instr {
                            MIRInstruction::Move {
                                dst,
                                src: MIRValue::Variable { name, .. },
                                ..
                            } if name == &current_var => {
                                if func
                                    .variables
                                    .get(dst)
                                    .and_then(Self::fixed_layout_shape_key)
                                    .as_deref()
                                    != Some(target_key)
                                {
                                    break 'analysis false;
                                }
                                use_aliases.push(dst.clone());
                            }
                            MIRInstruction::StoreMember {
                                obj: MIRValue::Variable { name, .. },
                                ..
                            }
                            | MIRInstruction::LoadMember {
                                obj: MIRValue::Variable { name, .. },
                                ..
                            } if name == &current_var => {}
                            MIRInstruction::StoreIndex {
                                obj,
                                src: MIRValue::Variable { name, .. },
                                ..
                            } if name == &current_var => {
                                if Self::array_element_fixed_layout_shape_key(obj.get_type())
                                    .as_deref()
                                    != Some(target_key)
                                {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::StoreMember {
                                obj,
                                src: MIRValue::Variable { name, .. },
                                ..
                            } if name == &current_var => {
                                if !self.value_uses_fixed_object_layout(func, obj, cache, visiting)
                                {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::Call { callee, args, .. } => {
                                for (arg_index, arg) in args.iter().enumerate() {
                                    let MIRValue::Variable { name, .. } = arg else {
                                        continue;
                                    };
                                    if name != &current_var {
                                        continue;
                                    }

                                    let safe_array_store = (callee == "rt_array_push"
                                        && arg_index == 1
                                        && args
                                            .first()
                                            .map(|array_arg| {
                                                Self::array_element_fixed_layout_shape_key(
                                                    array_arg.get_type(),
                                                )
                                                .as_deref()
                                                    == Some(target_key)
                                            })
                                            .unwrap_or(false))
                                        || (callee == "rt_array_set_fast"
                                            && arg_index == 2
                                            && args
                                                .first()
                                                .map(|array_arg| {
                                                    Self::array_element_fixed_layout_shape_key(
                                                        array_arg.get_type(),
                                                    )
                                                    .as_deref()
                                                        == Some(target_key)
                                                })
                                                .unwrap_or(false));

                                    if !safe_array_store {
                                        break 'analysis false;
                                    }
                                }
                            }
                            MIRInstruction::IndirectCall { args, .. } => {
                                for arg in args {
                                    if matches!(arg, MIRValue::Variable { name, .. } if name == &current_var)
                                    {
                                        break 'analysis false;
                                    }
                                }
                            }
                            MIRInstruction::Return { value, .. } => {
                                if matches!(value, Some(MIRValue::Variable { name, .. }) if name == &current_var)
                                {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::Cast { src, .. } => {
                                if matches!(src, MIRValue::Variable { name, .. } if name == &current_var)
                                {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::LoadIndex { obj, .. } => {
                                if matches!(obj, MIRValue::Variable { name, .. } if name == &current_var)
                                {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::Branch { condition, .. } => {
                                if matches!(condition, MIRValue::Variable { name, .. } if name == &current_var)
                                {
                                    break 'analysis false;
                                }
                            }
                            MIRInstruction::BinaryOp {
                                left, op, right, ..
                            } => {
                                let left_matches = matches!(
                                    left,
                                    MIRValue::Variable { name, .. } if name == &current_var
                                );
                                let right_matches = matches!(
                                    right,
                                    MIRValue::Variable { name, .. } if name == &current_var
                                );
                                if left_matches || right_matches {
                                    let is_null_compare =
                                        matches!(op, TokenType::EqualEqual | TokenType::BangEqual)
                                            && ((left_matches
                                                && (right_matches
                                                    || Self::is_null_like_value(right)))
                                                || (right_matches
                                                    && (left_matches
                                                        || Self::is_null_like_value(left))));
                                    if !is_null_compare {
                                        break 'analysis false;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            has_trusted_source
        };

        visiting.remove(var_name);
        cache.insert(var_name.to_string(), result);
        result
    }

    pub(crate) fn can_use_fixed_object_layout_for_ty(
        &self,
        func: &MIRFunction,
        var_name: &str,
        ty: &TejxType,
    ) -> bool {
        let Some(target_key) = Self::fixed_layout_shape_key(ty) else {
            return false;
        };
        self.can_use_fixed_object_layout_for_key(func, var_name, &target_key)
    }

    fn can_use_fixed_object_layout_for_key(
        &self,
        func: &MIRFunction,
        var_name: &str,
        target_key: &str,
    ) -> bool {
        let mut cache: HashMap<String, bool> = HashMap::new();
        let mut visiting: HashSet<String> = HashSet::new();
        self.can_use_fixed_object_layout_with_key(
            func,
            var_name,
            target_key,
            &mut cache,
            &mut visiting,
        )
    }

    pub(crate) fn fixed_layout_object_type(ty: &TejxType) -> Option<TejxType> {
        match ty {
            TejxType::Object(_) => Some(ty.clone()),
            TejxType::Optional(inner) => Self::fixed_layout_object_type(inner),
            _ => None,
        }
    }

    pub(crate) fn needs_arena(&self, func: &MIRFunction) -> bool {
        for bb in &func.blocks {
            for inst in &bb.instructions {
                if let MIRInstruction::Call {
                    callee, args, dst, ..
                } = inst
                {
                    if callee == "f_Array_constructor"
                        || callee == "rt_Array_new_fixed"
                        || callee == "f_Function_constructor"
                    {
                        return true;
                    }
                    if callee == "rt_object_new" && !dst.is_empty() && !self.does_escape(func, dst)
                    {
                        return true;
                    }
                    if callee == RT_CLASS_NEW && !dst.is_empty() && !self.does_escape(func, dst) {
                        let _class_name = match args.first() {
                            Some(MIRValue::Constant { value, .. }) => {
                                value.trim_matches('"').to_string()
                            }
                            _ => continue,
                        };
                        return true;
                    }
                }
            }
        }
        false
    }

    pub(crate) fn get_captured_index(&self, name: &str) -> Option<usize> {
        if let Some(pos) = self.captured_vars.iter().position(|c| c == name) {
            return Some(pos);
        }
        // Handle MIR mangling suffixes like _123
        for (i, cap) in self.captured_vars.iter().enumerate() {
            if name.starts_with(cap)
                && (name.len() == cap.len() || name[cap.len()..].starts_with('_'))
            {
                return Some(i);
            }
        }
        None
    }

    pub(crate) fn is_captured(&self, name: &str) -> bool {
        self.get_captured_index(name).is_some()
    }

    pub fn generate_with_blocks(
        &mut self,
        functions: &[MIRFunction],
        captured_vars_by_function: HashMap<String, HashSet<String>>,
    ) -> String {
        self.captured_vars_by_function.clear();
        self.object_shape_names.clear();
        for (name, vars) in captured_vars_by_function {
            let mut sorted: Vec<String> = vars.into_iter().collect();
            sorted.sort();
            self.captured_vars_by_function.insert(name, sorted);
        }
        self.captured_vars.clear();
        self.buffer.clear();
        self.global_buffer.clear();
        self.declared_functions.clear();
        self.function_param_counts.clear();
        self.function_param_types.clear();
        self.declared_globals.clear();
        self.global_types.clear();

        self.collect_object_shapes(functions);

        // Module headers
        self.global_buffer.push_str("; ModuleID = 'tejx_module'\n");
        self.global_buffer
            .push_str("source_filename = \"tejx_module\"\n");
        let (datalayout, triple) = Self::get_target_info();
        self.global_buffer
            .push_str(&format!("target datalayout = \"{}\"\n", datalayout));
        self.global_buffer
            .push_str(&format!("target triple = \"{}\"\n", triple));

        // Register defined functions and their param counts
        let mut has_tejx_main = false;
        for f in functions {
            self.declared_functions.insert(f.name.clone());
            self.function_param_counts
                .insert(f.name.clone(), f.params.len());
            self.function_param_types.insert(
                f.name.clone(),
                f.params
                    .iter()
                    .filter_map(|param_name| f.variables.get(param_name).cloned())
                    .collect(),
            );
            if f.name == "tejx_main" {
                has_tejx_main = true;
            }
        }

        // Collect and declare global variables
        let mut globals = HashSet::new();
        for func in functions {
            for (name, ty) in &func.variables {
                if name.starts_with("g_") {
                    globals.insert(name.clone());
                    self.global_types
                        .entry(name.clone())
                        .or_insert_with(|| ty.clone());
                }
            }
            for bb in &func.blocks {
                for inst in &bb.instructions {
                    match inst {
                        MIRInstruction::Move { dst, src, .. } => {
                            if dst.starts_with("g_") {
                                globals.insert(dst.clone());
                            }
                            if let MIRValue::Variable { name, .. } = src {
                                if name.starts_with("g_") {
                                    globals.insert(name.clone());
                                }
                            }
                        }
                        MIRInstruction::BinaryOp {
                            dst,
                            left,
                            right,
                            op_width: _,
                            ..
                        } => {
                            if dst.starts_with("g_") {
                                globals.insert(dst.clone());
                            }
                            if let MIRValue::Variable { name, .. } = left {
                                if name.starts_with("g_") {
                                    globals.insert(name.clone());
                                }
                            }
                            if let MIRValue::Variable { name, .. } = right {
                                if name.starts_with("g_") {
                                    globals.insert(name.clone());
                                }
                            }
                        }
                        MIRInstruction::Call { dst, args, .. } => {
                            if dst.starts_with("g_") {
                                globals.insert(dst.clone());
                            }
                            for arg in args {
                                if let MIRValue::Variable { name, .. } = arg {
                                    if name.starts_with("g_") {
                                        globals.insert(name.clone());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let mut has_gc_globals = false;
        for g in &globals {
            if !self.declared_globals.contains(g) {
                self.global_buffer
                    .push_str(&format!("@{} = global i64 0\n", g));
                self.declared_globals.insert(g.clone());
            }
            if self.is_gc_global(g) {
                has_gc_globals = true;
                let slot_name = Self::static_root_slot_name(g);
                self.global_buffer
                    .push_str(&format!("@{} = global i64 -1\n", slot_name));
            }
        }

        if has_gc_globals {
            self.declare_runtime_fn(
                "rt_add_static_root_global",
                "i64 @rt_add_static_root_global(i64)",
            );
            self.declare_runtime_fn(
                "rt_get_static_root_global",
                "i64 @rt_get_static_root_global(i64)",
            );
            self.declare_runtime_fn(
                "rt_set_static_root_global",
                "void @rt_set_static_root_global(i64, i64)",
            );

            self.global_buffer.push_str(
                "define i64 @tejx_get_global_root(i64* %slot_ptr, i64* %value_ptr) nounwind {\n\
entry:\n\
  %slot0 = load i64, i64* %slot_ptr\n\
  %ready0 = icmp sge i64 %slot0, 0\n\
  br i1 %ready0, label %loaded, label %init\n\
\n\
init:\n\
  %legacy = load i64, i64* %value_ptr\n\
  %slot1 = call i64 @rt_add_static_root_global(i64 %legacy)\n\
  store i64 %slot1, i64* %slot_ptr\n\
  ret i64 %legacy\n\
\n\
loaded:\n\
  %value = call i64 @rt_get_static_root_global(i64 %slot0)\n\
  ret i64 %value\n\
}\n",
            );
            self.global_buffer.push_str(
                "define void @tejx_set_global_root(i64* %slot_ptr, i64* %value_ptr, i64 %value) nounwind {\n\
entry:\n\
  store i64 %value, i64* %value_ptr\n\
  %slot0 = load i64, i64* %slot_ptr\n\
  %ready0 = icmp sge i64 %slot0, 0\n\
  br i1 %ready0, label %update, label %init\n\
\n\
init:\n\
  %slot1 = call i64 @rt_add_static_root_global(i64 %value)\n\
  store i64 %slot1, i64* %slot_ptr\n\
  ret void\n\
\n\
update:\n\
  call void @rt_set_static_root_global(i64 %slot0, i64 %value)\n\
  ret void\n\
}\n",
            );
        }

        self.global_buffer
            .push_str("@.fmt_d = private unnamed_addr constant [5 x i8] c\"%lld\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_f = private unnamed_addr constant [3 x i8] c\"%f\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_s = private unnamed_addr constant [3 x i8] c\"%s\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_nl = private unnamed_addr constant [2 x i8] c\"\\0A\\00\"\n");
        self.global_buffer
            .push_str("@.fmt_sp = private unnamed_addr constant [2 x i8] c\" \\00\"\n");

        self.global_buffer
            .push_str("%struct.ObjectHeader = type { i64, i16, i16, i32, i32, i32 }\n");

        // --- Type Registration ---
        let mut type_id = 100; // Start user types from 100
        let mut init_type_buffer = String::new();
        init_type_buffer.push_str("define void @rt_init_types() {\n");

        let class_defs: Vec<(String, Vec<(String, TejxType)>)> = self
            .class_fields
            .iter()
            .map(|(class_name, fields)| (class_name.clone(), fields.clone()))
            .collect();

        for (class_name, fields) in class_defs {
            let id = type_id;
            type_id += 1;
            self.type_id_map.insert(class_name.clone(), id);

            let mut ptr_offsets = Vec::new();
            let mut field_offsets = Vec::new();
            let mut field_kinds = Vec::new();
            let mut current_offset = 0;
            for (_name, ty) in &fields {
                current_offset = Self::get_aligned_offset(current_offset, ty);
                field_offsets.push(current_offset);
                field_kinds.push(Self::stringify_field_kind(ty));
                if Self::is_gc_managed(ty) {
                    ptr_offsets.push(current_offset);
                }
                current_offset += ty.size();
            }
            let size = (current_offset + 7) & !7;

            // Create global array for offsets
            let offset_arr_name = format!("@type_{}_offsets", id);
            if !ptr_offsets.is_empty() {
                let offsets_str: Vec<String> =
                    ptr_offsets.iter().map(|o| format!("i64 {}", o)).collect();
                self.global_buffer.push_str(&format!(
                    "{} = private constant [{} x i64] [{}]\n",
                    offset_arr_name,
                    ptr_offsets.len(),
                    offsets_str.join(", ")
                ));
            }

            // Register call: rt_register_type(id, size, ptr_count, offsets_ptr, finalizer)
            let offsets_ptr = if ptr_offsets.is_empty() {
                "null".to_string()
            } else {
                format!(
                    "bitcast ([{} x i64]* {} to i64*)",
                    ptr_offsets.len(),
                    offset_arr_name
                )
            };

            // Check for finalizer
            let mut finalizer_ptr = "null".to_string();
            if let Some(methods) = self.class_methods.get(&class_name) {
                if methods.contains(&"finalize".to_string())
                    || methods.contains(&"~destructor".to_string())
                {
                    let method_name = if methods.contains(&"finalize".to_string()) {
                        "finalize"
                    } else {
                        "~destructor"
                    };
                    let wrapper_name = format!("@finalizer_wrapper_{}", id);
                    let real_method = format!("f_{}_{}", class_name, method_name);

                    self.global_buffer.push_str(&format!(
                        "define void {}(i64 %body) nounwind {{\n  call void @{}(i64 %body)\n  ret void\n}}\n",
                        wrapper_name, real_method
                    ));
                    finalizer_ptr = format!("bitcast (void (i64)* {} to i8*)", wrapper_name);
                }
            }

            let display_name = if class_name.starts_with("__objshape_") {
                "object"
            } else {
                class_name.as_str()
            };
            let type_name_ptr = self.emit_string_constant(display_name);

            let field_offsets_arr_name = format!("@type_{}_field_offsets", id);
            if !field_offsets.is_empty() {
                let offsets_str: Vec<String> =
                    field_offsets.iter().map(|o| format!("i64 {}", o)).collect();
                self.global_buffer.push_str(&format!(
                    "{} = private constant [{} x i64] [{}]\n",
                    field_offsets_arr_name,
                    field_offsets.len(),
                    offsets_str.join(", ")
                ));
            }

            let field_kinds_arr_name = format!("@type_{}_field_kinds", id);
            if !field_kinds.is_empty() {
                let kinds_str: Vec<String> =
                    field_kinds.iter().map(|k| format!("i8 {}", k)).collect();
                self.global_buffer.push_str(&format!(
                    "{} = private constant [{} x i8] [{}]\n",
                    field_kinds_arr_name,
                    field_kinds.len(),
                    kinds_str.join(", ")
                ));
            }

            let field_names_arr_name = format!("@type_{}_field_names", id);
            if !fields.is_empty() {
                let name_ptrs: Vec<String> = fields
                    .iter()
                    .map(|(name, _)| format!("i64 {}", self.emit_string_constant(name)))
                    .collect();
                self.global_buffer.push_str(&format!(
                    "{} = private constant [{} x i64] [{}]\n",
                    field_names_arr_name,
                    fields.len(),
                    name_ptrs.join(", ")
                ));
            }

            let field_offsets_ptr = if field_offsets.is_empty() {
                "null".to_string()
            } else {
                format!(
                    "bitcast ([{} x i64]* {} to i64*)",
                    field_offsets.len(),
                    field_offsets_arr_name
                )
            };
            let field_kinds_ptr = if field_kinds.is_empty() {
                "null".to_string()
            } else {
                format!(
                    "bitcast ([{} x i8]* {} to i8*)",
                    field_kinds.len(),
                    field_kinds_arr_name
                )
            };
            let field_names_ptr = if fields.is_empty() {
                "null".to_string()
            } else {
                format!(
                    "bitcast ([{} x i64]* {} to i64*)",
                    fields.len(),
                    field_names_arr_name
                )
            };

            init_type_buffer.push_str(&format!(
                "  call void @rt_register_type(i32 {}, i64 {}, i64 {}, i64* {}, i8* {})\n",
                id,
                size,
                ptr_offsets.len(),
                offsets_ptr,
                finalizer_ptr
            ));
            init_type_buffer.push_str(&format!(
                "  call void @rt_register_type_info(i32 {}, i64 {}, i64 {}, i64* {}, i8* {}, i64* {})\n",
                id,
                type_name_ptr,
                fields.len(),
                field_offsets_ptr,
                field_kinds_ptr,
                field_names_ptr
            ));
        }
        init_type_buffer.push_str("  ret void\n}\n");
        self.buffer.push_str(&init_type_buffer);

        self.declare_runtime_fn(
            "rt_register_type",
            "void @rt_register_type(i32, i64, i64, i64*, i8*)",
        );
        self.declare_runtime_fn(
            "rt_register_type_info",
            "void @rt_register_type_info(i32, i64, i64, i64*, i8*, i64*)",
        );

        // Pre-declare commonly used runtime functions
        self.declare_runtime_fn(
            "rt_class_new",
            "i64 @rt_class_new(i32, i64, i64, i64*, i64) nounwind",
        );
        self.declare_runtime_fn("rt_object_new", "i64 @rt_object_new()");
        self.declare_runtime_fn("rt_len", "i64 @rt_len(i64)");
        self.declare_runtime_fn("rt_strlen", "i64 @rt_strlen(i64)");
        self.declare_runtime_fn("rt_typeof", "i64 @rt_typeof(i64)");
        self.declare_runtime_fn("rt_to_string", "i64 @rt_to_string(i64)");
        self.declare_runtime_fn(
            "rt_exception_report_for_print",
            "i64 @rt_exception_report_for_print(i64)",
        );
        self.declare_runtime_fn("rt_str_concat_v2", "i64 @rt_str_concat_v2(i64, i64)");
        self.declare_runtime_fn("rt_str_append_local", "i64 @rt_str_append_local(i64, i64)");
        self.declare_runtime_fn("rt_str_clear_local", "i64 @rt_str_clear_local(i64)");
        self.declare_runtime_fn("rt_String_concat", "i64 @rt_String_concat(i64, i64)");
        self.declare_runtime_fn("rt_str_equals", "i32 @rt_str_equals(i64, i64)");
        self.declare_runtime_fn("rt_box_int", "i64 @rt_box_int(i64)");
        self.declare_runtime_fn("rt_box_number", "i64 @rt_box_number(double)");
        self.declare_runtime_fn("rt_unbox_int", "i64 @rt_unbox_int(i64)");
        self.declare_runtime_fn("rt_unbox_number", "double @rt_unbox_number(i64)");
        self.declare_runtime_fn("rt_array_push", "i64 @rt_array_push(i64, i64)");
        self.declare_runtime_fn("rt_array_pop", "i64 @rt_array_pop(i64)");
        self.declare_runtime_fn("rt_array_get_fast", "i64 @rt_array_get_fast(i64, i64)");
        self.declare_runtime_fn("rt_array_set_fast", "i64 @rt_array_set_fast(i64, i64, i64)");
        self.declare_runtime_fn("rt_Object_keys", "i64 @rt_Object_keys(i64)");
        self.declare_runtime_fn("rt_Object_values", "i64 @rt_Object_values(i64)");
        self.declare_runtime_fn("rt_Object_entries", "i64 @rt_Object_entries(i64)");
        self.declare_runtime_fn("rt_Object_assign", "i64 @rt_Object_assign(i64, i64)");
        self.declare_runtime_fn("rt_Object_freeze", "i64 @rt_Object_freeze(i64)");
        self.declare_runtime_fn("rt_is_nullish", "i64 @rt_is_nullish(i64)");
        self.declare_runtime_fn("rt_not", "i64 @rt_not(i64)");
        self.declare_runtime_fn("rt_panic", "void @rt_panic(i64)");
        self.declare_runtime_fn("rt_Arena_create", "i64 @rt_Arena_create()");
        self.declare_runtime_fn("rt_Arena_destroy", "void @rt_Arena_destroy(i64)");
        self.declare_runtime_fn("rt_closure_from_ptr", "i64 @rt_closure_from_ptr(i64)");
        self.declare_runtime_fn("rt_enter_frame", "void @rt_enter_frame(i64, i64, i64) nounwind");
        self.declare_runtime_fn("rt_leave_frame", "void @rt_leave_frame() nounwind");
        self.declare_runtime_fn("rt_set_location", "void @rt_set_location(i64, i64) nounwind");

        // Filter functions to remove duplicates, prioritizing non-empty tejx_main
        let mut unique_functions = Vec::new();
        let mut seen = HashSet::new();
        let mut tejx_main_to_keep = None;

        for (i, func) in functions.iter().enumerate() {
            if func.name == "tejx_main" {
                if tejx_main_to_keep.is_none() || func.blocks[0].instructions.len() > 1 {
                    tejx_main_to_keep = Some(i);
                }
                continue;
            }
            if !seen.contains(&func.name) {
                seen.insert(func.name.clone());
                unique_functions.push(func);
            }
        }
        if let Some(idx) = tejx_main_to_keep {
            unique_functions.push(&functions[idx]);
        }

        let (tracked_runtime_functions, known_mir_functions, extern_mir_functions) =
            Self::compute_tracked_runtime_functions(&unique_functions);
        self.tracked_runtime_functions = tracked_runtime_functions;
        self.known_mir_functions = known_mir_functions;
        self.extern_mir_functions = extern_mir_functions;

        for func in unique_functions {
            self.captured_vars = self
                .captured_vars_by_function
                .get(&func.name)
                .cloned()
                .unwrap_or_default();
            self.gen_function_v2(func);
        }

        // Exception handling runtime functions
        self.declare_runtime_fn("_setjmp", "i32 @_setjmp(i8*) returns_twice");
        self.declare_runtime_fn(
            TEJX_PUSH_HANDLER,
            &format!("void @{}(i8*)", TEJX_PUSH_HANDLER),
        );
        self.declare_runtime_fn(TEJX_POP_HANDLER, &format!("void @{}()", TEJX_POP_HANDLER));
        self.declare_runtime_fn(TEJX_THROW, &format!("void @{}(i64)", TEJX_THROW));
        self.declare_runtime_fn(
            TEJX_GET_EXCEPTION,
            &format!("i64 @{}()", TEJX_GET_EXCEPTION),
        );
        self.declare_runtime_fn("rt_string_from_c_str", "i64 @rt_string_from_c_str(i64)");
        self.declare_runtime_fn(
            "rt_string_from_c_str_const",
            "i64 @rt_string_from_c_str_const(i64)",
        );

        // Generate main wrapper if tejx_main exists
        if has_tejx_main {
            self.buffer.push('\n');
            self.buffer
                .push_str(&format!("declare i32 @{}(i32, i8**)\n", TEJX_RUNTIME_MAIN));
            self.buffer
                .push_str("define i32 @main(i32 %argc, i8** %argv) {\n");
            self.buffer.push_str("entry:\n");
            self.buffer.push_str(&format!(
                "  %call = call i32 @{}(i32 %argc, i8** %argv)\n",
                TEJX_RUNTIME_MAIN
            ));
            self.buffer.push_str("  ret i32 %call\n");
            self.buffer.push_str("}\n");
        }

        format!("{}{}", self.global_buffer, self.buffer)
    }

    pub(crate) fn gen_function_v2(&mut self, func: &MIRFunction) {
        self.value_map.clear();
        self.ptr_types.clear();
        self.stack_arrays.clear();
        self.heap_array_ptrs.clear();
        self.float_ssa_vars.clear();
        self.boxed_string_cache.clear();
        self.temp_counter = 0;
        self.current_function_params.clear();
        self.local_vars.clear();
        self.current_env = None;
        self.current_arena = None;
        self.num_roots = 0;
        self.current_debug_line = None;
        self.volatile_locals = func.blocks.iter().any(|b| b.exception_handler.is_some());
        self.current_function_has_runtime_frame =
            !self.source_file.is_empty() && self.tracked_runtime_functions.contains(&func.name);
        self.current_function_tracks_location =
            self.current_function_has_runtime_frame
                && Self::function_tracks_runtime_location(&func.name);

        let ret_llvm_ty = Self::get_llvm_type(&func.return_type);

        if func.is_extern {
            let decl_params = if func.params.is_empty() {
                String::new()
            } else {
                func.params
                    .iter()
                    .map(|p| {
                        let ty = func.variables.get(p).unwrap_or(&TejxType::Void);
                        Self::get_llvm_type(ty).to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            self.declare_runtime_fn(
                &func.name,
                &format!("{} @\"{}\"({})", ret_llvm_ty, func.name, decl_params),
            );
            return;
        }

        // Skip if function was already defined (prevents duplicate definitions from prelude)
        if self.defined_functions.contains(&func.name) {
            return;
        }
        self.defined_functions.insert(func.name.clone());

        let params_str = if func.params.is_empty() {
            String::new()
        } else {
            func.params
                .iter()
                .map(|p| {
                    let ty = func.variables.get(p).unwrap_or(&TejxType::Void);
                    format!("{} %{}", Self::get_llvm_type(ty), p)
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        // Track function parameter counts for Call/IndirectCall logic
        self.function_param_counts
            .insert(func.name.clone(), func.params.len());
        self.current_function_params = func.params.iter().cloned().collect();

        self.emit(&format!(
            "define {} @\"{}\"({}) {{\n",
            ret_llvm_ty, func.name, params_str
        ));

        // Entry: allocas for all variables used in the function
        self.emit("entry:\n");
        let entry_marker = self.buffer.len();

        self.declare_runtime_fn("rt_safepoint_poll", "void @rt_safepoint_poll() nounwind");

        // 1. Scan for all local variables
        // Create one arena per function (not per basic block)
        if self.needs_arena(func) {
            self.declare_runtime_fn(
                RT_ARENA_CREATE,
                &format!("i64 @{}(i64) nounwind", RT_ARENA_CREATE),
            );
            self.declare_runtime_fn(
                RT_ARENA_DESTROY,
                &format!("void @{}(i64) nounwind", RT_ARENA_DESTROY),
            );
            self.temp_counter += 1;
            let arena_reg = format!("%arena_{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call i64 @{}(i64 0)",
                arena_reg, RT_ARENA_CREATE
            ));
            self.current_arena = Some(arena_reg);
        }

        // 1. Scan for all local variables
        for bb in &func.blocks {
            for inst in &bb.instructions {
                let dest_var = match inst {
                    MIRInstruction::Move { dst, .. }
                    | MIRInstruction::BinaryOp { dst, .. }
                    | MIRInstruction::Call { dst, .. }
                    | MIRInstruction::IndirectCall { dst, .. }
                    | MIRInstruction::LoadMember { dst, .. }
                    | MIRInstruction::LoadIndex { dst, .. }
                    | MIRInstruction::Cast { dst, .. } => Some(dst.clone()),
                    _ => None,
                };
                if let Some(name) = dest_var {
                    if !name.starts_with("g_") && !self.current_function_params.contains(&name) {
                        self.local_vars.insert(name);
                    }
                }
            }
        }

        // 2. Deterministic IR: Collect and sort ALL variables that need an alloca (params + locals)
        let mut sorted_alloca_vars: Vec<String> = Vec::new();
        for p in &func.params {
            // Parameters ALWAYS get an alloca in their owning function.
            // If they are captured, they are COPIED to the environment after being stored in the alloca.
            sorted_alloca_vars.push(p.clone());
        }
        for name in &self.local_vars {
            // Local variables also get an alloca even if captured.
            // This simplifies the codegen as they can be treated as normal locals.
            sorted_alloca_vars.push(name.clone());
        }
        sorted_alloca_vars.sort();
        sorted_alloca_vars.dedup();

        // 3. Emit allocas deterministically
        for name in &sorted_alloca_vars {
            let ty = func.variables.get(name).unwrap_or(&TejxType::Void);
            // Void-typed temps (e.g. result of void-returning calls or unresolved types)
            // still need an alloca as they may be used as call destinations. Default to i64.
            let llvm_ty = if matches!(ty, TejxType::Void) {
                "i64"
            } else {
                Self::get_llvm_storage_type(ty)
            };
            let reg_name = format!("%{}_ptr", name.replace('$', "_"));
            self.alloca_buffer
                .push_str(&format!("  {} = alloca {}\n", reg_name, llvm_ty));
            self.ptr_types.insert(reg_name.clone(), llvm_ty.to_string());
            self.value_map.insert(name.clone(), reg_name.clone());
        }

        // GC roots must start as null. Otherwise a safepoint during heavy allocation can
        // scan uninitialized stack garbage as live heap pointers.
        for name in &sorted_alloca_vars {
            let ty = func.variables.get(name).unwrap_or(&TejxType::Void);
            if Self::needs_gc_root(name, ty) {
                if let Some(reg_name) = self.value_map.get(name) {
                    self.emit_line(&format!("store i64 0, i64* {}", reg_name));
                }
            }
        }

        // Create environment if needed
        let has_captures = self.local_vars.iter().any(|v| self.is_captured(v))
            || func.params.iter().any(|p| self.is_captured(p));

        if func.name.starts_with("lambda_") {
            if !func.params.is_empty() {
                // Reuse the passed environment pointer so closures share state.
                self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");

                self.temp_counter += 1;
                let env_alloca = format!("%env_alloca_{}", self.temp_counter);
                self.alloca_buffer
                    .push_str(&format!("  {} = alloca i64\n", env_alloca));

                let passed_env = format!("%{}", func.params[0]);
                self.emit_line(&format!("store i64 {}, i64* {}", passed_env, env_alloca));
                self.emit_line(&format!("call void @rt_push_root(i64* {})", env_alloca));
                self.num_roots += 1;

                self.current_env = Some(passed_env);
            }
        } else if has_captures {
            self.declare_runtime_fn("rt_array_new", "i64 @rt_array_new(i64, i64) nounwind");

            self.temp_counter += 1;
            let env_alloca = format!("%env_alloca_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", env_alloca));

            let env_reg = format!("%env_id{}", self.temp_counter);
            let cap_count = self.captured_vars.len();
            self.emit_line(&format!(
                "{} = call i64 @rt_array_new(i64 {}, i64 8)",
                env_reg, cap_count
            ));
            self.emit_line(&format!("store i64 {}, i64* {}", env_reg, env_alloca));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", env_alloca));
            self.num_roots += 1;
            self.current_env = Some(env_reg);
        }

        // 4. Store parameters into their allocas
        for p in &func.params {
            if let Some(reg_name) = self.value_map.get(p).cloned() {
                let ty = func.variables.get(p).unwrap_or(&TejxType::Void);
                self.store_ptr(&reg_name, &format!("%{}", p), Some(ty));
            }
        }

        // 5. GC Root Registration: Emit managed variables deterministically
        let mut sorted_managed_vars: Vec<String> = sorted_alloca_vars
            .iter()
            .filter(|name| {
                if let Some(ty) = func.variables.get(*name) {
                    Self::needs_gc_root(name, ty)
                } else {
                    false
                }
            })
            .cloned()
            .collect();
        sorted_managed_vars.sort();

        for name in sorted_managed_vars {
            if let Some(ptr_name) = self.value_map.get(&name).cloned() {
                self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                self.emit_line(&format!("call void @rt_push_root(i64* {})", ptr_name));
                self.num_roots += 1;
            }
        }

        // Sync parameters to environment if captured
        for p in &func.params {
            if let Some(cap_idx) = self.get_captured_index(p) {
                if let Some(env) = self.current_env.clone() {
                    self.declare_runtime_fn(
                        "rt_array_set_fast",
                        "i64 @rt_array_set_fast(i64, i64, i64)",
                    );

                    let ty = func.variables.get(p).unwrap();
                    let val_to_store = if ty.is_float() {
                        self.temp_counter += 1;
                        let bits = format!("%fbits_param_{}", self.temp_counter);
                        self.emit_line(&format!("{} = bitcast double %{} to i64", bits, p));
                        bits
                    } else if ty.is_numeric() || *ty == TejxType::Bool || *ty == TejxType::Char {
                        let casted = self.emit_abi_cast(&format!("%{}", p), ty, &TejxType::Int64);
                        self.temp_counter += 1;
                        let tmp = format!("%zext_param_{}", self.temp_counter);
                        self.emit_line(&format!("{} = or i64 0, {}", tmp, casted));
                        tmp
                    } else {
                        format!("%{}", p)
                    };

                    self.emit_line(&format!(
                        "call i64 @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                        env, cap_idx, val_to_store
                    ));
                }
            }
        }

        let entry_line = func
            .blocks
            .iter()
            .flat_map(|bb| bb.instructions.iter())
            .map(|inst| inst.get_line())
            .find(|line| *line > 0)
            .unwrap_or(0);

        if self.current_function_has_runtime_frame {
            let source_file = self.source_file.clone();
            let display_name = self
                .function_display_names
                .get(&func.name)
                .cloned()
                .unwrap_or_else(|| func.name.clone());
            let function_ptr = self.emit_string_constant(&display_name);
            let file_ptr = if self.current_function_tracks_location {
                self.emit_string_constant(&source_file)
            } else {
                "0".to_string()
            };
            let entry_line = if self.current_function_tracks_location {
                entry_line
            } else {
                0
            };
            self.emit_line(&format!(
                "call void @rt_enter_frame(i64 {}, i64 {}, i64 {})",
                function_ptr, file_ptr, entry_line
            ));
        }

        // Branch to first block
        if !func.blocks.is_empty() {
            self.emit_line("call void @rt_safepoint_poll()");
            self.emit_line(&format!("br label %{}", func.blocks[0].name));
        } else {
            if self.current_function_has_runtime_frame {
                self.emit_line("call void @rt_leave_frame()");
            }
            self.emit_line("ret i64 0");
        }

        // Generate blocks with block name resolution
        for (i, bb) in func.blocks.iter().enumerate() {
            self.emit(&format!("{}:\n", bb.name));

            let mut has_handler = false;
            if let Some(handler_idx) = bb.exception_handler {
                if handler_idx < func.blocks.len() {
                    has_handler = true;
                    let handler_name = &func.blocks[handler_idx].name;
                    // Allocate jmp_buf on THIS function's stack frame
                    self.temp_counter += 1;
                    let jmpbuf = format!("%jmpbuf{}", self.temp_counter);
                    self.alloca_buffer
                        .push_str(&format!("  {} = alloca [37 x i64]\n", jmpbuf));
                    self.temp_counter += 1;
                    let jmpbuf_ptr = format!("%jmpbuf_ptr{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = bitcast [37 x i64]* {} to i8*",
                        jmpbuf_ptr, jmpbuf
                    ));
                    // Call setjmp inline — this is the critical part
                    self.temp_counter += 1;
                    let handler_res = format!("%handler_res{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = call i32 @_setjmp(i8* {}) returns_twice",
                        handler_res, jmpbuf_ptr
                    ));
                    // If setjmp returned 0, register the handler and continue
                    self.temp_counter += 1;
                    let is_exception = format!("%is_exception{}", self.temp_counter);
                    self.emit_line(&format!(
                        "{} = icmp ne i32 {}, 0",
                        is_exception, handler_res
                    ));
                    let body_label = format!("{}_body", bb.name);
                    self.emit_line(&format!(
                        "br i1 {}, label %{}, label %{}",
                        is_exception, handler_name, body_label
                    ));
                    self.emit(&format!("{}:\n", body_label));
                    // Push handler AFTER setjmp returned 0 (normal path)
                    self.emit_line(&format!(
                        "call void @{}(i8* {})",
                        TEJX_PUSH_HANDLER, jmpbuf_ptr
                    ));
                }
            }

            let has_pop_handler = bb
                .instructions
                .iter()
                .any(|inst| matches!(inst, MIRInstruction::PopHandler { .. }));

            for inst in &bb.instructions {
                if bb.exception_handler.is_some()
                    && !has_pop_handler
                    && matches!(
                        inst,
                        MIRInstruction::Return { .. }
                            | MIRInstruction::Jump { .. }
                            | MIRInstruction::Branch { .. }
                    )
                {
                    self.emit_line("call void @tejx_pop_handler()");
                }
                self.gen_instruction_v2(inst, func, &bb.name, i);
            }

            if has_handler {
                // If the block is not terminated, we need to call tejx_try_end before the terminator
                // But MIR instructions usually end with a terminator (Jump/Branch/Return).
                // We should inject tejx_try_end BEFORE the terminator.
                // Wait, gen_instruction_v2 handles terminators.
                // I need to modify gen_instruction_v2 or handle it here if I see a terminator.
            }
        }

        self.emit("}\n\n");

        if !self.alloca_buffer.is_empty() {
            self.buffer.insert_str(entry_marker, &self.alloca_buffer);
            self.alloca_buffer.clear();
        }
    }
}
