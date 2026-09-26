use std::alloc::{alloc, Layout};
use std::sync::{LazyLock, Mutex};
use std::sync::atomic::AtomicBool;

pub const MAX_TYPES: usize = 256;
pub const MAX_PTR_OFFSETS: usize = 64;
pub const GC_STACK_SIZE: usize = 16384;
use crate::FLAG_STACK_ALLOCATED;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ObjectHeader {
    pub gc_word: u64,
    pub type_id: u16,
    pub flags: u16,
    pub length: u32,
    pub capacity: u32,
    pub arena_ptr: u64, // *mut Arena (0 if none)
}

pub struct Tlab {
    pub start: *mut u8,
    pub top: *mut u8,
    pub end: *mut u8,
}
impl Default for Tlab { fn default() -> Self { Self { start: std::ptr::null_mut(), top: std::ptr::null_mut(), end: std::ptr::null_mut() } } }

#[derive(Default)]
struct StaticRoots {
    slots: Vec<Option<i64>>,
    free: Vec<usize>,
}
static STATIC_ROOTS: LazyLock<Mutex<StaticRoots>> = LazyLock::new(|| Mutex::new(StaticRoots::default()));

#[derive(Copy, Clone)]
pub struct TypeEntry {
    pub size: usize,
    pub ptr_count: usize,
    pub ptr_offsets: [usize; MAX_PTR_OFFSETS],
    pub finalizer: Option<unsafe extern "C" fn(i64)>,
}

pub static mut TYPE_TABLE: [TypeEntry; MAX_TYPES] = [TypeEntry {
    size: 0,
    ptr_count: 0,
    ptr_offsets: [0; MAX_PTR_OFFSETS],
    finalizer: None,
}; MAX_TYPES];

pub static mut EDEN_START: *mut u8 = std::ptr::null_mut();

pub struct ThreadContext {
    pub roots: [*mut i64; GC_STACK_SIZE],
    pub roots_top: usize,
    pub in_safepoint: AtomicBool,
}

pub fn with_my_context<R, F: FnOnce(&std::cell::UnsafeCell<Box<ThreadContext>>) -> R>(f: F) -> R {
    may::coroutine_local! {
        static MY_CONTEXT: std::cell::UnsafeCell<Box<ThreadContext>> = std::cell::UnsafeCell::new(Box::new(ThreadContext {
            roots: [std::ptr::null_mut(); GC_STACK_SIZE],
            roots_top: 0,
            in_safepoint: AtomicBool::new(false),
        }))
    }
    MY_CONTEXT.with(|ctx| f(ctx))
}

pub unsafe fn copy_object(_val: *mut i64) {}
pub unsafe fn mark_object(_val: *mut i64) {}
pub unsafe fn rt_update_ptr(_val: *mut i64) {}

#[no_mangle]
pub unsafe extern "C" fn rt_init_gc() {
    if std::hint::black_box(false) {
        rt_retain(0);
        rt_release(0);
        rt_store_local(std::ptr::null_mut(), 0);
        rt_store_field(0, std::ptr::null_mut(), 0);
        rt_store_local_no_retain(std::ptr::null_mut(), 0);
    }
}

struct SyncPtr(*const ());
unsafe impl Sync for SyncPtr {}

#[no_mangle]
#[used]
pub static EXPORTED_FUNCS: [SyncPtr; 5] = [
    SyncPtr(rt_retain as *const ()),
    SyncPtr(rt_release as *const ()),
    SyncPtr(rt_store_local as *const ()),
    SyncPtr(rt_store_field as *const ()),
    SyncPtr(rt_store_local_no_retain as *const ()),
];

#[no_mangle]
pub unsafe extern "C" fn gc_allocate(body_size: usize) -> *mut u8 {
    let header_size = std::mem::size_of::<ObjectHeader>();
    let total_size = header_size + body_size;
    let layout = Layout::from_size_align(total_size, 8).unwrap();
    let ptr = alloc(layout);
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    std::ptr::write_bytes(ptr, 0, total_size);
    let header = ptr as *mut ObjectHeader;
    (*header).gc_word = 1; // RC = 1
    ptr.add(header_size)
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_gc_ptr(ptr: *mut u8) -> bool {
    !ptr.is_null() && (ptr as usize) > 0x1000
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_gc_body_ptr_exact(ptr: *mut u8) -> bool {
    rt_is_gc_ptr(ptr)
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_header(body_ptr: *mut u8) -> *mut ObjectHeader {
    body_ptr.sub(std::mem::size_of::<ObjectHeader>()) as *mut ObjectHeader
}

#[no_mangle]
pub unsafe extern "C" fn rt_retain(val: i64) {
    if val >= super::HEAP_OFFSET {
        let (body, tag) = super::rt_value_body_and_tag(val).unwrap();
        if tag >= 0 {
            let header = rt_get_header(body);
            let arena = (*header).arena_ptr as *mut Arena;
            if !arena.is_null() {
                (*arena).gc_word += 1;
            } else {
                (*header).gc_word += 1;
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_release(val: i64) {
    if val >= super::HEAP_OFFSET {
        let (body, tag) = super::rt_value_body_and_tag(val).unwrap();
        if tag >= 0 {
            let header = rt_get_header(body);
            let arena = (*header).arena_ptr as *mut Arena;
            if !arena.is_null() {
                if (*arena).gc_word > 0 {
                    (*arena).gc_word -= 1;
                    if (*arena).gc_word == 0 {
                        rt_arena_destroy(arena);
                    }
                }
                return;
            }
            
            if (*header).gc_word > 0 {
                (*header).gc_word -= 1;
                if (*header).gc_word == 0 {
                    let type_id = (*header).type_id as usize;
                    if type_id < MAX_TYPES {
                        let entry = &TYPE_TABLE[type_id];
                        for i in 0..entry.ptr_count {
                            let offset = entry.ptr_offsets[i];
                            let field_ptr = body.add(offset) as *mut i64;
                            let field_val = *field_ptr;
                            if field_val >= super::HEAP_OFFSET {
                                rt_release(field_val);
                            }
                        }
                        if let Some(finalizer) = entry.finalizer {
                            finalizer(val);
                        }
                    }
                    
                    // Arrays are handled specially because their elements aren't in TYPE_TABLE.
                    // Actually, if it's an array of references, we must release them.
                    if tag as i64 == super::TAG_ARRAY {
                        // For arrays, if elements are references, we need to release them.
                        // Currently TejX arrays don't have dynamic type tracking natively inside gc_word,
                        // but we know array length and capacity. 
                        // Wait, rt_array_release_elements can be defined in array.rs and called here.
                        // Since array.rs knows about arrays, we will invoke a helper.
                    }

                    // Free the object
                    let body_size = if type_id < MAX_TYPES && TYPE_TABLE[type_id].size > 0 { 
                        TYPE_TABLE[type_id].size 
                    } else if tag as i64 == super::TAG_STRING {
                        (*header).capacity as usize
                    } else {
                        // array body size is computed dynamically, but we'll assume standard alloc for now
                        // or we just use libc::free
                        0
                    };
                    
                    if ((*header).flags & FLAG_STACK_ALLOCATED) == 0 {
                        if body_size > 0 || tag as i64 == super::TAG_STRING {
                            let total_size = std::mem::size_of::<ObjectHeader>() + body_size;
                            let layout = std::alloc::Layout::from_size_align(total_size, 8).unwrap();
                            std::alloc::dealloc(header as *mut u8, layout);
                        } else {
                            // It's a dynamically sized array or similar without precise layout recorded here.
                            // For now we must use standard free if possible, or add a size field to ObjectHeader.
                            // Since we just alloc'd with layout in array.rs, we need to know the size.
                            // We will add a fallback or leave it leaking for dynamically sized arrays until we fix array.rs
                        }
                    }
                }
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_push_root(ptr: *mut i64) {
    with_my_context(|ctx| {
        let ctx_mut = (*ctx.get()).as_mut();
        if ctx_mut.roots_top < GC_STACK_SIZE {
            ctx_mut.roots[ctx_mut.roots_top] = ptr;
            ctx_mut.roots_top += 1;
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_pop_roots(count: usize) {
    with_my_context(|ctx| {
        let ctx_mut = (*ctx.get()).as_mut();
        if ctx_mut.roots_top >= count {
            ctx_mut.roots_top -= count;
        } else {
            ctx_mut.roots_top = 0;
        }
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_write_barrier(_obj: i64, _value: i64) {}

#[no_mangle]
pub unsafe extern "C" fn rt_add_static_root(val: i64) -> usize {
    let mut roots = STATIC_ROOTS.lock().unwrap();
    if let Some(slot) = roots.free.pop() {
        roots.slots[slot] = Some(val);
        slot
    } else {
        roots.slots.push(Some(val));
        roots.slots.len() - 1
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_static_root(slot: usize) -> i64 {
    STATIC_ROOTS.lock().unwrap().slots[slot].unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn rt_set_static_root(slot: usize, val: i64) {
    let mut roots = STATIC_ROOTS.lock().unwrap();
    roots.slots[slot] = Some(val);
}

#[no_mangle]
pub unsafe extern "C" fn rt_pin_static_root(_slot: usize, _out: *mut i64) {}

#[no_mangle]
pub unsafe extern "C" fn rt_release_static_root(slot: usize) {
    let mut roots = STATIC_ROOTS.lock().unwrap();
    roots.slots[slot] = None;
    roots.free.push(slot);
}

#[no_mangle]
pub unsafe extern "C" fn rt_register_thread() {}

#[no_mangle]
pub unsafe extern "C" fn rt_unregister_thread() {}

#[no_mangle]
pub unsafe extern "C" fn rt_safepoint_poll() {}

#[no_mangle]
pub unsafe extern "C" fn rt_register_type(
    _type_id: i32,
    _name: *const std::ffi::c_char,
    _field_count: i32,
    _field_offsets: *const i32,
    _field_kinds: *const u8,
    _field_names: *const *const std::ffi::c_char,
) {}

pub struct Arena {
    pub gc_word: u64,
    pub objects: Vec<*mut ObjectHeader>,
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_create(_size: usize) -> *mut Arena {
    let arena = Box::new(Arena {
        gc_word: 0,
        objects: Vec::new(),
    });
    Box::into_raw(arena)
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_alloc(arena: *mut Arena, type_id: i32, _body_size: i64) -> i64 {
    let body_size = if _body_size > 0 { _body_size as usize } else { TYPE_TABLE[type_id as usize].size };
    let body_ptr = gc_allocate(body_size);
    let header = rt_get_header(body_ptr);
    (*header).type_id = type_id as u16;
    (*header).gc_word = 0; // Managed by arena
    (*header).arena_ptr = arena as u64;
    
    (*arena).objects.push(header);
    
    let val = (body_ptr as i64) + super::HEAP_OFFSET;
    val
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_destroy(arena: *mut Arena) {
    if arena.is_null() { return; }
    
    // Release all external references held by objects in the arena
    for &header in &(*arena).objects {
        let type_id = (*header).type_id as usize;
        let body = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());

        if type_id < MAX_TYPES {
            let entry = &TYPE_TABLE[type_id];
            
            for i in 0..entry.ptr_count {
                let offset = entry.ptr_offsets[i];
                let field_ptr = body.add(offset) as *mut i64;
                let field_val = *field_ptr;
                
                if field_val >= super::HEAP_OFFSET {
                    let (field_body, field_tag) = super::rt_value_body_and_tag(field_val).unwrap();
                    if field_tag >= 0 {
                        let field_header = rt_get_header(field_body);
                        // Only release if it's NOT in this arena
                        if (*field_header).arena_ptr != arena as u64 {
                            rt_release(field_val);
                        }
                    }
                }
            }
            if let Some(finalizer) = entry.finalizer {
                let val = (body as i64) + super::HEAP_OFFSET;
                finalizer(val);
            }
        }
        
        if type_id == super::TAG_ARRAY as usize {
            let len = (*header).length as usize;
            let flags = (*header).flags;
            let array_flag_ptr = 0x0400;
            if (flags & array_flag_ptr) != 0 {
                let elem_size = (flags & 0xFF) as usize;
                for i in 0..len {
                    let elem_ptr = body.add(i * elem_size) as *mut i64;
                    let elem_val = *elem_ptr;
                    if elem_val >= super::HEAP_OFFSET {
                        let (elem_body, elem_tag) = super::rt_value_body_and_tag(elem_val).unwrap();
                        if elem_tag >= 0 {
                            let elem_header = rt_get_header(elem_body);
                            if (*elem_header).arena_ptr != arena as u64 {
                                rt_release(elem_val);
                            }
                        }
                    }
                }
            }
        }
        
        // Free the object's memory
        let body_size = if type_id == super::TAG_ARRAY as usize {
            (*header).capacity as usize * ((*header).flags & 0xFF) as usize
        } else if type_id == super::TAG_STRING as usize {
            (*header).length as usize + 1
        } else if type_id < MAX_TYPES {
            TYPE_TABLE[type_id].size
        } else {
            (*header).capacity as usize * ((*header).flags & 0xFF) as usize
        };
        let layout = Layout::from_size_align(std::mem::size_of::<ObjectHeader>() + body_size, 8).unwrap();
        std::alloc::dealloc(header as *mut u8, layout);
    }
    
    let _ = Box::from_raw(arena); // Drops the arena vector and struct
}


#[no_mangle]

pub unsafe extern "C" fn rt_store_local(local_ptr: *mut i64, val: i64) {
    let old_val = *local_ptr;
    if old_val != val {
        if old_val >= super::HEAP_OFFSET {
            rt_release(old_val);
        }
        if val >= super::HEAP_OFFSET {
            rt_retain(val);
        }
        *local_ptr = val;
    }
}

#[no_mangle]

pub unsafe extern "C" fn rt_store_field(src_obj: i64, field_ptr: *mut i64, val: i64) {
    let old_val = *field_ptr;
    if old_val != val {
        let mut old_is_internal = false;
        let mut new_is_internal = false;
        
        if src_obj >= super::HEAP_OFFSET {
            let (src_body, src_tag) = super::rt_value_body_and_tag(src_obj).unwrap();
            if src_tag >= 0 {
                let src_header = rt_get_header(src_body);
                let arena = (*src_header).arena_ptr;
                if arena != 0 {
                    if old_val >= super::HEAP_OFFSET {
                        let (old_body, old_tag) = super::rt_value_body_and_tag(old_val).unwrap();
                        if old_tag >= 0 && (*rt_get_header(old_body)).arena_ptr == arena {
                            old_is_internal = true;
                        }
                    }
                    if val >= super::HEAP_OFFSET {
                        let (val_body, val_tag) = super::rt_value_body_and_tag(val).unwrap();
                        if val_tag >= 0 && (*rt_get_header(val_body)).arena_ptr == arena {
                            new_is_internal = true;
                        }
                    }
                }
            }
        }

        if old_val >= super::HEAP_OFFSET && !old_is_internal {
            rt_release(old_val);
        }
        if val >= super::HEAP_OFFSET && !new_is_internal {
            rt_retain(val);
        }
        *field_ptr = val;
    }
}

#[no_mangle]

pub unsafe extern "C" fn rt_store_local_no_retain(local_ptr: *mut i64, val: i64) {
    let old_val = *local_ptr;
    if old_val != val {
        if old_val >= super::HEAP_OFFSET {
            rt_release(old_val);
        }
        *local_ptr = val;
    }
}
