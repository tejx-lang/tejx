#include "tejx/Lowering.h"
#include <iostream>

namespace tejx {

// Simple type resolver (mock for now, ideally needs a symbol table or context)
std::shared_ptr<Type> Lowering::resolveType(const std::string& typeName) {
    if (typeName == "number" || typeName == "int" || typeName == "float") return std::make_shared<PrimitiveType>("number");
    if (typeName == "string") return std::make_shared<PrimitiveType>("string");
    if (typeName == "boolean") return std::make_shared<PrimitiveType>("boolean");
    if (typeName == "void") return std::make_shared<VoidType>();
    if (typeName == "any" || typeName.empty()) return std::make_shared<AnyType>();
    return std::make_shared<ClassType>(typeName);
}

std::shared_ptr<HIRFunction> Lowering::lower(std::shared_ptr<Program> program) {
    // For simplicity, we wrap top-level statements in a "main" function
    // In a real compiler, we'd handle functions separately.
    std::vector<std::shared_ptr<HIRStatement>> hirStmts;
    
    for (const auto& node : program->statements) {
        if (auto stmt = std::dynamic_pointer_cast<Statement>(node)) {
            hirStmts.push_back(lowerStatement(stmt));
        } else if (auto funcArr = std::dynamic_pointer_cast<FunctionDeclaration>(node)) {
             // We encountered a function declaration.
             // In this simple lowering, we treat it as a statement that defines a function?
             // Or we just skip it for now as the prompt asked for "loops" mainly.
             // Let's implement main-body code lowering.
        }
    }
    
    auto block = std::make_shared<HIRBlock>();
    block->statements = hirStmts;
    
    // Create a synthetic 'main' function wrapper
    return std::make_shared<HIRFunction>("tejx_main", 
        std::vector<std::pair<std::string, std::shared_ptr<Type>>>(), 
        std::make_shared<PrimitiveType>("number"), 
        block);
}

std::shared_ptr<HIRStatement> Lowering::lowerStatement(std::shared_ptr<Statement> stmt) {
    if (auto block = std::dynamic_pointer_cast<BlockStmt>(stmt)) {
        auto hirBlock = std::make_shared<HIRBlock>();
        for (const auto& s : block->statements) {
            hirBlock->statements.push_back(lowerStatement(s));
        }
        return hirBlock;
    }
    
    if (auto varDecl = std::dynamic_pointer_cast<VarDeclaration>(stmt)) {
        // Assume simple identifier binding for now
        std::string name = "";
        if (auto id = std::dynamic_pointer_cast<IdentifierBinding>(varDecl->pattern)) {
            name = id->name;
        }
        
        auto init = varDecl->initializer ? lowerExpression(varDecl->initializer) : nullptr;
        // Basic type resolution
        auto type = resolveType(varDecl->type);
        return std::make_shared<HIRVarDecl>(name, init, type, varDecl->isConst);
    }
    
    if (auto exprStmt = std::dynamic_pointer_cast<ExpressionStmt>(stmt)) {
        auto expr = lowerExpression(exprStmt->expression);
        return std::make_shared<HIRExpressionStmt>(expr);
    }
    
    // --- Loop Lowering (Desugaring) ---
    
    if (auto whileStmt = std::dynamic_pointer_cast<WhileStmt>(stmt)) {
        // While(c) { b } -> Loop(c, b, null, false)
        auto cond = lowerExpression(whileStmt->condition);
        auto body = std::dynamic_pointer_cast<HIRBlock>(lowerStatement(whileStmt->body));
        if (!body) { // If body wasn't a block, wrap it
            body = std::make_shared<HIRBlock>();
            body->statements.push_back(lowerStatement(whileStmt->body));
        }
        return std::make_shared<HIRLoop>(cond, body, nullptr, false);
    }
    
    if (auto forStmt = std::dynamic_pointer_cast<ForStmt>(stmt)) {
        // For(init; cond; inc) { body }
        // -> Block { init; Loop(cond, body, inc, false) }
        
        auto outerBlock = std::make_shared<HIRBlock>();
        if (forStmt->init) {
            outerBlock->statements.push_back(lowerStatement(forStmt->init));
        }
        
        auto cond = forStmt->condition ? lowerExpression(forStmt->condition) : std::make_shared<HIRLiteral>("true", std::make_shared<PrimitiveType>("boolean"));
        
        auto body = std::dynamic_pointer_cast<HIRBlock>(lowerStatement(forStmt->body));
         if (!body) {
            body = std::make_shared<HIRBlock>();
            body->statements.push_back(lowerStatement(forStmt->body));
        }
        
        // Handle increment
        std::shared_ptr<HIRStatement> incStmt = nullptr;
        if (forStmt->increment) {
            auto incExpr = lowerExpression(forStmt->increment);
            incStmt = std::make_shared<HIRExpressionStmt>(incExpr);
        }
        
        auto loop = std::make_shared<HIRLoop>(cond, body, incStmt, false);
        outerBlock->statements.push_back(loop);
        
        return outerBlock;
    }

    if (auto retStmt = std::dynamic_pointer_cast<ReturnStmt>(stmt)) {
        auto val = retStmt->value ? lowerExpression(retStmt->value) : nullptr;
        return std::make_shared<HIRReturn>(val);
    }
    
    return nullptr;
}

std::shared_ptr<HIRExpression> Lowering::lowerExpression(std::shared_ptr<Expression> expr) {
    if (auto lit = std::dynamic_pointer_cast<NumberLiteral>(expr)) {
        std::string valStr;
        if (lit->value == (long long)lit->value) {
            valStr = std::to_string((long long)lit->value);
        } else {
            valStr = std::to_string(lit->value);
        }
        return std::make_shared<HIRLiteral>(valStr, std::make_shared<PrimitiveType>("number"));
    }
    if (auto str = std::dynamic_pointer_cast<StringLiteral>(expr)) {
        return std::make_shared<HIRLiteral>(str->value, std::make_shared<PrimitiveType>("string"));
    }
    if (auto id = std::dynamic_pointer_cast<Identifier>(expr)) {
        return std::make_shared<HIRVariable>(id->name, nullptr); // Type to be resolved later
    }
    if (auto bin = std::dynamic_pointer_cast<BinaryExpr>(expr)) {
        return std::make_shared<HIRBinaryExpr>(lowerExpression(bin->left), bin->op, lowerExpression(bin->right), nullptr);
    }
    if (auto newExpr = std::dynamic_pointer_cast<NewExpr>(expr)) {
        std::vector<std::shared_ptr<HIRExpression>> args;
        for (const auto& arg : newExpr->args) {
            args.push_back(lowerExpression(arg));
        }
        return std::make_shared<HIRNewExpr>(newExpr->className, args);
    }
    if (auto assign = std::dynamic_pointer_cast<AssignmentExpr>(expr)) {
        auto target = lowerExpression(assign->target);
        auto value = lowerExpression(assign->value);
        return std::make_shared<HIRAssignment>(target, value, value->type);
    }
    if (auto call = std::dynamic_pointer_cast<CallExpr>(expr)) {
        std::vector<std::shared_ptr<HIRExpression>> args;
        for (const auto& arg : call->args) {
             args.push_back(lowerExpression(arg));
        }
        return std::make_shared<HIRCall>(call->callee, args, std::make_shared<VoidType>());
    }
    return nullptr;
}

} // namespace tejx
