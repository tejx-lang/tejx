#pragma once

#include <string>
#include <vector>
#include <memory>
#include <iostream>

namespace tejx {

enum class TypeKind {
    Primitive,
    Class,
    Function,
    Array,
    Union,
    Any,
    Void,
    Start // Generic / Placeholder
};

struct Type {
    TypeKind kind;
    virtual ~Type() = default;
    Type(TypeKind k) : kind(k) {}
    
    virtual std::string toString() const = 0;
    virtual bool equals(const Type& other) const { return kind == other.kind; }
};

struct PrimitiveType : Type {
    std::string name; // number, string, boolean
    PrimitiveType(const std::string& n) : Type(TypeKind::Primitive), name(n) {}
    
    std::string toString() const override { return name; }
    bool equals(const Type& other) const override {
        if (auto p = dynamic_cast<const PrimitiveType*>(&other)) {
            return name == p->name;
        }
        return false;
    }
};

struct ClassType : Type {
    std::string name;
    ClassType(const std::string& n) : Type(TypeKind::Class), name(n) {}
    
    std::string toString() const override { return name; }
    bool equals(const Type& other) const override {
        if (auto c = dynamic_cast<const ClassType*>(&other)) {
            return name == c->name;
        }
        return false;
    }
};

struct ArrayType : Type {
    std::shared_ptr<Type> elementType;
    ArrayType(std::shared_ptr<Type> e) : Type(TypeKind::Array), elementType(e) {}
    
    std::string toString() const override { return elementType->toString() + "[]"; }
    bool equals(const Type& other) const override {
        if (auto a = dynamic_cast<const ArrayType*>(&other)) {
            return elementType->equals(*a->elementType);
        }
        return false;
    }
};

struct FunctionType : Type {
    std::vector<std::shared_ptr<Type>> paramTypes;
    std::shared_ptr<Type> returnType;
    
    FunctionType(const std::vector<std::shared_ptr<Type>>& p, std::shared_ptr<Type> r) 
        : Type(TypeKind::Function), paramTypes(p), returnType(r) {}
        
    std::string toString() const override {
        std::string s = "(";
        for (size_t i = 0; i < paramTypes.size(); ++i) {
            if (i > 0) s += ", ";
            s += paramTypes[i]->toString();
        }
        s += ") => " + returnType->toString();
        return s;
    }
};

struct VoidType : Type {
    VoidType() : Type(TypeKind::Void) {}
    std::string toString() const override { return "void"; }
};

struct AnyType : Type {
    AnyType() : Type(TypeKind::Any) {}
    std::string toString() const override { return "any"; }
    bool equals(const Type& other) const override { return true; } // Any matches anything
};

} // namespace tejx
