use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use corosensei::{Coroutine, Yielder};
use corosensei::stack::{Stack, StackPointer, MIN_STACK_SIZE, STACK_ALIGNMENT};
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use once_cell::sync::Lazy;
use std::cell::RefCell;
use std::sync::{Mutex, RwLock, Condvar};
use std::thread;
use std::collections::{HashMap, HashSet};
use mio::{Events, Poll, Registry, Token, Interest};
use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;

/// 128 KB dynamic baseline stack footprint (physical RAM committed by OS only on demand).
/// Provides ample headroom for large stack frames, JSON parsing, regex, and deep framework call chains.
/// User can customize down to 4 KB (MIN_STACK_SIZE) via TEJX_VT_STACK=4096.
pub const DEFAULT_VTHREAD_STACK_SIZE: usize = 128 * 1024;

pub fn get_vt_stack_size() -> usize {
    if let Ok(val) = std::env::var("TEJX_VT_STACK") {
        if let Ok(size) = val.parse::<usize>() {
            return size.max(MIN_STACK_SIZE);
        }
    }
    DEFAULT_VTHREAD_STACK_SIZE
}

const STACK_CANARY_MAGIC: u64 = 0xDEAD_BEEF_CAFE_BABE;

/// Truly dynamic heap-allocated stack for TejX virtual threads.
///
/// Unlike OS-backed mmap stacks (which require 32 KB minimum on Apple Silicon due to 16 KB pages)
/// and involve kernel syscalls (`mmap`, `mprotect`, `munmap`), `DynamicStack` is:
/// 1. Allocated directly from the heap with 16-byte alignment (matching Go's user-space mcache stack allocation).
/// 2. Sized dynamically (default 8 KB, down to 4 KB, or dynamically configured).
/// 3. Protected with an inline canary magic number at the limit address to catch stack overflows.
/// 4. Reused across virtual threads via thread-local caches with 0 kernel syscalls and 0 lock contention.
pub struct DynamicStack {
    ptr: NonNull<u8>,
    layout: Layout,
}

unsafe impl Send for DynamicStack {}
unsafe impl Sync for DynamicStack {}

impl DynamicStack {
    pub fn new(size: usize) -> Result<Self, ()> {
        let size = size.max(MIN_STACK_SIZE);
        let aligned_size = (size + STACK_ALIGNMENT - 1) & !(STACK_ALIGNMENT - 1);
        let layout = Layout::from_size_align(aligned_size, STACK_ALIGNMENT).map_err(|_| ())?;
        let raw = unsafe { alloc(layout) };
        if raw.is_null() {
            return Err(());
        }
        let ptr = NonNull::new(raw).ok_or(())?;
        // Plant stack canary at the bottom limit of the stack (lowest memory address)
        unsafe {
            (raw as *mut u64).write(STACK_CANARY_MAGIC);
        }
        Ok(Self { ptr, layout })
    }

    #[inline]
    pub fn check_canary(&self) -> bool {
        unsafe {
            (self.ptr.as_ptr() as *const u64).read() == STACK_CANARY_MAGIC
        }
    }

    #[inline]
    pub fn reset_canary(&self) {
        unsafe {
            (self.ptr.as_ptr() as *mut u64).write(STACK_CANARY_MAGIC);
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.layout.size()
    }
}

impl Drop for DynamicStack {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

unsafe impl Stack for DynamicStack {
    #[inline]
    fn base(&self) -> StackPointer {
        // Base is the highest address since stacks grow downwards towards limit
        let top = self.ptr.as_ptr() as usize + self.layout.size();
        StackPointer::new(top).expect("valid base stack pointer")
    }

    #[inline]
    fn limit(&self) -> StackPointer {
        // Limit is the lowest address
        StackPointer::new(self.ptr.as_ptr() as usize).expect("valid limit stack pointer")
    }
}

static STACK_POOL: Lazy<Mutex<Vec<DynamicStack>>> = Lazy::new(|| Mutex::new(Vec::with_capacity(1024)));

pub struct PooledStack {
    inner: Option<DynamicStack>,
}

unsafe impl Stack for PooledStack {
    #[inline]
    fn base(&self) -> StackPointer {
        self.inner.as_ref().unwrap().base()
    }
    #[inline]
    fn limit(&self) -> StackPointer {
        self.inner.as_ref().unwrap().limit()
    }
}

impl PooledStack {
    #[inline]
    pub fn check_canary(&self) -> bool {
        self.inner.as_ref().map(|s| s.check_canary()).unwrap_or(true)
    }
}

impl Drop for PooledStack {
    fn drop(&mut self) {
        if let Some(stack) = self.inner.take() {
            if !stack.check_canary() {
                eprintln!(
                    "[tejx-runtime] CRITICAL: Virtual thread stack canary violated! Stack size: {} bytes. Please increase TEJX_VT_STACK.",
                    stack.capacity()
                );
                // Do not recycle corrupted stack; let it deallocate
                return;
            }
            stack.reset_canary();
            let overflow = LOCAL_STACK_CACHE.with(|cache| {
                let mut c = cache.borrow_mut();
                if c.len() < 32 {
                    c.push(stack);
                    None
                } else {
                    let drain_n = c.len() / 2;
                    let mut drained: Vec<_> = c.drain(..drain_n).collect();
                    drained.push(stack);
                    Some(drained)
                }
            });
            if let Some(drained) = overflow {
                if let Ok(mut pool) = STACK_POOL.lock() {
                    if pool.len() < 512 {
                        pool.extend(drained);
                    }
                }
            }
        }
    }
}

pub fn vt_trim_stack_pool() {
    if let Ok(mut pool) = STACK_POOL.lock() {
        if pool.len() > 64 {
            pool.truncate(64);
        }
    }
}

fn acquire_stack(stack_size: usize) -> PooledStack {
    let stack = LOCAL_STACK_CACHE.with(|cache| {
        let mut c = cache.borrow_mut();
        if let Some(pos) = c.iter().position(|s| s.capacity() >= stack_size) {
            Some(c.remove(pos))
        } else {
            None
        }
    })
    .or_else(|| {
        if let Ok(mut pool) = STACK_POOL.lock() {
            if let Some(pos) = pool.iter().position(|s| s.capacity() >= stack_size) {
                Some(pool.remove(pos))
            } else {
                None
            }
        } else {
            None
        }
    })
    .unwrap_or_else(|| {
        DynamicStack::new(stack_size).unwrap_or_else(|_| DynamicStack::new(MIN_STACK_SIZE).unwrap())
    });
    PooledStack { inner: Some(stack) }
}

pub enum YieldReason {
    Cooperative,
    IoPark(usize),
    Sleep(std::time::Instant),
}

pub struct VThread {
    id: usize,
    coro: Coroutine<(), YieldReason, (), PooledStack>,
    gc_state: Arc<Mutex<Option<crate::gc::GcContextState>>>,
    local_state: Option<crate::VThreadLocalState>,
    yielder_ptr: Arc<AtomicUsize>,
}

unsafe impl Send for VThread {}

const NUM_GC_SHARDS: usize = 16;
static NEXT_VT_ID: AtomicUsize = AtomicUsize::new(1);
static VTHREAD_GC_SHARDS: Lazy<[Mutex<HashMap<usize, Arc<Mutex<Option<crate::gc::GcContextState>>>>>; NUM_GC_SHARDS]> =
    Lazy::new(|| std::array::from_fn(|_| Mutex::new(HashMap::new())));

fn register_vthread_gc(id: usize, gc_state: Arc<Mutex<Option<crate::gc::GcContextState>>>) {
    let shard_idx = id % NUM_GC_SHARDS;
    if let Ok(mut shard) = VTHREAD_GC_SHARDS[shard_idx].lock() {
        shard.insert(id, gc_state);
    }
}

fn unregister_vthread_gc(id: usize) {
    let shard_idx = id % NUM_GC_SHARDS;
    if let Ok(mut shard) = VTHREAD_GC_SHARDS[shard_idx].lock() {
        shard.remove(&id);
    }
}

impl Drop for VThread {
    fn drop(&mut self) {
        unregister_vthread_gc(self.id);
    }
}

pub unsafe fn vt_gc_scan_roots_minor() {
    for shard in VTHREAD_GC_SHARDS.iter() {
        let registry = match shard.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (_id, gc_arc) in registry.iter() {
            if let Ok(mut guard) = gc_arc.lock() {
                if let Some(ref mut state) = *guard {
                    for root in &state.roots {
                        crate::gc::copy_object(*root as *mut i64);
                    }
                }
            }
        }
    }
}

pub unsafe fn vt_gc_mark_roots_major() {
    for shard in VTHREAD_GC_SHARDS.iter() {
        let registry = match shard.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (_id, gc_arc) in registry.iter() {
            if let Ok(mut guard) = gc_arc.lock() {
                if let Some(ref mut state) = *guard {
                    for root in &state.roots {
                        crate::gc::mark_object(*root as *mut i64);
                    }
                }
            }
        }
    }
}

pub unsafe fn vt_gc_update_roots() {
    for shard in VTHREAD_GC_SHARDS.iter() {
        let registry = match shard.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (_id, gc_arc) in registry.iter() {
            if let Ok(mut guard) = gc_arc.lock() {
                if let Some(ref mut state) = *guard {
                    for root in &state.roots {
                        crate::gc::rt_update_ptr(*root as *mut i64);
                    }
                }
            }
        }
    }
}

const NUM_IO_SHARDS: usize = 16;

struct IoShardData {
    parked: HashMap<usize, Box<VThread>>,
    ready_tokens: HashSet<usize>,
}

struct IoShard {
    data: Mutex<IoShardData>,
}

impl IoShard {
    fn new() -> Self {
        Self {
            data: Mutex::new(IoShardData {
                parked: HashMap::new(),
                ready_tokens: HashSet::new(),
            }),
        }
    }
}

struct Scheduler {
    global_queue: Injector<Box<VThread>>,
    stealers: RwLock<Vec<Stealer<Box<VThread>>>>,
    io_shards: [IoShard; NUM_IO_SHARDS],
    registry: Registry,
    next_token: AtomicUsize,
    timers: Mutex<Vec<(std::time::Instant, Box<VThread>)>>,
    park_lock: Mutex<()>,
    park_cvar: Condvar,
    idle_workers: AtomicUsize,
}

static POLL: Lazy<Mutex<Option<Poll>>> = Lazy::new(|| Mutex::new(Some(Poll::new().unwrap())));

static SCHEDULER: Lazy<Scheduler> = Lazy::new(|| Scheduler {
    global_queue: Injector::new(),
    stealers: RwLock::new(Vec::new()),
    io_shards: std::array::from_fn(|_| IoShard::new()),
    registry: POLL.lock().unwrap().as_ref().unwrap().registry().try_clone().unwrap(),
    next_token: AtomicUsize::new(1),
    timers: Mutex::new(Vec::new()),
    park_lock: Mutex::new(()),
    park_cvar: Condvar::new(),
    idle_workers: AtomicUsize::new(0),
});

impl Scheduler {
    pub fn notify_worker(&self) {
        self.park_cvar.notify_one();
    }

    pub fn push_global(&self, vt: Box<VThread>) {
        self.global_queue.push(vt);
        self.notify_worker();
    }

    pub fn add_timer(&self, until: std::time::Instant, vt: Box<VThread>) {
        if let Ok(mut timers) = self.timers.lock() {
            timers.push((until, vt));
        }
    }

    pub fn next_timer_timeout(&self) -> Option<std::time::Duration> {
        let now = std::time::Instant::now();
        if let Ok(timers) = self.timers.lock() {
            if timers.is_empty() {
                return None;
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
                    Some(target.duration_since(now).min(std::time::Duration::from_millis(50)))
                }
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn drain_expired_timers(&self) {
        let now = std::time::Instant::now();
        let mut expired = Vec::new();
        if let Ok(mut timers) = self.timers.lock() {
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

thread_local! {
    static LOCAL_WORKER: RefCell<Option<Worker<Box<VThread>>>> = RefCell::new(None);
    static CURRENT_YIELDER: std::cell::Cell<Option<*const Yielder<(), YieldReason>>> = std::cell::Cell::new(None);
    static LOCAL_STACK_CACHE: RefCell<Vec<DynamicStack>> = RefCell::new(Vec::with_capacity(32));
    static STEAL_RNG: std::cell::Cell<u32> = std::cell::Cell::new(123456789);
}

pub fn vt_init(num_workers: usize) {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    // Force SCHEDULER initialization before start_netpoller takes the Poll instance
    let _ = SCHEDULER.next_token.load(Ordering::SeqCst);
    
    for i in 0..num_workers {
        thread::Builder::new()
            .name(format!("tejx-worker-{}", i))
            .spawn(move || worker_loop())
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
            let mut events = Events::with_capacity(1024);
            loop {
                let timeout = SCHEDULER.next_timer_timeout();
                let _ = poll.poll(&mut events, timeout);
                for event in events.iter() {
                    let token_id = event.token().0;
                    let shard_idx = token_id % NUM_IO_SHARDS;
                    let shard = &SCHEDULER.io_shards[shard_idx];
                    let mut data = shard.data.lock().unwrap();
                    if let Some(vt) = data.parked.remove(&token_id) {
                        drop(data);
                        SCHEDULER.push_global(vt);
                    } else {
                        data.ready_tokens.insert(token_id);
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

pub fn vt_deregister_io<S: mio::event::Source>(source: &mut S, token_id: usize) {
    let _ = SCHEDULER.registry.deregister(source);
    let shard_idx = token_id % NUM_IO_SHARDS;
    let shard = &SCHEDULER.io_shards[shard_idx];
    let mut data = shard.data.lock().unwrap();
    data.ready_tokens.remove(&token_id);
    data.parked.remove(&token_id);
}

pub fn vt_wait_io(token_id: usize) {
    let shard_idx = token_id % NUM_IO_SHARDS;
    let shard = &SCHEDULER.io_shards[shard_idx];
    {
        let mut data = shard.data.lock().unwrap();
        if data.ready_tokens.remove(&token_id) {
            return;
        }
    }
    let yielder_ptr = CURRENT_YIELDER.with(|y| y.get());
    if let Some(ptr) = yielder_ptr {
        unsafe { (*ptr).suspend(YieldReason::IoPark(token_id)) };
    } else {
        unsafe { crate::gc::rt_safepoint_poll(); }
        thread::yield_now();
    }
}

fn worker_loop() {
    unsafe { crate::gc::rt_register_thread(); }
    let local = Worker::new_fifo();
    {
        SCHEDULER.stealers.write().unwrap().push(local.stealer());
    }
    LOCAL_WORKER.with(|w| *w.borrow_mut() = Some(local));

    let ctx_ptr = unsafe { crate::gc::current_thread_context() };
    unsafe {
        (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
    }

    loop {
        let mut task = None;

        // 1. Try local queue
        LOCAL_WORKER.with(|w| {
            if let Some(local) = &*w.borrow() {
                task = local.pop();
            }
        });

        // 2. Try global queue (batch-steal into local queue, like Go runtime)
        if task.is_none() {
            LOCAL_WORKER.with(|w| {
                if let Some(local) = &*w.borrow() {
                    loop {
                        match SCHEDULER.global_queue.steal_batch_and_pop(local) {
                            Steal::Success(t) => {
                                task = Some(t);
                                break;
                            }
                            Steal::Empty => break,
                            Steal::Retry => {
                                continue;
                            }
                        }
                    }
                }
            });
        }

        // 3. Try steal from other worker deques (batch-steal with randomized start)
        if task.is_none() {
            if let Ok(stealers) = SCHEDULER.stealers.read() {
                let n = stealers.len();
                if n > 0 {
                    let offset = STEAL_RNG.with(|r| {
                        let val = r.get().wrapping_mul(1664525).wrapping_add(1013904223);
                        r.set(val);
                        (val as usize) % n
                    });
                    LOCAL_WORKER.with(|w| {
                        if let Some(local) = &*w.borrow() {
                            for i in 0..n {
                                let stealer = &stealers[(offset + i) % n];
                                loop {
                                    match stealer.steal_batch_and_pop(local) {
                                        Steal::Success(t) => {
                                            task = Some(t);
                                            break;
                                        }
                                        Steal::Empty => break,
                                        Steal::Retry => continue,
                                    }
                                }
                                if task.is_some() {
                                    break;
                                }
                            }
                        }
                    });
                }
            }
        }

        if let Some(mut t) = task {
            unsafe {
                // Ensure worker has no leftover roots from previous tasks before entering safepoints
                (*ctx_ptr).roots_top = 0;
                (*ctx_ptr).in_blocking_io.store(false, Ordering::SeqCst);
                crate::gc::rt_safepoint_poll();
            }

            if !t.coro.done() {
                let saved_gc = t.gc_state.lock().unwrap().take();
                if let Some(state) = saved_gc {
                    unsafe { crate::gc::rt_restore_gc_context(state); }
                } else {
                    unsafe { (*ctx_ptr).roots_top = 0; }
                }

                if let Some(state) = t.local_state.take() {
                    crate::restore_vthread_local_state(state);
                } else {
                    crate::clear_vthread_local_state();
                }

                let yptr = t.yielder_ptr.load(Ordering::SeqCst);
                if yptr != 0 {
                    CURRENT_YIELDER.with(|y| y.set(Some(yptr as *const _)));
                }

                let reason = t.coro.resume(());

                CURRENT_YIELDER.with(|y| y.set(None));

                if !t.coro.done() {
                    *t.gc_state.lock().unwrap() = Some(unsafe { crate::gc::rt_save_gc_context() });
                    t.local_state = Some(crate::save_vthread_local_state());
                    match reason {
                        corosensei::CoroutineResult::Yield(YieldReason::Cooperative) => {
                            SCHEDULER.push_global(t);
                        }
                        corosensei::CoroutineResult::Yield(YieldReason::Sleep(until)) => {
                            SCHEDULER.add_timer(until, t);
                        }
                        corosensei::CoroutineResult::Yield(YieldReason::IoPark(token_id)) => {
                            let shard_idx = token_id % NUM_IO_SHARDS;
                            let shard = &SCHEDULER.io_shards[shard_idx];
                            let mut data = shard.data.lock().unwrap();
                            if data.ready_tokens.remove(&token_id) {
                                drop(data);
                                SCHEDULER.push_global(t);
                            } else {
                                data.parked.insert(token_id, t);
                            }
                        }
                        _ => {}
                    }
                } else {
                    *t.gc_state.lock().unwrap() = None;
                    t.local_state = None;
                    crate::clear_vthread_local_state();
                    unsafe { (*ctx_ptr).roots_top = 0; }
                }
            } else {
                unsafe { (*ctx_ptr).roots_top = 0; }
            }

            unsafe {
                crate::gc::rt_clear_tlab();
                (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
                if crate::gc::is_safepoint_requested() {
                    (*ctx_ptr).in_blocking_io.store(false, Ordering::SeqCst);
                    crate::gc::rt_safepoint_poll();
                    (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
                }
            }
        } else {
            SCHEDULER.idle_workers.fetch_add(1, Ordering::SeqCst);
            {
                let lock = SCHEDULER.park_lock.lock().unwrap();
                let _ = SCHEDULER.park_cvar.wait_timeout(lock, std::time::Duration::from_millis(5));
            }
            SCHEDULER.idle_workers.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub fn vt_spawn_closure<F>(f: F, _cb_slot: usize, slot_live: Arc<AtomicBool>)
where
    F: FnOnce() + Send + 'static,
{
    let id = NEXT_VT_ID.fetch_add(1, Ordering::SeqCst);
    let gc_state = Arc::new(Mutex::new(None));
    register_vthread_gc(id, gc_state.clone());

    let yielder_ptr_arc = Arc::new(AtomicUsize::new(0));
    let yielder_ptr_clone = yielder_ptr_arc.clone();

    let stack_size = get_vt_stack_size();
    let stack = acquire_stack(stack_size);
    
    let coro: Coroutine<(), YieldReason, (), PooledStack> = Coroutine::with_stack(stack, move |yielder: &Yielder<(), YieldReason>, _| {
        let ptr = yielder as *const _ as usize;
        yielder_ptr_clone.store(ptr, Ordering::SeqCst);
        CURRENT_YIELDER.with(|y| y.set(Some(ptr as *const _)));
        
        f();
        
        slot_live.store(false, Ordering::Release);
        CURRENT_YIELDER.with(|y| y.set(None));
    });

    let vt = Box::new(VThread { id, coro, gc_state, local_state: None, yielder_ptr: yielder_ptr_arc });

    let vt_or_pushed = LOCAL_WORKER.with(|w| {
        if let Some(local) = &*w.borrow() {
            local.push(vt);
            SCHEDULER.notify_worker();
            Ok(())
        } else {
            Err(vt)
        }
    });
    if let Err(vt) = vt_or_pushed {
        SCHEDULER.push_global(vt);
    }
}

pub fn vt_sleep(ms: u64) {
    let yielder_ptr = CURRENT_YIELDER.with(|y| y.get());
    if let Some(ptr) = yielder_ptr {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        unsafe { (*ptr).suspend(YieldReason::Sleep(until)) };
    } else {
        let _guard = crate::ThreadIoGuard::new();
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

pub fn vt_join(slot_live: &Arc<AtomicBool>) {
    if vt_is_vthread() {
        let mut spins = 0;
        while slot_live.load(Ordering::Acquire) {
            if spins < 8 {
                vt_yield();
                spins += 1;
            } else {
                vt_sleep(1);
            }
        }
    } else {
        let _guard = crate::ThreadIoGuard::new();
        while slot_live.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

pub fn vt_yield() {
    let yielder_ptr = CURRENT_YIELDER.with(|y| y.get());
    if let Some(ptr) = yielder_ptr {
        unsafe { (*ptr).suspend(YieldReason::Cooperative) };
    } else {
        unsafe { crate::gc::rt_safepoint_poll(); }
        thread::yield_now();
    }
}

pub fn vt_with_syscall<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}

pub fn vt_is_vthread() -> bool {
    CURRENT_YIELDER.with(|y| y.get().is_some())
}

thread_local! {
    static PREEMPT_TICK: std::cell::Cell<u32> = std::cell::Cell::new(0);
    static PREEMPT_LAST_YIELD: std::cell::Cell<Option<std::time::Instant>> = std::cell::Cell::new(None);
}

pub fn vt_preempt_tick() {
    PREEMPT_TICK.with(|cell| {
        let count = cell.get().wrapping_add(1);
        cell.set(count);
        // Sample every 1024 safepoints (~few microseconds)
        if (count & 0x3ff) == 0 {
            if vt_is_vthread() {
                // If other tasks are waiting in the global queue, yield immediately
                if !SCHEDULER.global_queue.is_empty() {
                    vt_yield();
                    return;
                }
                // Check if quantum (10ms) exceeded
                PREEMPT_LAST_YIELD.with(|time_cell| {
                    let now = std::time::Instant::now();
                    if let Some(last) = time_cell.get() {
                        if now.duration_since(last).as_millis() >= 10 {
                            time_cell.set(Some(now));
                            vt_yield();
                        }
                    } else {
                        time_cell.set(Some(now));
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_stack_allocation_and_alignment() {
        let stack = DynamicStack::new(4096).expect("allocate 4KB stack");
        assert_eq!(stack.capacity(), 4096);
        assert!(stack.check_canary());
        assert_eq!(stack.base().get() % STACK_ALIGNMENT, 0);
        assert_eq!(stack.limit().get() % STACK_ALIGNMENT, 0);
        assert!(stack.base().get() > stack.limit().get());
        assert_eq!(stack.base().get() - stack.limit().get(), 4096);
    }

    #[test]
    fn test_coroutine_with_dynamic_stack() {
        let stack = acquire_stack(4096);
        let mut coro = Coroutine::with_stack(stack, |yielder, _| {
            yielder.suspend(YieldReason::Cooperative);
            42
        });
        match coro.resume(()) {
            corosensei::CoroutineResult::Yield(YieldReason::Cooperative) => {}
            _ => panic!("expected yield"),
        }
        match coro.resume(()) {
            corosensei::CoroutineResult::Return(val) => assert_eq!(val, 42),
            _ => panic!("expected return"),
        }
    }

    #[test]
    fn test_stack_canary_integrity() {
        let stack = DynamicStack::new(8192).expect("allocate 8KB stack");
        assert!(stack.check_canary());
        // Corrupt canary intentionally
        unsafe {
            (stack.ptr.as_ptr() as *mut u64).write(0);
        }
        assert!(!stack.check_canary());
        stack.reset_canary();
        assert!(stack.check_canary());
    }
}
