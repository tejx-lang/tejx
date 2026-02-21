
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    // Keywords
    Function, Return, Let, Const, If, Else, While, For, Break, Continue,
    Class, New, This, Super, Constructor, Extends, Implements, Extension, Abstract, Switch, Case, Default, Import, Export, From, Instanceof,
    Public, Private, Protected, Static, Async, Await, // OOP Modifiers
    Try, Catch, Finally, Throw, // Error handling
    Some, None, Option, // Option types
    Get, Set, Interface, TypeAlias, // Property accessors
    // Primitives
    TypeString, TypeBoolean, TypeVoid, TypeAny,
    TypeInt, TypeInt16, TypeInt64, TypeInt128,
    TypeFloat, TypeFloat16, TypeFloat64,
    TypeChar,
    // Literals
    Identifier, Number, String, TemplateString, True, False,
    // Symbols
    Plus, Minus, Star, Slash, Modulo, Equals,
    Dot, Comma, Colon, DoubleColon, Semicolon, Ellipsis,
    OpenParen, CloseParen, OpenBrace, CloseBrace, OpenBracket, CloseBracket,
    // Logic & Comparison
    EqualEqual, EqualEqualEqual, Bang, BangEqual, BangEqualEqual, Less, LessEqual, Greater, GreaterEqual,
    AmpersandAmpersand, PipePipe,
    Ampersand, Pipe, Caret, Tilde, LessLess, GreaterGreater,
    // Assignment & Unary Sugar
    PlusEquals, MinusEquals, StarEquals, SlashEquals, ModuloEquals,
    AmpersandEquals, PipeEquals, CaretEquals, LessLessEquals, GreaterGreaterEquals,
    PlusPlus, MinusMinus,
    // Control
    Question, QuestionDot, QuestionQuestion, Arrow, Enum,
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
