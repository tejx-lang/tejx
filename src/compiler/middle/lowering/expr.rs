use super::Lowering;
use crate::frontend::ast::*;
use crate::middle::hir::*;
use crate::frontend::token::TokenType;
use crate::common::types::TejxType;

impl Lowering {
    pub(crate) fn lower_expression(&self, expr: &Expression) -> HIRExpression {
        let line = expr.get_line();
        match expr {
            Expression::ObjectLiteralExpr {
                entries, _spreads, ..
            } => {
                let mut hir_entries = Vec::new();
                let mut object_props = Vec::new();
                for (key, val) in entries {
                    let lowered = self.lower_expression(val);
                    object_props.push((key.clone(), false, lowered.get_type()));
                    hir_entries.push((key.clone(), lowered));
                }

                let mut spread_vals = Vec::new();
                for spread in _spreads {
                    let spread_val = self.lower_expression(spread);
                    if let TejxType::Object(props) = spread_val.get_type() {
                        for (k, o, t) in props {
                            if !object_props.iter().any(|(name, _, _)| name == &k) {
                                object_props.push((k, o, t));
                            }
                        }
                    }
                    spread_vals.push(spread_val);
                }

                let merged_ty = TejxType::Object(object_props);
                let base_obj = HIRExpression::ObjectLiteral {
                    entries: hir_entries,
                    ty: merged_ty.clone(),
                    line,
                };

                if spread_vals.is_empty() {
                    base_obj
                } else {
                    // Handle spreads: merge spreads into base_obj by chaining calls
                    let mut expr = base_obj;
                    for spread_val in spread_vals {
                        expr = HIRExpression::Call {
                            callee: "rt_object_merge".to_string(),
                            args: vec![expr, spread_val],
                            ty: merged_ty.clone(),
                            line,
                        };
                    }
                    expr
                }
            }

            Expression::NumberLiteral {
                value, _is_float, ..
            } => {
                if let Some(expected) = self.current_expected_type.borrow().clone() {
                    if expected.is_numeric() {
                        let val_str = if expected.is_float() {
                            let mut s = value.to_string();
                            if !s.contains('.') && !s.contains('e') {
                                s.push_str(".0");
                            }
                            s
                        } else {
                            format!("{:.0}", value)
                        };
                        return HIRExpression::Literal {
                            line,
                            value: val_str,
                            ty: expected,
                        };
                    }
                }

                let (val_str, ty) = if *_is_float || value.fract() != 0.0 {
                    let mut s = value.to_string();
                    if !s.contains('.') && !s.contains('e') {
                        s.push_str(".0");
                    }
                    (s, TejxType::Float32)
                } else {
                    (format!("{:.0}", value), TejxType::Int32)
                };
                HIRExpression::Literal {
                    line: line,
                    value: val_str,
                    ty,
                }
            }
            Expression::StringLiteral { value, .. } => HIRExpression::Literal {
                line: line,
                value: value.clone(),
                ty: TejxType::String,
            },
            Expression::BooleanLiteral { value, .. } => HIRExpression::Literal {
                line: line,
                value: value.to_string(),
                ty: TejxType::Bool,
            },
            Expression::ThisExpr { .. } => {
                let (name, ty) = self
                    .lookup("this")
                    .unwrap_or_else(|| ("this".to_string(), TejxType::Int64));
                HIRExpression::Variable {
                    line: line,
                    name,
                    ty,
                }
            }
            Expression::SuperExpr { .. } => {
                let (name, ty) = self
                    .lookup("super")
                    .unwrap_or_else(|| ("super".to_string(), TejxType::Int64));
                HIRExpression::Variable {
                    line: line,
                    name,
                    ty,
                }
            }
            Expression::Identifier { name, .. } => {
                let (resolved_name, mut ty) = self
                    .lookup(name)
                    .unwrap_or_else(|| (name.clone(), TejxType::Int64));
                ty = self.resolve_alias_type(&ty);
                let f_name = format!("f_{}", name);
                let final_name = if (self.user_functions.borrow().contains_key(name)
                    || self.user_functions.borrow().contains_key(&f_name))
                    && name != "main"
                    && !resolved_name.starts_with("g_")
                    && !resolved_name.contains("$")
                {
                    if let Some(actual_ty) = self.user_functions.borrow().get(name) {
                        ty = actual_ty.clone();
                    } else if let Some(actual_ty) = self.user_functions.borrow().get(&f_name) {
                        ty = actual_ty.clone();
                    }
                    f_name
                } else {
                    resolved_name
                };
                HIRExpression::Variable {
                    line: line,
                    name: final_name,
                    ty,
                }
            }
            Expression::NullishCoalescingExpr { _left, _right, .. } => {
                let left_hir = self.lower_expression(_left);
                let right_hir = self.lower_expression(_right);
                if !self.is_optional_type(&left_hir.get_type()) {
                    return left_hir;
                }
                let result_ty = self.infer_coalesce_type(&left_hir.get_type(), &right_hir.get_type());

                let cond = self.build_not_none_condition(&left_hir, line);
                HIRExpression::If {
                    line: line,
                    condition: Box::new(cond),
                    then_branch: Box::new(left_hir),
                    else_branch: Box::new(right_hir),
                    ty: result_ty,
                }
            }
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                let target_hir = self.lower_expression(target);
                let index_hir = self.lower_expression(index);
                if !self.is_optional_type(&target_hir.get_type()) {
                    let target_ty = self.non_none_type(&target_hir.get_type());
                    let elem_ty = target_ty.get_array_element_type();
                    return HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(target_hir),
                        index: Box::new(index_hir),
                        ty: elem_ty,
                    };
                }

                let target_ty = self.non_none_type(&target_hir.get_type());
                let elem_ty = target_ty.get_array_element_type();

                let cond = self.build_not_none_condition(&target_hir, line);
                HIRExpression::If {
                    line: line,
                    condition: Box::new(cond),
                    then_branch: Box::new(HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(target_hir),
                        index: Box::new(index_hir),
                        ty: elem_ty.clone(),
                    }),
                    else_branch: Box::new(HIRExpression::NoneLiteral { line }),
                    ty: elem_ty,
                }
            }
            Expression::NoneLiteral { .. } => HIRExpression::NoneLiteral { line },
            Expression::SomeExpr { value, .. } => HIRExpression::SomeExpr {
                value: Box::new(self.lower_expression(value)),
                line,
            },
            Expression::CastExpr { expr, target_type, .. } => HIRExpression::Cast {
                line,
                expr: Box::new(self.lower_expression(expr)),
                ty: TejxType::from_node(target_type),
            },
            Expression::BinaryExpr {
                left, op, right, ..
            } => {
                // Desugar instanceof to runtime call
                if matches!(op, TokenType::Instanceof) {
                    let obj = self.lower_expression(left);
                    // Right side should be a class name identifier
                    let class_name = match right.as_ref() {
                        Expression::Identifier { name, .. } => name.clone(),
                        _ => "__unknown__".to_string(),
                    };
                    return HIRExpression::Call {
                        line: line,
                        callee: "rt_instanceof".to_string(),
                        args: vec![
                            obj,
                            HIRExpression::Literal {
                                line: line,
                                value: class_name,
                                ty: TejxType::String,
                            },
                        ],
                        ty: TejxType::Int32,
                    };
                }
                let l = self.lower_expression(left);
                let r = self.lower_expression(right);

                let bin_ty = self.infer_hir_binary_type(&l, op, &r);
                HIRExpression::BinaryExpr {
                    line: line,
                    left: Box::new(l),
                    op: op.clone(),
                    right: Box::new(r),
                    ty: bin_ty,
                }
            }
            Expression::AssignmentExpr {
                target, value, _op, ..
            } => {
                let v = self.lower_expression(value);
                let ty = v.get_type();

                // Desugar compound assignments: a += b  ->  a = a + b
                let final_value = match _op {
                    TokenType::PlusEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Plus,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::MinusEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Minus,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::StarEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Star,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::SlashEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Slash,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::ModuloEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Modulo,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::AmpersandEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Ampersand,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::PipeEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Pipe,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::CaretEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::Caret,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::LessLessEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::LessLess,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    TokenType::GreaterGreaterEquals => HIRExpression::BinaryExpr {
                        line: line,
                        left: Box::new(self.lower_expression(target)),
                        op: TokenType::GreaterGreater,
                        right: Box::new(v),
                        ty: ty.clone(),
                    },
                    _ => v, // Direct assignment
                };

                if let Expression::MemberAccessExpr { object, member, .. } = target.as_ref() {
                    let obj_ty = self.lower_expression(object).get_type();
                    if let TejxType::Class(ref full_class, _) = obj_ty {
                        let class_name = full_class
                            .split('<')
                            .next()
                            .unwrap_or(full_class)
                            .trim()
                            .to_string();
                        let setters = self.class_setters.borrow();
                        if let Some(s_set) = setters.get(&class_name) {
                            if s_set.contains(member) {
                                return HIRExpression::Call {
                                    line: line,
                                    callee: format!("f_{}_set_{}", class_name, member),
                                    args: vec![self.lower_expression(object), final_value],
                                    ty: TejxType::Void,
                                };
                            }
                        }
                    }
                }

                match target.as_ref() {
                    Expression::Identifier { .. }
                    | Expression::MemberAccessExpr { .. }
                    | Expression::ArrayAccessExpr { .. } => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            line: line,
                            target: Box::new(t),
                            value: Box::new(final_value),
                            ty,
                        }
                    }
                    _ => {
                        let t = self.lower_expression(target);
                        HIRExpression::Assignment {
                            line: line,
                            target: Box::new(t),
                            value: Box::new(final_value),
                            ty,
                        }
                    }
                }
            }
            Expression::UnaryExpr { op, right, .. } => {
                // ++i, --i, !x, -x
                match op {
                    TokenType::PlusPlus | TokenType::MinusMinus => {
                        let r_expr = self.lower_expression(right);
                        let ty = r_expr.get_type();
                        
                        let delta = "1".to_string();
                        let bin_op = if matches!(op, TokenType::PlusPlus) {
                            TokenType::Plus
                        } else {
                            TokenType::Minus
                        };

                        HIRExpression::Assignment {
                            line: line,
                            target: Box::new(r_expr.clone()),
                            value: Box::new(HIRExpression::BinaryExpr {
                                line: line,
                                left: Box::new(r_expr),
                                op: bin_op,
                                right: Box::new(HIRExpression::Literal {
                                    line: line,
                                    value: delta,
                                    ty: TejxType::Int32,
                                }),
                                ty: ty.clone(),
                            }),
                            ty,
                        }
                    }
                    TokenType::Bang => HIRExpression::Call {
                        line: line,
                        callee: "rt_not".to_string(),
                        args: vec![self.lower_expression(right)],
                        ty: TejxType::Bool,
                    },
                    TokenType::Minus => {
                        // -x -> 0 - x
                        let right_hir = self.lower_expression(right);
                        let ty = right_hir.get_type();
                        let zero_val = if ty.is_float() { "0.0" } else { "0" };
                        
                        HIRExpression::BinaryExpr {
                            line: line,
                            left: Box::new(HIRExpression::Literal {
                                line: line,
                                value: zero_val.to_string(),
                                ty: ty.clone(),
                            }),
                            op: TokenType::Minus,
                            right: Box::new(right_hir),
                            ty,
                        }
                    }
                    _ => self.lower_expression(right), // Fallback
                }
            }
            Expression::CallExpr { callee, args, type_args, .. } => {
                let hir_args: Vec<HIRExpression> =
                    args.iter().map(|a| self.lower_expression(a)).collect();
                let callee_str = callee.to_callee_name();
                let normalized = callee_str
                    .replace('.', "_")
                    .replace("::", "_")
                    .replace(":", "_");
                let mut final_callee = normalized.clone();
                let hir_args = hir_args;
                let mut ty = TejxType::Int64;

                if callee_str == "typeof" {
                    if let Some(arg) = hir_args.get(0) {
                        let arg_ty = arg.get_type();
                        if matches!(arg_ty, TejxType::Class(_, _) | TejxType::Any | TejxType::Union(_)) {
                            // Let objects be evaluated at runtime for inheritance paths
                            final_callee = "rt_typeof".to_string();
                            ty = TejxType::String;
                        } else {
                            // Extract known static type string
                            let type_str = match &arg_ty {
                                TejxType::Int32 | TejxType::Int64 | TejxType::Int16 | TejxType::Int128 => "int",
                                TejxType::Float32 | TejxType::Float64 => "float",
                                TejxType::Bool => "bool",
                                TejxType::String => "string",
                                TejxType::Char => "char",
                                TejxType::FixedArray(_, _) | TejxType::DynamicArray(_) | TejxType::Slice(_) => "array",
                                TejxType::Function(_, _) => "function",
                                TejxType::Class(_, _) => "object",
                                TejxType::Object(_) => "object",
                                _ => "void",
                            };
                            return HIRExpression::Literal {
                                line,
                                value: type_str.to_string(),
                                ty: TejxType::String,
                            };
                        }
                    } else {
                        return HIRExpression::Literal {
                            line,
                            value: "undefined".to_string(),
                            ty: TejxType::String,
                        };
                    }
                }

                if callee_str == "sizeof" {
                    let sizeof_const = |ty: &TejxType| -> Option<i64> {
                        match ty {
                            TejxType::Int16 => Some(2),
                            TejxType::Int32 => Some(4),
                            TejxType::Int64 => Some(8),
                            TejxType::Int128 => Some(16),
                            TejxType::Float16 => Some(2),
                            TejxType::Float32 => Some(4),
                            TejxType::Float64 => Some(8),
                            TejxType::Bool => Some(1),
                            TejxType::Char => Some(4),
                            TejxType::String => Some(8),
                            TejxType::Any => Some(8),
                            TejxType::Function(_, _) => Some(16),
                            TejxType::DynamicArray(_) => Some(8),
                            TejxType::Slice(_) => Some(16),
                            TejxType::FixedArray(_, count) => Some((*count as i64) * 8),
                            TejxType::Object(props) => Some((props.len() as i64) * 8),
                            TejxType::Class(_, _) => Some(8),
                            _ => None,
                        }
                    };

                    if let Some(arg) = hir_args.get(0) {
                        if let Some(sz) = sizeof_const(&arg.get_type()) {
                            return HIRExpression::Literal {
                                line,
                                value: sz.to_string(),
                                ty: TejxType::Int32,
                            };
                        }
                    }

                    final_callee = "rt_sizeof".to_string();
                }
                let mut final_args = hir_args.clone();

                // Indirect call check: if name is a variable holding a function
                if !callee_str.is_empty() && !callee_str.contains('.') {
                    if let Some((mangled, var_ty)) = self.lookup(&callee_str) {
                        if !self.user_functions.borrow().contains_key(&callee_str)
                            && !self
                                .user_functions
                                .borrow()
                                .contains_key(&format!("f_{}", callee_str))
                        {
                            let ret_ty = if let TejxType::Function(_, ret) = &var_ty {
                                (**ret).clone()
                            } else {
                                TejxType::Int64
                            };
                            return HIRExpression::IndirectCall {
                                line,
                                callee: Box::new(HIRExpression::Variable {
                                    line,
                                    name: mangled,
                                    ty: var_ty,
                                }),
                                args: hir_args.clone(),
                                ty: ret_ty,
                            };
                        } else {
                            // It's a valid direct call to a user function, so use the mangled name!
                            final_callee = mangled;
                        }
                    }
                }

                // 1. Built-in special functions (Most are now in prelude or intrinsics)
                if callee_str == "super" {
                    if let Some(parent) = &*self.parent_class.borrow() {
                        final_callee = format!("f_{}_constructor", parent);
                        let (mangled_this, _) = self
                            .lookup("this")
                            .unwrap_or_else(|| ("this".to_string(), TejxType::Int64));
                        final_args = vec![HIRExpression::Variable {
                            line,
                            name: mangled_this,
                            ty: TejxType::Int64,
                        }];
                        final_args.extend(hir_args);
                        ty = TejxType::Void;
                    }
                } else if let Expression::MemberAccessExpr { object, member, .. } = callee.as_ref()
                {
                    if let Expression::SuperExpr { .. } = object.as_ref() {
                        if let Some(parent) = &*self.parent_class.borrow() {
                            final_callee = format!("f_{}_{}", parent, member);
                            let (mangled_this, _) = self
                                .lookup("this")
                                .unwrap_or_else(|| ("this".to_string(), TejxType::Int64));
                            final_args = vec![HIRExpression::Variable {
                                line,
                                name: mangled_this,
                                ty: TejxType::Int64,
                            }];
                            final_args.extend(hir_args);

                            if let Some(ret_ty) = self.user_functions.borrow().get(&final_callee) {
                                ty = match ret_ty {
                                    TejxType::Function(_, ret) => (**ret).clone(),
                                    _ => ret_ty.clone(),
                                };
                            }
                        }
                    } else {
                        let mut resolved = false;

                        if !resolved {
                            // Priority 2: Static Methods
                            if let Expression::Identifier { name: obj_name, .. } = object.as_ref() {
                                if obj_name == "Promise" && member == "all" {
                                    final_callee = "f_Promise_all".to_string();
                                    if let Some(ret_ty) =
                                        self.user_functions.borrow().get(&final_callee).cloned()
                                    {
                                        ty = match ret_ty {
                                            TejxType::Function(_, ret) => (*ret).clone(),
                                            _ => ret_ty.clone(),
                                        };
                                    }
                                    resolved = true;
                                }

                                if !resolved && self.class_methods.borrow().contains_key(obj_name) {
                                    let static_candidates = [
                                        format!("f_{}_{}", obj_name, member),
                                        format!(
                                            "f_{}_{}",
                                            self.monomorphized_class_name(&TejxType::from_name(obj_name)),
                                            member
                                        ),
                                    ];

                                    if let Some((static_callee, ret_ty)) = static_candidates
                                        .iter()
                                        .find_map(|candidate| {
                                            self.user_functions
                                                .borrow()
                                                .get(candidate)
                                                .cloned()
                                                .map(|ret_ty| (candidate.clone(), ret_ty))
                                        })
                                    {
                                        final_callee = static_callee;
                                        ty = match ret_ty {
                                            TejxType::Function(_, ret) => (*ret).clone(),
                                            _ => ret_ty.clone(),
                                        };

                                        resolved = true;
                                    }
                                }
                            }
                        }

                        if !resolved {
                            // Priority 3: Instance/Runtime Methods (General Resolution)
                            let mut obj_hir = self.lower_expression(object);
                            let mut obj_ty = obj_hir.get_type();

                            if self.is_optional_type(&obj_ty) {
                                let narrowed = self.non_none_type(&obj_ty);
                                if narrowed != obj_ty {
                                    obj_hir = HIRExpression::Cast {
                                        line,
                                        expr: Box::new(obj_hir),
                                        ty: narrowed.clone(),
                                    };
                                    obj_ty = narrowed;
                                }
                            }

                            if obj_ty == TejxType::String || obj_ty.is_array() || obj_ty.is_slice() {
                                if let Some(builtin_callee) =
                                    self.resolve_builtin_method_callee(&obj_ty, member)
                                {
                                    final_callee = builtin_callee;
                                    if let Some(ret_ty) =
                                        self.user_functions.borrow().get(&final_callee).cloned()
                                    {
                                        ty = match ret_ty {
                                            TejxType::Function(_, ret) => {
                                                self.substitute_generics(&ret, &obj_ty, &final_callee)
                                            }
                                            other => self.substitute_generics(
                                                &other,
                                                &obj_ty,
                                                &final_callee,
                                            ),
                                        };
                                    } else if let Some(builtin_ret_ty) =
                                        self.builtin_method_return_type(&obj_ty, member)
                                    {
                                        ty = builtin_ret_ty;
                                    }
                                } else {
                                    // Fallback if not found in prelude: might be a method that needs class mangling
                                    let class_name = obj_ty.to_name();
                                    final_callee = format!("f_{}_{}", class_name, member);
                                }

                                let mut n_args = vec![obj_hir];
                                n_args.extend(hir_args.clone());
                                final_args = n_args;
                                // resolved = true;
                            } else {
                                if obj_ty == TejxType::Any && member == "length" {
                                    final_callee = "rt_len".to_string();
                                    ty = TejxType::Int32;
                                    let mut n_args = vec![obj_hir.clone()];
                                    n_args.extend(hir_args.clone());
                                    final_args = n_args;
                                    // resolved = true;
                                } else {
                                let template_type_name = match obj_ty {
                                    TejxType::Class(ref c, _) => c.clone(),
                                    _ => format!("{:?}", obj_ty),
                                };
                                let type_name = template_type_name
                                    .split('<')
                                    .next()
                                    .unwrap_or(&template_type_name)
                                    .trim()
                                    .to_string();
                                let concrete_type_name = self.monomorphized_class_name(&obj_hir.get_type());
                                let concrete_method_key =
                                    format!("f_{}_{}", concrete_type_name, member);
                                let method_key = format!("f_{}_{}", type_name, member);

                                if let Some(ret_ty) = self
                                    .user_functions
                                    .borrow()
                                    .get(&concrete_method_key)
                                    .cloned()
                                {
                                    final_callee = concrete_method_key.clone();
                                    // Substitute generic type params from the concrete object type
                                    ty = self.substitute_generics(
                                        &ret_ty,
                                        &obj_hir.get_type(),
                                        &final_callee,
                                    );
                                } else {
                                    let mut resolved_by_mono = false;
                                    if let Some(insts) =
                                        self.generic_instantiations.borrow().get(&type_name)
                                    {
                                        if insts.len() == 1 {
                                            if let Some(args) = insts.iter().next() {
                                                let mono_class =
                                                    self.monomorphized_name(&type_name, args);
                                                let mono_key =
                                                    format!("f_{}_{}", mono_class, member);
                                                if let Some(ret_ty) = self
                                                    .user_functions
                                                    .borrow()
                                                    .get(&mono_key)
                                                    .cloned()
                                                {
                                                    final_callee = mono_key.clone();
                                                    ty = self.substitute_generics(
                                                        &ret_ty,
                                                        &obj_hir.get_type(),
                                                        &final_callee,
                                                    );
                                                    resolved_by_mono = true;
                                                }
                                            }
                                        }
                                    }

                                    if !resolved_by_mono {
                                        if let Some(ret_ty) = self
                                            .user_functions
                                            .borrow()
                                            .get(&method_key)
                                            .cloned()
                                        {
                                            final_callee = method_key.clone();
                                            ty = self.substitute_generics(
                                                &ret_ty,
                                                &obj_hir.get_type(),
                                                &final_callee,
                                            );
                                        } else if self.extern_functions.borrow().contains(&method_key)
                                        {
                                            final_callee = method_key;
                                        } else {
                                            // Walk class hierarchy to find inherited methods
                                            let mut found = false;
                                            let mut parent_class = { self
                                                .class_parents
                                                .borrow()
                                                .get(&type_name)
                                                .cloned() };
                                            while let Some(ref parent) = parent_class {
                                                let parent_method_key =
                                                    format!("f_{}_{}", parent, member);
                                                if let Some(ret_ty) = self
                                                    .user_functions
                                                    .borrow()
                                                    .get(&parent_method_key)
                                                {
                                                    final_callee = parent_method_key;
                                                    ty = self.substitute_generics(
                                                        ret_ty,
                                                        &obj_hir.get_type(),
                                                        &final_callee,
                                                    );
                                                    found = true;
                                                    break;
                                                } else if self
                                                    .extern_functions
                                                    .borrow()
                                                    .contains(&parent_method_key)
                                                {
                                                    final_callee = parent_method_key;
                                                    found = true;
                                                    break;
                                                }
                                                parent_class =
                                                    self.class_parents.borrow().get(parent).cloned();
                                            }
                                            if !found {
                                                // Fallback to dynamic or best-effort mangling
                                                if type_name == "Any" || type_name == "any" {
                                                    if let Expression::Identifier { ref name, .. } =
                                                        object.as_ref()
                                                    {
                                                        let mangled = self
                                                            .lookup(name)
                                                            .map(|(m, _)| m)
                                                            .unwrap_or_else(|| name.clone());
                                                        final_callee = format!("{}.{}", mangled, member);
                                                    } else {
                                                        final_callee = method_key;
                                                    }
                                                } else {
                                                    final_callee = method_key;
                                                }
                                            }
                                        }
                                    }
                                }
                                    let mut n_args = vec![obj_hir];
                                    n_args.extend(hir_args.clone());
                                    final_args = n_args;
                                }
                                // resolved = true;
                            }
                        }
                    }
                } else if let Some(ret_ty) = {
                    let mut found = self.user_functions.borrow().get(&normalized).cloned();
                    if found.is_none() && !normalized.starts_with("f_") {
                        let f_name = format!("f_{}", normalized);
                        if let Some(ty) = self.user_functions.borrow().get(&f_name).cloned() {
                            final_callee = f_name;
                            found = Some(ty);
                        }
                    }
                    found
                } {
                    if final_callee == "main" {
                        final_callee = "tejx_main".to_string();
                    } else if self.extern_functions.borrow().contains(&normalized) {
                        final_callee = normalized.clone();
                    }
                    // if it was found as f_normalized, final_callee is already updated above

                    ty = match ret_ty {
                        TejxType::Function(_, ret) => *ret,
                        _ => ret_ty,
                    };

                    // Also try to substitute generics for top-level calls if they have generic params
                    if !final_args.is_empty() {
                        let first_arg_ty = final_args[0].get_type();
                        ty = self.substitute_generics(&ty, &first_arg_ty, &final_callee);
                    }
                } else if self.class_methods.borrow().contains_key(&normalized) {
                    let f_cons = format!("f_{}_constructor", normalized);
                    let cons = format!("{}_constructor", normalized);
                    if let Some(_ret_ty) = self.user_functions.borrow().get(&f_cons) {
                        final_callee = f_cons;
                        ty = TejxType::Class(normalized.clone(), vec![]);
                    } else if let Some(_ret_ty) = self.user_functions.borrow().get(&cons) {
                        final_callee = cons;
                        ty = TejxType::Class(normalized.clone(), vec![]);
                    } else {
                        final_callee = f_cons;
                        ty = TejxType::Class(normalized.clone(), vec![]);
                    }
                }

                if let Some((monomorphized_callee, bindings)) =
                    self.resolve_function_monomorph(&final_callee, &final_args, type_args.as_ref())
                {
                    if let Some(base_ty) = self.user_functions.borrow().get(&final_callee).cloned() {
                        ty = match base_ty {
                            TejxType::Function(_, ret) => (*ret).substitute_generics(&bindings),
                            other => other.substitute_generics(&bindings),
                        };
                    }
                    final_callee = monomorphized_callee;
                }

                // CHECK FOR VARIADIC PACKING (Unmangled name or Mangled name)
                let lookup_name = if final_callee.starts_with("f_") {
                    &final_callee[2..]
                } else {
                    &final_callee
                };

                if let Some(&fixed_count) = self.variadic_functions.borrow().get(lookup_name) {
                    if final_args.len() >= fixed_count {
                        let (fixed, rest) = final_args.split_at(fixed_count);
                        let mut new_var_args = fixed.to_vec();

                        new_var_args.push(HIRExpression::ArrayLiteral {
                            line: line,
                            elements: rest.to_vec(),
                            ty: TejxType::DynamicArray(Box::new(TejxType::Any)),
                            sized_allocation: None,
                        });
                        final_args = new_var_args;
                    }
                } else {
                    // Non-variadic: pad missing arguments with None for Optionals/T|None
                    let expected_count_opt =
                        self.user_function_args.borrow().get(&final_callee).copied();
                    if let Some(expected_count) = expected_count_opt {
                        while final_args.len() < expected_count {
                            final_args.push(HIRExpression::NoneLiteral { line });
                        }
                    }
                }

                HIRExpression::Call {
                    line: line,
                    callee: final_callee,
                    args: final_args,
                    ty,
                }
            }
            Expression::MemberAccessExpr { object, member, .. } => {
                let obj_name = match object.as_ref() {
                    Expression::Identifier { name, .. } => name.clone(),
                    _ => "".to_string(),
                };
                let lowered_object = self.lower_expression(object);
                self.lower_member_access_with_obj(line, &obj_name, lowered_object, member)
            }
            Expression::ArrayAccessExpr { target, index, .. } => {
                let lowered_target = self.lower_expression(target);
                let target_ty = lowered_target.get_type();

                let ty = target_ty.get_array_element_type();
                HIRExpression::IndexAccess {
                    line: line,
                    target: Box::new(lowered_target),
                    index: Box::new(self.lower_expression(index)),
                    ty,
                }
            }
            Expression::ArrayLiteral { elements, ty, .. } => {
                let inferred_ty = match &*ty.borrow() {
                    Some(t) => TejxType::from_name(t),
                    None => TejxType::DynamicArray(Box::new(TejxType::Int64)),
                };
                let sized_allocation = match &inferred_ty {
                    TejxType::FixedArray(_, size) => Some(Box::new(HIRExpression::Literal {
                        line,
                        value: size.to_string(),
                        ty: TejxType::Int64,
                    })),
                    _ => None,
                };
                // Handle spreads: [a, ...b, c] -> concat(concat([a], b), [c])
                let mut chunks: Vec<HIRExpression> = Vec::new();
                let mut current_chunk: Vec<HIRExpression> = Vec::new();
                let mut has_spread = false;

                for e in elements {
                    if let Expression::SpreadExpr { _expr, .. } = e {
                        has_spread = true;
                        // Push accumulated static chunk if any
                        if !current_chunk.is_empty() {
                            chunks.push(HIRExpression::ArrayLiteral {
                                line: line,
                                elements: current_chunk.clone(),
                                ty: inferred_ty.clone(),
                                sized_allocation: sized_allocation.clone(),
                            });
                            current_chunk.clear();
                        }
                        // Push spread expr (lowered)
                        chunks.push(self.lower_expression(_expr));
                    } else {
                        current_chunk.push(self.lower_expression(e));
                    }
                }
                // Push final chunk
                if !current_chunk.is_empty() {
                    chunks.push(HIRExpression::ArrayLiteral {
                        line: line,
                        elements: current_chunk,
                        ty: inferred_ty.clone(),
                        sized_allocation: sized_allocation.clone(),
                    });
                }

                if chunks.is_empty() {
                    // Empty array []
                    HIRExpression::ArrayLiteral {
                        line: line,
                        elements: vec![],
                        sized_allocation,
                        ty: inferred_ty.clone(),
                    }
                } else {
                    // If it's only a spread, force a copy so the source array isn't reused.
                    if has_spread && chunks.len() == 1 {
                        let empty = HIRExpression::ArrayLiteral {
                            line: line,
                            elements: vec![],
                            sized_allocation: None,
                            ty: inferred_ty.clone(),
                        };
                        return HIRExpression::Call {
                            line: line,
                            callee: "rt_array_concat".to_string(),
                            args: vec![empty, chunks[0].clone()],
                            ty: inferred_ty.clone(),
                        };
                    }
                    // Reduce chunks with Array_concat
                    let mut expr = chunks[0].clone();
                    for next_chunk in chunks.into_iter().skip(1) {
                        expr = HIRExpression::Call {
                            line: line,
                            callee: "rt_array_concat".to_string(),
                            args: vec![expr, next_chunk],
                            ty: inferred_ty.clone(),
                        };
                    }
                    expr
                }
            }
            Expression::SequenceExpr { expressions, .. } => {
                let mut lower_exprs = Vec::new();
                for e in expressions {
                    lower_exprs.push(self.lower_expression(e));
                }
                let ty = lower_exprs
                    .last()
                    .map(|e| e.get_type())
                    .unwrap_or(TejxType::Void);
                HIRExpression::Sequence {
                    expressions: lower_exprs,
                    ty,
                    line,
                }
            }
            Expression::LambdaExpr {
                params,
                body,
                _line,
                _col,
            } => {
                let id = {
                    let mut counter = self.lambda_counter.borrow_mut();
                    let val = *counter;
                    *counter += 1;
                    val
                };
                let lambda_name = format!("lambda_{}", id);
                self.register_lambda_env_owner(&lambda_name);

                // Use inferred types from TypeChecker if available
                let inferred = self.lambda_inferred_types.get(&(*_line, *_col));

                let hir_params: Vec<(String, TejxType)> = params
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        (
                            p.name.clone(),
                            if let Some(inf) = inferred {
                                if i < inf.len() {
                                    inf[i].clone()
                                } else if p.type_name.to_string().is_empty() {
                                    TejxType::Int64
                                } else {
                                    TejxType::from_node(&p.type_name)
                                }
                            } else if p.type_name.to_string().is_empty() {
                                TejxType::Int64
                            } else {
                                TejxType::from_node(&p.type_name)
                            },
                        )
                    })
                    .collect();

                self.enter_lambda_scope();
                let mut mangled_params: Vec<(String, TejxType)> = hir_params
                    .iter()
                    .map(|(name, ty)| (self.define(name.clone(), ty.clone()), ty.clone()))
                    .collect();

                // Pad up to 4 user arguments so all lambdas have consistent signatures for array methods
                while mangled_params.len() < 4 {
                    mangled_params.push((
                        self.define(
                            format!("__dummy_pad_{}", mangled_params.len()),
                            TejxType::Int64,
                        ),
                        TejxType::Int64,
                    ));
                }

                // Add implicit environment parameter - all lambdas called from JS-like env need this
                mangled_params.insert(
                    0,
                    (
                        self.define("__env".to_string(), TejxType::Int64),
                        TejxType::Int64,
                    ),
                );

                let hir_body = self.lower_statement(body).unwrap_or(HIRStatement::Block {
                    line: line,
                    statements: vec![],
                });

                self._exit_scope();

                self.lambda_functions
                    .borrow_mut()
                    .push(HIRStatement::Function {
                        async_params: None,
                        line: line,
                        name: lambda_name.clone(),
                        params: mangled_params,
                        // Lambdas return through the Any/i64 ABI so closures are uniform.
                        _return_type: TejxType::Any,
                        body: Box::new(hir_body),
                        is_extern: false,
                    });

                let fn_ty = TejxType::Function(
                    hir_params.iter().map(|(_, ty)| ty.clone()).collect(),
                    Box::new(
                        self.lambda_inferred_returns
                            .get(&(*_line, *_col))
                            .cloned()
                            .unwrap_or(TejxType::Void),
                    ),
                );
                HIRExpression::Literal {
                    line: line,
                    value: lambda_name,
                    ty: fn_ty, // Track as function type for sizeof/type-aware ops
                }
            }
            Expression::AwaitExpr { expr, .. } => {
                let lowered = self.lower_expression(expr);
                let awaited_ty = match lowered.get_type() {
                    TejxType::Class(name, generics) if name == "Promise" && !generics.is_empty() => {
                        generics[0].clone()
                    }
                    TejxType::Class(name, _) if name.starts_with("Promise<") && name.ends_with('>') => {
                        TejxType::from_name(&name[8..name.len() - 1])
                    }
                    other => other,
                };
                HIRExpression::Await {
                    line,
                    expr: Box::new(lowered),
                    ty: awaited_ty,
                }
            }
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                let obj_name = match object.as_ref() {
                    Expression::Identifier { name, .. } => name.clone(),
                    _ => "".to_string(),
                };
                let lowered_object = self.lower_expression(object);
                let access_hir =
                    self.lower_member_access_with_obj(line, &obj_name, lowered_object.clone(), member);
                let result_ty = access_hir.get_type();
                if !self.is_optional_type(&lowered_object.get_type()) {
                    return access_hir;
                }
                let cond = self.build_not_none_condition(&lowered_object, line);

                HIRExpression::If {
                    line: line,
                    condition: Box::new(cond),
                    then_branch: Box::new(access_hir),
                    else_branch: Box::new(HIRExpression::NoneLiteral { line }),
                    ty: result_ty,
                }
            }
            Expression::NewExpr {
                class_name, args, ..
            } => {
                let mut hir_args: Vec<HIRExpression> =
                    args.iter().map(|a| self.lower_expression(a)).collect();

                let mut class_ty = TejxType::from_name(class_name);
                let (base_name_opt, base_generics) = match &class_ty {
                    TejxType::Class(name, generics) => (Some(name.clone()), generics.clone()),
                    _ => (None, Vec::new()),
                };

                if let Some(base_name) = base_name_opt {
                    if !base_generics.is_empty() {
                        let is_generic = self
                            .class_generic_params
                            .borrow()
                            .get(&base_name)
                            .map(|g| !g.is_empty())
                            .unwrap_or(false);
                        if !is_generic {
                            class_ty = TejxType::Class(base_name.clone(), vec![]);
                        }
                    }
                    if base_generics.is_empty() {
                        if let Some(expected) = self
                            .current_expected_type
                            .borrow()
                            .clone()
                            .map(|t| self.resolve_alias_type(&t))
                        {
                            if let TejxType::Class(exp_name, exp_generics) = expected {
                                if exp_name == base_name && !exp_generics.is_empty() {
                                    class_ty = TejxType::Class(base_name.clone(), exp_generics);
                                }
                            }
                        }

                        if let Some(generic_params) =
                            self.class_generic_params.borrow().get(&base_name).cloned()
                        {
                            if !generic_params.is_empty() {
                                let ctor_name = format!("f_{}_constructor", base_name);
                                if let Some(TejxType::Function(param_tys, _)) =
                                    self.user_functions.borrow().get(&ctor_name).cloned()
                                {
                                    let generic_set: std::collections::HashSet<String> =
                                        generic_params.iter().cloned().collect();
                                    let mut bindings: std::collections::HashMap<String, TejxType> =
                                        std::collections::HashMap::new();
                                    for (formal, actual) in
                                        param_tys.iter().zip(hir_args.iter().map(|a| a.get_type()))
                                    {
                                        self.collect_generic_bindings(
                                            formal,
                                            &actual,
                                            &generic_set,
                                            &mut bindings,
                                        );
                                    }
                                    let mut concrete_args = Vec::new();
                                    let mut all_inferred = true;
                                    for gp in &generic_params {
                                        if let Some(arg) = bindings.get(gp) {
                                            concrete_args.push(arg.clone());
                                        } else {
                                            all_inferred = false;
                                            break;
                                        }
                                    }
                                    if all_inferred
                                        && !concrete_args.is_empty()
                                        && concrete_args
                                            .iter()
                                            .all(|t| self.is_concrete_type(t))
                                    {
                                        class_ty =
                                            TejxType::Class(base_name.clone(), concrete_args);
                                    }
                                }
                            }
                        }
                        if let TejxType::Class(_, ref generics) = class_ty {
                            if generics.is_empty() {
                                if let Some(insts) =
                                    self.generic_instantiations.borrow().get(&base_name)
                                {
                                    if insts.len() == 1 {
                                        if let Some(args) = insts.iter().next() {
                                            class_ty =
                                                TejxType::Class(base_name.clone(), args.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let normalized_class = self.monomorphized_class_name(&class_ty);

                // Variadic check for constructor
                let cons_unmangled = format!("{}_constructor", normalized_class);
                if let Some(&fixed_count) = self.variadic_functions.borrow().get(&cons_unmangled) {
                    if hir_args.len() >= fixed_count {
                        let (fixed, rest) = hir_args.split_at(fixed_count);
                        let mut new_var_args = fixed.to_vec();
                        new_var_args.push(HIRExpression::ArrayLiteral {
                            line: line,
                            elements: rest.to_vec(),
                            sized_allocation: None,
                            ty: TejxType::DynamicArray(Box::new(TejxType::Any)),
                        });
                        hir_args = new_var_args;
                    }
                }

                if let TejxType::Class(base, generics) = &class_ty {
                    if !generics.is_empty()
                        && generics.iter().all(|t| self.is_concrete_type(t))
                    {
                        self.generic_instantiations
                            .borrow_mut()
                            .entry(base.clone())
                            .or_default()
                            .insert(generics.clone());
                    }
                }

                HIRExpression::NewExpr {
                    line: line,
                    class_name: normalized_class,
                    _args: hir_args,
                    ty: class_ty,
                }
            }
            Expression::OptionalCallExpr {
                callee,
                args: _args,
                ..
            } => {
                let callee_expr = self.lower_expression(callee);

                HIRExpression::OptionalChain {
                    line: line,
                    target: Box::new(callee_expr),
                    operation: "()".to_string(), // In HIR/MIR, OptionalChain "()" means call
                    ty: TejxType::Int64,
                }
            }
            Expression::TernaryExpr {
                _condition,
                _true_branch,
                _false_branch,
                ..
            } => {
                let cond = self.lower_expression(_condition);
                let t_branch = self.lower_expression(_true_branch);
                let f_branch = self.lower_expression(_false_branch);
                let ty = self.infer_ternary_type(&t_branch.get_type(), &f_branch.get_type());
                HIRExpression::If {
                    line: line,
                    condition: Box::new(cond),
                    then_branch: Box::new(t_branch),
                    else_branch: Box::new(f_branch),
                    ty,
                }
            }

            _ => HIRExpression::Literal {
                line: line,
                value: "0".to_string(),
                ty: TejxType::Int64,
            },
        }
    }

    fn non_none_type(&self, ty: &TejxType) -> TejxType {
        match ty {
            TejxType::Union(types) => types
                .iter()
                .find(|t| t.to_name() != "None")
                .cloned()
                .unwrap_or_else(|| ty.clone()),
            TejxType::Class(name, generics) if name == "Option" && !generics.is_empty() => {
                generics[0].clone()
            }
            _ => ty.clone(),
        }
    }

    fn is_optional_type(&self, ty: &TejxType) -> bool {
        match ty {
            TejxType::Union(types) => types.iter().any(|t| t.to_name() == "None"),
            TejxType::Class(name, _) if name == "Option" || name == "None" => true,
            _ => false,
        }
    }

    fn infer_coalesce_type(&self, left: &TejxType, right: &TejxType) -> TejxType {
        if left.to_name() == "<inferred>" {
            return right.clone();
        }
        match left {
            TejxType::Union(_) => self.non_none_type(left),
            TejxType::Class(name, _) if name == "Option" => self.non_none_type(left),
            TejxType::Class(name, _) if name == "None" => right.clone(),
            _ => left.clone(),
        }
    }

    fn infer_ternary_type(&self, t_ty: &TejxType, f_ty: &TejxType) -> TejxType {
        if t_ty == f_ty {
            t_ty.clone()
        } else if t_ty.to_name() != "<inferred>" {
            t_ty.clone()
        } else {
            f_ty.clone()
        }
    }

    fn build_not_none_condition(&self, expr: &HIRExpression, line: usize) -> HIRExpression {
        HIRExpression::BinaryExpr {
            line,
            op: TokenType::BangEqual,
            left: Box::new(HIRExpression::Cast {
                line,
                expr: Box::new(expr.clone()),
                ty: TejxType::Int64,
            }),
            right: Box::new(HIRExpression::Literal {
                line,
                value: "0".to_string(),
                ty: TejxType::Int64,
            }),
            ty: TejxType::Bool,
        }
    }

    fn lower_member_access_with_obj(
        &self,
        line: usize,
        obj_name: &str,
        lowered_object: HIRExpression,
        member: &str,
    ) -> HIRExpression {
        // Static Field Resolution
        if !obj_name.is_empty() {
            let s_fields = self.class_static_fields.borrow();
            if let Some(f_list) = s_fields.get(obj_name) {
                if let Some((_name, f_ty, _)) = f_list.iter().find(|(n, _, _)| n == member) {
                    return HIRExpression::Variable {
                        line,
                        name: format!("g_{}_{}", obj_name, member),
                        ty: f_ty.clone(),
                    };
                }
            }
        }

        let mut obj_expr = lowered_object;
        let mut obj_ty = self.resolve_alias_type(&obj_expr.get_type());
        if let TejxType::Union(types) = &obj_ty {
            if let Some(non_none) = types.iter().find(|t| t.to_name() != "None") {
                obj_ty = non_none.clone();
                obj_expr = HIRExpression::Cast {
                    line,
                    expr: Box::new(obj_expr),
                    ty: obj_ty.clone(),
                };
            }
        }

        if let TejxType::Class(ref full_class, _) = obj_ty {
            let class_name = full_class
                .split('<')
                .next()
                .unwrap_or(full_class)
                .trim()
                .to_string();
            let getters = self.class_getters.borrow();
            if let Some(g_set) = getters.get(&class_name) {
                if g_set.contains(member) {
                    return HIRExpression::Call {
                        line,
                        callee: format!("f_{}_get_{}", class_name, member),
                        args: vec![obj_expr.clone()],
                        ty: TejxType::Int64,
                    };
                }
            }
        }

        if let TejxType::Class(ref full_class, _) = obj_ty {
            let class_name = full_class
                .split('<')
                .next()
                .unwrap_or(full_class)
                .trim()
                .to_string();
            let fields = self.class_instance_fields.borrow();
            if let Some(i_list) = fields.get(&class_name) {
                for (f_name, f_ty, _) in i_list {
                    if f_name == member {
                        return HIRExpression::MemberAccess {
                            line,
                            target: Box::new(obj_expr),
                            member: member.to_string(),
                            ty: f_ty.clone(),
                        };
                    }
                }
            }
        }

        if let TejxType::Object(props) = &obj_ty {
            if let Some((_, _, prop_ty)) = props.iter().find(|(name, _, _)| name == member) {
                return HIRExpression::MemberAccess {
                    line,
                    target: Box::new(obj_expr),
                    member: member.to_string(),
                    ty: prop_ty.clone(),
                };
            }
        }

        let combined = format!("{}_{}", obj_name, member);
        let f_combined = format!("f_{}", combined);
        let fallback_ty = if matches!(obj_ty, TejxType::Any) {
            TejxType::Any
        } else {
            TejxType::Int64
        };
        if self.user_functions.borrow().contains_key(&combined)
            || self.user_functions.borrow().contains_key(&f_combined)
        {
            HIRExpression::Variable {
                line,
                name: f_combined,
                ty: TejxType::Int64,
            }
        } else {
            HIRExpression::MemberAccess {
                line,
                target: Box::new(obj_expr),
                member: member.to_string(),
                ty: fallback_ty,
            }
        }
    }

    pub(crate) fn infer_hir_binary_type(
        &self,
        left: &HIRExpression,
        op: &TokenType,
        right: &HIRExpression,
    ) -> TejxType {
        let lt = left.get_type();
        let rt = right.get_type();

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
        ) {
            return TejxType::Bool;
        }

        if lt == TejxType::String || rt == TejxType::String {
            return TejxType::String;
        }

        let is_float = |t: &TejxType| -> bool {
            matches!(t, TejxType::Float16 | TejxType::Float32 | TejxType::Float64)
        };

        if lt == TejxType::Float64 || rt == TejxType::Float64 {
            return TejxType::Float64;
        }
        if is_float(&lt) || is_float(&rt) {
            return TejxType::Float32; // Default promotion
        }

        if lt == TejxType::Int64 || rt == TejxType::Int64 {
            return TejxType::Int64;
        }
        if matches!(lt, TejxType::Int16 | TejxType::Int32 | TejxType::Int128)
            || matches!(rt, TejxType::Int16 | TejxType::Int32 | TejxType::Int128)
        {
            return TejxType::Int32; // Default promotion
        }

        TejxType::Int64
    }
}
