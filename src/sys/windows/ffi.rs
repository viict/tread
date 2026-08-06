//! Hand-written `kernel32` bindings and the handle table, the only place in the
//! Windows backend that talks to the OS.
//!
//! Per SPEC.md §"Hard constraints" #1 there is no `windows-sys`, no `winapi` and
//! no `libc`: every entry point below is an `extern "system"` declaration
//! written out against the SDK headers. `extern "system"` is `stdcall` on
//! `i686` and the ordinary C convention on `x86_64`/`aarch64`, which is exactly
//! what the Windows API uses on each — spelling it this way is what makes the
//! same source correct for both the MSVC and the GNU targets.
//!
//! Nothing here decides anything: every value that can be computed without the
//! OS comes from [`crate::sys::win_abi`], which the Linux host tests.
//!
//! This is the *only* file in the Windows backend that may contain `unsafe`;
//! its parents `windows.rs` and `windows/io.rs` both `deny` it, and this opt-in
//! is deliberately narrow — one module, every block individually justified.
#![allow(unsafe_code)]

use crate::sys::win_abi as abi;
use crate::sys::win_layout::{InputRecord, ScreenBufferInfo};
use crate::sys::Fd;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

/// `HANDLE`. Opaque; only ever passed straight back to the console API.
pub type Handle = *mut c_void;

/// `BOOL`: zero is failure, anything else success.
type Bool = i32;

#[link(name = "kernel32")]
extern "system" {
    fn GetStdHandle(nStdHandle: u32) -> Handle;
    fn CloseHandle(hObject: Handle) -> Bool;
    fn GetLastError() -> u32;
    fn CreateFileW(
        lpFileName: *const u16,
        dwDesiredAccess: u32,
        dwShareMode: u32,
        lpSecurityAttributes: *mut c_void,
        dwCreationDisposition: u32,
        dwFlagsAndAttributes: u32,
        hTemplateFile: Handle,
    ) -> Handle;
    fn GetConsoleMode(hConsoleHandle: Handle, lpMode: *mut u32) -> Bool;
    fn SetConsoleMode(hConsoleHandle: Handle, dwMode: u32) -> Bool;
    fn GetConsoleScreenBufferInfo(hConsoleOutput: Handle, lpInfo: *mut ScreenBufferInfo) -> Bool;
    fn GetConsoleCP() -> u32;
    fn GetConsoleOutputCP() -> u32;
    fn SetConsoleCP(wCodePageID: u32) -> Bool;
    fn SetConsoleOutputCP(wCodePageID: u32) -> Bool;
    fn WaitForSingleObject(hHandle: Handle, dwMilliseconds: u32) -> u32;
    fn ReadFile(
        hFile: Handle,
        lpBuffer: *mut u8,
        nNumberOfBytesToRead: u32,
        lpNumberOfBytesRead: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> Bool;
    fn WriteFile(
        hFile: Handle,
        lpBuffer: *const u8,
        nNumberOfBytesToWrite: u32,
        lpNumberOfBytesWritten: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> Bool;
    fn PeekConsoleInputW(
        hConsoleInput: Handle,
        lpBuffer: *mut InputRecord,
        nLength: u32,
        lpNumberOfEventsRead: *mut u32,
    ) -> Bool;
    fn ReadConsoleInputW(
        hConsoleInput: Handle,
        lpBuffer: *mut InputRecord,
        nLength: u32,
        lpNumberOfEventsRead: *mut u32,
    ) -> Bool;
    fn SetConsoleCtrlHandler(
        HandlerRoutine: Option<extern "system" fn(u32) -> Bool>,
        Add: Bool,
    ) -> Bool;
}

// ---------------------------------------------------------------------------
// Handle table
// ---------------------------------------------------------------------------
//
// `Fd` is an `i32` the rest of the crate treats as opaque, so on Windows it is
// a small index rather than a `HANDLE as i32` (which would truncate a 64-bit
// pointer). 0/1/2 keep their unix meaning; anything from `FD_BASE` up is a slot
// in the table below, holding the **pair** of handles `open_tty` created:
// `CONIN$` to read keys from and `CONOUT$` to paint to. Keeping them together
// is what makes `term.rs`'s "if stdout is not a console, write to the tty
// handle instead" fall back to something writable.

const SLOTS: usize = 4;
const FD_BASE: Fd = 3;

/// 0 means "empty"; no console handle is ever NULL.
static TTY_READ: [AtomicUsize; SLOTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static TTY_WRITE: [AtomicUsize; SLOTS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

fn slot(fd: Fd) -> Option<usize> {
    let i = fd.checked_sub(FD_BASE)? as usize;
    if i < SLOTS {
        Some(i)
    } else {
        None
    }
}

/// Claim a free slot for a `(CONIN$, CONOUT$)` pair. `None` when the table is
/// full, which cannot happen in practice: `main` opens at most one tty.
pub fn alloc_tty(read: Handle, write: Handle) -> Option<Fd> {
    for i in 0..SLOTS {
        if TTY_READ[i]
            .compare_exchange(0, read as usize, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            TTY_WRITE[i].store(write as usize, Ordering::SeqCst);
            return Some(FD_BASE + i as Fd);
        }
    }
    None
}

/// Release a slot, returning the handles that were in it.
pub fn take_tty(fd: Fd) -> Option<(Handle, Handle)> {
    let i = slot(fd)?;
    let r = TTY_READ[i].swap(0, Ordering::SeqCst);
    let w = TTY_WRITE[i].swap(0, Ordering::SeqCst);
    if r == 0 {
        None
    } else {
        Some((r as Handle, w as Handle))
    }
}

/// The handle to read from for this `Fd`.
pub fn read_handle(fd: Fd) -> Handle {
    match fd {
        0 => std_handle(abi::STD_INPUT_HANDLE),
        1 => std_handle(abi::STD_OUTPUT_HANDLE),
        2 => std_handle(abi::STD_ERROR_HANDLE),
        _ => slot(fd)
            .map(|i| TTY_READ[i].load(Ordering::SeqCst) as Handle)
            .unwrap_or(core::ptr::null_mut()),
    }
}

/// The handle to write to — and to query the screen buffer of — for this `Fd`.
///
/// The write side of the *stdin* slot is the standard output handle: `set_raw`
/// is handed the input `Fd` and must still configure the screen it paints on,
/// exactly as `tcsetattr` on unix configures one tty for both directions.
pub fn write_handle(fd: Fd) -> Handle {
    match fd {
        0 | 1 => std_handle(abi::STD_OUTPUT_HANDLE),
        2 => std_handle(abi::STD_ERROR_HANDLE),
        _ => slot(fd)
            .map(|i| TTY_WRITE[i].load(Ordering::SeqCst) as Handle)
            .unwrap_or(core::ptr::null_mut()),
    }
}

/// True for the two values the API uses to mean "no handle".
pub fn is_invalid(h: Handle) -> bool {
    h.is_null() || h as usize == abi::INVALID_HANDLE_VALUE
}

// ---------------------------------------------------------------------------
// Thin safe wrappers
// ---------------------------------------------------------------------------

pub fn std_handle(which: u32) -> Handle {
    // SAFETY: `GetStdHandle` takes a `DWORD` selector and returns a handle; it
    // touches no memory of ours.
    unsafe { GetStdHandle(which) }
}

pub fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error slot.
    unsafe { GetLastError() }
}

/// `GetConsoleMode`, which is also the strongest available `isatty`: it
/// succeeds only for a real console handle, where `GetFileType` would also say
/// `FILE_TYPE_CHAR` for `NUL` and for a serial port.
pub fn console_mode(h: Handle) -> Option<u32> {
    if is_invalid(h) {
        return None;
    }
    let mut mode: u32 = 0;
    // SAFETY: `h` is non-null and `mode` is a live, aligned `DWORD` the call
    // writes at most once.
    let ok = unsafe { GetConsoleMode(h, &mut mode as *mut u32) };
    if ok != 0 {
        Some(mode)
    } else {
        None
    }
}

pub fn set_console_mode(h: Handle, mode: u32) -> bool {
    if is_invalid(h) {
        return false;
    }
    // SAFETY: both arguments are plain values; the call writes nothing of ours.
    unsafe { SetConsoleMode(h, mode) != 0 }
}

/// `GetConsoleScreenBufferInfo`. `None` on any failure — the caller must not
/// read the struct in that case, so it is not returned.
pub fn screen_info(h: Handle) -> Option<ScreenBufferInfo> {
    if is_invalid(h) {
        return None;
    }
    let mut info = ScreenBufferInfo::zeroed();
    // SAFETY: `info` is a live, correctly sized and aligned
    // CONSOLE_SCREEN_BUFFER_INFO — the layout is `const`-asserted in
    // `win_layout` on every target — and the call fills exactly that struct.
    let ok = unsafe { GetConsoleScreenBufferInfo(h, &mut info as *mut ScreenBufferInfo) };
    if ok != 0 {
        Some(info)
    } else {
        None
    }
}

/// Open one of the console pseudo-files. `name` must be NUL-terminated UTF-16.
pub fn open_console_file(name: &[u16], access: u32) -> Option<Handle> {
    // SAFETY: `name` is a NUL-terminated UTF-16 string (asserted by the caller's
    // constant); the security-attributes and template arguments are NULL, which
    // `CreateFileW` documents as "defaults".
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            abi::FILE_SHARE_READ | abi::FILE_SHARE_WRITE,
            core::ptr::null_mut(),
            abi::OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        )
    };
    if is_invalid(h) {
        None
    } else {
        Some(h)
    }
}

pub fn close_handle(h: Handle) {
    if !is_invalid(h) {
        // SAFETY: caller contract is that `h` came from `open_console_file` and
        // is not used afterwards.
        unsafe {
            CloseHandle(h);
        }
    }
}

/// `(input, output)` console code pages.
pub fn code_pages() -> (u32, u32) {
    // SAFETY: neither call takes arguments or touches our memory.
    unsafe { (GetConsoleCP(), GetConsoleOutputCP()) }
}

/// Set both console code pages, ignoring failure: a console that refuses UTF-8
/// still works, it just cannot render non-ASCII, which is not worth aborting a
/// document read for.
pub fn set_code_pages(input: u32, output: u32) {
    // SAFETY: plain integer arguments.
    unsafe {
        SetConsoleCP(input);
        SetConsoleOutputCP(output);
    }
}

pub fn wait_for(h: Handle, ms: u32) -> u32 {
    if is_invalid(h) {
        return abi::WAIT_FAILED;
    }
    // SAFETY: `h` is a live handle; the call blocks at most `ms` milliseconds
    // and touches no memory of ours.
    unsafe { WaitForSingleObject(h, ms) }
}

/// One `ReadFile`. Returns `(ok, bytes_read)`.
pub fn read_file(h: Handle, buf: &mut [u8]) -> (bool, u32) {
    let mut n: u32 = 0;
    let len = buf.len().min(u32::MAX as usize) as u32;
    // SAFETY: `buf` is a live slice and `len` never exceeds it; `n` is a live
    // `DWORD`; the overlapped pointer is NULL, i.e. a synchronous read.
    let ok = unsafe {
        ReadFile(
            h,
            buf.as_mut_ptr(),
            len,
            &mut n as *mut u32,
            core::ptr::null_mut(),
        )
    };
    (ok != 0, n)
}

/// One `WriteFile`. Returns `(ok, bytes_written)`.
pub fn write_file(h: Handle, buf: &[u8]) -> (bool, u32) {
    let mut n: u32 = 0;
    let len = buf.len().min(u32::MAX as usize) as u32;
    // SAFETY: `buf` is a live slice read for exactly `len` bytes; `n` is a live
    // `DWORD`; the overlapped pointer is NULL.
    let ok = unsafe {
        WriteFile(
            h,
            buf.as_ptr(),
            len,
            &mut n as *mut u32,
            core::ptr::null_mut(),
        )
    };
    (ok != 0, n)
}

/// `PeekConsoleInputW` — look at pending records without consuming them.
/// Returns the number filled in, or `None` if the handle is not a console.
pub fn peek_input(h: Handle, out: &mut [InputRecord]) -> Option<u32> {
    let mut n: u32 = 0;
    let len = out.len().min(u32::MAX as usize) as u32;
    // SAFETY: `out` is a live slice of correctly laid out INPUT_RECORDs (size
    // asserted in `win_layout`) and `len` is its true length.
    let ok = unsafe { PeekConsoleInputW(h, out.as_mut_ptr(), len, &mut n as *mut u32) };
    if ok != 0 {
        Some(n)
    } else {
        None
    }
}

/// `ReadConsoleInputW` — consume records. Used only to discard the ones VT
/// input will never turn into bytes.
pub fn read_input_records(h: Handle, out: &mut [InputRecord]) -> Option<u32> {
    let mut n: u32 = 0;
    let len = out.len().min(u32::MAX as usize) as u32;
    if len == 0 {
        return Some(0);
    }
    // SAFETY: as `peek_input`; this variant additionally removes the records it
    // returns from the console's input queue.
    let ok = unsafe { ReadConsoleInputW(h, out.as_mut_ptr(), len, &mut n as *mut u32) };
    if ok != 0 {
        Some(n)
    } else {
        None
    }
}

/// Install a `HandlerRoutine`. Returns false if the console refuses it.
pub fn add_ctrl_handler(f: extern "system" fn(u32) -> i32) -> bool {
    // SAFETY: `f` is an `extern "system" fn(DWORD) -> BOOL`, the exact
    // `PHANDLER_ROUTINE` signature, with `'static` lifetime.
    unsafe { SetConsoleCtrlHandler(Some(f), 1) != 0 }
}
