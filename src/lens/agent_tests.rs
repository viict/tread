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

/// A typed prompt and a text block both put what was said under the row, and
/// the path names where it came from so the whole of it can be read back.
#[test]
fn a_message_carries_its_text_as_a_body() {
    use crate::lens::Step;
    let sum = read(
        r#"{"type":"user","message":{"role":"user","content":"one\ntwo\nthree"}}"#,
    )
    .expect("recognised");
    let body = sum.body.expect("a body");
    assert_eq!(body.head, "one\ntwo\nthree");
    assert_eq!(body.lines, 3);
    assert_eq!(body.at, vec![Step::Key("message"), Step::Key("content")]);

    // A block array: the body is the first text block, wherever it sits.
    let json = r#"{"type":"assistant","message":{"role":"assistant","content":[
        {"type":"thinking","thinking":"hm"},
        {"type":"text","text":"Here is the plan."}]}}"#;
    let sum = read(json).expect("recognised");
    let body = sum.body.expect("a body");
    assert_eq!(body.head, "Here is the plan.");
    assert_eq!(
        body.at,
        vec![Step::Key("message"), Step::Key("content"), Step::At(1), Step::Key("text")]
    );
    let record = crate::json::parse(json.as_bytes()).expect("fixture");
    assert_eq!(body.text_in(Some(&record)), "Here is the plan.");
}

/// Mechanics stay one line, which is also what keeps a folded run exactly as
/// tall as the records it holds.
#[test]
fn a_step_never_carries_a_body() {
    let mechanics = [
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
        r#"{"type":"system","subtype":"turn_duration","durationMs":1200}"#,
        r#"{"type":"mode","mode":"plan"}"#,
    ];
    for json in mechanics {
        let sum = read(json).expect("recognised");
        assert_eq!(sum.class, Class::Step, "{json}");
        assert!(sum.body.is_none(), "{json}");
    }
}

// -- the open level, and what this dialect can and cannot say about it -----------

fn detail(json: &str) -> Vec<Part> {
    Agent::default().detail(&crate::json::parse(json.as_bytes()).expect("fixture parses"))
}

/// The seam is not an ATIF special case: a Claude Code record's blocks are the
/// same two things — text, and calls to tools.
#[test]
fn a_call_block_becomes_a_part_with_every_argument_it_was_given() {
    let json = r#"{"type":"assistant","message":{"role":"assistant","content":[
        {"type":"tool_use","id":"t1","name":"Bash",
         "input":{"command":"cargo test","description":"run the suite","timeout":120}}]}}"#;
    match detail(json).first().expect("a part") {
        Part::Call { tool, arg, args, result } => {
            assert_eq!(tool, "Bash");
            assert_eq!(arg, "cargo test");
            let names: Vec<&str> = args.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(names, vec!["command", "description", "timeout"]);
            assert_eq!(args[2].1.head, "120");
            assert!(
                result.is_none(),
                "the answer is a later record, and a path starts at the record it is in"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// The other half of that pair: the result record carries the answer, and names
/// the call when the ring still remembers it.
#[test]
fn a_result_record_carries_the_answer_and_names_its_call_while_it_can() {
    let mut lens = Agent::default();
    let call = r#"{"type":"assistant","message":{"role":"assistant","content":[
        {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#;
    let result = r#"{"type":"user","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"t1","content":"a\nb\nc"}]}}"#;
    lens.read(&crate::json::parse(call.as_bytes()).expect("fixture"));
    let v = crate::json::parse(result.as_bytes()).expect("fixture");
    match lens.detail(&v).first().expect("a part") {
        Part::Call { tool, result, .. } => {
            assert_eq!(tool, "Bash", "named after the call it answers");
            let body = result.as_ref().expect("the output");
            assert_eq!(body.lines, 3);
            assert_eq!(
                body.at,
                vec![
                    Step::Key("message"),
                    Step::Key("content"),
                    Step::At(0),
                    Step::Key("content")
                ],
                "a string result is one node of this record"
            );
        }
        other => panic!("{other:?}"),
    }
    // A lens that has forgotten the call still shows the answer, and says
    // `result` rather than guessing a tool.
    match detail(result).first().expect("a part") {
        Part::Call { tool, result, .. } => {
            assert_eq!(tool, "result");
            assert!(result.is_some(), "the output is never lost with the name");
        }
        other => panic!("{other:?}"),
    }
}

/// The block-array form of a result is several nodes joined, and no path
/// addresses a join — so it is a bounded head that states its true size. Honest
/// rather than whole, and never silent.
#[test]
fn a_block_array_result_is_a_head_that_knows_its_size() {
    let json = r#"{"type":"user","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"t9","content":[
            {"type":"text","text":"one"},{"type":"text","text":"two"}]}]}}"#;
    match detail(json).first().expect("a part") {
        Part::Call { result, .. } => {
            let body = result.as_ref().expect("the output");
            assert_eq!(body.head, "one\ntwo");
            assert!(body.at.is_empty(), "there is no single node to point at");
            assert!(body.whole(), "and this one fits, so nothing is claimed to be missing");
        }
        other => panic!("{other:?}"),
    }
}

/// **Every** block of that array, not only the ones with a `text` key.
///
/// Keeping the text alone made the join — and so the `bytes` and `lines` the
/// call row states — the size of what survived rather than of what came back:
/// the clip then had nothing to report and an image beside the text was gone,
/// unannounced. That is the silent case SPEC.md §Lenses forbids.
#[test]
fn a_block_that_is_not_text_is_still_in_the_result() {
    let json = r#"{"type":"user","message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"t1","content":[
            {"type":"text","text":"line one"},
            {"type":"image","source":{"type":"base64","data":"AAAABBBBCCCC"}},
            {"type":"text","text":"line two"}]}]}}"#;
    match detail(json).first().expect("a part") {
        Part::Call { result, .. } => {
            let body = result.as_ref().expect("the output");
            assert_eq!(body.lines, 3, "three blocks, three lines: {:?}", body.head);
            assert!(body.head.contains("AAAABBBBCCCC"), "the image is there: {:?}", body.head);
            assert_eq!(body.bytes, body.head.len(), "and the size is of all of it");
        }
        other => panic!("{other:?}"),
    }
}

/// An `input` that is not an object keeps what it said, exactly as `atif`'s
/// `arguments` does — one list-reading, so the two dialects cannot disagree.
#[test]
fn an_input_that_is_not_an_object_is_shown_rather_than_dropped() {
    let json = r#"{"type":"assistant","message":{"role":"assistant","content":[
        {"type":"tool_use","id":"t1","name":"Bash","input":["cargo","test"]}]}}"#;
    match detail(json).first().expect("a part") {
        Part::Call { args, .. } => {
            assert_eq!(args.len(), 1, "{args:?}");
            assert_eq!(args[0].0, crate::lens::part::RAW_ARGS);
            assert_eq!(args[0].1.head, r#"["cargo","test"]"#);
        }
        other => panic!("{other:?}"),
    }
}

/// Thinking is text. A record whose only content is a thought puts it under the
/// row, exactly as a message goes under a message's row.
#[test]
fn a_thinking_only_record_shows_what_it_was_thinking() {
    let json = r#"{"type":"assistant","message":{"role":"assistant","content":[
        {"type":"thinking","thinking":"the fixture moved\nso the path is what fails","signature":"x"}]}}"#;
    let sum = read(json).expect("recognised");
    assert_eq!(sum.class, Class::Step);
    assert_eq!(sum.what, "thinking");
    let body = sum.body.expect("the thought is under the row");
    assert_eq!(body.lines, 2);
    assert!(!detail(json).iter().any(|p| matches!(p, Part::Text { .. })), "not twice");
}

/// Beside a message the thought has nowhere else to go, and becomes a part.
/// So does a second text block, which the row could only ever excerpt.
#[test]
fn text_a_row_could_not_hold_becomes_a_part() {
    let json = r#"{"type":"assistant","message":{"role":"assistant","content":[
        {"type":"text","text":"first"},
        {"type":"thinking","thinking":"a thought","signature":"x"},
        {"type":"text","text":"second"}]}}"#;
    let sum = read(json).expect("recognised");
    assert_eq!(sum.body.expect("the first text block").head, "first");
    let parts = detail(json);
    let labels: Vec<&str> = parts
        .iter()
        .filter_map(|p| match p {
            Part::Text { label, .. } => Some(*label),
            _ => None,
        })
        .collect();
    assert_eq!(labels, vec!["thinking", "message"], "in the record's own order");
    match &parts[1] {
        Part::Text { body, .. } => assert_eq!(body.head, "second"),
        other => panic!("{other:?}"),
    }
}

/// A record with no blocks at all — a typed prompt, a bookkeeping row — has no
/// parts, and the ladder there is one rung shorter.
#[test]
fn a_record_with_no_blocks_has_no_parts() {
    assert!(detail(r#"{"type":"user","message":{"role":"user","content":"just typing"}}"#).is_empty());
    assert!(detail(r#"{"type":"mode","mode":"plan"}"#).is_empty());
}
