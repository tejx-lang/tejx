// Re-declare modules from main src directory using #[path]
#[path = "../../src/compiler/backend/mod.rs"]
pub mod backend;
#[path = "../../src/compiler/common/mod.rs"]
pub mod common;
#[path = "../../src/compiler/frontend/mod.rs"]
pub mod frontend;
#[path = "../../src/compiler/middle/mod.rs"]
pub mod middle;

// WASM-local runtime shim (provides StdLib metadata without OS dependencies)
pub mod runtime;
// WASM-specific modules
pub mod wasm_codegen;

use crate::common::diagnostics::Diagnostic;
use crate::frontend::ast::{ImportItem, Statement};
use crate::frontend::lexer::Lexer;
use crate::frontend::parser::Parser;
use crate::middle::lowering::Lowering;
use crate::middle::mir::{BasicBlock, MIRFunction, MIRInstruction, MIRValue};
use crate::middle::mir::lowering::MIRLowering;
use crate::middle::semantic::TypeChecker;
use crate::wasm_codegen::WasmCodeGen;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::{Path, PathBuf};

const PRELUDE_TX: &str = include_str!("../../src/library/core/prelude.tx");
const ARRAY_TX: &str = include_str!("../../src/library/core/array.tx");
const STRING_TX: &str = include_str!("../../src/library/core/string.tx");
const COLLECTIONS_TX: &str = include_str!("../../src/library/std/collections.tx");
const FS_TX: &str = include_str!("../../src/library/std/fs.tx");
const JSON_TX: &str = include_str!("../../src/library/std/json.tx");
const MATH_TX: &str = include_str!("../../src/library/std/math.tx");
const NET_TX: &str = include_str!("../../src/library/std/net.tx");
const SYSTEM_TX: &str = include_str!("../../src/library/std/system.tx");
const THREAD_TX: &str = include_str!("../../src/library/std/thread.tx");
const TIME_TX: &str = include_str!("../../src/library/std/time.tx");

const DEFAULT_FILENAME: &str = "main.tx";

/// RAW FFI: Allocate memory for JS to write into.
#[no_mangle]
pub extern "C" fn tejx_alloc(size: usize) -> *mut u8 {
    let mut buf = vec![0u8; size];
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// RAW FFI: Free memory allocated by tejx_alloc.
#[no_mangle]
pub extern "C" fn tejx_free(ptr: *mut u8, size: usize) {
    unsafe {
        let _ = Vec::from_raw_parts(ptr, size, size);
    }
}

/// Free a CString returned by the compiler exports.
#[no_mangle]
pub extern "C" fn tejx_cstring_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = CString::from_raw(ptr);
    }
}

fn builtin_modules() -> HashMap<String, String> {
    HashMap::from([
        ("core/prelude.tx".to_string(), PRELUDE_TX.to_string()),
        ("core/array.tx".to_string(), ARRAY_TX.to_string()),
        ("core/string.tx".to_string(), STRING_TX.to_string()),
        ("std:collections".to_string(), COLLECTIONS_TX.to_string()),
        ("std:fs".to_string(), FS_TX.to_string()),
        ("std:json".to_string(), JSON_TX.to_string()),
        ("std:math".to_string(), MATH_TX.to_string()),
        ("std:net".to_string(), NET_TX.to_string()),
        ("std:system".to_string(), SYSTEM_TX.to_string()),
        ("std:thread".to_string(), THREAD_TX.to_string()),
        ("std:time".to_string(), TIME_TX.to_string()),
    ])
}

fn default_codegen_config() -> Value {
    json!({
        "ops": {
            "Plus": "i64.add",
            "Minus": "i64.sub",
            "Star": "i64.mul",
            "Slash": "i64.div_s",
            "Modulo": "i64.rem_s",
            "EqualEqual": "i64.eq",
            "BangEqual": "i64.ne",
            "Less": "i64.lt_s",
            "LessEqual": "i64.le_s",
            "Greater": "i64.gt_s",
            "GreaterEqual": "i64.ge_s",
            "Ampersand": "i64.and",
            "Pipe": "i64.or",
            "Caret": "i64.xor",
            "LessLess": "i64.shl",
            "GreaterGreater": "i64.shr_s"
        }
    })
}

fn merge_json(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, overlay_value) in overlay_map {
                match base_map.get_mut(key) {
                    Some(base_value) => merge_json(base_value, overlay_value),
                    None => {
                        base_map.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (base_value, overlay_value) => *base_value = overlay_value.clone(),
    }
}

fn merged_codegen_config(user: Value) -> Value {
    let mut config = default_codegen_config();
    merge_json(&mut config, &user);
    config
}

fn extra_modules_from_config(config: &Value) -> HashMap<String, String> {
    let mut modules = HashMap::new();
    if let Some(obj) = config.get("modules").and_then(|v| v.as_object()) {
        for (name, src) in obj {
            if let Some(src) = src.as_str() {
                modules.insert(name.clone(), src.to_string());
            }
        }
    }
    modules
}

fn normalize_filename(filename: &str) -> String {
    let trimmed = filename.trim();
    if trimmed.is_empty() {
        DEFAULT_FILENAME.to_string()
    } else {
        trimmed.replace('\\', "/")
    }
}

fn normalize_virtual_import(source: &str, current_file: &str) -> String {
    let source = source.trim_matches('"').replace('\\', "/");
    if source.starts_with("std:") || source.starts_with("core/") {
        return source;
    }

    let mut path = if source.starts_with("./") || source.starts_with("../") {
        let base = Path::new(current_file).parent().unwrap_or(Path::new(""));
        base.join(&source)
    } else {
        PathBuf::from(&source)
    };

    if path.extension().is_none() {
        path.set_extension("tx");
    }

    path.to_string_lossy().replace('\\', "/")
}

fn is_core_module(module_id: &str) -> bool {
    module_id == "core/prelude.tx"
        || module_id == "core/array.tx"
        || module_id == "core/string.tx"
}

fn parse_program_virtual(
    source: &str,
    filename: &str,
) -> Result<crate::frontend::ast::Program, Diagnostic> {
    let mut lexer = Lexer::new(source, filename);
    let tokens = lexer.tokenize();
    if let Some(diag) = lexer.errors.first() {
        return Err(diag.clone());
    }

    let mut parser = Parser::new(tokens, filename);
    let program = parser.parse_program();
    if let Some(diag) = parser.get_errors().first() {
        return Err(diag.clone());
    }

    Ok(program)
}

fn collect_statement_names(stmt: &Statement, names: &mut HashSet<String>) {
    match stmt {
        Statement::FunctionDeclaration(func) => {
            names.insert(func.name.clone());
        }
        Statement::ClassDeclaration(class) => {
            names.insert(class.name.clone());
        }
        Statement::VarDeclaration {
            pattern: crate::frontend::ast::BindingNode::Identifier(name),
            ..
        } => {
            names.insert(name.clone());
        }
        Statement::BlockStmt { statements, .. } => {
            for statement in statements {
                collect_statement_names(statement, names);
            }
        }
        Statement::ExportDecl { declaration, .. } => {
            collect_statement_names(declaration, names);
        }
        _ => {}
    }
}

fn rename_imported_symbol(stmt: &mut Statement, from: &str, to: &str, default_only: bool) {
    if let Statement::ExportDecl {
        declaration,
        _is_default,
        ..
    } = stmt
    {
        if default_only && !*_is_default {
            return;
        }

        match declaration.as_mut() {
            Statement::FunctionDeclaration(func) if default_only || func.name == from => {
                func.name = to.to_string();
            }
            Statement::ClassDeclaration(class) if default_only || class.name == from => {
                class.name = to.to_string();
            }
            Statement::VarDeclaration {
                pattern: crate::frontend::ast::BindingNode::Identifier(name),
                ..
            } if default_only || name == from => {
                *name = to.to_string();
            }
            _ => {}
        }
    }
}

fn import_items_exported(stmts: &[Statement]) -> (HashSet<String>, bool) {
    let mut exported_names = HashSet::new();
    let mut has_default_export = false;

    for stmt in stmts {
        if let Statement::ExportDecl {
            declaration,
            _is_default,
            ..
        } = stmt
        {
            if *_is_default {
                has_default_export = true;
            }
            collect_statement_names(declaration, &mut exported_names);
        }
    }

    (exported_names, has_default_export)
}

fn apply_import_aliases(
    stmts: &mut [Statement],
    import_items: &[ImportItem],
    is_default: bool,
) {
    if is_default {
        if let Some(item) = import_items.first() {
            let target_name = item.alias.as_deref().unwrap_or(&item.name);
            for stmt in stmts {
                rename_imported_symbol(stmt, "", target_name, true);
            }
        }
        return;
    }

    for item in import_items {
        if let Some(alias) = &item.alias {
            for stmt in stmts.iter_mut() {
                rename_imported_symbol(stmt, &item.name, alias, false);
            }
        }
    }
}

fn resolve_program_virtual(
    mut statements: Vec<Statement>,
    current_file: &str,
    modules: &HashMap<String, String>,
    processed: &mut HashSet<String>,
    import_stack: &mut Vec<String>,
) -> Result<Vec<Statement>, Diagnostic> {
    if !is_core_module(current_file) {
        for core_module in ["core/string.tx", "core/array.tx", "core/prelude.tx"] {
            let already_imports = statements.iter().any(|stmt| {
                matches!(
                    stmt,
                    Statement::ImportDecl { source, .. } if normalize_virtual_import(source, current_file) == core_module
                )
            });

            if !already_imports {
                statements.insert(
                    0,
                    Statement::ImportDecl {
                        source: core_module.to_string(),
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
        let import_snapshot = match &statements[i] {
            Statement::ImportDecl {
                source,
                _names,
                _is_default,
                _line,
                _col,
            } => Some((source.clone(), _names.clone(), *_is_default, *_line, *_col)),
            _ => None,
        };

        let Some((source, import_items, is_default, import_line, import_col)) = import_snapshot else {
            i += 1;
            continue;
        };

        let module_id = normalize_virtual_import(&source, current_file);
        if processed.contains(&module_id) {
            statements.remove(i);
            continue;
        }

        let source_text = modules.get(&module_id).ok_or_else(|| {
            Diagnostic::new(
                format!("Module not found: '{}'", source),
                import_line.max(1),
                import_col.max(1),
                current_file.to_string(),
            )
            .with_code("E0200")
        })?;

        import_stack.push(module_id.clone());
        processed.insert(module_id.clone());
        let imported_program = parse_program_virtual(source_text, &module_id)?;
        let mut new_stmts = resolve_program_virtual(
            imported_program.statements,
            &module_id,
            modules,
            processed,
            import_stack,
        )?;
        import_stack.pop();

        let (exported_names, has_default_export) = import_items_exported(&new_stmts);
        if is_default && !has_default_export {
            return Err(
                Diagnostic::new(
                    format!("Module '{}' has no default export", source),
                    import_line.max(1),
                    import_col.max(1),
                    current_file.to_string(),
                )
                .with_code("E0203"),
            );
        }

        if !is_default && !import_items.is_empty() {
            for item in &import_items {
                if !exported_names.contains(&item.name) {
                    return Err(
                        Diagnostic::new(
                            format!("'{}' is not exported from '{}'", item.name, source),
                            import_line.max(1),
                            import_col.max(1),
                            current_file.to_string(),
                        )
                        .with_code("E0202"),
                    );
                }
            }
        }

        apply_import_aliases(&mut new_stmts, &import_items, is_default);
        statements.splice(i..=i, new_stmts);
    }

    Ok(statements)
}

fn resolve_imports_virtual(
    statements: Vec<Statement>,
    filename: &str,
    extra_modules: &HashMap<String, String>,
) -> Result<Vec<Statement>, Diagnostic> {
    let mut modules = builtin_modules();
    for (name, source) in extra_modules {
        modules.insert(normalize_virtual_import(name, filename), source.clone());
    }

    let filename = normalize_filename(filename);
    let root_module_id = normalize_virtual_import(&filename, DEFAULT_FILENAME);
    modules.insert(root_module_id.clone(), String::new());

    let mut processed = HashSet::new();
    let mut import_stack = vec![root_module_id];
    resolve_program_virtual(statements, &filename, &modules, &mut processed, &mut import_stack)
}

fn mir_constant_to_json(value: &str) -> Value {
    if let Ok(i) = value.parse::<i64>() {
        Value::Number(i.into())
    } else if let Ok(b) = value.parse::<bool>() {
        Value::Bool(b)
    } else if value == "null" || value == "None" {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

fn mir_value_to_json(value: &MIRValue) -> Value {
    match value {
        MIRValue::Variable { name, ty } => json!({
            "_type": "Variable",
            "name": name,
            "ty": format!("{:?}", ty),
        }),
        MIRValue::Constant { value, ty } => json!({
            "_type": "Constant",
            "value": mir_constant_to_json(value),
            "ty": format!("{:?}", ty),
        }),
    }
}

fn mir_instruction_to_json(inst: &MIRInstruction) -> Value {
    match inst {
        MIRInstruction::Move { dst, src, line } => json!({
            "_type": "Move",
            "dst": dst,
            "src": mir_value_to_json(src),
            "line": line,
        }),
        MIRInstruction::BinaryOp {
            dst,
            left,
            op,
            right,
            op_width,
            line,
        } => json!({
            "_type": "BinaryOp",
            "dst": dst,
            "left": mir_value_to_json(left),
            "op": format!("{:?}", op),
            "right": mir_value_to_json(right),
            "op_width": format!("{:?}", op_width),
            "line": line,
        }),
        MIRInstruction::Branch {
            condition,
            true_target,
            false_target,
            line,
        } => json!({
            "_type": "Branch",
            "condition": mir_value_to_json(condition),
            "true_target": true_target,
            "false_target": false_target,
            "line": line,
        }),
        MIRInstruction::Jump { target, line } => json!({
            "_type": "Jump",
            "target": target,
            "line": line,
        }),
        MIRInstruction::Return { value, line } => json!({
            "_type": "Return",
            "value": value.as_ref().map(mir_value_to_json),
            "line": line,
        }),
        MIRInstruction::Call {
            dst,
            callee,
            args,
            line,
        } => json!({
            "_type": "Call",
            "dst": dst,
            "callee": callee,
            "args": args.iter().map(mir_value_to_json).collect::<Vec<_>>(),
            "line": line,
        }),
        MIRInstruction::IndirectCall {
            dst,
            callee,
            args,
            line,
        } => json!({
            "_type": "IndirectCall",
            "dst": dst,
            "callee": mir_value_to_json(callee),
            "args": args.iter().map(mir_value_to_json).collect::<Vec<_>>(),
            "line": line,
        }),
        MIRInstruction::LoadMember {
            dst,
            obj,
            member,
            line,
        } => json!({
            "_type": "LoadMember",
            "dst": dst,
            "obj": mir_value_to_json(obj),
            "member": member,
            "line": line,
        }),
        MIRInstruction::StoreMember {
            obj,
            member,
            src,
            line,
        } => json!({
            "_type": "StoreMember",
            "obj": mir_value_to_json(obj),
            "member": member,
            "src": mir_value_to_json(src),
            "line": line,
        }),
        MIRInstruction::LoadIndex {
            dst,
            obj,
            index,
            element_ty,
            line,
        } => json!({
            "_type": "LoadIndex",
            "dst": dst,
            "obj": mir_value_to_json(obj),
            "index": mir_value_to_json(index),
            "element_ty": format!("{:?}", element_ty),
            "line": line,
        }),
        MIRInstruction::StoreIndex {
            obj,
            index,
            src,
            element_ty,
            line,
        } => json!({
            "_type": "StoreIndex",
            "obj": mir_value_to_json(obj),
            "index": mir_value_to_json(index),
            "src": mir_value_to_json(src),
            "element_ty": format!("{:?}", element_ty),
            "line": line,
        }),
        MIRInstruction::Throw { value, line } => json!({
            "_type": "Throw",
            "value": mir_value_to_json(value),
            "line": line,
        }),
        MIRInstruction::Cast { dst, src, ty, line } => json!({
            "_type": "Cast",
            "dst": dst,
            "src": mir_value_to_json(src),
            "ty": format!("{:?}", ty),
            "line": line,
        }),
        MIRInstruction::TrySetup {
            try_target,
            _catch_target,
            line,
        } => json!({
            "_type": "TrySetup",
            "try_target": try_target,
            "catch_target": _catch_target,
            "line": line,
        }),
        MIRInstruction::PopHandler { line } => json!({
            "_type": "PopHandler",
            "line": line,
        }),
    }
}

fn mir_block_to_json(block: &BasicBlock) -> Value {
    json!({
        "name": block.name,
        "instructions": block
            .instructions
            .iter()
            .map(mir_instruction_to_json)
            .collect::<Vec<_>>(),
        "exception_handler": block.exception_handler,
    })
}

fn mir_function_to_json(func: &MIRFunction) -> Value {
    let variables = func
        .variables
        .iter()
        .map(|(name, ty)| (name.clone(), Value::String(format!("{:?}", ty))))
        .collect::<Map<String, Value>>();

    json!({
        "name": func.name,
        "params": func.params,
        "variables": variables,
        "return_type": format!("{:?}", func.return_type),
        "blocks": func.blocks.iter().map(mir_block_to_json).collect::<Vec<_>>(),
        "entry_block": func.entry_block,
        "is_extern": func.is_extern,
    })
}

fn diagnostic_to_json(stage: &str, diag: &Diagnostic, source: &str) -> Value {
    json!({
        "ok": false,
        "stage": stage,
        "message": diag.message,
        "line": diag.line,
        "col": diag.col,
        "code": diag.code,
        "file": diag.file,
        "full_error": render_diagnostic(diag, source),
    })
}

fn string_to_cstring_ptr(value: String) -> *mut c_char {
    CString::new(value)
        .unwrap_or_else(|_| CString::new("ERROR:internal null byte").unwrap())
        .into_raw()
}

fn read_config(config_ptr: *const c_char) -> Value {
    let config_str = unsafe { CStr::from_ptr(config_ptr).to_string_lossy().into_owned() };
    serde_json::from_str(&config_str).unwrap_or(Value::Null)
}

fn read_c_string(ptr: *const c_char) -> String {
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

fn compile_report(source: String, filename: String, config: Value) -> Result<Value, Value> {
    let filename = normalize_filename(&filename);
    let extra_modules = extra_modules_from_config(&config);
    let include_mir_debug = config
        .get("debug_mir")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let codegen_config = merged_codegen_config(config);

    log_internal("Lexing...");
    let mut lexer = Lexer::new(&source, &filename);
    let tokens = lexer.tokenize();
    if let Some(diag) = lexer.errors.first() {
        return Err(diagnostic_to_json("lex", diag, &source));
    }

    log_internal("Parsing...");
    let mut parser = Parser::new(tokens, &filename);
    let program = parser.parse_program();
    if let Some(diag) = parser.get_errors().first() {
        return Err(diagnostic_to_json("parse", diag, &source));
    }

    log_internal("Resolving imports...");
    let resolved_statements = match resolve_imports_virtual(program.statements.clone(), &filename, &extra_modules) {
        Ok(stmts) => stmts,
        Err(diag) => return Err(diagnostic_to_json("imports", &diag, &source)),
    };
    let mut resolved_program = program.clone();
    resolved_program.statements = resolved_statements;

    log_internal("Type checking...");
    let mut type_checker = TypeChecker::new();
    if type_checker.check(&resolved_program, &filename).is_err() {
        let diag = type_checker
            .diagnostics
            .first()
            .cloned()
            .unwrap_or_else(|| Diagnostic::new("Type check failed".to_string(), 1, 1, filename.clone()));
        return Err(diagnostic_to_json("typecheck", &diag, &source));
    }

    log_internal("Lowering...");
    let lowering = Lowering::new();
    let base_path = Path::new(&filename).parent().unwrap_or(Path::new("."));
    let lowering_result = lowering.lower(&resolved_program, base_path);

    {
        let diagnostics = lowering.diagnostics.borrow();
        if let Some(diag) = diagnostics.first() {
            return Err(diagnostic_to_json("lowering", diag, &source));
        }
    }

    log_internal("MIR lowering...");
    let mut mir_json_functions = Vec::new();
    let mut mir_debug_functions = Vec::new();
    for hir_func in &lowering_result.functions {
        let mir_func = MIRLowering::new(
            lowering_result.signatures.clone(),
            lowering_result.class_fields.clone(),
        )
        .lower(hir_func);
        if include_mir_debug {
            mir_debug_functions.push(format!("{:?}", mir_func));
        }
        mir_json_functions.push(mir_function_to_json(&mir_func));
    }

    log_internal("Codegen...");
    let mut wasm_codegen = WasmCodeGen::new(codegen_config);
    let mir_json_value = Value::Array(mir_json_functions.clone());
    let wat = wasm_codegen.generate_wat_generic(mir_json_value);
    let mut report = json!({
        "ok": true,
        "wat": wat,
    });
    if include_mir_debug {
        report["mir_debug"] = Value::Array(
            mir_debug_functions
                .into_iter()
                .map(Value::String)
                .collect(),
        );
        report["mir_json"] = Value::Array(mir_json_functions.clone());
    }
    Ok(report)
}

fn compile_internal(source: String, filename: String, config: Value) -> Result<String, String> {
    match compile_report(source, filename, config) {
        Ok(report) => report
            .get("wat")
            .and_then(|v| v.as_str())
            .map(|wat| wat.to_string())
            .ok_or_else(|| "Compiler report missing WAT output".to_string()),
        Err(report) => Err(report.to_string()),
    }
}

/// Compile source to JSON: `{ ok, wat }` or `{ ok: false, ...diagnostic }`.
#[no_mangle]
pub extern "C" fn compile_to_wat_json_raw(
    source_ptr: *const c_char,
    filename_ptr: *const c_char,
    config_ptr: *const c_char,
) -> *mut c_char {
    let source = read_c_string(source_ptr);
    let filename = read_c_string(filename_ptr);
    let config = read_config(config_ptr);

    let payload = match compile_report(source, filename, config) {
        Ok(report) => report.to_string(),
        Err(report) => report.to_string(),
    };
    string_to_cstring_ptr(payload)
}

/// Backward-compatible FFI entry point: returns WAT or `ERROR:<json>`.
#[no_mangle]
pub extern "C" fn compile_to_wat_raw(
    source_ptr: *const c_char,
    filename_ptr: *const c_char,
    config_ptr: *const c_char,
) -> *mut c_char {
    let source = read_c_string(source_ptr);
    let filename = read_c_string(filename_ptr);
    let config = read_config(config_ptr);

    match compile_internal(source, filename, config) {
        Ok(wat) => string_to_cstring_ptr(wat),
        Err(err) => string_to_cstring_ptr(format!("ERROR:{}", err)),
    }
}

#[no_mangle]
pub extern "C" fn compile_to_wasm_raw(
    source_ptr: *const c_char,
    filename_ptr: *const c_char,
    config_ptr: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    let source = read_c_string(source_ptr);
    let filename = read_c_string(filename_ptr);
    let config = read_config(config_ptr);

    match compile_internal(source, filename, config) {
        Ok(wat) => match wat::parse_str(&wat) {
            Ok(bin) => {
                let len = bin.len();
                unsafe {
                    *out_len = len;
                }
                let ptr = tejx_alloc(len);
                unsafe {
                    std::slice::from_raw_parts_mut(ptr, len).copy_from_slice(&bin);
                }
                ptr
            }
            Err(err) => {
                log_internal(&format!("wat::parse_str failed: {}", err));
                unsafe {
                    *out_len = 0;
                }
                std::ptr::null_mut()
            }
        },
        Err(err) => {
            log_internal(&format!("compile_to_wasm_raw failed: {}", err));
            unsafe {
                *out_len = 0;
            }
            std::ptr::null_mut()
        }
    }
}

fn render_diagnostic(diag: &Diagnostic, source: &str) -> String {
    let mut output = String::new();
    output.push_str(&format!("error"));
    if !diag.code.is_empty() {
        output.push_str(&format!("[{}]", diag.code));
    }
    output.push_str(&format!(": {}\n", diag.message));
    output.push_str(&format!(" --> {}:{}:{}\n", diag.file, diag.line, diag.col));

    let lines: Vec<&str> = source.lines().collect();
    if diag.line > 0 && diag.line <= lines.len() {
        let line_content = lines[diag.line - 1];
        output.push_str(&format!("{:>4} | {}\n", diag.line, line_content));
        output.push_str("     | ");
        for _ in 1..diag.col {
            output.push(' ');
        }
        for _ in 0..diag.length.max(1) {
            output.push('^');
        }
        if let Some(label) = &diag.label {
            output.push(' ');
            output.push_str(label);
        }
        output.push('\n');
        if let Some(hint) = &diag.hint {
            output.push_str(&format!("hint: {}\n", hint));
        }
    }

    output
}

pub fn log_internal(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        extern "C" {
            fn tejx_compiler_log(ptr: *const u8, len: usize);
        }

        tejx_compiler_log(msg.as_ptr(), msg.len());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = msg;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_fixture(source: &str) -> Result<String, String> {
        compile_internal(source.to_string(), "fixture.tx".to_string(), Value::Null)
    }

    #[test]
    fn compiles_basic_program_with_default_config() {
        let wat = compile_fixture("function main(): void { print(1); }").unwrap();
        assert!(wat.contains("(module"));
        assert!(wat.contains("(export \"main\""));
    }

    #[test]
    fn resolves_std_collections_in_virtual_mode() {
        let source = r#"
            import { Stack } from "std:collections";

            function main(): void {
                let s: Stack<int> = new Stack<int>();
                s.push(1);
                print(s.pop());
            }
        "#;

        let wat = compile_fixture(source).unwrap();
        assert!(wat.contains("rt_array_push"));
        assert!(wat.contains("rt_array_pop"));
    }

    #[test]
    fn returns_structured_errors() {
        let err = compile_internal("function main( {".to_string(), "bad.tx".to_string(), Value::Null)
            .unwrap_err();
        assert!(err.contains("\"ok\":false"));
        assert!(err.contains("\"stage\":\"parse\""));
    }
}
