pub mod class;
pub mod compat;
pub mod expr;
pub mod func;
pub mod generics;
pub mod stmt;

use crate::frontend::ast::{Program, Statement};
use crate::common::diagnostics::Diagnostic; // Import Diagnostic
use crate::common::types::TejxType;
use std::collections::HashMap;

// TypeInfo struct removed (unused)

#[derive(Clone, Debug, PartialEq)]
pub enum AccessLevel {
    Public,
    Private,
}

#[derive(Clone, Debug)]
pub struct MemberInfo {
    pub ty: TejxType,
    pub is_static: bool,
    pub access: AccessLevel,
    pub is_readonly: bool,
    pub generic_params: Vec<crate::frontend::ast::GenericParam>,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub ty: TejxType,
    pub is_const: bool,
    pub params: Vec<TejxType>,     // Parameter types if function
    pub min_params: Option<usize>, // Minimum required params (excluding defaulted ones)
    pub is_variadic: bool,
    pub aliased_type: Option<TejxType>,
    pub generic_params: Vec<crate::frontend::ast::GenericParam>,
}

pub struct TypeChecker {
    scopes: Vec<HashMap<String, Symbol>>,
    current_class: Option<String>,
    current_function_return: Option<TejxType>,
    current_function_is_async: bool,
    loop_depth: usize,
    pub diagnostics: Vec<Diagnostic>, // Collect errors
    current_file: String,
    class_hierarchy: HashMap<String, String>, // Child -> Parent
    interfaces: HashMap<String, HashMap<String, MemberInfo>>, // Interface -> Method Name -> Info
    class_members: HashMap<String, HashMap<String, MemberInfo>>, // Class -> Member info
    pub async_enabled: bool,
    abstract_classes: std::collections::HashSet<String>,
    /// SOI: Remaining statements in the current block, used for look-ahead
    remaining_stmts: Vec<Statement>,
    lambda_context_params: Option<Vec<TejxType>>,
    pub lambda_inferred_types: HashMap<(usize, usize), Vec<TejxType>>,
    pub lambda_inferred_returns: HashMap<(usize, usize), TejxType>,
    pub(crate) current_expected_type: Option<TejxType>,
    pub generic_instantiations: HashMap<String, std::collections::HashSet<Vec<TejxType>>>,
    pub function_instantiations: HashMap<String, std::collections::HashSet<Vec<TejxType>>>,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        let globals = HashMap::new();
        let class_members = HashMap::new();
        let class_hierarchy = HashMap::new();
        // Standard library symbols are now loaded from the prelude and explicit imports.
        
        TypeChecker {
            scopes: vec![globals],
            current_class: None,
            current_function_return: None,
            current_function_is_async: false,
            loop_depth: 0,
            diagnostics: Vec::new(),
            current_file: "<inferred>".to_string(),
            class_hierarchy,
            interfaces: HashMap::new(),
            class_members,
            async_enabled: true,
            abstract_classes: std::collections::HashSet::new(),
            remaining_stmts: Vec::new(),
            lambda_context_params: None,
            lambda_inferred_types: HashMap::new(),
            lambda_inferred_returns: HashMap::new(),
            current_expected_type: None,
            generic_instantiations: HashMap::new(),
            function_instantiations: HashMap::new(),
        }
    }

    pub fn check(&mut self, program: &Program, filename: &str) -> Result<(), ()> {
        self.current_file = filename.to_string();

        // Pass 1: Collect declarations for hoisting
        for stmt in &program.statements {
            self.collect_declarations(stmt);
        }

        // Pass 2: Basic pass
        // Pass 2: Basic pass
        for stmt in &program.statements {
            match stmt {
                Statement::ImportDecl { .. }
                | Statement::FunctionDeclaration(_)
                | Statement::ClassDeclaration(_)
                | Statement::EnumDeclaration(_)
                | Statement::InterfaceDeclaration { .. }
                | Statement::TypeAliasDeclaration { .. }
                | Statement::ExtensionDeclaration(_)
                | Statement::ExportDecl { .. }
                | Statement::VarDeclaration { .. } => {
                    // Allowed
                }
                _ => {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "Executable statements are not allowed at the top level".to_string(),
                            stmt.get_line(),
                            stmt.get_col(),
                            self.current_file.clone(),
                        )
                        .with_code("E0114")
                        .with_hint("Wrap executable code inside a 'function main() { ... }' block"),
                    );
                }
            }
            let _ = self.check_statement(stmt);
        }

        if self.diagnostics.is_empty() {
            Ok(())
        } else {
            Err(())
        }
    }

    pub(crate) fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    pub(crate) fn define(&mut self, name: String, type_name: String) {
        let (_, final_params, is_variadic) = self.parse_signature(type_name.clone());
        let ty_obj = if type_name.starts_with("function:") {
            let (ret_str, _, _) = self.parse_signature(type_name.clone());
            let final_param_types: Vec<TejxType> = final_params.iter().map(|p| TejxType::from_name(p)).collect();
            TejxType::Function(final_param_types, Box::new(TejxType::from_name(&ret_str)))
        } else {
            TejxType::from_name(&type_name)
        };
        
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name,
                Symbol {
                    ty: ty_obj,
                    is_const: false,
                    params: final_params
                        .iter()
                        .map(|p| TejxType::from_name(p))
                        .collect(),
                    min_params: None,
                    is_variadic,
                    aliased_type: None,
                    generic_params: Vec::new(),
                },
            );
        }
    }

    pub(crate) fn define_with_params(
        &mut self,
        name: String,
        type_name: String,
        params: Vec<String>,
    ) {
        self.define_with_params_variadic(name, type_name, params, false);
    }

    pub(crate) fn define_with_params_variadic(
        &mut self,
        name: String,
        type_name: String,
        params: Vec<String>,
        is_variadic: bool,
    ) {
        let ty_obj = if type_name.starts_with("function:") {
            let (ret_str, _, _) = self.parse_signature(type_name.clone());
            let param_types: Vec<TejxType> = params.iter().map(|p| TejxType::from_name(p)).collect();
            TejxType::Function(param_types, Box::new(TejxType::from_name(&ret_str)))
        } else {
            TejxType::from_name(&type_name)
        };
        
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name,
                Symbol {
                    ty: ty_obj,
                    is_const: false,
                    params: params.iter().map(|p| TejxType::from_name(p)).collect(),
                    min_params: None,
                    is_variadic,
                    aliased_type: None,
                    generic_params: Vec::new(),
                },
            );
        }
    }

    pub(crate) fn define_variable(
        &mut self,
        name: String,
        type_name: String,
        is_const: bool,
        line: usize,
        col: usize,
    ) {
        // Parse signature first, as it only needs an immutable borrow of self
        let (_, final_params, is_variadic) = self.parse_signature(type_name.clone());

        let ty_obj = if type_name.starts_with("function:") {
            let (ret_str, _, _) = self.parse_signature(type_name.clone());
            let final_param_types: Vec<TejxType> = final_params.iter().map(|p| TejxType::from_name(p)).collect();
            TejxType::Function(final_param_types, Box::new(TejxType::from_name(&ret_str)))
        } else {
            TejxType::from_name(&type_name)
        };

        if let Some(scope) = self.scopes.last_mut() {
            if scope.contains_key(&name) {
                self.report_error_detailed(
                    format!("Variable '{}' is already defined in this scope", name),
                    line,
                    col,
                    "E0109",
                    Some("Choose a different name or remove the duplicate declaration"),
                );
                return;
            }

            scope.insert(
                name,
                Symbol {
                    ty: ty_obj,
                    is_const,
                    params: final_params
                        .iter()
                        .map(|p| TejxType::from_name(p))
                        .collect(),
                    min_params: None,
                    is_variadic,
                    aliased_type: None,
                    generic_params: Vec::new(),
                },
            );
        }
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(s) = scope.get(name) {
                return Some(s.clone());
            }
        }
        None
    }

    pub(crate) fn report_error_detailed(
        &mut self,
        msg: String,
        line: usize,
        col: usize,
        code: &str,
        hint: Option<&str>,
    ) {
        let mut diag = Diagnostic::new(msg, line, col, self.current_file.clone()).with_code(code);
        if let Some(h) = hint {
            diag = diag.with_hint(h);
        }
        self.diagnostics.push(diag);
    }

    pub(crate) fn with_expected_type<F, R>(&mut self, ty: Option<TejxType>, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let prev = self.current_expected_type.take();
        self.current_expected_type = ty;
        let res = f(self);
        self.current_expected_type = prev;
        res
    }

    pub(crate) fn with_lambda_context<F, R>(&mut self, params: Option<Vec<TejxType>>, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let prev = self.lambda_context_params.take();
        self.lambda_context_params = params;
        let res = f(self);
        self.lambda_context_params = prev;
        res
    }
}
