#pragma once

#include "AST.h"
#include <unordered_map>
#include <string>
#include <variant>
#include <vector>

namespace tejx {

// Runtime Value
struct Value {
    // std::variant to hold number, string, boolean, etc.
    // For simplicity using struct with type tag
    enum Type { Number, String, Boolean, Void, Function };
    Type type;
    
    double numVal = 0;
    std::string strVal;
    bool boolVal = false;
    
    std::shared_ptr<FunctionDeclaration> funcVal = nullptr;

    std::string toString() const;
};

class Environment {
    std::unordered_map<std::string, Value> values;
    Environment* enclosing;
    
public:
    Environment(Environment* enclosing = nullptr) : enclosing(enclosing) {}
    
    void define(const std::string& name, Value value);
    Value get(const std::string& name);
    void assign(const std::string& name, Value value);
};

class Interpreter {
public:
    void execute(std::shared_ptr<Program> program);

private:
    Environment* globalEnv;
    Environment* currentEnv;
    
    // Helpers
    Value evaluate(std::shared_ptr<Expression> expr);
    void execute(std::shared_ptr<Statement> stmt);
    void executeBlock(std::shared_ptr<BlockStmt> block, Environment* env);
    
    Value callFunction(std::shared_ptr<FunctionDeclaration> func, const std::vector<Value>& args);
};

} // namespace tejx
