use super::*;
use crate::ast::*;
use crate::token::TokenType;
use std::collections::HashMap;

impl TypeChecker {
    fn is_unresolved_generic_name(name: &str) -> bool {
        name.starts_with("$MISSING_GENERIC_")
            || name == "$0"
            || (name.len() <= 2
                && name.chars().next().map_or(false, |c| c.is_uppercase())
                && name.chars().all(|c| c.is_alphanumeric()))
    }

    fn contains_unresolved_generic_type(ty: &TejxType) -> bool {
        match ty {
            TejxType::Class(name, generics) => {
                Self::is_unresolved_generic_name(name)
                    || generics.iter().any(Self::contains_unresolved_generic_type)
            }
            TejxType::DynamicArray(inner)
            | TejxType::FixedArray(inner, _)
            | TejxType::Slice(inner) => Self::contains_unresolved_generic_type(inner),
            TejxType::Function(params, ret) => {
                params.iter().any(Self::contains_unresolved_generic_type)
                    || Self::contains_unresolved_generic_type(ret)
            }
            TejxType::Union(types) => types.iter().any(Self::contains_unresolved_generic_type),
            TejxType::Object(props) => props
                .iter()
                .any(|(_, _, ty)| Self::contains_unresolved_generic_type(ty)),
            _ => false,
        }
    }

    fn collect_generic_bindings_from_types(
        formal: &TejxType,
        actual: &TejxType,
        generic_map: &mut HashMap<String, String>,
    ) {
        match formal {
            TejxType::Class(name, generics)
                if generics.is_empty() && Self::is_unresolved_generic_name(name) =>
            {
                generic_map.insert(name.clone(), actual.to_name());
            }
            TejxType::Class(formal_name, formal_generics) => {
                if let TejxType::Class(actual_name, actual_generics) = actual {
                    if formal_name == actual_name && formal_generics.len() == actual_generics.len()
                    {
                        for (formal_arg, actual_arg) in
                            formal_generics.iter().zip(actual_generics.iter())
                        {
                            Self::collect_generic_bindings_from_types(
                                formal_arg,
                                actual_arg,
                                generic_map,
                            );
                        }
                    }
                }
            }
            TejxType::DynamicArray(formal_inner) => {
                if let TejxType::DynamicArray(actual_inner) = actual {
                    Self::collect_generic_bindings_from_types(
                        formal_inner,
                        actual_inner,
                        generic_map,
                    );
                }
            }
            TejxType::FixedArray(formal_inner, _) => match actual {
                TejxType::FixedArray(actual_inner, _) | TejxType::DynamicArray(actual_inner) => {
                    Self::collect_generic_bindings_from_types(
                        formal_inner,
                        actual_inner,
                        generic_map,
                    );
                }
                _ => {}
            },
            TejxType::Slice(formal_inner) => match actual {
                TejxType::Slice(actual_inner)
                | TejxType::DynamicArray(actual_inner)
                | TejxType::FixedArray(actual_inner, _) => {
                    Self::collect_generic_bindings_from_types(
                        formal_inner,
                        actual_inner,
                        generic_map,
                    );
                }
                _ => {}
            },
            TejxType::Function(formal_params, formal_ret) => {
                if let TejxType::Function(actual_params, actual_ret) = actual {
                    for (formal_param, actual_param) in
                        formal_params.iter().zip(actual_params.iter())
                    {
                        Self::collect_generic_bindings_from_types(
                            formal_param,
                            actual_param,
                            generic_map,
                        );
                    }
                    Self::collect_generic_bindings_from_types(formal_ret, actual_ret, generic_map);
                }
            }
            TejxType::Object(formal_props) => {
                if let TejxType::Object(actual_props) = actual {
                    for (formal_name, _, formal_ty) in formal_props {
                        if let Some((_, _, actual_ty)) =
                            actual_props.iter().find(|(name, _, _)| name == formal_name)
                        {
                            Self::collect_generic_bindings_from_types(
                                formal_ty,
                                actual_ty,
                                generic_map,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn builtin_member_function_type(&self, receiver: &TejxType, member: &str) -> Option<TejxType> {
        if receiver == &TejxType::String {
            return match member {
                "length" => Some(TejxType::Function(
                    vec![TejxType::String],
                    Box::new(TejxType::Int32),
                )),
                "concat" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::String],
                    Box::new(TejxType::String),
                )),
                "includes" | "startsWith" | "endsWith" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::String],
                    Box::new(TejxType::Bool),
                )),
                "indexOf" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::String],
                    Box::new(TejxType::Int32),
                )),
                "toUpperCase" | "toLowerCase" | "trim" | "trimStart" | "trimEnd" => Some(
                    TejxType::Function(vec![TejxType::String], Box::new(TejxType::String)),
                ),
                "substring" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::Int32, TejxType::Int32],
                    Box::new(TejxType::String),
                )),
                "split" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::String],
                    Box::new(TejxType::DynamicArray(Box::new(TejxType::String))),
                )),
                "repeat" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::Int32],
                    Box::new(TejxType::String),
                )),
                "replace" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::String, TejxType::String],
                    Box::new(TejxType::String),
                )),
                "padStart" | "padEnd" => Some(TejxType::Function(
                    vec![TejxType::String, TejxType::Int32, TejxType::String],
                    Box::new(TejxType::String),
                )),
                _ => None,
            };
        }

        if receiver.is_array() || receiver.is_slice() {
            let elem = receiver.get_array_element_type();
            let u = TejxType::Class("U".to_string(), vec![]);
            let is_fixed_or_slice = matches!(receiver, TejxType::FixedArray(_, _) | TejxType::Slice(_));
            if is_fixed_or_slice && matches!(member, "push" | "pop" | "shift" | "unshift") {
                return None;
            }
            return match member {
                "length" => Some(TejxType::Function(
                    vec![receiver.clone()],
                    Box::new(TejxType::Int32),
                )),
                "push" => Some(TejxType::Function(
                    vec![receiver.clone(), elem.clone()],
                    Box::new(TejxType::Int32),
                )),
                "pop" | "shift" => Some(TejxType::Function(
                    vec![receiver.clone()],
                    Box::new(elem.clone()),
                )),
                "unshift" => Some(TejxType::Function(
                    vec![receiver.clone(), elem.clone()],
                    Box::new(TejxType::Int32),
                )),
                "indexOf" => Some(TejxType::Function(
                    vec![receiver.clone(), elem.clone()],
                    Box::new(TejxType::Int32),
                )),
                "concat" | "slice" | "reverse" | "fill" => Some(TejxType::Function(
                    vec![receiver.clone(), elem.clone()],
                    Box::new(TejxType::DynamicArray(Box::new(elem.clone()))),
                )),
                "filter" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![elem.clone(), TejxType::Int32],
                            Box::new(TejxType::Bool),
                        ),
                    ],
                    Box::new(TejxType::DynamicArray(Box::new(elem.clone()))),
                )),
                "join" => Some(TejxType::Function(
                    vec![receiver.clone(), TejxType::String],
                    Box::new(TejxType::String),
                )),
                "sort" => Some(TejxType::Function(
                    vec![receiver.clone()],
                    Box::new(TejxType::Void),
                )),
                "forEach" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![elem.clone(), TejxType::Int32],
                            Box::new(TejxType::Void),
                        ),
                    ],
                    Box::new(TejxType::Void),
                )),
                "map" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![elem.clone(), TejxType::Int32],
                            Box::new(u.clone()),
                        ),
                    ],
                    Box::new(TejxType::DynamicArray(Box::new(u.clone()))),
                )),
                "reduce" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![u.clone(), elem.clone(), TejxType::Int32],
                            Box::new(u.clone()),
                        ),
                        u.clone(),
                    ],
                    Box::new(u.clone()),
                )),
                "find" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![elem.clone(), TejxType::Int32],
                            Box::new(TejxType::Bool),
                        ),
                    ],
                    Box::new(elem.clone()),
                )),
                "findIndex" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![elem.clone(), TejxType::Int32],
                            Box::new(TejxType::Bool),
                        ),
                    ],
                    Box::new(TejxType::Int32),
                )),
                "every" | "some" => Some(TejxType::Function(
                    vec![
                        receiver.clone(),
                        TejxType::Function(
                            vec![elem.clone(), TejxType::Int32],
                            Box::new(TejxType::Bool),
                        ),
                    ],
                    Box::new(TejxType::Bool),
                )),
                "includes" => Some(TejxType::Function(
                    vec![receiver.clone(), elem.clone()],
                    Box::new(TejxType::Bool),
                )),
                _ => None,
            };
        }

        None
    }

    pub(crate) fn check_expression(&mut self, expr: &Expression) -> Result<TejxType, ()> {
        match expr {
            Expression::NumberLiteral { value, .. } => {
                if let Some(expected) = &self.current_expected_type {
                    if expected.is_numeric() {
                        return Ok(expected.clone());
                    }
                }
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
                        if right_type.is_numeric()
                            || right_type == TejxType::from_name("<inferred>")
                        {
                            Ok(right_type)
                        } else {
                            self.report_error_detailed(
                                format!(
                                    "Unary '-' cannot be applied to type '{}'",
                                    right_type.to_name()
                                ),
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

                self.lambda_inferred_returns
                    .insert((*_line, *_col), TejxType::from_name(&final_ret));

                self.current_function_return = prev_return;
                self.exit_scope();

                Ok(TejxType::Function(
                    actual_param_types,
                    Box::new(TejxType::from_name(&final_ret)),
                ))
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
                _is_namespace,
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
                    if name == "Promise" && member == "all" {
                        let missing = TejxType::Class("$MISSING_GENERIC_0".to_string(), vec![]);
                        return Ok(TejxType::Function(
                            vec![TejxType::DynamicArray(Box::new(TejxType::Class(
                                "Promise".to_string(),
                                vec![missing.clone()],
                            )))],
                            Box::new(TejxType::Class(
                                "Promise".to_string(),
                                vec![TejxType::DynamicArray(Box::new(missing))],
                            )),
                        ));
                    }
                    if let Some(s) = self.lookup(name) {
                        if s.ty.to_name() == "class" || s.ty.to_name() == "enum" {
                            if let Some(members) = self.class_members.get(name) {
                                if let Some(info) = members.get(member).cloned() {
                                    if !info.is_static {
                                        self.report_error_detailed(format!("Member '{}' is not static", member), *_line, *_col, "E0116", Some("Access this member on an instance, not the class itself"));
                                    }
                                    return Ok(TejxType::from_name(
                                        &self.substitute_generics(&info.ty.to_name(), name),
                                    ));
                                } else {
                                    let kind = if s.ty.to_name() == "enum" {
                                        "enum"
                                    } else if *_is_namespace {
                                        "namespace"
                                    } else {
                                        "class"
                                    };
                                    let available = self.collect_member_names(name, true);
                                    let hint = if available.is_empty() {
                                        None
                                    } else {
                                        Some(format!(
                                            "Available {} members: {}",
                                            kind,
                                            available.join(", ")
                                        ))
                                    };
                                    self.report_error_detailed(
                                        format!(
                                            "Member '{}' does not exist on {} '{}'",
                                            member, kind, name
                                        ),
                                        *_line,
                                        *_col,
                                        "E0105",
                                        hint.as_deref(),
                                    );
                                    return Ok(TejxType::from_name("<inferred>"));
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
                    return Ok(TejxType::from_name(
                        &self.substitute_generics(&info.ty.to_name(), &obj_type),
                    ));
                }

                let obj_ty = TejxType::from_name(&obj_type);
                if matches!(obj_ty, TejxType::FixedArray(_, _) | TejxType::Slice(_))
                    && matches!(member.as_str(), "push" | "pop" | "shift" | "unshift")
                {
                    self.report_error_detailed(
                        format!(
                            "Cannot call '{}' on fixed-size array or slice '{}'",
                            member, obj_type
                        ),
                        *_line,
                        *_col,
                        "E0105",
                        Some("Fixed-size arrays and slices do not support length-changing methods"),
                    );
                    return Ok(TejxType::from_name("<inferred>"));
                }
                // Built-in 'length' property for arrays, strings, and slices
                if member == "length" {
                    if obj_ty == TejxType::String || obj_ty.is_array() || obj_ty.is_slice() {
                        return Ok(TejxType::Int32);
                    }
                }
                if let Some(builtin_ty) = self.builtin_member_function_type(&obj_ty, member) {
                    return Ok(builtin_ty);
                }

                if let TejxType::Object(props) = TejxType::from_name(&obj_type) {
                    for (k, _, t) in props {
                        if k == *member {
                            return Ok(t);
                        }
                    }
                    // Allow dynamic extension on structural objects.
                    return Ok(TejxType::Any);
                }

                if TejxType::from_name(&obj_type) == TejxType::Any {
                    return Ok(TejxType::Any);
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
                let allow_ufcs = (obj_ty.is_array() || obj_ty.is_slice() || obj_ty == TejxType::String)
                    && obj_type != "any"
                    && obj_type != "<inferred>"
                    && !obj_type.is_empty();
                if allow_ufcs {
                    if let Some(s) = self.lookup(member) {
                        if let TejxType::Function(_, ref ret) = s.ty {
                            if !s.params.is_empty() {
                                let first_param = &s.params[0];
                                let is_compat = self
                                    .are_types_compatible(first_param, &TejxType::from_name(&obj_type));
                                if is_compat {
                                    // Full ty
                                    let full_ty = TejxType::Function(s.params.clone(), ret.clone());
                                    // Found a match! Return the function type but we keep note it's UFCS
                                    // Actually, for type checking, we just return the function type.
                                    // CodeGen will handle the translation.
                                    return Ok(TejxType::from_name(
                                        &self.substitute_generics(&full_ty.to_name(), &obj_type),
                                    ));
                                }
                            }
                        }
                    }
                }

                if !obj_type.is_empty() && obj_type != "<inferred>" && !obj_type.starts_with("{") {
                    let available = self.collect_member_names(&obj_type, false);
                    let hint = if available.is_empty() {
                        None
                    } else {
                        Some(format!("Available members: {}", available.join(", ")))
                    };
                    self.report_error_detailed(
                        format!(
                            "Property '{}' does not exist on type '{}'",
                            member, obj_type
                        ),
                        *_line,
                        *_col,
                        "E0105",
                        hint.as_deref().or(Some("Check the property name or define it in the class")),
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
                let target_ty = self.check_expression(target)?;
                self.check_expression(index)?;

                let mut unwrapped_type = target_ty.to_name();
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
                        .unwrap_or(target_ty.to_name());
                }

                let parsed = TejxType::from_name(&unwrapped_type);
                if let TejxType::FixedArray(_, size) = &parsed {
                    let mut const_index: Option<i64> = None;
                    match index.as_ref() {
                        Expression::NumberLiteral { value, .. } => {
                            if value.fract() == 0.0 {
                                const_index = Some(*value as i64);
                            }
                        }
                        Expression::UnaryExpr { op, right, .. } => {
                            if matches!(op, TokenType::Minus) {
                                if let Expression::NumberLiteral { value, .. } = right.as_ref() {
                                    if value.fract() == 0.0 {
                                        const_index = Some(-(*value as i64));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    if let Some(idx) = const_index {
                        if idx < 0 || (idx as usize) >= *size {
                            self.report_error_detailed(
                                format!(
                                    "Array index {} out of bounds for fixed array length {}",
                                    idx, size
                                ),
                                *_line,
                                *_col,
                                "E0100",
                                Some("Use a valid index within the fixed array bounds"),
                            );
                        }
                    }
                }
                if parsed == TejxType::Any {
                    return Ok(TejxType::Any);
                }
                if parsed.is_array() || parsed.is_slice() {
                    return Ok(parsed.get_array_element_type());
                }
                if parsed == TejxType::String {
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
                            if let TejxType::Object(props) = TejxType::from_name(&obj_type) {
                                let exists = props.iter().any(|(name, _, _)| name == member);
                                if !exists {
                                    // Structural objects are open: allow dynamic property additions.
                                }
                            }
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

                let value_ty_obj = self.with_expected_type(Some(target_ty_obj.clone()), |s| {
                    s.check_expression(value)
                })?;
                let value_type = value_ty_obj.to_name();

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
                type_args,
                args,
                _line,
                _col,
            } => {
                let mut callee_str = callee.to_callee_name();
                if let Expression::MemberAccessExpr { object, member, .. } = &**callee {
                    if let Expression::Identifier { name, .. } = object.as_ref() {
                        if name == "Promise" && member == "all" {
                            callee_str = "Promise_all".to_string();
                        }
                    }
                }

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

                let mut _signature_found = false;

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
                    _signature_found = true;
                }

                if !_signature_found
                    && (callee_type.starts_with("function:") || callee_type.contains("=>"))
                {
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
                    _signature_found = true;
                }

                if !_signature_found {
                    if let Expression::MemberAccessExpr { object, member, .. } = &**callee {
                        if let Ok(receiver_ty) = self.check_expression(object) {
                            if let Some(TejxType::Function(params, ret)) =
                                self.builtin_member_function_type(&receiver_ty, member)
                            {
                                return_type = ret.to_name();
                                s_params = params.iter().map(|p| p.to_name()).collect();
                                is_variadic = false;
                                _signature_found = true;
                            }
                        }
                    }
                }

                let mut generic_map: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                let mut call_generic_params: Vec<crate::ast::GenericParam> = Vec::new();
                let mut call_generic_owner = callee_str.clone();
                let mut member_lookup_resolved = false;
                if let Expression::MemberAccessExpr { object, member, .. } = &**callee {
                    if let Ok(obj_type) = self.check_expression(object) {
                        if let Some(info) =
                            self.resolve_instance_member(&obj_type.to_name(), member)
                        {
                            member_lookup_resolved = true;
                            if !info.generic_params.is_empty() {
                                call_generic_params = info.generic_params.clone();
                                call_generic_owner = member.clone();
                            }
                        } else if obj_type == TejxType::String
                            && self
                                .builtin_member_function_type(&obj_type, member)
                                .is_some()
                        {
                            // String builtins are not UFCS generics; skip global generic lookup.
                            member_lookup_resolved = true;
                        }
                    }
                }
                if call_generic_params.is_empty() && !member_lookup_resolved {
                    let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                    if let Some(s) = self.lookup(func_name) {
                        call_generic_params = s.generic_params.clone();
                        call_generic_owner = func_name.to_string();
                    }
                }

                // Pre-fill generic bindings from the receiver type (e.g., Stack<int> -> T = int)
                if let Expression::MemberAccessExpr { object, .. } = &**callee {
                    if let Ok(mut receiver_ty) = self.check_expression(object) {
                        if let TejxType::Union(parts) = receiver_ty.clone() {
                            if let Some(non_none) = parts
                                .iter()
                                .find(|t| t.to_name() != "None" && t.to_name() != "<inferred>")
                            {
                                receiver_ty = non_none.clone();
                            }
                        }
                        if let Some(sym) = self.lookup(&receiver_ty.to_name()) {
                            if let Some(aliased) = &sym.aliased_type {
                                receiver_ty = aliased.clone();
                            }
                        }
                        if let TejxType::Class(base, args) = receiver_ty {
                            if let Some(sym) = self.lookup(&base) {
                                for (gp, arg) in sym.generic_params.iter().zip(args.iter()) {
                                    generic_map
                                        .entry(gp.name.clone())
                                        .or_insert(arg.to_name());
                                }
                            }
                        }
                    }
                }

                // If `s_params` is still empty, fallback to looking up just the method name
                if !_signature_found {
                    let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                    if let Some(s) = self.lookup(func_name) {
                        s_params = s.params.iter().map(|p| p.to_name()).collect();
                        is_variadic = s.is_variadic;
                        _signature_found = true;
                    }
                }

                let mut param_offset = 0;
                if let Expression::MemberAccessExpr { .. } = &**callee {
                    if s_params.len() > 0 && s_params.len() >= args.len() + 1 {
                        param_offset = 1;
                    }
                }

                let parse_type_string =
                    |tc: &TypeChecker, ty_str: &str| -> TejxType {
                        if ty_str.starts_with("function:") || ty_str.contains("=>") {
                            let (ret, params, _) = tc.parse_signature(ty_str.to_string());
                            let parts: Vec<&str> = ret.split(':').collect();
                            let actual_ret = if parts.len() >= 2 { parts[1] } else { &ret };
                            TejxType::Function(
                                params.iter().map(|p| TejxType::from_name(p)).collect(),
                                Box::new(TejxType::from_name(actual_ret)),
                            )
                        } else {
                            TejxType::from_name(ty_str)
                        }
                    };

                let apply_bindings_to_type_str =
                    |tc: &TypeChecker, ty_str: &str, bindings: &HashMap<String, TejxType>| -> String {
                        let parsed = parse_type_string(tc, ty_str);
                        parsed.substitute_generics(bindings).to_name()
                    };

                let mut explicit_generic_bindings: HashMap<String, TejxType> = HashMap::new();
                let mut explicit_type_args_valid = false;
                let explicit_type_args =
                    type_args.as_ref().map(|args| args.iter().map(TejxType::from_node).collect::<Vec<_>>());
                let explicit_type_args_used = explicit_type_args.is_some();
                if let Some(explicit_args) = explicit_type_args.as_ref() {
                    if call_generic_params.is_empty() {
                        self.report_error_detailed(
                            format!(
                                "Type arguments are not allowed for non-generic call '{}'",
                                callee_str
                            ),
                            *_line,
                            *_col,
                            "E0122",
                            Some("Remove the explicit type arguments"),
                        );
                    } else if explicit_args.len() != call_generic_params.len() {
                        self.report_error_detailed(
                            format!(
                                "Generic type argument count mismatch for '{}': expected {}, got {}",
                                call_generic_owner,
                                call_generic_params.len(),
                                explicit_args.len()
                            ),
                            *_line,
                            *_col,
                            "E0122",
                            Some("Provide the correct number of type arguments"),
                        );
                    } else {
                        explicit_type_args_valid = true;
                        for (gp, concrete) in call_generic_params.iter().zip(explicit_args.iter()) {
                            if !self.is_valid_type(concrete) {
                                self.report_error_detailed(
                                    format!(
                                        "Unknown data type: '{}' for generic parameter '{}'",
                                        concrete.to_name(),
                                        gp.name
                                    ),
                                    *_line,
                                    *_col,
                                    "E0101",
                                    Some("Provide a valid concrete type"),
                                );
                                explicit_type_args_valid = false;
                                break;
                            }
                            if let Some(bound) = &gp.bound {
                                let bound_ty = TejxType::from_node(bound);
                                if !self.is_assignable(&bound_ty, concrete) {
                                    self.report_error_detailed(
                                        format!(
                                            "Type '{}' does not satisfy constraint '{}' for generic parameter '{}'",
                                            concrete.to_name(),
                                            bound_ty.to_name(),
                                            gp.name
                                        ),
                                        *_line,
                                        *_col,
                                        "E0120",
                                        Some(&format!(
                                            "Provide a type that satisfies the constraint '{}'",
                                            bound_ty.to_name()
                                        )),
                                    );
                                    explicit_type_args_valid = false;
                                    break;
                                }
                            }
                            explicit_generic_bindings.insert(gp.name.clone(), concrete.clone());
                            generic_map.entry(gp.name.clone()).or_insert_with(|| concrete.to_name());
                        }
                    }
                }

                if explicit_type_args_valid && !explicit_generic_bindings.is_empty() {
                    s_params = s_params
                        .iter()
                        .map(|p| apply_bindings_to_type_str(self, p, &explicit_generic_bindings))
                        .collect();
                    return_type = apply_bindings_to_type_str(
                        self,
                        &return_type,
                        &explicit_generic_bindings,
                    );
                }

                if param_offset == 1 {
                    let mut resolved_receiver = String::new();
                    if let Expression::MemberAccessExpr { object, .. } = &**callee {
                        resolved_receiver = self
                            .check_expression(object)
                            .map(|t| t.to_name())
                            .unwrap_or_default();
                    }

                    if let Some(first_param) = s_params.first() {
                        let receiver_ty = TejxType::from_name(&resolved_receiver);
                        if receiver_ty.is_array() || receiver_ty.is_slice() {
                            let inner = receiver_ty.get_array_element_type().to_name();
                            if Self::is_unresolved_generic_name(first_param) {
                                generic_map.entry(first_param.clone()).or_insert(inner.clone());
                            } else if first_param.ends_with("[]") {
                                let base = &first_param[..first_param.len() - 2];
                                if Self::is_unresolved_generic_name(base) {
                                    generic_map.entry(base.to_string()).or_insert(inner.clone());
                                }
                            }
                        } else if resolved_receiver.starts_with("Promise<")
                            && resolved_receiver.ends_with('>')
                        {
                            let inner = &resolved_receiver[8..resolved_receiver.len() - 1];
                            if first_param == "T"
                                || first_param == "$0"
                                || first_param.starts_with("$MISSING_GENERIC_")
                            {
                                generic_map
                                    .entry(first_param.clone())
                                    .or_insert(inner.to_string());
                            } else if first_param == "Promise<T>" {
                                generic_map
                                    .entry("T".to_string())
                                    .or_insert(inner.to_string());
                            }
                        }
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
                    } else if (callee_type.starts_with("function:") || callee_type.contains("=>"))
                        && !args.is_empty()
                    {
                        resolved_receiver = self
                            .check_expression(&args[0])
                            .map(|t| t.to_name())
                            .unwrap_or_default();
                    }

                    if resolved_receiver.is_empty() {
                        resolved_receiver = callee_type.clone();
                    }

                    if let Some(s) = self.lookup(&resolved_receiver) {
                        if let Some(alias) = &s.aliased_type {
                            resolved_receiver = alias.to_name();
                        }
                    }

                    let receiver_ty = TejxType::from_name(&resolved_receiver);
                    if receiver_ty.is_array() || receiver_ty.is_slice() {
                        let inner = receiver_ty.get_array_element_type().to_name();
                        if target_type == "T"
                            || target_type.starts_with("$MISSING_GENERIC_")
                            || target_type == "$0"
                        {
                            if let Some(explicit) = generic_map.get("T") {
                                target_type = explicit.clone();
                            } else {
                                target_type = inner.clone();
                            }
                        } else if target_type.ends_with("[]") {
                            let base = &target_type[..target_type.len() - 2];
                            if Self::is_unresolved_generic_name(base) {
                                if let Some(explicit) = generic_map.get(base) {
                                    target_type = format!("{}[]", explicit);
                                } else {
                                    target_type = format!("{}[]", inner);
                                }
                            }
                        }
                    } else if resolved_receiver.starts_with("Promise<") {
                        let inner = &resolved_receiver[8..resolved_receiver.len() - 1];
                        if target_type == "T"
                            || target_type.starts_with("$MISSING_GENERIC_")
                            || target_type == "$0"
                        {
                            if let Some(explicit) = generic_map.get("T") {
                                target_type = explicit.clone();
                            } else {
                                target_type = inner.to_string();
                            }
                        } else if target_type == "Promise<T>" {
                            if let Some(explicit) = generic_map.get("T") {
                                target_type = format!("Promise<{}>", explicit);
                            } else {
                                target_type = format!("Promise<{}>", inner);
                            }
                        } else if target_type == "T[]"
                            || target_type == "$0[]"
                            || target_type.ends_with("[]") && target_type.starts_with("T")
                        {
                            if let Some(explicit) = generic_map.get("T") {
                                target_type = format!("{}[]", explicit);
                            } else {
                                target_type = format!("{}[]", inner);
                            }
                        } else if target_type == "Promise<T[]>" {
                            if let Some(explicit) = generic_map.get("T") {
                                target_type = format!("Promise<{}[]>", explicit);
                            } else {
                                target_type = format!("Promise<{}[]>", inner);
                            }
                        }
                    }

                    let lambda_ctx_params = if matches!(arg, Expression::LambdaExpr { .. }) {
                        if target_type.starts_with("function:") || target_type.contains("=>") {
                            let (ret_sig, params, _) = self.parse_signature(target_type.clone());
                            let ret_ty = ret_sig
                                .splitn(2, ':')
                                .nth(1)
                                .unwrap_or("void")
                                .to_string();
                            let mut func_ty = TejxType::Function(
                                params.iter().map(|p| TejxType::from_name(p)).collect(),
                                Box::new(TejxType::from_name(&ret_ty)),
                            );
                            if !resolved_receiver.is_empty() {
                                let receiver_ty = TejxType::from_name(&resolved_receiver);
                                let mut bindings: std::collections::HashMap<String, TejxType> =
                                    std::collections::HashMap::new();
                                if receiver_ty.is_array() || receiver_ty.is_slice() {
                                    bindings.insert(
                                        "T".to_string(),
                                        receiver_ty.get_array_element_type(),
                                    );
                                }
                                if !bindings.is_empty() {
                                    func_ty = func_ty.substitute_generics(&bindings);
                                }
                            }
                            if let TejxType::Function(parsed_params, _) = func_ty {
                                Some(parsed_params)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let arg_type = self
                        .with_expected_type(Some(TejxType::from_name(&target_type)), |s| {
                            s.with_lambda_context(lambda_ctx_params, |s_inner| {
                                s_inner.check_expression(arg)
                            })
                        })?
                        .to_name();

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

                        Self::collect_generic_bindings_from_types(
                            &expected_obj,
                            &actual_obj,
                            &mut generic_map,
                        );

                        let is_print_call = callee_str == "print" || callee_str == "eprint";
                        if !self.are_types_compatible(&expected_obj, &actual_obj) {
                            if is_print_call && matches!(expected_obj, TejxType::String) {
                                // Allow automatic stringification for print/eprint
                            } else {
                                // Original generic check logic ...
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
                                let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                                let original_param_is_generic =
                                    if let Some(sym) = self.lookup(func_name) {
                                        if adjusted_i < sym.params.len() {
                                            let orig = sym.params[adjusted_i].to_name();
                                            orig.len() <= 2
                                                && orig
                                                    .chars()
                                                    .next()
                                                    .map_or(false, |c| c.is_uppercase())
                                                && orig.chars().all(|c| c.is_alphanumeric())
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    };
                                let expected_has_unresolved =
                                    Self::contains_unresolved_generic_type(&expected_obj);
                                if !is_generic_param(&target_type)
                                    && !is_generic_param(&arg_type)
                                    && !original_param_is_generic
                                    && !expected_has_unresolved
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
                        generic_map.entry(target_type.clone()).or_insert(arg_type.clone());
                    }
                }

                let func_name = callee_str.split('.').last().unwrap_or(&callee_str);
                if explicit_type_args_valid {
                    if !call_generic_params.is_empty() {
                        if let Some(explicit_args) = explicit_type_args.clone() {
                            self.function_instantiations
                                .entry(func_name.to_string())
                                .or_default()
                                .insert(explicit_args);
                        }
                    }
                } else if !explicit_type_args_used && !call_generic_params.is_empty() {
                    let mut concrete_args = Vec::new();
                    let mut missing_inference = None;
                    for gp in &call_generic_params {
                        let inferred_from_receiver = if let Some(existing) =
                            generic_map.get(&gp.name)
                        {
                            Some(existing.clone())
                        } else if let Some(existing) = generic_map
                            .get(&format!("$MISSING_GENERIC_{}", concrete_args.len()))
                        {
                            Some(existing.clone())
                        } else if let Expression::MemberAccessExpr { object, .. } =
                            &**callee
                        {
                            self.check_expression(object).ok().and_then(|receiver_ty| {
                                match gp.name.as_str() {
                                    "T" if receiver_ty.is_array() || receiver_ty.is_slice() => {
                                        Some(receiver_ty.get_array_element_type().to_name())
                                    }
                                    "T" => {
                                        let receiver_name = receiver_ty.to_name();
                                        if receiver_name.starts_with("Promise<")
                                            && receiver_name.ends_with('>')
                                        {
                                            Some(
                                                receiver_name[8..receiver_name.len() - 1]
                                                    .to_string(),
                                            )
                                        } else {
                                            None
                                        }
                                    }
                                    _ => None,
                                }
                            })
                        } else {
                            None
                        };

                        if let Some(concrete) = inferred_from_receiver.as_ref() {
                            generic_map
                                .entry(gp.name.clone())
                                .or_insert_with(|| concrete.clone());
                            if let Some(bound) = &gp.bound {
                                let bound_str = bound.to_string();
                                if !self.is_assignable(
                                    &TejxType::from_name(&bound_str),
                                    &TejxType::from_name(concrete),
                                ) {
                                    self.report_error_detailed(
                                        format!(
                                            "Type '{}' does not satisfy constraint '{}' for generic parameter '{}'",
                                            concrete, bound_str, gp.name
                                        ),
                                        *_line,
                                        *_col,
                                        "E0120",
                                        Some(&format!(
                                            "Provide a type that satisfies the constraint '{}'",
                                            bound_str
                                        )),
                                    );
                                }
                            }
                            concrete_args.push(TejxType::from_name(concrete));
                        } else {
                            missing_inference = Some(gp.name.clone());
                            break;
                        }
                    }
                    if let Some(missing) = missing_inference {
                        self.report_error_detailed(
                            format!(
                                "Cannot infer generic type parameter '{}' for '{}'",
                                missing, callee_str
                            ),
                            *_line,
                            *_col,
                            "E0121",
                            Some("Pass an argument with a concrete type or provide explicit type arguments"),
                        );
                    } else {
                        self.function_instantiations
                            .entry(func_name.to_string())
                            .or_default()
                            .insert(concrete_args);
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

                let mut bindings: HashMap<String, TejxType> = HashMap::new();
                for (k, v) in &generic_map {
                    bindings.insert(k.clone(), parse_type_string(self, v));
                }
                let final_ret_ty = apply_bindings_to_type_str(self, &return_type, &bindings);
                Ok(parse_type_string(self, &final_ret_ty))
            }
            Expression::ObjectLiteralExpr {
                entries, _spreads, ..
            } => {
                let mut props = Vec::new();
                let mut seen_keys = std::collections::HashSet::new();

                for (key, val_expr) in entries {
                    if !seen_keys.insert(key.clone()) {
                        self.report_error_detailed(
                            format!("Duplicate key '{}' in object literal", key),
                            0, // Ideally we have line/col for entries, but entries might not have them individualy
                            0,
                            "E0111",
                            Some("Each key in an object literal must be unique")
                        );
                    }
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
                    if let TejxType::Object(spread_props) = TejxType::from_name(&resolved.to_name())
                    {
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
                let expected_array =
                    self.current_expected_type
                        .as_ref()
                        .and_then(|expected| match expected {
                            TejxType::DynamicArray(inner) => Some(((*inner.clone()), None, false)),
                            TejxType::FixedArray(inner, size) => {
                                Some(((*inner.clone()), Some(*size), true))
                            }
                            TejxType::Slice(inner) => Some(((*inner.clone()), None, false)),
                            _ => None,
                        });

                if !elements.is_empty() {
                    let mut first_type_opt: Option<String> = None;
                    let expected_inner = expected_array.as_ref().map(|(inner, _, _)| inner.clone());
                    let expected_is_any = matches!(expected_inner, Some(TejxType::Any));
                    let has_spread = elements
                        .iter()
                        .any(|element| matches!(element, Expression::SpreadExpr { .. }));

                    for i in 0..elements.len() {
                        let mut lambda_ctx_params: Option<Vec<TejxType>> = None;
                        if let Some(inner) = &expected_inner {
                            if let TejxType::Function(params, _) = inner {
                                lambda_ctx_params = Some(params.clone());
                            }
                        }

                        let mut elem_ty = self
                            .with_expected_type(expected_inner.clone(), |s| {
                                s.with_lambda_context(lambda_ctx_params, |s_inner| {
                                    s_inner.check_expression(&elements[i])
                                })
                            })?
                            .to_name();

                        if let Expression::SpreadExpr { .. } = elements[i] {
                            let spread_ty = TejxType::from_name(&elem_ty);
                            let is_array_like = spread_ty.is_array() || spread_ty.is_slice();
                            if is_array_like {
                                elem_ty = spread_ty.get_array_element_type().to_name();
                            }
                        }

                        if expected_is_any {
                            if first_type_opt.is_none() {
                                first_type_opt = Some("any".to_string());
                            }
                            continue;
                        }

                        if let Some(first_type) = &first_type_opt {
                            let elem_ty_str = elem_ty.clone();
                            if &elem_ty_str != first_type && first_type != "<inferred>" {
                                let common = self
                                    .get_common_ancestor(
                                        &TejxType::from_name(first_type),
                                        &TejxType::from_name(&elem_ty_str),
                                    )
                                    .to_name();
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

                    let element_type = if expected_is_any {
                        "any".to_string()
                    } else {
                        first_type_opt.unwrap_or_else(|| "<inferred>".to_string())
                    };
                    let inferred_ty = if let Some((_, expected_len, is_fixed)) = &expected_array {
                        if *is_fixed {
                            if !has_spread
                                && !elements.is_empty()
                                && elements.len() != *expected_len.as_ref().unwrap()
                            {
                                self.report_error_detailed(
                                    format!(
                                        "Fixed array literal length mismatch: expected {}, got {}",
                                        expected_len.unwrap(),
                                        elements.len()
                                    ),
                                    *_line,
                                    *_col,
                                    "E0100",
                                    Some("Match the declared fixed-array length exactly"),
                                );
                            }
                            format!("{}[{}]", element_type, expected_len.unwrap())
                        } else {
                            format!("{}[]", element_type)
                        }
                    } else if has_spread {
                        format!("{}[]", element_type)
                    } else {
                        // Default array literals to dynamic arrays unless a fixed size is expected.
                        format!("{}[]", element_type)
                    };
                    *ty.borrow_mut() = Some(inferred_ty.clone());
                    Ok(TejxType::from_name(&inferred_ty))
                } else {
                    let inferred_ty = if let Some((inner, expected_len, is_fixed)) = &expected_array
                    {
                        if *is_fixed {
                            format!("{}[{}]", inner.to_name(), expected_len.unwrap())
                        } else {
                            format!("{}[]", inner.to_name())
                        }
                    } else {
                        "[]".to_string()
                    };
                    *ty.borrow_mut() = Some(inferred_ty.clone());
                    Ok(TejxType::from_name(&inferred_ty))
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
                let target_ty = self.check_expression(target)?;
                self.check_expression(index)?;
                let mut unwrapped_type = target_ty.to_name();
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
                let parsed = TejxType::from_name(&unwrapped_type);
                if parsed.is_array() || parsed.is_slice() {
                    return Ok(parsed.get_array_element_type());
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
                    return Ok(TejxType::from_name(
                        &self.substitute_generics(&info.ty.to_name(), &obj_type),
                    ));
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
                type_args,
                args,
                _line,
                _col,
            } => {
                // Try to resolve return type from callee
                let callee_type = self.check_expression(callee)?.to_name();
                for arg in args {
                    self.check_expression(arg)?;
                }

                let parse_type_string = |tc: &TypeChecker, ty_str: &str| -> TejxType {
                    if ty_str.starts_with("function:") || ty_str.contains("=>") {
                        let (ret, params, _) = tc.parse_signature(ty_str.to_string());
                        let parts: Vec<&str> = ret.split(':').collect();
                        let actual_ret = if parts.len() >= 2 { parts[1] } else { &ret };
                        TejxType::Function(
                            params.iter().map(|p| TejxType::from_name(p)).collect(),
                            Box::new(TejxType::from_name(actual_ret)),
                        )
                    } else {
                        TejxType::from_name(ty_str)
                    }
                };

                let mut return_type = if callee_type.starts_with("function:")
                    || callee_type.contains("=>")
                {
                    let (ret, _, _) = self.parse_signature(callee_type.clone());
                    ret
                } else {
                    callee_type.clone()
                };

                if let Some(explicit_args) = type_args.as_ref() {
                    let mut call_generic_params: Vec<crate::ast::GenericParam> = Vec::new();
                    if let Expression::MemberAccessExpr { object, member, .. } = &**callee {
                        if let Ok(obj_type) = self.check_expression(object) {
                            if let Some(info) =
                                self.resolve_instance_member(&obj_type.to_name(), member)
                            {
                                if !info.generic_params.is_empty() {
                                    call_generic_params = info.generic_params.clone();
                                }
                            }
                        }
                    }
                    if call_generic_params.is_empty() {
                        let func_name = callee.to_callee_name();
                        if let Some(s) = self.lookup(&func_name) {
                            call_generic_params = s.generic_params.clone();
                        }
                    }

                    if call_generic_params.is_empty() {
                        self.report_error_detailed(
                            format!(
                                "Type arguments are not allowed for non-generic call '{}'",
                                callee.to_callee_name()
                            ),
                            *_line,
                            *_col,
                            "E0122",
                            Some("Remove the explicit type arguments"),
                        );
                    } else if explicit_args.len() != call_generic_params.len() {
                        self.report_error_detailed(
                            format!(
                                "Generic type argument count mismatch for '{}': expected {}, got {}",
                                callee.to_callee_name(),
                                call_generic_params.len(),
                                explicit_args.len()
                            ),
                            *_line,
                            *_col,
                            "E0122",
                            Some("Provide the correct number of type arguments"),
                        );
                    } else {
                        let mut bindings: HashMap<String, TejxType> = HashMap::new();
                        for (gp, concrete) in
                            call_generic_params.iter().zip(explicit_args.iter())
                        {
                            let concrete_ty = TejxType::from_node(concrete);
                            if !self.is_valid_type(&concrete_ty) {
                                self.report_error_detailed(
                                    format!(
                                        "Unknown data type: '{}' for generic parameter '{}'",
                                        concrete_ty.to_name(),
                                        gp.name
                                    ),
                                    *_line,
                                    *_col,
                                    "E0101",
                                    Some("Provide a valid concrete type"),
                                );
                            }
                            if let Some(bound) = &gp.bound {
                                let bound_ty = TejxType::from_node(bound);
                                if !self.is_assignable(&bound_ty, &concrete_ty) {
                                    self.report_error_detailed(
                                        format!(
                                            "Type '{}' does not satisfy constraint '{}' for generic parameter '{}'",
                                            concrete_ty.to_name(),
                                            bound_ty.to_name(),
                                            gp.name
                                        ),
                                        *_line,
                                        *_col,
                                        "E0120",
                                        Some(&format!(
                                            "Provide a type that satisfies the constraint '{}'",
                                            bound_ty.to_name()
                                        )),
                                    );
                                }
                            }
                            bindings.insert(gp.name.clone(), concrete_ty);
                        }
                        return_type = parse_type_string(self, &return_type)
                            .substitute_generics(&bindings)
                            .to_name();
                    }
                }

                Ok(parse_type_string(self, &return_type))
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
                let mut class_ty = TejxType::from_name(class_name);
                let mut effective_class_name = class_name.clone();
                if let TejxType::Class(base, generics) = class_ty.clone() {
                    if !generics.is_empty() {
                        if let Some(sym) = self.lookup(&base) {
                            if sym.generic_params.is_empty() {
                                self.report_error_detailed(
                                    format!(
                                        "Type arguments are not allowed for non-generic class '{}'",
                                        base
                                    ),
                                    *_line,
                                    *_col,
                                    "E0122",
                                    Some("Remove the explicit type arguments"),
                                );
                                // Ignore type arguments on non-generic classes to avoid cascading errors.
                                class_ty = TejxType::Class(base.clone(), vec![]);
                                effective_class_name = base.clone();
                            }
                        }
                    }
                }
                if !self.is_valid_type(&class_ty) {
                    let base_name = match &class_ty {
                        TejxType::Class(name, _) => name.as_str(),
                        _ => class_name.as_str(),
                    };
                    let is_known = self.lookup(base_name).is_some();
                    if !(is_known && !class_name.contains('<')) {
                        self.report_error_detailed(
                            format!("Unknown class '{}'", class_name),
                            *_line,
                            *_col,
                            "E0101",
                            Some("Ensure the class is defined or imported before use"),
                        );
                    }
                }
                if self.abstract_classes.contains(&effective_class_name) {
                    self.report_error_detailed(format!("Cannot instantiate abstract class '{}'", class_name), *_line, *_col, "E0110", Some("Create a concrete subclass that implements all abstract methods, then instantiate that instead"));
                }

                let mut expected_arg_types = Vec::new();
                if let Some(info) =
                    self.resolve_instance_member(&effective_class_name, "constructor")
                {
                    if let TejxType::Function(params, _) = &info.ty {
                        expected_arg_types = params.clone();
                    } else if let TejxType::Class(sig, _) = &info.ty {
                        if sig.starts_with("function:") || sig.contains("=>") {
                            let (_ret, params, _) = self.parse_signature(sig.clone());
                            expected_arg_types = params
                                .iter()
                                .map(|p| TejxType::from_name(p))
                                .collect();
                        }
                    }
                }

                let mut actual_arg_types = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let expected_ty = if i < expected_arg_types.len() {
                        Some(expected_arg_types[i].clone())
                    } else {
                        None
                    };
                    let actual = self.with_expected_type(expected_ty, |s| s.check_expression(arg))?;
                    actual_arg_types.push(actual);
                }

                let is_optional_param = |ty: &TejxType| -> bool {
                    match ty {
                        TejxType::Class(name, gen) => name == "Option" && gen.len() == 1,
                        TejxType::Union(types) => types.iter().any(|t| t.to_name() == "None"),
                        _ => false,
                    }
                };
                if !expected_arg_types.is_empty() {
                    if args.len() > expected_arg_types.len() {
                        self.report_error_detailed(
                            format!(
                                "Constructor for '{}' expects {} argument(s), but {} were provided",
                                effective_class_name,
                                expected_arg_types.len(),
                                args.len()
                            ),
                            *_line,
                            *_col,
                            "E0109",
                            Some(&format!(
                                "Provide {} argument(s)",
                                expected_arg_types.len()
                            )),
                        );
                    } else if args.len() < expected_arg_types.len() {
                        let missing = &expected_arg_types[args.len()..];
                        if !missing.iter().all(|t| is_optional_param(t)) {
                            self.report_error_detailed(
                                format!(
                                    "Constructor for '{}' expects {} argument(s), but {} were provided",
                                    effective_class_name,
                                    expected_arg_types.len(),
                                    args.len()
                                ),
                                *_line,
                                *_col,
                                "E0109",
                                Some(&format!(
                                    "Provide {} argument(s)",
                                    expected_arg_types.len()
                                )),
                            );
                        }
                    }
                }

                let mut inferred_class_name = effective_class_name.clone();
                let mut inferred = false;

                if !effective_class_name.contains('<') {
                    if let Some(expected) = self.current_expected_type.as_ref() {
                        if let TejxType::Class(exp_name, exp_generics) = expected {
                            if exp_name == &effective_class_name && !exp_generics.is_empty() {
                                let args = exp_generics
                                    .iter()
                                    .map(|t| t.to_name())
                                    .collect::<Vec<_>>();
                                inferred_class_name =
                                    format!("{}<{}>", effective_class_name, args.join(", "));
                                inferred = true;
                            }
                        }
                    }
                }

                if !effective_class_name.contains('<') {
                    if let Some(sym) = self.lookup(&effective_class_name) {
                        if !sym.generic_params.is_empty() && !inferred {
                            if !expected_arg_types.is_empty() {
                                let mut bindings: std::collections::HashMap<String, String> =
                                    std::collections::HashMap::new();
                                for (formal, actual) in expected_arg_types
                                    .iter()
                                    .zip(actual_arg_types.iter())
                                {
                                    Self::collect_generic_bindings_from_types(
                                        formal,
                                        actual,
                                        &mut bindings,
                                    );
                                }
                                let mut concrete_args = Vec::new();
                                let mut all_inferred = true;
                                for gp in &sym.generic_params {
                                    if let Some(concrete) = bindings.get(&gp.name) {
                                        concrete_args.push(concrete.clone());
                                    } else {
                                        all_inferred = false;
                                        break;
                                    }
                                }
                                if !all_inferred
                                    && sym.generic_params.len() == 1
                                    && !actual_arg_types.is_empty()
                                {
                                    let fallback = actual_arg_types
                                        .last()
                                        .map(|t| t.to_name())
                                        .unwrap_or_default();
                                    if !fallback.is_empty() && fallback != "<inferred>" {
                                        concrete_args = vec![fallback];
                                        all_inferred = true;
                                    }
                                }
                                if all_inferred && !concrete_args.is_empty() {
                                    inferred_class_name = format!(
                                        "{}<{}>",
                                        effective_class_name,
                                        concrete_args.join(", ")
                                    );
                                    inferred = true;
                                }
                            }

                            if !inferred {
                                self.report_error_detailed(
                                    format!(
                                        "Generic type '{}' requires explicit type arguments",
                                        effective_class_name
                                    ),
                                    *_line,
                                    *_col,
                                    "E0121",
                                    Some(&format!(
                                        "Use 'new {}<...>(...)' or provide a typed context",
                                        effective_class_name
                                    )),
                                );
                            }
                        }
                    }
                }
                self.register_instantiation(&inferred_class_name, *_line, *_col);
                Ok(TejxType::from_name(&inferred_class_name))
            }
            Expression::ThisExpr { _line, _col } => {
                if let Some(sym) = self.lookup("this") {
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
