use crate::ast::{Program, Statement, Expression, BindingNode};
use std::collections::HashMap;

// TypeInfo struct removed (unused)


pub struct TypeChecker {
    scopes: Vec<HashMap<String, String>>,
    current_class: Option<String>,
    current_function_return: Option<String>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()], // Global scope
            current_class: None,
            current_function_return: None,
        }
    }

    pub fn check(&mut self, program: &Program) -> Result<(), String> {
        // Pre-pass: Define global logical types?
        // Basic pass
        for stmt in &program.statements {
            self.check_statement(stmt)?;
        }
        Ok(())
    }

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        self.scopes.pop();
    }

    fn define(&mut self, name: String, type_name: String) -> Result<(), String> {
        if let Some(scope) = self.scopes.last_mut() {
            if scope.contains_key(&name) {
                return Err(format!("Variable '{}' already declared in this scope", name));
            }
            scope.insert(name, type_name);
            Ok(())
        } else {
             Err("Internal Error: No scope found".to_string())
        }
    }

    fn lookup(&self, name: &str) -> Option<String> {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.get(name) {
                return Some(t.clone());
            }
        }
        None
    }

    fn check_statement(&mut self, stmt: &Statement) -> Result<(), String> {
        match stmt {
            Statement::VarDeclaration { pattern, type_annotation, initializer, is_const: _, line, _col: _ } => {
                if let Some(expr) = initializer {
                    let init_type = self.check_expression(expr)?;
                    let compatible = (type_annotation == "int" && init_type == "number") || (type_annotation == "number" && init_type == "int");
                    if type_annotation != "any" && init_type != "any" && init_type != *type_annotation && !compatible {
                         return Err(format!("Type mismatch at line {}: expected '{}', got '{}'", line, type_annotation, init_type));
                    }
                }
                
                self.define_pattern(pattern, type_annotation.clone())?;
                Ok(())
            },
            Statement::ExpressionStmt { _expression: expression, .. } => {
                self.check_expression(expression)?;
                Ok(())
            },
            Statement::BlockStmt { statements, .. } => {
                self.enter_scope();
                for s in statements {
                    if let Err(e) = self.check_statement(s) {
                        self.exit_scope();
                        return Err(e);
                    }
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
                 self.define(func.name.clone(), "function".to_string())?;
                 
                 self.enter_scope();
                 for param in &func.params {
                     self.define(param.name.clone(), param.type_name.clone())?;
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
                 self.define(class_decl.name.clone(), "class".to_string())?;
                 
                 self.enter_scope();
                 self.define("this".to_string(), class_decl.name.clone())?;
                 
                 for method in &class_decl.methods {
                     self.enter_scope();
                     for param in &method.func.params {
                         self.define(param.name.clone(), param.type_name.clone())?;
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
           Statement::ReturnStmt { value, _line: line, .. } => {
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
                               return Err(format!("Return type mismatch at line {}: expected '{}', got '{}'", line, expected_type, got));
                           }
                      }
                }
                Ok(())
            },
            Statement::EnumDeclaration(enum_decl) => {
                 self.define(enum_decl.name.clone(), "enum".to_string())?;
                 // Define members as static properties of enum? 
                 // For simplified type check, just defining enum name is enough to pass basic checks.
                 Ok(())
            },
            Statement::TypeAliasDeclaration { name, .. } => {
                 self.define(name.clone(), "type".to_string())?;
                 Ok(())
            },
            Statement::InterfaceDeclaration { name, .. } => {
                 self.define(name.clone(), "interface".to_string())?;
                 Ok(())
            },
            Statement::ExportDecl { declaration, .. } => {
                 self.check_statement(declaration)?;
                 Ok(())
            },
            Statement::ExtensionDeclaration(_) => Ok(()), // Ignore for now
            Statement::ProtocolDeclaration(_) => Ok(()), // Ignore for now
            _ => Ok(()), // Catch-all for others
        }
    }

    fn check_expression(&mut self, expr: &Expression) -> Result<String, String> {
        match expr {
            Expression::NumberLiteral { .. } => Ok("number".to_string()),
            Expression::StringLiteral { .. } => Ok("string".to_string()),
            Expression::BooleanLiteral { .. } => Ok("boolean".to_string()),
            Expression::ThisExpr { .. } => {
                if let Some(c) = &self.current_class {
                    Ok(c.clone())
                } else {
                    Err("Using 'this' outside of a class".to_string())
                }
            },
            Expression::Identifier { name, _line, .. } => {
                if let Some(t) = self.lookup(name) {
                    Ok(t)
                } else {
                      // Allow strict check?
                      // Err(format!("Undefined variable '{}' at line {}", name, line))
                      // For now, let's just log or return 'any' if we want to be lenient initially
                      // But for correctness, error is better.
                      // NOTE: Features like Console log might be undefined yet.
                      if name == "console" { return Ok("Console".to_string()); }
                      Err(format!("Undefined variable '{}' at line {}", name, _line))
                }
            },
            Expression::BinaryExpr { left, op: _, right, .. } => {
                let left_type = self.check_expression(left)?;
                let right_type = self.check_expression(right)?;
                // Very basic inference
                if left_type == "number" || right_type == "number" {
                    return Ok("number".to_string());
                }
                Ok("boolean".to_string()) // Assume comparison or logic
            },
            Expression::AssignmentExpr { target, value, .. } => {
                 let target_type = self.check_expression(target)?;
                 let value_type = self.check_expression(value)?;
                 if target_type != "any" && value_type != "any" && target_type != value_type {
                     return Err(format!("Type mismatch in assignment: expected '{}', got '{}'", target_type, value_type));
                 }
                 Ok(value_type)
            },
            Expression::CallExpr { callee: _, args, .. } => {
                 // Resolve callee type?
                 // For now, just check args
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

    fn define_pattern(&mut self, pattern: &BindingNode, type_name: String) -> Result<(), String> {
        match pattern {
            BindingNode::Identifier(name) => {
                self.define(name.clone(), type_name)?;
            }
            BindingNode::ArrayBinding { elements, .. } => {
                for el in elements {
                    self.define_pattern(el, "any".to_string())?;
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
