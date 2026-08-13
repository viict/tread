//! The `usage-atif` dialect over hand-written, synthetic ATIF-shaped steps.
//!
//! Nothing here is copied from a real trajectory; every fixture is written by
//! hand to the shape the schema documents.
#![deny(unsafe_code)]

use super::*;
use crate::render::str_width;

fn read(json: &str) -> Option<Summary> {
    let v = crate::json::parse(json.as_bytes()).expect("fixture parses");
    UsageAtif.read(&v)
}

fn sum(json: &str) -> Summary {
    read(json).expect("the dialect reads this record")
}

/// Three fields, twenty-eight columns, and no fourth.
#[test]
fn a_step_shows_the_three_counters_this_format_has() {
    let s = sum(
        r#"{"step_id":4,"source":"agent","timestamp":"2026-08-05T21:31:00+00:00",
            "metrics":{"prompt_tokens":1200,"completion_tokens":380,"cached_tokens":18000},
            "tool_calls":[{"function_name":"bash","tool_call_id":"c1",
                           "arguments":{"command":"cargo test"}}]}"#,
    );
    assert_eq!(s.what, "in  1.2k  out  380  read 18k  \u{b7}  bash(cargo test)");
    assert_eq!(&s.what[..28], "in  1.2k  out  380  read 18k");
    assert_eq!(str_width(&s.what[..28]), 28);
    assert_eq!(s.class, Class::Step);
    assert_eq!(s.actor, "agent");
    assert_eq!(s.time.as_deref(), Some("21:31"));
    assert_eq!(s.tokens, 1200 + 380 + 18_000);
    assert_eq!(s.calls, 1);
    assert!(s.body.is_none());
}

/// The format has no cache-creation counter, so no row of this file ever shows
/// a `new` column — not a `-`, and above all not a `0`, which would claim the
/// agent wrote nothing to cache when the truth is that ATIF does not record it.
#[test]
fn no_step_ever_shows_a_cache_creation_column() {
    for fixture in [
        r#"{"step_id":1,"source":"agent","metrics":{"prompt_tokens":10}}"#,
        r#"{"step_id":2,"source":"agent","metrics":{"prompt_tokens":0,"completion_tokens":0,"cached_tokens":0}}"#,
        r#"{"step_id":3,"source":"agent","metrics":{"cached_tokens":1823744}}"#,
    ] {
        let s = sum(fixture);
        assert!(!s.what.contains("new"), "{}", s.what);
    }
}

/// A step with no metrics at all: its kind, and nothing more.
#[test]
fn a_step_with_no_metrics_shows_its_kind_and_nothing_more() {
    let s = sum(r#"{"step_id":7,"source":"agent","message":"","tool_calls":[]}"#);
    assert_eq!(s.what, "agent");
    assert_eq!(s.tokens, 0);
    // An empty `metrics` object is the same thing: nothing was recorded.
    let empty = sum(r#"{"step_id":8,"source":"agent","metrics":{}}"#);
    assert_eq!(empty.what, "agent");
}

/// `source` is the discriminator and the turn boundary: a user step breaks the
/// run, an agent step joins it.
#[test]
fn source_decides_whether_a_step_breaks_the_run() {
    assert_eq!(sum(r#"{"step_id":0,"source":"user"}"#).class, Class::Message);
    assert_eq!(sum(r#"{"step_id":1,"source":"agent"}"#).class, Class::Step);
    // An unrecognised source reads as the agent rather than vanishing.
    assert_eq!(sum(r#"{"step_id":2,"source":"tool"}"#).class, Class::Step);
}

/// Record 0 names the agent and carries no numbers: ATIF records no
/// document-level total, and a total invented here would be one the file does
/// not contain.
#[test]
fn the_session_record_names_the_agent_and_totals_nothing() {
    let s = sum(
        r#"{"schema_version":"ATIF-v1.7","session_id":"s-1",
            "agent":{"name":"someagent","model_name":"a-model-id"}}"#,
    );
    assert_eq!(s.class, Class::Message, "the envelope never folds away");
    assert_eq!(s.actor, "session");
    assert_eq!(s.what, "someagent \u{b7} a-model-id \u{b7} ATIF-v1.7 \u{b7} s-1");
    assert_eq!(s.tokens, 0);
    assert_eq!(s.calls, 0);
    for label in ["in ", "out ", "read"] {
        assert!(!s.what.contains(label), "{}", s.what);
    }
}

/// Not an ATIF envelope and not a step: the generic tree, with nothing hidden.
#[test]
fn a_record_that_is_neither_a_step_nor_an_envelope_is_not_read() {
    assert!(read(r#"{"unrelated":true}"#).is_none());
    assert!(read("[1,2]").is_none());
    assert!(read("42").is_none());
}

/// The call count is the calls, and it is what the group row's `· N tool calls`
/// totals. `llm_call_count` is a count of model calls, not of tools, and is not
/// it.
#[test]
fn the_call_count_is_the_tool_calls() {
    let s = sum(
        r#"{"step_id":9,"source":"agent","llm_call_count":11,
            "metrics":{"prompt_tokens":5},
            "tool_calls":[{"function_name":"read","arguments":{"filePath":"a.rs"}},
                          {"function_name":"read","arguments":{"filePath":"a.rs"}},
                          {"function_name":"grep","arguments":{"pattern":"fn"}}]}"#,
    );
    assert_eq!(s.calls, 3);
    assert!(s.what.ends_with("read(a.rs) \u{d7}2 \u{b7} grep(fn)"), "{}", s.what);
}

/// Reasoning is a subset of `completion_tokens`; adding it would count the same
/// token twice. It is not lost — it is on the open level.
#[test]
fn reasoning_is_not_added_to_the_total() {
    let s = sum(
        r#"{"step_id":10,"source":"agent",
            "metrics":{"prompt_tokens":100,"completion_tokens":50,
                       "extra":{"reasoning_tokens":40}}}"#,
    );
    assert_eq!(s.tokens, 150, "not 190");
}
