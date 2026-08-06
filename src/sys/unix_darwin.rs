//! The Darwin-specific half of the unix backend — macOS on `x86_64` and on
//! `aarch64` (Apple Silicon), which are identical at this level.
//!
//! Two items, because two items is all Linux and Darwin disagree about once the
//! constant tables (`abi.rs`) and the struct layouts (`layout.rs`) are factored
//! out: which `struct termios` to pass, and how to reach `errno`.
//!
//! Authority: `xnu/bsd/sys/termios.h` for the layout — `tcflag_t` and `speed_t`
//! are `unsigned long` (8 bytes on both Apple architectures), `NCCS` is 20,
//! there is no `c_line`, and `sizeof(struct termios)` is 72 — and
//! `<sys/errno.h>`'s `#define errno (*__error())` for the accessor. Note that
//! `__error()` is a *different symbol* from Linux's `__errno_location()`;
//! nothing warns you, the link simply fails, which is the good case.
//!
//! Everything else this backend needs is in libSystem, which every Apple target
//! links by default (rustc passes `-lSystem` for `*-apple-darwin`), so
//! `tcgetattr`, `tcsetattr`, `ioctl`, `read`, `write`, `isatty`, `open`,
//! `close` and `signal` need no `#[link]` attribute. Deliberately so: Apple
//! does not support statically linking libSystem, and naming it explicitly
//! would only invite someone to try.

use crate::sys::abi::HOST_ABI;
use crate::sys::layout::{DarwinTermios, DARWIN_NCCS};

/// The `struct termios` this target's libc expects.
pub type RawTermios = DarwinTermios;

// A Linux table paired with a Darwin struct would put VMIN at c_cc[6] — inside
// the 20-slot array, so no bounds error, just VREPRINT set to 0 and VMIN left
// at whatever the terminal had (usually 1), which blocks the event loop
// forever instead of ticking every 100 ms.
const _: () = assert!(HOST_ABI.nccs == DARWIN_NCCS);
const _: () = assert!(HOST_ABI.vmin_idx == 16 && HOST_ABI.vtime_idx == 17);
// _IOR('t', 104, struct winsize), computed by `abi::bsd_ior` — not Linux's
// 0x5413, which on Darwin would be a request in group 0x54 ('T') that no tty
// driver implements.
const _: () = assert!(crate::sys::abi::TIOCGWINSZ == 0x4008_7468);
// `tcflag_t` is 8 bytes here, so the flag words are used at full width; this
// pins the four values that differ from Linux and would otherwise clear the
// wrong bits silently.
const _: () = assert!(HOST_ABI.ixon == 0x200 && HOST_ABI.isig == 0x80);
const _: () = assert!(HOST_ABI.icanon == 0x100 && HOST_ABI.iexten == 0x400);
const _: () = assert!(HOST_ABI.eagain == 35);

extern "C" {
    fn __error() -> *mut i32;
}

/// The current thread's `errno`.
pub fn errno() -> i32 {
    // SAFETY: `__error` returns a valid, thread-local pointer for the lifetime
    // of the thread; we only read through it.
    unsafe { *__error() }
}
