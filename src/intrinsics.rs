// Core runtime entry points (used by codegen)
pub const TEJX_MAIN: &str = "tejx_main";
pub const TEJX_RUNTIME_MAIN: &str = "tejx_runtime_main";
pub const TEJX_THROW: &str = "tejx_throw";
pub const TEJX_GET_EXCEPTION: &str = "tejx_get_exception";
pub const TEJX_PUSH_HANDLER: &str = "tejx_push_handler";
pub const TEJX_POP_HANDLER: &str = "tejx_pop_handler";

// Async task management
pub const TEJX_ENQUEUE_TASK: &str = "tejx_enqueue_task";
pub const TEJX_INC_ASYNC_OPS: &str = "tejx_inc_async_ops";
pub const TEJX_DEC_ASYNC_OPS: &str = "tejx_dec_async_ops";
pub const TEJX_RUN_EVENT_LOOP: &str = "tejx_run_event_loop";

// Promise intrinsics (used by lowering and codegen for async/await)
pub const RT_PROMISE_NEW: &str = "rt_promise_new";
pub const RT_PROMISE_RESOLVE: &str = "rt_promise_resolve";
pub const RT_PROMISE_REJECT: &str = "rt_promise_reject";
pub const RT_PROMISE_CLONE: &str = "rt_promise_clone";

// Runtime helpers referenced by codegen
pub const RT_STRING_FROM_C_STR: &str = "rt_string_from_c_str";
pub const RT_MOVE_MEMBER: &str = "rt_move_member";
pub const RT_LEN: &str = "rt_len";
pub const RT_SIZEOF: &str = "rt_sizeof";
pub const RT_MAP_NEW: &str = "rt_Map_constructor";
pub const RT_MAP_SET: &str = "rt_Map_set";
pub const RT_ARRAY_NEW: &str = "rt_Array_constructor";
pub const RT_ARRAY_PUSH: &str = "rt_array_push";
pub const RT_CLASS_NEW: &str = "rt_class_new";
pub const RT_ARENA_CREATE: &str = "rt_arena_create";
pub const RT_ARENA_ALLOC: &str = "rt_arena_alloc";
pub const RT_ARENA_DESTROY: &str = "rt_arena_destroy";
