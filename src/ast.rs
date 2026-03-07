use crate::token::TokenType;
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct Program {
    pub statements: Vec<Statement>, // In C++ this was vector<shared_ptr<ASTNode>>, but mostly statements
}

#[derive(Debug, Clone)]
pub enum Statement {
    VarDeclaration {
        pattern: BindingNode,
        type_annotation: String,
        initializer: Option<Box<Expression>>,
        is_const: bool,
        line: usize,
        _col: usize,
    },
    FunctionDeclaration(FunctionDeclaration),
    ClassDeclaration(ClassDeclaration),

    // ProtocolDeclaration(ProtocolDeclaration), // Removed
    ExtensionDeclaration(ExtensionDeclaration), // If needed
    EnumDeclaration(EnumDeclaration),
    TypeAliasDeclaration {
        name: String,
        _type_def: String,
        _line: usize,
        _col: usize,
    },
    InterfaceDeclaration {
        name: String,
        _methods: Vec<InterfaceMethod>,
        _line: usize,
        _col: usize,
    },
    ReturnStmt {
        value: Option<Box<Expression>>,
        _line: usize,
        _col: usize,
    },
    BreakStmt {
        _line: usize,
        _col: usize,
    },
    ContinueStmt {
        _line: usize,
        _col: usize,
    },
    BlockStmt {
        statements: Vec<Statement>,
        _line: usize,
        _col: usize,
    },
    IfStmt {
        condition: Box<Expression>,
        then_branch: Box<Statement>,
        else_branch: Option<Box<Statement>>,
        _line: usize,
        _col: usize,
    },
    WhileStmt {
        condition: Box<Expression>,
        body: Box<Statement>,
        _line: usize,
        _col: usize,
    },
    ForStmt {
        init: Option<Box<Statement>>, // Can be VarDecl or ExpressionStmt
        condition: Option<Box<Expression>>,
        increment: Option<Box<Expression>>,
        body: Box<Statement>,
        _line: usize,
        _col: usize,
    },
    ForOfStmt {
        variable: BindingNode,
        iterable: Box<Expression>,
        body: Box<Statement>,
        _line: usize,
        _col: usize,
    },
    SwitchStmt {
        condition: Box<Expression>,
        cases: Vec<Case>,
        _line: usize,
        _col: usize,
    },
    ExpressionStmt {
        _expression: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    TryStmt {
        _try_block: Box<Statement>, // BlockStmt
        _catch_var: String,
        _catch_block: Box<Statement>,           // BlockStmt
        _finally_block: Option<Box<Statement>>, // BlockStmt
        _line: usize,
        _col: usize,
    },
    DelStmt {
        target: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    ThrowStmt {
        _expression: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    ImportDecl {
        _names: Vec<ImportItem>,
        source: String,
        _is_default: bool,
        _line: usize,
        _col: usize,
    },
    ExportDecl {
        declaration: Box<Statement>, // Can be Function/Class/VarDecl
        _is_default: bool,
        _line: usize,
        _col: usize,
    },
}

#[derive(Debug, Clone)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Expression {
    NumberLiteral {
        value: f64,
        _is_float: bool,
        _line: usize,
        _col: usize,
    },
    StringLiteral {
        value: String,
        _line: usize,
        _col: usize,
    },
    BooleanLiteral {
        value: bool,
        _line: usize,
        _col: usize,
    },
    Identifier {
        name: String,
        _line: usize,
        _col: usize,
    },
    BinaryExpr {
        left: Box<Expression>,
        op: TokenType,
        right: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    UnaryExpr {
        op: TokenType,
        right: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    AssignmentExpr {
        target: Box<Expression>,
        value: Box<Expression>,
        _op: TokenType,
        _line: usize,
        _col: usize,
    },
    CallExpr {
        callee: Box<Expression>,
        args: Vec<Expression>,
        _line: usize,
        _col: usize,
    },
    SequenceExpr {
        expressions: Vec<Expression>,
        _line: usize,
        _col: usize,
    },
    MemberAccessExpr {
        object: Box<Expression>,
        member: String,
        _line: usize,
        _col: usize,
        _is_namespace: bool,
    },
    ArrayAccessExpr {
        target: Box<Expression>,
        index: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    ObjectLiteralExpr {
        entries: Vec<(String, Expression)>,
        _spreads: Vec<Expression>,
        _line: usize,
        _col: usize,
    },
    ArrayLiteral {
        elements: Vec<Expression>,
        ty: RefCell<Option<String>>,
        _line: usize,
        _col: usize,
    },
    NewExpr {
        class_name: String,
        args: Vec<Expression>,
        _line: usize,
        _col: usize,
    },
    ThisExpr {
        _line: usize,
        _col: usize,
    },
    SuperExpr {
        _line: usize,
        _col: usize,
    },
    // TemplateLiteralExpr removed
    LambdaExpr {
        params: Vec<Parameter>,
        body: Box<Statement>, // BlockStmt
        _line: usize,
        _col: usize,
    },
    AwaitExpr {
        expr: Box<Expression>,
        _line: usize,
        _col: usize,
    },

    TernaryExpr {
        _condition: Box<Expression>,
        _true_branch: Box<Expression>,
        _false_branch: Box<Expression>,
        _line: usize,
        _col: usize,
    },

    // Optional chaining
    OptionalMemberAccessExpr {
        object: Box<Expression>,
        member: String,
        _line: usize,
        _col: usize,
    },
    OptionalCallExpr {
        callee: Box<Expression>,
        args: Vec<Expression>,
        _line: usize,
        _col: usize,
    },
    OptionalArrayAccessExpr {
        target: Box<Expression>,
        index: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    NullishCoalescingExpr {
        _left: Box<Expression>,
        _right: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    SpreadExpr {
        _expr: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    NoneLiteral {
        _line: usize,
        _col: usize,
    },
    SomeExpr {
        value: Box<Expression>,
        _line: usize,
        _col: usize,
    },
    CastExpr {
        expr: Box<Expression>,
        target_type: String,
        _line: usize,
        _col: usize,
    },
}

#[derive(Debug, Clone)]
pub struct FunctionDeclaration {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: String,
    pub body: Box<Statement>, // BlockStmt
    pub _is_async: bool,
    pub is_extern: bool,
    pub generic_params: Vec<String>,
    pub _line: usize,
    pub _col: usize,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub type_name: String,
    pub _default_value: Option<Box<Expression>>,
    pub _is_rest: bool,
}

#[derive(Debug, Clone)]
pub struct ClassDeclaration {
    pub name: String,
    pub _parent_name: String,
    pub generic_params: Vec<String>,
    pub _is_abstract: bool,
    pub _implemented_protocols: Vec<String>,
    pub _members: Vec<ClassMember>,
    pub methods: Vec<ClassMethod>,
    pub _getters: Vec<ClassGetter>,
    pub _setters: Vec<ClassSetter>,
    pub _constructor: Option<FunctionDeclaration>,
    pub _line: usize,
    pub _col: usize,
}

#[derive(Debug, Clone)]
pub struct ClassMember {
    pub _name: String,
    pub _type_name: String,
    pub _access: AccessModifier,
    pub _is_static: bool,
    pub _initializer: Option<Box<Expression>>,
}

#[derive(Debug, Clone)]
pub struct ClassMethod {
    pub func: FunctionDeclaration,
    pub _access: AccessModifier,
    pub is_static: bool,
    pub _is_abstract: bool,
}

#[derive(Debug, Clone)]
pub struct ClassGetter {
    pub _name: String,
    pub _return_type: String,
    pub _body: Box<Statement>,
    pub _access: AccessModifier,
}

#[derive(Debug, Clone)]
pub struct ClassSetter {
    pub _name: String,
    pub _param_name: String,
    pub _param_type: String,
    pub _body: Box<Statement>,
    pub _access: AccessModifier,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AccessModifier {
    Public,
    Private,
    Protected,
}

#[derive(Debug, Clone)]
pub struct EnumDeclaration {
    pub name: String,
    pub _members: Vec<EnumMember>,
    pub _line: usize,
    pub _col: usize,
}

#[derive(Debug, Clone)]
pub struct EnumMember {
    pub _name: String,
    pub _value: Option<Box<Expression>>,
}

#[derive(Debug, Clone)]
pub struct Case {
    pub value: Option<Box<Expression>>, // None for default
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone)]
pub struct InterfaceMethod {
    pub _name: String,
    pub _params: Vec<Parameter>,
    pub _return_type: String,
}

// Removed ProtocolDeclaration

#[derive(Debug, Clone)]
pub struct ExtensionDeclaration {
    pub _target_type: String,
    pub _methods: Vec<FunctionDeclaration>,
    pub _line: usize,
    pub _col: usize,
}

#[derive(Debug, Clone)]
pub enum BindingNode {
    Identifier(String),
    ArrayBinding {
        elements: Vec<BindingNode>,
        rest: Option<Box<BindingNode>>,
    },
    ObjectBinding {
        entries: Vec<(String, BindingNode)>,
    },
    // C++ parsed literals in patterns too, but BindingNode usually implies destructuring.
    // We'll stick to this for now.
}

impl Expression {
    pub fn to_callee_name(&self) -> String {
        match self {
            Expression::Identifier { name, .. } => name.clone(),
            Expression::ThisExpr { .. } => "this".to_string(),
            Expression::SuperExpr { .. } => "super".to_string(),
            Expression::NewExpr { class_name, .. } => format!("$new_{}", class_name),
            Expression::MemberAccessExpr { object, member, .. } => {
                let base = object.to_callee_name();
                if base.is_empty() {
                    "".to_string() // Return empty if base is not a simple name
                } else {
                    format!("{}.{}", base, member)
                }
            }
            Expression::StringLiteral { .. } => "String".to_string(),
            Expression::NumberLiteral { .. } => "(number)".to_string(),
            Expression::BooleanLiteral { .. } => "(bool)".to_string(),
            Expression::ArrayLiteral { .. } => "Array".to_string(),
            Expression::ObjectLiteralExpr { .. } => "(object)".to_string(),
            Expression::NoneLiteral { .. } => "None".to_string(),
            Expression::SomeExpr { .. } => "Some".to_string(),
            Expression::CastExpr { expr, .. } => expr.to_callee_name(),
            _ => "".to_string(),
        }
    }

    pub fn get_line(&self) -> usize {
        match self {
            Expression::NumberLiteral { _line, .. } => *_line,
            Expression::StringLiteral { _line, .. } => *_line,
            Expression::BooleanLiteral { _line, .. } => *_line,
            Expression::Identifier { _line, .. } => *_line,
            Expression::BinaryExpr { _line, .. } => *_line,
            Expression::UnaryExpr { _line, .. } => *_line,
            Expression::AssignmentExpr { _line, .. } => *_line,
            Expression::CallExpr { _line, .. } => *_line,
            Expression::MemberAccessExpr { _line, .. } => *_line,
            Expression::ArrayAccessExpr { _line, .. } => *_line,
            Expression::ObjectLiteralExpr { _line, .. } => *_line,
            Expression::ArrayLiteral { _line, .. } => *_line,
            Expression::NewExpr { _line, .. } => *_line,
            Expression::ThisExpr { _line, .. } => *_line,
            Expression::SuperExpr { _line, .. } => *_line,
            Expression::LambdaExpr { _line, .. } => *_line,
            Expression::AwaitExpr { _line, .. } => *_line,
            Expression::TernaryExpr { _line, .. } => *_line,
            Expression::OptionalMemberAccessExpr { _line, .. } => *_line,
            Expression::OptionalCallExpr { _line, .. } => *_line,
            Expression::OptionalArrayAccessExpr { _line, .. } => *_line,
            Expression::NullishCoalescingExpr { _line, .. } => *_line,
            Expression::SpreadExpr { _line, .. } => *_line,
            Expression::SequenceExpr { _line, .. } => *_line,
            Expression::NoneLiteral { _line, .. } => *_line,
            Expression::SomeExpr { _line, .. } => *_line,
            Expression::CastExpr { _line, .. } => *_line,
        }
    }
}

impl Statement {
    pub fn get_line(&self) -> usize {
        match self {
            Statement::VarDeclaration { line, .. } => *line,
            Statement::FunctionDeclaration(f) => f._line,
            Statement::ClassDeclaration(c) => c._line,
            Statement::ExtensionDeclaration(e) => e._line,
            Statement::EnumDeclaration(e) => e._line,
            Statement::TypeAliasDeclaration { _line, .. } => *_line,
            Statement::InterfaceDeclaration { _line, .. } => *_line,
            Statement::ReturnStmt { _line, .. } => *_line,
            Statement::BreakStmt { _line, .. } => *_line,
            Statement::ContinueStmt { _line, .. } => *_line,
            Statement::BlockStmt { _line, .. } => *_line,
            Statement::IfStmt { _line, .. } => *_line,
            Statement::WhileStmt { _line, .. } => *_line,
            Statement::ForStmt { _line, .. } => *_line,
            Statement::ForOfStmt { _line, .. } => *_line,
            Statement::SwitchStmt { _line, .. } => *_line,
            Statement::ExpressionStmt { _line, .. } => *_line,
            Statement::TryStmt { _line, .. } => *_line,
            Statement::DelStmt { _line, .. } => *_line,
            Statement::ThrowStmt { _line, .. } => *_line,
            Statement::ImportDecl { _line, .. } => *_line,
            Statement::ExportDecl { _line, .. } => *_line,
        }
    }

    pub fn get_col(&self) -> usize {
        match self {
            Statement::VarDeclaration { _col, .. } => *_col,
            Statement::FunctionDeclaration(f) => f._col,
            Statement::ClassDeclaration(c) => c._col,
            Statement::ExtensionDeclaration(e) => e._col,
            Statement::EnumDeclaration(e) => e._col,
            Statement::TypeAliasDeclaration { _col, .. } => *_col,
            Statement::InterfaceDeclaration { _col, .. } => *_col,
            Statement::ReturnStmt { _col, .. } => *_col,
            Statement::BreakStmt { _col, .. } => *_col,
            Statement::ContinueStmt { _col, .. } => *_col,
            Statement::BlockStmt { _col, .. } => *_col,
            Statement::IfStmt { _col, .. } => *_col,
            Statement::WhileStmt { _col, .. } => *_col,
            Statement::ForStmt { _col, .. } => *_col,
            Statement::ForOfStmt { _col, .. } => *_col,
            Statement::SwitchStmt { _col, .. } => *_col,
            Statement::ExpressionStmt { _col, .. } => *_col,
            Statement::TryStmt { _col, .. } => *_col,
            Statement::DelStmt { _col, .. } => *_col,
            Statement::ThrowStmt { _col, .. } => *_col,
            Statement::ImportDecl { _col, .. } => *_col,
            Statement::ExportDecl { _col, .. } => *_col,
        }
    }
}
