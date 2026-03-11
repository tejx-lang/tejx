use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::types::TejxType;

impl Lowering {
    pub(crate) fn lower_binding_pattern(
        &self,
        pattern: &BindingNode,
        initializer: Option<HIRExpression>,
        ty: &TejxType,
        is_const: bool,
        stmts: &mut Vec<HIRStatement>,
    ) {
        let line = 0; // Default or pass as arg? Let us pass as arg maybe.
                      // Actually, many callers don't have a line here.
        match pattern {
            BindingNode::Identifier(name) => {
                let mangled = self.define(name.clone(), ty.clone());
                stmts.push(HIRStatement::VarDecl {
                    line: line,
                    name: mangled,
                    initializer,
                    ty: ty.clone(),
                    _is_const: is_const,
                });
            }
            BindingNode::ArrayBinding { elements, rest } => {
                // let [a, b] = init;
                // Lower as:
                // let tmp = init;
                // let a = tmp[0];
                // let b = tmp[1];
                let tmp_id = format!("destructure_tmp_{}", self.lambda_counter.borrow());
                *self.lambda_counter.borrow_mut() += 1;

                stmts.push(HIRStatement::VarDecl {
                    line: line,
                    name: tmp_id.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: true,
                });

                let element_ty = match ty {
                    TejxType::DynamicArray(inner) | TejxType::FixedArray(inner, _) | TejxType::Slice(inner) => *inner.clone(),
                    TejxType::Class(n, generics) if n == "Array" && generics.len() == 1 => generics[0].clone(),
                    _ => TejxType::Any,
                };

                for (i, el) in elements.iter().enumerate() {
                    let el_init = HIRExpression::IndexAccess {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
                            name: tmp_id.clone(),
                            ty: ty.clone(),
                        }),
                        index: Box::new(HIRExpression::Literal {
                            line: line,
                            value: i.to_string(),
                            ty: TejxType::Int32,
                        }),
                        ty: element_ty.clone(),
                    };
                    self.lower_binding_pattern(el, Some(el_init), &element_ty, is_const, stmts);
                }

                if let Some(r) = rest {
                    // handle rest ...tail
                    // let tail = Array_sliceRest(tmp, elements.len());
                    let slice_init = HIRExpression::Call {
                        line: line,
                        callee: "f_RT_Array_sliceRest".to_string(),
                        args: vec![
                            HIRExpression::Variable {
                                line: line,
                                name: tmp_id.clone(),
                                ty: ty.clone(),
                            },
                            HIRExpression::Literal {
                                line: line,
                                value: elements.len().to_string(),
                                ty: TejxType::Int32,
                            },
                        ],
                        ty: ty.clone(),
                    };
                    self.lower_binding_pattern(
                        r,
                        Some(slice_init),
                        ty,
                        is_const,
                        stmts,
                    );
                }
            }
            BindingNode::ObjectBinding { entries } => {
                let tmp_id = format!("destructure_tmp_{}", self.lambda_counter.borrow());
                *self.lambda_counter.borrow_mut() += 1;

                stmts.push(HIRStatement::VarDecl {
                    line: line,
                    name: tmp_id.clone(),
                    initializer,
                    ty: ty.clone(),
                    _is_const: true,
                });

                for (key, target) in entries {
                    let mut prop_ty = TejxType::Any;
                    if let TejxType::Object(props) = ty {
                        for (k, _, t) in props {
                            if k == key {
                                prop_ty = t.clone();
                                break;
                            }
                        }
                    } else if let TejxType::Class(name, _) = ty {
                        if let Some(fields) = self.class_instance_fields.borrow().get(name) {
                            for (k, t, _) in fields {
                                if k == key {
                                    prop_ty = t.clone();
                                    break;
                                }
                            }
                        }
                    }

                    let el_init = HIRExpression::MemberAccess {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
                            name: tmp_id.clone(),
                            ty: ty.clone(),
                        }),
                        member: key.clone(),
                        ty: prop_ty.clone(),
                    };
                    self.lower_binding_pattern(
                        target,
                        Some(el_init),
                        &prop_ty,
                        is_const,
                        stmts,
                    );
                }
            }
        }
    }
}
