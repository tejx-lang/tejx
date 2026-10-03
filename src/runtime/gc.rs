use super::*;
use std::sync::{LazyLock, Mutex, Once};

// =============================================================================
// GC MEMORY CONFIGURATION
// =============================================================================
//
// OLD_GEN_SIZE = maximum old-generation heap (virtual address reservation).
// Physical pages are only allocated by the OS as memory is actually touched.
//
// Configuration priority (highest to lowest):
//   1. Runtime argument:  ./myapp -Xmx16g       — Java-style  (or --tejx-heap 16gb)
//   2. Auto-detection:    min(50% of system RAM, 2GB)
//
// All size strings accept: "16gb", "8192mb", "512kb", "1073741824" (raw bytes)
// =============================================================================

use crate::constants::{
    DEFAULT_ARENA_SIZE, DEFAULT_GC_ROOT_STACK_SIZE, DEFAULT_LOS_GC_TRIGGER_BYTES,
    DEFAULT_OLD_GEN_SIZE, DEFAULT_SURVIVOR_SIZE, DEFAULT_YOUNG_GEN_SIZE,
    LARGE_OBJECT_THRESHOLD as CONST_LARGE_OBJECT_THRESHOLD, MAX_HEAP_CEILING, MIN_OLD_GEN_SIZE,
    NUM_FAST_BINS as CONST_NUM_FAST_BINS,
};

pub static mut YOUNG_GEN_SIZE: usize = DEFAULT_YOUNG_GEN_SIZE;
pub static mut SURVIVOR_SIZE: usize = DEFAULT_SURVIVOR_SIZE;
pub const LARGE_OBJECT_THRESHOLD: usize = CONST_LARGE_OBJECT_THRESHOLD;
pub const GC_MIN_HEAP: usize = MIN_OLD_GEN_SIZE;
pub const GC_MAX_HEAP: usize = MAX_HEAP_CEILING;

/// Final old-gen size, set once at startup by `rt_init_gc()`.
/// Default is overwritten to the detected value before mmap.
pub static mut OLD_GEN_SIZE: usize = DEFAULT_OLD_GEN_SIZE;

/// Set by argv parsing (`-Xmx16g`, `--tejx-heap 16gb`) in `tejx_runtime_main`.
/// 0 = not set (fall through to env/autodetect).
pub static mut ARGV_GC_HEAP_LIMIT: usize = 0;

/// Detect the optimal old-gen heap size at startup.
/// Called once from `rt_init_gc()` after argv/env parsing is complete.
pub(crate) unsafe fn detect_old_gen_size() -> usize {
    // Priority 1: -Xmx16g / --max-old-space-size / --tejx-heap runtime argument
    if ARGV_GC_HEAP_LIMIT > 0 {
        return ARGV_GC_HEAP_LIMIT.clamp(MIN_OLD_GEN_SIZE, GC_MAX_HEAP);
    }

    // Priority 2: TEJX_HEAP environment variable
    if let Ok(val) = std::env::var("TEJX_HEAP") {
        if let Some(bytes) = parse_size_str(&val) {
            return bytes.clamp(MIN_OLD_GEN_SIZE, GC_MAX_HEAP);
        }
    }

    // Priority 3: default initial old-gen size (512 MB)
    DEFAULT_OLD_GEN_SIZE
}

/// Parse size strings: "16gb", "8192mb", "512kb", or raw bytes "1073741824".
/// Case-insensitive. Returns None on parse failure.
pub(crate) fn parse_size_str(s: &str) -> Option<usize> {
    let s = s.trim().to_lowercase();
    // Support Java-style suffix (g/m/k) in addition to full words
    if s.ends_with("gib") || s.ends_with("gb") || s.ends_with('g') {
        let n = s.trim_end_matches(|c: char| c.is_alphabetic()).trim();
        return n.parse::<usize>().ok().map(|v| v * 1024 * 1024 * 1024);
    }
    if s.ends_with("mib") || s.ends_with("mb") || s.ends_with('m') {
        let n = s.trim_end_matches(|c: char| c.is_alphabetic()).trim();
        return n.parse::<usize>().ok().map(|v| v * 1024 * 1024);
    }
    if s.ends_with("kib") || s.ends_with("kb") || s.ends_with('k') {
        let n = s.trim_end_matches(|c: char| c.is_alphabetic()).trim();
        return n.parse::<usize>().ok().map(|v| v * 1024);
    }
    s.parse::<usize>().ok() // raw bytes
}

/// Parse GC-related arguments from argv at process start.
/// Supports:
///   --max-old-space-size=2048         (Node.js style, value in MB by default)
///   -Xmx16g / -Xmx16gb / -Xmx16384m   (Java-style)
///   --tejx-heap 16gb / --tejx-heap=16gb (long-form)
/// Must be called BEFORE rt_init_gc().
pub unsafe fn parse_gc_argv(argc: i32, argv: *mut *mut u8) {
    let args: Vec<String> = (0..argc as usize)
        .filter_map(|i| {
            let ptr = *argv.add(i);
            if ptr.is_null() {
                return None;
            }
            std::ffi::CStr::from_ptr(ptr as *const std::ffi::c_char)
                .to_str()
                .ok()
                .map(|s| s.to_string())
        })
        .collect();

    let mut i = 1usize;
    while i < args.len() {
        let arg = &args[i];

        // --max-old-space-size=2048 (Node.js style, defaults to MB if no suffix)
        if let Some(val) = arg.strip_prefix("--max-old-space-size=") {
            if let Some(bytes) = parse_size_str(val) {
                // If it's a raw number (like "2048"), parse_size_str parses it as bytes.
                // Node.js treats raw numbers as Megabytes.
                if val.chars().all(|c| c.is_ascii_digit()) {
                    ARGV_GC_HEAP_LIMIT = bytes * 1024 * 1024;
                } else {
                    ARGV_GC_HEAP_LIMIT = bytes; // user provided a suffix like 2gb
                }
            }
        }
        // -Xmx16g  (Java-style, no space)
        else if let Some(val) = arg.strip_prefix("-Xmx") {
            if let Some(bytes) = parse_size_str(val) {
                ARGV_GC_HEAP_LIMIT = bytes;
            }
        }
        // --tejx-heap=16gb  (no space)
        else if let Some(val) = arg.strip_prefix("--tejx-heap=") {
            if let Some(bytes) = parse_size_str(val) {
                ARGV_GC_HEAP_LIMIT = bytes;
            }
        }
        // --tejx-heap 16gb  (with space, next arg is value)
        else if arg == "--tejx-heap" {
            if i + 1 < args.len() {
                if let Some(bytes) = parse_size_str(&args[i + 1]) {
                    ARGV_GC_HEAP_LIMIT = bytes;
                    i += 1; // skip the value arg
                }
            }
        }
        // --max-old-space-size 2048 (with space, next arg is value)
        else if arg == "--max-old-space-size" {
            if i + 1 < args.len() {
                let val = &args[i + 1];
                if let Some(bytes) = parse_size_str(val) {
                    if val.chars().all(|c| c.is_ascii_digit()) {
                        ARGV_GC_HEAP_LIMIT = bytes * 1024 * 1024;
                    } else {
                        ARGV_GC_HEAP_LIMIT = bytes;
                    }
                    i += 1; // skip the value arg
                }
            }
        }
        // --vthread-stack=4k or --vt-stack=4k or -Xss=4k or -Xss4k
        else if let Some(val) = arg.strip_prefix("--vthread-stack=") {
            if let Some(bytes) = parse_size_str(val) {
                crate::vthread::ARGV_VT_STACK_SIZE = bytes;
            }
        } else if let Some(val) = arg.strip_prefix("--vt-stack=") {
            if let Some(bytes) = parse_size_str(val) {
                crate::vthread::ARGV_VT_STACK_SIZE = bytes;
            }
        } else if let Some(val) = arg.strip_prefix("-Xss=") {
            if let Some(bytes) = parse_size_str(val) {
                crate::vthread::ARGV_VT_STACK_SIZE = bytes;
            }
        } else if let Some(val) = arg.strip_prefix("-Xss") {
            if let Some(bytes) = parse_size_str(val) {
                crate::vthread::ARGV_VT_STACK_SIZE = bytes;
            }
        } else if (arg == "--vthread-stack" || arg == "--vt-stack" || arg == "-Xss")
            && i + 1 < args.len()
        {
            if let Some(bytes) = parse_size_str(&args[i + 1]) {
                crate::vthread::ARGV_VT_STACK_SIZE = bytes;
                i += 1;
            }
        }
        i += 1;
    }
}

static GC_LOCK: std::sync::LazyLock<std::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(()));
static GC_INIT: Once = Once::new();

pub static FINALIZER_QUEUE: std::sync::LazyLock<
    std::sync::Mutex<Vec<(unsafe extern "C" fn(i64), i64)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

pub static FINALIZER_CONDVAR: std::sync::LazyLock<std::sync::Condvar> =
    std::sync::LazyLock::new(|| std::sync::Condvar::new());

const FLAG_FINALIZED: u32 = 0x1;
pub const FLAG_REMSET_DIRTY: u32 = 0x2;

struct StaticRoots {
    slots: Vec<Option<i64>>,
    free: Vec<usize>,
}

static STATIC_ROOTS: LazyLock<crate::mutex::SpinMutex<StaticRoots>> = LazyLock::new(|| {
    let free = Vec::with_capacity(65536);
    let slots = Vec::with_capacity(65536);
    crate::mutex::SpinMutex::new(StaticRoots { slots, free })
});

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ObjectHeader {
    pub gc_word: u64,  // RC/GC Word (Lower bits for marking/age/fwd)
    pub type_id: u16,  // e.g. 0x01 for Int32
    pub flags: u16,    // Bitmask for internal states
    pub length: u32,   // Active elements (for arrays/strings)
    pub capacity: u32, // Total allocated slots (for arrays/strings)
    pub gc_flags: u32, // GC metadata flags (FLAG_REMSET_DIRTY, FLAG_FINALIZED)
}

const GC_MARK_BIT: u64 = 0x1;
const GC_FWD_BIT: u64 = 0x2;
const GC_FLAG_MASK: u64 = GC_MARK_BIT | GC_FWD_BIT;
const GC_AGE_SHIFT: u64 = 56;
const GC_AGE_MASK: u64 = 0xFFu64 << GC_AGE_SHIFT;
const GC_PTR_MASK: u64 = !(GC_FLAG_MASK | GC_AGE_MASK);

#[inline]
fn gc_is_marked(word: u64) -> bool {
    (word & GC_MARK_BIT) != 0
}

#[inline]
fn gc_is_forwarded(word: u64) -> bool {
    (word & GC_FWD_BIT) != 0
}

#[inline]
fn gc_forward_ptr(word: u64) -> *mut ObjectHeader {
    (word & GC_PTR_MASK) as *mut ObjectHeader
}

#[inline]
fn gc_get_age(word: u64) -> u8 {
    ((word & GC_AGE_MASK) >> GC_AGE_SHIFT) as u8
}

#[inline]
fn gc_set_age(word: u64, age: u8) -> u64 {
    (word & !GC_AGE_MASK) | ((age as u64) << GC_AGE_SHIFT)
}

// --- GC State Globals ---
#[no_mangle]
pub static mut EDEN_START: *mut u8 = 0 as *mut u8;
#[no_mangle]
pub static EDEN_TOP: std::sync::atomic::AtomicPtr<u8> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
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
#[no_mangle]
pub static mut OLD_BYTES_ALLOCATED: usize = 0;
// Adaptive GC trigger threshold.
// Initial trigger is dynamically set to 60% of OLD_GEN_SIZE at startup.
// After each major GC, it dynamically adapts based on live_set * growth_factor,
// but never drops below the 60% floor, ensuring maximum performance for small workloads.
pub static mut OLD_GEN_GC_THRESHOLD: usize = 0; // set in rt_init_gc

pub const NUM_FAST_BINS: usize = CONST_NUM_FAST_BINS;
pub static FAST_FREE_LIST: std::sync::LazyLock<[Mutex<Vec<usize>>; NUM_FAST_BINS]> =
    std::sync::LazyLock::new(|| std::array::from_fn(|_| Mutex::new(Vec::new())));
pub static LARGE_FREE_LIST: std::sync::LazyLock<
    Mutex<std::collections::BTreeMap<usize, Vec<usize>>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::BTreeMap::new()));
pub static PROMOTED_LIST: std::sync::LazyLock<Mutex<Vec<usize>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));
pub static GLOBAL_MARK_QUEUE: std::sync::LazyLock<Mutex<Vec<i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

pub const GC_PHASE_IDLE: u8 = 0;
pub const GC_PHASE_MARK: u8 = 1;
pub static GC_PHASE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(GC_PHASE_IDLE);
pub static GC_BACKGROUND_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// Growth factor for the GC trigger threshold, initialized from env TEJXGC at startup.
// TEJXGC=50 (default) → next trigger = live_bytes × 1.5 (50% headroom above live set)
// TEJXGC=100          → next trigger = live_bytes × 2.0 (Go's default — doubles)
// TEJXGC=10           → next trigger = live_bytes × 1.1 (tight, for low-memory containers)
pub static GC_PERCENTAGE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_GC_PERCENTAGE);
pub static GC_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[inline]
pub fn get_gc_growth_factor() -> f64 {
    1.0_f64 + (GC_PERCENTAGE.load(std::sync::atomic::Ordering::Relaxed) as f64 / 100.0)
}
// --- Large Object Space (LOS) ---
pub const MAX_LOS_OBJECTS: usize = 4096;
const MIN_LOS_GC_TRIGGER_BYTES: usize = DEFAULT_LOS_GC_TRIGGER_BYTES;
static LOS_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
#[no_mangle]
pub static mut LOS_OBJECTS: [*mut u8; MAX_LOS_OBJECTS] = [0 as *mut u8; MAX_LOS_OBJECTS];
#[no_mangle]
pub static mut LOS_SIZES: [usize; MAX_LOS_OBJECTS] = [0; MAX_LOS_OBJECTS];
#[no_mangle]
pub static mut LOS_COUNT: usize = 0;
#[no_mangle]
pub static mut LOS_BYTES: usize = 0;
#[no_mangle]
pub static mut LOS_NEXT_GC_THRESHOLD: usize = MIN_LOS_GC_TRIGGER_BYTES;

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
pub unsafe fn rt_update_ptr(ptr: *mut i64) {
    if ptr.is_null() || (ptr as usize) % 8 != 0 {
        return;
    }
    let val = *ptr;
    if val < HEAP_OFFSET {
        return;
    }
    let body = (val - HEAP_OFFSET) as *mut u8;
    // If it's in Old Gen and marked, it has a new address stored at its gc_word
    if body >= OLD_START && body < OLD_TOP {
        let header = rt_get_header(body);
        if gc_is_forwarded((*header).gc_word) {
            let new_header = gc_forward_ptr((*header).gc_word);
            let new_body =
                (new_header as u64).wrapping_add(std::mem::size_of::<ObjectHeader>() as u64);
            *ptr = (new_body as i64) + HEAP_OFFSET;
        }
    }
}

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

pub unsafe fn rt_add_static_root(val: i64) -> usize {
    let mut roots = STATIC_ROOTS.lock();
    if let Some(slot) = roots.free.pop() {
        roots.slots[slot] = Some(val);
        return slot;
    }
    roots.slots.push(Some(val));
    roots.slots.len() - 1
}

pub unsafe fn rt_get_static_root(slot: usize) -> i64 {
    let roots = STATIC_ROOTS.lock();
    roots.slots.get(slot).and_then(|root| *root).unwrap_or(0)
}

pub unsafe fn rt_pin_static_root(slot: usize, out: *mut i64) {
    if out.is_null() {
        return;
    }

    {
        let roots = STATIC_ROOTS.lock();
        *out = roots.slots.get(slot).and_then(|root| *root).unwrap_or(0);
    }
    rt_push_root(out);
}

pub unsafe fn rt_set_static_root(slot: usize, val: i64) {
    let mut roots = STATIC_ROOTS.lock();
    if let Some(root) = roots.slots.get_mut(slot) {
        if root.is_some() {
            *root = Some(val);
        }
    }
}

pub unsafe fn rt_release_static_root(slot: usize) {
    let mut roots = STATIC_ROOTS.lock();
    if let Some(root) = roots.slots.get_mut(slot) {
        if root.take().is_some() {
            roots.free.push(slot);
        }
    }
}

unsafe fn mark_static_roots() {
    let roots = STATIC_ROOTS.lock();
    for root in roots.slots.iter().flatten() {
        let mut tmp = *root;
        mark_object(&mut tmp);
    }
}

#[allow(dead_code)]
unsafe fn update_static_roots() {
    let mut roots = STATIC_ROOTS.lock();
    for root in roots.slots.iter_mut().flatten() {
        rt_update_ptr(root as *mut i64);
    }
}

unsafe fn copy_static_roots() {
    let mut roots = STATIC_ROOTS.lock();
    for root in roots.slots.iter_mut().flatten() {
        copy_object(root as *mut i64);
    }
}

#[allow(dead_code)]
unsafe fn update_finalizer_queue() {
    let mut queue = FINALIZER_QUEUE.lock().unwrap();
    for (_, obj_val) in queue.iter_mut() {
        rt_update_ptr(obj_val as *mut i64);
    }
}

// Removed CARD_TABLE
#[no_mangle]
pub unsafe extern "C" fn rt_write_barrier(obj: i64, value: i64) {
    if obj < HEAP_OFFSET || value < HEAP_OFFSET {
        return;
    }

    // 1. Concurrent Mark Write Barrier (Incremental Update)
    if GC_PHASE.load(std::sync::atomic::Ordering::Relaxed) == GC_PHASE_MARK {
        MY_CONTEXT.with(|ctx_cell| {
            let ctx = &*ctx_cell.get();
            ctx.mark_queue.lock().unwrap().push(value);
        });
    }

    // 2. Old -> Young Remembered Set
    let obj_ptr = (obj - HEAP_OFFSET) as *mut u8;
    let value_ptr = (value - HEAP_OFFSET) as *mut u8;

    if !in_young_gen(obj_ptr) {
        if in_young_gen(value_ptr) {
            let header = rt_get_header(obj_ptr);
            if ((*header).gc_flags & FLAG_REMSET_DIRTY) == 0 {
                (*header).gc_flags |= FLAG_REMSET_DIRTY;
                MY_CONTEXT.with(|ctx_cell| {
                    let ctx = &*ctx_cell.get();
                    ctx.remset.lock().unwrap().push(obj_ptr);
                });
            }
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

#[inline(always)]
pub unsafe fn in_from_space(ptr: *mut u8) -> bool {
    (ptr >= EDEN_START && ptr < EDEN_END)
        || (ptr >= FROM_SURVIVOR && ptr < FROM_SURVIVOR.add(SURVIVOR_SIZE))
}

pub unsafe fn rt_is_los_ptr(ptr: *mut u8) -> bool {
    let _los_lock = LOS_LOCK.lock().unwrap();
    for i in 0..LOS_COUNT {
        if LOS_OBJECTS[i] == ptr {
            return true;
        }
    }
    false
}

unsafe fn los_snapshot() -> Vec<(*mut u8, usize)> {
    let _los_lock = LOS_LOCK.lock().unwrap();
    let mut snapshot = Vec::with_capacity(LOS_COUNT);
    for i in 0..LOS_COUNT {
        snapshot.push((LOS_OBJECTS[i], LOS_SIZES[i]));
    }
    snapshot
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_gc_ptr(ptr: *mut u8) -> bool {
    if ptr.is_null() || EDEN_START.is_null() {
        return false;
    }
    let p = ptr as usize;
    let eden_end = unsafe { EDEN_START.add(YOUNG_GEN_SIZE + 2 * SURVIVOR_SIZE) as usize };
    let old_end = unsafe { OLD_START.add(OLD_GEN_SIZE) as usize };

    let in_eden = p >= EDEN_START as usize && p < eden_end;
    if in_eden {
        return true;
    }

    let in_old = p >= OLD_START as usize && p < old_end;
    if in_old {
        return true;
    }

    in_los(ptr)
}

unsafe fn region_contains_exact_body(
    mut scan: *mut u8,
    end: *mut u8,
    target_body: *mut u8,
) -> bool {
    while scan < end {
        let header = scan as *mut ObjectHeader;
        let body = scan.add(std::mem::size_of::<ObjectHeader>());
        if body == target_body {
            return true;
        }
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        if size == 0 {
            break;
        }
        scan = scan.add(size);
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn rt_is_gc_body_ptr_exact(ptr: *mut u8) -> bool {
    if ptr.is_null() || EDEN_START.is_null() {
        return false;
    }

    if !rt_is_gc_ptr(ptr) {
        return false;
    }

    {
        let _los_lock = LOS_LOCK.lock().unwrap();
        for i in 0..LOS_COUNT {
            let body = LOS_OBJECTS[i].add(std::mem::size_of::<ObjectHeader>());
            if body == ptr {
                return true;
            }
        }
    }

    let eden_top = EDEN_TOP.load(std::sync::atomic::Ordering::SeqCst);
    if region_contains_exact_body(EDEN_START, eden_top, ptr) {
        return true;
    }
    if region_contains_exact_body(FROM_SURVIVOR, FROM_SURVIVOR_TOP, ptr) {
        return true;
    }
    if region_contains_exact_body(TO_SURVIVOR, TO_SURVIVOR_TOP, ptr) {
        return true;
    }
    region_contains_exact_body(OLD_START, OLD_TOP, ptr)
}

// --- Thread Local Allocation Buffer (TLAB) ---
pub const TLAB_SIZE: usize = 64 * 1024; // 64KB

#[repr(C)]
#[derive(Copy, Clone)]
pub struct Tlab {
    pub top: *mut u8,
    pub end: *mut u8,
}

#[no_mangle]
pub unsafe extern "C" fn rt_clear_tlab() {
    let ctx_ptr = current_thread_context();
    if !ctx_ptr.is_null() {
        (*ctx_ptr).tlab_top = std::ptr::null_mut();
        (*ctx_ptr).tlab_end = std::ptr::null_mut();
    }
}

pub unsafe fn clear_all_tlabs() {
    let registry = lock_registry();
    for &ctx_wrapper in registry.iter() {
        let ctx = ctx_wrapper.0;
        if !ctx.is_null() {
            (*ctx).tlab_top = std::ptr::null_mut();
            (*ctx).tlab_end = std::ptr::null_mut();
        }
    }
}

// --- Arena Manager ---
pub const ARENA_DEFAULT_SIZE: usize = DEFAULT_ARENA_SIZE;

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
        let err = std::io::Error::last_os_error();
        eprintln!(
            "💥 [TejX Runtime Error] Failed to mmap Arena (size={}): {}",
            actual_size, err
        );
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
        //printf("FATAL: Arena overflow\n\0".as_ptr() as *const _);
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
    std::ptr::write_bytes(obj_ptr, 0, total_size);

    // Initialise header (type_id, etc.)
    let header = obj_ptr as *mut ObjectHeader;
    (*header).type_id = type_id as u16;
    (*header).length = body_size as u32;

    // Tag arena objects like stack objects so GC/runtime scan them but never move them.
    (obj_ptr as i64) + 24 + STACK_OFFSET
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_reset(arena: *mut Arena) {
    (*arena).offset = 0;
}

#[no_mangle]
pub unsafe extern "C" fn rt_arena_destroy(arena: *mut Arena) {
    munmap((*arena).base as *mut _, (*arena).capacity);
    free(arena as *mut std::ffi::c_void);
}

// --- GC Safepoints & Thread-Local Roots ---
// Each virtual thread owns one root stack. 4k slots (32 KiB) comfortably
// covers generated request handlers while avoiding a 512 KiB allocation per
// idle connection under high-concurrency servers.
pub const GC_STACK_SIZE: usize = DEFAULT_GC_ROOT_STACK_SIZE;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct ThreadContextPtr(pub *mut ThreadContext);
unsafe impl Send for ThreadContextPtr {}
unsafe impl Sync for ThreadContextPtr {}

// Global registry of all active thread contexts
pub static THREAD_REGISTRY: std::sync::LazyLock<Mutex<Vec<ThreadContextPtr>>> =
    std::sync::LazyLock::new(|| Mutex::new(Vec::new()));

#[inline]
pub(crate) fn lock_registry() -> std::sync::MutexGuard<'static, Vec<ThreadContextPtr>> {
    THREAD_REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

// Global Safepoint flags
pub static SAFEPOINT_REQUEST: AtomicBool = AtomicBool::new(false);
pub static SAFEPOINT_ACK: std::sync::LazyLock<Arc<(std::sync::Mutex<usize>, std::sync::Condvar)>> =
    std::sync::LazyLock::new(|| Arc::new((std::sync::Mutex::new(0), std::sync::Condvar::new())));
pub static SAFEPOINT_RESUME: std::sync::LazyLock<Arc<(std::sync::Mutex<u64>, std::sync::Condvar)>> =
    std::sync::LazyLock::new(|| Arc::new((std::sync::Mutex::new(0), std::sync::Condvar::new())));

#[repr(C)]
pub struct ThreadContext {
    pub roots: [*mut i64; GC_STACK_SIZE],
    pub roots_top: usize,
    pub in_safepoint: AtomicBool,
    pub in_blocking_io: AtomicBool,
    pub remset: Mutex<Vec<*mut u8>>,
    pub mark_queue: Mutex<Vec<i64>>,
    pub tlab_top: *mut u8,
    pub tlab_end: *mut u8,
}

/// Allocate a coroutine's GC context directly in zeroed heap memory.
///
/// `roots` is 512 KiB. Initializing it as part of a `Box::new` expression
/// first places that temporary on the coroutine stack, which makes small
/// virtual-thread stacks overflow before the program starts. The all-zero
/// representation is correct for root pointers and `AtomicBool(false)`;
/// initialize the mutex fields explicitly because they require constructors.
fn new_thread_context() -> std::cell::UnsafeCell<Box<ThreadContext>> {
    unsafe {
        let layout = std::alloc::Layout::new::<ThreadContext>();
        let raw = std::alloc::alloc_zeroed(layout) as *mut ThreadContext;
        if raw.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        std::ptr::addr_of_mut!((*raw).roots_top).write(0);
        std::ptr::addr_of_mut!((*raw).in_safepoint).write(AtomicBool::new(false));
        std::ptr::addr_of_mut!((*raw).in_blocking_io).write(AtomicBool::new(false));
        std::ptr::addr_of_mut!((*raw).remset).write(Mutex::new(Vec::new()));
        std::ptr::addr_of_mut!((*raw).mark_queue).write(Mutex::new(Vec::new()));
        std::ptr::addr_of_mut!((*raw).tlab_top).write(std::ptr::null_mut());
        std::ptr::addr_of_mut!((*raw).tlab_end).write(std::ptr::null_mut());

        std::cell::UnsafeCell::new(Box::from_raw(raw))
    }
}

thread_local! {
    static MY_CONTEXT: std::cell::UnsafeCell<Box<ThreadContext>> = new_thread_context()
}
thread_local! {
    static THREAD_REGISTRATION: std::cell::RefCell<Option<ThreadRegistrationGuard>> = std::cell::RefCell::new(None)
}

pub fn with_my_context<R, F: FnOnce(&std::cell::UnsafeCell<Box<ThreadContext>>) -> R>(f: F) -> R {
    MY_CONTEXT.with(f)
}

struct ThreadRegistrationGuard {
    ctx_ptr: *mut ThreadContext,
}

unsafe fn unregister_thread_context(ctx_ptr: *mut ThreadContext) {
    let removed = {
        let mut registry = lock_registry();
        if let Some(pos) = registry.iter().position(|entry| entry.0 == ctx_ptr) {
            registry.swap_remove(pos);
            true
        } else {
            false
        }
    };

    if removed
        && SAFEPOINT_REQUEST.load(Ordering::SeqCst)
        && !(*ctx_ptr).in_safepoint.load(Ordering::SeqCst)
    {
        let (lock, cvar) = &**SAFEPOINT_ACK;
        let mut count = lock.lock().unwrap_or_else(|e| e.into_inner());
        *count += 1;
        cvar.notify_one();
    }
}

impl Drop for ThreadRegistrationGuard {
    fn drop(&mut self) {
        unsafe {
            unregister_thread_context(self.ctx_ptr);
        }
    }
}

unsafe fn ensure_thread_registered() {
    // Install a 2 MB alternate signal stack on this OS thread if not yet done.
    // This is per-OS-thread (not per-coroutine) so we guard with a std thread_local.
    // The generator crate's overflow handler calls Backtrace::force_capture()
    // from within the alt-stack — the default 32 KB overflows; 2 MB is safe.
    {
        use std::cell::Cell;
        thread_local! {
            static ALT_STACK_INSTALLED: Cell<bool> = Cell::new(false);
        }
        ALT_STACK_INSTALLED.with(|installed| {
            if !installed.get() {
                installed.set(true);
                const ALT_STACK_SIZE: usize = 2 * 1024 * 1024;
                let mem = libc::mmap(
                    std::ptr::null_mut(),
                    ALT_STACK_SIZE,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                );
                if !std::ptr::eq(mem, libc::MAP_FAILED) {
                    let mut ss: libc::stack_t = std::mem::zeroed();
                    ss.ss_sp = mem;
                    ss.ss_size = ALT_STACK_SIZE;
                    ss.ss_flags = 0;
                    libc::sigaltstack(&ss, std::ptr::null_mut());
                }
            }
        });
    }

    MY_CONTEXT.with(|ctx| {
        let ctx_ptr = (*ctx.get()).as_mut() as *mut ThreadContext;
        THREAD_REGISTRATION.with(|registration| {
            let mut registration = registration.borrow_mut();
            if registration.is_some() {
                return;
            }

            let mut registry = lock_registry();
            if !registry.contains(&ThreadContextPtr(ctx_ptr)) {
                registry.push(ThreadContextPtr(ctx_ptr));
            }
            *registration = Some(ThreadRegistrationGuard { ctx_ptr });
        });
    });
}

pub fn is_safepoint_requested() -> bool {
    SAFEPOINT_REQUEST.load(Ordering::SeqCst)
}

thread_local! {
    static CACHED_CTX: std::cell::Cell<*mut ThreadContext> = std::cell::Cell::new(std::ptr::null_mut());
}

#[inline(always)]
pub(crate) unsafe fn current_thread_context() -> *mut ThreadContext {
    let cached = CACHED_CTX.with(|c| c.get());
    if !cached.is_null() {
        return cached;
    }
    ensure_thread_registered();
    let mut ctx_ptr = std::ptr::null_mut();
    MY_CONTEXT.with(|ctx| {
        ctx_ptr = (*ctx.get()).as_mut() as *mut ThreadContext;
    });
    CACHED_CTX.with(|c| c.set(ctx_ptr));
    ctx_ptr
}

#[no_mangle]
pub unsafe extern "C" fn rt_register_thread() {
    ensure_thread_registered();
}

#[no_mangle]
pub unsafe extern "C" fn rt_unregister_thread() {
    THREAD_REGISTRATION.with(|registration| {
        let guard = registration.borrow_mut().take();
        drop(guard);
    });
}

#[no_mangle]
pub unsafe extern "C" fn rt_safepoint_poll() {
    if SAFEPOINT_REQUEST.load(Ordering::Relaxed) {
        rt_safepoint_poll_slow();
        return;
    }
    crate::vthread::vt_preempt_tick();
}

#[cold]
pub(crate) unsafe fn rt_safepoint_poll_slow() {
    if !SAFEPOINT_REQUEST.load(Ordering::SeqCst) {
        return;
    }

    rt_clear_tlab();

    let ctx_ptr = current_thread_context();
    (*ctx_ptr).in_safepoint.store(true, Ordering::SeqCst);

    // Read current epoch from SAFEPOINT_RESUME before acknowledging
    let (lock, cvar) = &**SAFEPOINT_RESUME;
    let mut epoch_guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let current_epoch = *epoch_guard;

    // Acknowledge: increment the ack counter so the GC thread can proceed.
    {
        let (ack_lock, ack_cvar) = &**SAFEPOINT_ACK;
        let mut count = match ack_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *count += 1;
        ack_cvar.notify_one();
    }

    // Sleep OS thread on condvar until epoch advances OR SAFEPOINT_REQUEST is cleared.
    // Eliminates spin-yielding and prevents CPU spikes on all worker threads!
    while SAFEPOINT_REQUEST.load(Ordering::SeqCst) && *epoch_guard == current_epoch {
        epoch_guard = match cvar.wait_timeout(epoch_guard, std::time::Duration::from_millis(20)) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
    drop(epoch_guard);

    (*ctx_ptr).in_safepoint.store(false, Ordering::SeqCst);
}

pub struct ThreadIoGuard {
    ctx_ptr: *mut ThreadContext,
}

impl ThreadIoGuard {
    pub fn new() -> Self {
        unsafe {
            // Clear TLAB before yielding for I/O, because a GC might run and reset Eden.
            rt_clear_tlab();

            let ctx_ptr = current_thread_context();
            (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);

            // If a GC was requested BEFORE we set in_blocking_io to true,
            // the GC thread might have already included us in target_count.
            // We MUST wait for the GC to finish before we actually block in I/O!
            if SAFEPOINT_REQUEST.load(Ordering::SeqCst) {
                (*ctx_ptr).in_blocking_io.store(false, Ordering::SeqCst);
                rt_safepoint_poll_slow();
                (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
            }

            Self { ctx_ptr }
        }
    }
}

impl Drop for ThreadIoGuard {
    fn drop(&mut self) {
        unsafe {
            (*self.ctx_ptr)
                .in_blocking_io
                .store(false, Ordering::SeqCst);
            // If a GC was requested while we were blocked, we must wait before resuming!
            if SAFEPOINT_REQUEST.load(Ordering::SeqCst) {
                rt_safepoint_poll_slow();
            }
        }
    }
}

#[no_mangle]
pub unsafe fn rt_push_root(ptr: *mut i64) {
    if ptr.is_null() {
        return;
    }
    let ctx_ptr = current_thread_context();
    let top = (*ctx_ptr).roots_top;
    if top < GC_STACK_SIZE {
        (*ctx_ptr).roots[top] = ptr;
        (*ctx_ptr).roots_top = top + 1;
    }
}

#[no_mangle]
pub unsafe fn rt_pop_roots(count: usize) {
    let ctx_ptr = current_thread_context();
    if (*ctx_ptr).roots_top >= count {
        (*ctx_ptr).roots_top -= count;
    } else {
        (*ctx_ptr).roots_top = 0;
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_header(body_ptr: *mut u8) -> *mut ObjectHeader {
    (body_ptr as *mut ObjectHeader).offset(-1)
}

pub struct GcContextState {
    pub roots: Vec<usize>,
}

unsafe impl Send for GcContextState {}
unsafe impl Sync for GcContextState {}

pub unsafe fn rt_save_gc_context() -> GcContextState {
    let ctx_ptr = current_thread_context();
    let top = (*ctx_ptr).roots_top.min(GC_STACK_SIZE);
    let roots = (&(*ctx_ptr).roots)[0..top]
        .iter()
        .map(|p| *p as usize)
        .collect();
    (*ctx_ptr).roots_top = 0;
    GcContextState { roots }
}

pub unsafe fn rt_restore_gc_context(state: GcContextState) {
    let ctx_ptr = current_thread_context();
    let len = state.roots.len().min(GC_STACK_SIZE);
    for (i, val) in state.roots.into_iter().take(len).enumerate() {
        (*ctx_ptr).roots[i] = val as *mut i64;
    }
    (*ctx_ptr).roots_top = len;
}

extern "C" fn tejx_crash_handler(
    sig: i32,
    info: *mut libc::siginfo_t,
    _ucontext: *mut libc::c_void,
) {
    let fault_addr = if !info.is_null() {
        unsafe { (*info).si_addr() }
    } else {
        std::ptr::null_mut()
    };

    // If this is a page fault (SIGSEGV / SIGBUS), check if it's a growable vthread stack hitting guard pages!
    if (sig == libc::SIGSEGV || sig == libc::SIGBUS) && !fault_addr.is_null() {
        if unsafe { crate::vthread::try_grow_stack_at(fault_addr as usize) } {
            // Stack grew successfully! Transparently resume execution at the faulting instruction.
            return;
        }
    }

    eprintln!(
        "\n💥 CRASH CAUGHT: signal {} on thread {:?} at fault_addr={:p}",
        sig,
        std::thread::current().name().unwrap_or("unnamed"),
        fault_addr
    );
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    unsafe {
        let uc = _ucontext as *mut libc::ucontext_t;
        if !uc.is_null() {
            let mc = (*uc).uc_mcontext;
            if !mc.is_null() {
                let regs = &(*mc).__ss;
                eprintln!(
                    "  PC={:#x} LR={:#x} SP={:#x}",
                    regs.__pc, regs.__lr, regs.__sp
                );
                let mut dlinfo: libc::Dl_info = std::mem::zeroed();
                if libc::dladdr(regs.__pc as *const libc::c_void, &mut dlinfo) != 0 {
                    let sym = if !dlinfo.dli_sname.is_null() {
                        std::ffi::CStr::from_ptr(dlinfo.dli_sname)
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        "<unknown>".to_string()
                    };
                    let unslid = 0x100000000 + (regs.__pc.saturating_sub(dlinfo.dli_fbase as u64));
                    eprintln!("  PC symbol: {} (unslid {:#x})", sym, unslid);
                }
                let mut dlinfo_lr: libc::Dl_info = std::mem::zeroed();
                if libc::dladdr(regs.__lr as *const libc::c_void, &mut dlinfo_lr) != 0 {
                    let sym = if !dlinfo_lr.dli_sname.is_null() {
                        std::ffi::CStr::from_ptr(dlinfo_lr.dli_sname)
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        "<unknown>".to_string()
                    };
                    let unslid =
                        0x100000000 + (regs.__lr.saturating_sub(dlinfo_lr.dli_fbase as u64));
                    eprintln!("  LR symbol: {} (unslid {:#x})", sym, unslid);
                }
                eprintln!(
                    "  x0={:#x} x1={:#x} x2={:#x} x3={:#x}",
                    regs.__x[0], regs.__x[1], regs.__x[2], regs.__x[3]
                );
                eprintln!(
                    "  x4={:#x} x5={:#x} x6={:#x} x7={:#x}",
                    regs.__x[4], regs.__x[5], regs.__x[6], regs.__x[7]
                );
                eprintln!(
                    "  x8={:#x} x9={:#x} x10={:#x} x11={:#x}",
                    regs.__x[8], regs.__x[9], regs.__x[10], regs.__x[11]
                );
                eprintln!(
                    "  x12={:#x} x13={:#x} x14={:#x} x15={:#x}",
                    regs.__x[12], regs.__x[13], regs.__x[14], regs.__x[15]
                );
                eprintln!(
                    "  x16={:#x} x17={:#x} x18={:#x} x19={:#x}",
                    regs.__x[16], regs.__x[17], regs.__x[18], regs.__x[19]
                );
                eprintln!(
                    "  x20={:#x} x21={:#x} x22={:#x} x23={:#x}",
                    regs.__x[20], regs.__x[21], regs.__x[22], regs.__x[23]
                );
                eprintln!(
                    "  x24={:#x} x25={:#x} x26={:#x} x27={:#x}",
                    regs.__x[24], regs.__x[25], regs.__x[26], regs.__x[27]
                );
                eprintln!(
                    "  x28={:#x} x29(fp)={:#x} x30(lr)={:#x}",
                    regs.__x[28], regs.__fp, regs.__lr
                );
                let sp_ptr = regs.__sp as *const u64;
                if !sp_ptr.is_null() && (regs.__sp & 7) == 0 {
                    eprintln!("  Stack dump at SP ({:#x}):", regs.__sp);
                    for i in 0..32 {
                        let word = *sp_ptr.add(i);
                        let mut dl: libc::Dl_info = std::mem::zeroed();
                        if libc::dladdr(word as *const libc::c_void, &mut dl) != 0
                            && !dl.dli_fbase.is_null()
                        {
                            let sym = if !dl.dli_sname.is_null() {
                                std::ffi::CStr::from_ptr(dl.dli_sname)
                                    .to_string_lossy()
                                    .into_owned()
                            } else {
                                "<unknown>".to_string()
                            };
                            let unslid = 0x100000000 + (word.saturating_sub(dl.dli_fbase as u64));
                            eprintln!(
                                "    [SP + {:3}] = {:#018x} -> {} (unslid {:#x})",
                                i * 8,
                                word,
                                sym,
                                unslid
                            );
                        } else {
                            eprintln!("    [SP + {:3}] = {:#018x}", i * 8, word);
                        }
                    }
                }
                let fp_ptr = regs.__fp as *const u64;
                if !fp_ptr.is_null() && (regs.__fp & 7) == 0 {
                    eprintln!("  Frame dump at FP ({:#x}):", regs.__fp);
                    for i in 0..4 {
                        let word = *fp_ptr.add(i);
                        eprintln!("    [FP + {:2}] = {:#018x}", i * 8, word);
                    }
                }
            }
        }
    }
    let mut bt = [std::ptr::null_mut(); 32];
    let size = unsafe { libc::backtrace(bt.as_mut_ptr(), 32) };
    let symbols = unsafe { libc::backtrace_symbols(bt.as_ptr(), size) };
    if !symbols.is_null() {
        for i in 0..size {
            let sym = unsafe { std::ffi::CStr::from_ptr(*symbols.offset(i as isize)) };
            eprintln!("  [{}] {}", i, sym.to_string_lossy());
        }
    }
    unsafe {
        libc::_exit(139);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_init_gc() {
    GC_INIT.call_once(|| unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = tejx_crash_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGILL, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTRAP, &sa, std::ptr::null_mut());

        // Determine old-gen size: #[gc(heap)] attribute > TEJX_HEAP env var > default 512MB
        OLD_GEN_SIZE = (detect_old_gen_size() + 0xFFFF) & !0xFFFF;
        YOUNG_GEN_SIZE = crate::constants::DEFAULT_YOUNG_GEN_SIZE;
        SURVIVOR_SIZE = crate::constants::DEFAULT_SURVIVOR_SIZE;

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
            let err = std::io::Error::last_os_error();
            eprintln!(
                "💥 [TejX Runtime Error] Failed to mmap Eden heap (size={}): {}",
                total_young, err
            );
            exit(1);
        }

        EDEN_TOP.store(EDEN_START, std::sync::atomic::Ordering::SeqCst);
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
            let err = std::io::Error::last_os_error();
            let old_size = OLD_GEN_SIZE;
            eprintln!(
                "💥 [TejX Runtime Error] Failed to mmap Old Gen heap (size={}): {}",
                old_size, err
            );
            exit(1);
        }

        OLD_TOP = OLD_START;
        OLD_END = OLD_START.add(OLD_GEN_SIZE);

        // Initial GC threshold is exactly DEFAULT_GC_TRIGGER_FLOOR_PCT% of the max heap.
        // It acts as the absolute floor so we never GC before the heap reaches that capacity.
        OLD_GEN_GC_THRESHOLD = OLD_GEN_SIZE * DEFAULT_GC_TRIGGER_FLOOR_PCT / 100;

        if let Some(val) = std::env::var("TEJXGC")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        {
            GC_PERCENTAGE.store(val, std::sync::atomic::Ordering::SeqCst);
        }

        rt_start_gc_scheduler();
        rt_start_finalizer_thread();
    });

    ensure_thread_registered();
}

#[no_mangle]
pub unsafe extern "C" fn rt_start_finalizer_thread() {
    std::thread::Builder::new()
        .name("gc-finalizer".to_string())
        .stack_size(256 * 1024)
        .spawn(|| {
            unsafe {
                rt_register_thread();
            }
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| loop {
                let task = {
                    let mut queue = FINALIZER_QUEUE.lock().unwrap();
                    while queue.is_empty() {
                        unsafe {
                            let ctx_ptr = current_thread_context();
                            (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
                        }
                        queue = FINALIZER_CONDVAR.wait(queue).unwrap();
                        unsafe {
                            let ctx_ptr = current_thread_context();
                            (*ctx_ptr).in_blocking_io.store(false, Ordering::SeqCst);
                        }
                    }
                    if queue.is_empty() {
                        None
                    } else {
                        Some(queue.remove(0))
                    }
                };
                if let Some((f, mut obj_val)) = task {
                    unsafe {
                        if SAFEPOINT_REQUEST.load(Ordering::SeqCst) {
                            crate::gc::rt_safepoint_poll_slow();
                        }
                        rt_push_root(&mut obj_val);
                        f(obj_val);
                        rt_pop_roots(1);
                    }
                }
            }));
        })
        .unwrap();
}

#[no_mangle]
pub unsafe extern "C" fn rt_start_gc_scheduler() {
    /*
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if OLD_BYTES_ALLOCATED > OLD_GEN_GC_THRESHOLD {
                unsafe { major_gc() };
            }
        }
    });
    */
}

pub unsafe fn gc_allocate_large(size: usize) -> *mut u8 {
    ensure_thread_registered();
    if EDEN_START.is_null() {
        rt_init_gc();
    }

    let header_size = std::mem::size_of::<ObjectHeader>();
    let total_size = (size + header_size + 7) & !7;
    let _gc_lock = {
        let _io = crate::ThreadIoGuard::new();
        GC_LOCK.lock().unwrap()
    };

    let needs_major_gc = {
        let _los_lock = LOS_LOCK.lock().unwrap();
        LOS_COUNT >= MAX_LOS_OBJECTS
            || (GC_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
                && LOS_BYTES.saturating_add(total_size) > LOS_NEXT_GC_THRESHOLD)
    };

    if needs_major_gc {
        major_gc_locked_internal(true, false);
    }

    {
        let _los_lock = LOS_LOCK.lock().unwrap();
        if LOS_COUNT >= MAX_LOS_OBJECTS {
            eprintln!("FATAL: LOS Overflow");
            exit(1);
        }
    }

    let ptr = mmap(
        std::ptr::null_mut(),
        total_size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANON,
        -1,
        0,
    ) as *mut u8;

    if ptr as isize == -1 {
        eprintln!(
            "FATAL: Failed to allocate large object of size {} from mmap",
            size
        );
        exit(1);
    }

    {
        let _los_lock = LOS_LOCK.lock().unwrap();
        LOS_OBJECTS[LOS_COUNT] = ptr;
        LOS_SIZES[LOS_COUNT] = total_size;
        LOS_COUNT += 1;
        LOS_BYTES = LOS_BYTES.saturating_add(total_size);
    }

    ptr.add(header_size)
}

#[no_mangle]
pub unsafe extern "C" fn gc_allocate(size: usize) -> *mut u8 {
    gc_allocate_impl(size)
}

#[inline(always)]
unsafe fn gc_allocate_impl(size: usize) -> *mut u8 {
    ensure_thread_registered();
    let header_size = std::mem::size_of::<ObjectHeader>();
    let total_size = size + header_size;
    let aligned_size = (total_size + 7) & !7;

    if aligned_size >= LARGE_OBJECT_THRESHOLD {
        return gc_allocate_large(size);
    }

    if EDEN_START.is_null() {
        rt_init_gc();
    }

    let ctx_ptr = current_thread_context();
    let tlab_top = (*ctx_ptr).tlab_top;
    let tlab_end = (*ctx_ptr).tlab_end;

    // Try TLAB allocation
    if !tlab_top.is_null() && tlab_top.add(aligned_size) <= tlab_end {
        let p = tlab_top;
        (*ctx_ptr).tlab_top = tlab_top.add(aligned_size);

        let header = p as *mut ObjectHeader;
        (*header).gc_word = 0;
        (*header).type_id = 0;
        (*header).flags = 0;
        (*header).length = 0;
        (*header).capacity = 0;
        (*header).gc_flags = 0;
        let body_ptr = p.add(header_size);
        memset(body_ptr as *mut _, 0, size);
        return body_ptr;
    }

    // TLAB refill or slow path (atomic global allocation)
    let refill_size = if aligned_size > TLAB_SIZE / 2 {
        aligned_size // Too large for TLAB, allocate directly
    } else {
        TLAB_SIZE
    };

    loop {
        let current_top = EDEN_TOP.load(std::sync::atomic::Ordering::SeqCst);
        if current_top.add(refill_size) > EDEN_END {
            {
                let _lock = {
                    let _io = crate::ThreadIoGuard::new();
                    GC_LOCK.lock().unwrap()
                };
                let current_top = EDEN_TOP.load(std::sync::atomic::Ordering::SeqCst);
                if current_top.add(refill_size) > EDEN_END {
                    trigger_safepoint();
                    minor_gc_locked();
                    if GC_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
                        && OLD_BYTES_ALLOCATED > OLD_GEN_GC_THRESHOLD
                    {
                        major_gc_locked_internal(false, true);
                    }
                    resume_safepoint();
                }
            }
            FINALIZER_CONDVAR.notify_one();

            if EDEN_TOP
                .load(std::sync::atomic::Ordering::SeqCst)
                .add(refill_size)
                > EDEN_END
            {
                if refill_size > aligned_size {
                    // Try one more time without refill
                    continue;
                }
                eprintln!(
                    "FATAL: Out of memory in Eden after minor_gc (refill_size: {})",
                    refill_size
                );
                exit(1);
            }
            continue;
        }

        if EDEN_TOP
            .compare_exchange(
                current_top,
                current_top.add(refill_size),
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok()
        {
            if refill_size == aligned_size {
                memset(current_top as *mut _, 0, refill_size);
                let header = current_top as *mut ObjectHeader;
                (*header).gc_word = 0;
                (*header).type_id = 0;
                (*header).flags = 0;
                (*header).length = 0;
                (*header).capacity = 0;
                (*header).gc_flags = 0;
                let body_ptr = current_top.add(header_size);
                memset(body_ptr as *mut _, 0, size);
                return body_ptr;
            } else {
                // Refill TLAB
                memset(current_top as *mut _, 0, refill_size);
                (*ctx_ptr).tlab_top = current_top.add(aligned_size);
                (*ctx_ptr).tlab_end = current_top.add(refill_size);

                let header = current_top as *mut ObjectHeader;
                (*header).gc_word = 0;
                (*header).type_id = 0;
                (*header).flags = 0;
                (*header).length = 0;
                (*header).capacity = 0;
                (*header).gc_flags = 0;
                let body_ptr = current_top.add(header_size);
                memset(body_ptr as *mut _, 0, size);
                return body_ptr;
            }
        }
    }
}

pub unsafe fn in_los(ptr: *mut u8) -> bool {
    let _los_lock = LOS_LOCK.lock().unwrap();
    for i in 0..LOS_COUNT {
        let obj_ptr = LOS_OBJECTS[i];
        let size = LOS_SIZES[i];
        if ptr >= obj_ptr && ptr < obj_ptr.add(size) {
            return true;
        }
    }
    false
}

pub unsafe fn mark_object(root: *mut i64) {
    if root.is_null() || (root as usize) % 8 != 0 {
        return;
    }
    let mut val = *root;
    if val < STACK_OFFSET {
        return;
    }
    let resolved_val = crate::rt_gc_resolve_array_id(val);
    if resolved_val != val {
        *root = resolved_val;
        val = resolved_val;
    }
    GLOBAL_MARK_QUEUE.lock().unwrap().push(val);
}

pub unsafe fn process_mark_queue() {
    let mut local_queue = Vec::new();
    let mut seen_stack: Vec<usize> = Vec::new();

    loop {
        {
            let mut global_q = GLOBAL_MARK_QUEUE.lock().unwrap();
            local_queue.extend(global_q.drain(..));

            let registry = THREAD_REGISTRY.lock().unwrap();
            for &ctx_wrapper in registry.iter() {
                let ctx = &*ctx_wrapper.0;
                let mut mq = ctx.mark_queue.lock().unwrap();
                local_queue.extend(mq.drain(..));
            }
        }

        if local_queue.is_empty() {
            break;
        }

        while let Some(val) = local_queue.pop() {
            if val < STACK_OFFSET {
                continue;
            }

            let (body_ptr, is_stack) = if val >= HEAP_OFFSET {
                ((val - HEAP_OFFSET) as *mut u8, false)
            } else {
                ((val - STACK_OFFSET) as *mut u8, true)
            };

            if !is_stack && !rt_is_gc_ptr(body_ptr) {
                continue;
            }

            let header = (body_ptr as *mut ObjectHeader).offset(-1);

            if is_stack {
                let h_addr = header as usize;
                if seen_stack.contains(&h_addr) {
                    continue;
                }
                seen_stack.push(h_addr);
            } else {
                if gc_is_marked((*header).gc_word) {
                    continue;
                }
                (*header).gc_word |= GC_MARK_BIT;
            }

            let mut push_field = |field_ptr: *mut i64| {
                if field_ptr.is_null() {
                    return;
                }
                let mut field_val = *field_ptr;
                if field_val >= STACK_OFFSET {
                    let resolved = crate::rt_gc_resolve_array_id(field_val);
                    if resolved != field_val {
                        *field_ptr = resolved;
                        field_val = resolved;
                    }
                    local_queue.push(field_val);
                }
            };

            let type_id = (*header).type_id;
            if type_id == TAG_ARRAY as u16 {
                let len = (*header).length;
                let is_ptr_array = ((*header).flags & (ARRAY_FLAG_PTR as u16)) != 0;
                if is_ptr_array {
                    let data = body_ptr as *mut i64;
                    for i in 0..len {
                        push_field(data.add(i as usize));
                    }
                }
            } else if type_id == TAG_OBJECT as u16 {
                push_field(body_ptr.add(16) as *mut i64);
                push_field(body_ptr.add(24) as *mut i64);
            } else if type_id == TAG_PROMISE as u16 {
                push_field(body_ptr.add(8) as *mut i64);
                push_field(body_ptr.add(16) as *mut i64);
            } else if type_id == TAG_FUNCTION as u16 {
                push_field(body_ptr.add(8) as *mut i64);
            } else if (type_id as usize) < MAX_TYPES && TYPE_TABLE[type_id as usize].ptr_count > 0 {
                let entry = &TYPE_TABLE[type_id as usize];
                for i in 0..entry.ptr_count {
                    push_field(body_ptr.add(entry.ptr_offsets[i]) as *mut i64);
                }
            }
        }
    }
}

unsafe fn major_gc_locked_internal(run_minor_first: bool, safepoint_already: bool) {
    if GC_BACKGROUND_RUNNING.load(std::sync::atomic::Ordering::SeqCst) {
        // A GC is already running in the background. Don't start another one.
        // If we really need memory, we could spinloop here, but returning is safer to prevent deadlocks.
        return;
    }
    GC_BACKGROUND_RUNNING.store(true, std::sync::atomic::Ordering::SeqCst);

    clear_all_tlabs();
    crate::rt_gc_prepare_array_forward();

    if !safepoint_already {
        trigger_safepoint();
    }

    // 1. Run minor GC first (optional)
    if run_minor_first {
        minor_gc_locked();
    }

    // 2. Initial Mark (STW)
    GC_PHASE.store(GC_PHASE_MARK, std::sync::atomic::Ordering::SeqCst);
    let los_before_sweep = los_snapshot();

    {
        let registry = lock_registry();
        for &ctx_wrapper in registry.iter() {
            let ctx_ptr = ctx_wrapper.0;
            let top = (*ctx_ptr).roots_top;
            for i in 0..top {
                mark_object((*ctx_ptr).roots[i]);
            }
        }
    }
    mark_static_roots();
    super::rt_gc_mark_tasks();
    {
        let queue = FINALIZER_QUEUE.lock().unwrap();
        for (_, obj_val) in queue.iter() {
            let mut tmp = *obj_val;
            mark_object(&mut tmp as *mut i64);
        }
    }

    // End of Initial Mark STW
    if !safepoint_already {
        resume_safepoint();
    }

    // 3. Concurrent Trace
    process_mark_queue();

    // 4. Remark (STW)
    if !safepoint_already {
        trigger_safepoint();
    }
    GC_PHASE.store(GC_PHASE_IDLE, std::sync::atomic::Ordering::SeqCst);

    // Trace roots again to catch any missed updates
    {
        let registry = lock_registry();
        for &ctx_wrapper in registry.iter() {
            let ctx_ptr = ctx_wrapper.0;
            let top = (*ctx_ptr).roots_top;
            for i in 0..top {
                mark_object((*ctx_ptr).roots[i]);
            }
        }
    }
    mark_static_roots();
    super::rt_gc_mark_tasks();
    process_mark_queue();

    // 4.5 Filter RemSets to remove dead objects before Sweep
    {
        let registry = lock_registry();
        for &ctx_wrapper in registry.iter() {
            let ctx = &*ctx_wrapper.0;
            let mut remset = ctx.remset.lock().unwrap();
            remset.retain(|&obj_ptr| {
                let body = obj_ptr as *mut u8;
                let header = rt_get_header(body);
                if gc_is_marked((*header).gc_word) {
                    true
                } else {
                    (*header).gc_flags &= !FLAG_REMSET_DIRTY;
                    false
                }
            });
        }
    }

    // 5. Finalizer Queueing (Concurrent) is now moved to the background thread.
    let los_snapshot_for_finalizers: Vec<(usize, usize)> = los_before_sweep
        .iter()
        .map(|&(ptr, size)| (ptr as usize, size))
        .collect();

    // 3. Sweep LOS (Concurrent) is now moved to the background thread.

    // 4. Mark-Sweep for Old Gen (Concurrent)
    let sweep_limit = OLD_TOP as usize;
    // Capture the live bytes RIGHT NOW (before sweep) so the background thread
    // can calculate how effective this GC cycle was.
    let bytes_before_gc = OLD_BYTES_ALLOCATED;
    for bin in FAST_FREE_LIST.iter() {
        bin.lock().unwrap().clear();
    }
    LARGE_FREE_LIST.lock().unwrap().clear();

    // --- PHASE 1: Finalizer Resurrect ---
    let mut resurrected = false;
    let mut curr = OLD_START;
    let limit_ptr = sweep_limit as *mut u8;

    while curr < limit_ptr {
        let header = curr as *mut ObjectHeader;
        let type_id = (*header).type_id;
        if !gc_is_marked((*header).gc_word) {
            if ((*header).gc_flags & FLAG_FINALIZED) == 0 {
                if (type_id as usize) < MAX_TYPES {
                    if let Some(f) = TYPE_TABLE[type_id as usize].finalizer {
                        let mut obj_val =
                            (curr.add(std::mem::size_of::<ObjectHeader>()) as i64) + HEAP_OFFSET;
                        {
                            let mut queue = FINALIZER_QUEUE.lock().unwrap();
                            queue.push((f, obj_val));
                        }
                        (*header).gc_flags |= FLAG_FINALIZED;
                        mark_object(&mut obj_val as *mut i64);
                        resurrected = true;
                    }
                }
            }
        }
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        curr = curr.add(size);
    }

    // Also LOS finalizers
    for &(obj_usize, _) in &los_snapshot_for_finalizers {
        let obj_ptr = obj_usize as *mut u8;
        let header = obj_ptr as *mut ObjectHeader;
        let type_id = (*header).type_id;
        if !gc_is_marked((*header).gc_word) {
            if ((*header).gc_flags & FLAG_FINALIZED) == 0 {
                if (type_id as usize) < MAX_TYPES {
                    if let Some(f) = TYPE_TABLE[type_id as usize].finalizer {
                        let mut obj_val =
                            (obj_ptr.add(std::mem::size_of::<ObjectHeader>()) as i64) + HEAP_OFFSET;
                        {
                            let mut queue = FINALIZER_QUEUE.lock().unwrap();
                            queue.push((f, obj_val));
                        }
                        (*header).gc_flags |= FLAG_FINALIZED;
                        mark_object(&mut obj_val as *mut i64);
                        resurrected = true;
                    }
                }
            }
        }
    }

    if resurrected {
        process_mark_queue();
        FINALIZER_CONDVAR.notify_one();
    }

    // Clear any transient mark bits on young survivors before resuming mutators.
    let mut y_scan = FROM_SURVIVOR;
    while y_scan < FROM_SURVIVOR_TOP {
        let header = y_scan as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
        (*header).gc_word &= !GC_MARK_BIT;
        y_scan = y_scan.add(size);
    }

    if !safepoint_already {
        // End of STW phase! Mutator threads resume immediately.
        // Remark and finalizer resurrection are complete.
        // Sweeping old gen and LOS now proceeds concurrently.
        resume_safepoint();
    }

    // --- PHASE 2: Concurrent Sweep ---
    let mut scan_ptr = OLD_START;
    let mut local_free_list: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut new_old_bytes = 0usize;

    while scan_ptr < limit_ptr {
        let header = scan_ptr as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();

        if gc_is_marked((*header).gc_word) {
            // Unmark it for next time
            (*header).gc_word &= !GC_MARK_BIT;
            new_old_bytes += size;
        } else {
            // Dead object, add to local free list!
            local_free_list
                .entry(size)
                .or_default()
                .push(scan_ptr as usize);
        }
        scan_ptr = scan_ptr.add(size);
    }

    // Merge into global FREE_LIST
    let mut global_large = LARGE_FREE_LIST.lock().unwrap();
    for (size, mut vec) in local_free_list {
        if size < NUM_FAST_BINS * 8 {
            let bin_idx = size / 8;
            FAST_FREE_LIST[bin_idx].lock().unwrap().append(&mut vec);
        } else {
            global_large.entry(size).or_default().append(&mut vec);
        }
    }
    // --- PHASE 3: Concurrent Sweep LOS ---
    {
        let _los_lock = LOS_LOCK.lock().unwrap();
        let mut new_los_count = 0;
        let mut new_los_bytes = 0usize;
        let original_los_count = los_snapshot_for_finalizers.len();

        for i in 0..LOS_COUNT {
            let header = LOS_OBJECTS[i] as *mut ObjectHeader;

            let mut is_survivor = false;
            if i < original_los_count {
                if gc_is_marked((*header).gc_word) {
                    is_survivor = true;
                    (*header).gc_word &= !GC_MARK_BIT;
                }
            } else {
                // Object was allocated during concurrent GC. Implicitly alive!
                is_survivor = true;
            }

            if is_survivor {
                LOS_OBJECTS[new_los_count] = LOS_OBJECTS[i];
                LOS_SIZES[new_los_count] = LOS_SIZES[i];
                new_los_bytes = new_los_bytes.saturating_add(LOS_SIZES[i]);
                new_los_count += 1;
            } else {
                munmap(LOS_OBJECTS[i] as *mut _, LOS_SIZES[i]);
            }
        }
        LOS_COUNT = new_los_count;
        LOS_BYTES = new_los_bytes;
        LOS_NEXT_GC_THRESHOLD =
            std::cmp::max(MIN_LOS_GC_TRIGGER_BYTES, LOS_BYTES.saturating_mul(2));
    }

    OLD_BYTES_ALLOCATED = new_old_bytes;

    // =================================================================
    // DEFINITIVE GC TRIGGER THRESHOLD
    // =================================================================
    //
    // Formula:
    //   scaled   = live × growth_factor          (proportional headroom)
    //   next     = max(scaled, bytes_before_gc)  (← the key insight)
    //   next     = max(next, live + 128MB)        (absolute headroom floor)
    //   next     = max(next, 512MB)               (micro-thrash guard)
    //   next     = min(next, heap × 95%)          (OOM safety ceiling)
    //
    // Why `max(scaled, bytes_before_gc)` handles BOTH cases perfectly:
    //
    //   CASE A — GC freed well (100GB → 60GB live, 40% freed):
    //     scaled        = 60GB × 1.5 = 90GB
    //     bytes_before  = 100GB (where GC triggered)
    //     next          = max(90GB, 100GB) = 100GB  ✅ same high-water mark
    //     → App grows freely from 60GB back to 100GB before GC fires again.
    //     → No premature GC. No wasted cycles.
    //
    //   CASE B — GC barely freed (100GB → 99GB live, 1% freed):
    //     scaled        = 99GB × 1.5 = 148.5GB
    //     bytes_before  = 100GB
    //     next          = max(148.5GB, 100GB) = 148.5GB → capped at 121GB
    //     → Threshold rises. GC backs off. If live set keeps growing,
    //       GC fires again at 121GB (the 95% ceiling), and warns user.
    //
    //   CASE C — Small app (live = 10MB after first GC):
    //     scaled        = 10MB × 1.5 = 15MB
    //     bytes_before  = 512MB (initial threshold)
    //     next          = max(15MB, 512MB) = 512MB  ✅ no micro-thrashing
    //
    //   CASE D — Large cache (live = 90GB, threshold was 121GB ceiling):
    //     scaled        = 90GB × 1.5 = 135GB → capped at 121GB
    //     bytes_before  = 121GB
    //     next          = max(121GB, 121GB) = 121GB → warns user ✅
    //
    // Growth factor is read ONCE at startup (LazyLock) — zero cost per GC cycle.
    // Configurable: TEJXGC=50 (default, 50% headroom), TEJXGC=100 (Go's default).
    // =================================================================

    let growth_factor = get_gc_growth_factor();

    // Core: proportional headroom above live set
    let scaled = (new_old_bytes as f64 * growth_factor) as usize;

    // Never lower the trigger below where it was before this GC cycle.
    // This is what prevents premature re-triggering after a successful collection.
    let high_water = bytes_before_gc;

    // Combine: whichever gives more room
    let combined = scaled.max(high_water);

    // Floor: always at least GC_MIN_HEADROOM_BYTES above live set (minimum breathing room)
    let with_headroom = new_old_bytes.saturating_add(GC_MIN_HEADROOM_BYTES);

    // Floor: absolute minimum DEFAULT_GC_TRIGGER_FLOOR_PCT% of max heap
    let absolute_floor = OLD_GEN_SIZE * DEFAULT_GC_TRIGGER_FLOOR_PCT / 100;
    let floored = combined.max(with_headroom).max(absolute_floor);

    // Ceiling: never exceed DEFAULT_GC_TRIGGER_CEIL_PCT% of heap (OOM safety)
    let ceiling = OLD_GEN_SIZE * DEFAULT_GC_TRIGGER_CEIL_PCT / 100;
    OLD_GEN_GC_THRESHOLD = floored.min(ceiling);

    // Diagnostics: warn when live set is above 80% of heap (memory pressure)
    let collection_ratio =
        bytes_before_gc.saturating_sub(new_old_bytes) as f64 / bytes_before_gc.max(1) as f64;

    if new_old_bytes > OLD_GEN_SIZE * 98 / 100 {
        eprintln!(
                "FATAL: Out of Memory! Max heap limit of {} MB exceeded (Live set reached {} MB after GC). \
                 Please increase your heap limit using --max-old-space-size=... or -Xmx...",
                OLD_GEN_SIZE / (1024 * 1024),
                new_old_bytes / (1024 * 1024)
            );
        exit(1);
    } else if new_old_bytes > OLD_GEN_SIZE * 80 / 100 {
        eprintln!(
            "[GC] WARNING: Live set {:.1}% of heap ({} MB / {} MB total). \
                 Freed {:.1}% this cycle. Next GC at {} MB. \
                 Use --max-old-space-size=... or -Xmx... to increase heap. \
                 Tune GC headroom with TEJXGC (current: {:.1}x).",
            new_old_bytes as f64 / OLD_GEN_SIZE as f64 * 100.0,
            new_old_bytes / (1024 * 1024),
            OLD_GEN_SIZE / (1024 * 1024),
            collection_ratio * 100.0,
            OLD_GEN_GC_THRESHOLD / (1024 * 1024),
            growth_factor,
        );
    }
    crate::rt_gc_cleanup_array_forward();
    GC_BACKGROUND_RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn major_gc() {
    {
        let _lock = {
            let _io = crate::ThreadIoGuard::new();
            GC_LOCK.lock().unwrap()
        };
        major_gc_locked_internal(true, false);
    }
    crate::vthread::vt_trim_stack_pool();
    FINALIZER_CONDVAR.notify_one();
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_collect() {
    major_gc();
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_collect_young() {
    minor_gc();
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_get_heap_used() -> i64 {
    (OLD_BYTES_ALLOCATED + LOS_BYTES) as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_get_heap_total() -> i64 {
    OLD_GEN_SIZE as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_get_old_gen_used() -> i64 {
    OLD_BYTES_ALLOCATED as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_get_los_used() -> i64 {
    LOS_BYTES as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_get_threshold() -> i64 {
    OLD_GEN_GC_THRESHOLD as i64
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_enable() {
    GC_ENABLED.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_disable() {
    GC_ENABLED.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_is_enabled() -> bool {
    GC_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_set_growth_factor(pct: i64) {
    if pct > 0 {
        GC_PERCENTAGE.store(pct as usize, std::sync::atomic::Ordering::SeqCst);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_gc_get_growth_factor() -> f64 {
    get_gc_growth_factor()
}

#[allow(dead_code)]
unsafe fn update_object_fields(header: *mut ObjectHeader, updater: unsafe fn(*mut i64)) {
    let type_id = (*header).type_id;
    let body_ptr = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());

    if type_id == TAG_ARRAY as u16 {
        let len = (*header).length;
        let is_ptr_array = ((*header).flags & (ARRAY_FLAG_PTR as u16)) != 0;

        if is_ptr_array {
            let data = body_ptr as *mut i64;
            for i in 0..len {
                let val = *data.add(i as usize);
                if val >= HEAP_OFFSET {
                    let body = (val - HEAP_OFFSET) as *mut u8;
                    if rt_is_gc_ptr(body) {
                        updater(data.add(i as usize));
                    }
                }
            }
        }
    } else if type_id == TAG_OBJECT as u16 {
        updater(body_ptr.add(16) as *mut i64); // keys_handle
        updater(body_ptr.add(24) as *mut i64); // values_handle
    } else if type_id == TAG_PROMISE as u16 {
        updater(body_ptr.add(8) as *mut i64); // value
        updater(body_ptr.add(16) as *mut i64); // callbacks array
    } else if type_id == TAG_FUNCTION as u16 {
        updater(body_ptr.add(8) as *mut i64); // closure env
    } else if (type_id as usize) < MAX_TYPES && TYPE_TABLE[type_id as usize].ptr_count > 0 {
        let entry = &TYPE_TABLE[type_id as usize];
        for i in 0..entry.ptr_count {
            let offset = entry.ptr_offsets[i];
            updater(body_ptr.add(offset) as *mut i64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_gc_clears_mark_bits_on_young_survivors() {
        unsafe {
            let _guard = crate::RUNTIME_TEST_LOCK.lock().unwrap();
            rt_init_gc();

            let mut holder = crate::rt_Array_constructor_v2(0, 1, 8, crate::ARRAY_FLAG_PTR);
            crate::gc::rt_push_root(&mut holder);

            let len = (LARGE_OBJECT_THRESHOLD + 1024) as i64;
            let bytes = vec![b'g'; len as usize];
            let target = crate::new_string_from_bytes(bytes.as_ptr(), len);
            crate::rt_array_set_fast(holder, 0, target);

            minor_gc();

            let holder_body = (holder - crate::HEAP_OFFSET) as *mut u8;
            assert!(holder_body >= FROM_SURVIVOR && holder_body < FROM_SURVIVOR_TOP);

            major_gc_locked_internal(false, false);
            let los_count = LOS_COUNT;
            assert_eq!(los_count, 1);

            let holder_header = rt_get_header((holder - crate::HEAP_OFFSET) as *mut u8);
            assert!(!gc_is_marked((*holder_header).gc_word));

            major_gc_locked_internal(false, false);
            let los_count = LOS_COUNT;
            assert_eq!(
                los_count, 1,
                "young survivors must not retain stale mark bits across major GCs"
            );

            crate::gc::rt_pop_roots(1);
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
        40 // Object layout: [size, capacity, keys_ptr, values_ptr, data_base]
    } else if type_id == TAG_FUNCTION as u16 {
        16 // Closure layout: [fn_ptr (8), env_ptr (8)]
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
    } else if type_id == TAG_PROMISE as u16 {
        PROMISE_BODY_SIZE
    } else {
        8
    };

    let header_size = std::mem::size_of::<ObjectHeader>();
    let total_size = body_size + header_size;
    let aligned_total = (total_size + 7) & !7;
    aligned_total - header_size
}

pub const PROMOTION_THRESHOLD: u8 = 2;

unsafe fn copy_object_with_seen(root: *mut i64, seen_stack: &mut Vec<usize>) {
    if root.is_null() || (root as usize) % 8 != 0 {
        return;
    }
    let mut val = *root;
    if val < STACK_OFFSET {
        return;
    }

    let _orig_val = val;
    let resolved_val = crate::rt_gc_resolve_array_id(val);
    if resolved_val != val {
        *root = resolved_val;
        val = resolved_val;
    }

    if val >= STACK_OFFSET && val < HEAP_OFFSET {
        eprintln!(
            "[GC ALERT] Root is STACK object: root={:p} val={:#x}",
            root, val
        );
        // Stack object: doesn't move, but we MUST scan its fields
        let body = (val - STACK_OFFSET) as *mut u8;
        let header = rt_get_header(body);
        let h_addr = header as usize;
        if seen_stack.contains(&h_addr) {
            return;
        }
        seen_stack.push(h_addr);
        scan_object_fields_with_seen(header, seen_stack);
        return;
    }

    let old_body = (val - HEAP_OFFSET) as *mut u8;
    if !in_from_space(old_body) {
        return; // Not in From-space: already in To-Survivor or Old Gen, do NOT re-copy!
    }

    let header = rt_get_header(old_body);
    // Forwarding check: bit 9 of gc_word
    if gc_is_forwarded((*header).gc_word) {
        let forwarded_header = gc_forward_ptr((*header).gc_word);
        let forwarded_body =
            (forwarded_header as u64).wrapping_add(std::mem::size_of::<ObjectHeader>() as u64);
        *root = (forwarded_body as i64) + HEAP_OFFSET;
        return;
    }

    let size_without_header = get_object_size(header);
    let total_size = size_without_header + std::mem::size_of::<ObjectHeader>();

    // Increment age (lower 8 bits of gc_word)
    let mut age = gc_get_age((*header).gc_word);
    age += 1;
    (*header).gc_word = gc_set_age((*header).gc_word, age);

    let (new_header, is_promotion) = if age >= PROMOTION_THRESHOLD
        || (TO_SURVIVOR_TOP as usize + total_size > (TO_SURVIVOR as usize + SURVIVOR_SIZE))
    {
        let ptr = if total_size < NUM_FAST_BINS * 8 {
            let bin_idx = total_size / 8;
            FAST_FREE_LIST[bin_idx]
                .lock()
                .unwrap()
                .pop()
                .map(|p| p as *mut u8)
        } else {
            LARGE_FREE_LIST
                .lock()
                .unwrap()
                .get_mut(&total_size)
                .and_then(|vec| vec.pop())
                .map(|p| p as *mut u8)
        };

        let header_ptr = if let Some(p) = ptr {
            p as *mut ObjectHeader
        } else {
            if OLD_TOP as usize + total_size > OLD_END as usize {
                eprintln!(
                    "FATAL: Out of Memory! Max heap limit of {} MB exceeded during promotion. \
                     Please increase your heap limit using --max-old-space-size=... or -Xmx...",
                    OLD_GEN_SIZE / (1024 * 1024)
                );
                exit(1);
            }
            let alloc_ptr = OLD_TOP as *mut ObjectHeader;
            OLD_TOP = OLD_TOP.add(total_size);
            alloc_ptr
        };
        OLD_BYTES_ALLOCATED += total_size;
        (header_ptr, true)
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
    (*header).gc_word = ((new_header as u64) & GC_PTR_MASK) | GC_FWD_BIT;

    *root = (new_body as i64) + HEAP_OFFSET;

    if is_promotion {
        (*new_header).gc_flags &= !FLAG_REMSET_DIRTY;
        PROMOTED_LIST.lock().unwrap().push(new_header as usize);
    } else {
        TO_SURVIVOR_TOP = TO_SURVIVOR_TOP.add(total_size);
    }
}

pub unsafe fn copy_object(root: *mut i64) {
    let mut seen_stack: Vec<usize> = Vec::new();
    copy_object_with_seen(root, &mut seen_stack);
}

#[inline]
unsafe fn copy_object_with_seen_and_track_young(
    slot: *mut i64,
    seen_stack: &mut Vec<usize>,
    has_young_refs: &mut bool,
) {
    let val = *slot;
    if val < HEAP_OFFSET {
        return;
    }

    let body = (val - HEAP_OFFSET) as *mut u8;
    if !rt_is_gc_ptr(body) {
        return;
    }

    copy_object_with_seen(slot, seen_stack);

    let updated = *slot;
    if updated < HEAP_OFFSET {
        return;
    }

    let updated_body = (updated - HEAP_OFFSET) as *mut u8;
    if in_young_gen(updated_body) {
        *has_young_refs = true;
    }
}

unsafe fn scan_object_fields_with_seen(header: *mut ObjectHeader, seen_stack: &mut Vec<usize>) {
    let type_id = (*header).type_id;
    let body_ptr = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());

    if type_id == TAG_ARRAY as u16 {
        let len = (*header).length;
        let is_ptr_array = ((*header).flags & (ARRAY_FLAG_PTR as u16)) != 0;

        if is_ptr_array {
            let data = body_ptr as *mut i64;
            for i in 0..len {
                let val = *data.add(i as usize);
                if val >= HEAP_OFFSET {
                    let body = (val - HEAP_OFFSET) as *mut u8;
                    if rt_is_gc_ptr(body) {
                        copy_object_with_seen(data.add(i as usize), seen_stack);
                    }
                }
            }
        }
    } else if type_id == TAG_OBJECT as u16 {
        copy_object_with_seen(body_ptr.add(16) as *mut i64, seen_stack);
        copy_object_with_seen(body_ptr.add(24) as *mut i64, seen_stack);
    } else if type_id == TAG_FUNCTION as u16 {
        // Closure environment root
        copy_object_with_seen(body_ptr.add(8) as *mut i64, seen_stack);
    } else if type_id == TAG_PROMISE as u16 {
        copy_object_with_seen(body_ptr.add(8) as *mut i64, seen_stack);
        copy_object_with_seen(body_ptr.add(16) as *mut i64, seen_stack);
    } else if (type_id as usize) < MAX_TYPES && TYPE_TABLE[type_id as usize].ptr_count > 0 {
        let entry = &TYPE_TABLE[type_id as usize];
        for i in 0..entry.ptr_count {
            let offset = entry.ptr_offsets[i];
            copy_object_with_seen(body_ptr.add(offset) as *mut i64, seen_stack);
        }
    }
}

unsafe fn scan_object_fields_minor_with_seen(
    header: *mut ObjectHeader,
    seen_stack: &mut Vec<usize>,
) -> bool {
    let type_id = (*header).type_id;
    let body_ptr = (header as *mut u8).add(std::mem::size_of::<ObjectHeader>());
    let mut has_young_refs = false;

    if type_id == TAG_ARRAY as u16 {
        let len = (*header).length;
        let is_ptr_array = ((*header).flags & (ARRAY_FLAG_PTR as u16)) != 0;

        if is_ptr_array {
            let data = body_ptr as *mut i64;
            for i in 0..len {
                copy_object_with_seen_and_track_young(
                    data.add(i as usize),
                    seen_stack,
                    &mut has_young_refs,
                );
            }
        }
    } else if type_id == TAG_OBJECT as u16 {
        copy_object_with_seen_and_track_young(
            body_ptr.add(16) as *mut i64,
            seen_stack,
            &mut has_young_refs,
        );
        copy_object_with_seen_and_track_young(
            body_ptr.add(24) as *mut i64,
            seen_stack,
            &mut has_young_refs,
        );
    } else if type_id == TAG_FUNCTION as u16 {
        // Closure environment root
        copy_object_with_seen_and_track_young(
            body_ptr.add(8) as *mut i64,
            seen_stack,
            &mut has_young_refs,
        );
    } else if type_id == TAG_PROMISE as u16 {
        copy_object_with_seen_and_track_young(
            body_ptr.add(8) as *mut i64,
            seen_stack,
            &mut has_young_refs,
        );
        copy_object_with_seen_and_track_young(
            body_ptr.add(16) as *mut i64,
            seen_stack,
            &mut has_young_refs,
        );
    } else if (type_id as usize) < MAX_TYPES && TYPE_TABLE[type_id as usize].ptr_count > 0 {
        let entry = &TYPE_TABLE[type_id as usize];
        for i in 0..entry.ptr_count {
            let offset = entry.ptr_offsets[i];
            copy_object_with_seen_and_track_young(
                body_ptr.add(offset) as *mut i64,
                seen_stack,
                &mut has_young_refs,
            );
        }
    }

    has_young_refs
}

unsafe fn scan_object_fields_minor(header: *mut ObjectHeader) -> bool {
    let mut seen_stack: Vec<usize> = Vec::new();
    scan_object_fields_minor_with_seen(header, &mut seen_stack)
}

unsafe fn scan_object_fields(header: *mut ObjectHeader) {
    let mut seen_stack: Vec<usize> = Vec::new();
    scan_object_fields_with_seen(header, &mut seen_stack);
}

#[inline]
#[allow(dead_code)]
unsafe fn run_finalizer_for_header(header: *mut ObjectHeader) {
    let type_id = (*header).type_id as usize;
    if type_id >= MAX_TYPES {
        return;
    }
    if let Some(finalizer) = TYPE_TABLE[type_id].finalizer {
        let obj_val =
            ((header as *mut u8).add(std::mem::size_of::<ObjectHeader>()) as i64) + HEAP_OFFSET;
        finalizer(obj_val);
    }
}

#[allow(dead_code)]
unsafe fn run_young_finalizers_in_region(mut scan: *mut u8, end: *mut u8) {
    while scan < end {
        let header = scan as *mut ObjectHeader;
        let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();

        if size <= std::mem::size_of::<ObjectHeader>() {
            scan = scan.add(std::mem::size_of::<ObjectHeader>());
            continue;
        }

        if !gc_is_forwarded((*header).gc_word) {
            let type_id = (*header).type_id as usize;
            if type_id < MAX_TYPES {
                if let Some(f) = TYPE_TABLE[type_id].finalizer {
                    if ((*header).gc_flags & FLAG_FINALIZED) == 0 {
                        (*header).gc_flags |= FLAG_FINALIZED;
                        // Resurrect object
                        let mut root: i64 =
                            ((header as *mut u8).add(std::mem::size_of::<ObjectHeader>()) as i64)
                                + HEAP_OFFSET;
                        copy_object(&mut root as *mut i64);
                        {
                            let mut queue = FINALIZER_QUEUE.lock().unwrap();
                            queue.push((f, root));
                        }
                    }
                }
            }
        }
        scan = scan.add(size);
    }
}

#[no_mangle]
pub unsafe extern "C" fn minor_gc() {
    {
        let _lock = {
            let _io = crate::ThreadIoGuard::new();
            GC_LOCK.lock().unwrap()
        };
        trigger_safepoint();
        minor_gc_locked();
        resume_safepoint();
    }
    FINALIZER_CONDVAR.notify_one();
}

unsafe fn trigger_safepoint() {
    // No need to reset SAFEPOINT_RESUME — it's an epoch counter now.
    // Threads wait for the epoch to change, not for a boolean.
    SAFEPOINT_REQUEST.store(true, Ordering::SeqCst);
    {
        let (lock, cvar) = &**SAFEPOINT_ACK;
        let mut count = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *count = 0;
        let mut wait_loops = 0;
        loop {
            // Check if all threads are safe. We must drop the THREAD_REGISTRY
            // lock before waiting, otherwise threads trying to register/unregister
            // will deadlock.
            let all_safe = {
                let registry = match THREAD_REGISTRY.lock() {
                    Ok(g) => g,
                    Err(e) => e.into_inner(),
                };
                let total = registry.len();
                let mut safe_count = 0;

                MY_CONTEXT.with(|ctx| {
                    let ctx_ptr = (*ctx.get()).as_mut() as *mut ThreadContext;
                    for &ctx_wrapper in registry.iter() {
                        let other_ctx_ptr = ctx_wrapper.0;
                        if other_ctx_ptr == ctx_ptr {
                            safe_count += 1;
                        } else if (*other_ctx_ptr).in_blocking_io.load(Ordering::SeqCst) {
                            safe_count += 1;
                        } else if (*other_ctx_ptr).in_safepoint.load(Ordering::SeqCst) {
                            safe_count += 1;
                        }
                    }
                });

                if safe_count >= total {
                    true
                } else {
                    if wait_loops > 500 {
                        eprintln!(
                            "FATAL: GC safepoint timeout! safe_count={}, total={}",
                            safe_count, total
                        );
                        MY_CONTEXT.with(|ctx| {
                            let ctx_ptr = (*ctx.get()).as_mut() as *mut ThreadContext;
                            for &ctx_wrapper in registry.iter() {
                                let other_ctx_ptr = ctx_wrapper.0;
                                if other_ctx_ptr != ctx_ptr {
                                    eprintln!(
                                        "  Thread {:p}: in_blocking_io={}, in_safepoint={}",
                                        other_ctx_ptr,
                                        (*other_ctx_ptr).in_blocking_io.load(Ordering::SeqCst),
                                        (*other_ctx_ptr).in_safepoint.load(Ordering::SeqCst)
                                    );
                                }
                            }
                        });
                        std::process::abort();
                    }
                    false
                }
                // registry lock is dropped here
            };

            if all_safe {
                break;
            }

            let result = cvar.wait_timeout(count, std::time::Duration::from_millis(10));
            count = match result {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
            wait_loops += 1;
        }
    }
}

unsafe fn resume_safepoint() {
    SAFEPOINT_REQUEST.store(false, Ordering::SeqCst);
    {
        // Increment the epoch to wake all waiting threads.
        // Each thread captured the epoch before entering safepoint,
        // so they'll see the new value and exit their wait loop.
        let (lock, cvar) = &**SAFEPOINT_RESUME;
        let mut epoch = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *epoch += 1;
        cvar.notify_all();
    }
    // Reset ACK counter for next GC
    {
        let (lock, _) = &**SAFEPOINT_ACK;
        let mut l = match lock.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *l = 0;
    }
}

pub unsafe fn minor_gc_locked() {
    // minor_gc_locked: called under GC lock, evacuates young gen
    crate::rt_gc_prepare_array_forward();
    clear_all_tlabs();
    let _eden_top = EDEN_TOP.load(std::sync::atomic::Ordering::SeqCst);
    let _from_survivor_top = FROM_SURVIVOR_TOP;
    TO_SURVIVOR_TOP = TO_SURVIVOR;
    let mut survivor_scan_ptr = TO_SURVIVOR;
    let mut promoted_scan_idx = 0;
    let mut new_remset = Vec::new();

    // 1. Scan roots
    {
        let registry = lock_registry();
        for &ctx_wrapper in registry.iter() {
            let ctx_ptr = ctx_wrapper.0;
            let top = (*ctx_ptr).roots_top;
            for i in 0..top {
                copy_object((*ctx_ptr).roots[i]);
            }
        }
    }
    copy_static_roots();
    super::rt_gc_scan_tasks();
    {
        let mut queue = FINALIZER_QUEUE.lock().unwrap();
        for (_, obj_val) in queue.iter_mut() {
            copy_object(obj_val as *mut i64);
        }
    }
    crate::rt_gc_scan_array_forward_roots();

    // 1b. Scan RemSets
    {
        let registry = lock_registry();
        for &ctx_wrapper in registry.iter() {
            let ctx = &*ctx_wrapper.0;
            let mut remset = ctx.remset.lock().unwrap();
            let mut next_remset = Vec::new();
            for &obj_ptr in remset.iter() {
                let body = obj_ptr as *mut u8;
                let header = rt_get_header(body);
                // Temporarily clear dirty flag so new mutations can be caught
                (*header).gc_flags &= !FLAG_REMSET_DIRTY;
                if scan_object_fields_minor(header) {
                    (*header).gc_flags |= FLAG_REMSET_DIRTY;
                    next_remset.push(obj_ptr);
                }
            }
            *remset = next_remset;
        }
    }

    // 1c. Scan all LOS objects (they can store references to Young Gen)
    for (obj_ptr, _) in los_snapshot() {
        let header = obj_ptr as *mut ObjectHeader;
        scan_object_fields(header);
    }

    // 2. Trace both survivor objects and any young objects promoted into Old Gen.
    // Promoted containers can themselves hold references to more young objects, so keep
    // walking both frontiers until the collection reaches a fixed point.
    loop {
        let mut progressed = false;

        while survivor_scan_ptr < TO_SURVIVOR_TOP {
            let header = survivor_scan_ptr as *mut ObjectHeader;
            scan_object_fields(header);
            let size = get_object_size(header) + std::mem::size_of::<ObjectHeader>();
            survivor_scan_ptr = survivor_scan_ptr.add(size);
            progressed = true;
        }

        loop {
            let newly_promoted = {
                let list = PROMOTED_LIST.lock().unwrap();
                if promoted_scan_idx < list.len() {
                    let ptr = list[promoted_scan_idx] as *mut u8;
                    promoted_scan_idx += 1;
                    Some(ptr)
                } else {
                    None
                }
            };
            if let Some(promoted_ptr) = newly_promoted {
                let header = promoted_ptr as *mut ObjectHeader;
                if scan_object_fields_minor(header) {
                    (*header).gc_flags |= FLAG_REMSET_DIRTY;
                    let body_ptr =
                        (promoted_ptr as *mut u8).add(std::mem::size_of::<ObjectHeader>());
                    new_remset.push(body_ptr);
                }
                progressed = true;
            } else {
                break;
            }
        }

        if !progressed {
            break;
        }
    }

    // Swap survivors
    let temp = FROM_SURVIVOR;
    FROM_SURVIVOR = TO_SURVIVOR;
    TO_SURVIVOR = temp;
    FROM_SURVIVOR_TOP = TO_SURVIVOR_TOP;

    EDEN_TOP.store(EDEN_START, std::sync::atomic::Ordering::SeqCst);

    // Removed CARD_TABLE reset
    if !new_remset.is_empty() {
        let registry = lock_registry();
        if let Some(&ctx_wrapper) = registry.iter().next() {
            let ctx = &*ctx_wrapper.0;
            ctx.remset.lock().unwrap().extend(new_remset);
        }
    }
    PROMOTED_LIST.lock().unwrap().clear();
}
