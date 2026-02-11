#pragma once

#include "MIR.h"
#include <memory>
#include <string>
#include <vector>
#include <map>
#include <set>

namespace tejx {

class BorrowChecker {
public:
    void check(std::shared_ptr<MIRFunction> func);
    std::vector<std::string> errors;

private:
   void error(const std::string& msg);
   
   enum class VarState {
       Uninitialized,
       Live,
       Moved
   };
   
   // State of variables at the START of each block
   std::map<std::shared_ptr<BasicBlock>, std::map<std::string, VarState>> blockStates;
   
   void checkBlock(std::shared_ptr<BasicBlock> block, std::map<std::string, VarState> currentState);
};

} // namespace tejx
