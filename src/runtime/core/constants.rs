//! Centralized configuration and runtime constants for the TejX Runtime.
//! All environment variable keys, memory thresholds, stack dimensions,
//! and tuning parameters are defined here as the single source of truth.

// =============================================================================
// Virtual Thread & Stack Configuration (Growable Stacks — Go-like)
// =============================================================================

/// Initial stack commit size for TejX virtual threads (2 KB, matching Go goroutines).
pub const DEFAULT_VTHREAD_STACK_SIZE: usize = 2 * 1024;

/// Minimum stack size allowed for any virtual thread (2 KB).
pub const MIN_VTHREAD_STACK_SIZE: usize = 2 * 1024;

/// Maximum stack size a virtual thread can grow to (1 MB).
/// Equivalent to Go's default goroutine stack limit.
/// Prevents runaway recursion from consuming unbounded memory.
pub const MAX_VTHREAD_STACK_SIZE: usize = 1024 * 1024;

/// Default stack size for the initial main thread (1 MB, fully committed).
pub const DEFAULT_MAIN_THREAD_STACK_SIZE: usize = 1024 * 1024;

/// Stack alignment in bytes (16 bytes for ARM64 and x86_64 ABI compliance).
pub const STACK_ALIGNMENT: usize = 16;

/// Guard redzone placed below the stack limit for stack canary checking and context switch register preservation (256 bytes).
pub const STACK_REDZONE_SIZE: usize = 256;

/// Magic number planted in the stack redzone to detect stack overflow.
pub const STACK_CANARY_MAGIC: u64 = 0xDEAD_BEEF_CAFE_BABE;

// =============================================================================
// Memory & Garbage Collector Configuration
// =============================================================================

/// Default initial Young Generation (Eden) size (1024 MB).
pub const DEFAULT_YOUNG_GEN_SIZE: usize = 1024 * 1024 * 1024;

/// Default Survivor Space size (128 MB per survivor space; 2 survivor spaces).
pub const DEFAULT_SURVIVOR_SIZE: usize = 128 * 1024 * 1024;

/// Minimum Young Generation size (16 MB).
pub const MIN_YOUNG_GEN_SIZE: usize = 16 * 1024 * 1024;

/// Default initial Old Generation size (2730 MB, ~2/3 of 4GB heap).
pub const DEFAULT_OLD_GEN_SIZE: usize = 2730 * 1024 * 1024;

/// Minimum Old Generation floor (32 MB).
pub const MIN_OLD_GEN_SIZE: usize = 32 * 1024 * 1024;

/// Default maximum heap size (4 GB on 64-bit systems).
pub const DEFAULT_MAX_HEAP_SIZE: usize = 4 * 1024 * 1024 * 1024;

/// Absolute maximum heap ceiling (128 GB).
pub const MAX_HEAP_CEILING: usize = 128 * 1024 * 1024 * 1024;

/// Threshold above which objects are allocated in the Large Object Space (LOS) (128 KB).
pub const LARGE_OBJECT_THRESHOLD: usize = 128 * 1024;

/// Minimum trigger threshold for Large Object Space collection (starts at 4 MB, grows adaptively).
pub const DEFAULT_LOS_GC_TRIGGER_BYTES: usize = 4 * 1024 * 1024;

/// Card table resolution in bytes (each byte in card table covers 512 bytes of heap).
pub const CARD_SIZE: usize = 512;

/// Default GC growth factor (1.5 = 50% headroom above live heap).
pub const DEFAULT_GC_GROWTH_FACTOR: f64 = 1.5;

/// Number of fast free-list bins for Old Generation allocation.
pub const NUM_FAST_BINS: usize = 256;

/// Default Arena memory allocation size (512 MB).
pub const DEFAULT_ARENA_SIZE: usize = 512 * 1024 * 1024;

/// GC root stack capacity per thread context (64K slots).
pub const DEFAULT_GC_ROOT_STACK_SIZE: usize = 64 * 1024;

// =============================================================================
// Environment Variable Names
// =============================================================================

/// Environment variable for configuring virtual thread stack size (e.g. `TEJX_VT_STACK=4k`).
pub const ENV_VT_STACK: &str = "TEJX_VT_STACK";

/// Environment variable for configuring main thread stack size (e.g. `TEJX_MAIN_STACK=2M`).
pub const ENV_MAIN_STACK: &str = "TEJX_MAIN_STACK";

/// Environment variable for configuring GC headroom multiplier (e.g. `TEJXGC=100`).
pub const ENV_GC_FACTOR: &str = "TEJXGC";

/// Environment variable for configuring maximum heap limit (e.g. `TEJX_MAX_HEAP=2G`).
pub const ENV_MAX_HEAP: &str = "TEJX_MAX_HEAP";

/// Environment variable for configuring initial heap limit (e.g. `TEJX_INITIAL_HEAP=512M`).
pub const ENV_INITIAL_HEAP: &str = "TEJX_INITIAL_HEAP";

// =============================================================================
// Runtime CLI Parameter Prefixes
// =============================================================================

pub const CLI_VT_STACK: &str = "--vt-stack=";
pub const CLI_VTHREAD_STACK: &str = "--vthread-stack=";
pub const CLI_XSS: &str = "-Xss";
pub const CLI_MAX_OLD_SPACE_SIZE: &str = "--max-old-space-size=";
pub const CLI_XMX: &str = "-Xmx";
pub const CLI_XMS: &str = "-Xms";

// =============================================================================
// GC Runtime Defaults
// =============================================================================

/// Default GC growth factor percentage (50 = 50% headroom = 1.5× live set).
pub const DEFAULT_GC_PERCENTAGE: usize = 50;

/// Default GC heap trigger floor — 60% of old generation capacity.
pub const DEFAULT_GC_TRIGGER_FLOOR_PCT: usize = 60;

/// Default GC trigger ceiling — 95% of old generation capacity (OOM safety).
pub const DEFAULT_GC_TRIGGER_CEIL_PCT: usize = 95;

/// Absolute minimum breathing room above live set per GC cycle (128 MB).
pub const GC_MIN_HEADROOM_BYTES: usize = 128 * 1024 * 1024;

// =============================================================================
// Thread Pool & Concurrency (Unlimited Virtual Threads)
// =============================================================================

/// Number of default OS threads in the virtual thread executor pool.
pub const DEFAULT_THREAD_POOL_WORKERS: usize = 8;
