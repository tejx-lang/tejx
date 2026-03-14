#![warn(unsafe_op_in_unsafe_fn)]
mod ast;
mod borrow_checker;
mod codegen;
mod diagnostics;
mod hir;
mod intrinsics;
mod lexer;
mod linker;
mod lowering;
mod mir;
mod mir_lowering;
mod parser;
mod token;
mod type_checker;
mod types;
#[path = "../wasm/src/wasm_codegen.rs"]
mod wasm_codegen;

use std::env;
use std::fs;
use std::path::Path;
use std::process;

use borrow_checker::BorrowChecker;
use codegen::CodeGen;
use lexer::Lexer;
use linker::Linker;
use lowering::Lowering;
use mir::{MIRInstruction, MIRValue};
use mir_lowering::MIRLowering;
use parser::Parser;
use type_checker::TypeChecker;
use wasm_codegen::WasmCodeGen;

const RUNTIME_LIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/libruntime.a"));

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut input_files = Vec::new();
    let mut async_enabled = true;
    let mut emit_mir = false;
    let mut _emit_llvm = true;
    let mut target_wasm = false;
    let mut compile_only = false;
    let mut unsafe_arrays = false;
    let mut output_name = None;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return;
            }
            "-v" | "--version" => {
                print_version();
                return;
            }
            "--disable-async" => {
                async_enabled = false;
            }
            "--unsafe-arrays" => {
                unsafe_arrays = true;
            }
            "--emit-mir" => {
                emit_mir = true;
            }
            "--emit-llvm" => {
                _emit_llvm = true;
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
            "--target" => {
                if i + 1 < args.len() {
                    if args[i + 1] == "wasm" {
                        target_wasm = true;
                    }
                    i += 1;
                }
            }
            _ if arg.starts_with("--target=") => {
                if arg.ends_with("wasm") {
                    target_wasm = true;
                }
            }
            _ if arg.starts_with("--") => {
                eprintln!("Unknown option: {}", arg);
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

    let mut lowering = Lowering::new();
    lowering.async_enabled = async_enabled;
    if let Ok(path) = std::env::var("stdlib-path") {
        *lowering.stdlib_path.borrow_mut() = std::path::PathBuf::from(path);
    } else if std::path::Path::new("stdlib").exists() {
        *lowering.stdlib_path.borrow_mut() = std::path::PathBuf::from("stdlib");
    }
    *lowering.filename.borrow_mut() = filename.clone();
    let base_path = Path::new(&filename).parent().unwrap_or(Path::new("."));

    // Resolve imports before type checking
    let mut processed_files = std::collections::HashSet::new();
    let mut import_stack = Vec::new();
    let mut initial_file_path = None;
    if let Ok(p) = std::fs::canonicalize(base_path.join(&filename)) {
        processed_files.insert(p.clone());
        import_stack.push(p.clone());
        initial_file_path = Some(p);
    }
    let resolved_statements = lowering.resolve_imports(
        program.statements,
        base_path,
        &mut processed_files,
        &mut import_stack,
        initial_file_path.as_deref(),
    );
    let merged_program = ast::Program {
        statements: resolved_statements,
    };

    // Check for lowering errors (import validation happens in resolve_imports)
    {
        let diagnostics = lowering.diagnostics.borrow();
        if !diagnostics.is_empty() {
            for diag in diagnostics.iter() {
                diag.report(&contents);
            }
            process::exit(1);
        }
    }

    let mut type_checker = TypeChecker::new();
    type_checker.async_enabled = async_enabled;
    if let Err(_) = type_checker.check(&merged_program, &filename) {
        eprintln!("Type Checking Failed:");
        for diag in &type_checker.diagnostics {
            diag.report(&contents);
        }
        process::exit(1);
    }

    let lowering_result = lowering.lower(&merged_program, base_path);

    // Check for lowering errors (import validation, etc.)
    {
        let diagnostics = lowering.diagnostics.borrow();
        if !diagnostics.is_empty() {
            for diag in diagnostics.iter() {
                diag.report(&contents);
            }
            process::exit(1);
        }
    }

    let mut mir_functions = Vec::new();
    for hir_func in &lowering_result.functions {
        let mut mir_lowering = MIRLowering::new(lowering_result.signatures.clone());
        let mir_func = mir_lowering.lower(hir_func);
        mir_functions.push(mir_func);
    }

    if emit_mir {
        for mir_func in &mir_functions {
            eprintln!("--- BEFORE BORROW CHECKER ---");
            eprintln!("{:?}", mir_func);
        }
    }

    let mut borrow_checker = BorrowChecker::new();
    for mir_func in &mut mir_functions {
        let (drops, reassignment_drops, dead_frees) = borrow_checker.check(mir_func, &filename);

        // Remove dead Free instructions
        {
            let mut by_block: std::collections::HashMap<usize, Vec<usize>> =
                std::collections::HashMap::new();
            for (block_idx, inst_idx) in dead_frees {
                by_block.entry(block_idx).or_default().push(inst_idx);
            }
            for (block_idx, mut inst_indices) in by_block {
                inst_indices.sort_by(|a, b| b.cmp(a)); // Descending order
                let bb = &mut mir_func.blocks[block_idx];
                for inst_idx in inst_indices {
                    if inst_idx < bb.instructions.len() {
                        bb.instructions.remove(inst_idx);
                    }
                }
            }
        }

        // Inject reassignment drops: Free the old value before it's overwritten.
        // Process in reverse order within each block to maintain correct instruction indices.
        // Group by block_idx first, then sort by inst_idx descending within each block.
        {
            let mut by_block: std::collections::HashMap<usize, Vec<(usize, String)>> =
                std::collections::HashMap::new();
            for (block_idx, inst_idx, var_name) in reassignment_drops {
                by_block
                    .entry(block_idx)
                    .or_default()
                    .push((inst_idx, var_name));
            }
            for (block_idx, mut items) in by_block {
                // Sort descending by inst_idx so insertions don't shift later indices
                items.sort_by(|a, b| b.0.cmp(&a.0));
                items.dedup();
                let bb = &mut mir_func.blocks[block_idx];
                for (inst_idx, var_name) in items {
                    if let Some(ty) = mir_func.variables.get(&var_name) {
                        let line = if inst_idx < bb.instructions.len() {
                            bb.instructions[inst_idx].get_line()
                        } else {
                            0
                        };
                        bb.instructions.insert(
                            inst_idx,
                            MIRInstruction::Free {
                                value: MIRValue::Variable {
                                    name: var_name,
                                    ty: ty.clone(),
                                },
                                line,
                            },
                        );
                    }
                }
            }
        }

        // Inject leaf-block drops (existing logic): Free live variables before Return/Throw
        for (block_idx, var_names) in drops {
            let bb = &mut mir_func.blocks[block_idx];
            let mut insert_idx = bb.instructions.len();
            if insert_idx > 0 {
                match bb.instructions[insert_idx - 1] {
                    MIRInstruction::Return { .. }
                    | MIRInstruction::Jump { .. }
                    | MIRInstruction::Branch { .. }
                    | MIRInstruction::Throw { .. }
                    | MIRInstruction::TrySetup { .. } => {
                        insert_idx -= 1;
                    }
                    _ => {}
                }
            }
            for var_name in var_names {
                if let Some(ty) = mir_func.variables.get(&var_name) {
                    let line = if insert_idx > 0 {
                        bb.instructions[insert_idx - 1].get_line()
                    } else {
                        0
                    };
                    bb.instructions.insert(
                        insert_idx,
                        MIRInstruction::Free {
                            value: MIRValue::Variable {
                                name: var_name,
                                ty: ty.clone(),
                            },
                            line,
                        },
                    );
                    insert_idx += 1;
                }
            }
        }
    }

    if emit_mir {
        for mir_func in &mir_functions {
            eprintln!("--- AFTER BORROW CHECKER ---");
            eprintln!("{:?}", mir_func);
        }
    }

    if !borrow_checker.errors.is_empty() {
        for diag in &borrow_checker.errors {
            diag.report(&contents);
        }
        process::exit(1);
    }

    if target_wasm {
        let mut wasm_codegen = WasmCodeGen::new();
        let wat = wasm_codegen.generate_wat(&mir_functions);

        let wasm_bytes = wat::parse_str(&wat).unwrap_or_else(|err| {
            eprintln!("WAT to WASM conversion failed: {}", err);
            process::exit(1);
        });

        let output_wasm = if let Some(pos) = filename.rfind('.') {
            format!("{}.wasm", &filename[..pos])
        } else {
            "a.wasm".to_string()
        };

        fs::write(&output_wasm, wasm_bytes).unwrap_or_else(|err| {
            eprintln!("Error writing WASM file: {}", err);
            process::exit(1);
        });

        println!("Successfully compiled to {}", output_wasm);
        return;
    }

    let mut codegen = CodeGen::new();
    codegen.unsafe_arrays = unsafe_arrays;
    let llvm_code = codegen.generate_with_blocks(&mir_functions, lowering_result.captured_vars);

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

    let mut linker = Linker::new(Path::new(&output_name));
    linker.add_object(Path::new(&temp_ll_file));

    // Handle compiler-side runtime (libruntime.a)
    let temp_runtime_path = env::temp_dir().join(format!("libruntime_{}.a", process::id()));
    fs::write(&temp_runtime_path, RUNTIME_LIB).unwrap_or_else(|err| {
        eprintln!("Error writing runtime library: {}", err);
        process::exit(1);
    });
    linker.add_object(&temp_runtime_path);

    // Add other input files (objects or libraries)
    for i in 1..input_files.len() {
        linker.add_object(Path::new(&input_files[i]));
    }

    if compile_only {
        // Just rename temp_ll_file to something like .o if we were doing object generation
        // But here we emit .ll and the linker converts to .o and then links.
        // If compile_only is set, we'll stop after generating the object file.
        linker.set_compile_only(true);
    }

    match linker.link() {
        Ok(_) => {
            // Success - cleanup temp files if not compile_only or if otherwise needed
            if !compile_only {
                let _ = fs::remove_file(&temp_ll_file);
                let _ = fs::remove_file(&temp_runtime_path);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            // Keep .ll file for debugging
            process::exit(1);
        }
    }
}

fn print_help() {
    println!("tejxc - TejX Compiler");
    println!("Usage: tejxc [options] <input_files>");
    println!("");
    println!("Options:");
    println!("  -h, --help            Show this help message");
    println!("  -v, --version         Show version information");
    println!("  -o, --output <file>   Specify output file name");
    println!("  -c, --compile         Compile only; do not link");
    println!("  --disable-async       Disable async/await features");
    println!("  --emit-mir            Print MIR to stderr");
    println!("  --emit-llvm           Print LLVM IR to stderr");
    println!("  --target <target>     Specify target (e.g., wasm)");
    println!("");
    println!("Examples:");
    println!("  tejxc main.tx                        Compile and link main.tx");
    println!("  tejxc -o myapp main.tx util.tx       Compile and link multiple files");
    println!("  tejxc -c main.tx                     Compile main.tx to object file");
    println!("  tejxc main.o helper.o -o myapp       Link existing object files");
}

fn print_version() {
    println!("tejxc version 0.2.0");
}
