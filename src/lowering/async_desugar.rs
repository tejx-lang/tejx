use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::intrinsics::*;
use crate::types::TejxType;

impl Lowering {
    pub(crate) fn lower_async_function(
        &self,
        func: &FunctionDeclaration,
        functions: &mut Vec<HIRStatement>,
    ) {
        let line = func._line;
        let params: Vec<(String, TejxType)> = func
            .params
            .iter()
            .map(|p| (p.name.clone(), TejxType::from_node(&p.type_name)))
            .collect();

        self.enter_scope();
        let mangled_params: Vec<(String, TejxType)> = params
            .iter()
            .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
            .collect();

        let (worker, _state_struct, wrapper_body) = self.lower_async_function_impl(
            &func.name,
            &mangled_params,
            &func.return_type.to_string(),
            &func.body,
        );

        self._exit_scope();

        functions.push(worker);
        functions.push(_state_struct);

        functions.push(HIRStatement::Function {
            async_params: Some(params.to_vec()),
            line: line,
            name: format!("f_{}", func.name),
            params: mangled_params,
            _return_type: TejxType::Int64,
            body: Box::new(wrapper_body),
            is_extern: false,
        });
    }

    pub fn lower_async_function_impl(
        &self,
        name: &str,
        params: &[(String, TejxType)],
        return_type_decl: &str,
        body: &Statement,
    ) -> (HIRStatement, HIRStatement, HIRStatement) {
        let original_return = TejxType::from_name(return_type_decl);
        let actual_return_type = match &original_return {
            TejxType::Class(name, _) if name.starts_with("Promise<") && name.ends_with('>') => {
                let inner = &name[8..name.len() - 1];
                TejxType::from_name(inner)
            }
            _ => original_return.clone(),
        };
        *self.current_return_type.borrow_mut() = Some(actual_return_type);
        let line = body.get_line();
        let worker_name = format!("f_{}_worker", name);

        // --- Worker Function ---
        // any f_worker(any[] ctx) { ... }
        self.enter_scope();
        let ctx_name = "ctx".to_string();
        let ctx_ty = TejxType::DynamicArray(Box::new(TejxType::Int64));
        self.define(ctx_name.clone(), ctx_ty.clone());

        // unpack_promise_expr used to be here, now mir_lowering handles it
        let promise_id_var = self.define("promise_id_local".to_string(), TejxType::Int64);

        let mut worker_stmts = Vec::new();
        worker_stmts.push(HIRStatement::VarDecl {
            name: promise_id_var.clone(),
            initializer: None,
            ty: TejxType::Int64,
            _is_const: false,
            line: line,
        });
        *self.current_async_promise_id.borrow_mut() = Some(promise_id_var.clone());

        // 3. Lower original body
        let inner_body = self.lower_statement(body).unwrap_or(HIRStatement::Block {
            line: line,
            statements: vec![],
        });

        // 4. Wrap in Try/Catch
        let try_block = HIRStatement::Block {
            line: line,
            statements: vec![
                inner_body,
                HIRStatement::ExpressionStmt {
                    line: line,
                    expr: HIRExpression::Call {
                        line: line,
                        callee: RT_PROMISE_RESOLVE.to_string(),
                        args: vec![
                            HIRExpression::Variable {
                                line: line,
                                name: promise_id_var.clone(),
                                ty: TejxType::Int64,
                            },
                            HIRExpression::Literal {
                                line: line,
                                value: "0".to_string(),
                                ty: TejxType::Int64,
                            },
                        ],
                        ty: TejxType::Void,
                    },
                },
                HIRStatement::ExpressionStmt {
                    line: line,
                    expr: HIRExpression::Call {
                        line: line,
                        callee: TEJX_DEC_ASYNC_OPS.to_string(),
                        args: vec![],
                        ty: TejxType::Void,
                    },
                },
            ],
        };

        let catch_block = HIRStatement::Block {
            line: line,
            statements: vec![
                HIRStatement::ExpressionStmt {
                    line: line,
                    expr: HIRExpression::Call {
                        line: line,
                        callee: RT_PROMISE_REJECT.to_string(),
                        args: vec![
                            HIRExpression::Variable {
                                line: line,
                                name: promise_id_var.clone(),
                                ty: TejxType::Int64,
                            },
                            HIRExpression::Variable {
                                line: line,
                                name: "err".to_string(),
                                ty: TejxType::Class("Error".to_string(), vec![]),
                            },
                        ],
                        ty: TejxType::Void,
                    },
                },
                HIRStatement::ExpressionStmt {
                    line: line,
                    expr: HIRExpression::Call {
                        line: line,
                        callee: TEJX_DEC_ASYNC_OPS.to_string(),
                        args: vec![],
                        ty: TejxType::Void,
                    },
                },
            ],
        };

        worker_stmts.push(HIRStatement::Try {
            line: line,
            try_block: Box::new(try_block),
            catch_var: Some("err".to_string()),
            catch_block: Box::new(catch_block),
            finally_block: None,
        });

        let final_body = HIRStatement::Block {
            line: line,
            statements: worker_stmts,
        };

        *self.current_async_promise_id.borrow_mut() = None;
        self._exit_scope();

        let worker_func = HIRStatement::Function {
            name: worker_name.clone(),
            params: vec![(
                "ctx".to_string(),
                TejxType::DynamicArray(Box::new(TejxType::Int64)),
            )],
            _return_type: TejxType::Void,
            body: Box::new(final_body),
            is_extern: false,
            async_params: Some(params.to_vec()),
            line: line,
        };

        // --- Wrapper Body construction ---
        let mut wrapper_stmts = Vec::new();
        let p_var = format!("__p_{}", line);

        // let p = Promise_new();
        wrapper_stmts.push(HIRStatement::VarDecl {
            line: line,
            name: p_var.clone(),
            initializer: Some(HIRExpression::Call {
                line: line,
                callee: RT_PROMISE_NEW.to_string(),
                args: vec![],
                ty: TejxType::Int64,
            }),
            ty: TejxType::Int64,
            _is_const: false,
        });

        // let ctx = [p, 0 /*state*/, params...];
        let mut args_elems = vec![
            HIRExpression::Call {
                line: line,
                callee: RT_PROMISE_CLONE.to_string(),
                args: vec![HIRExpression::Variable {
                    line: line,
                    name: p_var.clone(),
                    ty: TejxType::Int64,
                }],
                ty: TejxType::Int64,
            },
            HIRExpression::Literal {
                line: line,
                value: "0".to_string(),
                ty: TejxType::Int32,
            },
        ];

        for (pname, pty) in params {
            let (mangled, _) = self.lookup(pname).unwrap_or((pname.clone(), pty.clone()));
            args_elems.push(HIRExpression::Variable {
                line: line,
                name: mangled,
                ty: pty.clone(),
            });
        }

        // Add safety padding for local variables crossing await points
        // Large async functions can generate many temps; keep this generous.
        for _ in 0..512 {
            args_elems.push(HIRExpression::Literal {
                line: line,
                value: "0".to_string(),
                ty: TejxType::Int32,
            });
        }

        let ctx_var = format!("__ctx_{}", line);
        let ctx_ty = TejxType::DynamicArray(Box::new(TejxType::Int64));
        wrapper_stmts.push(HIRStatement::VarDecl {
            line: line,
            name: ctx_var.clone(),
            initializer: Some(HIRExpression::ArrayLiteral {
                line: line,
                elements: args_elems,
                ty: ctx_ty.clone(),
                sized_allocation: None,
            }),
            ty: ctx_ty.clone(),
            _is_const: false,
        });

        // tejx_inc_async_ops();
        wrapper_stmts.push(HIRStatement::ExpressionStmt {
            line: line,
            expr: HIRExpression::Call {
                line: line,
                callee: TEJX_INC_ASYNC_OPS.to_string(),
                args: vec![],
                ty: TejxType::Void,
            },
        });

        // tejx_enqueue_task(worker_ptr, ctx);
        wrapper_stmts.push(HIRStatement::ExpressionStmt {
            line: line,
            expr: HIRExpression::Call {
                line: line,
                callee: TEJX_ENQUEUE_TASK.to_string(),
                args: vec![
                    HIRExpression::Literal {
                        line: line,
                        value: format!("@{}", worker_name),
                        ty: TejxType::Int64,
                    },
                    HIRExpression::Variable {
                        line: line,
                        name: ctx_var,
                        ty: ctx_ty.clone(),
                    },
                ],
                ty: TejxType::Void,
            },
        });

        // return p;
        wrapper_stmts.push(HIRStatement::Return {
            line: line,
            value: Some(HIRExpression::Variable {
                line: line,
                name: p_var,
                ty: TejxType::Int64,
            }),
        });

        (
            worker_func,
            HIRStatement::Block {
                line: 0,
                statements: vec![],
            }, // Dummy struct
            HIRStatement::Block {
                line: line,
                statements: wrapper_stmts,
            },
        )
    }
}
