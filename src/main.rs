#![allow(unsafe_op_in_unsafe_fn)]
mod token;
mod lexer;
mod parser;
mod ast;
mod types;
mod hir;
mod lowering;
pub mod runtime;
mod type_checker;
mod mir;
mod mir_lowering;
mod borrow_checker;
mod codegen;
mod linker;
mod diagnostics;

use std::env;
use std::fs;
use std::process;
use std::io::Write;
use std::path::{Path, PathBuf};

// Modules are declared above

use lexer::Lexer;
use parser::Parser;
use type_checker::TypeChecker;
use lowering::Lowering;
use mir_lowering::MIRLowering;
use borrow_checker::BorrowChecker;
use codegen::CodeGen;
use linker::Linker;

// Embed the pre-compiled runtime library
const RUNTIME_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libruntime.a"));

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
    let mut lexer = Lexer::new(&contents, filename);
    let tokens = lexer.tokenize();
    let mut has_errors = false;

    if !lexer.errors.is_empty() {
        eprintln!("Lexing failed with errors:");
        for diag in &lexer.errors {
            diag.report(&contents);
        }
        has_errors = true;
    }

    // 2. Parsing
    let mut parser = Parser::new(tokens, filename);
    let program = parser.parse_program();

    if parser.has_errors() {
        eprintln!("Parsing failed with errors:");
        for diag in parser.get_errors() {
            diag.report(&contents);
        }
        has_errors = true;
    }

    if has_errors {
        process::exit(1);
    }

    // 3. Type Checker
    let mut type_checker = TypeChecker::new();
    match type_checker.check(&program, filename) {
        Ok(_) => {},
        Err(_) => {
            eprintln!("Type Checking Failed:");
            for diag in &type_checker.diagnostics {
                diag.report(&contents);
            }
            process::exit(1);
        }
    }

    // 4. Lowering AST → HIR (produces multiple functions)
    let lowering = Lowering::new();
    let base_path = Path::new(filename).parent().unwrap_or(Path::new("."));
    let lowering_result = lowering.lower(&program, base_path);

    // 5. HIR → MIR Lowering (each function separately)
    let mut mir_functions = Vec::new();
    for hir_func in &lowering_result.functions {
        let mut mir_lowering = MIRLowering::new(lowering_result.signatures.clone());
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
        &filename[..pos]
    } else {
        "a.out"
    };

    let temp_ll_file = format!("{}.ll", output_name);
    // Write LLVM IR to a temporary file
    fs::write(&temp_ll_file, &llvm_code).unwrap_or_else(|err| {
        eprintln!("Error writing LLVM IR: {}", err);
        process::exit(1);
    });

    // 8. Link with Embedded Runtime using Linker module
    let pid = process::id();
    let temp_dir = env::temp_dir();
    let runtime_lib_path = temp_dir.join(format!("libruntime_{}.a", pid));
    
    // Write embedded library to disk so linker can use it
    if let Err(e) = fs::write(&runtime_lib_path, RUNTIME_LIB) {
        eprintln!("Error writing embedded runtime to temp file: {}", e);
        let _ = fs::remove_file(&temp_ll_file);
        process::exit(1);
    }

    let mut linker = Linker::new(Path::new(output_name));
    
    // Compiler drivers (cc/clang) verify extension to decide action (compile vs link)
    // Passing .ll directly often works for clang, but strict cc might want .o
    // But keeping .ll simplifies our codegen. 
    linker.add_object(Path::new(&temp_ll_file));
    linker.add_object(&runtime_lib_path);

    match linker.link() {
        Ok(_) => {
            // Success
        },
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("Hint: Ensure a C compiler (cc, clang, or gcc) is installed.");
            // Cleanup
            let _ = fs::remove_file(&runtime_lib_path);
            if env::var("TEJXR_DEBUG").is_err() {
                let _ = fs::remove_file(&temp_ll_file);
            }
            process::exit(1);
        }
    }

    // Cleanup temp files
    let _ = fs::remove_file(&runtime_lib_path);
    if env::var("TEJXR_DEBUG").is_err() {
        let _ = fs::remove_file(&temp_ll_file);
    }
}
