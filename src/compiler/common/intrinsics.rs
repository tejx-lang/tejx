// Core runtime entry points (used by codegen)
pub const TEJX_MAIN: &str = "tejx_main";
pub const TEJX_RUNTIME_MAIN: &str = "tejx_runtime_main";
pub const TEJX_THROW: &str = "tejx_throw";
pub const TEJX_GET_EXCEPTION: &str = "tejx_get_exception";
pub const TEJX_PUSH_HANDLER: &str = "tejx_push_handler";
pub const TEJX_POP_HANDLER: &str = "tejx_pop_handler";

// Runtime helpers referenced by codegen
pub const RT_STRING_FROM_C_STR: &str = "rt_string_from_c_str";
pub const RT_MOVE_MEMBER: &str = "rt_move_member";
pub const RT_LEN: &str = "rt_len";
pub const RT_SIZEOF: &str = "rt_sizeof";
pub const RT_ARRAY_NEW: &str = "rt_Array_constructor";
pub const RT_ARRAY_PUSH: &str = "rt_array_push";
pub const RT_CLASS_NEW: &str = "rt_class_new";
pub const RT_ARENA_CREATE: &str = "rt_arena_create";
pub const RT_ARENA_ALLOC: &str = "rt_arena_alloc";
pub const RT_ARENA_DESTROY: &str = "rt_arena_destroy";
