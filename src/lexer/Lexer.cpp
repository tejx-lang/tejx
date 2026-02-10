#include "tejx/Lexer.h"
#include <cctype>
#include <unordered_map>

namespace tejx {

Lexer::Lexer(const std::string& source) : source(source) {}

char Lexer::peek(int offset) const {
    if (position + offset >= source.length()) return '\0';
    return source[position + offset];
}

char Lexer::advance() {
    char current = peek();
    position++;
    if (current == '\n') {
        line++;
        column = 1;
    } else {
        column++;
    }
    return current;
}

bool Lexer::isAtEnd() const {
    return position >= source.length();
}

void Lexer::skipWhitespace() {
    while (!isAtEnd()) {
        char c = peek();
        if (c == ' ' || c == '\r' || c == '\t' || c == '\n') {
            advance();
        } else if (c == '/') {
            if (peek(1) == '/') {
                // Single-line comment
                while (peek() != '\n' && !isAtEnd()) advance();
            } else {
                return;
            }
        } else {
            break;
        }
    }
}

Token Lexer::readIdentifier() {
    std::string value;
    int startCol = column;
    while (!isAtEnd() && (isalnum(peek()) || peek() == '_')) {
        value += advance();
    }

    TokenType type = TokenType::Identifier;
    static const std::unordered_map<std::string, TokenType> keywords = {
        {"function", TokenType::Function},
        {"let", TokenType::Let},
        {"const", TokenType::Const},
        {"return", TokenType::Return},
        {"if", TokenType::If},
        {"else", TokenType::Else},
        {"while", TokenType::While},
        {"for", TokenType::For},
        {"break", TokenType::Break},
        {"continue", TokenType::Continue},
        {"switch", TokenType::Switch},
        {"case", TokenType::Case},
        {"default", TokenType::Default},
        {"extends", TokenType::Extends},
        {"number", TokenType::TypeNumber},
        {"string", TokenType::TypeString},
        {"boolean", TokenType::TypeBoolean},
        {"void", TokenType::TypeVoid},
        {"any", TokenType::TypeAny},
        {"int", TokenType::TypeInt},
        {"float", TokenType::TypeFloat},
        {"bigInt", TokenType::TypeBigInt},
        {"bigfloat", TokenType::TypeBigFloat},
        {"true", TokenType::True},
        {"false", TokenType::False},
        {"class", TokenType::Class},
        {"new", TokenType::New},
        {"this", TokenType::This},
        {"constructor", TokenType::Constructor},
        {"super", TokenType::Super},
        {"public", TokenType::Public},
        {"private", TokenType::Private},
        {"protected", TokenType::Protected},
        {"abstract", TokenType::Abstract},
        {"protocol", TokenType::Protocol},
        {"implements", TokenType::Implements},
        {"extension", TokenType::Extension},
        {"static", TokenType::Static},
        {"async", TokenType::Async},
        {"await", TokenType::Await},
        {"try", TokenType::Try},
        {"catch", TokenType::Catch},
        {"finally", TokenType::Finally},
        {"throw", TokenType::Throw},
        {"typeof", TokenType::Typeof},
        {"match", TokenType::Match},
        {"enum", TokenType::Enum},
        {"undefined", TokenType::Undefined},
        {"null", TokenType::Undefined},
        {"Some", TokenType::Some},
        {"None", TokenType::None},
        {"Option", TokenType::Option},
        {"import", TokenType::Import},
        {"export", TokenType::Export},
        {"from", TokenType::From}
    };

    if (keywords.count(value)) {
        type = keywords.at(value);
    }

    return {type, value, line, startCol};
}

Token Lexer::readNumber() {
    std::string value;
    int startCol = column;
    while (!isAtEnd() && isdigit(peek())) {
        value += advance();
    }
    
    // Look for fractional part
    if (peek() == '.' && isdigit(peek(1))) {
        value += advance(); // Consume .
        while (!isAtEnd() && isdigit(peek())) {
            value += advance();
        }
    }
    
    return {TokenType::Number, value, line, startCol};
}

Token Lexer::readString() {
    int startCol = column;
    advance(); // Skip opening quote
    std::string value;
    while (!isAtEnd() && peek() != '"') {
        value += advance();
    }
    advance(); // Skip closing quote
    return {TokenType::String, value, line, startCol};
}

std::vector<Token> Lexer::tokenize() {
    std::vector<Token> tokens;

    while (!isAtEnd()) {
        skipWhitespace();
        if (isAtEnd()) break;

        char c = peek();
        int startCol = column;

        if (isalpha(c) || c == '_') {
            tokens.push_back(readIdentifier());
        } else if (isdigit(c)) {
            tokens.push_back(readNumber());
        } else if (c == '"') {
            tokens.push_back(readString());
        } else if (c == '`') {
            // Template literal - need to track interpolation depth to find true closing backtick
            int sCol = column;
            advance(); // skip `
            std::string value;
            int braceDepth = 0;
            bool inInterpolation = false;
            while (!isAtEnd()) {
                if (peek() == '`' && !inInterpolation) break;
                
                if (peek() == '$' && peek(1) == '{' && !inInterpolation) {
                    inInterpolation = true;
                    braceDepth = 1;
                    value += advance(); // $
                    value += advance(); // {
                } else if (inInterpolation) {
                    if (peek() == '{') braceDepth++;
                    else if (peek() == '}') {
                        braceDepth--;
                        if (braceDepth == 0) inInterpolation = false;
                    }
                    value += advance();
                } else {
                    value += advance();
                }
            }
            if (!isAtEnd()) advance(); // skip closing `
            tokens.push_back({TokenType::TemplateString, value, line, sCol});
        } else {
            // Single char tokens
            TokenType type = TokenType::Unknown;
            std::string val(1, c);
            advance();

            switch (c) {
                case '+':
                    if (peek() == '=') { advance(); type = TokenType::PlusEquals; val = "+="; }
                    else if (peek() == '+') { advance(); type = TokenType::PlusPlus; val = "++"; }
                    else type = TokenType::Plus;
                    break;
                case '-':
                    if (peek() == '=') { advance(); type = TokenType::MinusEquals; val = "-="; }
                    else if (peek() == '-') { advance(); type = TokenType::MinusMinus; val = "--"; }
                    else type = TokenType::Minus;
                    break;
                case '*':
                    type = (peek() == '=') ? (advance(), TokenType::StarEquals) : TokenType::Star;
                    break;
                case '/':
                    // Check for comments first? consume(Slash) logic might be tricky if we don't peek properly.
                    // Existing logic for comments skips before this switch.
                    type = (peek() == '=') ? (advance(), TokenType::SlashEquals) : TokenType::Slash;
                    break;
                case '%': type = TokenType::Modulo; break;
                case '=': 
                    if (peek() == '=') {
                        advance();
                        if (peek() == '=') advance(); // Handle ===
                        type = TokenType::EqualEqual;
                    }
                    else if (peek() == '>') { advance(); type = TokenType::Arrow; val = "=>"; }
                    else type = TokenType::Equals;
                    break;
                case '!':
                    if (peek() == '=') {
                        advance();
                        if (peek() == '=') advance(); // Handle !==
                        type = TokenType::BangEqual;
                    } else {
                        type = TokenType::Bang;
                    }
                    break;
                case '<':
                    type = (peek() == '=') ? (advance(), TokenType::LessEqual) : TokenType::Less;
                    break;
                case '>':
                    type = (peek() == '=') ? (advance(), TokenType::GreaterEqual) : TokenType::Greater;
                    break;
                case '.': 
                    if (peek() == '.' && peek(1) == '.') { 
                        advance(); advance(); 
                        type = TokenType::Ellipsis; 
                        val = "..."; 
                    } else { 
                        type = TokenType::Dot; 
                    }
                    break;
                case '(': type = TokenType::OpenParen; break;
                case ')': type = TokenType::CloseParen; break;
                case '{': type = TokenType::OpenBrace; break;
                case '}': type = TokenType::CloseBrace; break;
                case '[': type = TokenType::OpenBracket; break;
                case ']': type = TokenType::CloseBracket; break;
                case ':': type = TokenType::Colon; break;
                case '?': 
                    if (peek() == '.') { advance(); type = TokenType::QuestionDot; val = "?."; }
                    else if (peek() == '?') { advance(); type = TokenType::QuestionQuestion; val = "??"; }
                    else type = TokenType::Question; 
                    break;
                case ';': type = TokenType::Semicolon; break;
                case ',': type = TokenType::Comma; break;
                case '&':
                    if (peek() == '&') { advance(); type = TokenType::AmpersandAmpersand; val = "&&"; }
                    else type = TokenType::Unknown; 
                    break;
                case '|':
                    if (peek() == '|') { advance(); type = TokenType::PipePipe; val = "||"; }
                    else type = TokenType::Unknown;
                    break;
            }
            tokens.push_back({type, val, line, startCol});
        }
    }

    tokens.push_back({TokenType::EndOfFile, "", line, column});
    return tokens;
}

std::string tokenTypeToString(TokenType type) {
    switch (type) {
        case TokenType::Function: return "Function";
        case TokenType::Let: return "Let";
        case TokenType::Const: return "Const";
        case TokenType::Return: return "Return";
        case TokenType::TypeNumber: return "TypeNumber";
        case TokenType::TypeString: return "TypeString";
        case TokenType::TypeBoolean: return "TypeBoolean";
        case TokenType::TypeVoid: return "TypeVoid";
        case TokenType::If: return "If";
        case TokenType::Else: return "Else";
        case TokenType::While: return "While";
        case TokenType::For: return "For";
        case TokenType::Break: return "Break";
        case TokenType::Continue: return "Continue";
        case TokenType::Switch: return "Switch";
        case TokenType::Case: return "Case";
        case TokenType::Default: return "Default";
        case TokenType::Extends: return "Extends";
        case TokenType::Question: return "Question";
        case TokenType::Arrow: return "Arrow";
        case TokenType::Public: return "Public";
        case TokenType::Private: return "Private";
        case TokenType::Protected: return "Protected";
        case TokenType::Abstract: return "Abstract";
        case TokenType::Static: return "Static";
        case TokenType::Identifier: return "Identifier";
        case TokenType::Number: return "NumberLiteral";
        case TokenType::String: return "StringLiteral";
        case TokenType::True: return "True";
        case TokenType::False: return "False";
        case TokenType::Plus: return "Plus";
        case TokenType::Minus: return "Minus";
        case TokenType::Star: return "Star";
        case TokenType::Slash: return "Slash";
        case TokenType::Modulo: return "Modulo";
        case TokenType::Dot: return "Dot";
        case TokenType::Equals: return "Equals";
        case TokenType::EqualEqual: return "EqualEqual";
        case TokenType::Bang: return "Bang";
        case TokenType::BangEqual: return "BangEqual";
        case TokenType::Less: return "Less";
        case TokenType::LessEqual: return "LessEqual";
        case TokenType::Greater: return "Greater";
        case TokenType::GreaterEqual: return "GreaterEqual";
        case TokenType::OpenParen: return "OpenParen";
        case TokenType::CloseParen: return "CloseParen";
        case TokenType::OpenBrace: return "OpenBrace";
        case TokenType::CloseBrace: return "CloseBrace";
        case TokenType::Colon: return "Colon";
        case TokenType::Semicolon: return "Semicolon";
        case TokenType::Comma: return "Comma";
        case TokenType::EndOfFile: return "EOF";
        case TokenType::Class: return "Class";
        case TokenType::New: return "New";
        case TokenType::This: return "This";
        case TokenType::Super: return "Super";
        case TokenType::Constructor: return "Constructor";
        case TokenType::Protocol: return "Protocol";
        case TokenType::Implements: return "Implements";
        case TokenType::AmpersandAmpersand: return "&&";
        case TokenType::PipePipe: return "||";
        case TokenType::Ellipsis: return "Ellipsis";
        case TokenType::Async: return "Async";
        case TokenType::Await: return "Await";
        case TokenType::Match: return "Match";
        case TokenType::Unknown: return "Unknown";
        default: return "Token";
    }
}

} // namespace tejx
