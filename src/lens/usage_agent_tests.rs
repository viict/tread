//! The `usage` dialect over hand-written, synthetic session records.
//!
//! Nothing here is copied from a real session log; every fixture is written by
//! hand to the shape the format documents.
#![deny(unsafe_code)]

use super::*;
use crate::render::str_width;

/// Columns the row's actor field is padded to (`lensrow::ACTOR`). Named here
/// because a subagent mark that outgrew it would shift every number on the row,
/// and this dialect's whole product is that the numbers line up.
const ACTOR: usize = 10;

fn read(json: &str) -> Option<Summary> {
    let v = crate::json::parse(json.as_bytes()).expect("fixture parses");
    Usage.read(&v)
}

fn sum(json: &str) -> Summary {
    read(json).expect("the dialect reads this record")
}

/// An assistant record with all four counters: the block, aligned, and a total
/// that is the exact sum.
#[test]
fn an_assistant_record_shows_the_four_counters_and_totals_them() {
    let s = sum(
        r#"{"type":"assistant","timestamp":"2026-08-05T21:31:00.000Z",
            "message":{"role":"assistant","content":[
              {"type":"tool_use","id":"a","name":"Bash","input":{"command":"cargo test"}}],
             "usage":{"input_tokens":1200,"output_tokens":380,
                      "cache_read_input_tokens":18000,
                      "cache_creation_input_tokens":2100}}}"#,
    );
    assert_eq!(s.what, "in  1.2k  out  380  read 18k  new 2.1k  \u{b7}  Bash(cargo test)");
    assert_eq!(s.class, Class::Step, "only a human turn is conversation");
    assert_eq!(s.actor, "assistant");
    assert_eq!(s.time.as_deref(), Some("21:31"));
    assert_eq!(s.tokens, 1200 + 380 + 18_000 + 2_100);
    assert_eq!(s.calls, 1);
    assert!(s.body.is_none(), "this lens shows no message text at all");
}

/// A turn that really did spend zero prints zeroes, not dashes.
#[test]
fn a_recorded_zero_is_still_a_row_of_numbers() {
    let s = sum(
        r#"{"type":"assistant","message":{"role":"assistant","content":[],
            "usage":{"input_tokens":0,"output_tokens":0}}}"#,
    );
    assert!(s.what.starts_with("in     0  out    0"), "{}", s.what);
    assert_eq!(s.tokens, 0);
    // The two counters this record did not write are dashes, not zeroes.
    assert!(s.what.contains("read   -"), "{}", s.what);
    assert!(s.what.contains("new    -"), "{}", s.what);
}

/// No usage at all: the record's kind, verbatim, and no number columns anywhere.
#[test]
fn a_record_with_no_usage_shows_its_kind_and_nothing_more() {
    for kind in ["file-history-snapshot", "queue-operation", "bridge-session", "agent-color"] {
        let s = sum(&format!(r#"{{"type":"{kind}","messageId":"x"}}"#));
        assert_eq!(s.what, kind, "the kind verbatim");
        assert_eq!(s.tokens, 0);
        for label in ["in", "out", "read", "new"] {
            assert!(!s.what.contains(&format!("{label} ")), "{kind} shows a number column");
        }
        assert_eq!(s.actor, "system", "and the actor field still fits");
    }
}

/// A type this dialect has never seen prints its own name rather than being
/// swallowed — the seam's rule that a lens never hides anything.
#[test]
fn an_unknown_type_prints_its_own_name() {
    let s = sum(r#"{"type":"some-future-thing"}"#);
    assert_eq!(s.what, "some-future-thing");
}

/// The load-bearing decision: a human turn breaks the run, and a tool result
/// does not. If the second were a `Message` too, every run would shred into
/// pairs and no group row would total a turn.
#[test]
fn only_a_human_turn_is_conversation() {
    let typed = sum(r#"{"type":"user","message":{"role":"user","content":"run the tests"}}"#);
    assert_eq!(typed.class, Class::Message);
    assert_eq!(typed.actor, "user");
    assert_eq!(typed.what, "user", "no numbers on a turn that recorded none");

    let result = sum(
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"a","content":"ok"}]}}"#,
    );
    assert_eq!(result.class, Class::Step, "a tool result is mechanics");

    // A record with blocks but no result is still a person typing.
    let blocks = sum(
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"text","text":"and again"}]}}"#,
    );
    assert_eq!(blocks.class, Class::Message);

    // Everything else is mechanics whatever it holds.
    assert_eq!(sum(r#"{"type":"system","subtype":"turn_duration"}"#).class, Class::Step);
}

/// A run is consecutive steps, so this is the item list the plan would build
/// from a whole turn: one message, its mechanics, then the next message.
#[test]
fn a_turn_is_one_message_then_a_run_of_mechanics() {
    let records = [
        r#"{"type":"user","message":{"role":"user","content":"go"}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[],
            "usage":{"input_tokens":10,"output_tokens":5}}}"#,
        r#"{"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"a","content":"ok"}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[],
            "usage":{"input_tokens":20,"output_tokens":1}}}"#,
        r#"{"type":"user","message":{"role":"user","content":"thanks"}}"#,
    ];
    let classes: Vec<Class> = records.iter().map(|r| sum(r).class).collect();
    assert_eq!(
        classes,
        vec![Class::Message, Class::Step, Class::Step, Class::Step, Class::Message],
        "one run of three between two turns, not three runs of one"
    );
}

/// The subagent mark costs no alignment: `↳assistant` is exactly the ten
/// columns the actor field is wide, and `↳ assistant` (which `agent` uses)
/// would be eleven. More than half the records in a real session carry the flag.
#[test]
fn a_subagent_is_marked_without_costing_a_column() {
    let s = sum(
        r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","content":[],
            "usage":{"input_tokens":1,"output_tokens":1}}}"#,
    );
    assert_eq!(s.actor, "\u{21b3}assistant");
    assert_eq!(str_width(&s.actor), ACTOR, "the mark must not push the numbers right");
    assert_eq!(s.who, Who::Assistant, "and it still paints as an assistant");

    let side_user = sum(r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"x"}}"#);
    assert_eq!(side_user.actor, "\u{21b3}user");
    assert!(str_width(&side_user.actor) <= ACTOR);
    assert!(str_width(&sum(r#"{"type":"assistant"}"#).actor) <= ACTOR);
}

/// `iterations` is a list whose elements repeat the outer counter names.
/// Summing both double-counts every token on the records that carry it, which is
/// the mistake this pins against.
#[test]
fn iterations_are_never_added_to_the_total() {
    let s = sum(
        r#"{"type":"assistant","message":{"role":"assistant","content":[],
            "usage":{"input_tokens":100,"output_tokens":50,
                     "iterations":[{"input_tokens":60,"output_tokens":30},
                                   {"input_tokens":40,"output_tokens":20}]}}}"#,
    );
    assert_eq!(s.tokens, 150, "the outer four once, the iterations not at all");
    assert!(s.what.starts_with("in   100  out   50"), "{}", s.what);
}

/// Several calls of one kind read as one move with a count.
#[test]
fn several_calls_of_one_kind_collapse() {
    let s = sum(
        r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"a","name":"Read","input":{"file_path":"a.rs"}},
            {"type":"tool_use","id":"b","name":"Read","input":{"file_path":"a.rs"}},
            {"type":"tool_use","id":"c","name":"Read","input":{"file_path":"a.rs"}}],
            "usage":{"input_tokens":1}}}"#,
    );
    assert!(s.what.ends_with("Read(a.rs) \u{d7}3"), "{}", s.what);
    assert_eq!(s.calls, 3);
}

/// Not this dialect's record: it falls back to the generic tree rather than
/// being summarised wrongly or hidden.
#[test]
fn a_record_that_is_not_an_object_is_not_read() {
    assert!(read("[1,2,3]").is_none());
    assert!(read(r#""just a string""#).is_none());
    assert!(read(r#"{"no":"type"}"#).is_none());
    assert!(read(r#"{"type":7}"#).is_none(), "a non-string type is not a type");
}

/// A `usage` object that is present but says none of the four is a record with
/// no numbers, and reads as its kind rather than as four dashes.
#[test]
fn a_usage_object_with_no_counters_is_not_a_row_of_dashes() {
    let s = sum(r#"{"type":"assistant","message":{"role":"assistant","usage":{"service_tier":"standard"}}}"#);
    assert_eq!(s.what, "assistant");
    assert_eq!(s.tokens, 0);
}

/// A counter that is not a non-negative integer is refused rather than clamped:
/// the cell then says `-`, which is true.
#[test]
fn a_counter_that_is_not_a_count_is_refused() {
    let s = sum(
        r#"{"type":"assistant","message":{"role":"assistant","content":[],
            "usage":{"input_tokens":-5,"output_tokens":"lots","cache_read_input_tokens":7}}}"#,
    );
    assert_eq!(s.tokens, 7);
    assert!(s.what.starts_with("in     -  out    -  read   7"), "{}", s.what);
}

// -- the open level --------------------------------------------------------------

/// Every part's label and text, for asking what the open level says.
fn detail_text(json: &str) -> Vec<(&'static str, String)> {
    let v = crate::json::parse(json.as_bytes()).expect("fixture parses");
    Usage
        .detail(&v)
        .into_iter()
        .map(|p| match p {
            Part::Text { label, body } => (label, body.head),
            Part::Call { tool, .. } => panic!("this lens builds no call parts, got {tool}"),
        })
        .collect()
}

fn part_of<'a>(parts: &'a [(&'static str, String)], label: &str) -> &'a str {
    &parts.iter().find(|(l, _)| *l == label).unwrap_or_else(|| panic!("no {label} part")).1
}

/// The row floors to four columns; the level under it is the integer the file
/// wrote. That is the whole bargain of flooring.
#[test]
fn the_open_level_is_where_the_numbers_are_exact() {
    let record = r#"{"type":"assistant","message":{"role":"assistant","content":[],
        "model":"a-model-id","usage":{"input_tokens":1999,"output_tokens":7,
        "cache_creation":{"ephemeral_5m_input_tokens":11,"ephemeral_1h_input_tokens":2},
        "output_tokens_details":{"thinking_tokens":3}}}}"#;
    assert!(sum(record).what.starts_with("in  1.9k"), "the row floors");
    let parts = detail_text(record);
    let tokens = part_of(&parts, "tokens");
    for want in [
        "input_tokens                1999",
        "output_tokens               7",
        "ephemeral_5m_input_tokens   11",
        "ephemeral_1h_input_tokens   2",
        "thinking_tokens             3",
    ] {
        assert!(tokens.contains(want), "{want:?} not in {tokens:?}");
    }
    assert!(part_of(&parts, "model").contains("a-model-id"));
}

/// `standard` is what nearly every record says, so saying it again is noise;
/// anything else is the anomaly a reader opened the row to find.
#[test]
fn a_tier_is_shown_only_when_it_is_not_standard() {
    let with = |tier: &str| {
        format!(
            r#"{{"type":"assistant","message":{{"role":"assistant",
               "usage":{{"input_tokens":1,"service_tier":{tier}}}}}}}"#
        )
    };
    let standard = detail_text(&with(r#""standard""#));
    assert!(!standard.iter().any(|(l, _)| *l == "request"), "{standard:?}");
    assert!(part_of(&detail_text(&with(r#""priority""#)), "request").contains("priority"));
    // A tier that is null is not a string and says nothing either.
    assert!(!detail_text(&with("null")).iter().any(|(l, _)| *l == "request"));
}

/// One attempt is the ordinary case and says nothing; two is a retry, and the
/// *length* is the fact. The elements are never summed into the total.
#[test]
fn iterations_appear_only_when_there_was_more_than_one() {
    let one = r#"{"type":"assistant","message":{"role":"assistant",
        "usage":{"input_tokens":1,"iterations":[{"input_tokens":1}]}}}"#;
    assert!(!detail_text(one).iter().any(|(l, _)| *l == "request"));
    let two = r#"{"type":"assistant","message":{"role":"assistant",
        "usage":{"input_tokens":1,"iterations":[{"input_tokens":1},{"input_tokens":1}]}}}"#;
    let parts = detail_text(two);
    let req = part_of(&parts, "request");
    assert!(req.contains("iterations"), "{req}");
    assert!(req.contains('2'), "{req}");
}

/// `version` is the schema-drift signal: a row whose numbers look wrong can be
/// checked against the build that wrote it.
#[test]
fn the_envelope_carries_what_wrote_the_record() {
    let parts = detail_text(
        r#"{"type":"assistant","requestId":"req-1","sessionId":"s-1","gitBranch":"main",
            "version":"9.9.9","message":{"role":"assistant","usage":{"input_tokens":1}}}"#,
    );
    let env = part_of(&parts, "envelope");
    for want in ["req-1", "s-1", "main", "9.9.9"] {
        assert!(env.contains(want), "{env}");
    }
}

/// A record with nothing to say at this level says nothing, rather than a row
/// with an empty part hanging under it.
#[test]
fn a_record_with_nothing_to_open_has_no_parts() {
    assert!(detail_text(r#"{"type":"file-history-snapshot"}"#).is_empty());
}

// -- against a real corpus --------------------------------------------------------

/// Columns the numeric block occupies on a `usage` row: four fields of eight,
/// joined by two. The contract the whole lens rests on.
const BLOCK: usize = 4 * 8 + 3 * 2;

/// Against a real corpus, when one is pointed at: set `TREAD_AGENT_CORPUS` to a
/// directory of session logs (`~/.claude/projects`).
///
/// Fixtures only prove the shapes someone thought of. Session logs are
/// **private**: this reads them, asserts *structure*, and prints **counts
/// only** — never a byte of their content, and nothing from them is ever copied
/// into this repository. Skipped when the variable is unset, so a clean checkout
/// and CI are unaffected.
#[test]
fn a_real_agent_corpus_is_read_as_what_it_spent() {
    let Ok(root) = std::env::var("TREAD_AGENT_CORPUS") else {
        return;
    };
    let mut files = Vec::new();
    collect(std::path::Path::new(&root), &mut files);
    assert!(!files.is_empty(), "no .jsonl under {root}");
    let (mut records, mut unparsed, mut with_usage, mut side, mut zero) = (0, 0, 0, 0, 0);
    let mut total = 0u64;
    for path in &files {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let mut lens = Usage;
        for line in bytes.split(|&b| b == b'\n') {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            records += 1;
            // A line that does not parse must reach the generic row, not abort:
            // real corpora carry truncated lines.
            let Ok(v) = crate::json::parse(line) else {
                unparsed += 1;
                continue;
            };
            let Some(s) = lens.read(&v) else { continue };
            if s.actor.starts_with('\u{21b3}') {
                side += 1;
                assert!(str_width(&s.actor) <= ACTOR, "a mark that costs a column");
            }
            total = total.saturating_add(s.tokens);
            check_row(&v, &s, &mut with_usage, &mut zero);
        }
    }
    println!(
        "{} files, {records} records ({unparsed} unparsed), {with_usage} with usage \
         ({zero} recording only zeroes), {side} sidechain, {} tokens",
        files.len(),
        crate::lens::tokens(total)
    );
    assert!(with_usage > 0, "no record in the corpus carried usage");
}

/// One record's row, against what the file actually says about it.
fn check_row(v: &crate::json::Value, s: &Summary, with_usage: &mut usize, zero: &mut usize) {
    let usage = v.get("message").and_then(|m| m.get("usage"));
    let counted: Vec<u64> = match usage {
        Some(u) => ["input_tokens", "output_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
            .into_iter()
            .filter_map(|k| super::count(u, k))
            .collect(),
        None => Vec::new(),
    };
    if counted.is_empty() {
        // No counters: the kind and nothing more, so no number column anywhere.
        assert!(!s.what.starts_with("in "), "a row of numbers with no numbers");
        assert_eq!(s.tokens, 0);
        return;
    }
    *with_usage += 1;
    let want: u64 = counted.iter().fold(0u64, |a, b| a.saturating_add(*b));
    assert_eq!(s.tokens, want, "the total is not the sum of the counters");
    if want == 0 {
        *zero += 1;
    }
    // The block is the same width on every row, which is the whole product.
    let block = s.what.split("  \u{b7}  ").next().unwrap_or("");
    assert_eq!(str_width(block), BLOCK, "the column bends on a real record");
}

/// Every `.jsonl` under `dir`, depth-first. No symlink following beyond what
/// `read_dir` gives, and errors are skipped rather than failing the sweep.
fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}
