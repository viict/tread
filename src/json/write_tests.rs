//! Tests for the serialiser: escaping, compactness, and the fact that it walks
//! iteratively.

use super::super::value::{Member, Number, Value};
use super::*;

fn s(v: &Value) -> String {
    to_compact(v)
}

#[test]
fn scalars() {
    assert_eq!(s(&Value::Null), "null");
    assert_eq!(s(&Value::Bool(true)), "true");
    assert_eq!(s(&Value::Bool(false)), "false");
    assert_eq!(s(&Value::string("")), r#""""#);
}

#[test]
fn numbers_are_written_verbatim_never_reformatted() {
    for text in ["0", "-0", "1e999", "1E+2", "0.10", "1.0", "00"] {
        // Even `00`, which the parser would never produce: the writer's job is
        // to write what the tree holds, not to second-guess it.
        assert_eq!(s(&Value::Number(Number::new(text))), text);
    }
}

#[test]
fn containers_are_compact() {
    assert_eq!(s(&Value::Array(vec![])), "[]");
    assert_eq!(s(&Value::Object(vec![])), "{}");
    let v = Value::Array(vec![Value::number("1"), Value::Array(vec![]), Value::Null]);
    assert_eq!(s(&v), "[1,[],null]");
    let o = Value::Object(vec![
        Member::new("a", Value::Object(vec![])),
        Member::new("b", Value::Array(vec![Value::Bool(true)])),
    ]);
    assert_eq!(s(&o), r#"{"a":{},"b":[true]}"#);
}

#[test]
fn duplicate_keys_survive_serialisation() {
    let o = Value::Object(vec![
        Member::new("a", Value::number("1")),
        Member::new("a", Value::number("2")),
    ]);
    assert_eq!(s(&o), r#"{"a":1,"a":2}"#);
}

#[test]
fn quotes_and_backslashes_are_escaped() {
    assert_eq!(escape(r#"a"b"#), r#""a\"b""#);
    assert_eq!(escape(r"a\b"), r#""a\\b""#);
    assert_eq!(escape(r#""\"#), r#""\"\\""#);
}

#[test]
fn control_characters_take_their_short_form_or_a_hex_escape() {
    assert_eq!(escape("\u{8}\u{c}\n\r\t"), r#""\b\f\n\r\t""#);
    assert_eq!(escape("\u{0}"), r#""\u0000""#);
    assert_eq!(escape("\u{1}\u{1f}"), r#""\u0001\u001f""#);
    assert_eq!(escape("\u{b}"), r#""\u000b""#);
}

#[test]
fn everything_else_is_written_literally() {
    // `/` needs no escape, and escaping non-ASCII would change the text a
    // reader is copying without changing what it means.
    assert_eq!(escape("a/b"), r#""a/b""#);
    assert_eq!(escape("é中😀"), "\"é中😀\"");
    assert_eq!(escape("\u{7f}"), "\"\u{7f}\"");
    assert_eq!(escape("\u{fffd}"), "\"\u{fffd}\"");
}

#[test]
fn the_unescaped_fast_path_and_the_escaping_path_agree() {
    // The fast path is a byte scan; make sure it never fires on a string that
    // needs work, and that a multi-byte scalar does not fool it.
    for text in ["plain", "é", "a\"b", "a\nb", "中\u{0}", "😀\\"] {
        let mut slow = String::from("\"");
        for c in text.chars() {
            match c {
                '"' => slow.push_str("\\\""),
                '\\' => slow.push_str("\\\\"),
                '\u{8}' => slow.push_str("\\b"),
                '\u{c}' => slow.push_str("\\f"),
                '\n' => slow.push_str("\\n"),
                '\r' => slow.push_str("\\r"),
                '\t' => slow.push_str("\\t"),
                c if (c as u32) < 0x20 => slow.push_str(&format!("\\u{:04x}", c as u32)),
                c => slow.push(c),
            }
        }
        slow.push('"');
        assert_eq!(escape(text), slow, "{text:?}");
    }
}

#[test]
fn writing_appends_rather_than_replacing() {
    let mut out = String::from("value: ");
    write_compact(&Value::number("1"), &mut out);
    assert_eq!(out, "value: 1");
}

#[test]
fn a_deep_tree_serialises_without_recursion() {
    let mut v = Value::number("1");
    for _ in 0..100_000 {
        v = Value::Array(vec![v]);
    }
    let text = to_compact(&v);
    assert_eq!(text.len(), 100_000 * 2 + 1);
    assert!(text.starts_with("[[[[[") && text.ends_with("]]]]]"));
}

#[test]
fn a_wide_tree_serialises() {
    let items: Vec<Value> = (0..100_000).map(|i| Value::number(i.to_string())).collect();
    let text = to_compact(&Value::Array(items));
    assert!(text.starts_with("[0,1,2,"));
    assert!(text.ends_with(",99998,99999]"));
}
