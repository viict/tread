//! Reading **a run the reader has opened** (SPEC.md §Lenses).
//!
//! The defect these pin: `j` moved by block, so it stepped over the very steps
//! `Enter` had just revealed. `j` is a row now, in every format, so it walks
//! whatever opening a run put on the screen — and `k` mirrors it, including
//! stepping back out of the run to the row above it. `Tab` is the block jump,
//! and it is the key that clears a member whole.
//!
//! Pager tests: every assertion is the pager's own state after a synthetic key
//! press. There is no terminal and no frame here to reconstruct.
#![deny(unsafe_code)]

use super::*;
use crate::source::jsonl::JsonlSource;

/// A prompt, four mechanical records that fold into one run, and an answer.
/// Hand-written, in the shape a Claude Code session file has.
fn run_of_four() -> String {
    concat!(
        r#"{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","#,
        r#""message":{"role":"user","content":"add a lens"}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{"role":"assistant","#,
        r#""content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status"}}]}}"#,
        "\n",
        r#"{"type":"user","timestamp":"2026-08-05T14:02:01.000Z","message":{"role":"user","#,
        r#""content":[{"type":"tool_result","tool_use_id":"t1","content":"clean"}]}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-05T14:02:20.000Z","message":{"role":"assistant","#,
        r#""content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"src/lens/mod.rs"}}]}}"#,
        "\n",
        r#"{"type":"user","timestamp":"2026-08-05T14:02:21.000Z","message":{"role":"user","#,
        r#""content":[{"type":"tool_result","tool_use_id":"t2","content":"a\nb"}]}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-05T14:03:00.000Z","message":{"role":"assistant","#,
        r#""content":[{"type":"text","text":"Done."}]}}"#,
        "\n"
    )
    .to_string()
}

/// The pager, with the run under the cursor already open: `j` onto it, `Enter`.
fn with_open_run(rows: usize) -> Pager {
    let mut src = JsonlSource::from_bytes(run_of_four().into_bytes());
    src.set_lens(crate::lens::find("agent").expect("the agent lens"));
    let mut p = Pager::new(Box::new(src), "session.jsonl".into(), 80, rows, None);
    let _ = p.visible_text();
    press(&mut p, "j");
    assert!(p.cursor_text().contains('\u{27e8}'), "on the run: {:?}", p.cursor_text());
    key(&mut p, Key::Enter);
    p
}

/// The user's own sequence: `j` onto the run, `Enter`, and the next `j` lands
/// on the **first step inside it** rather than on the block after the run.
#[test]
fn j_after_enter_lands_inside_the_run() {
    let mut p = with_open_run(40);
    let run = p.cursor;
    press(&mut p, "j");
    assert_eq!(p.cursor, run + 1, "the first step, not the block after the run");
    assert!(p.cursor_text().contains("Bash(git status)"), "{:?}", p.cursor_text());
}

/// And it keeps going: every member in order, then out of the run onto the
/// block after it.
#[test]
fn j_visits_every_member_then_leaves_the_run() {
    let mut p = with_open_run(40);
    let mut seen = Vec::new();
    for _ in 0..5 {
        press(&mut p, "j");
        seen.push(p.cursor_text());
    }
    assert!(seen[0].contains("Bash(git status)"), "{seen:#?}");
    assert!(seen[1].contains("Bash \u{2192}"), "{seen:#?}");
    assert!(seen[2].contains("Read(src/lens/mod.rs)"), "{seen:#?}");
    assert!(seen[3].contains("Read \u{2192}"), "{seen:#?}");
    assert!(seen[4].contains("Done."), "out of the run: {seen:#?}");
}

/// `k` mirrors `j` step for step over the same rows, out of the run and on up.
#[test]
fn k_mirrors_j_over_an_open_run() {
    let mut p = with_open_run(40);
    let start = p.cursor;
    let mut down = Vec::new();
    for _ in 0..5 {
        press(&mut p, "j");
        down.push(p.cursor);
    }
    let mut up = Vec::new();
    for _ in 0..5 {
        press(&mut p, "k");
        up.push(p.cursor);
    }
    let mut want: Vec<usize> = down[..down.len() - 1].to_vec();
    want.reverse();
    want.push(start);
    assert_eq!(up, want, "down {down:?}");
    press(&mut p, "k");
    assert_eq!(p.cursor, 0, "and out of the run to the block above it");
}

/// The user's second complaint, in a deterministic test: with a step's raw JSON
/// open, `j` walks **its** rows. A member and its tree are still one block —
/// that is what `Tab` clears in a press — but the cursor unit is the row, so
/// nothing that was just revealed is skipped.
#[test]
fn j_reads_the_tree_a_member_has_open_and_tab_clears_it() {
    let mut p = with_open_run(60);
    press(&mut p, "j");
    let member = p.cursor;
    // `zt` is what opens a step's raw record: a step has no message to unfold,
    // so `Enter` on one is not the key that gives it a tree.
    press(&mut p, "zt");
    let block = p.src_block_at(member).expect("the member's block");
    assert_eq!(block.start, member);
    assert!(block.end > member + 1, "the tree is in it: {block:?}");
    assert_eq!(
        p.src_block_at(block.end - 1),
        Some(block.clone()),
        "its last tree row too"
    );
    // Every row of that tree, one press each, in order.
    for row in member + 1..block.end {
        press(&mut p, "j");
        assert_eq!(p.cursor, row, "j walks the tree row by row");
    }
    press(&mut p, "k");
    assert_eq!(p.cursor, block.end - 2, "and k is its mirror, a row at a time");
    // The block jump is what steps over the whole of it, in either direction.
    key(&mut p, Key::Tab);
    assert_eq!(p.cursor, block.end, "one Tab clears the whole member");
    key(&mut p, Key::BackTab);
    assert_eq!(p.cursor, member, "and one S-Tab comes back to it");
}

/// The status bar counts what `Tab` steps by: opening the run adds its members
/// to the block total rather than letting the counter drift.
#[test]
fn the_block_counter_follows_the_open_run() {
    let mut src = JsonlSource::from_bytes(run_of_four().into_bytes());
    src.set_lens(crate::lens::find("agent").expect("the agent lens"));
    let mut p = Pager::new(Box::new(src), "session.jsonl".into(), 80, 40, None);
    let _ = p.visible_text();
    press(&mut p, "j");
    assert!(p.status_line().contains("block 2/3"), "{}", p.status_line());
    key(&mut p, Key::Enter);
    assert!(p.status_line().contains("block 2/7"), "{}", p.status_line());
    press(&mut p, "j");
    assert!(p.status_line().contains("block 3/7"), "{}", p.status_line());
    press(&mut p, "jjjj");
    assert!(p.cursor_text().contains("Done."), "{:?}", p.cursor_text());
    assert!(p.status_line().contains("block 7/7"), "{}", p.status_line());
}

/// Shutting the run puts it back to one block, and the cursor is left on the
/// run's own row — the thing it just closed, not a row that no longer exists.
#[test]
fn closing_the_run_collapses_it_back_to_one_block() {
    let mut p = with_open_run(40);
    let run = p.cursor;
    let open_rows = p.line_count();
    key(&mut p, Key::Enter);
    assert_eq!(p.cursor, run);
    assert!(p.cursor_text().contains('\u{27e8}'), "{:?}", p.cursor_text());
    assert_eq!(p.line_count(), open_rows - 4, "the four steps are folded again");
    assert_eq!(p.src_block_at(run), Some(run..run + 1), "one block again");
    assert!(p.status_line().contains("block 2/3"), "{}", p.status_line());
    key(&mut p, Key::Tab);
    assert!(p.cursor_text().contains("Done."), "and Tab steps over it again");
}

/// A prompt, an answer, and then four mechanical records with nothing after
/// them — a trajectory that ends in a run, which is where `Tab` runs out of
/// messages.
fn run_at_the_tail() -> String {
    concat!(
        r#"{"type":"user","timestamp":"2026-08-05T14:01:00.000Z","#,
        r#""message":{"role":"user","content":"add a lens"}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-05T14:01:30.000Z","message":{"role":"assistant","#,
        r#""content":[{"type":"text","text":"Done."}]}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-05T14:02:00.000Z","message":{"role":"assistant","#,
        r#""content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status"}}]}}"#,
        "\n",
        r#"{"type":"user","timestamp":"2026-08-05T14:02:01.000Z","message":{"role":"user","#,
        r#""content":[{"type":"tool_result","tool_use_id":"t1","content":"clean"}]}}"#,
        "\n",
        r#"{"type":"assistant","timestamp":"2026-08-05T14:02:20.000Z","message":{"role":"assistant","#,
        r#""content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"src/lens/mod.rs"}}]}}"#,
        "\n",
        r#"{"type":"user","timestamp":"2026-08-05T14:02:21.000Z","message":{"role":"user","#,
        r#""content":[{"type":"tool_result","tool_use_id":"t2","content":"a\nb"}]}}"#,
        "\n"
    )
    .to_string()
}

/// `Tab` is the block jump, so at the tail of a document — past the last thing
/// anyone said — it keeps going through the mechanics rather than dead-ending,
/// and it descends into a run that is open. That is the whole argument for
/// block over message: nothing a document has is out of the jump's reach.
#[test]
fn tab_steps_through_a_trailing_run_that_is_open() {
    let mut src = JsonlSource::from_bytes(run_at_the_tail().into_bytes());
    src.set_lens(crate::lens::find("agent").expect("the agent lens"));
    let mut p = Pager::new(Box::new(src), "session.jsonl".into(), 80, 40, None);
    let _ = p.visible_text();
    key(&mut p, Key::Tab);
    assert!(p.cursor_text().contains("Done."), "the last message: {:?}", p.cursor_text());
    key(&mut p, Key::Tab);
    let run = p.cursor;
    assert!(p.cursor_text().contains('\u{27e8}'), "the trailing run: {:?}", p.cursor_text());
    key(&mut p, Key::Enter);
    key(&mut p, Key::Tab);
    assert_eq!(p.cursor, run + 1, "into the open run rather than nowhere");
    assert!(p.cursor_text().contains("Bash(git status)"), "{:?}", p.cursor_text());
}

/// The invariant under everything above: `j` never moves backwards and never
/// stalls while rows remain, so every row of an open run stays reachable.
#[test]
fn j_is_monotone_and_reaches_the_last_row() {
    let mut p = with_open_run(40);
    press(&mut p, "g");
    assert_eq!(p.cursor, 0, "back to the top");
    let n = p.line_count();
    let mut at = p.cursor;
    for _ in 0..n * 2 {
        press(&mut p, "j");
        assert!(p.cursor > at || p.cursor == n - 1, "stalled at {at} of {n}");
        at = p.cursor;
    }
    assert_eq!(at, n - 1, "the last row is reached");
}
