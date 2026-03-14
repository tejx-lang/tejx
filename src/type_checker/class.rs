use super::*;
use crate::builtins;
use crate::ast::*;
use std::collections::HashMap;

impl TypeChecker {
    pub(crate) fn collect_declarations(&mut self, stmt: &Statement) {
        match stmt {
            Statement::ClassDeclaration(class_decl) => {
                if self.scopes.len() > 1 {
                    self.report_error_detailed(
                        format!("Class '{}' cannot be defined inside a function or block", class_decl.name),
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
                            params: Vec::new(),
                            min_params: None,
                            is_variadic: false,
                            aliased_type: None,
                            generic_params: class_decl.generic_params.clone(),
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
                            ty: TejxType::from_name(&self.parameterize_generics(
                                &member_ty,
                                &class_decl.generic_params,
                            )),
                            is_static: m._is_static,
                            access: if m._access == crate::ast::AccessModifier::Private {
                                AccessLevel::Private
                            } else {
                                AccessLevel::Public
                            },
                            is_readonly: false,
                            generic_params: Vec::new(),
                        },
                    );
                }
                for method in &class_decl.methods {
                    let ret_ty_str = method.func.return_type.to_string();
                    let mut ret_ty = if ret_ty_str.is_empty() {
                        "void".to_string()
                    } else {
                        ret_ty_str
                    };
                    if method.func._is_async
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
                            access: if method._access == crate::ast::AccessModifier::Private {
                                AccessLevel::Private
                            } else {
                                AccessLevel::Public
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
                        param_types.push(p_ty);
                    }
                    let p_str = param_types.join(",");
                    let sig_str = if p_str.is_empty() {
                        "function:void".to_string()
                    } else {
                        format!("function:void:{}", p_str)
                    };
                    members.insert(
                        "constructor".to_string(),
                        MemberInfo {
                            ty: TejxType::from_name(&sig_str),
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
                let mut ret_ty = if ret_ty_str == "any" || ret_ty_str.is_empty() {
                    "void".to_string()
                } else {
                    ret_ty_str
                };
                if func._is_async && !ret_ty.starts_with("Promise<") && ret_ty != "Promise" {
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
                        let (t, _, _) = self.parse_signature(p_ty);
                        TejxType::from_name(&t)
                    })
                    .collect::<Vec<TejxType>>();
                let (final_ret, _, _) = self.parse_signature(format!("function:{}", ret_ty));
                let has_defaults = min_required < params.len();
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        func.name.clone(),
                        Symbol {
                            ty: TejxType::Function(
                                params.clone(),
                                Box::new(TejxType::from_name(&final_ret)),
                            ),
                            is_const: false,
                            params,
                            min_params: if has_defaults {
                                Some(min_required)
                            } else {
                                None
                            },
                            is_variadic,
                            aliased_type: None,
                            generic_params: func.generic_params.clone(),
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
                            params: Vec::new(),
                            min_params: None,
                            is_variadic: false,
                            aliased_type: { Some(TejxType::from_node(&_type_def)) },
                            generic_params: Vec::new(),
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
        let obj_ty = TejxType::from_name(obj_type);
            if let TejxType::Object(props) = obj_ty {
                if let Some((_, _, ty)) = props.into_iter().find(|(name, _, _)| name == member) {
                    return Some(MemberInfo {
                        ty,
                        is_static: false,
                        access: AccessLevel::Public,
                        is_readonly: false,
                        generic_params: Vec::new(),
                    });
                }
            }
        let mut current_type = self.normalize_member_container(obj_type);

        while !current_type.is_empty() && current_type != "<inferred>" {
            if let Some(members) = self.class_members.get(&current_type) {
                if let Some(info) = members.get(member).cloned() {
                    return Some(info);
                }
            }

            // Check if it's an interface
            if let Some(members) = self.interfaces.get(&current_type) {
                if let Some(info) = members.get(member).cloned() {
                    return Some(info);
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
}
