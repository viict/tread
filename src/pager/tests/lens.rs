//! The pager driving a trajectory read through `--lens`: the message under a
//! row, the two states it has, `r`, and what a resize does to the cursor.
//!
//! Pager tests, not format tests — everything below goes through key presses
//! and the rows the source hands back, which is the only way to prove that a
//! key (`za`, `r`) reaches the source at all. No terminal, no pty: the
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

/// The two levels (SPEC.md §Lenses): `za` toggles clipped and open, and the
/// raw JSON is not one of them. Three presses, and the record's own bytes are
/// on the screen at no point in the cycle — that is `r`'s job now.
#[test]
fn za_toggles_the_two_levels_and_never_reaches_the_tree() {
    let mut p = lens_pager(80, 40);
    let clipped = p.line_count();
    press(&mut p, "za");
    let open = p.visible_text();
    assert!(p.line_count() > clipped, "the body grew");
    assert!(open.iter().any(|r| r == "line 12 of what was said"), "{open:#?}");
    assert!(!open.iter().any(|r| r.starts_with('\u{22ef}')), "nothing left to say");
    assert!(!open.iter().any(|r| r.contains("\"type\"")), "no JSON: {open:#?}");

    press(&mut p, "za");
    assert_eq!(p.line_count(), clipped, "and back to the clip in one press");
    let back = p.visible_text();
    assert!(back.iter().any(|r| r == "\u{22ef} +6 lines"), "{back:#?}");
    assert!(!back.iter().any(|r| r.contains("\"type\"")), "still no JSON: {back:#?}");

    press(&mut p, "za");
    assert!(p.line_count() > clipped, "and open again");
    assert!(
        !p.visible_text().iter().any(|r| r.contains("\"type\"")),
        "three presses and the JSON never appeared: {:#?}",
        p.visible_text()
    );
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
/// keypress away, and it is the record — not a reading of it. `r` from the
/// clipped level, and `r` again to shut it.
#[test]
fn r_shows_the_raw_record_whatever_the_body_is_doing() {
    let mut p = lens_pager(80, 40);
    let before = p.line_count();
    press(&mut p, "r");
    let rows = p.visible_text();
    assert!(p.line_count() > before, "the tree was spliced in");
    assert!(rows.iter().any(|r| r.contains("\"type\"")), "{rows:#?}");
    assert!(rows.iter().any(|r| r.contains("timestamp")), "{rows:#?}");
    // And the body is still doing what it was doing.
    assert!(rows.iter().any(|r| r == "\u{22ef} +6 lines"), "{rows:#?}");
    press(&mut p, "r");
    assert_eq!(p.line_count(), before, "and it shuts again");
    assert!(!p.visible_text().iter().any(|r| r.contains("\"type\"")));
}

/// The same key from the **open** level, which is the other rung it has to
/// work from: the tree goes in over the whole message, and comes back out
/// leaving that message open.
#[test]
fn r_shows_the_raw_record_from_the_open_level_too() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "za");
    let open = p.line_count();
    press(&mut p, "r");
    let with_tree = p.visible_text();
    assert!(p.line_count() > open, "the tree went in");
    assert!(with_tree.iter().any(|r| r.contains("\"type\"")), "{with_tree:#?}");
    assert!(
        with_tree.iter().any(|r| r == "line 12 of what was said"),
        "and the level is where it was: {with_tree:#?}"
    );
    press(&mut p, "r");
    assert_eq!(p.line_count(), open, "shut, and the message still open");
    assert!(p.visible_text().iter().any(|r| r == "line 12 of what was said"));
}

/// The documented answer to "what does `Enter` do while the tree is open": it
/// leaves the tree alone and toggles the record's own rows underneath it. `r`
/// owns the tree, and a key that silently undid another key's work is the thing
/// this change removed (SPEC.md §Lenses).
#[test]
fn enter_with_the_tree_open_leaves_the_tree_and_toggles_the_record() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "r");
    let with_tree = p.line_count();
    press(&mut p, "za");
    let open = p.visible_text();
    assert!(p.line_count() > with_tree, "the message opened under it");
    assert!(open.iter().any(|r| r.contains("\"type\"")), "tree still there: {open:#?}");
    assert!(open.iter().any(|r| r == "line 12 of what was said"), "{open:#?}");
    press(&mut p, "za");
    assert_eq!(p.line_count(), with_tree, "back to the clip, tree untouched");
    assert!(p.visible_text().iter().any(|r| r.contains("\"type\"")), "and it is still open");
    // And `r` is what shuts it, from either level.
    press(&mut p, "r");
    assert!(!p.visible_text().iter().any(|r| r.contains("\"type\"")));
}

/// From inside a message, `r` opens the record that message came from. Two
/// presses of `j` is how the cursor gets to line 3 of the first message: `j` is
/// a row, so it walks into the body rather than over the message.
#[test]
fn r_from_a_body_row_opens_the_record_it_belongs_to() {
    let mut p = lens_pager(80, 40);
    let before = p.line_count();
    press(&mut p, "jj");
    assert_eq!(p.cursor_text(), "line 3 of what was said");
    press(&mut p, "r");
    assert!(p.line_count() > before, "{}", p.status_line());
    assert!(p.visible_text().iter().any(|r| r.contains("\"uuid\"") || r.contains("\"type\"")));
}

/// On a folded run there is no one record to open, and the pager says so
/// rather than appearing to do nothing — `Enter` is the key that opens a run.
#[test]
fn r_on_a_run_says_there_is_nothing_to_open() {
    let mut p = lens_pager(80, 40);
    let rows = p.visible_text();
    let run = rows
        .iter()
        .position(|r| r.contains("\u{27e8}"))
        .expect("a folded run");
    p.goto(run);
    let before = p.line_count();
    press(&mut p, "r");
    assert_eq!(p.line_count(), before);
    assert!(p.status_line().contains("nothing to open"), "{}", p.status_line());
}

/// Taking `r` had to be free everywhere, not just on a lens row. In the search
/// prompt it is a letter of the query. In the outline overlay it is none of the
/// overlay's own keys, so the selection does not move and it falls through to
/// the normal dispatcher — which is what every unbound key there has always
/// done, and what the chord it replaced did from the same place. A live
/// visual selection is
/// `Mode::Normal`, so there it opens the record and the selection follows the
/// cursor.
///
/// The one other overlay that falls *through* is the CSV row detail, pinned in
/// `pager::tests::csv::r_is_not_one_of_the_row_details_keys`. The two that do
/// not need one swallow the key by construction rather than by binding it:
/// `Mode::Index` consumes every key it does not know (`Pager::index_key`'s
/// `_ => {}`, and its filter takes letters as query text), and `Mode::Help`
/// closes on any key at all.
#[test]
fn r_is_free_in_every_mode() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "/");
    assert_eq!(p.mode, Mode::Search(Dir::Forward));
    let clipped = p.line_count();
    press(&mut p, "r");
    assert_eq!(p.query, "r", "the prompt takes it as a letter");
    assert_eq!(p.line_count(), clipped, "and no tree opened behind it");
    key(&mut p, Key::Esc);
    assert_eq!(p.line_count(), clipped);

    p.mode = Mode::Outline;
    p.outline_sel = 0;
    press(&mut p, "r");
    assert_eq!(p.outline_sel, 0, "the overlay has no `r` of its own");
    assert!(p.line_count() > clipped, "it fell through, as any unbound key does");
    press(&mut p, "r");
    assert_eq!(p.line_count(), clipped, "and the second press shut it again");
    p.mode = Mode::Normal;

    press(&mut p, "v");
    assert!(p.select.is_some(), "visual mode");
    press(&mut p, "r");
    assert!(p.line_count() > clipped, "and there it is the raw record");
    assert!(p.select.is_some(), "the selection survives it");
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

/// The half of "`Enter` never opens a tree and never shuts one" that the
/// headline of a record *with* a rung cannot prove. `Done.` fits its row and
/// made no calls, so it has no rung at all — and a record with no rung is
/// exactly where the key used to fall through to the outline, whose entry for a
/// record is its raw tree. With the tree `r` opened, `Enter` there leaves it
/// alone; shut, `Enter` does not open one.
#[test]
fn enter_on_a_record_with_no_rung_leaves_its_tree_alone() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "G");
    assert!(p.cursor_text().contains("Done."), "{:?}", p.cursor_text());
    let shut = p.line_count();
    key(&mut p, Key::Enter);
    assert_eq!(p.line_count(), shut, "no rung, and no tree: `Enter` did nothing");
    assert!(!p.visible_text().iter().any(|r| r.contains("\"type\"")), "and no JSON");
    press(&mut p, "za");
    assert_eq!(p.line_count(), shut, "`za` is the same key and the same answer");

    press(&mut p, "r");
    let with_tree = p.line_count();
    assert!(with_tree > shut, "`r` is what opens it");
    key(&mut p, Key::Enter);
    assert_eq!(p.line_count(), with_tree, "and `Enter` leaves it exactly where it is");
    assert!(p.visible_text().iter().any(|r| r.contains("\"type\"")), "tree still there");
    press(&mut p, "r");
    assert_eq!(p.line_count(), shut, "the `r` that opened it is the way out");
}

/// The other row the fall-through reached: a row **inside** an open tree. It is
/// a row of the record, so the key is the record's — and this record has no
/// rung, so nothing moves. What it must not do is shut the tree the cursor is
/// standing in.
#[test]
fn enter_inside_an_open_tree_does_not_shut_it() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "G");
    press(&mut p, "r");
    let with_tree = p.line_count();
    press(&mut p, "j");
    assert!(p.cursor_text().contains('"'), "a row of the tree: {:?}", p.cursor_text());
    key(&mut p, Key::Enter);
    assert_eq!(p.line_count(), with_tree, "the tree the cursor is in survives");
    press(&mut p, "za");
    assert_eq!(p.line_count(), with_tree);
    assert!(p.visible_text().iter().any(|r| r.contains("\"type\"")));
}

/// `zR` puts every record at the open level, rungless ones included. `Enter`
/// there is still not a rung: it neither repaints the same rows and calls it a
/// descent, nor — on the press after — falls through and toggles the JSON `zR`
/// had opened.
#[test]
fn enter_on_a_record_with_no_rung_after_zr_is_inert_every_press() {
    let mut p = lens_pager(80, 40);
    press(&mut p, "zR");
    let rows = p.visible_text();
    let done = rows.iter().position(|r| r.contains("Done.")).expect("the answer");
    p.goto(done);
    let open = p.line_count();
    for press_no in 0..3 {
        key(&mut p, Key::Enter);
        assert_eq!(p.line_count(), open, "press {press_no} moved rows");
    }
}
