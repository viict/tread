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
fn run() -> String {
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

#[test]
fn za_opens_the_whole_message_and_shuts_it_again() {
    let mut p = lens_pager(80, 40);
    let clipped = p.line_count();
    press(&mut p, "za");
    let open = p.visible_text();
    assert!(p.line_count() > clipped, "the body grew");
    assert!(open.iter().any(|r| r == "line 12 of what was said"), "{open:#?}");
    assert!(!open.iter().any(|r| r.starts_with('\u{22ef}')), "nothing left to say");
    press(&mut p, "za");
    assert_eq!(p.line_count(), clipped, "and it clips again");
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

/// From inside a message, `zt` opens the record that message came from.
#[test]
fn zt_from_a_body_row_opens_the_record_it_belongs_to() {
    let mut p = lens_pager(80, 40);
    let before = p.line_count();
    press(&mut p, "jjzt");
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
