#include "tejx/BorrowChecker.h"
#include <iostream>

namespace tejx {

void BorrowChecker::error(const std::string& msg) {
    std::cerr << msg << std::endl;
    errors.push_back(msg);
}

void BorrowChecker::check(std::shared_ptr<MIRFunction> func) {
    errors.clear();
    blockStates.clear();
    
    // Initial state: all args are Live (omitted for now), locals Uninitialized
    std::map<std::string, VarState> entryState;
    checkBlock(func->entryBlock, entryState);
}

void BorrowChecker::checkBlock(std::shared_ptr<BasicBlock> block, std::map<std::string, VarState> currentState) {
    if (!block) return;
    
    // Cycle detection / convergence check (simplified: if processed with same/superset state, stop)
    // For now, simple visited check to avoid infinite recursion on loops
    if (blockStates.count(block)) return; // REAL borrowing checker needs fix-point iteration
    blockStates[block] = currentState;

    for (const auto& inst : block->instructions) {
        if (inst->opcode == MIROpcode::Move) {
            auto move = std::dynamic_pointer_cast<MIRMove>(inst);
            
            // Check Source (Use/Move)
            if (auto srcVar = std::dynamic_pointer_cast<MIRVariable>(move->src)) {
                if (currentState.count(srcVar->name) && currentState[srcVar->name] == VarState::Moved) {
                    error("Borrow Error: Use of moved variable '" + srcVar->name + "'");
                }
                
                // If type is not primitive, mark as Moved (Ownership transfer)
                // Assuming everything non-primitive is "Class" type or similar
                // Primitives (number/bool) are Copy, others are Move?
                // check type... for safely, let's say only "ClassType" is moved.
                if (srcVar->type && std::dynamic_pointer_cast<ClassType>(srcVar->type)) {
                     currentState[srcVar->name] = VarState::Moved;
                     // std::cout << "Moved " << srcVar->name << std::endl;
                }
            }
            
            // Define Dest
            if (move->dst) {
                currentState[move->dst->name] = VarState::Live;
                // std::cout << "Initialized " << move->dst->name << std::endl;
            }
        }
        else if (inst->opcode == MIROpcode::BinaryOp) {
             auto bin = std::dynamic_pointer_cast<MIRBinary>(inst);
             // Check operands
             if (auto l = std::dynamic_pointer_cast<MIRVariable>(bin->left)) {
                 if (currentState[l->name] == VarState::Moved) error("Borrow Error: Use of moved variable '" + l->name + "'");
             }
             if (auto r = std::dynamic_pointer_cast<MIRVariable>(bin->right)) {
                 if (currentState[r->name] == VarState::Moved) error("Borrow Error: Use of moved variable '" + r->name + "'");
             }
             // Define dst
             if (bin->dst) currentState[bin->dst->name] = VarState::Live;
        }
        else if (inst->opcode == MIROpcode::Branch) {
            auto br = std::dynamic_pointer_cast<MIRBranch>(inst);
            // Process successors with current state
            checkBlock(br->trueTarget, currentState);
            checkBlock(br->falseTarget, currentState);
            return; // Terminate this path
        }
        else if (inst->opcode == MIROpcode::Jump) {
            auto jmp = std::dynamic_pointer_cast<MIRJump>(inst);
            checkBlock(jmp->target, currentState);
            return;
        }
        else if (inst->opcode == MIROpcode::Return) {
            auto ret = std::dynamic_pointer_cast<MIRReturn>(inst);
            if (ret->value) {
                if (auto v = std::dynamic_pointer_cast<MIRVariable>(ret->value)) {
                    if (currentState[v->name] == VarState::Moved) error("Borrow Error: Return of moved variable '" + v->name + "'");
                }
            }
            return;
        }
    }
}

} // namespace tejx
