use super::*;
use crate::frontend::ast::*;
use crate::frontend::token::TokenType;
use std::collections::HashMap;

impl TypeChecker {
    pub(crate) fn check_statement(&mut self, stmt: &Statement) -> Result<(), ()> {
        match stmt {
            Statement::VarDeclaration {
                pattern,
                type_annotation,
                initializer,
                is_const,
                line,
                _col,
            } => {
                let ty_str = type_annotation.to_string();
                let has_explicit_type = !ty_str.is_empty();
                let is_explicit_any = ty_str == "any";
                if !is_explicit_any
                    && has_explicit_type
                    && !self.is_valid_type(&TejxType::from_name(&ty_str))
                {
                    self.report_error_detailed(format!("Unknown data type: '{}'", ty_str), *line, *_col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, or user-defined classes"));
                }
                if let Some(expr) = initializer {
                    let prev_expected = self.current_expected_type.take();
                    let prev_lambda_ctx = self.lambda_context_params.take();
                    if ty_str != "any" && !ty_str.is_empty() {
                        let expected_ty = TejxType::from_name(&ty_str);
                        self.current_expected_type = Some(expected_ty.clone());
                        if let TejxType::Function(params, _) = expected_ty {
                            self.lambda_context_params = Some(params);
                        }
                    }
                    let mut init_type = self.check_expression(expr)?.to_name();
                    self.current_expected_type = prev_expected;
                    self.lambda_context_params = prev_lambda_ctx;
                    if !has_explicit_type && init_type == "[]" {
                        self.report_error_detailed(
                            "Cannot infer type for empty array".to_string(),
                            *line,
                            *_col,
                            "E0106",
                            Some("Please provide an explicit type annotation (e.g., 'let arr: int[] = []')"),
                        );
                        init_type = "<inferred>".to_string(); // prevent cascading errors
                    }
                    if !has_explicit_type && (init_type == "any" || init_type == "None") {
                        self.report_error_detailed(
                            "Type annotation required for variable declaration".to_string(),
                            *line,
                            *_col,
                            "E0101",
                            Some(
                                "Type inference resolved to 'any' or 'None'. Provide an explicit type (e.g., 'let x: int = 1') or use 'any' explicitly",
                            ),
                        );
                        init_type = "<inferred>".to_string();
                    }
                    if !has_explicit_type && init_type == "<inferred>" {
                        self.report_error_detailed(
                            "Type annotation required for variable declaration".to_string(),
                            *line,
                            *_col,
                            "E0101",
                            Some(
                                "Provide an explicit type (e.g., 'let x: int = 1') or use 'any' explicitly",
                            ),
                        );
                    }

                    if ty_str != "any" && !ty_str.is_empty() {
                        self.check_numeric_bounds(
                            expr,
                            &TejxType::from_name(&ty_str),
                            *line,
                            *_col,
                        );
                    } else if init_type != "<inferred>" {
                        self.check_numeric_bounds(
                            expr,
                            &TejxType::from_name(&init_type),
                            *line,
                            *_col,
                        );
                    }

                    if !is_explicit_any
                        && has_explicit_type
                        && !self.are_types_compatible(
                            &TejxType::from_name(&ty_str),
                            &TejxType::from_name(&init_type),
                        )
                    {
                        if init_type == "[]" {
                            self.report_error_detailed(
                                format!(
                                    "Type mismatch: expected '{}', got empty array",
                                    ty_str
                                ),
                                *line,
                                *_col,
                                "E0100",
                                Some(&format!(
                                    "Empty arrays must be explicitly typed or match the target type '{}'",
                                    ty_str
                                )),
                            );
                        } else {
                            self.report_error_detailed(
                                format!(
                                    "Type mismatch: expected '{}', got '{}'",
                                    ty_str, init_type
                                ),
                                *line,
                                *_col,
                                "E0100",
                                Some(&format!(
                                    "Consider converting with 'as {}' or change the variable type",
                                    ty_str
                                )),
                            );
                        }
                    }

                    let _target_type = if is_explicit_any {
                        "any".to_string()
                    } else if !has_explicit_type {
                        if init_type == "<inferred>" || init_type == "[]" || init_type == "any" {
                            "{unknown}".to_string()
                        } else {
                            init_type.clone()
                        }
                    } else {
                        ty_str
                    };

                    let _ = self.define_pattern(pattern, _target_type, *is_const, *line, *_col);
                } else if !has_explicit_type {
                    self.report_error_detailed(
                        "Type annotation required for uninitialized variable".to_string(),
                        *line,
                        *_col,
                        "E0101",
                        Some("Provide an explicit type (e.g., 'let x: int;')"),
                    );
                    let _ = self.define_pattern(
                        pattern,
                        "{unknown}".to_string(),
                        *is_const,
                        *line,
                        *_col,
                    );
                } else {
                    let _ = self.define_pattern(
                        pattern,
                        type_annotation.to_string(),
                        *is_const,
                        *line,
                        *_col,
                    );
                }
                Ok(())
            }
            Statement::ExpressionStmt {
                _expression: expression,
                ..
            } => {
                self.check_expression(expression)?;
                Ok(())
            }
            Statement::DelStmt {
                target,
                _line,
                _col,
            } => {
                let _target_type = self.check_expression(target)?;
                if let Expression::Identifier { name, .. } = &**target {
                    if let Some(_s) = self.lookup(name) {
                        // deleted variable tracking removed along with borrow check
                    }
                } else if let Expression::MemberAccessExpr { .. } = &**target {
                    // Allowed to delete properties from objects
                } else {
                    self.report_error_detailed(
                        "Invalid target for 'del'".to_string(),
                        *_line,
                        *_col,
                        "E0100",
                        Some("'del' can only be used with variables or object properties"),
                    );
                }
                Ok(())
            }
            Statement::BlockStmt { statements, .. } => {
                self.enter_scope();
                for s in statements {
                    self.collect_declarations(s);
                }
                for (i, s) in statements.iter().enumerate() {
                    // SOI: Store remaining statements for look-ahead
                    self.remaining_stmts = statements[i + 1..].to_vec();
                    let _ = self.check_statement(s);
                }
                self.remaining_stmts.clear();
                self.exit_scope();
                Ok(())
            }
            Statement::IfStmt {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let _ = self.check_expression(condition)?;

                // Attempt type narrowing
                if let Some((name, narrowed_type, other_type)) =
                    self.get_narrowing_from_condition(condition)
                {
                    // Then branch narrowing
                    self.enter_scope();
                    if !narrowed_type.is_empty() {
                        self.define(name.clone(), narrowed_type);
                    }
                    self.check_statement(then_branch)?;
                    self.exit_scope();

                    // Else branch narrowing
                    if let Some(else_stmt) = else_branch {
                        self.enter_scope();
                        if !other_type.is_empty() {
                            self.define(name.clone(), other_type);
                        }
                        self.check_statement(else_stmt)?;
                        self.exit_scope();
                    }
                } else {
                    self.check_statement(then_branch)?;
                    if let Some(else_stmt) = else_branch {
                        self.check_statement(else_stmt)?;
                    }
                }
                Ok(())
            }
            Statement::WhileStmt {
                condition, body, ..
            } => {
                self.check_expression(condition)?;
                self.loop_depth += 1;

                // Two-pass check for move semantics in loops
                let _ = self.check_statement(body);
                let res = self.check_statement(body);

                self.loop_depth -= 1;
                res
            }
            Statement::ForStmt {
                init,
                condition,
                increment,
                body,
                ..
            } => {
                self.enter_scope();
                if let Some(init_stmt) = init {
                    // Special case: if init is a BlockStmt (e.g. from multiple declarations),
                    // we don't want it to start a nested scope that ends before the loop starts.
                    if let Statement::BlockStmt { statements, .. } = init_stmt.as_ref() {
                        for s in statements {
                            self.check_statement(s)?;
                        }
                    } else {
                        self.check_statement(init_stmt)?;
                    }
                }
                if let Some(cond_expr) = condition {
                    self.check_expression(cond_expr)?;
                }

                self.loop_depth += 1;
                // Two-pass check for move semantics in loops
                let _ = self.check_statement(body);
                if let Some(inc_expr) = increment {
                    let _ = self.check_expression(inc_expr);
                }

                let res = self.check_statement(body);
                if let Some(inc_expr) = increment {
                    self.check_expression(inc_expr)?;
                }
                self.loop_depth -= 1;

                self.exit_scope();
                res
            }
            Statement::BreakStmt { _line, _col } => {
                if self.loop_depth == 0 {
                    self.report_error_detailed(
                        "'break' can only be used inside a loop".to_string(),
                        *_line,
                        *_col,
                        "E0112",
                        Some("'break' can only be used inside 'for' or 'while' loops"),
                    );
                }
                Ok(())
            }
            Statement::ContinueStmt { _line, _col } => {
                if self.loop_depth == 0 {
                    self.report_error_detailed(
                        "'continue' can only be used inside a loop".to_string(),
                        *_line,
                        *_col,
                        "E0112",
                        Some("'continue' can only be used inside 'for' or 'while' loops"),
                    );
                }
                Ok(())
            }
            Statement::FunctionDeclaration(func) => {
                let mut ret_ty = if func.return_type.to_string().is_empty() {
                    "void".to_string()
                } else {
                    func.return_type.to_string()
                };
                if func._is_async && !ret_ty.starts_with("Promise<") {
                    ret_ty = format!("Promise<{}>", ret_ty);
                }
                let mut is_variadic = false;
                let min_required = func
                    .params
                    .iter()
                    .filter(|p| p._default_value.is_none() && !p._is_rest)
                    .count();
                let mut params: Vec<String> = Vec::new();
                for p in &func.params {
                    if p._is_rest {
                        is_variadic = true;
                    }
                    let mut p_ty = p.type_name.to_string();
                    if p_ty.is_empty() {
                        self.report_error_detailed(
                            format!("Type annotation required for parameter '{}'", p.name),
                            func._line,
                            func._col,
                            "E0101",
                            Some(
                                "Provide an explicit type (e.g., 'function f(x: int) { ... }') or use 'any' explicitly",
                            ),
                        );
                        p_ty = "<inferred>".to_string();
                    }
                    params.push(p_ty);
                }
                let has_defaults = min_required < params.len();
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        func.name.clone(),
                        Symbol {
                            ty: TejxType::Function(
                                params.iter().map(|p| TejxType::from_name(p)).collect(),
                                Box::new(TejxType::from_name(&ret_ty)),
                            ),
                            is_const: false,
                            min_params: if has_defaults {
                                Some(min_required)
                            } else {
                                None
                            },
                            params: params.iter().map(|p| TejxType::from_name(p)).collect(),
                            is_variadic,
                            aliased_type: None,
                            generic_params: func.generic_params.clone(),
                        },
                    );
                }

                self.current_function_return = Some(TejxType::from_name(&ret_ty));
                self.current_function_is_async = func._is_async;
                self.enter_scope();
                // Register function-level generic params as valid types
                for gp in &func.generic_params {
                    self.define(gp.name.clone(), gp.name.clone());
                }
                for (idx, param) in func.params.iter().enumerate() {
                    let param_ty = params
                        .get(idx)
                        .cloned()
                        .unwrap_or_else(|| "<inferred>".to_string());
                    self.define_with_params(param.name.clone(), param_ty, Vec::new());
                }
                self.check_statement(&func.body)?;
                self.exit_scope();
                self.current_function_return = None;
                self.current_function_is_async = false;
                Ok(())
            }
            Statement::ClassDeclaration(class_decl) => {
                self.current_class = Some(class_decl.name.clone());
                // Removed self.define(...) because collect_declarations already inserted the symbol WITH correctly parsed generic parameters.

                // Verify parent exists
                if !class_decl._parent_name.is_empty()
                    && self.lookup(&class_decl._parent_name).is_none() {
                        self.report_error_detailed(
                            format!("Parent class '{}' not found", class_decl._parent_name),
                            class_decl._line,
                            class_decl._col,
                            "E0101",
                            Some("Ensure the parent class is defined before the child class"),
                        );
                    }

                // Verify interface implementation
                for interface_name in &class_decl._implemented_protocols {
                    if self.lookup(interface_name).is_some() {
                        let required_methods = self.interfaces.get(interface_name).cloned();
                        if let Some(req_methods) = required_methods {
                            let mut class_method_names = Vec::new();
                            for m in &class_decl.methods {
                                class_method_names.push(m.func.name.clone());
                            }
                            for (req_name, _) in req_methods {
                                if !class_method_names.contains(&req_name) {
                                    self.report_error_detailed(format!("Class '{}' missing method '{}' required by interface '{}'", class_decl.name, req_name, interface_name), class_decl._line, class_decl._col, "E0111", Some(&format!("Add method '{}' to class '{}' to satisfy the interface contract", req_name, class_decl.name)));
                                }
                            }
                        }
                    } else {
                        self.report_error_detailed(format!("Interface '{}' not found", interface_name), class_decl._line, class_decl._col, "E0101", Some("Define the interface before implementing it, or check the spelling"));
                    }
                }

                self.enter_scope();
                let mut this_type = class_decl.name.clone();
                if !class_decl.generic_params.is_empty() {
                    let gp_names: Vec<String> = class_decl
                        .generic_params
                        .iter()
                        .map(|gp| gp.name.clone())
                        .collect();
                    this_type = format!("{}<{}>", this_type, gp_names.join(", "));
                }
                self.define("this".to_string(), this_type.clone());
                // Register class-level generic params as valid types
                for gp in &class_decl.generic_params {
                    self.define(gp.name.clone(), gp.name.clone());
                }
                if !class_decl._parent_name.is_empty() {
                    self.define("super".to_string(), class_decl._parent_name.clone());
                }

                for member in &class_decl._members {
                    let mut member_ty_str = member._type_name.to_string();
                    if member_ty_str.is_empty() {
                        self.report_error_detailed(
                            format!("Type annotation required for member '{}'", member._name),
                            class_decl._line,
                            class_decl._col,
                            "E0101",
                            Some(
                                "Provide an explicit type (e.g., 'field: int;') or use 'any' explicitly",
                            ),
                        );
                        member_ty_str = "<inferred>".to_string();
                    }
                    if member_ty_str != "any"
                        && !member_ty_str.is_empty()
                        && !self.is_valid_type(&TejxType::from_name(&member_ty_str))
                    {
                        self.report_error_detailed(
                            format!("Unknown data type: '{}'", member_ty_str),
                            class_decl._line,
                            class_decl._col,
                            "E0101",
                            Some(
                                "Valid types include: int, int32, float, float64, string, bool, or user-defined classes",
                            ),
                        );
                    }

                    if let Some(init) = &member._initializer {
                        let prev_expected = self.current_expected_type.take();
                        let prev_lambda_ctx = self.lambda_context_params.take();
                        if member_ty_str != "any" && !member_ty_str.is_empty() {
                            let expected_ty = TejxType::from_name(&member_ty_str);
                            self.current_expected_type = Some(expected_ty.clone());
                            if let TejxType::Function(params, _) = expected_ty {
                                self.lambda_context_params = Some(params);
                            }
                        }

                        let init_type = self.check_expression(init)?.to_name();
                        self.current_expected_type = prev_expected;
                        self.lambda_context_params = prev_lambda_ctx;

                        if member_ty_str != "any"
                            && !member_ty_str.is_empty()
                            && !self.are_types_compatible(
                                &TejxType::from_name(&member_ty_str),
                                &TejxType::from_name(&init_type),
                            )
                        {
                            self.report_error_detailed(
                                format!(
                                    "Type mismatch: expected '{}', got '{}'",
                                    member_ty_str, init_type
                                ),
                                class_decl._line,
                                class_decl._col,
                                "E0100",
                                Some(&format!(
                                    "Consider converting with 'as {}' or change the member type",
                                    member_ty_str
                                )),
                            );
                        }
                    }
                }

                for method in &class_decl.methods {
                    self.enter_scope();
                    // Register method-level generic params as valid types
                    for gp in &method.func.generic_params {
                        self.define(gp.name.clone(), gp.name.clone());
                    }
                    for param in &method.func.params {
                        let mut param_ty = param.type_name.to_string();
                        if param_ty.is_empty() {
                            self.report_error_detailed(
                                format!("Type annotation required for parameter '{}'", param.name),
                                class_decl._line,
                                class_decl._col,
                                "E0101",
                                Some(
                                    "Provide an explicit type (e.g., 'method(x: int) { ... }') or use 'any' explicitly",
                                ),
                            );
                            param_ty = "<inferred>".to_string();
                        }

                        if param_ty != "any"
                            && !param_ty.is_empty()
                            && !self.is_valid_type(&TejxType::from_name(&param_ty))
                        {
                            self.report_error_detailed(format!("Unknown data type: '{}'", param_ty), class_decl._line, class_decl._col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, or user-defined classes"));
                        }
                        self.define(param.name.clone(), param_ty);
                    }
                    let prev_return = self.current_function_return.take();
                    let prev_async = self.current_function_is_async;
                    let ret_ty = TejxType::from_node(&method.func.return_type);
                    if ret_ty != TejxType::Any
                        && ret_ty != TejxType::Void
                        && !self.is_valid_type(&ret_ty)
                    {
                        self.report_error_detailed(format!("Unknown data type: '{}' for return type of method '{}'", ret_ty.to_name(), method.func.name), class_decl._line, class_decl._col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, void, or user-defined classes"));
                    }
                    self.current_function_return = Some(ret_ty);
                    self.current_function_is_async = method.func._is_async;

                    self.check_statement(&method.func.body)?;

                    self.current_function_return = prev_return;
                    self.current_function_is_async = prev_async;
                    self.exit_scope();
                }

                if let Some(constructor) = &class_decl._constructor {
                    self.enter_scope();
                    for param in &constructor.params {
                        self.define(param.name.clone(), param.type_name.to_string());
                    }
                    let prev_return = self.current_function_return.take();
                    self.current_function_return = Some(TejxType::Void);
                    self.check_statement(&constructor.body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                for getter in &class_decl._getters {
                    self.enter_scope();
                    let prev_return = self.current_function_return.take();
                    self.current_function_return = Some(TejxType::from_node(&getter._return_type));
                    self.check_statement(&getter._body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                for setter in &class_decl._setters {
                    self.enter_scope();
                    self.define(setter._param_name.clone(), setter._param_type.to_string());
                    let prev_return = self.current_function_return.take();
                    self.current_function_return = Some(TejxType::Void);
                    self.check_statement(&setter._body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                self.exit_scope();
                self.current_class = None;
                Ok(())
            }
            Statement::ReturnStmt {
                value,
                _line: line,
                _col: col,
            } => {
                let expected = self.current_function_return.clone();
                if let Some(expected_original_ty) = expected {
                    let expected_original = expected_original_ty.to_name();
                    let got = if let Some(expr) = value {
                        let prev_expected = self.current_expected_type.take();
                        let prev_lambda_ctx = self.lambda_context_params.take();
                        self.current_expected_type = Some(expected_original_ty.clone());
                        if let TejxType::Function(params, _) = &expected_original_ty {
                            self.lambda_context_params = Some(params.clone());
                        } else if self.current_function_is_async {
                            if let TejxType::Class(n, g) = &expected_original_ty {
                                if n == "Promise" && g.len() == 1 {
                                    if let TejxType::Function(params, _) = &g[0] {
                                        self.lambda_context_params = Some(params.clone());
                                    }
                                }
                            }
                        }

                        let result = self.check_expression(expr)?.to_name();
                        self.current_expected_type = prev_expected;
                        self.lambda_context_params = prev_lambda_ctx;
                        result
                    } else {
                        "void".to_string()
                    };

                    if expected_original == "<inferred>" {
                        self.current_function_return = Some(TejxType::from_name(&got));
                        return Ok(());
                    }

                    let expected_type = expected_original.clone();
                    // If async, expected_type is Promise<T>, but we allow returning T
                    if self.current_function_is_async && expected_type.starts_with("Promise<") {
                        let inner = &expected_type[8..expected_type.len() - 1];
                        let is_numeric = |t: &str| -> bool {
                            matches!(
                                t,
                                "int"
                                    | "int16"
                                    | "int32"
                                    | "int64"
                                    | "int128"
                                    | "float"
                                    | "float16"
                                    | "float32"
                                    | "float64"
                            )
                        };
                        let is_bool = |t: &str| -> bool { matches!(t, "bool") };

                        if got == inner
                            || (is_numeric(inner) && is_numeric(&got))
                            || (is_bool(inner) && is_bool(&got))
                        {
                            // Implicit wrap: OK
                            return Ok(());
                        }
                    }

                    if !self.is_assignable(
                        &TejxType::from_name(&expected_type),
                        &TejxType::from_name(&got),
                    ) {
                        let is_numeric = |t: &str| -> bool {
                            matches!(
                                t,
                                "int"
                                    | "int16"
                                    | "int32"
                                    | "int64"
                                    | "int128"
                                    | "float"
                                    | "float16"
                                    | "float32"
                                    | "float64"
                            )
                        };
                        let is_bool = |t: &str| -> bool { matches!(t, "bool") };

                        if (is_numeric(&expected_type) && is_numeric(&got))
                            || (is_bool(&expected_type) && is_bool(&got))
                        {
                            // Ok
                        } else {
                            self.report_error_detailed(format!("Return type mismatch: expected '{}', got '{}'", expected_original, got), *line, *col, "E0107", Some(&format!("The function signature declares return type '{}'; ensure the returned value matches", expected_original)));
                        }
                    }
                }
                Ok(())
            }
            Statement::EnumDeclaration(enum_decl) => {
                self.define(enum_decl.name.clone(), "enum".to_string());
                // Define members as static properties of enum?
                // For simplified type check, just defining enum name is enough to pass basic checks.
                Ok(())
            }
            Statement::TypeAliasDeclaration { .. } => {
                // Already handled in collect_declarations
                Ok(())
            }
            Statement::InterfaceDeclaration { name, .. } => {
                self.define(name.clone(), "interface".to_string());
                Ok(())
            }
            Statement::ExportDecl { declaration, .. } => {
                self.check_statement(declaration)?;
                Ok(())
            }
            Statement::ImportDecl { _names, source, .. } => {
                // Register imported names/namespaces so type checker doesn't reject them
                if _names.is_empty() {
                    // Whole-module import: `import std:time;` → register `time` as any
                    let module_name = if source.starts_with("std:") {
                        source.trim_start_matches("std:").to_string()
                    } else {
                        std::path::Path::new(source)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default()
                    };
                    if !module_name.is_empty() {
                        self.define(module_name, "<inferred>".to_string());
                    }
                } else {
                    // Named imports: `import { parse, stringify } from "std:json"`
                    for item in _names {
                        if self.lookup(&item.name).is_none() {
                            self.define(item.name.clone(), "<inferred>".to_string());
                        }
                    }
                }
                Ok(())
            }
            Statement::ExtensionDeclaration(ext_decl) => {
                let name = &ext_decl._target_type;
                let methods = &ext_decl._methods;

                let mut existing_members = self
                    .class_members
                    .get(&name.to_string())
                    .cloned()
                    .unwrap_or(HashMap::new());
                for method in methods {
                    let m_name = &method.name;
                    // Build method type string
                    let mut param_types = Vec::new();
                    for p in &method.params {
                        param_types.push(p.type_name.to_string());
                    }
                    let p_str = param_types.join(",");
                    let type_str = format!("function:{}:{}", method.return_type.to_string(), p_str);

                    existing_members.insert(
                        m_name.clone(),
                        MemberInfo {
                            ty: TejxType::from_name(&type_str),
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true,
                            generic_params: method.generic_params.clone(),
                        },
                    );

                    // Check method body
                    let prev_class = self.current_class.clone();
                    self.current_class = Some(name.to_string());

                    self.enter_scope();
                    self.define("this".to_string(), name.to_string());

                    for param in &method.params {
                        self.define(param.name.clone(), param.type_name.to_string());
                    }

                    let prev_return = self.current_function_return.take();
                    let prev_async = self.current_function_is_async;

                    let ret_ty = if method.return_type.to_string().is_empty() {
                        "void".to_string()
                    } else {
                        method.return_type.to_string()
                    };
                    self.current_function_return = Some(TejxType::from_name(&ret_ty));
                    self.current_function_is_async = method._is_async;

                    self.check_statement(&method.body)?;

                    self.current_function_return = prev_return;
                    self.current_function_is_async = prev_async;

                    self.exit_scope();
                    self.current_class = prev_class;
                }
                self.class_members
                    .insert(name.to_string(), existing_members);
                Ok(())
            }
            // Statement::ProtocolDeclaration(_) => Ok(()), // Removed
            _ => Ok(()), // Catch-all for others
        }
    }

    pub(crate) fn define_pattern(
        &mut self,
        pattern: &BindingNode,
        type_name: String,
        is_const: bool,
        line: usize,
        col: usize,
    ) -> Result<(), ()> {
        match pattern {
            BindingNode::Identifier(name) => {
                self.define_variable(name.clone(), type_name, is_const, line, col);
            }
            BindingNode::ArrayBinding { elements, rest } => {
                let parsed = TejxType::from_name(&type_name);
                let inner_type = match parsed {
                    TejxType::DynamicArray(inner)
                    | TejxType::FixedArray(inner, _)
                    | TejxType::Slice(inner) => inner.to_name(),
                    _ => "<inferred>".to_string(),
                };

                for el in elements {
                    let _ = self.define_pattern(el, inner_type.clone(), is_const, line, col);
                }
                if let Some(rest_pattern) = rest {
                    let rest_type = if inner_type == "<inferred>" {
                        "<inferred>".to_string()
                    } else {
                        format!("{}[]", inner_type)
                    };
                    let _ = self.define_pattern(
                        rest_pattern,
                        rest_type,
                        is_const,
                        line,
                        col,
                    );
                }
            }
            BindingNode::ObjectBinding { entries } => {
                let parsed_ty = TejxType::from_name(&type_name);
                for (key, target) in entries {
                    let mut prop_ty = "<inferred>".to_string();
                    if let TejxType::Object(props) = &parsed_ty {
                        for (k, _, t) in props {
                            if k == key {
                                prop_ty = t.to_name();
                                break;
                            }
                        }
                    } else if type_name != "<inferred>" && !type_name.is_empty() {
                        if let Some(info) = self.resolve_instance_member(&type_name, key) {
                            prop_ty = self.substitute_generics(&info.ty.to_name(), &type_name);
                        }
                    }
                    let _ = self.define_pattern(target, prop_ty, is_const, line, col);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn get_narrowing_from_condition(
        &self,
        condition: &Expression,
    ) -> Option<(String, String, String)> {
        match condition {
            Expression::UnaryExpr {
                op: TokenType::Bang,
                right,
                ..
            } => {
                if let Some((name, then_ty, else_ty)) = self.get_narrowing_from_condition(right) {
                    return Some((name, else_ty, then_ty));
                }
                None
            }
            Expression::BinaryExpr {
                left, op, right, ..
            } => {
                if *op == TokenType::Instanceof {
                    if let (
                        Expression::Identifier { name: var_name, .. },
                        Expression::Identifier {
                            name: type_name, ..
                        },
                    ) = (left.as_ref(), right.as_ref())
                    {
                        if let Some(sym) = self.lookup(var_name) {
                            let original_type = sym.ty.to_name();
                            return Some((var_name.clone(), type_name.clone(), original_type));
                        }
                    }
                }

                let name;
                let is_not_none;

                match (left.as_ref(), right.as_ref()) {
                    (Expression::Identifier { name: n, .. }, Expression::NoneLiteral { .. }) => {
                        name = n.clone();
                        is_not_none = *op == TokenType::BangEqual;
                    }
                    (Expression::NoneLiteral { .. }, Expression::Identifier { name: n, .. }) => {
                        name = n.clone();
                        is_not_none = *op == TokenType::BangEqual;
                    }
                    _ => return None,
                }

                if *op != TokenType::BangEqual && *op != TokenType::EqualEqual {
                    return None;
                }

                if let Some(sym) = self.lookup(&name) {
                    let original_type = sym.ty.to_name().clone();
                    if original_type.contains('|') {
                        let non_none = self
                            .strip_none_from_union(&TejxType::from_name(&original_type))
                            .to_name();
                        if is_not_none {
                            // then: non_none, else: None
                            return Some((name, non_none, "None".to_string()));
                        } else {
                            // then: None, else: non_none
                            return Some((name, "None".to_string(), non_none));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}
