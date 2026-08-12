//! The `atif` dialect's **open level**: what [`Atif::detail`] says a step's
//! parts are (SPEC.md §Lenses).
//!
//! Split from `atif_tests.rs` for the reason `agent_parts.rs` is split from
//! `agent.rs`: two halves of one dialect, each under the size limit. Every
//! fixture is written by hand — the trajectory this dialect was derived from is
//! private, and is read to learn the shape, never copied.
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

fn detail(json: &str) -> Vec<Part> {
    Atif.detail(&crate::json::parse(json.as_bytes()).expect("fixture parses"))
}

/// A step that only thought puts the thought **under its row**, the way a
/// message puts what was said there. Its row keeps saying what it did.
#[test]
fn a_step_that_only_thought_carries_the_thought_as_its_body() {
    let sum = read(&step(
        7,
        "agent",
        "",
        r#""reasoning_content":"the fixture was renamed\nso the path is what fails""#,
    ))
    .expect("recognised");
    assert_eq!(sum.class, Class::Step, "still mechanics");
    assert!(sum.what.starts_with("thinking"), "{:?}", sum.what);
    let body = sum.body.expect("a step's reasoning is under its row");
    assert_eq!(body.lines, 2);
    assert!(body.head.starts_with("the fixture was renamed"), "{:?}", body.head);
    assert_eq!(body.at, vec![Step::Key("reasoning_content")], "the way back to the whole of it");
}

/// And it is not *also* a part, or the reader would be shown it twice.
#[test]
fn the_thought_under_a_step_is_not_repeated_as_a_part() {
    let json = step(7, "agent", "", r#""reasoning_content":"one thought""#);
    assert!(
        !detail(&json).iter().any(|p| matches!(p, Part::Text { .. })),
        "the body already says it"
    );
}

/// On a step that *said* something the body is the message, so the thought has
/// nowhere else to go and becomes a part of the open level.
#[test]
fn a_message_that_also_thought_keeps_the_thought_as_a_part() {
    let json = step(7, "agent", "here is what I found", r#""reasoning_content":"a thought""#);
    let sum = read(&json).expect("recognised");
    assert_eq!(sum.class, Class::Message);
    assert!(sum.body.expect("the message").head.starts_with("here is what"));
    match detail(&json).first().expect("a part") {
        Part::Text { label, body } => {
            assert_eq!(*label, "thinking");
            assert_eq!(body.head, "a thought");
        }
        other => panic!("{other:?}"),
    }
}

/// An empty or whitespace-only `reasoning_content` is not a body and not a
/// part: absent, `null` and `""` all mean the same thing here.
#[test]
fn an_empty_thought_is_no_body_at_all() {
    for raw in [r#""reasoning_content":"""#, r#""reasoning_content":"  ""#, r#""reasoning_content":null"#, ""] {
        let json = step(7, "agent", "", raw);
        assert!(read(&json).expect("recognised").body.is_none(), "{raw}");
        assert!(!detail(&json).iter().any(|p| matches!(p, Part::Text { .. })), "{raw}");
    }
}

// -- the parts of a step ---------------------------------------------------------

/// A call is a part: what was called, with what, and what came back — the
/// result matched to its call by `source_call_id`, within the step.
#[test]
fn a_call_becomes_a_part_with_its_arguments_and_its_result() {
    let json = step(
        7,
        "agent",
        "",
        r#""tool_calls":[{"tool_call_id":"c1","function_name":"bash",
              "arguments":{"command":"cargo test","timeout":120}}],
           "observation":{"results":[{"source_call_id":"c1","content":"a\nb\nc"}]}"#,
    );
    match detail(&json).first().expect("a part") {
        Part::Call { tool, arg, args, result } => {
            assert_eq!(tool, "bash");
            assert_eq!(arg, "cargo test", "the one argument the row names");
            let names: Vec<&str> = args.iter().map(|(k, _)| k.as_str()).collect();
            assert_eq!(names, vec!["command", "timeout"], "every argument, in the record's order");
            // A number is its own JSON, which is what the tree would show.
            assert_eq!(args[1].1.head, "120");
            let result = result.as_ref().expect("the answer it was given");
            assert_eq!(result.lines, 3);
            assert_eq!(
                result.at,
                vec![
                    Step::Key("observation"),
                    Step::Key("results"),
                    Step::At(0),
                    Step::Key("content")
                ],
                "a result is one node of this record, so opening it reads the whole of it"
            );
        }
        other => panic!("{other:?}"),
    }
}

/// An argument's value has no path — its key is its own name and a `Step::Key`
/// is static — so it is a bounded head that still states its true size. Never
/// silent (SPEC.md §Lenses).
#[test]
fn an_argument_is_a_bounded_head_that_knows_how_big_it_really_is() {
    let long = "x".repeat(4000);
    let json = step(
        7,
        "agent",
        "",
        &format!(r#""tool_calls":[{{"function_name":"bash","arguments":{{"command":"{long}"}}}}]"#),
    );
    match detail(&json).first().expect("a part") {
        Part::Call { args, .. } => {
            let body = &args[0].1;
            assert_eq!(body.bytes, 4000, "it knows what it is short of");
            assert_eq!(body.head.len(), crate::lens::BODY_KEEP);
            assert!(!body.whole(), "so the row under it says what it left out");
            assert!(body.at.is_empty(), "and it is honest about having no way back");
        }
        other => panic!("{other:?}"),
    }
}

/// `arguments` as the JSON-encoded string the wire format emits: the same
/// parts, read out of the decoded object.
#[test]
fn arguments_written_as_a_string_are_still_arguments() {
    let json = step(
        7,
        "agent",
        "",
        r#""tool_calls":[{"function_name":"bash","arguments":"{\"command\":\"make\"}"}]"#,
    );
    match detail(&json).first().expect("a part") {
        Part::Call { tool, args, .. } => {
            assert_eq!(tool, "bash");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0].0, "command");
            assert_eq!(args[0].1.head, "make");
        }
        other => panic!("{other:?}"),
    }
}

/// `arguments` that is not an object at all — an array, or a JSON-encoded
/// string streaming cut in half — keeps what the file said under one row.
///
/// Dropping it made the open level say the call had **no** arguments, which is
/// the one thing a clip may never do (SPEC.md §Lenses): the command existed and
/// nothing on the screen admitted to leaving it out.
#[test]
fn arguments_that_are_not_an_object_are_shown_rather_than_dropped() {
    let one = |args: &str| {
        let json = step(
            7,
            "agent",
            "",
            &format!(r#""tool_calls":[{{"tool_call_id":"c1","function_name":"bash","arguments":{args}}}]"#),
        );
        match detail(&json).into_iter().next().expect("a part") {
            Part::Call { args, .. } => args,
            other => panic!("{other:?}"),
        }
    };
    // A JSON-encoded string that does not parse is still what was sent.
    let cut = one(r#""cargo test --all-features -q""#);
    assert_eq!(cut.len(), 1, "{cut:?}");
    assert_eq!(cut[0].0, crate::lens::part::RAW_ARGS);
    assert_eq!(cut[0].1.head, "cargo test --all-features -q");
    // An array is its own JSON, which is what the record's tree shows.
    let list = one(r#"["cargo","test"]"#);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].1.head, r#"["cargo","test"]"#);
    // And the genuinely empty shapes stay genuinely empty, so the row that
    // carries them advertises no fold.
    for empty in ["{}", "null", "[]"] {
        assert!(one(empty).is_empty(), "{empty}");
    }
}

/// A call with no arguments and no answer has nothing under it, and says so —
/// `Part::opens` is what the row's fold marker is painted from.
#[test]
fn a_call_with_nothing_under_it_does_not_claim_to_open() {
    let bare = step(7, "agent", "", r#""tool_calls":[{"function_name":"ls","arguments":{}}]"#);
    assert!(!detail(&bare).first().expect("a part").opens(), "nothing under it");
    let armed = step(
        7,
        "agent",
        "",
        r#""tool_calls":[{"function_name":"ls","arguments":{"path":"/"}}]"#,
    );
    assert!(detail(&armed).first().expect("a part").opens(), "an argument to show");
}

/// A result whose `content` is not a string is measured the same way on both
/// rows that state its size: the shut row's `→ …` and the call row's.
#[test]
fn a_structured_result_states_one_size_on_both_rows() {
    let json = step(
        7,
        "agent",
        "",
        r#""tool_calls":[{"tool_call_id":"c1","function_name":"read","arguments":{"filePath":"a.rs"}}],
           "observation":{"results":[{"source_call_id":"c1","content":{"blocks":["one"]}}]}"#,
    );
    let body = match detail(&json).into_iter().next().expect("a part") {
        Part::Call { result, .. } => result.expect("the answer"),
        other => panic!("{other:?}"),
    };
    let what = read(&json).expect("recognised").what;
    assert_eq!(what, format!("read(a.rs) \u{2192} {} bytes", body.bytes), "{what:?}");
    assert_eq!(body.head, r#"{"blocks":["one"]}"#);
}

/// A result no call claimed gets a part of its own rather than a count: at this
/// level there is room to show it, and every byte stays reachable.
#[test]
fn an_orphan_result_is_a_part_rather_than_a_number() {
    let json = step(
        7,
        "agent",
        "",
        r#""tool_calls":[{"tool_call_id":"c1","function_name":"bash","arguments":{"command":"ls"}}],
           "observation":{"results":[{"source_call_id":"nobody","content":"orphaned"}]}"#,
    );
    let parts = detail(&json);
    assert_eq!(parts.len(), 2, "the call, and the answer nothing asked for: {parts:?}");
    match &parts[1] {
        Part::Call { tool, result, .. } => {
            assert_eq!(tool, "result");
            assert_eq!(result.as_ref().expect("the content").head, "orphaned");
        }
        other => panic!("{other:?}"),
    }
    // And the row still counts it, as it always did.
    assert!(read(&json).expect("recognised").what.contains("1 result"));
}

/// The envelope is not a step: `Enter` on record 0 opens the envelope, and
/// there is nothing between its row and that.
#[test]
fn the_session_row_has_no_parts() {
    assert!(detail(r#"{"schema_version":"ATIF-v1.7","session_id":"s"}"#).is_empty());
}

/// A step with no calls and no thought has no parts, which is what gives it one
/// rung fewer on the ladder rather than a keystroke that does nothing.
#[test]
fn a_step_with_nothing_in_it_has_no_parts() {
    assert!(detail(&step(7, "user", "just words", "")).is_empty());
}
