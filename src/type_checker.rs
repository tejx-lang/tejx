use crate::ast::{Program, Statement, Expression, BindingNode};
use crate::token::TokenType;
use crate::diagnostics::Diagnostic; // Import Diagnostic
use std::collections::HashMap;

// TypeInfo struct removed (unused)


pub struct TypeChecker {
    scopes: Vec<HashMap<String, String>>,
    current_class: Option<String>,
    current_function_return: Option<String>,
    current_function_is_async: bool,
    pub diagnostics: Vec<Diagnostic>, // Collect errors
    current_file: String,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut globals = HashMap::new();
        globals.insert("assert".to_string(), "function".to_string());
        globals.insert("len".to_string(), "function".to_string());
        globals.insert("eprint".to_string(), "function".to_string());
        globals.insert("random".to_string(), "function".to_string());
        globals.insert("parseInt".to_string(), "function".to_string());
        globals.insert("parseFloat".to_string(), "function".to_string());
        globals.insert("abs".to_string(), "function".to_string());
        globals.insert("min".to_string(), "function".to_string());
        globals.insert("max".to_string(), "function".to_string());
        Self {
            scopes: vec![globals], // Global scope
            current_class: None,
            current_function_return: None,
            current_function_is_async: false,
            diagnostics: Vec::new(),
            current_file: "unknown".to_string(),
        }
    }

    pub fn check(&mut self, program: &Program, filename: &str) -> Result<(), ()> {
        self.current_file = filename.to_string();
        // Pass 1: Collect declarations for hoisting
        for stmt in &program.statements {
            self.collect_declarations(stmt);
        }
        
        // Pass 2: Basic pass
        for stmt in &program.statements {
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
                self.define(class_decl.name.clone(), "class".to_string());
            }
            Statement::InterfaceDeclaration { name, .. } => {
                self.define(name.clone(), "interface".to_string());
            }
            Statement::TypeAliasDeclaration { name, .. } => {
                self.define(name.clone(), "type".to_string());
            }
            Statement::EnumDeclaration(enum_decl) => {
                self.define(enum_decl.name.clone(), "enum".to_string());
            }
            Statement::ProtocolDeclaration(proto) => {
                self.define(proto._name.clone(), "protocol".to_string());
            }
            Statement::ExportDecl { declaration, .. } => {
                self.collect_declarations(declaration);
            }
            Statement::FunctionDeclaration(func) => {
                let ret_ty = if func.return_type.is_empty() { "any".to_string() } else { func.return_type.clone() };
                self.define(func.name.clone(), format!("function:{}", ret_ty));
            }
            _ => {}
        }
    }

    fn define(&mut self, name: String, type_name: String) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, type_name);
        }
    }
    
    // Check if variable is defined in ANY scope
    fn lookup(&self, name: &str) -> Option<String> {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t.clone());
            }
        }
        None
    }

    fn report_error(&mut self, msg: String, line: usize, col: usize) {
        self.diagnostics.push(Diagnostic::new(msg, line, col, self.current_file.clone()));
    }

    fn is_valid_type(&self, type_name: &str) -> bool {
        if type_name == "void" || type_name == "any" || type_name == "" {
            return true;
        }

        // Handle Array types: number[], Array<number>, etc.
        if type_name.ends_with("[]") || type_name.contains("[") {
            let base = type_name.split('[').next().unwrap_or("");
            return self.is_valid_type(base);
        }
        if type_name.starts_with("Array<") && type_name.ends_with(">") {
            let inner = &type_name[6..type_name.len() - 1];
            return self.is_valid_type(inner);
        }
        if type_name.starts_with("Promise<") && type_name.ends_with(">") {
            let inner = &type_name[8..type_name.len() - 1];
            return self.is_valid_type(inner);
        }

        // Handle generic types: Type<Inner>
        if let Some(open) = type_name.find('<') {
            if type_name.ends_with('>') {
                let base = &type_name[..open];
                let inner = &type_name[open + 1..type_name.len() - 1];
                // Check if base is a known generic or just any type
                return self.is_valid_type(base) && self.is_valid_type(inner);
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
            "int", "int16", "int32", "int64", "int128",
            "float", "float16", "float32", "float64",
            "bool", "string", "char", "bigInt", "bigfloat",
            "object" 
        ];
        if primitives.contains(&type_name) {
            return true;
        }

        // Built-ins
        let builtins = ["Array", "Map", "Set", "Promise", "Console", "Error", "Date", "Math", "process", "console", "Option", "Result", "Some", "None"];
        if builtins.contains(&type_name) {
            return true;
        }

        // Defined in scopes (classes, interfaces, etc.)
        self.lookup(type_name).is_some()
    }

    fn check_statement(&mut self, stmt: &Statement) -> Result<(), ()> {
        match stmt {
            Statement::VarDeclaration { pattern, type_annotation, initializer, is_const: _, line, _col } => {
                if !self.is_valid_type(type_annotation) {
                    self.report_error(format!("Unknown data type: '{}'", type_annotation), *line, *_col);
                }
                if let Some(expr) = initializer {
                    let init_type = self.check_expression(expr)?;
                    let is_numeric = |t: &str| -> bool {
                        matches!(t, "int" | "int16" | "int32" | "int64" | "int128" | "float" | "float16" | "float32" | "float64")
                    };

                    let is_array = |t: &str| -> bool {
                        t.ends_with("[]") || t.contains("[") || t.starts_with("Array<")
                    };

                    let compatible = (is_numeric(type_annotation) && is_numeric(&init_type)) ||
                                     (type_annotation == "bool" && init_type == "bool") ||
                                     (is_array(type_annotation) && (init_type == "[]" || init_type == "any[]")) ||
                                     (type_annotation == "any[]" && is_array(&init_type)) ||
                                     (type_annotation == "any" || type_annotation == "");
                    
                    if type_annotation != "any" && type_annotation != "" && init_type != "any" && init_type != *type_annotation && !compatible {
                         self.report_error(format!("Type mismatch: expected '{}', got '{}'", type_annotation, init_type), *line, *_col);
                    }

                    if type_annotation == "any" || type_annotation == "" {
                        self.define_pattern(pattern, "any".to_string());
                    } else {
                        self.define_pattern(pattern, type_annotation.clone());
                    }
                } else {
                    if type_annotation == "any" || type_annotation == "" {
                        self.define_pattern(pattern, "any".to_string());
                    } else {
                        self.define_pattern(pattern, type_annotation.clone());
                    }
                }
                Ok(())
            },
            Statement::ExpressionStmt { _expression: expression, .. } => {
                self.check_expression(expression)?;
                Ok(())
            },
            Statement::BlockStmt { statements, .. } => {
                self.enter_scope();
                for s in statements {
                    let _ = self.check_statement(s);
                }
                self.exit_scope();
                Ok(())
            },
            Statement::IfStmt { condition, then_branch, else_branch, .. } => {
                self.check_expression(condition)?;
                self.check_statement(then_branch)?;
                if let Some(else_stmt) = else_branch {
                    self.check_statement(else_stmt)?;
                }
                Ok(())
            },
            Statement::WhileStmt { condition, body, .. } => {
                 self.check_expression(condition)?;
                 self.check_statement(body)?;
                 Ok(())
            },
            Statement::FunctionDeclaration(func) => {
                 // Define function in current scope (before entering body for recursion?)
                 // Actually this logic is for function name definition.
                 // "function foo()" -> variable foo of type function.
                 self.define(func.name.clone(), "function".to_string());
                 
                 self.enter_scope();
                 for param in &func.params {
                     if !self.is_valid_type(&param.type_name) {
                         self.report_error(format!("Unknown data type: '{}'", param.type_name), func._line, func._col);
                     }
                     self.define(param.name.clone(), param.type_name.clone());
                 }
                 
                 let prev_return = self.current_function_return.take();
                 let prev_async = self.current_function_is_async;
                 if !self.is_valid_type(&func.return_type) {
                     self.report_error(format!("Unknown data type: '{}'", func.return_type), func._line, func._col);
                 }
                  let ret_ty = if func.return_type.is_empty() { "any".to_string() } else { func.return_type.clone() };
                  self.current_function_return = Some(ret_ty);
                 self.current_function_is_async = func._is_async;
                 
                 self.check_statement(&func.body)?;
                 
                 self.current_function_return = prev_return;
                 self.current_function_is_async = prev_async;
                 self.exit_scope();
                 Ok(())
            },
            Statement::ClassDeclaration(class_decl) => {
                 self.current_class = Some(class_decl.name.clone());
                 self.define(class_decl.name.clone(), "class".to_string());
                 
                 self.enter_scope();
                 self.define("this".to_string(), class_decl.name.clone());
                 
                 for method in &class_decl.methods {
                     self.enter_scope();
                     for param in &method.func.params {
                         if !self.is_valid_type(&param.type_name) {
                             self.report_error(format!("Unknown data type: '{}'", param.type_name), class_decl._line, class_decl._col);
                         }
                         self.define(param.name.clone(), param.type_name.clone());
                     }
                     let prev_return = self.current_function_return.take();
                     let prev_async = self.current_function_is_async;
                     if !self.is_valid_type(&method.func.return_type) {
                         self.report_error(format!("Unknown data type: '{}'", method.func.return_type), class_decl._line, class_decl._col);
                     }
                      let ret_ty = if method.func.return_type.is_empty() { "any".to_string() } else { method.func.return_type.clone() };
                      self.current_function_return = Some(ret_ty);
                     self.current_function_is_async = method.func._is_async;
                     
                     self.check_statement(&method.func.body)?;
                     
                     self.current_function_return = prev_return;
                     self.current_function_is_async = prev_async;
                     self.exit_scope();
                 }
                 
                 self.exit_scope();
                 self.current_class = None;
                 Ok(())
            },
            Statement::ReturnStmt { value, _line: line, _col: col } => {
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
                           let inner = &expected_type[8..expected_type.len()-1];
                           let is_numeric = |t: &str| -> bool {
                               matches!(t, "int" | "int16" | "int32" | "int64" | "int128" | "float" | "float16" | "float32" | "float64")
                           };
                           let is_bool = |t: &str| -> bool { matches!(t, "bool") };

                           if got == inner || (is_numeric(inner) && is_numeric(&got)) || (is_bool(inner) && is_bool(&got)) {
                               // Implicit wrap: OK
                               return Ok(());
                           }
                      }

                       if expected_type != "any" && got != "any" && expected_type != got {
                             let is_numeric = |t: &str| -> bool {
                                 matches!(t, "int" | "int16" | "int32" | "int64" | "int128" | "float" | "float16" | "float32" | "float64")
                             };
                             let is_bool = |t: &str| -> bool { matches!(t, "bool") };

                            if (is_numeric(&expected_type) && is_numeric(&got)) || (is_bool(&expected_type) && is_bool(&got)) {
                                // Ok
                            } else {
                                self.report_error(format!("Return type mismatch: expected '{}', got '{}'", expected_original, got), *line, *col);
                            }
                       }
                 }
                 Ok(())
            },
            Statement::EnumDeclaration(enum_decl) => {
                 self.define(enum_decl.name.clone(), "enum".to_string());
                 // Define members as static properties of enum? 
                 // For simplified type check, just defining enum name is enough to pass basic checks.
                 Ok(())
            },
            Statement::TypeAliasDeclaration { name, .. } => {
                 self.define(name.clone(), "type".to_string());
                 Ok(())
            },
            Statement::InterfaceDeclaration { name, .. } => {
                 self.define(name.clone(), "interface".to_string());
                 Ok(())
            },
            Statement::ExportDecl { declaration, .. } => {
                 self.check_statement(declaration)?;
                 Ok(())
            },
            Statement::ImportDecl { _names, source, .. } => {
                 if !_names.is_empty() {
                     for name in _names {
                         self.define(name.clone(), "any".to_string());
                     }
                 } else {
                     // Module import: import std:math; etc.
                     // Define known exports based on source
                     if source == "std:math" {
                         self.define("min".to_string(), "function".to_string());
                         self.define("max".to_string(), "function".to_string());
                         self.define("abs".to_string(), "function".to_string());
                         self.define("round".to_string(), "function".to_string());
                         self.define("floor".to_string(), "function".to_string());
                         self.define("ceil".to_string(), "function".to_string());
                         self.define("pow".to_string(), "function".to_string());
                         self.define("sqrt".to_string(), "function".to_string());
                         self.define("sin".to_string(), "function".to_string());
                         self.define("cos".to_string(), "function".to_string());
                     } else if source == "std:json" {
                         self.define("parse".to_string(), "function".to_string());
                         self.define("stringify".to_string(), "function".to_string());
                     } else if source == "std:fs" {
                         self.define("read_to_string".to_string(), "function".to_string());
                         self.define("write".to_string(), "function".to_string());
                         self.define("remove".to_string(), "function".to_string());
                         self.define("exists".to_string(), "function".to_string());
                     } else if source == "std:time" {
                         self.define("now".to_string(), "function".to_string());
                         self.define("sleep".to_string(), "function".to_string());
                     } else if source == "std:os" {
                         self.define("args".to_string(), "function".to_string());
                     }
                 }
                 Ok(())
            },
            Statement::ExtensionDeclaration(_) => Ok(()), // Ignore for now
            Statement::ProtocolDeclaration(_) => Ok(()), // Ignore for now
            _ => Ok(()), // Catch-all for others
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
            Expression::StringLiteral { value, .. } => {
                if value.len() == 1 {
                    Ok("char".to_string())
                } else {
                    Ok("string".to_string())
                }
            }
            Expression::BooleanLiteral { .. } => Ok("bool".to_string()),
            Expression::UnaryExpr { op, right, _line, _col } => {
                let right_type = self.check_expression(right)?;
                match op {
                    TokenType::Bang => Ok("bool".to_string()),
                    TokenType::Minus => {
                        let is_numeric = |t: &str| -> bool {
                            matches!(t, "int" | "int16" | "int32" | "int64" | "int128" | "float" | "float16" | "float32" | "float64")
                        };
                        if is_numeric(&right_type) || right_type == "any" {
                            Ok(right_type)
                        } else {
                            self.report_error(format!("Unary '-' cannot be applied to type '{}'", right_type), *_line, *_col);
                            Ok("any".to_string())
                        }
                    },
                    TokenType::PlusPlus | TokenType::MinusMinus => Ok(right_type),
                    _ => Ok("any".to_string()),
                }
            },
            Expression::ThisExpr { _line, _col } => {
                if let Some(c) = &self.current_class {
                    Ok(c.clone())
                } else {
                    self.report_error("Using 'this' outside of a class".to_string(), *_line, *_col);
                    Ok("any".to_string())
                }
            },
            Expression::Identifier { name, _line, _col } => {
                if let Some(t) = self.lookup(name) {
                    Ok(t)
                } else {
                    if name == "console" { return Ok("Console".to_string()); }
                    self.report_error(format!("Undefined variable '{}'", name), *_line, *_col);
                    Ok("any".to_string())
                }
            },
            Expression::BinaryExpr { left, op, right, .. } => {
                let left_type = self.check_expression(left)?;
                let right_type = self.check_expression(right)?;
                
                let is_numeric = |t: &str| -> bool {
                    matches!(t, "int" | "int16" | "int32" | "int64" | "int128" | "float" | "float16" | "float32" | "float64")
                };
                let is_float = |t: &str| -> bool {
                    matches!(t, "float" | "float16" | "float32" | "float64")
                };

                if left_type == "string" || right_type == "string" {
                    return Ok("string".to_string());
                }

                if is_numeric(&left_type) && is_numeric(&right_type) {
                    if is_float(&left_type) || is_float(&right_type) {
                        return Ok("float32".to_string());
                    }
                    return Ok("int32".to_string());
                }

                // Boolean result for comparisons
                if matches!(op, TokenType::EqualEqual | TokenType::BangEqual | TokenType::Less | TokenType::LessEqual | TokenType::Greater | TokenType::GreaterEqual) {
                    return Ok("bool".to_string());
                }

                Ok("int32".to_string())
            },
            Expression::AssignmentExpr { target, value, _line, _col, .. } => {
                let target_type = self.check_expression(target)?;
                let value_type = self.check_expression(value)?;
                if target_type != "any" && value_type != "any" && target_type != value_type {
                    let is_numeric = |t: &str| -> bool {
                         matches!(t, "int" | "int16" | "int32" | "int64" | "int128" | "float" | "float16" | "float32" | "float64")
                    };
                    let is_array = |t: &str| -> bool {
                        t.ends_with("[]") || t.contains("[") || t.starts_with("Array<")
                    };
                    if is_numeric(&target_type) && is_numeric(&value_type) {
                        // Allow numeric assignments for now (implicit cast)
                    } else if (is_array(&target_type) && (value_type == "[]" || value_type == "any[]")) ||
                              (target_type == "any[]" && is_array(&value_type)) {
                        // Allow array compatibility
                    } else {
                        self.report_error(format!("Type mismatch in assignment: expected '{}', got '{}'", target_type, value_type), *_line, *_col);
                    }
                }
                Ok(value_type)
            },
            Expression::CallExpr { callee, args, _line, _col } => {
                if callee == "typeof" {
                    for arg in args { self.check_expression(arg)?; }
                    return Ok("string".to_string());
                }
                if callee == "sizeof" {
                    for arg in args { self.check_expression(arg)?; }
                    return Ok("int32".to_string());
                }

                if !self.lookup(callee).is_some() && callee != "print" && callee != "delay" {
                    if !callee.contains('.') {
                        self.report_error(format!("Undefined function '{}'", callee), *_line, *_col);
                    }
                }

                for arg in args {
                    self.check_expression(arg)?;
                }
                Ok("any".to_string())
            },
            Expression::ObjectLiteralExpr { .. } => Ok("any".to_string()),
            Expression::ArrayLiteral { elements, .. } => {
                if !elements.is_empty() {
                    let first_type = self.check_expression(&elements[0])?;
                    Ok(format!("{}[]", first_type))
                } else {
                    Ok("[]".to_string())
                }
            },
            Expression::AwaitExpr { expr, .. } => {
                self.check_expression(expr)?;
                Ok("any".to_string())
            },
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                self.check_expression(target)?;
                self.check_expression(index)?;
                Ok("any".to_string())
            },
            Expression::OptionalMemberAccessExpr { object, .. } => {
                self.check_expression(object)?;
                Ok("any".to_string())
            },
            Expression::OptionalCallExpr { callee, args, .. } => {
                self.check_expression(callee)?;
                for arg in args { self.check_expression(arg)?; }
                Ok("any".to_string())
            },
            _ => Ok("any".to_string()), // TODO
        }
    }

    fn define_pattern(&mut self, pattern: &BindingNode, type_name: String) -> Result<(), ()> {
        match pattern {
            BindingNode::Identifier(name) => {
                self.define(name.clone(), type_name);
            }
            BindingNode::ArrayBinding { elements, rest } => {
                for el in elements {
                    self.define_pattern(el, "any".to_string())?;
                }
                if let Some(rest_pattern) = rest {
                    self.define_pattern(rest_pattern, "any".to_string())?;
                }
            }
            BindingNode::ObjectBinding { entries } => {
                for (_, target) in entries {
                    self.define_pattern(target, "any".to_string())?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
