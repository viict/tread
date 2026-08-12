//! The **ladder**, driven through the seam: `Enter` on a record, `Enter` on a
//! call inside it, `zt` from anywhere, `zR`/`zM`, and the arithmetic under all
//! of it (SPEC.md §Lenses).
//!
//! Deterministic from end to end — a source, a row number, a key. No terminal
//! and no frame, so the composited-capture trap does not apply: every assertion
//! below reads rows the source itself handed back.
//!
//! Every fixture is written by hand, in the *shape* a real ATIF trajectory has.
#![deny(unsafe_code)]

use super::*;
use crate::source::record::Records;
use crate::source::{Anchor, Source};

/// A trajectory with one of everything the level has to answer for: a message
/// long enough to clip, a thought, two calls with several arguments apiece and
/// an output longer than the clip, a step that only worked, and a message with
/// nothing under it at all.
fn run() -> Vec<u8> {
    // Line 9 is long on purpose: a resize has to re-wrap it, which is the one
    // thing that moves rows without a fold changing.
    let said = (1..=9)
        .map(|n| match n {
            9 => format!("said line 9 {}", "and more words to wrap ".repeat(12)),
            n => format!("said line {n}"),
        })
        .collect::<Vec<_>>()
        .join("\\n");
    let out = (1..=30).map(|n| format!("out line {n}")).collect::<Vec<_>>().join("\\n");
    format!(
        r#"{{"schema_version":"ATIF-v1.7","session_id":"sxs_1","steps":[
 {{"step_id":1,"source":"user","message":"do the thing"}},
 {{"step_id":2,"source":"agent","message":"{said}",
   "reasoning_content":"a thought\nover two lines",
   "tool_calls":[{{"tool_call_id":"c1","function_name":"bash",
                   "arguments":{{"command":"cargo test -q","timeout":120,"workdir":"/w"}}}},
                 {{"tool_call_id":"c2","function_name":"read",
                   "arguments":{{"filePath":"src/parse.rs"}}}}],
   "observation":{{"results":[{{"source_call_id":"c1","content":"{out}"}},
                             {{"source_call_id":"c2","content":"fn parse() {{}}"}}]}}}},
 {{"step_id":3,"source":"agent","message":"","reasoning_content":"a short thought",
   "tool_calls":[{{"tool_call_id":"c3","function_name":"glob","arguments":{{"pattern":"**/*.rs"}}}}],
   "observation":{{"results":[{{"source_call_id":"c3","content":"a.rs"}}]}}}},
 {{"step_id":4,"source":"agent","message":"","tool_calls":[{{"tool_call_id":"c4",
   "function_name":"bash","arguments":{{"command":"make"}}}}],
   "observation":{{"results":[{{"source_call_id":"c4","content":"ok"}}]}}}},
 {{"step_id":5,"source":"agent","message":"done."}}]}}"#
    )
    .into_bytes()
}

fn lensed(cols: usize) -> ArraySource {
    let mut s = ArraySource::from_bytes(run(), At::Key("steps"));
    s.set_lens(crate::lens::find("atif").expect("the atif lens"));
    s.set_width(cols);
    let _ = s.len();
    s
}

fn rows(s: &mut ArraySource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text().trim_end().to_string()).collect()
}

/// Records here, 0-based: 0 the envelope, then `steps[0..5]`. So the long
/// message is record 2 and the one-line `done.` is record 5.
const SAID: usize = 2;
const DONE: usize = 5;

/// The row the long message's summary sits on.
fn message_row(s: &ArraySource) -> usize {
    s.row_of_record(SAID)
}

fn has(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|r| r.contains(needle))
}

/// Is `needle` a row **of its own** — a painted body or part row, rather than a
/// substring of the record's raw JSON on a tree row?
fn painted(rows: &[String], needle: &str) -> bool {
    rows.iter().any(|r| r.trim().starts_with(needle))
}

// -- the ladder ------------------------------------------------------------------

/// One rung a press, and round: clipped, the whole of what was said with its
/// calls listed as calls, the record itself, and back to the clip.
#[test]
fn enter_descends_one_rung_a_press_and_wraps() {
    let mut s = lensed(92);
    let row = message_row(&s);
    let clipped = s.len();
    let start = rows(&mut s);
    assert!(has(&start, "said line 1"), "{start:#?}");
    assert!(has(&start, "\u{22ef} +"), "the clip says what it left out: {start:#?}");
    assert!(!painted(&start, "said line 9"), "{start:#?}");

    // Rung two: the whole message, the thought it did not say aloud, and one
    // row per call.
    assert!(s.fold_here(row).is_some(), "the key is the ladder's");
    let open = rows(&mut s);
    assert!(painted(&open, "said line 9"), "the whole message: {open:#?}");
    assert!(has(&open, "thinking"), "{open:#?}");
    assert!(has(&open, "a thought"), "{open:#?}");
    assert!(has(&open, "bash") && has(&open, "cargo test -q"), "{open:#?}");
    assert!(has(&open, "read") && has(&open, "src/parse.rs"), "{open:#?}");
    assert!(has(&open, "\u{2192} 30 lines"), "and what came back: {open:#?}");
    assert!(!has(&open, "out line 1"), "a call is one row until it is opened: {open:#?}");
    assert!(s.len() > clipped);

    // Rung three: the record itself.
    assert!(s.fold_here(row).is_some());
    let tree = rows(&mut s);
    assert!(has(&tree, "\"step_id\""), "the raw record: {tree:#?}");
    assert!(has(&tree, "\"reasoning_content\""), "{tree:#?}");
    assert!(!painted(&tree, "said line 9"), "back to the clip above it: {tree:#?}");

    // And round.
    assert!(s.fold_here(row).is_some());
    assert_eq!(s.len(), clipped, "{:#?}", rows(&mut s));
}

/// A record with nothing under its headline and no calls has **one rung
/// fewer**: there is nothing between what it said and its JSON, so `Enter` goes
/// straight to the tree and straight back. Consuming the key for a repaint of
/// the same rows is the thing this avoids.
#[test]
fn a_record_with_no_body_and_no_calls_has_fewer_rungs() {
    let mut s = lensed(92);
    // `done.` — one line, no calls, no thought.
    let row = s.row_of_record(DONE);
    let clipped = s.len();
    assert!(s.fold_here(row).is_some(), "the ladder still has the tree");
    let tree = rows(&mut s);
    assert!(has(&tree, "\"step_id\""), "straight to the record: {tree:#?}");
    assert!(s.fold_here(row).is_some(), "and back");
    assert_eq!(s.len(), clipped);
}

/// `zt` is the way to the record from **any** rung, and the way back to it
/// without cycling — orthogonal to the level, exactly as it always was.
#[test]
fn zt_reaches_the_tree_from_every_rung() {
    for descents in [0usize, 1] {
        let mut s = lensed(92);
        let row = message_row(&s);
        for _ in 0..descents {
            assert!(s.fold_here(row).is_some(), "descent {descents}");
        }
        let before = s.len();
        assert!(s.toggle_tree(row).is_some(), "rung {descents}: zt opened it");
        let open = rows(&mut s);
        assert!(has(&open, "\"step_id\""), "rung {descents}: {open:#?}");
        // And the level is untouched: what was showing is still showing.
        assert_eq!(
            painted(&open, "said line 9"),
            descents == 1,
            "rung {descents}: zt did not move the level"
        );
        assert!(s.toggle_tree(row).is_some(), "and shuts again");
        assert_eq!(s.len(), before);
    }
}

// -- a call row is itself openable -------------------------------------------------

/// The user's words: "can we make this show the output if `Enter` on that line".
#[test]
fn enter_on_a_call_row_shows_its_arguments_and_its_output_and_shuts_again() {
    let mut s = lensed(92);
    let row = message_row(&s);
    s.fold_here(row).expect("the open level");
    let open = rows(&mut s);
    let shut = s.len();
    let call = open
        .iter()
        .position(|r| r.contains("cargo test -q") && r.contains('\u{25b8}'))
        .expect("a shut call row");

    assert!(s.fold_here(call).is_some(), "the key belongs to the call");
    let wide = rows(&mut s);
    assert!(s.len() > shut, "it grew");
    // Every argument, one per line — including the ones the headline never had
    // room for.
    for (key, value) in [("command", "cargo test -q"), ("timeout", "120"), ("workdir", "/w")] {
        assert!(
            wide.iter().any(|r| r.contains(key) && r.contains(value)),
            "{key}: {wide:#?}"
        );
    }
    // And the output, clipped like a message body, saying what it left out.
    assert!(has(&wide, "out line 1"), "{wide:#?}");
    assert!(!has(&wide, "out line 30"), "clipped: {wide:#?}");
    assert!(
        wide.iter().any(|r| r.trim().starts_with('\u{22ef}') && r.contains("lines")),
        "the clip states the remainder: {wide:#?}"
    );
    // The other call is untouched — one call opens, not all of them.
    assert!(
        wide.iter().any(|r| r.contains("src/parse.rs") && r.contains('\u{25b8}')),
        "{wide:#?}"
    );

    assert!(s.fold_here(call).is_some(), "and shuts again");
    assert_eq!(s.len(), shut, "{:#?}", rows(&mut s));
}

/// Leaving the open level takes the opened call with it: the rows are gone, and
/// a call left open would reappear on the next descent unasked.
#[test]
fn an_opened_call_does_not_survive_the_level_it_was_opened_at() {
    let mut s = lensed(92);
    let row = message_row(&s);
    s.fold_here(row).expect("open");
    let shut = s.len();
    let call = rows(&mut s)
        .iter()
        .position(|r| r.contains("cargo test -q") && r.contains('\u{25b8}'))
        .expect("a call row");
    s.fold_here(call).expect("the call opens");
    s.fold_here(row).expect("on down the ladder");
    s.fold_here(row).expect("and round");
    s.fold_here(row).expect("back to the open level");
    assert_eq!(s.len(), shut, "the call is shut again: {:#?}", rows(&mut s));
}

// -- zR and zM ---------------------------------------------------------------------

/// `zR` is every record at the open level and every run opened; `zM` puts all of
/// it back. Both have a defined answer for the level, and it is the one a pipe
/// gets.
#[test]
fn zr_opens_every_level_and_zm_shuts_them() {
    let mut s = lensed(92);
    let clipped = s.len();
    s.fold_all(false);
    let open = rows(&mut s);
    assert!(s.len() > clipped);
    assert!(painted(&open, "said line 9"), "every message whole: {open:#?}");
    assert!(has(&open, "a short thought"), "every step's reasoning: {open:#?}");
    assert!(has(&open, "**/*.rs"), "every run open, and its calls listed: {open:#?}");
    assert!(has(&open, "\"step_id\""), "and every record's own tree: {open:#?}");
    // Every byte of the longest output is in the tree, which is what makes the
    // clipped part row honest rather than lossy.
    assert!(has(&open, "out line 30"), "{open:#?}");
    s.fold_all(true);
    assert_eq!(s.len(), clipped, "{:#?}", rows(&mut s));
}

// -- the arithmetic ------------------------------------------------------------------

/// The invariant the whole two-level arithmetic rests on, now with a third kind
/// of row in it: at every width and every level, `at(row)` inverts
/// `row_of_record`, every row paints, and the two totals agree.
#[test]
fn rows_and_records_round_trip_at_every_level_and_width() {
    for width in [40usize, 92, 200] {
        for descents in 0..4usize {
            let mut s = lensed(width);
            let row = message_row(&s);
            for _ in 0..descents {
                s.fold_here(row);
            }
            // And a call opened at the level that has them.
            if descents == 1 {
                if let Some(call) = rows(&mut s).iter().position(|r| r.contains('\u{25b8}')) {
                    s.fold_here(call);
                }
            }
            let n = s.len();
            for r in 0..Records::known(&s) {
                let at = s.row_of_record(r);
                assert!(at < n, "{width}/{descents}: record {r} at {at} of {n}");
                // A record inside a shut run has no row of its own; the run's
                // row stands for it, and that row is the run's first record.
                match s.record_visible(r) {
                    true => assert_eq!(s.record_at(at), r, "{width}/{descents}: record {r}"),
                    false => assert!(s.record_at(at) <= r, "{width}/{descents}: record {r}"),
                }
            }
            for row in 0..n {
                assert!(s.row_line(row).is_some(), "{width}/{descents}: row {row} of {n}");
            }
            assert_eq!(s.lines(0..n).len(), n, "{width}/{descents}");
        }
    }
}

/// A resize re-wraps every level, and the cursor's record survives it — the
/// same promise a body already made, now with parts under it.
#[test]
fn a_resize_re_lays_the_open_level() {
    let mut s = lensed(200);
    let row = message_row(&s);
    s.fold_here(row).expect("open");
    let wide = s.len();
    let mark = s.mark(row).expect("a mark");
    s.set_width(40);
    let narrow = s.len();
    assert!(narrow > wide, "narrower columns are more rows: {narrow} vs {wide}");
    assert_eq!(s.locate(mark), Some(s.row_of_record(SAID)), "the mark is the record");
    let got = rows(&mut s);
    assert!(has(&got, "cargo test -q"), "the calls are still listed: {got:#?}");
}

// -- nothing is silently absent -------------------------------------------------------

/// The rule the whole seam is held to (SPEC.md §Lenses): what a level shows
/// plus what its clips *say* accounts for the whole of what is there.
///
/// Read the same record at every level and check the message and the tool's
/// output against what is on the screen: either every line of them is painted,
/// or a row states exactly how many are not.
#[test]
fn every_level_accounts_for_the_whole_message_and_the_whole_output() {
    let said: Vec<String> = (1..=9).map(|n| format!("said line {n}")).collect();
    let out: Vec<String> = (1..=30).map(|n| format!("out line {n}")).collect();
    let mut s = lensed(92);
    let row = message_row(&s);

    for level in 0..4usize {
        let painted = rows(&mut s);
        // Rows belonging to this record: from its own row to the next record's.
        let from = s.row_of_record(SAID);
        let to = s.row_of_record(SAID + 1);
        let mine: Vec<String> = painted[from..to].to_vec();
        let text = mine.join("\n");

        let shown = said.iter().filter(|l| text.contains(l.as_str())).count();
        let missing = said.len() - shown;
        if missing > 0 {
            let note = mine
                .iter()
                .find(|r| r.trim().starts_with('\u{22ef}'))
                .unwrap_or_else(|| panic!("level {level} hid {missing} lines and said nothing: {mine:#?}"));
            assert!(
                note.contains(&format!("+{missing} lines")),
                "level {level}: {missing} lines missing but the note says {note:?}"
            );
        }

        // The output is only ever *claimed* at the open level, and there its
        // headline states the size even while the call is shut.
        if level == 1 {
            assert!(
                mine.iter().any(|r| r.contains(&format!("\u{2192} {} lines", out.len()))),
                "the call row states the whole size: {mine:#?}"
            );
        }
        s.fold_here(row);
    }

    // And at the level that shows the output, what is missing from it is stated
    // in its own lines. Four descents is a whole turn of the ladder, so the
    // record is back at the open level and its calls are listed.
    let call = rows(&mut s)
        .iter()
        .position(|r| r.contains("cargo test -q") && r.contains('\u{25b8}'))
        .expect("a call row");
    s.fold_here(call).expect("the call opens");
    let wide = rows(&mut s);
    let shown = out.iter().filter(|l| wide.iter().any(|r| r.trim() == l.as_str())).count();
    let note = wide
        .iter()
        .filter(|r| r.trim().starts_with('\u{22ef}'))
        .find(|r| r.contains("lines"))
        .expect("the output's clip note");
    assert!(
        note.contains(&format!("+{} lines", out.len() - shown)),
        "{shown} of {} shown, note {note:?}",
        out.len()
    );

    // The floor under all of it: `zt` is every byte, always.
    s.toggle_tree(row).expect("the record itself");
    let raw = rows(&mut s).join("\n");
    assert!(raw.contains("out line 30"), "the record holds every byte");
    assert!(raw.contains("said line 9"));
}

/// A search hit inside a folded run still opens it, with the members' own rows
/// measured — the one path where a fold and a wrap have to agree in the same
/// keystroke.
#[test]
fn revealing_a_hit_inside_a_run_measures_what_it_revealed() {
    let mut s = lensed(92);
    s.set_query("**/*.rs");
    let hit = s.preview_match(Anchor(0), crate::source::Dir::Forward).expect("a hit");
    let n = s.len();
    assert!(hit.anchor.0 < n);
    for row in 0..n {
        assert!(s.row_line(row).is_some(), "row {row} of {n} after the reveal");
    }
    assert!(has(&rows(&mut s), "a short thought"), "the member's reasoning came with it");
}

/// A member of an open run is inset, and everything under its row goes with it.
///
/// SPEC.md §Lenses: the body is "indented to the same column" as the `what` it
/// belongs to. The summary row of a member is inset after its gutter, and its
/// body and part rows had no idea — so a step's reasoning sat two columns left
/// of the words it was reasoning about, while a non-member's lined up.
#[test]
fn a_members_body_and_parts_line_up_with_its_own_words() {
    let mut s = lensed(92);
    // The run of steps 3 and 4 — the two records with no message between the
    // long message and `done.`.
    let _ = rows(&mut s); // the outline is over what the last frame painted
    let run = s.row_of_record(3);
    let entry = s.section_at(run).expect("the run is an outline entry");
    assert!(s.set_fold(entry, false), "the run opens");
    let member = s.row_of_record(3);
    let painted = rows(&mut s);
    let thought = painted
        .iter()
        .position(|r| r.trim().starts_with("a short thought"))
        .expect("the member's reasoning is on a row of its own");
    assert!(thought > member, "under the member's row: {painted:#?}");
    // Where the member's own `what` starts: past its gutter, its inset, the
    // actor column and the clock.
    let at = painted[member].find("thinking").expect("the member says what it did");
    let what = crate::render::str_width(&painted[member][..at]);
    assert_eq!(column_of(&painted[thought]), what, "{painted:#?}");
    // Two columns right of where a record outside the run puts its own body,
    // which is exactly the inset its summary row carries.
    let outside = painted
        .iter()
        .position(|r| r.trim().starts_with("said line 2"))
        .expect("a non-member's body");
    assert_eq!(column_of(&painted[thought]), column_of(&painted[outside]) + 2, "{painted:#?}");
}

/// The **display column** the first non-blank character of a row sits in —
/// never the byte offset: a fold glyph is three bytes and one column wide.
fn column_of(row: &str) -> usize {
    crate::render::str_width(row) - crate::render::str_width(row.trim_start())
}
