//! A record file read through `--lens`, end to end: what the rows say, what
//! folds, what opens, and — the contract that matters — that nothing is lost.
#![deny(unsafe_code)]

use super::*;
use crate::source::Source;

/// A small trajectory in the shape a Claude Code session file has: a prompt,
/// an answer, a run of mechanics, another answer, and one record from a
/// dialect nobody has taught this lens.
const RUN: &str = concat!(
    r#"{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","message":{"role":"user","content":"add a lens"}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"On it."}]}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:02:10.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status"}}]}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-08-05T14:02:11.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"clean"}]}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:02:20.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"src/lens/mod.rs"}}]}}"#,
    "\n",
    r#"{"type":"user","timestamp":"2026-08-05T14:02:21.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"a\nb"}]}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:03:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Done."}]}}"#,
    "\n",
    r#"{"type":"telemetry","payload":{"kept":true}}"#,
    "\n",
);

fn lensed(text: &str) -> JsonlSource {
    let mut s = JsonlSource::from_bytes(text.as_bytes().to_vec());
    s.set_lens(crate::lens::find("agent").expect("the agent lens"));
    s.set_width(200);
    while s.extend() {}
    s
}

fn rows(s: &mut JsonlSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n)
        .iter()
        .map(|l| l.text().trim_end().to_string())
        .collect()
}

// -- the conversation ------------------------------------------------------------

#[test]
fn messages_stay_and_mechanics_fold_into_one_row() {
    let mut s = lensed(RUN);
    let got = rows(&mut s);
    assert_eq!(got.len(), 5, "{got:#?}");
    assert!(got[0].starts_with("\u{25be} user       14:01"), "{:?}", got[0]);
    assert!(got[0].ends_with("add a lens"), "{:?}", got[0]);
    assert!(got[1].contains("assistant  14:02   On it."), "{:?}", got[1]);
    // Four mechanical records — two calls, two results — as one row.
    assert!(
        got[2].contains("\u{27e8}4 steps \u{b7} 2 tool calls\u{27e9}"),
        "{:?}",
        got[2]
    );
    assert!(got[2].contains("14:02"), "the run is still on the clock: {:?}", got[2]);
    assert!(got[3].contains("assistant  14:03   Done."), "{:?}", got[3]);
    // The record the lens does not know is *not* hidden and not summarised: it
    // is the generic collapsed-record row, exactly as with no lens.
    assert!(got[4].contains("{\u{2026}2 keys}"), "{:?}", got[4]);
    assert!(got[4].contains("telemetry"), "{:?}", got[4]);
}

/// Without `--lens`, nothing changes (SPEC.md §Lenses).
#[test]
fn no_lens_is_the_generic_tree() {
    let mut plain = JsonlSource::from_bytes(RUN.as_bytes().to_vec());
    plain.set_width(200);
    while plain.extend() {}
    let got = rows(&mut plain);
    assert_eq!(got.len(), 8, "one row per record: {got:#?}");
    assert!(got.iter().all(|r| r.contains("keys}")), "{got:#?}");
}

// -- opening what was folded -------------------------------------------------------

#[test]
fn a_group_opens_into_its_records() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let group = 2;
    let entry = s.section_at(group).expect("the group is an outline entry");
    assert!(s.set_fold(entry, false), "the group opens");
    let got = rows(&mut s);
    assert_eq!(got.len(), 9, "four records appear under it: {got:#?}");
    assert!(got[3].contains("Bash(git status)"), "{:?}", got[3]);
    assert!(got[4].contains("Bash \u{2192} 5 bytes"), "{:?}", got[4]);
    assert!(got[5].contains("Read(src/lens/mod.rs)"), "{:?}", got[5]);
    assert!(got[6].contains("Read \u{2192} 2 lines"), "{:?}", got[6]);
    // Members are indented under the run they belong to — after the fold
    // marker, which has to stay the first thing on the row (the painter
    // rewrites it there when the row is shut).
    assert!(got[3].starts_with("\u{25be}   "), "{:?}", got[3]);
    let spans = s.lines(3..4).pop().expect("row").spans;
    assert!(spans[0].text.starts_with('\u{25be}'), "{:?}", spans[0].text);

    // And it closes again. (The outline is a window over the last frame, so
    // the whole document is painted again before it is asked about.)
    let _ = rows(&mut s);
    let entry = s.section_at(group).expect("still there");
    assert!(s.set_fold(entry, true));
    assert_eq!(rows(&mut s).len(), 5);
}

/// A closed run says how much it hides, so the gutter can show it and the
/// painter can draw the closed marker.
#[test]
fn a_closed_run_reports_what_it_hides() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.hidden_at(2), Some(4));
    // A message row is foldable too — it opens into the raw record — so what
    // it reports is its own tree, not a run.
    assert_eq!(s.hidden_at(0), Some(s.line(0).map(|_| 7).unwrap_or(0)));
}

/// A record inside a run still opens into its own tree, and the rows below it
/// stay in the right places: the two levels of folding compose.
#[test]
fn a_record_inside_a_run_still_opens_into_its_tree() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let entry = s.section_at(2).expect("group");
    s.set_fold(entry, false);
    let before = rows(&mut s);
    // Row 3 is the first record of the run.
    let entry = s.section_at(3).expect("a record inside the run");
    assert!(s.set_fold(entry, false), "the record opens");
    let after = rows(&mut s);
    assert!(after.len() > before.len(), "its tree was spliced in");
    assert!(after[4].contains("\"type\""), "{:?}", after[4]);
    // Everything after the expansion is still where it belongs.
    assert!(after[after.len() - 1].contains("telemetry"), "{:?}", after);
    assert!(after.iter().any(|r| r.contains("Done.")), "{after:#?}");
}

/// Closing a run closes the trees inside it: a hidden record may not keep
/// rows, or every row after it would be off by that many.
#[test]
fn closing_a_run_closes_what_was_open_inside_it() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let entry = s.section_at(2).expect("group");
    s.set_fold(entry, false);
    let _ = rows(&mut s);
    let inner = s.section_at(3).expect("record");
    s.set_fold(inner, false);
    let _ = rows(&mut s);
    let entry = s.section_at(2).expect("group");
    s.set_fold(entry, true);
    assert_eq!(rows(&mut s).len(), 5, "back to the folded conversation");
}

// -- the rest of the seam ----------------------------------------------------------

#[test]
fn the_status_bar_names_the_lens_and_the_record() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let text = s.position_text(2).expect("position");
    assert!(text.contains("agent"), "{text}");
    assert!(text.contains("record 3/8"), "{text}");
}

/// Search finds a record the run has folded away, and opens the run so the
/// match can actually be seen.
#[test]
fn a_match_inside_a_folded_run_is_revealed() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    s.set_query("git status");
    let hit = s.cycle_match(Anchor(0), Dir::Forward).expect("found");
    let got = rows(&mut s);
    assert!(got.len() > 5, "the run was opened: {got:#?}");
    assert!(got[hit.anchor.0].contains("Bash(git status)"), "{:?}", got);
}

/// `Y` on a run copies the records it holds — what was folded, as data.
#[test]
fn yanking_a_run_copies_every_record_in_it() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let y = s.yank_section(2).expect("a run yanks");
    assert_eq!(y.what, "4 records");
    assert_eq!(y.text.lines().count(), 4, "{}", y.text);
    for line in y.text.lines() {
        crate::json::parse(line.as_bytes()).expect("each line is valid JSON");
    }
}

/// Folds survive being stored and handed back, groups and records alike.
#[test]
fn fold_state_round_trips() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let entry = s.section_at(2).expect("group");
    s.set_fold(entry, false);
    let open = rows(&mut s);
    let folds = s.folds();
    assert!(folds.iter().any(|f| f.starts_with('g')), "{folds:?}");

    let mut other = lensed(RUN);
    let _ = rows(&mut other);
    other.set_folds(folds);
    assert_eq!(rows(&mut other), open);
}

/// `zM` shuts every run, `zR` opens what the viewport has reached — and
/// nothing between them loses a record.
#[test]
fn fold_all_both_ways_keeps_every_record() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    s.fold_all(false);
    let opened = rows(&mut s);
    for needle in ["add a lens", "On it.", "git status", "Done.", "telemetry"] {
        assert!(opened.iter().any(|r| r.contains(needle)), "{needle} missing");
    }
    s.fold_all(true);
    assert_eq!(rows(&mut s).len(), 5);
}

/// `Tab` moves between the things that stand on their own — messages and
/// runs — rather than through the records a run has folded away.
#[test]
fn landmarks_are_the_items() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.next_landmark(0, true), Some(1));
    assert_eq!(s.next_landmark(1, true), Some(2));
    assert_eq!(s.next_landmark(2, true), Some(3));
    assert_eq!(s.next_landmark(4, true), None);
    assert_eq!(s.next_landmark(3, false), Some(2));
    assert_eq!(s.next_landmark(0, false), None);
}

/// A line that is not JSON keeps its error row under a lens too: half a log is
/// still worth reading, and the lens must not be what hides the broken line.
#[test]
fn a_bad_line_still_renders_under_a_lens() {
    let text = format!("{}{}\n", RUN, "{not json");
    let mut s = lensed(&text);
    let got = rows(&mut s);
    assert!(
        got.iter().any(|r| r.contains("line 9")),
        "the bad line is named: {got:#?}"
    );
}

/// The row count and the rows agree at every fold state — the invariant the
/// whole two-level arithmetic rests on.
#[test]
fn rows_and_len_agree_through_every_fold() {
    let mut s = lensed(RUN);
    for &closed in &[true, false, true] {
        s.fold_all(closed);
        let n = s.len();
        assert_eq!(s.lines(0..n).len(), n);
        assert_eq!(s.lines(0..n + 20).len(), n, "asking past the end is clamped");
        for row in 0..n {
            assert!(s.line(row).is_some(), "row {row} of {n}");
        }
    }
}

/// `--toc` through the lens is the conversation as a list; without one it is
/// the generic record summary, unchanged.
#[test]
fn the_table_of_contents_follows_the_lens() {
    let mut s = lensed(RUN);
    let toc = s.summaries(100);
    assert_eq!(toc.len(), 8, "one line per record, none folded away: {toc:#?}");
    assert!(toc[0].starts_with("1\tuser\t14:01\tadd a lens"), "{:?}", toc[0]);
    assert!(toc[2].contains("Bash(git status)"), "{:?}", toc[2]);
    // The record the lens does not know keeps the generic line.
    assert!(toc[7].contains("keys}"), "{:?}", toc[7]);

    let mut plain = JsonlSource::from_bytes(RUN.as_bytes().to_vec());
    plain.set_width(200);
    assert!(plain.summaries(100)[0].contains("keys}"));
}

/// `--toc` writes straight to a terminal, so a control character inside a
/// record must not reach it — the same rule the painted rows follow.
#[test]
fn the_table_of_contents_is_sanitised() {
    let text = concat!(
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"be\\u0007ll\"}}",
        "\n"
    );
    let mut s = lensed(text);
    let toc = s.summaries(10);
    assert!(!toc[0].contains('\u{7}'), "{:?}", toc[0]);
    assert!(toc[0].contains("be\u{b7}ll"), "{:?}", toc[0]);
}

