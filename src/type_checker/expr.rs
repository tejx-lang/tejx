use super::*;
use crate::ast::*;
use crate::token::TokenType;
use std::collections::HashMap;

impl TypeChecker {
    pub(crate) fn check_expression(&mut self, expr: &Expression) -> Result<TejxType, ()> {
        match expr {
            Expression::NumberLiteral { value, .. } => {
                if value.fract() == 0.0 {
                    Ok(TejxType::Int32)
                } else {
                    Ok(TejxType::Float32)
                }
            }
            Expression::StringLiteral { .. } => Ok(TejxType::String),
            Expression::BooleanLiteral { .. } => Ok(TejxType::Bool),
            Expression::NoneLiteral { .. } => Ok(TejxType::from_name("None")),
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
                    TokenType::Bang => Ok(TejxType::Bool),
                    TokenType::Minus => {
                        if right_type.is_numeric() || right_type == TejxType::from_name("<inferred>") {
                            Ok(right_type)
                        } else {
                            self.report_error_detailed(
                                format!("Unary '-' cannot be applied to type '{}'", right_type.to_name()),
                                *_line,
                                *_col,
                                "E0100",
                                Some("Unary negation only works on numeric types (int, float)"),
                            );
                            Ok(TejxType::from_name("<inferred>"))
                        }
                    }
                    TokenType::PlusPlus | TokenType::MinusMinus => Ok(right_type),
                    _ => Ok(right_type),
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
                    if p_type.is_empty() || p_type == "<inferred>" || p_type == "any" {
                        if let Some(ctx_types) = &context_types {
                            if i < ctx_types.len() {
                                p_type = ctx_types[i].to_name();
                            }
                        }
                    }
                    self.define(p.name.clone(), p_type.clone());
                    actual_param_types.push(TejxType::from_name(&p_type));
                }

                // Save inferred types for CodeGen
                self.lambda_inferred_types
                    .insert((*_line, *_col), actual_param_types.clone());

                let prev_return = self.current_function_return.take();
                self.current_function_return = Some(TejxType::from_name("<inferred>"));

                self.check_statement(body)?;

                let inferred_ret = self
                    .current_function_return
                    .take()
                    .unwrap_or(TejxType::Void);
                let final_ret = if inferred_ret == TejxType::from_name("<inferred>") {
                    "void".to_string()
                } else {
                    inferred_ret.to_name()
                };

                self.current_function_return = prev_return;
                self.exit_scope();

                Ok(TejxType::Function(actual_param_types, Box::new(TejxType::from_name(&final_ret))))
            }
            Expression::Identifier { name, _line, _col } => {
                if let Some(s) = self.lookup(name) {
                    Ok(s.ty.clone())
                } else {
                    if name == "console" {
                        return Ok(TejxType::from_name("Console"));
                    }
                    self.report_error_detailed(
                        format!("Undefined variable '{}'", name),
                        *_line,
                        *_col,
                        "E0102",
                        Some("Check the spelling or ensure the variable is declared before use"),
                    );
                    Ok(TejxType::from_name("<inferred>"))
                }
            }
            Expression::CastExpr {
                expr, target_type, ..
            } => {
                let _expr_type = self.check_expression(expr)?;
                Ok(TejxType::from_node(target_type))
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

                if left_type == TejxType::String || right_type == TejxType::String {
                    if matches!(op, TokenType::Plus) {
                        return Ok(TejxType::String);
                    }
                    if matches!(
                        op,
                        TokenType::EqualEqual
                            | TokenType::BangEqual
                            | TokenType::EqualEqualEqual
                            | TokenType::BangEqualEqual
                    ) {
                        return Ok(TejxType::Bool);
                    }
                    if matches!(
                        op,
                        TokenType::Less
                            | TokenType::Greater
                            | TokenType::LessEqual
                            | TokenType::GreaterEqual
                    ) {
                        return Ok(TejxType::Bool);
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
                    return Ok(TejxType::from_name("<inferred>"));
                }

                if left_type.is_numeric() && right_type.is_numeric() {
                    if matches!(
                        op,
                        TokenType::EqualEqual
                            | TokenType::BangEqual
                            | TokenType::Less
                            | TokenType::LessEqual
                            | TokenType::Greater
                            | TokenType::GreaterEqual
                    ) {
                        return Ok(TejxType::Bool);
                    }

                    if matches!(
                        op,
                        TokenType::Minus | TokenType::Star | TokenType::Slash | TokenType::Plus
                    ) {
                        if left_type == TejxType::Float64 || right_type == TejxType::Float64 {
                            return Ok(TejxType::Float64);
                        }
                        if left_type.is_float() || right_type.is_float() {
                            return Ok(TejxType::Float32);
                        }
                        if left_type == TejxType::Int64 || right_type == TejxType::Int64 {
                            return Ok(TejxType::Int64);
                        }
                        return Ok(TejxType::Int32);
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
                    return Ok(TejxType::Bool);
                }

                Ok(TejxType::Int32)
            }
            Expression::MemberAccessExpr {
                object,
                member,
                _line,
                _col,
                ..
            } => {
                let mut obj_type = self.check_expression(object)?.to_name();

                // Resolve alias if needed
                if let Some(sym) = self.lookup(&obj_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        obj_type = aliased.to_name();
                    }
                }

                // Special case for class names (static access)
                if let Expression::Identifier { name, .. } = &**object {
                    if let Some(s) = self.lookup(name) {
                        if s.ty.to_name() == "class" || s.ty.to_name() == "enum" {
                            if let Some(members) = self.class_members.get(name) {
                                if let Some(info) = members.get(member).cloned() {
                                    if !info.is_static {
                                        self.report_error_detailed(format!("Member '{}' is not static", member), *_line, *_col, "E0116", Some("Access this member on an instance, not the class itself"));
                                    }
                                    return Ok(TejxType::from_name(&self.substitute_generics(&info.ty.to_name(), name)));
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
                            let current_base = current.split('<').next().unwrap_or(current);
                            let obj_base = obj_type.split('<').next().unwrap_or(&obj_type);
                            if current_base != obj_base && !obj_type.starts_with("function") {
                                // Check hierarchy if needed, but for now simple check
                                self.report_error_detailed(format!("Member '{}' is private and can only be accessed within class '{}'", member, obj_base), *_line, *_col, "E0106", Some("Mark the member as 'public' in the class definition, or access it from within the class"));
                            }
                        } else {
                            self.report_error_detailed(format!("Member '{}' is private and can only be accessed within class '{}'", member, obj_type), *_line, *_col, "E0106", Some("Mark the member as 'public' in the class definition, or access it from within the class"));
                        }
                    }
                    return Ok(TejxType::from_name(&self.substitute_generics(&info.ty.to_name(), &obj_type)));
                }

                // Built-in 'length' property for arrays, strings, and slices
                if member == "length" {
                    if obj_type == "string"
                        || obj_type.ends_with("[]")
                        || (obj_type.starts_with("slice<") && obj_type.ends_with(">"))
                    {
                        return Ok(TejxType::Int32);
                    }
                }

                if let TejxType::Object(props) = TejxType::from_name(&obj_type) {
                    for (k, _, t) in props {
                        if k == *member {
                            return Ok(t);
                        }
                    }
                }

                if !obj_type.is_empty() && obj_type != "<inferred>" && !obj_type.starts_with("{") {
                    // Fallback for enums: default to int32 if known enum
                    if obj_type == "enum"
                        || self
                            .lookup(&obj_type)
                            .map(|s| s.ty.to_name() == "enum")
                            .unwrap_or(false)
                    {
                        return Ok(TejxType::Int32);
                    }
                }

                // --- UFCS Lookup ---
                // If not found as a member, check if there is a global function name(obj, ...)
                if member == "push" || member == "fill" {
                    println!("DEBUG UFCS LOOKUP: looking up '{}' for object type '{}'", member, obj_type);
                    if let Some(s) = self.lookup(member) {
                        println!("DEBUG UFCS FOUND: ty={:?}", s.ty);
                        if let TejxType::Function(_, ref ret) = s.ty {
                            if !s.params.is_empty() {
                                let first_param = &s.params[0];
                                let is_compat = self.are_types_compatible(first_param, &TejxType::from_name(&obj_type));
                                println!("DEBUG UFCS: are_types_compatible({}, {}) -> {}", first_param.to_name(), obj_type, is_compat);
                            } else {
                                println!("DEBUG UFCS: params is empty!");
                            }
                        } else {
                            println!("DEBUG UFCS: s.ty is not a function!");
                        }
                    } else {
                        println!("DEBUG UFCS: not found in lookup!");
                    }
                }

                if let Some(s) = self.lookup(member) {
                    if let TejxType::Function(_, ref ret) = s.ty {
                        if !s.params.is_empty() {
                            let first_param = &s.params[0];
                            let is_compat = self.are_types_compatible(first_param, &TejxType::from_name(&obj_type));
                            if is_compat {
                                // Full ty
                                let full_ty = TejxType::Function(s.params.clone(), ret.clone());
                                // Found a match! Return the function type but we keep note it's UFCS
                                // Actually, for type checking, we just return the function type.
                                // CodeGen will handle the translation.
                                return Ok(TejxType::from_name(&self.substitute_generics(&full_ty.to_name(), &obj_type)));
                            }
                        }
                    }
                }

                if !obj_type.is_empty() && obj_type != "<inferred>" && !obj_type.starts_with("{") {
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
                Ok(TejxType::from_name("<inferred>"))
            }
            Expression::SequenceExpr { expressions, .. } => {
                let mut last_type = TejxType::from_name("<inferred>");
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
                let target_type = self.check_expression(target)?.to_name();
                self.check_expression(index)?;

                let mut unwrapped_type = target_type.clone();
                if let Some(sym) = self.lookup(&unwrapped_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        unwrapped_type = aliased.to_name();
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
                    return Ok(TejxType::from_name(&res));
                }
                if unwrapped_type.starts_with("Array<") && unwrapped_type.ends_with(">") {
                    let inner = &unwrapped_type[6..unwrapped_type.len() - 1];
                    return Ok(TejxType::from_name(inner));
                }
                if unwrapped_type == "string" {
                    return Ok(TejxType::String);
                }
                Ok(TejxType::from_name("<inferred>"))
            }
            Expression::AssignmentExpr {
                target,
                value,
                _line,
                _col,
                ..
            } => {
                let target_ty_obj = match target.as_ref() {
                    Expression::Identifier { name, .. } => {
                        if let Some(s) = self.lookup(name) {
                            Ok(s.ty.clone())
                        } else {
                            self.report_error_detailed(format!("Undefined variable '{}'", name), *_line, *_col, "E0102", Some("Check the spelling or ensure the variable is declared before use"));
                            Ok(TejxType::from_name("<inferred>"))
                        }
                    }
                    _ => self.check_expression(target),
                }?;
                let target_type = target_ty_obj.to_name();

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
                    if let Ok(obj_type) = self.check_expression(object).map(|t| t.to_name()) {
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
                                    if s.ty.to_name() == "class" || s.ty.to_name() == "enum" {
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
                self.current_expected_type = Some(target_ty_obj.clone());
                let value_ty_obj = self.check_expression(value)?;
                let value_type = value_ty_obj.to_name();
                self.current_expected_type = prev_expected;
                
                if target_type != "<inferred>" && value_type != "<inferred>" {
                    self.check_numeric_bounds(value, &target_ty_obj, *_line, *_col);
                    if !self.is_assignable(&target_ty_obj, &value_ty_obj) {
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

                Ok(TejxType::from_name(&value_type))
            }
            Expression::CallExpr {
                callee,
                type_args: _,
                args,
                _line,
                _col,
            } => {
                let callee_str = callee.to_callee_name();

                if callee_str == "typeof" {
                    for arg in args {
                        self.check_expression(arg)?;
                    }
                    return Ok(TejxType::String);
                }
                if callee_str == "sizeof" {
                    for arg in args {
                        self.check_expression(arg)?;
                    }
                    return Ok(TejxType::Int32);
                }
                if callee_str == "super" {
                    if let Some(Symbol { ty: _type_name, .. }) = self.lookup("super") {
                        for arg in args {
                            self.check_expression(arg)?;
                        }
                        return Ok(TejxType::Void);
                    } else {
                        self.report_error_detailed("Cannot use 'super' here".to_string(), *_line, *_col, "E0115", Some("'super' can only be used inside a class that extends another class"));
                        return Ok(TejxType::from_name("<inferred>"));
                    }
                }

                let callee_type = self.check_expression(callee)?.to_name();
                let mut return_type = "<inferred>".to_string();
                let mut s_params = Vec::new();
                let mut is_variadic = false;

                let mut signature_found = false;

                // Always try symbol lookup to fill s_params and is_variadic exactly.
                if let Some(s) = self.lookup(&callee_str) {
                    if return_type == "<inferred>" && s.ty.to_name().starts_with("function:") {
                        let type_name_str = s.ty.to_name();
                        let parts: Vec<&str> = type_name_str.split(':').collect();
                        if parts.len() >= 2 {
                            let mut ret = parts[1].to_string();
                            if ret.ends_with(':') {
                                ret.pop();
                            }
                            return_type = ret;
                        }
                    } else if let TejxType::Function(_, ret) = &s.ty {
                        return_type = ret.to_name();
                    }
                    s_params = s.params.iter().map(|p| p.to_name()).collect();
                    is_variadic = s.is_variadic;
                    signature_found = true;
                }

                if !signature_found && (callee_type.starts_with("function:") || callee_type.contains("=>")) {
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

                let mut generic_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();

                // If `s_params` is still empty, fallback to looking up just the method name
                if !signature_found {
                    let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                    if let Some(s) = self.lookup(func_name) {
                        s_params = s.params.iter().map(|p| p.to_name()).collect();
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
                            "<inferred>".to_string()
                        } else if adjusted_i >= s_params.len() - 1 {
                            let last_param = &s_params[s_params.len() - 1];
                            if last_param.ends_with("[]") {
                                last_param[..last_param.len() - 2].to_string()
                            } else {
                                "<inferred>".to_string()
                            }
                        } else {
                            s_params[adjusted_i].clone()
                        }
                    } else if adjusted_i < s_params.len() {
                        s_params[adjusted_i].clone()
                    } else {
                        "<inferred>".to_string()
                    };
                    // For Array methods, transform `T` and `T[]` parameters based on instance context
                    // If UFCS, the method receiver is actually the object of the MemberAccessExpr
                    let mut resolved_receiver = String::new();
                    if let Expression::MemberAccessExpr { object, .. } = &**callee {
                        if let Ok(obj_type) = self.check_expression(object).map(|t| t.to_name()) {
                            resolved_receiver = obj_type;
                        }
                    } else if (callee_type.starts_with("function:") || callee_type.contains("=>")) && !args.is_empty() {
                        resolved_receiver = self.check_expression(&args[0]).map(|t| t.to_name()).unwrap_or_default();
                    }

                    if resolved_receiver.is_empty() {
                        resolved_receiver = callee_type.clone();
                    }

                    if let Some(s) = self.lookup(&resolved_receiver) {
                        if let Some(alias) = &s.aliased_type {
                            resolved_receiver = alias.to_name();
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
                    } else if resolved_receiver.starts_with("Promise<") {
                        let inner = &resolved_receiver[8..resolved_receiver.len() - 1];
                        if target_type == "T"
                            || target_type.starts_with("$MISSING_GENERIC_")
                            || target_type == "$0"
                        {
                            target_type = inner.to_string();
                        } else if target_type == "Promise<T>" {
                            target_type = format!("Promise<{}>", inner);
                        } else if target_type == "T[]"
                            || target_type == "$0[]"
                            || target_type.ends_with("[]") && target_type.starts_with("T")
                        {
                            target_type = format!("{}[]", inner);
                        } else if target_type == "Promise<T[]>" {
                            target_type = format!("Promise<{}[]>", inner);
                        }
                    }

                    if matches!(arg, Expression::LambdaExpr { .. }) {
                        if target_type.starts_with("function:") || target_type.contains("=>") {
                            let (_, parsed_params, _) = self.parse_signature(target_type.clone());

                            let parsed_params: Vec<TejxType> = parsed_params.into_iter().map(|p| TejxType::from_name(&p)).collect();
                            self.lambda_context_params = Some(parsed_params);
                        }
                    } else {
                        self.lambda_context_params = None;
                    }

                    let prev_expected = self.current_expected_type.take();
                    self.current_expected_type = Some(TejxType::from_name(&target_type));
                    let arg_type = self.check_expression(arg)?.to_name();
                    self.current_expected_type = prev_expected;
                    self.lambda_context_params = None;

                    let is_unknown_check = |t: &str| t == "<inferred>" || t.ends_with(":unknown");
                    if !target_type.is_empty() && !is_unknown_check(&target_type) {
                        let mut expected_obj = TejxType::from_name(&target_type);
                        if target_type.contains("=>") || target_type.starts_with("function:") {
                            let (ret, params, _) = self.parse_signature(target_type.clone());
                            let parts: Vec<&str> = ret.split(':').collect();
                            let actual_ret = if parts.len() >= 2 { parts[1] } else { &ret };
                            expected_obj = TejxType::Function(
                                params.iter().map(|p| TejxType::from_name(p)).collect(),
                                Box::new(TejxType::from_name(actual_ret)),
                            );
                        }

                        let mut actual_obj = TejxType::from_name(&arg_type);
                        if arg_type.contains("=>") || arg_type.starts_with("function:") {
                            let (ret, params, _) = self.parse_signature(arg_type.clone());
                            let parts: Vec<&str> = ret.split(':').collect();
                            let actual_ret = if parts.len() >= 2 { parts[1] } else { &ret };
                            actual_obj = TejxType::Function(
                                params.iter().map(|p| TejxType::from_name(p)).collect(),
                                Box::new(TejxType::from_name(actual_ret)),
                            );
                        }

                        if !self.are_types_compatible(&expected_obj, &actual_obj) {
                            // Skip error if either side is a generic type param (defined as 'unknown' in scope)
                            let is_generic_param = |t: &str| {
                                if t.starts_with("$MISSING_GENERIC_") {
                                    return true;
                                }
                                if let Some(sym) = self.lookup(t) {
                                    sym.ty.to_name() == "<inferred>"
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
                                let orig = sym.params[adjusted_i].to_name();
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
                    }

                    let is_generic_param_check = |t: &str| {
                        if t.starts_with("$MISSING_GENERIC_") {
                            return true;
                        }
                        t.len() <= 2
                            && t.chars().next().map_or(false, |c| c.is_uppercase())
                            && t.chars().all(|c| c.is_alphanumeric())
                    };
                    if is_generic_param_check(&target_type) && arg_type != "<inferred>" {
                        generic_map.insert(target_type.clone(), arg_type.clone());
                    }
                }

                if !generic_map.is_empty() {
                    let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                    if let Some(s) = self.lookup(func_name) {
                        if !s.generic_params.is_empty() {
                            let mut concrete_args = Vec::new();
                            for gp in &s.generic_params {
                                if let Some(concrete) = generic_map.get(&gp.name) {
                                    if let Some(bound) = &gp.bound {
                                        let bound_str = bound.to_string();
                                        if !self.is_assignable(&TejxType::from_name(&bound_str), &TejxType::from_name(concrete)) {
                                            self.report_error_detailed(
                                                format!("Type '{}' does not satisfy constraint '{}' for generic parameter '{}'", concrete, bound_str, gp.name),
                                                *_line,
                                                *_col,
                                                "E0120",
                                                Some(&format!("Provide a type that satisfies the constraint '{}'", bound_str))
                                            );
                                        }
                                    }
                                    concrete_args.push(TejxType::from_name(&concrete));
                                } else {
                                    concrete_args.push(TejxType::from_name("<inferred>"));
                                }
                            }
                            self.function_instantiations
                                .entry(func_name.to_string())
                                .or_default()
                                .insert(concrete_args);
                        }
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

                if callee_type == "<inferred>" && return_type == "<inferred>" {
                    return Ok(TejxType::from_name("<inferred>"));
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

                Ok(TejxType::from_name(&final_ret))
            }
            Expression::ObjectLiteralExpr { entries, _spreads, .. } => {
                let mut props = Vec::new();

                for (key, val_expr) in entries {
                    let val_ty = self.check_expression(val_expr)?;
                    props.push((key.clone(), false, val_ty));
                }

                for spread_expr in _spreads {
                    let spread_ty = self.check_expression(spread_expr)?;
                    // resolve typedefs if necessary
                    let mut resolved = spread_ty.clone();
                    if let TejxType::Class(name, _) = &resolved {
                        if let Some(sym) = self.lookup(name) {
                            if let Some(aliased) = &sym.aliased_type {
                                resolved = aliased.clone();
                            }
                        }
                    }
                    if let TejxType::Object(spread_props) = TejxType::from_name(&resolved.to_name()) {
                        for (k, opt, t) in spread_props {
                            props.push((k, opt, t));
                        }
                    }
                }

                Ok(TejxType::Object(props))
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
                    let mut expected_inner: Option<TejxType> = None;
                    if let Some(expected) = &self.current_expected_type {
                        expected_inner = match expected {
                            TejxType::DynamicArray(inner) | TejxType::FixedArray(inner, _) | TejxType::Slice(inner) => Some(*inner.clone()),
                            TejxType::Class(name, generics) if name == "Array" && generics.len() == 1 => Some(generics[0].clone()),
                            _ => None,
                        };
                    }

                    for i in 0..elements.len() {
                        let prev_expected = self.current_expected_type.take();
                        let prev_lambda_ctx = self.lambda_context_params.take();
                        
                        if let Some(inner) = &expected_inner {
                            self.current_expected_type = Some(inner.clone());
                            if let TejxType::Function(params, _) = inner {
                                self.lambda_context_params = Some(params.clone());
                            }
                        }

                        let mut elem_ty = self.check_expression(&elements[i])?.to_name();

                        self.current_expected_type = prev_expected;
                        self.lambda_context_params = prev_lambda_ctx;

                        if let Expression::SpreadExpr { .. } = elements[i] {
                            let mut elem_ty_str = elem_ty.clone();
                            if elem_ty_str.ends_with("[]") {
                                elem_ty_str = elem_ty_str[..elem_ty_str.len() - 2].to_string();
                            } else if elem_ty_str.starts_with("Array<") {
                                elem_ty_str = elem_ty_str[6..elem_ty_str.len() - 1].to_string();
                            }
                            elem_ty = TejxType::from_name(&elem_ty_str).to_name();
                        }

                        if let Some(first_type) = &first_type_opt {
                            let elem_ty_str = elem_ty.clone();
                            if &elem_ty_str != first_type && first_type != "<inferred>" {
                                let common = self.get_common_ancestor(&TejxType::from_name(first_type), &TejxType::from_name(&elem_ty_str)).to_name();
                                if common == "<inferred>" {
                                    self.report_error_detailed(
                                        format!("Array elements have incompatible types: '{}' and '{}'", first_type, elem_ty),
                                        *_line,
                                        *_col,
                                        "E0100",
                                        Some("All elements in an array literal must have the same type")
                                    );
                                    first_type_opt = Some("<inferred>".to_string());
                                } else {
                                    first_type_opt = Some(common);
                                }
                            }
                        } else {
                            first_type_opt = Some(elem_ty.clone());
                        }
                    }
                    let t = first_type_opt.unwrap_or_else(|| "<inferred>".to_string()) + "[]";
                    *ty.borrow_mut() = Some(t.clone());
                    Ok(TejxType::from_name(&t))
                } else {
                    let mut t = "[]".to_string();
                    if let Some(expected) = &self.current_expected_type {
                        let expected_str = expected.to_name();
                        if expected_str.ends_with("[]") || expected_str.starts_with("Array<") {
                            t = expected_str;
                        }
                    }
                    *ty.borrow_mut() = Some(t.clone());
                    Ok(TejxType::from_name(&t))
                }
            }
            Expression::SpreadExpr { _expr, .. } => self.check_expression(_expr),

            Expression::AwaitExpr { expr, _line, _col } => {
                if !self.current_function_is_async && self.current_function_return.is_some() {
                    self.report_error_detailed(
                        "'await' can only be used inside 'async' function".to_string(),
                        *_line,
                        *_col,
                        "E0113",
                        Some("Mark the enclosing function with 'async' keyword"),
                    );
                }
                let t = self.check_expression(expr)?.to_name();
                if t.starts_with("Promise<") {
                    Ok(TejxType::from_name(&t[8..t.len() - 1]))
                } else {
                    Ok(TejxType::from_name(&t))
                }
            }
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                let target_type = self.check_expression(target)?.to_name();
                self.check_expression(index)?;
                let mut unwrapped_type = target_type.clone();
                if let Some(sym) = self.lookup(&unwrapped_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        unwrapped_type = aliased.to_name();
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
                    return Ok(TejxType::from_name(&unwrapped_type[..unwrapped_type.len() - 2]));
                }
                if unwrapped_type.starts_with("Array<") && unwrapped_type.ends_with(">") {
                    let inner = &unwrapped_type[6..unwrapped_type.len() - 1];
                    return Ok(TejxType::from_name(inner));
                }
                Ok(TejxType::from_name("<inferred>"))
            }
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                let mut obj_type = self.check_expression(object)?.to_name();
                if let Some(sym) = self.lookup(&obj_type) {
                    if let Some(aliased) = &sym.aliased_type {
                        obj_type = aliased.to_name();
                    }
                }
                if obj_type.contains('|') {
                    obj_type = obj_type
                        .split('|')
                        .map(|s| s.trim().to_string())
                        .find(|s| s != "None" && !s.is_empty())
                        .unwrap_or(obj_type.clone());
                }

                if let TejxType::Object(props) = TejxType::from_name(&obj_type) {
                    for (k, _, t) in props {
                        if k == *member {
                            return Ok(t);
                        }
                    }
                }

                if let Some(info) = self.resolve_instance_member(&obj_type, member) {
                    return Ok(TejxType::from_name(&self.substitute_generics(&info.ty.to_name(), &obj_type)));
                }

                Ok(TejxType::from_name("<inferred>"))
            }
            Expression::NullishCoalescingExpr { _left, _right, .. } => {
                let left_ty = self.check_expression(_left)?.to_name();
                let right_ty = self.check_expression(_right)?.to_name();
                // Strip "Option<>" or "| None" from left_ty ideally, but
                if left_ty != "<inferred>" {
                    if left_ty.starts_with("Option<") {
                        Ok(TejxType::from_name(&left_ty[7..left_ty.len() - 1]))
                    } else if left_ty.ends_with(" | None") {
                        Ok(TejxType::from_name(&left_ty[..left_ty.len() - 7]))
                    } else if left_ty == "None" {
                        Ok(TejxType::from_name(&right_ty))
                    } else {
                        Ok(TejxType::from_name(&left_ty))
                    }
                } else {
                    Ok(TejxType::from_name(&right_ty))
                }
            }
            Expression::TernaryExpr {
                _condition,
                _true_branch,
                _false_branch,
                ..
            } => {
                self.check_expression(_condition)?;
                let true_ty = self.check_expression(_true_branch)?.to_name();
                let false_ty = self.check_expression(_false_branch)?.to_name();
                if true_ty == false_ty {
                    Ok(TejxType::from_name(&true_ty))
                } else if true_ty != "<inferred>" {
                    Ok(TejxType::from_name(&true_ty))
                } else {
                    Ok(TejxType::from_name(&false_ty))
                }
            }
            Expression::OptionalCallExpr {
                callee,
                type_args: _,
                args,
                _line,
                _col,
            } => {
                // Try to resolve return type from callee
                let callee_type = self.check_expression(callee)?.to_name();
                if callee_type.starts_with("function:") {
                    let (ret, _, _) = self.parse_signature(callee_type);
                    Ok(TejxType::from_name(&ret))
                } else {
                    Ok(TejxType::from_name(&callee_type))
                }
            }
            Expression::NewExpr {
                class_name,
                args,
                _line,
                _col,
            } => {
                self.register_instantiation(class_name, *_line, *_col);
                // Generic type parameters are inferred from the variable declaration's
                // type annotation (e.g., `let m: Map<string, int> = new Map()`),
                // so we don't require explicit type args on the constructor call.
                if !self.is_valid_type(&TejxType::from_name(class_name)) {
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
                Ok(TejxType::from_name(class_name))
            }
            Expression::ThisExpr { _line, _col } => {
                if let Some(sym) = self.lookup("this") {
                    println!("DEBUG expr.rs ThisExpr evaluated to: {}", sym.ty.to_name());
                    Ok(sym.ty.clone())
                } else {
                    self.report_error_detailed(
                        "Using 'super' outside of a derived class".to_string(),
                        *_line,
                        *_col,
                        "E0115",
                        Some("'super' can only be used inside methods of a class that extends another class"),
                    );
                    Ok(TejxType::from_name("<inferred>"))
                }
            }
            Expression::SuperExpr { _line, _col } => {
                if let Some(s) = self.lookup("super") {
                    Ok(s.ty.clone())
                } else {
                    self.report_error_detailed(
                        "Using 'super' outside of a derived class".to_string(),
                        *_line,
                        *_col,
                        "E0115",
                        Some("'super' can only be used inside methods of a class that extends another class"),
                    );
                    Ok(TejxType::from_name("<inferred>"))
                }
            }
        }
    }
}
