#pragma once

#include "HIR.h"
#include <memory>
#include <string>
#include <vector>
#include <map>

namespace tejx {

class TypeChecker {
public:
    void check(std::shared_ptr<HIRFunction> func);
    std::vector<std::string> errors;

private:
   void error(const std::string& msg);
    void checkStatement(std::shared_ptr<HIRStatement> stmt);
    std::shared_ptr<Type> checkExpression(std::shared_ptr<HIRExpression> expr);

    // Symbol Table
    std::vector<std::map<std::string, std::shared_ptr<Type>>> scopes;
    void enterScope();
    void exitScope();
    void declare(const std::string& name, std::shared_ptr<Type> type);
    std::shared_ptr<Type> lookup(const std::string& name);
};

} // namespace tejx
