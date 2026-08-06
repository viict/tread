//! Host tests for [`crate::sys::win_layout`].
//!
//! The sizes are already pinned by `const _: () = assert!(…)` at the bottom of
//! `layout.rs`, so a wrong one is a build error on Linux too. What these add is
//! the *field* story: offsets, decoding, and the `srWindow`-not-`dwSize` rule.

use super::*;

#[test]
fn struct_sizes_match_the_windows_headers() {
    assert_eq!(size_of::<Coord>(), 4);
    assert_eq!(size_of::<SmallRect>(), 8);
    assert_eq!(size_of::<ScreenBufferInfo>(), 22);
    assert_eq!(size_of::<InputRecord>(), 20);
    assert_eq!(align_of::<InputRecord>(), 4);
}

#[test]
fn screen_buffer_info_field_offsets_are_the_c_ones() {
    let z = ScreenBufferInfo::zeroed();
    let base = &z as *const _ as usize;
    let off = |p: *const u8| p as usize - base;
    assert_eq!(off(&z.dw_size as *const _ as *const u8), 0);
    assert_eq!(off(&z.dw_cursor_position as *const _ as *const u8), 4);
    assert_eq!(off(&z.w_attributes as *const _ as *const u8), 8);
    assert_eq!(off(&z.sr_window as *const _ as *const u8), 10);
    assert_eq!(off(&z.dw_maximum_window_size as *const _ as *const u8), 18);
}

#[test]
fn window_size_uses_srwindow_and_ignores_the_scrollback_buffer() {
    let mut csbi = ScreenBufferInfo::zeroed();
    // A typical conhost: 120x9001 buffer, 120x30 window scrolled to the bottom.
    csbi.dw_size = Coord { x: 120, y: 9001 };
    csbi.sr_window = SmallRect {
        left: 0,
        top: 8971,
        right: 119,
        bottom: 9000,
    };
    assert_eq!(csbi.window_size(), Some((120, 30)));
    assert_ne!(csbi.window_size(), Some((120, 9001)), "dwSize is not the window");
}

#[test]
fn a_zeroed_screen_buffer_info_reports_one_by_one_not_a_panic() {
    // GetConsoleScreenBufferInfo failing leaves our zeroed struct untouched;
    // the caller gates on the return value, but the arithmetic must still be
    // total. An all-zero inclusive rect legitimately means 1x1.
    assert_eq!(ScreenBufferInfo::zeroed().window_size(), Some((1, 1)));
}

#[test]
fn key_records_decode_their_fields() {
    let r = InputRecord::key(true, 0x25, 0);
    let k = r.key_event().expect("KEY_EVENT");
    assert!(k.key_down);
    assert_eq!(k.repeat_count, 1);
    assert_eq!(k.virtual_key_code, 0x25);
    assert_eq!(k.unicode_char, 0);
    assert!(!r.is_resize());

    let r = InputRecord::key(false, b'A' as u16, b'a' as u16);
    let k = r.key_event().expect("KEY_EVENT");
    assert!(!k.key_down);
    assert_eq!(k.unicode_char, b'a' as u16);
}

#[test]
fn non_key_records_decode_as_none() {
    let r = InputRecord::resize();
    assert!(r.key_event().is_none());
    assert!(r.is_resize());

    let mut mouse = InputRecord::zeroed();
    mouse.event_type = crate::sys::win_abi::MOUSE_EVENT;
    assert!(mouse.key_event().is_none());
    assert!(!mouse.is_resize());
}

#[test]
fn key_event_record_byte_offsets_match_the_c_struct() {
    // bKeyDown 0..4, wRepeatCount 4..6, wVirtualKeyCode 6..8,
    // wVirtualScanCode 8..10, uChar 10..12, dwControlKeyState 12..16.
    let mut r = InputRecord::zeroed();
    r.event_type = crate::sys::win_abi::KEY_EVENT;
    r.event[0..4].copy_from_slice(&1u32.to_le_bytes());
    r.event[4..6].copy_from_slice(&3u16.to_le_bytes());
    r.event[6..8].copy_from_slice(&0x1234u16.to_le_bytes());
    r.event[8..10].copy_from_slice(&0x5678u16.to_le_bytes());
    r.event[10..12].copy_from_slice(&0x00E9u16.to_le_bytes());
    r.event[12..16].copy_from_slice(&0x0000_0008u32.to_le_bytes());
    let k = r.key_event().unwrap();
    assert_eq!(
        (
            k.key_down,
            k.repeat_count,
            k.virtual_key_code,
            k.virtual_scan_code,
            k.unicode_char,
            k.control_key_state
        ),
        (true, 3, 0x1234, 0x5678, 0x00E9, 8)
    );
}
