//! The numeric block: its width, its alignment, and the three different things
//! one cell can say.
//!
//! Every fixture here is hand-written and synthetic.
#![deny(unsafe_code)]

use super::*;
use crate::render::str_width;

fn four(input: u64, output: u64, read: u64, new: u64) -> Tokens {
    Tokens {
        input: Some(input),
        output: Some(output),
        cache_read: Some(read),
        cache_new: Some(new),
    }
}

/// The block is a constant width whatever the numbers are — that is the whole
/// product, and a field that grew by one column would bend the column below it.
#[test]
fn the_numeric_block_is_the_same_width_for_every_record() {
    let wide = four(1_234_567, 999_999, 18_999, 2_100);
    let narrow = four(0, 0, 0, 0);
    let block = |t: &Tokens| row_text(t, &Field::ALL, "");
    assert_eq!(str_width(&block(&wide)), 4 * FIELD + 3 * 2);
    assert_eq!(str_width(&block(&wide)), 38, "four fields are 38 columns");
    assert_eq!(str_width(&block(&narrow)), 38);
    // And three fields, for a format with no cache-creation counter.
    let three = row_text(&wide, &[Field::In, Field::Out, Field::Read], "");
    assert_eq!(str_width(&three), 3 * FIELD + 2 * 2);
    assert_eq!(str_width(&three), 28, "three fields are 28 columns");
}

/// Labels left, numbers right, so the digits line up under each other.
#[test]
fn a_field_is_its_label_then_its_number_right_aligned() {
    let t = four(1_200, 380, 18_000, 2_100);
    let row = row_text(&t, &Field::ALL, "Bash(cargo test)");
    assert_eq!(row, "in  1.2k  out  380  read 18k  new 2.1k  \u{b7}  Bash(cargo test)");
    // The action begins exactly one separator past the block.
    assert!(row.starts_with("in  1.2k"));
    assert_eq!(&row[..38], "in  1.2k  out  380  read 18k  new 2.1k");
}

/// A recorded zero is a fact about the session and is printed as one. Fourteen
/// records in the corpus this was measured against record exactly that.
#[test]
fn a_recorded_zero_is_shown_as_zero() {
    let t = four(0, 0, 0, 0);
    let row = row_text(&t, &Field::ALL, "");
    assert!(row.contains("in     0"), "{row}");
    assert!(!row.contains('-'), "a zero is not an absence: {row}");
}

/// A field this record did not record, inside a format that has it: `-`, never
/// `0`. The two are different claims and keeping them apart is why every
/// counter is an `Option`.
#[test]
fn a_field_this_record_did_not_record_is_a_dash() {
    let t = Tokens { input: Some(500), output: None, cache_read: None, cache_new: Some(0) };
    let row = row_text(&t, &Field::ALL, "");
    assert_eq!(row, "in   500  out    -  read   -  new    0");
    assert_eq!(str_width(&row), 38, "an absence costs no alignment");
}

/// A field the *format* does not have never reaches a row at all — no dash, no
/// zero, no column. The counter may even be set; the field list is the answer.
#[test]
fn a_field_the_format_does_not_have_has_no_column() {
    let t = four(500, 100, 20, 7);
    let row = row_text(&t, &[Field::In, Field::Out, Field::Read], "bash");
    assert!(!row.contains("new"), "no cache-creation column: {row}");
    assert!(row.contains("read"), "{row}");
}

/// The total is the four counters, added once each.
#[test]
fn a_total_adds_the_four_counters_once() {
    assert_eq!(four(1, 2, 4, 8).total(), 15);
    assert_eq!(Tokens::default().total(), 0);
    let partial = Tokens { input: Some(10), output: None, cache_read: Some(5), cache_new: None };
    assert_eq!(partial.total(), 15, "an unrecorded field adds nothing");
    // Saturating, so a corrupt file cannot panic a release build either.
    let huge = four(u64::MAX, u64::MAX, 0, 0);
    assert_eq!(huge.total(), u64::MAX);
}

/// A record that recorded nothing gets no columns at all.
#[test]
fn a_record_with_no_counters_records_nothing() {
    assert!(!Tokens::default().any());
    assert!(four(0, 0, 0, 0).any(), "a recorded zero is a recording");
}

#[test]
fn adjacent_repeats_collapse() {
    let items = vec!["Read".to_string(), "Read".to_string(), "Read".to_string()];
    assert_eq!(collapse(items), vec!["Read \u{d7}3".to_string()]);
    let mixed = vec!["Read".to_string(), "Bash".to_string(), "Read".to_string()];
    assert_eq!(collapse(mixed.clone()), mixed);
}
