use crate::token::TokenType;
/// High-level Intermediate Representation (HIR), mirroring C++ HIR.h
/// HIR is a typed AST with desugared control flow (all loops → unified HIRLoop)
use crate::types::TejxType;

#[derive(Debug, Clone)]
pub enum HIRExpression {
    Literal {
        value: String,
        ty: TejxType,
        line: usize,
    },
    Variable {
        name: String,
        ty: TejxType,
        line: usize,
    },
    BinaryExpr {
        left: Box<HIRExpression>,
        op: TokenType,
        right: Box<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    Call {
        callee: String,
        args: Vec<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    IndirectCall {
        callee: Box<HIRExpression>,
        args: Vec<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    NewExpr {
        class_name: String,
        _args: Vec<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    Assignment {
        target: Box<HIRExpression>,
        value: Box<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    Await {
        expr: Box<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    Sequence {
        expressions: Vec<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    OptionalChain {
        // Unified optional access
        target: Box<HIRExpression>, // Object or array
        operation: String,          // ".prop" or "[index]" or "()"
        ty: TejxType,
        line: usize,
    },
    IndexAccess {
        target: Box<HIRExpression>,
        index: Box<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    MemberAccess {
        target: Box<HIRExpression>,
        member: String,
        ty: TejxType,
        line: usize,
    },
    ObjectLiteral {
        entries: Vec<(String, HIRExpression)>,
        ty: TejxType,
        line: usize,
    },
    ArrayLiteral {
        elements: Vec<HIRExpression>,
        sized_allocation: Option<Box<HIRExpression>>,
        ty: TejxType,
        line: usize,
    },
    If {
        condition: Box<HIRExpression>,
        then_branch: Box<HIRExpression>,
        else_branch: Box<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
    NoneLiteral {
        line: usize,
    },
    SomeExpr {
        value: Box<HIRExpression>,
        line: usize,
    },
    Cast {
        expr: Box<HIRExpression>,
        ty: TejxType,
        line: usize,
    },
}

impl HIRExpression {
    pub fn get_type(&self) -> TejxType {
        match self {
            HIRExpression::Literal { ty, .. } => ty.clone(),
            HIRExpression::Variable { ty, .. } => ty.clone(),
            HIRExpression::BinaryExpr { ty, .. } => ty.clone(),
            HIRExpression::Call { ty, .. } => ty.clone(),
            HIRExpression::IndirectCall { ty, .. } => ty.clone(),
            HIRExpression::NewExpr { ty, .. } => ty.clone(),
            HIRExpression::Assignment { ty, .. } => ty.clone(),
            HIRExpression::Await { ty, .. } => ty.clone(),
            HIRExpression::OptionalChain { ty, .. } => ty.clone(),
            HIRExpression::IndexAccess { ty, .. } => ty.clone(),
            HIRExpression::MemberAccess { ty, .. } => ty.clone(),
            HIRExpression::ObjectLiteral { ty, .. } => ty.clone(),
            HIRExpression::ArrayLiteral { ty, .. } => ty.clone(),
            HIRExpression::If { ty, .. } => ty.clone(),
            HIRExpression::Sequence { ty, .. } => ty.clone(),
            HIRExpression::NoneLiteral { .. } => TejxType::Void, // None is generic
            HIRExpression::SomeExpr { value, .. } => value.get_type(), // Simplified
            HIRExpression::Cast { ty, .. } => ty.clone(),
        }
    }

    pub fn get_line(&self) -> usize {
        match self {
            HIRExpression::Literal { line, .. } => *line,
            HIRExpression::Variable { line, .. } => *line,
            HIRExpression::BinaryExpr { line, .. } => *line,
            HIRExpression::Call { line, .. } => *line,
            HIRExpression::IndirectCall { line, .. } => *line,
            HIRExpression::NewExpr { line, .. } => *line,
            HIRExpression::Assignment { line, .. } => *line,
            HIRExpression::Await { line, .. } => *line,
            HIRExpression::OptionalChain { line, .. } => *line,
            HIRExpression::IndexAccess { line, .. } => *line,
            HIRExpression::MemberAccess { line, .. } => *line,
            HIRExpression::ObjectLiteral { line, .. } => *line,
            HIRExpression::ArrayLiteral { line, .. } => *line,
            HIRExpression::If { line, .. } => *line,
            HIRExpression::Sequence { line, .. } => *line,
            HIRExpression::NoneLiteral { line, .. } => *line,
            HIRExpression::SomeExpr { line, .. } => *line,
            HIRExpression::Cast { line, .. } => *line,
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
        line: usize,
    },
    Block {
        statements: Vec<HIRStatement>,
        line: usize,
    },
    VarDecl {
        name: String,
        initializer: Option<HIRExpression>,
        ty: TejxType,
        _is_const: bool,
        line: usize,
    },
    Function {
        name: String,
        params: Vec<(String, TejxType)>,
        _return_type: TejxType,
        body: Box<HIRStatement>, // Should be a Block
        is_extern: bool,
        async_params: Option<Vec<(String, TejxType)>>,
        line: usize,
    },
    Return {
        value: Option<HIRExpression>,
        line: usize,
    },
    Loop {
        condition: HIRExpression,
        body: Box<HIRStatement>, // Should be a Block
        increment: Option<Box<HIRStatement>>,
        _is_do_while: bool,
        line: usize,
    },
    If {
        condition: HIRExpression,
        then_branch: Box<HIRStatement>,
        else_branch: Option<Box<HIRStatement>>,
        line: usize,
    },
    Switch {
        condition: HIRExpression,
        cases: Vec<HIRCase>,
        line: usize,
    },
    Break {
        line: usize,
    },
    Continue {
        line: usize,
    },
    Try {
        try_block: Box<HIRStatement>,
        catch_var: Option<String>,
        catch_block: Box<HIRStatement>,
        finally_block: Option<Box<HIRStatement>>,
        line: usize,
    },
    Throw {
        value: HIRExpression,
        line: usize,
    },
    Sequence {
        statements: Vec<HIRStatement>,
        line: usize,
    },
}

impl HIRStatement {
    pub fn get_line(&self) -> usize {
        match self {
            HIRStatement::VarDecl { line, .. } => *line,
            HIRStatement::ExpressionStmt { line, .. } => *line,
            HIRStatement::Block { line, .. } => *line,
            HIRStatement::Loop { line, .. } => *line,
            HIRStatement::If { line, .. } => *line,
            HIRStatement::Return { line, .. } => *line,
            HIRStatement::Break { line } => *line,
            HIRStatement::Continue { line } => *line,
            HIRStatement::Switch { line, .. } => *line,
            HIRStatement::Try { line, .. } => *line,
            HIRStatement::Throw { line, .. } => *line,
            HIRStatement::Function { line, .. } => *line,
            HIRStatement::Sequence { line, .. } => *line,
        }
    }
}
