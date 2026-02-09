#include "tejx/Parser.h"
#include <iostream>

namespace tejx {

Parser::Parser(const std::vector<Token>& tokens) : tokens(tokens) {}

std::shared_ptr<Program> Parser::parse() {
    auto program = std::make_shared<Program>();
    while (!isAtEnd()) {
        if (auto decl = parseDeclaration()) {
             // Cast to Statement because that's what we store now
             if (auto stmt = std::dynamic_pointer_cast<tejx::Statement>(decl)) {
                 program->statements.push_back(stmt);
             } else {
                 if (auto func = std::dynamic_pointer_cast<tejx::FunctionDeclaration>(decl)) {
                    // FunctionDeclaration inherits ASTNode, but we need Statement wrapper or change Program to hold ASTNode
                    // AST.h says: struct Program : ASTNode { std::vector<std::shared_ptr<ASTNode>> statements; ... }
                    program->statements.push_back(decl);
                 } else if (auto cls = std::dynamic_pointer_cast<tejx::ClassDeclaration>(decl)) {
                    program->statements.push_back(decl);
                 } else if (auto ext = std::dynamic_pointer_cast<tejx::ExtensionDeclaration>(decl)) {
                    program->statements.push_back(decl);
                 } else if (auto proto = std::dynamic_pointer_cast<tejx::ProtocolDeclaration>(decl)) {
                    program->statements.push_back(decl);
                 } else {
                    program->statements.push_back(decl);
                 }
             }
        }
    }
    return program;
}

// Helpers
std::shared_ptr<Expression> parseUnary();
Token Parser::peek(int offset) {
    if (current + offset >= tokens.size()) return tokens.back();
    return tokens[current + offset];
}
Token Parser::previous() { return tokens[current - 1]; }
bool Parser::isAtEnd() { return peek().type == TokenType::EndOfFile; }

Token Parser::advance() {
    if (!isAtEnd()) current++;
    return previous();
}

bool Parser::check(TokenType type) {
    if (isAtEnd()) return false;
    return peek().type == type;
}

Token Parser::consume(TokenType type, std::string message) {
    if (check(type)) return advance();
    std::cerr << "Error at token " << previous().value << ": " << message << std::endl;
    exit(1);
    throw std::runtime_error(message);
}

// Declarations
std::shared_ptr<tejx::ASTNode> Parser::parseDeclaration() {
    // Handle import: import { x, y } from "./file.tx" or import x from "./file.tx"
    if (check(TokenType::Import)) {
        advance(); // consume 'import'
        std::vector<std::string> names;
        bool isDefault = false;
        
        if (check(TokenType::OpenBrace)) {
            // Named imports: import { x, y } from "..."
            advance(); // consume '{'
            do {
                names.push_back(consume(TokenType::Identifier, "Expected import name").value);
            } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
            consume(TokenType::CloseBrace, "Expected '}'");
        } else if (check(TokenType::Identifier)) {
            // Default import: import x from "..."
            names.push_back(advance().value);
            isDefault = true;
        }
        
        consume(TokenType::From, "Expected 'from'");
        std::string source = consume(TokenType::String, "Expected module path").value;
        consume(TokenType::Semicolon, "Expected ';'");
        return std::make_shared<ImportDecl>(names, source, isDefault);
    }
    
    // Handle export: export function/const/default
    if (check(TokenType::Export)) {
        advance(); // consume 'export'
        bool isDefault = false;
        
        // Check for: export default ...
        if (check(TokenType::Default)) {
            advance();
            isDefault = true;
        }
        
        // Parse the declaration being exported
        std::shared_ptr<ASTNode> decl;
        if (check(TokenType::Function) || check(TokenType::Async)) {
            decl = parseDeclaration();
        } else if (check(TokenType::Let) || check(TokenType::Const)) {
            decl = parseVarDeclaration();
        } else if (check(TokenType::Class)) {
            decl = parseClassDeclaration();
        } else {
            std::cerr << "Error: Expected function, const, let, or class after export." << std::endl;
            exit(1);
        }
        
        return std::make_shared<ExportDecl>(decl, isDefault);
    }
    
    // Check for async function
    if (check(TokenType::Async)) {
        advance(); // consume 'async'
        if (check(TokenType::Function)) {
            return parseFunctionDeclaration(true); // isAsync = true
        }
        std::cerr << "Error: Expected 'function' after 'async'." << std::endl;
        exit(1);
    }
    if (check(TokenType::Function)) return parseFunctionDeclaration(false);
    if (check(TokenType::Class)) return parseClassDeclaration();
    if (check(TokenType::Protocol)) return parseProtocolDeclaration();
    if (check(TokenType::Extension)) return parseExtensionDeclaration();
    if (check(TokenType::Enum)) return parseEnumDeclaration();
    if (check(TokenType::Let) || check(TokenType::Const)) return parseVarDeclaration();
    return parseStatement();
}

    std::shared_ptr<ClassDeclaration> Parser::parseClassDeclaration() {
    consume(TokenType::Class, "Expected 'class'.");
    std::string name = consume(TokenType::Identifier, "Expected class name.").value;
    
    std::string parentName = "";
    if (check(TokenType::Extends)) {
        advance();
        parentName = consume(TokenType::Identifier, "Expected parent class name.").value;
    }

    std::vector<std::string> implementedProtocols;
    if (check(TokenType::Implements)) {
        advance(); // consume 'implements'
        do {
             implementedProtocols.push_back(consume(TokenType::Identifier, "Expected interface name").value);
        } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
    }
    
    consume(TokenType::OpenBrace, "Expected '{' before class body.");
    
    std::vector<ClassDeclaration::Member> members;
    std::vector<ClassDeclaration::Method> methods; // Updated to use Method struct
    std::vector<ClassDeclaration::Getter> getters;
    std::vector<ClassDeclaration::Setter> setters;
    std::shared_ptr<FunctionDeclaration> constructor = nullptr;
    
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        if (check(TokenType::Constructor)) {
            advance(); 
            consume(TokenType::OpenParen, "Expected '(' after constructor.");
            std::vector<Parameter> params;
            if (!check(TokenType::CloseParen)) {
                do {
                    std::string pName = consume(TokenType::Identifier, "Expected param name.").value;
                    consume(TokenType::Colon, "Expected ':'");
                    std::string pType = "";
                    if (check(TokenType::TypeNumber)) { pType = "number"; advance(); }
                    else if (check(TokenType::TypeString)) { pType = "string"; advance(); }
                    else if (check(TokenType::TypeBoolean)) { pType = "boolean"; advance(); }
                    else { pType = consume(TokenType::Identifier, "Expected type.").value; }
                    
                    while (check(TokenType::OpenBracket)) {
                        consume(TokenType::OpenBracket, "Expected '['");
                        consume(TokenType::CloseBracket, "Expected ']'");
                        pType += "[]";
                    }
                    params.push_back({pName, pType});
                } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
            }
            consume(TokenType::CloseParen, "Expected ')'");
            auto body = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
            constructor = std::make_shared<FunctionDeclaration>(name, params, "void", body);
        } else {
            // Check for Access Modifiers and Static
            ClassDeclaration::AccessModifier access = ClassDeclaration::AccessModifier::Public;
            bool isStatic = false;
            
            if (check(TokenType::Public)) { access = ClassDeclaration::AccessModifier::Public; advance(); }
            else if (check(TokenType::Private)) { access = ClassDeclaration::AccessModifier::Private; advance(); }
            else if (check(TokenType::Protected)) { access = ClassDeclaration::AccessModifier::Protected; advance(); }
            
            if (check(TokenType::Static)) { isStatic = true; advance(); }
            
            // Getter: get name() { ... }
            if (check(TokenType::Identifier) && peek().value == "get" && peek(1).type == TokenType::Identifier) {
                advance(); // consume 'get'
                std::string gName = consume(TokenType::Identifier, "Expected getter name.").value;
                consume(TokenType::OpenParen, "Expected '('");
                consume(TokenType::CloseParen, "Expected ')'");
                
                std::string retType = "any";
                if (check(TokenType::Colon)) {
                    advance();
                    if (check(TokenType::TypeNumber)) { retType = "number"; advance(); }
                    else if (check(TokenType::TypeString)) { retType = "string"; advance(); }
                    else if (check(TokenType::TypeBoolean)) { retType = "boolean"; advance(); }
                    else { retType = consume(TokenType::Identifier, "RetType").value; }
                }
                
                auto body = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
                getters.push_back({gName, retType, body, access});
                continue;
            }
            
            // Setter: set name(value) { ... }
            if (check(TokenType::Identifier) && peek().value == "set" && peek(1).type == TokenType::Identifier) {
                advance(); // consume 'set'
                std::string sName = consume(TokenType::Identifier, "Expected setter name.").value;
                consume(TokenType::OpenParen, "Expected '('");
                std::string paramName = consume(TokenType::Identifier, "Expected param name.").value;
                std::string paramType = "any";
                if (check(TokenType::Colon)) {
                    advance();
                    if (check(TokenType::TypeNumber)) { paramType = "number"; advance(); }
                    else if (check(TokenType::TypeString)) { paramType = "string"; advance(); }
                    else if (check(TokenType::TypeBoolean)) { paramType = "boolean"; advance(); }
                    else { paramType = consume(TokenType::Identifier, "ParamType").value; }
                }
                consume(TokenType::CloseParen, "Expected ')'");
                
                auto body = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
                setters.push_back({sName, paramName, paramType, body, access});
                continue;
            }
            
            // Method or Field?
            // Methods start with function name + (
            // Fields start with name + :
            // But we also support `function foo()` syntax? (Existing parser did: `else if (check(TokenType::Function))`)
            // Let's stick to TS style: `[mod] [static] name...`
            
            if (check(TokenType::Identifier) && peek(1).type == TokenType::OpenParen) {
                // Method
                std::string mName = consume(TokenType::Identifier, "Expected method name.").value;
                consume(TokenType::OpenParen, "Expected '('");
                std::vector<Parameter> params;
                if (!check(TokenType::CloseParen)) {
                    do {
                        std::string pName = consume(TokenType::Identifier, "Param name").value;
                        consume(TokenType::Colon, "Expected ':'");
                        std::string pType = "";
                        if (check(TokenType::TypeNumber)) { pType = "number"; advance(); }
                        else if (check(TokenType::TypeString)) { pType = "string"; advance(); }
                        else if (check(TokenType::TypeBoolean)) { pType = "boolean"; advance(); }
                        else { pType = consume(TokenType::Identifier, "type").value; }
                        while (check(TokenType::OpenBracket)) {
                            consume(TokenType::OpenBracket, "["); consume(TokenType::CloseBracket, "]");
                            pType += "[]";
                        }
                        params.push_back({pName, pType});
                    } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
                }
                consume(TokenType::CloseParen, "Expected ')'");
                
                std::string retType = "void";
                if (check(TokenType::Colon)) {
                    advance();
                    if (check(TokenType::TypeNumber)) retType = "number";
                    else if (check(TokenType::TypeString)) retType = "string";
                    else if (check(TokenType::TypeBoolean)) retType = "boolean";
                    else if (check(TokenType::TypeVoid)) retType = "void";
                    else { retType = consume(TokenType::Identifier, "RetType").value; }
                    advance(); 
                    while (check(TokenType::OpenBracket)) {
                         consume(TokenType::OpenBracket, "["); consume(TokenType::CloseBracket, "]");
                         retType += "[]";
                    }
                }
                
                auto body = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
                auto funcDecl = std::make_shared<FunctionDeclaration>(mName, params, retType, body);
                methods.push_back({funcDecl, access, isStatic});
                
            } else if (check(TokenType::Identifier)) {
                // Member variable
                std::string mName = consume(TokenType::Identifier, "Expected member name.").value;
                consume(TokenType::Colon, "Expected ':'");
                std::string mType = "";
                if (check(TokenType::TypeNumber)) { mType = "number"; advance(); }
                else if (check(TokenType::TypeString)) { mType = "string"; advance(); }
                else if (check(TokenType::TypeBoolean)) { mType = "boolean"; advance(); }
                else { mType = consume(TokenType::Identifier, "Expected type.").value; }
                 
                 while (check(TokenType::OpenBracket)) {
                     consume(TokenType::OpenBracket, "Expected '['");
                     consume(TokenType::CloseBracket, "Expected ']'");
                     mType += "[]";
                 }
                
                std::shared_ptr<Expression> init = nullptr;
                if (check(TokenType::Equals)) {
                    advance();
                    init = parseExpression();
                }
                 
                consume(TokenType::Semicolon, "Expected ';'");
                members.push_back({mName, mType, access, isStatic, init});
            } else {
                 // Unknown or syntax error
                 advance();
            }
        }
    }
    consume(TokenType::CloseBrace, "Expected '}' after class body.");
    return std::make_shared<ClassDeclaration>(name, members, methods, constructor, parentName, implementedProtocols, getters, setters);
}

std::shared_ptr<EnumDeclaration> Parser::parseEnumDeclaration() {
    consume(TokenType::Enum, "Expected 'enum'.");
    std::string name = consume(TokenType::Identifier, "Expected enum name.").value;
    consume(TokenType::OpenBrace, "Expected '{' before enum body.");
    
    std::vector<EnumMember> members;
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        std::string mName = consume(TokenType::Identifier, "Expected member name.").value;
        std::shared_ptr<Expression> mValue = nullptr;
        if (check(TokenType::Equals)) {
            advance();
            mValue = parseExpression();
        }
        members.push_back({mName, mValue});
        
        if (check(TokenType::Comma)) {
            advance();
        }
    }
    consume(TokenType::CloseBrace, "Expected '}' after enum body.");
    return std::make_shared<EnumDeclaration>(name, members);
}

std::shared_ptr<Expression> Parser::parseObjectLiteral() {
    consume(TokenType::OpenBrace, "Expected '{'");
    std::vector<std::pair<std::string, std::shared_ptr<Expression>>> entries;
    std::vector<std::shared_ptr<Expression>> spreads;
    
    if (!check(TokenType::CloseBrace)) {
        do {
            // Check for spread operator
            if (check(TokenType::Ellipsis)) {
                advance(); // consume '...'
                auto spreadExpr = parseExpression();
                spreads.push_back(spreadExpr);
            } else {
                std::string key = consume(TokenType::Identifier, "Expected key name.").value;
                consume(TokenType::Colon, "Expected ':' after key.");
                auto value = parseExpression();
                entries.push_back({key, value});
            }
        } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
    }
    consume(TokenType::CloseBrace, "Expected '}' after object literal.");
    return std::make_shared<ObjectLiteralExpr>(entries, spreads);
}

std::shared_ptr<FunctionDeclaration> Parser::parseFunctionDeclaration(bool isAsync) {
    consume(TokenType::Function, "Expected 'function'.");
    std::string name = consume(TokenType::Identifier, "Expected function name.").value;
    
    consume(TokenType::OpenParen, "Expected '(' after function name.");
    std::vector<Parameter> params;
    if (!check(TokenType::CloseParen)) {
        do {
            bool isRest = false;
            if (check(TokenType::Ellipsis)) {
                advance(); // consume ...
                isRest = true;
            }
            std::string paramName = consume(TokenType::Identifier, "Expected parameter name.").value;
            consume(TokenType::Colon, "Expected ':' after parameter name.");
            std::string type = parseType();
            
            std::shared_ptr<Expression> defaultValue = nullptr;
            if (check(TokenType::Equals)) {
                advance();
                defaultValue = parseExpression();
            }
            
            params.push_back({paramName, type, defaultValue, isRest});
        } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
    }
    consume(TokenType::CloseParen, "Expected ')' after parameters.");
    
    std::string returnType = "";
    if (check(TokenType::Colon)) {
        advance();
        returnType = parseType();
    }
    
    auto body = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
    return std::make_shared<FunctionDeclaration>(name, params, returnType, body, isAsync);
}

std::shared_ptr<ASTNode> Parser::parsePattern(bool allowLiterals) {
    if (check(TokenType::OpenBracket)) {
        advance();
        std::vector<std::shared_ptr<ASTNode>> elements;
        std::shared_ptr<ASTNode> rest = nullptr;
        while (!check(TokenType::CloseBracket) && !isAtEnd()) {
            if (check(TokenType::Ellipsis)) {
                advance(); // ...
                rest = parsePattern(allowLiterals);
                if (!check(TokenType::CloseBracket)) {
                    std::cerr << "Rest element must be last in array pattern." << std::endl;
                    exit(1);
                }
                break;
            }
            elements.push_back(parsePattern(allowLiterals));
            if (check(TokenType::Comma)) advance();
        }
        consume(TokenType::CloseBracket, "Expected ']'");
        return std::make_shared<ArrayBinding>(elements, rest);
    }
    if (check(TokenType::OpenBrace)) {
        advance();
        std::vector<ObjectBinding::Entry> entries;
        while (!check(TokenType::CloseBrace) && !isAtEnd()) {
            std::string source = consume(TokenType::Identifier, "Expected property name.").value;
            std::shared_ptr<ASTNode> target;
            if (check(TokenType::Colon)) {
                advance();
                target = parsePattern(allowLiterals);
            } else {
                target = std::make_shared<IdentifierBinding>(source);
            }
            entries.push_back({source, target});
            if (check(TokenType::Comma)) advance();
        }
        consume(TokenType::CloseBrace, "Expected '}'");
        return std::make_shared<ObjectBinding>(entries);
    }
    
    if (allowLiterals) {
        if (check(TokenType::Number) || check(TokenType::String) || 
            check(TokenType::True) || check(TokenType::False) || check(TokenType::Undefined) ||
            check(TokenType::None)) {
            return parsePrimary();
        }
        if (check(TokenType::Some)) {
            advance(); // Some
            consume(TokenType::OpenParen, "Expected '(' after 'Some' in pattern.");
            auto inner = parsePattern(allowLiterals);
            consume(TokenType::CloseParen, "Expected ')' after Some pattern.");
            return std::make_shared<SomeExpr>(inner);
        }
    }
    
    auto nameToken = consume(TokenType::Identifier, "Expected identifier in pattern.");
    if (allowLiterals && nameToken.value == "_") {
        return std::make_shared<Identifier>("_");
    }
    return std::make_shared<IdentifierBinding>(nameToken.value);
}

std::shared_ptr<BindingNode> Parser::parseBindingPattern() {
    auto p = parsePattern(false);
    return std::dynamic_pointer_cast<BindingNode>(p);
}

std::shared_ptr<Expression> Parser::parseMatchExpression() {
    consume(TokenType::Match, "Expected 'match'.");
    consume(TokenType::OpenParen, "Expected '(' after 'match'.");
    auto target = parseExpression();
    consume(TokenType::CloseParen, "Expected ')' after match target.");
    consume(TokenType::OpenBrace, "Expected '{' before match arms.");

    std::vector<MatchArm> arms;
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        auto pattern = parsePattern(true);
        
        std::shared_ptr<Expression> guard = nullptr;
        if (check(TokenType::If)) {
            advance(); // if
            consume(TokenType::OpenParen, "Expected '(' after 'if' in match guard.");
            guard = parseExpression();
            consume(TokenType::CloseParen, "Expected ')' after match guard condition.");
        }

        consume(TokenType::Arrow, "Expected '=>' after match pattern.");
        std::shared_ptr<Expression> body = nullptr;
        if (check(TokenType::OpenBrace)) {
            body = std::make_shared<BlockExpr>(std::dynamic_pointer_cast<BlockStmt>(parseBlock()));
        } else {
            body = parseExpression();
        }
        arms.push_back({pattern, guard, body});

        if (check(TokenType::Comma)) advance();
    }
    consume(TokenType::CloseBrace, "Expected '}' after match arms.");
    return std::make_shared<MatchExpr>(target, arms);
}

std::string Parser::parseType() {
    std::string type = "any";
    if (check(TokenType::OpenBrace)) {
        // Structural Type: { name: string, age: number }
        advance(); // consume {
        type = "{";
        bool first = true;
        while (!check(TokenType::CloseBrace) && !isAtEnd()) {
            if (!first) {
                consume(TokenType::Comma, "Expected ','");
                if (check(TokenType::CloseBrace)) break; // trailing comma
                type += ",";
            }
            std::string key = consume(TokenType::Identifier, "Expected key.").value;
            consume(TokenType::Colon, "Expected ':'");
            
            std::string fieldType = parseType();
            type += key + ":" + fieldType;
            first = false;
        }
        consume(TokenType::CloseBrace, "Expected '}'");
        type += "}";
    } else {
        // Standard Type
        Token typeToken = advance();
        if (typeToken.type == TokenType::TypeNumber) type = "number";
        else if (typeToken.type == TokenType::TypeString) type = "string";
        else if (typeToken.type == TokenType::TypeBoolean) type = "boolean";
        else if (typeToken.type == TokenType::TypeVoid) type = "void";
        else if (typeToken.type == TokenType::Option) {
            consume(TokenType::Less, "Expected '<' after Option.");
            type = "Option<" + parseType() + ">";
            consume(TokenType::Greater, "Expected '>' after inner type.");
        }
        else if (typeToken.value == "Promise") {
            consume(TokenType::Less, "Expected '<' after Promise.");
            type = "Promise<" + parseType() + ">";
            consume(TokenType::Greater, "Expected '>' after inner type.");
        }
        else type = typeToken.value; // Class name or fallback
        
        // Check for array type []
        while (check(TokenType::OpenBracket)) {
            consume(TokenType::OpenBracket, "Expected '['");
            consume(TokenType::CloseBracket, "Expected ']'");
            type += "[]";
        }
    }
    return type;
}

std::shared_ptr<Statement> Parser::parseVarDeclaration() {
    bool isConst = (advance().type == TokenType::Const);
    auto pattern = parseBindingPattern();
    
    std::string type = "any";
    if (check(TokenType::Colon)) {
        advance();
        type = parseType();
    }
    
    std::shared_ptr<Expression> initializer = nullptr;
    if (check(TokenType::Equals)) {
        advance();
        initializer = parseExpression();
    }
    consume(TokenType::Semicolon, "Expected ';' after variable declaration.");
    return std::make_shared<VarDeclaration>(pattern, type, initializer, isConst);
}

std::shared_ptr<Statement> Parser::parseStatement() {
    if (check(TokenType::If)) return parseIfStatement();
    if (check(TokenType::Switch)) return parseSwitchStatement();
    if (check(TokenType::While)) return parseWhileStatement();
    if (check(TokenType::For)) return parseForStatement();
    if (check(TokenType::Break)) {
        advance();
        consume(TokenType::Semicolon, "Expected ';'");
        return std::make_shared<BreakStmt>();
    }
    if (check(TokenType::Continue)) {
        advance();
        consume(TokenType::Semicolon, "Expected ';'");
        return std::make_shared<ContinueStmt>();
    }
    if (check(TokenType::Return)) return parseReturnStatement();
    if (check(TokenType::Try)) return parseTryStatement();
    if (check(TokenType::Throw)) return parseThrowStatement();
    if (check(TokenType::OpenBrace)) return parseBlock();
    
    return parseExpressionStatement();
}

std::shared_ptr<Statement> Parser::parseBlock() {
    auto block = std::make_shared<BlockStmt>();
    consume(TokenType::OpenBrace, "Expected '{'");
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        block->statements.push_back(std::dynamic_pointer_cast<Statement>(parseDeclaration()));
    }
    consume(TokenType::CloseBrace, "Expected '}'");
    return block;
}

std::shared_ptr<Statement> Parser::parseIfStatement() {
    consume(TokenType::If, "Expected 'if'");
    consume(TokenType::OpenParen, "Expected '('");
    auto condition = parseExpression();
    consume(TokenType::CloseParen, "Expected ')'");
    
    auto thenBranch = parseStatement();
    std::shared_ptr<Statement> elseBranch = nullptr;
    if (check(TokenType::Else)) {
        advance();
        elseBranch = parseStatement();
    }
    return std::make_shared<IfStmt>(condition, thenBranch, elseBranch);
}

std::shared_ptr<Statement> Parser::parseWhileStatement() {
    consume(TokenType::While, "Expected 'while'");
    consume(TokenType::OpenParen, "Expected '('");
    auto condition = parseExpression();
    consume(TokenType::CloseParen, "Expected ')'");
    auto body = parseStatement();
    return std::make_shared<WhileStmt>(condition, body);
}

std::shared_ptr<Statement> Parser::parseForStatement() {
    consume(TokenType::For, "Expected 'for'");
    consume(TokenType::OpenParen, "Expected '('");
    
    // Check for for-of loop: for (let x of arr)
    if (check(TokenType::Let) || check(TokenType::Const)) {
        size_t savedPos = current;
        advance(); // skip let/const
        if (check(TokenType::Identifier)) {
            std::string varName = advance().value;
            // Check if next token is 'of' (as identifier)
            if (check(TokenType::Identifier) && peek().value == "of") {
                advance(); // skip 'of'
                auto iterable = parseExpression();
                consume(TokenType::CloseParen, "Expected ')'");
                auto body = parseStatement();
                return std::make_shared<ForOfStmt>(varName, iterable, body);
            }
        }
        // Not a for-of loop, restore position
        current = savedPos;
    }
    
    // C-style for loop
    std::shared_ptr<Statement> initializer;
    if (check(TokenType::Semicolon)) {
        advance();
        initializer = nullptr;
    } else if (check(TokenType::Let) || check(TokenType::Const)) {
        initializer = parseVarDeclaration();
    } else {
        initializer = parseExpressionStatement(); // Handles expr + semicolon
    }
    
    std::shared_ptr<Expression> condition = nullptr;
    if (!check(TokenType::Semicolon)) {
        condition = parseExpression();
    }
    consume(TokenType::Semicolon, "Expected ';'");
    
    std::shared_ptr<Expression> increment = nullptr;
    if (!check(TokenType::CloseParen)) {
        increment = parseExpression();
    }
    consume(TokenType::CloseParen, "Expected ')'");
    
    auto body = parseStatement();
    
    if (!condition) condition = std::make_shared<BooleanLiteral>(true);
    
    return std::make_shared<ForStmt>(initializer, condition, increment, body);
}

std::shared_ptr<Statement> Parser::parseSwitchStatement() {
    consume(TokenType::Switch, "Expected 'switch'");
    consume(TokenType::OpenParen, "Expected '('");
    auto condition = parseExpression();
    consume(TokenType::CloseParen, "Expected ')'");
    consume(TokenType::OpenBrace, "Expected '{'");
    
    std::vector<Case> cases;
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        std::shared_ptr<Expression> value = nullptr;
        if (check(TokenType::Case)) {
            advance();
            value = parseExpression();
            consume(TokenType::Colon, "Expected ':' after case value");
        } else if (check(TokenType::Default)) {
            advance();
            consume(TokenType::Colon, "Expected ':' after default");
        } else {
             // Maybe comments or empty lines? Or error.
             throw std::runtime_error("Expected 'case' or 'default' inside switch.");
        }
        
        std::vector<std::shared_ptr<Statement>> stmts;
        // Parse statements until next Case, Default, or CloseBrace
        while (!check(TokenType::Case) && !check(TokenType::Default) && !check(TokenType::CloseBrace) && !isAtEnd()) {
            auto decl = parseDeclaration();
            if (auto stmt = std::dynamic_pointer_cast<Statement>(decl)) {
                stmts.push_back(stmt);
            } else {
                throw std::runtime_error("Declaration is not a statement inside switch.");
            }
        }
        
        cases.push_back({value, stmts});
    }
    consume(TokenType::CloseBrace, "Expected '}'");
    return std::make_shared<SwitchStmt>(condition, cases);
}

std::shared_ptr<Statement> Parser::parseReturnStatement() {
    consume(TokenType::Return, "Expected 'return'");
    std::shared_ptr<Expression> value = nullptr;
    if (!check(TokenType::Semicolon)) {
        value = parseExpression();
    }
    consume(TokenType::Semicolon, "Expected ';'");
    return std::make_shared<ReturnStmt>(value);
}

std::shared_ptr<Statement> Parser::parseTryStatement() {
    consume(TokenType::Try, "Expected 'try'");
    auto tryBlock = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
    
    std::string catchVar = "";
    std::shared_ptr<BlockStmt> catchBlock = nullptr;
    std::shared_ptr<BlockStmt> finallyBlock = nullptr;
    
    if (check(TokenType::Catch)) {
        advance();
        consume(TokenType::OpenParen, "Expected '(' after catch");
        catchVar = consume(TokenType::Identifier, "Expected exception variable name").value;
        consume(TokenType::CloseParen, "Expected ')' after catch variable");
        catchBlock = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
    }
    
    if (check(TokenType::Finally)) {
        advance();
        finallyBlock = std::dynamic_pointer_cast<BlockStmt>(parseBlock());
    }
    
    return std::make_shared<TryStmt>(tryBlock, catchVar, catchBlock, finallyBlock);
}

std::shared_ptr<Statement> Parser::parseThrowStatement() {
    consume(TokenType::Throw, "Expected 'throw'");
    auto expr = parseExpression();
    consume(TokenType::Semicolon, "Expected ';' after throw");
    return std::make_shared<ThrowStmt>(expr);
}

std::shared_ptr<Statement> Parser::parseExpressionStatement() {
    auto expr = parseExpression();
    consume(TokenType::Semicolon, "Expected ';'");
    return std::make_shared<ExpressionStmt>(expr);
}

// Expressions
std::shared_ptr<Expression> Parser::parseExpression() {
    return parseAssignment(); 
}

std::shared_ptr<Expression> Parser::parseAssignment() {
    auto expr = parseNullishCoalescing(); 
    
    // Ternary Operator: a ? b : c
    if (check(TokenType::Question)) {
        advance();
        auto trueBranch = parseAssignment(); // Recursive for nested ternary
        consume(TokenType::Colon, "Expected ':' in ternary operator.");
        auto falseBranch = parseAssignment();
        return std::make_shared<TernaryExpr>(expr, trueBranch, falseBranch);
    }
    
    if (check(TokenType::Equals) || check(TokenType::PlusEquals) || 
        check(TokenType::MinusEquals) || check(TokenType::StarEquals) || 
        check(TokenType::SlashEquals)) {
        Token op = advance();
        auto value = parseAssignment();
        
        if (std::dynamic_pointer_cast<Identifier>(expr) || 
            std::dynamic_pointer_cast<ArrayAccessExpr>(expr) ||
            std::dynamic_pointer_cast<MemberAccessExpr>(expr)) {
            return std::make_shared<AssignmentExpr>(expr, value, op.type);
        }
        
        std::cerr << "Invalid assignment target." << std::endl;
        exit(1);
    }
    
    return expr;
}

std::shared_ptr<Expression> Parser::parseNullishCoalescing() {
    auto expr = parseLogicalOr();
    while (check(TokenType::QuestionQuestion)) {
        advance();
        auto right = parseLogicalOr();
        expr = std::make_shared<NullishCoalescingExpr>(expr, right);
    }
    return expr;
}

// Logical OR (||)
std::shared_ptr<Expression> Parser::parseLogicalOr() {
    auto expr = parseLogicalAnd();
    while (check(TokenType::PipePipe)) {
        Token op = advance();
        auto right = parseLogicalAnd();
        expr = std::make_shared<BinaryExpr>(expr, op.type, right);
    }
    return expr;
}

// Logical AND (&&)
std::shared_ptr<Expression> Parser::parseLogicalAnd() {
    auto expr = parseEquality();
    while (check(TokenType::AmpersandAmpersand)) {
        Token op = advance();
        auto right = parseEquality();
        expr = std::make_shared<BinaryExpr>(expr, op.type, right);
    }
    return expr;
}

std::shared_ptr<Expression> Parser::parseEquality() {
    auto expr = parseComparison();
    while (check(TokenType::EqualEqual) || check(TokenType::BangEqual)) {
        Token op = advance();
        auto right = parseComparison();
        expr = std::make_shared<BinaryExpr>(expr, op.type, right);
    }
    return expr;
}

std::shared_ptr<Expression> Parser::parseComparison() {
    auto expr = parseTerm();
    while (check(TokenType::Greater) || check(TokenType::GreaterEqual) ||
           check(TokenType::Less) || check(TokenType::LessEqual)) {
        Token op = advance();
        auto right = parseTerm();
        expr = std::make_shared<BinaryExpr>(expr, op.type, right);
    }
    return expr;
}

std::shared_ptr<Expression> Parser::parseTerm() {
    auto expr = parseFactor();
    while (check(TokenType::Plus) || check(TokenType::Minus)) {
        Token op = advance();
        auto right = parseFactor();
        expr = std::make_shared<BinaryExpr>(expr, op.type, right);
    }
    return expr;
}

std::shared_ptr<Expression> Parser::parseFactor() {
    auto expr = parseUnary();
    while (check(TokenType::Slash) || check(TokenType::Star) || check(TokenType::Modulo)) {
        Token op = advance();
        auto right = parseUnary(); // Corrected recursion to be Unary
        expr = std::make_shared<BinaryExpr>(expr, op.type, right);
    }
    return expr;
}

std::shared_ptr<Expression> Parser::parseUnary() {
    // Handle await expression
    if (check(TokenType::Await)) {
        advance(); // consume 'await'
        auto expr = parseUnary();
        return std::make_shared<AwaitExpr>(expr);
    }
    if (check(TokenType::Typeof)) {
        advance();
        auto expr = parseUnary();
        return std::make_shared<TypeofExpr>(expr);
    }
    if (check(TokenType::Minus) || check(TokenType::Bang) || 
        check(TokenType::PlusPlus) || check(TokenType::MinusMinus)) {
        Token op = advance();
        auto right = parseUnary();
        if (op.type == TokenType::Minus) {
            return std::make_shared<BinaryExpr>(std::make_shared<NumberLiteral>(0), TokenType::Minus, right);
        }
        if (op.type == TokenType::Bang) {
            return std::make_shared<BinaryExpr>(right, TokenType::EqualEqual, std::make_shared<BooleanLiteral>(false));
        }
        if (op.type == TokenType::PlusPlus || op.type == TokenType::MinusMinus) {
            return std::make_shared<UnaryExpr>(op.type, right);
        }
    }
    return parseCall();
}

std::shared_ptr<Expression> Parser::parseCall() {
    auto expr = parsePrimary();
    while (true) {
        if (check(TokenType::OpenParen)) {
            advance();
            std::vector<std::shared_ptr<Expression>> args;
            if (!check(TokenType::CloseParen)) {
                do {
                    args.push_back(parseExpression());
                } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
            }
            consume(TokenType::CloseParen, "Expected ')'");
            
            if (auto id = std::dynamic_pointer_cast<Identifier>(expr)) {
                expr = std::make_shared<CallExpr>(id->name, args);
            } else if (auto mem = std::dynamic_pointer_cast<MemberAccessExpr>(expr)) {
                 if (auto innerId = std::dynamic_pointer_cast<Identifier>(mem->object)) {
                     expr = std::make_shared<CallExpr>(innerId->name + "." + mem->member, args);
                 } else if (std::dynamic_pointer_cast<ThisExpr>(mem->object)) {
                     expr = std::make_shared<CallExpr>("this." + mem->member, args);
                 }
            }
        } else if (check(TokenType::Dot)) {
             advance();
             auto prop = consume(TokenType::Identifier, "Expected property name.");
             expr = std::make_shared<MemberAccessExpr>(expr, prop.value);
        } else if (check(TokenType::QuestionDot)) {
            advance();
            if (check(TokenType::OpenBracket)) {
                advance();
                auto index = parseExpression();
                consume(TokenType::CloseBracket, "Expected ']' after index.");
                expr = std::make_shared<OptionalArrayAccessExpr>(expr, index);
            } else if (check(TokenType::OpenParen)) {
                advance();
                std::vector<std::shared_ptr<Expression>> args;
                if (!check(TokenType::CloseParen)) {
                    do {
                        args.push_back(parseExpression());
                    } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
                }
                consume(TokenType::CloseParen, "Expected ')'");
                expr = std::make_shared<OptionalCallExpr>(expr, args);
            } else {
                auto prop = consume(TokenType::Identifier, "Expected property name.");
                expr = std::make_shared<OptionalMemberAccessExpr>(expr, prop.value);
            }
        } else if (check(TokenType::OpenBracket)) {
             advance();
             auto index = parseExpression();
             consume(TokenType::CloseBracket, "Expected ']' after index.");
             expr = std::make_shared<ArrayAccessExpr>(expr, index);
        } else {
            break;
        }
    }
    return expr;
}

std::shared_ptr<Expression> Parser::parsePrimary() {
    if (check(TokenType::Undefined)) { advance(); return std::make_shared<UndefinedExpr>(); }
    if (check(TokenType::False)) { advance(); return std::make_shared<BooleanLiteral>(false); }
    if (check(TokenType::True)) { advance(); return std::make_shared<BooleanLiteral>(true); }
    if (check(TokenType::None)) { advance(); return std::make_shared<NoneExpr>(); }
    if (check(TokenType::Some)) {
        advance();
        consume(TokenType::OpenParen, "Expected '(' after 'Some'.");
        auto val = parseExpression();
        consume(TokenType::CloseParen, "Expected ')' after Some value.");
        return std::make_shared<SomeExpr>(val);
    }
    
    if (check(TokenType::Number)) {
        return std::make_shared<NumberLiteral>(std::stod(advance().value));
    }
    if (check(TokenType::String)) {
        return std::make_shared<StringLiteral>(advance().value);
    }
    if (check(TokenType::Identifier)) {
        if (peek(1).type == TokenType::Arrow) {
             return parseLambda();
        }
        auto name = advance().value;
        return std::make_shared<Identifier>(name);
    }
    if (check(TokenType::New)) {
        advance();
        std::string className = consume(TokenType::Identifier, "Expected class name.").value;
        consume(TokenType::OpenParen, "Expected '('");
        std::vector<std::shared_ptr<Expression>> args;
        if (!check(TokenType::CloseParen)) {
            do {
                args.push_back(parseExpression());
            } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
        }
        consume(TokenType::CloseParen, "Expected ')'");
        return std::make_shared<NewExpr>(className, args);
    }
    if (check(TokenType::This)) {
        advance();
        return std::make_shared<ThisExpr>();
    }
    if (check(TokenType::OpenParen)) {
        // Lambda Check: (args) => ...
        int i = 1;
        while (peek(i).type != TokenType::CloseParen && peek(i).type != TokenType::EndOfFile) {
            i++;
        }
        if (peek(i).type == TokenType::CloseParen && peek(i + 1).type == TokenType::Arrow) {
            return parseLambda();
        }

        advance();
        auto expr = parseExpression();
        consume(TokenType::CloseParen, "Expected ')'");
        return expr;
    }
    
    if (check(TokenType::OpenBracket)) {
        advance(); // consume [
        std::vector<std::shared_ptr<Expression>> elements;
        if (!check(TokenType::CloseBracket)) {
            do {
                // Check for spread operator
                if (check(TokenType::Ellipsis)) {
                    advance(); // consume ...
                    auto expr = parseExpression();
                    elements.push_back(std::make_shared<SpreadExpr>(expr));
                } else {
                    elements.push_back(parseExpression());
                }
            } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
        }
        consume(TokenType::CloseBracket, "Expected ']' after array elements.");
        return std::make_shared<ArrayLiteral>(elements);
    }
    
    if (check(TokenType::TemplateString)) {
        // Parse template literal with ${...} interpolations
        Token tok = advance();
        std::string raw = tok.value;
        std::vector<std::string> parts;
        std::vector<std::shared_ptr<Expression>> exprs;
        
        std::string currentPart;
        size_t i = 0;
        while (i < raw.length()) {
            if (i + 1 < raw.length() && raw[i] == '$' && raw[i+1] == '{') {
                parts.push_back(currentPart);
                currentPart = "";
                i += 2; // skip ${
                std::string exprStr;
                int braceCount = 1;
                while (i < raw.length() && braceCount > 0) {
                    if (raw[i] == '{') braceCount++;
                    else if (raw[i] == '}') braceCount--;
                    if (braceCount > 0) exprStr += raw[i];
                    i++;
                }
                // Parse the expression string
                Lexer exprLexer(exprStr);
                Parser exprParser(exprLexer.tokenize());
                exprs.push_back(exprParser.parseExpression());
            } else {
                currentPart += raw[i];
                i++;
            }
        }
        parts.push_back(currentPart);
        return std::make_shared<TemplateLiteralExpr>(parts, exprs);
    }
    
    if (check(TokenType::OpenBrace)) {
        return parseObjectLiteral();
    }
    
    if (check(TokenType::Match)) {
        return parseMatchExpression();
    }
    
    std::cerr << "Unexpected token: '" << peek().value << "' (Type: " << (int)peek().type << ") at line " << peek().line << std::endl;
    exit(1);
    throw std::runtime_error("Unexpected token.");
}

std::shared_ptr<Expression> Parser::parseLambda() {
    std::vector<LambdaExpr::Param> params;
    
    if (check(TokenType::Identifier)) {
        // Single param: x => ...
        std::string name = consume(TokenType::Identifier, "Expected param name.").value;
        params.push_back({name, ""}); 
    } else {
        // (x, y: number) => ...
        consume(TokenType::OpenParen, "Expected '('");
        if (!check(TokenType::CloseParen)) {
            do {
                std::string name = consume(TokenType::Identifier, "Expected param name.").value;
                std::string type = ""; 
                if (check(TokenType::Colon)) {
                    advance();
                    if (check(TokenType::TypeNumber)) { type="number"; advance(); }
                    else if (check(TokenType::TypeString)) { type="string"; advance(); }
                    else if (check(TokenType::TypeBoolean)) { type="boolean"; advance(); }
                    else { type = consume(TokenType::Identifier, "Type").value; }
                    
                    while (check(TokenType::OpenBracket)) {
                        consume(TokenType::OpenBracket, "["); consume(TokenType::CloseBracket, "]");
                        type += "[]";
                    }
                }
                params.push_back({name, type});
            } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
        }
        consume(TokenType::CloseParen, "Expected ')'");
    }
    
    consume(TokenType::Arrow, "Expected '=>'");
    
    std::shared_ptr<Statement> body;
    if (check(TokenType::OpenBrace)) {
        body = parseBlock();
    } else {
        // Expression lambda: x => x + 1
        auto expr = parseExpression();
        auto ret = std::make_shared<ReturnStmt>(expr);
        auto block = std::make_shared<BlockStmt>();
        block->statements.push_back(ret);
        body = block;
    }
    
    return std::make_shared<LambdaExpr>(params, body);
}

std::shared_ptr<ProtocolDeclaration> Parser::parseProtocolDeclaration() {
    consume(TokenType::Protocol, "Expected 'protocol'");
    std::string name = consume(TokenType::Identifier, "Expected protocol name").value;
    consume(TokenType::OpenBrace, "Expected '{'");
    
    std::vector<ProtocolDeclaration::Method> methods;
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        std::string methodName = consume(TokenType::Identifier, "Expected method name").value;
        consume(TokenType::OpenParen, "Expected '('");
        
        std::vector<Parameter> params;
        if (!check(TokenType::CloseParen)) {
            do {
                std::string pName = consume(TokenType::Identifier, "Param name").value;
                consume(TokenType::Colon, "Expected ':'");
                std::string pType = parseType();
                params.push_back({pName, pType, nullptr, false});
            } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
        }
        consume(TokenType::CloseParen, "Expected ')'");
        consume(TokenType::Colon, "Expected ':'");
        std::string returnType = parseType();
        consume(TokenType::Semicolon, "Expected ';'");
        
        methods.push_back({methodName, params, returnType});
    }
    consume(TokenType::CloseBrace, "Expected '}'");
    return std::make_shared<ProtocolDeclaration>(name, methods);
}

std::shared_ptr<ExtensionDeclaration> Parser::parseExtensionDeclaration() {
    consume(TokenType::Extension, "Expected 'extension'");
    std::string target = consume(TokenType::Identifier, "Expected type name to extend").value;
    
    consume(TokenType::OpenBrace, "Expected '{'");
    
    std::vector<std::shared_ptr<FunctionDeclaration>> methods;
    while (!check(TokenType::CloseBrace) && !isAtEnd()) {
        std::string name = consume(TokenType::Identifier, "Expected method name").value;
        consume(TokenType::OpenParen, "Expected '('");
        std::vector<Parameter> params;
        if (!check(TokenType::CloseParen)) {
            do {
                std::string pName = consume(TokenType::Identifier, "Param name").value;
                std::string pType = "tejx_runtime::Var";
                if (check(TokenType::Colon)) {
                     advance();
                     pType = parseType();
                }
                params.push_back({pName, pType, nullptr, false}); 
            } while (check(TokenType::Comma) && advance().type == TokenType::Comma);
        }
        consume(TokenType::CloseParen, "Expected ')'");
        
        std::string returnType = "void";
        if (check(TokenType::Colon)) {
            advance();
            returnType = parseType();
        }
        
        // Do NOT consume OpenBrace here, parseBlock does it.
        std::shared_ptr<Statement> bodyStmt = parseBlock();
        std::shared_ptr<BlockStmt> body = std::static_pointer_cast<BlockStmt>(bodyStmt);
        
        methods.push_back(std::shared_ptr<FunctionDeclaration>(new FunctionDeclaration(name, params, returnType, body, false)));
    }
    consume(TokenType::CloseBrace, "Expected '}'");
    return std::make_shared<ExtensionDeclaration>(target, methods);
}

} // namespace tejx
