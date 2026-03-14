use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::intrinsics::*;
use crate::token::TokenType;
use crate::types::TejxType;

impl Lowering {
    pub(crate) fn lower_statement(&self, stmt: &Statement) -> Option<HIRStatement> {
        let line = stmt.get_line();
        match stmt {
            Statement::TypeAliasDeclaration { name, _type_def, .. } => {
                let alias_ty = TejxType::from_node(_type_def);
                self.register_type_alias(name, alias_ty);
                None
            }
            Statement::ExportDecl { declaration, .. } => {
                if let Statement::TypeAliasDeclaration { name, _type_def, .. } =
                    declaration.as_ref()
                {
                    let alias_ty = TejxType::from_node(_type_def);
                    self.register_type_alias(name, alias_ty);
                    return None;
                }
                // Fall through to handle other exports
                self.lower_statement(declaration)
            }
            Statement::BlockStmt { statements, .. } => {
                self.enter_scope();

                // Pre-pass: Hoist function declarations within the block
                for s in statements {
                    if let Statement::FunctionDeclaration(func) = s {
                        let name = if func.is_extern {
                            func.name.clone()
                        } else {
                            format!("f_{}", func.name)
                        };
                        if let Some((scope, _)) = self.scopes.borrow_mut().last_mut() {
                            scope.insert(
                                func.name.clone(),
                                (
                                    name.clone(),
                                    self.resolve_alias_type(&TejxType::from_node(&func.return_type)),
                                ),
                            );
                        }
                        self.user_functions.borrow_mut().insert(
                            name.clone(),
                            self.resolve_alias_type(&TejxType::from_node(&func.return_type)),
                        );
                        self.user_function_args
                            .borrow_mut()
                            .insert(name, func.params.len());
                    }
                }

                let mut hir_stmts = Vec::new();
                for s in statements {
                    if let Some(h) = self.lower_statement(s) {
                        hir_stmts.push(h);
                    }
                }
                self._exit_scope();
                Some(HIRStatement::Block {
                    line: line,
                    statements: hir_stmts,
                })
            }
            Statement::VarDeclaration {
                pattern,
                type_annotation,
                initializer,
                is_const,
                ..
            } => {
                let expected_ty = if type_annotation.to_string().is_empty() {
                    None
                } else {
                    Some(self.resolve_alias_type(&TejxType::from_node(&type_annotation)))
                };

                let prev_expected = self.current_expected_type.borrow_mut().take();
                *self.current_expected_type.borrow_mut() = expected_ty.clone();
                let mut init = initializer.as_ref().map(|e| self.lower_expression(e));
                *self.current_expected_type.borrow_mut() = prev_expected;

                let ty = if let Some(expected) = expected_ty {
                    expected
                } else {
                    init.as_ref()
                        .map(|e| e.get_type())
                        .unwrap_or(TejxType::Int64)
                };

                // Sized allocations are now handled during AST parsing/transforming where possible,
                // or we can extract size from TypeNode::Array if needed later.

                let mut stmts = Vec::new();
                self.lower_binding_pattern(pattern, init, &ty, *is_const, &mut stmts);
                // Use Sequence to avoid creating a new scope, allowing variables to be visible in the containing block
                Some(HIRStatement::Sequence {
                    line: line,
                    statements: stmts,
                })
            }
            Statement::ExpressionStmt { _expression, .. } => {
                let expr = self.lower_expression(_expression);
                Some(HIRStatement::ExpressionStmt { line: line, expr })
            }
            Statement::WhileStmt {
                condition, body, ..
            } => {
                let cond = self.lower_expression(condition);
                let body_hir = self.lower_statement_as_block(body);
                Some(HIRStatement::Loop {
                    line: line,
                    condition: cond,
                    body: Box::new(body_hir),
                    increment: None,
                    _is_do_while: false,
                })
            }
            Statement::ForStmt {
                init,
                condition,
                increment,
                body,
                ..
            } => {
                self.enter_scope();
                let mut outer_stmts = Vec::new();
                if let Some(init_stmt) = init {
                    if let Statement::BlockStmt { statements, .. } = init_stmt.as_ref() {
                        for s in statements {
                            if let Some(h) = self.lower_statement(s) {
                                outer_stmts.push(h);
                            }
                        }
                    } else {
                        if let Some(h) = self.lower_statement(init_stmt) {
                            outer_stmts.push(h);
                        }
                    }
                }
                let cond = condition
                    .as_ref()
                    .map(|c| self.lower_expression(c))
                    .unwrap_or(HIRExpression::Literal {
                        line: line,
                        value: "true".to_string(),
                        ty: TejxType::Bool,
                    });

                let body_hir = self.lower_statement_as_block(body);

                let inc = increment.as_ref().map(|e| {
                    let expr = self.lower_expression(e);
                    Box::new(HIRStatement::ExpressionStmt { line: line, expr })
                });

                outer_stmts.push(HIRStatement::Loop {
                    line: line,
                    condition: cond,
                    body: Box::new(body_hir),
                    increment: inc,
                    _is_do_while: false,
                });

                self._exit_scope();

                Some(HIRStatement::Block {
                    line: line,
                    statements: outer_stmts,
                })
            }
            Statement::ForOfStmt {
                variable,
                iterable,
                body,
                ..
            } => {
                // Desugar:
                // match variable { BindingNode::Identifier(var_name) => ... }
                // let _arr = iterable;
                // let _len = _arr.length;
                // let _i = 0;
                // while (_i < _len) {
                //    let var_name = _arr[_i];
                //    body;
                //    _i = _i + 1;
                // }

                if let BindingNode::Identifier(var_name) = variable {
                    let mut stmts = Vec::new();

                    // 1. Evaluate iterable once
                    let iter_expr = self.lower_expression(iterable);
                    let array_ty = iter_expr.get_type().clone();

                    let arr_name = format!("__arr_{}", var_name);
                    stmts.push(HIRStatement::VarDecl {
                        line: line,
                        name: arr_name.clone(),
                        initializer: Some(iter_expr),
                        ty: array_ty.clone(),
                        _is_const: true,
                    });

                    // 2. Length
                    let len_name = format!("__len_{}", var_name);
                    stmts.push(HIRStatement::VarDecl {
                        line: line,
                        name: len_name.clone(),
                        initializer: Some(HIRExpression::Call {
                            line: line,
                            callee: RT_LEN.to_string(),
                            args: vec![HIRExpression::Variable {
                                line: line,
                                name: arr_name.clone(),
                                ty: array_ty.clone(),
                            }],
                            ty: TejxType::Int32,
                        }),
                        ty: TejxType::Int32,
                        _is_const: true,
                    });

                    // 3. Index
                    let idx_name = format!("__idx_{}", var_name);
                    stmts.push(HIRStatement::VarDecl {
                        line: line,
                        name: idx_name.clone(),
                        initializer: Some(HIRExpression::Literal {
                            line: 0,
                            value: "0".to_string(),
                            ty: TejxType::Int32,
                        }),
                        ty: TejxType::Int32,
                        _is_const: false,
                    });

                    // 4. Loop
                    // Condition: idx < len
                    let cond = HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(HIRExpression::Variable {
                            line: line,
                            name: idx_name.clone(),
                            ty: TejxType::Int32,
                        }),
                        op: TokenType::Less,
                        right: Box::new(HIRExpression::Variable {
                            line: line,
                            name: len_name,
                            ty: TejxType::Int32,
                        }),
                        ty: TejxType::Bool,
                    };

                    // Body construction
                    let mut body_stmts = Vec::new();

                    let elem_ty = array_ty.get_array_element_type();
                    let val_expr = HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
                            name: arr_name,
                            ty: array_ty.clone(),
                        }),
                        index: Box::new(HIRExpression::Variable {
                            line: line,
                            name: idx_name.clone(),
                            ty: TejxType::Int32,
                        }),
                        ty: elem_ty.clone(),
                    };
                    body_stmts.push(HIRStatement::VarDecl {
                        line: line,
                        name: var_name.clone(),
                        initializer: Some(val_expr),
                        ty: elem_ty.clone(),
                        _is_const: false,
                    });
                    self.define(var_name.clone(), elem_ty.clone());

                    // User Body
                    if let Some(user_body) = self.lower_statement(body) {
                        body_stmts.push(user_body);
                    }

                    // Increment: idx = idx + 1
                    let inc_stmt = Box::new(HIRStatement::ExpressionStmt {
                        line: line,
                        expr: HIRExpression::Assignment {
                            line: line,
                            target: Box::new(HIRExpression::Variable {
                                line: line,
                                name: idx_name.clone(),
                                ty: TejxType::Int32,
                            }),
                            value: Box::new(HIRExpression::BinaryExpr {
                                line: line,
                                left: Box::new(HIRExpression::Variable {
                                    line: line,
                                    name: idx_name.clone(),
                                    ty: TejxType::Int32,
                                }),
                                op: TokenType::Plus,
                                right: Box::new(HIRExpression::Literal {
                                    line: line,
                                    value: "1".to_string(),
                                    ty: TejxType::Int32,
                                }),
                                ty: TejxType::Int32,
                            }),
                            ty: TejxType::Int32,
                        },
                    });

                    stmts.push(HIRStatement::Loop {
                        line: line,
                        condition: cond,
                        body: Box::new(HIRStatement::Block {
                            line: line,
                            statements: body_stmts,
                        }),
                        increment: Some(inc_stmt),
                        _is_do_while: false,
                    });

                    Some(HIRStatement::Block {
                        line: line,
                        statements: stmts,
                    })
                } else {
                    None // Destructuring in for-of not supported yet
                }
            }
            Statement::IfStmt {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let cond = self.lower_expression(condition);
                let narrowing = self.get_narrowing_from_condition(condition);

                self.enter_scope();
                if let Some((ref name, ref then_ty, _)) = narrowing {
                    if then_ty.to_name() != "None" && !then_ty.to_name().is_empty() {
                        self.narrow_type(name.clone(), then_ty.clone());
                    }
                }
                
                let then_hir = self
                    .lower_statement(then_branch)
                    .unwrap_or(HIRStatement::Block {
                        line: line,
                        statements: vec![],
                    });
                self._exit_scope();

                let else_hir = else_branch.as_ref().map(|e| {
                    self.enter_scope();
                    if let Some((ref name, _, ref else_ty)) = narrowing {
                        if else_ty.to_name() != "None" && !else_ty.to_name().is_empty() {
                            self.narrow_type(name.clone(), else_ty.clone());
                        }
                    }
                    let res = self.lower_statement(e).unwrap_or(HIRStatement::Block {
                        line: line,
                        statements: vec![],
                    });
                    self._exit_scope();
                    res
                });

                Some(HIRStatement::If {
                    line: line,
                    condition: cond,
                    then_branch: Box::new(then_hir),
                    else_branch: else_hir.map(Box::new),
                })
            }
            Statement::ReturnStmt { value, .. } => {
                let val = value.as_ref().map(|e| self.lower_expression(e));
                if let Some(p_id) = self.current_async_promise_id.borrow().as_ref() {
                    let mut stmts = Vec::new();

                    // Prevent double evaluation of function calls or complex expressions
                    let is_pure = val.as_ref().map_or(true, |v| {
                        matches!(
                            v,
                            HIRExpression::Variable { .. } | HIRExpression::Literal { .. }
                        )
                    });

                    let promise_inner = |ty: &TejxType| -> Option<TejxType> {
                        match ty {
                            TejxType::Class(name, generics)
                                if name == "Promise" && !generics.is_empty() =>
                            {
                                Some(generics[0].clone())
                            }
                            TejxType::Class(name, _) if name.starts_with("Promise<") && name.ends_with('>') => {
                                Some(TejxType::from_name(&name[8..name.len() - 1]))
                            }
                            _ => None,
                        }
                    };

                    let val = val.map(|v| {
                        if let Some(inner) = promise_inner(&v.get_type()) {
                            HIRExpression::Await {
                                line,
                                expr: Box::new(v),
                                ty: inner,
                            }
                        } else {
                            v
                        }
                    });

                    let (eval_val, final_val) = if !is_pure {
                        let mut counter = self.lambda_counter.borrow_mut();
                        let id = *counter;
                        *counter += 1;
                        drop(counter);
                        let temp_name = format!("__async_ret_{}", id);
                        let ty = val.as_ref().unwrap().get_type();
                        let decl = HIRStatement::VarDecl {
                            name: temp_name.clone(),
                            initializer: val.clone(),
                            ty: ty.clone(),
                            _is_const: true,
                            line,
                        };
                        stmts.push(decl);
                        let var_expr = HIRExpression::Variable {
                            name: temp_name,
                            ty,
                            line,
                        };
                        (Some(var_expr.clone()), Some(var_expr))
                    } else {
                        (val.clone(), val.clone())
                    };

                    let resolved_expr =
                        if let Some(target_ty) = self.current_return_type.borrow().as_ref() {
                            if val.is_some()
                                && *target_ty != TejxType::Int64
                                && *target_ty != TejxType::Void
                            {
                                let mut counter = self.lambda_counter.borrow_mut();
                                let id = *counter;
                                *counter += 1;
                                drop(counter);
                                let temp_name = format!("__async_unbox_{}", id);
                                let decl = HIRStatement::VarDecl {
                                    name: temp_name.clone(),
                                    initializer: eval_val.clone(),
                                    ty: target_ty.clone(),
                                    _is_const: true,
                                    line,
                                };
                                stmts.push(decl);
                                Some(HIRExpression::Variable {
                                    name: temp_name,
                                    ty: target_ty.clone(),
                                    line,
                                })
                            } else {
                                eval_val.clone()
                            }
                        } else {
                            eval_val.clone()
                        };

                    stmts.push(HIRStatement::ExpressionStmt {
                        line: line,
                        expr: HIRExpression::Call {
                            line: line,
                            callee: "rt_promise_resolve".to_string(),
                            args: vec![
                                HIRExpression::Variable {
                                    line: line,
                                    name: p_id.clone(),
                                    ty: TejxType::Int64,
                                },
                                resolved_expr.unwrap_or(HIRExpression::Literal {
                                    line: line,
                                    value: "0".to_string(),
                                    ty: TejxType::Int64,
                                }),
                            ],
                            ty: TejxType::Void,
                        },
                    });
                    stmts.push(HIRStatement::ExpressionStmt {
                        line: line,
                        expr: HIRExpression::Call {
                            line: line,
                            callee: TEJX_DEC_ASYNC_OPS.to_string(),
                            args: vec![],
                            ty: TejxType::Void,
                        },
                    });
                    stmts.push(HIRStatement::Return {
                        line: line,
                        value: final_val,
                    });
                    Some(HIRStatement::Block {
                        line: line,
                        statements: stmts,
                    })
                } else {
                    Some(HIRStatement::Return {
                        line: line,
                        value: val,
                    })
                }
            }
            Statement::DelStmt { target, .. } => {
                let t = self.lower_expression(target);
                let ty = t.get_type();
                Some(HIRStatement::ExpressionStmt {
                    line,
                    expr: HIRExpression::Assignment {
                        line,
                        target: Box::new(t),
                        value: Box::new(HIRExpression::NoneLiteral { line }),
                        ty,
                    },
                })
            }
            Statement::BreakStmt { .. } => Some(HIRStatement::Break { line }),
            Statement::ContinueStmt { .. } => Some(HIRStatement::Continue { line }),
            Statement::SwitchStmt {
                condition, cases, ..
            } => {
                let cond = self.lower_expression(condition);
                let mut hir_cases = Vec::new();
                for c in cases {
                    let val = c.value.as_ref().map(|e| self.lower_expression(e));

                    let mut stmts = Vec::new();
                    for s in &c.statements {
                        if let Some(h) = self.lower_statement(s) {
                            stmts.push(h);
                        }
                    }
                    hir_cases.push(HIRCase {
                        value: val,
                        body: Box::new(HIRStatement::Block {
                            line: line,
                            statements: stmts,
                        }),
                    });
                }
                Some(HIRStatement::Switch {
                    line: line,
                    condition: cond,
                    cases: hir_cases,
                })
            }
            Statement::TryStmt {
                _try_block,
                _catch_var,
                _catch_block,
                _finally_block,
                ..
            } => {
                let try_hir = self
                    .lower_statement(_try_block)
                    .unwrap_or(HIRStatement::Block {
                        line: line,
                        statements: vec![],
                    });
                let mut catch_var_mangled = None;
                let catch_hir = {
                    self.enter_scope();
                    if !_catch_var.is_empty() {
                        catch_var_mangled = Some(self.define(
                            _catch_var.clone(),
                            TejxType::Class("Error".to_string(), vec![]),
                        ));
                    }
                    let res = self
                        .lower_statement(_catch_block)
                        .unwrap_or(HIRStatement::Block {
                            line: line,
                            statements: vec![],
                        });
                    self._exit_scope();
                    res
                };
                let finally_hir = _finally_block
                    .as_ref()
                    .and_then(|f| self.lower_statement(f));

                Some(HIRStatement::Try {
                    line: line,
                    try_block: Box::new(try_hir),
                    catch_var: catch_var_mangled,
                    catch_block: Box::new(catch_hir),
                    finally_block: finally_hir.map(Box::new),
                })
            }
            Statement::ThrowStmt { _expression, .. } => {
                let val = self.lower_expression(_expression);
                Some(HIRStatement::Throw {
                    line: line,
                    value: val,
                })
            }
            Statement::FunctionDeclaration(func) => {
                let params: Vec<(String, TejxType)> = func
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), TejxType::from_node(&p.type_name)))
                    .collect();
                let return_type = TejxType::from_node(&func.return_type);

                self.enter_scope();
                for (name, ty) in &params {
                    self.define(name.clone(), ty.clone());
                }

                let body = self
                    .lower_statement(&func.body)
                    .unwrap_or(HIRStatement::Block {
                        line: line,
                        statements: vec![],
                    });

                self._exit_scope();

                let name = if func.is_extern {
                    func.name.clone()
                } else {
                    format!("f_{}", func.name)
                };

                Some(HIRStatement::Function {
                    async_params: None,
                    line: line,
                    name: name.clone(),
                    params: params.clone(),
                    _return_type: return_type.clone(),
                    body: Box::new(body.clone()),
                    is_extern: func.is_extern,
                });

                self.nested_functions
                    .borrow_mut()
                    .push(HIRStatement::Function {
                        async_params: None,
                        line: line,
                        name,
                        params,
                        _return_type: return_type,
                        body: Box::new(body),
                        is_extern: func.is_extern,
                    });

                // Since it's extracted to global, we don't return an inline closure,
                // but if someone expects an inline block we return empty block, but really
                // it shouldn't be executed inline! Function declarations are hoisted anyway.
                None
            }
            _ => None,
        }
    }

    pub(crate) fn lower_statement_as_block(&self, stmt: &Statement) -> HIRStatement {
        let line = stmt.get_line();
        match self.lower_statement(stmt) {
            Some(HIRStatement::Block { statements, .. }) => {
                HIRStatement::Block { line, statements }
            }
            Some(other) => HIRStatement::Block {
                line,
                statements: vec![other],
            },
            None => HIRStatement::Block {
                line,
                statements: vec![],
            },
        }
    }

    pub(crate) fn get_narrowing_from_condition(
        &self,
        condition: &Expression,
    ) -> Option<(String, TejxType, TejxType)> {
        match condition {
            Expression::UnaryExpr { op: TokenType::Bang, right, .. } => {
                if let Some((name, then_ty, else_ty)) = self.get_narrowing_from_condition(right) {
                    return Some((name, else_ty, then_ty));
                }
                None
            }
            Expression::BinaryExpr {
                left, op, right, ..
            } => {
                if *op == TokenType::Instanceof {
                    if let (Expression::Identifier { name: var_name, .. }, Expression::Identifier { name: type_name, .. }) = (left.as_ref(), right.as_ref()) {
                        if let Some((_, original_ty)) = self.lookup(var_name) {
                            let original_type = original_ty.clone();
                            return Some((var_name.clone(), TejxType::from_name(type_name), original_type));
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

                if let Some((_, ty)) = self.lookup(&name) {
                    let original_type = ty.to_name();
                    if original_type.contains('|') {
                        let mut non_none_str = "".to_string();
                        for p in original_type.split('|') {
                            if p.trim() != "None" {
                                if !non_none_str.is_empty() {
                                    non_none_str.push_str(" | ");
                                }
                                non_none_str.push_str(p.trim());
                            }
                        }
                        let non_none = TejxType::from_name(&non_none_str);
                        let none_ty = TejxType::from_name("None");
                        if is_not_none {
                            return Some((name, non_none, none_ty));
                        } else {
                            return Some((name, none_ty, non_none));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }
}
