#pragma once

#include <vector>
#include <memory>
#include "Lexer.h"
#include "AST.h"

namespace tejx {

class Parser {
public:
    Parser(const std::vector<Token>& tokens);
    std::shared_ptr<Program> parse();

private:
    std::vector<Token> tokens;
    size_t current = 0;

    // Helper methods
    Token peek(int offset = 0);
    Token previous();
    bool isAtEnd();
    bool check(TokenType type);
    Token advance();
    Token consume(TokenType type, std::string message);

    // Parse methods
    std::shared_ptr<ASTNode> parseDeclaration();
    std::shared_ptr<ClassDeclaration> parseClassDeclaration();
    std::shared_ptr<ProtocolDeclaration> parseProtocolDeclaration(); // New
    std::shared_ptr<ExtensionDeclaration> parseExtensionDeclaration(); // New
    std::shared_ptr<EnumDeclaration> parseEnumDeclaration();
    std::shared_ptr<FunctionDeclaration> parseFunctionDeclaration(bool isAsync = false);
    
    // New
    std::shared_ptr<Expression> parseObjectLiteral();
    std::shared_ptr<ASTNode> parsePattern(bool allowLiterals = false); // Global pattern parser
    std::shared_ptr<BindingNode> parseBindingPattern();
    std::string parseType();
    std::shared_ptr<Statement> parseVarDeclaration();
    std::shared_ptr<Statement> parseStatement();
    std::shared_ptr<Statement> parseBlock();
    std::shared_ptr<Statement> parseIfStatement();
    std::shared_ptr<Statement> parseWhileStatement();
    std::shared_ptr<Statement> parseForStatement();
    std::shared_ptr<Statement> parseReturnStatement();
    std::shared_ptr<Statement> parseSwitchStatement();
    std::shared_ptr<Statement> parseTryStatement();
    std::shared_ptr<Statement> parseThrowStatement();
    std::shared_ptr<Statement> parseExpressionStatement();
    
    std::shared_ptr<Expression> parseExpression();
    std::shared_ptr<Expression> parseAssignment();
    std::shared_ptr<Expression> parseNullishCoalescing();
    std::shared_ptr<Expression> parseLogicalOr();
    std::shared_ptr<Expression> parseLogicalAnd();
    std::shared_ptr<Expression> parseEquality();
    std::shared_ptr<Expression> parseComparison();
    std::shared_ptr<Expression> parseTerm();
    std::shared_ptr<Expression> parseFactor();
    std::shared_ptr<Expression> parseUnary();
    std::shared_ptr<Expression> parseCall();
    std::shared_ptr<Expression> parsePrimary();
    std::shared_ptr<Expression> parseLambda(); // New
    std::shared_ptr<Expression> parseMatchExpression(); // New Match
};

} // namespace tejx
