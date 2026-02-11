#pragma once

#include "MIR.h"
#include <string>
#include <vector>
#include <sstream>
#include <unordered_map>

namespace tejx {

class MIRCodeGen {
public:
    std::string generate(const std::vector<std::shared_ptr<MIRFunction>>& functions);

private:
    std::stringstream buffer;
    std::stringstream globalBuffer;
    
    // Map MIR variable/constant names to LLVM IR values (registers or constants)
    std::unordered_map<std::string, std::string> valueMap;
    int tempCounter = 0;
    int labelCounter = 0;

    void emit(const std::string& code);
    void emitLine(const std::string& code);
    
    void genFunction(std::shared_ptr<MIRFunction> func);
    void genBasicBlock(std::shared_ptr<BasicBlock> bb);
    void genInstruction(std::shared_ptr<MIRInstruction> inst);
    
    std::string resolveValue(std::shared_ptr<MIRValue> val);
    std::string getLLVMType(std::shared_ptr<Type> type);
};

} // namespace tejx
