pub mod async_desugar;
pub mod class;
pub mod expr;
pub mod func;
pub mod imports;
pub mod patterns;
pub mod stmt;

use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::hir::*;
use crate::intrinsics::*;
use crate::types::TejxType;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;

pub struct Lowering {
    lambda_counter: RefCell<usize>,
    user_functions: RefCell<HashMap<String, TejxType>>,
    user_function_args: RefCell<HashMap<String, usize>>,
    extern_functions: RefCell<HashSet<String>>,
    variadic_functions: RefCell<HashMap<String, usize>>,
    lambda_functions: RefCell<Vec<HIRStatement>>,
    nested_functions: RefCell<Vec<HIRStatement>>,
    scopes: RefCell<Vec<(HashMap<String, (String, TejxType)>, usize)>>, // (name -> (mangled_name, type), lambda_depth)
    lambda_depth: RefCell<usize>,
    next_scope_id: RefCell<usize>,
    current_class: RefCell<Option<String>>,
    parent_class: RefCell<Option<String>>,
    class_methods: RefCell<HashMap<String, Vec<String>>>,
    class_instance_fields: RefCell<HashMap<String, Vec<(String, TejxType, Expression)>>>,
    class_static_fields: RefCell<HashMap<String, Vec<(String, TejxType, Expression)>>>,
    class_getters: RefCell<HashMap<String, HashSet<String>>>,
    class_setters: RefCell<HashMap<String, HashSet<String>>>,
    class_parents: RefCell<HashMap<String, String>>,
    class_generic_params: RefCell<HashMap<String, Vec<String>>>,
    function_generic_params: RefCell<HashMap<String, Vec<String>>>,
    pub async_enabled: bool,
    current_async_promise_id: RefCell<Option<String>>,
    pub diagnostics: RefCell<Vec<Diagnostic>>,
    pub filename: RefCell<String>,
    pub captured_vars: RefCell<HashSet<String>>,
    pub stdlib_path: RefCell<std::path::PathBuf>,
    pub lambda_inferred_types: HashMap<(usize, usize), Vec<TejxType>>,
    current_return_type: RefCell<Option<TejxType>>,
    pub generic_instantiations: HashMap<String, std::collections::HashSet<Vec<TejxType>>>,
    pub function_instantiations: HashMap<String, std::collections::HashSet<Vec<TejxType>>>,
}

/// Result of lowering: a list of top-level HIR functions.
/// The last one is always "tejx_main" containing non-function statements.
pub struct LoweringResult {
    pub functions: Vec<HIRStatement>, // Each should be HIRStatement::Function
    pub signatures: HashMap<String, Vec<TejxType>>,
    pub captured_vars: HashSet<String>,
    pub class_fields: HashMap<String, Vec<(String, TejxType)>>,
    pub class_methods: HashMap<String, Vec<String>>,
}

impl Lowering {
    pub fn new() -> Self {
        Lowering {
            lambda_counter: RefCell::new(0),
            user_functions: RefCell::new(HashMap::new()),
            user_function_args: RefCell::new(HashMap::new()),
            extern_functions: RefCell::new(HashSet::new()),
            variadic_functions: RefCell::new(HashMap::new()),
            lambda_functions: RefCell::new(Vec::new()),
            nested_functions: RefCell::new(Vec::new()),
            scopes: RefCell::new(vec![(HashMap::new(), 0)]), // Global scope at lambda_depth=0
            lambda_depth: RefCell::new(0),
            next_scope_id: RefCell::new(1), // Global is 0
            current_class: RefCell::new(None),
            parent_class: RefCell::new(None),
            class_methods: RefCell::new(HashMap::new()),
            class_instance_fields: RefCell::new(HashMap::new()),
            class_static_fields: RefCell::new(HashMap::new()),
            class_getters: RefCell::new(HashMap::new()),
            class_setters: RefCell::new(HashMap::new()),
            class_parents: RefCell::new(HashMap::new()),
            class_generic_params: RefCell::new(HashMap::new()),
            function_generic_params: RefCell::new(HashMap::new()),
            async_enabled: false,
            current_async_promise_id: RefCell::new(None),
            diagnostics: RefCell::new(Vec::new()),
            filename: RefCell::new(String::new()),
            captured_vars: RefCell::new(HashSet::new()),
            stdlib_path: RefCell::new(std::path::PathBuf::from("stdlib")),
            lambda_inferred_types: HashMap::new(),
            current_return_type: RefCell::new(None),
            generic_instantiations: HashMap::new(),
            function_instantiations: HashMap::new(),
        }
    }

    pub(crate) fn enter_scope(&self) {
        let depth = *self.lambda_depth.borrow();
        self.scopes.borrow_mut().push((HashMap::new(), depth));
    }

    pub(crate) fn enter_lambda_scope(&self) {
        *self.lambda_depth.borrow_mut() += 1;
        let depth = *self.lambda_depth.borrow();
        self.scopes.borrow_mut().push((HashMap::new(), depth));
    }

    pub(crate) fn _exit_scope(&self) {
        let popped = self.scopes.borrow_mut().pop();
        // If we're exiting a lambda scope, restore the lambda_depth
        if let Some((_, scope_depth)) = popped {
            let current = *self.lambda_depth.borrow();
            if scope_depth > 0 && scope_depth == current {
                // Check if any remaining scope has this depth
                let scopes = self.scopes.borrow();
                let any_at_depth = scopes.iter().any(|(_, d)| *d == scope_depth);
                if !any_at_depth {
                    *self.lambda_depth.borrow_mut() = scope_depth - 1;
                }
            }
        }
    }

    pub(crate) fn define(&self, name: String, ty: TejxType) -> String {
        let depth = self.scopes.borrow().len() - 1;
        let mangled = if depth == 0 {
            format!("g_{}", name)
        } else {
            // Include a unique ID per scope to avoid collisions between siblings
            let mut id = self.next_scope_id.borrow_mut();
            let mangled = format!("{}${}", name, *id);
            *id += 1;
            mangled
        };

        if let Some((scope, _)) = self.scopes.borrow_mut().last_mut() {
            scope.insert(name, (mangled.clone(), ty));
        }
        mangled
    }

    pub(crate) fn narrow_type(&self, name: String, ty: TejxType) {
        if let Some((mangled, _)) = self.lookup(&name) {
            if let Some((scope, _)) = self.scopes.borrow_mut().last_mut() {
                scope.insert(name, (mangled, ty));
            }
        }
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<(String, TejxType)> {
        let scopes = self.scopes.borrow();
        let current_lambda_depth = scopes.last().map(|(_, d)| *d).unwrap_or(0);
        for (i, (scope, scope_lambda_depth)) in scopes.iter().enumerate().rev() {
            if let Some(info) = scope.get(name) {
                let mangled = &info.0;
                // Only capture if accessing across a lambda boundary
                // (not from simple nested blocks like for-loops/if-blocks)
                if *scope_lambda_depth < current_lambda_depth && i > 0 {
                    self.captured_vars.borrow_mut().insert(mangled.clone());
                }
                return Some(info.clone());
            }
        }
        None
    }

    pub fn lower(&self, program: &Program, _base_path: &std::path::Path) -> LoweringResult {
        let line = 0; // Top level
        let mut functions = Vec::new();
        let mut main_stmts = Vec::new();
        let mut merged_statements = program.statements.clone();

        // Pass 0.5: Scan for Variadic Functions
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    let fixed_count = func.params.iter().take_while(|p| !p._is_rest).count();
                    if fixed_count < func.params.len() {
                        self.variadic_functions
                            .borrow_mut()
                            .insert(func.name.clone(), fixed_count);
                    }
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = declaration.as_ref() {
                        let fixed_count = func.params.iter().take_while(|p| !p._is_rest).count();
                        if fixed_count < func.params.len() {
                            self.variadic_functions
                                .borrow_mut()
                                .insert(func.name.clone(), fixed_count);
                        }
                    } else if let Statement::ClassDeclaration(class_decl) = declaration.as_ref() {
                        self.scan_variadic_class(class_decl);
                    }
                }
                Statement::ClassDeclaration(class_decl) => {
                    self.scan_variadic_class(class_decl);
                }
                _ => {}
            }
        }

        // Pass 1: Collect user functions and top-level variables
        self.scopes.borrow_mut().clear(); // Reset scopes
        self.enter_scope(); // Global scope

        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    self.register_function(func);
                }
                Statement::ExportDecl { declaration, .. } => match declaration.as_ref() {
                    Statement::FunctionDeclaration(func) => {
                        self.register_function(func);
                    }
                    Statement::ClassDeclaration(class_decl) => {
                        self.register_class(class_decl);
                    }
                    _ => {}
                },
                Statement::ClassDeclaration(class_decl) => {
                    self.register_class(class_decl);
                }
                Statement::VarDeclaration {
                    pattern,
                    type_annotation,
                    ..
                } => {
                    if let BindingNode::Identifier(name) = pattern {
                        let ty = TejxType::from_node(&type_annotation);
                        self.define(name.clone(), ty);
                    }
                }
                Statement::ExtensionDeclaration(ext_decl) => {
                    let mut class_methods = self.class_methods.borrow_mut();
                    let methods = class_methods
                        .entry(ext_decl._target_type.to_string())
                        .or_insert_with(Vec::new);
                    for method in &ext_decl._methods {
                        let mangled = if method.name.starts_with("f_") {
                            method.name.clone()
                        } else {
                            format!("f_{}_{}", ext_decl._target_type, method.name)
                        };

                        self.user_functions
                            .borrow_mut()
                            .insert(mangled, TejxType::from_node(&method.return_type));
                        methods.push(method.name.clone());
                    }
                }
                _ => {}
            }
        }

        // Pass 1.5: Monomorphize generic classes
        let mut monomorphized_stmts = Vec::new();
        for (base_name, instantiations) in &self.generic_instantiations {
            for concrete_args in instantiations {
                let mut original_class_decl = None;
                for stmt in &merged_statements {
                    match stmt {
                        Statement::ClassDeclaration(c) if &c.name == base_name => {
                            original_class_decl = Some(c.clone());
                            break;
                        }
                        Statement::ExportDecl { declaration, .. } => {
                            if let Statement::ClassDeclaration(c) = &**declaration {
                                if &c.name == base_name {
                                    original_class_decl = Some(c.clone());
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(mut c) = original_class_decl {
                    if c.generic_params.len() == concrete_args.len() {
                        let mut substitutions = HashMap::new();
                        let mut mangled_suffix = String::new();
                        for (i, param) in c.generic_params.iter().enumerate() {
                            let arg_type = &concrete_args[i];
                            let arg_name = arg_type.to_name();
                            substitutions.insert(param.name.clone(), arg_type.to_type_node());
                            mangled_suffix.push('_');
                            let safe_arg = arg_name
                                .replace("[]", "_arr")
                                .replace("<", "_")
                                .replace(">", "_")
                                .replace(", ", "_");
                            mangled_suffix.push_str(&safe_arg);
                        }

                        let mangled_name = format!("{}{}", base_name, mangled_suffix);
                        c.name = mangled_name.clone();
                        c.generic_params.clear();

                        let transformer =
                            crate::ast_transformer::TypeSubstitutor::new(&substitutions);
                        transformer.transform_class(&mut c);

                        self.register_class(&c);
                        monomorphized_stmts.push(Statement::ClassDeclaration(c));
                    }
                }
            }
        }

        for (func_name, instantiations) in &self.function_instantiations {
            for concrete_args in instantiations {
                let mut original_func_decl = None;
                for stmt in &merged_statements {
                    match stmt {
                        Statement::FunctionDeclaration(f) if &f.name == func_name => {
                            original_func_decl = Some(f.clone());
                            break;
                        }
                        Statement::ExportDecl { declaration, .. } => {
                            if let Statement::FunctionDeclaration(f) = &**declaration {
                                if &f.name == func_name {
                                    original_func_decl = Some(f.clone());
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }
                }

                if let Some(mut f) = original_func_decl {
                    if f.generic_params.len() == concrete_args.len() {
                        let mut substitutions = HashMap::new();
                        let mut mangled_suffix = String::new();
                        for (i, param) in f.generic_params.iter().enumerate() {
                            let arg_type = &concrete_args[i];
                            let arg_name = arg_type.to_name();
                            substitutions.insert(param.name.clone(), arg_type.to_type_node());
                            mangled_suffix.push('_');
                            let safe_arg = arg_name
                                .replace("[]", "_arr")
                                .replace("<", "_")
                                .replace(">", "_")
                                .replace(", ", "_");
                            mangled_suffix.push_str(&safe_arg);
                        }

                        let mangled_name = format!("{}{}", func_name, mangled_suffix);
                        f.name = mangled_name.clone();
                        f.generic_params.clear();

                        let transformer =
                            crate::ast_transformer::TypeSubstitutor::new(&substitutions);
                        transformer.transform_function(&mut f);

                        self.register_function(&f);
                        monomorphized_stmts.push(Statement::FunctionDeclaration(f));
                    }
                }
            }
        }

        merged_statements.extend(monomorphized_stmts);

        // Pass 2: Lower
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    self.lower_function_declaration(func, &mut functions);
                }
                Statement::ClassDeclaration(class_decl) => {
                    self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                }
                Statement::ExportDecl { declaration, .. } => match &**declaration {
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
                },
                Statement::ExtensionDeclaration(ext_decl) => {
                    self.lower_extension_declaration(ext_decl, &mut functions);
                }
                Statement::ImportDecl { .. } => {
                    // Handled in Pass 0
                }
                Statement::VarDeclaration {
                    pattern,
                    type_annotation,
                    initializer: _,
                    is_const: _,
                    ..
                } => {
                    // Register first for this scope
                    if let BindingNode::Identifier(name) = pattern {
                        let ty = TejxType::from_node(&type_annotation);
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

        let mut nested = self.nested_functions.borrow_mut();
        functions.append(&mut nested);

        // Add a "tejx_main" wrapper for non-function top-level statements
        // Check for user-defined main function (lowered as "f_main")
        // Identify main function
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

        if let Some(_) = main_func_idx {
            // Call the main function (f_main)
            let main_ret_ty = self
                .user_functions
                .borrow()
                .get("f_main")
                .cloned()
                .unwrap_or(TejxType::Void);
            entry_body_stmts.push(HIRStatement::ExpressionStmt {
                line: 0,
                expr: HIRExpression::Call {
                    line: 0,
                    callee: "f_main".to_string(),
                    args: vec![],
                    ty: main_ret_ty,
                },
            });
        }

        // Finalize entry point: Run event loop (moved to runtime.rs)
        // entry_body_stmts.push(HIRStatement::ExpressionStmt { line: line,
        //     expr: HIRExpression::Call { line: line,
        //         callee: TEJX_RUN_EVENT_LOOP.to_string(),
        //         args: vec![],
        //         ty: TejxType::Void,
        //     }
        // });
        // entry_body_stmts.push(HIRStatement::Return { line: line,  value: Some(HIRExpression::Literal { line: line,  value: "0".to_string(), ty: TejxType::Int32 }) });

        // Create the actual entry point function
        functions.push(HIRStatement::Function {
            name: "f_async_add".to_string(),
            async_params: None,
            params: vec![
                ("a".to_string(), TejxType::Int32),
                ("b".to_string(), TejxType::Int32),
            ],
            _return_type: TejxType::Int64,
            body: Box::new(HIRStatement::Block {
                line: 0,
                statements: vec![],
            }),
            is_extern: false,
            line: 0,
        });
        functions.push(HIRStatement::Function {
            name: "rt_main_async_worker".to_string(),
            params: vec![(
                "ctx".to_string(),
                TejxType::Class("Int64[]".to_string(), vec![]),
            )],
            _return_type: TejxType::Void,
            body: Box::new(HIRStatement::Block {
                line: 0,
                statements: vec![],
            }),
            is_extern: false,
            async_params: None,
            line: 0,
        });
        functions.push(HIRStatement::Function {
            async_params: None,
            line: line,
            name: TEJX_MAIN.to_string(),
            params: vec![],
            _return_type: TejxType::Void,
            body: Box::new(HIRStatement::Block {
                line: line,
                statements: entry_body_stmts,
            }),
            is_extern: false,
        });

        let mut signatures = HashMap::new();
        // Add built-in runtime signatures for proper auto-boxing in MIR
        signatures.insert("rt_len".to_string(), vec![TejxType::Int64]);
        signatures.insert(
            "rt_promise_resolve".to_string(),
            vec![TejxType::Int64, TejxType::Int64],
        );
        signatures.insert(
            "rt_promise_reject".to_string(),
            vec![TejxType::Int64, TejxType::Int64],
        );
        signatures.insert(
            "Thread_new".to_string(),
            vec![TejxType::Int64, TejxType::Int64, TejxType::Int64],
        );
        signatures.insert(
            RT_MOVE_MEMBER.to_string(),
            vec![TejxType::Int64, TejxType::Int32],
        );
        signatures.insert(
            TEJX_ENQUEUE_TASK.to_string(),
            vec![TejxType::Int64, TejxType::Int64],
        );
        signatures.insert(TEJX_INC_ASYNC_OPS.to_string(), vec![]);
        signatures.insert(TEJX_DEC_ASYNC_OPS.to_string(), vec![]);
        signatures.insert(TEJX_RUN_EVENT_LOOP.to_string(), vec![]);
        signatures.insert("rt_sleep".to_string(), vec![TejxType::Int64]);
        signatures.insert(
            "__optional_chain".to_string(),
            vec![TejxType::Int64, TejxType::Int64],
        );
        signatures.insert(
            "rt_object_merge".to_string(),
            vec![TejxType::Int64, TejxType::Int64],
        );
        signatures.insert("rt_len".to_string(), vec![TejxType::Int64]);
        signatures.insert("rt_typeof".to_string(), vec![TejxType::Int64]);

        for func in &functions {
            if let HIRStatement::Function { name, params, .. } = func {
                let param_types: Vec<TejxType> = params.iter().map(|(_, ty)| ty.clone()).collect();
                signatures.insert(name.clone(), param_types);
            }
        }

        let mut class_fields = HashMap::new();
        for (class_name, fields) in self.class_instance_fields.borrow().iter() {
            let mut field_names = Vec::new();
            for (name, ty, _) in fields {
                field_names.push((name.clone(), ty.clone()));
            }
            class_fields.insert(class_name.clone(), field_names);
        }

        LoweringResult {
            functions,
            signatures,
            captured_vars: self.captured_vars.borrow().clone(),
            class_fields,
            class_methods: self.class_methods.borrow().clone(),
        }
    }
}
