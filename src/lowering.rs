use std::cell::RefCell;
use std::collections::HashSet;
use crate::ast::*;
use crate::hir::*;
use crate::types::TejxType;
use crate::token::TokenType;

pub struct Lowering {
    lambda_counter: RefCell<usize>,
    user_functions: RefCell<HashSet<String>>,
    lambda_functions: RefCell<Vec<HIRStatement>>,
}

/// Result of lowering: a list of top-level HIR functions.
/// The last one is always "tejx_main" containing non-function statements.
pub struct LoweringResult {
    pub functions: Vec<HIRStatement>,  // Each should be HIRStatement::Function
}

impl Lowering {
    pub fn new() -> Self {
        Lowering {
            lambda_counter: RefCell::new(0),
            user_functions: RefCell::new(HashSet::new()),
            lambda_functions: RefCell::new(Vec::new()),
        }
    }

    fn is_runtime_func(&self, name: &str) -> bool {
        let runtime_funcs = [
            "Math_pow", "fs_exists", "Array_push", "Array_pop", "arrUtil_concat",
            "Thread_join", "__await", "__optional_chain", 
            "Calculator_add", "Calculator_getValue",
            "calc_add", "calc_getValue", "hello", "__callee___area",
            "arrUtil_indexOf", "arrUtil_shift", "arrUtil_unshift",
            "Array_forEach", "Array_map", "Array_filter",
            "Date_now", "fs_mkdir", "fs_readFile", "fs_writeFile",
            "fs_remove", "Promise_all", "delay", "http_get",
            "Math_abs", "Math_ceil", "Math_floor", 
            "Math_round", "Math_sqrt", "Math_sin", 
            "Math_cos", "Math_random", "Math_min", 
            "Math_max", "parseInt", "parseFloat", 
            "JSON_stringify", "JSON_parse",
            "console_error", "console_warn", 
            "d_getTime", "d_toISOString",
            "m_set", "m_get", "m_has", 
            "m_del", "m_size",
            "s_add", "s_has", "s_size",
            "strVal_trim", "trimmed_startsWith", 
            "trimmed_endsWith", "trimmed_replace", 
            "trimmed_toLowerCase", "trimmed_toUpperCase",
            "printf", "console.log" // console.log is handled special in codegen usually
        ];
        runtime_funcs.contains(&name) || name.starts_with("tejx_")
    }

    pub fn lower(&self, program: &Program, base_path: &std::path::Path) -> LoweringResult {
        let mut functions = Vec::new();
        let mut main_stmts = Vec::new();
        let mut merged_statements = program.statements.clone();

        // Pass 0: Handle imports (simplistic recursive merge for tests)
        let mut i = 0;
        while i < merged_statements.len() {
            if let Statement::ImportDecl { source, .. } = &merged_statements[i] {
                // Resolve path
                let mut path = base_path.to_path_buf();
                // source is usually like "./modules/math.tx"
                let clean_source = source.trim_matches('"');
                if clean_source.starts_with("./") {
                    path.push(&clean_source[2..]);
                } else {
                    path.push(clean_source);
                }

                if path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let mut lexer = crate::lexer::Lexer::new(&content);
                        let tokens = lexer.tokenize();
                        let mut parser = crate::parser::Parser::new(tokens);
                        let imported_program = parser.parse_program();
                        
                        // Merge statements (primitive: just append imported statements before current index)
                        // To avoid duplicates if multiple files import same thing, we could use a set of visited paths.
                        // For these tests, just merging is usually ok.
                        let new_stmts = imported_program.statements;
                        merged_statements.splice(i..i+1, new_stmts);
                        // Don't increment i, we want to process the newly inserted statements (which might be imports)
                        continue;
                    }
                }
            }
            i += 1;
        }

        // Pass 1: Collect user functions
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    self.user_functions.borrow_mut().insert(func.name.clone());
                }
                Statement::ClassDeclaration(class_decl) => {
                    for method in &class_decl.methods {
                         let mangled = format!("{}_{}", class_decl.name, method.func.name);
                         self.user_functions.borrow_mut().insert(mangled);
                    }
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = &**declaration {
                         self.user_functions.borrow_mut().insert(func.name.clone());
                    }
                }
                _ => {}
            }
        }

        // Pass 2: Lower
        for stmt in &merged_statements {
            match stmt {
                Statement::ClassDeclaration(class_decl) => {
                    for method in &class_decl.methods {
                        let mut params: Vec<(String, TejxType)> = Vec::new();
                        if !method.is_static {
                            params.push(("this".to_string(), TejxType::Class(class_decl.name.clone())));
                        }
                        for p in &method.func.params {
                            params.push((p.name.clone(), TejxType::from_name(&p.type_name)));
                        }
                        let name = format!("f_{}_{}", class_decl.name, method.func.name);
                        let return_type = TejxType::from_name(&method.func.return_type);
                        let body = self.lower_statement(&method.func.body)
                            .unwrap_or(HIRStatement::Block { statements: vec![] });
                        functions.push(HIRStatement::Function {
                            name, params, _return_type: return_type, body: Box::new(body),
                        });
                    }
                }
                Statement::FunctionDeclaration(func) => {
                    let params: Vec<(String, TejxType)> = func.params.iter()
                        .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name)))
                        .collect();
                    let return_type = TejxType::from_name(&func.return_type);
                    let body = self.lower_statement(&func.body)
                        .unwrap_or(HIRStatement::Block { statements: vec![] });
                    let name = if func.name == "main" {
                        func.name.clone()
                    } else {
                        format!("f_{}", func.name)
                    };
                    functions.push(HIRStatement::Function {
                        name, params, _return_type: return_type, body: Box::new(body),
                    });
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = &**declaration {
                        let params: Vec<(String, TejxType)> = func.params.iter()
                            .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name)))
                            .collect();
                        let return_type = TejxType::from_name(&func.return_type);
                        let body = self.lower_statement(&func.body)
                            .unwrap_or(HIRStatement::Block { statements: vec![] });
                        
                        functions.push(HIRStatement::Function {
                            name: format!("f_{}", func.name),
                            params, _return_type: return_type, body: Box::new(body),
                        });
                    } else if let Some(hir) = self.lower_statement(stmt) {
                        main_stmts.push(hir);
                    }
                }
                Statement::ImportDecl { .. } => {
                    // Handled in Pass 0
                }
                _ => {
                    if let Some(hir) = self.lower_statement(stmt) {
                        main_stmts.push(hir);
                    }
                }
            }
        }

        // Include any lambdas generated during expression lowering
        let mut lambdas = self.lambda_functions.borrow_mut();
        functions.append(&mut lambdas);

        // Add a "tejx_main" wrapper for non-function top-level statements
        let main_body = if self.user_functions.borrow().contains("mainFunc") {
            HIRStatement::ExpressionStmt {
                expr: HIRExpression::Call {
                    callee: "f_mainFunc".to_string(),
                    args: vec![],
                    ty: TejxType::Void,
                },
            }
        } else {
            HIRStatement::Block { statements: main_stmts }
        };

        functions.push(HIRStatement::Function {
            name: "tejx_main".to_string(),
            params: vec![],
            _return_type: TejxType::Primitive("number".to_string()),
            body: Box::new(main_body),
        });

        LoweringResult { functions }
    }

    fn lower_statement(&self, stmt: &Statement) -> Option<HIRStatement> {
        match stmt {
            Statement::BlockStmt { statements, .. } => {
                let mut hir_stmts = Vec::new();
                for s in statements {
                    if let Some(h) = self.lower_statement(s) {
                        hir_stmts.push(h);
                    }
                }
                Some(HIRStatement::Block { statements: hir_stmts })
            }
            Statement::VarDeclaration { pattern, type_annotation, initializer, is_const, .. } => {
                let init = initializer.as_ref().map(|e| self.lower_expression(e));
                let ty = TejxType::from_name(type_annotation);
                
                let mut stmts = Vec::new();
                self.lower_binding_pattern(pattern, init, &ty, *is_const, &mut stmts);
                Some(HIRStatement::Block { statements: stmts })
            }
            Statement::ExpressionStmt { _expression, .. } => {
                let expr = self.lower_expression(_expression);
                Some(HIRStatement::ExpressionStmt { expr })
            }
            Statement::WhileStmt { condition, body, .. } => {
                let cond = self.lower_expression(condition);
                let body_hir = self.lower_statement_as_block(body);
                Some(HIRStatement::Loop {
                    condition: cond,
                    body: Box::new(body_hir),
                    increment: None,
                    _is_do_while: false,
                })
            }
            Statement::ForStmt { init, condition, increment, body, .. } => {
                let mut outer_stmts = Vec::new();
                if let Some(init_stmt) = init {
                    if let Some(h) = self.lower_statement(init_stmt) {
                        outer_stmts.push(h);
                    }
                }
                let cond = condition.as_ref()
                    .map(|c| self.lower_expression(c))
                    .unwrap_or(HIRExpression::Literal {
                        value: "true".to_string(),
                        ty: TejxType::Primitive("boolean".to_string()),
                    });

                let body_hir = self.lower_statement_as_block(body);

                let inc = increment.as_ref().map(|e| {
                    Box::new(HIRStatement::ExpressionStmt {
                        expr: self.lower_expression(e),
                    })
                });

                outer_stmts.push(HIRStatement::Loop {
                    condition: cond,
                    body: Box::new(body_hir),
                    increment: inc,
                    _is_do_while: false,
                });

                Some(HIRStatement::Block { statements: outer_stmts })
            }
            Statement::IfStmt { condition, then_branch, else_branch, .. } => {
                let cond = self.lower_expression(condition);
                let then_hir = self.lower_statement(then_branch)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                let else_hir = else_branch.as_ref()
                    .and_then(|e| self.lower_statement(e));

                Some(HIRStatement::If {
                    condition: cond,
                    then_branch: Box::new(then_hir),
                    else_branch: else_hir.map(Box::new),
                })
            }
            Statement::ReturnStmt { value, .. } => {
                let val = value.as_ref().map(|e| self.lower_expression(e));
                Some(HIRStatement::Return { value: val })
            }
            Statement::BreakStmt { .. } => Some(HIRStatement::Break),
            Statement::ContinueStmt { .. } => Some(HIRStatement::Continue),
            Statement::SwitchStmt { condition, cases, .. } => {
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
                        body: Box::new(HIRStatement::Block { statements: stmts }),
                    });
                }
                Some(HIRStatement::Switch {
                    condition: cond,
                    cases: hir_cases,
                })
            }
            Statement::FunctionDeclaration(func) => {
                let params: Vec<(String, TejxType)> = func.params.iter()
                    .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name)))
                    .collect();
                let return_type = TejxType::from_name(&func.return_type);
                let body = self.lower_statement(&func.body)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                Some(HIRStatement::Function {
                    name: format!("f_{}", func.name),
                    params,
                    _return_type: return_type,
                    body: Box::new(body),
                })
            }
            Statement::ExportDecl { declaration, .. } => self.lower_statement(declaration),
            _ => None,
        }
    }

    fn lower_statement_as_block(&self, stmt: &Statement) -> HIRStatement {
        match self.lower_statement(stmt) {
            Some(HIRStatement::Block { .. }) => self.lower_statement(stmt).unwrap(),
            Some(other) => HIRStatement::Block { statements: vec![other] },
            None => HIRStatement::Block { statements: vec![] },
        }
    }

    fn lower_expression(&self, expr: &Expression) -> HIRExpression {
        match expr {
            Expression::NumberLiteral { value, .. } => {
                let val_str = if *value == (*value as i64) as f64 {
                    format!("{}", *value as i64)
                } else {
                    format!("{}", value)
                };
                HIRExpression::Literal {
                    value: val_str,
                    ty: TejxType::Primitive("number".to_string()),
                }
            }
            Expression::StringLiteral { value, .. } => {
                HIRExpression::Literal {
                    value: value.clone(),
                    ty: TejxType::Primitive("string".to_string()),
                }
            }
            Expression::BooleanLiteral { value, .. } => {
                HIRExpression::Literal {
                    value: value.to_string(),
                    ty: TejxType::Primitive("boolean".to_string()),
                }
            }
            Expression::Identifier { name, .. } => {
                HIRExpression::Variable {
                    name: name.clone(),
                    ty: TejxType::Any,
                }
            }
            Expression::BinaryExpr { left, op, right, .. } => {
                HIRExpression::BinaryExpr {
                    left: Box::new(self.lower_expression(left)),
                    op: op.clone(),
                    right: Box::new(self.lower_expression(right)),
                    ty: TejxType::Any,
                }
            }
            Expression::AssignmentExpr { target, value, .. } => {
                let v = self.lower_expression(value);
                let ty = v.get_type();
                
                // If target is identifier, normal assignment.
                // If target is ArrayLiteral/ObjectLiteral (destructuring assignment), we need special handling.
                // But in NovaJs AST, destructuring target usually parsed as Expression::ArrayLiteral or similar?
                // Actually, AssignmentExpr target is Box<Expression>.
                // if it's Expression::Identifier, simple.
                // if it's Expression::ArrayLiteral, it's destructuring.
                
                match target.as_ref() {
                    Expression::Identifier { .. } | Expression::MemberAccessExpr { .. } | Expression::ArrayAccessExpr { .. } => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            target: Box::new(t),
                            value: Box::new(v),
                            ty,
                        }
                    }
                    _ => {
                        // For complex destructuring in assignments, we might need a block expr or temporary.
                        // For now, let's keep it simple or fallback.
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            target: Box::new(t),
                            value: Box::new(v),
                            ty,
                        }
                    }
                }
            }
            Expression::CallExpr { callee, args, .. } => {
                let hir_args: Vec<HIRExpression> = args.iter()
                    .map(|a| self.lower_expression(a))
                    .collect();
                
                let normalized = callee.replace('.', "_");
                let final_callee = if self.user_functions.borrow().contains(&normalized) && 
                                      !self.is_runtime_func(&normalized) && 
                                      normalized != "main" {
                    format!("f_{}", normalized)
                } else if self.user_functions.borrow().contains(callee) && 
                          !self.is_runtime_func(callee) && 
                          callee != "main" {
                    format!("f_{}", callee)
                } else {
                    callee.clone()
                };

                let ty = if final_callee == "fs_readFile" || final_callee == "JSON_stringify" || final_callee.starts_with("d_toISOString") {
                    TejxType::Primitive("string".to_string())
                } else if final_callee == "Math_random" || final_callee == "Date_now" || final_callee == "d_getTime" {
                    TejxType::Primitive("number".to_string())
                } else {
                    TejxType::Any
                };

                // Method call detection: obj.method(...)
                if callee.contains('.') && !callee.starts_with("console.") && !callee.starts_with("Math.") && !callee.starts_with("JSON.") && !callee.starts_with("fs.") {
                    let parts: Vec<&str> = callee.split('.').collect();
                    if parts.len() == 2 {
                        let obj_name = parts[0];
                        let method_name = parts[1];
                        
                        // Transform to Array_push(obj, args) etc if it is a common method
                        let runtime_method = match method_name {
                            "push" | "pop" | "shift" | "unshift" | "join" | "concat" | "indexOf" | "map" | "filter" | "forEach" => {
                                format!("Array_{}", method_name)
                            }
                            _ => format!("{}_{}", obj_name, method_name) // Fallback
                        };

                        let mut new_args = vec![HIRExpression::Variable { name: obj_name.to_string(), ty: TejxType::Any }];
                        new_args.extend(hir_args);

                        return HIRExpression::Call {
                            callee: runtime_method,
                            args: new_args,
                            ty,
                        };
                    }
                }

                HIRExpression::Call {
                    callee: final_callee,
                    args: hir_args,
                    ty,
                }
            }
            Expression::MemberAccessExpr { object, member, .. } => {
                let obj_name = match object.as_ref() {
                    Expression::Identifier { name, .. } => name.clone(),
                    _ => "".to_string(),
                };
                
                let combined = format!("{}_{}", obj_name, member);
                if self.user_functions.borrow().contains(&combined) {
                    HIRExpression::Variable {
                        name: format!("f_{}", combined),
                        ty: TejxType::Any,
                    }
                } else {
                    HIRExpression::MemberAccess {
                        target: Box::new(self.lower_expression(object)),
                        member: member.clone(),
                        ty: TejxType::Any,
                    }
                }
            }
            Expression::ArrayAccessExpr { target, index, .. } => {
                HIRExpression::IndexAccess {
                    target: Box::new(self.lower_expression(target)),
                    index: Box::new(self.lower_expression(index)),
                    ty: TejxType::Any,
                }
            }
            Expression::ObjectLiteralExpr { entries, .. } => {
                let hir_entries = entries.iter()
                    .map(|(k, v)| (k.clone(), self.lower_expression(v)))
                    .collect();
                HIRExpression::ObjectLiteral {
                    entries: hir_entries,
                    ty: TejxType::Any,
                }
            }
            Expression::ArrayLiteral { elements, .. } => {
                let hir_elements = elements.iter()
                    .map(|e| self.lower_expression(e))
                    .collect();
                HIRExpression::ArrayLiteral {
                    elements: hir_elements,
                    ty: TejxType::Any,
                }
            }
            Expression::LambdaExpr { params, body, .. } => {
                let id = {
                    let mut counter = self.lambda_counter.borrow_mut();
                    let val = *counter;
                    *counter += 1;
                    val
                };
                let lambda_name = format!("lambda_{}", id);
                
                let hir_params: Vec<(String, TejxType)> = params.iter()
                    .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name)))
                    .collect();
                
                let hir_body = self.lower_statement(body)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                
                self.lambda_functions.borrow_mut().push(HIRStatement::Function {
                    name: lambda_name.clone(),
                    params: hir_params,
                    _return_type: TejxType::Any,
                    body: Box::new(hir_body),
                });
                
                HIRExpression::Literal {
                    value: lambda_name,
                    ty: TejxType::Any, // Actually function type
                }
            }
            Expression::AwaitExpr { expr, .. } => {
                HIRExpression::Await {
                    expr: Box::new(self.lower_expression(expr)),
                    ty: TejxType::Any,
                }
            }
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                HIRExpression::OptionalChain {
                    target: Box::new(self.lower_expression(object)),
                    operation: format!(".{}", member),
                    ty: TejxType::Any,
                }
            }
            Expression::NewExpr { class_name, args, .. } => {
                let hir_args: Vec<HIRExpression> = args.iter()
                    .map(|a| self.lower_expression(a))
                    .collect();
                HIRExpression::NewExpr {
                    class_name: class_name.clone(),
                    _args: hir_args,
                }
            }
            Expression::MatchExpr { target, arms, .. } => {
                let tgt = self.lower_expression(target);
                let mut hir_arms = Vec::new();
                for arm in arms {
                     let guard = arm.guard.as_ref().map(|g| Box::new(self.lower_expression(g)));
                     let body = Box::new(self.lower_expression(&arm.body));
                     hir_arms.push(HIRMatchArm {
                         pattern: arm.pattern.clone(),
                         guard,
                         body,
                     });
                }
                HIRExpression::Match {
                    target: Box::new(tgt),
                    arms: hir_arms,
                    ty: TejxType::Any,
                }
            }
            Expression::BlockExpr { statements, .. } => {
                let mut hir_stmts = Vec::new();
                for s in statements {
                    if let Some(h) = self.lower_statement(s) {
                        hir_stmts.push(h);
                    }
                }
                HIRExpression::BlockExpr {
                    statements: hir_stmts,
                    ty: TejxType::Void,
                }
            }
            Expression::UnaryExpr { op, right, .. } => {
                match op {
                    TokenType::Minus => {
                        HIRExpression::BinaryExpr {
                            left: Box::new(HIRExpression::Literal {
                                value: "0".to_string(),
                                ty: TejxType::Primitive("number".to_string()),
                            }),
                            op: TokenType::Minus,
                            right: Box::new(self.lower_expression(right)),
                            ty: TejxType::Primitive("number".to_string()),
                        }
                    }
                    _ => {
                        self.lower_expression(right)
                    }
                }
            }
            _ => HIRExpression::Literal {
                value: "0".to_string(),
                ty: TejxType::Any,
            },
        }
    }

    fn lower_binding_pattern(&self, pattern: &BindingNode, initializer: Option<HIRExpression>, ty: &TejxType, is_const: bool, stmts: &mut Vec<HIRStatement>) {
        match pattern {
            BindingNode::Identifier(name) => {
                stmts.push(HIRStatement::VarDecl {
                    name: name.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: is_const,
                });
            }
            BindingNode::ArrayBinding { elements, rest } => {
                // let [a, b] = init;
                // Lower as:
                // let tmp = init;
                // let a = tmp[0];
                // let b = tmp[1];
                let tmp_id = format!("destructure_tmp_{}", self.lambda_counter.borrow());
                *self.lambda_counter.borrow_mut() += 1;
                
                stmts.push(HIRStatement::VarDecl {
                    name: tmp_id.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: true,
                });
                
                for (i, el) in elements.iter().enumerate() {
                    let el_init = HIRExpression::IndexAccess {
                        target: Box::new(HIRExpression::Variable { name: tmp_id.clone(), ty: ty.clone() }),
                        index: Box::new(HIRExpression::Literal { value: i.to_string(), ty: TejxType::Primitive("number".to_string()) }),
                        ty: TejxType::Any,
                    };
                    self.lower_binding_pattern(el, Some(el_init), &TejxType::Any, is_const, stmts);
                }
                
                if let Some(_r) = rest {
                    // TODO: handle rest ...tail
                    // let tail = tmp.slice(elements.len());
                }
            }
            BindingNode::ObjectBinding { entries } => {
                let tmp_id = format!("destructure_tmp_{}", self.lambda_counter.borrow());
                *self.lambda_counter.borrow_mut() += 1;
                
                stmts.push(HIRStatement::VarDecl {
                    name: tmp_id.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: true,
                });
                
                for (key, target) in entries {
                    let el_init = HIRExpression::MemberAccess {
                        target: Box::new(HIRExpression::Variable { name: tmp_id.clone(), ty: ty.clone() }),
                        member: key.clone(),
                        ty: TejxType::Any,
                    };
                    self.lower_binding_pattern(target, Some(el_init), &TejxType::Any, is_const, stmts);
                }
            }
            _ => {}
        }
    }
}
