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
use std::path::Path;

use lexer::Lexer;
use parser::Parser;
use type_checker::TypeChecker;
use lowering::Lowering;
use mir::{MIRInstruction, MIRValue};
use mir_lowering::MIRLowering;
use borrow_checker::BorrowChecker;
use codegen::CodeGen;
use linker::Linker;

const RUNTIME_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libruntime.a"));

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: tejxr [options] <filename>");
        eprintln!("Options:");
        eprintln!("  --disable-async    Disable async/await features");
        process::exit(1);
    }

    let mut filename = String::new();
    let mut async_enabled = true;

    let mut emit_mir = false;
    let mut emit_llvm = false;
    for arg in args.iter().skip(1) {
        if arg == "--disable-async" {
            async_enabled = false;
        } else if arg == "--emit-mir" {
             emit_mir = true;
        } else if arg == "--emit-llvm" {
             emit_llvm = true;
        } else if arg.starts_with("--") {
            eprintln!("Unknown option: {}", arg);
            process::exit(1);
        } else {
            filename = arg.clone();
        }
    }

    if filename.is_empty() {
        eprintln!("Error: No input file specified.");
        process::exit(1);
    }

    let contents = fs::read_to_string(&filename).unwrap_or_else(|err| {
        eprintln!("Error reading file {}: {}", filename, err);
        process::exit(1);
    });


    let mut lexer = Lexer::new(&contents, &filename);
    let tokens = lexer.tokenize();


    if !lexer.errors.is_empty() {
        eprintln!("Lexing failed with errors:");
        for diag in &lexer.errors {
            diag.report(&contents);
        }
        process::exit(1);
    }


    let mut parser = Parser::new(tokens, &filename);
    parser.async_enabled = async_enabled;
    let program = parser.parse_program();


    if parser.has_errors() {
        eprintln!("Parsing failed with errors:");
        for diag in parser.get_errors() {
            diag.report(&contents);
        }
        process::exit(1);
    }


    let mut type_checker = TypeChecker::new();
    type_checker.async_enabled = async_enabled;
    if let Err(_) = type_checker.check(&program, &filename) {
        eprintln!("Type Checking Failed:");
        for diag in &type_checker.diagnostics {
            diag.report(&contents);
        }
        process::exit(1);
    }


    let mut lowering = Lowering::new();
    lowering.async_enabled = async_enabled;
    let base_path = Path::new(&filename).parent().unwrap_or(Path::new("."));
    let lowering_result = lowering.lower(&program, base_path);

    let mut mir_functions = Vec::new();
    for hir_func in &lowering_result.functions {
        let mut mir_lowering = MIRLowering::new(lowering_result.signatures.clone());
        let mir_func = mir_lowering.lower(hir_func);
        mir_functions.push(mir_func);
    }

    let mut borrow_checker = BorrowChecker::new();
    for mir_func in &mut mir_functions {
        let drops = borrow_checker.check(mir_func, &filename);
        for (block_idx, var_names) in drops {
            let bb = &mut mir_func.blocks[block_idx];
            let mut insert_idx = bb.instructions.len();
            if insert_idx > 0 {
                match bb.instructions[insert_idx - 1] {
                    MIRInstruction::Return { .. } | MIRInstruction::Jump { .. } | MIRInstruction::Branch { .. } | MIRInstruction::Throw { .. } => {
                        insert_idx -= 1;
                    }
                    _ => {}
                }
            }
            for var_name in var_names {
                if let Some(ty) = mir_func.variables.get(&var_name) {
                    let line = if insert_idx > 0 { bb.instructions[insert_idx - 1].get_line() } else { 0 };
                    bb.instructions.insert(insert_idx, MIRInstruction::Free {
                        value: MIRValue::Variable { name: var_name, ty: ty.clone() },
                        line,
                    });
                    insert_idx += 1;
                }
            }
        }
    }

    if emit_mir {
        for mir_func in &mir_functions {
            eprintln!("{:?}", mir_func);
        }
    }

    if !borrow_checker.errors.is_empty() {
        for diag in &borrow_checker.errors {
            diag.report(&contents);
        }
        process::exit(1);
    }

    let mut codegen = CodeGen::new();
    let llvm_code = codegen.generate_with_blocks(&mir_functions);

    if emit_llvm {
        eprintln!("{}", llvm_code);
    }

    let output_name = if let Some(pos) = filename.rfind('.') {
        &filename[..pos]
    } else {
        "a.out"
    };

    let temp_ll_file = format!("{}.ll", output_name);
    fs::write(&temp_ll_file, &llvm_code).unwrap_or_else(|err| {
        eprintln!("Error writing LLVM IR: {}", err);
        process::exit(1);
    });

    let pid = process::id();
    let temp_dir = env::temp_dir();
    let runtime_lib_path = temp_dir.join(format!("libruntime_{}.a", pid));
    
    if let Err(e) = fs::write(&runtime_lib_path, RUNTIME_LIB) {
        eprintln!("Error writing embedded runtime to temp file: {}", e);
        let _ = fs::remove_file(&temp_ll_file);
        process::exit(1);
    }

    let mut linker = Linker::new(Path::new(output_name));
    linker.add_object(Path::new(&temp_ll_file));
    linker.add_object(&runtime_lib_path);

    match linker.link() {
        Ok(_) => {
            // Success - cleanup temp files
            // let _ = fs::remove_file(&temp_ll_file);
            let _ = fs::remove_file(&runtime_lib_path);
        },
        Err(e) => {
            eprintln!("Error: {}", e);
            let _ = fs::remove_file(&runtime_lib_path);
            // Keep .ll file for debugging
            process::exit(1);
        }
    }
}
