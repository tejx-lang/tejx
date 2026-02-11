mod token;
mod lexer;
mod parser;
mod ast;
mod types;
mod hir;
mod lowering;
mod type_checker;
mod mir;
mod mir_lowering;
mod borrow_checker;
mod codegen;

use std::env;
use std::fs;
use std::process;

use lexer::Lexer;
use parser::Parser;
use type_checker::TypeChecker;
use lowering::Lowering;
use mir_lowering::MIRLowering;
use borrow_checker::BorrowChecker;
use codegen::CodeGen;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: tejxr <filename>");
        process::exit(1);
    }

    let filename = &args[1];
    let contents = fs::read_to_string(filename).unwrap_or_else(|err| {
        eprintln!("Error reading file {}: {}", filename, err);
        process::exit(1);
    });

    // 1. Lexing
    let mut lexer = Lexer::new(&contents);
    let tokens = lexer.tokenize();

    // 2. Parsing
    let mut parser = Parser::new(tokens);
    let program = parser.parse_program();

    if parser.has_errors() {
        eprintln!("Parsing failed with errors:");
        for error in parser.get_errors() {
            eprintln!("  - {}", error);
        }
        process::exit(1);
    }

    // 3. Type Checker
    let mut type_checker = TypeChecker::new();
    match type_checker.check(&program) {
        Ok(_) => {},
        Err(e) => {
            eprintln!("Type checking warning: {}", e);
        }
    }

    // 4. Lowering AST → HIR (produces multiple functions)
    let lowering = Lowering::new();
    let base_path = std::path::Path::new(filename).parent().unwrap_or(std::path::Path::new("."));
    let lowering_result = lowering.lower(&program, base_path);

    // 5. HIR → MIR Lowering (each function separately)
    let mut mir_functions = Vec::new();
    for hir_func in &lowering_result.functions {
        let mut mir_lowering = MIRLowering::new();
        let mir_func = mir_lowering.lower(hir_func);
        mir_functions.push(mir_func);
    }

    // 6. Borrow Checking (each function)
    let mut borrow_checker = BorrowChecker::new();
    for mir_func in &mir_functions {
        borrow_checker.check(mir_func);
    }
    if !borrow_checker.errors.is_empty() {
        eprintln!("Borrow Checker Found Errors!");
        for err in &borrow_checker.errors {
            eprintln!("  - {}", err);
        }
        process::exit(1);
    }

    // 7. LLVM IR Generation (all functions)
    let mut code_gen = CodeGen::new();
    let llvm_code = code_gen.generate_with_blocks(&mir_functions);

    // Determine output file names
    let output_name = if let Some(pos) = filename.rfind('.') {
        filename[..pos].to_string()
    } else {
        "a.out".to_string()
    };

    let temp_file = format!("{}.ll", output_name);
    fs::write(&temp_file, &llvm_code).unwrap_or_else(|err| {
        eprintln!("Error writing LLVM IR: {}", err);
        process::exit(1);
    });

    // 8. Compile & Link with clang
    // Find runtime.c relative to the executable
    let exe_path = env::current_exe().unwrap_or_default();
    let exe_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
    
    // Search order: ../../libruntime.a (from target/release/), then ./libruntime.a (from project root)
    let runtime_candidates = vec![
        exe_dir.join("../../libruntime.a"),          // target/release/ → project root
        exe_dir.join("../../../libruntime.a"),        // target/debug/ → project root  
        std::path::PathBuf::from("libruntime.a"),     // current directory
    ];
    
    let runtime_path = runtime_candidates.iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            eprintln!("Error: libruntime.a not found. Searched: {:?}", runtime_candidates);
            process::exit(1);
        });

    let result = process::Command::new("clang++")
        .args(&["-O3", "-Wno-deprecated", "-Wno-override-module"])
        .arg(&temp_file)
        .arg(&runtime_path)
        .arg("-o")
        .arg(&output_name)
        .status();

    match result {
        Ok(status) if status.success() => {
            // let _ = fs::remove_file(&temp_file);
        }
        _ => {
            eprintln!("Compilation failed. LLVM IR saved to: {}", temp_file);
            process::exit(1);
        }
    }
}
