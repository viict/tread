//! `--to-jsonl` and the minifier.
#![deny(unsafe_code)]

use super::*;

fn jsonl(doc: &str) -> Result<String, String> {
    let mut out: Vec<u8> = Vec::new();
    to_jsonl(Reader::memory(doc.as_bytes().to_vec()), &mut out)?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

#[test]
fn a_top_level_array_becomes_one_element_per_line() {
    let doc = r#"[{"a": 1}, "two", 3, null, [4, 5]]"#;
    assert_eq!(
        jsonl(doc).unwrap(),
        "{\"a\":1}\n\"two\"\n3\nnull\n[4,5]\n"
    );
}

#[test]
fn a_pretty_printed_element_becomes_one_line() {
    let doc = "[\n  {\n    \"id\": 1,\n    \"tags\": [\n      \"a\",\n      \"b\"\n    ]\n  }\n]";
    assert_eq!(jsonl(doc).unwrap(), "{\"id\":1,\"tags\":[\"a\",\"b\"]}\n");
}

/// Whitespace inside a string is data, and a newline inside one would split a
/// record in two if it were copied through — it is escaped in the source, so
/// the escape is what is copied.
#[test]
fn whitespace_inside_a_string_survives() {
    let doc = r#"[{"s": "a b\tc\nd"}, "  padded  "]"#;
    assert_eq!(
        jsonl(doc).unwrap(),
        "{\"s\":\"a b\\tc\\nd\"}\n\"  padded  \"\n"
    );
}

/// The export copies bytes; it does not re-encode. A number that no `f64`
/// could hold comes out exactly as it went in.
#[test]
fn numbers_keep_their_source_text() {
    let doc = "[1e999, 0.1, 12345678901234567890123456789012345678901234, -0.0]";
    assert_eq!(
        jsonl(doc).unwrap(),
        "1e999\n0.1\n12345678901234567890123456789012345678901234\n-0.0\n"
    );
}

#[test]
fn an_empty_array_writes_nothing() {
    assert_eq!(jsonl("[]").unwrap(), "");
    assert_eq!(jsonl("[  \n ]").unwrap(), "");
}

#[test]
fn anything_but_an_array_is_refused_with_the_reason() {
    for (doc, want) in [
        (r#"{"a": 1}"#, "is an object, not an array"),
        ("\"hi\"", "is a string, not an array"),
        ("42", "is a number, not an array"),
        ("true", "is a boolean, not an array"),
        ("null", "is null, not an array"),
        ("# a heading", "does not begin with a JSON value"),
    ] {
        let err = jsonl(doc).expect_err("refused");
        assert!(err.contains(want), "{doc}: {err}");
        assert!(err.contains("one array element per line"), "{doc}: {err}");
    }
    assert_eq!(jsonl("   ").expect_err("refused"), "the document is empty");
}

/// An element bigger than the read window is streamed through it rather than
/// held: the whole point of the flag.
#[test]
fn an_element_larger_than_the_read_window_still_writes() {
    let big = "x".repeat(WINDOW * 3);
    let doc = format!("[1, \"{big}\", 2]");
    let out = jsonl(&doc).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1].len(), WINDOW * 3 + 2, "the whole string, quoted");
    assert_eq!(lines[2], "2");
}

/// Many elements, so the array itself spans many windows: every one of them
/// comes out, in order, exactly once.
#[test]
fn a_document_of_many_windows_writes_every_element_once() {
    let n = 100_000;
    let doc = format!("[{}]", (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(", "));
    assert!(doc.len() > WINDOW * 2);
    let out = jsonl(&doc).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), n);
    assert_eq!(lines[0], "0");
    assert_eq!(lines[n - 1], (n - 1).to_string());
}

#[test]
fn a_cut_off_document_writes_what_it_had() {
    let out = jsonl(r#"[1, 2, {"a": 3"#).unwrap();
    assert_eq!(out, "1\n2\n{\"a\":3\n");
}

#[test]
fn the_minifier_leaves_a_value_valid_and_unchanged() {
    assert_eq!(minify(b"{ \"a\" : [ 1 , 2 ] }"), "{\"a\":[1,2]}");
    assert_eq!(minify(b"\"a b\""), "\"a b\"");
    assert_eq!(minify(b"\"a \\\" b\""), "\"a \\\" b\"");
    assert_eq!(minify(b""), "");
    // Round trip: the minified form parses to the same value.
    let src = "{\n \"a\": [1, {\"b\": \"x y\"}],\n \"c\": null\n}";
    let a = crate::json::parse(src.as_bytes()).unwrap();
    let b = crate::json::parse(minify(src.as_bytes()).as_bytes()).unwrap();
    assert_eq!(a, b);
}
