/// High-level Intermediate Representation (HIR), mirroring C++ HIR.h
/// HIR is a typed AST with desugared control flow (all loops → unified HIRLoop)

use crate::types::TejxType;
use crate::token::TokenType;
use crate::ast::BindingNode;

#[derive(Debug, Clone)]
pub enum HIRExpression {
    Literal {
        value: String,
        ty: TejxType,
    },
    Variable {
        name: String,
        ty: TejxType,
    },
    BinaryExpr {
        left: Box<HIRExpression>,
        op: TokenType,
        right: Box<HIRExpression>,
        ty: TejxType,
    },
    Call {
        callee: String,
        args: Vec<HIRExpression>,
        ty: TejxType,
    },
    IndirectCall {
        callee: Box<HIRExpression>,
        args: Vec<HIRExpression>,
        ty: TejxType,
    },
    NewExpr {
        class_name: String,
        _args: Vec<HIRExpression>,
    },
    Assignment {
        target: Box<HIRExpression>,
        value: Box<HIRExpression>,
        ty: TejxType,
    },
    Await {
        expr: Box<HIRExpression>,
        ty: TejxType,
    },
    OptionalChain { // Unified optional access
        target: Box<HIRExpression>, // Object or array
        operation: String, // ".prop" or "[index]" or "()"
        ty: TejxType,
    },
    IndexAccess {
        target: Box<HIRExpression>,
        index: Box<HIRExpression>,
        ty: TejxType,
    },
    MemberAccess {
        target: Box<HIRExpression>,
        member: String,
        ty: TejxType,
    },
    ObjectLiteral {
        entries: Vec<(String, HIRExpression)>,
        ty: TejxType,
    },
    ArrayLiteral {
        elements: Vec<HIRExpression>,
        ty: TejxType,
    },
    Match {
        target: Box<HIRExpression>,
        arms: Vec<HIRMatchArm>,
        ty: TejxType,
    },
    BlockExpr {
        statements: Vec<HIRStatement>,
        ty: TejxType,
    },
    If {
        condition: Box<HIRExpression>,
        then_branch: Box<HIRExpression>,
        else_branch: Box<HIRExpression>,
        ty: TejxType,
    },
}

#[derive(Debug, Clone)]
pub struct HIRMatchArm {
    pub pattern: BindingNode,
    pub guard: Option<Box<HIRExpression>>,
    pub body: Box<HIRExpression>,
}

impl HIRExpression {
    pub fn get_type(&self) -> TejxType {
        match self {
            HIRExpression::Literal { ty, .. } => ty.clone(),
            HIRExpression::Variable { ty, .. } => ty.clone(),
            HIRExpression::BinaryExpr { ty, .. } => ty.clone(),
            HIRExpression::Call { ty, .. } => ty.clone(),
            HIRExpression::IndirectCall { ty, .. } => ty.clone(),
            HIRExpression::NewExpr { class_name, .. } => TejxType::Class(class_name.clone()),
            HIRExpression::Assignment { ty, .. } => ty.clone(),
            HIRExpression::Await { ty, .. } => ty.clone(),
            HIRExpression::OptionalChain { ty, .. } => ty.clone(),
            HIRExpression::IndexAccess { ty, .. } => ty.clone(),
            HIRExpression::MemberAccess { ty, .. } => ty.clone(),
            HIRExpression::ObjectLiteral { ty, .. } => ty.clone(),
            HIRExpression::ArrayLiteral { ty, .. } => ty.clone(),
            HIRExpression::Match { ty, .. } => ty.clone(),
            HIRExpression::BlockExpr { ty, .. } => ty.clone(),
            HIRExpression::If { ty, .. } => ty.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HIRCase {
    pub value: Option<HIRExpression>,
    pub body: Box<HIRStatement>,
}

#[derive(Debug, Clone)]
pub enum HIRStatement {
    ExpressionStmt {
        expr: HIRExpression,
    },
    Block {
        statements: Vec<HIRStatement>,
    },
    VarDecl {
        name: String,
        initializer: Option<HIRExpression>,
        ty: TejxType,
        _is_const: bool,
    },
    Function {
        name: String,
        params: Vec<(String, TejxType)>,
        _return_type: TejxType,
        body: Box<HIRStatement>,  // Should be a Block
    },
    Return {
        value: Option<HIRExpression>,
    },
    Loop {
        condition: HIRExpression,
        body: Box<HIRStatement>,  // Should be a Block
        increment: Option<Box<HIRStatement>>,
        _is_do_while: bool,
    },
    If {
        condition: HIRExpression,
        then_branch: Box<HIRStatement>,
        else_branch: Option<Box<HIRStatement>>,
    },
    Switch {
        condition: HIRExpression,
        cases: Vec<HIRCase>,
    },
    Break,
    Continue,
    Try {
        try_block: Box<HIRStatement>,
        catch_var: Option<String>,
        catch_block: Box<HIRStatement>,
        finally_block: Option<Box<HIRStatement>>,
    },
    Throw {
        value: HIRExpression,
    },
}
