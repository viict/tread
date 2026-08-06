//! Host tests for [`crate::sys::win_abi`].
//!
//! These run on Linux — that is the entire point. There is no Windows machine
//! in this project's loop, so the console-mode arithmetic, the `srWindow`
//! geometry, the resize comparison and the record classification are all pure
//! functions over plain integers, tested here, and the FFI file is left with
//! nothing but the calls themselves.

use super::*;

// --- console mode ----------------------------------------------------------

/// A plausible default conhost input mode: processed + line + echo + insert +
/// quick edit + extended flags + auto position (what a fresh `cmd.exe` reports).
const DEFAULT_IN: u32 = ENABLE_PROCESSED_INPUT
    | ENABLE_LINE_INPUT
    | ENABLE_ECHO_INPUT
    | ENABLE_INSERT_MODE
    | ENABLE_QUICK_EDIT_MODE
    | ENABLE_EXTENDED_FLAGS
    | ENABLE_AUTO_POSITION;

#[test]
fn raw_input_clears_cooked_mode_bits() {
    let raw = raw_input_mode(DEFAULT_IN);
    assert_eq!(raw & ENABLE_LINE_INPUT, 0);
    assert_eq!(raw & ENABLE_ECHO_INPUT, 0);
    assert_eq!(raw & ENABLE_PROCESSED_INPUT, 0, "Ctrl-C must arrive as a key");
}

#[test]
fn raw_input_sets_virtual_terminal_input() {
    // The single most important flag: it is what lets key.rs stay shared.
    assert_ne!(raw_input_mode(DEFAULT_IN) & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
    assert_ne!(raw_input_mode(0) & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
}

#[test]
fn raw_input_never_enables_the_mouse() {
    // SPEC.md §Hard constraints #5, the Windows half of "never emit ?1000h".
    for cur in [0, DEFAULT_IN, u32::MAX, ENABLE_MOUSE_INPUT] {
        assert_eq!(
            raw_input_mode(cur) & ENABLE_MOUSE_INPUT,
            0,
            "mouse input enabled for mode {cur:#x}"
        );
    }
}

#[test]
fn raw_input_keeps_quick_edit_so_drag_select_survives() {
    let raw = raw_input_mode(DEFAULT_IN);
    assert_ne!(raw & ENABLE_QUICK_EDIT_MODE, 0);
    // The classic bug: extended flags set, quick edit dropped, selection dead.
    assert_ne!(raw & ENABLE_EXTENDED_FLAGS, 0);
}

#[test]
fn raw_input_reasserts_extended_flags_whenever_quick_edit_is_on() {
    // Quick edit is only honoured while ENABLE_EXTENDED_FLAGS is set, so a mode
    // that reports quick edit without it must come back with both.
    let raw = raw_input_mode(ENABLE_QUICK_EDIT_MODE);
    assert_eq!(
        raw & (ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS),
        ENABLE_QUICK_EDIT_MODE | ENABLE_EXTENDED_FLAGS
    );
}

#[test]
fn raw_input_does_not_turn_quick_edit_on_for_a_user_who_had_it_off() {
    let raw = raw_input_mode(ENABLE_EXTENDED_FLAGS | ENABLE_LINE_INPUT);
    assert_eq!(raw & ENABLE_QUICK_EDIT_MODE, 0);
}

#[test]
fn raw_input_preserves_unrelated_bits() {
    let raw = raw_input_mode(DEFAULT_IN);
    assert_ne!(raw & ENABLE_INSERT_MODE, 0);
    assert_ne!(raw & ENABLE_AUTO_POSITION, 0);
}

#[test]
fn raw_input_is_idempotent() {
    let once = raw_input_mode(DEFAULT_IN);
    assert_eq!(raw_input_mode(once), once);
}

#[test]
fn raw_output_enables_vt_and_disables_autowrap_scroll() {
    let cur = ENABLE_PROCESSED_OUTPUT | ENABLE_WRAP_AT_EOL_OUTPUT;
    let raw = raw_output_mode(cur);
    assert_ne!(raw & ENABLE_VIRTUAL_TERMINAL_PROCESSING, 0);
    assert_ne!(raw & DISABLE_NEWLINE_AUTO_RETURN, 0);
    assert_ne!(raw & ENABLE_PROCESSED_OUTPUT, 0);
    assert_eq!(raw & ENABLE_WRAP_AT_EOL_OUTPUT, 0, "last cell must not scroll");
    assert_eq!(raw_output_mode(raw), raw);
}

// --- window geometry -------------------------------------------------------

#[test]
fn window_dims_are_inclusive_of_both_edges() {
    assert_eq!(window_dims(0, 0, 79, 23), Some((80, 24)));
    assert_eq!(window_dims(0, 0, 0, 0), Some((1, 1)));
    // A window scrolled down the buffer: only the extent matters, not the origin.
    assert_eq!(window_dims(0, 900, 119, 949), Some((120, 50)));
}

#[test]
fn window_dims_reject_degenerate_rectangles() {
    // GetConsoleScreenBufferInfo on a console being torn down, and the zeroed
    // struct we hand it when the call fails.
    assert_eq!(window_dims(0, 0, -1, 23), None);
    assert_eq!(window_dims(0, 0, 79, -1), None);
    assert_eq!(window_dims(10, 0, 9, 23), None, "inverted rect");
    assert_eq!(window_dims(0, 10, 79, 9), None, "inverted rect");
}

#[test]
fn window_dims_saturate_instead_of_overflowing_u16() {
    // The widest rectangle SHORT can express is 65536 columns; u16 cannot hold
    // it, and the arithmetic must clamp rather than wrap to 0.
    assert_eq!(window_dims(-32768, -32768, 32767, 32767), Some((65535, 65535)));
}

#[test]
fn winsize_result_needs_a_successful_call() {
    assert_eq!(winsize_result(true, Some((80, 24))), Some((80, 24)));
    assert_eq!(winsize_result(false, Some((80, 24))), None);
    assert_eq!(winsize_result(true, None), None);
}

// --- resize detection ------------------------------------------------------

#[test]
fn pack_dims_round_trips_and_reserves_zero() {
    assert_eq!(pack_dims(None), 0);
    assert_eq!(pack_dims(Some((80, 24))), (80 << 16) | 24);
    assert_ne!(pack_dims(Some((1, 1))), 0);
    assert_ne!(pack_dims(Some((80, 24))), pack_dims(Some((24, 80))));
}

#[test]
fn size_changed_only_fires_between_two_known_sizes() {
    let a = pack_dims(Some((80, 24)));
    let b = pack_dims(Some((100, 24)));
    let c = pack_dims(Some((80, 30)));
    assert!(size_changed(a, b), "width change is a resize");
    assert!(size_changed(a, c), "height change is a resize");
    assert!(!size_changed(a, a), "no change, no resize");
    assert!(!size_changed(0, a), "first observation is not a resize");
    assert!(!size_changed(a, 0), "a failed query is not a resize");
    assert!(!size_changed(0, 0));
}

// --- input records ---------------------------------------------------------

#[test]
fn key_up_and_modifier_keys_yield_no_bytes() {
    assert!(!key_record_yields_bytes(false, b'a' as u16, b'a' as u16));
    for vk in [0x10u16, 0x11, 0x12, 0x14, 0x5B, 0x5C, 0x90, 0x91, 0xA0, 0xA5] {
        assert!(!key_record_yields_bytes(true, vk, 0), "vk {vk:#x} blocked ReadFile");
    }
}

#[test]
fn printable_and_control_characters_yield_bytes() {
    assert!(key_record_yields_bytes(true, b'A' as u16, b'a' as u16));
    assert!(key_record_yields_bytes(true, 0x43, 0x03), "Ctrl-C is a key here");
    assert!(key_record_yields_bytes(true, 0x0D, 0x0D), "Enter");
    assert!(key_record_yields_bytes(true, 0x1B, 0x1B), "Escape");
}

#[test]
fn navigation_keys_yield_bytes_even_with_no_unicode_char() {
    // VT input turns these into escape sequences; the record carries no char.
    for vk in [0x25u16, 0x26, 0x27, 0x28, 0x24, 0x23, 0x21, 0x22, 0x2D, 0x2E, 0x70, 0x7B] {
        assert!(key_record_yields_bytes(true, vk, 0), "vk {vk:#x} would be dropped");
    }
}

// --- read / write classification ------------------------------------------

#[test]
fn a_successful_zero_byte_read_is_eof_only_off_a_console() {
    // A console consumed a record that produced no bytes (focus change, menu,
    // a key-up that raced in behind the peek). Quitting the pager for that would
    // close the reader when the user clicks on another window.
    assert_eq!(classify_read(true, 0, 0, true), WinRead::Idle);
    // A pipe or file really is at its end.
    assert_eq!(classify_read(true, 0, 0, false), WinRead::Eof);
    assert_eq!(classify_read(true, 7, 0, true), WinRead::Bytes(7));
    assert_eq!(classify_read(true, 7, 0, false), WinRead::Bytes(7));
}

#[test]
fn read_errors_map_to_eof_retry_or_failure() {
    for console in [true, false] {
        assert_eq!(classify_read(false, 0, ERROR_BROKEN_PIPE, console), WinRead::Eof);
        assert_eq!(classify_read(false, 0, ERROR_HANDLE_EOF, console), WinRead::Eof);
        assert_eq!(classify_read(false, 0, ERROR_NO_DATA, console), WinRead::Eof);
        assert_eq!(
            classify_read(false, 0, ERROR_OPERATION_ABORTED, console),
            WinRead::Retry
        );
        assert_eq!(
            classify_read(false, 0, ERROR_INVALID_HANDLE, console),
            WinRead::Error(6)
        );
    }
}

#[test]
fn write_classification_matches_the_unix_rules() {
    use crate::sys::abi::WriteStep;
    assert_eq!(classify_write(true, 12, 0), WriteStep::Advance(12));
    assert_eq!(classify_write(true, 0, 0), WriteStep::Fail(0), "no progress");
    assert_eq!(classify_write(false, 0, ERROR_OPERATION_ABORTED), WriteStep::Retry);
    assert_eq!(classify_write(false, 0, ERROR_BROKEN_PIPE), WriteStep::Fail(109));
}

// --- control events --------------------------------------------------------

#[test]
fn ctrl_events_map_to_the_same_flags_as_the_unix_signals() {
    assert_eq!(ctrl_action(CTRL_C_EVENT), CtrlAction::Interrupt);
    assert_eq!(ctrl_action(CTRL_BREAK_EVENT), CtrlAction::Interrupt);
    for e in [CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT] {
        assert_eq!(ctrl_action(e), CtrlAction::Terminate);
    }
    for e in [3, 4, 7, 99] {
        assert_eq!(ctrl_action(e), CtrlAction::Ignore);
    }
}

#[test]
fn the_control_handler_teardown_leaves_alt_and_captures_no_mouse() {
    let s = std::str::from_utf8(LEAVE_SCREEN).unwrap();
    assert!(s.contains("\x1b[?1049l"), "must leave the alternate screen");
    assert!(s.contains("\x1b[?25h"), "must show the cursor");
    assert!(s.ends_with("\x1b[0m"), "must reset SGR last");
    for bad in ["?1000", "?1002", "?1003", "?1006", "?1015", "?1049h"] {
        assert!(!s.contains(bad), "{bad} has no business in a teardown");
    }
}

// --- constants -------------------------------------------------------------

#[test]
fn constants_match_the_windows_headers() {
    assert_eq!(STD_INPUT_HANDLE, 0xFFFF_FFF6);
    assert_eq!(STD_OUTPUT_HANDLE, 0xFFFF_FFF5);
    assert_eq!(STD_ERROR_HANDLE, 0xFFFF_FFF4);
    assert_eq!(CP_UTF8, 65001);
    assert_eq!(WAIT_TIMEOUT, 0x102);
    assert_eq!(READ_POLL_MS, 100, "must match the unix VTIME=1 tick");
    assert_eq!(
        (ENABLE_PROCESSED_INPUT, ENABLE_LINE_INPUT, ENABLE_ECHO_INPUT),
        (1, 2, 4)
    );
    assert_eq!((ENABLE_MOUSE_INPUT, ENABLE_QUICK_EDIT_MODE), (0x10, 0x40));
    assert_eq!(ENABLE_VIRTUAL_TERMINAL_INPUT, 0x200);
    assert_eq!(ENABLE_VIRTUAL_TERMINAL_PROCESSING, 4);
    assert_eq!(DISABLE_NEWLINE_AUTO_RETURN, 8);
    assert_eq!((KEY_EVENT, WINDOW_BUFFER_SIZE_EVENT), (1, 4));
}

#[test]
fn conin_and_conout_are_nul_terminated_utf16() {
    assert_eq!(CONIN.last(), Some(&0));
    assert_eq!(CONOUT.last(), Some(&0));
    let name: String = CONIN[..CONIN.len() - 1]
        .iter()
        .map(|&u| u as u8 as char)
        .collect();
    assert_eq!(name, "CONIN$");
    let name: String = CONOUT[..CONOUT.len() - 1]
        .iter()
        .map(|&u| u as u8 as char)
        .collect();
    assert_eq!(name, "CONOUT$");
}
