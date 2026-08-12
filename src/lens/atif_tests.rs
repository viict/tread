//! The `atif` dialect, against the shapes a real ATIF-v1.7 trajectory has.
//!
//! Every fixture here is written by hand. The trajectory this dialect was
//! derived from is a private file: it is read to learn the shape — which keys
//! exist, which are missing, which are empty — and never copied into the
//! repository.
#![deny(unsafe_code)]

use super::*;

fn read(json: &str) -> Option<Summary> {
    Atif.read(&crate::json::parse(json.as_bytes()).expect("fixture parses"))
}

/// A step with a message, its tool calls, and whatever else it carries.
fn step(id: u32, source: &str, message: &str, rest: &str) -> String {
    format!(
        r#"{{"step_id":{id},"source":"{source}","timestamp":"2026-04-23T10:55:31.123456+00:00",
            "message":"{message}"{}{rest}}}"#,
        if rest.is_empty() { "" } else { "," }
    )
}

// -- the envelope ---------------------------------------------------------------

/// Record 0 is the document's own keys, which is the whole reason they are kept
/// (SPEC.md §Lenses: a lens never hides data).
#[test]
fn the_documents_other_keys_are_a_session_row() {
    let sum = read(
        r#"{"schema_version":"ATIF-v1.7","session_id":"sxs_abc",
            "agent":{"name":"opencode","version":"1.2.3","model_name":"vendor/model"}}"#,
    )
    .expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Message, Who::System));
    assert_eq!(sum.actor, "session");
    assert_eq!(sum.time, None);
    assert_eq!(sum.what, "ATIF-v1.7 \u{b7} opencode 1.2.3 \u{b7} vendor/model \u{b7} sxs_abc");
}

/// A document that is not an ATIF envelope is not summarised at all — it keeps
/// its generic collapsed row rather than being described wrongly.
#[test]
fn a_stranger_at_the_top_level_is_left_to_the_generic_tree() {
    assert!(read(r#"{"title":"notes","items":[1,2]}"#).is_none());
    assert!(read("[1,2,3]").is_none());
    assert!(read("42").is_none());
}

/// Half an envelope is still this dialect's: a trajectory written by a tool
/// that omits `agent` must not fall off the lens.
#[test]
fn a_partial_envelope_still_reads() {
    let sum = read(r#"{"session_id":"sxs_1"}"#).expect("recognised");
    assert_eq!(sum.what, "sxs_1");
    let sum = read(r#"{"schema_version":"ATIF-v1.7","agent":{}}"#).expect("recognised");
    assert_eq!(sum.what, "ATIF-v1.7 \u{b7} agent");
}

// -- conversation ---------------------------------------------------------------

#[test]
fn a_step_that_says_something_is_a_message() {
    let sum = read(&step(2, "user", "build me a reader\\nplease", "")).expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Message, Who::User));
    assert_eq!(sum.actor, "user");
    assert_eq!(sum.time.as_deref(), Some("10:55"));
    assert_eq!(sum.what, "build me a reader please");
    assert_eq!(sum.calls, 0);
}

#[test]
fn an_agent_step_is_the_assistant() {
    let sum = read(&step(3, "agent", "Here is the plan.", "")).expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Message, Who::Assistant));
    assert_eq!(sum.actor, "assistant");
}

/// The settled shape: a message keeps its tool calls as a **count** on its own
/// row. They are not rows and they are not a run — the message is what the
/// reader came for, and the calls are one clause on the end of it.
#[test]
fn a_message_collapses_its_calls_to_a_count() {
    let sum = read(&step(
        4,
        "agent",
        "Checking the tree.",
        r#""reasoning_content":"weighing it up",
           "tool_calls":[
             {"function_name":"bash","arguments":{"command":"git status"},"tool_call_id":"c1"},
             {"function_name":"read","arguments":{"filePath":"/a/b.rs"},"tool_call_id":"c2"}]"#,
    ))
    .expect("recognised");
    assert_eq!(sum.class, Class::Message);
    assert_eq!(sum.what, "Checking the tree. \u{b7} thinking \u{b7} 2 tool calls");
    assert_eq!(sum.calls, 2);
    // One call is singular, and a message with none says nothing about them.
    let one = read(&step(
        5,
        "agent",
        "Looking.",
        r#""tool_calls":[{"function_name":"bash","arguments":{"command":"ls"}}]"#,
    ))
    .expect("recognised");
    assert_eq!(one.what, "Looking. \u{b7} 1 tool call");
}

/// A whitespace-only message is not something anyone said.
#[test]
fn a_blank_message_is_mechanics_not_speech() {
    let sum = read(&step(6, "agent", "   ", r#""reasoning_content":"hm""#)).expect("recognised");
    assert_eq!(sum.class, Class::Step);
    assert_eq!(sum.what, "thinking");
}

// -- mechanics ------------------------------------------------------------------

/// A step with no message is a step: the call, and what came back, matched by
/// `source_call_id` within the step.
#[test]
fn a_call_names_its_argument_and_its_answer() {
    let sum = read(&step(
        7,
        "agent",
        "",
        r#""tool_calls":[{"function_name":"bash","arguments":{"command":"make -j8"},
                          "tool_call_id":"c1"}],
           "observation":{"results":[{"source_call_id":"c1","content":"a\nb\nc"}]}"#,
    ))
    .expect("recognised");
    assert_eq!((sum.class, sum.who), (Class::Step, Who::Tool));
    assert_eq!(sum.actor, "tool");
    assert_eq!(sum.what, "bash(make -j8) \u{2192} 3 lines");
    assert_eq!(sum.calls, 1);
    assert_eq!(sum.time.as_deref(), Some("10:55"));
}

/// The argument is named in order, so a call carrying both `path` and
/// `pattern` shows the pattern — the thing a reader scans for.
#[test]
fn the_argument_shown_is_the_one_that_says_what_happened() {
    let one = |args: &str| {
        read(&step(
            8,
            "agent",
            "",
            &format!(r#""tool_calls":[{{"function_name":"t","arguments":{args}}}]"#),
        ))
        .expect("recognised")
        .what
    };
    assert_eq!(one(r#"{"path":"/x","pattern":"*.rs"}"#), "t(*.rs)");
    assert_eq!(one(r#"{"filePath":"/x/y","limit":40}"#), "t(/x/y)");
    assert_eq!(one(r#"{"url":"https://x/y"}"#), "t(https://x/y)");
    assert_eq!(one(r#"{"query":"who"}"#), "t(who)");
    // A tool whose arguments name none of them keeps its name, and a
    // non-string value is not an argument a row can show.
    assert_eq!(one(r#"{"todos":[{"text":"a"}]}"#), "t()");
    assert_eq!(one(r#"{"command":41}"#), "t()");
    assert_eq!(one("{}"), "t()");
    // The wire format this schema descends from encodes the arguments as a
    // string; both spellings mean the same call.
    assert_eq!(one(r#""{\"command\":\"ls -l\"}""#), "t(ls -l)");
    assert_eq!(one(r#""not json at all""#), "t()");
}

/// Adjacent repeats collapse. A run of the same call is one fact, and spelling
/// it three times pushes what happened next off the row.
#[test]
fn adjacent_identical_entries_collapse_to_a_count() {
    let sum = read(&step(
        9,
        "agent",
        "",
        r#""tool_calls":[
             {"function_name":"bash","arguments":{"command":"pkg-config --exists onig"}},
             {"function_name":"bash","arguments":{"command":"pkg-config --exists onig"}},
             {"function_name":"read","arguments":{"filePath":"/a"}}]"#,
    ))
    .expect("recognised");
    assert_eq!(
        sum.what,
        "bash(pkg-config --exists onig) \u{d7}2 \u{b7} read(/a)"
    );
    assert_eq!(sum.calls, 3);
}

/// Thinking rides in front of the calls it led to, on the same row.
#[test]
fn a_thought_and_the_call_it_led_to_share_a_row() {
    let sum = read(&step(
        10,
        "agent",
        "",
        r#""reasoning_content":"try pkg-config",
           "tool_calls":[{"function_name":"bash","arguments":{"command":"ls"}}]"#,
    ))
    .expect("recognised");
    assert_eq!(sum.what, "thinking \u{b7} bash(ls)");
}

/// An answer nothing asked for, and a call nothing answered: both are visible
/// rather than dropped, because a headline may not lie about what is there.
#[test]
fn an_orphan_result_is_counted_and_an_unanswered_call_is_bare() {
    let sum = read(&step(
        11,
        "agent",
        "",
        r#""tool_calls":[{"function_name":"bash","arguments":{"command":"ls"},
                          "tool_call_id":"c1"}],
           "observation":{"results":[{"source_call_id":"zz","content":"x"}]}"#,
    ))
    .expect("recognised");
    assert_eq!(sum.what, "bash(ls) \u{b7} 1 result");
    // Results and no calls at all.
    let only = read(&step(
        12,
        "agent",
        "",
        r#""observation":{"results":[{"source_call_id":"a","content":"x"},
                                     {"source_call_id":"b","content":"y"}]}"#,
    ))
    .expect("recognised");
    assert_eq!(only.what, "2 results");
    assert_eq!((only.who, only.actor.as_str()), (Who::Assistant, "assistant"));
}

/// What a result says about itself: lines when it has them, a size when it is
/// one long line, and neither invented when there is no content.
#[test]
fn a_result_says_how_much_came_back() {
    let one = |content: &str| {
        read(&step(
            13,
            "agent",
            "",
            &format!(
                r#""tool_calls":[{{"function_name":"t","arguments":{{"command":"c"}},
                                   "tool_call_id":"c1"}}],
                   "observation":{{"results":[{{"source_call_id":"c1","content":{content}}}]}}"#
            ),
        ))
        .expect("recognised")
        .what
    };
    assert_eq!(one(r#""a\nb""#), "t(c) \u{2192} 2 lines");
    assert_eq!(one(r#""hello""#), "t(c) \u{2192} 5 bytes");
    assert_eq!(one(r#""""#), "t(c) \u{2192} empty");
    assert_eq!(one("null"), "t(c) \u{2192} ok");
    assert_eq!(one(r#"{"a":1}"#), "t(c) \u{2192} ok");
}

// -- what a trajectory actually throws at it ------------------------------------

/// Step 1 of the real trajectory carries `message`, `source` and `step_id` and
/// nothing else — no timestamp. The row must not invent one, and this is the
/// first row on the first screen, so getting it wrong is the most visible bug
/// there is.
#[test]
fn the_opening_step_has_no_clock_and_none_is_invented() {
    let sum = read(r#"{"step_id":1,"source":"user","message":"do the thing"}"#).expect("read");
    assert_eq!(sum.time, None);
    assert_eq!((sum.class, sum.who), (Class::Message, Who::User));
    assert_eq!(sum.what, "do the thing");
}

/// Absent, `null` and empty are three spellings of the same thing. The sample
/// only ever omits; nothing in the format forbids the other two.
#[test]
fn null_and_empty_mean_the_same_as_absent() {
    for rest in [
        r#""tool_calls":null,"observation":null,"reasoning_content":null"#,
        r#""tool_calls":[],"observation":{},"reasoning_content":"""#,
        r#""tool_calls":[],"observation":{"results":[]}"#,
    ] {
        let sum = read(&step(14, "agent", "", rest)).expect("recognised");
        assert_eq!(sum.what, "(no content)", "{rest}");
        assert_eq!(sum.calls, 0);
    }
}

/// A timestamp that is not one, or is cut off mid-character, is no clock at all
/// — never a slice of some other field and never a panic.
#[test]
fn a_broken_timestamp_is_no_clock_rather_than_a_wrong_one() {
    for ts in [r#""2026-04-23T10:€z""#, r#""nope""#, r#""""#, "12345", "null"] {
        let json = format!(r#"{{"step_id":1,"source":"agent","message":"x","timestamp":{ts}}}"#);
        assert_eq!(read(&json).expect("recognised").time, None, "{ts}");
    }
}

/// A `source` nobody has seen reads as the assistant rather than dropping the
/// step out of the conversation, and a step that is not an object at all is
/// left to the generic tree.
#[test]
fn an_unknown_source_still_reads_and_a_scalar_step_does_not() {
    let sum = read(r#"{"step_id":9,"source":"orchestrator","message":"hi"}"#).expect("read");
    assert_eq!((sum.who, sum.actor.as_str()), (Who::Assistant, "assistant"));
    // No `step_id`, no envelope keys: not this dialect's, and nothing is
    // hidden — the record keeps its generic row.
    assert!(read(r#"{"note":"stray"}"#).is_none());
}

/// A step big enough to matter is still one line. The excerpt bounds are the
/// shared ones, so a 15KB result and a 2KB message cannot blow a row open.
#[test]
fn a_huge_step_is_still_one_line() {
    let long = "x".repeat(4000);
    let sum = read(&step(
        15,
        "agent",
        &long,
        &format!(r#""tool_calls":[{{"function_name":"bash","arguments":{{"command":"{long}"}}}}]"#),
    ))
    .expect("recognised");
    assert!(crate::render::str_width(&sum.what) <= EXCERPT + ARG + 32, "{}", sum.what);
    assert!(sum.what.contains('\u{2026}'));
}

/// The dialect declares where its records live, and that declaration is the
/// only thing routing needs to know about it.
#[test]
fn the_dialect_says_its_records_are_the_steps_of_a_document() {
    assert_eq!(Atif.records_at(), RecordsAt::Member("steps"));
    assert_eq!(Atif.name(), "atif");
    assert!(super::super::exists("atif"));
    assert!(super::super::list_text().contains("atif"));
}
