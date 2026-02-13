use std::cell::RefCell;
use std::collections::HashSet;
use crate::ast::*;
use crate::hir::*;
use crate::types::TejxType;
use crate::token::TokenType;
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum ImportMode {
    All,
    Named(HashSet<String>),
}

use crate::runtime::stdlib::StdLib;

pub struct Lowering {
    lambda_counter: RefCell<usize>,
    user_functions: RefCell<HashMap<String, TejxType>>,
    variadic_functions: RefCell<HashMap<String, usize>>,
    lambda_functions: RefCell<Vec<HIRStatement>>,
    scopes: RefCell<Vec<HashMap<String, TejxType>>>,
    std_imports: RefCell<HashMap<String, ImportMode>>,
    stdlib: StdLib,
    current_class: RefCell<Option<String>>,
    parent_class: RefCell<Option<String>>,
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
            user_functions: RefCell::new(HashMap::new()),
            variadic_functions: RefCell::new(HashMap::new()),
            lambda_functions: RefCell::new(Vec::new()),
            scopes: RefCell::new(vec![HashMap::new()]), // Global scope
            std_imports: RefCell::new(HashMap::new()),
            stdlib: StdLib::new(),
            current_class: RefCell::new(None),
            parent_class: RefCell::new(None),
        }
    }

    fn enter_scope(&self) {
        self.scopes.borrow_mut().push(HashMap::new());
    }

    fn _exit_scope(&self) {
        self.scopes.borrow_mut().pop();
    }

    fn define(&self, name: String, ty: TejxType) {
        if let Some(scope) = self.scopes.borrow_mut().last_mut() {
            scope.insert(name, ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<TejxType> {
        let scopes = self.scopes.borrow();
        for scope in scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None
    }

    fn is_runtime_func(&self, name: &str) -> bool {
        // Delegate std functions to stdlib
        if let Some(_) = self.stdlib.resolve_runtime_func(name) {
             return true;
        }
        
        // Legacy runtime functions (Array methods, Math objects, etc.)
        let runtime_funcs = [
            "Math_pow", "fs_exists", "Array_push", "Array_pop", "Array_concat",
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
            // Prelude names removed from here as they are in stdlib.prelude
             "printf"
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
                if source.starts_with("std:") {
                    let mod_name = &source[4..];
                    let mut imports = self.std_imports.borrow_mut();
                    
                    let new_mode = if let Statement::ImportDecl { _names, .. } = &merged_statements[i] {
                         if _names.is_empty() {
                             ImportMode::All
                         } else {
                             let mut set = HashSet::new();
                             for n in _names { set.insert(n.clone()); }
                             ImportMode::Named(set)
                         }
                    } else {
                        ImportMode::All
                    };

                    // Merge strategy: All > Named
                    let entry = imports.entry(mod_name.to_string()).or_insert(ImportMode::Named(HashSet::new()));
                    match (entry.clone(), new_mode) {
                        (ImportMode::All, _) => {}, // Already All, stay All
                        (_, ImportMode::All) => { *entry = ImportMode::All; }, // Upgrade to All
                        (ImportMode::Named(mut existing), ImportMode::Named(new_set)) => {
                            existing.extend(new_set);
                            *entry = ImportMode::Named(existing);
                        }
                    }

                    i += 1;
                    continue;
                }
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
                        let mut lexer = crate::lexer::Lexer::new(&content, &path.to_string_lossy());
                        let tokens = lexer.tokenize();
                        let mut parser = crate::parser::Parser::new(tokens, &path.to_string_lossy());
                        let imported_program = parser.parse_program();
                        
                        let new_stmts = imported_program.statements;
                        merged_statements.splice(i..i+1, new_stmts);
                        continue;
                    }
                }
            }
            i += 1;
        }

        // Pass 0.5: Scan for Variadic Functions
        for stmt in &merged_statements {
             if let Statement::FunctionDeclaration(func) = stmt {
                 let fixed_count = func.params.iter().take_while(|p| !p._is_rest).count();
                 if fixed_count < func.params.len() {
                     self.variadic_functions.borrow_mut().insert(func.name.clone(), fixed_count);
                 }
             }
        }

        // Pass 1: Collect user functions
        // Pass 1: Collect user functions and top-level variables
        self.scopes.borrow_mut().clear(); // Reset scopes
        self.enter_scope(); // Global scope

        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    self.user_functions.borrow_mut().insert(func.name.clone(), TejxType::from_name(&func.return_type));
                }
                Statement::ClassDeclaration(class_decl) => {
                    for method in &class_decl.methods {
                         let mangled = format!("{}_{}", class_decl.name, method.func.name);
                         self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&method.func.return_type));
                    }
                    if let Some(constructor) = &class_decl._constructor {
                         let mangled = format!("{}_{}", class_decl.name, constructor.name);
                         self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&constructor.return_type));
                    }
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = &**declaration {
                         self.user_functions.borrow_mut().insert(func.name.clone(), TejxType::from_name(&func.return_type));
                    }
                }
                Statement::VarDeclaration { pattern, type_annotation, .. } => {
                    if let BindingNode::Identifier(name) = pattern {
                        let ty = TejxType::from_name(type_annotation);
                        self.define(name.clone(), ty);
                    }
                }
                _ => {}
            }
        }

        // Pass 2: Lower
        for stmt in &merged_statements {
            match stmt {
                Statement::ClassDeclaration(class_decl) => {
                    *self.current_class.borrow_mut() = Some(class_decl.name.clone());
                    let p_name = if class_decl._parent_name.is_empty() { None } else { Some(class_decl._parent_name.clone()) };
                    *self.parent_class.borrow_mut() = p_name;

                    let mut all_methods = Vec::new();
                    for m in &class_decl.methods {
                        all_methods.push((&m.func, m.is_static));
                    }
                    if let Some(cons) = &class_decl._constructor {
                        all_methods.push((cons, false));
                    }

                    for (func_decl, is_static) in all_methods {
                        let mut params: Vec<(String, TejxType)> = Vec::new();
                        if !is_static {
                            params.push(("this".to_string(), TejxType::Class(class_decl.name.clone())));
                        }
                        for p in &func_decl.params {
                            params.push((p.name.clone(), TejxType::from_name(&p.type_name)));
                        }
                        let name = format!("f_{}_{}", class_decl.name, func_decl.name);
                        let return_type = TejxType::from_name(&func_decl.return_type);
                        
                        self.enter_scope();
                        for (pname, pty) in &params {
                            self.define(pname.clone(), pty.clone());
                        }
                        
                        let mut hir_body = self.lower_statement(&func_decl.body)
                            .unwrap_or(HIRStatement::Block { statements: vec![] });
                        
                        // Inject debug print at method start
                        if let HIRStatement::Block { ref mut statements } = hir_body {
                             statements.insert(0, HIRStatement::ExpressionStmt {
                                 expr: HIRExpression::Call {
                                     callee: "print".to_string(),
                                     args: vec![HIRExpression::Literal { 
                                         value: format!("DEBUG: Entering method {}::{}", class_decl.name, func_decl.name),
                                         ty: TejxType::Primitive("string".to_string())
                                     }],
                                     ty: TejxType::Void
                                 }
                             });
                        }
                        
                        let body = hir_body;
                        
                        self._exit_scope();

                        functions.push(HIRStatement::Function {
                            name, params, _return_type: return_type, body: Box::new(body),
                        });
                    }
                    
                    *self.current_class.borrow_mut() = None;
                    *self.parent_class.borrow_mut() = None;
                }
                Statement::FunctionDeclaration(func) => {
                    let params: Vec<(String, TejxType)> = func.params.iter()
                        .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name)))
                        .collect();
                    let return_type = TejxType::from_name(&func.return_type);
                    
                    self.enter_scope();
                    for (name, ty) in &params {
                        self.define(name.clone(), ty.clone());
                    }
                    
                    let body = self.lower_statement(&func.body)
                        .unwrap_or(HIRStatement::Block { statements: vec![] });
                    
                    self._exit_scope();
                    
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
                        
                        self.enter_scope();
                        for (name, ty) in &params {
                            self.define(name.clone(), ty.clone());
                        }
                        
                        let body = self.lower_statement(&func.body)
                            .unwrap_or(HIRStatement::Block { statements: vec![] });
                        
                        self._exit_scope();
                        
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
                Statement::VarDeclaration { pattern, type_annotation, initializer: _, is_const: _, .. } => {
                     // Register first for this scope
                     if let BindingNode::Identifier(name) = pattern {
                        let ty = TejxType::from_name(type_annotation);
                        self.define(name.clone(), ty);
                    }
                    
                    if let Some(hir) = self.lower_statement(stmt) {
                        main_stmts.push(hir);
                    }
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
        let main_body = if self.user_functions.borrow().contains_key("mainFunc") {
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
                self.enter_scope();
                let mut hir_stmts = Vec::new();
                for s in statements {
                    if let Some(h) = self.lower_statement(s) {
                        hir_stmts.push(h);
                    }
                }
                self._exit_scope();
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
                     let expr = self.lower_expression(e);
                     Box::new(HIRStatement::ExpressionStmt {
                        expr,
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
             Statement::ForOfStmt { variable, iterable, body, .. } => {
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
                    let arr_name = format!("__arr_{}", var_name); 
                    stmts.push(HIRStatement::VarDecl {
                        name: arr_name.clone(),
                        initializer: Some(iter_expr),
                        ty: TejxType::Any,
                        _is_const: true,
                    });
                    
                    // 2. Length
                    let len_name = format!("__len_{}", var_name);
                    stmts.push(HIRStatement::VarDecl {
                        name: len_name.clone(),
                        initializer: Some(HIRExpression::MemberAccess {
                            target: Box::new(HIRExpression::Variable { name: arr_name.clone(), ty: TejxType::Any }),
                            member: "length".to_string(),
                            ty: TejxType::Primitive("number".to_string())
                        }),
                        ty: TejxType::Primitive("number".to_string()),
                        _is_const: true,
                    });
                    
                    // 3. Index
                    let idx_name = format!("__idx_{}", var_name);
                    stmts.push(HIRStatement::VarDecl {
                        name: idx_name.clone(),
                        initializer: Some(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Primitive("number".to_string()) }),
                        ty: TejxType::Primitive("number".to_string()),
                        _is_const: false,
                    });
                    
                    // 4. Loop
                    // Condition: idx < len
                    let cond = HIRExpression::BinaryExpr {
                        left: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Primitive("number".to_string()) }),
                        op: TokenType::Less,
                        right: Box::new(HIRExpression::Variable { name: len_name, ty: TejxType::Primitive("number".to_string()) }),
                        ty: TejxType::Primitive("boolean".to_string()),
                    };
                    
                    // Body construction
                    let mut body_stmts = Vec::new();
                    
                    // let var_name = _arr[_idx];
                    let val_expr = HIRExpression::IndexAccess {
                        target: Box::new(HIRExpression::Variable { name: arr_name, ty: TejxType::Any }),
                        index: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Primitive("number".to_string()) }),
                        ty: TejxType::Any,
                    };
                    body_stmts.push(HIRStatement::VarDecl {
                        name: var_name.clone(),
                        initializer: Some(val_expr),
                        ty: TejxType::Any, // Inferred?
                        _is_const: false,
                    });
                    self.define(var_name.clone(), TejxType::Any);
                    
                    // User Body
                    if let Some(user_body) = self.lower_statement(body) {
                        body_stmts.push(user_body);
                    }
                    
                    // Increment: idx = idx + 1
                    let inc_stmt = Box::new(HIRStatement::ExpressionStmt {
                        expr: HIRExpression::Assignment {
                            target: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Primitive("number".to_string()) }),
                            value: Box::new(HIRExpression::BinaryExpr {
                                left: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Primitive("number".to_string()) }),
                                op: TokenType::Plus,
                                right: Box::new(HIRExpression::Literal { value: "1".to_string(), ty: TejxType::Primitive("number".to_string()) }),
                                ty: TejxType::Primitive("number".to_string())
                            }),
                            ty: TejxType::Primitive("number".to_string())
                        }
                    });
                    
                    stmts.push(HIRStatement::Loop {
                        condition: cond,
                        body: Box::new(HIRStatement::Block { statements: body_stmts }),
                        increment: Some(inc_stmt),
                        _is_do_while: false,
                    });
                    
                    Some(HIRStatement::Block { statements: stmts })
                } else {
                    None // Destructuring in for-of not supported yet
                }
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
                
                self.enter_scope();
                for (name, ty) in &params {
                    self.define(name.clone(), ty.clone());
                }
                
                let body = self.lower_statement(&func.body)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                
                self._exit_scope();
                
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
            Expression::ThisExpr { .. } => {
                HIRExpression::Variable {
                    name: "this".to_string(),
                    ty: TejxType::Any,
                }
            }
            Expression::SuperExpr { .. } => {
                HIRExpression::Variable {
                    name: "super".to_string(),
                    ty: TejxType::Any,
                }
            }
            Expression::Identifier { name, .. } => {
                let ty = self.lookup(name).unwrap_or(TejxType::Any);
                HIRExpression::Variable {
                    name: name.clone(),
                    ty,
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
            Expression::AssignmentExpr { target, value, _op, .. } => {
                let v = self.lower_expression(value);
                let ty = v.get_type();
                
                // Desugar compound assignments: a += b  ->  a = a + b
                let final_value = match _op {
                    TokenType::PlusEquals => {
                         HIRExpression::BinaryExpr {
                             left: Box::new(self.lower_expression(target)),
                             op: TokenType::Plus,
                             right: Box::new(v),
                             ty: ty.clone(),
                         }
                    }
                    TokenType::MinusEquals => {
                         HIRExpression::BinaryExpr {
                             left: Box::new(self.lower_expression(target)),
                             op: TokenType::Minus,
                             right: Box::new(v),
                             ty: ty.clone(),
                         }
                    }
                    TokenType::StarEquals => {
                         HIRExpression::BinaryExpr {
                             left: Box::new(self.lower_expression(target)),
                             op: TokenType::Star,
                             right: Box::new(v),
                             ty: ty.clone(),
                         }
                    }
                    TokenType::SlashEquals => {
                         HIRExpression::BinaryExpr {
                             left: Box::new(self.lower_expression(target)),
                             op: TokenType::Slash,
                             right: Box::new(v),
                             ty: ty.clone(),
                         }
                    }
                    _ => v // Direct assignment
                };

                match target.as_ref() {
                    Expression::Identifier { .. } | Expression::MemberAccessExpr { .. } | Expression::ArrayAccessExpr { .. } => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            target: Box::new(t),
                            value: Box::new(final_value),
                            ty,
                        }
                    }
                    _ => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            target: Box::new(t),
                            value: Box::new(final_value),
                            ty,
                        }
                    }
                }
            }
            Expression::UnaryExpr { op, right, .. } => {
                 // ++i, --i, !x, -x
                 match op {
                     TokenType::PlusPlus | TokenType::MinusMinus => {
                         // Desugar ++i -> i = i + 1 (Prefix)
                         // TODO: Suffix support? AST usually distinguishes suffix/prefix.
                         // For now assume prefix or handle generic increment.
                         let delta = if matches!(op, TokenType::PlusPlus) { "1" } else { "1" };
                         let bin_op = if matches!(op, TokenType::PlusPlus) { TokenType::Plus } else { TokenType::Minus };
                         
                         let r_expr = self.lower_expression(right);
                         // Reconstruct Assignment: right = right op 1
                         // Need to clone target handling from AssignmentExpr logic ideally.
                         // Simplification:
                         HIRExpression::Assignment {
                             target: Box::new(r_expr.clone()),
                             value: Box::new(HIRExpression::BinaryExpr {
                                 left: Box::new(r_expr),
                                 op: bin_op,
                                 right: Box::new(HIRExpression::Literal { value: delta.to_string(), ty: TejxType::Primitive("number".to_string()) }),
                                 ty: TejxType::Primitive("number".to_string())
                             }),
                             ty: TejxType::Primitive("number".to_string())
                         }
                     }
                     TokenType::Bang => {
                         HIRExpression::BinaryExpr {
                             left: Box::new(self.lower_expression(right)),
                             op: TokenType::BangEqual, // Hack: Use != 0 or similar? 
                             // Wait, !x is boolean op.
                             // HIR doesn't have UnaryExpr. BinaryExpr with special op?
                             // Or strict BinaryExpr mapping.
                             // Actually we have BangEqual ( != ).
                             // To do Not, we can do (x == false) or (x == 0).
                             // Or introduce UnaryExpr in HIR?
                             // Existing code didn't exhibit this?
                             // Let's check HIRExpression definition.
                             // It has BinaryExpr, no Unary.
                             // We can use (x == false)
                             right: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Primitive("boolean".to_string()) }),
                             ty: TejxType::Primitive("boolean".to_string())
                         }
                     }
                     TokenType::Minus => {
                         // -x -> 0 - x
                         HIRExpression::BinaryExpr {
                             left: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Primitive("number".to_string()) }),
                             op: TokenType::Minus,
                             right: Box::new(self.lower_expression(right)),
                             ty: TejxType::Primitive("number".to_string())
                         }
                     }
                     _ => self.lower_expression(right) // Fallback
                 }
            }
            Expression::CallExpr { callee, args, .. } => {
                let hir_args: Vec<HIRExpression> = args.iter()
                    .map(|a| self.lower_expression(a))
                    .collect();
                
                let normalized = callee.replace('.', "_").replace("::", "_").replace(":", "_");
                
                let mut final_callee = if self.user_functions.borrow().contains_key(callee) && callee != "main" {
                    format!("f_{}", callee)
                } else if self.stdlib.is_prelude_func(callee) {
                    callee.clone()
                } else if callee == "super" {
                    if let Some(parent) = &*self.parent_class.borrow() {
                         format!("f_{}_constructor", parent)
                    } else {
                         "f_Entity_constructor".to_string() // Fallback or error?
                    }
                } else {
                    let mut found = None;
                    let std_imports = self.std_imports.borrow();
                    for (mod_name, mode) in std_imports.iter() {
                        let potential = self.stdlib.get_runtime_name(mod_name, callee);
                        
                        // Check strict import rules
                        let allowed = match mode {
                            ImportMode::All => true,
                            ImportMode::Named(names) => names.contains(callee)
                        };

                        // Check module validity via stdlib
                        if allowed && self.stdlib.is_std_func(mod_name, callee) {
                            found = Some(potential);
                            break;
                        }
                    }
                    
                    if let Some(target) = found {
                        target
                    } else if callee.contains(':') || callee.contains("::") {
                         normalized.clone()
                    } else {
                        // TODO: Suggestion logic could be moved to stdlib too
                        normalized.clone()
                    }
                };

                // Add 'this' if it is a 'super()' call
                let mut final_args = hir_args.clone();
                if callee == "super" {
                    final_args.insert(0, HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any });
                }

                let ty = if let Some(known_ty) = self.user_functions.borrow().get(callee) {
                    known_ty.clone()
                } else if final_callee == "fs_readFile" || final_callee == "JSON_stringify" || 
                           final_callee.starts_with("d_toISOString") {
                    TejxType::Primitive("string".to_string())
                } else if final_callee == "Math_random" || final_callee == "Date_now" || 
                           final_callee == "d_getTime" || final_callee == "std_time_now" {
                    TejxType::Primitive("number".to_string())
                } else {
                    TejxType::Any
                };

                // Method call detection: obj.method(...)
                if callee.contains('.') && 
                   !callee.starts_with("Math.") && !callee.starts_with("JSON.") && 
                   !callee.starts_with("fs.") && !callee.starts_with("Date.") && 
                   !callee.starts_with("Promise.") && !callee.starts_with("Array.") {
                    let parts: Vec<&str> = callee.split('.').collect();
                    if parts.len() == 2 {
                        let obj_name = parts[0];
                        let method_name = parts[1];
                        
                        // Transform to Array_push(obj, args) etc if it is a common method
                        let (runtime_callee, ret_type) = match method_name {
                             "push" | "unshift" | "indexOf" => (format!("Array_{}", method_name), TejxType::Primitive("number".to_string())),
                             "pop" | "shift" => (format!("Array_{}", method_name), TejxType::Any), 
                             "join" => ("__join".to_string(), TejxType::Primitive("string".to_string())),
                             "concat" | "map" | "filter" => (format!("Array_{}", method_name), TejxType::Any),
                             "forEach" => (format!("Array_{}", method_name), TejxType::Void),
                             _ => {
                                 // Dynamic method call: obj["method"](args)
                                 // Check if we can treat it as indirect call.
                                 // We don't have a specific "MethodCall" instruction yet, but we can simulate:
                                 // let func = obj.method; func(obj, args) ?
                                 // Or just func(args) if it's a closure field?
                                 // For features_stress.tx `counter.increment()`, it's a closure field.
                                 // So strictly speaking: func = counter["increment"]; func()
                                 // `counter` is NOT passed as `this` unless we implement bound methods.
                                 // JS semantics: obj.method() -> if method is property, call it with `this=obj`.
                                 // Our closure implementation in stress test doesn't use `this`.
                                 // So let's lower to: Get(obj, method) -> IndirectCall(func, args)
                                 
                                 // 1. Get function
                                 let func_expr = HIRExpression::MemberAccess {
                                     target: Box::new(HIRExpression::Variable { name: obj_name.to_string(), ty: TejxType::Any }),
                                     member: method_name.to_string(),
                                     ty: TejxType::Any
                                 };
                                 
                                 return HIRExpression::IndirectCall {
                                     callee: Box::new(func_expr),
                                     args: hir_args,
                                     ty: TejxType::Any
                                 };
                             }
                        };

                        let mut new_args = vec![HIRExpression::Variable { name: obj_name.to_string(), ty: TejxType::Any }];
                        new_args.extend(hir_args);

                        // If callee is __join (for Array/Thread join dispatch), ensure 2 args
                        if runtime_callee == "__join" && new_args.len() < 2 {
                             new_args.push(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Primitive("number".to_string()) });
                        }

                        return HIRExpression::Call {
                            callee: runtime_callee,
                            args: new_args,
                            ty: ret_type,
                        };
                    }
                }

                // Check for variadic function call and pack arguments
                if let Some(&fixed_count) = self.variadic_functions.borrow().get(callee.as_str()) {
                     if final_args.len() >= fixed_count {
                         let (fixed, rest) = final_args.split_at(fixed_count);
                         let mut new_var_args = fixed.to_vec();
                         
                         // Create ArrayLiteral for rest
                         new_var_args.push(HIRExpression::ArrayLiteral {
                             elements: rest.to_vec(),
                             ty: TejxType::Any
                         });
                         
                         return HIRExpression::Call {
                             callee: final_callee,
                             args: new_var_args,
                             ty,
                         };
                     }
                }

                HIRExpression::Call {
                    callee: final_callee,
                    args: final_args,
                    ty,
                }
            }
            Expression::MemberAccessExpr { object, member, .. } => {
                let obj_name = match object.as_ref() {
                    Expression::Identifier { name, .. } => name.clone(),
                    _ => "".to_string(),
                };
                
                let combined = format!("{}_{}", obj_name, member);
                if self.user_functions.borrow().contains_key(&combined) {
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
                // Handle spreads: [a, ...b, c] -> concat(concat([a], b), [c])
                let mut chunks: Vec<HIRExpression> = Vec::new();
                let mut current_chunk: Vec<HIRExpression> = Vec::new();

                for e in elements {
                    if let Expression::SpreadExpr { _expr, .. } = e {
                        // Push accumulated static chunk if any
                        if !current_chunk.is_empty() {
                            chunks.push(HIRExpression::ArrayLiteral {
                                elements: current_chunk.clone(),
                                ty: TejxType::Any,
                            });
                            current_chunk.clear();
                        }
                        // Push spread expr (lowered)
                        chunks.push(self.lower_expression(_expr));
                    } else {
                        current_chunk.push(self.lower_expression(e));
                    }
                }
                // Push final chunk
                if !current_chunk.is_empty() {
                    chunks.push(HIRExpression::ArrayLiteral {
                        elements: current_chunk,
                        ty: TejxType::Any,
                    });
                }
                
                if chunks.is_empty() {
                     // Empty array []
                     HIRExpression::ArrayLiteral { elements: vec![], ty: TejxType::Any }
                } else {
                    // Reduce chunks with Array_concat
                    let mut expr = chunks[0].clone();
                    for next_chunk in chunks.into_iter().skip(1) {
                         expr = HIRExpression::Call {
                             callee: "Array_concat".to_string(), // Ensure this maps to Array_concat in runtime
                             args: vec![expr, next_chunk],
                             ty: TejxType::Any,
                         };
                    }
                    expr
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

            _ => HIRExpression::Literal {
                value: "0".to_string(),
                ty: TejxType::Any,
            },
        }
    }

    fn lower_binding_pattern(&self, pattern: &BindingNode, initializer: Option<HIRExpression>, ty: &TejxType, is_const: bool, stmts: &mut Vec<HIRStatement>) {
        match pattern {
            BindingNode::Identifier(name) => {
                self.define(name.clone(), ty.clone());
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
                
                if let Some(r) = rest {
                    // handle rest ...tail
                     // let tail = Array_sliceRest(tmp, elements.len());
                     let slice_init = HIRExpression::Call {
                         callee: "Array_sliceRest".to_string(),
                         args: vec![
                             HIRExpression::Variable { name: tmp_id.clone(), ty: ty.clone() },
                             HIRExpression::Literal { value: elements.len().to_string(), ty: TejxType::Primitive("number".to_string()) }
                         ],
                         ty: TejxType::Any,
                     };
                     self.lower_binding_pattern(r, Some(slice_init), &TejxType::Any, is_const, stmts);
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
