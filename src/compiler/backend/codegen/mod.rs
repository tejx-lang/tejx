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
    function_param_types: HashMap<String, Vec<TejxType>>,
    declared_globals: HashSet<String>,
    global_types: HashMap<String, TejxType>,
    string_constant_cache: HashMap<String, (String, usize)>,
    boxed_string_cache: HashMap<String, String>,
    current_function_params: HashSet<String>,
    pub local_vars: HashSet<String>,

    captured_vars: Vec<String>,
    captured_vars_by_function: HashMap<String, Vec<String>>,
    current_env: Option<String>,
    alloca_buffer: String,
    entry_init_buffer: String,
    stack_arrays: HashSet<String>,
    heap_array_ptrs: HashMap<String, (String, i64)>, // var_name -> (data_ptr_alloca, elem_size)
    pub unsafe_arrays: bool,
    float_ssa_vars: HashMap<String, String>, // var_name -> LLVM double SSA variable
    num_roots: usize,
    volatile_locals: bool,
    pub class_fields: HashMap<String, Vec<(String, TejxType)>>,
    pub class_methods: HashMap<String, Vec<String>>,
    pub class_parents: HashMap<String, String>,
    pub type_id_map: HashMap<String, u32>,
    closure_adapters: HashMap<String, String>,
    pub object_shape_names: HashMap<String, String>,
    pub function_display_names: HashMap<String, String>,
    current_arena: Option<String>,
    pub source_file: String,
    current_debug_line: Option<usize>,
    tracked_runtime_functions: HashSet<String>,
    known_mir_functions: HashSet<String>,
    extern_mir_functions: HashSet<String>,
    current_function_has_runtime_frame: bool,
    current_function_tracks_location: bool,
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
            | TejxType::Optional(_)
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

    pub(crate) fn needs_gc_root(name: &str, ty: &TejxType) -> bool {
        Self::is_gc_managed(ty)
            || name.starts_with("promise_id_local")
            || name.starts_with("__p_")
    }

    pub(crate) fn canonical_global_name(name: &str) -> String {
        if name.starts_with("g_") {
            return name.to_string();
        }
        format!("g_{}", name)
    }

    pub(crate) fn static_root_slot_name(name: &str) -> String {
        format!("{}__slot", Self::canonical_global_name(name))
    }

    pub(crate) fn global_type(&self, name: &str) -> Option<&TejxType> {
        let global_name = Self::canonical_global_name(name);
        self.global_types.get(&global_name)
    }

    pub(crate) fn is_gc_global(&self, name: &str) -> bool {
        self.global_type(name)
            .map(Self::is_gc_managed)
            .unwrap_or(false)
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
            function_param_types: HashMap::new(),
            declared_globals: HashSet::new(),
            global_types: HashMap::new(),
            string_constant_cache: HashMap::new(),
            boxed_string_cache: HashMap::new(),
            current_function_params: HashSet::new(),
            local_vars: HashSet::new(),

            captured_vars: Vec::new(),
            captured_vars_by_function: HashMap::new(),
            current_env: None,
            alloca_buffer: String::new(),
            entry_init_buffer: String::new(),
            stack_arrays: HashSet::new(),
            heap_array_ptrs: HashMap::new(),
            unsafe_arrays: false,
            float_ssa_vars: HashMap::new(),
            num_roots: 0,
            volatile_locals: false,
            class_fields: HashMap::new(),
            class_methods: HashMap::new(),
            class_parents: HashMap::new(),
            type_id_map: HashMap::new(),
            closure_adapters: HashMap::new(),
            object_shape_names: HashMap::new(),
            function_display_names: HashMap::new(),
            current_arena: None,
            source_file: String::new(),
            current_debug_line: None,
            tracked_runtime_functions: HashSet::new(),
            known_mir_functions: HashSet::new(),
            extern_mir_functions: HashSet::new(),
            current_function_has_runtime_frame: false,
            current_function_tracks_location: false,
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

    pub(crate) fn class_matches_instanceof(&self, class_name: &str, target_class: &str) -> bool {
        let mut current = Some(class_name.to_string());
        while let Some(name) = current {
            if name == target_class {
                return true;
            }
            current = self.class_parents.get(&name).cloned();
        }
        false
    }

    pub(crate) fn instanceof_type_ids(&self, target_class: &str) -> Vec<u32> {
        let mut type_ids = Vec::new();
        for (class_name, type_id) in &self.type_id_map {
            if self.class_matches_instanceof(class_name, target_class) {
                type_ids.push(*type_id);
            }
        }
        type_ids.sort_unstable();
        type_ids.dedup();
        type_ids
    }
}
