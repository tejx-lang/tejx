#pragma once

#include "HIR.h"
#include "MIR.h"
#include <memory>
#include <string>

namespace tejx {

class MIRLowering {
public:
    std::shared_ptr<MIRFunction> lower(std::shared_ptr<HIRFunction> hirFunc);

private:
    std::shared_ptr<BasicBlock> currentBlock;
    std::shared_ptr<MIRFunction> currentFunction;
    int tempCounter = 0;
    int blockCounter = 0;

    void emit(std::shared_ptr<MIRInstruction> inst);
    std::shared_ptr<MIRValue> lowerExpression(std::shared_ptr<HIRExpression> expr);
    void lowerStatement(std::shared_ptr<HIRStatement> stmt);
    
    std::shared_ptr<MIRVariable> newTemp(std::shared_ptr<Type> type);
    std::shared_ptr<BasicBlock> newBlock(const std::string& prefix = "bb");
};

} // namespace tejx
