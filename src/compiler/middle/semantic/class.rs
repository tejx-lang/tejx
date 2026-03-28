use super::*;
use crate::common::builtins;
use crate::frontend::ast::*;
use std::collections::{HashMap, HashSet};

impl TypeChecker {
    pub(crate) fn member_signature_matches_strict(
        &self,
        expected: &TejxType,
        actual: &TejxType,
    ) -> bool {
        let normalize_callable = |ty: &TejxType| -> TejxType {
            match ty {
                TejxType::Class(name, generics)
                    if generics.is_empty()
                        && (name.starts_with("function:") || name.contains("=>")) =>
                {
                    let (ret, params, _) = self.parse_signature(name.clone());
                    let actual_ret = ret
                        .split_once(':')
                        .map(|(_, ret_ty)| ret_ty)
                        .unwrap_or(ret.as_str());
                    TejxType::Function(
                        params.iter().map(|param| TejxType::from_name(param)).collect(),
                        Box::new(TejxType::from_name(actual_ret)),
                    )
                }
                other => other.clone(),
            }
        };

        let expected = normalize_callable(expected);
        let actual = normalize_callable(actual);

        match (&expected, &actual) {
            (
                TejxType::Function(expected_params, expected_ret),
                TejxType::Function(actual_params, actual_ret),
            ) => {
                expected_params.len() == actual_params.len()
                    && expected_params.iter().zip(actual_params.iter()).all(
                        |(expected_param, actual_param)| {
                            self.are_types_compatible(expected_param, actual_param)
                                && self.are_types_compatible(actual_param, expected_param)
                        },
                    )
                    && self.are_types_compatible(expected_ret, actual_ret)
                    && self.are_types_compatible(actual_ret, expected_ret)
            }
            _ => {
                self.are_types_compatible(&expected, &actual)
                    && self.are_types_compatible(&actual, &expected)
            }
        }
    }

    pub(crate) fn access_rank(&self, access: &AccessLevel) -> usize {
        match access {
            AccessLevel::Private => 0,
            AccessLevel::Protected => 1,
            AccessLevel::Public => 2,
        }
    }

    pub(crate) fn function_signature_type(&self, func: &FunctionDeclaration) -> TejxType {
        let ret_ty_str = func.return_type.to_string();
        let mut ret_ty = if ret_ty_str.is_empty() {
            "void".to_string()
        } else {
            ret_ty_str
        };
        if func._is_async && !ret_ty.starts_with("Promise<") && ret_ty != "Promise" {
            ret_ty = format!("Promise<{}>", ret_ty);
        }

        let params = func
            .params
            .iter()
            .map(|param| {
                let mut param_ty = param.type_name.to_string();
                if param_ty.is_empty() {
                    param_ty = "<inferred>".to_string();
                }
                TejxType::from_name(&param_ty)
            })
            .collect();

        TejxType::Function(params, Box::new(TejxType::from_name(&ret_ty)))
    }

    pub(crate) fn callable_return_type(&self, ty: &TejxType) -> Option<TejxType> {
        match ty {
            TejxType::Function(_, ret) => Some((**ret).clone()),
            TejxType::Class(name, generics)
                if generics.is_empty() && (name.starts_with("function:") || name.contains("=>")) =>
            {
                let (ret, _, _) = self.parse_signature(name.clone());
                let actual_ret = ret
                    .split_once(':')
                    .map(|(_, ret_ty)| ret_ty)
                    .unwrap_or(ret.as_str());
                Some(TejxType::from_name(actual_ret))
            }
            _ => None,
        }
    }

    pub(crate) fn effective_async_return_type(&self, ty: TejxType, is_async: bool) -> TejxType {
        if is_async
            && !matches!(ty, TejxType::Class(ref name, _) if name == "Promise")
        {
            TejxType::Class("Promise".to_string(), vec![ty])
        } else {
            ty
        }
    }

    pub(crate) fn remember_inferred_function_return(
        &mut self,
        func: &FunctionDeclaration,
        return_ty: &TejxType,
    ) {
        if func.return_type.to_string().is_empty() && return_ty.to_name() != "<inferred>" {
            self.inferred_function_returns.insert(
                (
                    self.current_file.clone(),
                    func._line,
                    func._col,
                    func.name.clone(),
                ),
                return_ty.clone(),
            );
        }
    }

    pub(crate) fn remember_inferred_member_return(
        &mut self,
        owner_name: &str,
        func: &FunctionDeclaration,
        return_ty: &TejxType,
    ) {
        if func.return_type.to_string().is_empty() && return_ty.to_name() != "<inferred>" {
            self.inferred_member_returns.insert(
                (
                    self.current_file.clone(),
                    owner_name.to_string(),
                    func._line,
                    func._col,
                    func.name.clone(),
                ),
                return_ty.clone(),
            );
        }
    }

    fn class_this_type(&self, class_decl: &ClassDeclaration) -> String {
        if class_decl.generic_params.is_empty() {
            class_decl.name.clone()
        } else {
            let gp_names: Vec<String> = class_decl
                .generic_params
                .iter()
                .map(|gp| gp.name.clone())
                .collect();
            format!("{}<{}>", class_decl.name, gp_names.join(", "))
        }
    }

    fn infer_return_type_from_body(
        &mut self,
        params: &[Parameter],
        body: &Statement,
        generic_params: &[GenericParam],
        is_async: bool,
    ) -> TejxType {
        let diagnostics_len = self.diagnostics.len();
        let prev_return = self.current_function_return.take();
        let prev_async = self.current_function_is_async;

        self.enter_scope();
        for gp in generic_params {
            self.define(gp.name.clone(), gp.name.clone());
        }
        for param in params {
            let mut param_ty = param.type_name.to_string();
            if param_ty.is_empty() {
                param_ty = "<inferred>".to_string();
            }
            self.define(param.name.clone(), param_ty);
        }

        self.current_function_return = Some(TejxType::from_name("<inferred>"));
        self.current_function_is_async = is_async;
        let _ = self.check_statement(body);

        let inferred = self
            .current_function_return
            .take()
            .unwrap_or(TejxType::Void);

        self.current_function_return = prev_return;
        self.current_function_is_async = prev_async;
        self.exit_scope();
        self.diagnostics.truncate(diagnostics_len);

        let inferred = if inferred == TejxType::from_name("<inferred>") {
            TejxType::Void
        } else {
            inferred
        };

        self.effective_async_return_type(inferred, is_async)
    }

    pub(crate) fn update_function_symbol_return_type(
        &mut self,
        name: &str,
        return_ty: TejxType,
    ) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(symbol) = scope.get_mut(name) {
                if let TejxType::Function(params, existing_ret) = &symbol.ty {
                    if existing_ret.as_ref() != &return_ty {
                        symbol.ty = TejxType::Function(params.clone(), Box::new(return_ty));
                        return true;
                    }
                }
                break;
            }
        }
        false
    }

    pub(crate) fn update_class_member_return_type(
        &mut self,
        class_name: &str,
        member_name: &str,
        return_ty: TejxType,
    ) -> bool {
        let existing_ty = self
            .class_members
            .get(class_name)
            .and_then(|members| members.get(member_name))
            .map(|info| info.ty.clone());

        let Some(existing_ty) = existing_ty else {
            return false;
        };

        let params = match &existing_ty {
            TejxType::Function(params, _) => params.clone(),
            _ => {
                let ty_name = existing_ty.to_name();
                if ty_name.starts_with("function:") || ty_name.contains("=>") {
                    let (_, param_names, _) = self.parse_signature(ty_name);
                    param_names
                        .iter()
                        .map(|param| TejxType::from_name(param))
                        .collect()
                } else {
                    Vec::new()
                }
            }
        };

        let new_ty = TejxType::Function(params, Box::new(return_ty));
        if let Some(members) = self.class_members.get_mut(class_name) {
            if let Some(info) = members.get_mut(member_name) {
                if info.ty != new_ty {
                    info.ty = new_ty;
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn refine_inferred_return_types(&mut self, stmt: &Statement) -> bool {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                if !func.return_type.to_string().is_empty() {
                    return false;
                }
                let inferred = self.infer_return_type_from_body(
                    &func.params,
                    &func.body,
                    &func.generic_params,
                    func._is_async,
                );
                self.remember_inferred_function_return(func, &inferred);
                self.update_function_symbol_return_type(&func.name, inferred)
            }
            Statement::ClassDeclaration(class_decl) => {
                let prev_class = self.current_class.clone();
                let prev_member_static = self.current_member_is_static;
                let prev_inside_constructor = self.current_inside_constructor;
                self.current_class = Some(class_decl.name.clone());
                self.current_member_is_static = false;
                self.current_inside_constructor = false;

                self.enter_scope();
                self.define("this".to_string(), self.class_this_type(class_decl));
                for gp in &class_decl.generic_params {
                    self.define(gp.name.clone(), gp.name.clone());
                }
                if !class_decl._parent_name.is_empty() {
                    self.define("super".to_string(), class_decl._parent_name.clone());
                }

                let mut changed = false;
                for method in &class_decl.methods {
                    if !method.func.return_type.to_string().is_empty() {
                        continue;
                    }
                    self.current_member_is_static = method.is_static;
                    let inferred = self.infer_return_type_from_body(
                        &method.func.params,
                        &method.func.body,
                        &method.func.generic_params,
                        method.func._is_async,
                    );
                    self.remember_inferred_member_return(
                        &class_decl.name,
                        &method.func,
                        &inferred,
                    );
                    changed |= self.update_class_member_return_type(
                        &class_decl.name,
                        &method.func.name,
                        inferred,
                    );
                }

                self.exit_scope();
                self.current_class = prev_class;
                self.current_member_is_static = prev_member_static;
                self.current_inside_constructor = prev_inside_constructor;
                changed
            }
            Statement::ExtensionDeclaration(ext_decl) => {
                let prev_class = self.current_class.clone();
                let prev_member_static = self.current_member_is_static;
                let prev_inside_constructor = self.current_inside_constructor;
                self.current_class = Some(ext_decl._target_type.to_string());
                self.current_member_is_static = false;
                self.current_inside_constructor = false;

                self.enter_scope();
                self.define("this".to_string(), ext_decl._target_type.to_string());

                let mut changed = false;
                for method in &ext_decl._methods {
                    if !method.return_type.to_string().is_empty() {
                        continue;
                    }
                    let inferred = self.infer_return_type_from_body(
                        &method.params,
                        &method.body,
                        &method.generic_params,
                        method._is_async,
                    );
                    self.remember_inferred_member_return(
                        &ext_decl._target_type.to_string(),
                        method,
                        &inferred,
                    );
                    changed |= self.update_class_member_return_type(
                        &ext_decl._target_type.to_string(),
                        &method.name,
                        inferred,
                    );
                }

                self.exit_scope();
                self.current_class = prev_class;
                self.current_member_is_static = prev_member_static;
                self.current_inside_constructor = prev_inside_constructor;
                changed
            }
            Statement::ExportDecl { declaration, .. } => self.refine_inferred_return_types(declaration),
            _ => false,
        }
    }

    pub(crate) fn base_class_name<'a>(&self, class_name: &'a str) -> &'a str {
        class_name.split('<').next().unwrap_or(class_name)
    }

    pub(crate) fn parent_chain(&self, class_name: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut current = self.base_class_name(class_name).to_string();

        while let Some(parent) = self.class_hierarchy.get(&current) {
            let parent_base = self.base_class_name(parent).to_string();
            if !visited.insert(parent_base.clone()) {
                break;
            }
            chain.push(parent_base.clone());
            current = parent_base;
        }

        chain
    }

    pub(crate) fn is_same_or_subclass(&self, child: &str, ancestor: &str) -> bool {
        let child_base = self.base_class_name(child);
        let ancestor_base = self.base_class_name(ancestor);
        child_base == ancestor_base
            || self
                .parent_chain(child_base)
                .iter()
                .any(|parent| parent == ancestor_base)
    }

    pub(crate) fn detect_inheritance_cycle(&self, class_name: &str) -> Option<Vec<String>> {
        let mut order = Vec::new();
        let mut seen = HashMap::new();
        let mut current = self.base_class_name(class_name).to_string();

        loop {
            if let Some(&start_idx) = seen.get(&current) {
                let mut cycle = order[start_idx..].to_vec();
                cycle.push(current);
                return Some(cycle);
            }

            seen.insert(current.clone(), order.len());
            order.push(current.clone());

            let parent = self.class_hierarchy.get(&current)?;
            current = self.base_class_name(parent).to_string();
        }
    }

    pub(crate) fn report_inheritance_cycle_if_needed(
        &mut self,
        class_name: &str,
        line: usize,
        col: usize,
    ) -> bool {
        if let Some(cycle) = self.detect_inheritance_cycle(class_name) {
            self.report_error_detailed(
                format!(
                    "Circular inheritance detected: {}",
                    cycle.join(" -> ")
                ),
                line,
                col,
                "E0111",
                Some("Break the cycle so every class has a finite parent chain"),
            );
            return true;
        }

        false
    }

    pub(crate) fn collect_declarations(&mut self, stmt: &Statement) {
        match stmt {
            Statement::ClassDeclaration(class_decl) => {
                if self.scopes.len() > 1 {
                    self.report_error_detailed(
                        format!(
                            "Class '{}' cannot be defined inside a function or block",
                            class_decl.name
                        ),
                        class_decl._line,
                        class_decl._col,
                        "E0114",
                        Some("Move the class definition to the top level of the file"),
                    );
                }
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        class_decl.name.clone(),
                        Symbol {
                            ty: TejxType::from_name("class"),
                            is_const: false,
                            is_narrowed: false,
                            params: Vec::new(),
                            min_params: None,
                            is_variadic: false,
                            aliased_type: None,
                            generic_params: class_decl.generic_params.clone(),
                            literal_length: None,
                            declared_in_file: self.current_file.clone(),
                        },
                    );
                }
                if class_decl._is_abstract {
                    self.abstract_classes.insert(class_decl.name.clone());
                }
                if !class_decl._parent_name.is_empty() {
                    self.class_hierarchy
                        .insert(class_decl.name.clone(), class_decl._parent_name.clone());
                }
                let mut members = HashMap::new();
                for m in &class_decl._members {
                    let mut member_ty = m._type_name.to_string();
                    if member_ty.is_empty() {
                        member_ty = "<inferred>".to_string();
                    }
                    members.insert(
                        m._name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(
                                &self.parameterize_generics(&member_ty, &class_decl.generic_params),
                            ),
                            is_static: m._is_static,
                            access: match m._access {
                                crate::frontend::ast::AccessModifier::Private => {
                                    AccessLevel::Private
                                }
                                crate::frontend::ast::AccessModifier::Protected => {
                                    AccessLevel::Protected
                                }
                                _ => AccessLevel::Public,
                            },
                            is_readonly: false,
                            generic_params: Vec::new(),
                        },
                    );
                }
                for method in &class_decl.methods {
                    let ret_ty_str = method.func.return_type.to_string();
                    let mut ret_ty = if ret_ty_str.is_empty() {
                        "<inferred>".to_string()
                    } else {
                        ret_ty_str
                    };
                    if ret_ty != "<inferred>"
                        && method.func._is_async
                        && !ret_ty.starts_with("Promise<")
                        && ret_ty != "Promise"
                    {
                        ret_ty = format!("Promise<{}>", ret_ty);
                    }
                    let mut param_types = Vec::new();
                    for p in &method.func.params {
                        let mut p_ty = p.type_name.to_string();
                        if p_ty.is_empty() {
                            p_ty = "<inferred>".to_string();
                        }
                        param_types.push(p_ty);
                    }
                    let p_str = param_types.join(",");
                    let sig_str = if p_str.is_empty() {
                        format!("function:{}", ret_ty)
                    } else {
                        format!("function:{}:{}", ret_ty, p_str)
                    };
                    let (final_type, final_params, _) = self.parse_signature(sig_str);
                    let full_sig = if final_params.is_empty() {
                        final_type
                    } else {
                        format!("{}:{}", final_type, final_params.join(","))
                    };
                    let parameterized_type = self
                        .parameterize_generics(&full_sig.to_string(), &class_decl.generic_params);
                    members.insert(
                        method.func.name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(&parameterized_type),
                            is_static: method.is_static,
                            access: match method._access {
                                crate::frontend::ast::AccessModifier::Private => {
                                    AccessLevel::Private
                                }
                                crate::frontend::ast::AccessModifier::Protected => {
                                    AccessLevel::Protected
                                }
                                _ => AccessLevel::Public,
                            },
                            is_readonly: true, // Methods are readonly
                            generic_params: method.func.generic_params.clone(),
                        },
                    );
                }
                for getter in &class_decl._getters {
                    members.insert(
                        getter._name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(&self.parameterize_generics(
                                &getter._return_type.to_string(),
                                &class_decl.generic_params,
                            )),
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true, // Default to readonly, setter can clear it
                            generic_params: Vec::new(),
                        },
                    );
                }
                for setter in &class_decl._setters {
                    if let Some(existing) = members.get_mut(&setter._name) {
                        existing.is_readonly = false;
                    } else {
                        members.insert(
                            setter._name.clone(),
                            MemberInfo {
                                ty: TejxType::from_name(&self.parameterize_generics(
                                    &setter._param_type.to_string(),
                                    &class_decl.generic_params,
                                )), // or void?
                                is_static: false,
                                access: AccessLevel::Public,
                                is_readonly: false,
                                generic_params: Vec::new(),
                            },
                        );
                    }
                }
                if let Some(constructor) = &class_decl._constructor {
                    let mut param_types = Vec::new();
                    for p in &constructor.params {
                        let mut p_ty = p.type_name.to_string();
                        if p_ty.is_empty() {
                            p_ty = "<inferred>".to_string();
                        }
                        p_ty = self.parameterize_generics(&p_ty, &class_decl.generic_params);
                        param_types.push(p_ty);
                    }
                    let mut params = Vec::new();
                    for p in &param_types {
                        params.push(TejxType::from_name(p));
                    }
                    members.insert(
                        "constructor".to_string(),
                        MemberInfo {
                            ty: TejxType::Function(params, Box::new(TejxType::Void)),
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true,
                            generic_params: Vec::new(),
                        },
                    );
                }
                self.class_members.insert(class_decl.name.clone(), members);
            }
            Statement::FunctionDeclaration(func) => {
                let ret_ty_str = func.return_type.to_string();
                let mut ret_ty = if ret_ty_str.is_empty() {
                    "<inferred>".to_string()
                } else {
                    ret_ty_str
                };
                if ret_ty != "<inferred>"
                    && func._is_async
                    && !ret_ty.starts_with("Promise<")
                    && ret_ty != "Promise"
                {
                    ret_ty = format!("Promise<{}>", ret_ty);
                }
                let mut is_variadic = false;
                let min_required = func
                    .params
                    .iter()
                    .filter(|p| p._default_value.is_none() && !p._is_rest)
                    .count();
                let params = func
                    .params
                    .iter()
                    .map(|p| {
                        if p._is_rest {
                            is_variadic = true;
                        }
                        let mut p_ty = p.type_name.to_string();
                        if p_ty.is_empty() {
                            p_ty = "<inferred>".to_string();
                        }
                        TejxType::from_name(&p_ty)
                    })
                    .collect::<Vec<TejxType>>();
                let has_defaults = min_required < params.len();
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        func.name.clone(),
                        Symbol {
                            ty: TejxType::Function(
                                params.clone(),
                                Box::new(TejxType::from_name(&ret_ty)),
                            ),
                            is_const: false,
                            is_narrowed: false,
                            params,
                            min_params: if has_defaults {
                                Some(min_required)
                            } else {
                                None
                            },
                            is_variadic,
                            aliased_type: None,
                            generic_params: func.generic_params.clone(),
                            literal_length: None,
                            declared_in_file: self.current_file.clone(),
                        },
                    );
                }
            }
            Statement::TypeAliasDeclaration {
                name, _type_def, ..
            } => {
                // self.define(name.clone(), "type".to_string());
                // Handle alias definition manually to set aliased_type
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        name.clone(),
                        Symbol {
                            ty: TejxType::from_name("type"),
                            is_const: true,
                            is_narrowed: false,
                            params: Vec::new(),
                            min_params: None,
                            is_variadic: false,
                            aliased_type: { Some(TejxType::from_node(_type_def)) },
                            generic_params: Vec::new(),
                            literal_length: None,
                            declared_in_file: self.current_file.clone(),
                        },
                    );
                }
            }
            Statement::EnumDeclaration(enum_decl) => {
                self.define(enum_decl.name.clone(), "enum".to_string());
                let mut members = HashMap::new();
                for member in &enum_decl._members {
                    members.insert(
                        member._name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(&enum_decl.name),
                            is_static: true,
                            access: AccessLevel::Public,
                            is_readonly: true, // Enum members are constants
                            generic_params: Vec::new(),
                        },
                    );
                }
                self.class_members.insert(enum_decl.name.clone(), members);
            }
            // Statement::ProtocolDeclaration(proto) => {
            //     self.define(proto._name.clone(), "protocol".to_string());
            //     self.interfaces.insert(proto._name.clone(), proto._methods.iter().map(|m| m._name.clone()).collect());
            // }
            Statement::InterfaceDeclaration {
                name,
                _methods: methods,
                ..
            } => {
                self.define(name.clone(), "interface".to_string());
                let mut interface_methods = HashMap::new();
                for m in methods {
                    // Extract method info
                    let mut param_types = Vec::new();
                    for p in &m._params {
                        param_types.push(p.type_name.to_string());
                    }
                    let p_str = param_types.join(",");
                    let type_str = format!("function:{}:{}", m._return_type.to_string(), p_str);
                    interface_methods.insert(
                        m._name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(&type_str),
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true,
                            generic_params: Vec::new(),
                        },
                    );
                }
                self.interfaces.insert(name.clone(), interface_methods);
            }
            Statement::ImportDecl {
                _names, source: _, ..
            } => {
                // Stdlib files will be processed as normal TejX files through explicit inclusion
            }
            Statement::ExportDecl { declaration, .. } => {
                self.collect_declarations(declaration);
            }
            _ => {}
        }
    }

    fn normalize_member_container(&self, obj_type: &str) -> String {
        let mut unwrapped_type = obj_type.to_string();
        if obj_type.contains('|') {
            unwrapped_type = obj_type
                .split('|')
                .map(|s| s.trim().to_string())
                .find(|s| s != "None" && !s.is_empty())
                .unwrap_or(obj_type.to_string());
        }

        let mut current_type = if unwrapped_type == "enum"
            || self
                .lookup(&unwrapped_type)
                .map(|s| s.ty.to_name() == "enum")
                .unwrap_or(false)
        {
            unwrapped_type.clone()
        } else {
            unwrapped_type.clone()
        };

        // Normalize generic types: Node<int> -> Node
        if !self.class_members.contains_key(&current_type) {
            if let Some(angle) = current_type.find('<') {
                current_type = current_type[..angle].to_string();
            }
        }

        current_type
    }

    pub(crate) fn collect_member_names(&self, obj_type: &str, want_static: bool) -> Vec<String> {
        let mut current_type = self.normalize_member_container(obj_type);
        let mut names = std::collections::HashSet::new();
        let obj_ty = TejxType::from_name(obj_type);

        if !want_static {
            if let Some(builtin_names) = builtins::member_names(&obj_ty) {
                for name in builtin_names {
                    names.insert(name.to_string());
                }
            }
            if let TejxType::Object(props) = obj_ty {
                for (name, _, _) in props {
                    names.insert(name);
                }
            }
        }

        while !current_type.is_empty() && current_type != "<inferred>" {
            if let Some(members) = self.class_members.get(&current_type) {
                for (name, info) in members {
                    if info.is_static == want_static {
                        names.insert(name.clone());
                    }
                }
            }

            if !want_static {
                if let Some(members) = self.interfaces.get(&current_type) {
                    for name in members.keys() {
                        names.insert(name.clone());
                    }
                }
            }

            if let Some(parent) = self.class_hierarchy.get(&current_type) {
                current_type = parent.clone();
            } else {
                break;
            }
        }

        let mut list: Vec<String> = names.into_iter().collect();
        list.sort();
        list
    }

    pub(crate) fn resolve_instance_member(
        &self,
        obj_type: &str,
        member: &str,
    ) -> Option<MemberInfo> {
        self.resolve_instance_member_with_owner(obj_type, member)
            .map(|(_, info)| info)
    }

    pub(crate) fn resolve_instance_member_with_owner(
        &self,
        obj_type: &str,
        member: &str,
    ) -> Option<(String, MemberInfo)> {
        let obj_ty = TejxType::from_name(obj_type);
        if let TejxType::Object(props) = obj_ty {
            if let Some((_, _, ty)) = props.into_iter().find(|(name, _, _)| name == member) {
                return Some((
                    String::new(),
                    MemberInfo {
                        ty,
                        is_static: false,
                        access: AccessLevel::Public,
                        is_readonly: false,
                        generic_params: Vec::new(),
                    },
                ));
            }
        }
        let mut current_type = self.normalize_member_container(obj_type);

        while !current_type.is_empty() && current_type != "<inferred>" {
            if let Some(members) = self.class_members.get(&current_type) {
                if let Some(info) = members.get(member).cloned() {
                    return Some((current_type.clone(), info));
                }
            }

            // Check if it's an interface
            if let Some(members) = self.interfaces.get(&current_type) {
                if let Some(info) = members.get(member).cloned() {
                    return Some((current_type.clone(), info));
                }
            }

            // Follow hierarchy
            if let Some(parent) = self.class_hierarchy.get(&current_type) {
                current_type = parent.clone();
            } else {
                break;
            }
        }
        None
    }

    pub(crate) fn is_member_accessible_from_current_class(
        &self,
        declaring_type: &str,
        access: &AccessLevel,
    ) -> bool {
        match access {
            AccessLevel::Public => true,
            AccessLevel::Private => {
                let declaring_base = declaring_type.split('<').next().unwrap_or(declaring_type);
                self.current_class
                    .as_ref()
                    .map(|c| c.split('<').next().unwrap_or(c))
                    == Some(declaring_base)
            }
            AccessLevel::Protected => {
                let declaring_base = declaring_type.split('<').next().unwrap_or(declaring_type);
                if let Some(current) = &self.current_class {
                    let current_base = current.split('<').next().unwrap_or(current);
                    self.is_same_or_subclass(current_base, declaring_base)
                } else {
                    false
                }
            }
        }
    }
}
