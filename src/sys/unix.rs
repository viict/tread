//! Unix backend: termios, `ioctl(TIOCGWINSZ)`, `read`/`write`, `signal`.
//!
//! Hand-written `extern "C"` FFI — no `libc` crate, per SPEC.md §"Hard
//! constraints" #1. All of the ABI *arithmetic* (which flag bits raw mode
//! clears, what `TIOCGWINSZ` encodes to, which `c_cc` slots VMIN/VTIME are, how
//! a `read`/`write` return value is classified) lives in [`crate::sys::abi`],
//! and every C struct layout lives in [`crate::sys::layout`]; both are pure and
//! unit-tested on the host for *every* OS, not just the one being built.
//!
//! This file is therefore only the syscalls. It is shared by Linux and Darwin,
//! which agree on the whole of the POSIX surface used here —
//! `tcgetattr`/`tcsetattr`/`ioctl`/`read`/`write`/`isatty`/`open`/`close`/`signal`
//! have the same names, signatures and semantics on both. What they do *not*
//! agree on is
//!
//! | | Linux | Darwin |
//! | --- | --- | --- |
//! | `struct termios` | 4-byte `tcflag_t`, `c_line`, `NCCS` 32 | 8-byte `tcflag_t`, no `c_line`, `NCCS` 20 |
//! | `VMIN` / `VTIME` | `c_cc[6]` / `c_cc[5]` | `c_cc[16]` / `c_cc[17]` |
//! | `TIOCGWINSZ` | `0x5413` (legacy literal) | `_IOR('t', 104, …)` = `0x40087468` |
//! | `IXON` `ISIG` `ICANON` `IEXTEN` `CSIZE` | `0x400` `1` `2` `0x8000` `0o60` | `0x200` `0x80` `0x100` `0x400` `0x300` |
//! | `O_NOCTTY` | `0o400` | `0x20000` |
//! | errno accessor | `__errno_location()` | `__error()` |
//!
//! and so the divergent parts are exactly two things: the constant tables
//! (in `abi.rs`, host-tested for both) and the two-item `os` module selected
//! below (the termios layout alias and the errno symbol).
//!
//! Linking: on Linux these symbols come from the libc rustc already links
//! (glibc, or the self-contained musl for the static target); on Darwin they
//! come from `libSystem.B.dylib`, which the Apple targets link unconditionally.
//! No `#[link]` attribute is needed or wanted on either.

use super::abi::{
    self, classify_read, classify_write, ReadStep, WriteStep, HOST_ABI, HOST_POSIX, TIOCGWINSZ,
};
use super::layout::Winsize;
use super::{Fd, ReadOutcome, INTR, TERM, WINCH};
use core::sync::atomic::Ordering;

// ---- which unix -------------------------------------------------------------
// A unix that is neither of these has a third termios ABI (the BSDs are close
// to Darwin but not identical, Solaris is closer to Linux), and a wrong layout
// compiles cleanly and then corrupts the terminal at runtime. So it is refused
// here rather than silently mis-served: adding one is a table in `abi.rs`, a
// struct in `layout.rs`, and a sibling of the two modules below.
#[cfg(target_os = "linux")]
#[path = "unix_linux.rs"]
mod os;

#[cfg(target_os = "macos")]
#[path = "unix_darwin.rs"]
mod os;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!(
    "sys/unix.rs supports the Linux and Darwin termios/ioctl ABIs only. This \
     target needs its constants added to sys/abi.rs, its struct termios added \
     to sys/layout.rs (both host-tested), and a two-item module alongside \
     sys/unix_linux.rs giving the layout alias and the errno accessor."
);

/// This target's `struct termios`, from [`crate::sys::layout`].
use os::RawTermios;

// ---- C types ---------------------------------------------------------------
#[allow(non_camel_case_types)]
type c_int = i32;
#[allow(non_camel_case_types)]
type c_ulong = u64;
/// `char` is signed on x86_64/aarch64 Linux and on every Apple target.
#[allow(non_camel_case_types)]
type c_char = i8;
#[allow(non_camel_case_types)]
type size_t = usize;
#[allow(non_camel_case_types)]
type ssize_t = isize;

/// Opaque saved terminal state handed back by [`set_raw`].
#[derive(Clone, Copy)]
pub struct SavedTermios(RawTermios);

// ---- compile-time ABI assertions -------------------------------------------
// The layouts themselves are asserted in `layout.rs` on every target. What is
// asserted here is that this build paired the right table with the right
// struct — the failure mode being a Darwin binary driving Linux's c_cc indices.
const _: () = assert!(core::mem::size_of::<RawTermios>() >= 60);
const _: () = assert!(HOST_ABI.vmin_idx < HOST_ABI.nccs);
const _: () = assert!(HOST_ABI.vtime_idx < HOST_ABI.nccs);
const _: () = assert!(core::mem::size_of::<Winsize>() == abi::WINSIZE_LEN);

// `ioctl` and `open` must be declared **variadic**, exactly as the C headers
// declare them, and not as fixed-arity three-argument functions. On
// `aarch64-apple-darwin` Apple's ABI passes variadic arguments on the stack
// while fixed parameters stay in registers, so a fixed-arity declaration would
// put the `struct winsize *` in `x2` where libSystem's `ioctl` never looks —
// a silent wrong-pointer read on Apple Silicon only. Declaring the `...` makes
// rustc emit the Apple variadic sequence. Both prototypes are the same on
// Linux and Darwin: `int ioctl(int, unsigned long, ...)`,
// `int open(const char *, int, ...)`.
extern "C" {
    fn tcgetattr(fd: c_int, termios_p: *mut RawTermios) -> c_int;
    fn tcsetattr(fd: c_int, optional_actions: c_int, termios_p: *const RawTermios) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn read(fd: c_int, buf: *mut u8, count: size_t) -> ssize_t;
    fn write(fd: c_int, buf: *const u8, count: size_t) -> ssize_t;
    fn isatty(fd: c_int) -> c_int;
    fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn signal(signum: c_int, handler: usize) -> usize;
}

// ---- signals ---------------------------------------------------------------

/// Async-signal-safe: touches nothing but a lock-free atomic.
extern "C" fn on_winch(_sig: c_int) {
    WINCH.store(true, Ordering::SeqCst);
}

/// Async-signal-safe: touches nothing but a lock-free atomic.
extern "C" fn on_intr(_sig: c_int) {
    INTR.store(true, Ordering::SeqCst);
}

/// Async-signal-safe: touches nothing but a lock-free atomic.
extern "C" fn on_term(_sig: c_int) {
    TERM.store(true, Ordering::SeqCst);
}

/// Install the SIGWINCH / SIGINT / SIGTERM / SIGHUP / SIGQUIT handlers.
/// Idempotent. The three termination signals are caught, not left at their
/// default disposition, so the event loop can restore the terminal first.
///
/// `signal(3)`, not `sigaction(2)`, deliberately: `struct sigaction` has a
/// different shape on Linux and Darwin (member order, `sa_mask` size, the
/// `sa_restorer` tail) and would be a third struct layout to get right blind,
/// while buying nothing here. Both libcs give `signal` BSD semantics — the
/// handler stays installed across deliveries — and this backend depends on no
/// `SA_*` flag: interrupted reads are handled by retrying on `EINTR`, and the
/// event-loop tick comes from `VTIME`, not from signal-driven wakeups.
pub fn install_signal_handlers() {
    let p = &HOST_POSIX;
    // SAFETY: every handler is an `extern "C" fn(c_int)` with the signature
    // `signal(2)` expects, and each does nothing but store to a static
    // atomic (async-signal-safe).
    unsafe {
        signal(p.sigwinch, on_winch as extern "C" fn(c_int) as usize);
        signal(p.sigint, on_intr as extern "C" fn(c_int) as usize);
        for sig in [p.sigterm, p.sighup, p.sigquit] {
            signal(sig, on_term as extern "C" fn(c_int) as usize);
        }
    }
}

// ---- tty plumbing ----------------------------------------------------------

/// Is `fd` connected to a terminal?
pub fn is_tty(fd: Fd) -> bool {
    // SAFETY: `isatty` only inspects the descriptor table.
    unsafe { isatty(fd) == 1 }
}

/// Open `/dev/tty` for read+write, without making it the controlling tty.
/// Returns `None` when there is no controlling terminal.
pub fn open_tty() -> Option<Fd> {
    const PATH: &[u8] = b"/dev/tty\0";
    // SAFETY: PATH is a NUL-terminated byte string with static lifetime;
    // `open` reads it and returns an int.
    let fd = unsafe {
        open(
            PATH.as_ptr() as *const c_char,
            HOST_POSIX.o_rdwr | HOST_POSIX.o_noctty,
        )
    };
    if fd < 0 {
        None
    } else {
        Some(fd)
    }
}

/// Pick a descriptor suitable for reading keys: stdin when it is a tty,
/// otherwise `/dev/tty` (so `cat x.md | mdr` still has a keyboard).
/// The `bool` is `true` when the caller owns the fd and must [`close_fd`].
pub fn tty_fd() -> Option<(Fd, bool)> {
    if is_tty(super::STDIN) {
        Some((super::STDIN, false))
    } else {
        open_tty().map(|fd| (fd, true))
    }
}

/// Close a descriptor obtained from [`open_tty`].
pub fn close_fd(fd: Fd) {
    if fd > 2 {
        // SAFETY: caller contract is that `fd` came from `open_tty` and is
        // not used afterwards.
        unsafe {
            close(fd);
        }
    }
}

/// Terminal size as `(cols, rows)`, trying stdout then `/dev/tty`.
pub fn winsize() -> Option<(u16, u16)> {
    winsize_of(super::STDOUT)
        .or_else(|| winsize_of(super::STDIN))
        .or_else(|| {
            let fd = open_tty()?;
            let r = winsize_of(fd);
            close_fd(fd);
            r
        })
}

/// Terminal size of a specific descriptor as `(cols, rows)`.
pub fn winsize_of(fd: Fd) -> Option<(u16, u16)> {
    let mut ws = Winsize::zeroed();
    // SAFETY: TIOCGWINSZ writes exactly one `struct winsize` through the
    // pointer; `ws` is a live, correctly-sized, correctly-aligned local.
    let rc = unsafe { ioctl(fd, TIOCGWINSZ as c_ulong, &mut ws as *mut Winsize) };
    let (cols, rows) = ws.cols_rows();
    abi::winsize_result(rc, cols, rows)
}

// ---- raw mode --------------------------------------------------------------

/// Put `fd` into raw mode, returning the previous state for [`restore`].
pub fn set_raw(fd: Fd) -> Option<SavedTermios> {
    let mut t = RawTermios::zeroed();
    // SAFETY: `tcgetattr` fills exactly `sizeof(struct termios)` bytes of our
    // (at least that large) struct; the layout is asserted in `layout.rs`.
    if unsafe { tcgetattr(fd, &mut t as *mut RawTermios) } != 0 {
        return None;
    }
    let saved = SavedTermios(t);
    let mut raw = t;
    raw.set_flags(abi::apply_raw_mode(t.flags(&HOST_ABI), &HOST_ABI), &HOST_ABI);
    // SAFETY: `raw` is a fully initialised termios; TCSAFLUSH is valid.
    if unsafe { tcsetattr(fd, HOST_ABI.tcsaflush, &raw as *const RawTermios) } != 0 {
        return None;
    }
    Some(saved)
}

/// Restore terminal state previously captured by [`set_raw`].
pub fn restore(fd: Fd, saved: &SavedTermios) -> bool {
    // SAFETY: `saved.0` is a termios we obtained from `tcgetattr` on this
    // same descriptor; the pointer is read-only and valid for the call.
    unsafe { tcsetattr(fd, HOST_ABI.tcsaflush, &saved.0 as *const RawTermios) == 0 }
}

// ---- io --------------------------------------------------------------------

/// Read up to `buf.len()` bytes. Retries on `EINTR`.
pub fn read_input(fd: Fd, buf: &mut [u8]) -> ReadOutcome {
    if buf.is_empty() {
        return ReadOutcome::Timeout;
    }
    loop {
        // SAFETY: `buf` is a live slice; we pass its true pointer/length and
        // `read` writes at most `len` bytes.
        let n = unsafe { read(fd, buf.as_mut_ptr(), buf.len()) };
        let e = if n < 0 { os::errno() } else { 0 };
        match classify_read(n, e, &HOST_ABI) {
            ReadStep::Bytes(k) => return ReadOutcome::Bytes(k),
            ReadStep::Timeout => return ReadOutcome::Timeout,
            ReadStep::Retry => continue,
            ReadStep::Error(code) => return ReadOutcome::Error(code),
        }
    }
}

/// Write the whole buffer, looping over short writes and `EINTR`.
/// `Err(errno)` on failure.
pub fn write_all(fd: Fd, buf: &[u8]) -> Result<(), i32> {
    let mut off = 0usize;
    while off < buf.len() {
        // SAFETY: `buf[off..]` is in bounds; `write` reads at most that many
        // bytes from the pointer.
        let n = unsafe { write(fd, buf[off..].as_ptr(), buf.len() - off) };
        let e = if n < 0 { os::errno() } else { 0 };
        match classify_write(n, e, &HOST_ABI) {
            WriteStep::Advance(k) => off += k,
            WriteStep::Retry => continue,
            WriteStep::Fail(code) => return Err(code),
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;
