//! The platform seam: the **only** part of the crate allowed to contain
//! `unsafe`, and the only part that knows which operating system this is.
//!
//! This file itself contains no `unsafe` and no syscalls. It declares the
//! public surface, owns the three signal flags (which are portable atomics),
//! re-exports the pure [`abi`] core, and `cfg`-dispatches to exactly one
//! backend:
//!
//! ```text
//! main.rs / term.rs / key.rs      safe, portable, no cfg(target_os) anywhere
//! ────────────────────────────────────────────────────────────────────────────
//! sys/mod.rs     public surface + dispatch  (no unsafe)
//! sys/abi.rs     pure ABI arithmetic        (no unsafe, no syscalls, tested)
//! sys/layout.rs  pure C struct layouts      (no unsafe, every OS, tested)
//! ────────────────────────────────────────────────────────────────────────────
//! sys/unix.rs  termios + ioctl        │  sys/stub.rs  do-nothing fallback
//!   unix_linux.rs / unix_darwin.rs    │
//! ```
//!
//! The unix backend covers Linux (glibc and musl) and Darwin (macOS on x86_64
//! and aarch64). Those two share every syscall used here and differ only in
//! `struct termios`, the errno symbol, and a handful of constants — so the
//! constants live in [`abi`], the layouts in [`layout`], both host-tested for
//! *both* OSes, and the per-OS files hold two items each.
//!
//! Per SPEC.md §"Hard constraints" #1 every syscall is a hand-written
//! `extern "C"` declaration; there is no `libc` crate and never will be.
//!
//! # The contract a backend must satisfy
//!
//! A backend is one module that provides *exactly* the names below with these
//! signatures and these semantics. Nothing above `sys` changes when one is
//! added. Anything a backend can compute without touching the OS belongs in
//! [`abi`] instead, so it is unit-tested on whatever host CI happens to run.
//!
//! | Item | Signature | Contract |
//! | --- | --- | --- |
//! | `SavedTermios` | opaque `Copy` type | Whatever [`restore`] needs to put the terminal back byte-for-byte as found. May be a `struct termios`, a pair of console-mode `DWORD`s, anything. |
//! | `install_signal_handlers()` | `fn()` | Idempotent. Arrange for the three `*_pending` flags in this module to become true when the corresponding event happens. Whatever mechanism is used must be safe to run in that context (the unix backend's handlers touch nothing but a lock-free atomic). |
//! | `is_tty(Fd)` | `fn(Fd) -> bool` | Does this handle refer to a terminal? |
//! | `open_tty()` | `fn() -> Option<Fd>` | A fresh read+write handle to the controlling terminal, even when stdin is a pipe. `None` when there is none. |
//! | `tty_fd()` | `fn() -> Option<(Fd, bool)>` | Handle to read keys from. The `bool` is "the caller owns this and must [`close_fd`] it". Must prefer [`STDIN`] when it is a terminal. |
//! | `close_fd(Fd)` | `fn(Fd)` | Close a handle from [`open_tty`]. Must ignore the standard handles. |
//! | `winsize()` | `fn() -> Option<(u16, u16)>` | Terminal size as `(cols, rows)`, trying whatever handles are plausible. |
//! | `winsize_of(Fd)` | `fn(Fd) -> Option<(u16, u16)>` | Size of one handle. `None` on failure **and** on a zero dimension; `term.rs` substitutes 80x24. Must report the *visible window*, not any scrollback buffer. |
//! | `set_raw(Fd)` | `fn(Fd) -> Option<SavedTermios>` | Enter raw mode — no echo, no line buffering, no signal generation from keys, no output post-processing — and return the previous state. `None` if that is impossible; `main.rs` treats it as "no tty" and dumps instead. |
//! | `restore(Fd, &SavedTermios)` | `fn(Fd, &SavedTermios) -> bool` | Undo `set_raw` exactly. Called from a panic hook, so it must not allocate or panic. |
//! | `read_input(Fd, &mut [u8])` | `fn(Fd, &mut [u8]) -> ReadOutcome` | Up to `buf.len()` bytes of **UTF-8 encoded terminal input**, retrying past interruptions. **Must return [`ReadOutcome::Timeout`] after roughly 100 ms of silence** — that tick is how the event loop notices a resize. Never split a UTF-8 sequence's meaning: `key.rs` will wait for continuation bytes forever. |
//! | `write_all(Fd, &[u8])` | `fn(Fd, &[u8]) -> Result<(), i32>` | Write the whole buffer, looping over short writes. `Err` carries the platform error code. |
//!
//! Two further obligations are behavioural rather than typed:
//!
//! * **The mouse is never captured.** A backend must not emit `?1000h`,
//!   `?1002h`, `?1003h` or `?1006h`, and must not enable any OS-level mouse
//!   reporting mode. Terminal-native click-drag selection has to keep working
//!   (SPEC.md §"Hard constraints" #5). On Windows that additionally means
//!   leaving `ENABLE_QUICK_EDIT_MODE` set and `ENABLE_MOUSE_INPUT` clear.
//! * **Input arrives as ANSI.** `key.rs` decodes an escape-sequence byte
//!   stream and has no platform branch. A backend whose OS does not natively
//!   speak VT must translate (on Windows: set
//!   `ENABLE_VIRTUAL_TERMINAL_INPUT`), not push a branch upwards.
//!
//! Adding an OS is therefore: a new file next to `unix.rs`, its constants added
//! to [`abi`] and its C struct layouts to [`layout`] — both with host tests and
//! `const _: () = assert!(size_of::<T>() == N);` pins, because there is no
//! machine in this project's loop that can *run* a non-Linux backend — and one
//! arm added to the dispatch below.

use core::sync::atomic::{AtomicBool, Ordering};

pub mod abi;
pub mod layout;

// The Windows console ABI gets the same treatment as the Darwin tables above:
// declared on *every* target, never behind `cfg(windows)`, so its constants,
// its console-mode arithmetic and its C struct sizes are compile-asserted and
// `cargo test`-ed on the Linux host. `windows.rs` itself is only the FFI.
#[path = "windows/abi.rs"]
pub mod win_abi;
#[path = "windows/layout.rs"]
pub mod win_layout;

// ---------------------------------------------------------------------------
// Types shared by every backend
// ---------------------------------------------------------------------------

/// A raw file descriptor / handle. Opaque to callers above this module: they
/// only ever pass one back to a `sys` function, never inspect it.
pub type Fd = i32;

/// Standard output handle.
pub const STDOUT: Fd = 1;
/// Standard input handle.
pub const STDIN: Fd = 0;

/// Outcome of a single non-blocking-ish read from the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadOutcome {
    /// `n` bytes were placed at the front of the caller's buffer.
    Bytes(usize),
    /// The read timed out with nothing to show. The event loop uses this as its
    /// tick: it is when the signal flags below get polled.
    Timeout,
    /// End of file on the input handle. The unix backend cannot tell EOF from a
    /// `VTIME` timeout (both are a zero-length `read`) so it never produces
    /// this; it is part of the contract for backends that can.
    #[allow(dead_code)]
    Eof,
    /// Errno, or the platform's equivalent error code.
    Error(i32),
}

// ---------------------------------------------------------------------------
// Signal flags — portable, shared by every backend
// ---------------------------------------------------------------------------

static WINCH: AtomicBool = AtomicBool::new(false);
static INTR: AtomicBool = AtomicBool::new(false);
static TERM: AtomicBool = AtomicBool::new(false);

/// True (and cleared) if a terminal-resize event arrived since the last call.
pub fn winch_pending() -> bool {
    WINCH.swap(false, Ordering::SeqCst)
}

/// True (and cleared) if an interrupt (Ctrl-C at the OS level) arrived.
pub fn interrupt_pending() -> bool {
    INTR.swap(false, Ordering::SeqCst)
}

/// True (and cleared) if a termination signal (SIGTERM/SIGHUP/SIGQUIT, or the
/// platform equivalent) arrived. Their default disposition kills the process
/// outright, stranding the tty in raw mode on the alternate screen (release is
/// `panic = "abort"`, so no `Drop` and no panic hook); catching them makes
/// `kill <pid>` an ordinary quit.
pub fn terminate_pending() -> bool {
    TERM.swap(false, Ordering::SeqCst)
}

// ---------------------------------------------------------------------------
// Backend dispatch
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[path = "unix.rs"]
mod backend;

#[cfg(windows)]
#[path = "windows.rs"]
mod backend;

// No real backend for this target yet: a placeholder with the same surface and
// no `unsafe`, so the crate still compiles. See WINDOWS.md.
#[cfg(not(any(unix, windows)))]
#[path = "stub.rs"]
mod backend;

pub use self::backend::*;

/// Can the terminal interpret ANSI escape sequences?
///
/// Part of the contract, but portable rather than per-backend: on unix a
/// terminal that speaks no VT is not a case this reader has ever handled, so the
/// answer is a constant `true`. Windows is the one platform where a *real*
/// console can refuse `ENABLE_VIRTUAL_TERMINAL_PROCESSING` (conhost before
/// Windows 10 1703), and this is how that is signalled upward: the safe layer
/// can consult it and fall back to plain, uncoloured output instead of printing
/// escape codes at the user. The Windows backend additionally refuses raw mode
/// outright in that case, so the reader degrades to the non-interactive dump.
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn vt_output_supported() -> bool {
    true
}
