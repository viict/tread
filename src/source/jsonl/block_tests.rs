//! Blocks, under a lens: what one is, where it ends, and what `Tab` means once
//! `j` is stepping between them (SPEC.md §Lenses).
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

/// `Tab` is the conversation turn: it skips the folded run of mechanics, and
/// it counts the record no dialect recognised as something someone said.
#[test]
fn tab_steps_between_messages_and_j_between_blocks() {
    let mut s = lensed(RUN);
    let _ = rows(&mut s);
    assert_eq!(s.next_message(0, true), Some(1));
    assert_eq!(s.next_message(1, true), Some(4), "the run is not a message");
    assert_eq!(s.next_message(2, true), Some(4), "and from inside the body too");
    assert_eq!(s.next_message(4, true), Some(5), "an unrecognised record is one");
    assert_eq!(s.next_message(5, true), None);
    assert_eq!(s.next_message(2, false), Some(1), "back to this message first");
    assert_eq!(s.next_message(4, false), Some(1), "then over the run");
    assert_eq!(s.next_message(0, false), None);
    // And `j` still stops on the run, which is what makes it openable.
    assert_eq!(s.next_landmark(1, true), Some(3));
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
