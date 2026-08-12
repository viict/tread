//! The **levels** a record has, and the rows they add: the parts between a body
//! and the tree, and the members of an open run that now show rows of their own.
//!
//! The other half of `plan_tests.rs`, split to stay under the size limit and
//! sharing its double ([`super::fixture`]). What both halves hold to is the one
//! invariant: if `rows()` and `at()` disagree by one, every row after the
//! disagreement shows the wrong record.
#![deny(unsafe_code)]

use super::fixture::*;
use super::*;

// -- the third kind of row, and the members that now have rows of their own -----

/// The order inside one record: its summary row, what it said, its parts, then
/// its own tree. Getting this wrong reads a call row as a tree row.
#[test]
fn parts_sit_between_the_body_and_the_tree() {
    let (mut plan, mut map) = plan_of("mm");
    map.open(0, 2);
    plan.set_under(0, Under { body: 2, parts: 3 }, false);
    plan.set_under(1, Under { body: 1, parts: 0 }, false);
    plan.sync();
    assert_eq!(
        walk(&plan, &map, 2),
        vec!["0.0", "0b0", "0b1", "0p0", "0p1", "0p2", "0.1", "0.2", "1.0", "1b0"]
    );
    assert_eq!(plan.row_of_record(1, &map), 8);
}

/// Reasoning is text, so it is shown: a step inside a run the reader has opened
/// has rows of its own, and the run is exactly as tall as what it holds.
///
/// This is the invariant that used to read "a step never has a body". It was
/// traded deliberately: `own` for an open group is still `1 + count`, and a
/// member's own rows are the second prefix sum's — the same treatment its tree
/// rows have always had.
#[test]
fn an_open_run_is_as_tall_as_the_steps_and_what_they_show() {
    let (mut plan, mut map) = plan_of("mssm");
    plan.set_open(1, true, &mut map);
    with_under(&mut plan, Under { body: 2, parts: 1 });
    assert_eq!(
        walk(&plan, &map, 4),
        vec![
            "0.0", "0b0", "0b1", "0p0", "group 1", "1.0", "1b0", "1b1", "1p0", "2.0", "2b0",
            "2b1", "2p0", "3.0", "3b0", "3b1", "3p0"
        ]
    );
    // And every member's row still maps back to that member.
    for record in 0..4 {
        let row = plan.row_of_record(record, &map);
        assert_eq!(plan.at(row, 4, &map), Spot::Record { record, sub: 0 }, "record {record}");
    }
}

/// A member's tree opens *under* what it showed, not in place of it.
#[test]
fn a_members_own_rows_and_its_tree_compose() {
    let (mut plan, mut map) = plan_of("ssm");
    plan.set_open(0, true, &mut map);
    with_under(&mut plan, Under { body: 1, parts: 2 });
    assert!(map.open(1, 2));
    plan.sync();
    assert_eq!(
        walk(&plan, &map, 3),
        vec!["group 0", "0.0", "0b0", "0p0", "0p1", "1.0", "1b0", "1p0", "1p1", "1.1", "1.2", "2.0", "2b0", "2p0", "2p1"]
    );
    assert_eq!(plan.row_of_record(1, &map), 5);
    assert_eq!(plan.row_of_record(2, &map), 11);
}

/// Closing a run takes its members' own rows with it, exactly as it takes their
/// trees — a hidden record owns no rows, which is what both prefix sums assume.
#[test]
fn closing_a_run_takes_the_rows_its_members_showed() {
    let (mut plan, mut map) = plan_of("mssm");
    plan.set_open(1, true, &mut map);
    with_under(&mut plan, Under { body: 3, parts: 2 });
    let open = plan.rows(4, &map);
    plan.set_open(1, false, &mut map);
    plan.sync();
    let shut = plan.rows(4, &map);
    assert_eq!(shut, open - 2 * (1 + 3 + 2), "both members and everything they showed");
    assert_eq!(walk(&plan, &map, 4).len(), shut);
}

/// The whole thing at once: every fold combination, every level combination,
/// at three widths' worth of heights — `at(row)` inverts `row_of_record` and
/// the two totals agree, which is the failure this file exists to prevent.
#[test]
fn rows_and_records_round_trip_at_every_level() {
    let kinds = "mssm?msm";
    let known = kinds.chars().count() + 1;
    // Heights a body and its parts take at 40, 92 and 200 columns, plus the two
    // ends: nothing under the row at all, and a level with only parts.
    for under in [
        Under { body: 0, parts: 0 },
        Under { body: 6, parts: 0 },
        Under { body: 0, parts: 4 },
        Under { body: 12, parts: 9 },
        Under { body: 1, parts: 1 },
    ] {
        for mask in 0..8u32 {
            let (mut plan, mut map) = plan_of(kinds);
            for item in 0..plan.items().len() {
                if mask & (1 << (item % 3)) != 0 {
                    plan.set_open(item, true, &mut map);
                }
            }
            with_under(&mut plan, under);
            map.open(0, 3);
            plan.sync();
            let total = plan.rows(known, &map);
            for record in 0..known {
                let row = plan.row_of_record(record, &map);
                assert!(row < total, "record {record} at {row} of {total} ({under:?})");
                match plan.at(row, known, &map) {
                    Spot::Record { record: got, sub } => {
                        assert_eq!((got, sub), (record, 0), "{under:?} mask {mask}");
                    }
                    Spot::Group { item } => {
                        let it = plan.item(item).expect("item");
                        assert!(record >= it.first && record < it.first + it.count);
                    }
                    other => panic!("record {record} landed on {other:?} ({under:?})"),
                }
            }
            assert_eq!(walk(&plan, &map, known).len(), total, "{under:?} mask {mask}");
        }
    }
}

/// The level is an *exception* to the default, per record — which is what lets
/// `zR` mean "everything, now" and one record clip again afterwards.
#[test]
fn a_level_is_an_exception_to_the_default() {
    let (mut plan, _) = plan_of("mm");
    assert!(!plan.full_at(0));
    assert!(plan.set_full(0, true));
    assert!(plan.full_at(0) && !plan.full_at(1));
    plan.set_all_full(true);
    assert!(plan.full_at(0) && plan.full_at(1), "everything, now");
    assert!(plan.set_full(0, false), "and one of them can clip again");
    assert!(!plan.full_at(0) && plan.full_at(1));
    plan.set_all_full(false);
    assert!(!plan.full_at(0) && !plan.full_at(1), "exceptions are dropped either way");
}

/// An opened call is an exception too, and leaving the open level drops it: the
/// rows are gone, and a call left open would reappear on the next descent.
#[test]
fn an_opened_call_belongs_to_the_level_it_was_opened_at() {
    let (mut plan, _) = plan_of("mm");
    plan.set_full(0, true);
    assert!(plan.set_part_open(0, 1, true));
    assert!(plan.part_open(0, 1) && !plan.part_open(0, 0) && !plan.part_open(1, 1));
    assert!(!plan.set_part_open(0, 1, true), "twice is not a change");
    plan.set_full(0, false);
    assert!(!plan.part_open(0, 1), "the level took it with it");
    plan.set_full(0, true);
    plan.set_part_open(0, 0, true);
    plan.set_all_full(false);
    assert!(!plan.part_open(0, 0), "and so does `zM`");
}


/// **One call opens at a time** (SPEC.md §Lenses). Opening a second one shuts
/// the first, wherever it was: two open calls put two screens of arguments and
/// output on the screen together, which is what the level exists to avoid.
#[test]
fn opening_a_call_shuts_whichever_was_open() {
    let (mut plan, _) = plan_of("mm");
    plan.set_full(0, true);
    plan.set_full(1, true);
    assert!(plan.set_part_open(0, 0, true));
    assert_eq!(plan.open_part_record(), Some(0));
    // A second call on the same record.
    assert!(plan.set_part_open(0, 2, true));
    assert!(plan.part_open(0, 2) && !plan.part_open(0, 0), "not both at once");
    // And one on another record, which is how the caller knows to re-measure it.
    assert_eq!(plan.open_part_record(), Some(0));
    assert!(plan.set_part_open(1, 1, true));
    assert!(plan.part_open(1, 1) && !plan.part_open(0, 2));
    assert_eq!(plan.open_part_record(), Some(1));
    assert!(plan.set_part_open(1, 1, false));
    assert_eq!(plan.open_part_record(), None);
}

// -- a block inside an open run ---------------------------------------------------

/// A member's block is that member and everything under its row — its body, its
/// parts, its tree — never the whole run (docs/lenses.md).
///
/// Attributing those rows to the run's own block made the jump move the cursor
/// *backwards* out of them, `k` skip the member's row, and the status bar count
/// down inside an open run. Before members had rows of their own the case could
/// not arise, so the arm that got it wrong was unreachable.
#[test]
fn a_members_under_rows_belong_to_the_members_block() {
    let (mut plan, mut map) = plan_of("mssm");
    plan.set_open(1, true, &mut map);
    with_under(&mut plan, Under { body: 2, parts: 1 });
    map.open(2, 2);
    plan.sync();
    // 0.0 0b0 0b1 0p0 | group | 1.0 1b0 1b1 1p0 | 2.0 2b0 2b1 2p0 2.1 2.2 | 3.0 …
    let rows = walk(&plan, &map, 4);
    let block_of: Vec<usize> = (0..rows.len())
        .map(|r| plan.block_index_at(r, &map).expect("in the prefix"))
        .collect();
    assert_eq!(
        block_of,
        vec![0, 0, 0, 0, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4],
        "{rows:?}"
    );
    // The jump never goes backwards, and back from anywhere inside a member's
    // block comes back to that member's own row.
    for (r, row) in rows.iter().enumerate() {
        if let Some(next) = plan.next_block(r, &map, true) {
            assert!(next > r, "Tab from row {r} ({row}) went to {next}");
        }
        if let Some(back) = plan.next_block(r, &map, false) {
            assert!(back < r, "S-Tab from row {r} ({row}) went to {back}");
        }
    }
    // The member's own extent is the member, not the run.
    let member = plan.row_of_record(2, &map);
    assert_eq!(plan.block_at(member, &map), Some(member..member + 6));
    let back = plan.next_block(member + 3, &map, false);
    assert_eq!(back, Some(member), "S-Tab comes back to the member's own row");
    assert_eq!(plan.blocks(), 5, "the run's own row, then one block per step");
}
