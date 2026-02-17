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
    scopes: RefCell<Vec<HashMap<String, (String, TejxType)>>>, // name -> (mangled_name, type)
    next_scope_id: RefCell<usize>,
    std_imports: RefCell<HashMap<String, ImportMode>>,
    stdlib: StdLib,
    current_class: RefCell<Option<String>>,
    parent_class: RefCell<Option<String>>,
    class_methods: RefCell<HashMap<String, Vec<String>>>,
    class_instance_fields: RefCell<HashMap<String, Vec<(String, TejxType, Expression)>>>,
    class_static_fields: RefCell<HashMap<String, Vec<(String, TejxType, Expression)>>>,
    class_getters: RefCell<HashMap<String, HashSet<String>>>,
    class_setters: RefCell<HashMap<String, HashSet<String>>>,
    class_parents: RefCell<HashMap<String, String>>,
    pub async_enabled: bool,
    current_async_promise_id: RefCell<Option<String>>,
}

/// Result of lowering: a list of top-level HIR functions.
/// The last one is always "tejx_main" containing non-function statements.
pub struct LoweringResult {
    pub functions: Vec<HIRStatement>,  // Each should be HIRStatement::Function
    pub signatures: HashMap<String, Vec<TejxType>>,
}

impl Lowering {
    pub fn new() -> Self {
        Lowering {
            lambda_counter: RefCell::new(0),
            user_functions: RefCell::new(HashMap::new()),
            variadic_functions: RefCell::new(HashMap::new()),
            lambda_functions: RefCell::new(Vec::new()),
            scopes: RefCell::new(vec![HashMap::new()]), // Global scope
            next_scope_id: RefCell::new(1), // Global is 0
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
            async_enabled: true,
            current_async_promise_id: RefCell::new(None),
        }
    }

    fn enter_scope(&self) {
        self.scopes.borrow_mut().push(HashMap::new());
    }

    fn _exit_scope(&self) {
        self.scopes.borrow_mut().pop();
    }

    fn define(&self, name: String, ty: TejxType) -> String {
        let depth = self.scopes.borrow().len() - 1;
        let mangled = if depth == 0 {
            name.clone()
        } else {
            // Include a unique ID per scope to avoid collisions between siblings
            let id = self.next_scope_id.borrow_mut();
            let mangled = format!("{}${}", name, *id);
            // Wait, we only need to increment if we actually define something in a NEW scope?
            // Actually, incrementing on enter_scope is better.
            mangled
        };

        if let Some(scope) = self.scopes.borrow_mut().last_mut() {
            scope.insert(name, (mangled.clone(), ty));
        }
        mangled
    }

    fn lookup(&self, name: &str) -> Option<(String, TejxType)> {
        let scopes = self.scopes.borrow();
        for scope in scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info.clone());
            }
        }
        None
    }

    #[allow(dead_code)]
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
            "rt_not",
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

                    if mod_name == "net" {
                        imports.insert("http".to_string(), ImportMode::All);
                        imports.insert("https".to_string(), ImportMode::All);
                    }

                    if mod_name == "collections" {
                        let mut class_methods = self.class_methods.borrow_mut();
                        let mut user_functions = self.user_functions.borrow_mut();
                        
                        let collections = [
                            ("Stack", vec!["push", "pop", "peek", "size", "isEmpty"]),
                            ("Queue", vec!["enqueue", "dequeue", "size", "isEmpty"]),
                            ("MinHeap", vec!["insert", "extractMin", "size", "isEmpty"]),
                            ("MaxHeap", vec!["insertMax", "extractMax", "size", "isEmpty"]),
                            ("PriorityQueue", vec!["insert", "extractMin", "size", "isEmpty"]),
                            ("Map", vec!["set", "get", "put", "at", "delete", "remove", "has", "size", "isEmpty", "keys", "values"]),
                            ("Set", vec!["add", "delete", "remove", "has", "size", "isEmpty", "values"]),
                            ("OrderedMap", vec!["put", "at", "has", "size", "isEmpty"]),
                            ("OrderedSet", vec!["add", "has", "size", "isEmpty"]),
                            ("BloomFilter", vec!["add", "contains"]),
                            ("Trie", vec!["addPath", "find"]),
                        ];

                        for (cls, methods) in collections {
                            class_methods.insert(cls.to_string(), methods.iter().map(|s| s.to_string()).collect());
                            for m in methods {
                                let mangled = format!("{}_{}", cls, m);
                                user_functions.insert(mangled, TejxType::Any);
                            }
                            // Also register constructor
                            user_functions.insert(format!("{}_constructor", cls), TejxType::Void);
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
                         let mangled = if method.func.name.starts_with("f_") {
                             method.func.name.clone()
                         } else {
                             format!("f_{}_{}", class_decl.name, method.func.name)
                         };
                         self.user_functions.borrow_mut().insert(mangled, TejxType::from_name(&method.func.return_type));
                         if !method.is_static {
                             methods.push(method.func.name.clone());
                         }
                    }
                    self.class_methods.borrow_mut().insert(class_decl.name.clone(), methods);

                    let mut i_fields = Vec::new();
                    let mut s_fields = Vec::new();
                    for member in &class_decl._members {
                        let ty = TejxType::from_name(&member._type_name);
                        let init = member._initializer.as_ref().map(|e| *e.clone()).unwrap_or(Expression::NumberLiteral { value: 0.0, _line: 0, _col: 0 });
                        if member._is_static {
                            s_fields.push((member._name.clone(), ty, init));
                        } else {
                            i_fields.push((member._name.clone(), ty, init));
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
                             let mangled = if method.func.name.starts_with("f_") {
                                 method.func.name.clone()
                             } else {
                                 format!("f_{}_{}", class_decl.name, method.func.name)
                             };
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
                        let mangled = if method.name.starts_with("f_") {
                            method.name.clone()
                        } else {
                            format!("f_{}_{}", ext_decl._target_type, method.name)
                        };
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
        // Check for user-defined main function (lowered as "f_main")
        let mut main_func_idx = None;
        for (i, func) in functions.iter().enumerate() {
            if let HIRStatement::Function { name, .. } = func {
                if name == "f_main" {
                    main_func_idx = Some(i);
                    break;
                }
            }
        }

        let mut entry_body_stmts = main_stmts;

        if let Some(idx) = main_func_idx {
            // Found a user-defined main. Use its body as the entry point base.
            let main_func = functions.remove(idx);
            
            if let HIRStatement::Function { body, .. } = main_func {
                if let HIRStatement::Block { statements } = *body {
                    entry_body_stmts.extend(statements);
                } else {
                    entry_body_stmts.push(*body);
                }
            }
        } 
        
        // Finalize entry point: Run event loop (moved to runtime.rs)
        // entry_body_stmts.push(HIRStatement::ExpressionStmt {
        //     expr: HIRExpression::Call {
        //         callee: "tejx_run_event_loop".to_string(),
        //         args: vec![],
        //         ty: TejxType::Void,
        //     }
        // });
        // entry_body_stmts.push(HIRStatement::Return { value: Some(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }) });

        // Create the actual entry point function
        functions.push(HIRStatement::Function {
            name: "tejx_main".to_string(),
            params: vec![],
            _return_type: TejxType::Void,
            body: Box::new(HIRStatement::Block {
                statements: entry_body_stmts
            }),
        });

        let mut signatures = HashMap::new();
        // Add built-in runtime signatures for proper auto-boxing in MIR
        signatures.insert("trimmed_concat".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("trimmed_indexOf".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("trimmed_includes".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("trimmed_slice".to_string(), vec![TejxType::Any, TejxType::Any, TejxType::Any]);
        signatures.insert("trimmed_trimStart".to_string(), vec![TejxType::Any]);
        signatures.insert("trimmed_trimEnd".to_string(), vec![TejxType::Any]);
        signatures.insert("Array_sort".to_string(), vec![TejxType::Any]);
        signatures.insert("Array_flat".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("Array_includes".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("Array_concat".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("__resolve_promise".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("__reject_promise".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("Thread_new".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("tejx_enqueue_task".to_string(), vec![TejxType::Any, TejxType::Any]);
        signatures.insert("tejx_inc_async_ops".to_string(), vec![]);
        signatures.insert("tejx_dec_async_ops".to_string(), vec![]);
        signatures.insert("tejx_run_event_loop".to_string(), vec![]);
        signatures.insert("Promise_new".to_string(), vec![TejxType::Any]);
        signatures.insert("rt_sleep".to_string(), vec![TejxType::Any]);

        for func in &functions {
            if let HIRStatement::Function { name, params, .. } = func {
                let param_types: Vec<TejxType> = params.iter().map(|(_, ty)| ty.clone()).collect();
                signatures.insert(name.clone(), param_types);
            }
        }

        LoweringResult { functions, signatures }
    }

    fn lower_function_declaration(&self, func: &FunctionDeclaration, functions: &mut Vec<HIRStatement>) {
        if self.async_enabled && func._is_async {
            self.lower_async_function(func, functions);
            return;
        }

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

    fn lower_async_function(&self, func: &FunctionDeclaration, functions: &mut Vec<HIRStatement>) {
        let params: Vec<(String, TejxType)> = func.params.iter()
            .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name)))
            .collect();
            
        self.enter_scope();
        for (name, ty) in &params {
            self.define(name.clone(), ty.clone());
        }

        let (worker, _state_struct, wrapper_body) = self.lower_async_function_impl(&func.name, &params, &func.return_type, &func.body);
        
        self._exit_scope();
        
        functions.push(worker);
        // If _state_struct was not a dummy, it would be pushed here:
        // functions.push(_state_struct); 
        
        functions.push(HIRStatement::Function {
            name: format!("f_{}", func.name),
            params,
            _return_type: TejxType::Any,
            body: Box::new(wrapper_body),
        });
    }

    // This function generates the worker function and the body of the wrapper function for an async function.
    // It returns (worker_function_HIR, dummy_state_struct_HIR, wrapper_body_HIR_block).
    fn lower_async_function_impl(&self, name: &str, params: &Vec<(String, TejxType)>, _return_type_str: &str, body: &Statement) -> (HIRStatement, HIRStatement, HIRStatement) {
        let worker_name = format!("f_{}_worker", name);

        // --- Worker Function ---
        // any f_worker(any args_id) {
        //   let promise_id = args_id[0];
        //   let p1 = args_id[1]; ...
        //   try {
        //     let res = body;
        //     __resolve_promise(promise_id, res);
        //   } catch (e) {
        //     __reject_promise(promise_id, e);
        //   }
        // }
        
        self.enter_scope();
        let args_id_name = "args_id".to_string();
        self.define(args_id_name.clone(), TejxType::Any);

        let mut worker_body_stmts = Vec::new();

        // 1. Unpack promise_id
        let promise_id_expr = HIRExpression::IndexAccess {
            target: Box::new(HIRExpression::Variable { name: args_id_name.clone(), ty: TejxType::Any }),
            index: Box::new(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }),
            ty: TejxType::Any,
        };
        let promise_id_var = "promise_id_local".to_string();
        worker_body_stmts.push(HIRStatement::VarDecl {
            name: promise_id_var.clone(),
            initializer: Some(promise_id_expr),
            ty: TejxType::Any,
            _is_const: true,
        });
        self.define(promise_id_var.clone(), TejxType::Any);
        *self.current_async_promise_id.borrow_mut() = Some(promise_id_var.clone());

        // 2. Unpack params
        for (i, (pname, pty)) in params.iter().enumerate() {
            let unpack_expr = HIRExpression::IndexAccess {
                target: Box::new(HIRExpression::Variable { name: args_id_name.clone(), ty: TejxType::Any }),
                index: Box::new(HIRExpression::Literal { value: (i + 1).to_string(), ty: TejxType::Int32 }),
                ty: pty.clone(),
            };
            worker_body_stmts.push(HIRStatement::VarDecl {
                name: pname.clone(),
                initializer: Some(unpack_expr),
                ty: pty.clone(),
                _is_const: false,
            });
            self.define(pname.clone(), pty.clone());
        }

        // 3. Lower original body
        let inner_body = self.lower_statement(body)
            .unwrap_or(HIRStatement::Block { statements: vec![] });

        // 4. Wrap in Try/Catch
        let try_block = HIRStatement::Block {
            statements: vec![
                inner_body,
                // For simplicity, if the body doesn't return, we resolve with void/0.
                HIRStatement::ExpressionStmt {
                    expr: HIRExpression::Call {
                        callee: "__resolve_promise".to_string(),
                        args: vec![
                            HIRExpression::Variable { name: promise_id_var.clone(), ty: TejxType::Any },
                            HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Any },
                        ],
                        ty: TejxType::Void,
                    }
                },
                HIRStatement::ExpressionStmt {
                    expr: HIRExpression::Call {
                        callee: "tejx_dec_async_ops".to_string(),
                        args: vec![],
                        ty: TejxType::Void,
                    }
                }
            ]
        };

        let catch_block = HIRStatement::Block {
            statements: vec![
                HIRStatement::ExpressionStmt {
                    expr: HIRExpression::Call {
                        callee: "__reject_promise".to_string(),
                        args: vec![
                            HIRExpression::Variable { name: promise_id_var.clone(), ty: TejxType::Any },
                            HIRExpression::Variable { name: "err".to_string(), ty: TejxType::Any },
                        ],
                        ty: TejxType::Void,
                    }
                },
                HIRStatement::ExpressionStmt {
                    expr: HIRExpression::Call {
                        callee: "tejx_dec_async_ops".to_string(),
                        args: vec![],
                        ty: TejxType::Void,
                    }
                }
            ]
        };

        worker_body_stmts.push(HIRStatement::Try {
            try_block: Box::new(try_block),
            catch_var: Some("err".to_string()),
            catch_block: Box::new(catch_block),
            finally_block: None,
        });

        *self.current_async_promise_id.borrow_mut() = None;
        self._exit_scope();

        let worker_func = HIRStatement::Function {
            name: worker_name.clone(),
            params: vec![(args_id_name, TejxType::Any)],
            _return_type: TejxType::Any,
            body: Box::new(HIRStatement::Block { statements: worker_body_stmts }),
        };

        // --- Wrapper Body construction ---
        // This block assumes its parameters are already defined in the current scope.
        // let p = Promise_new();
        // let args = [p, x, y];
        // Thread_new(worker_ptr, args);
        // return p;
        
        let mut wrapper_stmts = Vec::new();
        let p_var = "p".to_string();
        wrapper_stmts.push(HIRStatement::VarDecl {
            name: p_var.clone(),
            initializer: Some(HIRExpression::Call {
                callee: "Promise_new".to_string(),
                args: vec![HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Int32 }],
                ty: TejxType::Any,
            }),
            ty: TejxType::Any,
            _is_const: false, // Can be const if not reassigned, but for simplicity, let's keep it mutable
        });

        // let args = [p, x, y];
        let mut args_elems = vec![HIRExpression::Variable { name: p_var.clone(), ty: TejxType::Any }];
        for (pname, pty) in params {
            args_elems.push(HIRExpression::Variable { name: pname.clone(), ty: pty.clone() });
        }

        let args_array = HIRExpression::ArrayLiteral {
            elements: args_elems,
            ty: TejxType::Class("any[]".to_string()), // Type of the array
        };

        let args_var = "args".to_string();
        wrapper_stmts.push(HIRStatement::VarDecl {
            name: args_var.clone(),
            initializer: Some(args_array),
            ty: TejxType::Class("any[]".to_string()),
            _is_const: false,
        });

        // tejx_inc_async_ops();
        wrapper_stmts.push(HIRStatement::ExpressionStmt {
            expr: HIRExpression::Call {
                callee: "tejx_inc_async_ops".to_string(),
                args: vec![],
                ty: TejxType::Void,
            }
        });

        // tejx_enqueue_task(worker_ptr, args);
        wrapper_stmts.push(HIRStatement::ExpressionStmt {
            expr: HIRExpression::Call {
                callee: "tejx_enqueue_task".to_string(),
                args: vec![
                    HIRExpression::Literal { value: format!("@{}", worker_name), ty: TejxType::Any }, // @ prefix for function pointer literal in our IR
                    HIRExpression::Variable { name: args_var, ty: TejxType::Any },
                ],
                ty: TejxType::Void,
            }
        });

        // return p;
        wrapper_stmts.push(HIRStatement::Return {
            value: Some(HIRExpression::Variable { name: p_var, ty: TejxType::Any }),
        });

        // Return the worker function, a dummy state struct, and the wrapper body block
        (worker_func, HIRStatement::Block { statements: vec![] }, HIRStatement::Block { statements: wrapper_stmts })
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
                         for (f_name, f_ty, f_init) in i_list {
                             let hir_init = self.lower_expression(f_init);
                             // Insert before other logic
                             statements.insert(0, HIRStatement::ExpressionStmt {
                                 expr: HIRExpression::Assignment {
                                     target: Box::new(HIRExpression::MemberAccess {
                                         target: Box::new(HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Class(class_decl.name.clone()) }),
                                         member: f_name.clone(),
                                         ty: f_ty.clone(),
                                     }),
                                     value: Box::new(hir_init),
                                     ty: TejxType::Any,
                                 },
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
                     statements.push(HIRStatement::ExpressionStmt {
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
                         statements.push(HIRStatement::ExpressionStmt {
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
                 self.enter_scope();
                 for (pname, pty) in &params {
                     self.define(pname.clone(), pty.clone());
                 }
                 
                 // We need to lower the body to HIR first to find captured vars (variables used in body)
                 // But lower_async_function does that.
                 // Let's call a modified version or duplicate logic. 
                 // Duplicating logic for now to ensure 'this' handling is correct.
                 
                 let mangled_name = if name.starts_with("f_") {
                     name.to_string()
                 } else {
                     format!("f_{}_{}", class_decl.name, name)
                 };
                 
                 let (worker_func, _state_struct, wrapper_body) = self.lower_async_function_impl(
                     &mangled_name, &params, &func_decl.return_type, &func_decl.body
                 );
                 
                 self._exit_scope();
                 
                 // Register the worker and state struct (which are global/top-level in HIR)
                 // But wait, lower_async_function_impl returns HIRStatement::Function for worker
                 // and HIRStatement::Struct for state.
                 // We should push them to 'functions' (which ends up in global HIR functions).
                 
                 // The wrapper body returned is what goes into the method body.
                 
                 functions.push(worker_func);
                 // functions.push(state_struct);
                 
                  functions.push(HIRStatement::Function {
                    name: mangled_name, params, _return_type: TejxType::Any, body: Box::new(wrapper_body),
                 });

            } else {
                 // Sync method
                 let hir_body = self.lower_statement(&func_decl.body)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                 
                 self._exit_scope();

                 let mangled_name = if name.starts_with("f_") {
                     name.to_string()
                 } else {
                     format!("f_{}_{}", class_decl.name.replace("[", "_").replace("]", "_"), name)
                 };

                 functions.push(HIRStatement::Function {
                    name: mangled_name, params, _return_type: return_type, body: Box::new(hir_body),
                 });
            }
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
            for (f_name, f_ty, f_init) in s_list {
                let hir_init = self.lower_expression(f_init);
                // Static fields are mangled as g_Class_Field
                let mangled_name = format!("g_{}_{}", class_decl.name, f_name);
                main_stmts.push(HIRStatement::ExpressionStmt {
                    expr: HIRExpression::Assignment {
                        target: Box::new(HIRExpression::Variable { name: mangled_name, ty: f_ty.clone() }),
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
            
            let name = if func_decl.name.starts_with("f_") {
                func_decl.name.clone()
            } else {
                format!("f_{}_{}", ext_decl._target_type.replace("[", "_").replace("]", "_"), func_decl.name)
            };
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
                if let Some(p_id) = self.current_async_promise_id.borrow().as_ref() {
                    let mut stmts = Vec::new();
                    stmts.push(HIRStatement::ExpressionStmt {
                        expr: HIRExpression::Call {
                            callee: "__resolve_promise".to_string(),
                            args: vec![
                                HIRExpression::Variable { name: p_id.clone(), ty: TejxType::Any },
                                val.clone().unwrap_or(HIRExpression::Literal { value: "0".to_string(), ty: TejxType::Any }),
                            ],
                            ty: TejxType::Void,
                        }
                    });
                    stmts.push(HIRStatement::ExpressionStmt {
                        expr: HIRExpression::Call {
                            callee: "tejx_dec_async_ops".to_string(),
                            args: vec![],
                            ty: TejxType::Void,
                        }
                    });
                    stmts.push(HIRStatement::Return { value: val });
                    Some(HIRStatement::Block { statements: stmts })
                } else {
                    Some(HIRStatement::Return { value: val })
                }
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
            Statement::TryStmt { _try_block, _catch_var, _catch_block, _finally_block, .. } => {
                let try_hir = self.lower_statement(_try_block)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                let catch_hir = self.lower_statement(_catch_block)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                let finally_hir = _finally_block.as_ref()
                    .and_then(|f| self.lower_statement(f));
                
                Some(HIRStatement::Try {
                    try_block: Box::new(try_hir),
                    catch_var: if _catch_var.is_empty() { None } else { Some(_catch_var.clone()) },
                    catch_block: Box::new(catch_hir),
                    finally_block: finally_hir.map(Box::new),
                })
            }
            Statement::ThrowStmt { _expression, .. } => {
                let val = self.lower_expression(_expression);
                Some(HIRStatement::Throw { value: val })
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
                let ty = self.lookup(name).map(|(_, t)| t).unwrap_or(TejxType::Any);
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
                         HIRExpression::Call {
                             callee: "rt_not".to_string(),
                             args: vec![self.lower_expression(right)],
                             ty: TejxType::Bool,
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
                // Check for super.method()
                if let Expression::MemberAccessExpr { object, member, .. } = callee.as_ref() {
                    if let Expression::SuperExpr { .. } = object.as_ref() {
                         // super.method(...)
                         // Resolve parent class
                         if let Some(parent) = self.parent_class.borrow().as_ref() {
                             let method_name = format!("f_{}_{}", parent, member);
                             
                             let mut hir_args: Vec<HIRExpression> = args.iter()
                                .map(|a| self.lower_expression(a))
                                .collect();
                             
                             // Prepend 'this'
                             // 'this' in class method context is available as local var "this"
                             // (defined in lower_class_declaration or lower_async_function)
                             let this_ty = if let Some(cls) = self.current_class.borrow().as_ref() {
                                 TejxType::Class(cls.clone()) 
                             } else {
                                 TejxType::Any
                             };
                             hir_args.insert(0, HIRExpression::Variable { name: "this".to_string(), ty: this_ty });

                             return HIRExpression::Call {
                                 callee: method_name,
                                 args: hir_args,
                                 ty: TejxType::Any, // Inferred?
                             };
                         } else {
                             // Error: super used but no parent?
                             // Fallthrough or return dummy
                         }
                    }
                }


                
                // Check for obj.method() where obj is a class instance
                if let Expression::MemberAccessExpr { object, member, .. } = callee.as_ref() {
                     let obj_hir = self.lower_expression(object);
                     let obj_ty = obj_hir.get_type();
                     
                     if let TejxType::Class(class_name) = obj_ty {
                         let clean_class = if let Some(pos) = class_name.find('<') { class_name[..pos].to_string() } else { class_name.clone() };
                         let prefix = match clean_class.as_str() {
                             "Stack" | "Queue" | "PriorityQueue" | "MinHeap" | "MaxHeap" | "Map" | "Set" | 
                             "OrderedMap" | "OrderedSet" | "BloomFilter" | "Trie" => "rt",
                             _ => "f",
                         };
                         let method_name = format!("{}_{}_{}", prefix, clean_class.replace("[", "_").replace("]", "_"), member);
                         // Verify method exists or just assume? 
                         // Check user_functions directly
                         let return_ty = self.user_functions.borrow().get(&method_name).cloned().unwrap_or(TejxType::Any);
                         
                         let mut hir_args: Vec<HIRExpression> = args.iter()
                            .map(|a| self.lower_expression(a))
                            .collect();
                         
                         // Prepend object as 'this'
                         hir_args.insert(0, obj_hir);
                         
                         return HIRExpression::Call {
                             callee: method_name,
                             args: hir_args,
                             ty: return_ty,
                         };
                     }
                }

                let hir_args: Vec<HIRExpression> = args.iter()
                    .map(|a| self.lower_expression(a))
                    .collect();
                
                let callee_str = callee.to_callee_name();
                
                // If simple name resolution failed (e.g. strict indirect call), we might still want to try?
                // But for now, reliance on string name implies we expect simple callees for builtins.
                
                let normalized = callee_str.replace('.', "_").replace("::", "_").replace(":", "_");
                let mut final_callee = normalized.clone();
                let mut final_args = hir_args.clone();
                let mut ty = TejxType::Any;

                if callee_str == "typeof" {
                    let r_expr = hir_args[0].clone();
                    let r_ty = r_expr.get_type();
                    if matches!(r_ty, TejxType::Any) {
                        return HIRExpression::Call {
                            callee: "rt_typeof".to_string(),
                            args: vec![r_expr],
                            ty: TejxType::Any,
                        };
                    } else {
                        let type_name = match &r_ty {
                            TejxType::Int32 | TejxType::Int16 | TejxType::Int64 | TejxType::Int128 => "number",
                            TejxType::Float32 | TejxType::Float16 | TejxType::Float64 => "number",
                            TejxType::Bool => "boolean",
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
                } else if callee_str == "sizeof" {
                    let r_expr = hir_args[0].clone();
                    let r_ty = r_expr.get_type();
                    return HIRExpression::Literal {
                        value: r_ty.size().to_string(),
                        ty: TejxType::Int32,
                    };
                }

                if callee_str == "super" {
                    if let Some(parent) = &*self.parent_class.borrow() {
                        final_callee = format!("f_{}_constructor", parent);
                        final_args = vec![HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }];
                        final_args.extend(hir_args.clone());
                    }
                } else if let Expression::MemberAccessExpr { object, member, .. } = callee.as_ref() {
                     if let Expression::SuperExpr { .. } = object.as_ref() {
                         if let Some(parent) = &*self.parent_class.borrow() {
                             final_callee = format!("f_{}_{}", parent, member);
                             final_args = vec![HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }];
                             final_args.extend(hir_args.clone());
                         }
                     } else if let Some(ret_ty) = self.user_functions.borrow().get(&callee_str) {
                         final_callee = if callee_str == "main" { "f_main".to_string() } else { format!("f_{}", callee_str) };
                         ty = ret_ty.clone();
                     } else {
                         // Check stdlib imports for MemberAccess (e.g. fs.readFile)
                         let imports = self.std_imports.borrow();
                         let mut found_import = false;
                         for (mod_name, mode) in imports.iter() {
                             let is_imported = match mode {
                                 ImportMode::All => {
                                      if callee_str.contains('.') {
                                          let parts: Vec<&str> = callee_str.split('.').collect();
                                          if parts[0].eq_ignore_ascii_case(mod_name) {
                                              self.stdlib.is_std_func(mod_name, parts[1])
                                          } else { false }
                                      } else { false }
                                 }
                                 ImportMode::Named(_set) => {
                                     // Not relevant for dot access usually, unless aliased?
                                     // But if we have `import { readFile } from fs`, we call `readFile`, not `fs.readFile`.
                                     false
                                 }
                             };
                             
                             if is_imported {
                                 found_import = true;
                                 let parts: Vec<&str> = callee_str.split('.').collect();
                                 let func_name = parts[1];
                                 final_callee = self.stdlib.get_runtime_name(mod_name, func_name);
                                 break;
                             }
                         }
                         drop(imports);
                         
                         // If not a stdlib import, handle runtime method calls
                         // by prepending the object as the first argument
                         if !found_import {
                             // First: check if this.method() inside a class context
                             let is_this = matches!(object.as_ref(), Expression::Identifier { name, .. } if name == "this");
                             let mut did_resolve = false;
                             
                             if is_this {
                                 let class_opt = self.current_class.borrow().clone();
                                 if let Some(cn) = class_opt {
                                     let stripped = if let Some(pos) = cn.find('<') { cn[..pos].to_string() } else { cn.clone() };
                                     let method_key = format!("{}_{}", stripped, member);
                                     if self.user_functions.borrow().contains_key(&method_key) {
                                         final_callee = format!("f_{}", method_key);
                                         let obj_hir = self.lower_expression(object);
                                         let mut n_args = vec![obj_hir];
                                         n_args.extend(hir_args.clone());
                                         final_args = n_args;
                                         did_resolve = true;
                                     }
                                 }
                             }
                             
                             // Second: runtime method dispatch (expanded)
                             if !did_resolve {
                                 let obj_hir = self.lower_expression(object);
                                 let runtime_callee = match member.as_str() {
                                     "lock" | "unlock" | "acquire" | "release" => {
                                         let base = match member.as_str() {
                                             "lock" | "acquire" => "m_lock",
                                             _ => "m_unlock",
                                         };
                                         base.to_string()
                                     },
                                     "join" => "Thread_join".to_string(),
                                     "sleep" => "Thread_sleep".to_string(),
                                     "then" | "catch" => format!("Promise_{}", member),
                                     "push" | "unshift" | "fill" => format!("Array_{}", member),
                                     "pop" | "shift" => format!("Array_{}", member),
                                     "forEach" => "Array_forEach".to_string(),
                                     "map" | "filter" | "reduce" | "find" | "findIndex" | "reverse" | "splice" | "sort" | "flat" => format!("Array_{}", member),
                                     "concat" | "slice" | "indexOf" | "includes" => {
                                         if obj_hir.get_type() == TejxType::String {
                                             format!("rt_string_{}", member)
                                         } else {
                                             format!("Array_{}", member)
                                         }
                                     },
                                     "get" => "Map_get".to_string(),
                                     "set" | "put" => "Map_put".to_string(),
                                     "has" => "Collection_has".to_string(),
                                     "delete" | "del" => "Collection_delete".to_string(),
                                     "size" => "rt_collections_size".to_string(),
                                     "clear" => "Collection_clear".to_string(),
                                     "add" => "Collection_add".to_string(),
                                     "keys" => "Collection_keys".to_string(),
                                     "values" => "Collection_values".to_string(),
                                     "entries" => "Collection_entries".to_string(),
                                     "enqueue" | "dequeue" | "peek" | "isEmpty" | "extractMin" | "extractMax" | "insertMin" | "insertMax" | "insert" => {
                                         // Collection class methods - try user function lookup
                                         let obj_ty_name = obj_hir.get_type();
                                         let class_name = match &obj_ty_name {
                                             TejxType::Class(cn) => {
                                                 if let Some(pos) = cn.find('<') { cn[..pos].to_string() } else { cn.clone() }
                                             }
                                             _ => String::new(),
                                         };
                                         if !class_name.is_empty() {
                                             let key = format!("{}_{}", class_name, member);
                                             let prefix = match class_name.as_str() {
                                                 "Stack" | "Queue" | "PriorityQueue" | "MinHeap" | "MaxHeap" | "Map" | "Set" | 
                                                 "OrderedMap" | "OrderedSet" | "BloomFilter" | "Trie" => "rt",
                                                 _ => "f",
                                             };
                                             if self.user_functions.borrow().contains_key(&key) || prefix == "rt" {
                                                 format!("{}_{}", prefix, key)
                                             } else { String::new() }
                                         } else { String::new() }
                                     },
                                     "trim" => "rt_string_trim".to_string(),
                                     "toLowerCase" => "rt_string_to_lower".to_string(),
                                     "toUpperCase" => "rt_string_to_upper".to_string(),
                                     "startsWith" => "rt_string_starts_with".to_string(),
                                     "endsWith" => "rt_string_ends_with".to_string(),
                                     "replace" => "rt_string_replace".to_string(),
                                     "padStart" => "s_padStart".to_string(),
                                     "padEnd" => "s_padEnd".to_string(),
                                     "repeat" => "s_repeat".to_string(),
                                     "trimStart" => "s_trimStart".to_string(),
                                     "trimEnd" => "s_trimEnd".to_string(),
                                     "getTime" => "d_getTime".to_string(),
                                     "toISOString" => "d_toISOString".to_string(),
                                     "toString" => "e_toString".to_string(),
                                     "describe" => "n_describe".to_string(),
                                     _ => String::new(),
                                 };
                                 
                                 if !runtime_callee.is_empty() {
                                     final_callee = runtime_callee;
                                     
                                     let is_static_object = match object.as_ref() {
                                         Expression::Identifier { name, .. } => name == "Object",
                                         _ => false,
                                     };
                                     
                                     if is_static_object && matches!(member.as_str(), "keys" | "values" | "entries") {
                                         final_args = hir_args.clone();
                                     } else {
                                         let mut n_args = vec![obj_hir];
                                         n_args.extend(hir_args.clone());
                                         final_args = n_args;
                                     }
                                 }
                             }
                         }
                     }
                } else if let Some(ret_ty) = self.user_functions.borrow().get(&callee_str) {
                    final_callee = if callee_str == "main" { "tejx_main".to_string() } else { format!("f_{}", callee_str) };
                    ty = ret_ty.clone();
                } else if self.stdlib.is_prelude_func(&normalized) || normalized == "log" || normalized == "console_log" {
                    final_callee = if normalized == "log" || normalized == "console_log" { "print".to_string() } else { normalized.clone() };
                } else {
                    // Check stdlib imports
                    let imports = self.std_imports.borrow();
                    let mut found_std = false;
                    for (mod_name, mode) in imports.iter() {
                        let is_imported = match mode {
                            ImportMode::All => {
                                if callee_str.contains('.') {
                                    let parts: Vec<&str> = callee_str.split('.').collect();
                                    if parts[0].eq_ignore_ascii_case(mod_name) {
                                        self.stdlib.is_std_func(mod_name, parts[1])
                                    } else if mod_name == "net" && (parts[0] == "http" || parts[0] == "https") {
                                        self.stdlib.is_std_func(parts[0], parts[1])
                                    } else {
                                        false
                                    }
                                } else {
                                    self.stdlib.is_std_func(mod_name, &callee_str)
                                }
                            }
                            ImportMode::Named(set) => {
                                if callee_str.contains('.') {
                                    let parts: Vec<&str> = callee_str.split('.').collect();
                                    if parts[0].eq_ignore_ascii_case(mod_name) {
                                        set.contains(parts[1])
                                    } else if mod_name == "net" && (parts[0] == "http" || parts[0] == "https") {
                                        // For named imports, we might need more logic, but for now allow strict match if namespaced
                                        true 
                                    } else {
                                        false
                                    }
                                } else {
                                    set.contains(&callee_str)
                                }
                            }
                        };
                        
                        if is_imported {
                            found_std = true;
                            let func_name = if callee_str.contains('.') {
                                callee_str.split('.').collect::<Vec<&str>>()[1]
                            } else {
                                &callee_str
                            };
                            
                            let final_mod_name = if mod_name == "net" && callee_str.contains('.') {
                                let parts: Vec<&str> = callee_str.split('.').collect();
                                if parts[0] == "http" || parts[0] == "https" {
                                    parts[0]
                                } else {
                                    mod_name
                                }
                            } else {
                                mod_name
                            };

                            final_callee = self.stdlib.get_runtime_name(final_mod_name, func_name);
                            if mod_name == "math" {
                                ty = TejxType::Any;
                            }
                            break;
                        }
                    }

                    if !found_std && callee_str.contains('.') && 
                       !callee_str.starts_with("Math.") && 
                       !callee_str.starts_with("fs.") && !callee_str.starts_with("Date.") && 
                       !callee_str.starts_with("http.") &&
                       !callee_str.starts_with("Promise.") && !callee_str.starts_with("Array.") {
                        
                        let parts: Vec<&str> = callee_str.split('.').collect();
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
                                // Resolve nested members (e.g. this.map.get)
                                let mut resolved_type = None;
                                if let Some((_, ty)) = self.lookup(parts[0]) {
                                    resolved_type = Some(ty);
                                }
                                
                                // Iterate intermediate parts
                                if let Some(mut current_ty) = resolved_type.clone() {
                                    for i in 1..parts.len()-1 {
                                        if let TejxType::Class(current_class) = current_ty {
                                            if let Some(fields) = self.class_instance_fields.borrow().get(&current_class) {
                                                if let Some((_, field_ty, _)) = fields.iter().find(|(name, _, _)| name == parts[i]) {
                                                    current_ty = field_ty.clone();
                                                    resolved_type = Some(current_ty.clone());
                                                    continue;
                                                }
                                            }
                                        }
                                        resolved_type = None; // Failed to resolve path
                                        break;
                                    }
                                }

                                if let Some(ty) = resolved_type {
                                    is_instance = true;
                                    if let TejxType::Class(cn) = ty {
                                        class_name_opt = Some(cn);
                                    } else if ty.is_array() {
                                        class_name_opt = Some("Array".to_string());
                                    } else if matches!(ty, TejxType::String) {
                                        class_name_opt = Some("String".to_string());
                                    }
                                } else if self.class_methods.borrow().contains_key(parts[0]) {
                                    class_name_opt = Some(parts[0].to_string());
                                } else if parts[0].starts_with("$new_") {
                                    class_name_opt = Some(parts[0][5..].to_string());
                                    is_instance = true;
                                } else if parts[0].starts_with("(string)") {
                                    class_name_opt = Some("String".to_string());
                                    is_instance = true;
                                } else if parts[0].starts_with("(array)") {
                                    class_name_opt = Some("Array".to_string());
                                    is_instance = true;
                                }

                                 if let Some(ref mut class_name) = class_name_opt {
                                    if let Some(pos) = class_name.find('<') {
                                        *class_name = class_name[..pos].to_string();
                                    }
                                    let mangled_key = format!("{}_{}", class_name, method_name);
                                    if self.user_functions.borrow().contains_key(&mangled_key) {
                                        let is_std_collection = ["Stack", "Queue", "PriorityQueue", "MinHeap", "MaxHeap", "Map", "Set", "OrderedMap", "OrderedSet", "BloomFilter", "Trie"]
                                            .contains(&class_name.as_str());
                                        if is_std_collection {
                                            final_callee = format!("f_{}", mangled_key);
                                        } else {
                                            final_callee = format!("f_{}", mangled_key);
                                        }
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
                                      let obj_expr = if let Expression::MemberAccessExpr { object, .. } = callee.as_ref() {
                                           self.lower_expression(object)
                                      } else if obj_path == "this" || obj_path == "super" {
                                          HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }
                                      } else if obj_path.contains('.') {
                                          // ... existing complex path logic ...
                                          // Actually, let's keep the existing logic but ensure it handles literals
                                          let sub_parts: Vec<&str> = obj_path.split('.').collect();
                                          let mut current = if sub_parts[0] == "this" {
                                              HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }
                                          } else if sub_parts[0] == "(string)" {
                                               // This is a literal that was mangled by to_callee_name
                                               // But wait, if it's a literal, we don't have the value here.
                                               // The best way is to look at the AST object.
                                               if let Expression::MemberAccessExpr { object, .. } = callee.as_ref() {
                                                   self.lower_expression(object)
                                               } else {
                                                   HIRExpression::Variable { name: "this".to_string(), ty: TejxType::Any }
                                               }
                                          } else {
                                              let base_ty = self.lookup(sub_parts[0]).map(|(_, t)| t).unwrap_or(TejxType::Any);
                                              HIRExpression::Variable { name: sub_parts[0].to_string(), ty: base_ty }
                                          };
                                          // ... rest of loop ...
                                          for idx in 1..sub_parts.len() {
                                              let mut next_ty = TejxType::Any;
                                              if let TejxType::Class(class_name) = current.get_type() {
                                                  let fields = self.class_instance_fields.borrow();
                                                  if let Some(i_list) = fields.get(&class_name) {
                                                      for (f_name, f_ty, _) in i_list {
                                                          if f_name == sub_parts[idx] {
                                                              next_ty = f_ty.clone();
                                                              break;
                                                          }
                                                      }
                                                  }
                                              }
                                              current = HIRExpression::MemberAccess {
                                                  target: Box::new(current),
                                                  member: sub_parts[idx].to_string(),
                                                  ty: next_ty,
                                              };
                                          }
                                          current
                                      } else {
                                          if let Expression::MemberAccessExpr { object, .. } = callee.as_ref() {
                                              self.lower_expression(object)
                                          } else {
                                              let ty = self.lookup(&obj_path).map(|(_, t)| t).unwrap_or(TejxType::Any);
                                              HIRExpression::Variable { name: obj_path.clone(), ty }
                                          }
                                      };

                                     let (runtime_callee, _ret_type) = match method_name.as_str() {
                                         "push" | "unshift" | "fill" => (format!("Array_{}", method_name), TejxType::Int32),
                                         "pop" | "shift" => (format!("Array_{}", method_name), TejxType::Any), 
                                         "join" => ("__join".to_string(), TejxType::Any),
                                         "lock" | "unlock" => (format!("m_{}", method_name), TejxType::Any),
                                         "map" | "filter" | "reduce" | "find" | "findIndex" | "reverse" | "splice" | "sort" | "flat" => (format!("Array_{}", method_name), TejxType::Any),
                                         "concat" | "slice" | "indexOf" | "includes" => {
                                             let base_ty = class_name_opt.as_deref().unwrap_or("any");
                                             let base_is_array = base_ty == "Array" || base_ty.ends_with("[]") || base_ty.contains("any[]");
                                             if base_is_array {
                                                 (format!("Array_{}", method_name), TejxType::Any)
                                             } else {
                                                 (format!("trimmed_{}", method_name), TejxType::Any)
                                             }
                                         },
                                         "forEach" => (format!("Array_{}", method_name), TejxType::Void),
                                         "set" | "put" => ("Map_put".to_string(), TejxType::Any), // Map.put
                                         "get" => ("Map_get".to_string(), TejxType::Any),
                                         "has" => ("Collection_has".to_string(), TejxType::Any),
                                         "delete" | "del" => ("Collection_delete".to_string(), TejxType::Any),
                                         "size" => ("rt_collections_size".to_string(), TejxType::Any),
                                         "clear" => ("Collection_clear".to_string(), TejxType::Any),
                                         "add" => {
                                             if class_name_opt.as_deref() == Some("Atomic") {
                                                  ("rt_atomic_add".to_string(), TejxType::Int32)
                                             } else if let Some(cn) = class_name_opt.as_deref() {
                                                  if cn == "Set" {
                                                      ("Collection_add".to_string(), TejxType::Any)
                                                  } else {
                                                      (format!("f_{}_add", cn), TejxType::Any)
                                                  }
                                             } else {
                                                  ("Collection_add".to_string(), TejxType::Any)
                                             }
                                         },
                                         "sub" if class_name_opt.as_deref() == Some("Atomic") => ("rt_atomic_sub".to_string(), TejxType::Int32),
                                         "load" if class_name_opt.as_deref() == Some("Atomic") => ("rt_atomic_load".to_string(), TejxType::Int32),
                                         "store" if class_name_opt.as_deref() == Some("Atomic") => ("rt_atomic_store".to_string(), TejxType::Int32),
                                         "exchange" if class_name_opt.as_deref() == Some("Atomic") => ("rt_atomic_exchange".to_string(), TejxType::Int32),
                                         "compareExchange" if class_name_opt.as_deref() == Some("Atomic") => ("rt_atomic_compare_exchange".to_string(), TejxType::Int32),
                                         "wait" if class_name_opt.as_deref() == Some("Condition") => ("rt_cond_wait".to_string(), TejxType::Int32),
                                         "notify" if class_name_opt.as_deref() == Some("Condition") => ("rt_cond_notify".to_string(), TejxType::Int32),
                                         "notifyAll" if class_name_opt.as_deref() == Some("Condition") => ("rt_cond_notify_all".to_string(), TejxType::Int32),
                                         "trim" => ("rt_string_trim".to_string(), TejxType::Any),
                                         "padStart" => ("trimmed_padStart".to_string(), TejxType::Any),
                                         "padEnd" => ("trimmed_padEnd".to_string(), TejxType::Any),
                                         "repeat" => ("trimmed_repeat".to_string(), TejxType::Any),
                                         "trimStart" => ("trimmed_trimStart".to_string(), TejxType::Any),
                                         "trimEnd" => ("trimmed_trimEnd".to_string(), TejxType::Any),
                                         "keys" => {
                                             let is_static_obj = if let HIRExpression::Variable { name, .. } = &obj_expr {
                                                 name == "Object"
                                             } else { false };
                                             if is_static_obj {
                                                 ("Object_keys".to_string(), TejxType::Any)
                                             } else {
                                                 ("Collection_keys".to_string(), TejxType::Any)
                                             }
                                         },
                                         "values" => {
                                             let is_static_obj = if let HIRExpression::Variable { name, .. } = &obj_expr {
                                                 name == "Object"
                                             } else { false };
                                             if is_static_obj {
                                                 ("Object_values".to_string(), TejxType::Any)
                                             } else {
                                                 ("Collection_values".to_string(), TejxType::Any)
                                             }
                                         },
                                         "entries" => {
                                             let is_static_obj = if let HIRExpression::Variable { name, .. } = &obj_expr {
                                                 name == "Object"
                                             } else { false };
                                             if is_static_obj {
                                                 ("Object_entries".to_string(), TejxType::Any)
                                             } else {
                                                 ("Collection_entries".to_string(), TejxType::Any)
                                             }
                                         },
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
                        if let Some((_name, f_ty, _)) = f_list.iter().find(|(n, _, _)| n == member) {
                            return HIRExpression::Variable {
                                name: format!("g_{}_{}", obj_name, member),
                                ty: f_ty.clone(),
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

                // Field Resolution
                let lowered_object = self.lower_expression(object);
                let obj_ty = lowered_object.get_type();
                if let TejxType::Class(class_name) = obj_ty {
                    let fields = self.class_instance_fields.borrow();
                    if let Some(i_list) = fields.get(&class_name) {
                        for (f_name, f_ty, _) in i_list {
                            if f_name == member {
                                return HIRExpression::MemberAccess {
                                    target: Box::new(lowered_object),
                                    member: member.clone(),
                                    ty: f_ty.clone(),
                                };
                            }
                        }
                    }
                    // Static field? (If object name matches class name, handled differently usually, but let's check)
                }

                let combined = format!("{}_{}", obj_name, member);
                if self.user_functions.borrow().contains_key(&combined) {
                    HIRExpression::Variable {
                        name: format!("f_{}", combined),
                        ty: TejxType::Any,
                    }
                } else {
                    HIRExpression::MemberAccess {
                        target: Box::new(lowered_object),
                        member: member.clone(),
                        ty: if member == "length" { TejxType::Int32 } else { TejxType::Any },
                    }
                }
            }
            Expression::ArrayAccessExpr { target, index, .. } => {
                let lowered_target = self.lower_expression(target);
                let ty = lowered_target.get_type().get_array_element_type();
                HIRExpression::IndexAccess {
                    target: Box::new(lowered_target),
                    index: Box::new(self.lower_expression(index)),
                    ty,
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
                                ty: TejxType::Class("any[]".to_string()),
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
                        ty: TejxType::Class("any[]".to_string()),
                    });
                }
                
                if chunks.is_empty() {
                     // Empty array []
                     HIRExpression::ArrayLiteral { elements: vec![], ty: TejxType::Class("any[]".to_string()) }
                } else {
                    // Reduce chunks with Array_concat
                    let mut expr = chunks[0].clone();
                    for next_chunk in chunks.into_iter().skip(1) {
                         expr = HIRExpression::Call {
                             callee: "Array_concat".to_string(), // Ensure this maps to Array_concat in runtime
                             args: vec![expr, next_chunk],
                             ty: TejxType::Class("any[]".to_string()),
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
                
                // Enforce Any type for lambda parameters to handle boxed values from indirect calls
                let hir_params: Vec<(String, TejxType)> = params.iter()
                    .map(|p| (p.name.clone(), if p.type_name.is_empty() { TejxType::Any } else { TejxType::from_name(&p.type_name) }))
                    .collect();
                
                // CRITICAL: Enter a new scope for the lambda body and define parameters
                self.enter_scope();
                for (name, ty) in &hir_params {
                    self.define(name.clone(), ty.clone());
                }
                
                let hir_body = self.lower_statement(body)
                    .unwrap_or(HIRStatement::Block { statements: vec![] });
                
                self._exit_scope();
                
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
                
                let mut normalized_class = class_name.clone();
                if let Some(pos) = normalized_class.find('<') {
                    normalized_class = normalized_class[..pos].to_string();
                }

                // Variadic check for constructor
                let cons_unmangled = format!("{}_constructor", normalized_class);
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
                    class_name: normalized_class, // Keep full name for type info? Or normalized? 
                    // Actually, HIR NewExpr's class_name is used in mir_lowering to pick constructor.
                    // But we might need the full name for type inference elsewhere.
                    // Let's use normalized_class for the string used in matching.
                    // WAIT! HIR NewExpr's class_name is used in MIRLowering::lower_expression (line 533-548).
                    // If I change it to normalized_class, MIRLowering will see "Stack".
                    // That's exactly what it needs!
                    _args: hir_args,
                }
            }
            Expression::OptionalCallExpr { callee, args: _args, .. } => {
                let callee_expr = self.lower_expression(callee);

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
