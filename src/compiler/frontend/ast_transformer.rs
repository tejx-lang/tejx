use crate::frontend::ast::*;
use crate::common::types::TejxType;
use std::collections::HashMap;

pub struct TypeSubstitutor<'a> {
    pub substitutions: &'a HashMap<String, TypeNode>,
}

impl<'a> TypeSubstitutor<'a> {
    pub fn new(substitutions: &'a HashMap<String, TypeNode>) -> Self {
        Self { substitutions }
    }

    fn substitute_type_node(&self, node: &mut TypeNode) {
        let mut replacement = None;
        if let TypeNode::Named(name) = node {
            if let Some(sub) = self.substitutions.get(name) {
                replacement = Some(sub.clone());
            }
        }
        if let Some(r) = replacement {
            *node = r;
            return;
        }

        match node {
            TypeNode::Named(_) => {}
            TypeNode::Generic(name, args) => {
                if let Some(sub) = self.substitutions.get(name) {
                    *name = sub.to_string();
                }
                for arg in args {
                    self.substitute_type_node(arg);
                }
            }
            TypeNode::Array(inner) => self.substitute_type_node(inner),
            TypeNode::Function(params, ret) => {
                for p in params {
                    self.substitute_type_node(p);
                }
                self.substitute_type_node(ret);
            }
            TypeNode::Object(members) => {
                for (_, _, ty) in members {
                    self.substitute_type_node(ty);
                }
            }
            TypeNode::Union(types) | TypeNode::Intersection(types) => {
                for t in types {
                    self.substitute_type_node(t);
                }
            }
            TypeNode::Any => {}
        }
    }

    pub fn substitute_type(&self, type_node: &mut TypeNode) {
        // Substitute strictly within the structured node AST
        self.substitute_type_node(type_node);
    }

    pub fn transform_statement(&self, stmt: &mut Statement) {
        match stmt {
            Statement::VarDeclaration { type_annotation, initializer, .. } => {
                self.substitute_type(type_annotation);
                if let Some(expr) = initializer {
                    self.transform_expression(expr);
                }
            }
            Statement::FunctionDeclaration(f) => self.transform_function(f),
            Statement::ClassDeclaration(c) => self.transform_class(c),
            Statement::ExtensionDeclaration(e) => {
                self.substitute_type(&mut e._target_type);
                for m in &mut e._methods {
                    self.transform_function(m);
                }
            }
            Statement::EnumDeclaration(e) => {
                for m in &mut e._members {
                    if let Some(expr) = &mut m._value {
                        self.transform_expression(expr);
                    }
                }
            }
            Statement::TypeAliasDeclaration { _type_def, .. } => {
                self.substitute_type(_type_def);
            }
            Statement::InterfaceDeclaration { _methods, .. } => {
                for m in _methods {
                    self.substitute_type(&mut m._return_type);
                    for p in &mut m._params {
                        self.substitute_type(&mut p.type_name);
                    }
                }
            }
            Statement::ReturnStmt { value, .. } => {
                if let Some(expr) = value {
                    self.transform_expression(expr);
                }
            }
            Statement::BlockStmt { statements, .. } => {
                for s in statements {
                    self.transform_statement(s);
                }
            }
            Statement::IfStmt { condition, then_branch, else_branch, .. } => {
                self.transform_expression(condition);
                self.transform_statement(then_branch);
                if let Some(e) = else_branch {
                    self.transform_statement(e);
                }
            }
            Statement::WhileStmt { condition, body, .. } => {
                self.transform_expression(condition);
                self.transform_statement(body);
            }
            Statement::ForStmt { init, condition, increment, body, .. } => {
                if let Some(i) = init { self.transform_statement(i); }
                if let Some(c) = condition { self.transform_expression(c); }
                if let Some(inc) = increment { self.transform_expression(inc); }
                self.transform_statement(body);
            }
            Statement::ForOfStmt { iterable, body, .. } => {
                self.transform_expression(iterable);
                self.transform_statement(body);
            }
            Statement::SwitchStmt { condition, cases, .. } => {
                self.transform_expression(condition);
                for c in cases {
                    if let Some(v) = &mut c.value { self.transform_expression(v); }
                    for s in &mut c.statements { self.transform_statement(s); }
                }
            }
            Statement::ExpressionStmt { _expression, .. } => {
                self.transform_expression(_expression);
            }
            Statement::TryStmt { _try_block, _catch_block, _finally_block, .. } => {
                self.transform_statement(_try_block);
                self.transform_statement(_catch_block);
                if let Some(f) = _finally_block { self.transform_statement(f); }
            }
            Statement::DelStmt { target, .. } => self.transform_expression(target),
            Statement::ThrowStmt { _expression, .. } => self.transform_expression(_expression),
            Statement::ExportDecl { declaration, .. } => self.transform_statement(declaration),
            _ => {}
        }
    }

    pub fn transform_expression(&self, expr: &mut Expression) {
        match expr {
            Expression::BinaryExpr { left, right, .. } => {
                self.transform_expression(left);
                self.transform_expression(right);
            }
            Expression::UnaryExpr { right, .. } => self.transform_expression(right),
            Expression::AssignmentExpr { target, value, .. } => {
                self.transform_expression(target);
                self.transform_expression(value);
            }
            Expression::CallExpr { callee, args, .. } => {
                self.transform_expression(callee);
                for a in args { self.transform_expression(a); }
            }
            Expression::SequenceExpr { expressions, .. } => {
                for e in expressions { self.transform_expression(e); }
            }
            Expression::MemberAccessExpr { object, .. } => self.transform_expression(object),
            Expression::ArrayAccessExpr { target, index, .. } => {
                self.transform_expression(target);
                self.transform_expression(index);
            }
            Expression::ObjectLiteralExpr { entries, _spreads, .. } => {
                for (_, e) in entries { self.transform_expression(e); }
                for s in _spreads { self.transform_expression(s); }
            }
            Expression::ArrayLiteral { elements, .. } => {
                for e in elements { self.transform_expression(e); }
            }
            Expression::NewExpr { args, class_name, .. } => {
                for a in args { self.transform_expression(a); }

                let mut bindings = HashMap::new();
                for (k, v) in self.substitutions.iter() {
                    bindings.insert(k.clone(), TejxType::from_node(v));
                }
                let class_ty = TejxType::from_name(class_name);
                let substituted = class_ty.substitute_generics(&bindings);
                *class_name = substituted.to_name();
            }
            Expression::LambdaExpr { params, body, .. } => {
                for p in params { self.substitute_type(&mut p.type_name); }
                self.transform_statement(body);
            }
            Expression::AwaitExpr { expr, .. } => self.transform_expression(expr),
            Expression::TernaryExpr { _condition, _true_branch, _false_branch, .. } => {
                self.transform_expression(_condition);
                self.transform_expression(_true_branch);
                self.transform_expression(_false_branch);
            }
            Expression::OptionalMemberAccessExpr { object, .. } => self.transform_expression(object),
            Expression::OptionalCallExpr { callee, args, .. } => {
                self.transform_expression(callee);
                for a in args { self.transform_expression(a); }
            }
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                self.transform_expression(target);
                self.transform_expression(index);
            }
            Expression::NullishCoalescingExpr { _left, _right, .. } => {
                self.transform_expression(_left);
                self.transform_expression(_right);
            }
            Expression::SpreadExpr { _expr, .. } => self.transform_expression(_expr),
            Expression::SomeExpr { value, .. } => self.transform_expression(value),
            Expression::CastExpr { expr, target_type, .. } => {
                self.transform_expression(expr);
                self.substitute_type(target_type);
            }
            _ => {}
        }
    }

    pub fn transform_function(&self, func: &mut FunctionDeclaration) {
        self.substitute_type(&mut func.return_type);
        for p in &mut func.params {
            self.substitute_type(&mut p.type_name);
            if let Some(expr) = &mut p._default_value {
                self.transform_expression(expr);
            }
        }
        self.transform_statement(&mut func.body);
    }

    pub fn transform_class(&self, class: &mut ClassDeclaration) {
        for m in &mut class._members {
            self.substitute_type(&mut m._type_name);
            if let Some(expr) = &mut m._initializer {
                self.transform_expression(expr);
            }
        }
        for m in &mut class.methods {
            self.transform_function(&mut m.func);
        }
        for g in &mut class._getters {
            self.substitute_type(&mut g._return_type);
            self.transform_statement(&mut g._body);
        }
        for s in &mut class._setters {
            self.substitute_type(&mut s._param_type);
            self.transform_statement(&mut s._body);
        }
        if let Some(c) = &mut class._constructor {
            self.transform_function(c);
        }
    }
}
