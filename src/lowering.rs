use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::hir::*;
use crate::intrinsics::*;
use crate::token::TokenType;
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
    pub lambda_inferred_types: HashMap<(usize, usize), Vec<String>>,
    current_return_type: RefCell<Option<TejxType>>,
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
        }
    }

    fn enter_scope(&self) {
        let depth = *self.lambda_depth.borrow();
        self.scopes.borrow_mut().push((HashMap::new(), depth));
    }

    /// Enter a scope that represents a lambda/closure boundary.
    /// Variables accessed from inside a lambda that were defined outside
    /// will be marked as captured.
    fn enter_lambda_scope(&self) {
        *self.lambda_depth.borrow_mut() += 1;
        let depth = *self.lambda_depth.borrow();
        self.scopes.borrow_mut().push((HashMap::new(), depth));
    }

    fn _exit_scope(&self) {
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

    fn define(&self, name: String, ty: TejxType) -> String {
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

    fn lookup(&self, name: &str) -> Option<(String, TejxType)> {
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
        let merged_statements = program.statements.clone();

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
                        let ty = TejxType::from_name(&type_annotation.raw_name);
                        self.define(name.clone(), ty);
                    }
                }
                Statement::ExtensionDeclaration(ext_decl) => {
                    let mut class_methods = self.class_methods.borrow_mut();
                    let methods = class_methods
                        .entry(ext_decl._target_type.raw_name.clone())
                        .or_insert_with(Vec::new);
                    for method in &ext_decl._methods {
                        let mangled = if method.name.starts_with("f_") {
                            method.name.clone()
                        } else {
                            format!("f_{}_{}", ext_decl._target_type, method.name)
                        };

                        self.user_functions
                            .borrow_mut()
                            .insert(mangled, TejxType::from_name(&method.return_type.raw_name));
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
                        let ty = TejxType::from_name(&type_annotation.raw_name);
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
            entry_body_stmts.push(HIRStatement::ExpressionStmt {
                line: 0,
                expr: HIRExpression::Call {
                    line: 0,
                    callee: "f_main".to_string(),
                    args: vec![],
                    ty: TejxType::Class("Object".to_string(), vec![]),
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
            _return_type: TejxType::Class("Object".to_string(), vec![]),
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
                TejxType::Class("Object[]".to_string(), vec![]),
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
        signatures.insert(
            "rt_len".to_string(),
            vec![TejxType::Class("Object".to_string(), vec![])],
        );
        signatures.insert(
            "rt_promise_resolve".to_string(),
            vec![
                TejxType::Class("Object".to_string(), vec![]),
                TejxType::Class("Object".to_string(), vec![]),
            ],
        );
        signatures.insert(
            "rt_promise_reject".to_string(),
            vec![
                TejxType::Class("Object".to_string(), vec![]),
                TejxType::Class("Object".to_string(), vec![]),
            ],
        );
        signatures.insert(
            "Thread_new".to_string(),
            vec![
                TejxType::Class("Object".to_string(), vec![]),
                TejxType::Class("Object".to_string(), vec![]),
                TejxType::Class("Object".to_string(), vec![]),
            ],
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
        signatures.insert(
            "rt_sleep".to_string(),
            vec![TejxType::Class("Object".to_string(), vec![])],
        );
        signatures.insert(
            "__optional_chain".to_string(),
            vec![
                TejxType::Class("Object".to_string(), vec![]),
                TejxType::Class("Object".to_string(), vec![]),
            ],
        );
        signatures.insert(
            "rt_object_merge".to_string(),
            vec![
                TejxType::Class("Object".to_string(), vec![]),
                TejxType::Class("Object".to_string(), vec![]),
            ],
        );
        signatures.insert(
            "rt_len".to_string(),
            vec![TejxType::Class("Object".to_string(), vec![])],
        );
        signatures.insert(
            "rt_typeof".to_string(),
            vec![TejxType::Class("Object".to_string(), vec![])],
        );

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

    fn scan_variadic_class(&self, class_decl: &ClassDeclaration) {
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

    fn register_function(&self, func: &FunctionDeclaration) {
        let param_types: Vec<TejxType> = func
            .params
            .iter()
            .map(|p| TejxType::from_name(&p.type_name.raw_name))
            .collect();
        let ret_type = TejxType::from_name(&func.return_type.raw_name);

        let name = if func.is_extern {
            func.name.clone()
        } else {
            format!("f_{}", func.name)
        };

        if !func.generic_params.is_empty() {
            self.function_generic_params
                .borrow_mut()
                .insert(name.clone(), func.generic_params.clone());
        }

        self.user_functions.borrow_mut().insert(
            name.clone(),
            TejxType::Function(param_types, Box::new(ret_type)),
        );
        self.user_function_args
            .borrow_mut()
            .insert(name, func.params.len());
        if func.is_extern {
            self.extern_functions.borrow_mut().insert(func.name.clone());
        }
    }

    /// Substitute generic type parameters in a return type based on the
    /// concrete object type. E.g., if Map<K,V>.get() returns TejxType::Class("V"),
    /// and object type is TejxType::Class("Map<string, string[]>"), this
    /// resolves to TejxType::Class("string[]").
    fn substitute_generics(
        &self,
        ret_ty: &TejxType,
        obj_type: &TejxType,
        callee_name: &str,
    ) -> TejxType {
        let type_name = ret_ty.to_name();

        // Find base class name from object type or assume Array/String
        let obj_full = obj_type.to_name();
        let base_class = if obj_type.is_array() {
            "Array".to_string()
        } else if *obj_type == TejxType::String {
            "String".to_string()
        } else {
            obj_full
                .split('<')
                .next()
                .unwrap_or(&obj_full)
                .trim()
                .to_string()
        };

        // Try getting params from class or function
        let mut params = Vec::new();
        if let Some(p) = self.class_generic_params.borrow().get(&base_class) {
            params = p.clone();
        } else if let Some(p) = self.function_generic_params.borrow().get(callee_name) {
            params = p.clone();
        } else if base_class == "Array" {
            params = vec!["T".to_string()];
        }

        if params.is_empty() {
            return ret_ty.clone();
        }

        // Extract concrete type args
        let concrete_args = if obj_type.is_array() {
            vec![obj_type.get_array_element_type().to_name()]
        } else if let Some(open) = obj_full.find('<') {
            if let Some(close) = obj_full.rfind('>') {
                let inner = &obj_full[open + 1..close];
                let mut args = Vec::new();
                let mut start = 0;
                let mut depth = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        ',' if depth == 0 => {
                            args.push(inner[start..i].trim().to_string());
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                args.push(inner[start..].trim().to_string());
                args
            } else {
                return ret_ty.clone();
            }
        } else {
            return ret_ty.clone();
        };

        let mut result_str = type_name.clone();
        for (i, param) in params.iter().enumerate() {
            if i < concrete_args.len() {
                // Use a regex-like boundary-aware replacement if possible,
                // but for now simple replace with some heuristics
                let from = param;
                let to = &concrete_args[i];

                // Replace "T" with "int", but avoid replacing "Type" with "intype"
                // Simple version: replace if it's the whole string or surrounded by non-alphanumerics
                // We'll just do simple string replacement for now as it's common in this codebase
                result_str = result_str.replace(from, to);
            }
        }

        if result_str != type_name {
            TejxType::from_name(&result_str)
        } else {
            ret_ty.clone()
        }
    }

    fn register_class(&self, class_decl: &ClassDeclaration) {
        if let Some(cons) = &class_decl._constructor {
            let mangled = format!("f_{}_constructor", class_decl.name);
            self.user_functions.borrow_mut().insert(
                mangled.clone(),
                TejxType::from_name(&cons.return_type.raw_name),
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
                TejxType::from_name(&method.func.return_type.raw_name),
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
            self.class_generic_params
                .borrow_mut()
                .insert(class_decl.name.clone(), class_decl.generic_params.clone());
        }

        let mut i_fields = Vec::new();
        let mut s_fields = Vec::new();
        for member in &class_decl._members {
            let ty = TejxType::from_name(&member._type_name.raw_name);
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
                TejxType::from_name(&constructor.return_type.raw_name),
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
                TejxType::from_name(&getter._return_type.raw_name),
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

    fn lower_function_declaration(
        &self,
        func: &FunctionDeclaration,
        functions: &mut Vec<HIRStatement>,
    ) {
        let line = func._line;
        if self.async_enabled && func._is_async {
            self.lower_async_function(func, functions);
            return;
        }

        let params: Vec<(String, TejxType)> = func
            .params
            .iter()
            .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name.raw_name)))
            .collect();
        let return_type = TejxType::from_name(&func.return_type.raw_name);

        self.enter_scope();
        let mangled_params: Vec<(String, TejxType)> = params
            .iter()
            .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
            .collect();

        let body = self
            .lower_statement(&func.body)
            .unwrap_or(HIRStatement::Block {
                line: line,
                statements: vec![],
            });

        self._exit_scope();

        let name = format!("f_{}", func.name);
        functions.push(HIRStatement::Function {
            async_params: None,
            line: line,
            name,
            params: mangled_params,
            _return_type: return_type,
            body: Box::new(body),
            is_extern: func.is_extern,
        });
    }

    fn lower_async_function(&self, func: &FunctionDeclaration, functions: &mut Vec<HIRStatement>) {
        let line = func._line;
        let params: Vec<(String, TejxType)> = func
            .params
            .iter()
            .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name.raw_name)))
            .collect();

        self.enter_scope();
        let mangled_params: Vec<(String, TejxType)> = params
            .iter()
            .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
            .collect();

        let (worker, _state_struct, wrapper_body) = self.lower_async_function_impl(
            &func.name,
            &mangled_params,
            &func.return_type.raw_name,
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
            _return_type: TejxType::Class("Object".to_string(), vec![]),
            body: Box::new(wrapper_body),
            is_extern: false,
        });
    }

    // This function generates the worker function and the body of the wrapper function for an async function.
    // It returns (worker_function_HIR, dummy_state_struct_HIR, wrapper_body_HIR_block).
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
        let ctx_ty = TejxType::Class("any[]".to_string(), vec![]);
        self.define(ctx_name.clone(), ctx_ty.clone());

        // unpack_promise_expr used to be here, now mir_lowering handles it
        let promise_id_var = self.define(
            "promise_id_local".to_string(),
            TejxType::Class("Object".to_string(), vec![]),
        );

        let mut worker_stmts = Vec::new();
        worker_stmts.push(HIRStatement::VarDecl {
            name: promise_id_var.clone(),
            initializer: None,
            ty: TejxType::Class("Object".to_string(), vec![]),
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
                                ty: TejxType::Class("Object".to_string(), vec![]),
                            },
                            HIRExpression::Literal {
                                line: line,
                                value: "0".to_string(),
                                ty: TejxType::Class("Object".to_string(), vec![]),
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
                                ty: TejxType::Class("Object".to_string(), vec![]),
                            },
                            HIRExpression::Variable {
                                line: line,
                                name: "err".to_string(),
                                ty: TejxType::Class("Object".to_string(), vec![]),
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
                TejxType::Class("any[]".to_string(), vec![]),
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
                ty: TejxType::Class("Object".to_string(), vec![]),
            }),
            ty: TejxType::Class("Object".to_string(), vec![]),
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
                    ty: TejxType::Class("Object".to_string(), vec![]),
                }],
                ty: TejxType::Class("Object".to_string(), vec![]),
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
        for _ in 0..128 {
            args_elems.push(HIRExpression::Literal {
                line: line,
                value: "0".to_string(),
                ty: TejxType::Int32,
            });
        }

        let ctx_var = format!("__ctx_{}", line);
        let ctx_ty = TejxType::Class("any[]".to_string(), vec![]);
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
                ty: TejxType::Class("Object".to_string(), vec![]),
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

    fn lower_class_declaration(
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
            return_type: TypeAnnotation::from_name("void".to_string()),
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
                params.push((p.name.clone(), TejxType::from_name(&p.type_name.raw_name)));
            }
            let name = format!("f_{}_{}", class_decl.name, func_decl.name);
            let return_type = TejxType::from_name(&func_decl.return_type.raw_name);

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
                                        ty: TejxType::Class("Object".to_string(), vec![]),
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
                                                ty: TejxType::Class("Object".to_string(), vec![]),
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
                                                ty: TejxType::Class("Object".to_string(), vec![]),
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
                                                ty: TejxType::Class("Object".to_string(), vec![]),
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
                    &func_decl.return_type.raw_name,
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
                    _return_type: TejxType::Class("Object".to_string(), vec![]),
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
            let return_type = TejxType::from_name(&getter._return_type.raw_name);

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
                TejxType::from_name(&setter._param_type.raw_name),
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
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    },
                });
            }
        }

        *self.current_class.borrow_mut() = None;
        *self.parent_class.borrow_mut() = None;
    }

    fn lower_extension_declaration(
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
                params.push((p.name.clone(), TejxType::from_name(&p.type_name.raw_name)));
            }

            let name = if func_decl.name.starts_with("f_") {
                func_decl.name.clone()
            } else {
                format!(
                    "f_{}_{}",
                    ext_decl
                        ._target_type
                        .raw_name
                        .replace("[", "_")
                        .replace("]", "_"),
                    func_decl.name
                )
            };
            let return_type = TejxType::from_name(&func_decl.return_type.raw_name);

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

    fn lower_statement(&self, stmt: &Statement) -> Option<HIRStatement> {
        let line = stmt.get_line();
        match stmt {
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
                                (name.clone(), TejxType::Class("Object".to_string(), vec![])),
                            );
                        }
                        self.user_functions.borrow_mut().insert(
                            name.clone(),
                            TejxType::from_name(&func.return_type.raw_name),
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
                let mut init = initializer.as_ref().map(|e| self.lower_expression(e));
                let ty = if type_annotation.is_empty() {
                    init.as_ref()
                        .map(|e| e.get_type())
                        .unwrap_or(TejxType::Class("Object".to_string(), vec![]))
                } else {
                    TejxType::from_name(&type_annotation.raw_name)
                };

                // If initializer is an empty array literal and type has a size_expr, set sized_allocation
                if let Some(size_ast) = &type_annotation.size_expr {
                    if let Some(HIRExpression::ArrayLiteral {
                        elements,
                        sized_allocation,
                        ..
                    }) = &mut init
                    {
                        if elements.is_empty() && sized_allocation.is_none() {
                            let size_hir = self.lower_expression(size_ast);
                            *sized_allocation = Some(Box::new(size_hir));
                        }
                    }
                }

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
                        initializer: Some(HIRExpression::ArrayLiteral {
                            elements: vec![],
                            sized_allocation: None,
                            ty: TejxType::Class("Object".to_string(), vec![]),
                            line: 0,
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
                let then_hir = self
                    .lower_statement(then_branch)
                    .unwrap_or(HIRStatement::Block {
                        line: line,
                        statements: vec![],
                    });
                let else_hir = else_branch.as_ref().and_then(|e| self.lower_statement(e));

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
                                && *target_ty != TejxType::Class("Object".to_string(), vec![])
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
                                    ty: TejxType::Class("Object".to_string(), vec![]),
                                },
                                resolved_expr.unwrap_or(HIRExpression::Literal {
                                    line: line,
                                    value: "0".to_string(),
                                    ty: TejxType::Class("Object".to_string(), vec![]),
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
                            TejxType::Class("Object".to_string(), vec![]),
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
                    .map(|p| (p.name.clone(), TejxType::from_name(&p.type_name.raw_name)))
                    .collect();
                let return_type = TejxType::from_name(&func.return_type.raw_name);

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
            Statement::ExportDecl {
                declaration,
                _is_default,
                ..
            } => {
                // Just lower the declaration - default export aliasing is handled during import splicing
                self.lower_statement(declaration)
            }
            _ => None,
        }
    }

    fn lower_statement_as_block(&self, stmt: &Statement) -> HIRStatement {
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

    fn lower_expression(&self, expr: &Expression) -> HIRExpression {
        let line = expr.get_line();
        match expr {
            Expression::ObjectLiteralExpr {
                entries, _spreads, ..
            } => {
                let mut hir_entries = Vec::new();
                for (key, val) in entries {
                    hir_entries.push((key.clone(), self.lower_expression(val)));
                }

                let base_obj = HIRExpression::ObjectLiteral {
                    entries: hir_entries,
                    ty: TejxType::Class("Object".to_string(), vec![]), // Map
                    line,
                };

                if _spreads.is_empty() {
                    base_obj
                } else {
                    // Handle spreads: merge spreads into base_obj by chaining calls
                    let mut expr = base_obj;
                    for spread in _spreads {
                        let spread_val = self.lower_expression(spread);
                        expr = HIRExpression::Call {
                            callee: "rt_object_merge".to_string(),
                            args: vec![expr, spread_val],
                            ty: TejxType::Class("Object".to_string(), vec![]),
                            line,
                        };
                    }
                    expr
                }
            }

            Expression::NumberLiteral {
                value, _is_float, ..
            } => {
                let (val_str, ty) = if *_is_float {
                    let mut s = value.to_string();
                    if !s.contains('.') && !s.contains('e') {
                        s.push_str(".0");
                    }
                    (s, TejxType::Float64)
                } else if value.fract() != 0.0 {
                    (value.to_string(), TejxType::Float64)
                } else {
                    (format!("{:.0}", value), TejxType::Int32)
                };
                HIRExpression::Literal {
                    line: line,
                    value: val_str,
                    ty,
                }
            }
            Expression::StringLiteral { value, .. } => HIRExpression::Literal {
                line: line,
                value: value.clone(),
                ty: TejxType::String,
            },
            Expression::BooleanLiteral { value, .. } => HIRExpression::Literal {
                line: line,
                value: value.to_string(),
                ty: TejxType::Bool,
            },
            Expression::ThisExpr { .. } => {
                let (name, ty) = self.lookup("this").unwrap_or_else(|| {
                    (
                        "this".to_string(),
                        TejxType::Class("Object".to_string(), vec![]),
                    )
                });
                HIRExpression::Variable {
                    line: line,
                    name,
                    ty,
                }
            }
            Expression::SuperExpr { .. } => {
                let (name, ty) = self.lookup("super").unwrap_or_else(|| {
                    (
                        "super".to_string(),
                        TejxType::Class("Object".to_string(), vec![]),
                    )
                });
                HIRExpression::Variable {
                    line: line,
                    name,
                    ty,
                }
            }
            Expression::Identifier { name, .. } => {
                let (resolved_name, mut ty) = self.lookup(name).unwrap_or_else(|| {
                    (name.clone(), TejxType::Class("Object".to_string(), vec![]))
                });
                let f_name = format!("f_{}", name);
                let final_name = if (self.user_functions.borrow().contains_key(name)
                    || self.user_functions.borrow().contains_key(&f_name))
                    && name != "main"
                    && !resolved_name.starts_with("g_")
                    && !resolved_name.contains("$")
                {
                    if let Some(actual_ty) = self.user_functions.borrow().get(name) {
                        ty = actual_ty.clone();
                    } else if let Some(actual_ty) = self.user_functions.borrow().get(&f_name) {
                        ty = actual_ty.clone();
                    }
                    f_name
                } else {
                    resolved_name
                };
                HIRExpression::Variable {
                    line: line,
                    name: final_name,
                    ty,
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
                    line: line,
                    op: TokenType::PipePipe,
                    left: Box::new(left_hir),
                    right: Box::new(right_hir),
                    ty: TejxType::Class("Object".to_string(), vec![]),
                }
            }
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                let target_hir = self.lower_expression(target);
                let index_hir = self.lower_expression(index);

                HIRExpression::If {
                    line: line,
                    condition: Box::new(HIRExpression::BinaryExpr {
                        line: line,
                        op: TokenType::BangEqual,
                        left: Box::new(target_hir.clone()),
                        right: Box::new(HIRExpression::Literal {
                            line: line,
                            value: "0".to_string(),
                            ty: TejxType::Int32,
                        }),
                        ty: TejxType::Bool,
                    }),
                    then_branch: Box::new(HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(target_hir),
                        index: Box::new(index_hir),
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    }),
                    else_branch: Box::new(HIRExpression::Literal {
                        line: line,
                        value: "0".to_string(),
                        ty: TejxType::Int32,
                    }),
                    ty: TejxType::Class("Object".to_string(), vec![]),
                }
            }
            Expression::NoneLiteral { .. } => HIRExpression::NoneLiteral { line },
            Expression::SomeExpr { value, .. } => HIRExpression::SomeExpr {
                value: Box::new(self.lower_expression(value)),
                line,
            },
            Expression::CastExpr { expr, .. } => self.lower_expression(expr),
            Expression::BinaryExpr {
                left, op, right, ..
            } => {
                // Desugar instanceof to runtime call
                if matches!(op, TokenType::Instanceof) {
                    let obj = self.lower_expression(left);
                    // Right side should be a class name identifier
                    let class_name = match right.as_ref() {
                        Expression::Identifier { name, .. } => name.clone(),
                        _ => "__unknown__".to_string(),
                    };
                    return HIRExpression::Call {
                        line: line,
                        callee: "rt_instanceof".to_string(),
                        args: vec![
                            obj,
                            HIRExpression::Literal {
                                line: line,
                                value: class_name,
                                ty: TejxType::String,
                            },
                        ],
                        ty: TejxType::Int32,
                    };
                }
                let l = self.lower_expression(left);
                let r = self.lower_expression(right);

                // Desugar === and !== to runtime calls
                if matches!(op, TokenType::EqualEqualEqual)
                    || matches!(op, TokenType::BangEqualEqual)
                {
                    let callee = if matches!(op, TokenType::EqualEqualEqual) {
                        "rt_strict_equal"
                    } else {
                        "rt_strict_ne"
                    };
                    return HIRExpression::Call {
                        line: line,
                        callee: callee.to_string(),
                        args: vec![l, r],
                        ty: TejxType::Bool,
                    };
                }

                let bin_ty = self.infer_hir_binary_type(&l, op, &r);
                HIRExpression::BinaryExpr {
                    line: line,
                    left: Box::new(l),
                    op: op.clone(),
                    right: Box::new(r),
                    ty: bin_ty,
                }
            }
            Expression::AssignmentExpr {
                target, value, _op, ..
            } => {
                let v = self.lower_expression(value);
                let ty = v.get_type();

                // Desugar compound assignments: a += b  ->  a = a + b
                let final_value = match _op {
                    TokenType::PlusEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Plus,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::MinusEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Minus,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::StarEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Star,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::SlashEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Slash,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::ModuloEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Modulo,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::AmpersandEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Ampersand,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::PipeEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Pipe,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::CaretEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Caret,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::LessLessEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::LessLess,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::GreaterGreaterEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::GreaterGreater,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    _ => v, // Direct assignment
                };

                if let Expression::MemberAccessExpr { object, member, .. } = target.as_ref() {
                    let obj_ty = self.lower_expression(object).get_type();
                    if let TejxType::Class(ref full_class, _) = obj_ty {
                        let class_name = full_class
                            .split('<')
                            .next()
                            .unwrap_or(full_class)
                            .trim()
                            .to_string();
                        let setters = self.class_setters.borrow();
                        if let Some(s_set) = setters.get(&class_name) {
                            if s_set.contains(member) {
                                return HIRExpression::Call {
                                    line: line,
                                    callee: format!("f_{}_set_{}", class_name, member),
                                    args: vec![self.lower_expression(object), final_value],
                                    ty: TejxType::Void,
                                };
                            }
                        }
                    }
                }

                match target.as_ref() {
                    Expression::Identifier { .. }
                    | Expression::MemberAccessExpr { .. }
                    | Expression::ArrayAccessExpr { .. } => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            line: line,
                            target: Box::new(t),
                            value: Box::new(final_value),
                            ty,
                        }
                    }
                    _ => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            line: line,
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
                        let delta = if matches!(op, TokenType::PlusPlus) {
                            "1"
                        } else {
                            "1"
                        };
                        let bin_op = if matches!(op, TokenType::PlusPlus) {
                            TokenType::Plus
                        } else {
                            TokenType::Minus
                        };

                        let r_expr = self.lower_expression(right);
                        // Reconstruct Assignment: right = right op 1
                        // Need to clone target handling from AssignmentExpr logic ideally.
                        // Simplification:
                        HIRExpression::Assignment {
                            line: line,
                            target: Box::new(r_expr.clone()),
                            value: Box::new(HIRExpression::BinaryExpr {
                                line: line,
                                left: Box::new(r_expr),
                                op: bin_op,
                                right: Box::new(HIRExpression::Literal {
                                    line: line,
                                    value: delta.to_string(),
                                    ty: TejxType::Int32,
                                }),
                                ty: TejxType::Int32,
                            }),
                            ty: TejxType::Int32,
                        }
                    }
                    TokenType::Bang => HIRExpression::Call {
                        line: line,
                        callee: "rt_not".to_string(),
                        args: vec![self.lower_expression(right)],
                        ty: TejxType::Bool,
                    },
                    TokenType::Minus => {
                        // -x -> 0 - x
                        HIRExpression::BinaryExpr {
                            line: line,
                            left: Box::new(HIRExpression::Literal {
                                line: line,
                                value: "0".to_string(),
                                ty: TejxType::Int32,
                            }),
                            op: TokenType::Minus,
                            right: Box::new(self.lower_expression(right)),
                            ty: TejxType::Int32,
                        }
                    }
                    _ => self.lower_expression(right), // Fallback
                }
            }
            Expression::CallExpr { callee, args, .. } => {
                let hir_args: Vec<HIRExpression> =
                    args.iter().map(|a| self.lower_expression(a)).collect();
                let callee_str = callee.to_callee_name();
                let normalized = callee_str
                    .replace('.', "_")
                    .replace("::", "_")
                    .replace(":", "_");
                let mut final_callee = normalized.clone();
                let mut hir_args = hir_args;
                if callee_str == "typeof" {
                    if let Some(arg) = hir_args.get(0) {
                        let arg_ty = arg.get_type();
                        if let TejxType::Class(_name, _) = &arg_ty {
                            // Let objects be evaluated at runtime for inheritance paths
                            final_callee = "rt_typeof".to_string();
                            let _ty = TejxType::String;
                        } else {
                            // Extract known static type string
                            let type_str = match &arg_ty {
                                TejxType::Int32 => "int",
                                TejxType::Float64 => "float",
                                TejxType::Bool => "boolean",
                                TejxType::String => "string",
                                TejxType::Char => "char",
                                _ => "unknown",
                            };
                            return HIRExpression::Literal {
                                line,
                                value: type_str.to_string(),
                                ty: TejxType::String,
                            };
                        }
                    } else {
                        return HIRExpression::Literal {
                            line,
                            value: "undefined".to_string(),
                            ty: TejxType::String,
                        };
                    }
                }
                let mut final_args = hir_args.clone();
                let mut ty = TejxType::Class("Object".to_string(), vec![]);

                // Indirect call check: if name is a variable holding a function
                if !callee_str.is_empty() && !callee_str.contains('.') {
                    if let Some((mangled, var_ty)) = self.lookup(&callee_str) {
                        if !self.user_functions.borrow().contains_key(&callee_str)
                            && !self
                                .user_functions
                                .borrow()
                                .contains_key(&format!("f_{}", callee_str))
                        {
                            let ret_ty = if let TejxType::Function(_, ret) = &var_ty {
                                (**ret).clone()
                            } else {
                                TejxType::Class("Object".to_string(), vec![])
                            };
                            return HIRExpression::IndirectCall {
                                line,
                                callee: Box::new(HIRExpression::Variable {
                                    line,
                                    name: mangled,
                                    ty: var_ty,
                                }),
                                args: hir_args.clone(),
                                ty: ret_ty,
                            };
                        } else {
                            // It's a valid direct call to a user function, so use the mangled name!
                            final_callee = mangled;
                        }
                    }
                }

                // 1. Built-in special functions (Most are now in prelude or intrinsics)
                if callee_str == "super" {
                    if let Some(parent) = &*self.parent_class.borrow() {
                        final_callee = format!("f_{}_constructor", parent);
                        let (mangled_this, _) = self.lookup("this").unwrap_or_else(|| {
                            (
                                "this".to_string(),
                                TejxType::Class("Object".to_string(), vec![]),
                            )
                        });
                        final_args = vec![HIRExpression::Variable {
                            line,
                            name: mangled_this,
                            ty: TejxType::Class("Object".to_string(), vec![]),
                        }];
                        final_args.extend(hir_args);
                        ty = TejxType::Void;
                    }
                } else if let Expression::MemberAccessExpr { object, member, .. } = callee.as_ref()
                {
                    if let Expression::SuperExpr { .. } = object.as_ref() {
                        if let Some(parent) = &*self.parent_class.borrow() {
                            final_callee = format!("f_{}_{}", parent, member);
                            let (mangled_this, _) = self.lookup("this").unwrap_or_else(|| {
                                (
                                    "this".to_string(),
                                    TejxType::Class("Object".to_string(), vec![]),
                                )
                            });
                            final_args = vec![HIRExpression::Variable {
                                line,
                                name: mangled_this,
                                ty: TejxType::Class("Object".to_string(), vec![]),
                            }];
                            final_args.extend(final_args.clone()); // Still wrong.

                            if let Some(ret_ty) = self.user_functions.borrow().get(&final_callee) {
                                ty = match ret_ty {
                                    TejxType::Function(_, ret) => (**ret).clone(),
                                    _ => ret_ty.clone(),
                                };
                            }
                        }
                    } else {
                        let mut resolved = false;

                        if !resolved {
                            // Priority 2: Static Methods
                            if let Expression::Identifier { name: obj_name, .. } = object.as_ref() {
                                if self.class_methods.borrow().contains_key(obj_name) {
                                    let static_callee = format!("f_{}_{}", obj_name, member);
                                    if let Some(ret_ty) =
                                        self.user_functions.borrow().get(&static_callee)
                                    {
                                        final_callee = static_callee;
                                        ty = match ret_ty {
                                            TejxType::Function(_, ret) => (**ret).clone(),
                                            _ => ret_ty.clone(),
                                        };

                                        resolved = true;
                                    }
                                }
                            }
                        }

                        if !resolved {
                            // Priority 3: Instance/Runtime Methods (General Resolution)
                            let obj_hir = self.lower_expression(object);
                            let obj_ty = obj_hir.get_type();

                            if obj_ty == TejxType::String || obj_ty.is_array() || obj_ty.is_slice()
                            {
                                // UFCS for built-in types: arr.push(v) -> rt_array_push(arr, v) or f_push(arr, v)
                                let rt_name = match member.as_str() {
                                    "push" => Some("rt_array_push"),
                                    "pop" => Some("rt_array_pop"),
                                    "shift" => Some("rt_array_shift"),
                                    "unshift" => Some("rt_array_unshift"),
                                    "includes" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_includes")
                                        } else {
                                            Some("f_includes") // Use generic prelude version
                                        }
                                    }
                                    "startsWith" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_startsWith")
                                        } else {
                                            None
                                        }
                                    }
                                    "endsWith" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_endsWith")
                                        } else {
                                            None
                                        }
                                    }
                                    "indexOf" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_indexOf")
                                        } else {
                                            Some("rt_array_indexOf")
                                        }
                                    }
                                    "concat" => Some("rt_array_concat"),
                                    "join" => Some("rt_array_join"),
                                    "slice" => Some("rt_array_slice"),
                                    "reverse" => Some("rt_array_reverse"),
                                    "fill" => Some("rt_array_fill"),
                                    "sort" => Some("rt_array_sort"),
                                    "map" => Some("f_map"),
                                    "filter" => Some("f_filter"),
                                    "forEach" => Some("f_forEach"),
                                    "reduce" => Some("f_reduce"),
                                    "every" => Some("f_every"),
                                    "some" => Some("f_some"),
                                    "find" => Some("f_find"),
                                    "findIndex" => Some("f_findIndex"),
                                    // String specific
                                    "toUpperCase" => Some("rt_String_toUpperCase"),
                                    "toLowerCase" => Some("rt_String_toLowerCase"),
                                    "trim" => Some("rt_String_trim"),
                                    "trimStart" => Some("rt_String_trimStart"),
                                    "trimEnd" => Some("rt_String_trimEnd"),
                                    "substring" => Some("rt_String_substring"),
                                    "split" => Some("rt_String_split"),
                                    "repeat" => Some("rt_String_repeat"),
                                    "replace" => Some("rt_String_replace"),
                                    _ => None,
                                };

                                final_callee = if let Some(n) = rt_name {
                                    n.to_string()
                                } else {
                                    format!("f_{}", member)
                                };

                                // Resolve return type for the UFCS call
                                if let Some(ret_ty) =
                                    self.user_functions.borrow().get(&final_callee)
                                {
                                    ty = self.substitute_generics(ret_ty, &obj_ty, &final_callee);
                                } else {
                                    // Fallback if not found in prelude: might be a method that needs class mangling
                                    let class_name = obj_ty.to_name();
                                    final_callee = format!("f_{}_{}", class_name, member);
                                }

                                let mut n_args = vec![obj_hir];
                                n_args.extend(hir_args.clone());
                                final_args = n_args;
                                // resolved = true;
                            } else {
                                let type_name = match obj_ty {
                                    TejxType::Class(ref c, _) => c.clone(),
                                    _ => format!("{:?}", obj_ty),
                                };

                                let type_name = type_name
                                    .split('<')
                                    .next()
                                    .unwrap_or(&type_name)
                                    .trim()
                                    .to_string();
                                let method_key = format!("f_{}_{}", type_name, member);

                                if let Some(ret_ty) = self.user_functions.borrow().get(&method_key)
                                {
                                    final_callee = method_key.clone();
                                    // Substitute generic type params from the concrete object type
                                    ty = self.substitute_generics(
                                        ret_ty,
                                        &obj_hir.get_type(),
                                        &final_callee,
                                    );
                                } else if self.extern_functions.borrow().contains(&method_key) {
                                    final_callee = method_key;
                                } else {
                                    // Walk class hierarchy to find inherited methods
                                    let mut found = false;
                                    let mut parent_class =
                                        { self.class_parents.borrow().get(&type_name).cloned() };
                                    while let Some(ref parent) = parent_class {
                                        let parent_method_key = format!("f_{}_{}", parent, member);
                                        if let Some(ret_ty) =
                                            self.user_functions.borrow().get(&parent_method_key)
                                        {
                                            final_callee = parent_method_key;
                                            ty = self.substitute_generics(
                                                ret_ty,
                                                &obj_hir.get_type(),
                                                &final_callee,
                                            );
                                            found = true;
                                            break;
                                        } else if self
                                            .extern_functions
                                            .borrow()
                                            .contains(&parent_method_key)
                                        {
                                            final_callee = parent_method_key;
                                            found = true;
                                            break;
                                        }
                                        parent_class =
                                            self.class_parents.borrow().get(parent).cloned();
                                    }
                                    if !found {
                                        // Fallback to dynamic or best-effort mangling
                                        final_callee = method_key;
                                    }
                                }
                                let mut n_args = vec![obj_hir];
                                n_args.extend(hir_args.clone());
                                final_args = n_args;
                                // resolved = true;
                            }
                        }
                    }
                } else if let Some(ret_ty) = {
                    let mut found = self.user_functions.borrow().get(&normalized).cloned();
                    if found.is_none() && !normalized.starts_with("f_") {
                        let f_name = format!("f_{}", normalized);
                        if let Some(ty) = self.user_functions.borrow().get(&f_name).cloned() {
                            final_callee = f_name;
                            found = Some(ty);
                        }
                    }
                    found
                } {
                    if final_callee == "main" {
                        final_callee = "tejx_main".to_string();
                    } else if self.extern_functions.borrow().contains(&normalized) {
                        final_callee = normalized.clone();
                    }
                    // if it was found as f_normalized, final_callee is already updated above

                    ty = match ret_ty {
                        TejxType::Function(_, ret) => *ret,
                        _ => ret_ty,
                    };

                    // Also try to substitute generics for top-level calls if they have generic params
                    if !final_args.is_empty() {
                        let first_arg_ty = final_args[0].get_type();
                        ty = self.substitute_generics(&ty, &first_arg_ty, &final_callee);
                    }
                } else if self.class_methods.borrow().contains_key(&normalized) {
                    let f_cons = format!("f_{}_constructor", normalized);
                    let cons = format!("{}_constructor", normalized);
                    if let Some(_ret_ty) = self.user_functions.borrow().get(&f_cons) {
                        final_callee = f_cons;
                        ty = TejxType::Class(normalized.clone(), vec![]);
                    } else if let Some(_ret_ty) = self.user_functions.borrow().get(&cons) {
                        final_callee = cons;
                        ty = TejxType::Class(normalized.clone(), vec![]);
                    } else {
                        final_callee = f_cons;
                        ty = TejxType::Class(normalized.clone(), vec![]);
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
                            line: line,
                            elements: rest.to_vec(),
                            ty: TejxType::Class("Object".to_string(), vec![]),
                            sized_allocation: None,
                        });
                        final_args = new_var_args;
                    }
                } else {
                    // Non-variadic: pad missing arguments with None for Optionals/T|None
                    let expected_count_opt =
                        self.user_function_args.borrow().get(&final_callee).copied();
                    if let Some(expected_count) = expected_count_opt {
                        while final_args.len() < expected_count {
                            final_args.push(HIRExpression::NoneLiteral { line });
                        }
                    }
                }

                HIRExpression::Call {
                    line: line,
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
                        if let Some((_name, f_ty, _)) = f_list.iter().find(|(n, _, _)| n == member)
                        {
                            return HIRExpression::Variable {
                                line: line,
                                name: format!("g_{}_{}", obj_name, member),
                                ty: f_ty.clone(),
                            };
                        }
                    }
                }

                // Getter Resolution
                let obj_ty = self.lower_expression(object).get_type();
                if let TejxType::Class(ref full_class, _) = obj_ty {
                    let class_name = full_class
                        .split('<')
                        .next()
                        .unwrap_or(full_class)
                        .trim()
                        .to_string();
                    let getters = self.class_getters.borrow();
                    if let Some(g_set) = getters.get(&class_name) {
                        if g_set.contains(member) {
                            return HIRExpression::Call {
                                line: line,
                                callee: format!("f_{}_get_{}", class_name, member),
                                args: vec![self.lower_expression(object)],
                                ty: TejxType::Class("Object".to_string(), vec![]),
                            };
                        }
                    }
                }

                // Field Resolution
                let lowered_object = self.lower_expression(object);
                let obj_ty = lowered_object.get_type();
                if let TejxType::Class(ref full_class, _) = obj_ty {
                    let class_name = full_class
                        .split('<')
                        .next()
                        .unwrap_or(full_class)
                        .trim()
                        .to_string();
                    let fields = self.class_instance_fields.borrow();
                    if let Some(i_list) = fields.get(&class_name) {
                        for (f_name, f_ty, _) in i_list {
                            if f_name == member {
                                return HIRExpression::MemberAccess {
                                    line: line,
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
                let f_combined = format!("f_{}", combined);
                if self.user_functions.borrow().contains_key(&combined)
                    || self.user_functions.borrow().contains_key(&f_combined)
                {
                    HIRExpression::Variable {
                        line: line,
                        name: f_combined,
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    }
                } else {
                    HIRExpression::MemberAccess {
                        line: line,
                        target: Box::new(lowered_object),
                        member: member.clone(),
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    }
                }
            }
            Expression::ArrayAccessExpr { target, index, .. } => {
                let lowered_target = self.lower_expression(target);
                let target_ty = lowered_target.get_type();

                // If it's an Array<T> class instance, desugar to arr.data[i]
                if let TejxType::Class(name, _) = &target_ty {
                    if name.starts_with("Array<") || name == "Array" {
                        let elem_ty = target_ty.get_array_element_type();
                        return HIRExpression::IndexAccess {
                            line: line,
                            target: Box::new(HIRExpression::MemberAccess {
                                line: line,
                                target: Box::new(lowered_target),
                                member: "data".to_string(),
                                ty: TejxType::Class(format!("{}[]", elem_ty.to_name()), vec![]),
                            }),
                            index: Box::new(self.lower_expression(index)),
                            ty: elem_ty,
                        };
                    }
                }

                let ty = target_ty.get_array_element_type();
                HIRExpression::IndexAccess {
                    line: line,
                    target: Box::new(lowered_target),
                    index: Box::new(self.lower_expression(index)),
                    ty,
                }
            }
            Expression::ArrayLiteral { elements, ty, .. } => {
                let inferred_ty = match &*ty.borrow() {
                    Some(t) => TejxType::from_name(t),
                    None => TejxType::Class("any[]".to_string(), vec![]),
                };
                // Handle spreads: [a, ...b, c] -> concat(concat([a], b), [c])
                let mut chunks: Vec<HIRExpression> = Vec::new();
                let mut current_chunk: Vec<HIRExpression> = Vec::new();

                for e in elements {
                    if let Expression::SpreadExpr { _expr, .. } = e {
                        // Push accumulated static chunk if any
                        if !current_chunk.is_empty() {
                            chunks.push(HIRExpression::ArrayLiteral {
                                line: line,
                                elements: current_chunk.clone(),
                                ty: inferred_ty.clone(),
                                sized_allocation: None,
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
                        line: line,
                        elements: current_chunk,
                        ty: inferred_ty.clone(),
                        sized_allocation: None,
                    });
                }

                if chunks.is_empty() {
                    // Empty array []
                    HIRExpression::ArrayLiteral {
                        line: line,
                        elements: vec![],
                        sized_allocation: None,
                        ty: inferred_ty.clone(),
                    }
                } else {
                    // Reduce chunks with Array_concat
                    let mut expr = chunks[0].clone();
                    for next_chunk in chunks.into_iter().skip(1) {
                        expr = HIRExpression::Call {
                            line: line,
                            callee: "rt_array_concat".to_string(),
                            args: vec![expr, next_chunk],
                            ty: TejxType::Class("any[]".to_string(), vec![]),
                        };
                    }
                    expr
                }
            }
            Expression::SequenceExpr { expressions, .. } => {
                let mut lower_exprs = Vec::new();
                for e in expressions {
                    lower_exprs.push(self.lower_expression(e));
                }
                let ty = lower_exprs
                    .last()
                    .map(|e| e.get_type())
                    .unwrap_or(TejxType::Void);
                HIRExpression::Sequence {
                    expressions: lower_exprs,
                    ty,
                    line,
                }
            }
            Expression::LambdaExpr {
                params,
                body,
                _line,
                _col,
            } => {
                let id = {
                    let mut counter = self.lambda_counter.borrow_mut();
                    let val = *counter;
                    *counter += 1;
                    val
                };
                let lambda_name = format!("lambda_{}", id);

                // Use inferred types from TypeChecker if available
                let inferred = self.lambda_inferred_types.get(&(*_line, *_col));

                let hir_params: Vec<(String, TejxType)> = params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        (
                            p.name.clone(),
                            if let Some(inf) = inferred {
                                if i < inf.len() {
                                    TejxType::from_name(&inf[i])
                                } else if p.type_name.is_empty() {
                                    TejxType::Class("Object".to_string(), vec![])
                                } else {
                                    TejxType::from_name(&p.type_name.raw_name)
                                }
                            } else if p.type_name.is_empty() {
                                TejxType::Class("Object".to_string(), vec![])
                            } else {
                                TejxType::from_name(&p.type_name.raw_name)
                            },
                        )
                    })
                    .collect();

                self.enter_lambda_scope();
                let mut mangled_params: Vec<(String, TejxType)> = hir_params
                    .iter()
                    .map(|(name, ty)| (self.define(name.clone(), ty.clone()), ty.clone()))
                    .collect();

                // Pad up to 4 user arguments so all lambdas have consistent signatures for array methods
                while mangled_params.len() < 4 {
                    mangled_params.push((
                        self.define(
                            format!("__dummy_pad_{}", mangled_params.len()),
                            TejxType::Class("Object".to_string(), vec![]),
                        ),
                        TejxType::Class("Object".to_string(), vec![]),
                    ));
                }

                // Add implicit environment parameter - all lambdas called from JS-like env need this
                mangled_params.insert(
                    0,
                    (
                        self.define(
                            "__env".to_string(),
                            TejxType::Class("Object".to_string(), vec![]),
                        ),
                        TejxType::Class("Object".to_string(), vec![]),
                    ),
                );

                let hir_body = self.lower_statement(body).unwrap_or(HIRStatement::Block {
                    line: line,
                    statements: vec![],
                });

                self._exit_scope();

                self.lambda_functions
                    .borrow_mut()
                    .push(HIRStatement::Function {
                        async_params: None,
                        line: line,
                        name: lambda_name.clone(),
                        params: mangled_params,
                        _return_type: TejxType::Class("Object".to_string(), vec![]),
                        body: Box::new(hir_body),
                        is_extern: false,
                    });

                HIRExpression::Literal {
                    line: line,
                    value: lambda_name,
                    ty: TejxType::Class("Object".to_string(), vec![]), // Actually function type
                }
            }
            Expression::AwaitExpr { expr, .. } => HIRExpression::Await {
                line: line,
                expr: Box::new(self.lower_expression(expr)),
                ty: TejxType::Class("Object".to_string(), vec![]),
            },
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                HIRExpression::OptionalChain {
                    line: line,
                    target: Box::new(self.lower_expression(object)),
                    operation: format!(".{}", member),
                    ty: TejxType::Class("Object".to_string(), vec![]),
                }
            }
            Expression::NewExpr {
                class_name, args, ..
            } => {
                let mut hir_args: Vec<HIRExpression> =
                    args.iter().map(|a| self.lower_expression(a)).collect();

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
                            line: line,
                            elements: rest.to_vec(),
                            sized_allocation: None,
                            ty: TejxType::Class("Object".to_string(), vec![]),
                        });
                        hir_args = new_var_args;
                    }
                }

                HIRExpression::NewExpr {
                    line: line,
                    class_name: normalized_class,
                    _args: hir_args,
                    ty: TejxType::from_name(class_name),
                }
            }
            Expression::OptionalCallExpr {
                callee,
                args: _args,
                ..
            } => {
                let callee_expr = self.lower_expression(callee);

                HIRExpression::OptionalChain {
                    line: line,
                    target: Box::new(callee_expr),
                    operation: "()".to_string(), // In HIR/MIR, OptionalChain "()" means call
                    ty: TejxType::Class("Object".to_string(), vec![]),
                }
            }
            Expression::TernaryExpr {
                _condition,
                _true_branch,
                _false_branch,
                ..
            } => {
                let cond = self.lower_expression(_condition);
                let t_branch = self.lower_expression(_true_branch);
                let f_branch = self.lower_expression(_false_branch);
                HIRExpression::If {
                    line: line,
                    condition: Box::new(cond),
                    then_branch: Box::new(t_branch),
                    else_branch: Box::new(f_branch),
                    ty: TejxType::Class("Object".to_string(), vec![]),
                }
            }

            _ => HIRExpression::Literal {
                line: line,
                value: "0".to_string(),
                ty: TejxType::Class("Object".to_string(), vec![]),
            },
        }
    }

    fn lower_binding_pattern(
        &self,
        pattern: &BindingNode,
        initializer: Option<HIRExpression>,
        ty: &TejxType,
        is_const: bool,
        stmts: &mut Vec<HIRStatement>,
    ) {
        let line = 0; // Default or pass as arg? Let us pass as arg maybe.
                      // Actually, many callers don't have a line here.
        match pattern {
            BindingNode::Identifier(name) => {
                let mangled = self.define(name.clone(), ty.clone());
                stmts.push(HIRStatement::VarDecl {
                    line: line,
                    name: mangled,
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
                    line: line,
                    name: tmp_id.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: true,
                });

                for (i, el) in elements.iter().enumerate() {
                    let el_init = HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
                            name: tmp_id.clone(),
                            ty: ty.clone(),
                        }),
                        index: Box::new(HIRExpression::Literal {
                            line: line,
                            value: i.to_string(),
                            ty: TejxType::Int32,
                        }),
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    };
                    self.lower_binding_pattern(el, Some(el_init), &TejxType::Void, is_const, stmts);
                }

                if let Some(r) = rest {
                    // handle rest ...tail
                    // let tail = Array_sliceRest(tmp, elements.len());
                    let slice_init = HIRExpression::Call {
                        line: line,
                        callee: "f_RT_Array_sliceRest".to_string(),
                        args: vec![
                            HIRExpression::Variable {
                                line: line,
                                name: tmp_id.clone(),
                                ty: ty.clone(),
                            },
                            HIRExpression::Literal {
                                line: line,
                                value: elements.len().to_string(),
                                ty: TejxType::Int32,
                            },
                        ],
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    };
                    self.lower_binding_pattern(
                        r,
                        Some(slice_init),
                        &TejxType::Void,
                        is_const,
                        stmts,
                    );
                }
            }
            BindingNode::ObjectBinding { entries } => {
                let tmp_id = format!("destructure_tmp_{}", self.lambda_counter.borrow());
                *self.lambda_counter.borrow_mut() += 1;

                stmts.push(HIRStatement::VarDecl {
                    line: line,
                    name: tmp_id.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: true,
                });

                for (key, target) in entries {
                    let el_init = HIRExpression::MemberAccess {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
                            name: tmp_id.clone(),
                            ty: ty.clone(),
                        }),
                        member: key.clone(),
                        ty: TejxType::Class("Object".to_string(), vec![]),
                    };
                    self.lower_binding_pattern(
                        target,
                        Some(el_init),
                        &TejxType::Void,
                        is_const,
                        stmts,
                    );
                }
            }
        }
    }
}

impl Lowering {
    fn infer_hir_binary_type(
        &self,
        left: &HIRExpression,
        op: &TokenType,
        right: &HIRExpression,
    ) -> TejxType {
        let lt = left.get_type();
        let rt = right.get_type();

        if matches!(
            op,
            TokenType::EqualEqual
                | TokenType::BangEqual
                | TokenType::Less
                | TokenType::LessEqual
                | TokenType::Greater
                | TokenType::GreaterEqual
                | TokenType::AmpersandAmpersand
                | TokenType::PipePipe
        ) {
            return TejxType::Bool;
        }

        if lt == TejxType::String || rt == TejxType::String {
            return TejxType::String;
        }

        let is_float = |t: &TejxType| -> bool {
            matches!(t, TejxType::Float16 | TejxType::Float32 | TejxType::Float64)
        };

        if lt == TejxType::Float64 || rt == TejxType::Float64 {
            return TejxType::Float64;
        }
        if is_float(&lt) || is_float(&rt) {
            return TejxType::Float32; // Default promotion
        }

        if lt == TejxType::Int64 || rt == TejxType::Int64 {
            return TejxType::Int64;
        }
        if matches!(lt, TejxType::Int16 | TejxType::Int32 | TejxType::Int128)
            || matches!(rt, TejxType::Int16 | TejxType::Int32 | TejxType::Int128)
        {
            return TejxType::Int32; // Default promotion
        }

        TejxType::Class("Object".to_string(), vec![])
    }

    pub fn resolve_imports(
        &self,
        mut statements: Vec<Statement>,
        current_dir: &std::path::Path,
        processed_files: &mut HashSet<std::path::PathBuf>,
        import_stack: &mut Vec<std::path::PathBuf>,
        current_file: Option<&std::path::Path>,
    ) -> Vec<Statement> {
        let filename = current_file
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| self.filename.borrow().clone());

        // Implicitly import prelude if this isn't the prelude or runtime itself
        let is_prelude = filename.ends_with("prelude.tx") || filename.contains("prelude.tx");
        let is_runtime = filename.ends_with("runtime.tx") || filename.contains("runtime.tx");
        if !is_prelude && !is_runtime {
            let mut already_imports_prelude = false;
            for stmt in &statements {
                if let Statement::ImportDecl { source, .. } = stmt {
                    if source == "std:prelude" {
                        already_imports_prelude = true;
                        break;
                    }
                }
            }
            if !already_imports_prelude {
                statements.insert(
                    0,
                    Statement::ImportDecl {
                        source: "std:prelude".to_string(),
                        _names: Vec::new(),
                        _is_default: false,
                        _line: 0,
                        _col: 0,
                    },
                );
            }
        }

        let mut i = 0;
        while i < statements.len() {
            if let Statement::ImportDecl {
                source,
                _names,
                _is_default,
                _line,
                _col,
            } = &statements[i]
            {
                let import_items = _names.clone();
                let import_line = *_line;
                let import_col = *_col;
                let is_default = *_is_default;
                let source_str = source.clone();

                let path = if source_str.starts_with("std:") {
                    let mod_name = &source_str[4..];
                    let mut p = self.stdlib_path.borrow().clone();
                    p.push(mod_name);
                    p.set_extension("tx");
                    p
                } else {
                    let mut p = current_dir.to_path_buf();
                    let clean_source = source_str.trim_matches('"');
                    if clean_source.starts_with("./") {
                        p.push(&clean_source[2..]);
                    } else {
                        p.push(clean_source);
                    }
                    if !p.to_string_lossy().ends_with(".tx") {
                        p.set_extension("tx");
                    }
                    p
                };

                if !path.exists() {
                    self.diagnostics.borrow_mut().push(
                        Diagnostic::new(
                            format!("Module not found: '{}'", source_str),
                            import_line,
                            import_col,
                            filename.clone(),
                        )
                        .with_code("E0200")
                        .with_label(&format!("Module not found: '{}'", source_str)),
                    );
                    i += 1;
                    continue;
                }

                let canon_path = match std::fs::canonicalize(&path) {
                    Ok(p) => p,
                    Err(_) => path.clone(),
                };

                if import_stack.contains(&canon_path) {
                    self.diagnostics.borrow_mut().push(
                        Diagnostic::new(
                            format!("Circular dependency detected: '{}'", source_str),
                            import_line,
                            import_col,
                            filename.clone(),
                        )
                        .with_code("E0204")
                        .with_label("circularly imported here"),
                    );
                    statements.remove(i);
                    continue;
                }

                if processed_files.contains(&canon_path) {
                    statements.remove(i);
                    continue;
                }

                processed_files.insert(canon_path.clone());
                import_stack.push(canon_path.clone());

                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        self.diagnostics.borrow_mut().push(Diagnostic::new(
                            format!("Failed to read module '{}': {}", source_str, e),
                            import_line,
                            import_col,
                            filename.clone(),
                        ));
                        i += 1;
                        continue;
                    }
                };

                let mut lexer = crate::lexer::Lexer::new(&content, &path.to_string_lossy());
                let tokens = lexer.tokenize();
                if !lexer.errors.is_empty() {
                    for diag in &lexer.errors {
                        self.diagnostics.borrow_mut().push(diag.clone());
                    }
                    i += 1;
                    continue;
                }

                let mut parser = crate::parser::Parser::new(tokens, &path.to_string_lossy());
                let imported_program = parser.parse_program();
                if parser.has_errors() {
                    for diag in parser.get_errors() {
                        self.diagnostics.borrow_mut().push(diag.clone());
                    }
                    i += 1;
                    continue;
                }

                let mut new_stmts = self.resolve_imports(
                    imported_program.statements,
                    path.parent().unwrap_or(std::path::Path::new(".")),
                    processed_files,
                    import_stack,
                    Some(&path),
                );
                import_stack.pop();

                // Handle Aliasing
                for item in &import_items {
                    if is_default {
                        let target_name = item.alias.as_ref().unwrap_or(&item.name);
                        for stmt in new_stmts.iter_mut() {
                            if let Statement::ExportDecl {
                                declaration,
                                _is_default: true,
                                ..
                            } = stmt
                            {
                                match declaration.as_mut() {
                                    Statement::FunctionDeclaration(func) => {
                                        func.name = target_name.clone()
                                    }
                                    Statement::ClassDeclaration(class) => {
                                        class.name = target_name.clone()
                                    }
                                    Statement::VarDeclaration {
                                        pattern: crate::ast::BindingNode::Identifier(name),
                                        ..
                                    } => *name = target_name.clone(),
                                    _ => {}
                                }
                            }
                        }
                    } else if let Some(alias) = &item.alias {
                        for stmt in new_stmts.iter_mut() {
                            if let Statement::ExportDecl { declaration, .. } = stmt {
                                match declaration.as_mut() {
                                    Statement::FunctionDeclaration(func)
                                        if func.name == item.name =>
                                    {
                                        func.name = alias.clone()
                                    }
                                    Statement::ClassDeclaration(class)
                                        if class.name == item.name =>
                                    {
                                        class.name = alias.clone()
                                    }
                                    Statement::VarDeclaration {
                                        pattern: crate::ast::BindingNode::Identifier(name),
                                        ..
                                    } if name == &item.name => *name = alias.clone(),
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                // Validate exports
                let mut exported_names: HashSet<String> = HashSet::new();
                let mut has_default_export = false;

                fn collect_names(stmt: &Statement, names: &mut HashSet<String>) {
                    match stmt {
                        Statement::FunctionDeclaration(f) => {
                            names.insert(f.name.clone());
                        }
                        Statement::ClassDeclaration(c) => {
                            names.insert(c.name.clone());
                        }
                        Statement::VarDeclaration {
                            pattern: crate::ast::BindingNode::Identifier(n),
                            ..
                        } => {
                            names.insert(n.clone());
                        }
                        Statement::BlockStmt { statements, .. } => {
                            for s in statements {
                                collect_names(s, names);
                            }
                        }
                        Statement::ExportDecl { declaration, .. } => {
                            collect_names(declaration, names);
                        }
                        _ => {}
                    }
                }

                for stmt in &new_stmts {
                    if let Statement::ExportDecl {
                        declaration,
                        _is_default: is_def,
                        ..
                    } = stmt
                    {
                        if *is_def {
                            has_default_export = true;
                        }
                        collect_names(declaration, &mut exported_names);
                    }
                }

                if is_default && !has_default_export {
                    self.diagnostics.borrow_mut().push(
                        Diagnostic::new(
                            format!("Module '{}' has no default export", source_str),
                            import_line,
                            import_col,
                            filename.clone(),
                        )
                        .with_code("E0203"),
                    );
                } else if !is_default && !import_items.is_empty() {
                    for item in &import_items {
                        let lookup_name = item.alias.as_ref().unwrap_or(&item.name);
                        if !exported_names.contains(lookup_name) {
                            self.diagnostics.borrow_mut().push(
                                Diagnostic::new(
                                    format!(
                                        "'{}' is not exported from '{}'",
                                        item.name, source_str
                                    ),
                                    import_line,
                                    import_col,
                                    filename.clone(),
                                )
                                .with_code("E0202"),
                            );
                        }
                    }
                }

                statements.splice(i..i + 1, new_stmts);
                continue;
            }
            i += 1;
        }
        statements
    }
}
