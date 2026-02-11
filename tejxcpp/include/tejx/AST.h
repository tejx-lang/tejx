#pragma once

#include <string>
#include <vector>
#include <memory>
#include "Lexer.h"

namespace tejx {

struct Statement;

// Base class for all AST nodes
struct ASTNode {
    int line = 0;
    int col = 0;
    std::string file;
    virtual ~ASTNode() = default;
};

// Expressions (produce values)
struct Expression : ASTNode {};

struct NumberLiteral : Expression {
    double value;
    NumberLiteral(double v) : value(v) {}
};

struct StringLiteral : Expression {
    std::string value;
    StringLiteral(const std::string& v) : value(v) {}
};

struct BooleanLiteral : Expression {
    bool value;
    BooleanLiteral(bool v) : value(v) {}
};

struct Identifier : Expression {
    std::string name;
    Identifier(const std::string& n) : name(n) {}
};

struct TernaryExpr : Expression {
    std::shared_ptr<Expression> condition;
    std::shared_ptr<Expression> trueBranch;
    std::shared_ptr<Expression> falseBranch;
    
    TernaryExpr(std::shared_ptr<Expression> c, std::shared_ptr<Expression> t, std::shared_ptr<Expression> f)
        : condition(c), trueBranch(t), falseBranch(f) {}
};

struct AssignmentExpr : Expression {
    std::shared_ptr<Expression> target;
    std::shared_ptr<Expression> value;
    TokenType op;
    AssignmentExpr(std::shared_ptr<Expression> t, std::shared_ptr<Expression> v, TokenType o = TokenType::Equals) 
        : target(t), value(v), op(o) {}
};

struct BinaryExpr : Expression {
    std::shared_ptr<Expression> left;
    TokenType op;
    std::shared_ptr<Expression> right;
    
    BinaryExpr(std::shared_ptr<Expression> l, TokenType o, std::shared_ptr<Expression> r)
        : left(l), op(o), right(r) {}
};

struct InstanceofExpr : Expression {
    std::shared_ptr<Expression> left;
    std::string className;

    InstanceofExpr(std::shared_ptr<Expression> l, const std::string& c)
        : left(l), className(c) {}
};

struct CallExpr : Expression {
    std::string callee; 
    std::vector<std::shared_ptr<Expression>> args;
    
    CallExpr(const std::string& name, std::vector<std::shared_ptr<Expression>> a)
        : callee(name), args(a) {}
};

// Template literal with parts and expressions: `Hello ${name}!`
struct TemplateLiteralExpr : Expression {
    std::vector<std::string> parts;  // String parts between expressions
    std::vector<std::shared_ptr<Expression>> expressions;  // Interpolated expressions
    TemplateLiteralExpr(const std::vector<std::string>& p, const std::vector<std::shared_ptr<Expression>>& e)
        : parts(p), expressions(e) {}
};

struct NewExpr : Expression {
    std::string className;
    std::vector<std::shared_ptr<Expression>> args;
    NewExpr(const std::string& cls, const std::vector<std::shared_ptr<Expression>>& a) 
        : className(cls), args(a) {}
};

struct ThisExpr : Expression {};

struct UnaryExpr : Expression {
    TokenType op;
    std::shared_ptr<Expression> right;
    UnaryExpr(TokenType o, std::shared_ptr<Expression> r) : op(o), right(r) {}
};

struct MemberAccessExpr : Expression {
    std::shared_ptr<Expression> object;
    std::string member;
    MemberAccessExpr(std::shared_ptr<Expression> obj, const std::string& m) : object(obj), member(m) {}
};

struct ArrayLiteral : Expression {
    std::vector<std::shared_ptr<Expression>> elements;
    ArrayLiteral(const std::vector<std::shared_ptr<Expression>>& e) : elements(e) {}
};

// Spread expression: ...arr
struct SpreadExpr : Expression {
    std::shared_ptr<Expression> expr;
    SpreadExpr(std::shared_ptr<Expression> e) : expr(e) {}
};

struct ArrayAccessExpr : Expression {
    std::shared_ptr<Expression> target;
    std::shared_ptr<Expression> index;
    ArrayAccessExpr(std::shared_ptr<Expression> t, std::shared_ptr<Expression> i) : target(t), index(i) {}
};

struct ObjectLiteralExpr : Expression {
    std::vector<std::pair<std::string, std::shared_ptr<Expression>>> entries;
    std::vector<std::shared_ptr<Expression>> spreads; // For ...obj syntax
    ObjectLiteralExpr(const std::vector<std::pair<std::string, std::shared_ptr<Expression>>>& e, 
                      const std::vector<std::shared_ptr<Expression>>& s = {}) 
        : entries(e), spreads(s) {}
};

struct LambdaExpr : Expression {
    struct Param { std::string name; std::string type; };
    std::vector<Param> params;
    std::shared_ptr<Statement> body; // Usually BlockStmt

    LambdaExpr(const std::vector<Param>& p, std::shared_ptr<Statement> b) : params(p), body(b) {}
};

struct AwaitExpr : Expression {
    std::shared_ptr<Expression> expr;
    AwaitExpr(std::shared_ptr<Expression> e) : expr(e) {}
};

struct TypeofExpr : Expression {
    std::shared_ptr<Expression> expr;
    TypeofExpr(std::shared_ptr<Expression> e) : expr(e) {}
};

struct OptionalMemberAccessExpr : Expression {
    std::shared_ptr<Expression> object;
    std::string member;
    OptionalMemberAccessExpr(std::shared_ptr<Expression> obj, const std::string& m) : object(obj), member(m) {}
};

struct OptionalArrayAccessExpr : Expression {
    std::shared_ptr<Expression> target;
    std::shared_ptr<Expression> index;
    OptionalArrayAccessExpr(std::shared_ptr<Expression> t, std::shared_ptr<Expression> i) : target(t), index(i) {}
};

struct OptionalCallExpr : Expression {
    std::shared_ptr<Expression> callee;
    std::vector<std::shared_ptr<Expression>> args;
    OptionalCallExpr(std::shared_ptr<Expression> c, const std::vector<std::shared_ptr<Expression>>& a) : callee(c), args(a) {}
};

struct NullishCoalescingExpr : Expression {
    std::shared_ptr<Expression> left;
    std::shared_ptr<Expression> right;
    NullishCoalescingExpr(std::shared_ptr<Expression> l, std::shared_ptr<Expression> r) : left(l), right(r) {}
};

struct BlockStmt; // Forward declaration

struct BlockExpr : Expression {
    std::shared_ptr<BlockStmt> block;
    BlockExpr(std::shared_ptr<BlockStmt> b) : block(b) {}
};

struct UndefinedExpr : Expression {};
struct NoneExpr : Expression {};

struct SomeExpr : Expression {
    std::shared_ptr<ASTNode> value;
    SomeExpr(std::shared_ptr<ASTNode> v) : value(v) {}
};

struct MatchArm {
    std::shared_ptr<ASTNode> pattern; // Identifier (binding), BindingNode (destructuring), or Expression (literal value)
    std::shared_ptr<Expression> guard; // Optional 'if' guard
    std::shared_ptr<Expression> body;
};

struct MatchExpr : Expression {
    std::shared_ptr<Expression> target;
    std::vector<MatchArm> arms;
    MatchExpr(std::shared_ptr<Expression> t, const std::vector<MatchArm>& a) : target(t), arms(a) {}
};

// Binding Patterns for Destructuring
struct BindingNode : ASTNode {};

struct IdentifierBinding : BindingNode {
    std::string name;
    IdentifierBinding(const std::string& n) : name(n) {}
};

struct ArrayBinding : BindingNode {
    std::vector<std::shared_ptr<ASTNode>> elements;
    std::shared_ptr<ASTNode> rest; // Optional ...rest pattern
    ArrayBinding(const std::vector<std::shared_ptr<ASTNode>>& e, std::shared_ptr<ASTNode> r = nullptr) 
        : elements(e), rest(r) {}
};

struct ObjectBinding : BindingNode {
    struct Entry {
        std::string source; // property name
        std::shared_ptr<ASTNode> target; // pattern for this property
    };
    std::vector<Entry> entries;
    ObjectBinding(const std::vector<Entry>& e) : entries(e) {}
};

// Statements (perform actions)
struct Statement : ASTNode {};

struct VarDeclaration : Statement {
    std::shared_ptr<BindingNode> pattern;
    std::string type; 
    std::shared_ptr<Expression> initializer;
    bool isConst;
    
    VarDeclaration(std::shared_ptr<BindingNode> p, const std::string& t, std::shared_ptr<Expression> init, bool c)
        : pattern(p), type(t), initializer(init), isConst(c) {}
};

struct ReturnStmt : Statement {
    std::shared_ptr<Expression> value;
    ReturnStmt(std::shared_ptr<Expression> v) : value(v) {}
};

struct BreakStmt : Statement {};
struct ContinueStmt : Statement {};

struct BlockStmt : Statement {
    std::vector<std::shared_ptr<Statement>> statements;
};

struct IfStmt : Statement {
    std::shared_ptr<Expression> condition;
    std::shared_ptr<Statement> thenBranch;
    std::shared_ptr<Statement> elseBranch; 
    
    IfStmt(std::shared_ptr<Expression> c, std::shared_ptr<Statement> t, std::shared_ptr<Statement> e)
        : condition(c), thenBranch(t), elseBranch(e) {}
};

struct WhileStmt : Statement {
    std::shared_ptr<Expression> condition;
    std::shared_ptr<Statement> body;
    
    WhileStmt(std::shared_ptr<Expression> c, std::shared_ptr<Statement> b)
        : condition(c), body(b) {}
};

struct ForStmt : Statement {
    std::shared_ptr<Statement> init;
    std::shared_ptr<Expression> condition;
    std::shared_ptr<Expression> increment;
    std::shared_ptr<Statement> body;
    
    ForStmt(std::shared_ptr<Statement> i, std::shared_ptr<Expression> c, 
            std::shared_ptr<Expression> inc, std::shared_ptr<Statement> b)
        : init(i), condition(c), increment(inc), body(b) {}
};

// For-of loop: for (let x of arr) { ... }
struct ForOfStmt : Statement {
    std::string variable;
    std::shared_ptr<Expression> iterable;
    std::shared_ptr<Statement> body;
    
    ForOfStmt(const std::string& v, std::shared_ptr<Expression> i, std::shared_ptr<Statement> b)
        : variable(v), iterable(i), body(b) {}
};

struct Case {
    std::shared_ptr<Expression> value; // nullptr for default
    std::vector<std::shared_ptr<Statement>> statements;
};

struct SwitchStmt : Statement {
    std::shared_ptr<Expression> condition;
    std::vector<Case> cases;
    
    SwitchStmt(std::shared_ptr<Expression> c, const std::vector<Case>& cs)
        : condition(c), cases(cs) {}
};

struct ExpressionStmt : Statement {
    std::shared_ptr<Expression> expression;
    ExpressionStmt(std::shared_ptr<Expression> e) : expression(e) {}
};

struct ThrowStmt : Statement {
    std::shared_ptr<Expression> expression;
    ThrowStmt(std::shared_ptr<Expression> e) : expression(e) {}
};

struct TryStmt : Statement {
    std::shared_ptr<BlockStmt> tryBlock;
    std::string catchVar;  // Variable name for caught exception
    std::shared_ptr<BlockStmt> catchBlock;
    std::shared_ptr<BlockStmt> finallyBlock;  // Optional
    
    TryStmt(std::shared_ptr<BlockStmt> t, const std::string& cv, std::shared_ptr<BlockStmt> c, std::shared_ptr<BlockStmt> f = nullptr)
        : tryBlock(t), catchVar(cv), catchBlock(c), finallyBlock(f) {}
};

struct Parameter {
    std::string name; 
    std::string type; 
    std::shared_ptr<Expression> defaultValue; 
    bool isRest = false; 
};

struct FunctionDeclaration : ASTNode {
    std::string name;
    std::vector<Parameter> params;
    std::string returnType;
    std::shared_ptr<BlockStmt> body;
    bool isAsync = false;
    
    FunctionDeclaration(const std::string& n, const std::vector<Parameter>& p, const std::string& ret, std::shared_ptr<BlockStmt> b, bool async = false) 
        : name(n), params(p), returnType(ret), body(b), isAsync(async) {}
};

struct ProtocolDeclaration : ASTNode {
    struct Method {
        std::string name;
        std::vector<Parameter> params;
        std::string returnType;
    };
    std::string name;
    std::vector<Method> methods;
    ProtocolDeclaration(const std::string& n, const std::vector<Method>& m) : name(n), methods(m) {}
};

struct ExtensionDeclaration : ASTNode {
    std::string targetType; // Class or Protocol name
    std::vector<std::shared_ptr<FunctionDeclaration>> functions;

    ExtensionDeclaration(const std::string& target, const std::vector<std::shared_ptr<FunctionDeclaration>>& funcs)
        : targetType(target), functions(funcs) {}
};

struct ClassDeclaration : ASTNode {
    enum class AccessModifier { Public, Private, Protected };
    
    struct Member { 
        std::string name; 
        std::string type; 
        AccessModifier access = AccessModifier::Public;
        bool isStatic = false;
        std::shared_ptr<Expression> initializer = nullptr; // Inline init support
    };
    
    struct Method {
        std::shared_ptr<FunctionDeclaration> func;
        AccessModifier access = AccessModifier::Public;
        bool isStatic = false;
        bool isAbstract = false;
    };
    
    struct Getter {
        std::string name;
        std::string returnType;
        std::shared_ptr<BlockStmt> body;
        AccessModifier access = AccessModifier::Public;
    };
    
    struct Setter {
        std::string name;
        std::string paramName;
        std::string paramType;
        std::shared_ptr<BlockStmt> body;
        AccessModifier access = AccessModifier::Public;
    };

    std::string name;
    std::string parentName;
    bool isAbstract = false;
    std::vector<std::string> implementedProtocols; // Interfaces
    std::vector<Member> members;
    std::vector<Method> methods;
    std::vector<Getter> getters;
    std::vector<Setter> setters;
    std::shared_ptr<FunctionDeclaration> constructor;

    ClassDeclaration(const std::string& n, const std::vector<Member>& mems, 
                     const std::vector<Method>& meths,
                     std::shared_ptr<FunctionDeclaration> ctor,
                     const std::string& parent = "",
                     const std::vector<std::string>& impls = {},
                     const std::vector<Getter>& gets = {},
                     const std::vector<Setter>& sets = {},
                     bool abstract = false) 
        : name(n), members(mems), methods(meths), constructor(ctor), parentName(parent), 
          implementedProtocols(impls), getters(gets), setters(sets), isAbstract(abstract) {}
};

struct SuperExpr : Expression {};

struct EnumMember {
    std::string name;
    std::shared_ptr<Expression> value; // Optional
};

struct EnumDeclaration : Statement {
    std::string name;
    std::vector<EnumMember> members;
    EnumDeclaration(const std::string& n, const std::vector<EnumMember>& m) : name(n), members(m) {}
};

// Module system: import { x, y } from "./file.tx" or import x from "./file.tx"
struct ImportDecl : Statement {
    std::vector<std::string> names;  // Named imports, or single for default
    std::string source;              // Module path
    bool isDefault;                  // true for: import x from "..."
    
    ImportDecl(const std::vector<std::string>& n, const std::string& s, bool def = false)
        : names(n), source(s), isDefault(def) {}
};

// Module system: export function/const/default
struct ExportDecl : Statement {
    std::shared_ptr<ASTNode> declaration;  // The exported declaration (function, var, class)
    bool isDefault;                         // true for: export default ...
    
    ExportDecl(std::shared_ptr<ASTNode> decl, bool def = false)
        : declaration(decl), isDefault(def) {}
};

struct Program : ASTNode {
    std::vector<std::shared_ptr<ASTNode>> statements; // Renamed from globals but type is ASTNode
    Program() = default;
    Program(const std::vector<std::shared_ptr<ASTNode>>& s) : statements(s) {}
};

} // namespace tejx
