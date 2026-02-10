#pragma once

#include <string>
#include <string_view>
#include <vector>
#include <ostream>

namespace tejx {

enum class TokenType {
    // Keywords
    Function, Return, Var, Let, Const, If, Else, While, For, Break, Continue,
    Class, New, This, Super, Constructor, Extends, Abstract, Switch, Case, Default, Import, Export, From, Instanceof,
    Public, Private, Protected, Static, Async, Await, // OOP Modifiers
    Protocol, Implements, Extension, // Interfaces
    Try, Catch, Finally, Throw, Match, Undefined, // Error handling & Types
    Some, None, Option, // Option types
    Get, Set, // Property accessors
    // Primitives
    TypeNumber, TypeString, TypeBoolean, TypeVoid, TypeAny,
    TypeInt, TypeFloat, TypeBigInt, TypeBigFloat,
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
    Lexer(std::string_view source);
    std::vector<Token> tokenize();

private:
    std::string_view source;
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
