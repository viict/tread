//! Windows backend: the Console API half of the [`crate::sys`] contract.
//!
//! Hand-written `extern "system"` FFI against `kernel32` — no `windows-sys`, no
//! `winapi`, no `libc`, per SPEC.md §"Hard constraints" #1. As on unix, this
//! file is only the plumbing: every value that can be computed without the OS
//! lives in [`crate::sys::win_abi`] (console-mode arithmetic, `srWindow`
//! geometry, resize comparison, record and error classification) and every C
//! struct in [`crate::sys::win_layout`], both compiled and unit-tested on the
//! Linux host, because nothing in this project can *run* a Windows binary.
//!
//! ```text
//! windows.rs        the contract: handles, raw mode, size, restore   <- here
//!   windows/ffi.rs  kernel32 declarations + the Fd -> HANDLE table
//!   windows/io.rs   read_input / write_all / resize polling
//!   windows/abi.rs  pure arithmetic and constants   (compiled everywhere)
//!   windows/layout.rs  pure C struct layouts        (compiled everywhere)
//! ```
//!
//! Three things are worth stating plainly because they are product
//! requirements rather than implementation details:
//!
//! * **The mouse is never captured.** `ENABLE_MOUSE_INPUT` is never set and
//!   `ENABLE_QUICK_EDIT_MODE` is never cleared — quick edit *is* how console
//!   users drag-select, and it is only honoured while `ENABLE_EXTENDED_FLAGS`
//!   is set, so [`crate::sys::win_abi::raw_input_mode`] re-asserts both bits
//!   together instead of trusting the read-modify-write round trip. This is the
//!   Windows spelling of "never emit `?1000h`" (SPEC.md §"Hard constraints" #5).
//! * **Input arrives as ANSI.** `ENABLE_VIRTUAL_TERMINAL_INPUT` makes the
//!   console deliver arrows, Home/End, function keys and bracketed paste as
//!   escape sequences, and `SetConsoleCP(CP_UTF8)` makes the bytes UTF-8, so
//!   `src/key.rs` is used unchanged and stays host-tested.
//! * **VT output can be unavailable.** On a pre-1703 conhost
//!   `ENABLE_VIRTUAL_TERMINAL_PROCESSING` cannot be set. Rather than paint that
//!   console with escape codes it would print literally, [`set_raw`] undoes
//!   everything it changed and returns `None` — the documented "no raw mode"
//!   signal (WINDOWS.md §1) — and [`vt_output_supported`] stays false so a
//!   caller can distinguish "no console" from "console without VT".
//!
//! This file contains no `unsafe` itself — all of it is in `windows/ffi.rs`,
//! which opts back in explicitly. The `deny` below is what keeps it that way.
#![deny(unsafe_code)]

use crate::sys::win_abi as abi;
use crate::sys::Fd;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

#[path = "windows/ffi.rs"]
mod ffi;
#[path = "windows/io.rs"]
mod io;

pub use io::{read_input, write_all};

// ---------------------------------------------------------------------------
// Saved state
// ---------------------------------------------------------------------------

/// Everything [`restore`] needs to put the console back byte-for-byte: the two
/// console modes and the two code pages, plus the handles they belong to (a
/// `HANDLE` does not fit in an `Fd`, and `restore` must work from the panic hook
/// even if the handle table has already been torn down).
///
/// `out_mode` is `None` when the write side was not a console at all — stdout
/// redirected to a file — in which case nothing was changed there.
#[derive(Clone, Copy)]
pub struct SavedTermios {
    in_h: usize,
    out_h: usize,
    in_mode: u32,
    out_mode: Option<u32>,
    in_cp: u32,
    out_cp: u32,
}

// The same state again, in atomics, for the console control handler: it runs on
// a thread the OS injects while the process is being torn down, so it can touch
// nothing but lock-free storage and syscalls.
static EMERG_ACTIVE: AtomicBool = AtomicBool::new(false);
static EMERG_IN_H: AtomicUsize = AtomicUsize::new(0);
static EMERG_OUT_H: AtomicUsize = AtomicUsize::new(0);
static EMERG_IN_MODE: AtomicU32 = AtomicU32::new(0);
/// `u32::MAX` is the "output was not a console" sentinel; no real console mode
/// has every bit set.
static EMERG_OUT_MODE: AtomicU32 = AtomicU32::new(u32::MAX);
static EMERG_IN_CP: AtomicU32 = AtomicU32::new(0);
static EMERG_OUT_CP: AtomicU32 = AtomicU32::new(0);
static HANDLERS_INSTALLED: AtomicBool = AtomicBool::new(false);
static VT_OUTPUT: AtomicBool = AtomicBool::new(false);

fn arm_emergency(s: &SavedTermios) {
    EMERG_IN_H.store(s.in_h, Ordering::SeqCst);
    EMERG_OUT_H.store(s.out_h, Ordering::SeqCst);
    EMERG_IN_MODE.store(s.in_mode, Ordering::SeqCst);
    EMERG_OUT_MODE.store(s.out_mode.unwrap_or(u32::MAX), Ordering::SeqCst);
    EMERG_IN_CP.store(s.in_cp, Ordering::SeqCst);
    EMERG_OUT_CP.store(s.out_cp, Ordering::SeqCst);
    EMERG_ACTIVE.store(true, Ordering::SeqCst);
}

/// Put the console modes back from whatever context: the control handler, or
/// [`restore`]. Idempotent, allocation-free, cannot panic.
fn disarm_emergency() {
    if !EMERG_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    let in_h = EMERG_IN_H.load(Ordering::SeqCst) as ffi::Handle;
    let out_h = EMERG_OUT_H.load(Ordering::SeqCst) as ffi::Handle;
    let out_mode = EMERG_OUT_MODE.load(Ordering::SeqCst);
    // Only when the write side is a console we configured: `u32::MAX` means it
    // was a redirected file, and escape bytes do not belong in the user's file.
    // VT processing is still on at this point — that is why this goes first.
    if out_mode != u32::MAX {
        ffi::write_file(out_h, abi::LEAVE_SCREEN);
    }
    ffi::set_console_mode(in_h, EMERG_IN_MODE.load(Ordering::SeqCst));
    if out_mode != u32::MAX {
        ffi::set_console_mode(out_h, out_mode);
    }
    ffi::set_code_pages(
        EMERG_IN_CP.load(Ordering::SeqCst),
        EMERG_OUT_CP.load(Ordering::SeqCst),
    );
}

// ---------------------------------------------------------------------------
// Control events — the Windows counterpart of the unix signal handlers
// ---------------------------------------------------------------------------

/// `PHANDLER_ROUTINE`. Touches only atomics and console calls.
///
/// Ctrl-C also arrives as a `\x03` keystroke (this backend clears
/// `ENABLE_PROCESSED_INPUT`), and `key.rs` decodes it; the handler is here for
/// the out-of-band cases. `CTRL_CLOSE_EVENT` / `CTRL_LOGOFF_EVENT` /
/// `CTRL_SHUTDOWN_EVENT` kill the process shortly after the handler returns, so
/// the console mode is put back *inside* the handler rather than by trusting
/// the event loop to get another turn — the same reason the unix backend
/// catches SIGTERM/SIGHUP/SIGQUIT.
extern "system" fn ctrl_handler(event: u32) -> i32 {
    match abi::ctrl_action(event) {
        abi::CtrlAction::Interrupt => {
            crate::sys::INTR.store(true, Ordering::SeqCst);
            1
        }
        abi::CtrlAction::Terminate => {
            crate::sys::TERM.store(true, Ordering::SeqCst);
            disarm_emergency();
            1
        }
        abi::CtrlAction::Ignore => 0,
    }
}

/// Install the console control handler. Idempotent.
pub fn install_signal_handlers() {
    if HANDLERS_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    ffi::add_ctrl_handler(ctrl_handler);
}

// ---------------------------------------------------------------------------
// Handles
// ---------------------------------------------------------------------------

/// Is this handle a console? `GetConsoleMode` succeeding is the exact test;
/// `GetFileType == FILE_TYPE_CHAR` is weaker and also true of `NUL`.
pub fn is_tty(fd: Fd) -> bool {
    ffi::console_mode(ffi::read_handle(fd)).is_some()
}

/// The `/dev/tty` equivalent: `CONIN$` plus `CONOUT$`.
///
/// This is what makes `type x.md | mdr` work — stdin is a pipe, so the keys come
/// from `CONIN$` — and the `CONOUT$` half means the pager can still paint when
/// stdout is redirected too. Both are opened read+write and shared, as the
/// console documentation requires.
pub fn open_tty() -> Option<Fd> {
    let access = abi::GENERIC_READ | abi::GENERIC_WRITE;
    let read = ffi::open_console_file(&abi::CONIN, access)?;
    let write = match ffi::open_console_file(&abi::CONOUT, access) {
        Some(w) => w,
        None => {
            ffi::close_handle(read);
            return None;
        }
    };
    match ffi::alloc_tty(read, write) {
        Some(fd) => Some(fd),
        None => {
            ffi::close_handle(read);
            ffi::close_handle(write);
            None
        }
    }
}

/// A handle to read keys from: stdin when it is a console, otherwise `CONIN$`.
/// The `bool` is "the caller owns this and must [`close_fd`] it".
pub fn tty_fd() -> Option<(Fd, bool)> {
    if is_tty(crate::sys::STDIN) {
        Some((crate::sys::STDIN, false))
    } else {
        open_tty().map(|fd| (fd, true))
    }
}

/// Close a handle pair obtained from [`open_tty`]. Standard handles are ignored.
pub fn close_fd(fd: Fd) {
    if fd > 2 {
        if let Some((r, w)) = ffi::take_tty(fd) {
            ffi::close_handle(r);
            ffi::close_handle(w);
        }
    }
}

// ---------------------------------------------------------------------------
// Size
// ---------------------------------------------------------------------------

/// Visible terminal size as `(cols, rows)`, trying stdout, then stdin's console,
/// then a freshly opened `CONOUT$`.
pub fn winsize() -> Option<(u16, u16)> {
    winsize_of(crate::sys::STDOUT)
        .or_else(|| winsize_of(crate::sys::STDIN))
        .or_else(|| {
            let fd = open_tty()?;
            let r = winsize_of(fd);
            close_fd(fd);
            r
        })
}

/// Size of one handle's console, from `srWindow` — the visible window, never
/// `dwSize`, which is the scrollback buffer and would make the pager lay out
/// frames the user cannot see.
///
/// Every successful query also feeds the resize detector: Windows has no
/// `SIGWINCH`, so `winch_pending()` is driven by comparing this against the last
/// observed size.
pub fn winsize_of(fd: Fd) -> Option<(u16, u16)> {
    let info = ffi::screen_info(ffi::write_handle(fd));
    let dims = info.and_then(|i| i.window_size());
    let out = abi::winsize_result(info.is_some(), dims);
    io::note_dims(out);
    out
}

// ---------------------------------------------------------------------------
// Raw mode
// ---------------------------------------------------------------------------

/// What happened to the output handle in [`set_raw`].
enum OutSetup {
    /// VT processing enabled; the previous mode is inside.
    Configured(u32),
    /// Not a console (redirected to a file or pipe): nothing to configure and
    /// nothing to restore. Raw mode still succeeds — the keyboard is what
    /// matters — and the ANSI simply goes into the file, exactly as on unix.
    NotAConsole,
    /// A console that refuses `ENABLE_VIRTUAL_TERMINAL_PROCESSING`.
    VtUnsupported,
}

fn configure_output(h: ffi::Handle) -> OutSetup {
    match ffi::console_mode(h) {
        None => OutSetup::NotAConsole,
        Some(m) => {
            if ffi::set_console_mode(h, abi::raw_output_mode(m)) {
                OutSetup::Configured(m)
            } else {
                OutSetup::VtUnsupported
            }
        }
    }
}

/// Enter raw mode on `fd`'s console pair, returning the previous state.
///
/// `None` means "this cannot be an interactive terminal": either the input
/// handle is not a console, or it is one whose VT output cannot be turned on.
/// In the second case everything already changed is undone first, so a refusal
/// leaves the console exactly as found.
pub fn set_raw(fd: Fd) -> Option<SavedTermios> {
    let hin = ffi::read_handle(fd);
    let hout = ffi::write_handle(fd);
    let in_mode = ffi::console_mode(hin)?;
    if !ffi::set_console_mode(hin, abi::raw_input_mode(in_mode)) {
        return None;
    }
    let (in_cp, out_cp) = ffi::code_pages();
    ffi::set_code_pages(abi::CP_UTF8, abi::CP_UTF8);
    let out_mode = match configure_output(hout) {
        OutSetup::Configured(m) => {
            VT_OUTPUT.store(true, Ordering::SeqCst);
            Some(m)
        }
        OutSetup::NotAConsole => {
            VT_OUTPUT.store(false, Ordering::SeqCst);
            None
        }
        OutSetup::VtUnsupported => {
            VT_OUTPUT.store(false, Ordering::SeqCst);
            ffi::set_console_mode(hin, in_mode);
            ffi::set_code_pages(in_cp, out_cp);
            return None;
        }
    };
    let saved = SavedTermios {
        in_h: hin as usize,
        out_h: hout as usize,
        in_mode,
        out_mode,
        in_cp,
        out_cp,
    };
    arm_emergency(&saved);
    io::note_dims(ffi::screen_info(hout).and_then(|i| i.window_size()));
    Some(saved)
}

/// Undo [`set_raw`] exactly. Called from a panic hook, so it allocates nothing
/// and cannot panic. Idempotent.
pub fn restore(_fd: Fd, saved: &SavedTermios) -> bool {
    // The handles come from `saved`, not from `_fd`: this can run after the
    // handle table has been emptied.
    let ok_in = ffi::set_console_mode(saved.in_h as ffi::Handle, saved.in_mode);
    let ok_out = match saved.out_mode {
        Some(m) => ffi::set_console_mode(saved.out_h as ffi::Handle, m),
        None => true,
    };
    ffi::set_code_pages(saved.in_cp, saved.out_cp);
    EMERG_ACTIVE.store(false, Ordering::SeqCst);
    ok_in && ok_out
}

/// Can the console interpret ANSI escape sequences?
///
/// False before [`set_raw`] has run, and false afterwards on a console too old
/// for `ENABLE_VIRTUAL_TERMINAL_PROCESSING` — though in that case `set_raw`
/// itself returns `None`, so the reader never reaches an interactive frame.
/// Exposed so the safe layer can degrade to plain, colourless output instead of
/// assuming ANSI works. (Not yet consulted by `term.rs`, hence `dead_code`:
/// the mirror of this function for every other OS is in `sys/mod.rs`.)
#[allow(dead_code)]
pub fn vt_output_supported() -> bool {
    VT_OUTPUT.load(Ordering::SeqCst)
}
