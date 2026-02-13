use crate::ast::{Program, Statement, Expression, BindingNode};
use crate::diagnostics::Diagnostic; // Import Diagnostic
use std::collections::HashMap;

// TypeInfo struct removed (unused)


pub struct TypeChecker {
    scopes: Vec<HashMap<String, String>>,
    current_class: Option<String>,
    current_function_return: Option<String>,
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
            diagnostics: Vec::new(),
            current_file: "unknown".to_string(),
        }
    }

    pub fn check(&mut self, program: &Program, filename: &str) -> Result<(), ()> {
        self.current_file = filename.to_string();
        // Pre-pass: Define global logical types?
        // Basic pass
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

    fn check_statement(&mut self, stmt: &Statement) -> Result<(), ()> {
        match stmt {
            Statement::VarDeclaration { pattern, type_annotation, initializer, is_const: _, line, _col } => {
                if let Some(expr) = initializer {
                    let init_type = self.check_expression(expr)?;
                    let compatible = (type_annotation == "int" && init_type == "number") || 
                                     (type_annotation == "number" && init_type == "int") ||
                                     (type_annotation == "float" && init_type == "number") || 
                                     (type_annotation == "bigInt" && init_type == "number") ||
                                     (type_annotation == "bigfloat" && init_type == "number");
                    
                    if type_annotation != "any" && init_type != "any" && init_type != *type_annotation && !compatible {
                         self.report_error(format!("Type mismatch: expected '{}', got '{}'", type_annotation, init_type), *line, *_col);
                         // Don't return error, continue checking
                    }
                }
                
                self.define_pattern(pattern, type_annotation.clone());
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
                     self.define(param.name.clone(), param.type_name.clone());
                 }
                 
                 let prev_return = self.current_function_return.take();
                 self.current_function_return = Some(func.return_type.clone());
                 
                 self.check_statement(&func.body)?;
                 
                 self.current_function_return = prev_return;
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
                         self.define(param.name.clone(), param.type_name.clone());
                     }
                     let prev_return = self.current_function_return.take();
                     self.current_function_return = Some(method.func.return_type.clone());
                     
                     self.check_statement(&method.func.body)?;
                     
                     self.current_function_return = prev_return;
                     self.exit_scope();
                 }
                 
                 self.exit_scope();
                 self.current_class = None;
                 Ok(())
            },
            Statement::ReturnStmt { value, _line: line, _col: col } => {
                 let expected = self.current_function_return.clone();
                 if let Some(expected_type) = expected {
                      let got = if let Some(expr) = value {
                          self.check_expression(expr)?
                      } else {
                          "void".to_string()
                      };
                      if expected_type != "any" && got != "any" && expected_type != got {
                            // Allow number to match int for now
                            if (expected_type == "int" && got == "number") || (expected_type == "number" && got == "int") {
                                // Ok
                            } else {
                                self.report_error(format!("Return type mismatch: expected '{}', got '{}'", expected_type, got), *line, *col);
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
            Expression::NumberLiteral { .. } => Ok("number".to_string()),
            Expression::StringLiteral { .. } => Ok("string".to_string()),
            Expression::BooleanLiteral { .. } => Ok("boolean".to_string()),
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
                      // Catch undefined variable usage
                      self.report_error(format!("Undefined variable '{}'", name), *_line, *_col);
                      Ok("any".to_string())
                }
            },
            Expression::BinaryExpr { left, op: _, right, .. } => {
                let left_type = self.check_expression(left)?;
                let right_type = self.check_expression(right)?;
                // Very basic inference
                if left_type == "number" || right_type == "number" {
                    if left_type == "string" || right_type == "string" {
                         return Ok("string".to_string());
                    }
                    return Ok("number".to_string());
                }
                if left_type == "string" || right_type == "string" {
                     return Ok("string".to_string());
                }
                Ok("boolean".to_string()) // Assume comparison or logic
            },
            Expression::AssignmentExpr { target, value, _line, _col, .. } => {
                 let target_type = self.check_expression(target)?;
                 let value_type = self.check_expression(value)?;
                 if target_type != "any" && value_type != "any" && target_type != value_type {
                      self.report_error(format!("Type mismatch in assignment: expected '{}', got '{}'", target_type, value_type), *_line, *_col);
                 }
                 Ok(value_type)
            },
            Expression::CallExpr { callee, args, _line, _col } => {
                 // Check if callee exists
                 if !self.lookup(callee).is_some() && callee != "print" && callee != "delay" {
                     // If callee contains '.', assume it is a method call or property access
                     // We don't have enough type info to validate these yet.
                     if callee.contains('.') {
                         // Ignore for now
                     } else {
                         // Function lookup failed
                         self.report_error(format!("Undefined function '{}'", callee), *_line, *_col);
                     }
                 }

                 for arg in args {
                     self.check_expression(arg)?;
                 }
                 // If we knew the function type we could check return type.
                 Ok("any".to_string())
            },
            Expression::AwaitExpr { expr, .. } => {
                 // Return unpacked type if possible, or just "any"
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
