#pragma once

#include <string>
#include <vector>
#include <ostream>

namespace tejx {

enum class TokenType {
    // Keywords
    Function, Return, Var, Let, Const, If, Else, While, For, Break, Continue,
    Class, New, This, Constructor, Extends, Switch, Case, Default, Import, Export, From,
    Public, Private, Protected, Static, Async, Await, // OOP Modifiers
    Protocol, Implements, Extension, // Interfaces
    Try, Catch, Finally, Throw, Match, Undefined, // Error handling & Types
    Some, None, Option, // Option types
    Get, Set, // Property accessors
    // Types
    TypeNumber, TypeString, TypeBoolean, TypeVoid,
    // Literals
    Identifier, Number, String, TemplateString, True, False,
    // Symbols
    Plus, Minus, Star, Slash, Modulo, Equals,
    Dot, Comma, Colon, Semicolon, Ellipsis,
    OpenParen, CloseParen, OpenBrace, CloseBrace, OpenBracket, CloseBracket,
    // Logic & Comparison
    EqualEqual, Bang, BangEqual, Less, LessEqual, Greater, GreaterEqual,
    AmpersandAmpersand, PipePipe,
    // Assignment & Unary Sugar
    PlusEquals, MinusEquals, StarEquals, SlashEquals,
    PlusPlus, MinusMinus,
    // Control
    Question, QuestionDot, QuestionQuestion, Arrow, Typeof, Enum,
    // OOP
    EndOfFile, Unknown
};

struct Token {
    TokenType type;
    std::string value;
    int line;
    int column;
};

class Lexer {
public:
    Lexer(const std::string& source);
    std::vector<Token> tokenize();

private:
    std::string source;
    size_t position = 0;
    int line = 1;
    int column = 1;

    char peek(int offset = 0) const;
    char advance();
    bool isAtEnd() const;
    void skipWhitespace();
    
    Token readIdentifier();
    Token readNumber();
    Token readString();
};

std::string tokenTypeToString(TokenType type);

} // namespace tejx
