use crate::ast::{BindingNode, Expression, Program, Statement};
use crate::diagnostics::Diagnostic; // Import Diagnostic
use crate::token::TokenType;
use std::collections::HashMap;

// TypeInfo struct removed (unused)

#[derive(Clone, Debug, PartialEq)]
pub enum AccessLevel {
    Public,
    Private,
}

#[derive(Clone, Debug)]
pub struct MemberInfo {
    pub type_name: String,
    pub is_static: bool,
    pub access: AccessLevel,
    pub is_readonly: bool,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub type_name: String,
    pub is_const: bool,
    pub params: Vec<String>, // Parameter types if function

    pub is_variadic: bool,
    pub aliased_type: Option<String>,
    pub is_moved: bool,
}

pub struct TypeChecker {
    scopes: Vec<HashMap<String, Symbol>>,
    current_class: Option<String>,
    current_function_return: Option<String>,
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
}

impl TypeChecker {
    /// SOI: Check if an expression tree contains a reference to a given identifier
    fn expr_contains_identifier(expr: &Expression, name: &str) -> bool {
        match expr {
            Expression::Identifier { name: n, .. } => n == name,
            Expression::MemberAccessExpr { object, .. } => {
                Self::expr_contains_identifier(object, name)
            }
            Expression::CallExpr { callee, args, .. } => {
                Self::expr_contains_identifier(callee, name)
                    || args.iter().any(|a| Self::expr_contains_identifier(a, name))
            }
            Expression::BinaryExpr { left, right, .. } => {
                Self::expr_contains_identifier(left, name)
                    || Self::expr_contains_identifier(right, name)
            }
            Expression::UnaryExpr { right, .. } => Self::expr_contains_identifier(right, name),
            Expression::ArrayAccessExpr { target, index, .. } => {
                Self::expr_contains_identifier(target, name)
                    || Self::expr_contains_identifier(index, name)
            }
            Expression::AssignmentExpr { target, value, .. } => {
                Self::expr_contains_identifier(target, name)
                    || Self::expr_contains_identifier(value, name)
            }
            Expression::NewExpr { args, .. } => {
                args.iter().any(|a| Self::expr_contains_identifier(a, name))
            }
            Expression::ArrayLiteral { elements, .. } => elements
                .iter()
                .any(|e| Self::expr_contains_identifier(e, name)),
            Expression::ObjectLiteralExpr { entries, .. } => entries
                .iter()
                .any(|(_, e)| Self::expr_contains_identifier(e, name)),
            Expression::LambdaExpr { body, .. } => Self::stmt_contains_identifier(body, name),
            Expression::AwaitExpr { expr, .. } => Self::expr_contains_identifier(expr, name),
            Expression::OptionalMemberAccessExpr { object, .. } => {
                Self::expr_contains_identifier(object, name)
            }
            Expression::OptionalCallExpr { callee, args, .. } => {
                Self::expr_contains_identifier(callee, name)
                    || args.iter().any(|a| Self::expr_contains_identifier(a, name))
            }
            Expression::NoneLiteral { .. } => false,
            Expression::SomeExpr { value, .. } => Self::expr_contains_identifier(value, name),
            _ => false,
        }
    }

    /// SOI: Check if a statement tree contains a reference to a given identifier
    fn stmt_contains_identifier(stmt: &Statement, name: &str) -> bool {
        match stmt {
            Statement::ExpressionStmt { _expression, .. } => {
                Self::expr_contains_identifier(_expression, name)
            }
            Statement::VarDeclaration { initializer, .. } => {
                if let Some(init) = initializer {
                    Self::expr_contains_identifier(init, name)
                } else {
                    false
                }
            }
            Statement::ReturnStmt { value, .. } => {
                if let Some(val) = value {
                    Self::expr_contains_identifier(val, name)
                } else {
                    false
                }
            }
            Statement::BlockStmt { statements, .. } => statements
                .iter()
                .any(|s| Self::stmt_contains_identifier(s, name)),
            Statement::IfStmt {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                Self::expr_contains_identifier(condition, name)
                    || Self::stmt_contains_identifier(then_branch, name)
                    || else_branch
                        .as_ref()
                        .map_or(false, |e| Self::stmt_contains_identifier(e, name))
            }
            Statement::WhileStmt {
                condition, body, ..
            } => {
                Self::expr_contains_identifier(condition, name)
                    || Self::stmt_contains_identifier(body, name)
            }
            Statement::ForStmt {
                condition,
                increment,
                body,
                ..
            } => {
                condition
                    .as_ref()
                    .map_or(false, |c| Self::expr_contains_identifier(c, name))
                    || increment
                        .as_ref()
                        .map_or(false, |i| Self::expr_contains_identifier(i, name))
                    || Self::stmt_contains_identifier(body, name)
            }
            _ => false,
        }
    }

    pub fn new() -> Self {
        let globals = HashMap::new();
        let class_members = HashMap::new();
        let class_hierarchy = HashMap::new();
        // Standard library symbols are now loaded from the prelude and explicit imports.
        let checker = TypeChecker {
            scopes: vec![globals],
            current_class: None,
            current_function_return: None,
            current_function_is_async: false,
            loop_depth: 0,
            diagnostics: Vec::new(),
            current_file: "unknown".to_string(),
            class_hierarchy,
            interfaces: HashMap::new(),
            class_members,
            async_enabled: true,
            abstract_classes: std::collections::HashSet::new(),
            remaining_stmts: Vec::new(),
        };
        checker
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

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    fn collect_declarations(&mut self, stmt: &Statement) {
        match stmt {
            Statement::ClassDeclaration(class_decl) => {
                self.define_with_params(
                    class_decl.name.clone(),
                    "class".to_string(),
                    class_decl.generic_params.clone(),
                );
                if class_decl._is_abstract {
                    self.abstract_classes.insert(class_decl.name.clone());
                }
                if !class_decl._parent_name.is_empty() {
                    self.class_hierarchy
                        .insert(class_decl.name.clone(), class_decl._parent_name.clone());
                }
                let mut members = HashMap::new();
                for m in &class_decl._members {
                    members.insert(
                        m._name.clone(),
                        MemberInfo {
                            type_name: self
                                .parameterize_generics(&m._type_name, &class_decl.generic_params),
                            is_static: m._is_static,
                            access: if m._access == crate::ast::AccessModifier::Private {
                                AccessLevel::Private
                            } else {
                                AccessLevel::Public
                            },
                            is_readonly: false,
                        },
                    );
                }
                for method in &class_decl.methods {
                    let ret_ty = if method.func.return_type.is_empty() {
                        "any".to_string()
                    } else {
                        method.func.return_type.clone()
                    };
                    let (final_type, _, _) = self.parse_signature(format!("function:{}", ret_ty));
                    let parameterized_type =
                        self.parameterize_generics(&final_type, &class_decl.generic_params);
                    members.insert(
                        method.func.name.clone(),
                        MemberInfo {
                            type_name: parameterized_type,
                            is_static: method.is_static,
                            access: if method._access == crate::ast::AccessModifier::Private {
                                AccessLevel::Private
                            } else {
                                AccessLevel::Public
                            },
                            is_readonly: true, // Methods are readonly
                        },
                    );
                }
                for getter in &class_decl._getters {
                    members.insert(
                        getter._name.clone(),
                        MemberInfo {
                            type_name: self.parameterize_generics(
                                &getter._return_type,
                                &class_decl.generic_params,
                            ),
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true, // Default to readonly, setter can clear it
                        },
                    );
                }
                for setter in &class_decl._setters {
                    if let Some(existing) = members.get_mut(&setter._name) {
                        existing.is_readonly = false;
                    } else {
                        members.insert(
                            setter._name.clone(),
                            MemberInfo {
                                type_name: self.parameterize_generics(
                                    &setter._param_type,
                                    &class_decl.generic_params,
                                ), // or void?
                                is_static: false,
                                access: AccessLevel::Public,
                                is_readonly: false,
                            },
                        );
                    }
                }
                self.class_members.insert(class_decl.name.clone(), members);
            }
            Statement::FunctionDeclaration(func) => {
                let ret_ty = if func.return_type.is_empty() {
                    "any".to_string()
                } else {
                    func.return_type.clone()
                };
                let mut is_variadic = false;
                let params = func
                    .params
                    .iter()
                    .map(|p| {
                        if p._is_rest {
                            is_variadic = true;
                        }
                        let (t, _, _) = self.parse_signature(p.type_name.clone());
                        t
                    })
                    .collect::<Vec<String>>();
                let (final_ret, _, _) = self.parse_signature(format!("function:{}", ret_ty));
                self.define_with_params_variadic(func.name.clone(), final_ret, params, is_variadic);
            }
            Statement::TypeAliasDeclaration {
                name, _type_def, ..
            } => {
                // self.define(name.clone(), "type".to_string());
                // Handle alias definition manually to set aliased_type
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        name.clone(),
                        Symbol {
                            type_name: "type".to_string(),
                            is_const: true,
                            params: Vec::new(),
                            is_variadic: false,
                            aliased_type: Some(_type_def.clone()),
                            is_moved: false,
                        },
                    );
                }
            }
            Statement::EnumDeclaration(enum_decl) => {
                self.define(enum_decl.name.clone(), "enum".to_string());
                let mut members = HashMap::new();
                for member in &enum_decl._members {
                    members.insert(
                        member._name.clone(),
                        MemberInfo {
                            type_name: enum_decl.name.clone(),
                            is_static: true,
                            access: AccessLevel::Public,
                            is_readonly: true, // Enum members are constants
                        },
                    );
                }
                self.class_members.insert(enum_decl.name.clone(), members);
            }
            // Statement::ProtocolDeclaration(proto) => {
            //     self.define(proto._name.clone(), "protocol".to_string());
            //     self.interfaces.insert(proto._name.clone(), proto._methods.iter().map(|m| m._name.clone()).collect());
            // }
            Statement::InterfaceDeclaration {
                name,
                _methods: methods,
                ..
            } => {
                self.define(name.clone(), "interface".to_string());
                let mut interface_methods = HashMap::new();
                for m in methods {
                    // Extract method info
                    let mut param_types = Vec::new();
                    for p in &m._params {
                        param_types.push(p.type_name.clone());
                    }
                    let p_str = param_types.join(",");
                    let type_str = format!("function:{}:{}", m._return_type, p_str);
                    interface_methods.insert(
                        m._name.clone(),
                        MemberInfo {
                            type_name: type_str,
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true,
                        },
                    );
                }
                self.interfaces.insert(name.clone(), interface_methods);
            }
            Statement::ImportDecl {
                _names, source: _, ..
            } => {
                // Stdlib files will be processed as normal TejX files through explicit inclusion
            }
            Statement::ExportDecl { declaration, .. } => {
                self.collect_declarations(declaration);
            }
            _ => {}
        }
    }

    fn parse_signature(&self, type_name: String) -> (String, Vec<String>, bool) {
        let mut final_params = Vec::new();
        let mut final_type = type_name.clone();
        let mut is_variadic = false;

        let split_params = |params_str: &str| -> Vec<String> {
            let mut params = Vec::new();
            let mut current = String::new();
            let mut depth_brace = 0;
            let mut depth_angle = 0;
            let mut depth_paren = 0;

            for ch in params_str.chars() {
                match ch {
                    '{' => depth_brace += 1,
                    '}' => depth_brace -= 1,
                    '<' => depth_angle += 1,
                    '>' => {
                        if depth_angle > 0 {
                            depth_angle -= 1;
                        }
                    }
                    '(' => depth_paren += 1,
                    ')' => depth_paren -= 1,
                    ',' if depth_brace == 0 && depth_angle == 0 && depth_paren == 0 => {
                        params.push(current.trim().to_string());
                        current.clear();
                        continue;
                    }
                    _ => {}
                }
                current.push(ch);
            }
            if !current.trim().is_empty() {
                params.push(current.trim().to_string());
            }
            params
        };

        if type_name.starts_with("function:") {
            let parts: Vec<&str> = type_name.splitn(3, ':').collect();
            if parts.len() >= 3 {
                // function:ret_ty:p1,p2,p3
                final_type = format!("function:{}", parts[1]);
                let params = split_params(parts[2]);
                for mut p in params {
                    if p.ends_with("...") {
                        is_variadic = true;
                        p = p[..p.len() - 3].to_string();
                    }
                    if !p.is_empty() {
                        final_params.push(p);
                    }
                }
            }
        } else if type_name.contains("=>") {
            // (p1: t1, p2: t2) => ret
            if let Some(start) = type_name.find('(') {
                if let Some(end) = type_name.rfind(')') {
                    let params_str = &type_name[start + 1..end];
                    let params = split_params(params_str);
                    for p in params {
                        if p.ends_with("...") {
                            is_variadic = true;
                        }
                        let p = p.trim_end_matches("...").trim();
                        if let Some(colon) = p.find(':') {
                            final_params.push(p[colon + 1..].trim().to_string());
                        } else if !p.is_empty() {
                            final_params.push("any".to_string());
                        }
                    }
                    if let Some(arrow) = type_name.rfind("=>") {
                        let ret_part = &type_name[arrow + 2..].trim();
                        final_type = format!("function:{}", ret_part);
                    }
                }
            }
        }
        (final_type, final_params, is_variadic)
    }

    fn define(&mut self, name: String, type_name: String) {
        let (final_type, final_params, is_variadic) = self.parse_signature(type_name);
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name,
                Symbol {
                    type_name: final_type,
                    is_const: false,
                    params: final_params,
                    is_variadic,
                    aliased_type: None,
                    is_moved: false,
                },
            );
        }
    }

    fn define_with_params(&mut self, name: String, type_name: String, params: Vec<String>) {
        self.define_with_params_variadic(name, type_name, params, false);
    }

    fn define_with_params_variadic(
        &mut self,
        name: String,
        type_name: String,
        params: Vec<String>,
        is_variadic: bool,
    ) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name,
                Symbol {
                    type_name,
                    is_const: false,
                    params,
                    is_variadic,
                    aliased_type: None,
                    is_moved: false,
                },
            );
        }
    }

    fn define_variable(
        &mut self,
        name: String,
        type_name: String,
        is_const: bool,
        line: usize,
        col: usize,
    ) {
        // Parse signature first, as it only needs an immutable borrow of self
        let (final_type, final_params, is_variadic) = self.parse_signature(type_name);

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
                    type_name: final_type,
                    is_const,
                    params: final_params,
                    is_variadic,
                    aliased_type: None,
                    is_moved: false,
                },
            );
        }
    }

    // Check if variable is defined in ANY scope
    fn lookup(&self, name: &str) -> Option<Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(s) = scope.get(name) {
                return Some(s.clone());
            }
        }
        None
    }

    fn report_error_detailed(
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

    fn is_assignable(&self, target: &str, value: &str) -> bool {
        if target == "{unknown}" || value == "{unknown}" {
            return true; // prevent cascading errors
        }
        self.are_types_compatible(target, value)
    }

    fn is_valid_type(&self, type_name: &str) -> bool {
        if type_name == ""
            || type_name == "any"
            || type_name == "void"
            || type_name == "object"
            || type_name == "boolean"
            || type_name == "bool"
            || type_name == "string"
            || type_name == "int"
            || type_name == "float"
            || type_name == "char"
            || type_name == "None"
        {
            return true;
        }

        // SOI: Allow references as valid variable/argument types
        if type_name.starts_with("ref ") || type_name.starts_with("weak ") {
            return self.is_valid_type(&type_name[4..].trim_start())
                || self.is_valid_type(&type_name[5..].trim_start());
        }

        if type_name.contains('|') {
            return type_name
                .split('|')
                .all(|part| self.is_valid_type(part.trim()));
        }

        // Handle generic types: Type<Inner1, Inner2>
        if let Some(open) = type_name.find('<') {
            if type_name.ends_with('>') {
                let base = &type_name[..open];
                if !self.is_valid_type(base) {
                    return false;
                }

                let inner = &type_name[open + 1..type_name.len() - 1];
                let mut depth = 0;
                let mut start = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        ',' if depth == 0 => {
                            if !self.is_valid_type(inner[start..i].trim()) {
                                return false;
                            }
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                return self.is_valid_type(inner[start..].trim());
            }
        }

        // Handle Array types: number[], etc.
        if type_name.ends_with("[]") {
            let base = &type_name[..type_name.len() - 2];
            return self.is_valid_type(base);
        }

        // Handle fixed-size arrays: type[10]
        if type_name.ends_with("]") {
            if let Some(open) = type_name.find('[') {
                let base = &type_name[..open];
                return self.is_valid_type(base);
            }
        }

        // Handle function types: (a: T) => R
        if type_name.starts_with("(") && type_name.contains("=>") {
            // Simplified: if it looks like a function type, it's valid for now
            // or we could split and check return type
            return true;
        }

        // Handle object types: { a: T }
        if type_name.starts_with("{") && type_name.ends_with("}") {
            return true;
        }

        // Primitives
        let primitives = [
            "int", "int16", "int32", "int64", "int128", "float", "float16", "float32", "float64",
            "bool", "string", "char", "bigInt", "bigfloat", "object",
        ];
        if primitives.contains(&type_name) {
            return true;
        }

        // Defined in scopes (classes, interfaces, etc.)
        self.lookup(type_name).is_some()
    }

    fn is_numeric(&self, t: &str) -> bool {
        matches!(
            t,
            "int"
                | "int16"
                | "int32"
                | "int64"
                | "int128"
                | "float"
                | "float16"
                | "float32"
                | "float64"
        )
    }

    fn is_copy_type(&self, t: &str) -> bool {
        if t.starts_with("ref ")
            || t.starts_with("function:")
            || t == "function"
            || t == "any"
            || t == "object"
            || t == "string"
        {
            return true;
        }
        // Arrays ARE copy — they are heap-allocated but passed as reference pointers (i64 handles)
        // at runtime, so passing to functions borrows rather than moves ownership.
        if t.ends_with("[]") {
            return true;
        }
        if matches!(
            t,
            "int"
                | "int16"
                | "int32"
                | "int64"
                | "float"
                | "float32"
                | "float64"
                | "bool"
                | "boolean"
                | "char"
        ) {
            return true;
        }
        // Union types: if it's a union of copy types, it's copyable
        if t.contains('|') {
            return t.split('|').all(|s| self.is_copy_type(s.trim()));
        }
        // Generics: Node<int> should NOT be copy (it's a class)
        if let Some(angle) = t.find('<') {
            return self.is_copy_type(&t[..angle]);
        }
        // Anonymous objects (structs): copyable if all their members are copyable
        if t.starts_with('{') && t.ends_with('}') {
            let props = self.parse_struct_props(t);
            if props.is_empty() {
                return true;
            }
            return props.values().all(|v| self.is_copy_type(v));
        }

        // Built-in prelude classes are i64 handles at runtime — they use copy/borrow semantics
        let builtins = ["Array", "String", "Promise", "Error", "Map", "Set"];
        if builtins.contains(&t) {
            return true;
        }

        // User-defined classes use MOVE semantics — they are NOT copy
        false
    }

    fn get_common_ancestor(&self, t1: &str, t2: &str) -> String {
        if t1 == t2 {
            return t1.to_string();
        }
        if t1 == "{unknown}" {
            return t2.to_string();
        }
        if t2 == "{unknown}" {
            return t1.to_string();
        }
        if t1 == "any" || t2 == "any" {
            return "any".to_string();
        }

        let mut t1_ancestors = std::collections::HashSet::new();
        let mut curr = t1.to_string();
        t1_ancestors.insert(curr.clone());
        while let Some(parent) = self.class_hierarchy.get(&curr) {
            t1_ancestors.insert(parent.clone());
            curr = parent.clone();
        }

        curr = t2.to_string();
        if t1_ancestors.contains(&curr) {
            return curr;
        }
        while let Some(parent) = self.class_hierarchy.get(&curr) {
            if t1_ancestors.contains(parent) {
                return parent.clone();
            }
            curr = parent.clone();
        }
        "{unknown}".to_string()
    }

    fn are_types_compatible(&self, expected: &str, actual: &str) -> bool {
        // Fast path: exact match (handles T==T, Array<T>==Array<T>, etc.)
        if expected == actual {
            return true;
        }
        if expected.is_empty() || actual.is_empty() || expected == "any" || actual == "any" {
            return true;
        }

        // Built-in type aliases: String == string, Array == array, etc.
        fn normalize_builtin(t: &str) -> &str {
            match t {
                "String" => "string",
                "Int" => "int",
                "Float" => "float",
                "Bool" | "Boolean" | "boolean" => "bool",
                "Void" => "void",
                other => other,
            }
        }
        if normalize_builtin(expected) == normalize_builtin(actual) {
            return true;
        }

        // Generic type param wildcard: single uppercase letter (or short like K,V) defined as 'any'
        let is_generic_wildcard = |t: &str| -> bool {
            t.len() <= 2
                && t.chars().next().map_or(false, |c| c.is_uppercase())
                && t.chars().all(|c| c.is_alphanumeric())
        };
        if is_generic_wildcard(expected) || is_generic_wildcard(actual) {
            return true;
        }

        // Handle Optional<T> explicitly before generic base comparison
        if expected.starts_with("Optional<") && expected.ends_with(">") {
            let inner = &expected[9..expected.len() - 1]; // extract T
            if actual == "None" || self.are_types_compatible(inner, actual) {
                return true;
            }
        }

        // If types contain generic params, compare base types only
        // e.g. Array<T> vs Array<int> → compare Array vs Array
        if expected.contains('<') || actual.contains('<') {
            let e_base = expected.split('<').next().unwrap_or(expected);
            let a_base = actual.split('<').next().unwrap_or(actual);
            if e_base == a_base {
                return true;
            }
        }

        let e_is_ref = expected.starts_with("ref ");
        let a_is_ref = actual.starts_with("ref ");

        let expected_base = if e_is_ref {
            expected[4..].trim()
        } else {
            expected
        };
        let actual_base = if a_is_ref { actual[4..].trim() } else { actual };

        // Ownership logic: We cannot implicitly assign a borrowed reference to an owned slot without cloning
        // (unless the underlying type does not require drops, i.e. primitives which copy natively).
        if a_is_ref && !e_is_ref {
            // Let the underlying type signature compatibility fall through instead of aggressively blocking
        }

        let (e_norm, _, _) = self.parse_signature(expected_base.to_string());
        let (a_norm, _, _) = self.parse_signature(actual_base.to_string());

        let mut e_str = e_norm;
        let mut a_str = a_norm;

        // Normalize generics for compatibility check
        // Normalize generics for compatibility check - REMOVED to support generics
        // if e_str.contains('<') {
        //     e_str = e_str.split('<').next().unwrap_or(&e_str).to_string();
        // }
        // if a_str.contains('<') {
        //     a_str = a_str.split('<').next().unwrap_or(&a_str).to_string();
        // }

        let mut expected = e_str.as_str();
        let mut actual = a_str.as_str();

        // Resolve aliases
        let mut resolved_expected = String::new();
        if let Some(sym) = self.lookup(expected) {
            if let Some(aliased) = &sym.aliased_type {
                resolved_expected = aliased.clone();
            }
        }
        if !resolved_expected.is_empty() {
            // Re-parse signature if needed?
            // Simple replacement for now.
            // Ideally we should fully resolve recursively, but let's stick to one level or loop?
            // Since `lookup` finds the symbol, let's use the resolved type.
            // But we need to handle reference holding.
        }

        let mut resolved_actual = String::new();
        if let Some(sym) = self.lookup(actual) {
            if let Some(aliased) = &sym.aliased_type {
                resolved_actual = aliased.clone();
            }
        }

        let expected_storage;
        if !resolved_expected.is_empty() {
            expected_storage = resolved_expected;
            expected = expected_storage.as_str();
        }

        let actual_storage;
        if !resolved_actual.is_empty() {
            actual_storage = resolved_actual;
            actual = actual_storage.as_str();
        }

        // Re-check recursive aliases (simple loop)
        let mut loops = 0;
        while loops < 10 {
            let mut changed = false;
            if let Some(sym) = self.lookup(expected) {
                if let Some(aliased) = &sym.aliased_type {
                    e_str = aliased.clone();
                    expected = e_str.as_str();
                    changed = true;
                }
            }
            if let Some(sym) = self.lookup(actual) {
                if let Some(aliased) = &sym.aliased_type {
                    a_str = aliased.clone();
                    actual = a_str.as_str();
                    changed = true;
                }
            }
            if !changed {
                break;
            }
            loops += 1;
        }

        if expected == "any..." {
            return true;
        }

        if expected == "{unknown}"
            || actual == "{unknown}"
            || expected == "any"
            || actual == "any"
            || expected == ""
            || actual == ""
            || actual == "any:"
            || expected == "any:"
            || actual == "object"
            || expected == "any[]"
            || actual == "any[]"
        {
            return true;
        }
        if expected == actual {
            return true;
        }

        // Object normalization
        let e_norm_obj = expected.replace(" ", "").replace(";", ",");
        let a_norm_obj = actual.replace(" ", "").replace(";", ",");
        if e_norm_obj == a_norm_obj && !e_norm_obj.is_empty() {
            return true;
        }

        if expected == "object" && (actual.starts_with('{') || actual.starts_with("Map<")) {
            return true;
        }

        // char is compatible with string
        if (expected == "string" && actual == "char") || (expected == "char" && actual == "string")
        {
            return true;
        }

        // Handle union types (e.g., "TreeNode | None")
        if expected.contains('|') {
            for part in expected.split('|') {
                let part = part.trim();
                if self.are_types_compatible(part, actual) {
                    return true;
                }
            }
        }
        if actual.contains('|') {
            // If actual is a union, all its parts must be compatible with expected (strict)
            // But for now, let's be lenient or handle it if needed.
            // In these tests, we mostly pass T to T | None.
        }

        if actual == "None" && expected.contains("| None") {
            return true;
        }

        // Enum compatibility with int32
        let is_enum = |t: &str| {
            t == "enum"
                || self
                    .lookup(t)
                    .map(|s| s.type_name == "enum")
                    .unwrap_or(false)
        };
        let is_int = |t: &str| t == "int" || t == "int32";
        if (is_enum(expected) && is_int(actual)) || (is_enum(actual) && is_int(expected)) {
            return true;
        }

        if expected == "object" && actual.starts_with('{') {
            return true;
        }

        if expected.ends_with("[]") && actual.ends_with("[]") {
            let e_elem = &expected[..expected.len() - 2];
            let a_elem = &actual[..actual.len() - 2];
            if self.are_types_compatible(e_elem, a_elem) {
                return true;
            }
        }

        // Inheritance check
        if let Some(parent) = self.class_hierarchy.get(actual) {
            return self.are_types_compatible(expected, parent);
        }

        // Interface check
        if let Some(interfaces) = self.interfaces.get(expected) {
            // If expected is an interface, check if actual (class) implements all its methods
            if let Some(actual_members) = self.class_members.get(actual) {
                for method_name in interfaces.keys() {
                    if !actual_members.contains_key(method_name) {
                        return false;
                    }
                }
                return true;
            }
        }

        // Function type compatibility
        if expected == "function" && (actual == "function" || actual.starts_with("function:")) {
            return true;
        }

        if expected.starts_with("function:") && actual.starts_with("function:") {
            // For now, allow loosely (missing param types in lambda like 'function:any')
            if actual.contains(":any") || actual.ends_with(":") {
                return true;
            }
            // More strict check could be added here
        }

        // Alias check: int == int32
        let is_int_alias = |t: &str| t == "int" || t == "int32";
        if is_int_alias(expected) && is_int_alias(actual) {
            return true;
        }

        // Recursively check array types: int[] vs int32[]
        if expected.ends_with("[]") && actual.ends_with("[]") {
            let inner_expected = &expected[..expected.len() - 2];
            let inner_actual = &actual[..actual.len() - 2];
            return self.are_types_compatible(inner_expected, inner_actual);
        }

        // Check Array<T> vs T[] compatibility
        let is_array_class = |t: &str| t.starts_with("Array<") && t.ends_with(">");
        let is_array_syntax = |t: &str| t.ends_with("[]");

        if is_array_class(expected) && is_array_syntax(actual) {
            let inner_expected = &expected[6..expected.len() - 1];
            let inner_actual = &actual[..actual.len() - 2];
            return self.are_types_compatible(inner_expected, inner_actual);
        }
        if is_array_class(actual) && is_array_syntax(expected) {
            let inner_actual = &actual[6..actual.len() - 1];
            let inner_expected = &expected[..expected.len() - 2];
            return self.are_types_compatible(inner_expected, inner_actual);
        }

        if is_array_class(expected) && is_array_class(actual) {
            let inner_expected = &expected[6..expected.len() - 1];
            let inner_actual = &actual[6..actual.len() - 1];
            return self.are_types_compatible(inner_expected, inner_actual);
        }

        let is_fixed_array = |t: &str| t.ends_with("]") && t.contains("[") && !t.ends_with("[]");
        if expected.ends_with("[]") && is_fixed_array(actual) {
            let inner_expected = &expected[..expected.len() - 2];
            let inner_actual = actual.split('[').next().unwrap_or("");
            return self.are_types_compatible(inner_expected, inner_actual);
        }
        if actual.ends_with("[]") && is_fixed_array(expected) {
            let inner_actual = &actual[..actual.len() - 2];
            let inner_expected = expected.split('[').next().unwrap_or("");
            return self.are_types_compatible(inner_expected, inner_actual);
        }
        if is_array_class(expected) && is_fixed_array(actual) {
            let inner_expected = &expected[6..expected.len() - 1];
            let inner_actual = actual.split('[').next().unwrap_or("");
            return self.are_types_compatible(inner_expected, inner_actual);
        }
        if is_array_class(actual) && is_fixed_array(expected) {
            let inner_actual = &actual[6..actual.len() - 1];
            let inner_expected = expected.split('[').next().unwrap_or("");
            return self.are_types_compatible(inner_expected, inner_actual);
        }

        // Empty array assignment
        if actual == "[]" && expected.ends_with("[]") {
            return true;
        }

        // Numeric compatibility (implicit casts)
        let is_numeric = |t: &str| -> bool {
            matches!(
                t,
                "int"
                    | "int16"
                    | "int32"
                    | "int64"
                    | "int128"
                    | "float"
                    | "float16"
                    | "float32"
                    | "float64"
            )
        };
        if is_numeric(expected) && is_numeric(actual) {
            return true;
        }

        // Check if actual is a raw type compatible with expected generic type
        if expected.contains('<') && !actual.contains('<') {
            let base_expected = expected.split('<').next().unwrap_or("");
            if base_expected == actual {
                return true;
            }
        }

        // Check if actual is 'Array' and expected is an array type (T[])
        if actual == "Array" && expected.ends_with("[]") {
            return true;
        }

        // Structural Object Type check
        if expected.starts_with("{")
            && expected.ends_with("}")
            && actual.starts_with("{")
            && actual.ends_with("}")
        {
            let e_props = self.parse_struct_props(expected);
            let a_props = self.parse_struct_props(actual);

            if e_props.len() != a_props.len() {
                return false;
            }
            for (k, expected_k_ty) in &e_props {
                if let Some(actual_k_ty) = a_props.get(k) {
                    if !self.are_types_compatible(expected_k_ty, actual_k_ty) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            return true;
        }

        false
    }

    fn parse_struct_props(&self, s: &str) -> HashMap<String, String> {
        let mut props = HashMap::new();
        let s = s.trim();
        if !s.starts_with('{') || !s.ends_with('}') {
            return props;
        }
        let inner = s[1..s.len() - 1].trim();
        if inner.is_empty() {
            return props;
        }

        let mut brace_level = 0;
        let mut bracket_level = 0;
        let mut angle_level = 0;
        let mut current_key = String::new();
        let mut current_val = String::new();
        let mut parsing_key = true;

        for c in inner.chars() {
            match c {
                '{' => {
                    brace_level += 1;
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
                '}' => {
                    brace_level -= 1;
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
                '[' => {
                    bracket_level += 1;
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
                ']' => {
                    bracket_level -= 1;
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
                '<' => {
                    angle_level += 1;
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
                '>' => {
                    angle_level -= 1;
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
                ':' if brace_level == 0
                    && bracket_level == 0
                    && angle_level == 0
                    && parsing_key =>
                {
                    parsing_key = false;
                }
                ',' | ';' if brace_level == 0 && bracket_level == 0 && angle_level == 0 => {
                    if !current_key.trim().is_empty() {
                        props.insert(
                            current_key.trim().to_string(),
                            current_val.trim().to_string(),
                        );
                    }
                    current_key.clear();
                    current_val.clear();
                    parsing_key = true;
                }
                _ => {
                    if parsing_key {
                        current_key.push(c);
                    } else {
                        current_val.push(c);
                    }
                }
            }
        }
        if !current_key.trim().is_empty() {
            props.insert(
                current_key.trim().to_string(),
                current_val.trim().to_string(),
            );
        }
        props
    }

    fn check_statement(&mut self, stmt: &Statement) -> Result<(), ()> {
        match stmt {
            Statement::VarDeclaration {
                pattern,
                type_annotation,
                initializer,
                is_const,
                line,
                _col,
            } => {
                if !self.is_valid_type(type_annotation) {
                    self.report_error_detailed(format!("Unknown data type: '{}'", type_annotation), *line, *_col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, or user-defined classes"));
                }
                if let Some(expr) = initializer {
                    let init_type = self.check_expression(expr)?;
                    if !type_annotation.is_empty()
                        && !self.are_types_compatible(type_annotation, &init_type)
                    {
                        self.report_error_detailed(
                            format!(
                                "Type mismatch: expected '{}', got '{}'",
                                type_annotation, init_type
                            ),
                            *line,
                            *_col,
                            "E0100",
                            Some(&format!(
                                "Consider converting with 'as {}' or change the variable type",
                                type_annotation
                            )),
                        );
                    }

                    // Handle Move Semantics: If initializer is an Identifier
                    if let Expression::Identifier { name: src_name, .. } = &**expr {
                        let is_copy_type = |t: &str| -> bool {
                            t.starts_with("ref ")
                                || matches!(
                                    t,
                                    "int"
                                        | "int16"
                                        | "int32"
                                        | "int64"
                                        | "float"
                                        | "float64"
                                        | "bool"
                                        | "char"
                                )
                        };
                        if !is_copy_type(&init_type) && init_type != "any" {
                            // SOI: Do not mark moved if we are explicitly assigning to a reference!
                            let is_ref_assignment = type_annotation.starts_with("ref ")
                                || type_annotation.starts_with("weak ");
                            if !is_ref_assignment {
                                self.mark_moved(src_name, *line, *_col);
                            }
                        }
                    }

                    if type_annotation == "any" || type_annotation == "" {
                        if type_annotation == "" && init_type != "any" {
                            let _ = self.define_pattern(
                                pattern,
                                init_type.clone(),
                                *is_const,
                                *line,
                                *_col,
                            );
                        } else {
                            let _ = self.define_pattern(
                                pattern,
                                "any".to_string(),
                                *is_const,
                                *line,
                                *_col,
                            );
                        }
                    } else {
                        let _ = self.define_pattern(
                            pattern,
                            type_annotation.clone(),
                            *is_const,
                            *line,
                            *_col,
                        );
                    }
                } else {
                    if type_annotation == "any" || type_annotation == "" {
                        let _ = self.define_pattern(
                            pattern,
                            "any".to_string(),
                            *is_const,
                            *line,
                            *_col,
                        );
                    } else {
                        let _ = self.define_pattern(
                            pattern,
                            type_annotation.clone(),
                            *is_const,
                            *line,
                            *_col,
                        );
                    }
                }
                Ok(())
            }
            Statement::ExpressionStmt {
                _expression: expression,
                ..
            } => {
                self.check_expression(expression)?;
                Ok(())
            }
            Statement::BlockStmt { statements, .. } => {
                self.enter_scope();
                for (i, s) in statements.iter().enumerate() {
                    // SOI: Store remaining statements for look-ahead
                    self.remaining_stmts = statements[i + 1..].to_vec();
                    let _ = self.check_statement(s);
                }
                self.remaining_stmts.clear();
                self.exit_scope();
                Ok(())
            }
            Statement::IfStmt {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let _ = self.check_expression(condition)?;

                // Attempt type narrowing
                if let Some((name, narrowed_type, other_type)) =
                    self.get_narrowing_from_condition(condition)
                {
                    // Then branch narrowing
                    self.enter_scope();
                    if !narrowed_type.is_empty() {
                        self.define(name.clone(), narrowed_type);
                    }
                    self.check_statement(then_branch)?;
                    self.exit_scope();

                    // Else branch narrowing
                    if let Some(else_stmt) = else_branch {
                        self.enter_scope();
                        if !other_type.is_empty() {
                            self.define(name.clone(), other_type);
                        }
                        self.check_statement(else_stmt)?;
                        self.exit_scope();
                    }
                } else {
                    self.check_statement(then_branch)?;
                    if let Some(else_stmt) = else_branch {
                        self.check_statement(else_stmt)?;
                    }
                }
                Ok(())
            }
            Statement::WhileStmt {
                condition, body, ..
            } => {
                self.check_expression(condition)?;
                self.loop_depth += 1;

                // Two-pass check for move semantics in loops
                let _ = self.check_statement(body);
                let res = self.check_statement(body);

                self.loop_depth -= 1;
                res
            }
            Statement::ForStmt {
                init,
                condition,
                increment,
                body,
                ..
            } => {
                self.enter_scope();
                if let Some(init_stmt) = init {
                    // Special case: if init is a BlockStmt (e.g. from multiple declarations),
                    // we don't want it to start a nested scope that ends before the loop starts.
                    if let Statement::BlockStmt { statements, .. } = init_stmt.as_ref() {
                        for s in statements {
                            self.check_statement(s)?;
                        }
                    } else {
                        self.check_statement(init_stmt)?;
                    }
                }
                if let Some(cond_expr) = condition {
                    self.check_expression(cond_expr)?;
                }

                self.loop_depth += 1;
                // Two-pass check for move semantics in loops
                let _ = self.check_statement(body);
                if let Some(inc_expr) = increment {
                    let _ = self.check_expression(inc_expr);
                }

                let res = self.check_statement(body);
                if let Some(inc_expr) = increment {
                    self.check_expression(inc_expr)?;
                }
                self.loop_depth -= 1;

                self.exit_scope();
                res
            }
            Statement::BreakStmt { _line, _col } => {
                if self.loop_depth == 0 {
                    self.report_error_detailed(
                        "'break' can only be used inside a loop".to_string(),
                        *_line,
                        *_col,
                        "E0112",
                        Some("'break' can only be used inside 'for' or 'while' loops"),
                    );
                }
                Ok(())
            }
            Statement::ContinueStmt { _line, _col } => {
                if self.loop_depth == 0 {
                    self.report_error_detailed(
                        "'continue' can only be used inside a loop".to_string(),
                        *_line,
                        *_col,
                        "E0112",
                        Some("'continue' can only be used inside 'for' or 'while' loops"),
                    );
                }
                Ok(())
            }
            Statement::FunctionDeclaration(func) => {
                let mut ret_ty = if func.return_type.is_empty() {
                    "any".to_string()
                } else {
                    func.return_type.clone()
                };
                if func._is_async && !ret_ty.starts_with("Promise<") {
                    ret_ty = format!("Promise<{}>", ret_ty);
                }
                let mut is_variadic = false;
                let params = func
                    .params
                    .iter()
                    .map(|p| {
                        if p._is_rest {
                            is_variadic = true;
                        }
                        p.type_name.clone()
                    })
                    .collect();
                self.define_with_params_variadic(
                    func.name.clone(),
                    format!("function:{}", ret_ty),
                    params,
                    is_variadic,
                );

                self.current_function_return = Some(ret_ty);
                self.current_function_is_async = func._is_async;
                self.enter_scope();
                // Register function-level generic params as valid types
                for gp in &func.generic_params {
                    self.define(gp.clone(), "any".to_string());
                }
                for param in &func.params {
                    self.define_with_params(
                        param.name.clone(),
                        param.type_name.clone(),
                        Vec::new(),
                    );
                }
                self.check_statement(&func.body)?;
                self.exit_scope();
                self.current_function_return = None;
                self.current_function_is_async = false;
                Ok(())
            }
            Statement::ClassDeclaration(class_decl) => {
                self.current_class = Some(class_decl.name.clone());
                self.define(class_decl.name.clone(), "class".to_string());

                // Verify parent exists
                if !class_decl._parent_name.is_empty() {
                    if self.lookup(&class_decl._parent_name).is_none() {
                        self.report_error_detailed(
                            format!("Parent class '{}' not found", class_decl._parent_name),
                            class_decl._line,
                            class_decl._col,
                            "E0101",
                            Some("Ensure the parent class is defined before the child class"),
                        );
                    }
                }

                // Verify interface implementation
                for interface_name in &class_decl._implemented_protocols {
                    if let Some(_) = self.lookup(interface_name) {
                        let required_methods = self.interfaces.get(interface_name).cloned();
                        if let Some(req_methods) = required_methods {
                            let mut class_method_names = Vec::new();
                            for m in &class_decl.methods {
                                class_method_names.push(m.func.name.clone());
                            }
                            for (req_name, _) in req_methods {
                                if !class_method_names.contains(&req_name) {
                                    self.report_error_detailed(format!("Class '{}' missing method '{}' required by interface '{}'", class_decl.name, req_name, interface_name), class_decl._line, class_decl._col, "E0111", Some(&format!("Add method '{}' to class '{}' to satisfy the interface contract", req_name, class_decl.name)));
                                }
                            }
                        }
                    } else {
                        self.report_error_detailed(format!("Interface '{}' not found", interface_name), class_decl._line, class_decl._col, "E0101", Some("Define the interface before implementing it, or check the spelling"));
                    }
                }

                self.enter_scope();
                self.define("this".to_string(), class_decl.name.clone());
                // Register class-level generic params as valid types
                for gp in &class_decl.generic_params {
                    self.define(gp.clone(), "any".to_string());
                }
                if !class_decl._parent_name.is_empty() {
                    self.define("super".to_string(), class_decl._parent_name.clone());
                }

                for method in &class_decl.methods {
                    self.enter_scope();
                    // Register method-level generic params as valid types
                    for gp in &method.func.generic_params {
                        self.define(gp.clone(), "any".to_string());
                    }
                    for param in &method.func.params {
                        if !self.is_valid_type(&param.type_name) {
                            self.report_error_detailed(format!("Unknown data type: '{}'", param.type_name), class_decl._line, class_decl._col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, or user-defined classes"));
                        }
                        self.define(param.name.clone(), param.type_name.clone());
                    }
                    let prev_return = self.current_function_return.take();
                    let prev_async = self.current_function_is_async;
                    if !self.is_valid_type(&method.func.return_type) {
                        self.report_error_detailed(format!("Unknown data type: '{}' for return type of method '{}'", method.func.return_type, method.func.name), class_decl._line, class_decl._col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, void, or user-defined classes"));
                    }
                    let ret_ty = if method.func.return_type.is_empty() {
                        "any".to_string()
                    } else {
                        method.func.return_type.clone()
                    };
                    self.current_function_return = Some(ret_ty);
                    self.current_function_is_async = method.func._is_async;

                    self.check_statement(&method.func.body)?;

                    self.current_function_return = prev_return;
                    self.current_function_is_async = prev_async;
                    self.exit_scope();
                }

                if let Some(constructor) = &class_decl._constructor {
                    self.enter_scope();
                    for param in &constructor.params {
                        self.define(param.name.clone(), param.type_name.clone());
                    }
                    let prev_return = self.current_function_return.take();
                    self.current_function_return = Some("void".to_string());
                    self.check_statement(&constructor.body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                for getter in &class_decl._getters {
                    self.enter_scope();
                    let prev_return = self.current_function_return.take();
                    self.current_function_return = Some(getter._return_type.clone());
                    self.check_statement(&getter._body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                for setter in &class_decl._setters {
                    self.enter_scope();
                    self.define(setter._param_name.clone(), setter._param_type.clone());
                    let prev_return = self.current_function_return.take();
                    self.current_function_return = Some("void".to_string());
                    self.check_statement(&setter._body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                self.exit_scope();
                self.current_class = None;
                Ok(())
            }
            Statement::ReturnStmt {
                value,
                _line: line,
                _col: col,
            } => {
                let expected = self.current_function_return.clone();
                if let Some(expected_original) = expected {
                    let got = if let Some(expr) = value {
                        self.check_expression(expr)?
                    } else {
                        "void".to_string()
                    };

                    let expected_type = expected_original.clone();
                    // If async, expected_type is Promise<T>, but we allow returning T
                    if self.current_function_is_async && expected_type.starts_with("Promise<") {
                        let inner = &expected_type[8..expected_type.len() - 1];
                        let is_numeric = |t: &str| -> bool {
                            matches!(
                                t,
                                "int"
                                    | "int16"
                                    | "int32"
                                    | "int64"
                                    | "int128"
                                    | "float"
                                    | "float16"
                                    | "float32"
                                    | "float64"
                            )
                        };
                        let is_bool = |t: &str| -> bool { matches!(t, "bool") };

                        if inner == "any"
                            || got == "any"
                            || got == inner
                            || (is_numeric(inner) && is_numeric(&got))
                            || (is_bool(inner) && is_bool(&got))
                        {
                            // Implicit wrap: OK
                            return Ok(());
                        }
                    }

                    if expected_type != "any"
                        && got != "any"
                        && !self.is_assignable(&expected_type, &got)
                    {
                        let is_numeric = |t: &str| -> bool {
                            matches!(
                                t,
                                "int"
                                    | "int16"
                                    | "int32"
                                    | "int64"
                                    | "int128"
                                    | "float"
                                    | "float16"
                                    | "float32"
                                    | "float64"
                            )
                        };
                        let is_bool = |t: &str| -> bool { matches!(t, "bool") };

                        if (is_numeric(&expected_type) && is_numeric(&got))
                            || (is_bool(&expected_type) && is_bool(&got))
                        {
                            // Ok
                        } else {
                            self.report_error_detailed(format!("Return type mismatch: expected '{}', got '{}'", expected_original, got), *line, *col, "E0107", Some(&format!("The function signature declares return type '{}'; ensure the returned value matches", expected_original)));
                        }
                    }
                }
                Ok(())
            }
            Statement::EnumDeclaration(enum_decl) => {
                self.define(enum_decl.name.clone(), "enum".to_string());
                // Define members as static properties of enum?
                // For simplified type check, just defining enum name is enough to pass basic checks.
                Ok(())
            }
            Statement::TypeAliasDeclaration { .. } => {
                // Already handled in collect_declarations
                Ok(())
            }
            Statement::InterfaceDeclaration { name, .. } => {
                self.define(name.clone(), "interface".to_string());
                Ok(())
            }
            Statement::ExportDecl { declaration, .. } => {
                self.check_statement(declaration)?;
                Ok(())
            }
            Statement::ImportDecl {
                _names, source: _, ..
            } => {
                // Previously, this contained hardcoded globals and function definitions for `std:` modules.
                // We now rely on the lowering phase to actually include the `.tx` stdlib files and typecheck those directly.
                Ok(())
            }
            Statement::ExtensionDeclaration(ext_decl) => {
                let name = &ext_decl._target_type;
                let methods = &ext_decl._methods;

                let mut existing_members = self
                    .class_members
                    .get(name)
                    .cloned()
                    .unwrap_or(HashMap::new());
                for method in methods {
                    let m_name = &method.name;
                    // Build method type string
                    let mut param_types = Vec::new();
                    for p in &method.params {
                        param_types.push(p.type_name.clone());
                    }
                    let p_str = param_types.join(",");
                    let type_str = format!("function:{}:{}", method.return_type, p_str);

                    existing_members.insert(
                        m_name.clone(),
                        MemberInfo {
                            type_name: type_str,
                            is_static: false,
                            access: AccessLevel::Public,
                            is_readonly: true,
                        },
                    );

                    // Check method body
                    let prev_class = self.current_class.clone();
                    self.current_class = Some(name.clone());

                    self.enter_scope();
                    self.define("this".to_string(), name.clone());

                    for param in &method.params {
                        self.define(param.name.clone(), param.type_name.clone());
                    }

                    let prev_return = self.current_function_return.take();
                    let prev_async = self.current_function_is_async;

                    let ret_ty = if method.return_type.is_empty() {
                        "any".to_string()
                    } else {
                        method.return_type.clone()
                    };
                    self.current_function_return = Some(ret_ty);
                    self.current_function_is_async = method._is_async;

                    self.check_statement(&method.body)?;

                    self.current_function_return = prev_return;
                    self.current_function_is_async = prev_async;

                    self.exit_scope();
                    self.current_class = prev_class;
                }
                self.class_members.insert(name.clone(), existing_members);
                Ok(())
            }
            // Statement::ProtocolDeclaration(_) => Ok(()), // Removed
            _ => Ok(()), // Catch-all for others
        }
    }

    fn substitute_generics(&self, member_type: &str, obj_type: &str) -> String {
        let mut parts = Vec::new();
        if obj_type.ends_with("[]") {
            parts.push(&obj_type[..obj_type.len() - 2]);
        } else if let Some(open) = obj_type.find('<') {
            if let Some(close) = obj_type.rfind('>') {
                let inner = &obj_type[open + 1..close];
                // Split inner by comma, respecting nested generics and object literals
                let mut start = 0;
                let mut depth = 0;
                let mut depth_brace = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        '{' => depth_brace += 1,
                        '}' => depth_brace -= 1,
                        ',' if depth == 0 && depth_brace == 0 => {
                            parts.push(inner[start..i].trim());
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                parts.push(inner[start..].trim());
            }
        }

        let mut result = member_type.to_string();
        // Check for $0, $1... up to some reasonable limit or until no more are found
        for i in 0..5 {
            let placeholder = format!("${}", i);
            if result.contains(&placeholder) {
                let replacement = if i < parts.len() { parts[i] } else { "any" };
                result = result.replace(&placeholder, replacement);
            }
        }

        result
    }

    fn parameterize_generics(&self, type_name: &str, params: &Vec<String>) -> String {
        let mut result = type_name.to_string();
        for (i, param) in params.iter().enumerate() {
            let placeholder = format!("${}", i);
            let mut new_res = String::new();
            let mut last_pos = 0;
            let p_len = param.len();

            while let Some(idx) = result[last_pos..].find(param) {
                let abs_idx = last_pos + idx;
                // Fix indexing: operate on byte slices
                let before_char = if abs_idx > 0 {
                    result[..abs_idx].chars().last()
                } else {
                    None
                };
                let after_char = result[abs_idx + p_len..].chars().next();

                let is_word_start = match before_char {
                    Some(c) => !c.is_alphanumeric() && c != '_',
                    None => true,
                };
                let is_word_end = match after_char {
                    Some(c) => !c.is_alphanumeric() && c != '_',
                    None => true,
                };

                new_res.push_str(&result[last_pos..abs_idx]);

                if is_word_start && is_word_end {
                    new_res.push_str(&placeholder);
                } else {
                    new_res.push_str(param);
                }
                last_pos = abs_idx + p_len;
            }
            new_res.push_str(&result[last_pos..]);
            result = new_res;
        }
        result
    }

    fn resolve_instance_member(&self, obj_type: &str, member: &str) -> Option<MemberInfo> {
        let mut unwrapped_type = obj_type.to_string();
        if obj_type.contains('|') {
            unwrapped_type = obj_type
                .split('|')
                .map(|s| s.trim().to_string())
                .find(|s| s != "None" && !s.is_empty())
                .unwrap_or(obj_type.to_string());
        }

        let mut current_type = if unwrapped_type.ends_with("[]") {
            "Array".to_string()
        } else if unwrapped_type.contains('[') && unwrapped_type.ends_with(']') {
            // Fixed-size arrays like int32[5] should also map to Array
            "Array".to_string()
        } else if unwrapped_type.starts_with("Array<") {
            // Generic Array<T> maps to Array
            "Array".to_string()
        } else if unwrapped_type == "string" {
            "String".to_string()
        } else if unwrapped_type.starts_with('{') {
            // Object literals map to Map
            "Map".to_string()
        } else if unwrapped_type == "enum"
            || self
                .lookup(&unwrapped_type)
                .map(|s| s.type_name == "enum")
                .unwrap_or(false)
        {
            unwrapped_type.clone()
        } else {
            unwrapped_type.clone()
        };

        // Normalize generic types: Node<int> -> Node
        if !self.class_members.contains_key(&current_type) {
            if let Some(angle) = current_type.find('<') {
                current_type = current_type[..angle].to_string();
            }
        }

        while !current_type.is_empty() && current_type != "any" {
            if let Some(members) = self.class_members.get(&current_type) {
                if let Some(info) = members.get(member).cloned() {
                    return Some(info);
                }
            }

            // Check if it's an interface
            if let Some(members) = self.interfaces.get(&current_type) {
                if let Some(info) = members.get(member).cloned() {
                    return Some(info);
                }
            }

            // Follow hierarchy
            if let Some(parent) = self.class_hierarchy.get(&current_type) {
                current_type = parent.clone();
            } else {
                break;
            }
        }
        None
    }

    fn mark_moved(&mut self, name: &str, _line: usize, _col: usize) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(s) = scope.get_mut(name) {
                if s.is_moved {
                    // Already moved
                } else {
                    s.is_moved = true;
                }
                return;
            }
        }
    }

    fn unmark_moved(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(s) = scope.get_mut(name) {
                s.is_moved = false;
                return;
            }
        }
    }

    fn strip_none_from_union(&self, type_name: &str) -> String {
        if !type_name.contains('|') {
            return type_name.to_string();
        }
        let parts: Vec<&str> = type_name.split('|').map(|s| s.trim()).collect();
        let filtered: Vec<&str> = parts.into_iter().filter(|&p| p != "None").collect();
        if filtered.len() == 1 {
            filtered[0].to_string()
        } else {
            filtered.join(" | ")
        }
    }

    fn get_narrowing_from_condition(
        &self,
        condition: &Expression,
    ) -> Option<(String, String, String)> {
        match condition {
            Expression::BinaryExpr {
                left, op, right, ..
            } => {
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

                if let Some(sym) = self.lookup(&name) {
                    let original_type = sym.type_name.clone();
                    if original_type.contains('|') {
                        let non_none = self.strip_none_from_union(&original_type);
                        if is_not_none {
                            // then: non_none, else: None
                            return Some((name, non_none, "None".to_string()));
                        } else {
                            // then: None, else: non_none
                            return Some((name, "None".to_string(), non_none));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_expression(&mut self, expr: &Expression) -> Result<String, ()> {
        match expr {
            Expression::NumberLiteral { value, .. } => {
                if value.fract() == 0.0 {
                    Ok("int32".to_string())
                } else {
                    Ok("float32".to_string())
                }
            }
            Expression::StringLiteral { .. } => Ok("string".to_string()),
            Expression::BooleanLiteral { .. } => Ok("bool".to_string()),
            Expression::NoneLiteral { .. } => Ok("None".to_string()),
            Expression::SomeExpr { value, .. } => {
                let inner = self.check_expression(value)?;
                Ok(inner) // Transparent for now, or maybe wrap in Option<T>?
            }
            Expression::UnaryExpr {
                op,
                right,
                _line,
                _col,
            } => {
                let right_type = self.check_expression(right)?;
                match op {
                    TokenType::Bang => Ok("bool".to_string()),
                    TokenType::Minus => {
                        let is_numeric = |t: &str| -> bool {
                            matches!(
                                t,
                                "int"
                                    | "int16"
                                    | "int32"
                                    | "int64"
                                    | "int128"
                                    | "float"
                                    | "float16"
                                    | "float32"
                                    | "float64"
                            )
                        };
                        if is_numeric(&right_type) || right_type == "any" {
                            Ok(right_type)
                        } else {
                            self.report_error_detailed(
                                format!("Unary '-' cannot be applied to type '{}'", right_type),
                                *_line,
                                *_col,
                                "E0100",
                                Some("Unary negation only works on numeric types (int, float)"),
                            );
                            Ok("any".to_string())
                        }
                    }
                    TokenType::PlusPlus | TokenType::MinusMinus => Ok(right_type),
                    _ => Ok("any".to_string()),
                }
            }
            Expression::ThisExpr { _line, _col } => {
                if let Some(c) = &self.current_class {
                    Ok(c.clone())
                } else {
                    self.report_error_detailed(
                        "Using 'this' outside of a class".to_string(),
                        *_line,
                        *_col,
                        "E0115",
                        Some("'this' can only be used inside class methods or constructors"),
                    );
                    Ok("any".to_string())
                }
            }
            Expression::LambdaExpr { params, body, .. } => {
                self.enter_scope();
                for p in params {
                    self.define(p.name.clone(), p.type_name.clone());
                }

                let old_return = self.current_function_return.clone();
                self.current_function_return = Some("any".to_string());

                self.check_statement(body)?;

                self.current_function_return = old_return;
                self.exit_scope();

                let param_types: Vec<String> = params.iter().map(|p| p.type_name.clone()).collect();
                Ok(format!("function:any:{}", param_types.join(",")))
            }
            Expression::Identifier { name, _line, _col } => {
                if let Some(s) = self.lookup(name) {
                    if s.is_moved {
                        self.report_error_detailed(format!("Use of moved variable '{}'", name), *_line, *_col, "E0103", Some("Value was moved to another variable; consider cloning before the move"));
                    }
                    Ok(s.type_name)
                } else {
                    if name == "console" {
                        return Ok("Console".to_string());
                    }
                    self.report_error_detailed(
                        format!("Undefined variable '{}'", name),
                        *_line,
                        *_col,
                        "E0102",
                        Some("Check the spelling or ensure the variable is declared before use"),
                    );
                    Ok("any".to_string())
                }
            }
            Expression::CastExpr {
                expr, target_type, ..
            } => {
                let _expr_type = self.check_expression(expr)?;
                Ok(target_type.clone())
            }
            Expression::BinaryExpr {
                left,
                op,
                right,
                _line,
                _col,
            } => {
                let left_type = self.check_expression(left)?;
                let right_type = self.check_expression(right)?;

                let is_float =
                    |t: &str| -> bool { matches!(t, "float" | "float16" | "float32" | "float64") };

                if left_type == "string" || right_type == "string" {
                    if matches!(op, TokenType::Plus) {
                        return Ok("string".to_string());
                    }
                    if matches!(
                        op,
                        TokenType::EqualEqual
                            | TokenType::BangEqual
                            | TokenType::EqualEqualEqual
                            | TokenType::BangEqualEqual
                    ) {
                        return Ok("bool".to_string());
                    }
                    if matches!(
                        op,
                        TokenType::Less
                            | TokenType::Greater
                            | TokenType::LessEqual
                            | TokenType::GreaterEqual
                    ) {
                        return Ok("bool".to_string());
                    }
                    self.report_error_detailed(
                        format!(
                            "Binary operator '{:?}' cannot be applied to type 'string'",
                            op
                        ),
                        *_line,
                        *_col,
                        "E0100",
                        Some(
                            "Use string methods for comparison, or convert to a numeric type first",
                        ),
                    );
                    return Ok("any".to_string());
                }

                if self.is_numeric(&left_type) && self.is_numeric(&right_type) {
                    if matches!(
                        op,
                        TokenType::EqualEqual
                            | TokenType::BangEqual
                            | TokenType::Less
                            | TokenType::LessEqual
                            | TokenType::Greater
                            | TokenType::GreaterEqual
                    ) {
                        return Ok("bool".to_string());
                    }

                    if matches!(
                        op,
                        TokenType::Minus | TokenType::Star | TokenType::Slash | TokenType::Plus
                    ) {
                        if left_type == "float64" || right_type == "float64" {
                            return Ok("float64".to_string());
                        }
                        if is_float(&left_type) || is_float(&right_type) {
                            return Ok("float32".to_string());
                        }
                        if left_type == "int64" || right_type == "int64" {
                            return Ok("int64".to_string());
                        }
                        return Ok("int32".to_string());
                    }
                }

                // Boolean result for comparisons and logic
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
                        | TokenType::Instanceof
                ) {
                    return Ok("bool".to_string());
                }

                Ok("int32".to_string())
            }
            Expression::MemberAccessExpr {
                object,
                member,
                _line,
                _col,
                ..
            } => {
                let mut obj_type = self.check_expression(object)?;

                if obj_type.starts_with("ref ") {
                    obj_type = obj_type[4..].to_string();
                }

                // Resolve alias if needed
                if let Some(sym) = self.lookup(&obj_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        obj_type = aliased.clone();
                    }
                }

                // Special case for class names (static access)
                if let Expression::Identifier { name, .. } = &**object {
                    if let Some(s) = self.lookup(name) {
                        if s.type_name == "class" || s.type_name == "enum" {
                            if let Some(members) = self.class_members.get(name) {
                                if let Some(info) = members.get(member).cloned() {
                                    if !info.is_static {
                                        self.report_error_detailed(format!("Member '{}' is not static", member), *_line, *_col, "E0116", Some("Access this member on an instance, not the class itself"));
                                    }
                                    return Ok(self.substitute_generics(&info.type_name, name));
                                }
                            }
                        }
                    }
                }

                // Instance access
                if let Some(info) = self.resolve_instance_member(&obj_type, member) {
                    if info.is_static {
                        self.report_error_detailed(format!("Static member '{}' accessed on instance", member), *_line, *_col, "E0116", Some("Access static members using the class name, e.g., ClassName.member"));
                    }
                    if info.access == AccessLevel::Private {
                        if let Some(current) = &self.current_class {
                            if current != &obj_type && !obj_type.starts_with("function") {
                                if current != &obj_type {
                                    // Check hierarchy if needed, but for now simple check
                                    self.report_error_detailed(format!("Member '{}' is private and can only be accessed within class '{}'", member, obj_type), *_line, *_col, "E0106", Some("Mark the member as 'public' in the class definition, or access it from within the class"));
                                }
                            }
                        } else {
                            self.report_error_detailed(format!("Member '{}' is private and can only be accessed within class '{}'", member, obj_type), *_line, *_col, "E0106", Some("Mark the member as 'public' in the class definition, or access it from within the class"));
                        }
                    }
                    return Ok(self.substitute_generics(&info.type_name, &obj_type));
                }

                if obj_type != "any"
                    && !obj_type.is_empty()
                    && obj_type != "object"
                    && !obj_type.starts_with("{")
                {
                    // Fallback for enums: default to int32 if known enum
                    if obj_type == "enum"
                        || self
                            .lookup(&obj_type)
                            .map(|s| s.type_name == "enum")
                            .unwrap_or(false)
                    {
                        return Ok("int32".to_string());
                    }
                    self.report_error_detailed(
                        format!(
                            "Property '{}' does not exist on type '{}'",
                            member, obj_type
                        ),
                        *_line,
                        *_col,
                        "E0105",
                        Some(&format!(
                            "Check the spelling or add '{}' as a member of class '{}'",
                            member, obj_type
                        )),
                    );
                }
                Ok("any".to_string())
            }
            Expression::SequenceExpr { expressions, .. } => {
                let mut last_type = "any".to_string();
                for expr in expressions {
                    last_type = self.check_expression(expr)?;
                }
                Ok(last_type)
            }
            Expression::ArrayAccessExpr {
                target,
                index,
                _line,
                _col,
            } => {
                let target_type = self.check_expression(target)?;
                self.check_expression(index)?;

                let mut unwrapped_type = target_type.clone();
                if target_type.contains('|') {
                    unwrapped_type = target_type
                        .split('|')
                        .map(|s| s.trim().to_string())
                        .find(|s| s != "None" && !s.is_empty())
                        .unwrap_or(target_type.clone());
                }

                if unwrapped_type.starts_with("ref ") {
                    unwrapped_type = unwrapped_type[4..].to_string();
                }

                if unwrapped_type.ends_with("[]") {
                    return Ok(format!(
                        "ref {}",
                        &unwrapped_type[..unwrapped_type.len() - 2]
                    ));
                }
                if unwrapped_type == "string" {
                    return Ok("ref string".to_string());
                }
                Ok("ref any".to_string())
            }
            Expression::AssignmentExpr {
                target,
                value,
                _line,
                _col,
                ..
            } => {
                let target_type = match target.as_ref() {
                    Expression::Identifier { name, .. } => {
                        if let Some(s) = self.lookup(name) {
                            Ok(s.type_name.clone())
                        } else {
                            self.report_error_detailed(format!("Undefined variable '{}'", name), *_line, *_col, "E0102", Some("Check the spelling or ensure the variable is declared before use"));
                            Ok("any".to_string())
                        }
                    }
                    _ => self.check_expression(target),
                }?;

                // Check for const reassignment
                if let Expression::Identifier { name, .. } = &**target {
                    if let Some(symbol) = self.lookup(name) {
                        if symbol.is_const {
                            self.report_error_detailed(format!("Cannot reassign to constant variable '{}'", name), *_line, *_col, "E0104", Some("Variable was declared with 'const'; use 'let' instead if you need to reassign"));
                        }
                    }
                }

                // Check for readonly member assignment (getters without setters)
                if let Expression::MemberAccessExpr { object, member, .. } = &**target {
                    // We need obj_type.
                    // Since check_expression(target) succeeded, check_expression(object) should succeed/be consistent.
                    // But strictly, we shouldn't re-run checks that might duplicate errors.
                    // But we have no cache. Assuming re-running is okay or we can suppress errors?
                    // Actually, calling check_expression(object) is safe because parsing/definition already happened.
                    if let Ok(obj_type) = self.check_expression(object) {
                        // Check instance members
                        if let Some(info) = self.resolve_instance_member(&obj_type, member) {
                            if info.is_readonly {
                                self.report_error_detailed(
                                    format!("Cannot assign to read-only property '{}'", member),
                                    *_line,
                                    *_col,
                                    "E0104",
                                    Some("This property is read-only; add a setter to modify it"),
                                );
                            }
                        } else {
                            // Static access??
                            if let Expression::Identifier { name, .. } = &**object {
                                if let Some(s) = self.lookup(name) {
                                    if s.type_name == "class" || s.type_name == "enum" {
                                        if let Some(members) = self.class_members.get(name) {
                                            if let Some(info) = members.get(member) {
                                                if info.is_readonly {
                                                    self.report_error_detailed(format!("Cannot assign to read-only static property '{}'", member), *_line, *_col, "E0104", Some("Static properties declared as read-only cannot be modified"));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let value_type = self.check_expression(value)?;
                if target_type != "any" && value_type != "any" {
                    if !self.is_assignable(&target_type, &value_type) {
                        self.report_error_detailed(
                            format!(
                                "Type mismatch in assignment: expected '{}', got '{}'",
                                target_type, value_type
                            ),
                            *_line,
                            *_col,
                            "E0100",
                            Some(&format!(
                                "Consider converting with 'as {}' or change the variable type",
                                target_type
                            )),
                        );
                    }
                }

                // Handle Move Semantics: If value is an Identifier and it's not Copy type, mark as moved
                if let Expression::Identifier { name: src_name, .. } = &**value {
                    if !self.is_copy_type(&value_type) && value_type != "any" {
                        let is_ref_assignment =
                            target_type.starts_with("ref ") || target_type.starts_with("weak ");
                        if !is_ref_assignment {
                            self.mark_moved(src_name, *_line, *_col);
                        }
                    }
                }

                // Unmark target as moved if it was an identifier
                if let Expression::Identifier {
                    name: target_name, ..
                } = &**target
                {
                    self.unmark_moved(target_name);
                }

                Ok(value_type)
            }
            Expression::CallExpr {
                callee,
                args,
                _line,
                _col,
            } => {
                let callee_str = callee.to_callee_name();

                if callee_str == "typeof" {
                    for arg in args {
                        self.check_expression(arg)?;
                    }
                    return Ok("string".to_string());
                }
                if callee_str == "sizeof" {
                    for arg in args {
                        self.check_expression(arg)?;
                    }
                    return Ok("int32".to_string());
                }
                if callee_str == "super" {
                    if let Some(Symbol {
                        type_name: _type_name,
                        ..
                    }) = self.lookup("super")
                    {
                        for arg in args {
                            self.check_expression(arg)?;
                        }
                        return Ok("void".to_string());
                    } else {
                        self.report_error_detailed("Cannot use 'super' here".to_string(), *_line, *_col, "E0115", Some("'super' can only be used inside a class that extends another class"));
                        return Ok("any".to_string());
                    }
                }

                let callee_type = self.check_expression(callee)?;
                let mut return_type = "any".to_string();
                let mut s_params = Vec::new();
                let mut is_variadic = false;

                if callee_type.starts_with("function:") || callee_type.contains("=>") {
                    let (parsed_ret, parsed_params, parsed_variadic) =
                        self.parse_signature(callee_type.clone());
                    // The returned final_type from parse_signature is "function:ret", so we strip it.
                    let parts: Vec<&str> = parsed_ret.splitn(2, ':').collect();
                    if parts.len() >= 2 {
                        let mut ret = parts[1].to_string();
                        if ret.ends_with(':') {
                            ret.pop();
                        }
                        return_type = ret;
                    }
                    s_params = parsed_params;
                    is_variadic = parsed_variadic;
                }
                // Always try lookup to fill s_params if not yet populated from type string
                if s_params.is_empty() {
                    if let Some(s) = self.lookup(&callee_str) {
                        if return_type == "any" && s.type_name.starts_with("function:") {
                            let parts: Vec<&str> = s.type_name.split(':').collect();
                            if parts.len() >= 2 {
                                let mut ret = parts[1].to_string();
                                if ret.ends_with(':') {
                                    ret.pop();
                                }
                                return_type = ret;
                            }
                        }
                        s_params = s.params.clone();
                        is_variadic = s.is_variadic;
                    }
                }

                // Check arguments
                for (i, arg) in args.iter().enumerate() {
                    let arg_type = self.check_expression(arg)?;
                    // ... (skipped some lines)

                    let target_type = if is_variadic {
                        if s_params.is_empty() {
                            "any".to_string()
                        } else if i >= s_params.len() - 1 {
                            let last_param = &s_params[s_params.len() - 1];
                            if last_param.ends_with("[]") {
                                last_param[..last_param.len() - 2].to_string()
                            } else {
                                "any".to_string()
                            }
                        } else {
                            s_params[i].clone()
                        }
                    } else if i < s_params.len() {
                        s_params[i].clone()
                    } else {
                        "any".to_string()
                    };

                    if !target_type.is_empty()
                        && target_type != "any"
                        && !self.are_types_compatible(&target_type, &arg_type)
                    {
                        // Skip error if either side is a generic type param (defined as 'any' in scope)
                        let is_generic_param = |t: &str| {
                            if let Some(sym) = self.lookup(t) {
                                sym.type_name == "any"
                                    && t.len() <= 2
                                    && t.chars().next().map_or(false, |c| c.is_uppercase())
                            } else {
                                false
                            }
                        };
                        if !is_generic_param(&target_type) && !is_generic_param(&arg_type) {
                            self.report_error_detailed(
                                format!(
                                    "Argument type mismatch for '{}': expected '{}', got '{}'",
                                    callee_str, target_type, arg_type
                                ),
                                *_line,
                                *_col,
                                "E0108",
                                Some(&format!(
                                    "Pass a value of type '{}' or convert using 'as {}'",
                                    target_type, target_type
                                )),
                            );
                        }
                    }

                    // Handle Move Semantics in Call (SOI: Implicit Borrow)
                    if let Expression::Identifier { name: src_name, .. } = arg {
                        let is_borrowing = callee_str.starts_with("console.")
                            || callee_str.starts_with("assert_")
                            || callee_str.starts_with("rt_")
                            || callee_str.starts_with("RT_")
                            || callee_str.starts_with("tejx_");
                        // SOI: Check if variable is used later in the current block.
                        // If used later → implicit borrow (don't mark moved).
                        // If last use → implicit move (mark moved).
                        let is_used_later = self
                            .remaining_stmts
                            .iter()
                            .any(|s| Self::stmt_contains_identifier(s, src_name));
                        if !is_borrowing
                            && !self.is_copy_type(&arg_type)
                            && arg_type != "any"
                            && !is_used_later
                        {
                            self.mark_moved(src_name, *_line, *_col);
                        }
                    }
                }

                if !is_variadic && !s_params.is_empty() && args.len() != s_params.len() {
                    let mut all_missing_are_optional = true;
                    if args.len() < s_params.len() {
                        for missing_param in &s_params[args.len()..] {
                            if !missing_param.starts_with("Optional<")
                                && !missing_param.contains("| None")
                            {
                                all_missing_are_optional = false;
                                break;
                            }
                        }
                    } else {
                        all_missing_are_optional = false;
                    }

                    if !all_missing_are_optional {
                        self.report_error_detailed(
                            format!(
                                "Function '{}' expects {} argument(s), but {} were provided",
                                callee_str,
                                s_params.len(),
                                args.len()
                            ),
                            *_line,
                            *_col,
                            "E0109",
                            Some(&format!("Provide exactly {} argument(s)", s_params.len())),
                        );
                    }
                }

                if callee_type == "any" && return_type == "any" {
                    // Possible dynamic call or lookup failed but we used any
                    if !callee_str.contains('.') {
                        if self.lookup(&callee_str).is_none() {
                            // self.report_error(format!("Undefined function '{}'", callee_str), *_line, *_col);
                        }
                    }
                }

                Ok(return_type)
            }
            Expression::ObjectLiteralExpr { entries, .. } => {
                let mut type_str = String::from("{");
                for (i, (key, val_expr)) in entries.iter().enumerate() {
                    let mut val_ty = self.check_expression(val_expr)?;
                    if val_ty.starts_with("ref ") {
                        val_ty = val_ty[4..].to_string();
                    }
                    type_str.push_str(&format!("{}: {}", key, val_ty));
                    if i < entries.len() - 1 {
                        type_str.push_str(", ");
                    }
                }
                type_str.push('}');
                Ok(type_str)
            }
            Expression::ArrayLiteral { elements, ty, .. } => {
                if !elements.is_empty() {
                    let mut first_type = self.check_expression(&elements[0])?;
                    if first_type.starts_with("ref ") {
                        first_type = first_type[4..].to_string();
                    }
                    for i in 1..elements.len() {
                        let mut elem_ty = self.check_expression(&elements[i])?;
                        if elem_ty.starts_with("ref ") {
                            elem_ty = elem_ty[4..].to_string();
                        }
                        if elem_ty != first_type && first_type != "{unknown}" {
                            let common = self.get_common_ancestor(&first_type, &elem_ty);
                            if common == "{unknown}" {
                                self.report_error_detailed(
                                    format!("Array elements must have consistent types. Expected '{}' but found '{}'", first_type, elem_ty),
                                    elements[i].get_line(),
                                    0,
                                    "E0091",
                                    Some("Ensure all elements in the array literal match the type of the first element or share a common ancestor"),
                                );
                                first_type = "{unknown}".to_string();
                            } else {
                                first_type = common;
                            }
                        }
                    }
                    let inferred = format!("{}[{}]", first_type, elements.len());
                    *ty.borrow_mut() = Some(inferred.clone());
                    Ok(inferred)
                } else {
                    let inferred = "[]".to_string();
                    *ty.borrow_mut() = Some(inferred.clone());
                    Ok(inferred)
                }
            }

            Expression::AwaitExpr { expr, _line, _col } => {
                if !self.current_function_is_async && self.current_function_return.is_some() {
                    self.report_error_detailed(
                        "'await' can only be used inside an 'async' function".to_string(),
                        *_line,
                        *_col,
                        "E0113",
                        Some("Mark the enclosing function with 'async' keyword"),
                    );
                }
                let t = self.check_expression(expr)?;
                if t.starts_with("Promise<") {
                    Ok(t[8..t.len() - 1].to_string())
                } else {
                    Ok(t)
                }
            }
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                self.check_expression(target)?;
                self.check_expression(index)?;
                Ok("any".to_string())
            }
            Expression::OptionalMemberAccessExpr { object, .. } => {
                self.check_expression(object)?;
                Ok("any".to_string())
            }
            Expression::OptionalCallExpr {
                callee,
                args,
                _line,
                _col,
            } => {
                self.check_expression(callee)?;
                for arg in args {
                    let arg_type = self.check_expression(arg)?;
                    if let Expression::Identifier { name: src_name, .. } = arg {
                        // SOI: Check Liveness Auto-Borrowing
                        let is_used_later = self
                            .remaining_stmts
                            .iter()
                            .any(|s| Self::stmt_contains_identifier(s, src_name));

                        if !self.is_copy_type(&arg_type) && arg_type != "any" && !is_used_later {
                            self.mark_moved(src_name, *_line, *_col);
                        }
                    }
                }
                Ok("any".to_string())
            }
            Expression::NewExpr {
                class_name,
                args,
                _line,
                _col,
            } => {
                // Check if this class requires generic type parameters
                if let Some(members) = self.class_members.get(class_name) {
                    // Classes with generic params should require explicit type args
                    if members.contains_key("__generic")
                        || class_name == "Map"
                        || class_name == "Set"
                    {
                        self.report_error_detailed(
                            format!("{} requires explicit type arguments", class_name),
                            *_line,
                            *_col,
                            "E0101",
                            Some(&format!(
                                "Use {}<K, V> or equivalent to strictly type this collection",
                                class_name
                            )),
                        );
                    }
                }
                if !self.is_valid_type(class_name) {
                    self.report_error_detailed(
                        format!("Unknown class '{}'", class_name),
                        *_line,
                        *_col,
                        "E0101",
                        Some("Ensure the class is defined or imported before use"),
                    );
                }
                if self.abstract_classes.contains(class_name) {
                    self.report_error_detailed(format!("Cannot instantiate abstract class '{}'", class_name), *_line, *_col, "E0110", Some("Create a concrete subclass that implements all abstract methods, then instantiate that instead"));
                }
                for arg in args {
                    let arg_type = self.check_expression(arg)?;
                    if let Expression::Identifier { name: src_name, .. } = arg {
                        if !self.is_copy_type(&arg_type) && arg_type != "any" {
                            self.mark_moved(src_name, *_line, *_col);
                        }
                    }
                }
                Ok(class_name.clone())
            }
            _ => Ok("any".to_string()), // TODO
        }
    }

    fn define_pattern(
        &mut self,
        pattern: &BindingNode,
        type_name: String,
        is_const: bool,
        line: usize,
        col: usize,
    ) -> Result<(), ()> {
        match pattern {
            BindingNode::Identifier(name) => {
                self.define_variable(name.clone(), type_name, is_const, line, col);
            }
            BindingNode::ArrayBinding { elements, rest } => {
                for el in elements {
                    let _ = self.define_pattern(el, "any".to_string(), is_const, line, col);
                }
                if let Some(rest_pattern) = rest {
                    let _ =
                        self.define_pattern(rest_pattern, "any".to_string(), is_const, line, col);
                }
            }
            BindingNode::ObjectBinding { entries } => {
                for (_, target) in entries {
                    let _ = self.define_pattern(target, "any".to_string(), is_const, line, col);
                }
            }
        }
        Ok(())
    }
}
