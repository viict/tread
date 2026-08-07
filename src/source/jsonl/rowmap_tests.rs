//! The row arithmetic on its own: which record a screen row belongs to when
//! some records are expanded. Nothing here reads a file or parses a record.
#![deny(unsafe_code)]

use super::RowMap;

#[test]
fn a_closed_document_is_one_row_per_record() {
    let m = RowMap::default();
    assert_eq!(m.extra_total(), 0);
    for r in 0..5 {
        assert_eq!(m.row_of(r), r);
        assert_eq!(m.at(r), (r, 0));
    }
}

#[test]
fn opening_a_record_splices_its_rows_under_it() {
    let mut m = RowMap::default();
    assert!(m.open(1, 3));
    // Record 1's own row does not move; records after it do.
    assert_eq!(m.row_of(0), 0);
    assert_eq!(m.row_of(1), 1);
    assert_eq!(m.row_of(2), 5);
    assert_eq!(m.extra_total(), 3);
    assert_eq!(m.at(0), (0, 0));
    for sub in 0..=3 {
        assert_eq!(m.at(1 + sub), (1, sub));
    }
    assert_eq!(m.at(5), (2, 0));
    assert_eq!(m.at(6), (3, 0));
}

#[test]
fn several_open_records_stay_in_order_and_close_cleanly() {
    let mut m = RowMap::default();
    assert!(m.open(4, 2));
    assert!(m.open(1, 3));
    assert!(m.open(7, 1));
    assert!(!m.open(4, 9), "already open");
    assert_eq!(m.open_count(), 3);
    assert_eq!(m.records().collect::<Vec<_>>(), vec![1, 4, 7]);
    assert_eq!(m.row_of(1), 1);
    assert_eq!(m.row_of(4), 4 + 3);
    assert_eq!(m.row_of(7), 7 + 3 + 2);
    assert_eq!(m.extra_total(), 6);
    // Every row maps back to the record it came from.
    for record in 0..10 {
        let base = m.row_of(record);
        assert_eq!(m.at(base), (record, 0), "record {record}");
        for sub in 1..=m.extra_of(record) {
            assert_eq!(m.at(base + sub), (record, sub));
        }
    }
    assert!(m.close(4));
    assert!(!m.close(4));
    assert_eq!(m.row_of(7), 7 + 3);
    assert_eq!(m.extra_total(), 4);
    m.clear();
    assert_eq!(m.extra_total(), 0);
}

#[test]
fn a_record_with_nothing_under_it_does_not_open() {
    let mut m = RowMap::default();
    assert!(!m.open(2, 0), "a leaf has no rows to splice");
    assert!(!m.is_open(2));
}
