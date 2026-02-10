#include "tejx/TypeChecker.h"
#include <iostream>
#include <stdexcept>

namespace tejx {

void TypeChecker::error(const std::string& msg) {
    std::cerr << msg << std::endl;
    errors.push_back(msg);
}

void TypeChecker::check(std::shared_ptr<HIRFunction> func) {
    errors.clear();
    enterScope();
    // Declare args
    // ...
    checkStatement(func->body);
    exitScope();
}

void TypeChecker::checkStatement(std::shared_ptr<HIRStatement> stmt) {
    if (!stmt) return;

    if (auto block = std::dynamic_pointer_cast<HIRBlock>(stmt)) {
        enterScope();
        for (const auto& s : block->statements) {
            checkStatement(s);
        }
        exitScope();
    }
    else if (auto varDecl = std::dynamic_pointer_cast<HIRVarDecl>(stmt)) {
        if (varDecl->initializer) {
            auto initType = checkExpression(varDecl->initializer);
            // Type Inference: If decl type is Any, infer from initializer
            if (std::dynamic_pointer_cast<AnyType>(varDecl->type) && !std::dynamic_pointer_cast<AnyType>(initType)) {
                varDecl->type = initType;
            }
        }
        declare(varDecl->name, varDecl->type);
    }
    else if (auto loop = std::dynamic_pointer_cast<HIRLoop>(stmt)) {
        auto condType = checkExpression(loop->condition);
        if (!std::dynamic_pointer_cast<PrimitiveType>(condType) || std::dynamic_pointer_cast<PrimitiveType>(condType)->name != "boolean") {
            // Allow numbers as boolean for now (C-style) logic widely used
             if (std::dynamic_pointer_cast<PrimitiveType>(condType) && std::dynamic_pointer_cast<PrimitiveType>(condType)->name == "number") {
                 // warning
             } else {
                 // std::cerr << "Type Error: Loop condition must be boolean." << std::endl;
             }
        }
        checkStatement(loop->body);
    }
}

std::shared_ptr<Type> TypeChecker::checkExpression(std::shared_ptr<HIRExpression> expr) {
    if (!expr) return std::make_shared<VoidType>();

    if (auto lit = std::dynamic_pointer_cast<HIRLiteral>(expr)) {
        return lit->type;
    }
    
    if (auto var = std::dynamic_pointer_cast<HIRVariable>(expr)) {
        auto type = lookup(var->name);
        if (!type) {
            error("Semantic Error: Undeclared variable '" + var->name + "'");
            return std::make_shared<AnyType>();
        }
        var->type = type; // Annotate HIR
        return type;
    }
    
    if (auto bin = std::dynamic_pointer_cast<HIRBinaryExpr>(expr)) {
        auto lType = checkExpression(bin->left);
        auto rType = checkExpression(bin->right);
        
        // Infer return type based on op (simplified)
        // Infer return type based on op (simplified)
        if (bin->op == TokenType::Plus) {
            // number + number = number
             auto pTypeL = std::dynamic_pointer_cast<PrimitiveType>(lType);
             auto pTypeR = std::dynamic_pointer_cast<PrimitiveType>(rType);
             if (pTypeL && pTypeR && pTypeL->name == "number" && pTypeR->name == "number") {
                 bin->type = std::make_shared<PrimitiveType>("number");
                 return bin->type;
             }
             // string + string = string
        }
        // logical ops -> boolean
        
        return std::make_shared<AnyType>(); // Fallback
    }
    
    if (auto newExpr = std::dynamic_pointer_cast<HIRNewExpr>(expr)) {
        return std::make_shared<ClassType>(newExpr->className);
    }

    return std::make_shared<AnyType>();
}

void TypeChecker::enterScope() {
    scopes.push_back({});
}

void TypeChecker::exitScope() {
    scopes.pop_back();
}

void TypeChecker::declare(const std::string& name, std::shared_ptr<Type> type) {
    if (scopes.empty()) return;
    scopes.back()[name] = type;
}

std::shared_ptr<Type> TypeChecker::lookup(const std::string& name) {
    for (auto it = scopes.rbegin(); it != scopes.rend(); ++it) {
        if (it->count(name)) return (*it)[name];
    }
    return nullptr;
}

} // namespace tejx
