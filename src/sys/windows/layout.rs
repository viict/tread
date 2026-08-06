//! The C struct layouts the Windows backend passes to the console API — on
//! every target, not just Windows.
//!
//! Nothing here is `unsafe` and nothing here calls the OS. Like
//! [`crate::sys::layout`], these types are declared unconditionally so that
//! their sizes and alignments are `const`-asserted by the Linux host build and
//! their field decoding is exercised by `cargo test` on Linux: a wrong
//! `#[repr(C)]` compiles cleanly and then corrupts memory on a machine this
//! project cannot run.
//!
//! Authority: Windows SDK `wincontypes.h` (`COORD`, `SMALL_RECT`,
//! `INPUT_RECORD`, `KEY_EVENT_RECORD`, `WINDOW_BUFFER_SIZE_RECORD`) and
//! `wincon.h` (`CONSOLE_SCREEN_BUFFER_INFO`). All of these are pointer-free, so
//! the layouts are identical on `i686`, `x86_64` and `aarch64` Windows and
//! identical between the MSVC and GNU ABIs.
//!
//! ```text
//! typedef struct _COORD { SHORT X; SHORT Y; } COORD;                    // 4
//! typedef struct _SMALL_RECT { SHORT Left, Top, Right, Bottom; };       // 8
//! typedef struct _CONSOLE_SCREEN_BUFFER_INFO {
//!     COORD dwSize; COORD dwCursorPosition; WORD wAttributes;
//!     SMALL_RECT srWindow; COORD dwMaximumWindowSize; };                // 22
//! typedef struct _KEY_EVENT_RECORD {
//!     BOOL bKeyDown; WORD wRepeatCount; WORD wVirtualKeyCode;
//!     WORD wVirtualScanCode; union { WCHAR; CHAR; } uChar;
//!     DWORD dwControlKeyState; };                                       // 16
//! typedef struct _INPUT_RECORD { WORD EventType; union { … } Event; };  // 20
//! ```
#![deny(unsafe_code)]
#![allow(dead_code)]

use super::win_abi::{window_dims, KEY_EVENT, WINDOW_BUFFER_SIZE_EVENT};

/// `COORD`: two `SHORT`s, column then row.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Coord {
    pub x: i16,
    pub y: i16,
}

/// `SMALL_RECT`: an **inclusive** rectangle of `SHORT`s.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SmallRect {
    pub left: i16,
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
}

/// `CONSOLE_SCREEN_BUFFER_INFO`, filled by `GetConsoleScreenBufferInfo`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ScreenBufferInfo {
    /// The **scrollback buffer** size. Never used for layout — see
    /// [`ScreenBufferInfo::window_size`].
    pub dw_size: Coord,
    pub dw_cursor_position: Coord,
    pub w_attributes: u16,
    /// The visible window, in buffer coordinates.
    pub sr_window: SmallRect,
    pub dw_maximum_window_size: Coord,
}

impl ScreenBufferInfo {
    pub const fn zeroed() -> Self {
        ScreenBufferInfo {
            dw_size: Coord { x: 0, y: 0 },
            dw_cursor_position: Coord { x: 0, y: 0 },
            w_attributes: 0,
            sr_window: SmallRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dw_maximum_window_size: Coord { x: 0, y: 0 },
        }
    }

    /// `(cols, rows)` of the *visible window*, from `srWindow`.
    ///
    /// `dwSize` is ignored on purpose: it describes the scrollback buffer, which
    /// is typically hundreds of rows taller than the window (docs/windows.md §3).
    pub fn window_size(&self) -> Option<(u16, u16)> {
        let r = self.sr_window;
        window_dims(r.left, r.top, r.right, r.bottom)
    }
}

/// The decoded payload of a `KEY_EVENT` record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key_down: bool,
    pub repeat_count: u16,
    pub virtual_key_code: u16,
    pub virtual_scan_code: u16,
    /// The `uChar.UnicodeChar` member — a UTF-16 code unit, `0` for keys that
    /// have no character (arrows, function keys, bare modifiers).
    pub unicode_char: u16,
    pub control_key_state: u32,
}

/// `INPUT_RECORD`. The `Event` union is held as raw bytes and decoded by the
/// safe accessors below, so the whole record parse is ordinary `u16::from_le_bytes`
/// arithmetic that runs — and is tested — on the Linux host. Windows is
/// little-endian on every supported architecture, which is what makes that
/// legitimate.
#[repr(C, align(4))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InputRecord {
    pub event_type: u16,
    _pad: u16,
    event: [u8; 16],
}

impl InputRecord {
    pub const fn zeroed() -> Self {
        InputRecord {
            event_type: 0,
            _pad: 0,
            event: [0; 16],
        }
    }

    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.event[off], self.event[off + 1]])
    }

    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([
            self.event[off],
            self.event[off + 1],
            self.event[off + 2],
            self.event[off + 3],
        ])
    }

    /// The `KEY_EVENT_RECORD` payload, or `None` for any other record type.
    pub fn key_event(&self) -> Option<KeyEvent> {
        if self.event_type != KEY_EVENT {
            return None;
        }
        Some(KeyEvent {
            key_down: self.u32_at(0) != 0,
            repeat_count: self.u16_at(4),
            virtual_key_code: self.u16_at(6),
            virtual_scan_code: self.u16_at(8),
            unicode_char: self.u16_at(10),
            control_key_state: self.u32_at(12),
        })
    }

    /// True for `WINDOW_BUFFER_SIZE_EVENT`, the console's resize notification —
    /// there is no `SIGWINCH` here.
    pub fn is_resize(&self) -> bool {
        self.event_type == WINDOW_BUFFER_SIZE_EVENT
    }

    /// Test constructor for a key record; also the documentation of the
    /// `KEY_EVENT_RECORD` field offsets.
    pub fn key(down: bool, vk: u16, unicode_char: u16) -> Self {
        let mut r = InputRecord::zeroed();
        r.event_type = KEY_EVENT;
        r.event[0] = down as u8;
        r.event[4..6].copy_from_slice(&1u16.to_le_bytes());
        r.event[6..8].copy_from_slice(&vk.to_le_bytes());
        r.event[10..12].copy_from_slice(&unicode_char.to_le_bytes());
        r
    }

    /// Test constructor for a resize record.
    pub fn resize() -> Self {
        let mut r = InputRecord::zeroed();
        r.event_type = WINDOW_BUFFER_SIZE_EVENT;
        r
    }
}

// ---------------------------------------------------------------------------
// Compile-time ABI assertions — enforced on every target, Linux host included.
// ---------------------------------------------------------------------------

use core::mem::{align_of, size_of};

const _: () = assert!(size_of::<Coord>() == 4 && align_of::<Coord>() == 2);
const _: () = assert!(size_of::<SmallRect>() == 8 && align_of::<SmallRect>() == 2);
// 4 + 4 + 2 + 8 + 4 = 22, with no padding anywhere: every member is 2-aligned.
const _: () = assert!(size_of::<ScreenBufferInfo>() == 22);
const _: () = assert!(align_of::<ScreenBufferInfo>() == 2);
// WORD EventType, 2 bytes of padding to the union's 4-byte alignment, then the
// largest arm (KEY_EVENT_RECORD and MOUSE_EVENT_RECORD are both 16 bytes).
const _: () = assert!(size_of::<InputRecord>() == 20);
const _: () = assert!(align_of::<InputRecord>() == 4);

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
