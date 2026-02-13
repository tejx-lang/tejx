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
    class_methods: RefCell<HashMap<String, Vec<String>>>,
    class_instance_fields: RefCell<HashMap<String, Vec<(String, Expression)>>>,
    class_static_fields: RefCell<HashMap<String, Vec<(String, Expression)>>>,
    class_getters: RefCell<HashMap<String, HashSet<String>>>,
    class_setters: RefCell<HashMap<String, HashSet<String>>>,
    class_parents: RefCell<HashMap<String, String>>,
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
            class_methods: RefCell::new(HashMap::new()),
            class_instance_fields: RefCell::new(HashMap::new()),
            class_static_fields: RefCell::new(HashMap::new()),
            class_getters: RefCell::new(HashMap::new()),
            class_setters: RefCell::new(HashMap::new()),
            class_parents: RefCell::new(HashMap::new()),
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
             match stmt {
                 Statement::FunctionDeclaration(func) => {
                     let fixed_count = func.params.iter().take_while(|p| !p._is_rest).count();
                     if fixed_count < func.params.len() {
                         self.variadic_functions.borrow_mut().insert(func.name.clone(), fixed_count);
                     }
                 }
                 Statement::ClassDeclaration(class_decl) => {
                     for method in &class_decl.methods {
                         let fixed_count = method.func.params.iter().take_while(|p| !p._is_rest).count();
                         if fixed_count < method.func.params.len() {
                             let mangled = format!("{}_{}", class_decl.name, method.func.name);
                             self.variadic_functions.borrow_mut().insert(mangled, fixed_count);
                         }
                     }
                     if let Some(cons) = &class_decl._constructor {
                         let fixed_count = cons.params.iter().take_while(|p| !p._is_rest).count();
                         if fixed_count < cons.params.len() {
                             let mangled = format!("{}_constructor", class_decl.name);
                             self.variadic_functions.borrow_mut().insert(mangled, fixed_count);
                         }
                     }
                 }
                 _ => {}
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
                    let mut methods = Vec::new();
                    for method in &class_decl.methods {
                         let mangled = format!("{}_{}", class_decl.name, method.func.name);
                         self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&method.func.return_type));
                         if !method.is_static {
                             methods.push(method.func.name.clone());
                         }
                    }
                    self.class_methods.borrow_mut().insert(class_decl.name.clone(), methods);

                    let mut i_fields = Vec::new();
                    let mut s_fields = Vec::new();
                    for member in &class_decl._members {
                        if member._is_static {
                            if let Some(init) = &member._initializer {
                                s_fields.push((member._name.clone(), *init.clone()));
                            }
                        } else {
                            if let Some(init) = &member._initializer {
                                i_fields.push((member._name.clone(), *init.clone()));
                            }
                        }
                    }
                    self.class_instance_fields.borrow_mut().insert(class_decl.name.clone(), i_fields);
                    self.class_static_fields.borrow_mut().insert(class_decl.name.clone(), s_fields);

                    if let Some(constructor) = &class_decl._constructor {
                         let mangled = format!("{}_{}", class_decl.name, constructor.name);
                         self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&constructor.return_type));
                    }

                    let mut getters = HashSet::new();
                    for getter in &class_decl._getters {
                         let mangled = format!("{}_get_{}", class_decl.name, getter._name);
                         self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&getter._return_type));
                         getters.insert(getter._name.clone());
                    }
                    self.class_getters.borrow_mut().insert(class_decl.name.clone(), getters);

                    let mut setters = HashSet::new();
                    for setter in &class_decl._setters {
                         let mangled = format!("{}_set_{}", class_decl.name, setter._name);
                         self.user_functions.borrow_mut().insert(mangled, TejxType::Void);
                         setters.insert(setter._name.clone());
                    }
                    self.class_setters.borrow_mut().insert(class_decl.name.clone(), setters);

                    // Track parent class for instanceof support
                    self.class_parents.borrow_mut().insert(class_decl.name.clone(), class_decl._parent_name.clone());
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = &**declaration {
                         self.user_functions.borrow_mut().insert(func.name.clone(), TejxType::from_name(&func.return_type));
                    } else if let Statement::ClassDeclaration(class_decl) = &**declaration {
                        let mut methods = Vec::new();
                        for method in &class_decl.methods {
                             let mangled = format!("{}_{}", class_decl.name, method.func.name);
                             self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&method.func.return_type));
                             if !method.is_static {
                                 methods.push(method.func.name.clone());
                             }
                        }
                        self.class_methods.borrow_mut().insert(class_decl.name.clone(), methods);

                        if let Some(constructor) = &class_decl._constructor {
                             let mangled = format!("{}_{}", class_decl.name, constructor.name);
                             self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&constructor.return_type));
                        }
                    }
                }
                Statement::VarDeclaration { pattern, type_annotation, .. } => {
                    if let BindingNode::Identifier(name) = pattern {
                        let ty = TejxType::from_name(type_annotation);
                        self.define(name.clone(), ty);
                    }
                }
                Statement::ExtensionDeclaration(ext_decl) => {
                    let mut class_methods = self.class_methods.borrow_mut();
                    let methods = class_methods.entry(ext_decl._target_type.clone()).or_insert_with(Vec::new);
                    for method in &ext_decl._methods {
                        let mangled = format!("{}_{}", ext_decl._target_type, method.name);
                        self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&method.return_type));
                        methods.push(method.name.clone());
                    }
                }
                _ => {}
            }
        }

        // Pass 2: Lower
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    self.lower_function_declaration(func, &mut functions);
                }
                Statement::ClassDeclaration(class_decl) => {
                    self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                }
                Statement::ExportDecl { declaration, .. } => {
                    match &**declaration {
                        Statement::FunctionDeclaration(func) => {
                            self.lower_function_declaration(func, &mut functions);
                        }
                        Statement::ClassDeclaration(class_decl) => {
                            self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                        }
                        Statement::ExtensionDeclaration(ext_decl) => {
                            self.lower_extension_declaration(ext_decl, &mut functions);
                        }
                        _ => {
                            if let Some(hir) = self.lower_statement(stmt) {
                                main_stmts.push(hir);
                            }
                        }
                    }
                }
                Statement::ExtensionDeclaration(ext_decl) => {
                    self.lower_extension_declaration(ext_decl, &mut functions);
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
            _return_type: TejxType::Int32,
            body: Box::new(main_body),
        });

        LoweringResult { functions }
    }

    fn lower_function_declaration(&self, func: &FunctionDeclaration, functions: &mut Vec<HIRStatement>) {
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
        
        let name = format!("f_{}", func.name);
        functions.push(HIRStatement::Function {
            name, params, _return_type: return_type, body: Box::new(body),
        });
    }

    fn lower_class_declaration(&self, class_decl: &ClassDeclaration, functions: &mut Vec<HIRStatement>, main_stmts: &mut Vec<HIRStatement>) {
        *self.current_class.borrow_mut() = Some(class_decl.name.clone());
        let p_name = if class_decl._parent_name.is_empty() { None } else { Some(class_decl._parent_name.clone()) };
        *self.parent_class.borrow_mut() = p_name;

        let mut all_methods = Vec::new();
        for m in &class_decl.methods {
            all_methods.push((&m.func, m.is_static));
        }

        // Handle constructor (explicit or default)
        let default_body = Box::new(Statement::BlockStmt { statements: vec![], _line: 0, _col: 0 });
        let default_cons = FunctionDeclaration {
            name: "constructor".to_string(),
            params: vec![],
            return_type: "void".to_string(),
            body: default_body,
            _is_async: false,
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
            
            if let HIRStatement::Block { ref mut statements } = hir_body {
                 // Inject method attachments for constructor
                 if func_decl.name == "constructor" {
                     // Instance fields
                     let i_fields_borrow = self.class_instance_fields.borrow();
                     if let Some(i_list) = i_fields_borrow.get(&class_decl.name) {
                         for (f_name, f_init) in i_list {
                             let hir_init = self.lower_expression(f_init);
                             // Insert before other logic
                             statements.insert(0, HIRStatement::ExpressionStmt {
                                 expr: HIRExpression::Assignment {
                                     target: Box::new(HIRExpression::MemberAccess {
                                         target: Box::new(HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) }),
                                         member: f_name.clone(),
                                         ty: TejxType::Any
                                     }),
                                     value: Box::new(hir_init),
                                     ty: TejxType::Any
                                 }
                             });
                         }
                     }

                     let methods_borrow = self.class_methods.borrow();
                     if let Some(m_list) = methods_borrow.get(&class_decl.name) {
                         for m_name in m_list {
                             let mangled_func = format!("f_{}_{}", class_decl.name, m_name);
                             // Insert after debug print
                             statements.insert(0, HIRStatement::ExpressionStmt {
                                 expr: HIRExpression::Call {
                                     callee: "m_set".to_string(),
                                     args: vec![
                                         HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) },
                                         HIRExpression::Literal { value: m_name.clone(), ty: TejxType::String },
                                         HIRExpression::Variable { name: mangled_func, ty: TejxType::Any }
                                     ],
                                     ty: TejxType::Void
                                 }
                             });
                         }
                     }

                     // Getters
                     let getters_borrow = self.class_getters.borrow();
                     if let Some(g_list) = getters_borrow.get(&class_decl.name) {
                         for g_name in g_list {
                             let mangled_func = format!("f_{}_get_{}", class_decl.name, g_name);
                             statements.insert(0, HIRStatement::ExpressionStmt {
                                 expr: HIRExpression::Call {
                                     callee: "m_set".to_string(),
                                     args: vec![
                                         HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) },
                                         HIRExpression::Literal { value: format!("get_{}", g_name), ty: TejxType::String },
                                         HIRExpression::Variable { name: mangled_func, ty: TejxType::Any }
                                     ],
                                     ty: TejxType::Void
                                 }
                             });
                         }
                     }

                     // Setters
                     let setters_borrow = self.class_setters.borrow();
                     if let Some(s_list) = setters_borrow.get(&class_decl.name) {
                         for s_name in s_list {
                             let mangled_func = format!("f_{}_set_{}", class_decl.name, s_name);
                             statements.insert(0, HIRStatement::ExpressionStmt {
                                 expr: HIRExpression::Call {
                                     callee: "m_set".to_string(),
                                     args: vec![
                                         HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) },
                                         HIRExpression::Literal { value: format!("set_{}", s_name), ty: TejxType::String },
                                         HIRExpression::Variable { name: mangled_func, ty: TejxType::Any }
                                     ],
                                     ty: TejxType::Void
                                 }
                             });
                         }
                     }

                     // Inject class metadata for instanceof support
                     // Set __class__ = "ClassName" on this
                     statements.insert(0, HIRStatement::ExpressionStmt {
                         expr: HIRExpression::Call {
                             callee: "m_set".to_string(),
                             args: vec![
                                 HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) },
                                 HIRExpression::Literal { value: "__class__".to_string(), ty: TejxType::String },
                                 HIRExpression::Literal { value: class_decl.name.clone(), ty: TejxType::String },
                             ],
                             ty: TejxType::Void
                         }
                     });

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
                         statements.insert(0, HIRStatement::ExpressionStmt {
                             expr: HIRExpression::Call {
                                 callee: "m_set".to_string(),
                                 args: vec![
                                     HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) },
                                     HIRExpression::Literal { value: "__parents__".to_string(), ty: TejxType::String },
                                     HIRExpression::Literal { value: parents_str, ty: TejxType::String },
                                 ],
                                 ty: TejxType::Void
                             }
                         });
                     }
                 }
            }
            
            let body = hir_body;
            
            self._exit_scope();

            functions.push(HIRStatement::Function {
                name, params, _return_type: return_type, body: Box::new(body),
            });
        }
        

        // Lower getters
        for getter in &class_decl._getters {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            params.push(("this".to_string(), TejxType::Class(class_decl.name.clone())));
            
            let name = format!("f_{}_get_{}", class_decl.name, getter._name);
            let return_type = TejxType::from_name(&getter._return_type);
            
            self.enter_scope();
            for (pname, pty) in &params {
                self.define(pname.clone(), pty.clone());
            }
            
            let hir_body = self.lower_statement(&getter._body)
                .unwrap_or(HIRStatement::Block { statements: vec![] });

            self._exit_scope();
            functions.push(HIRStatement::Function {
                name, params, _return_type: return_type, body: Box::new(hir_body),
            });
        }

        // Lower setters
        for setter in &class_decl._setters {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            params.push(("this".to_string(), TejxType::Class(class_decl.name.clone())));
            params.push((setter._param_name.clone(), TejxType::from_name(&setter._param_type)));
            
            let name = format!("f_{}_set_{}", class_decl.name, setter._name);
            
            self.enter_scope();
            for (pname, pty) in &params {
                self.define(pname.clone(), pty.clone());
            }
            
            let hir_body = self.lower_statement(&setter._body)
                .unwrap_or(HIRStatement::Block { statements: vec![] });

            self._exit_scope();
            functions.push(HIRStatement::Function {
                name, params, _return_type: TejxType::Void, body: Box::new(hir_body),
            });
        }

        // Lower static fields into global assignments
        let s_fields_borrow = self.class_static_fields.borrow();
        if let Some(s_list) = s_fields_borrow.get(&class_decl.name) {
            for (f_name, f_init) in s_list {
                let hir_init = self.lower_expression(f_init);
                // Static fields are mangled as g_Class_Field
                let mangled_name = format!("g_{}_{}", class_decl.name, f_name);
                main_stmts.push(HIRStatement::ExpressionStmt {
                    expr: HIRExpression::Assignment {
                        target: Box::new(HIRExpression::Variable { name: mangled_name, ty: TejxType::Any }),
                        value: Box::new(hir_init),
                        ty: TejxType::Any
                    }
                });
            }
        }

        *self.current_class.borrow_mut() = None;
        *self.parent_class.borrow_mut() = None;
    }

    fn lower_extension_declaration(&self, ext_decl: &ExtensionDeclaration, functions: &mut Vec<HIRStatement>) {
        for func_decl in &ext_decl._methods {
            let mut params: Vec<(String, TejxType)> = Vec::new();
            params.push(("this".to_string(), TejxType::Class(ext_decl._target_type.clone())));
            
            for p in &func_decl.params {
                params.push((p.name.clone(), TejxType::from_name(&p.type_name)));
            }
            
            let name = format!("f_{}_{}", ext_decl._target_type, func_decl.name);
            let return_type = TejxType::from_name(&func_decl.return_type);
            
            self.enter_scope();
            for (pname, pty) in &params {
                self.define(pname.clone(), pty.clone());
            }
            
            let hir_body = self.lower_statement(&func_decl.body)
                .unwrap_or(HIRStatement::Block { statements: vec![] });
            
            self._exit_scope();

            functions.push(HIRStatement::Function {
                name, params, _return_type: return_type, body: Box::new(hir_body),
            });
        }
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
                let ty = if type_annotation.is_empty() {
                    init.as_ref().map(|e| e.get_type()).unwrap_or(TejxType::Any)
                } else {
                    TejxType::from_name(type_annotation)
                };
                
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
                        ty: TejxType::Bool,
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
                            ty: TejxType::Int32
                        }),
                        ty: TejxType::Int32,
                        _is_const: true,
                    });
                    
                    // 3. Index
                    let idx_name = format!("__idx_{}", var_name);
                    stmts.push(HIRStatement::VarDecl {
                        name: idx_name.clone(),
                        initializer: Some(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                        ty: TejxType::Int32,
                        _is_const: false,
                    });
                    
                    // 4. Loop
                    // Condition: idx < len
                    let cond = HIRExpression::BinaryExpr {
                        left: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Int32 }),
                        op: TokenType::Less,
                        right: Box::new(HIRExpression::Variable { name: len_name, ty: TejxType::Int32 }),
                        ty: TejxType::Bool,
                    };
                    
                    // Body construction
                    let mut body_stmts = Vec::new();
                    
                    // let var_name = _arr[_idx];
                    let val_expr = HIRExpression::IndexAccess {
                        target: Box::new(HIRExpression::Variable { name: arr_name, ty: TejxType::Any }),
                        index: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Int32 }),
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
                            target: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Int32 }),
                            value: Box::new(HIRExpression::BinaryExpr {
                                left: Box::new(HIRExpression::Variable { name: idx_name.clone(), ty: TejxType::Int32 }),
                                op: TokenType::Plus,
                                right: Box::new(HIRExpression::Literal { value: "1".to_string(), ty: TejxType::Int32 }),
                                ty: TejxType::Int32
                            }),
                            ty: TejxType::Int32
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
                let (val_str, ty) = if value.fract() == 0.0 {
                    (format!("{:.0}", value), TejxType::Int32)
                } else {
                    (value.to_string(), TejxType::Float32)
                };
                HIRExpression::Literal {
                    value: val_str,
                    ty,
                }
            }
            Expression::StringLiteral { value, .. } => {
                HIRExpression::Literal {
                    value: value.clone(),
                    ty: TejxType::String,
                }
            }
            Expression::BooleanLiteral { value, .. } => {
                HIRExpression::Literal {
                    value: value.to_string(),
                    ty: TejxType::Bool,
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
                let final_name = if self.user_functions.borrow().contains_key(name) && name != "main" {
                    format!("f_{}", name)
                } else {
                    name.clone()
                };
                HIRExpression::Variable {
                    name: final_name,
                    ty,
                }
            }
            Expression::NoneExpr { .. } | Expression::UndefinedExpr { .. } => {
                 HIRExpression::Literal {
                    value: "0".to_string(),
                    ty: TejxType::Int32, // Or Any? Using Int32 for 0 is safer for now.
                 }
            }
            Expression::NullishCoalescingExpr { _left, _right, .. } => {
                // Desugar to: let temp = left; if temp != 0 { temp } else { right }
                // Since we don't have block expressions easily here without generating a function or specialized HIR,
                // we'll implement it as a conditional (Ternary) if possible, or BinaryOp if we treat it as ||
                // For now, treat as || (PipePipe) as we lack separate None value from 0
                let left_hir = self.lower_expression(_left);
                let right_hir = self.lower_expression(_right);
                
                HIRExpression::BinaryExpr {
                    op: TokenType::PipePipe,
                    left: Box::new(left_hir),
                    right: Box::new(right_hir),
                    ty: TejxType::Any
                }
            }
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                 let obj_hir = self.lower_expression(object);
                 
                 HIRExpression::If {
                     condition: Box::new(HIRExpression::BinaryExpr {
                         op: TokenType::BangEqual,
                         left: Box::new(obj_hir.clone()), 
                         right: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                         ty: TejxType::Bool
                     }),
                     then_branch: Box::new(HIRExpression::MemberAccess {
                         target: Box::new(obj_hir),
                         member: member.clone(),
                         ty: TejxType::Any
                     }),
                     else_branch: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                     ty: TejxType::Any
                 }
            }
            Expression::OptionalCallExpr { callee, args, .. } => {
                 let callee_hir = self.lower_expression(callee);
                 let args_hir: Vec<HIRExpression> = args.iter().map(|a| self.lower_expression(a)).collect();
                 
                  HIRExpression::If {
                     condition: Box::new(HIRExpression::BinaryExpr {
                         op: TokenType::BangEqual,
                         left: Box::new(callee_hir.clone()), 
                         right: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                         ty: TejxType::Bool
                     }),
                     then_branch: Box::new(HIRExpression::IndirectCall {
                         callee: Box::new(callee_hir),
                         args: args_hir,
                         ty: TejxType::Any
                     }),
                     else_branch: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                     ty: TejxType::Any
                 }
            }
             Expression::OptionalArrayAccessExpr { target, index, .. } => {
                 let target_hir = self.lower_expression(target);
                 let index_hir = self.lower_expression(index);
                 
                 HIRExpression::If {
                     condition: Box::new(HIRExpression::BinaryExpr {
                         op: TokenType::BangEqual,
                         left: Box::new(target_hir.clone()), 
                         right: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                         ty: TejxType::Bool
                     }),
                     then_branch: Box::new(HIRExpression::IndexAccess {
                         target: Box::new(target_hir),
                         index: Box::new(index_hir),
                         ty: TejxType::Any
                     }),
                     else_branch: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                     ty: TejxType::Any
                 }
            }
            Expression::BinaryExpr { left, op, right, .. } => {
                // Desugar instanceof to runtime call
                if matches!(op, TokenType::Instanceof) {
                    let obj = self.lower_expression(left);
                    // Right side should be a class name identifier
                    let class_name = match right.as_ref() {
                        Expression::Identifier { name, .. } => name.clone(),
                        _ => "__unknown__".to_string(),
                    };
                    return HIRExpression::Call {
                        callee: "rt_instanceof".to_string(),
                        args: vec![
                            obj,
                            HIRExpression::Literal { value: class_name, ty: TejxType::String },
                        ],
                        ty: TejxType::Int32,
                    };
                }
                let l = self.lower_expression(left);
                let r = self.lower_expression(right);

                // Desugar === and !== to runtime calls
                if matches!(op, TokenType::EqualEqualEqual) || matches!(op, TokenType::BangEqualEqual) {
                     let callee = if matches!(op, TokenType::EqualEqualEqual) { "rt_strict_equal" } else { "rt_strict_ne" };
                     return HIRExpression::Call {
                         callee: callee.to_string(),
                         args: vec![l, r],
                         ty: TejxType::Bool
                     };
                }

                let bin_ty = self.infer_hir_binary_type(&l, op, &r);
                HIRExpression::BinaryExpr {
                    left: Box::new(l),
                    op: op.clone(),
                    right: Box::new(r),
                    ty: bin_ty,
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

                if let Expression::MemberAccessExpr { object, member, .. } = target.as_ref() {
                    let obj_ty = self.lower_expression(object).get_type();
                    if let TejxType::Class(class_name) = obj_ty {
                        let setters = self.class_setters.borrow();
                        if let Some(s_set) = setters.get(&class_name) {
                            if s_set.contains(member) {
                                return HIRExpression::Call {
                                    callee: format!("f_{}_set_{}", class_name, member),
                                    args: vec![self.lower_expression(object), final_value],
                                    ty: TejxType::Void
                                };
                            }
                        }
                    }
                }

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
                                 right: Box::new(HIRExpression::Literal { value: delta.to_string(), ty: TejxType::Int32 }),
                                 ty: TejxType::Int32
                             }),
                             ty: TejxType::Int32
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
                             right: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Bool }),
                             ty: TejxType::Bool
                         }
                     }
                     TokenType::Minus => {
                         // -x -> 0 - x
                         HIRExpression::BinaryExpr {
                             left: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
                             op: TokenType::Minus,
                             right: Box::new(self.lower_expression(right)),
                             ty: TejxType::Int32
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
                let mut final_callee = normalized.clone();
                let mut final_args = hir_args.clone();
                let mut ty = TejxType::Any;

                if callee == "typeof" {
                    let r_expr = hir_args[0].clone();
                    let r_ty = r_expr.get_type();
                    if matches!(r_ty, TejxType::Any) {
                        return HIRExpression::Call {
                            callee: "rt_typeof".to_string(),
                            args: vec![r_expr],
                            ty: TejxType::String,
                        };
                    } else {
                        let type_name = match &r_ty {
                            TejxType::Int32 | TejxType::Int16 | TejxType::Int64 | TejxType::Int128 => "number",
                            TejxType::Float32 | TejxType::Float16 | TejxType::Float64 => "number",
                            TejxType::Bool => "bool",
                            TejxType::String => "string",
                            TejxType::Char => "char",
                            TejxType::Class(c) => c,
                            _ => "object",
                        };
                        return HIRExpression::Literal {
                            value: format!("\"{}\"", type_name),
                            ty: TejxType::String,
                        };
                    }
                } else if callee == "sizeof" {
                    let r_expr = hir_args[0].clone();
                    let r_ty = r_expr.get_type();
                    return HIRExpression::Literal {
                        value: r_ty.size().to_string(),
                        ty: TejxType::Int32,
                    };
                }

                if callee == "super" {
                    if let Some(parent) = &*self.parent_class.borrow() {
                        final_callee = format!("f_{}_constructor", parent);
                        final_args = vec![HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }];
                        final_args.extend(hir_args.clone());
                    }
                } else if self.user_functions.borrow().contains_key(callee) {
                    final_callee = if callee == "main" { "f_main".to_string() } else { format!("f_{}", callee) };
                } else if self.stdlib.is_prelude_func(&normalized) || normalized == "log" || normalized == "console_log" {
                    final_callee = if normalized == "log" || normalized == "console_log" { "print".to_string() } else { normalized.clone() };
                } else {
                    // Check stdlib imports
                    let imports = self.std_imports.borrow();
                    let mut found_std = false;
                    for (mod_name, mode) in imports.iter() {
                        let is_imported = match mode {
                            ImportMode::All => {
                                if callee.contains('.') {
                                    let parts: Vec<&str> = callee.split('.').collect();
                                    if parts[0] == mod_name {
                                        self.stdlib.is_std_func(mod_name, parts[1])
                                    } else {
                                        false
                                    }
                                } else {
                                    self.stdlib.is_std_func(mod_name, callee)
                                }
                            }
                            ImportMode::Named(set) => {
                                if callee.contains('.') {
                                    let parts: Vec<&str> = callee.split('.').collect();
                                    if parts[0] == mod_name {
                                        set.contains(parts[1])
                                    } else {
                                        false
                                    }
                                } else {
                                    set.contains(callee)
                                }
                            }
                        };
                        
                        if is_imported {
                            found_std = true;
                            let func_name = if callee.contains('.') {
                                callee.split('.').collect::<Vec<&str>>()[1]
                            } else {
                                callee
                            };
                            final_callee = self.stdlib.get_runtime_name(mod_name, func_name);
                            if mod_name == "math" {
                                ty = TejxType::Float32;
                            }
                            break;
                        }
                    }

                    if !found_std && callee.contains('.') && 
                       !callee.starts_with("Math.") && 
                       !callee.starts_with("fs.") && !callee.starts_with("Date.") && 
                       !callee.starts_with("http.") &&
                       !callee.starts_with("Promise.") && !callee.starts_with("Array.") {
                        
                        let parts: Vec<&str> = callee.split('.').collect();
                        if parts.len() >= 2 {
                            let method_name = parts.last().unwrap().to_string();
                            let obj_path = parts[0..parts.len()-1].join(".");
                            let obj_name = parts[0]; 

                            if obj_name == "super" {
                                if let Some(parent) = &*self.parent_class.borrow() {
                                    final_callee = format!("f_{}_{}", parent, method_name);
                                    final_args = vec![HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }];
                                    final_args.extend(hir_args.clone());
                                }
                            } else {
                                // Try Static Dispatch for Classes
                                let mut class_name_opt = None;
                                let mut is_instance = false;
                                if let Some(TejxType::Class(cn)) = self.lookup(obj_name) {
                                    class_name_opt = Some(cn);
                                    is_instance = true;
                                } else if self.class_methods.borrow().contains_key(obj_name) {
                                    class_name_opt = Some(obj_name.to_string());
                                }

                                if let Some(class_name) = class_name_opt {
                                    let mangled_key = format!("{}_{}", class_name, method_name);
                                    if self.user_functions.borrow().contains_key(&mangled_key) {
                                        final_callee = format!("f_{}", mangled_key);
                                        let mut m_args = Vec::new();
                                        if is_instance {
                                            m_args.push(HIRExpression::Variable { 
                                                name: obj_name.to_string(), 
                                                ty: TejxType::Class(class_name.clone())
                                            });
                                        }
                                        m_args.extend(hir_args.clone());
                                        final_args = m_args;
                                    }
                                }

                                // If not resolved yet, check common methods or fallback to dynamic
                                if final_callee == normalized {
                                    let obj_expr = if obj_path == "this" || obj_path == "super" {
                                        HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }
                                    } else if obj_path.contains('.') {
                                         let sub_parts: Vec<&str> = obj_path.split('.').collect();
                                         let mut current = if sub_parts[0] == "this" {
                                             HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }
                                         } else {
                                             HIRExpression::Variable { name: sub_parts[0].to_string(), ty: TejxType::Any }
                                         };
                                         for idx in 1..sub_parts.len() {
                                             current = HIRExpression::MemberAccess {
                                                 target: Box::new(current),
                                                 member: sub_parts[idx].to_string(),
                                                 ty: TejxType::Any
                                             };
                                         }
                                         current
                                    } else {
                                        HIRExpression::Variable { name: obj_path.clone(), ty: TejxType::Any }
                                    };

                                    let (runtime_callee, ret_type) = match method_name.as_str() {
                                         "push" | "unshift" | "indexOf" => (format!("Array_{}", method_name), TejxType::Int32),
                                         "pop" | "shift" => (format!("Array_{}", method_name), TejxType::Any), 
                                         "join" => ("__join".to_string(), TejxType::Any),
                                         "lock" | "unlock" => (format!("m_{}", method_name), TejxType::Any),
                                         "concat" | "map" | "filter" => (format!("Array_{}", method_name), TejxType::Any),
                                         "forEach" => (format!("Array_{}", method_name), TejxType::Void),
                                         "set" => ("m_set".to_string(), TejxType::Any),
                                         "get" => ("m_get".to_string(), TejxType::Any),
                                         "has" => ("rt_has".to_string(), TejxType::Any),
                                         "delete" | "del" => ("rt_del".to_string(), TejxType::Any),
                                         "size" => ("s_size".to_string(), TejxType::Any),
                                         "clear" => ("m_clear".to_string(), TejxType::Any),
                                         "add" => ("s_add".to_string(), TejxType::Any),
                                         "trim" => ("rt_string_trim".to_string(), TejxType::Any),
                                         "toLowerCase" => ("rt_string_to_lower".to_string(), TejxType::Any),
                                         "toUpperCase" => ("rt_string_to_upper".to_string(), TejxType::Any),
                                         "startsWith" => ("rt_string_starts_with".to_string(), TejxType::Bool),
                                         "endsWith" => ("rt_string_ends_with".to_string(), TejxType::Bool),
                                         "replace" => ("rt_string_replace".to_string(), TejxType::Any),
                                         "getTime" => ("d_getTime".to_string(), TejxType::Int32),
                                         "toISOString" => ("d_toISOString".to_string(), TejxType::Any),
                                         _ => ("".to_string(), TejxType::Any)
                                    };

                                    if !runtime_callee.is_empty() {
                                        final_callee = runtime_callee;
                                        let mut n_args = vec![obj_expr];
                                        n_args.extend(hir_args.clone());
                                        if final_callee == "__join" && n_args.len() < 2 {
                                             n_args.push(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 });
                                        }
                                        final_args = n_args;
                                    } else {
                                         // Indirect Call fallback
                                         let mut i_args = vec![obj_expr.clone()];
                                         i_args.extend(hir_args.clone());
                                         return HIRExpression::IndirectCall {
                                             callee: Box::new(HIRExpression::MemberAccess {
                                                 target: Box::new(obj_expr),
                                                 member: method_name.clone(),
                                                 ty: TejxType::Any
                                             }),
                                             args: i_args,
                                             ty: TejxType::Any
                                         };
                                    }
                                }
                            }
                        }
                    }
                }

                // CHECK FOR VARIADIC PACKING (Unmangled name or Mangled name)
                let lookup_name = if final_callee.starts_with("f_") {
                    &final_callee[2..]
                } else {
                    &final_callee
                };

                if let Some(&fixed_count) = self.variadic_functions.borrow().get(lookup_name) {
                     if final_args.len() >= fixed_count {
                         let (fixed, rest) = final_args.split_at(fixed_count);
                         let mut new_var_args = fixed.to_vec();
                         new_var_args.push(HIRExpression::ArrayLiteral {
                             elements: rest.to_vec(),
                             ty: TejxType::Any
                         });
                         final_args = new_var_args;
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
                
                // Static Field Resolution
                if !obj_name.is_empty() {
                    let s_fields = self.class_static_fields.borrow();
                    if let Some(f_list) = s_fields.get(&obj_name) {
                        if f_list.iter().any(|(n, _)| n == member) {
                            return HIRExpression::Variable {
                                name: format!("g_{}_{}", obj_name, member),
                                ty: TejxType::Any
                            };
                        }
                    }
                }

                // Getter Resolution
                let obj_ty = self.lower_expression(object).get_type();
                if let TejxType::Class(class_name) = obj_ty {
                    let getters = self.class_getters.borrow();
                    if let Some(g_set) = getters.get(&class_name) {
                        if g_set.contains(member) {
                            return HIRExpression::Call {
                                callee: format!("f_{}_get_{}", class_name, member),
                                args: vec![self.lower_expression(object)],
                                ty: TejxType::Any
                            };
                        }
                    }
                }

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
                let mut hir_args: Vec<HIRExpression> = args.iter()
                    .map(|a| self.lower_expression(a))
                    .collect();
                
                // Variadic check for constructor
                let cons_unmangled = format!("{}_constructor", class_name);
                if let Some(&fixed_count) = self.variadic_functions.borrow().get(&cons_unmangled) {
                     if hir_args.len() >= fixed_count {
                         let (fixed, rest) = hir_args.split_at(fixed_count);
                         let mut new_var_args = fixed.to_vec();
                         new_var_args.push(HIRExpression::ArrayLiteral {
                             elements: rest.to_vec(),
                             ty: TejxType::Any
                         });
                         hir_args = new_var_args;
                     }
                }

                HIRExpression::NewExpr {
                    class_name: class_name.clone(),
                    _args: hir_args,
                }
            }
            Expression::OptionalCallExpr { callee, args, .. } => {
                let mut hir_args: Vec<HIRExpression> = args.iter()
                    .map(|a| self.lower_expression(a))
                    .collect();
                
                let callee_expr = self.lower_expression(callee);
                
                // Attempt variadic packing if callee is an identifier
                let callee_name = match callee.as_ref() {
                    Expression::Identifier { name, .. } => Some(name.clone()),
                    _ => None
                };
                
                if let Some(name) = callee_name {
                    let lookup_name = if name.starts_with("f_") { &name[2..] } else { &name };
                    if let Some(&fixed_count) = self.variadic_functions.borrow().get(lookup_name) {
                        if hir_args.len() >= fixed_count {
                            let (fixed, rest) = hir_args.split_at(fixed_count);
                            let mut new_var_args = fixed.to_vec();
                            new_var_args.push(HIRExpression::ArrayLiteral {
                                elements: rest.to_vec(),
                                ty: TejxType::Any
                            });
                            hir_args = new_var_args;
                        }
                    }
                }

                HIRExpression::OptionalChain {
                    target: Box::new(callee_expr),
                    operation: "()".to_string(), // In HIR/MIR, OptionalChain "()" means call
                    ty: TejxType::Any,
                }
            }
            Expression::TernaryExpr { _condition, _true_branch, _false_branch, .. } => {
                let cond = self.lower_expression(_condition);
                let t_branch = self.lower_expression(_true_branch);
                let f_branch = self.lower_expression(_false_branch);
                HIRExpression::If {
                    condition: Box::new(cond),
                    then_branch: Box::new(t_branch),
                    else_branch: Box::new(f_branch),
                    ty: TejxType::Any,
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
                        index: Box::new(HIRExpression::Literal { value: i.to_string(), ty: TejxType::Int32 }),
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
                             HIRExpression::Literal { value: elements.len().to_string(), ty: TejxType::Int32 }
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

impl Lowering {

    fn infer_hir_binary_type(&self, left: &HIRExpression, op: &TokenType, right: &HIRExpression) -> TejxType {
        let lt = left.get_type();
        let rt = right.get_type();

        if matches!(op, TokenType::EqualEqual | TokenType::BangEqual | TokenType::Less | TokenType::LessEqual | TokenType::Greater | TokenType::GreaterEqual | TokenType::AmpersandAmpersand | TokenType::PipePipe) {
            return TejxType::Bool;
        }

        if lt == TejxType::String || rt == TejxType::String {
            return TejxType::String;
        }

        let is_float = |t: &TejxType| -> bool {
            matches!(t, TejxType::Float16 | TejxType::Float32 | TejxType::Float64)
        };

        if is_float(&lt) || is_float(&rt) {
            return TejxType::Float32; // Default promotion
        }

        if matches!(lt, TejxType::Int16 | TejxType::Int32 | TejxType::Int64 | TejxType::Int128) ||
           matches!(rt, TejxType::Int16 | TejxType::Int32 | TejxType::Int64 | TejxType::Int128) {
            return TejxType::Int32; // Default promotion
        }

        TejxType::Any
    }
}
