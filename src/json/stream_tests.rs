//! Parsing one value at a time: the `.jsonl` and stream path.
//!
//! `parse_prefix` is what a record file and a concatenated stream both run on,
//! and the property that matters for a reader is in
//! `a_jsonl_body_parses_line_by_line...`: a line that is not JSON becomes an
//! error carrying its reason, and the rest of the file still reads.

use super::*;
use super::tests::ok;
use crate::json::{Kind, Value};

// -- prefix parsing (the .jsonl and stream path) ----------------------------

#[test]
fn a_prefix_parse_reports_where_the_value_ended() {
    let (v, end) = parse_prefix(b"{\"a\":1} trailing junk").unwrap();
    assert_eq!(v.to_json(), r#"{"a":1}"#);
    assert_eq!(end, 7);
    let (v, end) = parse_prefix(b"  123  ").unwrap();
    assert_eq!(v, Value::number("123"));
    assert_eq!(end, 5);
}

#[test]
fn a_concatenated_stream_is_read_one_value_at_a_time() {
    let src = b"{\"a\":1}\n[2,3]  \"x\" 4\ntrue";
    let mut at = 0usize;
    let mut out = Vec::new();
    while at < src.len() {
        if src[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        let (v, end) = parse_prefix(&src[at..]).expect("each value parses");
        at += end;
        out.push(v.to_json());
    }
    assert_eq!(out, [r#"{"a":1}"#, "[2,3]", "\"x\"", "4", "true"]);
}

#[test]
fn a_jsonl_body_parses_line_by_line_and_a_bad_line_does_not_stop_the_rest() {
    let body = "{\"i\":0}\n{\"i\":1,}\n{\"i\":2}\nnot json\n{\"i\":3}\n";
    let mut good = Vec::new();
    let mut bad = Vec::new();
    for (n, line) in body.lines().enumerate() {
        match parse(line.as_bytes()) {
            Ok(v) => good.push(v.get("i").unwrap().as_number().unwrap().text().to_string()),
            Err(e) => bad.push(format!("line {}: {e}", n + 1)),
        }
    }
    // The bad lines are skipped and the rest of the file still reads.
    assert_eq!(good, ["0", "2", "3"]);
    assert_eq!(bad, ["line 2: unexpected } at byte 7", "line 4: expected `null` at byte 0"]);
}

#[test]
fn a_prefix_parse_of_an_incomplete_value_still_errors() {
    assert_eq!(parse_prefix(b"{\"a\":").unwrap_err().reason, Reason::Eof);
    assert_eq!(parse_prefix(b"").unwrap_err().reason, Reason::Eof);
    assert_eq!(parse_prefix(b"   ").unwrap_err().reason, Reason::Eof);
}

// -- a document shaped like the real thing ----------------------------------

/// A generated stand-in for an agent trajectory line: the shape tread's own
/// session logs have (nested tool calls, escaped text, unicode), built here
/// rather than copied from a private log.
fn trajectory_line(i: usize) -> String {
    format!(
        r#"{{"type":"assistant","uuid":"u{i}","timestamp":"2026-08-05T12:0{i}:00Z","message":{{"role":"assistant","content":[{{"type":"text","text":"line one\nline two \"quoted\" \u00e9 \ud83d\ude00"}},{{"type":"tool_use","id":"t{i}","name":"Read","input":{{"file_path":"/a/b.rs","limit":null,"offset":{i}}}}}]}},"costUSD":0.0123456789,"nested":{{"a":{{"b":{{"c":{{"d":{{"e":{{"f":[1,2,3]}}}}}}}}}}}}}}"#
    )
}

#[test]
fn a_generated_trajectory_line_parses_and_round_trips() {
    let src = trajectory_line(7);
    let v = ok(&src);
    assert_eq!(v.get("type").unwrap().as_str(), Some("assistant"));
    assert_eq!(v.depth(), 9);
    let content = v.get("message").unwrap().get("content").unwrap();
    assert_eq!(content.len(), 2);
    let text = content.index(0).unwrap().get("text").unwrap().as_str().unwrap();
    assert!(text.contains("😀") && text.contains('\n') && text.contains('"'));
    assert_eq!(v.get("costUSD").unwrap().as_number().unwrap().text(), "0.0123456789");
    // Re-parsing the serialisation gives the same value.
    assert_eq!(ok(&v.to_json()), v);
}

#[test]
fn a_generated_trajectory_file_parses_line_by_line() {
    let body: String = (0..500).map(|i| trajectory_line(i % 10) + "\n").collect();
    let mut n = 0;
    for line in body.lines() {
        let v = parse(line.as_bytes()).expect("every line parses");
        assert_eq!(v.kind(), Kind::Object);
        n += 1;
    }
    assert_eq!(n, 500);
}
