pub mod func;
pub mod inst;
pub mod memory;
pub mod utils;

/// MIR → LLVM IR Code Generator, mirroring C++ MIRCodeGen.cpp
/// Generates textual LLVM IR from MIR basic blocks.
use crate::common::types::TejxType;
use std::collections::{HashMap, HashSet};

pub struct CodeGen {
    buffer: String,
    global_buffer: String,
    value_map: HashMap<String, String>,
    ptr_types: HashMap<String, String>, // MIR var name → LLVM alloca ptr name
    temp_counter: usize,
    label_counter: usize,
    declared_functions: HashSet<String>,
    defined_functions: HashSet<String>,
    function_param_counts: HashMap<String, usize>,
    declared_globals: HashSet<String>,
    current_function_params: HashSet<String>,
    pub local_vars: HashSet<String>,

    captured_vars: Vec<String>,
    captured_vars_by_function: HashMap<String, Vec<String>>,
    current_env: Option<String>,
    alloca_buffer: String,
    stack_arrays: HashSet<String>,
    heap_array_ptrs: HashMap<String, (String, i64)>, // var_name -> (data_ptr_alloca, elem_size)
    pub unsafe_arrays: bool,
    float_ssa_vars: HashMap<String, String>, // var_name -> LLVM double SSA variable
    num_roots: usize,
    volatile_locals: bool,
    pub class_fields: HashMap<String, Vec<(String, TejxType)>>,
    pub class_methods: HashMap<String, Vec<String>>,
    pub type_id_map: HashMap<String, u32>,
    current_arena: Option<String>,
}

impl Default for CodeGen {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGen {
    pub(crate) fn get_aligned_offset(current: usize, ty: &TejxType) -> usize {
        let size = ty.size();
        let align = if size <= 1 {
            1
        } else if size <= 2 {
            2
        } else if size <= 4 {
            4
        } else {
            8
        };
        (current + align - 1) & !(align - 1)
    }

    pub(crate) fn get_llvm_type(ty: &TejxType) -> &str {
        match ty {
            TejxType::Bool => "i8",
            TejxType::Int16 => "i16",
            TejxType::Int32 | TejxType::Char => "i32",
            TejxType::Int64 => "i64",
            TejxType::Float32 => "float",
            TejxType::Float64 => "double",
            TejxType::Void => "void",
            _ => "i64", // Pointers to GC objects, arrays, closures, strings
        }
    }

    pub(crate) fn get_llvm_storage_type(ty: &TejxType) -> &str {
        match ty {
            TejxType::Bool => "i8", // 1-byte storage
            TejxType::Int16 => "i16",
            TejxType::Int32 | TejxType::Char => "i32",
            TejxType::Int64 => "i64",
            TejxType::Float32 => "float",
            TejxType::Float64 => "double",
            TejxType::Void => "void",
            _ => "i64", // Pointers to GC objects, arrays, closures, strings
        }
    }

    pub(crate) fn is_gc_managed(ty: &TejxType) -> bool {
        match ty {
            TejxType::Class(_, _)
            | TejxType::FixedArray(_, _)
            | TejxType::DynamicArray(_)
            | TejxType::Function(_, _)
            | TejxType::String
            | TejxType::Any
            | TejxType::Slice(_)
            | TejxType::Object(_) => true,
            _ => false,
        }
    }

    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            global_buffer: String::new(),
            value_map: HashMap::new(),
            ptr_types: HashMap::new(),
            temp_counter: 0,
            label_counter: 0,
            declared_functions: HashSet::new(),
            defined_functions: HashSet::new(),
            function_param_counts: HashMap::new(),
            declared_globals: HashSet::new(),
            current_function_params: HashSet::new(),
            local_vars: HashSet::new(),

            captured_vars: Vec::new(),
            captured_vars_by_function: HashMap::new(),
            current_env: None,
            alloca_buffer: String::new(),
            stack_arrays: HashSet::new(),
            heap_array_ptrs: HashMap::new(),
            unsafe_arrays: false,
            float_ssa_vars: HashMap::new(),
            num_roots: 0,
            volatile_locals: false,
            class_fields: HashMap::new(),
            class_methods: HashMap::new(),
            type_id_map: HashMap::new(),
            current_arena: None,
        }
    }

    pub(crate) fn emit(&mut self, code: &str) {
        self.buffer.push_str(code);
    }

    pub(crate) fn get_target_info() -> (&'static str, String) {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else {
            "x86_64" // fallback
        };

        if cfg!(target_os = "macos") {
            (
                "e-m:o-i64:64-i128:128-n32:64-S128-Fn32",
                format!("{}-apple-macosx14.0.0", arch),
            )
        } else if cfg!(target_os = "linux") {
            (
                "e-m:e-i64:64-i128:128-n32:64-S128",
                format!("{}-unknown-linux-gnu", arch),
            )
        } else {
            (
                "e-m:e-i64:64-i128:128-n32:64-S128",
                format!("{}-unknown-unknown", arch),
            )
        }
    }

    pub(crate) fn declare_runtime_fn(&mut self, name: &str, signature: &str) {
        if !self.declared_functions.contains(name) {
            self.global_buffer
                .push_str(&format!("declare {}\n", signature));
            self.declared_functions.insert(name.to_string());
        }
    }

    pub(crate) fn emit_line(&mut self, code: &str) {
        self.buffer.push_str("  ");
        self.buffer.push_str(code);
        self.buffer.push('\n');
    }
}
