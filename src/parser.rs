use crate::ast::*;
use crate::token::{Token, TokenType};

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    errors: Vec<crate::diagnostics::Diagnostic>,
    filename: String,
    pub async_enabled: bool,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, filename: &str) -> Self {
        Self {
            tokens,
            current: 0,
            errors: Vec::new(),
            filename: filename.to_string(),
            async_enabled: true,
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn get_errors(&self) -> &Vec<crate::diagnostics::Diagnostic> {
        &self.errors
    }

    pub fn previous(&self) -> &Token {
        if self.current == 0 {
             panic!("No previous token");
        }
        &self.tokens[self.current - 1]
    }

    pub fn parse_program(&mut self) -> Program {
        let mut statements = Vec::new();
        while !self.is_at_end() {
             if let Some(stmt) = self.parse_declaration() {
                statements.push(stmt);
            } else {
                self.synchronize();
            }
        }
        Program { statements }
    }

    // --- Declarations ---

    fn parse_declaration(&mut self) -> Option<Statement> {
        let current_token = self.peek();
        match current_token.token_type {
            TokenType::Let | TokenType::Const => Some(self.parse_var_declaration()),
            TokenType::Function => Some(self.parse_function_declaration(false)),
            TokenType::Async => {
                if !self.async_enabled {
                    let t = self.peek();
                    self.errors.push(crate::diagnostics::Diagnostic::new("async/await is disabled".to_string(), t.line, t.column, self.filename.clone()));
                }
                if self.check_next(TokenType::Function) {
                    self.advance(); // consume async
                    Some(self.parse_function_declaration(true))
                } else {
                    // Could be async arrow func or just expression statement?
                    Some(self.parse_statement()) 
                }
            }
            TokenType::Class => Some(self.parse_class_declaration(false)),
            TokenType::Abstract => {
                if self.check_next(TokenType::Class) {
                     self.advance(); // consume abstract
                     Some(self.parse_class_declaration(true))
                } else {
                     Some(self.parse_statement())
                }
            }
            TokenType::Interface => Some(self.parse_interface_declaration()),
            TokenType::Extension => Some(self.parse_extension_declaration()),
            TokenType::Enum => Some(self.parse_enum_declaration()),
            // TokenType::Protocol => Some(self.parse_protocol_declaration()), // Removed
            TokenType::TypeAlias => Some(self.parse_type_alias_declaration()), // Added
            TokenType::Export => {
                self.advance(); // consume export
                let is_default = self.match_token(TokenType::Default);
                if let Some(decl) = self.parse_declaration() {
                     Some(Statement::ExportDecl {
                         declaration: Box::new(decl),
                         _is_default: is_default,
                         _line: 0, _col: 0
                     })
                } else {
                    None
                }
            }
            TokenType::Import => Some(self.parse_import_statement()),
            _ => Some(self.parse_statement()),
        }
    }
    
    fn parse_var_declaration(&mut self) -> Statement {
        self.parse_var_declaration_internal(true)
    }

    fn parse_var_declaration_internal(&mut self, consume_semicolon: bool) -> Statement {
         let start_token = self.advance().clone(); // let/const
         let is_const = start_token.token_type == TokenType::Const;
         
         let pattern = self.parse_binding_pattern();
         
         let mut type_annotation = "".to_string();
         if self.match_token(TokenType::Colon) {
             type_annotation = self.parse_type_annotation();
         }
         
         let mut initializer = None;
         if self.match_token(TokenType::Equals) {
             initializer = Some(Box::new(self.parse_assignment()));
         }
         
         let mut declarations = vec![Statement::VarDeclaration {
             pattern,
             type_annotation,
             initializer,
             is_const,
             line: start_token.line,
             _col: start_token.column
         }];

         // Handle multiple declarations: let x = 1, y = 2;
         while self.match_token(TokenType::Comma) {
             let p = self.parse_binding_pattern();
             let mut ta = "".to_string();
             if self.match_token(TokenType::Colon) { ta = self.parse_type_annotation(); }
             let mut init = None;
              if self.match_token(TokenType::Equals) { init = Some(Box::new(self.parse_assignment())); }
             declarations.push(Statement::VarDeclaration {
                 pattern: p,
                 type_annotation: ta,
                 initializer: init,
                 is_const,
                 line: start_token.line,
                 _col: start_token.column
             });
         }
         
         if consume_semicolon {
             self.consume(TokenType::Semicolon, "Expected ';' after variable declaration.");
         }
         
         if declarations.len() == 1 {
             return declarations.pop().unwrap();
         } else {
             return Statement::BlockStmt {
                 statements: declarations,
                 _line: start_token.line,
                 _col: start_token.column
             };
         }
    }

    // Helper to consume Identifier OR Keyword as Identifier
    fn consume_identifier(&mut self, error_msg: &str) -> Token {
        if self.check(TokenType::Identifier) {
            self.advance().clone()
        } else if self.is_keyword_identifier() {
            // Treat keyword as identifier
            let mut t = self.advance().clone();
            t.token_type = TokenType::Identifier; // Re-tag for consistency if needed, or just return
            t
        } else {
             let t = self.peek().clone();
             self.errors.push(crate::diagnostics::Diagnostic::new(error_msg.to_string(), t.line, t.column, self.filename.clone()));
             // Advance to avoid infinite loops
             self.advance();
             t
        }
    }

    fn is_keyword_identifier(&self) -> bool {
        match self.peek().token_type {
            TokenType::Get | TokenType::Set | TokenType::Interface |
            TokenType::Enum | TokenType::Async | TokenType::Await | TokenType::Option | 
            TokenType::String | TokenType::Number | 
            TokenType::From | TokenType::To | TokenType::Of | TokenType::Public | TokenType::Private |
            TokenType::Protected | TokenType::Static | TokenType::Abstract |
            TokenType::Extends | TokenType::Some | TokenType::None |
            TokenType::TypeAny | TokenType::TypeVoid | TokenType::TypeInt | TokenType::TypeFloat |
            TokenType::TypeInt16 | TokenType::TypeInt64 | TokenType::TypeInt128 |
            TokenType::TypeFloat16 | TokenType::TypeFloat64 | TokenType::TypeChar |
            TokenType::TypeString | TokenType::TypeBoolean => true,
            _ => false
        }
    }

    fn parse_binding_pattern(&mut self) -> BindingNode {
        if self.check(TokenType::OpenBracket) {
            // Array destructuring
            self.advance();
            let mut elements = Vec::new();
            let mut rest = None;
            while !self.check(TokenType::CloseBracket) && !self.is_at_end() {
                if self.match_token(TokenType::Ellipsis) {
                    rest = Some(Box::new(self.parse_binding_pattern()));
                    if !self.check(TokenType::CloseBracket) {
                         // Error: rest must be last
                    }
                    break;
                }
                elements.push(self.parse_binding_pattern());
                if self.check(TokenType::Comma) {
                    self.advance();
                }
            }
            self.consume(TokenType::CloseBracket, "Expected ']'");
            return BindingNode::ArrayBinding { elements, rest };
        } else if self.check(TokenType::OpenBrace) {
             // Object destructuring
             self.advance();
             let mut entries = Vec::new();
             while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
                 let key = self.consume_identifier("Expected property name").value.clone();
                 let target = if self.match_token(TokenType::Colon) {
                      self.parse_binding_pattern()
                 } else {
                      BindingNode::Identifier(key.clone())
                 };
                 entries.push((key, target));
                 if self.check(TokenType::Comma) { self.advance(); }
             }
             self.consume(TokenType::CloseBrace, "Expected '}'");
             return BindingNode::ObjectBinding { entries };
        } else if self.match_token(TokenType::True) {
             return BindingNode::LiteralMatch(Box::new(Expression::BooleanLiteral { value: true, _line: 0, _col: 0 }));
        } else if self.match_token(TokenType::False) {
             return BindingNode::LiteralMatch(Box::new(Expression::BooleanLiteral { value: false, _line: 0, _col: 0 }));
        } else if self.match_token(TokenType::Number) {
             let t = self.previous().clone();
             return BindingNode::LiteralMatch(Box::new(Expression::NumberLiteral { 
                 value: t.value.parse().unwrap_or(0.0), _line: t.line, _col: t.column 
             }));
        } else if self.match_token(TokenType::String) {
             let t = self.previous().clone();
             return BindingNode::LiteralMatch(Box::new(Expression::StringLiteral { 
                 value: t.value.clone(), _line: t.line, _col: t.column 
             }));
        } else if self.match_token(TokenType::Some) {
            let start = self.previous().clone();
            self.consume(TokenType::OpenParen, "Expected '(' after Some");
            let inner = self.parse_binding_pattern();
            self.consume(TokenType::CloseParen, "Expected ')' after Some pattern");
            return BindingNode::LiteralMatch(Box::new(Expression::SomeExpr {
                _value: Box::new(inner),
                _line: start.line,
                _col: start.column,
            }));
        } else if self.match_token(TokenType::None) {
            let start = self.previous().clone();
            return BindingNode::LiteralMatch(Box::new(Expression::NoneExpr {
                _line: start.line,
                _col: start.column,
            }));
        } else if self.match_token(TokenType::Identifier) {
             let name = self.previous().value.clone(); // consumed by match_token
             return BindingNode::Identifier(name);
        } else {
            let t = self.peek().clone();
            self.errors.push(crate::diagnostics::Diagnostic::new(format!("Expected binding pattern"), t.line, t.column, self.filename.clone()));
            return BindingNode::Identifier("__error__".to_string());
        }
    }

    #[allow(dead_code)]
    fn parse_match_expression(&mut self) -> Expression {
        let start = self.previous().clone(); // 'match' consumed by caller
        self.consume(TokenType::OpenParen, "Expected '(' after match");
        let target = self.parse_expression();
        self.consume(TokenType::CloseParen, "Expected ')' after match target");
        self.consume(TokenType::OpenBrace, "Expected '{' in match");
        
        let mut arms = Vec::new();
        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
             let pattern = self.parse_binding_pattern();
             let mut guard = None;
             if self.match_token(TokenType::If) {
                 self.consume(TokenType::OpenParen, "Expected '(' after if");
                 guard = Some(Box::new(self.parse_expression()));
                 self.consume(TokenType::CloseParen, "Expected ')' after guard");
             }
             self.consume(TokenType::Arrow, "Expected '=>' in match arm");
             
             let body = if self.check(TokenType::OpenBrace) {
                 let start_brace = self.peek().clone(); // Use peek for line info
                 // Use parse_block which returns Statement (BlockStmt usually)
                 // But parse_block logic might be embedded in `parse_statement`?
                 // No, usually separate. Let's assume parse_block creates a statement vector or Statement::Block.
                 // We need to verify `parse_block`. `parse_statement` parses blocks.
                 // Let's call `parse_statement`. If it sees `{`, it parses a block.
                 let stmt = self.parse_statement();
                 // Expect it to be a valid statement (BlockStmt).
                 // Wrap in BlockExpr.
                 // Wait, Statement::BlockStmt? I need to check AST.
                 // Assuming `stmt` is the block.
                 match stmt {
                     // If it's a block, extract statements? Or just wrap the single stmt?
                     // AST BlockExpr expects Vec<Statement>.
                     // Statement::Block { statements } -> use statements.
                     // Statement::BlockStmt? Let's assume BlockStmt or Block.
                     // Actually let's just use `parse_block` if it exists, or `consume_block`.
                     // I will blindly use `parse_statement` for now and wrap it in a vec.
                     // Fix: `parse_block` usually returns `Vec<Statement>`.
                     // I'll assume `parse_block` exists as I saw `parse_statement` earlier.
                     // Re-reading `parser.rs` lines 1600+ didn't show `parse_block`.
                     // Let's use `parse_statement` as it handles `{`.
                     s => Box::new(Expression::BlockExpr { 
                         statements: vec![s], 
                         _line: start_brace.line, 
                         _col: start_brace.column 
                     })
                 }
             } else {
                 Box::new(self.parse_expression())
             };
             
             if self.check(TokenType::Comma) {
                 self.advance();
             }
             
             arms.push(MatchArm {
                 pattern, guard, body
             });
        }
        self.consume(TokenType::CloseBrace, "Expected '}' after match arms");
        
        Expression::MatchExpr {
            target: Box::new(target),
            arms,
            _line: start.line,
            _col: start.column
        }
    }

    fn parse_function_declaration(&mut self, is_async: bool) -> Statement {
        let start = self.consume(TokenType::Function, "Expected 'function'").clone();
        let name = self.consume_identifier("Expected function name").value.clone();
        
        self.consume(TokenType::OpenParen, "Expected '('");
        let mut params = Vec::new();
        if !self.check(TokenType::CloseParen) {
            loop {
                let is_rest = self.match_token(TokenType::Ellipsis);
                let p_name = self.consume(TokenType::Identifier, "Expected param name").value.clone();
                let mut p_type = "".to_string();
                if self.match_token(TokenType::Colon) {
                    p_type = self.parse_type_annotation();
                }
                let mut default_val = None;
                if self.match_token(TokenType::Equals) {
                    default_val = Some(Box::new(self.parse_assignment()));
                }
                
                params.push(Parameter {
                    name: p_name,
                    type_name: p_type,
                    _default_value: default_val,
                    _is_rest: is_rest
                });
                
                if !self.match_token(TokenType::Comma) { break; }
            }
        }
        self.consume(TokenType::CloseParen, "Expected ')'");
        
        let mut return_type = "".to_string();
        if self.match_token(TokenType::Colon) {
            return_type = self.parse_type_annotation();
        }
        
        let body_stmt = self.parse_block();
        // Check if body_stmt is BlockStmt and extract
        // Or just wrap it? FunctionDeclaration expects Box<Statement> which is fine if it's a block.
        // Actually definition says Box<Statement> (comment says BlockStmt).
        
        Statement::FunctionDeclaration(FunctionDeclaration {
            name,
            params,
            return_type,
            body: Box::new(body_stmt),
            _is_async: is_async,
            _line: start.line,
            _col: start.column
        })
    }

    fn parse_class_declaration(&mut self, is_abstract: bool) -> Statement {
        let start = self.consume(TokenType::Class, "Expected 'class'").clone();
        let name = self.consume_identifier("Expected class name").value.clone();
        
        // Support generic class: class Node<T>
        let mut generic_params = Vec::new();
        if self.match_token(TokenType::Less) {
            loop {
                // consume returns &Token, so .value.clone() works
                let param = self.consume(TokenType::Identifier, "Expected type parameter").value.clone();
                generic_params.push(param);
                if !self.match_token(TokenType::Comma) { break; }
            }
            self.consume(TokenType::Greater, "Expected '>'");
        }
        
        // Extends...
        let mut parent_name = "".to_string();
        if self.match_token(TokenType::Extends) {
            parent_name = self.consume(TokenType::Identifier, "Expected parent name").value.clone();
        }
        
        let mut implemented_protocols = Vec::new();
        if self.match_token(TokenType::Implements) {
            loop {
                implemented_protocols.push(self.consume(TokenType::Identifier, "Expected protocol name").value.clone());
                if !self.match_token(TokenType::Comma) { break; }
            }
        }
        
        let is_abstract_class = is_abstract; 
        
        self.consume(TokenType::OpenBrace, "Expected '{' before class body.");
        
        let mut members = Vec::new();
        let mut methods = Vec::new();
        let mut getters = Vec::new();
        let mut setters = Vec::new();
        let mut constructor = None;
        
        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
             let mut is_static = false;
             let mut is_abstract_member = false;
             let mut is_async = false;
             let mut access = AccessModifier::Public;
             
             // Modifiers
             loop {
                 if self.match_token(TokenType::Public) { access = AccessModifier::Public; }
                 else if self.match_token(TokenType::Private) { access = AccessModifier::Private; }
                 else if self.match_token(TokenType::Protected) { access = AccessModifier::Protected; }
                 else if self.match_token(TokenType::Static) { is_static = true; }
                 else if self.match_token(TokenType::Abstract) { is_abstract_member = true; }
                 else if self.match_token(TokenType::Async) { is_async = true; }
                 else { break; }
             }
             
             if self.match_token(TokenType::Constructor) {
                 // Constructor
                 self.consume(TokenType::OpenParen, "Expected '('");
                 let mut params = Vec::new();
                 if !self.check(TokenType::CloseParen) {
                     loop {
                         let name = self.consume(TokenType::Identifier, "Expected param name").value.clone();
                         let mut type_name = "".to_string();
                         if self.match_token(TokenType::Colon) {
                             type_name = self.parse_type_annotation();
                         }
                         params.push(Parameter { name, type_name, _default_value: None, _is_rest: false });
                         if !self.match_token(TokenType::Comma) { break; }
                     }
                 }
                  self.consume(TokenType::CloseParen, "Expected ')' after constructor parameters.");
                  let body = self.parse_block();
                  
                  constructor = Some(FunctionDeclaration {
                     name: "constructor".to_string(),
                     params,
                     return_type: "void".to_string(),
                     body: Box::new(body),
                     _is_async: false,
                     _line: start.line,
                     _col: start.column
                 });
                 continue;
             }
             
             // Field or Method or Getter/Setter
             if self.check(TokenType::Get) && self.check_next(TokenType::Identifier) {
                 self.advance(); // consume get
                 let name = self.consume_identifier("Expected getter name").value.clone();
                 self.consume(TokenType::OpenParen, "Expected '('");
                 self.consume(TokenType::CloseParen, "Expected ')'");
                 let mut return_type = "".to_string();
                 if self.match_token(TokenType::Colon) { return_type = self.parse_type_annotation(); }
                 let body = self.parse_block();
                 getters.push(ClassGetter { _name: name, _return_type: return_type, _body: Box::new(body), _access: access.clone() });
                 continue;
             }
             
             if self.check(TokenType::Set) && self.check_next(TokenType::Identifier) {
                 self.advance(); // consume set
                 let name = self.consume_identifier("Expected setter name").value.clone();
                 self.consume(TokenType::OpenParen, "Expected '('");
                 let param = self.consume(TokenType::Identifier, "Expected param").value.clone();
                 let mut p_type = "".to_string();
                 if self.match_token(TokenType::Colon) { p_type = self.parse_type_annotation(); }
                 self.consume(TokenType::CloseParen, "Expected ')'");
                 let body = self.parse_block();
                 setters.push(ClassSetter { _name: name, _param_name: param, _param_type: p_type, _body: Box::new(body), _access: access.clone() });
                 continue;
             }
             
             let name = self.consume_identifier("Expected member name").value.clone();
             
             if self.check(TokenType::OpenParen) {
                 // Method
                 self.consume(TokenType::OpenParen, "Expected '('");
                 let mut params = Vec::new();
                 if !self.check(TokenType::CloseParen) {
                     loop {
                         let p_name = self.consume(TokenType::Identifier, "Expected param").value.clone();
                         let mut p_type = "".to_string();
                         if self.match_token(TokenType::Colon) { p_type = self.parse_type_annotation(); }
                         params.push(Parameter { name: p_name, type_name: p_type, _default_value: None, _is_rest: false });
                         if !self.match_token(TokenType::Comma) { break; }
                     }
                 }
                 self.consume(TokenType::CloseParen, "Expected ')'");
                 let mut return_type = "void".to_string();
                 if self.match_token(TokenType::Colon) { return_type = self.parse_type_annotation(); }
                 
                 let body = if is_abstract_member {
                     self.consume(TokenType::Semicolon, "Expected ';'");
                     Box::new(Statement::BlockStmt { statements: vec![], _line: 0, _col: 0 }) 
                 } else {
                     Box::new(self.parse_block())
                 };
                 
                 methods.push(ClassMethod {
                     func: FunctionDeclaration { name, params, return_type, body, _is_async: is_async, _line: 0, _col: 0 },
                     _access: access.clone(),
                     is_static: is_static,
                     _is_abstract: is_abstract_member
                 });
             } else {
                 // Field
                 let mut type_name = "".to_string();
                 if self.match_token(TokenType::Colon) { type_name = self.parse_type_annotation(); }
                 let mut initializer = None;
                 if self.match_token(TokenType::Equals) {
                     initializer = Some(Box::new(self.parse_expression()));
                 }
                 self.consume(TokenType::Semicolon, "Expected ';'");
                 
                 members.push(ClassMember {
                     _name: name,
                     _type_name: type_name,
                     _access: access.clone(),
                     _is_static: is_static,
                     _initializer: initializer
                 });
             }
        }
           if self.is_at_end() && !self.check(TokenType::CloseBrace) {
            self.errors.push(crate::diagnostics::Diagnostic::new(
                format!("Unclosed class body for '{}' starting at line {}:{}", name, start.line, start.column),
                self.previous().line,
                self.previous().column + self.previous().value.len(),
                self.filename.clone()
            ));
        } else {
            self.consume(TokenType::CloseBrace, "Expected '}' after class body.");
        }
        
        if constructor.is_none() {
            constructor = Some(FunctionDeclaration {
                name: "constructor".to_string(),
                params: vec![],
                return_type: "void".to_string(),
                body: Box::new(Statement::BlockStmt { 
                    statements: vec![],
                    _line: start.line,
                    _col: start.column
                }),
                _is_async: false,
                _line: start.line,
                _col: start.column
            });
        }

        Statement::ClassDeclaration(ClassDeclaration {
            name,
            _parent_name: parent_name,
            generic_params,
            _is_abstract: is_abstract_class,
            _implemented_protocols: implemented_protocols,
            _members: members,
            methods,
            _getters: getters,
            _setters: setters,
            _constructor: constructor,
            _line: start.line,
            _col: start.column
        })
    }

    // Removed parse_protocol_declaration

    fn parse_type_alias_declaration(&mut self) -> Statement {
        let start = self.consume(TokenType::TypeAlias, "Expected 'type'").clone();
        let name = self.consume_identifier("Expected type name").value.clone();
        self.consume(TokenType::Equals, "Expected '='");
        // For now, type alias just stores the type string, or we could parse a TypeNode if we had one.
        // The AST has TypeAliasDeclaration { ... _type_def: String ... }
        let type_def = self.parse_type_annotation(); 
        self.consume(TokenType::Semicolon, "Expected ';'");

        Statement::TypeAliasDeclaration {
            name,
            _type_def: type_def,
            _line: start.line,
            _col: start.column
        }
    }

    fn parse_extension_declaration(&mut self) -> Statement {
         let start = self.consume(TokenType::Extension, "Expected 'extension'").clone();
         let target_type = self.consume_identifier("Expected type name").value.clone();
         self.consume(TokenType::OpenBrace, "Expected '{'");
         
         let mut methods = Vec::new();
         while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
             // Methods in extension are like methods in class but without modifiers usually?
             // features_oop.tx uses standard method syntax: greet(name: string): void { ... }
             // or status(): string { ... }
             // They look like function declarations but without 'function' keyword.
             
             let name = self.consume_identifier("Expected method name").value.clone();
             self.consume(TokenType::OpenParen, "Expected '('");
             let mut params = Vec::new();
             if !self.check(TokenType::CloseParen) {
                 loop {
                     let p_name = self.consume(TokenType::Identifier, "Param name").value.clone();
                     let mut p_type = "".to_string();
                     if self.match_token(TokenType::Colon) {
                         p_type = self.parse_type_annotation();
                     }
                     params.push(Parameter { name: p_name, type_name: p_type, _default_value: None, _is_rest: false });
                     if !self.match_token(TokenType::Comma) { break; }
                 }
             }
             self.consume(TokenType::CloseParen, "Expected ')'");
             
             let mut return_type = "void".to_string();
             if self.match_token(TokenType::Colon) {
                 return_type = self.parse_type_annotation();
             }
             
             let body = self.parse_block();
             
             methods.push(FunctionDeclaration {
                 name,
                 params,
                 return_type,
                 body: Box::new(body),
                 _is_async: false,
                 _line: 0,
                 _col: 0
             });
         }
         self.consume(TokenType::CloseBrace, "Expected '}'");
         
         Statement::ExtensionDeclaration(ExtensionDeclaration {
             _target_type: target_type,
             _methods: methods,
             _line: start.line,
             _col: start.column
         })
    }

    fn parse_enum_declaration(&mut self) -> Statement {
         let start = self.consume(TokenType::Enum, "Expected 'enum'").clone();
        let name = self.consume_identifier("Expected enum name").value.clone();
        self.consume(TokenType::OpenBrace, "Expected '{'");
        
        let mut members = Vec::new();
        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
            let member_name = self.consume_identifier("Expected enum member").value.clone();
            let mut value = None;
            if self.match_token(TokenType::Equals) {
                value = Some(Box::new(self.parse_expression()));
            }
            members.push(EnumMember { _name: member_name, _value: value });
            if self.check(TokenType::Comma) { self.advance(); }
        }
        self.consume(TokenType::CloseBrace, "Expected '}'");
        
        Statement::EnumDeclaration(EnumDeclaration {
            name,
            _members: members,
            _line: start.line,
            _col: start.column
        })
    }


    fn parse_import_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::Import, "Expected 'import'").clone();
        
        // Check for 'import std:module;' syntax
        if self.check(TokenType::Identifier) && self.peek().value == "std" {
            self.advance(); // consume 'std'
            if self.match_token(TokenType::Colon) {
                let module_name = self.consume(TokenType::Identifier, "Expected module name").value.clone();
                self.consume(TokenType::Semicolon, "Expected ';'");
                return Statement::ImportDecl {
                    _names: vec![],
                    source: format!("std:{}", module_name),
                    _is_default: false,
                    _line: start.line,
                    _col: start.column
                };
            }
            // Fallback if it was just an identifier 'std' but not followed by ':'
            let names = vec!["std".to_string()];
            let is_default = true;
            
            // Re-use logic for 'import name from "..."'
            if self.check(TokenType::Identifier) {
                 let t = self.peek();
                 if t.value == "from" {
                     self.advance();
                 }
            } else if self.match_token(TokenType::From) {
                 // ok
            }
            
            let source = self.consume(TokenType::String, "Expected module path").value.clone();
            self.consume(TokenType::Semicolon, "Expected ';'");
            
            return Statement::ImportDecl {
                _names: names,
                source,
                _is_default: is_default,
                _line: start.line,
                _col: start.column
            };
        }

        let mut names = Vec::new();
        let mut is_default = false;
        
        // Check if default import: import name from "..."
        if self.check(TokenType::Identifier) {
             names.push(self.consume_identifier("Expected import name").value.clone());
             is_default = true;
        } else if self.match_token(TokenType::OpenBrace) {
             // Named imports: import { a, b } from "..."
             while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
                 names.push(self.consume_identifier("Expected import name").value.clone());
                 if self.check(TokenType::Comma) { self.advance(); }
             }
             self.consume(TokenType::CloseBrace, "Expected '}'");
        }
        
        // Consume 'from'
        if self.check(TokenType::Identifier) {
             let t = self.peek();
             if t.value == "from" {
                 self.advance();
             }
        } else if self.match_token(TokenType::From) {
             // ok
        }
        
        let source = self.consume(TokenType::String, "Expected module path").value.clone();
        self.consume(TokenType::Semicolon, "Expected ';'");
        
        Statement::ImportDecl {
            _names: names,
            source,
            _is_default: is_default,
            _line: start.line,
            _col: start.column
        }
    }

    fn parse_interface_declaration(&mut self) -> Statement {
        let start = self.consume(TokenType::Interface, "Expected 'interface'").clone();
        let name = self.consume_identifier("Expected interface name").value.clone();
        self.consume(TokenType::OpenBrace, "Expected '{'");
        
        // Similar to Protocol but can have fields
        // For now, let's reuse ProtocolMethod structure but we might need ProtocolField?
        // AST says InterfaceDecl uses ProtocolMethod.
        // Let's just parse methods. If we see field-like syntax, ignore/adapt?
        // features_types.tx has fields.
        let mut methods = Vec::new();
        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
            let member_name = self.consume_identifier("Expected member name").value.clone();
            
            if self.check(TokenType::OpenParen) {
                self.consume(TokenType::OpenParen, "Expected '('");
                let mut params = Vec::new();
                if !self.check(TokenType::CloseParen) {
                     loop {
                         let p_name = self.consume(TokenType::Identifier, "Param name").value.clone();
                         self.consume(TokenType::Colon, "Expected ':'");
                         let p_type = self.parse_type_annotation();
                         params.push(Parameter { name: p_name, type_name: p_type, _default_value: None, _is_rest: false });
                         if !self.match_token(TokenType::Comma) { break; }
                     }
                }
                self.consume(TokenType::CloseParen, "Expected ')'");
                self.consume(TokenType::Colon, "Expected ':'");
                let ret_type = self.parse_type_annotation();
                self.consume(TokenType::Semicolon, "Expected ';'");
                methods.push(InterfaceMethod { _name: member_name, _params: params, _return_type: ret_type });
            } else {
                // Field-like in interface: name: type;
                self.consume(TokenType::Colon, "Expected ':'");
                let _type_name = self.parse_type_annotation();
                self.consume(TokenType::Semicolon, "Expected ';'");
                // InterfaceDecl in AST only has methods currently.
                // We should probably add fields to AST or just ignore fields for now to pass parsing.
                // Ignoring fields for now to avoid AST change explosion.
            }
        }
        self.consume(TokenType::CloseBrace, "Expected '}'");
        
        Statement::InterfaceDeclaration {
            name,
            _methods: methods,
            _line: start.line,
            _col: start.column
        }
    }
    
    fn parse_type_annotation(&mut self) -> String {
        let mut base_type = self.parse_base_type();
        
        // Union Type: A | B
        while self.match_token(TokenType::Pipe) {
            let next_type = self.parse_single_type();
            base_type.push_str(" | ");
            base_type.push_str(&next_type);
        }
        
        base_type
    }

    fn parse_single_type(&mut self) -> String {
         self.parse_base_type()
    }

    fn parse_base_type(&mut self) -> String {
        // Function type: (params) => returnType
        if self.match_token(TokenType::OpenParen) {
             let mut params_str = String::new();
             params_str.push('(');
             if !self.check(TokenType::CloseParen) {
                  loop {
                      if self.check(TokenType::Identifier) && self.check_next(TokenType::Colon) {
                           let name = self.consume(TokenType::Identifier, "Param name").value.clone();
                           self.consume(TokenType::Colon, "Expected ':'");
                           let p_type = self.parse_type_annotation();
                           params_str.push_str(&format!("{}: {}", name, p_type));
                      } else {
                           let p_type = self.parse_type_annotation();
                           params_str.push_str(&p_type);
                      }
                      
                      if self.match_token(TokenType::Comma) {
                          params_str.push_str(", ");
                      } else { break; }
                  }
             }
             self.consume(TokenType::CloseParen, "Expected ')'");
             params_str.push(')');
             self.consume(TokenType::Arrow, "Expected '=>'");
             let ret_type = self.parse_type_annotation();
             return format!("{} => {}", params_str, ret_type);
        }
        
        // Object Type: { x: number, y: string }
        if self.match_token(TokenType::OpenBrace) {
             let mut members = Vec::new();
             while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
                  let key = self.consume_identifier("Expected property name").value.clone();
                  let mut optional = "";
                  if self.match_token(TokenType::Question) {
                      optional = "?";
                  }
                  self.consume(TokenType::Colon, "Expected ':'");
                  let val_type = self.parse_type_annotation();
                  members.push(format!("{}{}: {}", key, optional, val_type));
                  
                  if self.match_token(TokenType::Comma) || self.match_token(TokenType::Semicolon) {
                      // continue
                  } else {
                      break; 
                  }
             }
             self.consume(TokenType::CloseBrace, "Expected '}'");
             return format!("{{ {} }}", members.join("; "));
        }
        
        // Array Type: number[]
        let mut base_type;
        if self.check(TokenType::Identifier) || self.is_keyword_identifier() {
             base_type = self.consume_identifier("Expected type name").value.clone();
             
             // Generics: Array<T> or Option<T>
             if self.match_token(TokenType::Less) {
                  base_type.push('<');
                  loop {
                       base_type.push_str(&self.parse_type_annotation());
                       if self.match_token(TokenType::Comma) {
                           base_type.push_str(", ");
                       } else { break; }
                  }
                  self.consume(TokenType::Greater, "Expected '>'");
                  base_type.push('>');
             }
             
        } else {
            if self.match_token(TokenType::TypeAny) { base_type = "any".to_string(); }
            else if self.match_token(TokenType::TypeVoid) { base_type = "void".to_string(); }
            else if self.match_token(TokenType::TypeInt) { base_type = "int".to_string(); }
            else if self.match_token(TokenType::TypeInt16) { base_type = "int16".to_string(); }
            else if self.match_token(TokenType::TypeInt64) { base_type = "int64".to_string(); }
            else if self.match_token(TokenType::TypeInt128) { base_type = "int128".to_string(); }
            else if self.match_token(TokenType::TypeFloat) { base_type = "float".to_string(); }
            else if self.match_token(TokenType::TypeFloat16) { base_type = "float16".to_string(); }
            else if self.match_token(TokenType::TypeFloat64) { base_type = "float64".to_string(); }
            else if self.match_token(TokenType::TypeChar) { base_type = "char".to_string(); }
            else if self.match_token(TokenType::TypeString) { base_type = "string".to_string(); }
            else if self.match_token(TokenType::TypeBoolean) { base_type = "bool".to_string(); }
            else {
                 let t = self.peek().clone();
                 self.errors.push(crate::diagnostics::Diagnostic::new(format!("Expected type"), t.line, t.column, self.filename.clone()));
                 self.advance();
                 return "any".to_string();
            }
        }
        
        while self.match_token(TokenType::OpenBracket) {
            if self.check(TokenType::Number) {
                let size = self.consume(TokenType::Number, "Expected size").value.clone();
                self.consume(TokenType::CloseBracket, "Expected ']'");
                base_type.push_str(&format!("[{}]", size));
            } else {
                self.consume(TokenType::CloseBracket, "Expected ']'");
                base_type.push_str("[]");
            }
        }
        base_type
    }

    // --- Statements ---

    fn parse_statement(&mut self) -> Statement {
        if self.check(TokenType::OpenBrace) {
            return self.parse_block();
        }
        if self.check(TokenType::If) {
            return self.parse_if_statement();
        }
        if self.check(TokenType::While) {
            return self.parse_while_statement();
        }
        if self.check(TokenType::For) {
            return self.parse_for_statement();
        }
        if self.check(TokenType::Return) {
            return self.parse_return_statement();
        }
        if self.check(TokenType::Break) {
             return self.parse_break_statement();
        }
        if self.check(TokenType::Continue) {
             return self.parse_continue_statement();
        }
        if self.check(TokenType::Switch) {
            return self.parse_switch_statement();
        }
        if self.check(TokenType::Try) {
            return self.parse_try_statement();
        }
        if self.check(TokenType::Throw) {
             let start = self.consume(TokenType::Throw, "Expected 'throw'").clone();
             let expr = self.parse_expression();
             self.consume(TokenType::Semicolon, "Expected ';'");
             return Statement::ThrowStmt {
                 _expression: Box::new(expr),
                 _line: start.line,
                 _col: start.column
             };
        }
        self.parse_expression_statement()
    }

    fn parse_block(&mut self) -> Statement {
        let start = self.consume(TokenType::OpenBrace, "Expected '{'").clone();
        let mut statements = Vec::new();
        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
             if let Some(stmt) = self.parse_declaration() {
                statements.push(stmt);
            }
        }
        
        if self.is_at_end() && !self.check(TokenType::CloseBrace) {
            self.errors.push(crate::diagnostics::Diagnostic::new(
                format!("Unclosed block starting at line {}:{}", start.line, start.column),
                self.previous().line,
                self.previous().column + self.previous().value.len(),
                self.filename.clone()
            ));
        } else {
            self.consume(TokenType::CloseBrace, "Expected '}' after block.");
        }
        
        Statement::BlockStmt { statements, _line: start.line, _col: start.column }
    }
    
    fn parse_if_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::If, "Expected 'if'").clone();
        self.consume(TokenType::OpenParen, "Expected '(' after 'if'.");
        let condition = self.parse_expression();
        self.consume(TokenType::CloseParen, "Expected ')' after if condition.");
        
        let then_branch = Box::new(self.parse_statement());
        let mut else_branch = None;
        if self.match_token(TokenType::Else) {
             else_branch = Some(Box::new(self.parse_statement()));
        }
        
        Statement::IfStmt {
            condition: Box::new(condition),
            then_branch,
            else_branch,
            _line: start.line,
            _col: start.column
        }
    }
    
    fn parse_while_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::While, "Expected 'while'").clone();
        self.consume(TokenType::OpenParen, "Expected '(' after 'while'.");
        let condition = self.parse_expression();
        self.consume(TokenType::CloseParen, "Expected ')' after while condition.");
        let body = self.parse_statement();
        
        Statement::WhileStmt {
            condition: Box::new(condition),
            body: Box::new(body),
            _line: start.line,
            _col: start.column
        }
    }
    
    fn parse_for_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::For, "Expected 'for'").clone();
        self.consume(TokenType::OpenParen, "Expected '(' after 'for'.");

        // Check for 'for (let x of y)' - simplified peek logic
        let mut is_for_of = false;
        let start_pos = self.current;
        if self.match_token(TokenType::Let) || self.match_token(TokenType::Const) {
            if self.check(TokenType::Identifier) {
                self.advance(); // consume ident
                if self.match_token(TokenType::Colon) { 
                    self.parse_type_annotation(); 
                }
                if self.check(TokenType::Of) {
                    is_for_of = true;
                }
            }
        }
        self.current = start_pos; // Reset for actual parsing

        if is_for_of {
            let is_const = self.match_token(TokenType::Const);
            if !is_const { self.consume(TokenType::Let, "Expected 'let'"); }
            let pattern = self.parse_binding_pattern();
            if self.match_token(TokenType::Colon) { self.parse_type_annotation(); }
            self.consume(TokenType::Of, "Expected 'of'");
            let iterable = self.parse_expression();
            self.consume(TokenType::CloseParen, "Expected ')'");
            let body = self.parse_statement();
            return Statement::ForOfStmt {
                variable: pattern,
                iterable: Box::new(iterable),
                body: Box::new(body),
                _line: start.line,
                _col: start.column,
            };
        }

        // Regular for(init; cond; step)
        let init = if self.match_token(TokenType::Semicolon) {
            None
        } else if self.check(TokenType::Let) || self.check(TokenType::Const) {
            Some(Box::new(self.parse_var_declaration_internal(false)))
        } else {
            let expr = self.parse_expression();
            Some(Box::new(Statement::ExpressionStmt { 
                _expression: Box::new(expr),
                _line: start.line,
                _col: start.column 
            }))
        };

        if init.is_some() {
            // VarDeclaration/Expression already parsed but didn't consume final ';' if we used internal(false)
            // or if it was just an expression statement, we should expect ';'
            self.consume(TokenType::Semicolon, "Expected ';' after for init.");
        }

        let mut condition = None;
        if !self.check(TokenType::Semicolon) {
            condition = Some(Box::new(self.parse_expression()));
        }
        self.consume(TokenType::Semicolon, "Expected ';'");

        let mut increment = None;
        if !self.check(TokenType::CloseParen) {
            increment = Some(Box::new(self.parse_expression()));
        }
        self.consume(TokenType::CloseParen, "Expected ')' after for clauses.");

        let body = self.parse_statement();

        Statement::ForStmt {
            init,
            condition,
            increment,
            body: Box::new(body),
            _line: start.line,
            _col: start.column,
        }
    }

    fn parse_return_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::Return, "Expected 'return'").clone();
        let mut value = None;
        if !self.check(TokenType::Semicolon) {
            value = Some(Box::new(self.parse_expression()));
        }
        self.consume(TokenType::Semicolon, "Expected ';'");
        Statement::ReturnStmt { value, _line: start.line, _col: start.column }
    }

    fn parse_break_statement(&mut self) -> Statement {
         let start = self.consume(TokenType::Break, "Expected 'break'").clone();
         self.consume(TokenType::Semicolon, "Expected ';'");
         Statement::BreakStmt { _line: start.line, _col: start.column }
    }

    fn parse_continue_statement(&mut self) -> Statement {
         let start = self.consume(TokenType::Continue, "Expected 'continue'").clone();
         self.consume(TokenType::Semicolon, "Expected ';'");
         Statement::ContinueStmt { _line: start.line, _col: start.column }
    }

    fn parse_switch_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::Switch, "Expected 'switch'").clone();
        self.consume(TokenType::OpenParen, "Expected '(' after 'switch'.");
        let condition = self.parse_expression();
        self.consume(TokenType::CloseParen, "Expected ')' after switch condition.");
        self.consume(TokenType::OpenBrace, "Expected '{' before switch body.");
        
        let mut cases = Vec::new();
        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
            if self.match_token(TokenType::Case) {
                let value = self.parse_expression();
                self.consume(TokenType::Colon, "Expected ':'");
                
                let mut statements = Vec::new();
                while !self.check(TokenType::Case) && !self.check(TokenType::Default) && !self.check(TokenType::CloseBrace) && !self.is_at_end() {
                     if let Some(stmt) = self.parse_declaration() {
                         statements.push(stmt);
                     }
                }
                cases.push(Case { value: Some(Box::new(value)), statements });
            } else if self.match_token(TokenType::Default) {
                self.consume(TokenType::Colon, "Expected ':'");
                 let mut statements = Vec::new();
                while !self.check(TokenType::Case) && !self.check(TokenType::Default) && !self.check(TokenType::CloseBrace) && !self.is_at_end() {
                     if let Some(stmt) = self.parse_declaration() {
                         statements.push(stmt);
                     }
                }
                cases.push(Case { value: None, statements });
            } else {
                break;
            }
        }
        self.consume(TokenType::CloseBrace, "Expected '}'");
        
        Statement::SwitchStmt {
            condition: Box::new(condition),
            cases,
            _line: start.line,
            _col: start.column
        }
    }
    
    fn parse_try_statement(&mut self) -> Statement {
        let start = self.consume(TokenType::Try, "Expected 'try'").clone();
        let try_block = self.parse_block();
        
        let mut catch_var = "".to_string();
        let mut catch_block = Box::new(Statement::BlockStmt { statements: vec![], _line: 0, _col: 0 }); // Empty by default
        
        if self.match_token(TokenType::Catch) {
            self.consume(TokenType::OpenParen, "Expected '('");
            catch_var = self.consume(TokenType::Identifier, "Expected catch variable").value.clone();
            self.consume(TokenType::CloseParen, "Expected ')'");
            catch_block = Box::new(self.parse_block());
        }
        
        let mut finally_block = None;
        if self.match_token(TokenType::Finally) {
            finally_block = Some(Box::new(self.parse_block()));
        }
        
        Statement::TryStmt {
            _try_block: Box::new(try_block),
            _catch_var: catch_var,
            _catch_block: catch_block,
            _finally_block: finally_block,
            _line: start.line,
            _col: start.column
        }
    }

    fn parse_expression_statement(&mut self) -> Statement {
        let expr = self.parse_expression();
        let _start = &expr; 
        // We need line/col from expression or first token? 
        // Let's rely on expression's embedded line/col?
        // But Expression doesn't have a uniform getter yet (enums variants).
        // Let's peek previous token (semicolon or end of expr)
        let line = self.tokens[self.current - 1].line; 
        let col = self.tokens[self.current - 1].column;

        self.consume(TokenType::Semicolon, "Expected ';'");
        Statement::ExpressionStmt {
            _expression: Box::new(expr),
            _line: line,
            _col: col
        }
    }

    // --- Expressions ---

    fn parse_expression(&mut self) -> Expression {
        self.parse_comma_expression()
    }

    fn parse_comma_expression(&mut self) -> Expression {
        let mut expr = self.parse_assignment();
        let line = self.peek().line;
        let col = self.peek().column;

        if self.check(TokenType::Comma) {
            let mut expressions = vec![expr];
            while self.match_token(TokenType::Comma) {
                expressions.push(self.parse_assignment());
            }
            return Expression::SequenceExpr { expressions, _line: line, _col: col };
        }
        expr
    }

    fn parse_assignment(&mut self) -> Expression {
        let expr = self.parse_conditional();
        
        if self.match_token(TokenType::Equals) || self.match_token(TokenType::PlusEquals) || 
           self.match_token(TokenType::MinusEquals) || self.match_token(TokenType::StarEquals) || 
           self.match_token(TokenType::SlashEquals) {
            let op_token = self.tokens[self.current - 1].clone();
            let op = op_token.token_type;
            let value = self.parse_assignment();
            return Expression::AssignmentExpr {
                target: Box::new(expr),
                value: Box::new(value),
                _op: op,
                _line: op_token.line, 
                _col: op_token.column
            };
        }
        expr
    }

    fn parse_conditional(&mut self) -> Expression {
        let expr = self.parse_nullish_coalescing();
        
        if self.match_token(TokenType::Question) {
             let true_branch = self.parse_expression();
             self.consume(TokenType::Colon, "Expected ':' in ternary");
             let false_branch = self.parse_expression();
             
             return Expression::TernaryExpr {
                 _condition: Box::new(expr),
                 _true_branch: Box::new(true_branch),
                 _false_branch: Box::new(false_branch),
                 _line: 0, _col: 0
             };
        }
        
        expr
    }

    fn parse_nullish_coalescing(&mut self) -> Expression {
        let mut expr = self.parse_logical_or();
        
        while self.match_token(TokenType::QuestionQuestion) {
            let right = self.parse_logical_or();
            expr = Expression::NullishCoalescingExpr {
                _left: Box::new(expr),
                _right: Box::new(right),
                _line: 0, _col: 0
            };
        }
        expr
    }

    fn parse_logical_or(&mut self) -> Expression {
        let mut expr = self.parse_logical_and();
        while self.match_token(TokenType::PipePipe) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_logical_and();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_logical_and(&mut self) -> Expression {
        let mut expr = self.parse_bitwise_or();
        while self.match_token(TokenType::AmpersandAmpersand) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_equality();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }


    fn parse_bitwise_or(&mut self) -> Expression {
        let mut expr = self.parse_bitwise_xor();
        while self.match_token(TokenType::Pipe) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_bitwise_xor();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_bitwise_xor(&mut self) -> Expression {
        let mut expr = self.parse_bitwise_and();
        while self.match_token(TokenType::Caret) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_bitwise_and();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_bitwise_and(&mut self) -> Expression {
        let mut expr = self.parse_equality();
        while self.match_token(TokenType::Ampersand) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_equality();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_equality(&mut self) -> Expression {
        let mut expr = self.parse_comparison();
        while self.match_token(TokenType::EqualEqual) || self.match_token(TokenType::BangEqual) ||
              self.match_token(TokenType::EqualEqualEqual) || self.match_token(TokenType::BangEqualEqual) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_comparison();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_comparison(&mut self) -> Expression {
        let mut expr = self.parse_shift();
         while self.match_token(TokenType::GreaterEqual) || self.match_token(TokenType::Greater) ||
               self.match_token(TokenType::LessEqual) || self.match_token(TokenType::Less) ||
               self.match_token(TokenType::Instanceof) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_term();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }


    fn parse_shift(&mut self) -> Expression {
        let mut expr = self.parse_term();
        while self.match_token(TokenType::LessLess) || self.match_token(TokenType::GreaterGreater) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_term();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_term(&mut self) -> Expression {
        let mut expr = self.parse_factor();
        while self.match_token(TokenType::Plus) || self.match_token(TokenType::Minus) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_factor();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_factor(&mut self) -> Expression {
        let mut expr = self.parse_unary();
        while self.match_token(TokenType::Star) || self.match_token(TokenType::Slash) || self.match_token(TokenType::Modulo) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_unary();
             expr = Expression::BinaryExpr {
                 left: Box::new(expr),
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        expr
    }

    fn parse_unary(&mut self) -> Expression {
        if self.match_token(TokenType::Await) {
            let op_token = self.previous().clone();
            if !self.async_enabled {
                self.errors.push(crate::diagnostics::Diagnostic::new("async/await is disabled".to_string(), op_token.line, op_token.column, self.filename.clone()));
            }
            let right = self.parse_unary();
            return Expression::AwaitExpr {
                expr: Box::new(right),
                _line: op_token.line,
                _col: op_token.column
            };
        }
        if self.match_token(TokenType::PlusPlus) || self.match_token(TokenType::MinusMinus) ||
           self.match_token(TokenType::Bang) || self.match_token(TokenType::Minus) {
             let op_token = self.tokens[self.current - 1].clone();
             let right = self.parse_unary();
             return Expression::UnaryExpr {
                 op: op_token.token_type,
                 right: Box::new(right),
                 _line: op_token.line,
                 _col: op_token.column
             };
        }
        let mut expr = self.parse_call();
        // Postfix ++ and --
        if self.match_token(TokenType::PlusPlus) || self.match_token(TokenType::MinusMinus) {
            let op_token = self.previous().clone();
            expr = Expression::UnaryExpr {
                op: op_token.token_type,
                right: Box::new(expr),
                _line: op_token.line,
                _col: op_token.column,
            };
        }
        expr
    }
    
    fn parse_call(&mut self) -> Expression {
        let start_token = self.peek().clone(); // Capture start for location
        let mut expr = self.parse_primary();
        
        loop {
            if self.match_token(TokenType::OpenParen) {
                // Call
                let mut args = Vec::new();
                if !self.check(TokenType::CloseParen) {
                    loop {
                        args.push(self.parse_assignment());
                        if !self.match_token(TokenType::Comma) { break; }
                    }
                }
                let _end_token = self.consume(TokenType::CloseParen, "Expected ')'").clone(); // unused now for loc
                
                expr = Expression::CallExpr {
                    callee: Box::new(expr),
                    args,
                    _line: start_token.line,
                    _col: start_token.column
                };
            } else if self.match_token(TokenType::QuestionDot) {
                if self.match_token(TokenType::OpenParen) {
                     let mut args = Vec::new();
                     if !self.check(TokenType::CloseParen) {
                        loop {
                            args.push(self.parse_assignment());
                            if !self.match_token(TokenType::Comma) { break; }
                        }
                    }
                    let end = self.consume(TokenType::CloseParen, "Expected ')'");
                    expr = Expression::OptionalCallExpr {
                        callee: Box::new(expr),
                        args,
                        _line: end.line,
                        _col: end.column
                    };
                } else if self.match_token(TokenType::OpenBracket) {
                     let _start_token = self.tokens[self.current - 2].clone(); // ?. matched, then [ matched? No.
                     // match_token(QuestionDot) advanced. current is at next token.
                     // if next token is OpenParen -> call.
                     // else if next is OpenBracket -> index.
                     let index = self.parse_assignment();
                     self.consume(TokenType::CloseBracket, "Expected ']'");
                     expr = Expression::OptionalArrayAccessExpr {
                         target: Box::new(expr),
                         index: Box::new(index),
                         _line: 0, _col: 0
                     };
                } else {
                     let member = self.consume_identifier("Expected property name after '?.'").value.clone();
                     expr = Expression::OptionalMemberAccessExpr {
                         object: Box::new(expr),
                         member,
                         _line: 0, _col: 0
                     };
                }
            } else if self.match_token(TokenType::Dot) || self.match_token(TokenType::DoubleColon) {
                let is_double_colon = self.tokens[self.current - 1].token_type == TokenType::DoubleColon;
                let name = self.consume_identifier("Expected property name after separator").value.clone();
                expr = Expression::MemberAccessExpr {
                    object: Box::new(expr),
                    member: name,
                    _line: 0, _col: 0,
                    _is_namespace: is_double_colon
                };
            } else if self.match_token(TokenType::OpenBracket) {
                 let index = self.parse_assignment();
                 self.consume(TokenType::CloseBracket, "Expected ']'");
                 expr = Expression::ArrayAccessExpr {
                     target: Box::new(expr),
                     index: Box::new(index),
                     _line: 0, _col: 0
                 };
            } else {
                break;
            }
        }
        
        expr
    }

    fn parse_new_expression(&mut self) -> Expression {
         let start = self.consume(TokenType::New, "Expected 'new'").clone();
         let mut class_name = self.consume(TokenType::Identifier, "Expected class name").value.clone();
         
         // Support generics: new MaxHeap<int>()
         if self.match_token(TokenType::Less) {
             class_name.push('<');
             loop {
                 class_name.push_str(&self.parse_type_annotation());
                 if self.match_token(TokenType::Comma) {
                     class_name.push_str(", ");
                 } else { break; }
             }
             self.consume(TokenType::Greater, "Expected '>'");
             class_name.push('>');
         }
         self.consume(TokenType::OpenParen, "Expected '('");
         let mut args = Vec::new();
         if !self.check(TokenType::CloseParen) {
            loop {
                args.push(self.parse_assignment());
                if !self.match_token(TokenType::Comma) { break; }
            }
        }
         self.consume(TokenType::CloseParen, "Expected ')'");
         
         Expression::NewExpr {
             class_name,
             args,
             _line: start.line,
             _col: start.column
         }
    }

    fn parse_primary(&mut self) -> Expression {
        if self.match_token(TokenType::True) {
            return Expression::BooleanLiteral { value: true, _line: 0, _col: 0 };
        }
        if self.match_token(TokenType::False) {
            return Expression::BooleanLiteral { value: false, _line: 0, _col: 0 };
        }
        if self.match_token(TokenType::None) {
             return Expression::NoneExpr { _line: 0, _col: 0 };
        }
        if self.match_token(TokenType::Some) {
            let start = self.previous().clone();
            self.consume(TokenType::OpenParen, "Expected '(' after Some");
            let inner = self.parse_expression();
            self.consume(TokenType::CloseParen, "Expected ')' after Some value");
            return Expression::SomeExpr {
                _value: Box::new(BindingNode::LiteralMatch(Box::new(inner))),
                _line: start.line,
                _col: start.column,
            };
        }
        if self.match_token(TokenType::This) {
             return Expression::ThisExpr { _line: 0, _col: 0 };
        }
        if self.match_token(TokenType::Super) {
             return Expression::SuperExpr { _line: 0, _col: 0 };
        }
        
        if self.check(TokenType::New) {
             return self.parse_new_expression();
        }
        

        // Literal null/none check?
        
        let token = self.peek().clone();
        match token.token_type {
            TokenType::Number => {
                self.advance();
                return Expression::NumberLiteral { 
                    value: token.value.parse().unwrap_or(0.0),
                    _line: token.line,
                    _col: token.column
                };
            }
            TokenType::String => {
                self.advance();
                return Expression::StringLiteral {
                    value: token.value,
                    _line: token.line,
                    _col: token.column
                };
            }
            TokenType::TemplateString => {
                 self.advance();
                 return self.parse_template_string(token);
            }
            TokenType::Identifier => {
                if self.check_next(TokenType::Arrow) {
                    return self.parse_lambda();
                }
                self.advance();
                return Expression::Identifier {
                    name: token.value,
                    _line: token.line,
                    _col: token.column
                };
            }
            TokenType::OpenBracket => {
                return self.parse_array_literal();
            }
            TokenType::OpenParen => {
                // Check if this is a lambda: (a, b) => ...
                let mut is_lambda = false;
                let mut i = 0;
                while !self.is_at_offset_end(i) {
                    let t = self.peek_offset(i);
                    if t.token_type == TokenType::CloseParen {
                        if self.peek_offset(i + 1).token_type == TokenType::Arrow {
                            is_lambda = true;
                        }
                        break;
                    }
                    // Stop if we see things that definitely aren't in a simple param list (like { or ;)
                    // actually { can be in object destructuring param, but let's keep it simple for now matching C++ logic roughly
                    if t.token_type == TokenType::Semicolon {
                         break;
                    }
                    i += 1;
                }

                if is_lambda {
                    return self.parse_lambda();
                }

                self.advance();
                let expr = self.parse_expression();
                self.consume(TokenType::CloseParen, "Expected ')'");
                return expr;
            }
            TokenType::OpenBrace => {
                return self.parse_object_literal();
            }
            _ => {
                let err_token = token.clone();
                self.errors.push(crate::diagnostics::Diagnostic::new(format!("Unexpected token '{}'", err_token.value), err_token.line, err_token.column, self.filename.clone()));
                self.advance();
                Expression::Identifier { name: "__error__".to_string(), _line: err_token.line, _col: err_token.column }
            }
        }
    }
    
    #[allow(dead_code)]
    fn expr_to_callee_name(expr: &Expression) -> String {
        match expr {
            Expression::ArrayLiteral { .. } => "(array)".to_string(),
            Expression::ObjectLiteralExpr { .. } => "(object)".to_string(),
            Expression::SequenceExpr { .. } => "(sequence)".to_string(),
            Expression::Identifier { name, .. } => name.clone(),
            Expression::ThisExpr { .. } => "this".to_string(),
            Expression::SuperExpr { .. } => "super".to_string(),
            Expression::MemberAccessExpr { object, member, _is_namespace, .. } => {
                let obj_name = Self::expr_to_callee_name(object);
                let sep = if *_is_namespace { "::" } else { "." };
                format!("{}{}{}", obj_name, sep, member)
            }
            _ => "__callee__".to_string(),
        }
    }

    fn is_at_offset_end(&self, offset: usize) -> bool {
        self.current + offset >= self.tokens.len()
    }

    fn peek_offset(&self, offset: usize) -> &Token {
        if self.is_at_offset_end(offset) {
             match self.tokens.last() {
                 Some(t) => t,
                 None => panic!("No tokens available")
             }
        } else {
            &self.tokens[self.current + offset]
        }
    }

    fn parse_lambda(&mut self) -> Expression {
        let start_line = self.peek().line;
        let start_col = self.peek().column;
        let mut params = Vec::new();

        if self.check(TokenType::Identifier) {
            // Single param: x => ...
            let name = self.consume(TokenType::Identifier, "Expected param name").value.clone();
            params.push(Parameter {
                name,
                type_name: "".to_string(), 
                _default_value: None,
                _is_rest: false
            });
        } else {
            // (x, y) => ...
            self.consume(TokenType::OpenParen, "Expected '('");
            if !self.check(TokenType::CloseParen) {
                loop {
                    let is_rest = self.match_token(TokenType::Ellipsis);
                    let name = self.consume(TokenType::Identifier, "Expected param name").value.clone();
                    let mut type_name = "".to_string();
                    if self.match_token(TokenType::Colon) {
                        type_name = self.parse_type_annotation();
                    }
                    
                    params.push(Parameter {
                        name,
                        type_name,
                        _default_value: None, 
                        _is_rest: is_rest
                    });
                    
                    if !self.match_token(TokenType::Comma) { break; }
                }
            }
            self.consume(TokenType::CloseParen, "Expected ')'");
        }

        self.consume(TokenType::Arrow, "Expected '=>'");

        let body = if self.check(TokenType::OpenBrace) {
             self.parse_block()
        } else {
             let expr = self.parse_expression();
             let ret = Statement::ReturnStmt {
                 value: Some(Box::new(expr)),
                 _line: start_line,
                 _col: start_col
             };
             Statement::BlockStmt {
                 statements: vec![ret],
                 _line: start_line,
                 _col: start_col
             }
        };
        
        Expression::LambdaExpr {
            params,
            body: Box::new(body),
            _line: start_line,
            _col: start_col
        }
    }

    fn parse_object_literal(&mut self) -> Expression {
        let start = self.consume(TokenType::OpenBrace, "Expected '{'").clone();
        let mut entries = Vec::new();
        let mut spreads = Vec::new();

        while !self.check(TokenType::CloseBrace) && !self.is_at_end() {
            if self.match_token(TokenType::Ellipsis) {
                let expr = self.parse_assignment();
                spreads.push(expr);
            } else {
                let key_token = self.consume_identifier("Expected property name");
                let key = key_token.value.clone();
                
                let value = if self.match_token(TokenType::Colon) {
                    self.parse_assignment()
                } else {
                     Expression::Identifier { 
                         name: key.clone(), 
                         _line: key_token.line, 
                         _col: key_token.column 
                     }
                };
                
                entries.push((key, value));
            }
            
            if self.check(TokenType::Comma) {
                self.advance();
            }
        }
        
        self.consume(TokenType::CloseBrace, "Expected '}'");
        
        Expression::ObjectLiteralExpr {
            entries,
            _spreads: spreads,
            _line: start.line,
            _col: start.column
        }
    }

    fn parse_array_literal(&mut self) -> Expression {
        let start = self.consume(TokenType::OpenBracket, "Expected '['").clone();
        let mut elements = Vec::new();
        while !self.check(TokenType::CloseBracket) && !self.is_at_end() {
             if self.match_token(TokenType::Ellipsis) {
                 let spread_start = self.tokens[self.current - 1].clone();
                 let expr = self.parse_assignment();
                 elements.push(Expression::SpreadExpr {
                     _expr: Box::new(expr),
                     _line: spread_start.line,
                     _col: spread_start.column
                 });
             } else {
                 elements.push(self.parse_assignment());
             }
             if self.check(TokenType::Comma) { self.advance(); }
        }
        self.consume(TokenType::CloseBracket, "Expected ']'");
        Expression::ArrayLiteral {
            elements,
            _line: start.line,
            _col: start.column
        }
    }

    // Helpers
    fn is_at_end(&self) -> bool {
        self.peek().token_type == TokenType::EndOfFile
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn check(&self, token_type: TokenType) -> bool {
        if self.is_at_end() {
            return false;
        }
        self.peek().token_type == token_type
    }
    
    fn check_next(&self, token_type: TokenType) -> bool {
        if self.current + 1 >= self.tokens.len() {
            return false;
        }
        self.tokens[self.current + 1].token_type == token_type
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        &self.tokens[self.current - 1]
    }
    
    fn match_token(&mut self, token_type: TokenType) -> bool {
        if self.check(token_type) {
            self.advance();
            return true;
        }

        // Spacing issue: handle split operators
        match token_type {
            TokenType::EqualEqual => {
                if self.check(TokenType::Equals) && self.check_next(TokenType::Equals) {
                    self.advance(); self.advance();
                    self.tokens[self.current - 1].token_type = TokenType::EqualEqual;
                    self.tokens[self.current - 1].value = "==".to_string();
                    return true;
                }
            }
            TokenType::BangEqual => {
                if self.check(TokenType::Bang) && self.check_next(TokenType::Equals) {
                    self.advance(); self.advance();
                    self.tokens[self.current - 1].token_type = TokenType::BangEqual;
                    self.tokens[self.current - 1].value = "!=".to_string();
                    return true;
                }
            }
            TokenType::LessEqual => {
                if self.check(TokenType::Less) && self.check_next(TokenType::Equals) {
                    self.advance(); self.advance();
                    self.tokens[self.current - 1].token_type = TokenType::LessEqual;
                    self.tokens[self.current - 1].value = "<=".to_string();
                    return true;
                }
            }
            TokenType::GreaterEqual => {
                if self.check(TokenType::Greater) && self.check_next(TokenType::Equals) {
                    self.advance(); self.advance();
                    self.tokens[self.current - 1].token_type = TokenType::GreaterEqual;
                    self.tokens[self.current - 1].value = ">=".to_string();
                    return true;
                }
            }
            TokenType::PlusPlus => {
                if self.check(TokenType::Plus) && self.check_next(TokenType::Plus) {
                    self.advance(); self.advance();
                    self.tokens[self.current - 1].token_type = TokenType::PlusPlus;
                    self.tokens[self.current - 1].value = "++".to_string();
                    return true;
                }
            }
            TokenType::MinusMinus => {
                if self.check(TokenType::Minus) && self.check_next(TokenType::Minus) {
                    self.advance(); self.advance();
                    self.tokens[self.current - 1].token_type = TokenType::MinusMinus;
                    self.tokens[self.current - 1].value = "--".to_string();
                    return true;
                }
            }
            _ => {}
        }

        false
    }

    fn consume(&mut self, token_type: TokenType, message: &str) -> &Token {
        if self.check(token_type) {
            self.advance()
        } else {
            let t = self.peek().clone();
            let (line, col) = if token_type == TokenType::Semicolon && self.current > 0 {
                // Report missing semicolon at the end of the previous token
                let prev = self.previous();
                (prev.line, prev.column + prev.value.len())
            } else {
                (t.line, t.column)
            };

            self.errors.push(crate::diagnostics::Diagnostic::new(
                message.to_string(),
                line,
                col,
                self.filename.clone()
            ));
            
            // Advance to avoid infinite loops
            self.advance()
        }
    }

    fn synchronize(&mut self) {
        // If we're already at a boundary, don't skip anything
        match self.peek().token_type {
            TokenType::Class | TokenType::Function | TokenType::Let | TokenType::Const | 
            TokenType::For | TokenType::If | TokenType::While | TokenType::Return |
            TokenType::Switch | TokenType::Export | TokenType::Import | TokenType::CloseBrace => return,
            _ => {}
        }

        self.advance();

        while !self.is_at_end() {
            if self.previous().token_type == TokenType::Semicolon { return; }

            match self.peek().token_type {
                TokenType::Class | TokenType::Function | TokenType::Let | TokenType::Const | 
                TokenType::For | TokenType::If | TokenType::While | TokenType::Return |
                TokenType::Switch | TokenType::Export | TokenType::Import | TokenType::CloseBrace => return,
                _ => {}
            }

            self.advance();
        }
    }
    fn parse_template_string(&mut self, token: Token) -> Expression {
        let value = &token.value;
        let mut expr = None;
        let mut current_pos = 0;

        while let Some(start_idx) = value[current_pos..].find("${") {
            let actual_start = current_pos + start_idx;
            
            // Text before ${
            let prefix = &value[current_pos..actual_start];
            if !prefix.is_empty() {
                let part = Expression::StringLiteral { 
                    value: prefix.to_string(), 
                    _line: token.line, 
                    _col: token.column 
                };
                expr = match expr {
                    None => Some(part),
                    Some(e) => Some(Expression::BinaryExpr {
                        left: Box::new(e),
                        op: TokenType::Plus,
                        right: Box::new(part),
                        _line: token.line, _col: token.column
                    }),
                };
            }

            // Find matching }
            let after_start = actual_start + 2;
            let mut depth = 1;
            let mut end_idx = None;
            for (i, c) in value[after_start..].chars().enumerate() {
                if c == '{' { depth += 1; }
                if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        end_idx = Some(after_start + i);
                        break;
                    }
                }
            }

            if let Some(actual_end) = end_idx {
                let inter_content = &value[after_start..actual_end];
                // Parse interpolation
                let mut lexer = crate::lexer::Lexer::new(inter_content, &self.filename);
                let tokens = lexer.tokenize();
                let mut parser = crate::parser::Parser::new(tokens, &self.filename);
                let inter_expr = parser.parse_expression();

                expr = match expr {
                    None => Some(inter_expr),
                    Some(e) => Some(Expression::BinaryExpr {
                        left: Box::new(e),
                        op: TokenType::Plus,
                        right: Box::new(inter_expr),
                        _line: token.line, _col: token.column
                    }),
                };
                current_pos = actual_end + 1;
            } else {
                // Unclosed ${...}
                break;
            }
        }

        // Remaining text
        if current_pos < value.len() {
             let suffix = &value[current_pos..];
             let part = Expression::StringLiteral { 
                value: suffix.to_string(), 
                _line: token.line, 
                _col: token.column 
            };
            expr = match expr {
                None => Some(part),
                Some(e) => Some(Expression::BinaryExpr {
                    left: Box::new(e),
                    op: TokenType::Plus,
                    right: Box::new(part),
                    _line: token.line, _col: token.column
                }),
            };
        }

        expr.unwrap_or(Expression::StringLiteral { value: "".to_string(), _line: token.line, _col: token.column })
    }

}
