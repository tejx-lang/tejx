#include "tejx/MIRCodeGen.h"
#include <iostream>

namespace tejx {

std::string MIRCodeGen::generate(const std::vector<std::shared_ptr<MIRFunction>>& functions) {
    buffer.str("");
    globalBuffer.str("");
    
    // Standard declarations
     // Standard declarations
    globalBuffer << "declare i32 @printf(i8*, ...)\n";
    globalBuffer << "@.fmt_d = private unnamed_addr constant [5 x i8] c\"%lld\\00\"\n";
    globalBuffer << "@.fmt_s = private unnamed_addr constant [3 x i8] c\"%s\\00\"\n";
    globalBuffer << "@.fmt_nl = private unnamed_addr constant [2 x i8] c\"\\0A\\00\"\n";
    globalBuffer << "@.fmt_sp = private unnamed_addr constant [2 x i8] c\" \\00\"\n";
    
    for (const auto& func : functions) {
        genFunction(func);
    }
    
    return globalBuffer.str() + buffer.str();
}



void MIRCodeGen::emit(const std::string& code) {
    buffer << code;
}

void MIRCodeGen::emitLine(const std::string& code) {
    buffer << "  " << code << "\n";
}

std::string MIRCodeGen::getLLVMType(std::shared_ptr<Type> type) {
    // Simplified: everything is i64 for now
    return "i64";
}

void MIRCodeGen::genFunction(std::shared_ptr<MIRFunction> func) {
    // Reset function state
    valueMap.clear();
    tempCounter = 0;
    
    buffer << "define i64 @" << func->name << "() {\n";
    
    // ... (allocas)
    
    // Entry block handling
    buffer << "entry:\n";
    for (const auto& bb : func->blocks) {
        for (const auto& inst : bb->instructions) {
            std::shared_ptr<MIRVariable> destVar = nullptr;
            if (auto move = std::dynamic_pointer_cast<MIRMove>(inst)) destVar = move->dst;
            else if (auto bin = std::dynamic_pointer_cast<MIRBinary>(inst)) destVar = bin->dst;
            else if (auto call = std::dynamic_pointer_cast<MIRCall>(inst)) destVar = call->dst;
            
            if (destVar && valueMap.find(destVar->name) == valueMap.end()) {
                 std::string regName = "%" + destVar->name + "_ptr";
                 emitLine(regName + " = alloca i64");
                 valueMap[destVar->name] = regName;
            }
        }
    }
    
    // Branch to first block
    if (!func->blocks.empty()) {
        emitLine("br label %" + func->blocks[0]->name);
    } else {
        emitLine("ret i64 0"); 
    }

    // Generate blocks
    for (const auto& bb : func->blocks) {
        genBasicBlock(bb);
    }
    
    buffer << "}\n\n";
}

void MIRCodeGen::genBasicBlock(std::shared_ptr<BasicBlock> bb) {
    // We need LLVM labels for blocks.
    // MIR blocks have names like `entry_0`, `loop_header_1`.
    // Valid LLVM labels.
    buffer << bb->name << ":\n";
    
    for (const auto& inst : bb->instructions) {
        genInstruction(inst);
    }
}

void MIRCodeGen::genInstruction(std::shared_ptr<MIRInstruction> inst) {
    if (inst->opcode == MIROpcode::Move) {
        auto move = std::dynamic_pointer_cast<MIRMove>(inst);
        std::string val = resolveValue(move->src);
        std::string ptrval = valueMap[move->dst->name];
        emitLine("store i64 " + val + ", i64* " + ptrval);
    }
    else if (inst->opcode == MIROpcode::BinaryOp) {
        auto bin = std::dynamic_pointer_cast<MIRBinary>(inst);
        std::string l = resolveValue(bin->left);
        std::string r = resolveValue(bin->right);
        std::string tmp = "%tmp" + std::to_string(++tempCounter);
        
        std::string op = "add";
        bool isCompare = false;
        std::string cmpPred = "";

        if (bin->op == TokenType::Minus) op = "sub";
        else if (bin->op == TokenType::Star) op = "mul";
        else if (bin->op == TokenType::Slash) op = "sdiv";
        else if (bin->op == TokenType::Less) { isCompare = true; cmpPred = "slt"; }
        else if (bin->op == TokenType::Greater) { isCompare = true; cmpPred = "sgt"; }
        else if (bin->op == TokenType::EqualEqual) { isCompare = true; cmpPred = "eq"; }
        else if (bin->op == TokenType::BangEqual) { isCompare = true; cmpPred = "ne"; }
        else if (bin->op == TokenType::LessEqual) { isCompare = true; cmpPred = "sle"; }
        else if (bin->op == TokenType::GreaterEqual) { isCompare = true; cmpPred = "sge"; }
        
        if (isCompare) {
            std::string cmpTmp = "%cmp" + std::to_string(++tempCounter);
            emitLine(cmpTmp + " = icmp " + cmpPred + " i64 " + l + ", " + r);
            emitLine(tmp + " = zext i1 " + cmpTmp + " to i64");
        } else {
            emitLine(tmp + " = " + op + " i64 " + l + ", " + r);
        }
        
        // Store result
        std::string ptrval = valueMap[bin->dst->name];
        emitLine("store i64 " + tmp + ", i64* " + ptrval);
    }
    else if (inst->opcode == MIROpcode::Jump) {
         auto jmp = std::dynamic_pointer_cast<MIRJump>(inst);
         emitLine("br label %" + jmp->target->name);
    }
    else if (inst->opcode == MIROpcode::Branch) {
         auto br = std::dynamic_pointer_cast<MIRBranch>(inst);
         std::string cond = resolveValue(br->condition);
         // Compare cond != 0
         std::string cmp = "%cmp" + std::to_string(++tempCounter);
         emitLine(cmp + " = icmp ne i64 " + cond + ", 0");
         emitLine("br i1 " + cmp + ", label %" + br->trueTarget->name + ", label %" + br->falseTarget->name);
    }
    else if (inst->opcode == MIROpcode::Return) {
         auto ret = std::dynamic_pointer_cast<MIRReturn>(inst);
         if (ret->value) {
             std::string val = resolveValue(ret->value);
             emitLine("ret i64 " + val);
         } else {
             emitLine("ret i64 0");
         }
    }
    else if (inst->opcode == MIROpcode::Call) {
         auto call = std::dynamic_pointer_cast<MIRCall>(inst);
         if (call->callee == "console.log" || call->callee == "printf") {
             // Handle console.log specially: pass first arg as is?
             // If multiple args, tough.
             // Assume first arg is compatible with printf.
             if (!call->args.empty()) {
                 for (size_t i = 0; i < call->args.size(); ++i) {
                     std::string argVal = resolveValue(call->args[i]);
                     
                     bool isStr = false;
                     // Check type
                     std::shared_ptr<Type> type = nullptr;
                     if (auto v = std::dynamic_pointer_cast<MIRVariable>(call->args[i])) type = v->type;
                     else if (auto c = std::dynamic_pointer_cast<MIRConstant>(call->args[i])) type = c->type;
                     
                     if (type) {
                         if (auto prim = std::dynamic_pointer_cast<PrimitiveType>(type)) {
                             if (prim->name == "string") isStr = true;
                         }
                     }
                     // Fallback/Legacy check for constants
                     if (!isStr && argVal.find("ptrtoint") != std::string::npos) {
                         isStr = true;
                     }
                     
                     if (isStr) {
                         std::string castTmp = "%str" + std::to_string(++tempCounter);
                         emitLine(castTmp + " = inttoptr i64 " + argVal + " to i8*");
                         
                         std::string fmtTmp = "%fmt" + std::to_string(++tempCounter);
                         emitLine(fmtTmp + " = getelementptr inbounds [3 x i8], [3 x i8]* @.fmt_s, i64 0, i64 0");
                         emitLine("call i32 (i8*, ...) @printf(i8* " + fmtTmp + ", i8* " + castTmp + ")");
                     } else {
                         std::string fmtTmp = "%fmt" + std::to_string(++tempCounter);
                         emitLine(fmtTmp + " = getelementptr inbounds [5 x i8], [5 x i8]* @.fmt_d, i64 0, i64 0");
                         emitLine("call i32 (i8*, ...) @printf(i8* " + fmtTmp + ", i64 " + argVal + ")");
                     }
                     
                     // Print space if not last arg
                     /*
                     if (i < call->args.size() - 1) {
                         std::string spTmp = "%sp" + std::to_string(++tempCounter);
                         emitLine(spTmp + " = getelementptr inbounds [2 x i8], [2 x i8]* @.fmt_sp, i64 0, i64 0");
                         emitLine("call i32 (i8*, ...) @printf(i8* " + spTmp + ")");
                     }
                     */
                 }
                 // Print newline at end
                 std::string nlTmp = "%nl" + std::to_string(++tempCounter);
                 emitLine(nlTmp + " = getelementptr inbounds [2 x i8], [2 x i8]* @.fmt_nl, i64 0, i64 0");
                 emitLine("call i32 (i8*, ...) @printf(i8* " + nlTmp + ")");
             }
         } else {
             // Generic call
             // Not implemented for this demo
         }
    }
}

std::string MIRCodeGen::resolveValue(std::shared_ptr<MIRValue> val) {
    if (auto c = std::dynamic_pointer_cast<MIRConstant>(val)) {
        // Handle "new Class" hack from earlier
        if (c->value.rfind("new ", 0) == 0) return "0"; 
        
        // Check type if available
        bool isNumber = false;
        if (c->type) {
            if (auto prim = std::dynamic_pointer_cast<PrimitiveType>(c->type)) {
               if (prim->name == "number" || prim->name == "int" || prim->name == "float") {
                    isNumber = true;
               }
            }
        } else {
            // Fallback heuristic
            isNumber = !c->value.empty() && (isdigit(c->value[0]) || (c->value[0] == '-' && c->value.size() > 1 && isdigit(c->value[1])));
        }

        if (isNumber) {
             if (c->value.find('.') != std::string::npos) {
                 // It's a float. For i64 backend, we truncate to int.
                 try {
                    double d = std::stod(c->value);
                    return std::to_string((long long)d);
                 } catch (...) { return "0"; }
             }
             return c->value;
        }

        // processing string literal
        std::string strLbl = "@.str" + std::to_string(labelCounter++);
        std::string content = c->value;
        // Basic escaping (incomplete but works for simple cases)
        // We should escape quotes, backslashes etc. 
        // For now, assume simple alphanumeric + space
        
        globalBuffer << strLbl << " = private unnamed_addr constant [" << (content.size()+1) << " x i8] c\"" << content << "\\00\"\n";
        
        // Return ptrtoint cast to i64
        return "ptrtoint ([ " + std::to_string(content.size()+1) + " x i8]* " + strLbl + " to i64)"; 
    }
    if (auto v = std::dynamic_pointer_cast<MIRVariable>(val)) {
        // Load from alloca
        std::string ptr = valueMap[v->name];
        // If variable is not found (e.g. implicitly declared?), return 0
        if (ptr.empty()) return "0";
        
        std::string tmp = "%v" + std::to_string(++tempCounter);
        emitLine(tmp + " = load i64, i64* " + ptr);
        return tmp;
    }
    return "0";
}

} // namespace tejx
