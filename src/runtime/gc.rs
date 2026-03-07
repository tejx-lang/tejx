use super::*;

// --- GC Memory System Constants ---
pub const YOUNG_GEN_SIZE: usize = 16 * 1024 * 1024; // 16MB Eden
pub const SURVIVOR_SIZE: usize = 2 * 1024 * 1024; // 2MB each
pub const OLD_GEN_SIZE: usize = 64 * 1024 * 1024; // 64MB Old Gen
pub const LARGE_OBJECT_THRESHOLD: usize = 128 * 1024; // 128KB

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ObjectHeader {
    pub gc_word: u64,  // RC/GC Word (Lower bits for marking/age/fwd)
    pub type_id: u16,  // e.g. 0x01 for Int32
    pub flags: u16,    // Bitmask for internal states
    pub length: u32,   // Active elements (for arrays/strings)
    pub capacity: u32, // Total allocated slots (for arrays/strings)
    pub padding: u32,  // Ensure 8-byte alignment
}

// --- GC State Globals ---
#[no_mangle]
pub static mut EDEN_START: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut EDEN_TOP: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut EDEN_END: *mut u8 = 0 as *mut u8;

#[no_mangle]
pub static mut FROM_SURVIVOR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut FROM_SURVIVOR_TOP: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut TO_SURVIVOR: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut TO_SURVIVOR_TOP: *mut u8 = 0 as *mut u8;

#[no_mangle]
pub static mut OLD_START: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut OLD_TOP: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static mut OLD_END: *mut u8 = 0 as *mut u8;

// --- Large Object Space (LOS) ---
pub const MAX_LOS_OBJECTS: usize = 4096;
#[no_mangle]
pub static mut LOS_OBJECTS: [*mut u8; MAX_LOS_OBJECTS] = [0 as *mut u8; MAX_LOS_OBJECTS];
#[no_mangle]
pub static mut LOS_COUNT: usize = 0;

// --- Type Metadata / Type Table ---
pub const MAX_TYPES: usize = 1024;
pub const MAX_PTR_OFFSETS: usize = 64;

#[repr(C)]
pub struct TypeEntry {
    pub size: usize,
    pub ptr_count: usize,
    pub ptr_offsets: [usize; MAX_PTR_OFFSETS],
    pub finalizer: Option<unsafe extern "C" fn(i64)>,
}

#[no_mangle]
pub static mut TYPE_TABLE: [TypeEntry; MAX_TYPES] = unsafe { std::mem::zeroed() };

#[no_mangle]
pub unsafe extern "C" fn rt_register_type(
    id: u32,
    size: usize,
    ptr_count: usize,
    offsets: *const usize,
    finalizer: Option<unsafe extern "C" fn(i64)>,
) {
    if id as usize >= MAX_TYPES {
        return;
    }
    TYPE_TABLE[id as usize].size = size;
    TYPE_TABLE[id as usize].ptr_count = ptr_count;
    TYPE_TABLE[id as usize].finalizer = finalizer;
    if !offsets.is_null() && ptr_count > 0 {
        let count = if ptr_count > MAX_PTR_OFFSETS {
            MAX_PTR_OFFSETS
        } else {
            ptr_count
        };
        for i in 0..count {
            TYPE_TABLE[id as usize].ptr_offsets[i] = *offsets.add(i);
        }
    }
}

// --- Write Barrier System ---
pub const CARD_SHIFT: usize = 9; // 512 bytes per card
pub static mut CARD_TABLE: *mut u8 = 0 as *mut u8;
pub static mut CARD_TABLE_SIZE: usize = 0;

#[no_mangle]
pub unsafe extern "C" fn rt_write_barrier(obj: i64, value: i64) {
    if obj < HEAP_OFFSET || value < HEAP_OFFSET {
        return;
    }

    let obj_ptr = (obj - HEAP_OFFSET) as *mut u8;
    let value_ptr = (value - HEAP_OFFSET) as *mut u8;

    // We only care about Old -> Young references
    // Safety: only mark if obj is in Old Gen
    if obj_ptr >= OLD_START && obj_ptr < OLD_END {
        if in_young_gen(value_ptr) {
            let offset = obj_ptr as usize - OLD_START as usize;
            let card_idx = offset >> CARD_SHIFT;
            *CARD_TABLE.add(card_idx) = 1; // Mark dirty
        }
    }
}

pub fn in_young_gen(ptr: *mut u8) -> bool {
    unsafe {
        (ptr >= EDEN_START && ptr < EDEN_END)
            || (ptr >= FROM_SURVIVOR && ptr < FROM_SURVIVOR.add(SURVIVOR_SIZE))
            || (ptr >= TO_SURVIVOR && ptr < TO_SURVIVOR.add(SURVIVOR_SIZE))
    }
}

pub unsafe fn rt_is_los_ptr(ptr: *mut u8) -> bool {
    for i in 0..LOS_COUNT {
        if LOS_OBJECTS[i] == ptr {
            return true;
        }
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_gc_ptr(ptr: *mut u8) -> bool {
    if ptr.is_null() || EDEN_START.is_null() {
        return false;
    }
    let p = ptr as usize;

    let res = if p >= EDEN_START as usize && p < EDEN_START.add(YOUNG_GEN_SIZE) as usize {
        true
    } else if p >= FROM_SURVIVOR as usize && p < FROM_SURVIVOR.add(SURVIVOR_SIZE) as usize {
        true
    } else if p >= TO_SURVIVOR as usize && p < TO_SURVIVOR.add(SURVIVOR_SIZE) as usize {
        true
    } else if p >= OLD_START as usize && p < OLD_START.add(OLD_GEN_SIZE) as usize {
        true
    } else {
        in_los(ptr)
    };

    if ptr as usize == 7 {}

    /*
    if !res && p >= 0x10000 {
    }
    */
    res
}

// --- Arena Manager ---
pub const ARENA_DEFAULT_SIZE: usize = 4 * 1024 * 1024; // 4MB

#[repr(C)]
pub struct Arena {
    pub base: *mut u8,
    pub offset: usize,
    pub capacity: usize,
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_create(size: usize) -> *mut Arena {
    let actual_size = if size == 0 { ARENA_DEFAULT_SIZE } else { size };
    let base = mmap(
        std::ptr::null_mut(),
        actual_size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON,
        -1,
        0,
    ) as *mut u8;

    if base as isize == -1 {
        printf("FATAL: Failed to mmap Arena\n\0".as_ptr() as *const _);
        exit(1);
    }

    let arena_obj = malloc(std::mem::size_of::<Arena>()) as *mut Arena;
    (*arena_obj).base = base;
    (*arena_obj).offset = 0;
    (*arena_obj).capacity = actual_size;
    arena_obj
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_alloc_raw(arena: *mut Arena, size: usize) -> *mut u8 {
    let aligned_size = (size + 7) & !7;
    if (*arena).offset + aligned_size > (*arena).capacity {
        printf("FATAL: Arena overflow\n\0".as_ptr() as *const _);
        exit(1);
    }
    let ptr = (*arena).base.add((*arena).offset);
    (*arena).offset += aligned_size;
    ptr
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_alloc(arena: *mut Arena, type_id: i32, body_size: i64) -> i64 {
    // Total size = 24 bytes header + body_size
    let total_size = 24 + body_size as usize;
    let obj_ptr = rt_arena_alloc_raw(arena, total_size);

    // Initialise header (type_id, etc.)
    let header = obj_ptr as *mut ObjectHeader;
    (*header).type_id = type_id as u16;
    (*header).length = body_size as u32;

    // Return tagged pointer (body = obj_ptr + 24)
    (obj_ptr as i64) + 24
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_reset(arena: *mut Arena) {
    (*arena).offset = 0;
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_destroy(arena: *mut Arena) {
    // Note: munmap should ideally be called here but we don't have it in extern C block yet
    free(arena as *mut std::ffi::c_void);
}

// --- GC Shadow Stack ---
pub const GC_STACK_SIZE: usize = 1024 * 64;
#[no_mangle]
pub static mut GC_ROOTS: [*mut i64; GC_STACK_SIZE] = [0 as *mut i64; GC_STACK_SIZE];
#[no_mangle]
pub static mut GC_ROOTS_TOP: usize = 0;

const PROT_READ: i32 = 0x01;
const PROT_WRITE: i32 = 0x02;
const MAP_PRIVATE: i32 = 0x0002;
const MAP_ANON: i32 = 0x1000;

#[no_mangle]
pub unsafe fn rt_push_root(ptr: *mut i64) {
    if GC_ROOTS_TOP >= GC_STACK_SIZE {
        printf(
            "FATAL: Root stack overflow (top=%zu)\n\0".as_ptr() as *const _,
            GC_ROOTS_TOP,
        );
        exit(1);
    }
    /*
    printf(
        "DEBUG: push_root[%zu] = %p\n\0".as_ptr() as *const _,
        GC_ROOTS_TOP,
        ptr,
    );
    */

    GC_ROOTS[GC_ROOTS_TOP] = ptr;
    GC_ROOTS_TOP += 1;
}

#[no_mangle]
pub unsafe fn rt_pop_roots(count: usize) {
    if GC_ROOTS_TOP >= count {
        for _ in 0..count {
            GC_ROOTS_TOP -= 1;
            /*
            if count <= GC_ROOTS_TOP {
                GC_ROOTS_TOP -= count;
                printf(
                    "DEBUG: pop_roots(%zu) -> new top=%zu\n\0".as_ptr() as *const _,
                    count,
                    GC_ROOTS_TOP,
                );
            }
            */
            GC_ROOTS[GC_ROOTS_TOP] = std::ptr::null_mut();
        }
    } else {
        printf(
            "FATAL: Root stack underflow: popping %zu but top is %zu\n\0".as_ptr() as *const _,
            count,
            GC_ROOTS_TOP,
        );
        exit(1);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_header(body_ptr: *mut u8) -> *mut ObjectHeader {
    (body_ptr as *mut ObjectHeader).offset(-1)
}

#[no_mangle]
pub unsafe extern "C" fn rt_init_gc() {
    if !EDEN_START.is_null() {
        return;
    }
    let total_young = YOUNG_GEN_SIZE + 2 * SURVIVOR_SIZE;
    EDEN_START = mmap(
        std::ptr::null_mut(),
        total_young,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON,
        -1,
        0,
    ) as *mut u8;

    if EDEN_START as isize == -1 {
        printf("FATAL: Failed to mmap Young Gen\n\0".as_ptr() as *const _);
        exit(1);
    }

    EDEN_TOP = EDEN_START;
    EDEN_END = EDEN_START.add(YOUNG_GEN_SIZE);

    FROM_SURVIVOR = EDEN_END;
    TO_SURVIVOR = FROM_SURVIVOR.add(SURVIVOR_SIZE);

    OLD_START = mmap(
        std::ptr::null_mut(),
        OLD_GEN_SIZE,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON,
        -1,
        0,
    ) as *mut u8;

    if OLD_START as isize == -1 {
        printf("FATAL: Failed to mmap Old Gen\n\0".as_ptr() as *const _);
        exit(1);
    }

    OLD_TOP = OLD_START;
    OLD_END = OLD_START.add(OLD_GEN_SIZE);

    /*
    printf(
        "DEBUG: GC Init: EDEN=%p-%p, SURVIVOR=%p-%p, OLD=%p-%p\n\0".as_ptr() as *const _,
        EDEN_START,
        EDEN_END,
        FROM_SURVIVOR,
        FROM_SURVIVOR.add(SURVIVOR_SIZE),
        OLD_START,
        OLD_END,
    );
    */

    // Initialize Card Table

    CARD_TABLE_SIZE = OLD_GEN_SIZE >> CARD_SHIFT;
    CARD_TABLE = mmap(
        std::ptr::null_mut(),
        CARD_TABLE_SIZE,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON,
        -1,
        0,
    ) as *mut u8;

    if CARD_TABLE as isize == -1 {
        printf("FATAL: Failed to mmap Card Table\n\0".as_ptr() as *const _);
        exit(1);
    }
}

unsafe fn gc_allocate_large(size: usize) -> *mut u8 {
    let total_size = size + std::mem::size_of::<ObjectHeader>();
    let ptr = mmap(
        std::ptr::null_mut(),
        total_size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON,
        -1,
        0,
    ) as *mut u8;

    if ptr as isize == -1 {
        printf("FATAL: Failed to allocate large object\n\0".as_ptr() as *const _);
        exit(1);
    }

    if LOS_COUNT >= MAX_LOS_OBJECTS {
        printf("FATAL: LOS Overflow\n\0".as_ptr() as *const _);
        exit(1);
    }

    LOS_OBJECTS[LOS_COUNT] = ptr;
    LOS_COUNT += 1;

    ptr.add(std::mem::size_of::<ObjectHeader>())
}

#[no_mangle]
pub unsafe extern "C" fn gc_allocate(size: usize) -> *mut u8 {
    let header_size = std::mem::size_of::<ObjectHeader>();
    let total_size = size + header_size;
    let aligned_size = (total_size + 7) & !7;

    if aligned_size >= LARGE_OBJECT_THRESHOLD {
        return gc_allocate_large(size); // gc_allocate_large already handles header
    }

    if EDEN_START.is_null() {
        rt_init_gc();
    }

    /*
     */

    if EDEN_TOP.add(aligned_size) > EDEN_END {
        /*
         */
        minor_gc();
        // If still almost full, trigger major_gc
        if OLD_TOP as usize - OLD_START as usize > (OLD_GEN_SIZE * 9 / 10) {
            major_gc();
        }
        if EDEN_TOP.add(aligned_size) > EDEN_END {
            printf("FATAL: Out of memory after Global GC (EDEN_TOP=%p, aligned_size=%zu, EDEN_END=%p)\n\0".as_ptr() as *const _, EDEN_TOP, aligned_size, EDEN_END);
            exit(1);
        }
    }

    let p = EDEN_TOP;
    /*
    printf(
        "DEBUG: gc_allocate: p=%p, aligned_size=%zu\n\0".as_ptr() as *const _,
        p,
        aligned_size,
    );
    */

    EDEN_TOP = EDEN_TOP.add(aligned_size);

    // Initialize header to zero
    let header = p as *mut ObjectHeader;
    /*
    printf(
        "DEBUG: gc_allocate: initializing header at %p\n\0".as_ptr() as *const _,
        header,
    );
    */

    (*header).gc_word = 0; // mark_bit = 0, age = 0
    (*header).type_id = 0;
    (*header).flags = 0;
    (*header).length = 0;
    (*header).capacity = 0;

    let res = p.add(header_size);
    /*
    printf(
        "DEBUG: gc_allocate(%zu) -> %p (header=%p)\n\0".as_ptr() as *const _,
        size,
        res,
        header,
    );
    */
    res
}

pub unsafe fn in_los(ptr: *mut u8) -> bool {
    for i in 0..LOS_COUNT {
        let obj_ptr = LOS_OBJECTS[i];
        let size =
            get_object_size(obj_ptr as *mut ObjectHeader) + std::mem::size_of::<ObjectHeader>();
        if ptr >= obj_ptr && ptr < obj_ptr.add(size) {
            return true;
        }
    }
    false
}

unsafe fn mark_object(root: *mut i64) {
    let val = *root;
    if val < STACK_OFFSET {
        return;
    }

    let (body_ptr, is_stack) = if val >= HEAP_OFFSET {
        ((val - HEAP_OFFSET) as *mut u8, false)
    } else {
        ((val - STACK_OFFSET) as *mut u8, true)
    };

    if !is_stack && !rt_is_gc_ptr(body_ptr) {
        return;
    }

    let header = (body_ptr as *mut ObjectHeader).offset(-1);

    if ((*header).gc_word & 0x100) != 0 {
        return; // Already marked (using bit 8 for mark_bit)
    }

    (*header).gc_word |= 0x100;

    // Recursively mark fields
    let type_id = (*header).type_id;
    if type_id == TAG_ARRAY as u16 {
        let len = (*header).length;
        let elem_size = ((*header).flags & 0xFF) as u32;

        // Only scan if elements are 8 bytes (could be pointers)
        if elem_size == 8 {
            let data = body_ptr as *mut i64;
            for i in 0..len {
                mark_object(data.add(i as usize));
            }
        }
    } else if type_id == TAG_OBJECT as u16 {
        mark_object(body_ptr.add(16) as *mut i64); // keys_handle at offset 2 (16 bytes)
        mark_object(body_ptr.add(24) as *mut i64); // values_handle at offset 3 (24 bytes)
    } else if (type_id as usize) < MAX_TYPES && TYPE_TABLE[type_id as usize].size > 0 {
        // Fixed-layout object scanning
        let entry = &TYPE_TABLE[type_id as usize];
        for i in 0..entry.ptr_count {
            let offset = entry.ptr_offsets[i];
            let field_ptr = body_ptr.add(offset) as *mut i64;
            mark_object(field_ptr);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn major_gc() {
    // 1. Run minor GC first
    minor_gc();

    // 2. Mark phase (from roots, recursively)
    // Clear marks
    let mut current = OLD_START;
    while current < OLD_TOP {
        let header = current as *mut ObjectHeader;
        (*header).gc_word &= !0x100;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        current = current.add(size);
    }
    for i in 0..LOS_COUNT {
        (*(LOS_OBJECTS[i] as *mut ObjectHeader)).gc_word &= !0x100;
    }

    for i in 0..GC_ROOTS_TOP {
        mark_object(GC_ROOTS[i]);
    }

    // 2.5 Run Finalizers for unmarked objects
    let mut curr = OLD_START;
    while curr < OLD_TOP {
        let header = curr as *mut ObjectHeader;
        let type_id = (*header).type_id;
        if ((*header).gc_word & 0x100) == 0 {
            if (type_id as usize) < MAX_TYPES {
                if let Some(f) = TYPE_TABLE[type_id as usize].finalizer {
                    let obj_val =
                        (curr.add(std::mem::size_of::<ObjectHeader>()) as i64) + HEAP_OFFSET;
                    f(obj_val);
                }
            }
        }
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        curr = curr.add(size);
    }
    // Also LOS finalizers
    for i in 0..LOS_COUNT {
        let header = LOS_OBJECTS[i] as *mut ObjectHeader;
        let type_id = (*header).type_id;
        if ((*header).gc_word & 0x100) == 0 {
            if (type_id as usize) < MAX_TYPES {
                if let Some(f) = TYPE_TABLE[type_id as usize].finalizer {
                    let obj_val = (LOS_OBJECTS[i].add(std::mem::size_of::<ObjectHeader>()) as i64)
                        + HEAP_OFFSET;
                    f(obj_val);
                }
            }
        }
    }

    // 3. Sweep LOS
    let mut new_los_count = 0;
    for i in 0..LOS_COUNT {
        let header = LOS_OBJECTS[i] as *mut ObjectHeader;
        if ((*header).gc_word & 0x100) != 0 {
            LOS_OBJECTS[new_los_count] = LOS_OBJECTS[i];
            new_los_count += 1;
        } else {
            let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
            munmap(LOS_OBJECTS[i] as *mut _, size);
        }
    }
    LOS_COUNT = new_los_count;

    // 4. Mark-Compact for Old Gen (3 passes)

    // Pass 1: Compute new addresses and store in ObjectHeader (gc_word)
    let mut free_ptr = OLD_START;
    let mut scan_ptr = OLD_START;
    while scan_ptr < OLD_TOP {
        let header = scan_ptr as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        if ((*header).gc_word & 0x100) != 0 {
            // Store new address in gc_word (bit 9 = forwarded)
            let new_addr = free_ptr;
            (*header).gc_word = (new_addr as u64) | 0x300; // Mark + Forwarded
            free_ptr = free_ptr.add(size);
        }
        scan_ptr = scan_ptr.add(size);
    }

    // Pass 2: Update pointers
    unsafe fn update_ptr(ptr: *mut i64) {
        let val = *ptr;
        if val < HEAP_OFFSET {
            return;
        }
        let body = (val - HEAP_OFFSET) as *mut u8;
        // If it's in Old Gen and marked, it has a new address stored at its gc_word
        if body >= OLD_START && body < OLD_TOP {
            let header = rt_get_header(body);
            if ((*header).gc_word & 0x200) != 0 {
                let new_header = ((*header).gc_word & !0x3FF) as *mut ObjectHeader;
                let new_body =
                    (new_header as u64).wrapping_add(std::mem::size_of::<ObjectHeader>() as u64);
                *ptr = (new_body as i64) + HEAP_OFFSET;
            }
        }
    }

    // Update roots
    for i in 0..GC_ROOTS_TOP {
        update_ptr(GC_ROOTS[i]);
    }

    // Update Young Gen (Survivor)
    let mut y_scan = FROM_SURVIVOR;
    while y_scan < FROM_SURVIVOR_TOP {
        let header = y_scan as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        update_object_fields(header, update_ptr);
        y_scan = y_scan.add(size);
    }

    // Update Old Gen objects' fields
    scan_ptr = OLD_START;
    while scan_ptr < OLD_TOP {
        let header = scan_ptr as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        if ((*header).gc_word & 0x100) != 0 {
            update_object_fields(header, update_ptr);
        }
        scan_ptr = scan_ptr.add(size);
    }

    // Update LOS objects' fields
    for i in 0..LOS_COUNT {
        update_object_fields(LOS_OBJECTS[i] as *mut ObjectHeader, update_ptr);
    }

    // Pass 3: Actually move
    // Pass 3: Move objects
    let mut move_ptr = OLD_START;
    let mut dest_ptr = OLD_START; // Re-introduce dest_ptr for OLD_TOP update
    while move_ptr < OLD_TOP {
        let header = move_ptr as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        if ((*header).gc_word & 0x100) != 0 {
            let new_addr = ((*header).gc_word & !0x3FF) as *mut u8;
            if new_addr != move_ptr {
                memcpy(new_addr as *mut _, move_ptr as *const _, size);
                // Clear mark bit in new header
                (*(new_addr as *mut ObjectHeader)).gc_word &= !0x300;
            } else {
                (*header).gc_word &= !0x300; // Just clear mark/fwd bits
            }
            dest_ptr = dest_ptr.add(size); // Update dest_ptr for the next available slot
        }
        move_ptr = move_ptr.add(size);
    }
    OLD_TOP = dest_ptr;
}

unsafe fn update_object_fields(header: *mut ObjectHeader, updater: unsafe fn(*mut i64)) {
    let type_id = (*header).type_id;
    let body_ptr = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());

    if type_id == TAG_ARRAY as u16 {
        let len = (*header).length;
        let elem_size = ((*header).flags & 0xFF) as u32;

        if elem_size == 8 {
            let data = body_ptr as *mut i64;
            for i in 0..len {
                updater(data.add(i as usize));
            }
        }
    } else if type_id == TAG_OBJECT as u16 {
        // Map layout: [size, capacity, keys_ptr, values_ptr, data_base] (after ObjectHeader)
        // size: 0, capacity: 8, keys_ptr: 16, values_ptr: 24, data_base: 32
        let cap = *(body_ptr.add(8) as *mut i64);
        let keys_ptr = *(body_ptr.add(16) as *mut *mut i64);
        let values_ptr = *(body_ptr.add(24) as *mut *mut i64);
        if !keys_ptr.is_null() {
            for i in 0..cap {
                updater(keys_ptr.add(i as usize));
            }
        }
        if !values_ptr.is_null() {
            for i in 0..cap {
                updater(values_ptr.add(i as usize));
            }
        }
    }
}

unsafe fn get_object_size(header: *mut ObjectHeader) -> usize {
    let type_id = (*header).type_id;
    let body_size = if type_id == TAG_STRING as u16 {
        (*header).length as usize + 1
    } else if type_id == TAG_ARRAY as u16 {
        let elem_size = ((*header).flags & 0xFF) as usize;
        (*header).capacity as usize * elem_size
    } else if type_id == TAG_OBJECT as u16 {
        40 // Map layout: [size, capacity, keys_ptr, values_ptr, data_base]
    } else if type_id == TAG_INT as u16
        || type_id == TAG_FLOAT as u16
        || type_id == TAG_CHAR as u16
        || type_id == TAG_BOOLEAN as u16
    {
        8 // Boxed primitive (no more tag in body)
    } else if type_id == TAG_RAW_DATA as u16 {
        let body_ptr = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());
        *(body_ptr as *mut i64) as usize
    } else if (type_id as usize) < MAX_TYPES && TYPE_TABLE[type_id as usize].size > 0 {
        TYPE_TABLE[type_id as usize].size
    } else {
        8
    };

    let header_size = std::mem::size_of::<ObjectHeader>();
    let total_size = body_size + header_size;
    let aligned_total = (total_size + 7) & !7;
    aligned_total - header_size
}

pub const PROMOTION_THRESHOLD: u8 = 2;

unsafe fn copy_object(root: *mut i64) {
    let val = *root;
    if val < STACK_OFFSET {
        return;
    }

    if val >= STACK_OFFSET && val < HEAP_OFFSET {
        // Stack object: doesn't move, but we MUST scan its fields
        let body = (val - STACK_OFFSET) as *mut u8;
        scan_object_fields(rt_get_header(body));
        return;
    }

    let old_body = (val - HEAP_OFFSET) as *mut u8;
    if !in_young_gen(old_body) {
        return; // Not in Young Gen
    }

    let header = rt_get_header(old_body);
    // Forwarding check: bit 9 of gc_word
    if ((*header).gc_word & 0x200) != 0 {
        let forwarded_header = ((*header).gc_word & !0x3FF) as *mut ObjectHeader;
        let forwarded_body =
            (forwarded_header as u64).wrapping_add(std::mem::size_of::<ObjectHeader>() as u64);
        *root = (forwarded_body as i64) + HEAP_OFFSET;
        return;
    }

    let size_without_header = get_object_size(header);
    let total_size = size_without_header + std::mem::size_of::<ObjectHeader>();

    // Increment age (lower 8 bits of gc_word)
    let mut age = ((*header).gc_word & 0xFF) as u8;
    age += 1;
    (*header).gc_word = ((*header).gc_word & !0xFF) | (age as u64);

    let (new_header, is_promotion) = if age >= PROMOTION_THRESHOLD
        || (TO_SURVIVOR_TOP as usize + total_size > (TO_SURVIVOR as usize + SURVIVOR_SIZE))
    {
        if OLD_TOP as usize + total_size > OLD_END as usize {
            printf(
                "FATAL: Old Generation Overflow during promotion/fallback\n\0".as_ptr() as *const _,
            );
            exit(1);
        }
        (OLD_TOP as *mut ObjectHeader, true)
    } else {
        (TO_SURVIVOR_TOP as *mut ObjectHeader, false)
    };

    memcpy(
        new_header as *mut std::ffi::c_void,
        header as *const std::ffi::c_void,
        total_size,
    );

    let new_body = (new_header as *mut u8).add(std::mem::size_of::<ObjectHeader>());

    // Mark as forwarded and store pointer in gc_word
    (*header).gc_word = (new_header as u64) | 0x200;

    *root = (new_body as i64) + HEAP_OFFSET;

    if is_promotion {
        OLD_TOP = OLD_TOP.add(total_size);
    } else {
        TO_SURVIVOR_TOP = TO_SURVIVOR_TOP.add(total_size);
    }
}

unsafe fn scan_object_fields(header: *mut ObjectHeader) {
    let type_id = (*header).type_id;
    let body_ptr = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());

    if type_id == TAG_ARRAY as u16 {
        let len = (*header).length;
        let elem_size = ((*header).flags & 0xFF) as u32;

        if elem_size == 8 {
            let data = body_ptr as *mut i64;
            for i in 0..len {
                copy_object(data.add(i as usize));
            }
        }
    } else if type_id == TAG_OBJECT as u16 {
        copy_object(body_ptr.add(24) as *mut i64); // keys_handle
        copy_object(body_ptr.add(32) as *mut i64); // values_handle
    }
}

#[no_mangle]
pub unsafe extern "C" fn minor_gc() {
    TO_SURVIVOR_TOP = TO_SURVIVOR;
    let mut scan_ptr = TO_SURVIVOR;

    // 1. Scan roots
    for i in 0..GC_ROOTS_TOP {
        copy_object(GC_ROOTS[i]);
    }

    // 1b. Scan dirty cards in Old Gen
    let mut current = OLD_START;
    while current < OLD_TOP {
        let header = current as *mut ObjectHeader;
        let offset = current as usize - OLD_START as usize;
        let card_idx = offset >> CARD_SHIFT;

        if *CARD_TABLE.add(card_idx) != 0 {
            scan_object_fields(header);
        }

        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        current = current.add(size);
    }

    // 2. Cheney's scan
    while scan_ptr < TO_SURVIVOR_TOP {
        let header = scan_ptr as *mut ObjectHeader;
        scan_object_fields(header);
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        scan_ptr = scan_ptr.add(size);
    }

    // Swap survivors
    let temp = FROM_SURVIVOR;
    FROM_SURVIVOR = TO_SURVIVOR;
    TO_SURVIVOR = temp;
    FROM_SURVIVOR_TOP = TO_SURVIVOR_TOP;

    EDEN_TOP = EDEN_START;

    // Reset Card Table
    memset(CARD_TABLE as *mut std::ffi::c_void, 0, CARD_TABLE_SIZE);
}
