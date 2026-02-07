#pragma once

#include "AST.h"
#include <sstream>
#include <string>
#include <set>
#include <unordered_set>
#include <unordered_map>

namespace tejx {

class CodeGen {
public:
    std::string generate(std::shared_ptr<Program> program);

private:
    std::stringstream buffer;
    std::stringstream structBuffer; // Buffer for anonymous structs
    std::set<std::string> generatedStructs;
    std::unordered_set<std::string> knownClasses; // For static vs instance call distinction
    std::unordered_set<std::string> knownProtocols; // For type mapping
    std::unordered_set<std::string> knownEnums;   // For enum vs class member access
    std::unordered_set<std::string> extensionMethodNames; // Names of methods defined in extensions
    std::string currentSelfVar; // For mapping 'this' to 'self' in extensions
    std::unordered_map<std::string, std::string> restFunctions; // funcName -> restParamElementType
    int indentLevel = 0;

    void emit(const std::string& code);
    void emitLine(const std::string& code);
    void indent();
    void dedent();

    void genStatement(std::shared_ptr<Statement> stmt);
    void genBlock(std::shared_ptr<BlockStmt> block);
    void genExpression(std::shared_ptr<Expression> expr);
    void genClassDeclaration(std::shared_ptr<ClassDeclaration> cls);
    void genProtocolDeclaration(std::shared_ptr<ProtocolDeclaration> proto); // New
    void genExtensionDeclaration(std::shared_ptr<ExtensionDeclaration> ext);
    void genEnumDeclaration(std::shared_ptr<EnumDeclaration> enumDecl);
    void genMatchExpression(std::shared_ptr<MatchExpr> match);
    void genMatchCondition(std::shared_ptr<ASTNode> pattern, const std::string& valName);
    void genMatchBindings(std::shared_ptr<ASTNode> pattern, const std::string& valName);
    void genDestructuring(std::shared_ptr<ASTNode> pattern, const std::string& rhsExpr, bool isFirstDecl);
    std::string genStructForSignature(const std::string& signature);
    std::string mapType(const std::string& type);
};

} // namespace tejx
