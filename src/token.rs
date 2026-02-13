
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    // Keywords
    Function, Return, Let, Const, If, Else, While, For, Break, Continue,
    Class, New, This, Super, Constructor, Extends, Abstract, Switch, Case, Default, Import, Export, From, Instanceof,
    Public, Private, Protected, Static, Async, Await, // OOP Modifiers
    Protocol, Implements, Extension, // Interfaces
    Try, Catch, Finally, Throw, Match, Undefined, // Error handling & Types
    Some, None, Option, // Option types
    Get, Set, TypeAlias, Interface, // Property accessors & Type defs
    // Primitives
    TypeNumber, TypeString, TypeBoolean, TypeVoid, TypeAny,
    TypeInt, TypeFloat, TypeBigInt, TypeBigFloat,
    // Literals
    Identifier, Number, String, TemplateString, True, False,
    // Symbols
    Plus, Minus, Star, Slash, Modulo, Equals,
    Dot, Comma, Colon, DoubleColon, Semicolon, Ellipsis,
    OpenParen, CloseParen, OpenBrace, CloseBrace, OpenBracket, CloseBracket,
    // Logic & Comparison
    EqualEqual, Bang, BangEqual, Less, LessEqual, Greater, GreaterEqual,
    AmpersandAmpersand, PipePipe,
    // Assignment & Unary Sugar
    PlusEquals, MinusEquals, StarEquals, SlashEquals,
    PlusPlus, MinusMinus,
    // Control
    Question, QuestionDot, QuestionQuestion, Arrow, Typeof, Enum,
    // Contextual keywords
    To, Of,
    // OOP
    EndOfFile, Unknown
}

#[derive(Debug, Clone)]
pub struct Token {
    pub token_type: TokenType,
    pub value: String,
    pub line: usize,
    pub column: usize,
}

impl Token {
    pub fn new(token_type: TokenType, value: String, line: usize, column: usize) -> Self {
        Self {
            token_type,
            value,
            line,
            column,
        }
    }
}
