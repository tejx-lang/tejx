use super::Lowering;
use crate::common::types::TejxType;
use crate::frontend::ast::*;
use crate::middle::hir::*;
use std::collections::HashSet;

impl Lowering {
    pub(crate) fn scan_variadic_class(&self, class_decl: &ClassDeclaration) {
        for method in &class_decl.methods {
            let fixed_count = method
                .func
                .params
                .iter()
                .take_while(|p| !p._is_rest)
                .count();
            if fixed_count < method.func.params.len() {
                let mangled = format!("{}_{}", class_decl.name, method.func.name);
                self.variadic_functions
                    .borrow_mut()
                    .insert(mangled, fixed_count);
            }
        }
        if let Some(cons) = &class_decl._constructor {
            let fixed_count = cons.params.iter().take_while(|p| !p._is_rest).count();
            if fixed_count < cons.params.len() {
                let mangled = format!("{}_constructor", class_decl.name);
                self.variadic_functions
                    .borrow_mut()
                    .insert(mangled, fixed_count);
            }
        }
    }

    pub(crate) fn register_class(&self, class_decl: &ClassDeclaration) {
        if let Some(cons) = &class_decl._constructor {
            let mangled = format!("f_{}_constructor", class_decl.name);
            let param_types = cons
                .params
                .iter()
                .map(|p| self.resolve_alias_type(&TejxType::from_node(&p.type_name)))
                .collect::<Vec<_>>();
            let ret_type = self.resolve_alias_type(&TejxType::from_node(&cons.return_type));
            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                TejxType::Function(param_types, Box::new(ret_type)),
            );
            self.user_function_args
                .borrow_mut()
                .insert(mangled.clone(), cons.params.len());
            let ctor_defaults: Vec<Option<Expression>> = cons
                .params
                .iter()
                .map(|p| p._default_value.as_ref().map(|b| (**b).clone()))
                .collect();
            self.constructor_param_defaults
                .borrow_mut()
                .insert(class_decl.name.clone(), ctor_defaults.clone());
            self.user_function_param_defaults
                .borrow_mut()
                .insert(mangled.clone(), ctor_defaults);
        }
        let mut methods = Vec::new();
        for method in &class_decl.methods {
            let mangled = if method.func.name.starts_with("f_") {
                method.func.name.clone()
            } else {
                format!("f_{}_{}", class_decl.name, method.func.name)
            };

            let mut ret_type =
                self.resolve_alias_type(&TejxType::from_node(&method.func.return_type));
            if false {
                let is_promise = matches!(ret_type, TejxType::Class(ref n, _) if n == "Promise");
                if !is_promise {
                    ret_type = TejxType::Class("Promise".to_string(), vec![ret_type]);
                }
            }
            self.user_functions
                .borrow_mut()
                .insert(mangled.clone(), ret_type);
            let total_args = if method.is_static {
                method.func.params.len()
            } else {
                1 + method.func.params.len()
            };
            self.user_function_args
                .borrow_mut()
                .insert(mangled.clone(), total_args);
            let mut method_defaults: Vec<Option<Expression>> = Vec::new();
            if !method.is_static {
                method_defaults.push(None); // index 0 is 'this'
            }
            for p in &method.func.params {
                method_defaults.push(p._default_value.as_ref().map(|b| (**b).clone()));
            }
            self.user_function_param_defaults
                .borrow_mut()
                .insert(mangled.clone(), method_defaults);
            if !method.is_static {
                methods.push(method.func.name.clone());
            }
        }
        self.class_methods
            .borrow_mut()
            .insert(class_decl.name.clone(), methods);
        // Store generic params for the class (e.g. Map -> [K, V])
        if !class_decl.generic_params.is_empty() {
            self.class_generic_params.borrow_mut().insert(
                class_decl.name.clone(),
                class_decl
                    .generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect(),
            );
        }

        let mut i_fields = Vec::new();
        let mut s_fields = Vec::new();
        for member in &class_decl._members {
            let ty = self.resolve_alias_type(&TejxType::from_node(&member._type_name));
            let init = if let Some(e) = &member._initializer {
                *e.clone()
            } else {
                match &ty {
                    TejxType::Bool => Expression::BooleanLiteral {
                        value: false,
                        _line: 0,
                        _col: 0,
                    },
                    TejxType::String => Expression::StringLiteral {
                        value: "".to_string(),
                        _line: 0,
                        _col: 0,
                    },
                    TejxType::Float32 | TejxType::Float64 => Expression::NumberLiteral {
                        value: 0.0,
                        _is_float: true,
                        _line: 0,
                        _col: 0,
                    },
                    TejxType::Optional(_) => Expression::NoneLiteral { _line: 0, _col: 0 },
                    TejxType::DynamicArray(_) => Expression::ArrayLiteral {
                        elements: vec![],
                        ty: std::cell::RefCell::new(None),
                        _line: 0,
                        _col: 0,
                    },
                    _ => Expression::NumberLiteral {
                        value: 0.0,
                        _is_float: false,
                        _line: 0,
                        _col: 0,
                    },
                }
            };
            if member._is_static {
                s_fields.push((member._name.clone(), ty, init));
            } else {
                i_fields.push((member._name.clone(), ty, init));
            }
        }
        self.class_instance_fields
            .borrow_mut()
            .insert(class_decl.name.clone(), i_fields);
        self.class_static_fields
            .borrow_mut()
            .insert(class_decl.name.clone(), s_fields);

        if let Some(constructor) = &class_decl._constructor {
            let mangled = format!("{}_{}", class_decl.name, constructor.name);
            let param_types = constructor
                .params
                .iter()
                .map(|p| self.resolve_alias_type(&TejxType::from_node(&p.type_name)))
                .collect::<Vec<_>>();
            let ret_type = self.resolve_alias_type(&TejxType::from_node(&constructor.return_type));
            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                TejxType::Function(param_types, Box::new(ret_type)),
            );
            self.user_function_args
                .borrow_mut()
                .insert(mangled, constructor.params.len());
        }

        let mut getters = HashSet::new();
        for getter in &class_decl._getters {
            let mangled = format!("{}_get_{}", class_decl.name, getter._name);
            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                self.resolve_alias_type(&TejxType::from_node(&getter._return_type)),
            );
            self.user_function_args.borrow_mut().insert(mangled, 1); // Getters take 'this'
            getters.insert(getter._name.clone());
        }
        self.class_getters
            .borrow_mut()
            .insert(class_decl.name.clone(), getters);

        let mut setters = HashSet::new();
        for setter in &class_decl._setters {
            let mangled = format!("{}_set_{}", class_decl.name, setter._name);
            self.user_functions
                .borrow_mut()
                .insert(mangled.clone(), TejxType::Void);
            self.user_function_args.borrow_mut().insert(mangled, 2); // Setters take 'this', 'value'
            setters.insert(setter._name.clone());
        }
        self.class_setters
            .borrow_mut()
            .insert(class_decl.name.clone(), setters);

        if !class_decl._parent_name.is_empty() {
            self.class_parents
                .borrow_mut()
                .insert(class_decl.name.clone(), class_decl._parent_name.clone());
        }
    }

    pub(crate) fn lower_class_declaration(
        &self,
        class_decl: &ClassDeclaration,
        functions: &mut Vec<HIRStatement>,
        main_stmts: &mut Vec<HIRStatement>,
    ) {
        let line = class_decl._line;
        *self.current_class.borrow_mut() = Some(class_decl.name.clone());
        let p_name = if class_decl._parent_name.is_empty() {
            None
        } else {
            Some(class_decl._parent_name.clone())
        };
        *self.parent_class.borrow_mut() = p_name;

        let mut all_methods = Vec::new();
        for m in &class_decl.methods {
            all_methods.push((&m.func, m.is_static));
        }

        // Handle constructor (explicit or default)
        let default_body = Box::new(Statement::BlockStmt {
            statements: vec![],
            _line: 0,
            _col: 0,
        });
        let default_cons = FunctionDeclaration {
            name: "constructor".to_string(),
            params: vec![],
            return_type: TypeNode::Named("void".to_string()),
            body: default_body,
            is_extern: false,
            generic_params: vec![],
            _line: 0,
            _col: 0,
        };

        if let Some(cons) = &class_decl._constructor {
            all_methods.push((cons, false));
        } else {
            all_methods.push((&default_cons, false));
        }

        for (func_decl, is_static) in all_methods {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            if !is_static {
                params.push((
                    "this".to_string(),
                    TejxType::Class(class_decl.name.clone(), vec![]),
                ));
            }
            for p in &func_decl.params {
                params.push((
                    p.name.clone(),
                    self.resolve_alias_type(&TejxType::from_node(&p.type_name)),
                ));
            }
            let name = format!("f_{}_{}", class_decl.name, func_decl.name);
            let un_name = format!("{}_{}", class_decl.name, func_decl.name);
            let return_type = self.resolve_alias_type(&TejxType::from_node(&func_decl.return_type));

            let mut func_defaults: Vec<Option<Expression>> = Vec::new();
            if !is_static {
                func_defaults.push(None); // index 0 is 'this'
            }
            for p in &func_decl.params {
                func_defaults.push(p._default_value.as_ref().map(|b| (**b).clone()));
            }
            self.user_function_param_defaults
                .borrow_mut()
                .insert(name.clone(), func_defaults.clone());
            if func_decl.name != "constructor" {
                self.user_function_param_defaults
                    .borrow_mut()
                    .insert(un_name.clone(), func_defaults);
                self.user_function_args
                    .borrow_mut()
                    .insert(un_name, params.len());
            }

            if !func_decl.generic_params.is_empty() {
                self.function_generic_params.borrow_mut().insert(
                    name.clone(),
                    func_decl
                        .generic_params
                        .iter()
                        .map(|gp| gp.name.clone())
                        .collect(),
                );
            }

            let env_owner_name = if false {
                format!("f_{}_worker", name)
            } else {
                name.clone()
            };
            self.push_env_owner(env_owner_name);
            self.enter_scope();
            let mangled_params: Vec<(String, TejxType)> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();

            self.return_type_stack.borrow_mut().push(TejxType::Void);
            let mut hir_body =
                self.lower_statement(&func_decl.body)
                    .unwrap_or(HIRStatement::Block {
                        line,
                        statements: vec![],
                    });
            self.return_type_stack.borrow_mut().pop();

            if let HIRStatement::Block {
                line,
                ref mut statements,
            } = hir_body
            {
                if func_decl.name == "constructor" {
                    let mut insert_pos = 0;
                    // Find position after super() call if present
                    if let Some(parent_name) = &*self.parent_class.borrow() {
                        let base_parent = parent_name.split('<').next().unwrap_or(parent_name);
                        let super_callee = format!("f_{}_constructor", base_parent);
                        for (i, stmt) in statements.iter().enumerate() {
                            if let HIRStatement::ExpressionStmt { expr, .. } = stmt {
                                if let HIRExpression::Call { callee, .. } = expr {
                                    if callee == &super_callee {
                                        insert_pos = i + 1;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    let mangled_this = self
                        .lookup("this")
                        .map(|(n, _)| n)
                        .unwrap_or("this".to_string());

                    let mut injections = Vec::<HIRStatement>::new();

                    // 3. Instance fields initialization
                    let i_fields_borrow = self.class_instance_fields.borrow();
                    if let Some(i_list) = i_fields_borrow.get(&class_decl.name) {
                        for (f_name, f_ty, f_init) in i_list {
                            let prev_expected = self.current_expected_type.borrow_mut().take();
                            *self.current_expected_type.borrow_mut() = Some(f_ty.clone());
                            let hir_init = self.lower_expression(f_init);
                            *self.current_expected_type.borrow_mut() = prev_expected;
                            injections.push(HIRStatement::ExpressionStmt {
                                line,
                                expr: HIRExpression::Assignment {
                                    line,
                                    target: Box::new(HIRExpression::MemberAccess {
                                        line,
                                        target: Box::new(HIRExpression::Variable {
                                            line,
                                            name: mangled_this.clone(),
                                            ty: TejxType::Class(class_decl.name.clone(), vec![]),
                                        }),
                                        member: f_name.clone(),
                                        ty: f_ty.clone(),
                                    }),
                                    value: Box::new(hir_init),
                                    ty: f_ty.clone(),
                                },
                            });
                        }
                    }

                    // 5. Getters/Setters: Modern compiler doesn't attach them to instances
                    // (They should be resolved via VTable or static dispatch in CodeGen)

                    // Splice injections into statements at insert_pos
                    for (i, injection) in injections.into_iter().enumerate() {
                        statements.insert(insert_pos + i, injection);
                    }
                }
            }

            self._exit_scope();
            self.pop_env_owner();
            let mangled_name = format!(
                "f_{}_{}",
                class_decl.name.replace("[", "_").replace("]", "_"),
                func_decl.name
            );
            self.function_source_files
                .borrow_mut()
                .insert(mangled_name.clone(), self.current_file.borrow().clone());
            self.function_source_files.borrow_mut().insert(
                format!("{}_{}", class_decl.name, func_decl.name),
                self.current_file.borrow().clone(),
            );
            functions.push(HIRStatement::Function {
                line,
                name: mangled_name,
                params: mangled_params,
                _return_type: return_type,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }

        // Lower getters
        for getter in &class_decl._getters {
            let params = [(
                "this".to_string(),
                TejxType::Class(class_decl.name.clone(), vec![]),
            )];
            let name = format!("f_{}_get_{}", class_decl.name, getter._name);
            let return_type = self.resolve_alias_type(&TejxType::from_node(&getter._return_type));
            self.push_env_owner(name.clone());
            self.enter_scope();
            let mangled_params: Vec<_> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();
            self.return_type_stack
                .borrow_mut()
                .push(return_type.clone());
            let hir_body = self
                .lower_statement(&getter._body)
                .unwrap_or(HIRStatement::Block {
                    line,
                    statements: vec![],
                });
            self.return_type_stack.borrow_mut().pop();
            self._exit_scope();
            self.pop_env_owner();
            self.function_source_files
                .borrow_mut()
                .insert(name.clone(), self.current_file.borrow().clone());
            functions.push(HIRStatement::Function {
                line,
                name,
                params: mangled_params,
                _return_type: return_type,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }

        // Lower setters
        for setter in &class_decl._setters {
            let params = [
                (
                    "this".to_string(),
                    TejxType::Class(class_decl.name.clone(), vec![]),
                ),
                (
                    setter._param_name.clone(),
                    self.resolve_alias_type(&TejxType::from_node(&setter._param_type)),
                ),
            ];
            let name = format!("f_{}_set_{}", class_decl.name, setter._name);
            self.push_env_owner(name.clone());
            self.enter_scope();
            let mangled_params: Vec<_> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();
            self.return_type_stack.borrow_mut().push(TejxType::Void);
            let hir_body = self
                .lower_statement(&setter._body)
                .unwrap_or(HIRStatement::Block {
                    line,
                    statements: vec![],
                });
            self.return_type_stack.borrow_mut().pop();
            self._exit_scope();
            self.pop_env_owner();
            self.function_source_files
                .borrow_mut()
                .insert(name.clone(), self.current_file.borrow().clone());
            functions.push(HIRStatement::Function {
                line,
                name,
                params: mangled_params,
                _return_type: TejxType::Void,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }

        // Lower static fields
        let s_fields_borrow = self.class_static_fields.borrow();
        if let Some(s_list) = s_fields_borrow.get(&class_decl.name) {
            for (f_name, f_ty, f_init) in s_list {
                let hir_init = self.lower_expression(f_init);
                let mangled_name = format!("g_{}_{}", class_decl.name, f_name);
                main_stmts.push(HIRStatement::ExpressionStmt {
                    line,
                    expr: HIRExpression::Assignment {
                        line,
                        target: Box::new(HIRExpression::Variable {
                            line,
                            name: mangled_name,
                            ty: f_ty.clone(),
                        }),
                        value: Box::new(hir_init),
                        ty: TejxType::Int64,
                    },
                });
            }
        }

        // Lower static fields into global assignments
        let s_fields_borrow = self.class_static_fields.borrow();
        if let Some(s_list) = s_fields_borrow.get(&class_decl.name) {
            for (f_name, f_ty, f_init) in s_list {
                let hir_init = self.lower_expression(f_init);
                // Static fields are mangled as g_Class_Field
                let mangled_name = format!("g_{}_{}", class_decl.name, f_name);
                main_stmts.push(HIRStatement::ExpressionStmt {
                    line,
                    expr: HIRExpression::Assignment {
                        line,
                        target: Box::new(HIRExpression::Variable {
                            line,
                            name: mangled_name,
                            ty: f_ty.clone(),
                        }),
                        value: Box::new(hir_init),
                        ty: TejxType::Int64,
                    },
                });
            }
        }

        *self.current_class.borrow_mut() = None;
        *self.parent_class.borrow_mut() = None;
    }

    pub(crate) fn lower_extension_declaration(
        &self,
        ext_decl: &ExtensionDeclaration,
        functions: &mut Vec<HIRStatement>,
    ) {
        let line = ext_decl._line;
        for func_decl in &ext_decl._methods {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            params.push((
                "this".to_string(),
                TejxType::Class(ext_decl._target_type.to_string(), vec![]),
            ));

            for p in &func_decl.params {
                params.push((
                    p.name.clone(),
                    self.resolve_alias_type(&TejxType::from_node(&p.type_name)),
                ));
            }

            let name = if func_decl.name.starts_with("f_") {
                func_decl.name.clone()
            } else {
                format!(
                    "f_{}_{}",
                    ext_decl
                        ._target_type
                        .to_string()
                        .replace("[", "_")
                        .replace("]", "_"),
                    func_decl.name
                )
            };
            let return_type = self.resolve_alias_type(&TejxType::from_node(&func_decl.return_type));

            self.push_env_owner(name.clone());
            self.enter_scope();
            let mangled_params: Vec<(String, TejxType)> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();

            self.return_type_stack
                .borrow_mut()
                .push(return_type.clone());
            let hir_body = self
                .lower_statement(&func_decl.body)
                .unwrap_or(HIRStatement::Block {
                    line,
                    statements: vec![],
                });
            self.return_type_stack.borrow_mut().pop();

            self._exit_scope();
            self.pop_env_owner();

            self.function_source_files
                .borrow_mut()
                .insert(name.clone(), self.current_file.borrow().clone());

            functions.push(HIRStatement::Function {
                line,
                name,
                params: mangled_params,
                _return_type: return_type,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }
    }
}
