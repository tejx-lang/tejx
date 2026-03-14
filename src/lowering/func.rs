use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::types::TejxType;
use std::collections::{HashMap, HashSet};

impl Lowering {
    pub(crate) fn is_concrete_type(&self, ty: &TejxType) -> bool {
        match ty {
            TejxType::Any => true,
            TejxType::Void
            | TejxType::Int16
            | TejxType::Int32
            | TejxType::Int64
            | TejxType::Int128
            | TejxType::Float16
            | TejxType::Float32
            | TejxType::Float64
            | TejxType::Bool
            | TejxType::String
            | TejxType::Char => true,
            TejxType::Class(name, generics) => {
                if name == "<inferred>" || name.starts_with("$MISSING_GENERIC_") {
                    return false;
                }
                if generics.is_empty() {
                    let looks_like_generic_param = name.len() <= 2
                        && name.chars().next().map_or(false, |c| c.is_uppercase())
                        && name.chars().all(|c| c.is_alphanumeric());
                    !looks_like_generic_param
                } else {
                    generics.iter().all(|generic| self.is_concrete_type(generic))
                }
            }
            TejxType::FixedArray(inner, _) | TejxType::DynamicArray(inner) | TejxType::Slice(inner) => {
                self.is_concrete_type(inner)
            }
            TejxType::Function(params, ret) => {
                params.iter().all(|param| self.is_concrete_type(param)) && self.is_concrete_type(ret)
            }
            TejxType::Union(types) => types.iter().all(|ty| self.is_concrete_type(ty)),
            TejxType::Object(props) => props
                .iter()
                .all(|(_, _, prop_ty)| self.is_concrete_type(prop_ty)),
        }
    }

    pub(crate) fn function_template_name(&self, lowered_name: &str) -> String {
        lowered_name
            .strip_prefix("f_")
            .unwrap_or(lowered_name)
            .to_string()
    }

    pub(crate) fn instantiation_key(&self, base_name: &str, concrete_args: &[TejxType]) -> String {
        self.monomorphized_name(base_name, concrete_args)
    }

    pub(crate) fn should_lower_function(&self, func: &FunctionDeclaration) -> bool {
        func.generic_params.is_empty()
            || self
                .erased_generic_functions
                .borrow()
                .contains(&format!("f_{}", func.name))
    }

    pub(crate) fn should_lower_class(&self, class_decl: &ClassDeclaration) -> bool {
        class_decl.generic_params.is_empty()
    }

    pub(crate) fn register_function(&self, func: &FunctionDeclaration) {
        let param_types: Vec<TejxType> = func
            .params
            .iter()
            .map(|p| TejxType::from_node(&p.type_name))
            .collect();
        let ret_type = TejxType::from_node(&func.return_type);

        let name = if func.is_extern {
            func.name.clone()
        } else {
            format!("f_{}", func.name)
        };

        if !func.generic_params.is_empty() {
            self.function_generic_params.borrow_mut().insert(
                name.clone(),
                func.generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect(),
            );
            if self.can_erase_generic_function(func) {
                self.erased_generic_functions.borrow_mut().insert(name.clone());
            }
        }

        self.user_functions.borrow_mut().insert(
            name.clone(),
            TejxType::Function(param_types, Box::new(ret_type)),
        );
        self.user_function_args
            .borrow_mut()
            .insert(name, func.params.len());
        if func.is_extern {
            self.extern_functions.borrow_mut().insert(func.name.clone());
        }
    }

    pub(crate) fn substitute_generics(
        &self,
        ret_ty: &TejxType,
        obj_type: &TejxType,
        callee_name: &str,
    ) -> TejxType {
        let type_name = ret_ty.to_name();

        // Find base class name from object type
        let obj_full = obj_type.to_name();
        let base_class = obj_full
            .split('<')
            .next()
            .unwrap_or(&obj_full)
            .trim()
            .to_string();

        // Try getting params from class or function
        let mut params = Vec::new();
        if matches!(obj_type, TejxType::Class(_, _)) {
            if let Some(p) = self.class_generic_params.borrow().get(&base_class) {
                params = p.clone();
            }
        }
        if params.is_empty() {
            if let Some(p) = self.function_generic_params.borrow().get(callee_name) {
                params = p.clone();
            }
        }
        if params.is_empty() && obj_type.is_array() {
            params = vec!["T".to_string()];
        }

        if params.is_empty() {
            return ret_ty.clone();
        }

        // Extract concrete type args
        let concrete_args = if obj_type.is_array() {
            vec![obj_type.get_array_element_type().to_name()]
        } else if let Some(open) = obj_full.find('<') {
            if let Some(close) = obj_full.rfind('>') {
                let inner = &obj_full[open + 1..close];
                let mut args = Vec::new();
                let mut start = 0;
                let mut depth = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        ',' if depth == 0 => {
                            args.push(inner[start..i].trim().to_string());
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                args.push(inner[start..].trim().to_string());
                args
            } else {
                return ret_ty.clone();
            }
        } else {
            return ret_ty.clone();
        };

        let mut bindings = std::collections::HashMap::new();
        for (i, param) in params.iter().enumerate() {
            if i < concrete_args.len() {
                bindings.insert(param.clone(), TejxType::from_name(&concrete_args[i]));
            }
        }

        ret_ty.substitute_generics(&bindings)
    }

    fn sanitize_monomorph_part(&self, raw: &str) -> String {
        raw.replace("[]", "_arr")
            .replace('<', "_")
            .replace('>', "_")
            .replace(", ", "_")
            .replace(',', "_")
            .replace(" | ", "_or_")
            .replace('|', "_")
            .replace(" & ", "_and_")
            .replace('&', "_")
            .replace("{ ", "_obj_")
            .replace(" }", "_")
            .replace('{', "_")
            .replace('}', "_")
            .replace(": ", "_")
            .replace(':', "_")
            .replace("; ", "_")
            .replace(';', "_")
            .replace(' ', "_")
    }

    pub(crate) fn monomorphized_name(&self, base_name: &str, concrete_args: &[TejxType]) -> String {
        let mut mangled = base_name.split('<').next().unwrap_or(base_name).trim().to_string();
        for arg in concrete_args {
            mangled.push('_');
            mangled.push_str(&self.sanitize_monomorph_part(&arg.to_name()));
        }
        mangled
    }

    pub(crate) fn monomorphized_class_name(&self, ty: &TejxType) -> String {
        match ty {
            TejxType::Class(name, generics) if !generics.is_empty() => {
                self.monomorphized_name(name, generics)
            }
            TejxType::Class(name, _) => {
                if let Some(insts) = self.generic_instantiations.borrow().get(name) {
                    if insts.len() == 1 {
                        if let Some(args) = insts.iter().next() {
                            return self.monomorphized_name(name, args);
                        }
                    }
                }
                name.clone()
            }
            _ => ty.to_name(),
        }
    }

    pub(crate) fn collect_generic_bindings(
        &self,
        formal: &TejxType,
        actual: &TejxType,
        generic_params: &HashSet<String>,
        bindings: &mut HashMap<String, TejxType>,
    ) {
        match formal {
            TejxType::Class(name, generics) if generics.is_empty() && generic_params.contains(name) => {
                bindings.entry(name.clone()).or_insert_with(|| actual.clone());
            }
            TejxType::Class(name, formal_generics) => {
                if let TejxType::Class(actual_name, actual_generics) = actual {
                    if name == actual_name && formal_generics.len() == actual_generics.len() {
                        for (formal_arg, actual_arg) in formal_generics.iter().zip(actual_generics.iter()) {
                            self.collect_generic_bindings(formal_arg, actual_arg, generic_params, bindings);
                        }
                    }
                }
            }
            TejxType::DynamicArray(formal_inner) => match actual {
                TejxType::DynamicArray(actual_inner)
                | TejxType::FixedArray(actual_inner, _)
                | TejxType::Slice(actual_inner) => {
                    self.collect_generic_bindings(formal_inner, actual_inner, generic_params, bindings);
                }
                _ => {}
            },
            TejxType::FixedArray(formal_inner, _) | TejxType::Slice(formal_inner) => match actual {
                TejxType::DynamicArray(actual_inner)
                | TejxType::FixedArray(actual_inner, _)
                | TejxType::Slice(actual_inner) => {
                    self.collect_generic_bindings(formal_inner, actual_inner, generic_params, bindings);
                }
                _ => {}
            },
            TejxType::Function(formal_params, formal_ret) => {
                if let TejxType::Function(actual_params, actual_ret) = actual {
                    for (formal_param, actual_param) in formal_params.iter().zip(actual_params.iter()) {
                        self.collect_generic_bindings(formal_param, actual_param, generic_params, bindings);
                    }
                    self.collect_generic_bindings(formal_ret, actual_ret, generic_params, bindings);
                }
            }
            TejxType::Union(formal_members) => {
                for formal_member in formal_members {
                    self.collect_generic_bindings(formal_member, actual, generic_params, bindings);
                }
            }
            TejxType::Object(formal_props) => {
                if let TejxType::Object(actual_props) = actual {
                    for (formal_key, _, formal_ty) in formal_props {
                        if let Some((_, _, actual_ty)) =
                            actual_props.iter().find(|(actual_key, _, _)| actual_key == formal_key)
                        {
                            self.collect_generic_bindings(formal_ty, actual_ty, generic_params, bindings);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn resolve_function_monomorph(
        &self,
        callee_name: &str,
        args: &[HIRExpression],
        explicit_type_args: Option<&Vec<TypeNode>>,
    ) -> Option<(String, HashMap<String, TejxType>)> {
        if callee_name.starts_with("rt_")
            || callee_name.starts_with("std_")
            || callee_name.starts_with("tejx_")
            || self.extern_functions.borrow().contains(callee_name)
        {
            return None;
        }
        if self.erased_generic_functions.borrow().contains(callee_name) {
            return None;
        }

        let generic_params = self
            .function_generic_params
            .borrow()
            .get(callee_name)
            .cloned()?;
        if generic_params.is_empty() {
            return None;
        }

        let generic_set: HashSet<String> = generic_params.iter().cloned().collect();
        let mut bindings = HashMap::new();

        if let Some(type_args) = explicit_type_args {
            if type_args.len() != generic_params.len() {
                return None;
            }
            for (generic_name, type_arg) in generic_params.iter().zip(type_args.iter()) {
                bindings.insert(generic_name.clone(), TejxType::from_node(type_arg));
            }
        } else {
            let function_type = self.user_functions.borrow().get(callee_name).cloned()?;
            let TejxType::Function(param_types, _) = function_type else {
                return None;
            };

            for (formal, actual) in param_types.iter().zip(args.iter().map(|arg| arg.get_type())) {
                self.collect_generic_bindings(formal, &actual, &generic_set, &mut bindings);
            }
        }

        if generic_params
            .iter()
            .any(|generic_name| !bindings.contains_key(generic_name))
        {
            return None;
        }

        let ordered_args: Vec<TejxType> = generic_params
            .iter()
            .filter_map(|generic_name| bindings.get(generic_name).cloned())
            .collect();

        if ordered_args.iter().any(|arg| !self.is_concrete_type(arg)) {
            return None;
        }

        self.discovered_function_instantiations
            .borrow_mut()
            .entry(self.function_template_name(callee_name))
            .or_default()
            .insert(ordered_args.clone());

        Some((self.monomorphized_name(callee_name, &ordered_args), bindings))
    }

    fn can_erase_generic_function(&self, func: &FunctionDeclaration) -> bool {
        fn contains_generic(node: &TypeNode, generic_names: &HashSet<&str>) -> bool {
            match node {
                TypeNode::Named(name) => generic_names.contains(name.as_str()),
                TypeNode::Generic(name, args) => {
                    generic_names.contains(name.as_str())
                        || args
                            .iter()
                            .any(|arg| contains_generic(arg, generic_names))
                }
                TypeNode::Array(inner) => contains_generic(inner, generic_names),
                TypeNode::Function(params, ret) => {
                    params
                        .iter()
                        .any(|param| contains_generic(param, generic_names))
                        || contains_generic(ret, generic_names)
                }
                TypeNode::Object(members) => members
                    .iter()
                    .any(|(_, _, member_ty)| contains_generic(member_ty, generic_names)),
                TypeNode::Union(types) | TypeNode::Intersection(types) => {
                    types.iter().any(|ty| contains_generic(ty, generic_names))
                }
                TypeNode::Any => false,
            }
        }

        if func.generic_params.is_empty() {
            return false;
        }

        let generic_names: HashSet<&str> =
            func.generic_params.iter().map(|param| param.name.as_str()).collect();

        !contains_generic(&func.return_type, &generic_names)
    }

    pub(crate) fn lower_function_declaration(
        &self,
        func: &FunctionDeclaration,
        functions: &mut Vec<HIRStatement>,
    ) {
        let line = func._line;
        if self.async_enabled && func._is_async {
            self.lower_async_function(func, functions);
            return;
        }

        let params: Vec<(String, TejxType)> = func
            .params
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    self.resolve_alias_type(&TejxType::from_node(&p.type_name)),
                )
            })
            .collect();
        let return_type = self.resolve_alias_type(&TejxType::from_node(&func.return_type));

        self.enter_scope();
        let mangled_params: Vec<(String, TejxType)> = params
            .iter()
            .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
            .collect();

        let body = self
            .lower_statement(&func.body)
            .unwrap_or(HIRStatement::Block {
                line: line,
                statements: vec![],
            });

        self._exit_scope();

        let name = format!("f_{}", func.name);
        functions.push(HIRStatement::Function {
            async_params: None,
            line: line,
            name,
            params: mangled_params,
            _return_type: return_type,
            body: Box::new(body),
            is_extern: func.is_extern,
        });
    }
}
