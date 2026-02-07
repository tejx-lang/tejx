#include "tejx/Interpreter.h"
#include <iostream>
#include <cmath>

namespace tejx {

std::string Value::toString() const {
    switch (type) {
        case Number: {
             // Remove trailing zeros if integer
             std::string s = std::to_string(numVal);
             s.erase(s.find_last_not_of('0') + 1, std::string::npos);
             if (s.back() == '.') s.pop_back();
             return s;
        }
        case String: return strVal;
        case Boolean: return boolVal ? "true" : "false";
        case Function: return "<function>";
        default: return "undefined";
    }
}

// Environment
void Environment::define(const std::string& name, Value value) {
    values[name] = value;
}

Value Environment::get(const std::string& name) {
    if (values.count(name)) return values[name];
    if (enclosing) return enclosing->get(name);
    std::cerr << "Undefined variable '" << name << "'." << std::endl;
    exit(1);
}

void Environment::assign(const std::string& name, Value value) {
    if (values.count(name)) {
        values[name] = value;
        return;
    }
    if (enclosing) {
        enclosing->assign(name, value);
        return;
    }
    std::cerr << "Undefined variable '" << name << "'." << std::endl;
    exit(1);
}

// Interpreter
void Interpreter::execute(std::shared_ptr<Program> program) {
    globalEnv = new Environment();
    currentEnv = globalEnv;
    
    // Define global vars or functions
    for (auto& node : program->globals) {
        if (auto func = std::dynamic_pointer_cast<FunctionDeclaration>(node)) {
            Value val;
            val.type = Value::Function;
            val.funcVal = func;
            currentEnv->define(func->name, val);
        } else if (auto stmt = std::dynamic_pointer_cast<Statement>(node)) {
            execute(stmt);
        }
    }
}

void Interpreter::execute(std::shared_ptr<Statement> stmt) {
    if (auto block = std::dynamic_pointer_cast<BlockStmt>(stmt)) {
        Environment* newEnv = new Environment(currentEnv);
        executeBlock(block, newEnv);
        delete newEnv; // Clean up
    }
    else if (auto varDecl = std::dynamic_pointer_cast<VarDeclaration>(stmt)) {
        Value val;
        val.type = Value::Void;
        if (varDecl->initializer) {
            val = evaluate(varDecl->initializer);
        }
        currentEnv->define(varDecl->name, val);
    }
    else if (auto exprStmt = std::dynamic_pointer_cast<ExpressionStmt>(stmt)) {
        evaluate(exprStmt->expression);
    }
    else if (auto ifStmt = std::dynamic_pointer_cast<IfStmt>(stmt)) {
        Value cond = evaluate(ifStmt->condition);
        if (cond.type == Value::Boolean && cond.boolVal) {
            execute(ifStmt->thenBranch);
        } else if (ifStmt->elseBranch) {
            execute(ifStmt->elseBranch);
        }
    }
    else if (auto whileStmt = std::dynamic_pointer_cast<WhileStmt>(stmt)) {
        while (true) {
            Value cond = evaluate(whileStmt->condition);
            if (cond.type == Value::Boolean && !cond.boolVal) break;
            execute(whileStmt->body);
        }
    }
    else if (auto returnStmt = std::dynamic_pointer_cast<ReturnStmt>(stmt)) {
        Value val;
        val.type = Value::Void;
        if (returnStmt->value) val = evaluate(returnStmt->value);
        throw val; // Using exceptions for return (simple hack)
    }
}

void Interpreter::executeBlock(std::shared_ptr<BlockStmt> block, Environment* env) {
    Environment* previous = currentEnv;
    currentEnv = env;
    
    try {
        for (auto& stmt : block->statements) {
            execute(stmt);
        }
    } catch (Value v) {
        currentEnv = previous;
        throw v;
    }
    
    currentEnv = previous;
}

Value Interpreter::evaluate(std::shared_ptr<Expression> expr) {
    if (auto num = std::dynamic_pointer_cast<NumberLiteral>(expr)) {
        return Value{Value::Number, num->value};
    }
    if (auto str = std::dynamic_pointer_cast<StringLiteral>(expr)) {
        Value v; v.type = Value::String; v.strVal = str->value; return v;
    }
    if (auto b = std::dynamic_pointer_cast<BooleanLiteral>(expr)) {
        Value v; v.type = Value::Boolean; v.boolVal = b->value; return v;
    }
    if (auto id = std::dynamic_pointer_cast<Identifier>(expr)) {
        return currentEnv->get(id->name);
    }
    if (auto assign = std::dynamic_pointer_cast<AssignmentExpr>(expr)) {
        if (auto id = std::dynamic_pointer_cast<Identifier>(assign->target)) {
            Value val = evaluate(assign->value);
            currentEnv->assign(id->name, val);
            return val;
        }
        std::cerr << "Interpreter does not support complex assignment (e.g. arrays) yet." << std::endl;
        exit(1);
    }
    if (auto binary = std::dynamic_pointer_cast<BinaryExpr>(expr)) {
        Value left = evaluate(binary->left);
        Value right = evaluate(binary->right);
        
        // Simpification: Assume numbers
        Value result;
        if (binary->op == TokenType::Plus) {
            // Check for string concat
            if (left.type == Value::String || right.type == Value::String) {
                result.type = Value::String;
                result.strVal = left.toString() + right.toString();
            } else {
                result.type = Value::Number;
                result.numVal = left.numVal + right.numVal;
            }
        } 
        else if (binary->op == TokenType::Minus) { result.type = Value::Number; result.numVal = left.numVal - right.numVal; }
        else if (binary->op == TokenType::Star) { result.type = Value::Number; result.numVal = left.numVal * right.numVal; }
        else if (binary->op == TokenType::Slash) { result.type = Value::Number; result.numVal = left.numVal / right.numVal; }
        else if (binary->op == TokenType::Modulo) { result.type = Value::Number; result.numVal = std::fmod(left.numVal, right.numVal); }
        else if (binary->op == TokenType::Less) { result.type = Value::Boolean; result.boolVal = left.numVal < right.numVal; }
        else if (binary->op == TokenType::Greater) { result.type = Value::Boolean; result.boolVal = left.numVal > right.numVal; }
        else if (binary->op == TokenType::LessEqual) { result.type = Value::Boolean; result.boolVal = left.numVal <= right.numVal; }
        else if (binary->op == TokenType::GreaterEqual) { result.type = Value::Boolean; result.boolVal = left.numVal >= right.numVal; }
        else if (binary->op == TokenType::EqualEqual) { 
             result.type = Value::Boolean; 
             // Simplistic equality
             if(left.type != right.type) result.boolVal = false;
             else if(left.type == Value::Number) result.boolVal = left.numVal == right.numVal;
             else if(left.type == Value::Boolean) result.boolVal = left.boolVal == right.boolVal;
             else result.boolVal = false;
        }
        
        // Assignment ?? Not implemented in Lexer properly as separate precedence yet 
        // Need to handle '=' in parser. Parser treats '=' as binary op but it needs lvalue.
        // For now, let's just handle it here if we assume parser produces it?
        // Wait, Lexer produces 'Equals'.
        // My parser loops 'parseEquality' ... 'parseTerm'. No 'parseAssignment'.
        // So 'x = 5' is not parsed.
        // User example has 'let i = 0', 'i = i + 1'.
        // I need to add Assignment expression in Parser + Interpreter.
        
        return result;
    }
    
    // Ugly Hack for Assignment: Parser currently doesn't create AssignmentExpr.
    // It likely treats it as BinaryExpr with 'Equals' token if I allowed it in parser.
    // In Parser.cpp, 'parseEquality' is the top. Need 'parseAssignment' above it.
    
    if (auto call = std::dynamic_pointer_cast<CallExpr>(expr)) {
        // Special case: console.log
        if (call->callee == "console.log") {
            for (auto& arg : call->args) {
                std::cout << evaluate(arg).toString() << std::endl;
            }
            Value v; v.type = Value::Void; return v;
        }
        
        Value func = currentEnv->get(call->callee);
        if (func.type != Value::Function) {
            std::cerr << "Attempt to call non-function." << std::endl;
            exit(1);
        }
        
        std::vector<Value> args;
        for (auto& arg : call->args) args.push_back(evaluate(arg));
        
        if (func.funcVal) return callFunction(func.funcVal, args);
    }
    
    return Value();
}

Value Interpreter::callFunction(std::shared_ptr<FunctionDeclaration> func, const std::vector<Value>& args) {
    Environment* fnEnv = new Environment(globalEnv); // Closure support? Using global as parent for now typical of C.
    
    for (size_t i = 0; i < func->params.size(); ++i) {
        if (i < args.size()) {
            fnEnv->define(func->params[i].name, args[i]);
        }
    }
    
    // We need to restore 'currentEnv' after call
    Environment* previous = currentEnv;
    currentEnv = fnEnv;
    
    Value result;
    result.type = Value::Void;
    
    try {
        executeBlock(func->body, fnEnv);
    } catch (Value retVal) {
        result = retVal;
    }
    
    currentEnv = previous;
    delete fnEnv;
    return result;
}

} // namespace tejx
