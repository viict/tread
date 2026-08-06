//! The **only** module in this crate that contains `unsafe` code.
//!
//! Everything here is hand-written `extern "C"` FFI — no `libc` crate, per
//! SPEC.md §"Hard constraints" #1. The public surface below is deliberately
//! platform-neutral (`Fd`, `SavedTermios`, `ReadOutcome`, plain functions) so a
//! future `sys_windows.rs` can be dropped in behind the same names without any
//! change to `term.rs`, `key.rs`, or anything above them.
//!
//! Layout notes (x86_64 Linux, verified against both glibc and musl headers):
//!   * `tcflag_t` = `c_uint`, `cc_t` = `c_uchar`, `speed_t` = `c_uint`.
//!   * `NCCS` = 32 on Linux for both libcs.
//!   * Both glibc's `<bits/termios.h>` and musl's `arch/x86_64/bits/termios.h`
//!     lay the struct out as iflag/oflag/cflag/lflag, `c_line`, `c_cc[NCCS]`,
//!     then the ispeed/ospeed tail. `sizeof` is 60. We over-allocate a small
//!     tail pad so that any libc writing a *shorter* struct is still safe and a
//!     hypothetical longer one cannot overflow our storage.

use core::sync::atomic::{AtomicBool, Ordering};

/// A raw file descriptor / handle. Opaque to callers above this module.
pub type Fd = i32;

/// Outcome of a non-blocking-ish read from the terminal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReadOutcome {
    /// `n` bytes were placed at the front of the caller's buffer.
    Bytes(usize),
    /// VTIME elapsed with no input. Caller should poll `winch_pending()`.
    Timeout,
    /// End of file on the input handle. The Linux backend cannot tell EOF from
    /// a VTIME timeout (both are a zero-length `read`) so it never produces
    /// this; it is part of the contract `sys_windows.rs` will use (WINDOWS.md).
    #[allow(dead_code)]
    Eof,
    /// Errno (or platform error code).
    Error(i32),
}

static WINCH: AtomicBool = AtomicBool::new(false);
static INTR: AtomicBool = AtomicBool::new(false);
static TERM: AtomicBool = AtomicBool::new(false);

/// True (and cleared) if a terminal-resize signal arrived since the last call.
pub fn winch_pending() -> bool {
    WINCH.swap(false, Ordering::SeqCst)
}

/// True (and cleared) if an interrupt (Ctrl-C at the OS level) arrived.
pub fn interrupt_pending() -> bool {
    INTR.swap(false, Ordering::SeqCst)
}

/// True (and cleared) if a termination signal (SIGTERM/SIGHUP/SIGQUIT) arrived.
/// Their default disposition kills the process outright, stranding the tty in
/// raw mode on the alternate screen (release is `panic = "abort"`, so no `Drop`
/// and no panic hook); catching them makes `kill <pid>` an ordinary quit.
pub fn terminate_pending() -> bool {
    TERM.swap(false, Ordering::SeqCst)
}

/// Standard output handle.
pub const STDOUT: Fd = 1;
/// Standard input handle.
pub const STDIN: Fd = 0;

#[cfg(unix)]
pub use self::unix::*;

#[cfg(unix)]
mod unix {
    use super::{Fd, ReadOutcome, INTR, TERM, WINCH};
    use core::sync::atomic::Ordering;

    // ---- C types -----------------------------------------------------------
    #[allow(non_camel_case_types)]
    type c_int = i32;
    #[allow(non_camel_case_types)]
    type c_uint = u32;
    #[allow(non_camel_case_types)]
    type c_ulong = u64;
    #[allow(non_camel_case_types)]
    type c_char = i8;
    #[allow(non_camel_case_types)]
    type size_t = usize;
    #[allow(non_camel_case_types)]
    type ssize_t = isize;

    const NCCS: usize = 32;

    /// Linux `struct termios` (identical under glibc and musl on x86_64).
    /// `_tail_pad` is reserve only; no libc reads or writes it.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct RawTermios {
        c_iflag: c_uint,
        c_oflag: c_uint,
        c_cflag: c_uint,
        c_lflag: c_uint,
        c_line: u8,
        c_cc: [u8; NCCS],
        c_ispeed: c_uint,
        c_ospeed: c_uint,
        _tail_pad: [u8; 16],
    }

    impl RawTermios {
        const fn zeroed() -> Self {
            RawTermios {
                c_iflag: 0,
                c_oflag: 0,
                c_cflag: 0,
                c_lflag: 0,
                c_line: 0,
                c_cc: [0; NCCS],
                c_ispeed: 0,
                c_ospeed: 0,
                _tail_pad: [0; 16],
            }
        }
    }

    /// Opaque saved terminal state handed back by [`set_raw`].
    #[derive(Clone, Copy)]
    pub struct SavedTermios(RawTermios);

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    // ---- flag constants (x86_64 Linux ABI) ---------------------------------
    const BRKINT: c_uint = 0o000002;
    const INPCK: c_uint = 0o000020;
    const ISTRIP: c_uint = 0o000040;
    const ICRNL: c_uint = 0o000400;
    const IXON: c_uint = 0o002000;
    const OPOST: c_uint = 0o000001;
    const CSIZE: c_uint = 0o000060;
    const CS8: c_uint = 0o000060;
    const ISIG: c_uint = 0o000001;
    const ICANON: c_uint = 0o000002;
    const ECHO: c_uint = 0o000010;
    const IEXTEN: c_uint = 0o100000;
    const VTIME: usize = 5;
    const VMIN: usize = 6;
    const TCSAFLUSH: c_int = 2;

    const TIOCGWINSZ: c_ulong = 0x5413;
    const O_RDWR: c_int = 0o2;
    const O_NOCTTY: c_int = 0o400;
    const EINTR: c_int = 4;
    const EAGAIN: c_int = 11;

    const SIGHUP: c_int = 1;
    const SIGINT: c_int = 2;
    const SIGQUIT: c_int = 3;
    const SIGTERM: c_int = 15;
    const SIGWINCH: c_int = 28;

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

    // Everything below -- the termios layout, the flag octals, TIOCGWINSZ, and
    // the `__errno_location` accessor -- is the *Linux* ABI, shared by glibc
    // and musl. Darwin and the BSDs differ in all four (8-byte `tcflag_t`,
    // `NCCS` = 20, `TIOCGWINSZ` = 0x40087468, `__error()`), and a wrong termios
    // layout compiles cleanly and then corrupts the terminal at runtime. So
    // other unixes are refused here rather than silently mis-served; adding one
    // means a sibling module, exactly like the Windows backend (see WINDOWS.md).
    #[cfg(not(target_os = "linux"))]
    compile_error!(
        "rmarktui's sys.rs implements the Linux termios/ioctl ABI only. \
         Porting to another unix needs its own struct layout and ioctl numbers; \
         see WINDOWS.md for the shape of a backend."
    );

    extern "C" {
        fn __errno_location() -> *mut c_int;
    }

    fn errno() -> c_int {
        // SAFETY: `__errno_location` returns a valid, thread-local pointer for
        // the lifetime of the thread; we only read through it.
        unsafe { *__errno_location() }
    }

    // ---- signals -----------------------------------------------------------

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
    pub fn install_signal_handlers() {
        // SAFETY: every handler is an `extern "C" fn(c_int)` with the signature
        // `signal(2)` expects, and each does nothing but store to a static
        // atomic (async-signal-safe).
        unsafe {
            signal(SIGWINCH, on_winch as extern "C" fn(c_int) as usize);
            signal(SIGINT, on_intr as extern "C" fn(c_int) as usize);
            for sig in [SIGTERM, SIGHUP, SIGQUIT] {
                signal(sig, on_term as extern "C" fn(c_int) as usize);
            }
        }
    }

    // ---- tty plumbing ------------------------------------------------------

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
        let fd = unsafe { open(PATH.as_ptr() as *const c_char, O_RDWR | O_NOCTTY) };
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
        let mut ws = Winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: TIOCGWINSZ writes exactly one `struct winsize` through the
        // pointer; `ws` is a live, correctly-sized, correctly-aligned local.
        let rc = unsafe { ioctl(fd, TIOCGWINSZ, &mut ws as *mut Winsize) };
        if rc != 0 || ws.ws_col == 0 || ws.ws_row == 0 {
            None
        } else {
            Some((ws.ws_col, ws.ws_row))
        }
    }

    // ---- raw mode ----------------------------------------------------------

    /// Put `fd` into raw mode, returning the previous state for [`restore`].
    pub fn set_raw(fd: Fd) -> Option<SavedTermios> {
        let mut t = RawTermios::zeroed();
        // SAFETY: `tcgetattr` fills at most `sizeof(struct termios)` (60) bytes
        // of our (larger) struct.
        if unsafe { tcgetattr(fd, &mut t as *mut RawTermios) } != 0 {
            return None;
        }
        let saved = SavedTermios(t);
        let mut raw = t;
        raw.c_iflag &= !(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
        raw.c_oflag &= !OPOST;
        raw.c_cflag = (raw.c_cflag & !CSIZE) | CS8;
        raw.c_lflag &= !(ECHO | ICANON | IEXTEN | ISIG);
        // VMIN=0 / VTIME=1 -> read returns after at most 100ms with 0 bytes.
        // This gives the event loop a timeout tick to observe `winch_pending()`
        // without relying on EINTR semantics, which differ across libcs.
        raw.c_cc[VMIN] = 0;
        raw.c_cc[VTIME] = 1;
        // SAFETY: `raw` is a fully initialised termios; TCSAFLUSH is valid.
        if unsafe { tcsetattr(fd, TCSAFLUSH, &raw as *const RawTermios) } != 0 {
            return None;
        }
        Some(saved)
    }

    /// Restore terminal state previously captured by [`set_raw`].
    pub fn restore(fd: Fd, saved: &SavedTermios) -> bool {
        // SAFETY: `saved.0` is a termios we obtained from `tcgetattr` on this
        // same descriptor; the pointer is read-only and valid for the call.
        unsafe { tcsetattr(fd, TCSAFLUSH, &saved.0 as *const RawTermios) == 0 }
    }

    // ---- io ----------------------------------------------------------------

    /// Read up to `buf.len()` bytes. Retries on `EINTR`.
    pub fn read_input(fd: Fd, buf: &mut [u8]) -> ReadOutcome {
        if buf.is_empty() {
            return ReadOutcome::Timeout;
        }
        loop {
            // SAFETY: `buf` is a live slice; we pass its true pointer/length and
            // `read` writes at most `len` bytes.
            let n = unsafe { read(fd, buf.as_mut_ptr(), buf.len()) };
            if n > 0 {
                return ReadOutcome::Bytes(n as usize);
            }
            if n == 0 {
                // With VMIN=0/VTIME>0 a zero-length read means "timed out".
                return ReadOutcome::Timeout;
            }
            let e = errno();
            if e == EINTR {
                continue;
            }
            if e == EAGAIN {
                return ReadOutcome::Timeout;
            }
            return ReadOutcome::Error(e);
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
            if n > 0 {
                off += n as usize;
                continue;
            }
            if n == 0 {
                return Err(0);
            }
            let e = errno();
            if e == EINTR || e == EAGAIN {
                continue;
            }
            return Err(e);
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn termios_layout_matches_linux_abi() {
            // glibc and musl both report sizeof(struct termios) == 60 on
            // x86_64-linux. Our struct carries a 16-byte reserve tail, so it
            // must be at least 60 and never smaller than the real one.
            assert!(core::mem::size_of::<RawTermios>() >= 60);
            assert_eq!(core::mem::align_of::<RawTermios>(), 4);
            let t = RawTermios::zeroed();
            let base = &t as *const RawTermios as usize;
            let off = |p: usize| p - base;
            assert_eq!(off(&t.c_iflag as *const _ as usize), 0);
            assert_eq!(off(&t.c_oflag as *const _ as usize), 4);
            assert_eq!(off(&t.c_cflag as *const _ as usize), 8);
            assert_eq!(off(&t.c_lflag as *const _ as usize), 12);
            assert_eq!(off(&t.c_line as *const _ as usize), 16);
            assert_eq!(off(&t.c_cc as *const _ as usize), 17);
            assert_eq!(off(&t.c_ispeed as *const _ as usize), 52);
            assert_eq!(off(&t.c_ospeed as *const _ as usize), 56);
        }

        #[test]
        fn winsize_layout() {
            assert_eq!(core::mem::size_of::<Winsize>(), 8);
        }

        #[test]
        fn signal_flags_have_expected_values() {
            assert_eq!(ICANON, 2);
            assert_eq!(ECHO, 8);
            assert_eq!(ISIG, 1);
            assert_eq!(IEXTEN, 0x8000);
            assert_eq!(IXON, 0x400);
            assert_eq!(ICRNL, 0x100);
            assert_eq!(OPOST, 1);
            assert_eq!(TIOCGWINSZ, 0x5413);
            assert_eq!(VMIN, 6);
            assert_eq!(VTIME, 5);
        }

        #[test]
        fn writing_to_a_closed_fd_reports_an_error() {
            // fd 1_000_000 is guaranteed not to be open in the test harness.
            assert!(write_all(1_000_000, b"x").is_err());
        }

        #[test]
        fn winsize_of_a_non_tty_is_none() {
            assert_eq!(winsize_of(1_000_000), None);
        }

        #[test]
        fn signal_numbers_are_the_linux_ones() {
            assert_eq!(SIGHUP, 1);
            assert_eq!(SIGINT, 2);
            assert_eq!(SIGQUIT, 3);
            assert_eq!(SIGTERM, 15);
            assert_eq!(SIGWINCH, 28);
        }

        #[test]
        fn signal_handlers_install_and_flags_start_clear() {
            install_signal_handlers();
            // Nothing has been raised in-process, so both must read false.
            // (`terminate_pending` is deliberately not asserted here: another
            // test raises SIGTERM, and the flag is process-wide.)
            assert!(!super::super::winch_pending());
            assert!(!super::super::interrupt_pending());
        }

        /// Raising SIGTERM in-process must set the flag instead of killing
        /// us. The end-to-end proof (raw mode and the alternate screen really
        /// come back) is `tools/soak_pty.py`'s signal-teardown pass.
        #[test]
        fn a_termination_signal_sets_the_flag_and_does_not_kill_us() {
            extern "C" {
                fn raise(sig: c_int) -> c_int;
            }
            install_signal_handlers();
            let _ = super::super::terminate_pending();
            // SAFETY: `raise` delivers a signal to this process only; the
            // handler installed above stores to an atomic and returns.
            unsafe { raise(SIGTERM) };
            assert!(super::super::terminate_pending());
            assert!(!super::super::terminate_pending(), "the flag is consumed");
        }
    }
}

// The non-unix placeholder backend lives next door: it contains no `unsafe`,
// and keeping it out of this file leaves `sys.rs` purely the Linux FFI.
#[cfg(not(unix))]
#[path = "sys_stub.rs"]
mod portable_stub;

#[cfg(not(unix))]
pub use self::portable_stub::*;
