use super::*;
use crate::ast::*;
use std::collections::HashMap;

impl TypeChecker {
    pub(crate) fn collect_declarations(&mut self, stmt: &Statement) {
        match stmt {
            Statement::ClassDeclaration(class_decl) => {
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
                    members.insert(
                        m._name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(&self.parameterize_generics(
                                &m._type_name.to_string(),
                                &class_decl.generic_params,
                            )),
                            is_static: m._is_static,
                            access: if m._access == crate::ast::AccessModifier::Private {
                                AccessLevel::Private
                            } else {
                                AccessLevel::Public
                            },
                            is_readonly: false,
                        },
                    );
                }
                for method in &class_decl.methods {
                    let ret_ty_str = method.func.return_type.to_string();
                    let ret_ty = if ret_ty_str == "any" || ret_ty_str.is_empty() {
                        "void".to_string()
                    } else {
                        ret_ty_str
                    };
                    let mut param_types = Vec::new();
                    for p in &method.func.params {
                        param_types.push(p.type_name.to_string());
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
                            },
                        );
                    }
                }
                self.class_members.insert(class_decl.name.clone(), members);
            }
            Statement::FunctionDeclaration(func) => {
                let ret_ty_str = func.return_type.to_string();
                let ret_ty = if ret_ty_str == "any" || ret_ty_str.is_empty() {
                    "void".to_string()
                } else {
                    ret_ty_str
                };
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
                        let (t, _, _) = self.parse_signature(p.type_name.to_string());
                        TejxType::from_name(&t)
                    })
                    .collect::<Vec<TejxType>>();
                let (final_ret, _, _) = self.parse_signature(format!("function:{}", ret_ty));
                let has_defaults = min_required < params.len();
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        func.name.clone(),
                        Symbol {
                            ty: TejxType::Function(params.clone(), Box::new(TejxType::from_name(&final_ret))),
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
                            aliased_type: {
                                if name == "Node" {
                                    println!("DEBUG NODE ALIAS STRING: '{}'", _type_def.to_string());
                                }
                                Some(TejxType::from_node(&_type_def))
                            },
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

    pub(crate) fn resolve_instance_member(
        &self,
        obj_type: &str,
        member: &str,
    ) -> Option<MemberInfo> {
        let mut unwrapped_type = obj_type.to_string();
        if obj_type.contains('|') {
            unwrapped_type = obj_type
                .split('|')
                .map(|s| s.trim().to_string())
                .find(|s| s != "None" && !s.is_empty())
                .unwrap_or(obj_type.to_string());
        }

        let mut current_type = if unwrapped_type.ends_with("[]") {
            "Array".to_string()
        } else if unwrapped_type.contains('[') && unwrapped_type.ends_with(']') {
            // Fixed-size arrays like int32[5] should also map to Array
            "Array".to_string()
        } else if unwrapped_type.starts_with("Array<") {
            // Generic Array<T> maps to Array
            "Array".to_string()
        } else if unwrapped_type == "string" {
            "String".to_string()
        } else if unwrapped_type.starts_with('{') {
            // Object literals map to Map
            "Map".to_string()
        } else if unwrapped_type == "enum"
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
