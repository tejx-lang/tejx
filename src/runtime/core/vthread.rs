use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use corosensei::{Coroutine, Yielder};
use corosensei::stack::{Stack, StackPointer, STACK_ALIGNMENT};
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use once_cell::sync::Lazy;
use std::cell::RefCell;
use std::sync::Mutex;
use std::thread;
use std::collections::{HashMap, HashSet};
use mio::{Events, Poll, Registry, Token, Interest};
use std::alloc::{alloc, dealloc, Layout};
use std::ptr::NonNull;
use crate::constants::*;

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

/// Truly dynamic heap-allocated stack for TejX virtual threads.
///
/// Unlike OS-backed mmap stacks (which require 32 KB minimum on Apple Silicon due to 16 KB pages)
/// and involve kernel syscalls (`mmap`, `mprotect`, `munmap`), `DynamicStack` is:
/// 1. Allocated directly from the heap with 16-byte alignment (matching Go's user-space mcache stack allocation).
/// 2. Sized dynamically (default 2 KB, matching Go's goroutines, or dynamically configured).
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
        let size = size.max(MIN_VTHREAD_STACK_SIZE);
        let aligned_size = (size + STACK_ALIGNMENT - 1) & !(STACK_ALIGNMENT - 1);
        let total_size = aligned_size + STACK_REDZONE_SIZE;
        let layout = Layout::from_size_align(total_size, STACK_ALIGNMENT).map_err(|_| ())?;
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
        self.layout.size() - STACK_REDZONE_SIZE
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
        // Limit is the lowest address of the allocated stack buffer including redzone
        let lim = self.ptr.as_ptr() as usize;
        StackPointer::new(lim).expect("valid limit stack pointer")
    }
}

thread_local! {
    static STACK_POOL: RefCell<Vec<DynamicStack>> = RefCell::new(Vec::with_capacity(64));
}

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
            if stack.check_canary() {
                STACK_POOL.with(|pool| {
                    let mut p = pool.borrow_mut();
                    if p.len() < 64 {
                        p.push(stack);
                    }
                });
            }
        }
    }
}

pub fn vt_trim_stack_pool() {
    STACK_POOL.with(|pool| {
        pool.borrow_mut().clear();
    });
}

fn acquire_stack(stack_size: usize) -> PooledStack {
    let target_size = stack_size.max(MIN_VTHREAD_STACK_SIZE);
    let pooled = STACK_POOL.with(|pool| {
        let mut p = pool.borrow_mut();
        // Look for a stack with suitable capacity
        if let Some(pos) = p.iter().position(|s| s.capacity() >= target_size) {
            Some(p.swap_remove(pos))
        } else {
            None
        }
    });
    if let Some(stack) = pooled {
        stack.reset_canary();
        return PooledStack { inner: Some(stack) };
    }
    let stack = DynamicStack::new(target_size).expect("Failed to allocate dynamic stack for virtual thread");
    PooledStack { inner: Some(stack) }
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
    pub const TAG_IOPARK: u64 = 1;
    pub const TAG_SLEEP: u64 = 2;
    pub const PAYLOAD_MASK: u64 = (1 << 60) - 1;

    #[inline(always)]
    pub fn cooperative() -> Self {
        YieldReason(Self::TAG_COOPERATIVE << 60)
    }

    #[inline(always)]
    pub fn io_park(token: usize) -> Self {
        YieldReason((Self::TAG_IOPARK << 60) | ((token as u64) & Self::PAYLOAD_MASK))
    }

    #[inline(always)]
    pub fn sleep(until_ms: u64) -> Self {
        YieldReason((Self::TAG_SLEEP << 60) | (until_ms & Self::PAYLOAD_MASK))
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
    pub fn as_io_park(&self) -> Option<usize> {
        if self.tag() == Self::TAG_IOPARK {
            Some((self.0 & Self::PAYLOAD_MASK) as usize)
        } else {
            None
        }
    }
}

pub struct VThread {
    id: usize,
    coro: Coroutine<(), YieldReason, (), PooledStack>,
    gc_state: Arc<Mutex<Option<crate::gc::GcContextState>>>,
    local_state: Option<crate::VThreadLocalState>,
    yielder_ptr: Arc<AtomicUsize>,
    slot_live: Arc<AtomicBool>,
}

unsafe impl Send for VThread {}

const NUM_GC_SHARDS: usize = 16;
static NEXT_VT_ID: AtomicUsize = AtomicUsize::new(1);
static VTHREAD_GC_SHARDS: Lazy<[Mutex<HashMap<usize, Arc<Mutex<Option<crate::gc::GcContextState>>>>>; NUM_GC_SHARDS]> =
    Lazy::new(|| std::array::from_fn(|_| Mutex::new(HashMap::new())));

fn register_vthread_gc(id: usize, gc_state: Arc<Mutex<Option<crate::gc::GcContextState>>>) {
    let shard_idx = id % NUM_GC_SHARDS;
    let mut shard = match VTHREAD_GC_SHARDS[shard_idx].lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    shard.insert(id, gc_state);
}

fn unregister_vthread_gc(id: usize) {
    let shard_idx = id % NUM_GC_SHARDS;
    let mut shard = match VTHREAD_GC_SHARDS[shard_idx].lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    shard.remove(&id);
}

#[inline]
fn lock_gc_state(m: &Mutex<Option<crate::gc::GcContextState>>) -> std::sync::MutexGuard<'_, Option<crate::gc::GcContextState>> {
    match m.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
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

const NUM_IO_SHARDS: usize = 64;

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
    stealers: once_cell::sync::OnceCell<Vec<Stealer<Box<VThread>>>>,
    io_shards: [IoShard; NUM_IO_SHARDS],
    registry: Registry,
    next_token: AtomicUsize,
    timers: Mutex<Vec<(u64, Box<VThread>)>>,
    idle_workers: AtomicUsize,
}

static POLL: Lazy<Mutex<Option<Poll>>> = Lazy::new(|| Mutex::new(Some(Poll::new().unwrap())));

static SCHEDULER: Lazy<Scheduler> = Lazy::new(|| Scheduler {
    global_queue: Injector::new(),
    stealers: once_cell::sync::OnceCell::new(),
    io_shards: std::array::from_fn(|_| IoShard::new()),
    registry: POLL.lock().unwrap().as_ref().unwrap().registry().try_clone().unwrap(),
    next_token: AtomicUsize::new(1),
    timers: Mutex::new(Vec::new()),
    idle_workers: AtomicUsize::new(0),
});

impl Scheduler {
    pub fn notify_worker(&self) {
    }

    pub fn push_global(&self, vt: Box<VThread>) {
        self.global_queue.push(vt);
        self.notify_worker();
    }

    pub fn add_timer(&self, until: u64, vt: Box<VThread>) {
        let mut timers = match self.timers.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        timers.push((until, vt));
        drop(timers);
        self.notify_worker();
    }

    pub fn next_timer_timeout(&self) -> Option<std::time::Duration> {
        let now = current_time_ms();
        let timers = match self.timers.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
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
            let mut timers = match self.timers.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
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
    static LOCAL_WORKER: std::cell::Cell<*const Worker<Box<VThread>>> = std::cell::Cell::new(std::ptr::null());
    static CURRENT_YIELDER: std::cell::Cell<Option<*const Yielder<(), YieldReason>>> = std::cell::Cell::new(None);
    static STEAL_RNG: std::cell::Cell<u32> = std::cell::Cell::new(123456789);
}

pub fn vt_init(num_workers: usize) {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    // Force SCHEDULER initialization before start_netpoller takes the Poll instance
    let _ = SCHEDULER.next_token.load(Ordering::SeqCst);
    
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
            let mut events = Events::with_capacity(16384);
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

#[inline(always)]
unsafe fn vthread_suspend(yielder: *const Yielder<(), YieldReason>, reason: YieldReason) {
    (*yielder).suspend(reason);
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
        unsafe { vthread_suspend(ptr, YieldReason::io_park(token_id)) };
    } else {
        unsafe {
            if crate::gc::is_safepoint_requested() {
                crate::gc::rt_safepoint_poll_slow();
            }
        }
        thread::yield_now();
    }
}

#[inline(always)]
fn resume_vthread_coro(coro: &mut Coroutine<(), YieldReason, (), PooledStack>) -> corosensei::CoroutineResult<YieldReason, ()> {
    coro.resume(())
}

fn worker_loop(worker_id: usize, local: Worker<Box<VThread>>) {
    unsafe { crate::gc::rt_register_thread(); }
    let local_box = Box::new(local);
    let local_ptr: *const Worker<Box<VThread>> = &*local_box;
    LOCAL_WORKER.with(|w| w.set(local_ptr));

    let ctx_ptr = unsafe { crate::gc::current_thread_context() };
    unsafe {
        (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
    }

    loop {
        let mut task = None;

        // 1. Try local queue
        LOCAL_WORKER.with(|w| {
            let ptr = w.get();
            if !ptr.is_null() {
                task = unsafe { (*ptr).pop() };
            }
        });

        // 2. Try global queue (batch-steal into local queue, like Go runtime)
        if task.is_none() {
            LOCAL_WORKER.with(|w| {
                let ptr = w.get();
                if !ptr.is_null() {
                    let local = unsafe { &*ptr };
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

        // 3. Try steal from other worker deques (batch-steal with randomized start, skipping worker's own deque)
        if task.is_none() {
            if let Some(stealers) = SCHEDULER.stealers.get() {
                let n = stealers.len();
                if n > 1 {
                    let offset = STEAL_RNG.with(|r| {
                        let val = r.get().wrapping_mul(1664525).wrapping_add(1013904223);
                        r.set(val);
                        (val as usize) % n
                    });
                    LOCAL_WORKER.with(|w| {
                        let ptr = w.get();
                        if !ptr.is_null() {
                            let local = unsafe { &*ptr };
                            for i in 0..n {
                                let target_idx = (offset + i) % n;
                                if target_idx == worker_id {
                                    continue;
                                }
                                let stealer = &stealers[target_idx];
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

        if task.is_none() {
            SCHEDULER.drain_expired_timers();
            LOCAL_WORKER.with(|w| {
                let ptr = w.get();
                if !ptr.is_null() {
                    let local = unsafe { &*ptr };
                    task = local.pop();
                    if task.is_none() {
                        loop {
                            match SCHEDULER.global_queue.steal_batch_and_pop(local) {
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
            });
        }

        if let Some(mut t) = task {
            unsafe {
                // Ensure worker has no leftover roots from previous tasks before entering safepoints
                (*ctx_ptr).roots_top = 0;
                (*ctx_ptr).in_blocking_io.store(false, Ordering::SeqCst);
                if crate::gc::is_safepoint_requested() {
                    crate::gc::rt_safepoint_poll_slow();
                }
            }

            if !t.coro.done() {
                let saved_gc = lock_gc_state(&t.gc_state).take();
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

                PREEMPT_LAST_MS.with(|c| c.set(current_time_ms()));
                PREEMPT_TICK.with(|c| c.set(0));

                let reason = resume_vthread_coro(&mut t.coro);

                CURRENT_YIELDER.with(|y| y.set(None));

                match reason {
                    corosensei::CoroutineResult::Yield(r) => {
                        *lock_gc_state(&t.gc_state) = Some(unsafe { crate::gc::rt_save_gc_context() });
                        t.local_state = Some(crate::save_vthread_local_state());
                        if r.is_cooperative() {
                            SCHEDULER.push_global(t);
                        } else if let Some(until) = r.as_sleep() {
                            SCHEDULER.add_timer(until, t);
                        } else if let Some(token_id) = r.as_io_park() {
                            let shard_idx = token_id % NUM_IO_SHARDS;
                            let shard = &SCHEDULER.io_shards[shard_idx];
                            let mut data = match shard.data.lock() {
                                Ok(g) => g,
                                Err(e) => e.into_inner(),
                            };
                            if data.ready_tokens.remove(&token_id) {
                                drop(data);
                                SCHEDULER.push_global(t);
                            } else {
                                data.parked.insert(token_id, t);
                            }
                        }
                    }
                    corosensei::CoroutineResult::Return(()) => {
                        *lock_gc_state(&t.gc_state) = None;
                        t.local_state = None;
                        crate::clear_vthread_local_state();
                        unsafe { (*ctx_ptr).roots_top = 0; }
                        let slot_live = t.slot_live.clone();
                        drop(t);
                        slot_live.store(false, Ordering::SeqCst);
                    }
                }
            } else {
                *lock_gc_state(&t.gc_state) = None;
                t.local_state = None;
                crate::clear_vthread_local_state();
                unsafe { (*ctx_ptr).roots_top = 0; }
                t.slot_live.store(false, Ordering::SeqCst);
            }

            unsafe {
                crate::gc::rt_clear_tlab();
                (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
                if crate::gc::is_safepoint_requested() {
                    (*ctx_ptr).in_blocking_io.store(false, Ordering::SeqCst);
                    crate::gc::rt_safepoint_poll_slow();
                    (*ctx_ptr).in_blocking_io.store(true, Ordering::SeqCst);
                }
            }
        } else {
            SCHEDULER.idle_workers.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(1));
            SCHEDULER.idle_workers.fetch_sub(1, Ordering::Relaxed);
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
    let gc_state = Arc::new(Mutex::new(None));
    register_vthread_gc(id, gc_state.clone());

    let yielder_ptr_arc = Arc::new(AtomicUsize::new(0));
    let yielder_ptr_clone = yielder_ptr_arc.clone();

    let stack = acquire_stack(stack_size);
    
    let coro: Coroutine<(), YieldReason, (), PooledStack> = Coroutine::with_stack(stack, move |yielder: &Yielder<(), YieldReason>, _| {
        let ptr = yielder as *const _ as usize;
        yielder_ptr_clone.store(ptr, Ordering::SeqCst);
        CURRENT_YIELDER.with(|y| y.set(Some(ptr as *const _)));
        
        f();
        
        CURRENT_YIELDER.with(|y| y.set(None));
    });

    let vt = Box::new(VThread {
        id,
        coro,
        gc_state,
        local_state: None,
        yielder_ptr: yielder_ptr_arc,
        slot_live,
    });

    let mut to_push = Some(vt);
    LOCAL_WORKER.with(|w| {
        let ptr = w.get();
        if !ptr.is_null() {
            if let Some(t) = to_push.take() {
                unsafe { (*ptr).push(t) };
                SCHEDULER.notify_worker();
            }
        }
    });
    if let Some(t) = to_push {
        SCHEDULER.push_global(t);
    }
}

#[inline(always)]
pub fn vt_sleep(ms: u64) {
    let yielder_ptr = CURRENT_YIELDER.with(|y| y.get());
    if let Some(ptr) = yielder_ptr {
        let until = current_time_ms().saturating_add(ms);
        unsafe { vthread_suspend(ptr, YieldReason::sleep(until)) };
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

#[inline(always)]
pub fn vt_yield() {
    let yielder_ptr = CURRENT_YIELDER.with(|y| y.get());
    if let Some(ptr) = yielder_ptr {
        unsafe { vthread_suspend(ptr, YieldReason::cooperative()) };
    } else {
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
    static PREEMPT_LAST_MS: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

pub fn vt_preempt_tick() {
    let count = PREEMPT_TICK.with(|cell| {
        let count = cell.get().wrapping_add(1);
        cell.set(count);
        count
    });
    if (count & 0x3fff) != 0 {
        return;
    }
    if !vt_is_vthread() {
        return;
    }
    let now = current_time_ms();
    let last = PREEMPT_LAST_MS.with(|c| c.get());
    if now.saturating_sub(last) >= 10 {
        PREEMPT_LAST_MS.with(|c| c.set(now));
        vt_yield();
    }
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
            yielder.suspend(YieldReason::cooperative());
            42
        });
        match coro.resume(()) {
            corosensei::CoroutineResult::Yield(r) if r.is_cooperative() => {}
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
