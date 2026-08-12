//! A record file read through `--lens`, end to end: what the rows say, what
//! folds, what opens, and — the contract that matters — that nothing is lost.
#![deny(unsafe_code)]

use super::*;
use crate::source::Source;
use crate::source::{Anchor, Dir};

/// A small trajectory in the shape a Claude Code session file has: a prompt,
/// an answer, a run of mechanics, another answer, and one record from a
/// dialect nobody has taught this lens.
pub(super) const RUN: &str = concat!(
    r#"{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","message":{"role":"user","content":"add a lens"}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"On it.\nStarting now."}]}}"#,
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

pub(super) fn lensed(text: &str) -> JsonlSource {
    let mut s = JsonlSource::from_bytes(text.as_bytes().to_vec());
    s.set_lens(crate::lens::find("agent").expect("the agent lens"));
    s.set_width(200);
    while s.extend() {}
    s
}

pub(super) fn rows(s: &mut JsonlSource) -> Vec<String> {
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
    // Three messages — each *starting* on its own summary row, with whatever it
    // said past the first line under it — one folded run, and the record the
    // lens does not know.
    assert_eq!(got.len(), 6, "{got:#?}");
    assert!(got[0].starts_with("\u{25be} user       14:01"), "{:?}", got[0]);
    // A one-line message is entirely on its row: nothing under it repeats it.
    assert!(got[0].ends_with("add a lens"), "{:?}", got[0]);
    assert!(got[1].contains("assistant  14:02   On it."), "{:?}", got[1]);
    assert_eq!(got[2].trim(), "Starting now.", "the second line, once: {:?}", got[2]);
    // Four mechanical records — two calls, two results — as one row.
    assert!(
        got[3].contains("\u{27e8}4 steps \u{b7} 2 tool calls\u{27e9}"),
        "{:?}",
        got[3]
    );
    assert!(got[3].contains("14:02"), "the run is still on the clock: {:?}", got[3]);
    assert!(got[4].contains("assistant  14:03   Done."), "{:?}", got[4]);
    // The record the lens does not know is *not* hidden and not summarised: it
    // is the generic collapsed-record row, exactly as with no lens — and it has
    // no body, because nothing read it as a message.
    assert!(got[5].contains("{\u{2026}2 keys}"), "{:?}", got[5]);
    assert!(got[5].contains("telemetry"), "{:?}", got[5]);
}

/// The defect: the summary row painted the excerpt and the body then started
/// again from the same words. The row *is* the first line now, so the opening
/// of a message is on the screen exactly once.
#[test]
fn a_message_s_first_line_is_the_row_and_is_not_repeated_under_it() {
    let mut s = lensed(RUN);
    let got = rows(&mut s);
    let head = got[1].split("14:02").nth(1).expect("the what column").trim().to_string();
    assert_eq!(head, "On it.");
    let under: Vec<&String> = got[2..3].iter().collect();
    assert!(!under.iter().any(|r| r.trim() == head), "{head:?} repeated: {under:#?}");
    assert_eq!(
        got.iter().filter(|r| r.contains("On it.")).count(),
        1,
        "the opening words appear once: {got:#?}"
    );
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
    let group = 3;
    let entry = s.section_at(group).expect("the group is an outline entry");
    assert!(s.set_fold(entry, false), "the group opens");
    let got = rows(&mut s);
    assert_eq!(got.len(), 10, "four records appear under it: {got:#?}");
    assert!(got[4].contains("Bash(git status)"), "{:?}", got[4]);
    assert!(got[5].contains("Bash \u{2192} 5 bytes"), "{:?}", got[5]);
    assert!(got[6].contains("Read(src/lens/mod.rs)"), "{:?}", got[6]);
    assert!(got[7].contains("Read \u{2192} 2 lines"), "{:?}", got[7]);
    // Members are indented under the run they belong to — after the fold
    // marker, which has to stay the first thing on the row (the painter
    // rewrites it there when the row is shut).
    assert!(got[4].starts_with("\u{25be}   "), "{:?}", got[4]);
    let spans = s.lines(4..5).pop().expect("row").spans;
    assert!(spans[0].text.starts_with('\u{25be}'), "{:?}", spans[0].text);

    // And it closes again. (The outline is a window over the last frame, so
    // the whole document is painted again before it is asked about.)
    let _ = rows(&mut s);
    let entry = s.section_at(group).expect("still there");
    assert!(s.set_fold(entry, true));
    assert_eq!(rows(&mut s).len(), 6);
}

/// A closed run says how much it hides, so the gutter can show it and the
/// painter can draw the closed marker.
#[test]
fn a_closed_run_reports_what_it_hides() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.hidden_at(3), Some(4));
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
    let entry = s.section_at(3).expect("group");
    s.set_fold(entry, false);
    let before = rows(&mut s);
    // Row 4 is the first record of the run.
    let entry = s.section_at(4).expect("a record inside the run");
    assert!(s.set_fold(entry, false), "the record opens");
    let after = rows(&mut s);
    assert!(after.len() > before.len(), "its tree was spliced in");
    assert!(after[5].contains("\"type\""), "{:?}", after[5]);
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
    let entry = s.section_at(3).expect("group");
    s.set_fold(entry, false);
    let _ = rows(&mut s);
    let inner = s.section_at(4).expect("record");
    s.set_fold(inner, false);
    let _ = rows(&mut s);
    let entry = s.section_at(3).expect("group");
    s.set_fold(entry, true);
    assert_eq!(rows(&mut s).len(), 6, "back to the folded conversation");
}

// -- the rest of the seam ----------------------------------------------------------

#[test]
fn the_status_bar_names_the_lens_and_the_record() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let text = s.position_text(3).expect("position");
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
    assert!(got.len() > 6, "the run was opened: {got:#?}");
    assert!(got[hit.anchor.0].contains("Bash(git status)"), "{:?}", got);
}

/// `Y` on a run copies the records it holds — what was folded, as data.
#[test]
fn yanking_a_run_copies_every_record_in_it() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let y = s.yank_section(3).expect("a run yanks");
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
    let entry = s.section_at(3).expect("group");
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
    assert_eq!(rows(&mut s).len(), 6, "every body back to its clip too");
}

/// A landmark is the thing that stands on its own — a message, or a run —
/// rather than a record the run has folded away. Renamed for what it always
/// tested: every run here is **shut**, and a shut run is one landmark. Opening
/// one adds its steps to the sequence, which
/// `block_tests::a_boundary_descends_into_an_open_run` is the test for.
#[test]
fn landmarks_are_the_items_while_every_run_is_shut() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.next_landmark(0, true), Some(1));
    // From inside a message's body, the next item is the next item.
    assert_eq!(s.next_landmark(2, true), Some(3));
    assert_eq!(s.next_landmark(1, true), Some(3));
    assert_eq!(s.next_landmark(3, true), Some(4));
    assert_eq!(s.next_landmark(5, true), None);
    assert_eq!(s.next_landmark(4, false), Some(3));
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


// -- the two states of a message, and the key that reaches them ------------------

/// A message the clip already shows whole, that made no calls, has no **open**
/// rung: there is nothing between its headline and its JSON. `Enter` therefore
/// descends straight to the record's own tree, and again to come back — the
/// ladder with one rung missing rather than a key that does nothing.
#[test]
fn a_message_with_nothing_under_it_descends_straight_to_the_tree() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let clipped = s.len();
    // Row 1 is `assistant 14:02 On it.`, row 2 the rest of what it said.
    assert!(s.fold_here(1).is_some(), "the key is the ladder's");
    let got = rows(&mut s);
    assert!(got.iter().any(|r| r.contains("timestamp")), "the tree: {got:#?}");
    assert!(s.len() > clipped);
    assert!(s.fold_here(1).is_some(), "and round to the clip");
    assert_eq!(s.len(), clipped);
}

/// The ladder, on a message long enough to have every rung: clipped, the whole
/// of what was said, the record itself, and back to the clip.
#[test]
fn a_clipped_message_descends_the_ladder_and_comes_back() {
    let text = format!(
        "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
        (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\\n")
    );
    let mut s = lensed(&text);
    let clipped = s.len();
    assert_eq!(s.fold_here(0), Some(false), "rung two: the whole message");
    let open = s.len();
    assert!(open > clipped, "the whole message is on screen");
    assert!(rows(&mut s).iter().any(|r| r.trim() == "line 20"), "all of it");

    assert!(s.fold_here(0).is_some(), "rung three: the record itself");
    let tree = rows(&mut s);
    assert!(tree.iter().any(|r| r.contains("\"type\"")), "{tree:#?}");
    assert!(!tree.iter().any(|r| r.trim() == "line 20"), "clipped again: {tree:#?}");

    assert!(s.fold_here(0).is_some(), "and round to the clip");
    assert_eq!(s.len(), clipped);
}

/// The summary row and the body rows are one wrap split in two, so they must be
/// wraps of the *same* text. A terminal wide enough for a row to hold more than
/// `lens::BODY_KEEP` bytes wrapped the head into the row and the whole record
/// into the rows under it, and every word between the two first rows was
/// painted nowhere — a lens hiding something (SPEC.md §Lenses), silently,
/// because the row arithmetic stayed self-consistent throughout.
#[test]
fn a_message_wider_than_the_kept_head_is_whole_on_a_wide_terminal() {
    let words: Vec<String> = (0..500).map(|n| format!("w{n:03}")).collect();
    let text = format!(
        "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}\n",
        words.join(" ")
    );
    for width in [92usize, 1100, 2000] {
        let mut s = lensed(&text);
        s.set_width(width);
        assert_eq!(s.fold_here(0), Some(false), "{width}: the message opens");
        let got = rows(&mut s).join(" ");
        for w in &words {
            assert_eq!(
                got.split_whitespace().filter(|t| t == w).count(),
                1,
                "{width}: {w} is on the screen exactly once"
            );
        }
    }
}

// -- what a mark means -----------------------------------------------------------

/// With no lens nothing wraps: a tree row is one row at every width, so a mark
/// is the row and the cursor keeps its place inside an open record. Paying the
/// lens's price here would snap it back to the record's summary row for
/// nothing — and `Pager::relayout` runs on a height-only resize too.
#[test]
fn without_a_lens_a_mark_keeps_the_place_inside_a_record() {
    let mut plain = JsonlSource::from_bytes(RUN.as_bytes().to_vec());
    plain.set_width(200);
    while plain.extend() {}
    let n = plain.len();
    let _ = plain.lines(0..n);
    let entry = plain.section_at(0).expect("record 0 is an outline entry");
    assert!(plain.set_fold(entry, false), "open record 0");
    let m = plain.mark(3).expect("a mark on a tree row");
    plain.set_width(80);
    assert_eq!(plain.locate(m), Some(3), "the same row, because it is the same content");
}

/// Under a lens it is the record, because a resize re-wraps every body.
#[test]
fn under_a_lens_a_mark_is_the_record() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let m = s.mark(2).expect("a mark on a body row");
    s.set_width(60);
    assert_eq!(s.locate(m), Some(s.row_of_record(1)), "record 2's own row");
}


/// Search highlights the *painted* columns. The summary row's text is the
/// message's first wrapped line now, not the flattened excerpt, so `row_text`
/// and `matches_on` have to be looking at the same string — a highlight offset
/// by the difference would underline the wrong words.
#[test]
fn search_highlights_the_summary_row_where_the_words_actually_are() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    s.set_query("On it.");
    at(&mut s, 1, "On it.");
    // And the second line of the message is highlighted on its own row.
    s.set_query("Starting now.");
    at(&mut s, 2, "Starting now.");
}

/// The one hit on `row` covers exactly where `needle` is painted. A `MatchSpan`
/// is in **display columns**, so the check is against the width of the row text
/// before the needle — which is the arithmetic the painter does.
fn at(s: &mut JsonlSource, row: usize, needle: &str) {
    let spans = s.matches_on(row);
    assert_eq!(spans.len(), 1, "{row}: {spans:?}");
    let text = s.line(row).expect("the row").text();
    let byte = text.find(needle).unwrap_or_else(|| panic!("{needle:?} is not on {text:?}"));
    assert_eq!(spans[0].start, crate::render::str_width(&text[..byte]), "{text:?}");
    assert_eq!(spans[0].end - spans[0].start, crate::render::str_width(needle), "{text:?}");
}
