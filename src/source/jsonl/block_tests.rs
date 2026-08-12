//! Blocks, under a lens: what one is, where it ends, and what `Tab` / `S-Tab`
//! jump between (SPEC.md §Lenses). `j` and `k` are rows here as everywhere.
//!
//! Split from `lensrow_tests.rs`, whose fixture these share, to keep both files
//! under the size limit.
#![deny(unsafe_code)]

use super::lensrow_tests::{lensed, rows, RUN};
use crate::source::Source;

/// A lens is what makes a document read in blocks, and the blocks are exactly
/// the landmarks `Tab` already stepped between — one definition, not two.
#[test]
fn a_lens_makes_a_document_read_in_blocks() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert!(s.blocks());
    // The user prompt fits its summary row; the answer has one body row under
    // it; the folded run and the last two messages are one row each.
    assert_eq!(s.block_at(0), Some(0..1));
    assert_eq!(s.block_at(1), Some(1..3));
    assert_eq!(s.block_at(2), Some(1..3), "a body row is in its message's block");
    assert_eq!(s.block_at(3), Some(3..4));
    assert_eq!(s.block_at(5), Some(5..6));
    // Every block starts where the landmark before it ends.
    for row in 0..s.len() {
        let block = s.block_at(row).expect("a block");
        assert_eq!(s.next_landmark(block.start, true), Some(block.end).filter(|e| *e < s.len()));
    }
}

/// A block's rows include the tree of a record opened inside it, or framing
/// would scroll to the top of a record whose tree is off the bottom.
#[test]
fn an_open_record_grows_the_block_it_is_in() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let before = s.block_at(1).expect("a block");
    let entry = s.section_at(1).expect("an outline entry");
    assert!(s.set_fold(entry, false), "the record opened");
    let _ = rows(&mut s);
    let after = s.block_at(1).expect("a block");
    assert_eq!(after.start, before.start);
    assert!(after.end > before.end, "{before:?} -> {after:?}");
    assert_eq!(s.block_at(after.end - 1), Some(after.clone()), "the last row is still in it");
}

/// `Tab` is the block jump, and a block is every kind of thing the document
/// has: the messages, the folded run of mechanics, and the record no dialect
/// recognised. Nothing is skipped, which is why block and not message.
#[test]
fn tab_steps_between_blocks_and_skips_nothing() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.next_landmark(0, true), Some(1));
    assert_eq!(s.next_landmark(1, true), Some(3), "the run is a block too");
    assert_eq!(s.next_landmark(2, true), Some(3), "and from inside the body too");
    assert_eq!(s.next_landmark(3, true), Some(4));
    assert_eq!(s.next_landmark(4, true), Some(5), "the unrecognised record");
    assert_eq!(s.next_landmark(5, true), None);
    // `S-Tab` is the mirror: back to this block's own row first, then out.
    assert_eq!(s.next_landmark(2, false), Some(1), "back to this message first");
    assert_eq!(s.next_landmark(4, false), Some(3), "then the run");
    assert_eq!(s.next_landmark(0, false), None);
}

/// The status bar counts blocks alongside records, in one vocabulary. The
/// totals are exact here because the whole file has been read and classified.
#[test]
fn the_status_bar_counts_blocks_too() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let text = s.position_text(3).expect("position");
    assert!(text.contains("record 3/8"), "{text}");
    assert!(text.contains("block 3/5"), "{text}");
    let last = s.position_text(5).expect("position");
    assert!(last.contains("block 5/5"), "{last}");
}

/// A boundary **descends into an open run**: the run's own row, then one block
/// per step in it (SPEC.md §Lenses). A shut run is one block — that is what
/// makes it a summary — so opening one is what changes the sequence.
#[test]
fn a_boundary_descends_into_an_open_run() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    // Shut: the run is one block, and the jump steps over it.
    assert_eq!(s.block_at(3), Some(3..4));
    assert_eq!(s.next_landmark(3, true), Some(4));
    let entry = s.section_at(3).expect("the run is an outline entry");
    assert!(s.set_fold(entry, false), "the run opens");
    let _ = rows(&mut s);
    // Open: rows 4..8 are its four steps, each one a block of its own.
    assert_eq!(s.block_at(3), Some(3..4), "the run's own row");
    assert_eq!(s.next_landmark(3, true), Some(4), "then the first step");
    for row in 4..8 {
        assert_eq!(s.block_at(row), Some(row..row + 1), "step {row}");
        assert_eq!(s.next_landmark(row, true), Some(row + 1));
        assert_eq!(s.next_landmark(row, false), Some(row - 1), "and `S-Tab` mirrors it");
    }
    assert_eq!(s.next_landmark(8, false), Some(7), "back into the run from below");
    // And every block still starts where the one before it ends.
    for row in 0..s.len() {
        let block = s.block_at(row).expect("a block");
        assert_eq!(s.next_landmark(block.start, true), Some(block.end).filter(|e| *e < s.len()));
    }
}

/// A member and the tree it has open are one block, exactly as a message and
/// its body are: `Tab` clears the member's rows in one press.
#[test]
fn a_member_with_an_open_tree_is_one_block() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    let run = s.section_at(3).expect("the run");
    assert!(s.set_fold(run, false));
    let _ = rows(&mut s);
    let member = s.section_at(4).expect("the first step of the run");
    assert!(s.set_fold(member, false), "its record opens");
    let _ = rows(&mut s);
    let block = s.block_at(4).expect("the member's block");
    assert_eq!(block.start, 4);
    assert!(block.end > 5, "its tree is inside it: {block:?}");
    assert_eq!(s.block_at(block.end - 1), Some(block.clone()), "the last tree row too");
    assert_eq!(s.next_landmark(4, true), Some(block.end), "one step clears it");
    assert_eq!(s.next_landmark(block.end - 1, false), Some(4), "and one comes back");
}

/// The status bar counts what `Tab` jumps by, so opening a run grows the total
/// rather than leaving the counter saying something the jump disagrees with.
#[test]
fn the_block_count_grows_when_a_run_opens() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert!(s.position_text(3).expect("position").contains("block 3/5"), "shut");
    let entry = s.section_at(3).expect("the run");
    assert!(s.set_fold(entry, false));
    let _ = rows(&mut s);
    let at_run = s.position_text(3).expect("position");
    assert!(at_run.contains("block 3/9"), "{at_run}");
    let first_step = s.position_text(4).expect("position");
    assert!(first_step.contains("block 4/9"), "{first_step}");
    let after = s.position_text(9).expect("position");
    assert!(after.contains("block 9/9"), "{after}");
}

/// `Tab` **does** descend into an open run, because a block boundary does and
/// `Tab` is the block jump: opening a run is the reader asking for what is in
/// it, and a jump that stepped over it would put those steps out of reach of
/// everything but `j`.
#[test]
fn tab_descends_into_an_open_run() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.next_landmark(3, true), Some(4), "shut: over the run");
    let entry = s.section_at(3).expect("the run");
    assert!(s.set_fold(entry, false), "the run opens");
    let _ = rows(&mut s);
    assert_eq!(s.next_landmark(3, true), Some(4), "open: into its first step");
    assert_eq!(s.next_landmark(7, true), Some(8), "and out again at its end");
    assert_eq!(s.next_landmark(8, false), Some(7), "the mirror steps back in");
}
