use super::Lowering;
use crate::common::diagnostics::Diagnostic;
use crate::common::paths::{CORE_DIR, STD_DIR};
use crate::frontend::ast::*;
use std::collections::HashSet;

impl Lowering {
    pub fn resolve_imports(
        &self,
        mut statements: Vec<Statement>,
        current_dir: &std::path::Path,
        processed_files: &mut HashSet<std::path::PathBuf>,
        import_stack: &mut Vec<std::path::PathBuf>,
        current_file: Option<&std::path::Path>,
    ) -> Vec<Statement> {
        let filename = current_file
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| self.filename.borrow().clone());

        // Resolve standard paths
        let stdlib_path = self.stdlib_path.borrow();
        let core_dir = stdlib_path.join(CORE_DIR);

        // 1. Implicitly import prelude if this isn't the prelude itself
        let is_prelude = filename.ends_with("prelude.tx") || filename.contains("prelude.tx");
        if !is_prelude {
            let prelude_path = core_dir.join("prelude.tx");
            let prelude_path_str = prelude_path.to_string_lossy().to_string();

            let mut already_imports_prelude = false;
            for stmt in &statements {
                if let Statement::ImportDecl { source, .. } = stmt {
                    if source == &prelude_path_str {
                        already_imports_prelude = true;
                        break;
                    }
                }
            }
            if !already_imports_prelude {
                statements.insert(
                    0,
                    Statement::ImportDecl {
                        source: prelude_path_str,
                        _names: Vec::new(),
                        _is_default: false,
                        _line: 0,
                        _col: 0,
                    },
                );
            }
        }

        // 2. Implicitly import other core functionality (array, string, object) 
        // ONLY for modules outside of the library's core directory to avoid cycles.
        let is_in_lib_core = if let Ok(canon_file) = std::fs::canonicalize(&filename) {
            if let Ok(canon_core) = std::fs::canonicalize(&core_dir) {
                canon_file.starts_with(canon_core)
            } else {
                false
            }
        } else {
            filename.contains(&format!("/{}", CORE_DIR)) || filename.contains(&format!("{}/", CORE_DIR))
        };

        if !is_in_lib_core {
            for core_file in ["array.tx", "string.tx", "object.tx"] {
                let path = core_dir.join(core_file);
                let path_str = path.to_string_lossy().to_string();
                
                let mut already_imports = false;
                for stmt in &statements {
                    if let Statement::ImportDecl { source, .. } = stmt {
                        if source == &path_str {
                            already_imports = true;
                            break;
                        }
                    }
                }
                
                if !already_imports {
                    statements.insert(
                        0,
                        Statement::ImportDecl {
                            source: path_str,
                            _names: Vec::new(),
                            _is_default: false,
                            _line: 0,
                            _col: 0,
                        },
                    );
                }
            }
        }

        let mut i = 0;
        while i < statements.len() {
            if let Statement::ImportDecl {
                source,
                _names,
                _is_default,
                _line,
                _col,
            } = &statements[i]
            {
                let import_items = _names.clone();
                let import_line = *_line;
                let import_col = *_col;
                let is_default = *_is_default;
                let source_str = source.clone();

                let path = if source_str.starts_with("std:") {
                    let mod_name = &source_str[4..];
                    let base = self.stdlib_path.borrow().clone();
                    // Search strictly in lib/std/
                    base.join(STD_DIR).join(format!("{}.tx", mod_name))
                } else {
                    let mut p = current_dir.to_path_buf();
                    let clean_source = source_str.trim_matches('"');
                    if clean_source.starts_with("./") {
                        p.push(&clean_source[2..]);
                    } else {
                        p.push(clean_source);
                    }
                    if !p.to_string_lossy().ends_with(".tx") {
                        p.set_extension("tx");
                    }
                    p
                };

                if !path.exists() {
                    self.diagnostics.borrow_mut().push(
                        Diagnostic::new(
                            format!("Module not found: '{}'", source_str),
                            import_line,
                            import_col,
                            filename.clone(),
                        )
                        .with_code("E0200")
                        .with_label(&format!("Module not found: '{}'", source_str)),
                    );
                    i += 1;
                    continue;
                }

                let canon_path = match std::fs::canonicalize(&path) {
                    Ok(p) => p,
                    Err(_) => path.clone(),
                };

                if import_stack.contains(&canon_path) {
                    self.diagnostics.borrow_mut().push(
                        Diagnostic::new(
                            format!("Circular dependency detected: '{}'", source_str),
                            import_line,
                            import_col,
                            filename.clone(),
                        )
                        .with_code("E0204")
                        .with_label("circularly imported here"),
                    );
                    statements.remove(i);
                    continue;
                }

                if processed_files.contains(&canon_path) {
                    statements.remove(i);
                    continue;
                }

                processed_files.insert(canon_path.clone());
                import_stack.push(canon_path.clone());

                let content = match std::fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        self.diagnostics.borrow_mut().push(Diagnostic::new(
                            format!("Failed to read module '{}': {}", source_str, e),
                            import_line,
                            import_col,
                            filename.clone(),
                        ));
                        i += 1;
                        continue;
                    }
                };

                let mut lexer =
                    crate::frontend::lexer::Lexer::new(&content, &path.to_string_lossy());
                let tokens = lexer.tokenize();
                if !lexer.errors.is_empty() {
                    for diag in &lexer.errors {
                        self.diagnostics.borrow_mut().push(diag.clone());
                    }
                    i += 1;
                    continue;
                }

                let mut parser =
                    crate::frontend::parser::Parser::new(tokens, &path.to_string_lossy());
                let imported_program = parser.parse_program();
                if parser.has_errors() {
                    for diag in parser.get_errors() {
                        self.diagnostics.borrow_mut().push(diag.clone());
                    }
                    i += 1;
                    continue;
                }

                let mut new_stmts = self.resolve_imports(
                    imported_program.statements,
                    path.parent().unwrap_or(std::path::Path::new(".")),
                    processed_files,
                    import_stack,
                    Some(&path),
                );
                import_stack.pop();

                // Handle Aliasing
                for item in &import_items {
                    if is_default {
                        let target_name = item.alias.as_ref().unwrap_or(&item.name);
                        for stmt in new_stmts.iter_mut() {
                            if let Statement::ExportDecl {
                                declaration,
                                _is_default: true,
                                ..
                            } = stmt
                            {
                                match declaration.as_mut() {
                                    Statement::FunctionDeclaration(func) => {
                                        func.name = target_name.clone()
                                    }
                                    Statement::ClassDeclaration(class) => {
                                        class.name = target_name.clone()
                                    }
                                    Statement::VarDeclaration {
                                        pattern: crate::frontend::ast::BindingNode::Identifier(name),
                                        ..
                                    } => *name = target_name.clone(),
                                    _ => {}
                                }
                            }
                        }
                    } else if let Some(alias) = &item.alias {
                        for stmt in new_stmts.iter_mut() {
                            if let Statement::ExportDecl { declaration, .. } = stmt {
                                match declaration.as_mut() {
                                    Statement::FunctionDeclaration(func)
                                        if func.name == item.name =>
                                    {
                                        func.name = alias.clone()
                                    }
                                    Statement::ClassDeclaration(class)
                                        if class.name == item.name =>
                                    {
                                        class.name = alias.clone()
                                    }
                                    Statement::VarDeclaration {
                                        pattern: crate::frontend::ast::BindingNode::Identifier(name),
                                        ..
                                    } if name == &item.name => *name = alias.clone(),
                                    _ => {}
                                }
                            }
                        }
                    }
                }

                // Validate exports
                let mut exported_names: HashSet<String> = HashSet::new();
                let mut has_default_export = false;

                fn collect_names(stmt: &Statement, names: &mut HashSet<String>) {
                    match stmt {
                        Statement::FunctionDeclaration(f) => {
                            names.insert(f.name.clone());
                        }
                        Statement::ClassDeclaration(c) => {
                            names.insert(c.name.clone());
                        }
                        Statement::VarDeclaration {
                            pattern: crate::frontend::ast::BindingNode::Identifier(n),
                            ..
                        } => {
                            names.insert(n.clone());
                        }
                        Statement::BlockStmt { statements, .. } => {
                            for s in statements {
                                collect_names(s, names);
                            }
                        }
                        Statement::ExportDecl { declaration, .. } => {
                            collect_names(declaration, names);
                        }
                        _ => {}
                    }
                }

                for stmt in &new_stmts {
                    if let Statement::ExportDecl {
                        declaration,
                        _is_default: is_def,
                        ..
                    } = stmt
                    {
                        if *is_def {
                            has_default_export = true;
                        }
                        collect_names(declaration, &mut exported_names);
                    }
                }

                if is_default && !has_default_export {
                    self.diagnostics.borrow_mut().push(
                        Diagnostic::new(
                            format!("Module '{}' has no default export", source_str),
                            import_line,
                            import_col,
                            filename.clone(),
                        )
                        .with_code("E0203"),
                    );
                } else if !is_default && !import_items.is_empty() {
                    for item in &import_items {
                        let lookup_name = item.alias.as_ref().unwrap_or(&item.name);
                        if !exported_names.contains(lookup_name) {
                            self.diagnostics.borrow_mut().push(
                                Diagnostic::new(
                                    format!(
                                        "'{}' is not exported from '{}'",
                                        item.name, source_str
                                    ),
                                    import_line,
                                    import_col,
                                    filename.clone(),
                                )
                                .with_code("E0202"),
                            );
                        }
                    }
                }

                statements.splice(i..i + 1, new_stmts);
                continue;
            }
            i += 1;
        }
        statements
    }
}
