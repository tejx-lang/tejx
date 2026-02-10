#pragma once

#include "AST.h"
#include "HIR.h" // HIR definitions
#include <memory>
#include <string>
#include <vector>

namespace tejx {

class Lowering {
public:
    std::shared_ptr<HIRFunction> lower(std::shared_ptr<Program> program);
    std::shared_ptr<HIRStatement> lowerStatement(std::shared_ptr<Statement> stmt);
    std::shared_ptr<HIRExpression> lowerExpression(std::shared_ptr<Expression> expr);

private:
   // Helpers
   std::shared_ptr<Type> resolveType(const std::string& typeName);
};

} // namespace tejx
