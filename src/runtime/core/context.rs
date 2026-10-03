// Native TejX Context Switcher
// Zero external dependencies. Battle-tested C-speed fiber context switching for AArch64 and x86_64.

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
core::arch::global_asm!(
    ".global _tejx_context_switch",
    ".balign 4",
    "_tejx_context_switch:",
    // Full data memory barrier before saving context — ensures all stores
    // from this fiber are globally visible before we switch away.
    "dmb ish",
    "stp d14, d15, [sp, #-16]!",
    "stp d12, d13, [sp, #-16]!",
    "stp d10, d11, [sp, #-16]!",
    "stp d8, d9,   [sp, #-16]!",
    "stp x27, x28, [sp, #-16]!",
    "stp x25, x26, [sp, #-16]!",
    "stp x23, x24, [sp, #-16]!",
    "stp x21, x22, [sp, #-16]!",
    "stp x19, x20, [sp, #-16]!",
    "stp x29, x30, [sp, #-16]!",
    "mov x2, sp",
    "str x2, [x0]",
    "mov sp, x1",
    "ldp x29, x30, [sp], #16",
    "ldp x19, x20, [sp], #16",
    "ldp x21, x22, [sp], #16",
    "ldp x23, x24, [sp], #16",
    "ldp x25, x26, [sp], #16",
    "ldp x27, x28, [sp], #16",
    "ldp d8, d9,   [sp], #16",
    "ldp d10, d11, [sp], #16",
    "ldp d12, d13, [sp], #16",
    "ldp d14, d15, [sp], #16",
    // Full data memory barrier after restoring — ensures we see all stores
    // from the core that previously ran this fiber.
    "dmb ish",
    "ret",
);

#[cfg(all(not(target_os = "macos"), target_arch = "aarch64"))]
core::arch::global_asm!(
    ".global tejx_context_switch",
    ".type tejx_context_switch, %function",
    ".balign 4",
    "tejx_context_switch:",
    // Full data memory barrier before saving context.
    "dmb ish",
    "stp d14, d15, [sp, #-16]!",
    "stp d12, d13, [sp, #-16]!",
    "stp d10, d11, [sp, #-16]!",
    "stp d8, d9,   [sp, #-16]!",
    "stp x27, x28, [sp, #-16]!",
    "stp x25, x26, [sp, #-16]!",
    "stp x23, x24, [sp, #-16]!",
    "stp x21, x22, [sp, #-16]!",
    "stp x19, x20, [sp, #-16]!",
    "stp x29, x30, [sp, #-16]!",
    "mov x2, sp",
    "str x2, [x0]",
    "mov sp, x1",
    "ldp x29, x30, [sp], #16",
    "ldp x19, x20, [sp], #16",
    "ldp x21, x22, [sp], #16",
    "ldp x23, x24, [sp], #16",
    "ldp x25, x26, [sp], #16",
    "ldp x27, x28, [sp], #16",
    "ldp d8, d9,   [sp], #16",
    "ldp d10, d11, [sp], #16",
    "ldp d12, d13, [sp], #16",
    "ldp d14, d15, [sp], #16",
    // Full data memory barrier after restoring.
    "dmb ish",
    "ret",
);

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
core::arch::global_asm!(
    ".global _tejx_context_switch",
    "_tejx_context_switch:",
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdi], rsp",
    "mov rsp, rsi",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",
);

#[cfg(all(not(target_os = "macos"), target_arch = "x86_64"))]
core::arch::global_asm!(
    ".global tejx_context_switch",
    ".type tejx_context_switch, @function",
    "tejx_context_switch:",
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdi], rsp",
    "mov rsp, rsi",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",
);

extern "C" {
    /// Save current stack context into `*from_sp` and switch to `to_sp`.
    pub fn tejx_context_switch(from_sp: *mut *mut u8, to_sp: *mut u8);
}

/// Initialize a fiber stack so that calling `tejx_context_switch(..., sp)`
/// will start execution at `entry_fn`.
#[cfg(target_arch = "aarch64")]
pub unsafe fn init_fiber_stack(stack_top: *mut u8, entry_fn: extern "C" fn() -> !) -> *mut u8 {
    // ARM64 requires 16-byte stack alignment.
    let aligned_top = (stack_top as usize & !0xf) as *mut u8;

    // We need to save 10 pairs of 64-bit registers = 160 bytes:
    // [sp + 0]:  x29 (fp = 0), x30 (lr = entry_fn)
    // [sp + 16]: x19, x20 (0)
    // [sp + 32]: x21, x22 (0)
    // [sp + 48]: x23, x24 (0)
    // [sp + 64]: x25, x26 (0)
    // [sp + 80]: x27, x28 (0)
    // [sp + 96]: d8, d9 (0)
    // [sp + 112]: d10, d11 (0)
    // [sp + 128]: d12, d13 (0)
    // [sp + 144]: d14, d15 (0)
    let sp = aligned_top.sub(160);
    let slots = sp as *mut usize;

    // Zero out all registers
    std::ptr::write_bytes(sp, 0, 160);

    // x29 = 0, x30 = entry_fn
    *slots.add(0) = 0; // x29 (fp)
    *slots.add(1) = entry_fn as usize; // x30 (lr)

    sp
}

#[cfg(target_arch = "x86_64")]
pub unsafe fn init_fiber_stack(stack_top: *mut u8, entry_fn: extern "C" fn() -> !) -> *mut u8 {
    // x86_64 requires (rsp + 8) to be 16-byte aligned before call/ret.
    // That means at entry to entry_fn, rsp % 16 == 8.
    let aligned_top = (stack_top as usize & !0xf) as *mut u8;

    // We allocate 64 bytes (8 64-bit slots):
    // [sp + 0]:  r15 (0)
    // [sp + 8]:  r14 (0)
    // [sp + 16]: r13 (0)
    // [sp + 24]: r12 (0)
    // [sp + 32]: rbx (0)
    // [sp + 40]: rbp (0)
    // [sp + 48]: return address (entry_fn)
    // [sp + 56]: dummy return address (0) -> makes rsp % 16 == 8 after ret pops entry_fn!
    let sp = aligned_top.sub(64);
    let slots = sp as *mut usize;

    std::ptr::write_bytes(sp, 0, 64);

    *slots.add(6) = entry_fn as usize; // popped by ret
    *slots.add(7) = 0; // dummy caller return address

    sp
}
