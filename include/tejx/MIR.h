#pragma once

#include "Type.h"
#include "HIR.h" // For TokenType
#include <vector>
#include <string>
#include <memory>

namespace tejx {

enum class MIROpcode {
    Move,       // dst = src
    LoadConst,  // dst = constant
    BinaryOp,   // dst = src1 op src2
    Branch,     // if cond goto trueBlock else goto falseBlock
    Jump,       // goto targetBlock
    Return,     // return [value]
    Call,       // dst = call func(args...)
    // ...
};

struct MIRInstruction {
    MIROpcode opcode;
    virtual ~MIRInstruction() = default;
    MIRInstruction(MIROpcode op) : opcode(op) {}
    virtual std::string toString() const = 0;
};

struct MIRValue {
    virtual ~MIRValue() = default;
    virtual std::string toString() const = 0;
};

struct MIRVariable : MIRValue {
    std::string name;
    std::shared_ptr<Type> type;
    MIRVariable(const std::string& n, std::shared_ptr<Type> t) : name(n), type(t) {}
    std::string toString() const override { return name; }
};

struct MIRConstant : MIRValue {
    std::string value;
    std::shared_ptr<Type> type;
    MIRConstant(const std::string& v, std::shared_ptr<Type> t) : value(v), type(t) {}
    std::string toString() const override { return value; }
};

// Instructions

struct MIRMove : MIRInstruction {
    std::shared_ptr<MIRVariable> dst; // Destination must be a variable
    std::shared_ptr<MIRValue> src;
    MIRMove(std::shared_ptr<MIRVariable> d, std::shared_ptr<MIRValue> s) 
        : MIRInstruction(MIROpcode::Move), dst(d), src(s) {}
    std::string toString() const override { return dst->toString() + " = " + src->toString(); }
};

struct MIRBinary : MIRInstruction {
    std::shared_ptr<MIRVariable> dst;
    std::shared_ptr<MIRValue> left;
    TokenType op;
    std::shared_ptr<MIRValue> right;
    MIRBinary(std::shared_ptr<MIRVariable> d, std::shared_ptr<MIRValue> l, TokenType o, std::shared_ptr<MIRValue> r)
        : MIRInstruction(MIROpcode::BinaryOp), dst(d), left(l), op(o), right(r) {}
    std::string toString() const override { return dst->toString() + " = " + left->toString() + " op " + right->toString(); } // simplified op string
};

struct BasicBlock; // Forward decl

struct MIRBranch : MIRInstruction {
    std::shared_ptr<MIRValue> condition;
    std::shared_ptr<BasicBlock> trueTarget;
    std::shared_ptr<BasicBlock> falseTarget;
    MIRBranch(std::shared_ptr<MIRValue> c, std::shared_ptr<BasicBlock> t, std::shared_ptr<BasicBlock> f)
        : MIRInstruction(MIROpcode::Branch), condition(c), trueTarget(t), falseTarget(f) {}
    std::string toString() const override; // Need BB names
};

struct MIRJump : MIRInstruction {
    std::shared_ptr<BasicBlock> target;
    MIRJump(std::shared_ptr<BasicBlock> t) : MIRInstruction(MIROpcode::Jump), target(t) {}
    std::string toString() const override;
};

struct MIRReturn : MIRInstruction {
    std::shared_ptr<MIRValue> value; // Optional
    MIRReturn(std::shared_ptr<MIRValue> v) : MIRInstruction(MIROpcode::Return), value(v) {}

    std::string toString() const override { return "return " + (value ? value->toString() : ""); }
};

struct MIRCall : MIRInstruction {
    std::shared_ptr<MIRVariable> dst;
    std::string callee;
    std::vector<std::shared_ptr<MIRValue>> args;
    
    MIRCall(std::shared_ptr<MIRVariable> d, const std::string& c, const std::vector<std::shared_ptr<MIRValue>>& a)
        : MIRInstruction(MIROpcode::Call), dst(d), callee(c), args(a) {}
        
    std::string toString() const override { return dst->toString() + " = call " + callee + "(...)"; }
};

struct BasicBlock {
    std::string name;
    std::vector<std::shared_ptr<MIRInstruction>> instructions;
    std::vector<std::shared_ptr<BasicBlock>> predecessors;
    std::vector<std::shared_ptr<BasicBlock>> successors;

    BasicBlock(const std::string& n) : name(n) {}
    
    void addInstruction(std::shared_ptr<MIRInstruction> inst) {
        instructions.push_back(inst);
    }
};

// Out-of-line defs for circular dependency
inline std::string MIRBranch::toString() const { return "branch " + condition->toString() + " ? " + trueTarget->name + " : " + falseTarget->name; }
inline std::string MIRJump::toString() const { return "jump " + target->name; }

struct MIRFunction {
    std::string name;
    std::vector<std::shared_ptr<BasicBlock>> blocks;
    std::shared_ptr<BasicBlock> entryBlock;
    
    MIRFunction(const std::string& n) : name(n) {}
};

} // namespace tejx
