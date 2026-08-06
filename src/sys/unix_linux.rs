//! The Linux-specific half of the unix backend.
//!
//! Two items, because two items is all Linux and Darwin disagree about once the
//! constant tables (`abi.rs`) and the struct layouts (`layout.rs`) are factored
//! out: which `struct termios` to pass, and how to reach `errno`.
//!
//! Authority: glibc `<bits/termios.h>` / musl `arch/x86_64/bits/termios.h` for
//! the layout, and `__errno_location()` — the thread-local errno accessor both
//! glibc and musl export — for the second. (`errno` itself is a macro in C and
//! has no symbol to link against.)

use crate::sys::abi::HOST_ABI;
use crate::sys::layout::{LinuxTermios, LINUX_NCCS};

/// The `struct termios` this target's libc expects.
pub type RawTermios = LinuxTermios;

// A Darwin table paired with a Linux struct would put VMIN at c_cc[16] of a
// 32-slot array: legal memory, wrong slot, and the pager would block forever.
const _: () = assert!(HOST_ABI.nccs == LINUX_NCCS);
const _: () = assert!(HOST_ABI.vmin_idx == 6 && HOST_ABI.vtime_idx == 5);
const _: () = assert!(crate::sys::abi::TIOCGWINSZ == 0x5413);

extern "C" {
    fn __errno_location() -> *mut i32;
}

/// The current thread's `errno`.
pub fn errno() -> i32 {
    // SAFETY: `__errno_location` returns a valid, thread-local pointer for
    // the lifetime of the thread; we only read through it.
    unsafe { *__errno_location() }
}
