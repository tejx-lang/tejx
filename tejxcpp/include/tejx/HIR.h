#pragma once

#include "Type.h"
#include "AST.h" // Reuse TokenType
#include <vector>
#include <memory>
#include <string>

namespace tejx {

struct HIRNode {
    virtual ~HIRNode() = default;
    std::shared_ptr<Type> type; // Inferred/Checked type
};

struct HIRExpression : HIRNode {};

struct HIRStatement : HIRNode {};

// --- Expressions ---

struct HIRLiteral : HIRExpression {
    std::string value; // For now keeping simple string rep
    HIRLiteral(const std::string& v, std::shared_ptr<Type> t) : value(v) { type = t; }
};

struct HIRVariable : HIRExpression {
    std::string name;
    // We could store a pointer to the definition (HIRVarDecl) here for resolution
    HIRVariable(const std::string& n, std::shared_ptr<Type> t) : name(n) { type = t; }
};

struct HIRBinaryExpr : HIRExpression {
    std::shared_ptr<HIRExpression> left;
    TokenType op;
    std::shared_ptr<HIRExpression> right;
    HIRBinaryExpr(std::shared_ptr<HIRExpression> l, TokenType o, std::shared_ptr<HIRExpression> r, std::shared_ptr<Type> t)
        : left(l), op(o), right(r) { type = t; }
};

struct HIRCall : HIRExpression {
    std::string callee;
    std::vector<std::shared_ptr<HIRExpression>> args;
    HIRCall(const std::string& c, const std::vector<std::shared_ptr<HIRExpression>>& a, std::shared_ptr<Type> t) 
        : callee(c), args(a) { type = t; }
};

struct HIRNewExpr : HIRExpression {
    std::string className;
    std::vector<std::shared_ptr<HIRExpression>> args;
    HIRNewExpr(const std::string& c, const std::vector<std::shared_ptr<HIRExpression>>& a) 
        : className(c), args(a) {}
};

struct HIRAssignment : HIRExpression {
    std::shared_ptr<HIRExpression> target;
    std::shared_ptr<HIRExpression> value;
    HIRAssignment(std::shared_ptr<HIRExpression> t, std::shared_ptr<HIRExpression> v, std::shared_ptr<Type> ty) 
        : target(t), value(v) { type = ty; }
};

// ... Add other expression types as needed (Call, MemberAccess, etc)

// --- Statements ---

struct HIRExpressionStmt : HIRStatement {
    std::shared_ptr<HIRExpression> expr;
    HIRExpressionStmt(std::shared_ptr<HIRExpression> e) : expr(e) {}
};

struct HIRBlock : HIRStatement {
    std::vector<std::shared_ptr<HIRStatement>> statements;
};

struct HIRVarDecl : HIRStatement {
    std::string name;
    std::shared_ptr<HIRExpression> initializer;
    bool isConst;
    HIRVarDecl(const std::string& n, std::shared_ptr<HIRExpression> init, std::shared_ptr<Type> t, bool c)
        : name(n), initializer(init), isConst(c) { type = t; }
};

struct HIRFunction : HIRStatement {
    std::string name;
    std::vector<std::pair<std::string, std::shared_ptr<Type>>> params;
    std::shared_ptr<Type> returnType;
    std::shared_ptr<HIRBlock> body;
    HIRFunction(const std::string& n, const std::vector<std::pair<std::string, std::shared_ptr<Type>>>& p, 
                std::shared_ptr<Type> ret, std::shared_ptr<HIRBlock> b)
        : name(n), params(p), returnType(ret), body(b) {}
};

struct HIRReturn : HIRStatement {
    std::shared_ptr<HIRExpression> value;
    HIRReturn(std::shared_ptr<HIRExpression> v) : value(v) {}
};

// Unified control flow
struct HIRLoop : HIRStatement {
    std::shared_ptr<HIRExpression> condition;
    std::shared_ptr<HIRBlock> body;
    std::shared_ptr<HIRStatement> increment; // Optional (for C-style for loops)
    bool isDoWhile; // If true, condition is checked after body

    HIRLoop(std::shared_ptr<HIRExpression> c, std::shared_ptr<HIRBlock> b, std::shared_ptr<HIRStatement> inc = nullptr, bool dw = false)
        : condition(c), body(b), increment(inc), isDoWhile(dw) {}
};

struct HIRIf : HIRStatement {
    std::shared_ptr<HIRExpression> condition;
    std::shared_ptr<HIRStatement> thenBranch;
    std::shared_ptr<HIRStatement> elseBranch;
    HIRIf(std::shared_ptr<HIRExpression> c, std::shared_ptr<HIRStatement> t, std::shared_ptr<HIRStatement> e)
        : condition(c), thenBranch(t), elseBranch(e) {}
};

} // namespace tejx
