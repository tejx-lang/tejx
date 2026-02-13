use crate::token::{Token, TokenType};
use std::collections::HashMap;

pub struct Lexer {
    source: Vec<char>,
    position: usize,
    line: usize,
    column: usize,
    keywords: HashMap<String, TokenType>,
    pub errors: Vec<crate::diagnostics::Diagnostic>,
    filename: String,
}

impl Lexer {
    pub fn new(source: &str, filename: &str) -> Self {
        let mut keywords = HashMap::new();
        keywords.insert("function".to_string(), TokenType::Function);
        keywords.insert("let".to_string(), TokenType::Let);
        keywords.insert("const".to_string(), TokenType::Const);
        keywords.insert("return".to_string(), TokenType::Return);
        keywords.insert("if".to_string(), TokenType::If);
        keywords.insert("else".to_string(), TokenType::Else);
        keywords.insert("while".to_string(), TokenType::While);
        keywords.insert("for".to_string(), TokenType::For);
        keywords.insert("break".to_string(), TokenType::Break);
        keywords.insert("continue".to_string(), TokenType::Continue);
        keywords.insert("switch".to_string(), TokenType::Switch);
        keywords.insert("case".to_string(), TokenType::Case);
        keywords.insert("default".to_string(), TokenType::Default);
        keywords.insert("extends".to_string(), TokenType::Extends);
        keywords.insert("number".to_string(), TokenType::TypeNumber);
        keywords.insert("string".to_string(), TokenType::TypeString);
        keywords.insert("boolean".to_string(), TokenType::TypeBoolean);
        keywords.insert("void".to_string(), TokenType::TypeVoid);
        keywords.insert("any".to_string(), TokenType::TypeAny);
        keywords.insert("int".to_string(), TokenType::TypeInt);
        keywords.insert("float".to_string(), TokenType::TypeFloat);
        keywords.insert("bigInt".to_string(), TokenType::TypeBigInt);
        keywords.insert("bigfloat".to_string(), TokenType::TypeBigFloat);
        keywords.insert("true".to_string(), TokenType::True);
        keywords.insert("false".to_string(), TokenType::False);
        keywords.insert("class".to_string(), TokenType::Class);
        keywords.insert("new".to_string(), TokenType::New);
        keywords.insert("this".to_string(), TokenType::This);
        keywords.insert("constructor".to_string(), TokenType::Constructor);
        keywords.insert("super".to_string(), TokenType::Super);
        keywords.insert("public".to_string(), TokenType::Public);
        keywords.insert("private".to_string(), TokenType::Private);
        keywords.insert("protected".to_string(), TokenType::Protected);
        keywords.insert("abstract".to_string(), TokenType::Abstract);
        keywords.insert("protocol".to_string(), TokenType::Protocol);
        keywords.insert("implements".to_string(), TokenType::Implements);
        keywords.insert("extension".to_string(), TokenType::Extension);
        keywords.insert("static".to_string(), TokenType::Static);
        keywords.insert("async".to_string(), TokenType::Async);
        keywords.insert("await".to_string(), TokenType::Await);
        keywords.insert("try".to_string(), TokenType::Try);
        keywords.insert("catch".to_string(), TokenType::Catch);
        keywords.insert("finally".to_string(), TokenType::Finally);
        keywords.insert("throw".to_string(), TokenType::Throw);
        keywords.insert("typeof".to_string(), TokenType::Typeof);
        keywords.insert("match".to_string(), TokenType::Match);
        keywords.insert("enum".to_string(), TokenType::Enum);
        keywords.insert("undefined".to_string(), TokenType::Undefined);
        keywords.insert("null".to_string(), TokenType::Undefined);
        keywords.insert("Some".to_string(), TokenType::Some);
        keywords.insert("None".to_string(), TokenType::None);
        keywords.insert("Option".to_string(), TokenType::Option);
        keywords.insert("import".to_string(), TokenType::Import);
        keywords.insert("export".to_string(), TokenType::Export);
        keywords.insert("from".to_string(), TokenType::From);
        keywords.insert("instanceof".to_string(), TokenType::Instanceof);
        keywords.insert("get".to_string(), TokenType::Get);
        keywords.insert("set".to_string(), TokenType::Set);
        keywords.insert("type".to_string(), TokenType::TypeAlias);
        keywords.insert("interface".to_string(), TokenType::Interface);
        keywords.insert("to".to_string(), TokenType::To);
        keywords.insert("of".to_string(), TokenType::Of);

        Self {
            source: source.chars().collect(),
            position: 0,
            line: 1,
            column: 1,
            keywords,
            errors: Vec::new(),
            filename: filename.to_string(),
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace();
            if self.is_at_end() {
                break;
            }

            let c = self.peek(0);
            let start_col = self.column;

            if c.is_alphabetic() || c == '_' {
                tokens.push(self.read_identifier());
            } else if c.is_digit(10) {
                tokens.push(self.read_number());
            } else if c == '"' || c == '\'' {
                tokens.push(self.read_string(c));
            } else if c == '`' {
                tokens.push(self.read_template_string());
            } else {
                let token_type = match c {
                    '+' => {
                        if self.peek(1) == '=' {
                            self.advance();
                            TokenType::PlusEquals
                        } else if self.peek(1) == '+' {
                            self.advance();
                            TokenType::PlusPlus
                        } else {
                            TokenType::Plus
                        }
                    }
                    '-' => {
                        if self.peek(1) == '=' {
                            self.advance();
                            TokenType::MinusEquals
                        } else if self.peek(1) == '-' {
                            self.advance();
                            TokenType::MinusMinus
                        } else {
                            TokenType::Minus
                        }
                    }
                    '*' => {
                        if self.peek(1) == '=' {
                            self.advance();
                            TokenType::StarEquals
                        } else {
                            TokenType::Star
                        }
                    }
                    '/' => {
                         if self.peek(1) == '=' {
                            self.advance();
                            TokenType::SlashEquals
                        } else {
                            TokenType::Slash
                        }
                    }
                    '%' => TokenType::Modulo,
                    '=' => {
                        if self.peek(1) == '=' {
                            self.advance();
                            if self.peek(1) == '=' {
                                self.advance();
                            }
                            TokenType::EqualEqual
                        } else if self.peek(1) == '>' {
                            self.advance();
                            TokenType::Arrow
                        } else {
                            TokenType::Equals
                        }
                    }
                    '!' => {
                         if self.peek(1) == '=' {
                            self.advance();
                             if self.peek(1) == '=' {
                                self.advance();
                            }
                            TokenType::BangEqual
                        } else {
                            TokenType::Bang
                        }
                    }
                    '<' => {
                         if self.peek(1) == '=' {
                             self.advance();
                             TokenType::LessEqual
                         } else {
                             TokenType::Less
                         }
                    }
                    '>' => {
                        if self.peek(1) == '=' {
                            self.advance();
                            TokenType::GreaterEqual
                        } else {
                            TokenType::Greater
                        }
                    }
                    '.' => {
                        if self.peek(1) == '.' && self.peek(2) == '.' {
                            self.advance();
                            self.advance();
                            TokenType::Ellipsis
                        } else {
                            TokenType::Dot
                        }
                    }
                    '(' => TokenType::OpenParen,
                    ')' => TokenType::CloseParen,
                    '{' => TokenType::OpenBrace,
                    '}' => TokenType::CloseBrace,
                    '[' => TokenType::OpenBracket,
                    ']' => TokenType::CloseBracket,
                    ':' => {
                        if self.peek(1) == ':' {
                            self.advance();
                            TokenType::DoubleColon
                        } else {
                            TokenType::Colon
                        }
                    }
                    ';' => TokenType::Semicolon,
                    ',' => TokenType::Comma,
                    '?' => {
                        if self.peek(1) == '.' {
                            self.advance();
                            TokenType::QuestionDot
                        } else if self.peek(1) == '?' {
                             self.advance();
                             TokenType::QuestionQuestion
                        } else {
                            TokenType::Question
                        }
                    }
                    '&' => {
                        if self.peek(1) == '&' {
                            self.advance();
                            TokenType::AmpersandAmpersand
                        } else {
                            TokenType::Unknown
                        }
                    }
                    '|' => {
                        if self.peek(1) == '|' {
                            self.advance();
                            TokenType::PipePipe
                        } else {
                            TokenType::Unknown
                        }
                    }
                    _ => TokenType::Unknown,
                };
                
                let value = if token_type == TokenType::Unknown {
                    c.to_string()
                } else {
                    // Logic to extract value based on what we advanced? 
                    // To keep it simple, we can reconstruct or just store the char c
                    // But for multi-char operators we need the full string.
                    // For now let's just use the current char as value, or better, 
                    // we should probably just store the representation if needed.
                    c.to_string() 
                };
                 
                // Advance past the current char (we may have advanced more inside match)
                self.advance();
                
                // TODO: Fix value for multi-char tokens
                // In C++ it did: std::string val(1, c); ... val = "+=";
                // I should replicate that logic to be correct.
                
                let actual_value = match token_type {
                     TokenType::PlusEquals => "+=",
                     TokenType::PlusPlus => "++",
                     TokenType::MinusEquals => "-=",
                     TokenType::MinusMinus => "--",
                     TokenType::StarEquals => "*=",
                     TokenType::SlashEquals => "/=",
                     TokenType::Arrow => "=>",
                     TokenType::EqualEqual => "==", // or ===
                     TokenType::BangEqual => "!=", // or !==
                     TokenType::LessEqual => "<=",
                     TokenType::GreaterEqual => ">=",
                     TokenType::Ellipsis => "...",
                     TokenType::QuestionDot => "?.",
                     TokenType::QuestionQuestion => "??",
                     TokenType::AmpersandAmpersand => "&&",
                     TokenType::PipePipe => "||",
                     _ => {
                         // Convert char to string
                          // This is a bit hacky because `value` above intialized with `c`
                          // and we advanced. ensuring we get the right string.
                          // Let's just return the char as string for single chars.
                          // It's already in `value`.
                          // But wait, constructing `value` correctly is key.
                          // I'll fix this in a cleanup pass or now.
                          // Let's assume single char for now for others.
                          ""
                     }
                };
                
                let final_value = if actual_value.is_empty() { value } else { actual_value.to_string() };

                tokens.push(Token::new(token_type, final_value, self.line, start_col));
            }
        }

        tokens.push(Token::new(TokenType::EndOfFile, "".to_string(), self.line, self.column));
        tokens
    }

    fn peek(&self, offset: usize) -> char {
         if self.position + offset >= self.source.len() {
            '\0'
        } else {
            self.source[self.position + offset]
        }
    }

    fn advance(&mut self) -> char {
        let current = self.peek(0);
        self.position += 1;
        if current == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        current
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.source.len()
    }

    fn skip_whitespace(&mut self) {
        while !self.is_at_end() {
            let c = self.peek(0);
            if c == ' ' || c == '\r' || c == '\t' || c == '\n' {
                self.advance();
            } else if c == '/' {
                if self.peek(1) == '/' {
                     while self.peek(0) != '\n' && !self.is_at_end() {
                        self.advance();
                    }
                } else {
                    return;
                }
            } else {
                break;
            }
        }
    }

    fn read_identifier(&mut self) -> Token {
        let start_col = self.column;
        let mut text = String::new();
        while !self.is_at_end() && (self.peek(0).is_alphanumeric() || self.peek(0) == '_') {
            text.push(self.advance());
        }

        let token_type = self.keywords.get(&text).cloned().unwrap_or(TokenType::Identifier);
        Token::new(token_type, text, self.line, start_col)
    }

    fn read_number(&mut self) -> Token {
        let start_col = self.column;
        let mut value = String::new();
        while !self.is_at_end() && self.peek(0).is_digit(10) {
            value.push(self.advance());
        }

        if self.peek(0) == '.' && self.peek(1).is_digit(10) {
            value.push(self.advance());
            while !self.is_at_end() && self.peek(0).is_digit(10) {
                value.push(self.advance());
            }
        }

        Token::new(TokenType::Number, value, self.line, start_col)
    }

    fn read_string(&mut self, quote: char) -> Token {
        let start_col = self.column;
        let start_line = self.line;
        self.advance(); // Skip opening quote
        let mut value = String::new();
        let mut is_escaped = false;

        while !self.is_at_end() {
            let c = self.peek(0);
            if is_escaped {
                value.push(self.advance());
                is_escaped = false;
            } else if c == '\\' {
                is_escaped = true;
                self.advance();
            } else if c == quote {
                break;
            } else if c == '\n' {
                 // Error: Unclosed string (newline in string not allowed without escaping usually)
                 // But we can choose to allow it and just report error to continue lexing.
                 self.errors.push(crate::diagnostics::Diagnostic::new(
                    format!("Unclosed string literal starting with {}", quote),
                    start_line,
                    start_col,
                    self.filename.clone()
                 ));
                 // Break to avoid gobbling the whole file into one string if possible? 
                 // Actually, let's just break and treat the rest as new tokens.
                 break;
            } else {
                value.push(self.advance());
            }
        }
        
        if self.is_at_end() || self.peek(0) != quote {
            self.errors.push(crate::diagnostics::Diagnostic::new(
                format!("Unclosed string literal starting with {}", quote),
                start_line,
                start_col,
                self.filename.clone()
            ));
        } else {
            self.advance(); // Skip closing quote
        }
        
        Token::new(TokenType::String, value, start_line, start_col)
    }

    fn read_template_string(&mut self) -> Token {
         let start_col = self.column;
         self.advance(); // Skip `
         let mut value = String::new();
         // Basic template string support - skipping deep interpolation logic for now to get minimal working version
         // matching C++ logic but simplified for first pass. 
         // Actually, let's implement the brace counting if possible.
         
         let mut brace_depth = 0;
         let mut in_interpolation = false;

         while !self.is_at_end() {
             if self.peek(0) == '`' && !in_interpolation {
                 break;
             }
             
             if self.peek(0) == '$' && self.peek(1) == '{' && !in_interpolation {
                 in_interpolation = true;
                 brace_depth = 1;
                 value.push(self.advance()); // $
                 value.push(self.advance()); // {
             } else if in_interpolation {
                 if self.peek(0) == '{' {
                     brace_depth += 1;
                 } else if self.peek(0) == '}' {
                     brace_depth -= 1;
                     if brace_depth == 0 {
                         in_interpolation = false;
                     }
                 }
                 value.push(self.advance());
             } else {
                 value.push(self.advance());
             }
         }
         
         if !self.is_at_end() {
             self.advance();
         }
         
         Token::new(TokenType::TemplateString, value, self.line, start_col)
    }
}
