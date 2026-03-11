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
                        ty: TejxType::Int64,
                    };
                    self.lower_binding_pattern(el, Some(el_init), &TejxType::Void, is_const, stmts);
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
                        ty: TejxType::Int64,
                    };
                    self.lower_binding_pattern(
                        r,
                        Some(slice_init),
                        &TejxType::Void,
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
                    let el_init = HIRExpression::MemberAccess {
                        line: line,
                        target: Box::new(HIRExpression::Variable {
                            line: line,
                            name: tmp_id.clone(),
                            ty: ty.clone(),
                        }),
                        member: key.clone(),
                        ty: TejxType::Int64,
                    };
                    self.lower_binding_pattern(
                        target,
                        Some(el_init),
                        &TejxType::Void,
                        is_const,
                        stmts,
                    );
                }
            }
        }
    }
}
