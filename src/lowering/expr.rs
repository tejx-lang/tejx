use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::token::TokenType;
use crate::types::TejxType;

impl Lowering {
    pub(crate) fn lower_expression(&self, expr: &Expression) -> HIRExpression {
        let line = expr.get_line();
        match expr {
            Expression::ObjectLiteralExpr {
                entries, _spreads, ..
            } => {
                let mut hir_entries = Vec::new();
                for (key, val) in entries {
                    hir_entries.push((key.clone(), self.lower_expression(val)));
                }

                let base_obj = HIRExpression::ObjectLiteral {
                    entries: hir_entries,
                    ty: TejxType::Int64, // Map
                    line,
                };

                if _spreads.is_empty() {
                    base_obj
                } else {
                    // Handle spreads: merge spreads into base_obj by chaining calls
                    let mut expr = base_obj;
                    for spread in _spreads {
                        let spread_val = self.lower_expression(spread);
                        expr = HIRExpression::Call {
                            callee: "rt_object_merge".to_string(),
                            args: vec![expr, spread_val],
                            ty: TejxType::Int64,
                            line,
                        };
                    }
                    expr
                }
            }

            Expression::NumberLiteral {
                value, _is_float, ..
            } => {
                let (val_str, ty) = if *_is_float {
                    let mut s = value.to_string();
                    if !s.contains('.') && !s.contains('e') {
                        s.push_str(".0");
                    }
                    (s, TejxType::Float64)
                } else if value.fract() != 0.0 {
                    (value.to_string(), TejxType::Float64)
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
                // Desugar to: let temp = left; if temp != 0 { temp } else { right }
                // Since we don't have block expressions easily here without generating a function or specialized HIR,
                // we'll implement it as a conditional (Ternary) if possible, or BinaryOp if we treat it as ||
                // For now, treat as || (PipePipe) as we lack separate None value from 0
                let left_hir = self.lower_expression(_left);
                let right_hir = self.lower_expression(_right);

                HIRExpression::BinaryExpr {
                    line: line,
                    op: TokenType::PipePipe,
                    left: Box::new(left_hir),
                    right: Box::new(right_hir),
                    ty: TejxType::Int64,
                }
            }
            Expression::OptionalArrayAccessExpr { target, index, .. } => {
                let target_hir = self.lower_expression(target);
                let index_hir = self.lower_expression(index);

                HIRExpression::If {
                    line: line,
                    condition: Box::new(HIRExpression::BinaryExpr {
                        line: line,
                        op: TokenType::BangEqual,
                        left: Box::new(target_hir.clone()),
                        right: Box::new(HIRExpression::Literal {
                            line: line,
                            value: "0".to_string(),
                            ty: TejxType::Int32,
                        }),
                        ty: TejxType::Bool,
                    }),
                    then_branch: Box::new(HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(target_hir),
                        index: Box::new(index_hir),
                        ty: TejxType::Int64,
                    }),
                    else_branch: Box::new(HIRExpression::Literal {
                        line: line,
                        value: "0".to_string(),
                        ty: TejxType::Int32,
                    }),
                    ty: TejxType::Int64,
                }
            }
            Expression::NoneLiteral { .. } => HIRExpression::NoneLiteral { line },
            Expression::SomeExpr { value, .. } => HIRExpression::SomeExpr {
                value: Box::new(self.lower_expression(value)),
                line,
            },
            Expression::CastExpr { expr, .. } => self.lower_expression(expr),
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

                // Desugar === and !== to runtime calls
                if matches!(op, TokenType::EqualEqualEqual)
                    || matches!(op, TokenType::BangEqualEqual)
                {
                    let callee = if matches!(op, TokenType::EqualEqualEqual) {
                        "rt_strict_equal"
                    } else {
                        "rt_strict_ne"
                    };
                    return HIRExpression::Call {
                        line: line,
                        callee: callee.to_string(),
                        args: vec![l, r],
                        ty: TejxType::Bool,
                    };
                }

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
                        // Desugar ++i -> i = i + 1 (Prefix)
                        // TODO: Suffix support? AST usually distinguishes suffix/prefix.
                        // For now assume prefix or handle generic increment.
                        let delta = if matches!(op, TokenType::PlusPlus) {
                            "1"
                        } else {
                            "1"
                        };
                        let bin_op = if matches!(op, TokenType::PlusPlus) {
                            TokenType::Plus
                        } else {
                            TokenType::Minus
                        };

                        let r_expr = self.lower_expression(right);
                        // Reconstruct Assignment: right = right op 1
                        // Need to clone target handling from AssignmentExpr logic ideally.
                        // Simplification:
                        HIRExpression::Assignment {
                            line: line,
                            target: Box::new(r_expr.clone()),
                            value: Box::new(HIRExpression::BinaryExpr {
                                line: line,
                                left: Box::new(r_expr),
                                op: bin_op,
                                right: Box::new(HIRExpression::Literal {
                                    line: line,
                                    value: delta.to_string(),
                                    ty: TejxType::Int32,
                                }),
                                ty: TejxType::Int32,
                            }),
                            ty: TejxType::Int32,
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
                        HIRExpression::BinaryExpr {
                            line: line,
                            left: Box::new(HIRExpression::Literal {
                                line: line,
                                value: "0".to_string(),
                                ty: TejxType::Int32,
                            }),
                            op: TokenType::Minus,
                            right: Box::new(self.lower_expression(right)),
                            ty: TejxType::Int32,
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

                let mut generic_suffix = String::new();
                if let Some(targs) = type_args {
                    for ta in targs {
                        generic_suffix.push('_');
                        let safe_arg = TejxType::from_node(ta).to_name()
                                .replace("[]", "_arr")
                                .replace("<", "_")
                                .replace(">", "_")
                                .replace(", ", "_");
                        generic_suffix.push_str(&safe_arg);
                    }
                }

                if callee_str == "typeof" {
                    if let Some(arg) = hir_args.get(0) {
                        let arg_ty = arg.get_type();
                        if let TejxType::Class(_name, _) = &arg_ty {
                            // Let objects be evaluated at runtime for inheritance paths
                            final_callee = "rt_typeof".to_string();
                            let _ty = TejxType::String;
                        } else {
                            // Extract known static type string
                            let type_str = match &arg_ty {
                                TejxType::Int32 => "int",
                                TejxType::Float64 => "float",
                                TejxType::Bool => "boolean",
                                TejxType::String => "string",
                                TejxType::Char => "char",
                                TejxType::Class(_, _) => "object",
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
                let mut final_args = hir_args.clone();
                let mut ty = TejxType::Int64;

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
                            final_args.extend(final_args.clone()); // Still wrong.

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
                                if self.class_methods.borrow().contains_key(obj_name) {
                                    let static_callee = format!("f_{}_{}", obj_name, member);
                                    if let Some(ret_ty) =
                                        self.user_functions.borrow().get(&static_callee)
                                    {
                                        final_callee = static_callee;
                                        ty = match ret_ty {
                                            TejxType::Function(_, ret) => (**ret).clone(),
                                            _ => ret_ty.clone(),
                                        };

                                        resolved = true;
                                    }
                                }
                            }
                        }

                        if !resolved {
                            // Priority 3: Instance/Runtime Methods (General Resolution)
                            let obj_hir = self.lower_expression(object);
                            let obj_ty = obj_hir.get_type();

                            if obj_ty == TejxType::String || obj_ty.is_array() || obj_ty.is_slice()
                            {
                                // UFCS for built-in types: arr.push(v) -> rt_array_push(arr, v) or f_push(arr, v)
                                let rt_name = match member.as_str() {
                                    "push" => Some("rt_array_push"),
                                    "pop" => Some("rt_array_pop"),
                                    "shift" => Some("rt_array_shift"),
                                    "unshift" => Some("rt_array_unshift"),
                                    "includes" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_includes")
                                        } else {
                                            Some("f_includes") // Use generic prelude version
                                        }
                                    }
                                    "startsWith" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_startsWith")
                                        } else {
                                            None
                                        }
                                    }
                                    "endsWith" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_endsWith")
                                        } else {
                                            None
                                        }
                                    }
                                    "indexOf" => {
                                        if obj_ty == TejxType::String {
                                            Some("rt_String_indexOf")
                                        } else {
                                            Some("rt_array_indexOf")
                                        }
                                    }
                                    "concat" => Some("rt_array_concat"),
                                    "join" => Some("rt_array_join"),
                                    "slice" => Some("rt_array_slice"),
                                    "reverse" => Some("rt_array_reverse"),
                                    "fill" => Some("rt_array_fill"),
                                    "sort" => Some("rt_array_sort"),
                                    "map" => Some("f_map"),
                                    "filter" => Some("f_filter"),
                                    "forEach" => Some("f_forEach"),
                                    "reduce" => Some("f_reduce"),
                                    "every" => Some("f_every"),
                                    "some" => Some("f_some"),
                                    "find" => Some("f_find"),
                                    "findIndex" => Some("f_findIndex"),
                                    // String specific
                                    "toUpperCase" => Some("rt_String_toUpperCase"),
                                    "toLowerCase" => Some("rt_String_toLowerCase"),
                                    "trim" => Some("rt_String_trim"),
                                    "trimStart" => Some("rt_String_trimStart"),
                                    "trimEnd" => Some("rt_String_trimEnd"),
                                    "substring" => Some("rt_String_substring"),
                                    "split" => Some("rt_String_split"),
                                    "repeat" => Some("rt_String_repeat"),
                                    "replace" => Some("rt_String_replace"),
                                    _ => None,
                                };

                                final_callee = if let Some(n) = rt_name {
                                    n.to_string()
                                } else {
                                    format!("f_{}", member)
                                };

                                // Resolve return type for the UFCS call
                                if let Some(ret_ty) =
                                    self.user_functions.borrow().get(&final_callee)
                                {
                                    ty = self.substitute_generics(ret_ty, &obj_ty, &final_callee);
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
                                let type_name = match obj_ty {
                                    TejxType::Class(ref c, _) => c.clone(),
                                    _ => format!("{:?}", obj_ty),
                                };

                                let type_name = type_name
                                    .split('<')
                                    .next()
                                    .unwrap_or(&type_name)
                                    .trim()
                                    .to_string();
                                let method_key = format!("f_{}_{}", type_name, member);

                                if let Some(ret_ty) = self.user_functions.borrow().get(&method_key)
                                {
                                    final_callee = method_key.clone();
                                    // Substitute generic type params from the concrete object type
                                    ty = self.substitute_generics(
                                        ret_ty,
                                        &obj_hir.get_type(),
                                        &final_callee,
                                    );
                                } else if self.extern_functions.borrow().contains(&method_key) {
                                    final_callee = method_key;
                                } else {
                                    // Walk class hierarchy to find inherited methods
                                    let mut found = false;
                                    let mut parent_class =
                                        { self.class_parents.borrow().get(&type_name).cloned() };
                                    while let Some(ref parent) = parent_class {
                                        let parent_method_key = format!("f_{}_{}", parent, member);
                                        if let Some(ret_ty) =
                                            self.user_functions.borrow().get(&parent_method_key)
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
                                        final_callee = method_key;
                                    }
                                }
                                let mut n_args = vec![obj_hir];
                                n_args.extend(hir_args.clone());
                                final_args = n_args;
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

                    if !generic_suffix.is_empty() {
                        final_callee.push_str(&generic_suffix);
                    }

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
                            ty: TejxType::Int64,
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

                // Static Field Resolution
                if !obj_name.is_empty() {
                    let s_fields = self.class_static_fields.borrow();
                    if let Some(f_list) = s_fields.get(&obj_name) {
                        if let Some((_name, f_ty, _)) = f_list.iter().find(|(n, _, _)| n == member)
                        {
                            return HIRExpression::Variable {
                                line: line,
                                name: format!("g_{}_{}", obj_name, member),
                                ty: f_ty.clone(),
                            };
                        }
                    }
                }

                // Getter Resolution
                let obj_ty = self.lower_expression(object).get_type();
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
                                line: line,
                                callee: format!("f_{}_get_{}", class_name, member),
                                args: vec![self.lower_expression(object)],
                                ty: TejxType::Int64,
                            };
                        }
                    }
                }

                // Field Resolution
                let lowered_object = self.lower_expression(object);
                let obj_ty = lowered_object.get_type();
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
                                    line: line,
                                    target: Box::new(lowered_object),
                                    member: member.clone(),
                                    ty: f_ty.clone(),
                                };
                            }
                        }
                    }
                    // Static field? (If object name matches class name, handled differently usually, but let's check)
                }

                let combined = format!("{}_{}", obj_name, member);
                let f_combined = format!("f_{}", combined);
                if self.user_functions.borrow().contains_key(&combined)
                    || self.user_functions.borrow().contains_key(&f_combined)
                {
                    HIRExpression::Variable {
                        line: line,
                        name: f_combined,
                        ty: TejxType::Int64,
                    }
                } else {
                    HIRExpression::MemberAccess {
                        line: line,
                        target: Box::new(lowered_object),
                        member: member.clone(),
                        ty: TejxType::Int64,
                    }
                }
            }
            Expression::ArrayAccessExpr { target, index, .. } => {
                let lowered_target = self.lower_expression(target);
                let target_ty = lowered_target.get_type();

                // If it's an Array<T> class instance, desugar to arr.data[i]
                if let TejxType::Class(name, _) = &target_ty {
                    if name.starts_with("Array<") || name == "Array" {
                        let elem_ty = target_ty.get_array_element_type();
                        return HIRExpression::IndexAccess {
                            line: line,
                            target: Box::new(HIRExpression::MemberAccess {
                                line: line,
                                target: Box::new(lowered_target),
                                member: "data".to_string(),
                                ty: TejxType::Class(format!("{}[]", elem_ty.to_name()), vec![]),
                            }),
                            index: Box::new(self.lower_expression(index)),
                            ty: elem_ty,
                        };
                    }
                }

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
                    None => TejxType::Class("Int64[]".to_string(), vec![]),
                };
                // Handle spreads: [a, ...b, c] -> concat(concat([a], b), [c])
                let mut chunks: Vec<HIRExpression> = Vec::new();
                let mut current_chunk: Vec<HIRExpression> = Vec::new();

                for e in elements {
                    if let Expression::SpreadExpr { _expr, .. } = e {
                        // Push accumulated static chunk if any
                        if !current_chunk.is_empty() {
                            chunks.push(HIRExpression::ArrayLiteral {
                                line: line,
                                elements: current_chunk.clone(),
                                ty: inferred_ty.clone(),
                                sized_allocation: None,
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
                        sized_allocation: None,
                    });
                }

                if chunks.is_empty() {
                    // Empty array []
                    HIRExpression::ArrayLiteral {
                        line: line,
                        elements: vec![],
                        sized_allocation: None,
                        ty: inferred_ty.clone(),
                    }
                } else {
                    // Reduce chunks with Array_concat
                    let mut expr = chunks[0].clone();
                    for next_chunk in chunks.into_iter().skip(1) {
                        expr = HIRExpression::Call {
                            line: line,
                            callee: "rt_array_concat".to_string(),
                            args: vec![expr, next_chunk],
                            ty: TejxType::Class("Int64[]".to_string(), vec![]),
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
                        _return_type: TejxType::Int64,
                        body: Box::new(hir_body),
                        is_extern: false,
                    });

                HIRExpression::Literal {
                    line: line,
                    value: lambda_name,
                    ty: TejxType::Int64, // Actually function type
                }
            }
            Expression::AwaitExpr { expr, .. } => HIRExpression::Await {
                line: line,
                expr: Box::new(self.lower_expression(expr)),
                ty: TejxType::Int64,
            },
            Expression::OptionalMemberAccessExpr { object, member, .. } => {
                HIRExpression::OptionalChain {
                    line: line,
                    target: Box::new(self.lower_expression(object)),
                    operation: format!(".{}", member),
                    ty: TejxType::Int64,
                }
            }
            Expression::NewExpr {
                class_name, args, ..
            } => {
                let mut hir_args: Vec<HIRExpression> =
                    args.iter().map(|a| self.lower_expression(a)).collect();

                let mut normalized_class = class_name.clone();
                if let Some(pos) = normalized_class.find('<') {
                    normalized_class = normalized_class[..pos].to_string();
                }

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
                            ty: TejxType::Int64,
                        });
                        hir_args = new_var_args;
                    }
                }

                HIRExpression::NewExpr {
                    line: line,
                    class_name: normalized_class,
                    _args: hir_args,
                    ty: TejxType::from_name(class_name),
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
                HIRExpression::If {
                    line: line,
                    condition: Box::new(cond),
                    then_branch: Box::new(t_branch),
                    else_branch: Box::new(f_branch),
                    ty: TejxType::Int64,
                }
            }

            _ => HIRExpression::Literal {
                line: line,
                value: "0".to_string(),
                ty: TejxType::Int64,
            },
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
