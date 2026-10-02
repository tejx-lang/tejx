use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use once_cell::sync::Lazy;
use std::cell::{Cell, RefCell};
use std::sync::Mutex;
use std::thread;
use mio::{Events, Poll, Registry, Token, Interest};
use crate::constants::*;
use crate::context::{init_fiber_stack, tejx_context_switch};
use crate::SpinMutex;

/// Set at startup by runtime CLI arguments (`--vt-stack`, `--vthread-stack`, `-Xss`).
pub static mut ARGV_VT_STACK_SIZE: usize = 0;

#[no_mangle]
pub unsafe extern "C" fn rt_set_default_vthread_stack_size(size: usize) {
    ARGV_VT_STACK_SIZE = size.max(MIN_VTHREAD_STACK_SIZE);
}

#[no_mangle]
pub unsafe extern "C" fn rt_get_default_vthread_stack_size() -> usize {
    get_vt_stack_size()
}

#[no_mangle]
pub unsafe extern "C" fn rt_vthread_count() -> i64 {
    NEXT_VT_ID.load(Ordering::Relaxed) as i64
}

pub fn get_vt_stack_size() -> usize {
    let argv_size = unsafe { ARGV_VT_STACK_SIZE };
    if argv_size > 0 {
        return argv_size.max(MIN_VTHREAD_STACK_SIZE);
    }
    if let Ok(val) = std::env::var(ENV_VT_STACK) {
        if let Some(size) = crate::gc::parse_size_str(&val) {
            return size.max(MIN_VTHREAD_STACK_SIZE);
        } else if let Ok(size) = val.parse::<usize>() {
            return size.max(MIN_VTHREAD_STACK_SIZE);
        }
    }
    DEFAULT_VTHREAD_STACK_SIZE
}

pub fn get_main_vt_stack_size() -> usize {
    if let Ok(val) = std::env::var(ENV_MAIN_STACK) {
        if let Some(size) = crate::gc::parse_size_str(&val) {
            return size.max(64 * 1024);
        } else if let Ok(size) = val.parse::<usize>() {
            return size.max(64 * 1024);
        }
    }
    DEFAULT_MAIN_THREAD_STACK_SIZE
}

/// Go-like growable stack for TejX virtual threads using mmap.
///
/// How it works (identical concept to Go goroutines):
/// 1. Reserve MAX_VTHREAD_STACK_SIZE (1 MB) of virtual address space via mmap(PROT_NONE).
///    This costs ZERO physical memory — it's just address space reservation.
/// 2. Commit only the initial portion (1 page = 16 KB on ARM64) at the TOP of the range
///    by mprotect'ing it to PROT_READ|PROT_WRITE.
/// 3. The uncommitted pages below act as automatic guard pages.
/// 4. When SP grows into the guard region → SIGSEGV → signal handler commits more pages
///    → execution resumes transparently. The stack "grows" without copying or moving.
/// 5. Maximum growth is capped at MAX_VTHREAD_STACK_SIZE (1 MB) to prevent runaway recursion.
///
/// Memory layout (stack grows downward, high → low address):
///
///   [base]     ← highest address, SP starts here
///   |  committed pages (initially 1 page, grows on demand)  |
///   [committed_low] ← lowest committed address
///   |  guard pages (PROT_NONE, triggers SIGSEGV on access)  |
///   [limit]    ← lowest address of the reserved region
///
pub struct GrowableStack {
    /// Lowest address of the entire mmap'd region (PROT_NONE initially).
    mmap_base: *mut u8,
    /// Total size of the mmap'd region (= MAX_VTHREAD_STACK_SIZE).
    mmap_size: usize,
    /// Lowest committed (PROT_READ|PROT_WRITE) address. 
    /// Everything from committed_low..base is accessible.
    /// Everything from mmap_base..committed_low is guard (PROT_NONE).
    committed_low: Arc<AtomicUsize>,
    /// OS page size cached for growth operations.
    page_size: usize,
}

unsafe impl Send for GrowableStack {}
unsafe impl Sync for GrowableStack {}

/// Global registry of live growable stacks so the SIGSEGV / SIGBUS handler can find
/// which stack a fault address belongs to and grow it on demand.
use std::sync::atomic::AtomicPtr;
static STACK_REGISTRY_PTR: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static STACK_REGISTRY_INIT: std::sync::Once = std::sync::Once::new();

struct StackRegistryEntry {
    mmap_base: usize,             // lowest address of reserved region
    mmap_end: usize,              // highest address (mmap_base + mmap_size)
    committed_low: Arc<AtomicUsize>, // current committed boundary (atomic for signal handler access)
    page_size: usize,
    max_committed_low: usize,     // = mmap_base (absolute bottom, can't grow past this)
}

struct StackRegistry {
    entries: SpinMutex<Vec<StackRegistryEntry>>,
}

fn get_stack_registry() -> &'static StackRegistry {
    STACK_REGISTRY_INIT.call_once(|| {
        let registry = Box::new(StackRegistry {
            entries: SpinMutex::new(Vec::with_capacity(1024)),
        });
        STACK_REGISTRY_PTR.store(Box::into_raw(registry) as *mut (), Ordering::Release);
    });
    unsafe { &*(STACK_REGISTRY_PTR.load(Ordering::Acquire) as *const StackRegistry) }
}

/// Called from the SIGSEGV / SIGBUS signal handler (async-signal-safe).
/// Returns true if the fault address was in a growable stack's guard region
/// and the stack was successfully grown.
#[inline(never)]
pub unsafe fn try_grow_stack_at(fault_addr: usize) -> bool {
    let ptr = STACK_REGISTRY_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        return false;
    }
    let registry = &*(ptr as *const StackRegistry);
    // Non-blocking try_lock with a small spin to avoid signal-handler deadlock
    let mut entries = None;
    for _ in 0..100 {
        if let Some(guard) = registry.entries.try_lock() {
            entries = Some(guard);
            break;
        }
        std::hint::spin_loop();
    }
    let entries = match entries {
        Some(guard) => guard,
        None => return false,
    };
    
    for entry in entries.iter() {
        if fault_addr >= entry.mmap_base && fault_addr < entry.mmap_end {
            // This fault is within this stack's reserved region.
            let current_low = entry.committed_low.load(Ordering::Acquire);
            if fault_addr >= current_low {
                // Fault in already committed region — this is a real memory error, not stack growth
                return false;
            }
            // Fault is below committed_low → need to grow.
            // Grow in chunks of at least 16 KB (or page_size if larger) to minimize page faults.
            let chunk_size = entry.page_size.max(16 * 1024);
            let mut new_low = current_low;
            while new_low > fault_addr && new_low > entry.max_committed_low {
                new_low = new_low.saturating_sub(chunk_size);
            }
            if new_low < entry.max_committed_low {
                new_low = entry.max_committed_low;
            }
            if new_low >= current_low {
                // Can't grow anymore — true stack overflow
                return false;
            }
            // Commit the new pages
            let grow_size = current_low - new_low;
            let result = libc::mprotect(
                new_low as *mut libc::c_void,
                grow_size,
                libc::PROT_READ | libc::PROT_WRITE,
            );
            if result != 0 {
                return false;
            }
            entry.committed_low.store(new_low, Ordering::Release);
            return true;
        }
    }
    false
}

fn os_page_size() -> usize {
    unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize }
}

/// Round up to the nearest multiple of page_size.
fn page_align_up(size: usize, page_size: usize) -> usize {
    (size + page_size - 1) & !(page_size - 1)
}

impl GrowableStack {
    /// Allocate a new growable stack.
    /// `initial_commit` is the amount of usable stack space initially committed.
    /// The total reserved virtual address space is MAX_VTHREAD_STACK_SIZE.
    pub fn new(initial_commit: usize) -> Result<Self, ()> {
        let page_size = os_page_size();
        let total_reserved = page_align_up(MAX_VTHREAD_STACK_SIZE, page_size);
        let initial = page_align_up(initial_commit.max(page_size), page_size);
        
        // Step 1: Reserve the full virtual address space with PROT_NONE (no physical memory used)
        let mmap_base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                total_reserved,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if mmap_base == libc::MAP_FAILED {
            return Err(());
        }
        let mmap_base = mmap_base as *mut u8;
        
        // Step 2: Commit the top `initial` bytes (stack grows downward, so commit the high end)
        // Layout: [mmap_base ... committed_low ... base(=mmap_base+total_reserved)]
        let committed_low_ptr = unsafe { mmap_base.add(total_reserved - initial) };
        let result = unsafe {
            libc::mprotect(
                committed_low_ptr as *mut libc::c_void,
                initial,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };
        if result != 0 {
            unsafe { libc::munmap(mmap_base as *mut libc::c_void, total_reserved); }
            return Err(());
        }
        
        let committed_low = Arc::new(AtomicUsize::new(committed_low_ptr as usize));

        // Step 3: Register in the global stack registry for signal handler lookup
        let registry = get_stack_registry();
        let entry = StackRegistryEntry {
            mmap_base: mmap_base as usize,
            mmap_end: mmap_base as usize + total_reserved,
            committed_low: Arc::clone(&committed_low),
            page_size,
            max_committed_low: mmap_base as usize, // can grow all the way down
        };
        registry.entries.lock().push(entry);
        
        Ok(GrowableStack {
            mmap_base,
            mmap_size: total_reserved,
            committed_low,
            page_size,
        })
    }
    
    /// Returns the committed capacity (usable stack space).
    #[inline]
    pub fn capacity(&self) -> usize {
        (self.base() as usize).saturating_sub(self.committed_low.load(Ordering::Acquire))
    }

    /// Base = highest address of the stack (SP starts here).
    #[inline]
    pub fn base(&self) -> *mut u8 {
        unsafe { self.mmap_base.add(self.mmap_size) }
    }

    /// Limit = lowest address of the reserved region.
    #[inline]
    pub fn limit(&self) -> *mut u8 {
        self.mmap_base
    }
    
    /// Check canary — for GrowableStack, always true (guard pages handle overflow).
    #[inline]
    pub fn check_canary(&self) -> bool {
        true // mmap guard pages replace canary checking
    }
    
    /// Reset canary — no-op for GrowableStack.
    #[inline]
    pub fn reset_canary(&self) {}
    
    /// Decommit extra pages to save memory when returning to pool.
    /// Keeps only `keep_committed` bytes committed at the top.
    fn shrink_to(&mut self, keep_committed: usize) {
        let current_committed_low = self.committed_low.load(Ordering::Acquire);
        let keep = page_align_up(keep_committed.max(self.page_size), self.page_size);
        let new_committed_low = (self.mmap_base as usize) + self.mmap_size - keep;
        
        if new_committed_low > current_committed_low {
            // Decommit pages below the new boundary
            let decommit_size = new_committed_low - current_committed_low;
            unsafe {
                // MADV_DONTNEED releases physical pages back to the OS without unmapping
                libc::madvise(
                    current_committed_low as *mut libc::c_void,
                    decommit_size,
                    libc::MADV_DONTNEED,
                );
                // Re-protect as PROT_NONE so they act as guard pages again
                libc::mprotect(
                    current_committed_low as *mut libc::c_void,
                    decommit_size,
                    libc::PROT_NONE,
                );
            }
            self.committed_low.store(new_committed_low, Ordering::Release);
        }
    }
}

impl Drop for GrowableStack {
    fn drop(&mut self) {
        // Unregister from the stack registry
        let registry = get_stack_registry();
        {
            let mut entries = registry.entries.lock();
            let mmap_base = self.mmap_base as usize;
            if let Some(pos) = entries.iter().position(|e| e.mmap_base == mmap_base) {
                entries.swap_remove(pos);
            }
        }
        // Release all virtual address space back to the OS
        unsafe {
            libc::munmap(self.mmap_base as *mut libc::c_void, self.mmap_size);
        }
    }
}

pub struct SlabStack {
    ptr: *mut u8,
    size: usize,
}
unsafe impl Send for SlabStack {}

impl SlabStack {
    #[inline]
    pub fn new(size: usize) -> Self {
        let layout = std::alloc::Layout::from_size_align(size, STACK_ALIGNMENT).unwrap();
        let ptr = unsafe { std::alloc::alloc(layout) };
        if ptr.is_null() {
            panic!("Out of memory allocating stack slab");
        }
        unsafe {
            *(ptr as *mut u64) = STACK_CANARY_MAGIC;
        }
        SlabStack { ptr, size }
    }

    #[inline(always)]
    pub fn base(&self) -> *mut u8 {
        unsafe { self.ptr.add(self.size) }
    }

    #[inline(always)]
    pub fn limit(&self) -> *mut u8 {
        self.ptr
    }

    #[inline(always)]
    pub fn check_canary(&self) -> bool {
        unsafe { *(self.ptr as *const u64) == STACK_CANARY_MAGIC }
    }

    #[inline(always)]
    pub fn reset_canary(&self) {
        unsafe {
            *(self.ptr as *mut u64) = STACK_CANARY_MAGIC;
        }
    }
}

impl Drop for SlabStack {
    fn drop(&mut self) {
        let layout = std::alloc::Layout::from_size_align(self.size, STACK_ALIGNMENT).unwrap();
        unsafe {
            std::alloc::dealloc(self.ptr, layout);
        }
    }
}

pub enum StackKind {
    Slab(SlabStack),
    Growable(GrowableStack),
}

pub struct PooledStack {
    inner: Option<StackKind>,
}

impl PooledStack {
    #[inline(always)]
    pub fn base(&self) -> *mut u8 {
        match self.inner.as_ref().unwrap() {
            StackKind::Slab(s) => s.base(),
            StackKind::Growable(g) => g.base(),
        }
    }

    #[inline(always)]
    pub fn limit(&self) -> *mut u8 {
        match self.inner.as_ref().unwrap() {
            StackKind::Slab(s) => s.limit(),
            StackKind::Growable(g) => g.limit(),
        }
    }

    #[inline(always)]
    pub fn check_canary(&self) -> bool {
        match self.inner.as_ref().unwrap() {
            StackKind::Slab(s) => s.check_canary(),
            StackKind::Growable(_) => true,
        }
    }
}

thread_local! {
    static SLAB_POOL: RefCell<Vec<SlabStack>> = RefCell::new(Vec::new());
    static GROWABLE_POOL: RefCell<Vec<GrowableStack>> = RefCell::new(Vec::new());
}

static GLOBAL_SLAB_POOL: SpinMutex<Vec<SlabStack>> = SpinMutex::new(Vec::new());
static GLOBAL_GROWABLE_POOL: SpinMutex<Vec<GrowableStack>> = SpinMutex::new(Vec::new());

impl Drop for PooledStack {
    fn drop(&mut self) {
        if let Some(kind) = self.inner.take() {
            match kind {
                StackKind::Slab(s) => {
                    s.reset_canary();
                    let _ = SLAB_POOL.try_with(|pool| {
                        pool.borrow_mut().push(s);
                    });
                }
                StackKind::Growable(mut g) => {
                    g.shrink_to(os_page_size());
                    let _ = GROWABLE_POOL.try_with(|pool| {
                        pool.borrow_mut().push(g);
                    });
                }
            }
        }
    }
}

pub fn vt_trim_stack_pool() {
    let _ = SLAB_POOL.try_with(|pool| {
        pool.borrow_mut().clear();
    });
    let _ = GROWABLE_POOL.try_with(|pool| {
        pool.borrow_mut().clear();
    });
    GLOBAL_SLAB_POOL.lock().clear();
    GLOBAL_GROWABLE_POOL.lock().clear();
}

fn acquire_stack(stack_size: usize) -> PooledStack {
    let target_size = stack_size.max(get_vt_stack_size());
    let growable = GROWABLE_POOL.try_with(|pool| pool.borrow_mut().pop()).unwrap_or(None);
    if let Some(g) = growable {
        return PooledStack { inner: Some(StackKind::Growable(g)) };
    }
    {
        let mut global = GLOBAL_GROWABLE_POOL.lock();
        if let Some(g) = global.pop() {
            return PooledStack { inner: Some(StackKind::Growable(g)) };
        }
    }
    let stack = GrowableStack::new(target_size)
        .expect("Failed to allocate growable stack for virtual thread");
    PooledStack { inner: Some(StackKind::Growable(stack)) }
}

static START_INSTANT: Lazy<std::time::Instant> = Lazy::new(std::time::Instant::now);

#[inline]
pub fn current_time_ms() -> u64 {
    START_INSTANT.elapsed().as_millis() as u64
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct YieldReason(u64);

impl YieldReason {
    pub const TAG_COOPERATIVE: u64 = 0;
    pub const TAG_IOPARK_READ: u64 = 1;
    pub const TAG_SLEEP: u64 = 2;
    pub const TAG_PARK: u64 = 3;
    pub const TAG_IOPARK_WRITE: u64 = 4;
    pub const PAYLOAD_MASK: u64 = (1 << 60) - 1;

    #[inline(always)]
    pub fn cooperative() -> Self {
        YieldReason(Self::TAG_COOPERATIVE << 60)
    }

    #[inline(always)]
    pub fn io_park_read(token: usize) -> Self {
        YieldReason((Self::TAG_IOPARK_READ << 60) | ((token as u64) & Self::PAYLOAD_MASK))
    }

    #[inline(always)]
    pub fn io_park_write(token: usize) -> Self {
        YieldReason((Self::TAG_IOPARK_WRITE << 60) | ((token as u64) & Self::PAYLOAD_MASK))
    }

    #[inline(always)]
    pub fn io_park(token: usize) -> Self {
        Self::io_park_read(token)
    }

    #[inline(always)]
    pub fn sleep(until_ms: u64) -> Self {
        YieldReason((Self::TAG_SLEEP << 60) | (until_ms & Self::PAYLOAD_MASK))
    }

    #[inline(always)]
    pub fn park(token: usize) -> Self {
        YieldReason((Self::TAG_PARK << 60) | ((token as u64) & Self::PAYLOAD_MASK))
    }

    #[inline(always)]
    pub fn tag(&self) -> u64 {
        self.0 >> 60
    }

    #[inline(always)]
    pub fn is_cooperative(&self) -> bool {
        self.tag() == Self::TAG_COOPERATIVE
    }

    #[inline(always)]
    pub fn as_sleep(&self) -> Option<u64> {
        if self.tag() == Self::TAG_SLEEP {
            Some(self.0 & Self::PAYLOAD_MASK)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_io_park_read(&self) -> Option<usize> {
        if self.tag() == Self::TAG_IOPARK_READ {
            Some((self.0 & Self::PAYLOAD_MASK) as usize)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_io_park_write(&self) -> Option<usize> {
        if self.tag() == Self::TAG_IOPARK_WRITE {
            Some((self.0 & Self::PAYLOAD_MASK) as usize)
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn as_io_park(&self) -> Option<usize> {
        self.as_io_park_read()
    }

    #[inline(always)]
    pub fn as_park(&self) -> Option<usize> {
        if self.tag() == Self::TAG_PARK {
            Some((self.0 & Self::PAYLOAD_MASK) as usize)
        } else {
            None
        }
    }
}

pub struct VThread {
    id: usize,
    _stack: PooledStack,
    sp: *mut u8,
    worker_sp: *mut u8,
    done: bool,
    entry: Option<Box<dyn FnOnce() + Send + 'static>>,
    yield_reason: YieldReason,
    gc_state: Arc<SpinMutex<Option<crate::gc::GcContextState>>>,
    local_state: crate::VThreadLocalState,
    slot_live: Arc<AtomicBool>,
}

unsafe impl Send for VThread {}

const NUM_GC_SHARDS: usize = 16;
static NEXT_VT_ID: AtomicUsize = AtomicUsize::new(1);
static VTHREAD_GC_SHARDS: Lazy<Vec<SpinMutex<std::collections::HashMap<usize, Arc<SpinMutex<Option<crate::gc::GcContextState>>>>>>> =
    Lazy::new(|| (0..NUM_GC_SHARDS).map(|_| SpinMutex::new(std::collections::HashMap::new())).collect());

fn register_vthread_gc(id: usize, gc_state: Arc<SpinMutex<Option<crate::gc::GcContextState>>>) {
    let shard_idx = id % NUM_GC_SHARDS;
    VTHREAD_GC_SHARDS[shard_idx].lock().insert(id, gc_state);
}

fn unregister_vthread_gc(id: usize) {
    let shard_idx = id % NUM_GC_SHARDS;
    VTHREAD_GC_SHARDS[shard_idx].lock().remove(&id);
}

#[inline]
fn lock_gc_state(m: &SpinMutex<Option<crate::gc::GcContextState>>) -> crate::mutex::SpinMutexGuard<'_, Option<crate::gc::GcContextState>> {
    m.lock()
}

impl Drop for VThread {
    fn drop(&mut self) {
        unregister_vthread_gc(self.id);
    }
}

pub unsafe fn vt_gc_scan_roots_minor() {
    for shard in VTHREAD_GC_SHARDS.iter() {
        let registry = shard.lock();
        for gc_arc in registry.values() {
            let mut guard = gc_arc.lock();
            if let Some(ref mut state) = *guard {
                for root in &state.roots {
                    crate::gc::copy_object(*root as *mut i64);
                }
            }
        }
    }
}

pub unsafe fn vt_gc_mark_roots_major() {
    for shard in VTHREAD_GC_SHARDS.iter() {
        let registry = shard.lock();
        for gc_arc in registry.values() {
            let mut guard = gc_arc.lock();
            if let Some(ref mut state) = *guard {
                for root in &state.roots {
                    crate::gc::mark_object(*root as *mut i64);
                }
            }
        }
    }
}

pub unsafe fn vt_gc_update_roots() {
    for shard in VTHREAD_GC_SHARDS.iter() {
        let registry = shard.lock();
        for gc_arc in registry.values() {
            let mut guard = gc_arc.lock();
            if let Some(ref mut state) = *guard {
                for root in &state.roots {
                    crate::gc::rt_update_ptr(*root as *mut i64);
                }
            }
        }
    }
}

const NUM_IO_SHARDS: usize = 256;

struct IoShardData {
    parked_read: std::collections::HashMap<usize, Box<VThread>>,
    parked_write: std::collections::HashMap<usize, Box<VThread>>,
    ready_read: std::collections::HashSet<usize>,
    ready_write: std::collections::HashSet<usize>,
}

impl IoShardData {
    fn new() -> Self {
        Self {
            parked_read: std::collections::HashMap::new(),
            parked_write: std::collections::HashMap::new(),
            ready_read: std::collections::HashSet::new(),
            ready_write: std::collections::HashSet::new(),
        }
    }
}

struct IoShard {
    data: SpinMutex<IoShardData>,
}

impl IoShard {
    fn new() -> Self {
        Self {
            data: SpinMutex::new(IoShardData::new()),
        }
    }
}

const NUM_PARK_SHARDS: usize = 256;

struct ParkShard {
    parked: std::collections::HashMap<usize, Box<VThread>>,
    unparked: std::collections::HashSet<usize>,
}

impl ParkShard {
    fn new() -> Self {
        Self {
            parked: std::collections::HashMap::new(),
            unparked: std::collections::HashSet::new(),
        }
    }

    fn take_unparked(&mut self, token: usize) -> bool {
        self.unparked.remove(&token)
    }

    fn pop_parked(&mut self, token: usize) -> Option<Box<VThread>> {
        self.parked.remove(&token)
    }
}

static PARK_SHARDS: Lazy<Vec<SpinMutex<ParkShard>>> = Lazy::new(|| {
    (0..NUM_PARK_SHARDS).map(|_| {
        SpinMutex::new(ParkShard::new())
    }).collect()
});

static NEXT_PARK_TOKEN: AtomicUsize = AtomicUsize::new(1);

pub fn vt_next_park_token() -> usize {
    NEXT_PARK_TOKEN.fetch_add(1, Ordering::Relaxed)
}

struct Scheduler {
    priority_queue: Injector<Box<VThread>>,
    global_queue: Injector<Box<VThread>>,
    stealers: once_cell::sync::OnceCell<Vec<Stealer<Box<VThread>>>>,
    io_shards: Vec<IoShard>,
    registry: Registry,
    next_token: AtomicUsize,
    timers: SpinMutex<Vec<(u64, Box<VThread>)>>,
    idle_workers: AtomicUsize,
    idle_condvar: std::sync::Condvar,
    idle_lock: std::sync::Mutex<()>,
}

static POLL: Lazy<Mutex<Option<Poll>>> = Lazy::new(|| Mutex::new(Some(Poll::new().unwrap())));

static SCHEDULER: Lazy<Scheduler> = Lazy::new(|| Scheduler {
    priority_queue: Injector::new(),
    global_queue: Injector::new(),
    stealers: once_cell::sync::OnceCell::new(),
    io_shards: (0..NUM_IO_SHARDS).map(|_| IoShard::new()).collect(),
    registry: POLL.lock().unwrap().as_ref().unwrap().registry().try_clone().unwrap(),
    next_token: AtomicUsize::new(1),
    timers: SpinMutex::new(Vec::new()),
    idle_workers: AtomicUsize::new(0),
    idle_condvar: std::sync::Condvar::new(),
    idle_lock: std::sync::Mutex::new(()),
});

impl Scheduler {
    #[inline(always)]
    pub fn notify_worker(&self) {
        let idle = self.idle_workers.load(Ordering::Relaxed);
        if idle > 1 {
            self.idle_condvar.notify_all();
        } else {
            self.idle_condvar.notify_one();
        }
    }

    #[inline(always)]
    pub fn push_priority(&self, vt: Box<VThread>) {
        self.priority_queue.push(vt);
        self.notify_worker();
    }

    #[inline(always)]
    pub fn push_global(&self, vt: Box<VThread>) {
        self.global_queue.push(vt);
        self.notify_worker();
    }

    pub fn add_timer(&self, until: u64, vt: Box<VThread>) {
        let mut timers = self.timers.lock();
        timers.push((until, vt));
        drop(timers);
        self.notify_worker();
    }

    pub fn next_timer_timeout(&self) -> Option<std::time::Duration> {
        let now = current_time_ms();
        let timers = self.timers.lock();
        if timers.is_empty() {
            return Some(std::time::Duration::from_millis(10));
        }
        let mut min_until = None;
        for (until, _) in timers.iter() {
            min_until = match min_until {
                None => Some(*until),
                Some(m) if *until < m => Some(*until),
                Some(m) => Some(m),
            };
        }
        if let Some(target) = min_until {
            if target <= now {
                Some(std::time::Duration::from_millis(0))
            } else {
                Some(std::time::Duration::from_millis((target - now).min(10)))
            }
        } else {
            Some(std::time::Duration::from_millis(10))
        }
    }

    pub fn drain_expired_timers(&self) {
        let now = current_time_ms();
        let mut expired = Vec::new();
        {
            let mut timers = self.timers.lock();
            let mut i = 0;
            while i < timers.len() {
                if timers[i].0 <= now {
                    let (_, vt) = timers.swap_remove(i);
                    expired.push(vt);
                } else {
                    i += 1;
                }
            }
        }
        for vt in expired {
            self.push_global(vt);
        }
    }
}

#[derive(Copy, Clone)]
struct VThreadContext {
    current: *mut VThread,
    preempt_tick: u32,
    preempt_last_ms: u64,
}

thread_local! {
    static LOCAL_WORKER: Cell<*const Worker<Box<VThread>>> = Cell::new(std::ptr::null());
    static VTHREAD_CTX: Cell<VThreadContext> = Cell::new(VThreadContext {
        current: std::ptr::null_mut(),
        preempt_tick: 0,
        preempt_last_ms: 0,
    });
    static STEAL_RNG: Cell<u32> = Cell::new(123456789);
}

pub fn vt_init(num_workers: usize) {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    // Force SCHEDULER, PARK_SHARDS, and VTHREAD_GC_SHARDS initialization on OS stack
    let _ = SCHEDULER.next_token.load(Ordering::SeqCst);
    let _ = PARK_SHARDS.len();
    let _ = VTHREAD_GC_SHARDS.len();
    
    let mut stealers = Vec::with_capacity(num_workers);
    let mut workers = Vec::with_capacity(num_workers);
    for _ in 0..num_workers {
        let w = Worker::new_fifo();
        stealers.push(w.stealer());
        workers.push(w);
    }
    let _ = SCHEDULER.stealers.set(stealers);

    for (i, worker) in workers.into_iter().enumerate() {
        thread::Builder::new()
            .name(format!("tejx-worker-{}", i))
            .stack_size(2 * 1024 * 1024)
            .spawn(move || worker_loop(i, worker))
            .unwrap();
    }
}

pub fn start_netpoller() {
    thread::Builder::new()
        .name("tejx-netpoller".into())
        .spawn(move || {
            #[cfg(unix)]
            unsafe {
                libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            }
            let mut poll = POLL.lock().unwrap().take().unwrap();
            let mut events = Events::with_capacity(8192);
            loop {
                let timeout = SCHEDULER.next_timer_timeout();
                let _ = poll.poll(&mut events, timeout);
                for event in events.iter() {
                    let token_id = event.token().0;
                    let shard_idx = token_id % NUM_IO_SHARDS;
                    let shard = &SCHEDULER.io_shards[shard_idx];
                    let mut data = shard.data.lock();

                    let is_readable = event.is_readable() || event.is_read_closed();
                    let is_writable = event.is_writable() || event.is_write_closed();

                    if is_readable {
                        if let Some(vt) = data.parked_read.remove(&token_id) {
                            SCHEDULER.push_priority(vt);
                        } else {
                            data.ready_read.insert(token_id);
                        }
                    }
                    if is_writable {
                        if let Some(vt) = data.parked_write.remove(&token_id) {
                            SCHEDULER.push_priority(vt);
                        } else {
                            data.ready_write.insert(token_id);
                        }
                    }
                }
                SCHEDULER.drain_expired_timers();
            }
        }).unwrap();
}

pub fn vt_register_io<S: mio::event::Source>(source: &mut S) -> usize {
    let token_id = SCHEDULER.next_token.fetch_add(1, Ordering::SeqCst);
    let token = Token(token_id);
    let _ = SCHEDULER.registry.register(
        source,
        token,
        Interest::READABLE | Interest::WRITABLE
    );
    token_id
}

pub fn vt_register_io_read<S: mio::event::Source>(source: &mut S) -> usize {
    let token_id = SCHEDULER.next_token.fetch_add(1, Ordering::SeqCst);
    let token = Token(token_id);
    let _ = SCHEDULER.registry.register(
        source,
        token,
        Interest::READABLE
    );
    token_id
}

pub fn vt_deregister_io<S: mio::event::Source>(source: &mut S, token_id: usize) {
    let _ = SCHEDULER.registry.deregister(source);
    let shard_idx = token_id % NUM_IO_SHARDS;
    let shard = &SCHEDULER.io_shards[shard_idx];
    let mut data = shard.data.lock();
    data.ready_read.remove(&token_id);
    data.ready_write.remove(&token_id);
    data.parked_read.remove(&token_id);
    data.parked_write.remove(&token_id);
}

#[inline(never)]
unsafe fn vthread_terminate(vt_ptr: *mut VThread) -> ! {
    if !vt_ptr.is_null() {
        let vt = &mut *vt_ptr;
        vt.done = true;
        let worker_sp = vt.worker_sp;
        if !worker_sp.is_null() {
            let mut dummy_sp: *mut u8 = std::ptr::null_mut();
            tejx_context_switch(&mut dummy_sp, worker_sp);
        } else {
            eprintln!("[VT_ABORT] worker_sp is NULL for vt id={}", vt.id);
        }
    } else {
        eprintln!("[VT_ABORT] passed vt_ptr is NULL!");
    }

    std::process::abort();
}

extern "C" fn vthread_entry_trampoline() -> ! {
    let vt_ptr = VTHREAD_CTX.with(|c| c.get().current);
    if vt_ptr.is_null() {
        std::process::abort();
    }
    let entry = unsafe { (&mut *vt_ptr).entry.take() };

    if let Some(f) = entry {
        f();
    }

    unsafe {
        vthread_terminate(vt_ptr);
    }
}

#[inline(never)]
pub unsafe fn vthread_suspend(reason: YieldReason) {
    let vt_ptr = VTHREAD_CTX.with(|c| c.get().current);
    if vt_ptr.is_null() {
        return;
    }
    let vt = &mut *vt_ptr;
    vt.yield_reason = reason;

    let worker_sp = vt.worker_sp;
    if worker_sp.is_null() {
        return;
    }

    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    tejx_context_switch(&mut vt.sp, worker_sp);
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
}

#[inline(never)]
pub fn vt_wait_io_read(token_id: usize) {
    let shard_idx = token_id % NUM_IO_SHARDS;
    let shard = &SCHEDULER.io_shards[shard_idx];
    {
        let mut data = shard.data.lock();
        if data.ready_read.remove(&token_id) {
            return;
        }
    }
    if vt_is_vthread() {
        unsafe { vthread_suspend(YieldReason::io_park_read(token_id)) };
    } else {
        unsafe {
            if crate::gc::is_safepoint_requested() {
                crate::gc::rt_safepoint_poll_slow();
            }
        }
        thread::yield_now();
    }
}

#[inline(never)]
pub fn vt_wait_io_write(token_id: usize) {
    let shard_idx = token_id % NUM_IO_SHARDS;
    let shard = &SCHEDULER.io_shards[shard_idx];
    {
        let mut data = shard.data.lock();
        if data.ready_write.remove(&token_id) {
            return;
        }
    }
    if vt_is_vthread() {
        unsafe { vthread_suspend(YieldReason::io_park_write(token_id)) };
    } else {
        unsafe {
            if crate::gc::is_safepoint_requested() {
                crate::gc::rt_safepoint_poll_slow();
            }
        }
        thread::yield_now();
    }
}

#[inline(never)]
pub fn vt_wait_io(token_id: usize) {
    vt_wait_io_read(token_id);
}

fn worker_loop(worker_id: usize, local: Worker<Box<VThread>>) {
    unsafe { crate::gc::rt_register_thread(); }
    let local_box = Box::new(local);
    let local_ptr: *const Worker<Box<VThread>> = &*local_box;
    LOCAL_WORKER.with(|w| w.set(local_ptr));

    unsafe {
        (*crate::gc::current_thread_context()).in_blocking_io.store(true, Ordering::SeqCst);
    }

    loop {
        let mut task = None;
        loop {
            match SCHEDULER.priority_queue.steal() {
                Steal::Success(t) => {
                    task = Some(t);
                    break;
                }
                Steal::Empty => break,
                Steal::Retry => continue,
            }
        }

        if task.is_none() {
            loop {
                match SCHEDULER.global_queue.steal() {
                    Steal::Success(t) => {
                        task = Some(t);
                        break;
                    }
                    Steal::Empty => break,
                    Steal::Retry => continue,
                }
            }
        }

        if task.is_none() {
            SCHEDULER.drain_expired_timers();
            loop {
                match SCHEDULER.priority_queue.steal() {
                    Steal::Success(t) => {
                        task = Some(t);
                        break;
                    }
                    Steal::Empty => break,
                    Steal::Retry => continue,
                }
            }
            if task.is_none() {
                loop {
                    match SCHEDULER.global_queue.steal() {
                        Steal::Success(t) => {
                            task = Some(t);
                            break;
                        }
                        Steal::Empty => break,
                        Steal::Retry => continue,
                    }
                }
            }
        }

        if let Some(mut t) = task {
            unsafe {
                // Ensure worker has no leftover roots from previous tasks before entering safepoints
                let ctx = crate::gc::current_thread_context();
                (*ctx).roots_top = 0;
                (*ctx).in_blocking_io.store(false, Ordering::SeqCst);
                if crate::gc::is_safepoint_requested() {
                    crate::gc::rt_safepoint_poll_slow();
                }
            }

            if !t.done {
                let saved_gc = lock_gc_state(&t.gc_state).take();
                if let Some(state) = saved_gc {
                    unsafe { crate::gc::rt_restore_gc_context(state); }
                } else {
                    unsafe { (*crate::gc::current_thread_context()).roots_top = 0; }
                }

                crate::restore_vthread_local_state(&mut t.local_state);

                let vt_raw: *mut VThread = &mut *t;
                VTHREAD_CTX.with(|cell| {
                    cell.set(VThreadContext {
                        current: vt_raw,
                        preempt_tick: 0,
                        preempt_last_ms: current_time_ms(),
                    });
                });

                unsafe {
                    tejx_context_switch(&mut (*vt_raw).worker_sp, (*vt_raw).sp);
                }

                VTHREAD_CTX.with(|cell| {
                    cell.set(VThreadContext {
                        current: std::ptr::null_mut(),
                        preempt_tick: 0,
                        preempt_last_ms: 0,
                    });
                });

                if !t.done {
                    let r = t.yield_reason;
                    *lock_gc_state(&t.gc_state) = Some(unsafe { crate::gc::rt_save_gc_context() });
                    crate::save_vthread_local_state(&mut t.local_state);
                    if r.is_cooperative() {
                        SCHEDULER.push_global(t);
                    } else if let Some(until) = r.as_sleep() {
                        SCHEDULER.add_timer(until, t);
                    } else if let Some(token_id) = r.as_io_park_read() {
                        let shard_idx = token_id % NUM_IO_SHARDS;
                        let shard = &SCHEDULER.io_shards[shard_idx];
                        let mut data = shard.data.lock();
                        if data.ready_read.remove(&token_id) {
                            drop(data);
                            SCHEDULER.push_priority(t);
                        } else {
                            data.parked_read.insert(token_id, t);
                        }
                    } else if let Some(token_id) = r.as_io_park_write() {
                        let shard_idx = token_id % NUM_IO_SHARDS;
                        let shard = &SCHEDULER.io_shards[shard_idx];
                        let mut data = shard.data.lock();
                        if data.ready_write.remove(&token_id) {
                            drop(data);
                            SCHEDULER.push_priority(t);
                        } else {
                            data.parked_write.insert(token_id, t);
                        }
                    } else if let Some(token) = r.as_park() {
                        let shard_idx = token % NUM_PARK_SHARDS;
                        let mut shard = PARK_SHARDS[shard_idx].lock();
                        if shard.take_unparked(token) {
                            drop(shard);
                            SCHEDULER.push_global(t);
                        } else {
                            shard.parked.insert(token, t);
                        }
                    }
                } else {
                    *lock_gc_state(&t.gc_state) = None;
                    crate::clear_vthread_local_state();
                    unsafe { (*crate::gc::current_thread_context()).roots_top = 0; }
                    let slot_live = t.slot_live.clone();
                    drop(t);
                    slot_live.store(false, Ordering::SeqCst);
                }
            } else {
                *lock_gc_state(&t.gc_state) = None;
                crate::clear_vthread_local_state();
                unsafe { (*crate::gc::current_thread_context()).roots_top = 0; }
                let slot_live = t.slot_live.clone();
                drop(t);
                slot_live.store(false, Ordering::SeqCst);
            }

            unsafe {
                crate::gc::rt_clear_tlab();
                let ctx = crate::gc::current_thread_context();
                (*ctx).in_blocking_io.store(true, Ordering::SeqCst);
                if crate::gc::is_safepoint_requested() {
                    (*ctx).in_blocking_io.store(false, Ordering::SeqCst);
                    crate::gc::rt_safepoint_poll_slow();
                    (*ctx).in_blocking_io.store(true, Ordering::SeqCst);
                }
            }
        } else {
            SCHEDULER.idle_workers.fetch_add(1, Ordering::SeqCst);
            // Spin briefly before sleeping — keeps wakeup latency sub-microsecond
            // under load while being well-behaved when idle.
            let mut found = false;
            for _ in 0..128 {
                if !SCHEDULER.priority_queue.is_empty() || !SCHEDULER.global_queue.is_empty() {
                    found = true;
                    break;
                }
                std::hint::spin_loop();
            }
            if !found {
                SCHEDULER.drain_expired_timers();
                // Wait on condvar — wakes up in <2µs when a task is pushed to global_queue!
                let guard = SCHEDULER.idle_lock.lock().unwrap();
                let _ = SCHEDULER.idle_condvar.wait_timeout(guard, std::time::Duration::from_millis(1));
            }
            SCHEDULER.idle_workers.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub fn vt_spawn_closure<F>(f: F, cb_slot: usize, slot_live: Arc<AtomicBool>)
where
    F: FnOnce() + Send + 'static,
{
    vt_spawn_closure_with_stack(f, cb_slot, slot_live, get_vt_stack_size())
}

pub fn vt_spawn_closure_with_stack<F>(f: F, _cb_slot: usize, slot_live: Arc<AtomicBool>, stack_size: usize)
where
    F: FnOnce() + Send + 'static,
{
    let id = NEXT_VT_ID.fetch_add(1, Ordering::SeqCst);
    let gc_state = Arc::new(SpinMutex::new(None));
    register_vthread_gc(id, gc_state.clone());

    let stack = acquire_stack(stack_size);
    let initial_sp = unsafe { init_fiber_stack(stack.base(), vthread_entry_trampoline) };

    let vt = Box::new(VThread {
        id,
        _stack: stack,
        sp: initial_sp,
        worker_sp: std::ptr::null_mut(),
        done: false,
        entry: Some(Box::new(f)),
        yield_reason: YieldReason::cooperative(),
        gc_state,
        local_state: crate::VThreadLocalState::default(),
        slot_live,
    });

    SCHEDULER.push_global(vt);
}

#[inline(never)]
pub fn vt_sleep(ms: u64) {
    if vt_is_vthread() {
        let until = current_time_ms().saturating_add(ms).saturating_add(1);
        unsafe { vthread_suspend(YieldReason::sleep(until)) };
    } else {
        let _guard = crate::ThreadIoGuard::new();
        let start_ms = unsafe { crate::rt_time_now_ms() };
        let target_ms = start_ms.saturating_add(ms as i64);
        loop {
            let now_ms = unsafe { crate::rt_time_now_ms() };
            if now_ms >= target_ms {
                break;
            }
            let diff = (target_ms - now_ms).max(1) as u64;
            std::thread::sleep(std::time::Duration::from_millis(diff));
        }
    }
}

pub fn vt_join(slot_live: &Arc<AtomicBool>) {
    if vt_is_vthread() {
        while slot_live.load(Ordering::Acquire) {
            vt_yield();
        }
    } else {
        let _guard = crate::ThreadIoGuard::new();
        while slot_live.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
    }
}

#[inline(never)]
pub fn vt_yield() {
    if vt_is_vthread() {
        unsafe { vthread_suspend(YieldReason::cooperative()) };
    } else {
        thread::yield_now();
    }
}

#[inline(never)]
pub fn vt_park(token: usize) {
    let shard_idx = token % NUM_PARK_SHARDS;
    {
        let mut shard = PARK_SHARDS[shard_idx].lock();
        if shard.take_unparked(token) {
            return;
        }
    }
    if vt_is_vthread() {
        unsafe { vthread_suspend(YieldReason::park(token)) };
    } else {
        thread::yield_now();
    }
}

pub fn vt_unpark(token: usize) {
    let shard_idx = token % NUM_PARK_SHARDS;
    let mut shard = PARK_SHARDS[shard_idx].lock();
    if let Some(vt) = shard.pop_parked(token) {
        drop(shard);
        SCHEDULER.push_global(vt);
    } else {
        shard.unparked.insert(token);
    }
}

pub fn vt_with_syscall<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}

#[inline]
pub fn vt_is_vthread() -> bool {
    VTHREAD_CTX.with(|c| !c.get().current.is_null())
}

#[inline]
pub fn vt_preempt_tick() {
    VTHREAD_CTX.with(|cell| {
        let mut ctx = cell.get();
        if ctx.current.is_null() {
            return;
        }
        let count = ctx.preempt_tick.wrapping_add(1);
        ctx.preempt_tick = count;
        if (count & 0x3fff) != 0 {
            cell.set(ctx);
            return;
        }
        let now = current_time_ms();
        if now.saturating_sub(ctx.preempt_last_ms) >= 10 {
            ctx.preempt_last_ms = now;
            cell.set(ctx);
            vt_yield();
        } else {
            cell.set(ctx);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_growable_stack_allocation_and_alignment() {
        let stack = GrowableStack::new(DEFAULT_VTHREAD_STACK_SIZE).expect("allocate growable stack");
        assert!(stack.capacity() >= DEFAULT_VTHREAD_STACK_SIZE);
        assert_eq!(stack.base() as usize % STACK_ALIGNMENT, 0);
        assert_eq!(stack.limit() as usize % STACK_ALIGNMENT, 0);
        assert!(stack.base() > stack.limit());
        assert_eq!(stack.base() as usize - stack.limit() as usize, MAX_VTHREAD_STACK_SIZE);
    }

    #[test]
    fn test_native_context_switch_roundtrip() {
        static mut COMPLETED: bool = false;
        static mut TEST_WORKER_SP: *mut u8 = std::ptr::null_mut();
        unsafe { COMPLETED = false; }
        let stack = acquire_stack(DEFAULT_VTHREAD_STACK_SIZE);

        extern "C" fn test_fiber_entry() -> ! {
            unsafe {
                COMPLETED = true;
                let worker_sp = TEST_WORKER_SP;
                let mut dummy_sp: *mut u8 = std::ptr::null_mut();
                tejx_context_switch(&mut dummy_sp, worker_sp);
                unreachable!();
            }
        }

        let initial_sp = unsafe { init_fiber_stack(stack.base(), test_fiber_entry) };

        unsafe {
            tejx_context_switch(&raw mut TEST_WORKER_SP, initial_sp);
        }

        assert!(unsafe { COMPLETED });
    }

    #[test]
    fn test_native_context_switch_yield_and_resume() {
        static mut STEP: usize = 0;
        static mut TEST_WORKER_SP: *mut u8 = std::ptr::null_mut();
        static mut FIBER_SP: *mut u8 = std::ptr::null_mut();
        unsafe { STEP = 0; }
        let stack = acquire_stack(DEFAULT_VTHREAD_STACK_SIZE);

        extern "C" fn test_yield_entry() -> ! {
            unsafe {
                STEP = 1;
                tejx_context_switch(&raw mut FIBER_SP, TEST_WORKER_SP);

                // Resumed!
                STEP = 2;
                let mut dummy_sp: *mut u8 = std::ptr::null_mut();
                tejx_context_switch(&mut dummy_sp, TEST_WORKER_SP);
                unreachable!();
            }
        }

        let initial_sp = unsafe { init_fiber_stack(stack.base(), test_yield_entry) };

        // First switch to fiber
        unsafe {
            tejx_context_switch(&raw mut TEST_WORKER_SP, initial_sp);
        }
        assert_eq!(unsafe { STEP }, 1);

        // Resume fiber
        unsafe {
            tejx_context_switch(&raw mut TEST_WORKER_SP, FIBER_SP);
        }
        assert_eq!(unsafe { STEP }, 2);
    }

    #[test]
    fn test_growable_stack_growth_and_shrink() {
        let mut stack = GrowableStack::new(DEFAULT_VTHREAD_STACK_SIZE).expect("allocate growable stack");
        let initial_cap = stack.capacity();
        assert!(initial_cap >= DEFAULT_VTHREAD_STACK_SIZE);

        // Simulate stack growth via fault address below committed_low
        let current_low = stack.committed_low.load(Ordering::Acquire);
        let fault_addr = current_low - 8;
        let grew = unsafe { try_grow_stack_at(fault_addr) };
        assert!(grew, "try_grow_stack_at should succeed for guard page access");

        let new_cap = stack.capacity();
        assert!(new_cap > initial_cap, "stack capacity should have increased");

        // Verify the newly grown page is actually readable/writable without faulting
        unsafe {
            let ptr = fault_addr as *mut u64;
            ptr.write(0x1234_5678_9ABC_DEF0);
            assert_eq!(ptr.read(), 0x1234_5678_9ABC_DEF0);
        }

        // Test shrink_to releases the page and resets committed_low
        stack.shrink_to(DEFAULT_VTHREAD_STACK_SIZE);
        assert_eq!(stack.capacity(), initial_cap);
    }
}
