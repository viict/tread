//! A record file read through a lens that counts, end to end: the status bar's
//! session total, and the rows the two usage dialects paint over a real source.
//!
//! Every fixture is hand-written and synthetic — a real session log is never
//! copied into this repository.
#![deny(unsafe_code)]

use super::*;
use crate::source::Source;

/// Two turns: a prompt, an answer that spent, a tool result, an answer that
/// spent, another prompt.
const SPEND: &str = concat!(
    r#"{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","message":{"role":"user","content":"go"}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}],"usage":{"input_tokens":1200,"output_tokens":380,"cache_read_input_tokens":18000,"cache_creation_input_tokens":2100}}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-08-05T14:02:01.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:03:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Done."}],"usage":{"input_tokens":800,"output_tokens":20}}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-08-05T14:04:00.000Z","message":{"role":"user","content":"thanks"}}"#,
    "\n",
);

fn lensed(text: &str, lens: &str) -> JsonlSource {
    let mut s = JsonlSource::from_bytes(text.as_bytes().to_vec());
    s.set_lens(crate::lens::find(lens).expect("a registered lens"));
    s.set_width(200);
    while s.extend() {}
    s
}

fn rows(s: &mut JsonlSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect()
}

/// The status bar carries what has been counted so far, and once the whole file
/// is classified it is the session total with no qualification.
#[test]
fn the_status_bar_carries_the_session_total() {
    let mut s = lensed(SPEND, "usage");
    let _ = rows(&mut s);
    let text = s.position_text(0).expect("a position");
    let total = 1200 + 380 + 18_000 + 2_100 + 800 + 20;
    assert_eq!(crate::lens::tokens(total), "22k");
    assert!(text.contains("  \u{b7}  22k tokens"), "{text}");
    assert!(!text.contains('\u{2265}'), "the file is fully classified: {text}");
}

/// A lens that reads no counters says nothing about tokens, so the status line
/// of every shipped lens is exactly what it always was.
#[test]
fn a_lens_that_counts_nothing_says_nothing_about_tokens() {
    let mut s = lensed(SPEND, "agent");
    let _ = rows(&mut s);
    let text = s.position_text(0).expect("a position");
    assert!(!text.contains("tokens"), "{text}");
    assert!(text.starts_with("agent  \u{b7}  record 1/5"), "{text}");
}

/// The rows themselves, over a real source: the numbers line up under each
/// other, and the group row totals the turn the run belongs to.
#[test]
fn the_numbers_line_up_and_the_run_totals_the_turn() {
    let mut s = lensed(SPEND, "usage");
    let got = rows(&mut s);
    // A prompt, one folded run of three mechanics, and the next prompt.
    assert_eq!(got.len(), 3, "{got:#?}");
    assert!(got[0].contains("user       14:01   user"), "{:?}", got[0]);
    assert!(
        got[1].contains("\u{27e8}3 steps \u{b7} 1 tool call \u{b7} 22k tokens\u{27e9}"),
        "{:?}",
        got[1]
    );
    assert!(got[2].contains("user       14:04   user"), "{:?}", got[2]);
}

/// The numeric block is at the same column on every row of the file, which is
/// the whole product of this lens.
#[test]
fn every_usage_row_puts_its_numbers_in_the_same_column() {
    let mut s = lensed(SPEND, "usage");
    let _ = rows(&mut s);
    // Open the run so its members get rows of their own.
    let id = s.section_at(1).expect("the run's fold id");
    s.set_fold(id, false);
    let got = rows(&mut s);
    let at: Vec<usize> = got.iter().filter_map(|r| r.find("in  ")).collect();
    assert_eq!(at.len(), 2, "two records recorded numbers: {got:#?}");
    assert_eq!(at[0], at[1], "the column bends: {got:#?}");
}
