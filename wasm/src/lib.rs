#![allow(unsafe_op_in_unsafe_fn)]

// Re-declare modules from main src directory using #[path]
#[path = "../../src/token.rs"] pub mod token;
#[path = "../../src/lexer.rs"] pub mod lexer;
#[path = "../../src/parser.rs"] pub mod parser;
#[path = "../../src/ast.rs"] pub mod ast;
#[path = "../../src/types.rs"] pub mod types;
#[path = "../../src/hir.rs"] pub mod hir;
#[path = "../../src/lowering.rs"] pub mod lowering;
#[path = "../../src/runtime.rs"] pub mod runtime;
#[path = "../../src/type_checker.rs"] pub mod type_checker;
#[path = "../../src/mir.rs"] pub mod mir;
#[path = "../../src/mir_lowering.rs"] pub mod mir_lowering;
#[path = "../../src/borrow_checker.rs"] pub mod borrow_checker;
#[path = "../../src/codegen.rs"] pub mod codegen;
#[path = "../../src/linker.rs"] pub mod linker;
#[path = "../../src/diagnostics.rs"] pub mod diagnostics;

// WASM-specific modules
pub mod wasm_codegen;

use std::path::Path;
use crate::lexer::Lexer;
use crate::parser::Parser;
use crate::type_checker::TypeChecker;
use crate::lowering::Lowering;
use crate::mir_lowering::MIRLowering;
use crate::borrow_checker::BorrowChecker;
use crate::wasm_codegen::WasmCodeGen;

pub fn compile_to_wat(source: String, filename: String, async_enabled: bool) -> Result<String, String> {
    let mut lexer = Lexer::new(&source, &filename);
    let tokens = lexer.tokenize();

    if !lexer.errors.is_empty() {
        let err = &lexer.errors[0];
        return Err(error_json(&err.message, err.line, err.col, &render_diagnostic(err, &source)));
    }

    let mut parser = Parser::new(tokens, &filename);
    parser.async_enabled = async_enabled;
    let program = parser.parse_program();

    if parser.has_errors() {
        let err = &parser.get_errors()[0];
        return Err(error_json(&err.message, err.line, err.col, &render_diagnostic(err, &source)));
    }

    let mut type_checker = TypeChecker::new();
    type_checker.async_enabled = async_enabled;
    if let Err(_) = type_checker.check(&program, &filename) {
        let err = &type_checker.diagnostics[0];
        return Err(error_json(&err.message, err.line, err.col, &render_diagnostic(err, &source)));
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
        let (_, _) = borrow_checker.check(mir_func, &filename);
        // We skip drop injection for now in Wasm as it's a prototype
    }

    let mut wasm_codegen = WasmCodeGen::new();
    Ok(wasm_codegen.generate_wat(&mir_functions))
}

fn escape_json(s: &str) -> String {
    let mut output = String::new();
    for c in s.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            c => output.push(c),
        }
    }
    output
}

fn error_json(msg: &str, line: usize, col: usize, full_error: &str) -> String {
    format!(
        r#"{{"error":true,"message":"{}","line":{},"col":{},"full_error":"{}"}}"#,
        escape_json(msg), line, col, escape_json(full_error)
    )
}

fn render_diagnostic(diag: &crate::diagnostics::Diagnostic, source: &str) -> String {
    let mut output = String::new();
    let sev_name = match diag.severity {
        crate::diagnostics::Severity::Error   => "error",
    };

    if diag.code.is_empty() {
        output.push_str(&format!("{}: {}\n", sev_name, diag.message));
    } else {
        output.push_str(&format!("{}[{}]: {}\n", sev_name, diag.code, diag.message));
    }

    output.push_str(&format!("  --> {}:{}:{}\n", diag.file, diag.line, diag.col));

    let lines: Vec<&str> = source.lines().collect();
    if diag.line > 0 && diag.line <= lines.len() {
        let line_content = lines[diag.line - 1];
        let line_num_str = diag.line.to_string();
        let pad = " ".repeat(line_num_str.len());

        if diag.line >= 2 {
            let prev_line = lines[diag.line - 2];
            let prev_num = (diag.line - 1).to_string();
            let prev_pad = " ".repeat(line_num_str.len().saturating_sub(prev_num.len()));
            output.push_str(&format!("  {} |\n", pad));
            output.push_str(&format!("  {}{} | {}\n", prev_pad, prev_num, prev_line));
        } else {
            output.push_str(&format!("  {} |\n", pad));
        }

        output.push_str(&format!("  {} | {}\n", line_num_str, line_content));

        let mut pointer = String::new();
        for _ in 0..diag.col.saturating_sub(1) {
            pointer.push(' ');
        }
        for _ in 0..diag.length.max(1) {
            pointer.push('^');
        }

        let inline_label = diag.label.as_deref().unwrap_or(&diag.message);
        output.push_str(&format!("  {} | {}{}\n", pad, pointer, inline_label));
        output.push_str(&format!("  {} |\n", pad));

        if let Some(hint) = &diag.hint {
            output.push_str(&format!("  {} = hint: {}\n", pad, hint));
        }
    } else if diag.line > lines.len() {
        let line_num = diag.line;
        let pad = " ".repeat(line_num.to_string().len());
        output.push_str(&format!("  {} |\n", pad));
        output.push_str(&format!("  {} | (EOF)\n", line_num));
        output.push_str(&format!("  {} | ^{}\n", pad, diag.message));
        output.push_str(&format!("  {} |\n", pad));
        if let Some(hint) = &diag.hint {
            output.push_str(&format!("  {} = hint: {}\n", pad, hint));
        }
    } else {
        output.push_str(&format!("  (Unexpected line {} with total lines: {})\n", diag.line, lines.len()));
    }
    output
}

pub fn compile_to_wasm(source: String, filename: String, async_enabled: bool) -> Result<Vec<u8>, String> {
    let wat = compile_to_wat(source, filename, async_enabled)?;
    wat::parse_str(wat).map_err(|e| format!("WAT to WASM conversion failed: {}", e))
}

// --- C-style FFI for manual Wasm usage ---

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn compiler_log(ptr: *const u8, len: usize);
}

fn debug_print(msg: &str) {
    unsafe { compiler_log(msg.as_ptr(), msg.len()); }
}

static mut LAST_RESULT: Vec<u8> = Vec::new();

#[unsafe(no_mangle)]
pub extern "C" fn tejx_alloc(size: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(size);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tejx_free(ptr: *mut u8, size: usize) {
    let _ = Vec::from_raw_parts(ptr, 0, size);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tejx_compile(
    src_ptr: *const u8, 
    src_len: usize, 
    file_ptr: *const u8, 
    file_len: usize, 
    async_enabled: bool
) -> *const u8 {
    let source = String::from_utf8_lossy(std::slice::from_raw_parts(src_ptr, src_len)).into_owned();
    let filename = String::from_utf8_lossy(std::slice::from_raw_parts(file_ptr, file_len)).into_owned();
    
    debug_print(&format!("WASM Compiler: Starting generic compilation for file: {}", filename));
    let result = match compile_to_wasm(source, filename, async_enabled) {
        Ok(res) => {
            debug_print(&format!("WASM Compiler: Compilation Success ({} bytes)", res.len()));
            res
        },
        Err(e) => {
            debug_print(&format!("WASM Compiler: Compilation Error: {}", e));
            format!("WASM_COMPILE_ERROR: {}", e).into_bytes()
        },
    };

    let last_result = unsafe { &mut *std::ptr::addr_of_mut!(LAST_RESULT) };
    *last_result = result;
    last_result.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tejx_get_result_len() -> usize {
    unsafe { (*std::ptr::addr_of!(LAST_RESULT)).len() }
}
