//! The pager driving a trajectory read through `--lens`: the message under a
//! row, the two states it has, `zt`, and what a resize does to the cursor.
//!
//! Pager tests, not format tests — everything below goes through key presses
//! and the rows the source hands back, which is the only way to prove that a
//! chord (`za`, `zt`) reaches the source at all. No terminal, no pty: the
//! reconstructed-frame trap does not apply because there is no frame to
//! reconstruct.
#![deny(unsafe_code)]

use super::*;
use crate::source::jsonl::JsonlSource;

/// Twelve lines of message, so a clip has something to clip.
fn long_message() -> String {
    (1..=12)
        .map(|n| format!("line {n} of what was said"))
        .collect::<Vec<_>>()
        .join("\\n")
}

/// A prompt with a long message, a tool call and its result, and a short
/// answer. Hand-written, in the shape a Claude Code session file has.
pub(super) fn run() -> String {
    format!(
        concat!(
            r#"{{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","#,
            r#""message":{{"role":"user","content":"{}"}}}}"#,
            "\n",
            r#"{{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{{"role":"assistant","#,
            r#""content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":"git status"}}}}]}}}}"#,
            "\n",
            r#"{{"type":"user","timestamp":"2026-08-05T14:02:01.000Z","message":{{"role":"user","#,
            r#""content":[{{"type":"tool_result","tool_use_id":"t1","content":"clean"}}]}}}}"#,
            "\n",
            r#"{{"type":"assistant","timestamp":"2026-08-05T14:03:00.000Z","message":{{"role":"assistant","#,
            r#""content":[{{"type":"text","text":"Done."}}]}}}}"#,
            "\n"
        ),
        long_message()
    )
}

fn lens_pager(cols: usize, rows: usize) -> Pager {
    let mut src = JsonlSource::from_bytes(run().into_bytes());
    src.set_lens(crate::lens::find("agent").expect("the agent lens"));
    let mut p = Pager::new(Box::new(src), "session.jsonl".into(), cols, rows, None);
    // The index and the lens are lazy; a frame is what pushes them along.
    let _ = p.visible_text();
    p
}

#[test]
fn a_message_is_read_under_its_row_and_the_clip_says_what_it_hid() {
    let mut p = lens_pager(80, 40);
    let rows = p.visible_text();
    // The summary row *is* the message's first line, and the rest is under it:
    // six rows of message in all, then what the clip left out.
    assert!(rows[0].contains("user       14:01"), "{:?}", rows[0]);
    assert!(rows[0].ends_with("line 1 of what was said"), "{:?}", rows[0]);
    assert_eq!(rows[1], "line 2 of what was said");
    assert_eq!(rows[5], "line 6 of what was said", "{rows:#?}");
    assert_eq!(rows[6], "\u{22ef} +6 lines", "the clip states what it left out");
    assert!(
        !rows.iter().any(|r| r.contains("line 12")),
        "a clipped body is clipped: {rows:#?}"
    );
}

/// The defect: the row painted the excerpt and the body then repeated it. The
/// message's opening words are on the screen once.
#[test]
fn a_message_s_first_line_is_on_the_screen_exactly_once() {
    let mut p = lens_pager(80, 40);
    let rows = p.visible_text();
    let hits = rows.iter().filter(|r| r.contains("line 1 of what was said")).count();
    assert_eq!(hits, 1, "{rows:#?}");
    assert_ne!(rows[0].trim(), rows[1].trim(), "{rows:#?}");
    // And with the whole message open, still once.
    press(&mut p, "za");
    let open = p.visible_text();
    let hits = open.iter().filter(|r| r.contains("line 1 of what was said")).count();
    assert_eq!(hits, 1, "{open:#?}");
}

/// The ladder (SPEC.md §Lenses): `za` descends one rung a press and wraps.
/// Three presses, not two — the raw record is a rung of its own now, and the
/// third press is what comes back to the clip.
#[test]
fn za_descends_a_rung_a_press_and_comes_back_to_the_clip() {
    let mut p = lens_pager(80, 40);
    let clipped = p.line_count();
    press(&mut p, "za");
    let open = p.visible_text();
    assert!(p.line_count() > clipped, "the body grew");
    assert!(open.iter().any(|r| r == "line 12 of what was said"), "{open:#?}");
    assert!(!open.iter().any(|r| r.starts_with('\u{22ef}')), "nothing left to say");

    press(&mut p, "za");
    let tree = p.visible_text();
    assert!(tree.iter().any(|r| r.contains("\"type\"")), "the record itself: {tree:#?}");
    assert!(
        !tree.iter().any(|r| r == "line 12 of what was said"),
        "and what was said is back to its clip: {tree:#?}"
    );

    press(&mut p, "za");
    assert_eq!(p.line_count(), clipped, "and round to the clip");
}

/// `Enter` on a message row is the same key as `za` — it reaches the body
/// through `fold_here`, before the outline is consulted.
#[test]
fn enter_on_a_message_row_opens_its_body_too() {
    let mut p = lens_pager(80, 40);
    let clipped = p.line_count();
    key(&mut p, Key::Enter);
    assert!(p.line_count() > clipped);
}

/// The non-negotiable: whatever the body is doing, the record itself is one
/// keypress away, and it is the record — not a reading of it.
#[test]
fn zt_opens_the_raw_record_whatever_the_body_is_doing() {
    let mut p = lens_pager(80, 40);
    let before = p.line_count();
    press(&mut p, "zt");
    let rows = p.visible_text();
    assert!(p.line_count() > before, "the tree was spliced in");
    assert!(rows.iter().any(|r| r.contains("\"type\"")), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("timestamp")), "{rows:#?}");
    // And the body is still doing what it was doing.
    assert!(rows.iter().any(|r| r == "\u{22ef} +6 lines"), "{rows:#?}");
    press(&mut p, "zt");
    assert_eq!(p.line_count(), before, "and it shuts again");
}

/// From inside a message, `zt` opens the record that message came from. Two
/// presses of `j` is how the cursor gets to line 3 of the first message: `j` is
/// a row, so it walks into the body rather than over the message.
#[test]
fn zt_from_a_body_row_opens_the_record_it_belongs_to() {
    let mut p = lens_pager(80, 40);
    let before = p.line_count();
    press(&mut p, "jj");
    assert_eq!(p.cursor_text(), "line 3 of what was said");
    press(&mut p, "zt");
    assert!(p.line_count() > before, "{}", p.status_line());
    assert!(p.visible_text().iter().any(|r| r.contains("\"uuid\"") || r.contains("\"type\"")));
}

/// On a folded run there is no one record to open, and the pager says so
/// rather than appearing to do nothing — `Enter` is the key that opens a run.
#[test]
fn zt_on_a_run_says_there_is_nothing_to_open() {
    let mut p = lens_pager(80, 40);
    let rows = p.visible_text();
    let run = rows
        .iter()
        .position(|r| r.contains("\u{27e8}"))
        .expect("a folded run");
    p.goto(run);
    let before = p.line_count();
    press(&mut p, "zt");
    assert_eq!(p.line_count(), before);
    assert!(p.status_line().contains("nothing to open"), "{}", p.status_line());
}

/// A resize re-wraps every body, so rows move. The cursor stays on the record
/// it was on — which is what a `Mark` means here, and why it is a record
/// rather than a row.
#[test]
fn a_resize_keeps_the_cursor_on_the_same_record() {
    let mut p = lens_pager(80, 40);
    // Open the first message, so the wrap really does move what is below it:
    // a *clipped* body is six rows at any width.
    press(&mut p, "za");
    // Down to the answer at the end: it is one line, so `G` lands on its own
    // summary row, which is the whole of it.
    press(&mut p, "G");
    let _ = p.visible_text();
    let before = p.cursor_mark();
    let row = p.cursor;
    let said = p.cursor_text();
    assert!(said.contains("Done."), "{said:?}");
    p.resize(40, 40);
    let _ = p.visible_text();
    assert_eq!(p.cursor_mark(), before, "the mark is the record, not the row");
    assert_ne!(p.cursor, row, "the narrower body pushed the row down");
    assert_eq!(p.cursor_text(), said, "and the cursor is on the same content");
}

/// What a record-valued mark costs, stated: the cursor inside a message comes
/// back to that message, not to the line of it that it was on. A `Mark` is one
/// number, and it names the record.
#[test]
fn a_resize_from_inside_a_body_lands_on_its_summary_row() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "jjj");
    let _ = p.visible_text();
    assert_eq!(p.cursor_text(), "line 4 of what was said");
    let mark = p.cursor_mark();
    p.resize(40, 40);
    let _ = p.visible_text();
    assert_eq!(p.cursor_mark(), mark, "still the same record");
    assert!(p.cursor_text().contains("user       14:01"), "{:?}", p.cursor_text());
}

/// `zR` shows everything the viewport has reached, messages included — which
/// is also what a dump asks for — and `zM` puts every one of them back.
#[test]
fn zr_opens_the_bodies_and_zm_clips_them_again() {
    let mut p = lens_pager(80, 40);
    let clipped = p.line_count();
    press(&mut p, "zR");
    let open = p.visible_text();
    assert!(open.iter().any(|r| r == "line 12 of what was said"), "{open:#?}");
    press(&mut p, "zM");
    assert_eq!(p.line_count(), clipped);
}

// -- moving ------------------------------------------------------------------
//
// SPEC.md §"Moving through a document": `j`/`k` move one visible row, here as
// everywhere; `Tab`/`S-Tab` jump block to block and frame what they land on.
// Every assertion below is the pager's own state after a synthetic key press —
// there is no frame here to reconstruct, and none of these read one.

/// Two long messages, so a block reached by `j` is taller than one row and
/// framing has something to do.
fn two_long_messages() -> String {
    format!(
        concat!(
            r#"{{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","#,
            r#""message":{{"role":"user","content":"{}"}}}}"#,
            "\n",
            r#"{{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{{"role":"assistant","#,
            r#""content":[{{"type":"text","text":"{}"}}]}}}}"#,
            "\n"
        ),
        long_message(),
        long_message()
    )
}

fn pager_over(text: String, cols: usize, rows: usize) -> Pager {
    let mut src = JsonlSource::from_bytes(text.into_bytes());
    src.set_lens(crate::lens::find("agent").expect("the agent lens"));
    let mut p = Pager::new(Box::new(src), "session.jsonl".into(), cols, rows, None);
    let _ = p.visible_text();
    p
}

/// The user's first complaint, in a deterministic test: `j` under a lens walks
/// the message's own lines — the clipped body and the row that says what the
/// clip left out — before it ever reaches the run below. `k` mirrors it row for
/// row, and both stop at the ends rather than wrapping or stalling early.
#[test]
fn j_and_k_move_one_row_under_a_lens() {
    let mut p = lens_pager(80, 40);
    assert_eq!(p.cursor, 0);
    assert!(p.cursor_text().contains("line 1 of what was said"));
    for line in 2..=6 {
        press(&mut p, "j");
        assert_eq!(p.cursor_text(), format!("line {line} of what was said"));
    }
    press(&mut p, "j");
    assert_eq!(p.cursor_text(), "\u{22ef} +6 lines", "then the clip's own row");
    press(&mut p, "j");
    assert!(p.cursor_text().contains("\u{27e8}"), "the folded run: {:?}", p.cursor_text());
    press(&mut p, "j");
    assert!(p.cursor_text().contains("Done."), "{:?}", p.cursor_text());
    let last = p.cursor;
    assert_eq!(last, p.line_count() - 1, "which is the last row of the document");
    press(&mut p, "j");
    assert_eq!(p.cursor, last, "and `j` there stays put rather than running off");
    for row in (0..last).rev() {
        press(&mut p, "k");
        assert_eq!(p.cursor, row, "`k` is the mirror, a row at a time");
    }
    press(&mut p, "k");
    assert_eq!(p.cursor, 0, "and `k` on the first row stays put");
}

/// The invariant the reader actually depends on, over a document with a record
/// opened inside it: from the top, `j` alone reaches **every** painted row, in
/// order, and stops on the last one. Nothing on the screen needs a key that no
/// longer exists.
#[test]
fn every_painted_row_is_reachable_with_j_alone() {
    let mut p = lens_pager(80, 40);
    // The whole ladder open: the message in full, its parts, and the raw tree
    // spliced under it — everything the reader can put on the screen at once.
    press(&mut p, "zR");
    let _ = p.visible_text();
    let n = p.line_count();
    assert!(n > 20, "a full record is on the screen: {n} rows");
    let mut seen = vec![p.cursor];
    for _ in 0..n * 2 {
        let before = p.cursor;
        press(&mut p, "j");
        assert!(p.cursor <= before + 1, "one row at a time: {before} -> {}", p.cursor);
        if p.cursor != before {
            seen.push(p.cursor);
        }
    }
    assert_eq!(seen, (0..n).collect::<Vec<_>>(), "every row was reachable by j");
    for _ in 0..n * 2 {
        press(&mut p, "k");
    }
    assert_eq!(p.cursor, 0, "and k walks back to the top");
}

/// `j` inside a block taller than the viewport keeps the reader moving: the
/// cursor walks its rows and the window follows once the cursor reaches the
/// bottom of it, so the text is never frozen for a whole screen.
#[test]
fn j_walks_a_block_taller_than_the_viewport_and_the_window_follows() {
    let mut p = pager_over(two_long_messages(), 80, 6);
    key(&mut p, Key::Tab);
    let h = p.content_rows();
    let block = p.src_block_at(p.cursor).expect("the second block");
    assert!(block.end - block.start > h, "the block is taller: {block:?} in {h}");
    let (start, top) = (p.cursor, p.top);
    for i in 1..h {
        press(&mut p, "j");
        assert_eq!(p.cursor, start + i, "a row a press");
        assert_eq!(p.top, top, "still inside the window");
    }
    press(&mut p, "j");
    assert_eq!(p.top, top + 1, "and now the window follows the cursor down");
}

/// A block that fits the viewport is scrolled fully into view — not merely far
/// enough to show the row `Tab` landed on, which is all the cursor clamp does.
#[test]
fn landing_on_a_block_that_fits_brings_all_of_it_on_screen() {
    let mut p = pager_over(two_long_messages(), 80, 9);
    let h = p.content_rows();
    assert_eq!((p.cursor, p.top), (0, 0));
    key(&mut p, Key::Tab);
    let block = p.src_block_at(p.cursor).expect("the second block");
    assert!(block.end - block.start <= h, "the block fits: {block:?} in {h}");
    assert!(p.top <= block.start, "the block starts on screen: {block:?}, top {}", p.top);
    assert!(
        block.end <= p.top + h,
        "and ends on it: {block:?}, top {} + {h}",
        p.top
    );
    assert!(p.top > 0, "which took a scroll the cursor alone would not have");
}

/// A block taller than the viewport cannot be shown whole, so its first row
/// goes to the top and the reader starts at the beginning of what was said.
#[test]
fn landing_on_a_block_taller_than_the_viewport_puts_its_first_row_at_the_top() {
    let mut p = pager_over(two_long_messages(), 80, 6);
    let h = p.content_rows();
    key(&mut p, Key::Tab);
    let block = p.src_block_at(p.cursor).expect("the second block");
    assert!(block.end - block.start > h, "the block is taller: {block:?} in {h}");
    assert_eq!(p.cursor, block.start);
    assert_eq!(p.top, block.start, "its first row is the top of the screen");
}

/// `Tab` is the fast jump: block to block, over the message's body and over
/// the folded run alike, and `S-Tab` is its exact mirror back up the same
/// sequence. It stops at the end rather than running past it.
#[test]
fn tab_jumps_block_to_block_and_s_tab_mirrors_it() {
    let mut p = lens_pager(80, 40);
    let mut down = Vec::new();
    for _ in 0..2 {
        key(&mut p, Key::Tab);
        down.push(p.cursor);
    }
    assert!(p.cursor_text().contains("Done."), "{:?}", p.cursor_text());
    // Two presses crossed the whole message, body and clip row and all, and
    // then the folded run: one block a press.
    assert_eq!(down.len(), 2);
    let last = p.cursor;
    key(&mut p, Key::Tab);
    assert_eq!(p.cursor, last, "and Tab at the last block stays on it");
    assert!(p.cursor < p.line_count());
    let mut up = Vec::new();
    for _ in 0..2 {
        key(&mut p, Key::BackTab);
        up.push(p.cursor);
    }
    assert_eq!(up, vec![down[0], 0], "S-Tab retraces Tab exactly: down {down:?}");
}

/// When the jump runs out it says so in the unit this document is read in.
/// A trajectory has no headings, and the status bar on the same screen is
/// printing `block n/N`, so the exhaustion message says **block** too.
#[test]
fn the_exhausted_jump_says_block_under_a_lens() {
    let mut p = lens_pager(80, 40);
    key(&mut p, Key::BackTab);
    assert_eq!(p.message.as_deref(), Some("no previous block"), "at the top");
    for _ in 0..3 {
        key(&mut p, Key::Tab);
    }
    assert_eq!(p.message.as_deref(), Some("no further block"), "at the end");
}

/// The status bar says which block the cursor is on, in the same shape and the
/// same vocabulary as the record it already counted.
#[test]
fn the_status_bar_names_the_block() {
    let mut p = lens_pager(80, 40);
    assert!(p.status_line().contains("block 1/3"), "{}", p.status_line());
    press(&mut p, "j");
    assert!(p.status_line().contains("block 1/3"), "a body row is in its block");
    key(&mut p, Key::Tab);
    assert!(p.status_line().contains("block 2/3"), "{}", p.status_line());
    key(&mut p, Key::Tab);
    let last = p.status_line();
    assert!(last.contains("agent"), "{last}");
    assert!(last.contains("record 4/4"), "{last}");
    assert!(last.contains("block 3/3"), "{last}");
}
