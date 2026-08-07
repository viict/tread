//! Nesting depth: the crash this module exists to prevent.
//!
//! Every one of these builds a document deeper than any recursive parser could
//! survive and requires either a value or an `Error` — never a stack overflow.
//! They cover the walks as well as the parse: what is built here is cloned,
//! compared, serialised and dropped, because an iterative parser behind a
//! recursive walker is still a crash.

use super::*;
use super::tests::{err, ok};

// -- depth ------------------------------------------------------------------

/// `depth` levels of `[`, a `1`, and the matching `]`s.
fn deep_array(depth: usize) -> String {
    let mut s = String::with_capacity(depth * 2 + 1);
    s.push_str(&"[".repeat(depth));
    s.push('1');
    s.push_str(&"]".repeat(depth));
    s
}

/// `depth` levels of `{"k":`, a `1`, and the matching `}`s.
fn deep_object(depth: usize) -> String {
    let mut s = String::new();
    s.push_str(&r#"{"k":"#.repeat(depth));
    s.push('1');
    s.push_str(&"}".repeat(depth));
    s
}

#[test]
fn ten_thousand_levels_of_nesting_parse_rather_than_blowing_the_stack() {
    let v = ok(&deep_array(10_000));
    assert_eq!(v.depth(), 10_001);
    // And dropping it here — at the end of this test — must not overflow
    // either, which is what the hand-written `Drop` is for.
}

#[test]
fn ten_thousand_levels_of_objects_parse() {
    let v = ok(&deep_object(10_000));
    assert_eq!(v.depth(), 10_001);
}

#[test]
fn nesting_past_the_limit_is_refused_with_a_reason_not_a_crash() {
    let e = err(&deep_array(DEFAULT_MAX_DEPTH + 1));
    assert_eq!(e.reason, Reason::TooDeep(DEFAULT_MAX_DEPTH));
    assert_eq!(e.offset, DEFAULT_MAX_DEPTH);
    assert_eq!(e.to_string(), format!("nesting deeper than {DEFAULT_MAX_DEPTH} levels at byte {DEFAULT_MAX_DEPTH}"));
}

#[test]
fn a_hundred_thousand_open_brackets_are_refused_quickly_and_safely() {
    // Unterminated *and* far too deep: the depth limit must win before the
    // input runs out, and neither answer may be a stack overflow.
    let src = "[".repeat(100_000);
    assert_eq!(err(&src).reason, Reason::TooDeep(DEFAULT_MAX_DEPTH));
}

#[test]
fn the_depth_limit_is_configurable_in_both_directions() {
    let p = Parser::new().max_depth(3);
    assert!(p.parse(deep_array(3).as_bytes()).is_ok());
    assert_eq!(p.parse(deep_array(4).as_bytes()).unwrap_err().reason, Reason::TooDeep(3));
    // A scalar document needs no depth at all.
    assert!(Parser::new().max_depth(0).parse(b"1").is_ok());
    assert_eq!(Parser::new().max_depth(0).parse(b"[]").unwrap_err().reason, Reason::TooDeep(0));

    let deeper = Parser::new().max_depth(50_000);
    assert!(deeper.parse(deep_array(20_000).as_bytes()).is_ok());
}

#[test]
fn a_deep_value_can_be_cloned_compared_written_and_dropped() {
    let v = ok(&deep_array(5_000));
    let w = v.clone();
    assert_eq!(v, w);
    assert_eq!(w.to_json(), deep_array(5_000));
    assert_ne!(v, ok(&deep_array(4_999)));
    drop(w);
}

#[test]
fn wide_documents_are_not_a_problem_either() {
    let src = format!("[{}]", vec!["1"; 200_000].join(","));
    assert_eq!(ok(&src).len(), 200_000);
    let pairs: Vec<String> = (0..50_000).map(|i| format!("\"k{i}\":{i}")).collect();
    let src = format!("{{{}}}", pairs.join(","));
    assert_eq!(ok(&src).len(), 50_000);
}
