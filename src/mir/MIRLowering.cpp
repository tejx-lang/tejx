#include "tejx/MIRLowering.h"
#include <iostream>

namespace tejx {

std::shared_ptr<MIRFunction> MIRLowering::lower(std::shared_ptr<HIRFunction> hirFunc) {
    currentFunction = std::make_shared<MIRFunction>(hirFunc->name);
    currentBlock = newBlock("entry");
    currentFunction->entryBlock = currentBlock;
    
    lowerStatement(hirFunc->body);
    
    // Ensure terminated if falling off end
    // (simplified, assume void return for now if missing)
    if (currentBlock->instructions.empty() || 
       (currentBlock->instructions.back()->opcode != MIROpcode::Return && 
        currentBlock->instructions.back()->opcode != MIROpcode::Jump && 
        currentBlock->instructions.back()->opcode != MIROpcode::Branch)) {
        emit(std::make_shared<MIRReturn>(nullptr));
    }
    
    return currentFunction;
}

std::shared_ptr<BasicBlock> MIRLowering::newBlock(const std::string& prefix) {
    auto bb = std::make_shared<BasicBlock>(prefix + "_" + std::to_string(blockCounter++));
    currentFunction->blocks.push_back(bb);
    return bb;
}

std::shared_ptr<MIRVariable> MIRLowering::newTemp(std::shared_ptr<Type> type) {
    return std::make_shared<MIRVariable>("t" + std::to_string(tempCounter++), type);
}

void MIRLowering::emit(std::shared_ptr<MIRInstruction> inst) {
    currentBlock->addInstruction(inst);
}

void MIRLowering::lowerStatement(std::shared_ptr<HIRStatement> stmt) {
    if (!stmt) return;

    if (auto block = std::dynamic_pointer_cast<HIRBlock>(stmt)) {
        for (const auto& s : block->statements) {
            lowerStatement(s);
        }
    }
    else if (auto varDecl = std::dynamic_pointer_cast<HIRVarDecl>(stmt)) {
        if (varDecl->initializer) {
            auto src = lowerExpression(varDecl->initializer);
            auto dst = std::make_shared<MIRVariable>(varDecl->name, varDecl->type);
            emit(std::make_shared<MIRMove>(dst, src));
        } else {
             // Maybe init to default/zero?
        }
    }
    else if (auto loop = std::dynamic_pointer_cast<HIRLoop>(stmt)) {
        // While(cond) { body }
        // 
        //   jump loopHeader
        // loopHeader:
        //   condVal = eval(cond)
        //   branch condVal ? loopBody : loopExit
        // loopBody:
        //   ...
        //   jump loopHeader
        // loopExit:
        
        auto loopHeader = newBlock("loop_header");
        auto loopBody = newBlock("loop_body");
        auto loopExit = newBlock("loop_exit");
        
        emit(std::make_shared<MIRJump>(loopHeader));
        
        currentBlock = loopHeader;
        auto condVal = lowerExpression(loop->condition);
        emit(std::make_shared<MIRBranch>(condVal, loopBody, loopExit));
        
        currentBlock = loopBody;
        lowerStatement(loop->body);
        if (loop->increment) {
            lowerStatement(loop->increment);
        }
        emit(std::make_shared<MIRJump>(loopHeader));
        
        currentBlock = loopExit;
    }
    else if (auto ret = std::dynamic_pointer_cast<HIRReturn>(stmt)) {
        auto val = ret->value ? lowerExpression(ret->value) : nullptr;
        emit(std::make_shared<MIRReturn>(val));
    }
    else if (auto exprStmt = std::dynamic_pointer_cast<HIRExpressionStmt>(stmt)) {
        lowerExpression(exprStmt->expr);
    }
}

std::shared_ptr<MIRValue> MIRLowering::lowerExpression(std::shared_ptr<HIRExpression> expr) {
    if (auto lit = std::dynamic_pointer_cast<HIRLiteral>(expr)) {
        return std::make_shared<MIRConstant>(lit->value, lit->type);
    }
    if (auto var = std::dynamic_pointer_cast<HIRVariable>(expr)) {
        return std::make_shared<MIRVariable>(var->name, var->type);
    }
    if (auto newExpr = std::dynamic_pointer_cast<HIRNewExpr>(expr)) {
        // Simplified: Treat constructor as a Call or just return a temporary
        // For borrow checker testing, we just need a value with ClassType.
        // Let's create a MIRConstant or specialized instruction?
        // Let's use LoadConst or Call.
        // A "new" is technically an allocation.
        // For now, let's treat it as a Call to "new ClassName"
        // returning a ClassType variable.
        return std::make_shared<MIRConstant>("new " + newExpr->className, std::make_shared<ClassType>(newExpr->className));
    }
    if (auto bin = std::dynamic_pointer_cast<HIRBinaryExpr>(expr)) {
        auto left = lowerExpression(bin->left);
        auto right = lowerExpression(bin->right);
        auto temp = newTemp(bin->type);
        emit(std::make_shared<MIRBinary>(temp, left, bin->op, right));
        return temp;
    }

    if (auto assign = std::dynamic_pointer_cast<HIRAssignment>(expr)) {
        auto val = lowerExpression(assign->value);
        auto dst = lowerExpression(assign->target);
        if (auto dstVar = std::dynamic_pointer_cast<MIRVariable>(dst)) {
            emit(std::make_shared<MIRMove>(dstVar, val));
        }

        return val;
    }
    if (auto call = std::dynamic_pointer_cast<HIRCall>(expr)) {
        // Handle console.log specially or emit generic call
        if (call->callee == "console.log" || call->callee == "printf") {
             // For printf: first arg is format string?
             // console.log(args...) -> we might need multiple calls or a loop
             // For MVP, just printing one thing?
             // Let's implement variadic print helper or just loop args.
             // Simpler: Emit separate calls for each arg with "%lld\n" or "%s\n"
             // But MIRCall needs defined args.
             // We'll emit a special MIRPrint instruction? Or Call to "printf".
             
             // If multiple args, print each with space?
             // Since we have format string "@.fmt_d" (%lld\n), we can use that for numbers.
             // FOR THIS DEMO: Just print the first argument or all as ints.
             for (auto& argExpr : call->args) {
                 auto argVal = lowerExpression(argExpr);
                 // We need to call printf("%lld\n", arg) or printf("%s\n", arg)
                 // But we only have one MIRCall instruction.
                 // Let's manually create MIRCall to "printf".
                 // This is technically hacking MIRLowering to emit multiple instructions for one HIR Call.
                 
                 // How to differentiate int vs string format?
                 // We don't have type info at runtime easily in this textual IR.
                 // But MIRCodeGen defaults to %lld. 
                 // If string, resolveValue returns ptr.
                 // We need distinct format strings for %s vs %lld.
                 // For now, let's assume MIRCodeGen handles it?
                 // No, MIRCodeGen sees "call printf(arg)".
                 // We need to pass Format String as first arg.
                 
                 std::vector<std::shared_ptr<MIRValue>> printArgs;
                 // Add format string?
                 // Let's make "print" a special intrinsic in MIRCodeGen?
                 // Yes, simpler.
                 
                 // emit(MIRIntrinsic("print", argVal));
                 // But we don't have MIRIntrinsic.
                 // Let's construct a MIRCall to "print_int" or "print_str"?
                 // Let's use "printf" and let MIRCodeGen inject the format string if only 1 arg provided?
             }
             // Actually, simplest fix: proper MIRCall with args.
             std::vector<std::shared_ptr<MIRValue>> mirArgs;
             for(auto& a : call->args) mirArgs.push_back(lowerExpression(a));
             
             auto temp = newTemp(call->type);
             emit(std::make_shared<MIRCall>(temp, call->callee, mirArgs));
             return temp;
        }
        
        std::vector<std::shared_ptr<MIRValue>> mirArgs;
        for(auto& a : call->args) mirArgs.push_back(lowerExpression(a));
        auto temp = newTemp(call->type);
        emit(std::make_shared<MIRCall>(temp, call->callee, mirArgs));
        return temp;
    }
    return nullptr;
}

} // namespace tejx
