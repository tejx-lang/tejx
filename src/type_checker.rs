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
    pub params: Vec<String>,       // Parameter types if function
    pub min_params: Option<usize>, // Minimum required params (excluding defaulted ones)
    pub is_variadic: bool,
    pub aliased_type: Option<String>,
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
    lambda_context_params: Option<Vec<String>>,
    pub lambda_inferred_types: HashMap<(usize, usize), Vec<String>>,
    current_expected_type: Option<String>,
}

impl TypeChecker {
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
            lambda_context_params: None,
            lambda_inferred_types: HashMap::new(),
            current_expected_type: None,
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
                            type_name: self.parameterize_generics(
                                &m._type_name.to_string(),
                                &class_decl.generic_params,
                            ),
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
                    let ret_ty = if method.func.return_type.raw_name.is_empty() {
                        "void".to_string()
                    } else {
                        method.func.return_type.to_string()
                    };
                    let mut param_types = Vec::new();
                    for p in &method.func.params {
                        param_types.push(p.type_name.to_string());
                    }
                    let p_str = param_types.join(",");
                    let sig_str = if p_str.is_empty() {
                        format!("function:{}", ret_ty)
                    } else {
                        format!("function:{}:{}", ret_ty, p_str)
                    };
                    let (final_type, final_params, _) = self.parse_signature(sig_str);
                    let full_sig = if final_params.is_empty() {
                        final_type
                    } else {
                        format!("{}:{}", final_type, final_params.join(","))
                    };
                    let parameterized_type = self
                        .parameterize_generics(&full_sig.to_string(), &class_decl.generic_params);
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
                                &getter._return_type.to_string(),
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
                                    &setter._param_type.to_string(),
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
                let ret_ty = if func.return_type.raw_name.is_empty() {
                    "void".to_string()
                } else {
                    func.return_type.to_string()
                };
                let mut is_variadic = false;
                let min_required = func
                    .params
                    .iter()
                    .filter(|p| p._default_value.is_none() && !p._is_rest)
                    .count();
                let params = func
                    .params
                    .iter()
                    .map(|p| {
                        if p._is_rest {
                            is_variadic = true;
                        }
                        let (t, _, _) = self.parse_signature(p.type_name.to_string());
                        t
                    })
                    .collect::<Vec<String>>();
                let (final_ret, _, _) = self.parse_signature(format!("function:{}", ret_ty));
                let has_defaults = min_required < params.len();
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        func.name.clone(),
                        Symbol {
                            type_name: final_ret,
                            is_const: false,
                            params,
                            min_params: if has_defaults {
                                Some(min_required)
                            } else {
                                None
                            },
                            is_variadic,
                            aliased_type: None,
                        },
                    );
                }
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
                            min_params: None,
                            is_variadic: false,
                            aliased_type: Some(_type_def.to_string()),
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
                        param_types.push(p.type_name.to_string());
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
                    min_params: None,
                    is_variadic,
                    aliased_type: None,
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
                    min_params: None,
                    is_variadic,
                    aliased_type: None,
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
                    min_params: None,
                    is_variadic,
                    aliased_type: None,
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
            || type_name == "void"
            || type_name == "object"
            || type_name == "any"
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
                let mut p_count = 0;
                let mut depth = 0;
                let mut start = 0;
                if !inner.trim().is_empty() {
                    for (i, c) in inner.char_indices() {
                        match c {
                            '<' => depth += 1,
                            '>' => depth -= 1,
                            ',' if depth == 0 => {
                                if !self.is_valid_type(inner[start..i].trim()) {
                                    return false;
                                }
                                start = i + 1;
                                p_count += 1;
                            }
                            _ => {}
                        }
                    }
                    if !self.is_valid_type(inner[start..].trim()) {
                        return false;
                    }
                    p_count += 1;
                }

                let expected_count = match base {
                    "Array" | "Option" | "Promise" | "Ref" | "Weak" => Some(1),
                    "Map" | "Dict" | "Pair" | "Result" => Some(2),
                    _ => self.lookup(base).map(|s| s.params.len()),
                };
                if let Some(ec) = expected_count {
                    if ec != p_count && ec > 0 {
                        // Return false to let caller report invalid type,
                        // forcing the user to provide the right number of generic arguments!
                        return false;
                    }
                }
                return true;
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

    fn get_common_ancestor(&self, t1: &str, t2: &str) -> String {
        if t1 == t2 {
            return t1.to_string();
        }
        let is_any_int = |t: &str| {
            t == "int"
                || t == "int8"
                || t == "int16"
                || t == "int32"
                || t == "int64"
                || t == "uint8"
                || t == "uint16"
                || t == "uint32"
                || t == "uint64"
        };
        if is_any_int(t1) && is_any_int(t2) {
            return "int32".to_string(); // Default to int32 for mixed int arrays
        }
        if t1 == "unknown" {
            return t2.to_string();
        }
        if t2 == "unknown" {
            return t1.to_string();
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
        "Object".to_string()
    }

    fn are_types_compatible(&self, expected: &str, actual: &str) -> bool {
        // Fast path: exact match (handles T==T, Array<T>==Array<T>, etc.)
        if expected == actual || expected == "Object" {
            return true;
        }
        let is_object = |t: &str| t == "object" || t.ends_with(":Object") || t.ends_with(":any");
        if expected.is_empty() || actual.is_empty() || is_object(expected) {
            return true;
        }

        // Handle Union Types (e.g., actual is "Node | None", expected is "ref Node")
        if actual.contains('|') {
            // If expected is not a union, see if it is compatible with AT LEAST ONE of the union types
            // In strict mode, assigning `T | None` to `T` is unsafe without a check, but we allow it for now
            // for compatibility with `benchmark.tx` style unwrapping.
            let parts: Vec<&str> = actual.split('|').map(|s| s.trim()).collect();
            if parts
                .iter()
                .any(|&p| self.are_types_compatible(expected, p))
            {
                return true;
            }
        }

        if expected.contains('|') {
            let parts: Vec<&str> = expected.split('|').map(|s| s.trim()).collect();
            if parts.iter().any(|&p| self.are_types_compatible(p, actual)) {
                return true;
            }
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

        // Handle Option<T> explicitly before generic base comparison
        if expected.starts_with("Option<") && expected.ends_with(">") {
            let inner = &expected[7..expected.len() - 1]; // extract T
            let compatible = self.are_types_compatible(inner, actual);
            if actual == "None" || compatible {
                return true;
            }
        }
        if actual.starts_with("Option<") && actual.ends_with(">") {
            let inner = &actual[7..actual.len() - 1]; // extract T
            let compatible = self.are_types_compatible(expected, inner);
            if expected == "None" || compatible {
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

        let expected_base = expected;
        let actual_base = actual;

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
            || expected == ""
            || actual == ""
            || actual == "object"
        {
            return true;
        }
        if expected == actual {
            return true;
        }

        // `any` is compatible with all types
        if expected == "any" || actual == "any" {
            return true;
        }

        let is_missing_generic =
            |t: &str| t.starts_with("$MISSING_GENERIC_") || t.starts_with("ref $MISSING_GENERIC_");
        if is_missing_generic(expected) || is_missing_generic(actual) {
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

        // Integer types are mutually compatible (int, int8, int16, int32, etc.)
        let is_any_int = |t: &str| {
            t == "int"
                || t == "int8"
                || t == "int16"
                || t == "int32"
                || t == "int64"
                || t == "uint8"
                || t == "uint16"
                || t == "uint32"
                || t == "uint64"
        };
        if is_any_int(expected) && is_any_int(actual) {
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

        // Handle intersection types (e.g., "A & B")
        if expected.contains('&') {
            let parts: Vec<&str> = expected.split('&').map(|s| s.trim()).collect();
            let mut all_match = true;
            for part in parts {
                if !self.are_types_compatible(part, actual) {
                    all_match = false;
                    break;
                }
            }
            if all_match {
                return true;
            }
        }

        // Enum compatibility with int32
        let is_enum = |t: &str| {
            t == "enum"
                || self
                    .lookup(t)
                    .map(|s| s.type_name == "enum")
                    .unwrap_or(false)
        };
        let is_int_alias = |t: &str| t == "int" || t == "int16" || t == "int32" || t == "int64";
        if (is_enum(expected) && is_int_alias(actual))
            || (is_enum(actual) && is_int_alias(expected))
        {
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
            if actual.contains(":Object") || actual.ends_with(":") {
                return true;
            }
            // More strict check could be added here
        }

        // Alias check: int == int32 and float == float32
        let is_int_alias = |t: &str| t == "int" || t == "int32";
        if is_int_alias(expected) && is_int_alias(actual) {
            return true;
        }
        let is_float_alias = |t: &str| t == "float" || t == "float32";
        if is_float_alias(expected) && is_float_alias(actual) {
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

        // Slice Coercion: T[] -> slice<T>, T[N] -> slice<T>, string -> slice<char>
        let is_slice = |t: &str| t.starts_with("slice<") && t.ends_with(">");
        if is_slice(expected) {
            let inner_expected = &expected[6..expected.len() - 1];
            if actual == "string"
                && (inner_expected == "char" || inner_expected == "byte" || inner_expected == "int")
            {
                return true;
            }
            if actual.ends_with("[]") {
                let inner_actual = &actual[..actual.len() - 2];
                return self.are_types_compatible(inner_expected, inner_actual);
            }
            if is_fixed_array(actual) {
                let inner_actual = actual.split('[').next().unwrap_or("");
                return self.are_types_compatible(inner_expected, inner_actual);
            }
            if is_array_class(actual) {
                let inner_actual = &actual[6..actual.len() - 1];
                return self.are_types_compatible(inner_expected, inner_actual);
            }
        }

        // Empty array assignment
        if actual == "[]" && (expected.ends_with("[]") || expected.starts_with("Array<")) {
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

            // Exact type matching for object literals containing extra properties.
            // If actual has extra properties not in expected, we reject to enforce strict typing.
            for key in a_props.keys() {
                if !e_props.contains_key(key) {
                    return false; // Extra property found
                }
            }

            for (key, e_type) in &e_props {
                let is_optional = key.ends_with('?') || e_type.starts_with("Option<");
                let base_key = key.trim_end_matches('?');

                // Check actual props handling optional keys
                let mut found = false;
                for (a_key, a_type) in &a_props {
                    let base_a_key = a_key.trim_end_matches('?');
                    if base_key == base_a_key {
                        found = true;
                        // For Option<T>, inner T type match or actual type Option<T> is fine
                        let target_expected =
                            if e_type.starts_with("Option<") && e_type.ends_with(">") {
                                &e_type[7..e_type.len() - 1]
                            } else {
                                e_type.as_str()
                            };

                        let target_actual =
                            if a_type.starts_with("Option<") && a_type.ends_with(">") {
                                &a_type[7..a_type.len() - 1]
                            } else {
                                a_type.as_str()
                            };

                        if !self.are_types_compatible(target_expected, target_actual) {
                            return false;
                        }
                    }
                }
                if !found && !is_optional {
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

    fn check_numeric_bounds(
        &mut self,
        expr: &Expression,
        target_type: &str,
        line: usize,
        col: usize,
    ) {
        let mut val_to_check = None;
        if let Expression::NumberLiteral { value, .. } = expr {
            val_to_check = Some(*value);
        } else if let Expression::UnaryExpr {
            op: TokenType::Minus,
            right,
            ..
        } = expr
        {
            if let Expression::NumberLiteral { value, .. } = &**right {
                val_to_check = Some(-*value);
            }
        }

        if let Some(v) = val_to_check {
            let mut min = None;
            let mut max = None;
            match target_type {
                "int8" => {
                    min = Some(-128.0);
                    max = Some(127.0);
                }
                "uint8" => {
                    min = Some(0.0);
                    max = Some(255.0);
                }
                "int16" => {
                    min = Some(-32768.0);
                    max = Some(32767.0);
                }
                "uint16" => {
                    min = Some(0.0);
                    max = Some(65535.0);
                }
                "int32" | "int" => {
                    min = Some(-2147483648.0);
                    max = Some(2147483647.0);
                }
                "uint32" => {
                    min = Some(0.0);
                    max = Some(4294967295.0);
                }
                _ => {}
            }
            if let (Some(min_val), Some(max_val)) = (min, max) {
                if v < min_val || v > max_val {
                    self.report_error_detailed(
                        format!("Value {} is out of bounds for type '{}'", v, target_type),
                        line,
                        col,
                        "E0100",
                        Some(&format!("Valid range is {} to {}", min_val, max_val)),
                    );
                }
            }
        }
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
                if !self.is_valid_type(&type_annotation.to_string()) {
                    self.report_error_detailed(format!("Unknown data type: '{}'", type_annotation.to_string()), *line, *_col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, or user-defined classes"));
                }
                if let Some(expr) = initializer {
                    let prev_expected = self.current_expected_type.take();
                    if !type_annotation.raw_name.is_empty() {
                        self.current_expected_type = Some(type_annotation.to_string());
                    }
                    let mut init_type = self.check_expression(expr)?;
                    self.current_expected_type = prev_expected;
                    if type_annotation.raw_name.is_empty() && init_type == "[]" {
                        self.report_error_detailed(
                            "Cannot infer type for empty array".to_string(),
                            *line,
                            *_col,
                            "E0106",
                            Some("Please provide an explicit type annotation (e.g., 'let arr: int[] = []')"),
                        );
                        init_type = "unknown".to_string(); // prevent cascading errors
                    }

                    if !type_annotation.is_empty() {
                        self.check_numeric_bounds(expr, &type_annotation.to_string(), *line, *_col);
                    } else if init_type != "any" && init_type != "unknown" {
                        self.check_numeric_bounds(expr, &init_type, *line, *_col);
                    }

                    if !type_annotation.raw_name.is_empty()
                        && !self.are_types_compatible(&type_annotation.to_string(), &init_type)
                    {
                        if init_type == "[]" {
                            self.report_error_detailed(
                                format!(
                                    "Type mismatch: expected '{}', got empty array",
                                    type_annotation.to_string()
                                ),
                                *line,
                                *_col,
                                "E0100",
                                Some(&format!(
                                    "Empty arrays must be explicitly typed or match the target type '{}'",
                                    type_annotation.to_string()
                                )),
                            );
                        } else {
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
                    }

                    let _target_type = if type_annotation.is_empty() {
                        init_type
                    } else {
                        type_annotation.to_string()
                    };

                    let _ = self.define_pattern(pattern, _target_type, *is_const, *line, *_col);
                } else {
                    if type_annotation.is_empty() {
                        self.report_error_detailed(
                            "Type annotation required for uninitialized variable".to_string(),
                            *line,
                            *_col,
                            "E0101",
                            Some("Provide an explicit type (e.g., 'let x: int;')"),
                        );
                        let _ = self.define_pattern(
                            pattern,
                            "{unknown}".to_string(),
                            *is_const,
                            *line,
                            *_col,
                        );
                    } else {
                        let _ = self.define_pattern(
                            pattern,
                            type_annotation.to_string(),
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
            Statement::DelStmt {
                target,
                _line,
                _col,
            } => {
                let _target_type = self.check_expression(target)?;
                if let Expression::Identifier { name, .. } = &**target {
                    if let Some(_s) = self.lookup(name) {
                        // deleted variable tracking removed along with borrow check
                    }
                } else if let Expression::MemberAccessExpr { .. } = &**target {
                    // Allowed to delete properties from objects
                } else {
                    self.report_error_detailed(
                        "Invalid target for 'del'".to_string(),
                        *_line,
                        *_col,
                        "E0100",
                        Some("'del' can only be used with variables or object properties"),
                    );
                }
                Ok(())
            }
            Statement::BlockStmt { statements, .. } => {
                self.enter_scope();
                for s in statements {
                    self.collect_declarations(s);
                }
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
                let mut ret_ty = if func.return_type.raw_name.is_empty() {
                    "void".to_string()
                } else {
                    func.return_type.to_string()
                };
                if func._is_async && !ret_ty.starts_with("Promise<") {
                    ret_ty = format!("Promise<{}>", ret_ty);
                }
                let mut is_variadic = false;
                let min_required = func
                    .params
                    .iter()
                    .filter(|p| p._default_value.is_none() && !p._is_rest)
                    .count();
                let params: Vec<String> = func
                    .params
                    .iter()
                    .map(|p| {
                        if p._is_rest {
                            is_variadic = true;
                        }
                        p.type_name.to_string()
                    })
                    .collect();
                let has_defaults = min_required < params.len();
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(
                        func.name.clone(),
                        Symbol {
                            type_name: format!("function:{}", ret_ty),
                            is_const: false,
                            min_params: if has_defaults {
                                Some(min_required)
                            } else {
                                None
                            },
                            params,
                            is_variadic,
                            aliased_type: None,
                        },
                    );
                }

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
                        param.type_name.to_string(),
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
                        if !self.is_valid_type(&param.type_name.to_string()) {
                            self.report_error_detailed(format!("Unknown data type: '{}'", param.type_name.to_string()), class_decl._line, class_decl._col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, or user-defined classes"));
                        }
                        self.define(param.name.clone(), param.type_name.to_string());
                    }
                    let prev_return = self.current_function_return.take();
                    let prev_async = self.current_function_is_async;
                    if !self.is_valid_type(&method.func.return_type.to_string()) {
                        self.report_error_detailed(format!("Unknown data type: '{}' for return type of method '{}'", method.func.return_type.to_string(), method.func.name), class_decl._line, class_decl._col, "E0101", Some("Valid types include: int, int32, float, float64, string, bool, void, or user-defined classes"));
                    }
                    let ret_ty = if method.func.return_type.raw_name.is_empty() {
                        "void".to_string()
                    } else {
                        method.func.return_type.to_string()
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
                        self.define(param.name.clone(), param.type_name.to_string());
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
                    self.current_function_return = Some(getter._return_type.to_string());
                    self.check_statement(&getter._body)?;
                    self.current_function_return = prev_return;
                    self.exit_scope();
                }

                for setter in &class_decl._setters {
                    self.enter_scope();
                    self.define(setter._param_name.clone(), setter._param_type.to_string());
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

                        if got == inner
                            || (is_numeric(inner) && is_numeric(&got))
                            || (is_bool(inner) && is_bool(&got))
                        {
                            // Implicit wrap: OK
                            return Ok(());
                        }
                    }

                    if !self.is_assignable(&expected_type, &got) {
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
            Statement::ImportDecl { _names, source, .. } => {
                // Register imported names/namespaces so type checker doesn't reject them
                if _names.is_empty() {
                    // Whole-module import: `import std:time;` → register `time` as any
                    let module_name = if source.starts_with("std:") {
                        source.trim_start_matches("std:").to_string()
                    } else {
                        std::path::Path::new(source)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default()
                    };
                    if !module_name.is_empty() {
                        self.define(module_name, "any".to_string());
                    }
                } else {
                    // Named imports: `import { parse, stringify } from "std:json"`
                    for item in _names {
                        if self.lookup(&item.name).is_none() {
                            self.define(item.name.clone(), "any".to_string());
                        }
                    }
                }
                Ok(())
            }
            Statement::ExtensionDeclaration(ext_decl) => {
                let name = &ext_decl._target_type;
                let methods = &ext_decl._methods;

                let mut existing_members = self
                    .class_members
                    .get(&name.to_string())
                    .cloned()
                    .unwrap_or(HashMap::new());
                for method in methods {
                    let m_name = &method.name;
                    // Build method type string
                    let mut param_types = Vec::new();
                    for p in &method.params {
                        param_types.push(p.type_name.to_string());
                    }
                    let p_str = param_types.join(",");
                    let type_str = format!("function:{}:{}", method.return_type.to_string(), p_str);

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
                    self.current_class = Some(name.to_string());

                    self.enter_scope();
                    self.define("this".to_string(), name.to_string());

                    for param in &method.params {
                        self.define(param.name.clone(), param.type_name.to_string());
                    }

                    let prev_return = self.current_function_return.take();
                    let prev_async = self.current_function_is_async;

                    let ret_ty = if method.return_type.raw_name.is_empty() {
                        "void".to_string()
                    } else {
                        method.return_type.to_string()
                    };
                    self.current_function_return = Some(ret_ty);
                    self.current_function_is_async = method._is_async;

                    self.check_statement(&method.body)?;

                    self.current_function_return = prev_return;
                    self.current_function_is_async = prev_async;

                    self.exit_scope();
                    self.current_class = prev_class;
                }
                self.class_members
                    .insert(name.to_string(), existing_members);
                Ok(())
            }
            // Statement::ProtocolDeclaration(_) => Ok(()), // Removed
            _ => Ok(()), // Catch-all for others
        }
    }

    fn substitute_generics(&self, member_type: &str, obj_type: &str) -> String {
        let mut parts = Vec::new();
        if let Some(open) = obj_type.find('<') {
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
        } else if obj_type.ends_with("[]") {
            parts.push(&obj_type[..obj_type.len() - 2]);
        }

        let mut result = member_type.to_string();
        // Check for $0, $1... up to some reasonable limit or until no more are found
        for i in 0..5 {
            let placeholder = format!("${}", i);
            if result.contains(&placeholder) {
                let replacement = if i < parts.len() {
                    parts[i].to_string()
                } else {
                    format!("$MISSING_GENERIC_{}", i)
                };
                result = result.replace(&placeholder, &replacement);
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
                    _ => Ok(right_type),
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
            Expression::LambdaExpr {
                params,
                body,
                _line,
                _col,
            } => {
                self.enter_scope();

                let mut actual_param_types = Vec::new();
                let mut context_types = None;

                // Use lambda context if available
                if let Some(ctx) = self.lambda_context_params.clone() {
                    context_types = Some(ctx);
                }

                for (i, p) in params.iter().enumerate() {
                    let mut p_type = p.type_name.to_string();
                    if p_type.is_empty() || p_type == "any" {
                        if let Some(ctx_types) = &context_types {
                            if i < ctx_types.len() {
                                p_type = ctx_types[i].clone();
                            }
                        }
                    }
                    self.define(p.name.clone(), p_type.clone());
                    actual_param_types.push(p_type);
                }

                // Save inferred types for CodeGen
                self.lambda_inferred_types
                    .insert((*_line, *_col), actual_param_types.clone());

                let prev_return = self.current_function_return.take();
                self.current_function_return = Some("any".to_string());

                self.check_statement(body)?;

                self.current_function_return = prev_return;
                self.exit_scope();

                Ok(format!("function:Object:{}", actual_param_types.join(",")))
            }
            Expression::Identifier { name, _line, _col } => {
                if let Some(s) = self.lookup(name) {
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
                Ok(target_type.to_string())
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

                // Built-in 'length' property for arrays, strings, and slices
                if member == "length" {
                    if obj_type == "string"
                        || obj_type.ends_with("[]")
                        || (obj_type.starts_with("slice<") && obj_type.ends_with(">"))
                    {
                        return Ok("int32".to_string());
                    }
                }

                if obj_type.starts_with("{") {
                    let props = self.parse_struct_props(&obj_type);
                    if let Some(t) = props.get(member) {
                        return Ok(t.clone());
                    }
                }

                if !obj_type.is_empty() && obj_type != "object" && !obj_type.starts_with("{") {
                    // Fallback for enums: default to int32 if known enum
                    if obj_type == "enum"
                        || self
                            .lookup(&obj_type)
                            .map(|s| s.type_name == "enum")
                            .unwrap_or(false)
                    {
                        return Ok("int32".to_string());
                    }
                }

                // --- UFCS Lookup ---
                // If not found as a member, check if there is a global function name(obj, ...)
                if let Some(s) = self.lookup(member) {
                    if s.type_name.starts_with("function:") {
                        if !s.params.is_empty() {
                            let first_param = &s.params[0];
                            if self.are_types_compatible(first_param, &obj_type) {
                                // Found a match! Return the function type but we keep note it's UFCS
                                // Actually, for type checking, we just return the function type.
                                // CodeGen will handle the translation.
                                return Ok(self.substitute_generics(&s.type_name, &obj_type));
                            }
                        }
                    }
                }

                if !obj_type.is_empty()
                    && obj_type != "object"
                    && obj_type != "any"
                    && !obj_type.starts_with("{")
                {
                    self.report_error_detailed(
                        format!(
                            "Property '{}' does not exist on type '{}'",
                            member, obj_type
                        ),
                        *_line,
                        *_col,
                        "E0105",
                        Some("Check the property name or define it in the class"),
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
                if let Some(sym) = self.lookup(&unwrapped_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        unwrapped_type = aliased.clone();
                    }
                }

                if unwrapped_type.contains('|') {
                    unwrapped_type = unwrapped_type
                        .split('|')
                        .map(|s| s.trim().to_string())
                        .find(|s| s != "None" && !s.is_empty())
                        .unwrap_or(target_type.clone());
                }

                if unwrapped_type.ends_with("[]") {
                    let res = unwrapped_type[..unwrapped_type.len() - 2].to_string();
                    return Ok(res);
                }
                if unwrapped_type.starts_with("Array<") && unwrapped_type.ends_with(">") {
                    let inner = &unwrapped_type[6..unwrapped_type.len() - 1];
                    return Ok(inner.to_string());
                }
                if unwrapped_type == "string" {
                    return Ok("string".to_string());
                }
                Ok("Object".to_string())
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

                let prev_expected = self.current_expected_type.take();
                self.current_expected_type = Some(target_type.clone());
                let value_type = self.check_expression(value)?;
                self.current_expected_type = prev_expected;
                if target_type != "any" && value_type != "any" && target_type != "unknown" {
                    self.check_numeric_bounds(value, &target_type, *_line, *_col);
                    if !self.is_assignable(&target_type, &value_type) {
                        if value_type == "[]" {
                            self.report_error_detailed(
                                format!(
                                    "Type mismatch in assignment: expected '{}', got empty array",
                                    target_type
                                ),
                                *_line,
                                *_col,
                                "E0100",
                                Some(&format!(
                                    "Empty arrays must match the target type '{}'",
                                    target_type
                                )),
                            );
                        } else {
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

                let mut signature_found = false;

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
                    signature_found = true;
                }
                // Always try lookup to fill s_params if not yet populated from type string
                if !signature_found {
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
                        signature_found = true;
                    }
                }

                let mut generic_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();

                // If `s_params` is still empty, fallback to looking up just the method name
                if !signature_found {
                    let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                    if let Some(s) = self.lookup(func_name) {
                        s_params = s.params.clone();
                        is_variadic = s.is_variadic;
                        signature_found = true;
                    }
                }

                let mut param_offset = 0;
                if let Expression::MemberAccessExpr { .. } = &**callee {
                    if s_params.len() > 0 && s_params.len() >= args.len() + 1 {
                        param_offset = 1;
                    }
                }

                // Check arguments
                for (i, arg) in args.iter().enumerate() {
                    let adjusted_i = i + param_offset;
                    let mut target_type = if is_variadic {
                        if s_params.is_empty() {
                            "any".to_string()
                        } else if adjusted_i >= s_params.len() - 1 {
                            let last_param = &s_params[s_params.len() - 1];
                            if last_param.ends_with("[]") {
                                last_param[..last_param.len() - 2].to_string()
                            } else {
                                "any".to_string()
                            }
                        } else {
                            s_params[adjusted_i].clone()
                        }
                    } else if adjusted_i < s_params.len() {
                        s_params[adjusted_i].clone()
                    } else {
                        "any".to_string()
                    };
                    // For Array methods, transform `T` and `T[]` parameters based on instance context
                    // If UFCS, the method receiver is actually the object of the MemberAccessExpr
                    let mut resolved_receiver = String::new();
                    if let Expression::MemberAccessExpr { object, .. } = &**callee {
                        if let Ok(obj_type) = self.check_expression(object) {
                            resolved_receiver = obj_type;
                        }
                    } else if callee_type.starts_with("function:") && args.len() > 0 {
                        resolved_receiver = self.check_expression(&args[0]).unwrap_or_default();
                    }

                    if resolved_receiver.is_empty() {
                        resolved_receiver = callee_type.clone();
                    }

                    if let Some(s) = self.lookup(&resolved_receiver) {
                        if let Some(alias) = &s.aliased_type {
                            resolved_receiver = alias.clone();
                        }
                    }

                    if resolved_receiver.starts_with("Array<") {
                        let inner = &resolved_receiver[6..resolved_receiver.len() - 1];
                        if target_type == "T"
                            || target_type.starts_with("$MISSING_GENERIC_")
                            || target_type == "$0"
                        {
                            target_type = inner.to_string();
                        } else if target_type == "Array<T>" {
                            target_type = format!("Array<{}>", inner);
                        } else if target_type == "T[]"
                            || target_type == "$0[]"
                            || target_type.ends_with("[]") && target_type.starts_with("T")
                        {
                            target_type = format!("{}[]", inner);
                        }
                    } else if resolved_receiver.ends_with("[]") {
                        let inner = &resolved_receiver[..resolved_receiver.len() - 2];
                        if target_type == "T"
                            || target_type.starts_with("$MISSING_GENERIC_")
                            || target_type == "$0"
                        {
                            target_type = inner.to_string();
                        } else if target_type == "Array<T>" {
                            target_type = format!("Array<{}>", inner);
                        } else if target_type == "T[]"
                            || target_type == "$0[]"
                            || target_type.ends_with("[]") && target_type.starts_with("T")
                        {
                            target_type = format!("{}[]", inner);
                        }
                    }

                    if matches!(arg, Expression::LambdaExpr { .. }) {
                        if target_type.starts_with("function:") || target_type.contains("=>") {
                            let (_, parsed_params, _) = self.parse_signature(target_type.clone());

                            self.lambda_context_params = Some(parsed_params);
                        }
                    } else {
                        self.lambda_context_params = None;
                    }

                    let prev_expected = self.current_expected_type.take();
                    self.current_expected_type = Some(target_type.clone());
                    let arg_type = self.check_expression(arg)?;
                    self.current_expected_type = prev_expected;
                    self.lambda_context_params = None;

                    let is_object_check =
                        |t: &str| t == "object" || t.ends_with(":Object") || t.ends_with(":any");
                    if !target_type.is_empty()
                        && !is_object_check(&target_type)
                        && !self.are_types_compatible(&target_type, &arg_type)
                    {
                        // Skip error if either side is a generic type param (defined as 'any' in scope)
                        let is_generic_param = |t: &str| {
                            if t.starts_with("$MISSING_GENERIC_") {
                                return true;
                            }
                            if let Some(sym) = self.lookup(t) {
                                sym.type_name == "any"
                                    && t.len() <= 2
                                    && t.chars().next().map_or(false, |c| c.is_uppercase())
                            } else {
                                false
                            }
                        };
                        // Also check if the function's original param was a generic type variable
                        let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                        let original_param_is_generic = if let Some(sym) = self.lookup(func_name) {
                            if adjusted_i < sym.params.len() {
                                let orig = &sym.params[adjusted_i];
                                orig.len() <= 2
                                    && orig.chars().next().map_or(false, |c| c.is_uppercase())
                                    && orig.chars().all(|c| c.is_alphanumeric())
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !is_generic_param(&target_type)
                            && !is_generic_param(&arg_type)
                            && !original_param_is_generic
                        {
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

                    let is_generic_param_check = |t: &str| {
                        if t.starts_with("$MISSING_GENERIC_") {
                            return true;
                        }
                        t.len() <= 2
                            && t.chars().next().map_or(false, |c| c.is_uppercase())
                            && t.chars().all(|c| c.is_alphanumeric())
                    };
                    if is_generic_param_check(&target_type) && arg_type != "any" {
                        generic_map.insert(target_type.clone(), arg_type.clone());
                    }
                }

                let effective_param_count = s_params.len().saturating_sub(param_offset);
                if !is_variadic && !s_params.is_empty() && args.len() != effective_param_count {
                    // Check if the callee has min_params (default parameter support)
                    let min_required = self
                        .lookup(&callee_str)
                        .and_then(|s| s.min_params)
                        .map(|m| m.saturating_sub(param_offset))
                        .unwrap_or(effective_param_count);

                    if args.len() < min_required || args.len() > effective_param_count {
                        // Check if missing params are Optional or have defaults
                        let mut all_missing_are_optional = true;
                        if args.len() < min_required {
                            let start = args.len() + param_offset;
                            let end = min_required + param_offset;
                            if end <= s_params.len() {
                                for missing_param in &s_params[start..end] {
                                    if !missing_param.starts_with("Option<")
                                        && !missing_param.contains("| None")
                                    {
                                        all_missing_are_optional = false;
                                        break;
                                    }
                                }
                            }
                        } else if args.len() > effective_param_count {
                            all_missing_are_optional = false;
                        }

                        if !all_missing_are_optional {
                            let expected_msg = if min_required < effective_param_count {
                                format!("{} to {}", min_required, effective_param_count)
                            } else {
                                format!("{}", effective_param_count)
                            };
                            self.report_error_detailed(
                                format!(
                                    "Function '{}' expects {} argument(s), but {} were provided",
                                    callee_str,
                                    expected_msg,
                                    args.len()
                                ),
                                *_line,
                                *_col,
                                "E0109",
                                Some(&format!("Provide {} argument(s)", expected_msg)),
                            );
                        }
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

                let mut final_ret = return_type.clone();
                for (k, v) in &generic_map {
                    if final_ret == *k {
                        final_ret = v.clone();
                    } else if final_ret.contains(k) {
                        final_ret = final_ret.replace(&format!("<{}>", k), &format!("<{}>", v));
                        final_ret = final_ret.replace(&format!("{},", k), &format!("{},", v));
                        final_ret = final_ret.replace(&format!(", {}", k), &format!(", {}", v));
                        final_ret = final_ret.replace(&format!("{}[]", k), &format!("{}[]", v));
                    }
                }

                Ok(final_ret)
            }
            Expression::ObjectLiteralExpr {
                entries, _spreads, ..
            } => {
                let mut type_str = String::from("{");
                let mut is_first = true;
                for (key, val_expr) in entries {
                    let mut val_ty = self.check_expression(val_expr)?;
                    if !is_first {
                        type_str.push_str(", ");
                    }
                    type_str.push_str(&format!("{}: {}", key, val_ty));
                    is_first = false;
                }

                for spread_expr in _spreads {
                    let spread_ty = self.check_expression(spread_expr)?;
                    let props = self.parse_struct_props(&spread_ty);
                    for (k, val_ty) in props {
                        if !is_first {
                            type_str.push_str(", ");
                        }
                        type_str.push_str(&format!("{}: {}", k, val_ty));
                        is_first = false;
                    }
                }

                type_str.push('}');
                Ok(type_str)
            }
            Expression::ArrayLiteral {
                elements,
                ty,
                _line,
                _col,
                ..
            } => {
                if !elements.is_empty() {
                    let mut first_type_opt: Option<String> = None;
                    let expected_inner = if let Some(expected) = &self.current_expected_type {
                        if expected.ends_with("[]") {
                            Some(expected[..expected.len() - 2].to_string())
                        } else if expected.starts_with("Array<") && expected.ends_with(">") {
                            Some(expected[6..expected.len() - 1].to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    for i in 0..elements.len() {
                        let prev_expected = self.current_expected_type.take();
                        if let Some(inner) = &expected_inner {
                            self.current_expected_type = Some(inner.clone());
                        }

                        let mut elem_ty = self.check_expression(&elements[i])?;

                        self.current_expected_type = prev_expected;

                        if let Expression::SpreadExpr { .. } = elements[i] {
                            if elem_ty.ends_with("[]") {
                                elem_ty = elem_ty[..elem_ty.len() - 2].to_string();
                            } else if elem_ty.starts_with("Array<") {
                                elem_ty = elem_ty[6..elem_ty.len() - 1].to_string();
                            }
                        }

                        if let Some(first_type) = &first_type_opt {
                            if &elem_ty != first_type && first_type != "unknown" {
                                let common = self.get_common_ancestor(first_type, &elem_ty);
                                if common == "unknown" {
                                    self.report_error_detailed(
                                        format!("Array elements have incompatible types: '{}' and '{}'", first_type, elem_ty),
                                        *_line,
                                        *_col,
                                        "E0100",
                                        Some("All elements in an array literal must have the same type")
                                    );
                                    first_type_opt = Some("unknown".to_string());
                                } else {
                                    first_type_opt = Some(common);
                                }
                            }
                        } else {
                            first_type_opt = Some(elem_ty);
                        }
                    }
                    let t = first_type_opt.unwrap_or_else(|| "unknown".to_string()) + "[]";
                    *ty.borrow_mut() = Some(t.clone());
                    Ok(t)
                } else {
                    let mut t = "[]".to_string();
                    if let Some(expected) = &self.current_expected_type {
                        if expected.ends_with("[]") || expected.starts_with("Array<") {
                            t = expected.clone();
                        }
                    }
                    *ty.borrow_mut() = Some(t.clone());
                    Ok(t)
                }
            }
            Expression::SpreadExpr { _expr, .. } => self.check_expression(_expr),

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
                let target_type = self.check_expression(target)?;
                self.check_expression(index)?;
                let mut unwrapped_type = target_type.clone();
                if let Some(sym) = self.lookup(&unwrapped_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        unwrapped_type = aliased.clone();
                    }
                }
                if unwrapped_type.contains('|') {
                    unwrapped_type = unwrapped_type
                        .split('|')
                        .map(|s| s.trim().to_string())
                        .find(|s| s != "None" && !s.is_empty())
                        .unwrap_or(unwrapped_type.clone());
                }
                if unwrapped_type.ends_with("[]") {
                    return Ok(unwrapped_type[..unwrapped_type.len() - 2].to_string());
                }
                if unwrapped_type.starts_with("Array<") && unwrapped_type.ends_with(">") {
                    let inner = &unwrapped_type[6..unwrapped_type.len() - 1];
                    return Ok(inner.to_string());
                }
                Ok("any".to_string())
            }
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                let mut obj_type = self.check_expression(object)?;
                if let Some(sym) = self.lookup(&obj_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        obj_type = aliased.clone();
                    }
                }
                if obj_type.contains('|') {
                    obj_type = obj_type
                        .split('|')
                        .map(|s| s.trim().to_string())
                        .find(|s| s != "None" && !s.is_empty())
                        .unwrap_or(obj_type.clone());
                }

                if obj_type.starts_with("{") {
                    let props = self.parse_struct_props(&obj_type);
                    if let Some(t) = props.get(member) {
                        return Ok(t.clone());
                    }
                }

                if let Some(info) = self.resolve_instance_member(&obj_type, member) {
                    return Ok(self.substitute_generics(&info.type_name, &obj_type));
                }

                Ok("any".to_string())
            }
            Expression::NullishCoalescingExpr { _left, _right, .. } => {
                let left_ty = self.check_expression(_left)?;
                let right_ty = self.check_expression(_right)?;
                // Strip "Option<>" or "| None" from left_ty ideally, but for now just return the non-Object type
                if left_ty != "any" && left_ty != "unknown" {
                    if left_ty.starts_with("Option<") {
                        Ok(left_ty[7..left_ty.len() - 1].to_string())
                    } else if left_ty.ends_with(" | None") {
                        Ok(left_ty[..left_ty.len() - 7].to_string())
                    } else if left_ty == "None" {
                        Ok(right_ty)
                    } else {
                        Ok(left_ty)
                    }
                } else {
                    Ok(right_ty)
                }
            }
            Expression::TernaryExpr {
                _condition,
                _true_branch,
                _false_branch,
                ..
            } => {
                self.check_expression(_condition)?;
                let true_ty = self.check_expression(_true_branch)?;
                let false_ty = self.check_expression(_false_branch)?;
                if true_ty == false_ty {
                    Ok(true_ty)
                } else if true_ty != "any" && true_ty != "unknown" {
                    Ok(true_ty)
                } else {
                    Ok(false_ty)
                }
            }
            Expression::OptionalCallExpr {
                callee,
                args,
                _line,
                _col,
            } => {
                // Try to resolve return type from callee
                let callee_type = self.check_expression(callee)?;
                if callee_type.starts_with("function:") {
                    let (ret, _, _) = self.parse_signature(callee_type);
                    Ok(ret)
                } else {
                    Ok(callee_type)
                }
            }
            Expression::NewExpr {
                class_name,
                args,
                _line,
                _col,
            } => {
                // Generic type parameters are inferred from the variable declaration's
                // type annotation (e.g., `let m: Map<string, int> = new Map()`),
                // so we don't require explicit type args on the constructor call.
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
                    self.check_expression(arg)?;
                }
                Ok(class_name.clone())
            }
            Expression::SuperExpr { _line, _col } => {
                if let Some(s) = self.lookup("super") {
                    Ok(s.type_name)
                } else {
                    self.report_error_detailed(
                        "Using 'super' outside of a derived class".to_string(),
                        *_line,
                        *_col,
                        "E0115",
                        Some("'super' can only be used inside methods of a class that extends another class"),
                    );
                    Ok("any".to_string())
                }
            }
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
                let inner_type = if type_name.ends_with("[]") {
                    type_name[..type_name.len() - 2].to_string()
                } else if type_name.starts_with("Array<") && type_name.ends_with(">") {
                    type_name[6..type_name.len() - 1].to_string()
                } else {
                    "any".to_string()
                };

                for el in elements {
                    let _ = self.define_pattern(el, inner_type.clone(), is_const, line, col);
                }
                if let Some(rest_pattern) = rest {
                    let _ = self.define_pattern(
                        rest_pattern,
                        format!("{}[]", inner_type),
                        is_const,
                        line,
                        col,
                    );
                }
            }
            BindingNode::ObjectBinding { entries } => {
                // Determine property types if type_name is a known class or object literal
                for (key, target) in entries {
                    let mut prop_ty = "any".to_string();
                    if type_name.starts_with("{") {
                        let props = self.parse_struct_props(&type_name);
                        if let Some(t) = props.get(key) {
                            prop_ty = t.clone();
                        }
                    } else if type_name != "any" && type_name != "any" && !type_name.is_empty() {
                        if let Some(info) = self.resolve_instance_member(&type_name, key) {
                            prop_ty = self.substitute_generics(&info.type_name, &type_name);
                        }
                    }
                    let _ = self.define_pattern(target, prop_ty, is_const, line, col);
                }
            }
        }
        Ok(())
    }
}
