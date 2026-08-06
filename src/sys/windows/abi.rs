//! Pure Windows console ABI: constants and arithmetic, no `unsafe`, no calls.
//!
//! This module is compiled on **every** target — it is declared unconditionally
//! by `sys/mod.rs` next to [`crate::sys::abi`] — for the same reason the Darwin
//! tables are: there is no Windows machine in this project's loop, so anything
//! that can be written down as arithmetic is written down here, where the Linux
//! host compiles it, `const _: () = assert!(…)` pins it, and `cargo test` runs
//! it. `sys/windows.rs` and its FFI submodules are then only "call the console
//! API, hand the result to a function in this module".
//!
//! Authorities: Windows SDK `consoleapi.h` / `wincon.h` / `wincontypes.h`
//! (console mode flags, event-record types, `CTRL_*_EVENT`), `winbase.h`
//! (`STD_*_HANDLE`, `WAIT_*`), `winerror.h` (`ERROR_*`), `winnt.h`
//! (`GENERIC_*`), and the documented behaviour of `SetConsoleMode`,
//! `GetConsoleScreenBufferInfo` and `ENABLE_VIRTUAL_TERMINAL_INPUT`.
#![deny(unsafe_code)]
// Every item here exists for the Windows backend; on Linux/Darwin builds the
// module is compiled and tested but not called.
#![allow(dead_code)]

use crate::sys::abi::WriteStep;

// ---------------------------------------------------------------------------
// Handles, files, code pages
// ---------------------------------------------------------------------------

/// `GetStdHandle` selectors (`winbase.h`): `(DWORD)-10 / -11 / -12`.
pub const STD_INPUT_HANDLE: u32 = -10i32 as u32;
pub const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
pub const STD_ERROR_HANDLE: u32 = -12i32 as u32;

/// `INVALID_HANDLE_VALUE` is `(HANDLE)-1`, i.e. all bits set.
pub const INVALID_HANDLE_VALUE: usize = usize::MAX;

/// `CreateFileW` arguments for the `CONIN$` / `CONOUT$` open (`winnt.h`).
pub const GENERIC_READ: u32 = 0x8000_0000;
pub const GENERIC_WRITE: u32 = 0x4000_0000;
pub const FILE_SHARE_READ: u32 = 0x0000_0001;
pub const FILE_SHARE_WRITE: u32 = 0x0000_0002;
pub const OPEN_EXISTING: u32 = 3;

/// UTF-8 code page. Set on both console code pages so that the byte stream
/// `ReadFile`/`WriteFile` carry is the UTF-8 `key.rs` and `term.rs` assume.
pub const CP_UTF8: u32 = 65001;

/// `CONIN$` and `CONOUT$` as NUL-terminated UTF-16, ready for `CreateFileW`.
/// Written out rather than converted at runtime so no allocation happens on the
/// path that runs before the terminal is usable.
pub const CONIN: [u16; 7] = [b'C' as u16, b'O' as u16, b'N' as u16, b'I' as u16, b'N' as u16, b'$' as u16, 0];
pub const CONOUT: [u16; 8] = [
    b'C' as u16, b'O' as u16, b'N' as u16, b'O' as u16, b'U' as u16, b'T' as u16, b'$' as u16, 0,
];

// ---------------------------------------------------------------------------
// Console mode bits
// ---------------------------------------------------------------------------

// Input handle (`SetConsoleMode` on STD_INPUT_HANDLE).
pub const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
pub const ENABLE_LINE_INPUT: u32 = 0x0002;
pub const ENABLE_ECHO_INPUT: u32 = 0x0004;
pub const ENABLE_WINDOW_INPUT: u32 = 0x0008;
pub const ENABLE_MOUSE_INPUT: u32 = 0x0010;
pub const ENABLE_INSERT_MODE: u32 = 0x0020;
pub const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
pub const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
pub const ENABLE_AUTO_POSITION: u32 = 0x0100;
pub const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;

// Output handle (`SetConsoleMode` on STD_OUTPUT_HANDLE).
pub const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
pub const ENABLE_WRAP_AT_EOL_OUTPUT: u32 = 0x0002;
pub const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
pub const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x0008;
pub const ENABLE_LVB_GRID_WORLDWIDE: u32 = 0x0010;

/// The raw-mode transformation for the **input** handle: the Windows analogue
/// of [`crate::sys::abi::apply_raw_mode`].
///
/// Cleared: `ENABLE_LINE_INPUT` (keys must arrive unbuffered), `ENABLE_ECHO_INPUT`
/// (the pager paints its own screen), `ENABLE_PROCESSED_INPUT` (Ctrl-C arrives
/// as `\x03`, matching the `ISIG` clear on unix).
///
/// Set: `ENABLE_VIRTUAL_TERMINAL_INPUT`, which makes the console deliver arrows,
/// Home/End, function keys and bracketed paste as the very ANSI sequences
/// `key.rs` already decodes — the reason `key.rs` needs no Windows branch.
///
/// **Never** set: `ENABLE_MOUSE_INPUT`. **Never** cleared:
/// `ENABLE_QUICK_EDIT_MODE`, which is how console users drag-select
/// (SPEC.md §"Hard constraints" #5). Quick edit is only honoured while
/// `ENABLE_EXTENDED_FLAGS` is set, and the classic bug is to write back a mode
/// with the extended bit set but the quick-edit bit dropped — so when the
/// incoming mode had quick edit on, this re-asserts *both* bits explicitly
/// rather than trusting the round trip.
pub const fn raw_input_mode(cur: u32) -> u32 {
    let cleared = ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT | ENABLE_MOUSE_INPUT;
    let mut m = (cur & !cleared) | ENABLE_VIRTUAL_TERMINAL_INPUT;
    if cur & ENABLE_QUICK_EDIT_MODE != 0 {
        m |= ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS;
    }
    m
}

/// The raw-mode transformation for the **output** handle.
///
/// Sets `ENABLE_VIRTUAL_TERMINAL_PROCESSING` (the frame buffer emits SGR/CUP and
/// the console must interpret them), keeps `ENABLE_PROCESSED_OUTPUT`, sets
/// `DISABLE_NEWLINE_AUTO_RETURN` and clears `ENABLE_WRAP_AT_EOL_OUTPUT` so that
/// painting the last cell of a row does not scroll the screen — together the
/// equivalent of clearing `OPOST` on unix (docs/windows.md §1).
pub const fn raw_output_mode(cur: u32) -> u32 {
    (cur & !ENABLE_WRAP_AT_EOL_OUTPUT)
        | ENABLE_PROCESSED_OUTPUT
        | ENABLE_VIRTUAL_TERMINAL_PROCESSING
        | DISABLE_NEWLINE_AUTO_RETURN
}

// The two product invariants, pinned at compile time on every target so that a
// future edit to the arithmetic above cannot quietly take the mouse away.
const _: () = assert!(raw_input_mode(ENABLE_QUICK_EDIT_MODE) & ENABLE_QUICK_EDIT_MODE != 0);
const _: () = assert!(raw_input_mode(u32::MAX) & ENABLE_MOUSE_INPUT == 0);
const _: () = assert!(raw_input_mode(0) & ENABLE_VIRTUAL_TERMINAL_INPUT != 0);
const _: () = assert!(raw_output_mode(0) & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0);

// ---------------------------------------------------------------------------
// Window size
// ---------------------------------------------------------------------------

/// Visible-window dimensions as `(cols, rows)` from a `SMALL_RECT`.
///
/// This is `srWindow`, **not** `dwSize`: `dwSize` is the screen *buffer*
/// (scrollback), routinely 9000 rows tall, and laying frames out to it would
/// paint most of the pager where the user cannot see it (docs/windows.md §3).
///
/// The rectangle is inclusive on both ends, hence the `+ 1`. A degenerate or
/// inverted rectangle yields `None`, which `term.rs` turns into 80x24 — the
/// same treatment a zero dimension gets on unix.
pub fn window_dims(left: i16, top: i16, right: i16, bottom: i16) -> Option<(u16, u16)> {
    let cols = right as i32 - left as i32 + 1;
    let rows = bottom as i32 - top as i32 + 1;
    if cols <= 0 || rows <= 0 {
        return None;
    }
    Some((cols.min(u16::MAX as i32) as u16, rows.min(u16::MAX as i32) as u16))
}

/// Interpret a `GetConsoleScreenBufferInfo` result plus the rectangle it filled.
pub fn winsize_result(ok: bool, dims: Option<(u16, u16)>) -> Option<(u16, u16)> {
    if ok {
        dims
    } else {
        None
    }
}

/// Pack `(cols, rows)` into one word so the last observed size can live in a
/// single lock-free atomic. `0` is reserved for "nothing observed yet", which is
/// unambiguous because [`window_dims`] never returns a zero dimension.
pub const fn pack_dims(d: Option<(u16, u16)>) -> u32 {
    match d {
        Some((c, r)) => ((c as u32) << 16) | r as u32,
        None => 0,
    }
}

/// Resize detection, as a pure function: Windows has no `SIGWINCH`, so the
/// backend polls `srWindow` and compares. A transition to or from "unknown"
/// (a failed query, e.g. while the console is being torn down) is not a resize;
/// only two known, different sizes are.
pub const fn size_changed(prev: u32, cur: u32) -> bool {
    prev != 0 && cur != 0 && prev != cur
}

// ---------------------------------------------------------------------------
// Waiting and reading
// ---------------------------------------------------------------------------

pub const WAIT_OBJECT_0: u32 = 0x0000_0000;
pub const WAIT_ABANDONED: u32 = 0x0000_0080;
pub const WAIT_TIMEOUT: u32 = 0x0000_0102;
pub const WAIT_FAILED: u32 = 0xFFFF_FFFF;

/// The event-loop tick, in milliseconds. Mirrors `VTIME = 1` (tenths of a
/// second) on unix: `read_input` must come back within roughly this long even
/// when nothing is typed, because that return is when the loop polls
/// `winch_pending()`.
pub const READ_POLL_MS: u32 = 100;

// `winerror.h`.
pub const ERROR_HANDLE_EOF: u32 = 38;
pub const ERROR_BROKEN_PIPE: u32 = 109;
pub const ERROR_NO_DATA: u32 = 232;
pub const ERROR_OPERATION_ABORTED: u32 = 995;
pub const ERROR_INVALID_HANDLE: u32 = 6;

/// What the backend should do with the return value of one `ReadFile`.
///
/// Deliberately *not* [`crate::sys::abi::ReadStep`]: that enum's `Timeout` is
/// the unix "a zero-length read is a `VTIME` expiry, not EOF" rule, and Windows
/// splits the two cases apart — see [`classify_read`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WinRead {
    /// `n` bytes are in the caller's buffer.
    Bytes(usize),
    /// Nothing to report; tick the event loop and wait again. Not EOF.
    Idle,
    /// The input handle is at end of file.
    Eof,
    /// The read was cancelled out from under us; call `ReadFile` again.
    Retry,
    /// Give up with this `GetLastError` code.
    Error(i32),
}

/// Classify one `ReadFile` on the input handle.
///
/// `console` is what makes a successful zero-byte read mean two different
/// things. A **console** never reaches end of file: `ReadConsole` (which is what
/// `ReadFile` becomes on a console handle) can legitimately return zero
/// characters when the records it consumed — a focus change, a menu event, a
/// key-up that raced in after the peek — translate to no bytes at all. Calling
/// that EOF would quit the pager out from under the user for clicking on another
/// window, so it is [`WinRead::Idle`]: go back and wait.
///
/// On a **pipe or file** (`console == false`) there is no such record stream, so
/// a successful zero-byte read is the real thing, and this is what finally makes
/// [`crate::sys::ReadOutcome::Eof`] reachable — as the contract always allowed
/// but the unix backend cannot do.
pub fn classify_read(ok: bool, n: u32, err: u32, console: bool) -> WinRead {
    if ok {
        return match (n, console) {
            (0, true) => WinRead::Idle,
            (0, false) => WinRead::Eof,
            _ => WinRead::Bytes(n as usize),
        };
    }
    match err {
        ERROR_BROKEN_PIPE | ERROR_HANDLE_EOF | ERROR_NO_DATA => WinRead::Eof,
        ERROR_OPERATION_ABORTED => WinRead::Retry,
        e => WinRead::Error(e as i32),
    }
}

/// Classify one `WriteFile`. A successful zero-byte write on a non-empty buffer
/// cannot make progress, so it fails rather than spinning — same rule as unix.
pub fn classify_write(ok: bool, n: u32, err: u32) -> WriteStep {
    if ok {
        return if n == 0 {
            WriteStep::Fail(0)
        } else {
            WriteStep::Advance(n as usize)
        };
    }
    if err == ERROR_OPERATION_ABORTED {
        WriteStep::Retry
    } else {
        WriteStep::Fail(err as i32)
    }
}

// ---------------------------------------------------------------------------
// Input records
// ---------------------------------------------------------------------------

pub const KEY_EVENT: u16 = 0x0001;
pub const MOUSE_EVENT: u16 = 0x0002;
pub const WINDOW_BUFFER_SIZE_EVENT: u16 = 0x0004;
pub const MENU_EVENT: u16 = 0x0008;
pub const FOCUS_EVENT: u16 = 0x0010;

/// Will `ReadFile` produce bytes for this key record under
/// `ENABLE_VIRTUAL_TERMINAL_INPUT`?
///
/// The backend peeks before it reads, because `WaitForSingleObject` signals for
/// *any* input record while `ReadFile` only returns once one of them translates
/// to bytes — pressing and holding Shift would otherwise block the event loop
/// and stall the resize tick. Key-ups never produce bytes, modifier keys never
/// produce bytes, a record carrying a `UnicodeChar` always does, and the
/// navigation keys VT translates are whitelisted rather than assumed: an
/// unrecognised zero-`UnicodeChar` key is dropped, which loses a keystroke `tread`
/// binds nothing to, instead of hanging on one.
pub fn key_record_yields_bytes(down: bool, vk: u16, unicode_char: u16) -> bool {
    if !down {
        return false;
    }
    if is_modifier_vk(vk) {
        return false;
    }
    unicode_char != 0 || is_vt_translated_vk(vk)
}

/// Virtual keys that only ever change the state of another key.
const fn is_modifier_vk(vk: u16) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 // VK_SHIFT, VK_CONTROL, VK_MENU
            | 0x14         // VK_CAPITAL
            | 0x15         // VK_KANA / VK_HANGUL
            | 0x19         // VK_KANJI
            | 0x5B | 0x5C  // VK_LWIN, VK_RWIN
            | 0x90 | 0x91  // VK_NUMLOCK, VK_SCROLL
            | 0xA0..=0xA5  // VK_LSHIFT..VK_RMENU
    )
}

/// Virtual keys the console turns into an escape sequence even though the
/// record carries no `UnicodeChar`.
const fn is_vt_translated_vk(vk: u16) -> bool {
    matches!(
        vk,
        0x21..=0x28   // PRIOR, NEXT, END, HOME, LEFT, UP, RIGHT, DOWN
            | 0x2D | 0x2E // INSERT, DELETE
            | 0x70..=0x7B // F1..F12
    )
}

/// What a `SetConsoleCtrlHandler` callback should do with a control event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CtrlAction {
    /// Ctrl-C / Ctrl-Break: set the interrupt flag, let the loop quit tidily.
    Interrupt,
    /// Close / logoff / shutdown: the process is about to die, so the console
    /// mode must be put back from inside the handler.
    Terminate,
    /// Not ours; decline it.
    Ignore,
}

pub const CTRL_C_EVENT: u32 = 0;
pub const CTRL_BREAK_EVENT: u32 = 1;
pub const CTRL_CLOSE_EVENT: u32 = 2;
pub const CTRL_LOGOFF_EVENT: u32 = 5;
pub const CTRL_SHUTDOWN_EVENT: u32 = 6;

/// Leave the alternate screen, show the cursor, reset SGR — the teardown a
/// console control handler has to do for itself.
///
/// `term.rs` owns every other escape sequence in this crate and the backend
/// emits none. The exception is `CTRL_CLOSE_EVENT` and friends: they kill the
/// process a moment after the handler returns, so the event loop never gets
/// another turn and `term.rs` never runs its teardown. Calling back up into a
/// layer that allocates and takes a mutex from an injected handler thread is not
/// worth the deadlock, so the three sequences are one `const` buffer instead.
/// Leaving a screen that was never entered is a no-op in every terminal, so the
/// handler needs no state to decide.
///
/// It lives here, rather than next to its one caller in `windows.rs`, so the
/// Linux host can assert what is *not* in it: no mouse tracking, ever
/// (SPEC.md §"Hard constraints" #5).
pub const LEAVE_SCREEN: &[u8] = b"\x1b[?1049l\x1b[?25h\x1b[0m";

/// Map a `CTRL_*_EVENT` to the flag it should raise. The three termination
/// events are the Windows counterpart of SIGTERM/SIGHUP/SIGQUIT, which the unix
/// backend catches for exactly the same reason: not to be killed while the
/// terminal is still in raw mode.
pub const fn ctrl_action(event: u32) -> CtrlAction {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => CtrlAction::Interrupt,
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => CtrlAction::Terminate,
        _ => CtrlAction::Ignore,
    }
}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;
