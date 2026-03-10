#![warn(unsafe_op_in_unsafe_fn)]
mod ast;
mod ast_transformer;
mod codegen;
mod diagnostics;
mod hir;
mod intrinsics;
mod lexer;
mod linker;
mod lowering;
mod mir;
mod mir_lowering;
mod mir_opt;
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

use codegen::CodeGen;
use lexer::Lexer;
use linker::Linker;
use lowering::Lowering;
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
    let mut emit_llvm = false;
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

    lowering.lambda_inferred_types = type_checker.lambda_inferred_types;
    lowering.generic_instantiations = type_checker.generic_instantiations;
    lowering.function_instantiations = type_checker.function_instantiations;

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
    let mir_optimizer = mir_opt::MIROptimizer::new();
    
    for hir_func in &lowering_result.functions {
        let _name = match hir_func {
            crate::hir::HIRStatement::Function { name, .. } => name.clone(),
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
    codegen.class_fields = lowering_result.class_fields;
    codegen.class_methods = lowering_result.class_methods;
    let llvm_code = codegen.generate_with_blocks(&mir_functions, lowering_result.captured_vars);

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
                // let _ = fs::remove_file(&temp_ll_file);
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
