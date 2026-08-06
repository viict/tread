//! Reading keys, writing frames, and noticing a resize.
//!
//! The shape of `read_input` is dictated by the contract in `sys/mod.rs`: it
//! must come back within roughly 100 ms even when nothing is typed, because
//! that return is the event loop's tick. On unix that is `VMIN=0 / VTIME=1`;
//! here it is `WaitForSingleObject(h, 100)` returning `WAIT_TIMEOUT`.
//!
//! The console is read with plain `ReadFile`, not `ReadConsoleInputW`, because
//! `set_raw` has put the input handle in `ENABLE_VIRTUAL_TERMINAL_INPUT` mode:
//! the bytes that come out are the ANSI escape stream `src/key.rs` already
//! decodes, so the decoder stays shared and host-tested instead of growing a
//! Windows branch. `SetConsoleCP(CP_UTF8)` (see `set_raw`) is what makes those
//! bytes UTF-8 rather than the OEM code page, which the contract requires and
//! `key.rs` assumes.
//!
//! The one wrinkle is that `WaitForSingleObject` signals for *any* input
//! record while `ReadFile` only returns once one of them translates to bytes.
//! Holding Shift would therefore block the loop and stall the resize tick, so
//! the records are peeked first and the ones VT input will never turn into
//! bytes are discarded here. The classification is a pure function in
//! [`crate::sys::win_abi`], tested on Linux.
//!
//! No `unsafe` here either: every call goes through `ffi`.
#![deny(unsafe_code)]

use super::ffi;
use crate::sys::win_abi as abi;
use crate::sys::win_layout::InputRecord;
use crate::sys::{Fd, ReadOutcome, WINCH};
use core::sync::atomic::{AtomicU32, Ordering};

/// How many pending records to inspect per wakeup. Deep enough that a burst of
/// mouse-move records cannot hide a keystroke behind it, small enough to sit on
/// the stack.
const PEEK: usize = 32;

/// The last window size observed, packed by [`abi::pack_dims`]; `0` until the
/// first successful query.
static LAST_DIMS: AtomicU32 = AtomicU32::new(0);

/// Record a freshly observed size and raise the resize flag if it moved.
///
/// This is the whole of Windows' `SIGWINCH`: there is no signal, so the backend
/// polls. It is called from `read_input` (i.e. at most every 100 ms) and from
/// `winsize_of`, and costs one `GetConsoleScreenBufferInfo`.
pub fn note_dims(dims: Option<(u16, u16)>) {
    let cur = abi::pack_dims(dims);
    if cur == 0 {
        return;
    }
    let prev = LAST_DIMS.swap(cur, Ordering::SeqCst);
    if abi::size_changed(prev, cur) {
        WINCH.store(true, Ordering::SeqCst);
    }
}

/// Poll the console for a resize. Cheap enough for every event-loop iteration.
fn poll_resize(fd: Fd) {
    let h = ffi::write_handle(fd);
    if let Some(info) = ffi::screen_info(h) {
        note_dims(info.window_size());
    }
}

/// Read up to `buf.len()` bytes of UTF-8 terminal input.
pub fn read_input(fd: Fd, buf: &mut [u8]) -> ReadOutcome {
    if buf.is_empty() {
        return ReadOutcome::Timeout;
    }
    let h = ffi::read_handle(fd);
    if ffi::is_invalid(h) {
        return ReadOutcome::Error(abi::ERROR_INVALID_HANDLE as i32);
    }
    poll_resize(fd);
    match ffi::wait_for(h, abi::READ_POLL_MS) {
        abi::WAIT_OBJECT_0 => {}
        abi::WAIT_FAILED => return ReadOutcome::Error(ffi::last_error() as i32),
        // WAIT_TIMEOUT, and WAIT_ABANDONED which cannot happen for a console
        // handle: either way there is nothing to read, so tick the loop.
        _ => return ReadOutcome::Timeout,
    }
    match has_readable_bytes(h) {
        Readable::No => ReadOutcome::Timeout,
        Readable::Yes => read_bytes(h, buf, true),
        Readable::NotAConsole => read_bytes(h, buf, false),
    }
}

/// `ReadFile` until it says something conclusive. `console` decides what a
/// successful zero-byte read means — see [`abi::classify_read`].
fn read_bytes(h: ffi::Handle, buf: &mut [u8], console: bool) -> ReadOutcome {
    loop {
        let (ok, n) = ffi::read_file(h, buf);
        let err = if ok { 0 } else { ffi::last_error() };
        match abi::classify_read(ok, n, err, console) {
            abi::WinRead::Bytes(k) => return ReadOutcome::Bytes(k),
            abi::WinRead::Idle => return ReadOutcome::Timeout,
            abi::WinRead::Eof => return ReadOutcome::Eof,
            abi::WinRead::Retry => continue,
            abi::WinRead::Error(e) => return ReadOutcome::Error(e),
        }
    }
}

/// What the pre-read peek concluded about the input handle.
enum Readable {
    /// A console with at least one record that will turn into bytes.
    Yes,
    /// A console whose pending records were all discarded: tick the loop.
    No,
    /// Not a console (a pipe): there are no records, so let `ReadFile` speak
    /// for itself — that is also the only path on which EOF is real.
    NotAConsole,
}

/// Will a `ReadFile` on this handle return promptly with bytes?
///
/// Peeks the pending records; discards the batch when none of them can produce
/// bytes (key-ups, bare modifiers, focus and menu events, and mouse records
/// from a console whose owner left `ENABLE_MOUSE_INPUT` on — this backend never
/// sets it). A `WINDOW_BUFFER_SIZE_EVENT` seen along the way raises the resize
/// flag immediately instead of waiting for the next poll.
fn has_readable_bytes(h: ffi::Handle) -> Readable {
    let mut records = [InputRecord::zeroed(); PEEK];
    let n = match ffi::peek_input(h, &mut records) {
        Some(n) => n as usize,
        None => return Readable::NotAConsole,
    };
    if n == 0 {
        return Readable::No;
    }
    let mut yields = false;
    for r in &records[..n.min(PEEK)] {
        if r.is_resize() {
            WINCH.store(true, Ordering::SeqCst);
        }
        if let Some(k) = r.key_event() {
            if abi::key_record_yields_bytes(k.key_down, k.virtual_key_code, k.unicode_char) {
                yields = true;
                break;
            }
        }
    }
    if yields {
        return Readable::Yes;
    }
    // Consume exactly what was peeked, so the next wait is not woken by the
    // same records for ever.
    let _ = ffi::read_input_records(h, &mut records[..n.min(PEEK)]);
    Readable::No
}

/// Write the whole buffer, looping over short writes.
pub fn write_all(fd: Fd, buf: &[u8]) -> Result<(), i32> {
    let h = ffi::write_handle(fd);
    if ffi::is_invalid(h) {
        return Err(abi::ERROR_INVALID_HANDLE as i32);
    }
    let mut off = 0usize;
    while off < buf.len() {
        let (ok, n) = ffi::write_file(h, &buf[off..]);
        let err = if ok { 0 } else { ffi::last_error() };
        match abi::classify_write(ok, n, err) {
            crate::sys::abi::WriteStep::Advance(k) => off += k,
            crate::sys::abi::WriteStep::Retry => continue,
            crate::sys::abi::WriteStep::Fail(e) => return Err(e),
        }
    }
    Ok(())
}
