//! The group row's text: what a folded run says it swallowed.
//!
//! One of the two places a total appears (the other is the status bar in
//! `view.rs`), and both are here in the seam rather than in a dialect, because a
//! lens may not decide a row.
#![deny(unsafe_code)]

use super::*;

/// A run that recorded no tokens reads exactly as it always has. This pins that
/// adding the clause moved no shipped row: `agent` and `atif` both leave
/// `Summary::tokens` at 0, so every one of their group rows is byte-identical.
#[test]
fn a_run_that_counted_nothing_says_nothing_about_tokens() {
    assert_eq!(group_text(6, 4, 0), "\u{27e8}6 steps \u{b7} 4 tool calls\u{27e9}");
    assert_eq!(group_text(1, 0, 0), "\u{27e8}1 step\u{27e9}");
    assert_eq!(group_text(1, 1, 0), "\u{27e8}1 step \u{b7} 1 tool call\u{27e9}");
    assert_eq!(group_text(3, 0, 0), "\u{27e8}3 steps\u{27e9}");
}

/// The third clause, with the singular and plural the other two already have.
#[test]
fn a_run_that_counted_tokens_totals_them() {
    assert_eq!(
        group_text(15, 3, 128_000),
        "\u{27e8}15 steps \u{b7} 3 tool calls \u{b7} 128k tokens\u{27e9}"
    );
    assert_eq!(group_text(1, 0, 1), "\u{27e8}1 step \u{b7} 1 token\u{27e9}");
    assert_eq!(group_text(2, 0, 380), "\u{27e8}2 steps \u{b7} 380 tokens\u{27e9}");
    // Spelled by the one spelling there is, so the group row and a record row
    // cannot disagree about what `1.9k` means.
    assert_eq!(group_text(2, 0, 1_999), "\u{27e8}2 steps \u{b7} 1.9k tokens\u{27e9}");
}
