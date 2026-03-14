pub mod async_desugar;
pub mod builtins;
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
    erased_generic_functions: RefCell<HashSet<String>>,
    discovered_function_instantiations: RefCell<HashMap<String, std::collections::HashSet<Vec<TejxType>>>>,
    type_aliases: RefCell<HashMap<String, TejxType>>,
    pub async_enabled: bool,
    current_async_promise_id: RefCell<Option<String>>,
    pub diagnostics: RefCell<Vec<Diagnostic>>,
    pub filename: RefCell<String>,
    pub stdlib_path: RefCell<std::path::PathBuf>,
    pub lambda_inferred_types: HashMap<(usize, usize), Vec<TejxType>>,
    pub lambda_inferred_returns: HashMap<(usize, usize), TejxType>,
    current_return_type: RefCell<Option<TejxType>>,
    current_expected_type: RefCell<Option<TejxType>>,
    pub generic_instantiations: RefCell<HashMap<String, std::collections::HashSet<Vec<TejxType>>>>,
    pub function_instantiations: HashMap<String, std::collections::HashSet<Vec<TejxType>>>,
    env_owner_stack: RefCell<Vec<String>>,
    captured_vars_by_owner: RefCell<HashMap<String, HashSet<String>>>,
    lambda_env_owner: RefCell<HashMap<String, String>>,
}

/// Result of lowering: a list of top-level HIR functions.
/// The last one is always "tejx_main" containing non-function statements.
pub struct LoweringResult {
    pub functions: Vec<HIRStatement>, // Each should be HIRStatement::Function
    pub signatures: HashMap<String, Vec<TejxType>>,
    pub captured_vars_by_function: HashMap<String, HashSet<String>>,
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
            erased_generic_functions: RefCell::new(HashSet::new()),
            discovered_function_instantiations: RefCell::new(HashMap::new()),
            type_aliases: RefCell::new(HashMap::new()),
            async_enabled: false,
            current_async_promise_id: RefCell::new(None),
            diagnostics: RefCell::new(Vec::new()),
            filename: RefCell::new(String::new()),
            stdlib_path: RefCell::new(std::path::PathBuf::from("stdlib")),
            lambda_inferred_types: HashMap::new(),
            lambda_inferred_returns: HashMap::new(),
            current_return_type: RefCell::new(None),
            current_expected_type: RefCell::new(None),
            generic_instantiations: RefCell::new(HashMap::new()),
            function_instantiations: HashMap::new(),
            env_owner_stack: RefCell::new(Vec::new()),
            captured_vars_by_owner: RefCell::new(HashMap::new()),
            lambda_env_owner: RefCell::new(HashMap::new()),
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
        if let Some((scope, _)) = self.scopes.borrow().last() {
            if let Some((existing, _)) = scope.get(&name) {
                return existing.clone();
            }
        }
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
            let resolved = self.resolve_alias_type(&ty);
            scope.insert(name, (mangled.clone(), resolved));
        }
        mangled
    }

    pub(crate) fn narrow_type(&self, name: String, ty: TejxType) {
        if let Some((mangled, _)) = self.lookup(&name) {
            if let Some((scope, _)) = self.scopes.borrow_mut().last_mut() {
                let resolved = self.resolve_alias_type(&ty);
                scope.insert(name, (mangled, resolved));
            }
        }
    }

    pub(crate) fn register_type_alias(&self, name: &str, ty: TejxType) {
        self.type_aliases
            .borrow_mut()
            .insert(name.to_string(), ty);
    }

    pub(crate) fn resolve_alias_type(&self, ty: &TejxType) -> TejxType {
        fn resolve_inner(
            ctx: &Lowering,
            ty: &TejxType,
            depth: usize,
        ) -> TejxType {
            if depth > 20 {
                return ty.clone();
            }
            match ty {
                TejxType::Class(name, generics) => {
                    if let Some(alias) = ctx.type_aliases.borrow().get(name).cloned() {
                        return resolve_inner(ctx, &alias, depth + 1);
                    }
                    if generics.is_empty() {
                        TejxType::Class(name.clone(), vec![])
                    } else {
                        TejxType::Class(
                            name.clone(),
                            generics
                                .iter()
                                .map(|g| resolve_inner(ctx, g, depth + 1))
                                .collect(),
                        )
                    }
                }
                TejxType::FixedArray(inner, size) => TejxType::FixedArray(
                    Box::new(resolve_inner(ctx, inner, depth + 1)),
                    *size,
                ),
                TejxType::DynamicArray(inner) => TejxType::DynamicArray(Box::new(resolve_inner(
                    ctx,
                    inner,
                    depth + 1,
                ))),
                TejxType::Slice(inner) => TejxType::Slice(Box::new(resolve_inner(
                    ctx,
                    inner,
                    depth + 1,
                ))),
                TejxType::Function(params, ret) => TejxType::Function(
                    params
                        .iter()
                        .map(|p| resolve_inner(ctx, p, depth + 1))
                        .collect(),
                    Box::new(resolve_inner(ctx, ret, depth + 1)),
                ),
                TejxType::Union(types) => TejxType::Union(
                    types
                        .iter()
                        .map(|t| resolve_inner(ctx, t, depth + 1))
                        .collect(),
                ),
                TejxType::Object(props) => TejxType::Object(
                    props
                        .iter()
                        .map(|(k, o, t)| (k.clone(), *o, resolve_inner(ctx, t, depth + 1)))
                        .collect(),
                ),
                _ => ty.clone(),
            }
        }

        resolve_inner(self, ty, 0)
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
                    if let Some(owner) = self.current_env_owner() {
                        self.captured_vars_by_owner
                            .borrow_mut()
                            .entry(owner)
                            .or_insert_with(HashSet::new)
                            .insert(mangled.clone());
                    }
                }
                return Some(info.clone());
            }
        }
        None
    }

    fn push_env_owner(&self, name: String) {
        self.env_owner_stack.borrow_mut().push(name.clone());
        self.captured_vars_by_owner
            .borrow_mut()
            .entry(name)
            .or_insert_with(HashSet::new);
    }

    fn pop_env_owner(&self) {
        self.env_owner_stack.borrow_mut().pop();
    }

    fn current_env_owner(&self) -> Option<String> {
        self.env_owner_stack.borrow().last().cloned()
    }

    fn register_lambda_env_owner(&self, lambda_name: &str) {
        if let Some(owner) = self.current_env_owner() {
            self.lambda_env_owner
                .borrow_mut()
                .insert(lambda_name.to_string(), owner);
        }
    }

    fn find_class_template(
        &self,
        statements: &[Statement],
        base_name: &str,
    ) -> Option<ClassDeclaration> {
        for stmt in statements {
            match stmt {
                Statement::ClassDeclaration(class_decl) if class_decl.name == base_name => {
                    return Some(class_decl.clone());
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::ClassDeclaration(class_decl) = declaration.as_ref() {
                        if class_decl.name == base_name {
                            return Some(class_decl.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn find_function_template(
        &self,
        statements: &[Statement],
        base_name: &str,
    ) -> Option<FunctionDeclaration> {
        for stmt in statements {
            match stmt {
                Statement::FunctionDeclaration(func) if func.name == base_name => {
                    return Some(func.clone());
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = declaration.as_ref() {
                        if func.name == base_name {
                            return Some(func.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn discover_nested_monomorphs(&self, stmt: &Statement) {
        let lambda_len = self.lambda_functions.borrow().len();
        let nested_len = self.nested_functions.borrow().len();
        let mut functions = Vec::new();
        let mut main_stmts = Vec::new();

        match stmt {
            Statement::FunctionDeclaration(func) => {
                if self.should_lower_function(func) {
                    self.lower_function_declaration(func, &mut functions);
                }
            }
            Statement::ClassDeclaration(class_decl) => {
                if self.should_lower_class(class_decl) {
                    self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                }
            }
            Statement::ExportDecl { declaration, .. } => match declaration.as_ref() {
                Statement::FunctionDeclaration(func) => {
                    if self.should_lower_function(func) {
                        self.lower_function_declaration(func, &mut functions);
                    }
                }
                Statement::ClassDeclaration(class_decl) => {
                    if self.should_lower_class(class_decl) {
                        self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                    }
                }
                _ => {}
            },
            _ => {}
        }

        self.lambda_functions.borrow_mut().truncate(lambda_len);
        self.nested_functions.borrow_mut().truncate(nested_len);
    }

    fn monomorphize_to_fixed_point(&self, merged_statements: &mut Vec<Statement>) {
        let mut emitted_class_instantiations = HashSet::new();
        let mut emitted_function_instantiations = HashSet::new();

        loop {
            let mut new_statements = Vec::new();

            let class_instantiations: Vec<(String, Vec<TejxType>)> = self
                .generic_instantiations
                .borrow()
                .iter()
                .flat_map(|(base_name, instantiations)| {
                    instantiations
                        .iter()
                        .cloned()
                        .map(|args| (base_name.clone(), args))
                        .collect::<Vec<_>>()
                })
                .collect();

            for (base_name, concrete_args) in class_instantiations {
                if concrete_args.iter().any(|arg| !self.is_concrete_type(arg)) {
                    continue;
                }

                let instantiation_key = self.instantiation_key(&base_name, &concrete_args);
                if !emitted_class_instantiations.insert(instantiation_key) {
                    continue;
                }

                let Some(mut class_decl) = self.find_class_template(merged_statements, &base_name) else {
                    continue;
                };

                if class_decl.generic_params.len() != concrete_args.len() {
                    continue;
                }

                let mut substitutions = HashMap::new();
                let mangled_name = self.monomorphized_name(&base_name, &concrete_args);
                for (param, arg_type) in class_decl.generic_params.iter().zip(concrete_args.iter()) {
                    substitutions.insert(param.name.clone(), arg_type.to_type_node());
                }

                class_decl.name = mangled_name;
                class_decl.generic_params.clear();

                let transformer = crate::ast_transformer::TypeSubstitutor::new(&substitutions);
                transformer.transform_class(&mut class_decl);

                self.register_class(&class_decl);
                new_statements.push(Statement::ClassDeclaration(class_decl));
            }

            let mut function_instantiations = Vec::new();
            for (base_name, instantiations) in &self.function_instantiations {
                for args in instantiations {
                    function_instantiations.push((base_name.clone(), args.clone()));
                }
            }
            for (base_name, instantiations) in self.discovered_function_instantiations.borrow().iter() {
                for args in instantiations {
                    function_instantiations.push((base_name.clone(), args.clone()));
                }
            }

            for (base_name, concrete_args) in function_instantiations {
                if concrete_args.iter().any(|arg| !self.is_concrete_type(arg)) {
                    continue;
                }

                let instantiation_key = self.instantiation_key(&base_name, &concrete_args);
                if !emitted_function_instantiations.insert(instantiation_key) {
                    continue;
                }

                let Some(mut func_decl) = self.find_function_template(merged_statements, &base_name) else {
                    continue;
                };

                if func_decl.generic_params.len() != concrete_args.len() {
                    continue;
                }

                let mut substitutions = HashMap::new();
                let mangled_name = self.monomorphized_name(&base_name, &concrete_args);
                for (param, arg_type) in func_decl.generic_params.iter().zip(concrete_args.iter()) {
                    substitutions.insert(param.name.clone(), arg_type.to_type_node());
                }

                func_decl.name = mangled_name;
                func_decl.generic_params.clear();

                let transformer = crate::ast_transformer::TypeSubstitutor::new(&substitutions);
                transformer.transform_function(&mut func_decl);

                self.register_function(&func_decl);
                new_statements.push(Statement::FunctionDeclaration(func_decl));
            }

            if new_statements.is_empty() {
                break;
            }

            for stmt in &new_statements {
                self.discover_nested_monomorphs(stmt);
            }

            merged_statements.extend(new_statements);
        }
    }

    pub fn lower(&self, program: &Program, _base_path: &std::path::Path) -> LoweringResult {
        let line = 0; // Top level
        let mut functions = Vec::new();
        let mut main_stmts = Vec::new();
        let mut main_is_async = false;
        let mut merged_statements = program.statements.clone();

        // Pass 0.5: Scan for Variadic Functions + async main discovery
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    if func.name == "main" {
                        main_is_async = func._is_async;
                    }
                    let fixed_count = func.params.iter().take_while(|p| !p._is_rest).count();
                    if fixed_count < func.params.len() {
                        self.variadic_functions
                            .borrow_mut()
                            .insert(func.name.clone(), fixed_count);
                    }
                }
                Statement::ExportDecl { declaration, .. } => {
                    if let Statement::FunctionDeclaration(func) = declaration.as_ref() {
                        if func.name == "main" {
                            main_is_async = func._is_async;
                        }
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

        // Pass 1.5: Monomorphize generic declarations to a fixed point before HIR/MIR lowering.
        self.monomorphize_to_fixed_point(&mut merged_statements);

        // Pass 1.6: Register monomorphized functions/classes (and any new variants).
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    self.register_function(func);
                }
                Statement::ClassDeclaration(class_decl) => {
                    self.register_class(class_decl);
                    self.scan_variadic_class(class_decl);
                }
                Statement::ExportDecl { declaration, .. } => match declaration.as_ref() {
                    Statement::FunctionDeclaration(func) => {
                        self.register_function(func);
                    }
                    Statement::ClassDeclaration(class_decl) => {
                        self.register_class(class_decl);
                        self.scan_variadic_class(class_decl);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        // Pass 2: Lower
        self.push_env_owner(TEJX_MAIN.to_string());
        for stmt in &merged_statements {
            match stmt {
                Statement::FunctionDeclaration(func) => {
                    if self.should_lower_function(func) {
                        self.lower_function_declaration(func, &mut functions);
                    }
                }
                Statement::ClassDeclaration(class_decl) => {
                    if self.should_lower_class(class_decl) {
                        self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                    }
                }
                Statement::ExportDecl { declaration, .. } => match &**declaration {
                    Statement::FunctionDeclaration(func) => {
                        if self.should_lower_function(func) {
                            self.lower_function_declaration(func, &mut functions);
                        }
                    }
                    Statement::ClassDeclaration(class_decl) => {
                        if self.should_lower_class(class_decl) {
                            self.lower_class_declaration(class_decl, &mut functions, &mut main_stmts);
                        }
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
        self.pop_env_owner();

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

            if main_is_async {
                let main_call = HIRExpression::Call {
                    line: 0,
                    callee: "f_main".to_string(),
                    args: vec![],
                    ty: TejxType::Int64,
                };
                entry_body_stmts.push(HIRStatement::ExpressionStmt {
                    line: 0,
                    expr: HIRExpression::Call {
                        line: 0,
                        callee: "rt_await".to_string(),
                        args: vec![main_call],
                        ty: TejxType::Int64,
                    },
                });
            } else {
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
                TejxType::DynamicArray(Box::new(TejxType::Int64)),
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
        signatures.insert("rt_await".to_string(), vec![TejxType::Int64]);
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
        let parents = self.class_parents.borrow();
        let i_fields = self.class_instance_fields.borrow();
        
        for (class_name, _) in i_fields.iter() {
            let mut all_fields = Vec::new();
            let mut current = Some(class_name.clone());
            let mut chain = Vec::new();
            
            // Walk up to collect inheritance chain
            while let Some(c) = current {
                chain.push(c.clone());
                current = parents.get(&c).cloned();
            }
            
            // Collect fields from top of hierarchy down
            for c in chain.into_iter().rev() {
                if let Some(fields) = i_fields.get(&c) {
                    for (name, ty, _) in fields {
                        // Avoid duplicates if a class redeclares (though parser usually catches)
                        if !all_fields.iter().any(|(n, _): &(String, _)| n == name) {
                            all_fields.push((name.clone(), ty.clone()));
                        }
                    }
                }
            }
            class_fields.insert(class_name.clone(), all_fields);
        }

        let captured_by_owner = self.captured_vars_by_owner.borrow().clone();
        let lambda_owner = self.lambda_env_owner.borrow().clone();
        let mut captured_vars_by_function: HashMap<String, HashSet<String>> = HashMap::new();
        for func in &functions {
            if let HIRStatement::Function { name, .. } = func {
                let owner = lambda_owner
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                let vars = captured_by_owner.get(&owner).cloned().unwrap_or_default();
                captured_vars_by_function.insert(name.clone(), vars);
            }
        }

        LoweringResult {
            functions,
            signatures,
            captured_vars_by_function,
            class_fields,
            class_methods: self.class_methods.borrow().clone(),
        }
    }
}
