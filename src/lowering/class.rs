use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::types::TejxType;
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
            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                TejxType::from_node(&cons.return_type),
            );
            self.user_function_args
                .borrow_mut()
                .insert(mangled, cons.params.len());
        }
        let mut methods = Vec::new();
        for method in &class_decl.methods {
            let mangled = if method.func.name.starts_with("f_") {
                method.func.name.clone()
            } else {
                format!("f_{}_{}", class_decl.name, method.func.name)
            };

            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                TejxType::from_node(&method.func.return_type),
            );
            self.user_function_args
                .borrow_mut()
                .insert(mangled, method.func.params.len());
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
            let ty = TejxType::from_node(&member._type_name);
            let init = member._initializer.as_ref().map(|e| *e.clone()).unwrap_or(
                Expression::NumberLiteral {
                    value: 0.0,
                    _is_float: false,
                    _line: 0,
                    _col: 0,
                },
            );
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
            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                TejxType::from_node(&constructor.return_type),
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
                TejxType::from_node(&getter._return_type),
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
            _is_async: false,
            is_extern: false,
            generic_params: vec![],
            _line: 0,
            _col: 0,
        };

        if let Some(cons) = &class_decl._constructor {
            all_methods.push((cons, false));
        } else {
            // Check if we need to generate a default constructor
            // Always generate one to avoid linker errors in mir_lowering
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
                params.push((p.name.clone(), TejxType::from_node(&p.type_name)));
            }
            let name = format!("f_{}_{}", class_decl.name, func_decl.name);
            let return_type = TejxType::from_node(&func_decl.return_type);

            self.enter_scope();
            let mangled_params: Vec<(String, TejxType)> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();

            let mut hir_body =
                self.lower_statement(&func_decl.body)
                    .unwrap_or(HIRStatement::Block {
                        line: line,
                        statements: vec![],
                    });

            if let HIRStatement::Block {
                line,
                ref mut statements,
            } = hir_body
            {
                // Inject method attachments for constructor
                if func_decl.name == "constructor" {
                    let mangled_this = self
                        .lookup("this")
                        .map(|(n, _)| n)
                        .unwrap_or("this".to_string());
                    // Instance fields
                    let i_fields_borrow = self.class_instance_fields.borrow();
                    if let Some(i_list) = i_fields_borrow.get(&class_decl.name) {
                        for (f_name, f_ty, f_init) in i_list {
                            let hir_init = self.lower_expression(f_init);
                            // Insert before other logic
                            statements.insert(
                                0,
                                HIRStatement::ExpressionStmt {
                                    line: line,
                                    expr: HIRExpression::Assignment {
                                        line: line,
                                        target: Box::new(HIRExpression::MemberAccess {
                                            line: line,
                                            target: Box::new(HIRExpression::Variable {
                                                line: line,
                                                name: mangled_this.clone(),
                                                ty: TejxType::Class(
                                                    class_decl.name.clone(),
                                                    vec![],
                                                ),
                                            }),
                                            member: f_name.clone(),
                                            ty: f_ty.clone(),
                                        }),
                                        value: Box::new(hir_init),
                                        ty: f_ty.clone(),
                                    },
                                },
                            );
                        }
                    }

                    let methods_borrow = self.class_methods.borrow();
                    if let Some(m_list) = methods_borrow.get(&class_decl.name) {
                        for m_name in m_list {
                            let mangled_func = format!("f_{}_{}", class_decl.name, m_name);
                            // Insert after debug print
                            statements.insert(
                                0,
                                HIRStatement::ExpressionStmt {
                                    line: line,
                                    expr: HIRExpression::Call {
                                        line: line,
                                        callee: "m_set".to_string(),
                                        args: vec![
                                            HIRExpression::Variable {
                                                line: line,
                                                name: mangled_this.clone(),
                                                ty: TejxType::Class(
                                                    class_decl.name.clone(),
                                                    vec![],
                                                ),
                                            },
                                            HIRExpression::Literal {
                                                line: line,
                                                value: m_name.clone(),
                                                ty: TejxType::String,
                                            },
                                            HIRExpression::Variable {
                                                line: line,
                                                name: mangled_func,
                                                ty: TejxType::Int64,
                                            },
                                        ],
                                        ty: TejxType::Void,
                                    },
                                },
                            );
                        }
                    }

                    // Getters
                    let getters_borrow = self.class_getters.borrow();
                    if let Some(g_list) = getters_borrow.get(&class_decl.name) {
                        for g_name in g_list {
                            let mangled_func = format!("f_{}_get_{}", class_decl.name, g_name);
                            statements.insert(
                                0,
                                HIRStatement::ExpressionStmt {
                                    line: line,
                                    expr: HIRExpression::Call {
                                        line: line,
                                        callee: "m_set".to_string(),
                                        args: vec![
                                            HIRExpression::Variable {
                                                line: line,
                                                name: mangled_this.clone(),
                                                ty: TejxType::Class(
                                                    class_decl.name.clone(),
                                                    vec![],
                                                ),
                                            },
                                            HIRExpression::Literal {
                                                line: line,
                                                value: format!("get_{}", g_name),
                                                ty: TejxType::String,
                                            },
                                            HIRExpression::Variable {
                                                line: line,
                                                name: mangled_func,
                                                ty: TejxType::Int64,
                                            },
                                        ],
                                        ty: TejxType::Void,
                                    },
                                },
                            );
                        }
                    }

                    // Setters
                    let setters_borrow = self.class_setters.borrow();
                    if let Some(s_list) = setters_borrow.get(&class_decl.name) {
                        for s_name in s_list {
                            let mangled_func = format!("f_{}_set_{}", class_decl.name, s_name);
                            statements.insert(
                                0,
                                HIRStatement::ExpressionStmt {
                                    line: line,
                                    expr: HIRExpression::Call {
                                        line: line,
                                        callee: "m_set".to_string(),
                                        args: vec![
                                            HIRExpression::Variable {
                                                line: line,
                                                name: mangled_this.clone(),
                                                ty: TejxType::Class(
                                                    class_decl.name.clone(),
                                                    vec![],
                                                ),
                                            },
                                            HIRExpression::Literal {
                                                line: line,
                                                value: format!("set_{}", s_name),
                                                ty: TejxType::String,
                                            },
                                            HIRExpression::Variable {
                                                line: line,
                                                name: mangled_func,
                                                ty: TejxType::Int64,
                                            },
                                        ],
                                        ty: TejxType::Void,
                                    },
                                },
                            );
                        }
                    }

                    // Inject class metadata for instanceof support
                    // Set __class__ = "ClassName" on this
                    statements.insert(
                        0,
                        HIRStatement::ExpressionStmt {
                            line: line,
                            expr: HIRExpression::Call {
                                line: line,
                                callee: "m_set".to_string(),
                                args: vec![
                                    HIRExpression::Variable {
                                        line: line,
                                        name: mangled_this.clone(),
                                        ty: TejxType::Class(class_decl.name.clone(), vec![]),
                                    },
                                    HIRExpression::Literal {
                                        line: line,
                                        value: "__class__".to_string(),
                                        ty: TejxType::String,
                                    },
                                    HIRExpression::Literal {
                                        line: line,
                                        value: class_decl.name.clone(),
                                        ty: TejxType::String,
                                    },
                                ],
                                ty: TejxType::Void,
                            },
                        },
                    );

                    // Set __parents__ = "Parent1,Parent2,..." for inheritance chain
                    if !class_decl._parent_name.is_empty() {
                        // Build chain: walk up parent hierarchy
                        let mut parents = Vec::new();
                        let mut current_parent = class_decl._parent_name.clone();
                        parents.push(current_parent.clone());
                        // Also add transitive parents if known
                        let class_parents_borrow = self.class_parents.borrow();
                        loop {
                            if let Some(gp) = class_parents_borrow.get(&current_parent) {
                                if !gp.is_empty() {
                                    parents.push(gp.clone());
                                    current_parent = gp.clone();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        drop(class_parents_borrow);
                        let parents_str = parents.join(",");
                        statements.insert(
                            0,
                            HIRStatement::ExpressionStmt {
                                line: line,
                                expr: HIRExpression::Call {
                                    line: line,
                                    callee: "m_set".to_string(),
                                    args: vec![
                                        HIRExpression::Variable {
                                            line: line,
                                            name: mangled_this,
                                            ty: TejxType::Class(class_decl.name.clone(), vec![]),
                                        },
                                        HIRExpression::Literal {
                                            line: line,
                                            value: "__parents__".to_string(),
                                            ty: TejxType::String,
                                        },
                                        HIRExpression::Literal {
                                            line: line,
                                            value: parents_str,
                                            ty: TejxType::String,
                                        },
                                    ],
                                    ty: TejxType::Void,
                                },
                            },
                        );
                    }
                }
            }

            if func_decl._is_async {
                // Async method lowering
                // 1. Generate unique worker name: f_ClassName_MethodName_worker
                let worker_name = format!("{}_worker", name);

                // 2. Create state struct name
                let _state_struct_name = format!("State_{}", worker_name);

                // 3. Prepare params for worker (needs 'this' + original params)
                // The wrapper method 'name' has (this, p1, p2...)
                // The worker needs a state object that contains these.

                // Let's use the existing lower_async_function logic but we need to trick it or adapt it
                // to handle 'this' which is implicit in AST but explicit in HIR function params.
                // The easiest way is to treat 'this' as just another parameter for the async transformation.

                // We need to define 'this' in scope before lowering body so it's captured in state.
                // Wait, we already called define for all params (including 'this') above.

                let mangled_name = if name.starts_with("f_") {
                    name.to_string()
                } else {
                    format!("f_{}_{}", class_decl.name, name)
                };

                let (worker_func, _state_struct, wrapper_body) = self.lower_async_function_impl(
                    &mangled_name,
                    &mangled_params,
                    &func_decl.return_type.to_string(),
                    &func_decl.body,
                );

                self._exit_scope();

                // Register the worker and state struct (which are global/top-level in HIR)
                // But wait, lower_async_function_impl returns HIRStatement::Function for worker
                // and HIRStatement::Struct for state.
                // We should push them to 'functions' (which ends up in global HIR functions).

                // The wrapper body returned is what goes into the method body.

                functions.push(worker_func);
                functions.push(_state_struct);

                functions.push(HIRStatement::Function {
                    async_params: None,
                    line: line,
                    name: mangled_name,
                    params: mangled_params,
                    _return_type: TejxType::Int64,
                    body: Box::new(wrapper_body),
                    is_extern: false,
                });
            } else {
                // Sync method
                // hir_body was already lowered above for constructor, let's ensure it's lowered for others too
                // Actually, if it's not a constructor, it was lowered at line 830.

                self._exit_scope();

                let mangled_name = if name.starts_with("f_") {
                    name.to_string()
                } else {
                    format!(
                        "f_{}_{}",
                        class_decl.name.replace("[", "_").replace("]", "_"),
                        name
                    )
                };

                functions.push(HIRStatement::Function {
                    async_params: None,
                    line: line,
                    name: mangled_name,
                    params: mangled_params,
                    _return_type: return_type,
                    body: Box::new(hir_body),
                    is_extern: false,
                });
            }
        }

        // Lower getters
        for getter in &class_decl._getters {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            params.push((
                "this".to_string(),
                TejxType::Class(class_decl.name.clone(), vec![]),
            ));

            let name = format!("f_{}_get_{}", class_decl.name, getter._name);
            let return_type = TejxType::from_node(&getter._return_type);

            self.enter_scope();
            let mangled_params: Vec<(String, TejxType)> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();

            let hir_body = self
                .lower_statement(&getter._body)
                .unwrap_or(HIRStatement::Block {
                    line: line,
                    statements: vec![],
                });

            self._exit_scope();
            functions.push(HIRStatement::Function {
                async_params: None,
                line: line,
                name,
                params: mangled_params,
                _return_type: return_type,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }

        // Lower setters
        for setter in &class_decl._setters {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            params.push((
                "this".to_string(),
                TejxType::Class(class_decl.name.clone(), vec![]),
            ));
            params.push((
                setter._param_name.clone(),
                TejxType::from_node(&setter._param_type),
            ));

            let name = format!("f_{}_set_{}", class_decl.name, setter._name);

            self.enter_scope();
            let mangled_params: Vec<(String, TejxType)> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();

            let hir_body = self
                .lower_statement(&setter._body)
                .unwrap_or(HIRStatement::Block {
                    line: line,
                    statements: vec![],
                });

            self._exit_scope();
            functions.push(HIRStatement::Function {
                async_params: None,
                line: line,
                name,
                params: mangled_params,
                _return_type: TejxType::Void,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }

        // Lower static fields into global assignments
        let s_fields_borrow = self.class_static_fields.borrow();
        if let Some(s_list) = s_fields_borrow.get(&class_decl.name) {
            for (f_name, f_ty, f_init) in s_list {
                let hir_init = self.lower_expression(f_init);
                // Static fields are mangled as g_Class_Field
                let mangled_name = format!("g_{}_{}", class_decl.name, f_name);
                main_stmts.push(HIRStatement::ExpressionStmt {
                    line: line,
                    expr: HIRExpression::Assignment {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
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
                params.push((p.name.clone(), TejxType::from_node(&p.type_name)));
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
            let return_type = TejxType::from_node(&func_decl.return_type);

            self.enter_scope();
            let mangled_params: Vec<(String, TejxType)> = params
                .iter()
                .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
                .collect();

            let hir_body = self
                .lower_statement(&func_decl.body)
                .unwrap_or(HIRStatement::Block {
                    line: line,
                    statements: vec![],
                });

            self._exit_scope();

            functions.push(HIRStatement::Function {
                async_params: None,
                line: line,
                name,
                params: mangled_params,
                _return_type: return_type,
                body: Box::new(hir_body),
                is_extern: false,
            });
        }
    }
}
