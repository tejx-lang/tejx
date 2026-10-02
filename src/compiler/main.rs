#![warn(unsafe_op_in_unsafe_fn)]

pub mod backend;
pub mod common;
pub mod frontend;
pub mod middle;

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::process;

use crate::backend::codegen::CodeGen;
use crate::backend::linker::Linker;
use crate::common::diagnostics::Diagnostic;
use crate::frontend::ast;
use crate::frontend::lexer::Lexer;
use crate::frontend::parser::Parser;
use crate::middle::lowering::Lowering;
use crate::middle::mir::lowering::MIRLowering;
use crate::middle::mir::opt as mir_opt;
use crate::middle::semantic::TypeChecker;
use tejx_rt::constants::MIN_VTHREAD_STACK_SIZE;

// The runtime library is resolved at runtime from the filesystem

fn unique_diagnostics<'a, I>(diagnostics: I) -> Vec<Diagnostic>
where
    I: IntoIterator<Item = &'a Diagnostic>,
{
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for diag in diagnostics {
        let key = (
            diag.file.clone(),
            diag.line,
            diag.col,
            diag.length,
            diag.code.clone(),
            diag.message.clone(),
            diag.hint.clone(),
            diag.label.clone(),
        );
        if seen.insert(key) {
            unique.push(diag.clone());
        }
    }
    unique
}

fn report_diagnostics(
    stage: &str,
    diagnostics: &[Diagnostic],
    primary_file: &str,
    primary_source: &str,
) {
    let count = diagnostics.len();
    let max_display = 10;
    for diag in diagnostics.iter().take(max_display) {
        let mut d = diag.clone();
        if d.file.is_empty() || d.file == "<inferred>" {
            d.file = primary_file.to_string();
        }
        let loaded_source = if d.file == primary_file {
            None
        } else {
            fs::read_to_string(&d.file).ok()
        };
        let source = if let Some(source) = loaded_source.as_deref() {
            Some(source)
        } else {
            Some(primary_source)
        };
        d.report_with_source(source);
    }
    if count > max_display {
        eprintln!(
            "  \x1b[33m...\x1b[0m and {} more error{} omitted",
            count - max_display,
            if count - max_display == 1 { "" } else { "s" }
        );
    }
    let suffix = if count == 1 { "" } else { "s" };
    eprintln!("\x1b[31;1merror\x1b[0m: {} failed with {} error{}", stage.to_lowercase(), count, suffix);
}

fn apply_inferred_function_return_annotation(
    func: &mut ast::FunctionDeclaration,
    current_file: &str,
    type_checker: &TypeChecker,
) {
    if !func.return_type.to_string().is_empty() {
        return;
    }

    if let Some(return_ty) = type_checker.inferred_function_returns.get(&(
        current_file.to_string(),
        func._line,
        func._col,
        func.name.clone(),
    )) {
        func.return_type = return_ty.to_type_node();
    }
}

fn apply_inferred_member_return_annotation(
    owner_name: &str,
    func: &mut ast::FunctionDeclaration,
    current_file: &str,
    type_checker: &TypeChecker,
) {
    if !func.return_type.to_string().is_empty() {
        return;
    }

    if let Some(return_ty) = type_checker.inferred_member_returns.get(&(
        current_file.to_string(),
        owner_name.to_string(),
        func._line,
        func._col,
        func.name.clone(),
    )) {
        func.return_type = return_ty.to_type_node();
    }
}

fn apply_inferred_return_types_to_statement(
    stmt: &mut ast::Statement,
    current_file: &str,
    type_checker: &TypeChecker,
) {
    match stmt {
        ast::Statement::FunctionDeclaration(func) => {
            apply_inferred_function_return_annotation(func, current_file, type_checker);
            apply_inferred_return_types_to_statement(
                func.body.as_mut(),
                current_file,
                type_checker,
            );
        }
        ast::Statement::ClassDeclaration(class_decl) => {
            if let Some(constructor) = class_decl._constructor.as_mut() {
                apply_inferred_return_types_to_statement(
                    constructor.body.as_mut(),
                    current_file,
                    type_checker,
                );
            }

            for method in &mut class_decl.methods {
                apply_inferred_member_return_annotation(
                    &class_decl.name,
                    &mut method.func,
                    current_file,
                    type_checker,
                );
                apply_inferred_return_types_to_statement(
                    method.func.body.as_mut(),
                    current_file,
                    type_checker,
                );
            }

            for getter in &mut class_decl._getters {
                apply_inferred_return_types_to_statement(
                    getter._body.as_mut(),
                    current_file,
                    type_checker,
                );
            }

            for setter in &mut class_decl._setters {
                apply_inferred_return_types_to_statement(
                    setter._body.as_mut(),
                    current_file,
                    type_checker,
                );
            }
        }
        ast::Statement::ExtensionDeclaration(ext_decl) => {
            let owner_name = ext_decl._target_type.to_string();
            for method in &mut ext_decl._methods {
                apply_inferred_member_return_annotation(
                    &owner_name,
                    method,
                    current_file,
                    type_checker,
                );
                apply_inferred_return_types_to_statement(
                    method.body.as_mut(),
                    current_file,
                    type_checker,
                );
            }
        }
        ast::Statement::ExportDecl { declaration, .. } => {
            apply_inferred_return_types_to_statement(
                declaration.as_mut(),
                current_file,
                type_checker,
            );
        }
        ast::Statement::BlockStmt { statements, .. } => {
            for statement in statements {
                apply_inferred_return_types_to_statement(statement, current_file, type_checker);
            }
        }
        ast::Statement::IfStmt {
            then_branch,
            else_branch,
            ..
        } => {
            apply_inferred_return_types_to_statement(
                then_branch.as_mut(),
                current_file,
                type_checker,
            );
            if let Some(else_branch) = else_branch.as_mut() {
                apply_inferred_return_types_to_statement(
                    else_branch.as_mut(),
                    current_file,
                    type_checker,
                );
            }
        }
        ast::Statement::WhileStmt { body, .. } | ast::Statement::ForOfStmt { body, .. } => {
            apply_inferred_return_types_to_statement(body.as_mut(), current_file, type_checker);
        }
        ast::Statement::ForStmt { init, body, .. } => {
            if let Some(init) = init.as_mut() {
                apply_inferred_return_types_to_statement(init.as_mut(), current_file, type_checker);
            }
            apply_inferred_return_types_to_statement(body.as_mut(), current_file, type_checker);
        }
        ast::Statement::SwitchStmt { cases, .. } => {
            for case in cases {
                for statement in &mut case.statements {
                    apply_inferred_return_types_to_statement(statement, current_file, type_checker);
                }
            }
        }
        ast::Statement::TryStmt {
            _try_block,
            _catch_block,
            _finally_block,
            ..
        } => {
            apply_inferred_return_types_to_statement(
                _try_block.as_mut(),
                current_file,
                type_checker,
            );
            apply_inferred_return_types_to_statement(
                _catch_block.as_mut(),
                current_file,
                type_checker,
            );
            if let Some(finally_block) = _finally_block.as_mut() {
                apply_inferred_return_types_to_statement(
                    finally_block.as_mut(),
                    current_file,
                    type_checker,
                );
            }
        }
        _ => {}
    }
}

fn apply_inferred_return_types_to_program(
    program: &mut ast::Program,
    statement_files: &[String],
    default_file: &str,
    type_checker: &TypeChecker,
) {
    for (index, stmt) in program.statements.iter_mut().enumerate() {
        let current_file = statement_files
            .get(index)
            .map(|file| file.as_str())
            .unwrap_or(default_file);
        apply_inferred_return_types_to_statement(stmt, current_file, type_checker);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut input_files = Vec::new();

    let mut emit_mir = false;
    let mut emit_llvm = false;
    let mut emit_asm = false;
    let mut emit_ast = false;
    let mut emit_tokens = false;
    let mut compile_only = false;
    let mut check_only = false;
    let mut run_after_compile = false;
    let mut run_args = Vec::new();
    let mut unsafe_arrays = false;
    let mut opt_level = "-O3".to_string();
    let mut debug_symbols = false;
    let mut _warnings_as_errors = false;
    let mut verbose = false;
    let mut output_name = None;
    let mut cli_stdlib_path: Option<String> = None;
    let mut cli_runtime_path: Option<String> = None;
    let mut cli_vt_stack: Option<usize> = None;
    let mut cli_include_dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut cli_lib_dirs: Vec<std::path::PathBuf> = Vec::new();
    let mut cli_libs: Vec<String> = Vec::new();
    let mut cli_target: Option<String> = None;
    let mut show_stats = false;
    let mut _wall = false;
    let mut _wextra = false;

    let total_timer = std::time::Instant::now();

    fn parse_cli_size(s: &str) -> Option<usize> {
        let s = s.trim().to_lowercase();
        if s.ends_with("gib") || s.ends_with("gb") || s.ends_with('g') {
            let n = s.trim_end_matches(|c: char| c.is_alphabetic()).trim();
            return n.parse::<usize>().ok().map(|v| v * 1024 * 1024 * 1024);
        }
        if s.ends_with("mib") || s.ends_with("mb") || s.ends_with('m') {
            let n = s.trim_end_matches(|c: char| c.is_alphabetic()).trim();
            return n.parse::<usize>().ok().map(|v| v * 1024 * 1024);
        }
        if s.ends_with("kib") || s.ends_with("kb") || s.ends_with('k') {
            let n = s.trim_end_matches(|c: char| c.is_alphabetic()).trim();
            return n.parse::<usize>().ok().map(|v| v * 1024);
        }
        s.parse::<usize>().ok()
    }

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            run_args.extend(args[i + 1..].iter().cloned());
            break;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-v" | "--version" => {
                print_version();
                return;
            }
            "--check" | "-check" => {
                check_only = true;
            }
            "-r" | "--run" => {
                run_after_compile = true;
            }
            "-S" | "--emit-asm" => {
                emit_asm = true;
            }
            "-O0" => opt_level = "-O0".to_string(),
            "-O1" => opt_level = "-O1".to_string(),
            "-O2" => opt_level = "-O2".to_string(),
            "-O3" => opt_level = "-O3".to_string(),
            "-Os" => opt_level = "-Os".to_string(),
            "-g" | "--debug" => {
                debug_symbols = true;
            }
            "-Wall" => {
                _wall = true;
            }
            "-Wextra" => {
                _wextra = true;
            }
            "-Werror" => {
                _warnings_as_errors = true;
            }
            "--stats" | "--time-report" => {
                show_stats = true;
            }
            "--verbose" => {
                verbose = true;
            }
            "--emit-ast" => {
                emit_ast = true;
            }
            "--emit-tokens" => {
                emit_tokens = true;
            }
            "--disable-async" => {}
            "--unsafe-arrays" => {
                unsafe_arrays = true;
            }
            "--emit-mir" => {
                emit_mir = true;
            }
            "--emit-llvm" => {
                emit_llvm = true;
            }
            "-c" | "--compile" => {
                compile_only = true;
            }
            "-o" | "--output" => {
                if i + 1 < args.len() {
                    output_name = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: -o/--output requires an argument");
                    process::exit(1);
                }
            }
            "--stdlib-path" => {
                if i + 1 < args.len() {
                    cli_stdlib_path = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: --stdlib-path requires an argument");
                    process::exit(1);
                }
            }
            "--runtime-path" => {
                if i + 1 < args.len() {
                    cli_runtime_path = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: --runtime-path requires an argument");
                    process::exit(1);
                }
            }
            "--vt-stack" | "--vthread-stack" | "-Xss" => {
                if i + 1 < args.len() {
                    let val = &args[i + 1];
                    if let Some(bytes) = parse_cli_size(val) {
                        cli_vt_stack = Some(bytes.max(MIN_VTHREAD_STACK_SIZE));
                    } else {
                        eprintln!("Error: invalid stack size '{}'. Expected e.g. 2k, 4kb, 64k", val);
                        process::exit(1);
                    }
                    i += 1;
                } else {
                    eprintln!("Error: {} requires a size argument (e.g. 2k, 4k, 64k)", arg);
                    process::exit(1);
                }
            }
            "-I" => {
                if i + 1 < args.len() {
                    cli_include_dirs.push(std::path::PathBuf::from(&args[i + 1]));
                    i += 1;
                } else {
                    eprintln!("Error: -I requires a directory argument");
                    process::exit(1);
                }
            }
            "-L" => {
                if i + 1 < args.len() {
                    cli_lib_dirs.push(std::path::PathBuf::from(&args[i + 1]));
                    i += 1;
                } else {
                    eprintln!("Error: -L requires a directory argument");
                    process::exit(1);
                }
            }
            "-l" => {
                if i + 1 < args.len() {
                    cli_libs.push(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: -l requires a library name argument");
                    process::exit(1);
                }
            }
            "--target" => {
                if i + 1 < args.len() {
                    cli_target = Some(args[i + 1].clone());
                    i += 1;
                } else {
                    eprintln!("Error: --target requires a target triple argument");
                    process::exit(1);
                }
            }

            _ if arg.starts_with("--vt-stack=") => {
                let val = &arg["--vt-stack=".len()..];
                if let Some(bytes) = parse_cli_size(val) {
                    cli_vt_stack = Some(bytes.max(MIN_VTHREAD_STACK_SIZE));
                } else {
                    eprintln!("Error: invalid stack size '{}'. Expected e.g. 2k, 4kb, 64k", val);
                    process::exit(1);
                }
            }
            _ if arg.starts_with("--vthread-stack=") => {
                let val = &arg["--vthread-stack=".len()..];
                if let Some(bytes) = parse_cli_size(val) {
                    cli_vt_stack = Some(bytes.max(MIN_VTHREAD_STACK_SIZE));
                } else {
                    eprintln!("Error: invalid stack size '{}'. Expected e.g. 2k, 4kb, 64k", val);
                    process::exit(1);
                }
            }
            _ if arg.starts_with("-Xss=") => {
                let val = &arg["-Xss=".len()..];
                if let Some(bytes) = parse_cli_size(val) {
                    cli_vt_stack = Some(bytes.max(MIN_VTHREAD_STACK_SIZE));
                } else {
                    eprintln!("Error: invalid stack size '{}'. Expected e.g. 2k, 4kb, 64k", val);
                    process::exit(1);
                }
            }
            _ if arg.starts_with("-Xss") && arg.len() > 4 => {
                let val = &arg[4..];
                if let Some(bytes) = parse_cli_size(val) {
                    cli_vt_stack = Some(bytes.max(MIN_VTHREAD_STACK_SIZE));
                } else {
                    eprintln!("Error: invalid stack size '{}'. Expected e.g. 2k, 4kb, 64k", val);
                    process::exit(1);
                }
            }
            _ if arg.starts_with("-I") && arg.len() > 2 => {
                cli_include_dirs.push(std::path::PathBuf::from(&arg[2..]));
            }
            _ if arg.starts_with("-L") && arg.len() > 2 => {
                cli_lib_dirs.push(std::path::PathBuf::from(&arg[2..]));
            }
            _ if arg.starts_with("-l") && arg.len() > 2 => {
                cli_libs.push(arg[2..].to_string());
            }
            _ if arg.starts_with("--target=") => {
                cli_target = Some(arg["--target=".len()..].to_string());
            }

            _ if arg.starts_with("-") => {
                eprintln!(
                    "Unknown option: {}. Run 'tejxc --help' for all options.",
                    arg
                );
                process::exit(1);
            }
            _ => {
                input_files.push(arg.clone());
            }
        }
        i += 1;
    }

    if input_files.is_empty() {
        eprintln!("Error: No input files specified.");
        print_help();
        process::exit(1);
    }

    // For now, we mainly focus on the first input file for the primary compilation
    let filename = input_files[0].clone();

    let contents = fs::read_to_string(&filename).unwrap_or_else(|err| {
        eprintln!("Error reading file {}: {}", filename, err);
        process::exit(1);
    });

    let t_start_frontend = std::time::Instant::now();
    let mut lexer = Lexer::new(&contents, &filename);
    let tokens = lexer.tokenize();

    if !lexer.errors.is_empty() {
        report_diagnostics("Lexing", &lexer.errors, &filename, &contents);
        process::exit(1);
    }

    if emit_tokens {
        for token in &tokens {
            println!("{:?}", token);
        }
    }

    let mut parser = Parser::new(tokens, &filename);

    let program = parser.parse_program();

    if parser.has_errors() {
        report_diagnostics("Parsing", parser.get_errors(), &filename, &contents);
        process::exit(1);
    }

    let t_frontend = t_start_frontend.elapsed();

    if emit_ast {
        println!("{:#?}", program);
    }

    // Resolve stdlib path using centralized paths module
    let stdlib_resolved = crate::common::paths::resolve_stdlib_path(cli_stdlib_path.as_deref());
    let _runtime_resolved = crate::common::paths::resolve_runtime_path(cli_runtime_path.as_deref());

    let mut lowering = Lowering::new();

    *lowering.stdlib_path.borrow_mut() = stdlib_resolved;
    *lowering.include_dirs.borrow_mut() = cli_include_dirs.clone();
    *lowering.filename.borrow_mut() = filename.clone();
    let base_path = Path::new(&filename).parent().unwrap_or(Path::new("."));

    // Resolve imports before type checking
    let t_start_imports = std::time::Instant::now();
    let mut processed_files = std::collections::HashSet::new();
    let mut import_stack = Vec::new();
    let mut initial_file_path = None;
    if let Ok(p) = std::fs::canonicalize(base_path.join(&filename)) {
        processed_files.insert(p.clone());
        import_stack.push(p.clone());
        initial_file_path = Some(p);
    }
    let (resolved_statements, resolved_statement_files) = lowering.resolve_imports(
        program.statements,
        base_path,
        &mut processed_files,
        &mut import_stack,
        initial_file_path.as_deref(),
    );

    let mut merged_program = ast::Program {
        statements: resolved_statements,
    };

    // Check for lowering errors (import validation happens in resolve_imports)
    {
        let diagnostics = lowering.diagnostics.borrow();
        let unique = unique_diagnostics(diagnostics.iter());
        if !unique.is_empty() {
            report_diagnostics("Import resolution", &unique, &filename, &contents);
            process::exit(1);
        }
    }
    let t_imports = t_start_imports.elapsed();

    let t_start_typecheck = std::time::Instant::now();
    let mut type_checker = TypeChecker::new();

    type_checker.set_import_access(lowering.import_access.borrow().clone());
    if type_checker
        .check(&merged_program, &filename, Some(&resolved_statement_files))
        .is_err()
    {
        let unique = unique_diagnostics(type_checker.diagnostics.iter());
        report_diagnostics("Type checking", &unique, &filename, &contents);
        process::exit(1);
    }
    let t_typecheck = t_start_typecheck.elapsed();

    if check_only {
        if show_stats {
            eprintln!("\n=== Compilation Statistics (Check Only) ===");
            eprintln!("  Frontend (Lex & Parse):     {:>8.2?}", t_frontend);
            eprintln!("  Import Resolution:          {:>8.2?}", t_imports);
            eprintln!("  Semantic Analysis / Types:  {:>8.2?}", t_typecheck);
            eprintln!("  --------------------------------------");
            eprintln!("  Total Time:                 {:>8.2?}", total_timer.elapsed());
            eprintln!("==========================================\n");
        }
        if verbose {
            eprintln!("Check finished successfully: 0 errors.");
        }
        process::exit(0);
    }

    apply_inferred_return_types_to_program(
        &mut merged_program,
        &resolved_statement_files,
        &filename,
        &type_checker,
    );

    let t_start_lowering = std::time::Instant::now();
    lowering.lambda_inferred_types = type_checker.lambda_inferred_types;
    lowering.lambda_inferred_returns = type_checker.lambda_inferred_returns;
    lowering.call_instantiations = type_checker.call_instantiations;
    *lowering.generic_instantiations.borrow_mut() = type_checker.generic_instantiations;
    lowering.function_instantiations = type_checker.function_instantiations;

    let lowering_result = lowering.lower(&merged_program, base_path);

    // Check for lowering errors (import validation, etc.)
    {
        let diagnostics = lowering.diagnostics.borrow();
        let unique = unique_diagnostics(diagnostics.iter());
        if !unique.is_empty() {
            report_diagnostics("Lowering", &unique, &filename, &contents);
            process::exit(1);
        }
    }

    let mut mir_functions = Vec::new();
    let mir_optimizer = mir_opt::MIROptimizer::new();

    for hir_func in &lowering_result.functions {
        let _name = match hir_func {
            crate::middle::hir::HIRStatement::Function { name, .. } => name.clone(),
            _ => "unknown".to_string(),
        };
        let mut mir_lowering = MIRLowering::new(
            lowering_result.signatures.clone(),
            lowering_result.class_fields.clone(),
        );
        let mut mir_func = mir_lowering.lower(hir_func);
        mir_optimizer.optimize(&mut mir_func);
        mir_functions.push(mir_func);
    }
    let t_lowering = t_start_lowering.elapsed();

    if emit_mir {
        for mir_func in &mir_functions {
            eprintln!("--- BEFORE BORROW CHECKER ---");
            eprintln!("{:?}", mir_func);
        }
    }

    if emit_mir {
        for mir_func in &mir_functions {
            eprintln!("--- MIR ---");
            eprintln!("{:?}", mir_func);
        }
    }

    let t_start_codegen = std::time::Instant::now();
    let mut codegen = CodeGen::new();
    codegen.unsafe_arrays = unsafe_arrays;
    codegen.source_file = filename.clone();
    codegen.class_fields = lowering_result.class_fields;
    codegen.class_methods = lowering_result.class_methods;
    codegen.class_parents = lowering_result.class_parents;
    codegen.function_display_names = lowering_result.function_display_names;
    codegen.class_display_names = lowering_result.class_display_names;
    codegen.vt_stack_size = cli_vt_stack;
    let llvm_code =
        codegen.generate_with_blocks(&mir_functions, lowering_result.captured_vars_by_function);
    let t_codegen = t_start_codegen.elapsed();

    if emit_llvm {
        eprintln!("{}", llvm_code);
    }

    let output_name = output_name.unwrap_or_else(|| {
        if let Some(pos) = filename.rfind('.') {
            filename[..pos].to_string()
        } else {
            "a.out".to_string()
        }
    });

    let temp_ll_file = format!("{}.ll", output_name);
    fs::write(&temp_ll_file, &llvm_code).unwrap_or_else(|err| {
        eprintln!("Error writing LLVM IR: {}", err);
        process::exit(1);
    });

    let t_start_link = std::time::Instant::now();
    let mut linker = Linker::new(Path::new(&output_name));
    linker.set_opt_level(&opt_level);
    linker.set_debug(debug_symbols);
    linker.set_emit_asm(emit_asm);
    linker.set_verbose(verbose);
    if let Some(ref target) = cli_target {
        linker.set_target(target);
    }
    for dir in &cli_lib_dirs {
        linker.add_lib_dir(dir);
    }
    for lib in &cli_libs {
        linker.add_lib(lib);
    }
    linker.add_object(Path::new(&temp_ll_file));

    // Use the resolved runtime path
    let runtime_path = crate::common::paths::resolve_runtime_path(cli_runtime_path.as_deref());
    if !runtime_path.exists() {
        eprintln!("Error: Runtime library not found at {:?}", runtime_path);
        process::exit(1);
    }
    linker.add_object(&runtime_path);

    // Add other input files (objects or libraries)
    for i in 1..input_files.len() {
        linker.add_object(Path::new(&input_files[i]));
    }

    if compile_only {
        linker.set_compile_only(true);
    }

    match linker.link() {
        Ok(_) => {
            let _ = fs::remove_file(&temp_ll_file);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            let _ = fs::remove_file(&temp_ll_file);
            process::exit(1);
        }
    }
    let t_link = t_start_link.elapsed();

    if show_stats {
        eprintln!("\n=== Compilation Statistics ===");
        eprintln!("  Frontend (Lex & Parse):     {:>8.2?}", t_frontend);
        eprintln!("  Import Resolution:          {:>8.2?}", t_imports);
        eprintln!("  Semantic Analysis / Types:  {:>8.2?}", t_typecheck);
        eprintln!("  Lowering & Optimization:    {:>8.2?}", t_lowering);
        eprintln!("  LLVM Code Generation:       {:>8.2?}", t_codegen);
        eprintln!("  Assembly & Linking:         {:>8.2?}", t_link);
        eprintln!("  --------------------------------------");
        eprintln!("  Total Build Time:           {:>8.2?}", total_timer.elapsed());
        eprintln!("==============================\n");
    }

    if run_after_compile {
        let binary_path = if output_name.starts_with('/') || output_name.starts_with('.') {
            output_name.clone()
        } else {
            format!("./{}", output_name)
        };
        let mut child = process::Command::new(&binary_path);
        if let Some(vt_bytes) = cli_vt_stack {
            child.env("TEJX_VT_STACK", vt_bytes.to_string());
        }
        child.args(&run_args);
        match child.status() {
            Ok(status) => {
                process::exit(status.code().unwrap_or(0));
            }
            Err(e) => {
                eprintln!("Error executing binary {}: {}", binary_path, e);
                process::exit(1);
            }
        }
    }
}

fn print_help() {
    println!("tejxc - The TejX Production Compiler");
    println!("Usage: tejxc [options] <file.tx> [-- <run_args>...]");
    println!();
    println!("Actions:");
    println!("  --check                 Perform syntax and type checking only without code generation");
    println!("  -r, --run               Compile and immediately execute the output program");
    println!("  -c, --compile           Compile to object file (.o); do not link");
    println!("  -S, --emit-asm          Emit assembly (.s); do not assemble or link");
    println!();
    println!("Optimization & Diagnostics:");
    println!("  -O0, -O1, -O2, -O3, -Os Code optimization level (default: -O3)");
    println!("  -g, --debug             Generate debug symbols for lldb/gdb");
    println!("  -Wall, -Wextra          Enable compiler diagnostic warnings");
    println!("  -Werror                 Treat warnings as errors");
    println!("  --stats, --time-report  Show compilation stage timings and statistics");
    println!("  --verbose               Show verbose compilation steps and linker commands");
    println!();
    println!("Intermediate Representations:");
    println!("  --emit-tokens           Print lexer token stream");
    println!("  --emit-ast              Print abstract syntax tree");
    println!("  --emit-mir              Print Mid-level IR (MIR) to stderr");
    println!("  --emit-llvm             Print LLVM IR to stderr");
    println!();
    println!("Include & Link Paths:");
    println!("  -I <dir>                Add directory to module search path");
    println!("  -L <dir>                Add directory to library search path");
    println!("  -l <lib>                Link with library <lib>");
    println!("  --target <triple>       Set cross-compilation target triple");
    println!();
    println!("Runtime & Virtual Threads:");
    println!("  --vt-stack <size>       Default virtual thread stack size (e.g. 2k, 4k, 64k, default: 2k)");
    println!("  -Xss <size>             Alias for --vt-stack (e.g. -Xss2k, -Xss64k)");
    println!("  --stdlib-path <path>    Override standard library path");
    println!("  --runtime-path <path>   Override runtime archive path (tejx_rt.a)");
    println!();
    println!("General:");
    println!("  -o, --output <file>     Specify output binary/object name");
    println!("  -v, --version           Print compiler version and target information");
    println!("  -h, --help              Print this help menu");
    println!();
    println!("Examples:");
    println!("  tejxc main.tx                         # Compile to executable");
    println!("  tejxc -r main.tx -- arg1 arg2         # Compile and run with args");
    println!("  tejxc --stats -O3 main.tx             # Compile with stage timing stats");
    println!("  tejxc -I ./modules -l m main.tx       # Include modules and link libm");
    println!("  tejxc --vt-stack 4k -r server.tx      # Run with 4 KB virtual thread stacks");
}

fn print_version() {
    println!("tejxc {} ({})", crate::common::version::VERSION, std::env::consts::ARCH);
    println!("LLVM backend: clang/cc toolchain");
    println!("Host: {}-{}", std::env::consts::ARCH, std::env::consts::OS);
    println!();
    println!("Environment Variables:");
    println!("  TEJX_VT_STACK=<size>   Default virtual thread stack size (e.g. 2k, 4k)");
    println!("  TEJX_MAIN_STACK=<size> Main thread stack size (default: 1m)");
    println!("  TEJXGC=<pct>           GC growth factor headroom % (default: 50)");
    println!("  TEJX_HEAP=<size>       Heap limit (e.g. 4gb, 512mb, default: 50% RAM)");
    println!("  CC=<compiler>          Override C compiler (default: cc/clang)");
}
