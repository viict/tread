//! The agent dialect, against the shapes a real Claude Code session file has.
//!
//! Every fixture here is written by hand. The session log this dialect was
//! derived from is a private transcript: it is read to learn the shape, never
//! copied into the repository.
#![deny(unsafe_code)]

use super::*;

fn read(json: &str) -> Option<Summary> {
    Agent::default().read(&crate::json::parse(json.as_bytes()).expect("fixture parses"))
}

/// A lens instance that keeps its state, for the call/result correlation.
fn session(records: &[&str]) -> Vec<Option<Summary>> {
    let mut lens = Agent::default();
    records
        .iter()
        .map(|r| lens.read(&crate::json::parse(r.as_bytes()).expect("fixture parses")))
        .collect()
}

// -- conversation --------------------------------------------------------------

#[test]
fn a_typed_prompt_is_a_message() {
    let sum = read(
        r#"{"type":"user","timestamp":"2026-08-05T21:28:58.659Z",
            "message":{"role":"user","content":"build me a reader\nplease"}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.class, Class::Message);
    assert_eq!(sum.who, Who::User);
    assert_eq!(sum.actor, "user");
    assert_eq!(sum.time.as_deref(), Some("21:28"));
    assert_eq!(sum.what, "build me a reader please");
    assert_eq!(sum.calls, 0);
}

#[test]
fn assistant_prose_is_a_message() {
    let sum = read(
        r#"{"type":"assistant","timestamp":"2026-08-05T22:02:01.000Z",
            "message":{"role":"assistant","model":"claude-opus-5",
            "content":[{"type":"text","text":"Here is the plan."}]}}"#,
    )
    .expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Message, Who::Assistant));
    assert_eq!(sum.what, "Here is the plan.");
    assert_eq!(sum.time.as_deref(), Some("22:02"));
}

/// A record with text *and* a call stays a message — someone said something —
/// and still reports the call, so the count on a group row cannot be short.
#[test]
fn text_beside_a_call_stays_a_message_and_counts_the_call() {
    let sum = read(
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":"Checking."},
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status"}}]}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.class, Class::Message);
    assert_eq!(sum.what, "Checking. \u{b7} Bash(git status)");
    assert_eq!(sum.calls, 1);
}

/// An empty text block is not speech: the harness writes them, and a row that
/// said nothing would push a real message off the screen.
#[test]
fn an_empty_text_block_is_not_a_message() {
    let sum = read(
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":"   "},
            {"type":"thinking","thinking":"","signature":"x"}]}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.class, Class::Step);
    assert_eq!(sum.what, "thinking");
}

// -- mechanics -----------------------------------------------------------------

#[test]
fn a_tool_call_is_a_step_named_by_its_argument() {
    let sum = read(
        r#"{"type":"assistant","timestamp":"2026-08-05T21:31:00.000Z",
            "message":{"role":"assistant","content":[
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test --all"}}]}}"#,
    )
    .expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Step, Who::Assistant));
    assert_eq!(sum.what, "Bash(cargo test --all)");
    assert_eq!(sum.calls, 1);
}

/// The argument shown is the one a reader scans for, by name — not the first
/// field the object happens to hold.
#[test]
fn each_tool_shows_the_argument_that_says_what_it_did() {
    let cases = [
        (r#""name":"Read","input":{"offset":10,"file_path":"src/main.rs"}"#, "Read(src/main.rs)"),
        (r#""name":"Grep","input":{"pattern":"fn main","n":true}"#, "Grep(fn main)"),
        (r#""name":"WebFetch","input":{"url":"https://x.dev"}"#, "WebFetch(https://x.dev)"),
        (r#""name":"Task","input":{"subagent_type":"explore"}"#, "Task(explore)"),
        (r#""name":"Mystery","input":{"weird":42}"#, "Mystery"),
        (r#""name":"NoInput""#, "NoInput"),
    ];
    for (block, want) in cases {
        let json = format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[
               {{"type":"tool_use","id":"t",{block}}}]}}}}"#
        );
        let sum = read(&json).expect("recognised");
        assert_eq!(sum.what, want, "{block}");
    }
}

/// A result arrives as a `user` record. It is mechanics, it is attributed to
/// the tool, and it is named by the call it answers — which is state the lens
/// carries across records.
#[test]
fn a_result_is_named_after_the_call_it_answers() {
    let out = session(&[
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t1","content":"a\nb\nc","is_error":false}]}}"#,
    ]);
    let result = out[1].clone().expect("recognised");
    assert_eq!((result.class, result.who), (Class::Step, Who::Tool));
    assert_eq!(result.actor, "tool");
    assert_eq!(result.what, "Bash \u{2192} 3 lines");
}

#[test]
fn a_failed_result_says_so() {
    let out = session(&[
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"t9","name":"Edit","input":{"file_path":"a.rs"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t9","content":"boom","is_error":true}]}}"#,
    ]);
    assert_eq!(out[1].clone().expect("recognised").what, "Edit \u{2192} error");
}

/// A result whose call was never seen — a truncated log, a session resumed
/// mid-run — still renders. The lens is stateful, so this is the case that
/// must not turn into a missing row.
#[test]
fn a_result_with_no_remembered_call_still_renders() {
    let sum = read(
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"gone","content":"hello"}]}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.what, "result \u{2192} 5 bytes");
}

#[test]
fn a_result_carrying_blocks_is_measured_too() {
    let sum = read(
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t","content":[
              {"type":"text","text":"one\ntwo"}]}]}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.what, "result \u{2192} 2 lines");
}

#[test]
fn a_subagent_is_marked_on_the_row() {
    let sum = read(
        r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant",
            "content":[{"type":"text","text":"working"}]}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.actor, "\u{21b3} assistant");
    assert_eq!(sum.who, Who::Assistant, "a subagent is still an assistant");
}

// -- the transcript's own bookkeeping --------------------------------------------

#[test]
fn system_records_are_steps_with_their_subtype() {
    let sum = read(
        r#"{"type":"system","subtype":"turn_duration","durationMs":353331,
            "timestamp":"2026-08-05T21:36:14.673Z"}"#,
    )
    .expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Step, Who::System));
    assert_eq!(sum.what, "turn_duration 353.3s");
    assert_eq!(sum.time.as_deref(), Some("21:36"));
}

#[test]
fn bookkeeping_types_are_steps_that_name_themselves() {
    let cases = [
        (r#"{"type":"mode","mode":"normal"}"#, "mode normal"),
        (r#"{"type":"permission-mode","permissionMode":"auto"}"#, "permission-mode auto"),
        (r#"{"type":"ai-title","aiTitle":"Build a reader"}"#, "ai-title Build a reader"),
        (r#"{"type":"file-history-snapshot","messageId":"x"}"#, "file-history-snapshot"),
        (r#"{"type":"file-history-delta","trackingPath":"Cargo.toml"}"#, "file-history-delta Cargo.toml"),
        (r#"{"type":"attachment","attachment":{"type":"deferred_tools_delta"}}"#, "attachment deferred_tools_delta"),
        (r#"{"type":"queue-operation","operation":"enqueue","content":"do it"}"#, "queue-operation enqueue do it"),
    ];
    for (json, want) in cases {
        let sum = read(json).expect("recognised");
        assert_eq!(sum.class, Class::Step, "{json}");
        assert_eq!(sum.what, want, "{json}");
    }
}

// -- the fallback ----------------------------------------------------------------

/// The contract that makes a lens safe: a record it does not know is not
/// summarised, so the source renders it as the generic tree, whole.
#[test]
fn an_unknown_record_is_not_read() {
    for json in [
        r#"{"type":"something-new","payload":{"a":1}}"#,
        r#"{"no_type":true}"#,
        r#"{"type":123}"#,
        r#"[1,2,3]"#,
        r#""just a string""#,
        r#"null"#,
    ] {
        assert!(read(json).is_none(), "{json} should fall back");
    }
}

/// A message record missing the pieces the dialect needs falls back too,
/// rather than showing an empty row where a record was.
#[test]
fn a_malformed_message_falls_back() {
    assert!(read(r#"{"type":"user"}"#).is_none());
    assert!(read(r#"{"type":"assistant","message":{"role":"assistant"}}"#).is_none());
    assert!(read(r#"{"type":"user","message":"not an object"}"#).is_none());
}

/// A content array with nothing the dialect recognises is still a step: the
/// record exists, so it gets a row, and opening it shows what it really holds.
#[test]
fn unrecognised_content_still_gets_a_row() {
    let sum = read(
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"image","source":{"type":"base64"}}]}}"#,
    )
    .expect("recognised");
    assert_eq!(sum.class, Class::Step);
    assert_eq!(sum.what, "(no content)");
}

/// The ring of remembered calls is bounded: a long run must not grow one
/// entry per call for the life of the file.
#[test]
fn remembered_calls_are_bounded() {
    let mut lens = Agent::default();
    for i in 0..RECENT_CALLS * 3 {
        let json = format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[
               {{"type":"tool_use","id":"t{i}","name":"Bash","input":{{"command":"echo {i}"}}}}]}}}}"#
        );
        lens.read(&crate::json::parse(json.as_bytes()).expect("fixture")).expect("read");
    }
    assert_eq!(lens.calls.len(), RECENT_CALLS);
    // The newest call is the one still answerable.
    let last = RECENT_CALLS * 3 - 1;
    assert_eq!(lens.tool_of(&format!("t{last}")), Some("Bash"));
    assert_eq!(lens.tool_of("t0"), None);
}
