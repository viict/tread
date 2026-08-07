//! The shared grammar's own tests. Everything here is a fact both JSON sources
//! inherit, which is the point of the module.
#![deny(unsafe_code)]

use super::*;
use crate::json::parse;

fn text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

fn v(src: &str) -> Value {
    parse(src.as_bytes()).expect("valid json")
}

#[test]
fn a_collapsed_container_counts_itself_and_an_empty_one_says_so() {
    assert_eq!(summary_text(Shape::Object, 5, true), "{\u{2026}5 keys}");
    assert_eq!(summary_text(Shape::Object, 1, true), "{\u{2026}1 key}");
    assert_eq!(summary_text(Shape::Array, 120, true), "[\u{2026}120 items]");
    assert_eq!(summary_text(Shape::Array, 1, true), "[\u{2026}1 item]");
    // "0 keys" is a number where a shape would do.
    assert_eq!(summary_text(Shape::Object, 0, true), "{}");
    assert_eq!(summary_text(Shape::Array, 0, true), "[]");
    // A count still being walked says so rather than showing a number that
    // will change under the reader.
    assert_eq!(summary_text(Shape::Array, 7, false), "[\u{2026}\u{2265}7 items]");
    assert_eq!(summary_text(Shape::Object, 0, false), "{\u{2026}\u{2265}0 keys}");
}

#[test]
fn a_row_is_indent_then_gutter_then_label_then_value() {
    let val = v("\"ada\"");
    let row = spans(2, Mark::Leaf, Some("name"), Body::Scalar(&val));
    assert_eq!(text(&row), "      \"name\": \"ada\"");
    let open = spans(1, Mark::Open, Some("users"), Body::Bracket(Shape::Array));
    assert_eq!(text(&open), "  \u{25be} \"users\": [");
    let shut = spans(1, Mark::Closed, Some("users"), Body::Summary(Shape::Array, 3, true));
    assert_eq!(text(&shut), "  \u{25b8} \"users\": [\u{2026}3 items]");
    let close = spans(1, Mark::Leaf, None, Body::Close(Shape::Array));
    assert_eq!(text(&close), "    ]");
}

/// A container's two brackets share a column and its members sit one level in
/// — the gutter is what pays for it, and the same arithmetic has to hold on
/// both sources or a subtree looks nested one level deeper in one of them.
#[test]
fn a_containers_brackets_share_a_column_and_its_members_indent() {
    let head = text(&spans(0, Mark::Open, None, Body::Bracket(Shape::Object)));
    let member = text(&spans(1, Mark::Leaf, Some("a"), Body::Scalar(&v("1"))));
    let close = text(&spans(0, Mark::Leaf, None, Body::Close(Shape::Object)));
    let col = |row: &str, c: char| row.chars().position(|x| x == c);
    assert_eq!(col(&head, '{'), Some(2));
    assert_eq!(col(&member, '"'), Some(4));
    assert_eq!(col(&close, '}'), Some(2));
}

/// Indent stops growing well past any terminal, so ten thousand levels of
/// `[[[[` cannot make the *rows* quadratic in the nesting depth.
#[test]
fn indent_stops_where_a_terminal_would() {
    let deep = text(&spans(100_000, Mark::Leaf, None, Body::Scalar(&Value::Null)));
    assert_eq!(deep.len(), MAX_INDENT * INDENT + 2 + 4);
    let capped = text(&spans(MAX_INDENT, Mark::Leaf, None, Body::Scalar(&Value::Null)));
    assert_eq!(deep, capped);
}

#[test]
fn a_scalar_keeps_its_source_text_and_its_quotes() {
    assert_eq!(text(&scalar_spans(&v("1e999"))), "1e999");
    assert_eq!(text(&scalar_spans(&v("0.10"))), "0.10");
    let big = "1".repeat(40);
    assert_eq!(text(&scalar_spans(&v(&big))), big);
    assert_eq!(text(&scalar_spans(&v("\"1\""))), "\"1\"");
    assert_eq!(text(&scalar_spans(&v("1"))), "1");
    assert_eq!(text(&scalar_spans(&v("true"))), "true");
    assert_eq!(text(&scalar_spans(&v("null"))), "null");
}

#[test]
fn a_control_character_in_a_string_cannot_tear_the_frame() {
    let val = Value::string("a\u{1b}[31mb\nc");
    let out = text(&scalar_spans(&val));
    assert!(!out.contains('\u{1b}'), "{out:?}");
    assert!(!out.contains('\n'), "{out:?}");
    let key = text(&spans(0, Mark::Leaf, Some("k\u{7}ey"), Body::Scalar(&Value::Null)));
    assert!(!key.contains('\u{7}'), "{key:?}");
}

/// A row is never wrapped, so one pathological member must not become a
/// megabyte of spans either.
#[test]
fn a_scalar_far_too_wide_for_any_terminal_is_cut() {
    let val = Value::number("1".repeat(MAX_VALUE * 3));
    let out = text(&scalar_spans(&val));
    assert!(str_width(&out) <= MAX_VALUE + 2, "{}", str_width(&out));
    assert!(out.ends_with('\u{2026}'), "{}", &out[out.len() - 8..]);
}

#[test]
fn a_path_step_is_a_dot_a_quoted_key_or_an_index() {
    assert_eq!(path_step(Some("name"), 0), ".name");
    assert_eq!(path_step(Some("a_b-c9"), 0), ".a_b-c9");
    assert_eq!(path_step(Some("odd key"), 0), "[\"odd key\"]");
    assert_eq!(path_step(Some(""), 0), "[\"\"]");
    assert_eq!(path_step(Some("9lives"), 0), "[\"9lives\"]");
    assert_eq!(path_step(None, 3), "[3]");
}

#[test]
fn a_shape_survives_the_round_trip_through_a_value() {
    assert_eq!(shape_of(&v("{}")), Shape::Object);
    assert_eq!(shape_of(&v("[]")), Shape::Array);
    assert_eq!(shape_of(&v("\"s\"")), Shape::Str);
    assert_eq!(shape_of(&v("1")), Shape::Number);
    assert_eq!(shape_of(&v("false")), Shape::Bool);
    assert_eq!(shape_of(&v("null")), Shape::Null);
    // The same answer the index gets from the first byte alone.
    for src in ["{}", "[]", "\"s\"", "1", "false", "null"] {
        assert_eq!(shape_of(&v(src)), Shape::of(src.as_bytes()[0]), "{src}");
    }
}

#[test]
fn a_size_reads_as_a_person_would_say_it() {
    assert_eq!(size(512), "512 bytes");
    assert_eq!(size(1536), "1.5 KB");
    assert_eq!(size(3 << 20), "3.0 MB");
    assert_eq!(size(2 << 30), "2.0 GB");
    assert!(oversize(5 << 20, 1 << 20).contains("5.0 MB"));
    assert!(oversize(5 << 20, 1 << 20).contains("1.0 MB"));
}

/// The marker is the fold affordance and the leaf gutter is the same width, so
/// values line up whether or not their siblings can be opened.
#[test]
fn every_gutter_is_two_columns() {
    for m in [Mark::Open, Mark::Closed, Mark::Leaf] {
        let row = text(&spans(0, m, None, Body::Scalar(&Value::Null)));
        let gutter: String = row.chars().take(2).collect();
        assert_eq!(str_width(&gutter), 2, "{m:?}");
        assert_eq!(row, format!("{gutter}null"), "{m:?}");
    }
}

/// The fold-id vocabulary, which both sources now speak: a `/`-separated path
/// of member indices from the root. A record file's root is the implicit list
/// of records, so a record id and a root member's id are the same string.
#[test]
fn a_fold_id_is_a_member_index_path_from_the_root() {
    assert_eq!(child_id("", 4), "/4");
    assert_eq!(child_id("/0", 3), "/0/3");
    assert_eq!(child_id("/0/3", 7), "/0/3/7");
    assert_eq!(top_index("/4"), Some(4));
    assert_eq!(top_index(&child_id("", 41)), Some(41));
    // Not a top-level id, and not an id at all.
    assert_eq!(top_index("/0/3"), None);
    assert_eq!(top_index(ALL_OPEN), None);
    assert_eq!(top_index("nonsense"), None);
    assert_eq!(top_index(""), None);
    // The root is the empty string, so it can never be mistaken for a member.
    assert_ne!(child_id("", 0), "");
}

/// What is displayed must re-parse to the value being displayed. Showing the
/// decoded text in quotes does not: `"has \"q\""` becomes `"has "q""`, which is
/// not valid JSON and not what the file says.
#[test]
fn a_string_is_shown_as_the_literal_the_file_holds() {
    for src in [
        r#""has \"quotes\" in it""#,
        r#""back\\slash""#,
        r#""tab\there""#,
        r#""new\nline""#,
        r#""plain""#,
    ] {
        let v = crate::json::parse_str(src).expect("parses");
        let shown = text(&scalar_spans(&v));
        assert_eq!(shown, src, "shown text must be the literal");
        // And it round-trips: what is on screen parses back to the same value.
        assert_eq!(crate::json::parse_str(&shown).expect("re-parses"), v);
    }
}

/// A cut string is still a literal — an unbalanced quote reads as a bug.
#[test]
fn a_cut_string_keeps_its_closing_quote() {
    let v = Value::string("x".repeat(MAX_VALUE * 3));
    let out = text(&scalar_spans(&v));
    assert!(out.starts_with('"'), "{}", &out[..8]);
    assert!(out.ends_with("\u{2026}\""), "{}", &out[out.len() - 8..]);
    assert!(str_width(&out) <= MAX_VALUE + 2, "{}", str_width(&out));
}
