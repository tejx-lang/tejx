pub mod binary;
pub use binary::*;
pub mod crypto;
pub use crypto::*;
pub mod array;
pub use array::*;
pub mod atomic;
pub use atomic::*;
pub mod condition;
pub use condition::*;
pub mod mutex;
pub use mutex::*;
pub mod net;
pub use net::*;
pub mod object;
pub use object::*;
pub mod promise;
pub use promise::*;
pub mod queue;
pub use queue::*;
pub mod string;
pub use string::*;
pub mod thread;
pub use thread::*;

#[path = "../gc.rs"]
pub mod gc;
pub use gc::{
    gc_allocate, rt_add_static_root, rt_get_header, rt_get_static_root, rt_init_gc,
    rt_is_gc_body_ptr_exact, rt_is_gc_ptr, rt_pin_static_root, rt_pop_roots, rt_push_root,
    rt_register_thread, rt_register_type, rt_release_static_root, rt_set_static_root,
    rt_unregister_thread, rt_write_barrier, ObjectHeader, MAX_TYPES,
};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::panic::{self, PanicHookInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, Once};

const STRING_FLAG_FROZEN: u16 = 0x0800;
#[derive(Default)]
struct ConstStringRoots {
    slots_by_bytes: HashMap<Vec<u8>, usize>,
}

static CONST_STRING_ROOTS: LazyLock<Mutex<ConstStringRoots>> =
    LazyLock::new(|| Mutex::new(ConstStringRoots::default()));
static RNG_STATE: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));
static RAW_ATOMIC_OBJECTS: LazyLock<Mutex<HashMap<usize, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
#[cfg(test)]
pub(crate) static RUNTIME_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

const FORMAT_MAX_DEPTH: usize = 6;
const TYPE_INFO_MAX_FIELDS: usize = 64;
const FIELD_KIND_REF: u8 = 0;
const FIELD_KIND_BOOL: u8 = 1;
const FIELD_KIND_INT8: u8 = 2;
const FIELD_KIND_UINT8: u8 = 3;
const FIELD_KIND_INT16: u8 = 4;
const FIELD_KIND_UINT16: u8 = 5;
const FIELD_KIND_INT32: u8 = 6;
const FIELD_KIND_INT64: u8 = 7;
const FIELD_KIND_FLOAT32: u8 = 8;
const FIELD_KIND_FLOAT64: u8 = 9;
const FIELD_KIND_CHAR: u8 = 10;
const FIELD_KIND_UINT32: u8 = 11;
const FIELD_KIND_UINT64: u8 = 12;
const FIELD_KIND_INT128: u8 = 13;
const FIELD_KIND_UINT128: u8 = 14;
const FIELD_KIND_UNSUPPORTED: u8 = 255;

#[derive(Clone, Debug, Default)]
pub(crate) struct RuntimeFrame {
    function_ptr: i64,
    file_ptr: i64,
    line: usize,
    track_location: bool,
}

#[derive(Clone, Debug)]
struct StoredExceptionTrace {
    handle_id: usize,
    stack: Vec<RuntimeFrame>,
}

thread_local! {
    static RUNTIME_CALL_STACK: RefCell<Vec<RuntimeFrame>> = const { RefCell::new(Vec::new()) };
}

static RUNTIME_PANIC_HOOK: Once = Once::new();
const STORED_EXCEPTION_TRACE_LIMIT: usize = 256;
static STORED_EXCEPTION_TRACES: LazyLock<Mutex<VecDeque<StoredExceptionTrace>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

static mut TYPE_NAME_PTRS: [*const std::ffi::c_char; MAX_TYPES] = [std::ptr::null(); MAX_TYPES];
static mut TYPE_FIELD_COUNTS: [usize; MAX_TYPES] = [0; MAX_TYPES];
static mut TYPE_FIELD_OFFSETS: [[usize; TYPE_INFO_MAX_FIELDS]; MAX_TYPES] =
    [[0; TYPE_INFO_MAX_FIELDS]; MAX_TYPES];
static mut TYPE_FIELD_KINDS: [[u8; TYPE_INFO_MAX_FIELDS]; MAX_TYPES] =
    [[FIELD_KIND_UNSUPPORTED; TYPE_INFO_MAX_FIELDS]; MAX_TYPES];
static mut TYPE_FIELD_NAMES: [[*const std::ffi::c_char; TYPE_INFO_MAX_FIELDS]; MAX_TYPES] =
    [[std::ptr::null(); TYPE_INFO_MAX_FIELDS]; MAX_TYPES];

fn runtime_seed_now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let addr_mix = (&nanos as *const u64 as usize) as u64;
    let seed = nanos ^ addr_mix.rotate_left(13);
    if seed == 0 {
        0x9E37_79B9_7F4A_7C15
    } else {
        seed
    }
}

fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub unsafe fn rt_throw_runtime_error(msg: &str) -> ! {
    let msg_id = new_string_from_rust_str(msg);
    crate::event_loop::tejx_throw(msg_id);
    std::hint::unreachable_unchecked();
}

fn runtime_symbol_name(symbol: &str) -> String {
    if symbol == "tejx_main" {
        return "<entry>".to_string();
    }
    if let Some(name) = symbol.strip_prefix("f_") {
        return name.to_string();
    }
    if symbol.starts_with("lambda_") {
        return "<lambda>".to_string();
    }
    if symbol == "rt_main_async_worker" {
        return "<async worker>".to_string();
    }
    symbol.to_string()
}

unsafe fn runtime_symbol_name_from_ptr(ptr_value: i64) -> String {
    runtime_debug_string(ptr_value)
        .map(|symbol| runtime_symbol_name(&symbol))
        .unwrap_or_else(|| "<unknown>".to_string())
}

unsafe fn runtime_frame_file(frame: &RuntimeFrame) -> String {
    if frame.file_ptr == 0 {
        String::new()
    } else {
        runtime_debug_string(frame.file_ptr).unwrap_or_default()
    }
}

unsafe fn runtime_debug_string(ptr_value: i64) -> Option<String> {
    if ptr_value == 0 {
        return None;
    }
    let ptr = ptr_value as usize as *const std::ffi::c_char;
    if ptr.is_null() {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned(),
    )
}

pub(crate) fn runtime_call_stack_depth() -> usize {
    RUNTIME_CALL_STACK.with(|stack| stack.borrow().len())
}

pub(crate) fn runtime_restore_call_stack(depth: usize) {
    RUNTIME_CALL_STACK.with(|stack| stack.borrow_mut().truncate(depth));
}

pub(crate) fn runtime_call_stack_snapshot() -> Vec<RuntimeFrame> {
    RUNTIME_CALL_STACK.with(|stack| stack.borrow().clone())
}

unsafe fn runtime_exception_trace_position(
    traces: &VecDeque<StoredExceptionTrace>,
    exception: i64,
) -> Option<usize> {
    traces.iter().position(|entry| {
        crate::event_loop::tejx_get_global_handle(entry.handle_id) == exception
    })
}

#[cfg(test)]
pub(crate) unsafe fn clear_stored_exception_traces() {
    let mut traces = match STORED_EXCEPTION_TRACES.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    while let Some(entry) = traces.pop_front() {
        crate::event_loop::tejx_drop_global_handle(entry.handle_id);
    }
}

unsafe fn remember_exception_trace_with_stack(exception: i64, stack: &[RuntimeFrame]) {
    if stack.is_empty() {
        return;
    }

    let mut traces = match STORED_EXCEPTION_TRACES.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    if let Some(pos) = runtime_exception_trace_position(&traces, exception) {
        if let Some(entry) = traces.get_mut(pos) {
            if entry.stack.is_empty() && !stack.is_empty() {
                entry.stack = stack.to_vec();
            }
        }
        return;
    }

    let handle_id = crate::event_loop::tejx_create_global_handle(exception);
    traces.push_back(StoredExceptionTrace {
        handle_id,
        stack: stack.to_vec(),
    });

    while traces.len() > STORED_EXCEPTION_TRACE_LIMIT {
        if let Some(entry) = traces.pop_front() {
            crate::event_loop::tejx_drop_global_handle(entry.handle_id);
        }
    }
}

pub(crate) unsafe fn remember_exception_trace(exception: i64) {
    let stack = runtime_call_stack_snapshot();
    remember_exception_trace_with_stack(exception, &stack);
}

unsafe fn runtime_exception_trace_snapshot(exception: i64) -> Option<Vec<RuntimeFrame>> {
    let traces = match STORED_EXCEPTION_TRACES.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let pos = runtime_exception_trace_position(&traces, exception)?;
    traces.get(pos).map(|entry| entry.stack.clone())
}

pub(crate) unsafe fn exception_trace_exists(exception: i64) -> bool {
    runtime_exception_trace_snapshot(exception).is_some()
}

pub(crate) unsafe fn runtime_prepare_thrown_exception_value(exception: i64) -> i64 {
    let mut exception = exception;
    if let Some((_body, tag)) = rt_value_body_and_tag(exception) {
        if tag == TAG_STRING {
            exception = new_string_from_parts(exception, 0, rt_len(exception));
        }
    }

    let stack = runtime_call_stack_snapshot();

    if runtime_error_like_type_id(exception).is_some() {
        runtime_attach_stack_to_error_value(exception, &stack);
        return exception;
    }

    if let Some(wrapped) = runtime_build_error_wrapper(
        runtime_exception_wrapper_type_name(exception),
        &runtime_exception_message_text(exception),
        &stack,
    ) {
        return wrapped;
    }

    exception
}

unsafe fn runtime_error_like_summary(val: i64) -> Option<String> {
    let type_id = runtime_error_like_type_id(val)?;
    let type_name = rt_type_name_string(type_id);
    let message = runtime_class_field_string(val, "message").unwrap_or_default();
    let mut rendered = if message.is_empty() {
        type_name
    } else {
        format!("{}: {}", type_name, message)
    };
    if let Some(code) = runtime_class_field_i32(val, "code") {
        if code != 0 {
            rendered.push_str(&format!(" (code: {})", code));
        }
    }
    Some(rendered)
}

fn runtime_format_location(frame: &RuntimeFrame) -> String {
    let function = unsafe { runtime_symbol_name_from_ptr(frame.function_ptr) };
    let file = unsafe { runtime_frame_file(frame) };
    if !file.is_empty() && frame.line > 0 {
        format!("{}:{} in {}", file, frame.line, function)
    } else if frame.line > 0 {
        format!("line {} in {}", frame.line, function)
    } else {
        function
    }
}

fn runtime_format_stack_frame(frame: &RuntimeFrame) -> String {
    let function = unsafe { runtime_symbol_name_from_ptr(frame.function_ptr) };
    let file = unsafe { runtime_frame_file(frame) };
    if !file.is_empty() && frame.line > 0 {
        format!("  at {} ({}:{})", function, file, frame.line)
    } else if frame.line > 0 {
        format!("  at {} (line {})", function, frame.line)
    } else {
        format!("  at {}", function)
    }
}

fn runtime_read_source_line(file: &str, line: usize) -> Option<String> {
    if file.is_empty() || line == 0 {
        return None;
    }
    let contents = std::fs::read_to_string(file).ok()?;
    let source_line = contents.lines().nth(line.saturating_sub(1))?;
    Some(source_line.trim_end().to_string())
}

fn runtime_render_source_frame(file: &str, line: usize, label: &str) -> Option<String> {
    if file.is_empty() || line == 0 {
        return None;
    }

    let contents = std::fs::read_to_string(file).ok()?;
    let lines: Vec<&str> = contents.lines().collect();
    if line > lines.len() {
        return None;
    }

    let current = lines[line - 1].trim_end();
    let prev = if line > 1 {
        Some(lines[line - 2].trim_end())
    } else {
        None
    };
    let next = if line < lines.len() {
        Some(lines[line].trim_end())
    } else {
        None
    };

    let max_line = if next.is_some() { line + 1 } else { line };
    let width = max_line.to_string().len().max(1);
    let caret_col = current.chars().take_while(|ch| ch.is_whitespace()).count();

    let mut frame = String::new();
    frame.push_str("  --> ");
    frame.push_str(file);
    frame.push(':');
    frame.push_str(&line.to_string());
    frame.push('\n');
    frame.push_str("  ");
    frame.push_str(&" ".repeat(width));
    frame.push_str(" |\n");

    if let Some(prev_line) = prev {
        frame.push_str("  ");
        frame.push_str(&format!("{:>width$}", line - 1, width = width));
        frame.push_str(" | ");
        frame.push_str(prev_line);
        frame.push('\n');
    }

    frame.push_str("  ");
    frame.push_str(&format!("{:>width$}", line, width = width));
    frame.push_str(" | ");
    frame.push_str(current);
    frame.push('\n');
    frame.push_str("  ");
    frame.push_str(&" ".repeat(width));
    frame.push_str(" | ");
    frame.push_str(&" ".repeat(caret_col));
    frame.push_str("^ ");
    frame.push_str(label);
    frame.push('\n');

    if let Some(next_line) = next {
        frame.push_str("  ");
        frame.push_str(&format!("{:>width$}", line + 1, width = width));
        frame.push_str(" | ");
        frame.push_str(next_line);
        frame.push('\n');
    }

    Some(frame)
}

fn runtime_error_hint(message: &str) -> Option<&'static str> {
    if message.contains("Division by zero") {
        Some("Ensure the right-hand side of '/' or '%' is not zero before evaluating the expression.")
    } else if message.contains("Null pointer dereference") {
        Some("Check for None/nullish values before reading from or writing to this value.")
    } else if message.contains("out of bounds") {
        Some("Validate the index against the collection length before accessing or assigning.")
    } else if message.contains("constant array") {
        Some("Create a mutable copy before modifying a constant array.")
    } else if message.contains("fixed-size array") {
        Some("Resize or structural mutations are not allowed on fixed-size arrays.")
    } else if message.contains("Deadlock detected") {
        Some("A pending await has no remaining producer. Resolve, reject, or schedule the Promise before awaiting it.")
    } else if message.contains("Invalid byte array") {
        Some("Pass a valid byte array or binary buffer to this API.")
    } else {
        None
    }
}

unsafe fn runtime_value_to_string(val: i64) -> String {
    let mut rooted_val = val;
    rt_push_root(&mut rooted_val);

    let mut string_id = rt_to_string(rooted_val);
    rt_push_root(&mut string_id);

    let rendered = if let Some((data, len)) = get_str_parts(string_id) {
        String::from_utf8_lossy(std::slice::from_raw_parts(data, len as usize)).into_owned()
    } else {
        "<unprintable>".to_string()
    };

    rt_pop_roots(2);
    rendered
}

unsafe fn runtime_inline_value_to_string(val: i64) -> String {
    if val < STACK_OFFSET {
        if looks_like_unboxed_float_bits(val) {
            return rt_float_to_rust_string(f64::from_bits(val as u64));
        }
        return val.to_string();
    }
    runtime_value_to_string(val)
}

unsafe fn runtime_type_to_string(val: i64) -> String {
    let mut rooted_val = val;
    rt_push_root(&mut rooted_val);

    let mut type_id = rt_typeof(rooted_val);
    rt_push_root(&mut type_id);

    let rendered = if let Some((data, len)) = get_str_parts(type_id) {
        String::from_utf8_lossy(std::slice::from_raw_parts(data, len as usize)).into_owned()
    } else {
        "unknown".to_string()
    };

    rt_pop_roots(2);
    rendered
}

unsafe fn runtime_exception_type_name(val: i64) -> String {
    if val < STACK_OFFSET {
        if val == 0 {
            return "none".to_string();
        }
        if looks_like_unboxed_float_bits(val) {
            return "float".to_string();
        }
        return "int".to_string();
    }

    let Some((_body, tag)) = rt_value_body_and_tag(val) else {
        return runtime_type_to_string(val);
    };

    match tag {
        tag if tag == TAG_STRING => "string".to_string(),
        tag if tag == TAG_FUNCTION => "function".to_string(),
        tag if tag == TAG_ARRAY => "array".to_string(),
        tag if tag == TAG_OBJECT => "object".to_string(),
        tag if tag == TAG_BOOLEAN => "bool".to_string(),
        tag if tag == TAG_FLOAT => "float".to_string(),
        tag if tag == TAG_INT => "int".to_string(),
        tag if tag == TAG_CHAR => "char".to_string(),
        tag if tag == TAG_PROMISE => "promise".to_string(),
        other => rt_type_name_string(other as usize),
    }
}

unsafe fn runtime_class_body_and_type_id(val: i64) -> Option<(*mut u8, usize)> {
    let (body_ptr, tag) = rt_value_body_and_tag(val)?;
    if tag < 0 {
        return None;
    }

    let type_id = tag as usize;
    if type_id >= MAX_TYPES || TYPE_NAME_PTRS[type_id].is_null() {
        return None;
    }

    Some((body_ptr, type_id))
}

unsafe fn runtime_type_field_index(type_id: usize, field_name: &str) -> Option<usize> {
    let field_count = TYPE_FIELD_COUNTS[type_id].min(TYPE_INFO_MAX_FIELDS);
    (0..field_count).find(|&i| rt_field_name_string(type_id, i) == field_name)
}

unsafe fn runtime_class_field_string(val: i64, field_name: &str) -> Option<String> {
    let (body_ptr, type_id) = runtime_class_body_and_type_id(val)?;
    let field_index = runtime_type_field_index(type_id, field_name)?;
    let offset = TYPE_FIELD_OFFSETS[type_id][field_index];
    let kind = TYPE_FIELD_KINDS[type_id][field_index];

    let rendered = match kind {
        FIELD_KIND_REF => runtime_value_to_string(*(body_ptr.add(offset) as *const i64)),
        FIELD_KIND_BOOL => {
            if *(body_ptr.add(offset) as *const i8) != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        FIELD_KIND_INT8 => (*(body_ptr.add(offset) as *const i8)).to_string(),
        FIELD_KIND_UINT8 => (*(body_ptr.add(offset) as *const u8)).to_string(),
        FIELD_KIND_INT16 => (*(body_ptr.add(offset) as *const i16)).to_string(),
        FIELD_KIND_UINT16 => (*(body_ptr.add(offset) as *const u16)).to_string(),
        FIELD_KIND_INT32 => (*(body_ptr.add(offset) as *const i32)).to_string(),
        FIELD_KIND_INT64 => (*(body_ptr.add(offset) as *const i64)).to_string(),
        FIELD_KIND_UINT32 => (*(body_ptr.add(offset) as *const u32)).to_string(),
        FIELD_KIND_UINT64 => (*(body_ptr.add(offset) as *const u64)).to_string(),
        FIELD_KIND_INT128 => (*(body_ptr.add(offset) as *const i128)).to_string(),
        FIELD_KIND_UINT128 => (*(body_ptr.add(offset) as *const u128)).to_string(),
        FIELD_KIND_FLOAT32 => rt_float_to_rust_string(*(body_ptr.add(offset) as *const f32) as f64),
        FIELD_KIND_FLOAT64 => rt_float_to_rust_string(*(body_ptr.add(offset) as *const f64)),
        FIELD_KIND_CHAR => {
            let ch = *(body_ptr.add(offset) as *const i32) as u32;
            char::from_u32(ch).unwrap_or('?').to_string()
        }
        _ => return None,
    };

    Some(rendered)
}

unsafe fn runtime_class_field_i32(val: i64, field_name: &str) -> Option<i32> {
    let (body_ptr, type_id) = runtime_class_body_and_type_id(val)?;
    let field_index = runtime_type_field_index(type_id, field_name)?;
    let offset = TYPE_FIELD_OFFSETS[type_id][field_index];
    let kind = TYPE_FIELD_KINDS[type_id][field_index];

    match kind {
        FIELD_KIND_INT8 => Some(*(body_ptr.add(offset) as *const i8) as i32),
        FIELD_KIND_UINT8 => Some(*(body_ptr.add(offset) as *const u8) as i32),
        FIELD_KIND_INT16 => Some(*(body_ptr.add(offset) as *const i16) as i32),
        FIELD_KIND_UINT16 => Some(*(body_ptr.add(offset) as *const u16) as i32),
        FIELD_KIND_INT32 => Some(*(body_ptr.add(offset) as *const i32)),
        FIELD_KIND_INT64 => Some(*(body_ptr.add(offset) as *const i64) as i32),
        FIELD_KIND_UINT32 => Some(*(body_ptr.add(offset) as *const u32) as i32),
        _ => None,
    }
}

unsafe fn runtime_set_class_ref_field(val: i64, field_name: &str, field_val: i64) -> bool {
    let (body_ptr, type_id) = match runtime_class_body_and_type_id(val) {
        Some(info) => info,
        None => return false,
    };
    let field_index = match runtime_type_field_index(type_id, field_name) {
        Some(index) => index,
        None => return false,
    };
    if TYPE_FIELD_KINDS[type_id][field_index] != FIELD_KIND_REF {
        return false;
    }

    let slot = body_ptr.add(TYPE_FIELD_OFFSETS[type_id][field_index]) as *mut i64;
    if val >= HEAP_OFFSET {
        rt_store_ref_slot(val, slot, field_val);
    } else {
        *slot = field_val;
    }
    true
}

unsafe fn runtime_set_class_i32_field(val: i64, field_name: &str, field_val: i32) -> bool {
    let (body_ptr, type_id) = match runtime_class_body_and_type_id(val) {
        Some(info) => info,
        None => return false,
    };
    let field_index = match runtime_type_field_index(type_id, field_name) {
        Some(index) => index,
        None => return false,
    };
    let offset = TYPE_FIELD_OFFSETS[type_id][field_index];
    match TYPE_FIELD_KINDS[type_id][field_index] {
        FIELD_KIND_INT8 => {
            *(body_ptr.add(offset) as *mut i8) = field_val as i8;
            true
        }
        FIELD_KIND_UINT8 => {
            *(body_ptr.add(offset) as *mut u8) = field_val as u8;
            true
        }
        FIELD_KIND_INT16 => {
            *(body_ptr.add(offset) as *mut i16) = field_val as i16;
            true
        }
        FIELD_KIND_UINT16 => {
            *(body_ptr.add(offset) as *mut u16) = field_val as u16;
            true
        }
        FIELD_KIND_INT32 => {
            *(body_ptr.add(offset) as *mut i32) = field_val;
            true
        }
        FIELD_KIND_UINT32 => {
            *(body_ptr.add(offset) as *mut u32) = field_val as u32;
            true
        }
        FIELD_KIND_INT64 => {
            *(body_ptr.add(offset) as *mut i64) = field_val as i64;
            true
        }
        _ => false,
    }
}

unsafe fn runtime_error_like_type_id(val: i64) -> Option<usize> {
    let (_body_ptr, type_id) = runtime_class_body_and_type_id(val)?;
    if runtime_type_field_index(type_id, "message").is_some()
        && runtime_type_field_index(type_id, "stack").is_some()
    {
        return Some(type_id);
    }
    None
}

unsafe fn runtime_exception_message_field(val: i64) -> Option<String> {
    let (body_ptr, tag) = rt_value_body_and_tag(val)?;

    if tag == TAG_OBJECT {
        let key = rt_string_from_c_str_const("message\0".as_ptr() as *const _);
        let idx = rt_object_find_key_index(val, key);
        if idx >= 0 {
            return Some(runtime_value_to_string(rt_array_get_fast(
                rt_object_values_array(val),
                idx,
            )));
        }
        return None;
    }

    if tag < 0 {
        return None;
    }

    let type_id = tag as usize;
    if type_id >= MAX_TYPES {
        return None;
    }

    let field_count = TYPE_FIELD_COUNTS[type_id].min(TYPE_INFO_MAX_FIELDS);
    for i in 0..field_count {
        if rt_field_name_string(type_id, i) != "message" {
            continue;
        }

        let offset = TYPE_FIELD_OFFSETS[type_id][i];
        let kind = TYPE_FIELD_KINDS[type_id][i];
        let message = match kind {
            FIELD_KIND_REF => runtime_value_to_string(*(body_ptr.add(offset) as *const i64)),
            FIELD_KIND_BOOL => {
                if *(body_ptr.add(offset) as *const i8) != 0 {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            FIELD_KIND_INT8 => (*(body_ptr.add(offset) as *const i8)).to_string(),
            FIELD_KIND_UINT8 => (*(body_ptr.add(offset) as *const u8)).to_string(),
            FIELD_KIND_INT16 => (*(body_ptr.add(offset) as *const i16)).to_string(),
            FIELD_KIND_UINT16 => (*(body_ptr.add(offset) as *const u16)).to_string(),
            FIELD_KIND_INT32 => (*(body_ptr.add(offset) as *const i32)).to_string(),
            FIELD_KIND_INT64 => (*(body_ptr.add(offset) as *const i64)).to_string(),
            FIELD_KIND_UINT32 => (*(body_ptr.add(offset) as *const u32)).to_string(),
            FIELD_KIND_UINT64 => (*(body_ptr.add(offset) as *const u64)).to_string(),
            FIELD_KIND_INT128 => (*(body_ptr.add(offset) as *const i128)).to_string(),
            FIELD_KIND_UINT128 => (*(body_ptr.add(offset) as *const u128)).to_string(),
            FIELD_KIND_FLOAT32 => {
                rt_float_to_rust_string(*(body_ptr.add(offset) as *const f32) as f64)
            }
            FIELD_KIND_FLOAT64 => rt_float_to_rust_string(*(body_ptr.add(offset) as *const f64)),
            FIELD_KIND_CHAR => {
                let ch = *(body_ptr.add(offset) as *const i32) as u32;
                char::from_u32(ch).unwrap_or('?').to_string()
            }
            _ => continue,
        };
        return Some(message);
    }

    None
}

fn runtime_render_stack_trace_text(stack: &[RuntimeFrame]) -> String {
    let mut rendered = String::new();
    for (index, frame) in stack.iter().rev().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&runtime_format_stack_frame(frame));
    }
    rendered
}

unsafe fn runtime_exception_message_text(exception: i64) -> String {
    let thrown_value = runtime_value_to_string(exception);
    if let Some(message) = runtime_exception_message_field(exception) {
        return message;
    }
    if let Some(stripped) = thrown_value.strip_prefix("RuntimeError:") {
        return stripped.trim().to_string();
    }
    if let Some(stripped) = thrown_value.strip_prefix("PANIC:") {
        return stripped.trim().to_string();
    }
    thrown_value
}

unsafe fn runtime_exception_wrapper_type_name(exception: i64) -> &'static str {
    let thrown_value = runtime_value_to_string(exception);
    if thrown_value.starts_with("RuntimeError:") {
        "RuntimeError"
    } else if thrown_value.starts_with("PANIC:") {
        "PanicError"
    } else {
        "Error"
    }
}

unsafe fn runtime_find_registered_type_id(type_name: &str) -> Option<usize> {
    (0..MAX_TYPES).find(|&type_id| {
        !TYPE_NAME_PTRS[type_id].is_null() && rt_type_name_string(type_id) == type_name
    })
}

unsafe fn runtime_attach_stack_to_error_value(exception: i64, stack: &[RuntimeFrame]) {
    if stack.is_empty() {
        return;
    }

    if runtime_error_like_type_id(exception).is_none() {
        return;
    }

    let current_stack = runtime_class_field_string(exception, "stack").unwrap_or_default();
    if !current_stack.trim().is_empty() {
        return;
    }

    let mut stack_value = new_string_from_rust_str(&runtime_render_stack_trace_text(stack));
    rt_push_root(&mut stack_value);
    let _ = runtime_set_class_ref_field(exception, "stack", stack_value);
    rt_pop_roots(1);
}

unsafe fn runtime_build_error_wrapper(
    preferred_type_name: &str,
    message: &str,
    stack: &[RuntimeFrame],
) -> Option<i64> {
    let type_id = runtime_find_registered_type_id(preferred_type_name)
        .or_else(|| runtime_find_registered_type_id("Error"))?;
    let body_size = gc::TYPE_TABLE[type_id].size;
    if body_size == 0 {
        return None;
    }

    let mut message_value = new_string_from_rust_str(message);
    let mut stack_value = new_string_from_rust_str(&runtime_render_stack_trace_text(stack));
    rt_push_root(&mut message_value);
    rt_push_root(&mut stack_value);

    let mut error_value = rt_class_new(type_id as i32, body_size as i64, 0, std::ptr::null(), 0);
    rt_push_root(&mut error_value);

    let _ = runtime_set_class_ref_field(error_value, "message", message_value);
    let _ = runtime_set_class_ref_field(error_value, "stack", stack_value);
    let _ = runtime_set_class_i32_field(error_value, "code", 0);

    rt_pop_roots(3);
    Some(error_value)
}

unsafe fn runtime_exception_headline(
    prefix: &str,
    exception: i64,
    thrown_value: &str,
) -> (String, String) {
    if let Some(stripped) = thrown_value.strip_prefix("RuntimeError:") {
        return ("RuntimeError".to_string(), stripped.trim().to_string());
    }

    if let Some(stripped) = thrown_value.strip_prefix("PANIC:") {
        return ("Panic".to_string(), stripped.trim().to_string());
    }

    if prefix == "UnhandledPromiseRejection" {
        if let Some(message) = runtime_exception_message_field(exception) {
            return ("UnhandledPromiseRejection".to_string(), message);
        }
        return ("UnhandledPromiseRejection".to_string(), thrown_value.to_string());
    }

    let type_name = runtime_exception_type_name(exception);
    if let Some(message) = runtime_exception_message_field(exception) {
        return (type_name, message);
    }

    if prefix == "CapturedException" {
        return ("Exception".to_string(), thrown_value.to_string());
    }

    ("UnhandledException".to_string(), thrown_value.to_string())
}

unsafe fn render_runtime_exception_report_with_stack(
    prefix: &str,
    exception: i64,
    stack: &[RuntimeFrame],
) -> String {
    let thrown_value = runtime_value_to_string(exception);
    let exception_type = runtime_exception_type_name(exception);
    let (headline_kind, headline_message) =
        runtime_exception_headline(prefix, exception, &thrown_value);
    let is_runtime_fault =
        thrown_value.starts_with("RuntimeError:") || thrown_value.starts_with("PANIC:");
    let mut report = String::new();
    report.push_str(&headline_kind);
    report.push_str(": ");
    report.push_str(&headline_message);
    report.push('\n');

    if !stack.is_empty() {
        report.push_str("Stack trace:\n");
        for frame in stack.iter().rev() {
            report.push_str(&runtime_format_stack_frame(frame));
            report.push('\n');
        }
    }

    let primitive_exception = matches!(
        exception_type.as_str(),
        "string" | "int" | "float" | "bool" | "char" | "none"
    );
    let error_like_exception = runtime_error_like_type_id(exception).is_some();
    let full_headline = format!("{}: {}", headline_kind, headline_message);
    let show_thrown_value = if primitive_exception {
        thrown_value != headline_message && thrown_value != full_headline
    } else if error_like_exception {
        false
    } else {
        true
    };

    let mut detail_lines = Vec::new();
    detail_lines.push(format!("Exception type: {}", exception_type));
    if show_thrown_value {
        detail_lines.push(format!("Thrown value: {}", thrown_value));
    }

    if let Some(hint) = runtime_error_hint(&thrown_value) {
        detail_lines.push(format!("Hint: {}", hint));
    }

    if let Some(frame) = stack.last() {
        let file = unsafe { runtime_frame_file(frame) };
        let location_label = if !is_runtime_fault {
            "exception thrown here"
        } else if headline_kind == "Panic" {
            "panic triggered here"
        } else {
            "runtime error occurred here"
        };
        report.push_str("Code frame:\n");
        if let Some(source_frame) = runtime_render_source_frame(&file, frame.line, location_label) {
            report.push_str(&source_frame);
        } else if let Some(source_line) = runtime_read_source_line(&file, frame.line) {
            report.push_str("  ");
            report.push_str(&runtime_format_location(frame));
            report.push('\n');
            report.push_str("  ");
            report.push_str(&source_line);
            report.push('\n');
        }
    }

    if !detail_lines.is_empty() {
        report.push_str("Details:\n");
        for line in detail_lines {
            report.push_str("  ");
            report.push_str(&line);
            report.push('\n');
        }
    }

    report
}

pub(crate) unsafe fn render_runtime_exception_report(prefix: &str, exception: i64) -> String {
    if prefix == "UnhandledPromiseRejection" {
        if let Some(stored_stack) = runtime_exception_trace_snapshot(exception) {
            return render_runtime_exception_report_with_stack(prefix, exception, &stored_stack);
        }
    }

    let live_stack = runtime_call_stack_snapshot();
    if !live_stack.is_empty() {
        return render_runtime_exception_report_with_stack(prefix, exception, &live_stack);
    }

    if let Some(stored_stack) = runtime_exception_trace_snapshot(exception) {
        return render_runtime_exception_report_with_stack(prefix, exception, &stored_stack);
    }

    render_runtime_exception_report_with_stack(prefix, exception, &[])
}

fn panic_payload_message(info: &PanicHookInfo<'_>) -> String {
    if let Some(msg) = info.payload().downcast_ref::<&str>() {
        (*msg).to_string()
    } else if let Some(msg) = info.payload().downcast_ref::<String>() {
        msg.clone()
    } else {
        "panic without a string payload".to_string()
    }
}

fn render_runtime_panic_report(info: &PanicHookInfo<'_>) -> String {
    let mut report = String::new();
    report.push_str("Internal Runtime Panic\n");
    report.push_str("  Message: ");
    report.push_str(&panic_payload_message(info));
    report.push('\n');

    if let Some(location) = info.location() {
        report.push_str("  Rust Location: ");
        report.push_str(location.file());
        report.push(':');
        report.push_str(&location.line().to_string());
        report.push(':');
        report.push_str(&location.column().to_string());
        report.push('\n');
    }

    let stack = runtime_call_stack_snapshot();
    if let Some(frame) = stack.last() {
        report.push_str("  TejX Location: ");
        report.push_str(&runtime_format_location(frame));
        report.push('\n');
        let file = unsafe { runtime_frame_file(frame) };
        if let Some(source_frame) =
            runtime_render_source_frame(&file, frame.line, "panic observed here")
        {
            report.push_str(&source_frame);
        } else if let Some(source_line) = runtime_read_source_line(&file, frame.line) {
            report.push_str("  Source: ");
            report.push_str(&source_line);
            report.push('\n');
        }
    }

    if !stack.is_empty() {
        report.push_str("  Call Stack:\n");
        for frame in stack.iter().rev() {
            report.push_str("    at ");
            report.push_str(&runtime_format_location(frame));
            report.push('\n');
        }
    }

    report.push_str(
        "  Hint: This is likely a TejX runtime bug rather than a recoverable program exception.\n",
    );
    report
}

fn install_runtime_panic_hook() {
    RUNTIME_PANIC_HOOK.call_once(|| {
        panic::set_hook(Box::new(|info| {
            use std::io::Write;
            let report = render_runtime_panic_report(info);
            let _ = std::io::stderr().write_all(report.as_bytes());
        }));
    });
}

#[inline]
unsafe fn runtime_set_current_location(file_ptr: i64, line: i64) {
    let line = line.max(0) as usize;
    if file_ptr == 0 && line == 0 {
        return;
    }

    RUNTIME_CALL_STACK.with(|stack| {
        if let Some(frame) = stack.borrow_mut().last_mut() {
            if !frame.track_location {
                return;
            }
            if line > 0 {
                frame.line = line;
            }
            if file_ptr != 0 {
                frame.file_ptr = file_ptr;
            }
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_enter_frame(function_ptr: i64, file_ptr: i64, line: i64) {
    let track_location = file_ptr != 0 || line > 0;
    let line = if track_location {
        line.max(0) as usize
    } else {
        0
    };
    let file_ptr = if track_location { file_ptr } else { 0 };

    RUNTIME_CALL_STACK.with(|stack| {
        stack.borrow_mut().push(RuntimeFrame {
            function_ptr,
            file_ptr,
            line,
            track_location,
        });
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_leave_frame() {
    RUNTIME_CALL_STACK.with(|stack| {
        stack.borrow_mut().pop();
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_location(file_ptr: i64, line: i64) {
    runtime_set_current_location(file_ptr, line);
}

unsafe fn rt_ensure_type_finalizer(this: i64, finalizer: unsafe extern "C" fn(i64)) {
    let ptr = rt_obj_ptr(this) as *mut u8;
    if ptr.is_null() || !rt_is_gc_ptr(ptr) {
        return;
    }
    let header = rt_get_header(ptr);
    let type_id = (*header).type_id as usize;
    if type_id >= MAX_TYPES {
        return;
    }
    let entry = &mut gc::TYPE_TABLE[type_id];
    if let Some(existing) = entry.finalizer {
        if existing as usize != finalizer as usize {
            return;
        }
    } else {
        entry.finalizer = Some(finalizer);
    }
}

#[no_mangle]
pub static HEAP_OFFSET: i64 = 1i64 << 50;
#[no_mangle]
pub static STACK_OFFSET: i64 = 1i64 << 48;

#[inline]
pub unsafe fn rt_obj_ptr(val: i64) -> *mut i64 {
    if val >= HEAP_OFFSET {
        (val - HEAP_OFFSET) as *mut i64
    } else if val >= STACK_OFFSET {
        (val - STACK_OFFSET) as *mut i64
    } else {
        std::ptr::null_mut()
    }
}

#[no_mangle]
pub static TAG_BOOLEAN: i64 = 1;
#[no_mangle]
pub static TAG_STRING: i64 = 2;
#[no_mangle]
pub static TAG_ARRAY: i64 = 3;
#[no_mangle]
pub static TAG_CHAR: i64 = 4;
#[no_mangle]
pub static TAG_INT: i64 = 5;
#[no_mangle]
pub static TAG_FLOAT: i64 = 6;
#[no_mangle]
pub static TAG_OBJECT: i64 = 7;
#[no_mangle]
pub static TAG_FUNCTION: i64 = 8;
#[no_mangle]
pub static TAG_PROMISE: i64 = 10;
pub const PROMISE_BODY_SIZE: usize = 40;
#[no_mangle]
pub static TAG_RAW_DATA: i64 = 11;

// --- Object Layout Constants ---
pub const OBJECT_SIZE_OFFSET: isize = 0;
pub const OBJECT_CAP_OFFSET: isize = 8;
pub const OBJECT_KEYS_OFFSET: isize = 16;
pub const OBJECT_VALUES_OFFSET: isize = 24;

pub const ARRAY_FLAG_FIXED: i64 = 0x0100;
pub const ARRAY_FLAG_CONSTANT: i64 = 0x0200;
pub const ARRAY_FLAG_PTR: i64 = 0x0400;
pub const ARRAY_FLAG_KIND_MASK: i64 = 0xF000;
pub const ARRAY_FLAG_KIND_SIGNED: i64 = 0x0000;
pub const ARRAY_FLAG_KIND_UNSIGNED: i64 = 0x1000;
pub const ARRAY_FLAG_KIND_FLOAT: i64 = 0x2000;
pub const ARRAY_FLAG_KIND_BOOL: i64 = 0x3000;
pub const ARRAY_FLAG_KIND_CHAR: i64 = 0x4000;
// Boolean sentinels (below HEAP_OFFSET, above normal number range)
#[no_mangle]
pub static BOOL_FALSE: i64 = 0;
#[no_mangle]
pub static BOOL_TRUE: i64 = 1;

#[no_mangle]
pub static mut LAST_ID: i64 = 0;
#[no_mangle]
pub static mut LAST_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut LAST_LEN: i64 = 0;
#[no_mangle]
pub static mut LAST_ELEM_SIZE: i64 = 0;

#[no_mangle]
pub static mut PREV_ID: i64 = 0;
#[no_mangle]
pub static mut PREV_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut PREV_LEN: i64 = 0;
#[no_mangle]
pub static mut PREV_ELEM_SIZE: i64 = 0;

#[no_mangle]
pub unsafe extern "C" fn rt_invalidate_array_cache(id: i64) {
    if LAST_ID == id {
        LAST_ID = 0;
    }
    if PREV_ID == id {
        PREV_ID = 0;
    }
    if PREV2_ID == id {
        PREV2_ID = 0;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_update_array_cache(id: i64, data: *mut u8, len: i64, elem_size: i64) {
    if LAST_ID == id {
        LAST_PTR = data;
        LAST_LEN = len;
        LAST_ELEM_SIZE = elem_size;
        return;
    }
    PREV2_ID = PREV_ID;
    PREV2_PTR = PREV_PTR;
    PREV2_LEN = PREV_LEN;
    PREV2_ELEM_SIZE = PREV_ELEM_SIZE;

    PREV_ID = LAST_ID;
    PREV_PTR = LAST_PTR;
    PREV_LEN = LAST_LEN;
    PREV_ELEM_SIZE = LAST_ELEM_SIZE;

    LAST_ID = id;
    LAST_PTR = data;
    LAST_LEN = len;
    LAST_ELEM_SIZE = elem_size;
}

#[no_mangle]
pub static mut PREV2_ID: i64 = 0;
#[no_mangle]
pub static mut PREV2_PTR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut PREV2_LEN: i64 = 0;
#[no_mangle]
pub static mut PREV2_ELEM_SIZE: i64 = 0;

static ARRAY_FORWARD: LazyLock<Mutex<HashMap<i64, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static ARRAY_FORWARD_ACTIVE: AtomicBool = AtomicBool::new(false);

#[inline]
pub unsafe fn rt_resolve_array_id(mut id: i64) -> i64 {
    if id < HEAP_OFFSET {
        return id;
    }
    if !ARRAY_FORWARD_ACTIVE.load(Ordering::Acquire) {
        return id;
    }
    let map = ARRAY_FORWARD.lock().unwrap();
    while let Some(next) = map.get(&id) {
        if *next == id {
            break;
        }
        id = *next;
    }
    id
}

extern "C" {
    pub fn malloc(size: usize) -> *mut std::ffi::c_void;
    pub fn calloc(num: usize, size: usize) -> *mut std::ffi::c_void;
    pub fn free(p: *mut std::ffi::c_void);
    pub fn realloc(ptr: *mut std::ffi::c_void, size: usize) -> *mut std::ffi::c_void;
    pub fn strlen(s: *const std::ffi::c_char) -> usize;
    pub fn memcpy(
        dest: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
        n: usize,
    ) -> *mut std::ffi::c_void;
    pub fn memcmp(s1: *const std::ffi::c_void, s2: *const std::ffi::c_void, n: usize) -> i32;
    pub fn printf(fmt: *const std::ffi::c_char, ...) -> i32;
    pub fn sprintf(str: *mut std::ffi::c_char, fmt: *const std::ffi::c_char, ...) -> i32;
    pub fn atof(s: *const std::ffi::c_char) -> f64;
    pub fn exit(code: i32) -> !;
    pub fn write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
    pub fn mmap(
        addr: *mut std::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: isize,
    ) -> *mut std::ffi::c_void;
    pub fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
    pub fn memset(ptr: *mut std::ffi::c_void, value: i32, num: usize) -> *mut std::ffi::c_void;
    pub fn fflush(stream: *mut std::ffi::c_void) -> i32;
    pub fn memmove(
        dest: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
        n: usize,
    ) -> *mut std::ffi::c_void;
}

pub const PROT_READ: i32 = 0x1;
pub const PROT_WRITE: i32 = 0x2;
pub const MAP_PRIVATE: i32 = 0x02;
pub const MAP_ANON: i32 = 0x1000;

#[repr(C)]
pub struct Slice {
    pub ptr: i64,
    pub len: i64,
}

#[no_mangle]
pub unsafe fn rt_get_ptr(val: i64) -> *mut i64 {
    if val < HEAP_OFFSET {
        return std::ptr::null_mut();
    }
    (val - HEAP_OFFSET) as *mut i64
}

#[inline]
pub unsafe fn rt_store_ref_slot(owner: i64, slot: *mut i64, value: i64) {
    *slot = value;
    if owner >= HEAP_OFFSET {
        rt_write_barrier(owner, value);
    }
}

// --- Conversions ---

#[no_mangle]
pub extern "C" fn rt_i64_to_i8(n: i64) -> i8 {
    n as i8
}
#[no_mangle]
pub extern "C" fn rt_f64_to_i8(n: f64) -> i8 {
    n as i8
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_number(val: i64) -> f64 {
    if val < HEAP_OFFSET {
        return val as f64;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body) {
        return f64::from_bits(val as u64);
    }
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    let ptr = body as *const i64;
    if tag == TAG_FLOAT {
        return *(body as *const f64);
    }
    if tag == TAG_INT {
        return *ptr as f64;
    }
    if tag == TAG_CHAR {
        return *ptr as f64;
    }
    if tag == TAG_BOOLEAN {
        return *ptr as f64;
    }
    if tag == TAG_STRING {
        return atof(body as *const _);
    }

    // Fallback for other objects: return pointer bits as a double
    f64::from_bits(val as u64)
}

// --- Memory Management ---

#[no_mangle]
pub unsafe extern "C" fn rt_malloc(size: usize) -> *mut u8 {
    let header_size = std::mem::size_of::<ObjectHeader>();
    let p = calloc(1, size + header_size);
    if p.is_null() {
        //printf("FATAL: Out of memory\n\0".as_ptr() as *const _);
        std::process::exit(1);
    }
    let header = p as *mut ObjectHeader;
    (*header).gc_word = 0;
    (*header).type_id = 0;
    (*header).flags = 0;
    (*header).length = 0;
    (*header).capacity = 0;

    (p as *mut u8).add(header_size)
}

#[no_mangle]
pub unsafe extern "C" fn rt_free_raw(p: *mut std::ffi::c_void) {
    if !p.is_null() {
        if rt_is_gc_ptr(p as *mut u8) {
            return;
        }
        let orig_p = (p as *mut u8).sub(24); // ObjectHeader size is 24
        free(orig_p as *mut std::ffi::c_void);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_free(val: i64) {
    if val < HEAP_OFFSET {
        return;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    if rt_is_gc_ptr(body) {
        // GC objects are managed by the GC, don't free them manually
        return;
    }

    let header = rt_get_header(body);
    let type_id = (*header).type_id as i64;
    let ptr = body as *mut i64;

    if let Some(atom_ptr) = RAW_ATOMIC_OBJECTS.lock().unwrap().remove(&(body as usize)) {
        let _ = Box::from_raw(atom_ptr as *mut AtomicI64);
        rt_free_raw(ptr as *mut std::ffi::c_void);
        return;
    }

    if type_id == TAG_ARRAY {
        // Old layout support for arrays if any still exist using rt_malloc
        let len = *ptr.offset(1); // Old layout: tag, len, cap, ...
        let elem_size = *ptr.offset(4); // Old layout: ... flags, elem_size
        let data = *ptr.offset(3) as *mut i64;
        if !data.is_null() {
            if elem_size == 8 {
                for i in 0..len {
                    rt_free(*data.offset(i as isize));
                }
            }
            rt_free_raw(data as *mut std::ffi::c_void);
        }
    } else if type_id == TAG_OBJECT {
        let _cap = *ptr.offset(1);
        let keys = *ptr.offset(2) as *mut i64;
        let values = *ptr.offset(3) as *mut i64;
        if !keys.is_null() {
            for i in 0.._cap {
                let k = *keys.offset(i as isize);
                if k != 0 {
                    rt_free(k);
                    rt_free(*values.offset(i as isize));
                }
            }
            free(keys as *mut std::ffi::c_void);
        }
        if !values.is_null() {
            free(values as *mut std::ffi::c_void);
        }
    }

    rt_free_raw(ptr as *mut std::ffi::c_void);
}

// --- Tagging Primitives ---

#[no_mangle]
pub unsafe extern "C" fn rt_clone(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return val;
    }
    let mut source = val;
    rt_push_root(&mut source);

    let body = (source - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;

    let res = if tag == TAG_STRING {
        let len = rt_len(source);
        new_string_from_parts(source, 0, len)
    } else if tag == TAG_ARRAY {
        let len = (*header).length as i64;
        let elem_size = ((*header).flags & 0xFF) as i64;

        // Create new array with same elem_size
        let mut new_arr_val = rt_Array_new(len, elem_size);
        rt_push_root(&mut new_arr_val);
        let new_body = (new_arr_val - HEAP_OFFSET) as *mut u8;
        let new_data = new_body as *mut i8;

        if len > 0 {
            if elem_size == 8 {
                for i in 0..len {
                    *(new_data as *mut i64).offset(i as isize) = rt_clone(rt_array_get_fast(source, i));
                }
            } else {
                let src_body = (source - HEAP_OFFSET) as *mut u8;
                memcpy(
                    new_data as *mut _,
                    src_body as *const _,
                    (len * elem_size) as usize,
                );
            }
        }
        rt_pop_roots(1);
        new_arr_val
    } else {
        // For other types (Objects, Char, Int, Float, Boolean, etc.), we can do a shallow copy for now,
        // but for Map/Object we might want to eventually do a deeper clone if they have nested objects.
        // However, for anagram.tx, this should be enough.
        source
    };

    rt_pop_roots(1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_string_from_c_str(s: *const std::ffi::c_char) -> i64 {
    if s.is_null() {
        return 0;
    }
    let len = strlen(s);
    let body_ptr = alloc_string_body(len as i64, len as i64);

    std::ptr::copy_nonoverlapping(s as *const u8, body_ptr, len);
    *(body_ptr.add(len)) = 0;

    let res = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(res, body_ptr, len as i64, 1);
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_string_from_c_str_const(s: *const std::ffi::c_char) -> i64 {
    if s.is_null() {
        return 0;
    }

    let len = strlen(s);
    let bytes = std::slice::from_raw_parts(s as *const u8, len);

    if let Some(slot) = CONST_STRING_ROOTS
        .lock()
        .unwrap()
        .slots_by_bytes
        .get(bytes)
        .copied()
    {
        return rt_get_static_root(slot);
    }

    if gc::EDEN_START.is_null() {
        rt_init_gc();
    }

    let body_ptr = gc_allocate(len + 1);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_STRING as u16;
    (*header).length = len as u32;
    (*header).capacity = len as u32;
    (*header).flags |= STRING_FLAG_FROZEN;

    std::ptr::copy_nonoverlapping(s as *const u8, body_ptr, len);
    *(body_ptr.add(len)) = 0;

    let res = (body_ptr as i64) + HEAP_OFFSET;
    rt_update_array_cache(res, body_ptr, len as i64, 1);

    let mut roots = CONST_STRING_ROOTS.lock().unwrap();
    if let Some(slot) = roots.slots_by_bytes.get(bytes).copied() {
        return rt_get_static_root(slot);
    }
    let slot = rt_add_static_root(res);
    roots.slots_by_bytes.insert(bytes.to_vec(), slot);
    res
}

pub unsafe fn alloc_string_body(capacity: i64, len: i64) -> *mut u8 {
    let cap = if capacity < len { len } else { capacity };
    let body_ptr = gc_allocate(cap as usize + 1);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_STRING as u16;
    (*header).length = len as u32;
    (*header).capacity = cap as u32;
    body_ptr
}

// --- IO Primitives ---

#[no_mangle]
pub unsafe extern "C" fn tejx_libc_write(fd: i64, s_ptr: i64) -> i64 {
    if s_ptr >= HEAP_OFFSET {
        let body = (s_ptr - HEAP_OFFSET) as *const u8;
        let header = rt_get_header(body as *mut u8);
        if (*header).type_id == TAG_STRING as u16 {
            let len = (*header).length as usize;
            return write(fd as i32, body as *const _, len) as i64;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn tejx_libc_puts(s_ptr: i64) -> i64 {
    if s_ptr >= HEAP_OFFSET {
        let body = (s_ptr - HEAP_OFFSET) as *const u8;
        let header = rt_get_header(body as *mut u8);
        if (*header).type_id == TAG_STRING as u16 {
            printf("%s\n\0".as_ptr() as *const _, body as *const _);
            fflush(std::ptr::null_mut());
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string_int(v: i64) -> i64 {
    let mut buf = [0u8; 32];
    sprintf(buf.as_mut_ptr() as *mut _, "%lld\0".as_ptr() as *const _, v);
    rt_string_from_c_str(buf.as_ptr() as *const _)
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string_float(v: f64) -> i64 {
    let s = rt_float_to_rust_string(v);
    new_string_from_rust_str(&s)
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string_boolean(v: i64) -> i64 {
    if v != 0 {
        rt_string_from_c_str("true\0".as_ptr() as *const _)
    } else {
        rt_string_from_c_str("false\0".as_ptr() as *const _)
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_char(c: i32) -> i64 {
    let size = 1;
    let ptr = gc_allocate(size as usize);
    let header = rt_get_header(ptr);
    (*header).type_id = TAG_CHAR as u16;
    (*header).length = 1;

    *(ptr as *mut u8) = c as u8;
    (ptr as i64) + HEAP_OFFSET
}

#[inline]
fn looks_like_unboxed_float_bits(val: i64) -> bool {
    let bits = val as u64;
    let exp = (bits & 0x7FF0_0000_0000_0000) >> 52;
    if exp == 0 || exp == 2047 {
        return false;
    }

    val >= HEAP_OFFSET || (bits & (1u64 << 63)) != 0
}

fn rt_float_to_rust_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }

    let eps = 1e-5_f64.max(v.abs() * 1e-7);
    for decimals in 0..=6 {
        let factor = 10_f64.powi(decimals);
        let rounded = (v * factor).round() / factor;
        if (v - rounded).abs() <= eps {
            let mut s = format!("{:.*}", decimals as usize, rounded);
            if let Some(dot) = s.find('.') {
                let mut end = s.len();
                while end > dot + 1 && s.as_bytes()[end - 1] == b'0' {
                    end -= 1;
                }
                if end == dot + 1 {
                    end = dot;
                }
                s.truncate(end);
            }
            return s;
        }
    }

    v.to_string()
}

unsafe fn rt_value_body_and_tag(val: i64) -> Option<(*mut u8, i64)> {
    let is_gc = if val >= HEAP_OFFSET {
        rt_is_gc_ptr((val - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };
    let is_stack = val >= STACK_OFFSET && val < HEAP_OFFSET;
    if !is_gc && !is_stack {
        return None;
    }

    let body_ptr = if is_gc {
        (val - HEAP_OFFSET) as *mut u8
    } else {
        (val - STACK_OFFSET) as *mut u8
    };
    let header = rt_get_header(body_ptr);
    Some((body_ptr, (*header).type_id as i64))
}

unsafe fn rt_value_is_string(val: i64) -> bool {
    matches!(rt_value_body_and_tag(val), Some((_body, tag)) if tag == TAG_STRING)
}

fn rt_quote_rust_string(text: &str) -> String {
    format!("{:?}", text)
}

unsafe fn rt_type_name_string(type_id: usize) -> String {
    if type_id >= MAX_TYPES {
        return "object".to_string();
    }
    let ptr = TYPE_NAME_PTRS[type_id];
    if ptr.is_null() {
        return "object".to_string();
    }
    std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

unsafe fn rt_field_name_string(type_id: usize, index: usize) -> String {
    if type_id >= MAX_TYPES || index >= TYPE_INFO_MAX_FIELDS {
        return format!("field{}", index);
    }
    let ptr = TYPE_FIELD_NAMES[type_id][index];
    if ptr.is_null() {
        return format!("field{}", index);
    }
    std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

unsafe fn rt_format_scalar_value(val: i64) -> String {
    if val == 0 {
        return "None".to_string();
    }
    if looks_like_unboxed_float_bits(val) {
        return rt_float_to_rust_string(f64::from_bits(val as u64));
    }
    val.to_string()
}

unsafe fn rt_format_composite_value(val: i64, depth: usize, seen: &mut Vec<i64>) -> String {
    if depth == 0 {
        return "...".to_string();
    }

    if let Some((body_ptr, tag)) = rt_value_body_and_tag(val) {
        if tag == TAG_STRING {
            return i64_to_rust_str(val).unwrap_or_default();
        }
        if tag == TAG_FLOAT {
            return rt_float_to_rust_string(*(body_ptr as *const f64));
        }
        if tag == TAG_INT {
            let n = *(body_ptr as *const i64);
            if let Some((_nested_body, nested_tag)) = rt_value_body_and_tag(n) {
                if nested_tag == TAG_FUNCTION {
                    return "[Function]".to_string();
                }
            }
            return n.to_string();
        }
        if tag == TAG_CHAR {
            let ch = std::char::from_u32(*(body_ptr as *const i32) as u32).unwrap_or('\u{FFFD}');
            return ch.to_string();
        }
        if tag == TAG_BOOLEAN {
            return if *(body_ptr as *const i64) != 0 {
                "true".to_string()
            } else {
                "false".to_string()
            };
        }
        if tag == TAG_FUNCTION {
            return "[Function]".to_string();
        }
        if tag == TAG_PROMISE {
            return "[Promise]".to_string();
        }

        if seen.contains(&val) {
            return "[Circular]".to_string();
        }

        if tag == TAG_ARRAY {
            seen.push(val);
            let len = rt_len(val);
            let mut items = Vec::new();
            for i in 0..len {
                let item = rt_array_get_fast(val, i);
                let mut rendered = rt_format_composite_value(item, depth - 1, seen);
                if rt_value_is_string(item) {
                    rendered = rt_quote_rust_string(&rendered);
                }
                items.push(rendered);
            }
            seen.pop();
            return format!("[{}]", items.join(", "));
        }

        if tag == TAG_OBJECT {
            seen.push(val);
            let keys = rt_object_keys_array(val);
            let values = rt_object_values_array(val);
            let len = rt_len(keys);
            let mut items = Vec::new();
            for i in 0..len {
                let key_val = rt_array_get_fast(keys, i);
                let key = if rt_value_is_string(key_val) {
                    i64_to_rust_str(key_val).unwrap_or_default()
                } else {
                    rt_format_composite_value(key_val, depth - 1, seen)
                };
                let field_val = rt_array_get_fast(values, i);
                let mut value_text = rt_format_composite_value(field_val, depth - 1, seen);
                if rt_value_is_string(field_val) {
                    value_text = rt_quote_rust_string(&value_text);
                }
                items.push(format!("{}: {}", key, value_text));
            }
            seen.pop();
            if items.is_empty() {
                return "{}".to_string();
            }
            return format!("{{ {} }}", items.join(", "));
        }

        let type_id = tag as usize;
        if type_id < MAX_TYPES && !TYPE_NAME_PTRS[type_id].is_null() {
            if let Some(summary) = runtime_error_like_summary(val) {
                return summary;
            }

            seen.push(val);
            let type_name = rt_type_name_string(type_id);
            let field_count = TYPE_FIELD_COUNTS[type_id];
            let mut fields = Vec::new();
            for i in 0..field_count.min(TYPE_INFO_MAX_FIELDS) {
                let field_name = rt_field_name_string(type_id, i);
                let offset = TYPE_FIELD_OFFSETS[type_id][i];
                let kind = TYPE_FIELD_KINDS[type_id][i];
                let field_text = match kind {
                    FIELD_KIND_REF => {
                        let field_val = *(body_ptr.add(offset) as *const i64);
                        let mut rendered = rt_format_composite_value(field_val, depth - 1, seen);
                        if rt_value_is_string(field_val) {
                            rendered = rt_quote_rust_string(&rendered);
                        }
                        rendered
                    }
                    FIELD_KIND_BOOL => {
                        if *(body_ptr.add(offset) as *const i8) != 0 {
                            "true".to_string()
                        } else {
                            "false".to_string()
                        }
                    }
                    FIELD_KIND_INT8 => (*(body_ptr.add(offset) as *const i8)).to_string(),
                    FIELD_KIND_UINT8 => (*(body_ptr.add(offset) as *const u8)).to_string(),
                    FIELD_KIND_INT16 => (*(body_ptr.add(offset) as *const i16)).to_string(),
                    FIELD_KIND_UINT16 => (*(body_ptr.add(offset) as *const u16)).to_string(),
                    FIELD_KIND_INT32 => (*(body_ptr.add(offset) as *const i32)).to_string(),
                    FIELD_KIND_INT64 => (*(body_ptr.add(offset) as *const i64)).to_string(),
                    FIELD_KIND_UINT32 => (*(body_ptr.add(offset) as *const u32)).to_string(),
                    FIELD_KIND_UINT64 => (*(body_ptr.add(offset) as *const u64)).to_string(),
                    FIELD_KIND_INT128 => (*(body_ptr.add(offset) as *const i128)).to_string(),
                    FIELD_KIND_UINT128 => (*(body_ptr.add(offset) as *const u128)).to_string(),
                    FIELD_KIND_FLOAT32 => {
                        rt_float_to_rust_string(*(body_ptr.add(offset) as *const f32) as f64)
                    }
                    FIELD_KIND_FLOAT64 => {
                        rt_float_to_rust_string(*(body_ptr.add(offset) as *const f64))
                    }
                    FIELD_KIND_CHAR => {
                        let ch = std::char::from_u32(*(body_ptr.add(offset) as *const i32) as u32)
                            .unwrap_or('\u{FFFD}');
                        ch.to_string()
                    }
                    _ => "<unprintable>".to_string(),
                };
                fields.push(format!("{}: {}", field_name, field_text));
            }
            seen.pop();
            if fields.is_empty() {
                return format!("{} {{}}", type_name);
            }
            return format!("{} {{ {} }}", type_name, fields.join(", "));
        }

        return "[object Object]".to_string();
    }

    rt_format_scalar_value(val)
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_string(val: i64) -> i64 {
    let mut v = val;
    let mut res_id = 0i64;
    rt_push_root(&mut v);
    rt_push_root(&mut res_id);
    let rendered = rt_format_composite_value(v, FORMAT_MAX_DEPTH, &mut Vec::new());
    res_id = new_string_from_rust_str(&rendered);

    rt_pop_roots(2);
    res_id
}

unsafe fn render_captured_exception_report(exception: i64) -> Option<String> {
    let stack = runtime_exception_trace_snapshot(exception)?;
    Some(render_runtime_exception_report_with_stack(
        "CapturedException",
        exception,
        &stack,
    ))
}

#[no_mangle]
pub unsafe extern "C" fn rt_exception_report_for_print(val: i64) -> i64 {
    let mut v = val;
    let mut report_id = 0i64;
    rt_push_root(&mut v);
    rt_push_root(&mut report_id);

    if let Some(report) = render_captured_exception_report(v) {
        report_id = new_string_from_rust_str(&report);
    }

    rt_pop_roots(2);
    report_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_panic(msg_id: i64) {
    let msg = if let Some(s) = i64_to_rust_str(msg_id) {
        format!("PANIC: {}", s)
    } else {
        "PANIC: (invalid string object)".to_string()
    };
    rt_throw_runtime_error(&msg);
}

#[no_mangle]
pub unsafe extern "C" fn rt_div_zero_error(dividend: i64, divisor: i64) {
    let left = runtime_inline_value_to_string(dividend);
    let right = runtime_inline_value_to_string(divisor);
    let msg = format!(
        "RuntimeError: Division by zero while evaluating {} / {}",
        left, right
    );
    rt_throw_runtime_error(&msg);
}

#[no_mangle]
pub unsafe extern "C" fn rt_div_zero_error_at(
    dividend: i64,
    divisor: i64,
    file_ptr: i64,
    line: i64,
) {
    runtime_set_current_location(file_ptr, line);
    rt_div_zero_error(dividend, divisor);
}

// --- Metadata ---

#[no_mangle]
pub unsafe extern "C" fn rt_len(val: i64) -> i64 {
    let resolved = if val >= HEAP_OFFSET {
        rt_resolve_array_id(val)
    } else {
        val
    };
    let is_heap = resolved >= HEAP_OFFSET;
    let is_stack = resolved >= STACK_OFFSET && resolved < HEAP_OFFSET;
    if !is_heap && !is_stack {
        return 0;
    }
    let body = if is_heap {
        let body = (resolved - HEAP_OFFSET) as *mut u8;
        if !rt_is_gc_ptr(body) {
            return 0;
        }
        body
    } else {
        (resolved - STACK_OFFSET) as *mut u8
    };
    let header = rt_get_header(body);
    if (*header).type_id == TAG_STRING as u16 || (*header).type_id == TAG_ARRAY as u16 {
        return (*header).length as i64;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_strlen(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return 0;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    if (*header).type_id == TAG_STRING as u16 {
        return (*header).length as i64;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_array(val: i64) -> i64 {
    let resolved = if val >= HEAP_OFFSET {
        rt_resolve_array_id(val)
    } else {
        val
    };
    let is_heap = resolved >= HEAP_OFFSET;
    let is_stack = resolved >= STACK_OFFSET && resolved < HEAP_OFFSET;
    if !is_heap && !is_stack {
        return 0;
    }
    let body = if is_heap {
        let body = (resolved - HEAP_OFFSET) as *mut u8;
        if !rt_is_gc_ptr(body) {
            return 0;
        }
        body
    } else {
        (resolved - STACK_OFFSET) as *mut u8
    };
    let header = rt_get_header(body);
    if (*header).type_id == TAG_ARRAY as u16 {
        1
    } else {
        0
    }
}

// --- Raw Memory Access (Intrinsics) ---

#[no_mangle]
pub unsafe extern "C" fn rt_mem_set_i64(val_ptr: i64, offset: i64, val: i64) {
    let is_heap_owner = (val_ptr as u64) >= (HEAP_OFFSET as u64);
    let ptr = if is_heap_owner {
        (val_ptr - HEAP_OFFSET) as *mut u8
    } else {
        val_ptr as *mut u8
    };
    rt_store_ref_slot(
        if is_heap_owner { val_ptr } else { 0 },
        ptr.offset(offset as isize) as *mut i64,
        val,
    );
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_get_i64(val_ptr: i64, offset: i64) -> i64 {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *const u8
    } else {
        val_ptr as *const u8
    };
    *(ptr.offset(offset as isize) as *const i64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_set_f64(val_ptr: i64, offset: i64, val: f64) {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *mut u8
    } else {
        val_ptr as *mut u8
    };
    *(ptr.offset(offset as isize) as *mut f64) = val;
}

#[no_mangle]
pub unsafe extern "C" fn rt_mem_get_f64(val_ptr: i64, offset: i64) -> f64 {
    let ptr = if (val_ptr as u64) >= (HEAP_OFFSET as u64) {
        (val_ptr - HEAP_OFFSET) as *const u8
    } else {
        val_ptr as *const u8
    };
    *(ptr.offset(offset as isize) as *const f64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_class_new(
    type_id: i32,
    body_size: i64,
    _ptr_count: i64,
    _offsets_ptr: *const i64,
    stack_ptr: i64,
) -> i64 {
    if stack_ptr != 0 {
        // Tag it with STACK_OFFSET so runtime knows it's on stack
        return stack_ptr + STACK_OFFSET;
    }
    let size = (body_size) as usize; // Body size is now just for data, no internal tag
    let obj = gc_allocate(size) as *mut i64;

    // Primitives and fields now start at offset 0.

    let header = rt_get_header(obj as *mut u8);
    (*header).type_id = type_id as u16;
    // *obj = TAG_OBJECT; // Removed, type_id is in header
    (obj as i64) + HEAP_OFFSET
}

// --- Array Primitives ---

// Map layout: [size, capacity, keys_ptr, values_ptr, data_base] (after ObjectHeader)
// keys and values are parallel arrays of i64
// data_base: number of method-slot entries (set before user data)

#[no_mangle]
pub unsafe extern "C" fn rt_closure_new(_capacity: i64) -> i64 {
    let raw = rt_Array_new_fixed(2, 8);
    let body = (raw - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    (*header).type_id = TAG_FUNCTION as u16;
    raw
}

// rt_get_map_body removed.

#[no_mangle]
pub unsafe extern "C" fn rt_closure_from_ptr(ptr: i64) -> i64 {
    let closure = rt_closure_new(0);
    // index 0: fn_ptr, index 1: env_ptr (null)
    rt_array_set_fast(closure, 0, ptr);
    rt_array_set_fast(closure, 1, 0);
    closure
}

// Legacy Map and Set implementations removed.

unsafe fn i64_to_rust_str(val: i64) -> Option<String> {
    if (val as u64) < (HEAP_OFFSET as u64) {
        return None;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    if (*header).type_id != TAG_STRING as u16 {
        return None;
    }
    let len = (*header).length;
    let slice = std::slice::from_raw_parts(body, len as usize);
    Some(String::from_utf8_lossy(slice).to_string())
}

// Legacy Map and Set implementations removed.

#[no_mangle]
pub unsafe extern "C" fn rt_fs_read_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if let Ok(content) = std::fs::read_to_string(&p) {
            return new_string_from_rust_str(&content);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_write_sync(path: i64, content: i64) -> i64 {
    if let (Some(p), Some(c)) = (i64_to_rust_str(path), i64_to_rust_str(content)) {
        if std::fs::write(&p, c).is_ok() {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_append_sync(path: i64, content: i64) -> i64 {
    if let (Some(p), Some(c)) = (i64_to_rust_str(path), i64_to_rust_str(content)) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&p)
        {
            if f.write_all(c.as_bytes()).is_ok() {
                return 1;
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_exists(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if std::path::Path::new(&p).exists() {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_unlink_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        let path = std::path::Path::new(&p);
        if path.is_dir() {
            if std::fs::remove_dir_all(path).is_ok() {
                return 1;
            }
        } else {
            if std::fs::remove_file(path).is_ok() {
                return 1;
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_mkdir_sync(path: i64) -> i64 {
    if let Some(p) = i64_to_rust_str(path) {
        if std::fs::create_dir_all(&p).is_ok() {
            return 1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_fs_readdir_sync(path: i64) -> i64 {
    let mut v_path = path;
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut v_path);
    rt_push_root(&mut result);

    if let Some(p) = i64_to_rust_str(v_path) {
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    let mut name_id = new_string_from_rust_str(&name);
                    rt_push_root(&mut name_id);
                    result = rt_array_push(result, name_id);
                    rt_pop_roots(1);
                }
            }
        } else {
            rt_pop_roots(2);
            return 0;
        }
    } else {
        rt_pop_roots(2);
        return 0;
    }
    rt_pop_roots(2);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_args() -> i64 {
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut result);
    let args: Vec<String> = std::env::args().collect();
    for arg in args {
        let mut arg_id = new_string_from_rust_str(&arg);
        rt_push_root(&mut arg_id);
        result = rt_array_push(result, arg_id);
        rt_pop_roots(1);
    }
    rt_pop_roots(1);
    result
}

// --- String Helpers ---

unsafe fn get_str_parts(s: i64) -> Option<(*const u8, i64)> {
    if s < HEAP_OFFSET {
        return None;
    }
    let body = (s - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body) {
        return None;
    }
    let header = rt_get_header(body);
    if (*header).type_id != TAG_STRING as u16 {
        return None;
    }
    Some((body as *const u8, (*header).length as i64))
}

unsafe fn new_string_from_bytes(data: *const u8, len: i64) -> i64 {
    let body_ptr = alloc_string_body(len, len);

    memcpy(body_ptr as *mut _, data as *const _, len as usize);
    *(body_ptr.add(len as usize)) = 0;

    (body_ptr as i64) + HEAP_OFFSET
}

unsafe fn new_string_from_rust_str(s: &str) -> i64 {
    new_string_from_bytes(s.as_ptr(), s.len() as i64)
}

// --- String Operations ---

// Internal helper to create a string from parts of another, safe for GC moves
unsafe fn new_string_from_parts(source_s: i64, offset: i64, len: i64) -> i64 {
    if len <= 0 {
        return rt_string_from_c_str("\0".as_ptr() as *const _);
    }

    let mut s = source_s;
    rt_push_root(&mut s);

    let body_ptr = alloc_string_body(len, len);

    // RE-RESOLVE after potential GC
    if let Some((data, _)) = get_str_parts(s) {
        std::ptr::copy_nonoverlapping(data.offset(offset as isize), body_ptr, len as usize);
        *(body_ptr.add(len as usize)) = 0;

        let res = (body_ptr as i64) + HEAP_OFFSET;
        rt_update_array_cache(res, body_ptr, len as i64, 1);
        rt_pop_roots(1);
        res
    } else {
        rt_pop_roots(1);
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_exit(code: i64) {
    exit(code as i32)
}
#[no_mangle]
pub unsafe extern "C" fn rt_getenv(key: i64) -> i64 {
    if let Some(k) = i64_to_rust_str(key) {
        if let Ok(val) = std::env::var(&k) {
            return new_string_from_rust_str(&val);
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_all_env() -> i64 {
    let mut obj = rt_object_new();
    rt_push_root(&mut obj);

    for (key, value) in std::env::vars() {
        let mut key_id = new_string_from_rust_str(&key);
        rt_push_root(&mut key_id);
        let mut value_id = new_string_from_rust_str(&value);
        rt_push_root(&mut value_id);

        rt_set_property(obj, key_id, value_id);

        rt_pop_roots(2);
    }

    rt_pop_roots(1);
    obj
}

unsafe fn rt_string_from_owned_string(value: String) -> i64 {
    new_string_from_rust_str(&value)
}

unsafe fn rt_string_from_optional_string(value: Option<String>) -> i64 {
    if let Some(inner) = value {
        return rt_string_from_owned_string(inner);
    }
    0
}

fn runtime_home_dir() -> Option<String> {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return Some(home);
        }
    }

    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.is_empty() {
            return Some(home);
        }
    }

    let drive = std::env::var("HOMEDRIVE").ok()?;
    let path = std::env::var("HOMEPATH").ok()?;
    let full = format!("{}{}", drive, path);
    if full.is_empty() {
        None
    } else {
        Some(full)
    }
}

fn runtime_hostname() -> Option<String> {
    if let Ok(hostname) = std::env::var("HOSTNAME") {
        if !hostname.is_empty() {
            return Some(hostname);
        }
    }

    if let Ok(hostname) = std::env::var("COMPUTERNAME") {
        if !hostname.is_empty() {
            return Some(hostname);
        }
    }

    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } == 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            if end > 0 {
                return Some(String::from_utf8_lossy(&buf[..end]).into_owned());
            }
        }
    }

    None
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_os_type() -> i64 {
    rt_string_from_owned_string(std::env::consts::OS.to_string())
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_os_arch() -> i64 {
    rt_string_from_owned_string(std::env::consts::ARCH.to_string())
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_cwd() -> i64 {
    let cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    rt_string_from_optional_string(cwd)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_home_dir() -> i64 {
    rt_string_from_optional_string(runtime_home_dir())
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_temp_dir() -> i64 {
    rt_string_from_owned_string(std::env::temp_dir().to_string_lossy().into_owned())
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_hostname() -> i64 {
    rt_string_from_optional_string(runtime_hostname())
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_free_memory() -> i64 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_time_now() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[no_mangle]
pub unsafe extern "C" fn rt_sleep(ms: i64) {
    let actual_ms = rt_to_number(ms) as i64;
    if actual_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(actual_ms as u64));
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_time_now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_time_to_iso_string(timestamp_ms: i64) -> i64 {
    let seconds = timestamp_ms.div_euclid(1000);
    let millis = timestamp_ms.rem_euclid(1000);

    let mut tm: libc::tm = std::mem::zeroed();
    #[cfg(unix)]
    let ok = {
        let seconds_unix: libc::time_t = seconds as libc::time_t;
        !libc::gmtime_r(&seconds_unix, &mut tm).is_null()
    };
    #[cfg(windows)]
    let ok = {
        let seconds_windows: libc::time_t = seconds as libc::time_t;
        libc::gmtime_s(&mut tm, &seconds_windows) == 0
    };

    if !ok {
        return rt_string_from_c_str("Invalid Date\0".as_ptr() as *const _);
    }

    let formatted = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        millis
    );
    new_string_from_rust_str(&formatted)
}

#[no_mangle]
pub unsafe extern "C" fn rt_random_seed(seed: i64) {
    if let Ok(mut state) = RNG_STATE.lock() {
        let next = if seed == 0 {
            runtime_seed_now()
        } else {
            seed as u64
        };
        *state = if next == 0 { 1 } else { next };
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_random() -> f64 {
    let next = if let Ok(mut state) = RNG_STATE.lock() {
        if *state == 0 {
            *state = runtime_seed_now();
        }
        splitmix64_next(&mut state)
    } else {
        let mut fallback = runtime_seed_now();
        splitmix64_next(&mut fallback)
    };

    let mantissa = next >> 11;
    (mantissa as f64) * (1.0 / ((1u64 << 53) as f64))
}

#[no_mangle]
pub unsafe extern "C" fn rt_random_int(lower: i64, upper: i64) -> i64 {
    let (lo, hi) = if lower <= upper {
        (lower, upper)
    } else {
        (upper, lower)
    };

    let span = (hi as i128) - (lo as i128) + 1;
    if span <= 0 {
        return lo;
    }

    let next = if let Ok(mut state) = RNG_STATE.lock() {
        if *state == 0 {
            *state = runtime_seed_now();
        }
        splitmix64_next(&mut state)
    } else {
        let mut fallback = runtime_seed_now();
        splitmix64_next(&mut fallback)
    };

    lo + (next % (span as u64)) as i64
}

// --- Timer Management ---
use std::sync::atomic::AtomicI64;
use std::time::Duration;
use tokio::sync::oneshot;

static NEXT_TIMER_ID: AtomicI64 = AtomicI64::new(1);

struct TimeoutState {
    handle: usize,
    cancel_tx: Option<oneshot::Sender<()>>,
}

struct IntervalState {
    handle: usize,
    cancel_tx: oneshot::Sender<()>,
}

static TIMEOUT_CANCELS: LazyLock<Mutex<HashMap<i64, TimeoutState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static INTERVAL_CANCELS: LazyLock<Mutex<HashMap<i64, IntervalState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn timeout_duration_from_ms(ms: i64) -> Duration {
    Duration::from_millis(ms.max(0) as u64)
}

fn interval_duration_from_ms(ms: i64) -> Duration {
    Duration::from_millis(ms.max(1) as u64)
}

#[no_mangle]
pub unsafe extern "C" fn rt_timeout_worker(timer_id: i64) {
    let handle = TIMEOUT_CANCELS
        .lock()
        .ok()
        .and_then(|mut timeouts| timeouts.remove(&timer_id).map(|state| state.handle))
        .unwrap_or(0);
    if handle == 0 {
        return;
    }

    let closure_id = crate::event_loop::tejx_get_global_handle(handle);
    crate::event_loop::tejx_drop_global_handle(handle);
    if closure_id > 0 {
        rt_call_closure_no_args(closure_id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_interval_worker(timer_id: i64) {
    let handle = INTERVAL_CANCELS
        .lock()
        .ok()
        .and_then(|intervals| intervals.get(&timer_id).map(|state| state.handle))
        .unwrap_or(0);
    if handle == 0 {
        return;
    }

    let closure_id = crate::event_loop::tejx_get_global_handle(handle);
    if closure_id > 0 {
        rt_call_closure_no_args(closure_id);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_setTimeout(callback: i64, ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let handle = unsafe { crate::event_loop::tejx_create_global_handle(callback) };
    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    if let Ok(mut cancels) = TIMEOUT_CANCELS.lock() {
        cancels.insert(
            timer_id,
            TimeoutState {
                handle,
                cancel_tx: Some(cancel_tx),
            },
        );
    }
    crate::event_loop::TOKIO_RT.spawn(async move {
        let fired = tokio::select! {
            _ = tokio::time::sleep(timeout_duration_from_ms(ms)) => true,
            _ = &mut cancel_rx => false,
        };

        if fired {
            if let Ok(mut timeouts) = TIMEOUT_CANCELS.lock() {
                if let Some(timeout) = timeouts.get_mut(&timer_id) {
                    timeout.cancel_tx = None;
                    crate::event_loop::tejx_enqueue_task(
                        rt_timeout_worker as *const () as i64,
                        timer_id,
                    );
                } else {
                    crate::event_loop::tejx_drop_global_handle(handle);
                }
            }
        }
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    timer_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_setInterval(callback: i64, ms: i64) -> i64 {
    let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let handle = unsafe { crate::event_loop::tejx_create_global_handle(callback) };
    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    if let Ok(mut cancels) = INTERVAL_CANCELS.lock() {
        cancels.insert(timer_id, IntervalState { handle, cancel_tx });
    }
    crate::event_loop::TOKIO_RT.spawn(async move {
        let mut interval = tokio::time::interval(interval_duration_from_ms(ms));
        interval.tick().await; // first tick returns immediately

        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = &mut cancel_rx => {
                    break;
                }
            }

            crate::event_loop::tejx_enqueue_task(rt_interval_worker as *const () as i64, timer_id);
        }
        if let Ok(mut cancels) = INTERVAL_CANCELS.lock() {
            cancels.remove(&timer_id);
        }
        unsafe { crate::event_loop::tejx_drop_global_handle(handle) };
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    timer_id
}

#[no_mangle]
pub unsafe extern "C" fn rt_clearTimeout(id: i64) -> i64 {
    let timeout = TIMEOUT_CANCELS
        .lock()
        .ok()
        .and_then(|mut cancels| cancels.remove(&id));
    if let Some(timeout) = timeout {
        crate::event_loop::tejx_drop_global_handle(timeout.handle);
        if let Some(cancel_tx) = timeout.cancel_tx {
            let _ = cancel_tx.send(());
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_clearInterval(id: i64) -> i64 {
    let interval = INTERVAL_CANCELS
        .lock()
        .ok()
        .and_then(|mut cancels| cancels.remove(&id));
    if let Some(interval) = interval {
        crate::event_loop::tejx_drop_global_handle(interval.handle);
        let _ = interval.cancel_tx.send(());
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_Timeout_constructor(this: i64, id: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        if id > 0 {
            rt_clearTimeout(id);
        }
        return;
    }
    rt_ensure_type_finalizer(this, rt_timeout_object_finalizer);
    *ptr.offset(0) = id;
}

#[no_mangle]
pub unsafe extern "C" fn rt_Interval_constructor(this: i64, id: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        if id > 0 {
            rt_clearInterval(id);
        }
        return;
    }
    rt_ensure_type_finalizer(this, rt_interval_object_finalizer);
    *ptr.offset(0) = id;
}

#[no_mangle]
pub unsafe extern "C" fn rt_delay(ms: i64) -> i64 {
    let actual_ms = rt_to_number(ms).max(0.0) as u64;
    let pid = rt_promise_new();

    unsafe { crate::event_loop::tejx_inc_async_ops() };

    let handle = unsafe { crate::event_loop::tejx_create_global_handle(pid) };
    crate::event_loop::TOKIO_RT.spawn(async move {
        tokio::time::sleep(Duration::from_millis(actual_ms)).await;
        unsafe {
            crate::event_loop::tejx_enqueue_task(
                rt_delay_resolver_worker as *const () as i64,
                handle as i64,
            );
        }
    });

    pid
}

#[no_mangle]
pub unsafe extern "C" fn rt_delay_resolver_worker(handle_id: i64) {
    let handle = handle_id as usize;
    let actual_pid = crate::event_loop::tejx_get_global_handle(handle);
    crate::event_loop::tejx_drop_global_handle(handle);
    if actual_pid > 0 {
        rt_promise_resolve(actual_pid, 0);
    }
    crate::event_loop::tejx_dec_async_ops();
}

// --- Fast Path Helpers (for Codegen) ---

#[no_mangle]
pub unsafe extern "C" fn rt_to_number_v2(v: i64) -> i64 {
    // Unbox Any, convert it to f64 using standard rules, then return raw bits
    // instead of boxing it back in TAG_FLOAT. This allows LLVM to `bitcast` it directly to `double`.
    let f = rt_to_number(v); // Returns a f64
    f.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_add_static_root_global(val: i64) -> i64 {
    rt_add_static_root(val) as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_static_root_global(slot: i64) -> i64 {
    if slot < 0 {
        return 0;
    }
    rt_get_static_root(slot as usize)
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_static_root_global(slot: i64, val: i64) {
    if slot < 0 {
        return;
    }
    rt_set_static_root(slot as usize, val);
}

#[no_mangle]
pub unsafe extern "C" fn rt_register_type_info(
    id: i32,
    type_name_ptr: i64,
    field_count: i64,
    field_offsets: *const i64,
    field_kinds: *const u8,
    field_names: *const i64,
) {
    if id < 0 || id as usize >= MAX_TYPES {
        return;
    }

    let type_index = id as usize;
    TYPE_NAME_PTRS[type_index] = type_name_ptr as usize as *const std::ffi::c_char;

    let count = if field_count <= 0 {
        0
    } else if field_count as usize > TYPE_INFO_MAX_FIELDS {
        TYPE_INFO_MAX_FIELDS
    } else {
        field_count as usize
    };
    TYPE_FIELD_COUNTS[type_index] = count;

    for i in 0..count {
        TYPE_FIELD_OFFSETS[type_index][i] = if field_offsets.is_null() {
            0
        } else {
            *field_offsets.add(i) as usize
        };
        TYPE_FIELD_KINDS[type_index][i] = if field_kinds.is_null() {
            FIELD_KIND_UNSUPPORTED
        } else {
            *field_kinds.add(i)
        };
        TYPE_FIELD_NAMES[type_index][i] = if field_names.is_null() {
            std::ptr::null()
        } else {
            (*field_names.add(i) as usize) as *const std::ffi::c_char
        };
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_function_value(val: i64) -> i64 {
    if val == 0 {
        return 0;
    }
    if let Some((_body, tag)) = rt_value_body_and_tag(val) {
        if tag == TAG_FUNCTION {
            return val;
        }
    }
    rt_closure_from_ptr(val)
}

extern "C" {
    fn tejx_main();
    fn rt_init_types();
}

#[no_mangle]
pub unsafe extern "C" fn tejx_runtime_main(_argc: i32, _argv: *mut *mut u8) -> i32 {
    install_runtime_panic_hook();

    let run_result = panic::catch_unwind(|| unsafe {
        rt_init_gc();
        rt_register_thread();
        rt_init_types();
        tejx_main();
        tejx_run_event_loop();
    });

    if run_result.is_err() {
        return 1;
    }

    0
}

#[no_mangle]
pub unsafe extern "C" fn a_new() -> i64 {
    rt_Array_new_fixed(0, 8)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_ptr(closure: i64) -> i64 {
    // 1. Check if it's a GC pointer (a Closure object)
    let body_ptr_probe = (closure - HEAP_OFFSET) as *mut u8;
    if (closure >= HEAP_OFFSET) && rt_is_gc_ptr(body_ptr_probe) {
        let h = rt_get_header(body_ptr_probe);
        if (*h).type_id == TAG_INT as u16 {
            let inner = *(body_ptr_probe as *mut i64);
            return rt_get_closure_ptr(inner);
        }
        if (*h).type_id == TAG_FUNCTION as u16 || (*h).type_id == TAG_ARRAY as u16 {
            // It's a standard Closure (stored as an array where elem 0 is the func ptr)
            let mut res = rt_array_get_fast(closure, 0);
            if res >= HEAP_OFFSET {
                let body = (res - HEAP_OFFSET) as *mut u8;
                let h_inner = rt_get_header(body);
                if (*h_inner).type_id == TAG_INT as u16 {
                    res = *(body as *mut i64);
                }
            }
            return res;
        }
    }

    // 2. Otherwise treatment as a raw function address (as stored in Any)
    closure
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_closure_env(closure: i64) -> i64 {
    // If this is not a GC-managed closure object, treat env as null.
    if (closure as u64) >= (HEAP_OFFSET as u64) {
        let body_ptr_probe = (closure - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body_ptr_probe) {
            let h = rt_get_header(body_ptr_probe);
            if (*h).type_id == TAG_INT as u16 {
                let inner = *(body_ptr_probe as *mut i64);
                return rt_get_closure_env(inner);
            }
            if (*h).type_id == TAG_FUNCTION as u16 || (*h).type_id == TAG_ARRAY as u16 {
                return rt_array_get_fast(closure, 1);
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure(closure: i64, arg: i64) -> i64 {
    let mut c = closure;
    let mut a = arg;
    rt_push_root(&mut c);
    rt_push_root(&mut a);

    let is_raw_ptr = (c as u64) < (HEAP_OFFSET as u64) && c != 0;
    let ptr_val;
    let env;

    if !is_raw_ptr {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
        env = 0;
    }

    // Ensure ptr_val is unboxed if it's a heap object
    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(2);
        return 0;
    }
    let result = if is_raw_ptr {
        // Non-closure function: no env parameter
        let func: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(a, 0, 0, 0)
    } else {
        // Closure: first argument is env
        let func: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(env, a, 0, 0, 0)
    };

    rt_pop_roots(2);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure_void(closure: i64, arg: i64) {
    let mut c = closure;
    let mut a = arg;
    rt_push_root(&mut c);
    rt_push_root(&mut a);

    let is_raw_ptr = (c as u64) < (HEAP_OFFSET as u64) && c != 0;
    let ptr_val;
    let env;

    if !is_raw_ptr {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
        env = 0;
    }

    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(2);
        return;
    }

    if is_raw_ptr {
        let func: unsafe extern "C" fn(i64, i64, i64, i64) = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(i64, i64, i64, i64),
        >(raw_func_ptr as *const ());
        func(a, 0, 0, 0);
    } else {
        let func: unsafe extern "C" fn(i64, i64, i64, i64, i64) =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64, i64)>(
                raw_func_ptr as *const (),
            );
        func(env, a, 0, 0, 0);
    }

    rt_pop_roots(2);
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure_argv(closure: i64, args: i64) -> i64 {
    let mut c = closure;
    let mut a = args;
    rt_push_root(&mut c);
    rt_push_root(&mut a);

    let is_raw_ptr = (c as u64) < (HEAP_OFFSET as u64) && c != 0;
    let ptr_val;
    let env;

    if !is_raw_ptr {
        ptr_val = rt_get_closure_ptr(c);
        env = rt_get_closure_env(c);
    } else {
        ptr_val = c;
        env = 0;
    }

    // Ensure ptr_val is unboxed if it's a heap object
    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(2);
        return 0;
    }

    let mut a0 = 0;
    let mut a1 = 0;
    let mut a2 = 0;
    let mut a3 = 0;

    if a >= HEAP_OFFSET {
        let body = (a - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body) {
            let h = rt_get_header(body);
            if (*h).type_id == TAG_ARRAY as u16 {
                let len = (*h).length as i64;
                if len > 0 {
                    a0 = rt_array_get_fast(a, 0);
                }
                if len > 1 {
                    a1 = rt_array_get_fast(a, 1);
                }
                if len > 2 {
                    a2 = rt_array_get_fast(a, 2);
                }
                if len > 3 {
                    a3 = rt_array_get_fast(a, 3);
                }
            }
        }
    }

    let result = if is_raw_ptr {
        // Non-closure function: no env parameter
        let func: unsafe extern "C" fn(i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(a0, a1, a2, a3)
    } else {
        // Closure: first argument is env
        let func: unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64 =
            std::mem::transmute::<*const (), unsafe extern "C" fn(i64, i64, i64, i64, i64) -> i64>(
                raw_func_ptr as *const (),
            );
        func(env, a0, a1, a2, a3)
    };
    rt_pop_roots(2);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_test_invoke(func: i64) {
    let f: unsafe extern "C" fn() -> i64 = std::mem::transmute(func as *const ());
    f();
}

#[no_mangle]
pub unsafe extern "C" fn rt_call_closure_no_args(closure: i64) -> i64 {
    let mut c = closure;
    rt_push_root(&mut c);

    let is_raw_ptr = (c as u64) < (HEAP_OFFSET as u64) && c != 0;
    let ptr_val = if !is_raw_ptr {
        rt_get_closure_ptr(c)
    } else {
        c
    };
    let env = if !is_raw_ptr {
        rt_get_closure_env(c)
    } else {
        0
    };

    // Ensure ptr_val is unboxed if it's a heap object
    let mut raw_func_ptr = ptr_val;
    if raw_func_ptr >= HEAP_OFFSET {
        let body = (raw_func_ptr - HEAP_OFFSET) as *mut u8;
        let h = rt_get_header(body);
        if (*h).type_id == TAG_INT as u16 {
            raw_func_ptr = *(body as *mut i64);
        }
    }

    if raw_func_ptr == 0 {
        rt_pop_roots(1);
        return 0;
    }

    let result = if is_raw_ptr {
        // Raw function pointers explicitly have NO env argument.
        let func: unsafe extern "C" fn() -> i64 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn() -> i64,
        >(raw_func_ptr as *const ());
        func()
    } else if env == 0 {
        // Closure created from raw pointer: no env parameter
        let func: unsafe extern "C" fn() -> i64 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn() -> i64,
        >(raw_func_ptr as *const ());
        func()
    } else {
        // Heap closures expect at least an env argument.
        let func: unsafe extern "C" fn(i64) -> i64 = std::mem::transmute::<
            *const (),
            unsafe extern "C" fn(i64) -> i64,
        >(raw_func_ptr as *const ());
        func(env)
    };

    rt_pop_roots(1);
    result
}

#[no_mangle]
pub unsafe extern "C" fn rt_not(val: i64) -> i64 {
    if rt_to_boolean(val) != 0 {
        BOOL_FALSE
    } else {
        BOOL_TRUE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_boolean(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return if val != 0 { 1 } else { 0 };
    }
    let body = (val - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    let tag = (*header).type_id as i64;
    if tag == TAG_BOOLEAN {
        return if *body != 0 { 1 } else { 0 };
    }
    1 // Other objects are truthy
}
// --- Atomic Operations ---
// Atomic objects store an AtomicI64 at offset 0 (as a boxed pointer)

unsafe fn get_atomic(this: i64) -> Option<&'static AtomicI64> {
    let ptr = rt_obj_ptr(this) as *const i64;
    if ptr.is_null() {
        return None;
    }
    let atom_ptr = *ptr.offset(0) as *const AtomicI64;
    if atom_ptr.is_null() {
        return None;
    }
    Some(&*atom_ptr)
}

#[no_mangle]
pub unsafe extern "C" fn rt_atomic_new(val: i64) -> i64 {
    let atom = Box::new(AtomicI64::new(val));
    let atom_ptr = Box::into_raw(atom);
    let obj = rt_malloc(8) as *mut i64;
    *obj.offset(0) = atom_ptr as i64;
    RAW_ATOMIC_OBJECTS
        .lock()
        .unwrap()
        .insert(obj as usize, atom_ptr as usize);
    let result = (obj as i64) + HEAP_OFFSET;

    result
}

// --- Mutex Operations ---
// Mutex objects store a Box<std::sync::Mutex<()>> pointer at offset 0

struct HeldMutexGuard {
    guard: std::sync::MutexGuard<'static, ()>,
    _mutex: Option<std::sync::Arc<std::sync::Mutex<()>>>,
}

thread_local! {
    static HELD_MUTEX_GUARDS: std::cell::RefCell<std::collections::HashMap<usize, HeldMutexGuard>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

#[no_mangle]
pub unsafe extern "C" fn f_any_lock(m: i64) {
    rt_Mutex_acquire(m);
}
#[no_mangle]
pub unsafe extern "C" fn f_any_unlock(m: i64) {
    rt_Mutex_release(m);
}

// --- Thread Operations ---

struct ThreadData {
    handle: Option<std::thread::JoinHandle<()>>,
    started: bool,
    cb_slot: usize,
    slot_live: std::sync::Arc<AtomicBool>,
}

struct ThreadRunGuard {
    cb_slot: usize,
    slot_live: std::sync::Arc<AtomicBool>,
}

unsafe fn rt_release_thread_cb_slot(cb_slot: usize, slot_live: &AtomicBool) {
    if slot_live.swap(false, Ordering::SeqCst) {
        rt_release_static_root(cb_slot);
    }
}

impl Drop for ThreadRunGuard {
    fn drop(&mut self) {
        unsafe {
            rt_release_thread_cb_slot(self.cb_slot, &self.slot_live);
            rt_unregister_thread();
        }
    }
}

// --- SharedQueue Operations ---

// Map aliases removed.

// --- Condition Variables ---

use std::sync::Condvar;

struct ConditionData {
    condvar: Condvar,
}

unsafe extern "C" fn rt_atomic_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let atom_ptr = *ptr.offset(0) as *mut AtomicI64;
    if atom_ptr.is_null() {
        return;
    }
    *ptr.offset(0) = 0;
    let _ = Box::from_raw(atom_ptr);
}

unsafe extern "C" fn rt_mutex_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let mutex_ptr = *ptr.offset(0) as *mut std::sync::Arc<std::sync::Mutex<()>>;
    if mutex_ptr.is_null() {
        return;
    }
    *ptr.offset(0) = 0;
    let _ = Box::from_raw(mutex_ptr);
}

unsafe extern "C" fn rt_condition_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let data_ptr = *ptr.offset(0) as *mut std::sync::Arc<ConditionData>;
    if data_ptr.is_null() {
        return;
    }
    *ptr.offset(0) = 0;
    let _ = Box::from_raw(data_ptr);
}

unsafe extern "C" fn rt_thread_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let data_ptr = *ptr.offset(0) as *mut ThreadData;
    if data_ptr.is_null() {
        return;
    }

    *ptr.offset(0) = 0;
    *ptr.offset(1) = 0;
    let mut data = Box::from_raw(data_ptr);
    if !data.started {
        rt_release_thread_cb_slot(data.cb_slot, &data.slot_live);
    } else {
        let finished = data
            .handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true);
        if finished {
            rt_release_thread_cb_slot(data.cb_slot, &data.slot_live);
        }
        let _ = data.handle.take();
    }
}

unsafe extern "C" fn rt_timeout_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let id = *ptr.offset(0);
    if id <= 0 {
        return;
    }
    *ptr.offset(0) = 0;
    rt_clearTimeout(id);
}

unsafe extern "C" fn rt_interval_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let id = *ptr.offset(0);
    if id <= 0 {
        return;
    }
    *ptr.offset(0) = 0;
    rt_clearInterval(id);
}

unsafe extern "C" fn rt_tcp_stream_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let id = *ptr.offset(0);
    if id <= 0 {
        return;
    }
    *ptr.offset(0) = 0;
    let _ = rt_net_close(id);
}

unsafe extern "C" fn rt_tcp_listener_object_finalizer(obj: i64) {
    let ptr = rt_obj_ptr(obj);
    if ptr.is_null() {
        return;
    }
    let id = *ptr.offset(0);
    if id <= 0 {
        return;
    }
    *ptr.offset(0) = 0;
    let _ = rt_net_close_listener(id);
}

// --- Promises ---

#[no_mangle]
pub unsafe extern "C" fn rt_move_member(id: i64, index: i32) -> i64 {
    if id < HEAP_OFFSET {
        return 0;
    }
    let body = (id - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    if tag == TAG_ARRAY {
        let header = rt_get_header(body);
        let len = (*header).length as i64;
        let data = body as *mut i64;
        if index >= 0 && (index as i64) < len {
            let val = *data.offset(index as isize);
            *data.offset(index as isize) = 0; // Move out
            return val;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_instanceof(obj: i64, _class_name: i64) -> i64 {
    let is_stack = obj >= STACK_OFFSET && obj < HEAP_OFFSET;
    let is_heap = obj >= HEAP_OFFSET && rt_is_gc_ptr((obj - HEAP_OFFSET) as *mut u8);
    if !is_heap && !is_stack {
        return 0;
    }
    let body = if is_heap {
        (obj - HEAP_OFFSET) as *mut u8
    } else {
        (obj - STACK_OFFSET) as *mut u8
    };
    let header = rt_get_header(body);
    let tag = (*header).type_id as i64;
    // Return true for objects, arrays, and user-defined classes (tag >= 12)
    if tag == TAG_OBJECT || tag == TAG_ARRAY || tag >= 12 {
        return 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_eq(a: i64, b: i64) -> i64 {
    if a == b {
        return BOOL_TRUE;
    }
    if a == 0 || b == 0 {
        return BOOL_FALSE;
    }
    let a_is_gc = if a >= HEAP_OFFSET {
        rt_is_gc_ptr((a - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };
    let b_is_gc = if b >= HEAP_OFFSET {
        rt_is_gc_ptr((b - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };

    if a_is_gc && b_is_gc {
        let body_a = (a - HEAP_OFFSET) as *mut u8;
        let body_b = (b - HEAP_OFFSET) as *mut u8;
        let header_a = rt_get_header(body_a);
        let header_b = rt_get_header(body_b);
        let tag_a = (*header_a).type_id as i64;
        let tag_b = (*header_b).type_id as i64;

        if tag_a == TAG_STRING && tag_b == TAG_STRING {
            return if rt_str_equals(a, b) != 0 {
                BOOL_TRUE
            } else {
                BOOL_FALSE
            };
        }

        let a_num =
            tag_a == TAG_FLOAT || tag_a == TAG_INT || tag_a == TAG_CHAR || tag_a == TAG_BOOLEAN;
        let b_num =
            tag_b == TAG_FLOAT || tag_b == TAG_INT || tag_b == TAG_CHAR || tag_b == TAG_BOOLEAN;
        if a_num && b_num {
            return if rt_to_number(a) == rt_to_number(b) {
                BOOL_TRUE
            } else {
                BOOL_FALSE
            };
        }

        return BOOL_FALSE;
    }

    if a_is_gc {
        let tag = (*rt_get_header((a - HEAP_OFFSET) as *mut u8)).type_id as i64;
        if tag == TAG_STRING {
            return BOOL_FALSE;
        }
    }
    if b_is_gc {
        let tag = (*rt_get_header((b - HEAP_OFFSET) as *mut u8)).type_id as i64;
        if tag == TAG_STRING {
            return BOOL_FALSE;
        }
    }

    if rt_to_number(a) == rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_equal(a: i64, b: i64) -> i64 {
    if a == b {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_strict_ne(a: i64, b: i64) -> i64 {
    if a != b {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_print(val: i64) {
    let mut v = val;
    let mut s_id = 0i64;
    rt_push_root(&mut v);
    rt_push_root(&mut s_id);

    s_id = rt_to_string(v);

    if let Some((data, len)) = get_str_parts(s_id) {
        let mut bytes_written = 0;
        while bytes_written < len {
            let chunk = (len - bytes_written).min(1024) as usize;
            let s = std::slice::from_raw_parts(data.add(bytes_written as usize), chunk);
            let out = std::io::stdout();
            let mut handle = out.lock();
            use std::io::Write;
            let _ = handle.write_all(s);
            bytes_written += chunk as i64;
        }
    }
    println!();

    rt_pop_roots(2);
}

#[no_mangle]
pub unsafe extern "C" fn rt_print_string_array(args: i64) {
    let args = rt_resolve_array_id(args);
    if (args as u64) < (HEAP_OFFSET as u64) {
        return;
    }

    let body = (args - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let len = (*header).length as i64;
    let data = body as *const i64;
    let out = std::io::stdout();
    let mut handle = out.lock();
    use std::io::Write;

    for i in 0..len {
        let s_id = *data.add(i as usize);
        if let Some((data, chunk_len)) = get_str_parts(s_id) {
            let bytes = std::slice::from_raw_parts(data, chunk_len as usize);
            let _ = handle.write_all(bytes);
        }
        if i + 1 < len {
            let _ = handle.write_all(b" ");
        }
    }
    let _ = handle.write_all(b"\n");
}

#[no_mangle]
pub unsafe extern "C" fn rt_typeof(val: i64) -> i64 {
    let is_stack = val >= STACK_OFFSET && val < HEAP_OFFSET;
    let is_heap = if val >= HEAP_OFFSET {
        rt_is_gc_ptr((val - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };

    if !is_heap && !is_stack {
        if val == 0 {
            return rt_string_from_c_str("None\0".as_ptr() as *const _);
        } else if looks_like_unboxed_float_bits(val) {
            return rt_string_from_c_str("float\0".as_ptr() as *const _);
        } else {
            return rt_string_from_c_str("int\0".as_ptr() as *const _);
        }
    } else {
        let body = if is_heap {
            (val - HEAP_OFFSET) as *mut u8
        } else {
            (val - STACK_OFFSET) as *mut u8
        };
        let header = rt_get_header(body);
        let tag = (*header).type_id as i64;
        if tag == TAG_STRING {
            return rt_string_from_c_str("string\0".as_ptr() as *const _);
        } else if tag == TAG_FUNCTION {
            return rt_string_from_c_str("function\0".as_ptr() as *const _);
        } else if tag == TAG_ARRAY {
            return rt_string_from_c_str("array\0".as_ptr() as *const _);
        } else if tag == TAG_OBJECT {
            return rt_string_from_c_str("object\0".as_ptr() as *const _);
        } else if tag == TAG_BOOLEAN {
            return rt_string_from_c_str("bool\0".as_ptr() as *const _);
        } else if tag == TAG_FLOAT {
            return rt_string_from_c_str("float\0".as_ptr() as *const _);
        } else if tag == TAG_INT {
            return rt_string_from_c_str("int\0".as_ptr() as *const _);
        } else if tag == TAG_CHAR {
            return rt_string_from_c_str("char\0".as_ptr() as *const _);
        } else if tag == TAG_PROMISE {
            return rt_string_from_c_str("object\0".as_ptr() as *const _);
        } else {
            return rt_string_from_c_str("object\0".as_ptr() as *const _);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_sizeof(val: i64) -> i64 {
    if val < HEAP_OFFSET {
        return 8; // Non-GC primitive
    }
    let body_ptr = (val - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body_ptr) {
        return 8; // Likely a 64-bit float or other non-pointer
    }
    let header = rt_get_header(body_ptr);
    let type_id = (*header).type_id;

    let header_size = 24i64;
    let mut body_size = 8i64; // Default boxed size

    if type_id == TAG_STRING as u16 {
        body_size = ((*header).length as i64 + 1 + 7) & !7;
    } else if type_id == TAG_ARRAY as u16 {
        let elem_size = ((*header).flags & 0xFF) as i64;
        body_size = ((*header).capacity as i64 * elem_size + 7) & !7;
    } else if type_id == TAG_OBJECT as u16 {
        // Map layout: fixed body 40 + [keys_ptr, values_ptr] arrays
        let cap = *(body_ptr.add(8) as *const i64);
        body_size = 40 + (cap * 16); // 2 arrays of 'cap' 8-byte elements
    } else if type_id == TAG_FUNCTION as u16 {
        body_size = 32; // rt_closure_new allocates 32
    } else if type_id == TAG_PROMISE as u16 {
        body_size = PROMISE_BODY_SIZE as i64;
    }

    header_size + body_size
}

#[no_mangle]
pub unsafe extern "C" fn rt_await(p: i64) -> i64 {
    if p < HEAP_OFFSET {
        return p;
    }
    let mut v_p = p;
    rt_push_root(&mut v_p);

    let mut body = (v_p - HEAP_OFFSET) as *mut i64;
    let header = rt_get_header(body as *mut u8);
    if (*header).type_id != TAG_PROMISE as u16 {
        rt_pop_roots(1);
        return v_p;
    }

    // Pump the event loop until the promise is resolved/rejected
    while *body.offset(0) == 0 {
        let has_more = event_loop::tejx_run_event_loop_step();
        // Re-resolve body in case p moved during GC
        body = (v_p - HEAP_OFFSET) as *mut i64;

        if !has_more && *body.offset(0) == 0 {
            rt_throw_runtime_error(
                "RuntimeError: Deadlock detected. Awaited Promise will never resolve.",
            );
        }
    }

    let state = *body.offset(0);
    let res = *body.offset(1);
    rt_pop_roots(1);
    if state == 2 {
        crate::event_loop::tejx_throw(res);
        std::hint::unreachable_unchecked();
    }
    res
}

#[no_mangle]
pub unsafe extern "C" fn rt_to_slice(val: i64) -> Slice {
    if val < HEAP_OFFSET {
        return Slice { ptr: 0, len: 0 };
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    let header = rt_get_header(body);
    let len = (*header).length as i64;

    // For contiguous arrays and strings, the data starts directly at the body
    Slice {
        ptr: body as i64,
        len,
    }
}

#[path = "../event_loop.rs"]
pub mod event_loop;
pub use event_loop::*;

#[no_mangle]
pub unsafe extern "C" fn rt_box_boolean(b: i64) -> i64 {
    let body = gc_allocate(8);
    let header = rt_get_header(body);
    (*header).type_id = TAG_BOOLEAN as u16;
    (*header).length = 0;
    *(body as *mut i64) = if b != 0 { 1 } else { 0 };
    (body as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_int(n: i64) -> i64 {
    let body = gc_allocate(8);
    let header = rt_get_header(body);
    (*header).type_id = TAG_INT as u16;
    (*header).length = 0;
    *(body as *mut i64) = n;
    (body as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_float(f: f64) -> i64 {
    let body = gc_allocate(8);
    let header = rt_get_header(body);
    (*header).type_id = TAG_FLOAT as u16;
    (*header).length = 0;
    *(body as *mut f64) = f;
    (body as i64) + HEAP_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_box_number(f: f64) -> i64 {
    rt_box_float(f)
}

#[no_mangle]
pub unsafe extern "C" fn rt_unbox_int(v: i64) -> i64 {
    if v >= HEAP_OFFSET {
        let body = (v - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_INT as u16 {
            return *(body as *const i64);
        }
    }
    v
}

#[no_mangle]
pub unsafe extern "C" fn rt_unbox_number(v: i64) -> f64 {
    rt_to_number(v)
}

#[no_mangle]
pub unsafe extern "C" fn rt_add(a: i64, b: i64) -> i64 {
    let a_is_str = if a >= HEAP_OFFSET {
        let body = (a - HEAP_OFFSET) as *mut u8;
        rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_STRING as u16
    } else {
        false
    };

    let b_is_str = if b >= HEAP_OFFSET {
        let body = (b - HEAP_OFFSET) as *mut u8;
        rt_is_gc_ptr(body) && (*rt_get_header(body)).type_id == TAG_STRING as u16
    } else {
        false
    };

    if a_is_str || b_is_str {
        return rt_str_concat_v2(a, b);
    }
    let res = rt_to_number(a) + rt_to_number(b);
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_sub(a: i64, b: i64) -> i64 {
    let res = rt_to_number(a) - rt_to_number(b);
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_mul(a: i64, b: i64) -> i64 {
    let res = rt_to_number(a) * rt_to_number(b);
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_div(a: i64, b: i64) -> i64 {
    let fb = rt_to_number(b);
    if fb == 0.0 {
        rt_div_zero_error(a, b);
    }
    let res = rt_to_number(a) / fb;
    res.to_bits() as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_lt(a: i64, b: i64) -> i64 {
    if rt_to_number(a) < rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_gt(a: i64, b: i64) -> i64 {
    if rt_to_number(a) > rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_le(a: i64, b: i64) -> i64 {
    if rt_to_number(a) <= rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_ge(a: i64, b: i64) -> i64 {
    if rt_to_number(a) >= rt_to_number(b) {
        BOOL_TRUE
    } else {
        BOOL_FALSE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_ne(a: i64, b: i64) -> i64 {
    if rt_eq(a, b) == BOOL_TRUE {
        BOOL_FALSE
    } else {
        BOOL_TRUE
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_object_new() -> i64 {
    let body_ptr = gc_allocate(32);
    let header = rt_get_header(body_ptr);
    (*header).type_id = TAG_OBJECT as u16;

    let mut obj_id = (body_ptr as i64) + HEAP_OFFSET;
    rt_push_root(&mut obj_id);

    let mut keys = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
    rt_push_root(&mut keys);
    let mut values = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
    rt_push_root(&mut values);

    let body_ptr = (obj_id - HEAP_OFFSET) as *mut u8;

    *(body_ptr.offset(OBJECT_SIZE_OFFSET) as *mut i64) = 0;
    *(body_ptr.offset(OBJECT_CAP_OFFSET) as *mut i64) = 0;
    rt_store_ref_slot(obj_id, body_ptr.offset(OBJECT_KEYS_OFFSET) as *mut i64, keys);
    rt_store_ref_slot(
        obj_id,
        body_ptr.offset(OBJECT_VALUES_OFFSET) as *mut i64,
        values,
    );

    rt_pop_roots(3);
    obj_id
}

pub unsafe fn rt_is_object(val: i64) -> bool {
    let body = rt_object_body_ptr(val);
    if body.is_null() {
        return false;
    }
    (*rt_get_header(body)).type_id == TAG_OBJECT as u16
}

#[inline]
unsafe fn rt_object_body_ptr(obj: i64) -> *mut u8 {
    if obj >= HEAP_OFFSET {
        let body = (obj - HEAP_OFFSET) as *mut u8;
        if rt_is_gc_ptr(body) {
            return body;
        }
        return std::ptr::null_mut();
    }
    if obj >= STACK_OFFSET {
        return (obj - STACK_OFFSET) as *mut u8;
    }
    std::ptr::null_mut()
}

pub unsafe fn rt_object_keys_array(obj: i64) -> i64 {
    let body = rt_object_body_ptr(obj);
    if body.is_null() {
        return 0;
    }
    *(body.offset(OBJECT_KEYS_OFFSET) as *const i64)
}

pub unsafe fn rt_object_values_array(obj: i64) -> i64 {
    let body = rt_object_body_ptr(obj);
    if body.is_null() {
        return 0;
    }
    *(body.offset(OBJECT_VALUES_OFFSET) as *const i64)
}

pub unsafe fn rt_object_set_arrays(obj: i64, keys: i64, values: i64) {
    let body = rt_object_body_ptr(obj);
    if body.is_null() {
        return;
    }
    rt_store_ref_slot(obj, body.offset(OBJECT_KEYS_OFFSET) as *mut i64, keys);
    rt_store_ref_slot(obj, body.offset(OBJECT_VALUES_OFFSET) as *mut i64, values);
}

pub unsafe fn rt_object_refresh_meta(obj: i64) {
    let body = rt_object_body_ptr(obj);
    if body.is_null() {
        return;
    }
    let keys = rt_object_keys_array(obj);
    let len = rt_len(keys);
    *(body.offset(OBJECT_SIZE_OFFSET) as *mut i64) = len;
    *(body.offset(OBJECT_CAP_OFFSET) as *mut i64) = len;
}

pub unsafe fn rt_object_find_key_index(obj: i64, key: i64) -> i64 {
    let keys = rt_object_keys_array(obj);
    let len = rt_len(keys);
    let mut i = 0;
    while i < len {
        let existing = rt_array_get_fast(keys, i);
        if existing == key || rt_str_equals(existing, key) != 0 {
            return i;
        }
        i += 1;
    }
    -1
}

unsafe fn rt_property_key_for_error(key: i64) -> String {
    if let Some(name) = i64_to_rust_str(key) {
        format!("'{}'", name)
    } else {
        format!("[{}]", runtime_inline_value_to_string(key))
    }
}

unsafe fn rt_throw_property_null_error(action: &str, key: i64) -> ! {
    let msg = format!(
        "RuntimeError: Null pointer dereference while {} property {}",
        action,
        rt_property_key_for_error(key)
    );
    rt_throw_runtime_error(&msg);
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_property(obj: i64, key: i64) -> i64 {
    if (obj as u64) < (STACK_OFFSET as u64) {
        rt_throw_property_null_error("reading", key);
    }
    if !rt_is_object(obj) {
        return 0;
    }
    let idx = rt_object_find_key_index(obj, key);
    if idx < 0 {
        return 0;
    }
    rt_array_get_fast(rt_object_values_array(obj), idx)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_property_traced(obj: i64, key: i64, file_ptr: i64, line: i64) -> i64 {
    if (obj as u64) < (STACK_OFFSET as u64) {
        runtime_set_current_location(file_ptr, line);
        rt_throw_property_null_error("reading", key);
    }
    rt_get_property(obj, key)
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_property(obj: i64, key: i64, val: i64) {
    if (obj as u64) < (STACK_OFFSET as u64) {
        rt_throw_property_null_error("writing", key);
    }
    if !rt_is_object(obj) {
        return;
    }

    let mut obj = obj;
    let mut key = key;
    let mut val = val;
    rt_push_root(&mut obj);
    rt_push_root(&mut key);
    rt_push_root(&mut val);

    let mut keys = rt_object_keys_array(obj);
    let mut values = rt_object_values_array(obj);
    rt_push_root(&mut keys);
    rt_push_root(&mut values);

    let idx = rt_object_find_key_index(obj, key);
    if idx >= 0 {
        rt_array_set_fast(values, idx, val);
    } else {
        keys = rt_array_push(keys, key);
        values = rt_array_push(values, val);
        rt_object_set_arrays(obj, keys, values);
        rt_object_refresh_meta(obj);
    }

    rt_pop_roots(5);
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_property_traced(
    obj: i64,
    key: i64,
    val: i64,
    file_ptr: i64,
    line: i64,
) {
    if (obj as u64) < (STACK_OFFSET as u64) {
        runtime_set_current_location(file_ptr, line);
        rt_throw_property_null_error("writing", key);
    }
    rt_set_property(obj, key, val);
}

#[no_mangle]
pub unsafe extern "C" fn rt_obj_keys(obj: i64) -> i64 {
    rt_Object_keys(obj)
}
#[no_mangle]
pub unsafe extern "C" fn rt_tag_of(val: i64) -> i64 {
    let is_stack = val >= STACK_OFFSET && val < HEAP_OFFSET;
    let is_heap = if val >= HEAP_OFFSET {
        rt_is_gc_ptr((val - HEAP_OFFSET) as *mut u8)
    } else {
        false
    };
    if !is_heap && !is_stack {
        return -1;
    }
    let body = if is_heap {
        (val - HEAP_OFFSET) as *mut u8
    } else {
        (val - STACK_OFFSET) as *mut u8
    };
    (*rt_get_header(body)).type_id as i64
}

fn rt_mix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

pub unsafe fn rt_hash_bytes(data: *const u8, len: i64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < len {
        h ^= *data.add(i as usize) as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    rt_mix64(h)
}
#[no_mangle]
pub unsafe extern "C" fn rt_object_merge(obj: i64, other: i64) -> i64 {
    rt_Object_assign(obj, other)
}

#[no_mangle]
pub unsafe extern "C" fn rt_optional_chain(obj: i64, _op: *const u8) -> i64 {
    if obj < HEAP_OFFSET {
        return 0;
    }
    obj
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_rust_string(val: i64) -> String {
        unsafe { i64_to_rust_str(val).expect("runtime string") }
    }

    #[test]
    fn runtime_exception_report_includes_location_stack_and_hint() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            runtime_restore_call_stack(0);
            clear_stored_exception_traces();

            let temp_dir = std::env::temp_dir();
            let file = temp_dir.join("tejx_runtime_report_fixture.tx");
            std::fs::write(
                &file,
                "function div(a: int, b: int): int {\n    return a / b;\n}\n",
            )
            .unwrap();

            let function = std::ffi::CString::new("f_div").unwrap();
            let file_c = std::ffi::CString::new(file.to_string_lossy().into_owned()).unwrap();
            rt_enter_frame(
                function.as_ptr() as usize as i64,
                file_c.as_ptr() as usize as i64,
                1,
            );
            rt_set_location(file_c.as_ptr() as usize as i64, 2);

            let exception = new_string_from_rust_str("RuntimeError: Division by zero");
            let report = render_runtime_exception_report("UnhandledException", exception);

            assert!(report.contains("RuntimeError: Division by zero"));
            assert!(report.contains("Division by zero"));
            assert!(report.contains("Hint: Ensure the right-hand side"));
            assert!(report.contains("return a / b;"));
            assert!(report.contains("Stack trace:"));
            assert!(report.contains("at div"));
            assert!(report.contains("div"));

            runtime_restore_call_stack(0);
            clear_stored_exception_traces();
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    fn stored_exception_report_for_print_keeps_throw_stack() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            runtime_restore_call_stack(0);
            clear_stored_exception_traces();

            let temp_dir = std::env::temp_dir();
            let file = temp_dir.join("tejx_print_exception_fixture.tx");
            std::fs::write(&file, "function fail() {\n    throw \"boom\";\n}\n").unwrap();

            let function = std::ffi::CString::new("f_fail").unwrap();
            let file_c = std::ffi::CString::new(file.to_string_lossy().into_owned()).unwrap();
            rt_enter_frame(
                function.as_ptr() as usize as i64,
                file_c.as_ptr() as usize as i64,
                1,
            );
            rt_set_location(file_c.as_ptr() as usize as i64, 2);

            let exception = new_string_from_rust_str("boom");
            remember_exception_trace(exception);
            runtime_restore_call_stack(0);

            let report = to_rust_string(rt_exception_report_for_print(exception));
            assert!(report.contains("Exception: boom"));
            assert!(report.contains("Stack trace:"));
            assert!(report.contains("at fail"));
            assert!(report.contains("throw \"boom\";"));

            clear_stored_exception_traces();
            let _ = std::fs::remove_file(file);
        }
    }

    #[test]
    fn prints_none_and_bools_correctly() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            assert_eq!(to_rust_string(rt_to_string(0)), "None");
            assert_eq!(to_rust_string(rt_typeof(0)), "None");

            let boxed_false = rt_box_boolean(0);
            let boxed_true = rt_box_boolean(1);
            let boxed_zero = rt_box_int(0);

            assert_eq!(to_rust_string(rt_to_string(boxed_false)), "false");
            assert_eq!(to_rust_string(rt_to_string(boxed_true)), "true");
            assert_eq!(to_rust_string(rt_typeof(boxed_false)), "bool");
            assert_eq!(to_rust_string(rt_typeof(boxed_true)), "bool");

            assert_eq!(to_rust_string(rt_to_string(boxed_zero)), "0");
            assert_eq!(to_rust_string(rt_typeof(boxed_zero)), "int");
        }
    }

    #[test]
    fn stringifies_plain_objects_with_values() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut obj = rt_object_new();
            rt_push_root(&mut obj);

            let key_message = rt_string_from_c_str_const("message\0".as_ptr() as *const _);
            let value_message = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            let key_code = rt_string_from_c_str_const("code\0".as_ptr() as *const _);
            let value_code = rt_box_int(7);

            rt_set_property(obj, key_message, value_message);
            rt_set_property(obj, key_code, value_code);

            assert_eq!(
                to_rust_string(rt_to_string(obj)),
                "{ message: \"boom\", code: 7 }"
            );

            rt_pop_roots(1);
        }
    }

    #[test]
    fn stringifies_registered_class_fields() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let offsets = [0usize];
            rt_register_type(200, 16, 1, offsets.as_ptr(), None);

            let field_offsets = [0i64, 8i64];
            let field_kinds = [FIELD_KIND_REF, FIELD_KIND_BOOL];
            let field_names = [
                "message\0".as_ptr() as usize as i64,
                "active\0".as_ptr() as usize as i64,
            ];
            rt_register_type_info(
                200,
                "Sample\0".as_ptr() as usize as i64,
                2,
                field_offsets.as_ptr(),
                field_kinds.as_ptr(),
                field_names.as_ptr(),
            );

            let mut obj = rt_class_new(200, 16, 1, std::ptr::null(), 0);
            rt_push_root(&mut obj);

            let message = rt_string_from_c_str_const("boom\0".as_ptr() as *const _);
            let body = (obj - HEAP_OFFSET) as *mut u8;
            *(body as *mut i64) = message;
            *(body.add(8) as *mut i8) = 1;

            assert_eq!(
                to_rust_string(rt_to_string(obj)),
                "Sample { message: \"boom\", active: true }"
            );

            rt_pop_roots(1);
        }
    }

    #[test]
    fn string_case_helpers_reload_source_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut source = new_string_from_bytes(b"MiXeD".as_ptr(), 5);
            rt_push_root(&mut source);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let upper = crate::string::rt_String_toUpperCase(source);
            assert_eq!(to_rust_string(upper), "MIXED");

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let lower = crate::string::rt_String_toLowerCase(source);
            assert_eq!(to_rust_string(lower), "mixed");

            rt_pop_roots(1);
        }
    }

    #[test]
    fn string_append_local_reloads_sources_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut lhs = new_string_from_bytes(b"left".as_ptr(), 4);
            let mut rhs = new_string_from_bytes(b"-right".as_ptr(), 6);
            rt_push_root(&mut lhs);
            rt_push_root(&mut rhs);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let appended = crate::string::rt_str_append_local(lhs, rhs);
            assert_eq!(to_rust_string(appended), "left-right");

            rt_pop_roots(2);
        }
    }

    #[test]
    fn rust_string_helper_preserves_exact_length() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let source = String::from("alpha-beta-gamma");
            let value = new_string_from_rust_str(&source);
            assert_eq!(to_rust_string(value), source);
            assert_eq!(rt_len(value), source.len() as i64);
        }
    }

    #[test]
    fn string_helpers_preserve_embedded_null_bytes() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let bytes = [b'a', 0, b'b'];
            let mut source = new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64);
            rt_push_root(&mut source);

            let repeated = crate::string::rt_String_repeat(source, 2);
            assert_eq!(rt_len(repeated), 6);
            assert_eq!(crate::string::rt_String_charCodeAt(repeated, 0), b'a' as i32);
            assert_eq!(crate::string::rt_String_charCodeAt(repeated, 1), 0);
            assert_eq!(crate::string::rt_String_charCodeAt(repeated, 2), b'b' as i32);
            assert_eq!(crate::string::rt_String_charCodeAt(repeated, 3), b'a' as i32);
            assert_eq!(crate::string::rt_String_charCodeAt(repeated, 4), 0);
            assert_eq!(crate::string::rt_String_charCodeAt(repeated, 5), b'b' as i32);

            let nul_char = crate::string::rt_str_at(source, 1);
            assert_eq!(rt_len(nul_char), 1);
            assert_eq!(crate::string::rt_String_charCodeAt(nul_char, 0), 0);

            let mut single = new_string_from_bytes(b"a".as_ptr(), 1);
            let mut search = new_string_from_bytes(b"a".as_ptr(), 1);
            let replacement_bytes = [b'z', 0, b'y'];
            let mut replacement =
                new_string_from_bytes(replacement_bytes.as_ptr(), replacement_bytes.len() as i64);
            rt_push_root(&mut single);
            rt_push_root(&mut search);
            rt_push_root(&mut replacement);

            let replaced = crate::string::rt_String_replace(single, search, replacement);
            assert_eq!(rt_len(replaced), 3);
            assert_eq!(crate::string::rt_String_charCodeAt(replaced, 0), b'z' as i32);
            assert_eq!(crate::string::rt_String_charCodeAt(replaced, 1), 0);
            assert_eq!(crate::string::rt_String_charCodeAt(replaced, 2), b'y' as i32);

            rt_pop_roots(4);
        }
    }

    #[test]
    fn readdir_sync_preserves_exact_entry_names() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("tejx_readdir_exact_{}", stamp));
            std::fs::create_dir_all(&dir).unwrap();

            let file_name = "entry-name-without-c-terminator";
            std::fs::write(dir.join(file_name), b"x").unwrap();

            let path_string = dir.to_string_lossy().into_owned();
            let mut path_id = new_string_from_rust_str(&path_string);
            rt_push_root(&mut path_id);

            let entries = rt_fs_readdir_sync(path_id);
            assert_eq!(rt_len(entries), 1);
            assert_eq!(to_rust_string(rt_array_get_fast(entries, 0)), file_name);

            rt_pop_roots(1);
            let _ = std::fs::remove_file(dir.join(file_name));
            let _ = std::fs::remove_dir(&dir);
        }
    }

    #[test]
    fn fs_read_sync_preserves_embedded_null_bytes() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("tejx_fs_read_exact_{}", stamp));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("payload.txt");
            std::fs::write(&file, b"a\0b").unwrap();

            let path_string = file.to_string_lossy().into_owned();
            let mut path_id = new_string_from_rust_str(&path_string);
            rt_push_root(&mut path_id);

            let content = rt_fs_read_sync(path_id);
            assert_eq!(rt_len(content), 3);
            assert_eq!(crate::string::rt_String_charCodeAt(content, 0), b'a' as i32);
            assert_eq!(crate::string::rt_String_charCodeAt(content, 1), 0);
            assert_eq!(crate::string::rt_String_charCodeAt(content, 2), b'b' as i32);

            rt_pop_roots(1);
            let _ = std::fs::remove_file(&file);
            let _ = std::fs::remove_dir(&dir);
        }
    }

    #[test]
    fn clone_reloads_string_source_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut source = new_string_from_bytes(b"clone-source".as_ptr(), 12);
            rt_push_root(&mut source);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let cloned = rt_clone(source);
            assert_eq!(to_rust_string(cloned), "clone-source");

            rt_pop_roots(1);
        }
    }

    #[test]
    fn clone_reloads_array_source_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut source = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut source);
            let mut item = new_string_from_bytes(b"array-clone".as_ptr(), 11);
            rt_push_root(&mut item);
            source = rt_array_set_fast(source, 0, item);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let cloned = rt_clone(source);
            assert_eq!(to_rust_string(rt_array_get_fast(cloned, 0)), "array-clone");

            rt_pop_roots(2);
        }
    }

    #[test]
    fn object_entries_reloads_sources_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut obj = rt_object_new();
            rt_push_root(&mut obj);
            let mut key = new_string_from_bytes(b"name".as_ptr(), 4);
            let mut value = new_string_from_bytes(b"tejx".as_ptr(), 4);
            rt_push_root(&mut key);
            rt_push_root(&mut value);
            rt_set_property(obj, key, value);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let entries = crate::object::rt_Object_entries(obj);
            let first = rt_array_get_fast(entries, 0);
            assert_eq!(to_rust_string(rt_array_get_fast(first, 0)), "name");
            assert_eq!(to_rust_string(rt_array_get_fast(first, 1)), "tejx");

            rt_pop_roots(3);
        }
    }

    #[test]
    fn object_assign_reloads_sources_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut source = rt_object_new();
            let mut target = rt_object_new();
            rt_push_root(&mut source);
            rt_push_root(&mut target);
            let mut key = new_string_from_bytes(b"city".as_ptr(), 4);
            let mut value = new_string_from_bytes(b"pune".as_ptr(), 4);
            rt_push_root(&mut key);
            rt_push_root(&mut value);
            rt_set_property(source, key, value);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            crate::object::rt_Object_assign(target, source);
            assert_eq!(to_rust_string(rt_get_property(target, key)), "pune");

            rt_pop_roots(4);
        }
    }

    #[test]
    fn bytes_from_string_reloads_source_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut source = new_string_from_bytes(b"ABC".as_ptr(), 3);
            rt_push_root(&mut source);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);
            let bytes = crate::binary::rt_bytes_from_string(source);
            assert_eq!(rt_array_get_fast(bytes, 0), 65);
            assert_eq!(rt_array_get_fast(bytes, 1), 66);
            assert_eq!(rt_array_get_fast(bytes, 2), 67);

            rt_pop_roots(1);
        }
    }

    #[test]
    fn raw_atomic_helper_releases_native_state_on_free() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            let atom = rt_atomic_new(7);
            let body = (atom - HEAP_OFFSET) as *mut u8;

            assert!(RAW_ATOMIC_OBJECTS.lock().unwrap().contains_key(&(body as usize)));
            rt_free(atom);
            assert!(!RAW_ATOMIC_OBJECTS.lock().unwrap().contains_key(&(body as usize)));
        }
    }

    #[test]
    fn promise_size_matches_allocated_layout() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            let promise = rt_promise_new();
            assert_eq!(rt_sizeof(promise), 24 + PROMISE_BODY_SIZE as i64);
        }
    }

    #[test]
    fn const_string_interning_reuses_identical_bytes_across_addresses() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let one = std::ffi::CString::new("duplicate").unwrap();
            let two = std::ffi::CString::new("duplicate").unwrap();

            let a = rt_string_from_c_str_const(one.as_ptr());
            let b = rt_string_from_c_str_const(two.as_ptr());

            assert_eq!(a, b);
            assert!(!gc::rt_is_los_ptr(rt_get_header((a - HEAP_OFFSET) as *mut u8) as *mut u8));
        }
    }

    #[test]
    fn large_object_allocations_trigger_collection_before_overflow() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let initial_los_count = gc::LOS_COUNT;
            let bytes = vec![b'x'; gc::LARGE_OBJECT_THRESHOLD + 1024];

            let mut keeper = new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64);
            rt_push_root(&mut keeper);

            let baseline_count = gc::LOS_COUNT;
            for _ in 0..80 {
                let _ = new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64);
            }

            assert!(
                gc::LOS_COUNT < baseline_count + 32,
                "dead large objects should be reclaimed before LOS keeps growing"
            );

            rt_pop_roots(1);
            gc::major_gc();

            assert!(
                gc::LOS_COUNT <= initial_los_count,
                "explicit major GC should release unrooted large objects"
            );
        }
    }

    #[test]
    fn old_objects_dirty_cards_when_property_growth_relinks_arrays() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut obj = rt_object_new();
            rt_push_root(&mut obj);
            let mut pad = rt_Array_new(256, 8);
            rt_push_root(&mut pad);

            gc::minor_gc();
            gc::minor_gc();

            let obj_body = (obj - HEAP_OFFSET) as *mut u8;
            assert!(obj_body >= gc::OLD_START && obj_body < gc::OLD_TOP);

            let card_idx = (obj_body as usize - gc::OLD_START as usize) >> gc::CARD_SHIFT;

            let key0 = rt_string_from_c_str_const(b"k0\0".as_ptr() as *const _);
            let key1 = rt_string_from_c_str_const(b"k1\0".as_ptr() as *const _);
            let key2 = rt_string_from_c_str_const(b"k2\0".as_ptr() as *const _);
            let key3 = rt_string_from_c_str_const(b"k3\0".as_ptr() as *const _);
            let key4 = rt_string_from_c_str_const(b"k4\0".as_ptr() as *const _);

            rt_set_property(obj, key0, rt_box_int(0));
            rt_set_property(obj, key1, rt_box_int(1));
            rt_set_property(obj, key2, rt_box_int(2));
            rt_set_property(obj, key3, rt_box_int(3));

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                0,
                "filling old key/value arrays in place should not dirty the owning object card"
            );

            rt_set_property(obj, key4, rt_box_int(4));

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old objects must dirty their card when property growth swaps in young arrays"
            );

            rt_pop_roots(2);
        }
    }

    #[test]
    fn old_shared_queues_dirty_cards_when_backing_arrays_change() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let offsets = [0usize];
            rt_register_type(203, 8, 1, offsets.as_ptr(), None);

            let mut queue = rt_class_new(203, 8, 1, std::ptr::null(), 0);
            rt_push_root(&mut queue);
            let mut pad = rt_Array_new(256, 8);
            rt_push_root(&mut pad);
            crate::queue::rt_SharedQueue_constructor(queue);

            gc::minor_gc();
            gc::minor_gc();

            let queue_body = (queue - HEAP_OFFSET) as *mut u8;
            assert!(queue_body >= gc::OLD_START && queue_body < gc::OLD_TOP);

            let card_idx = (queue_body as usize - gc::OLD_START as usize) >> gc::CARD_SHIFT;
            *gc::CARD_TABLE.add(card_idx) = 0;

            crate::queue::rt_SharedQueue_enqueue(queue, rt_box_int(7));

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old shared queues must dirty their card when enqueue swaps in a young backing array"
            );

            rt_pop_roots(2);
        }
    }

    #[test]
    fn shared_queue_constructor_reloads_owner_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let offsets = [0usize];
            rt_register_type(204, 8, 1, offsets.as_ptr(), None);

            let mut queue = rt_class_new(204, 8, 1, std::ptr::null(), 0);
            rt_push_root(&mut queue);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            crate::queue::rt_SharedQueue_constructor(queue);

            let queue_body = (queue - HEAP_OFFSET) as *mut i64;
            assert_ne!(
                *queue_body, 0,
                "SharedQueue constructor must reload the owner body after a GC-triggering allocation"
            );
            assert_eq!(crate::queue::rt_SharedQueue_size(queue), 0);

            rt_pop_roots(1);
        }
    }

    #[test]
    fn shared_queue_enqueue_roots_owner_and_value_across_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let offsets = [0usize];
            rt_register_type(205, 8, 1, offsets.as_ptr(), None);

            let mut queue = rt_class_new(205, 8, 1, std::ptr::null(), 0);
            rt_push_root(&mut queue);
            crate::queue::rt_SharedQueue_constructor(queue);

            let payload = b"queued-through-gc";
            let value = new_string_from_bytes(payload.as_ptr(), payload.len() as i64);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            crate::queue::rt_SharedQueue_enqueue(queue, value);

            assert_eq!(crate::queue::rt_SharedQueue_size(queue), 1);
            let dequeued = crate::queue::rt_SharedQueue_dequeue(queue);
            assert_eq!(
                to_rust_string(dequeued),
                "queued-through-gc",
                "SharedQueue enqueue must root heap values and update the moved backing array"
            );

            rt_pop_roots(1);
        }
    }

    #[test]
    fn array_constructor_v2_reloads_source_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut source = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut source);
            let mut value = new_string_from_bytes(b"clone-me".as_ptr(), 8);
            rt_push_root(&mut value);
            rt_array_set_fast(source, 0, value);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            let cloned = rt_Array_constructor_v2(0, source, 8, ARRAY_FLAG_PTR);
            assert_eq!(to_rust_string(rt_array_get_fast(cloned, 0)), "clone-me");

            rt_pop_roots(2);
        }
    }

    #[test]
    fn array_set_fast_roots_value_across_gc_growth() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut arr = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut arr);
            let mut value = new_string_from_bytes(b"set-after-gc".as_ptr(), 12);
            rt_push_root(&mut value);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            arr = rt_array_set_fast(arr, 3, value);
            assert_eq!(to_rust_string(rt_array_get_fast(arr, 3)), "set-after-gc");

            rt_pop_roots(2);
        }
    }

    #[test]
    fn array_splice_roots_items_array_across_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut arr = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut arr);
            let mut first = new_string_from_bytes(b"first".as_ptr(), 5);
            rt_push_root(&mut first);
            arr = rt_array_set_fast(arr, 0, first);

            let mut items = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut items);
            let mut inserted = new_string_from_bytes(b"splice-gc".as_ptr(), 9);
            rt_push_root(&mut inserted);
            items = rt_array_set_fast(items, 0, inserted);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            arr = rt_array_splice(arr, 1, 0, items);
            assert_eq!(to_rust_string(rt_array_get_fast(arr, 1)), "splice-gc");

            rt_pop_roots(4);
        }
    }

    #[test]
    fn array_concat_reloads_sources_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut left = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut left);
            let mut left_value = new_string_from_bytes(b"left".as_ptr(), 4);
            rt_push_root(&mut left_value);
            left = rt_array_set_fast(left, 0, left_value);

            let mut right = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut right);
            let mut right_value = new_string_from_bytes(b"right".as_ptr(), 5);
            rt_push_root(&mut right_value);
            right = rt_array_set_fast(right, 0, right_value);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            let combined = crate::array::rt_array_concat(left, right);
            assert_eq!(to_rust_string(rt_array_get_fast(combined, 0)), "left");
            assert_eq!(to_rust_string(rt_array_get_fast(combined, 1)), "right");

            rt_pop_roots(4);
        }
    }

    #[test]
    fn array_join_reloads_separator_after_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut arr = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut arr);
            arr = rt_array_set_fast(arr, 0, 10);
            arr = rt_array_set_fast(arr, 1, 20);
            arr = rt_array_set_fast(arr, 2, 30);

            let mut sep = new_string_from_bytes(b"::".as_ptr(), 2);
            rt_push_root(&mut sep);

            gc::rt_clear_tlab();
            gc::EDEN_TOP.store(gc::EDEN_END, Ordering::SeqCst);

            let joined = crate::array::rt_array_join(arr, sep);
            assert_eq!(to_rust_string(joined), "10::20::30");

            rt_pop_roots(2);
        }
    }

    #[test]
    fn old_arrays_keep_dirty_cards_while_they_still_point_to_young_survivors() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut arr = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut arr);

            gc::minor_gc();
            gc::minor_gc();

            let arr_body = (arr - HEAP_OFFSET) as *mut u8;
            assert!(arr_body >= gc::OLD_START && arr_body < gc::OLD_TOP);

            let card_idx = (arr_body as usize - gc::OLD_START as usize) >> gc::CARD_SHIFT;
            *gc::CARD_TABLE.add(card_idx) = 0;

            let mut child = rt_box_int(7);
            rt_push_root(&mut child);
            assert!(gc::in_young_gen((child - HEAP_OFFSET) as *mut u8));
            rt_array_set_fast(arr, 0, child);
            assert_eq!(*gc::CARD_TABLE.add(card_idx), 1);

            rt_pop_roots(1);

            gc::minor_gc();
            let moved_child = rt_array_get_fast(arr, 0);
            assert!(gc::in_young_gen((moved_child - HEAP_OFFSET) as *mut u8));
            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old arrays must stay dirty across minor GC while they still reference a young survivor"
            );

            gc::minor_gc();
            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                0,
                "card should clear once the survivor ages out of young generation"
            );

            rt_pop_roots(1);
        }
    }

    #[test]
    fn old_arrays_dirty_cards_when_fill_writes_young_values() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut arr = rt_Array_new(2, 8);
            rt_push_root(&mut arr);

            gc::minor_gc();
            gc::minor_gc();

            let arr_body = (arr - HEAP_OFFSET) as *mut u8;
            assert!(arr_body >= gc::OLD_START && arr_body < gc::OLD_TOP);

            let card_idx = (arr_body as usize - gc::OLD_START as usize) >> gc::CARD_SHIFT;
            *gc::CARD_TABLE.add(card_idx) = 0;

            let mut value = rt_box_int(9);
            rt_push_root(&mut value);
            rt_array_fill(arr, value);

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old arrays must dirty their card when fill() writes a young heap value"
            );

            rt_pop_roots(2);
        }
    }

    #[test]
    fn old_arrays_dirty_cards_when_unshift_and_splice_write_young_values() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut arr = rt_Array_constructor_v2(0, 0, 8, ARRAY_FLAG_PTR);
            rt_push_root(&mut arr);

            gc::minor_gc();
            gc::minor_gc();

            let arr_body = (arr - HEAP_OFFSET) as *mut u8;
            assert!(arr_body >= gc::OLD_START && arr_body < gc::OLD_TOP);

            let card_idx = (arr_body as usize - gc::OLD_START as usize) >> gc::CARD_SHIFT;

            *gc::CARD_TABLE.add(card_idx) = 0;
            let mut unshift_value = rt_box_int(11);
            rt_push_root(&mut unshift_value);
            arr = rt_array_unshift(arr, unshift_value);

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old arrays must dirty their card when unshift() writes a young heap value"
            );

            *gc::CARD_TABLE.add(card_idx) = 0;
            let mut items = rt_Array_new(1, 8);
            rt_push_root(&mut items);
            let mut splice_value = rt_box_int(22);
            rt_push_root(&mut splice_value);
            rt_array_set_fast(items, 0, splice_value);
            arr = rt_array_splice(arr, 1, 0, items);

            assert_eq!(
                *gc::CARD_TABLE.add(card_idx),
                1,
                "old arrays must dirty their card when splice() copies young heap values into place"
            );
            assert_eq!(rt_array_get_fast(arr, 1), splice_value);

            rt_pop_roots(4);
        }
    }

    #[test]
    fn gc_thread_registration_is_idempotent() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();
            rt_register_thread();
            rt_register_thread();

            let ctx_ptr =
                gc::MY_CONTEXT.with(|ctx| (*ctx.get()).as_mut() as *mut gc::ThreadContext);
            let registry = gc::THREAD_REGISTRY.lock().unwrap();
            let matches = registry.iter().filter(|entry| entry.0 == ctx_ptr).count();

            assert_eq!(matches, 1);
        }
    }

    #[test]
    fn gc_auto_registers_allocating_threads_and_cleans_them_up() {
        use std::sync::mpsc;

        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let baseline = gc::THREAD_REGISTRY.lock().unwrap().len();
            let (ready_tx, ready_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();

            let handle = std::thread::spawn(move || {
                let _boxed = rt_box_int(123);
                let ctx_ptr =
                    gc::MY_CONTEXT.with(|ctx| (*ctx.get()).as_mut() as *mut gc::ThreadContext);
                ready_tx.send(ctx_ptr as usize).unwrap();
                release_rx.recv().unwrap();
            });

            let child_ctx = ready_rx.recv().unwrap() as *mut gc::ThreadContext;
            {
                let registry = gc::THREAD_REGISTRY.lock().unwrap();
                assert_eq!(registry.len(), baseline + 1);
                assert!(registry.iter().any(|entry| entry.0 == child_ctx));
            }

            release_tx.send(()).unwrap();
            handle.join().unwrap();

            let registry = gc::THREAD_REGISTRY.lock().unwrap();
            assert_eq!(registry.len(), baseline);
            assert!(!registry.iter().any(|entry| entry.0 == child_ctx));
        }
    }

    #[test]
    fn arena_roots_are_rescanned_across_major_gcs() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let offsets = [0usize];
            rt_register_type(201, 8, 1, offsets.as_ptr(), None);

            let arena = gc::rt_arena_create(4096);
            let mut holder = gc::rt_arena_alloc(arena, 201, 8);
            rt_push_root(&mut holder);

            let field = (holder - STACK_OFFSET) as *mut i64;
            let len = (gc::LARGE_OBJECT_THRESHOLD + 1024) as i64;
            let first_bytes = vec![b'a'; len as usize];
            let second_bytes = vec![b'b'; len as usize];

            let first = new_string_from_bytes(first_bytes.as_ptr(), len);
            *field = first;
            gc::major_gc();

            let second = new_string_from_bytes(second_bytes.as_ptr(), len);
            *field = second;
            gc::major_gc();

            assert_eq!(*field, second);
            let los_count = gc::LOS_COUNT;
            assert_eq!(los_count, 1);

            rt_pop_roots(1);
            gc::rt_arena_destroy(arena);
        }
    }

    #[test]
    fn arena_cycles_are_safe_during_minor_gc() {
        unsafe {
            let _guard = RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let offsets = [0usize, 8usize];
            rt_register_type(202, 16, 2, offsets.as_ptr(), None);

            let arena = gc::rt_arena_create(4096);
            let mut holder = gc::rt_arena_alloc(arena, 202, 16);
            rt_push_root(&mut holder);

            let fields = (holder - STACK_OFFSET) as *mut i64;
            *fields = holder;

            let payload = b"cycle";
            let value = new_string_from_bytes(payload.as_ptr(), payload.len() as i64);
            *fields.add(1) = value;

            gc::minor_gc();

            let moved = *fields.add(1);
            assert_eq!(
                i64_to_rust_str(moved).as_deref(),
                Some("cycle"),
                "minor GC should preserve stack-held young references through cyclic arena objects"
            );

            rt_pop_roots(1);
            gc::rt_arena_destroy(arena);
        }
    }
}
