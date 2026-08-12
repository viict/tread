//! The row arithmetic a lens imposes, on its own: no file, no parsing, no
//! rendering — just items, folds and rows.
//!
//! The two levels have to agree exactly. If `rows()` and `at()` ever disagree
//! by one, every row after the disagreement shows the wrong record, which is
//! the failure this file exists to make impossible.
#![deny(unsafe_code)]

use super::fixture::*;
use super::*;

#[test]
fn a_run_of_steps_becomes_one_item() {
    let (plan, _) = plan_of("msssm");
    let items: Vec<(usize, usize, bool)> = plan
        .items()
        .iter()
        .map(|it| (it.first, it.count, it.is_group()))
        .collect();
    assert_eq!(items, vec![(0, 1, false), (1, 3, true), (4, 1, false)]);
}

/// One step on its own is not a group: `⟨1 step⟩` hides a row and says less
/// than the row it hid.
#[test]
fn a_lone_step_keeps_its_own_row() {
    let (plan, map) = plan_of("msm");
    assert_eq!(plan.items().len(), 3);
    assert!(!plan.items()[1].is_group());
    assert_eq!(walk(&plan, &map, 3), vec!["0.0", "1.0", "2.0"]);
}

/// A record the lens does not recognise is its own item and never joins a run
/// — it is not mechanics, it is unknown, and the two must not be confused.
#[test]
fn an_unrecognised_record_is_never_folded_away() {
    let (plan, map) = plan_of("ss?ss");
    let rows = walk(&plan, &map, 5);
    assert_eq!(rows, vec!["group 0", "2.0", "group 2"]);
    assert_eq!(plan.items().len(), 3);
}

#[test]
fn a_closed_group_is_one_row_and_an_open_one_is_all_of_them() {
    let (mut plan, mut map) = plan_of("mssssm");
    assert_eq!(plan.rows(6, &map), 3);
    assert_eq!(walk(&plan, &map, 6), vec!["0.0", "group 1", "5.0"]);
    assert!(plan.set_open(1, true, &mut map));
    plan.sync();
    assert_eq!(plan.rows(6, &map), 7);
    assert_eq!(
        walk(&plan, &map, 6),
        vec!["0.0", "group 1", "1.0", "2.0", "3.0", "4.0", "5.0"]
    );
    assert_eq!(plan.hidden(1), 0, "an open group hides nothing");
    plan.set_open(1, false, &mut map);
    plan.sync();
    assert_eq!(plan.hidden(1), 4);
}

/// A record's tree rows and a group's member rows compose: the arithmetic has
/// to add both, in the right order.
#[test]
fn tree_rows_and_group_rows_compose() {
    let (mut plan, mut map) = plan_of("mssm");
    // The message at 0 opens into a three-row tree.
    assert!(map.open(0, 3));
    plan.sync();
    assert_eq!(walk(&plan, &map, 4), vec!["0.0", "0.1", "0.2", "0.3", "group 1", "3.0"]);
    // Now open the run, and one of its records too.
    plan.set_open(1, true, &mut map);
    plan.sync();
    assert!(map.open(2, 2));
    assert_eq!(
        walk(&plan, &map, 4),
        vec!["0.0", "0.1", "0.2", "0.3", "group 1", "1.0", "2.0", "2.1", "2.2", "3.0"]
    );
    assert_eq!(plan.row_of_record(2, &map), 6);
    assert_eq!(plan.row_of_record(3, &map), 9);
}

/// Closing a group closes the trees inside it. Otherwise a hidden record would
/// still own rows and every row after the group would be wrong.
#[test]
fn closing_a_group_takes_the_trees_inside_it_with_it() {
    let (mut plan, mut map) = plan_of("mssm");
    plan.set_open(1, true, &mut map);
    plan.sync();
    map.open(1, 4);
    map.open(2, 2);
    // Two messages, the group's own row, its two members, and the six tree
    // rows the two open records add.
    assert_eq!(plan.rows(4, &map), 3 + 2 + 6);
    plan.set_open(1, false, &mut map);
    plan.sync();
    assert_eq!(plan.rows(4, &map), 3, "the trees went with it");
    assert_eq!(map.extra_total(), 0);
}

/// A step that is swallowed into a new group loses its own expansion for the
/// same reason: it stops having a row.
#[test]
fn a_step_swallowed_into_a_group_is_closed() {
    let mut plan = Plan::new(Box::new(Fake));
    let mut map = RowMap::default();
    let step = crate::json::parse(br#"{"k":"s"}"#).expect("fixture");
    plan.classify(0, Some(&step), &mut map);
    plan.sync();
    assert!(map.open(0, 5), "a lone step can be opened");
    plan.classify(1, Some(&step), &mut map);
    plan.sync();
    assert_eq!(map.extra_total(), 0, "it was folded into the group");
    assert_eq!(plan.rows(2, &map), 1);
}

/// The unclassified tail is one row per record, so `len()` is answerable
/// before the lens has read the file — and every row still maps to a record.
#[test]
fn records_past_the_classified_prefix_are_one_row_each() {
    let (plan, map) = plan_of("mss");
    // Two more records exist in the file but have not been read yet.
    assert_eq!(plan.classified(), 3);
    assert_eq!(plan.rows(5, &map), 2 + 2);
    assert_eq!(walk(&plan, &map, 5), vec!["0.0", "group 1", "3.0", "4.0"]);
    assert_eq!(plan.row_of_record(4, &map), 3);
}

#[test]
fn an_empty_plan_answers_without_panicking() {
    let (plan, map) = plan_of("");
    assert_eq!(plan.rows(0, &map), 0);
    assert_eq!(plan.classified(), 0);
    assert_eq!(plan.item_of_record(0), None);
    assert_eq!(plan.item(0), None);
    assert_eq!(plan.hidden(0), 0);
    assert_eq!(plan.row_of_item(0, &map), 0);
    // Rows that cannot exist are answered, never panicked on.
    assert_eq!(plan.at(9, 0, &map), Spot::Record { record: 0, sub: 0 });
}

/// Out-of-order or repeated classification is ignored rather than corrupting
/// the item list: the source drives this in order, and a bug elsewhere must
/// not become a wrong row here.
#[test]
fn classification_out_of_order_is_ignored() {
    let mut plan = Plan::new(Box::new(Fake));
    let mut map = RowMap::default();
    let m = crate::json::parse(br#"{"k":"m"}"#).expect("fixture");
    plan.classify(3, Some(&m), &mut map);
    assert_eq!(plan.classified(), 0);
    plan.classify(0, Some(&m), &mut map);
    plan.classify(0, Some(&m), &mut map);
    assert_eq!(plan.classified(), 1);
}

/// `zR` opens as far as the reader has got and no further: opening every group
/// of a million-record log is the one thing this source must never do.
#[test]
fn opening_everything_stops_at_the_viewport() {
    let (mut plan, mut map) = plan_of("ssmssmssmss");
    plan.open_upto(2, &mut map);
    let open = plan.items().iter().filter(|it| it.is_group() && it.open).count();
    assert!((1..4).contains(&open), "opened {open} of the runs");
    plan.open_upto(usize::MAX, &mut map);
    assert_eq!(
        plan.items().iter().filter(|it| it.is_group() && it.open).count(),
        4
    );
    plan.close_all(&mut map);
    plan.sync();
    assert_eq!(plan.rows(11, &map), 7);
}

#[test]
fn group_ids_cannot_collide_with_record_ids() {
    assert_eq!(group_first(&group_id(12)), Some(12));
    assert_eq!(group_first("/12"), None, "a record id is not a group id");
    assert_eq!(crate::source::jsonrow::top_index(&group_id(12)), None);
    assert_eq!(group_first("g"), None);
    assert_eq!(group_first("gx"), None);
}

/// The one that would ruin a long log: after every combination of folds, the
/// row count and the row lookup still agree, and every record is reachable.
#[test]
fn rows_and_lookups_agree_under_every_fold_combination() {
    let kinds = "msssmssm?ssm";
    let known = kinds.chars().count();
    for mask in 0..16u32 {
        let (mut plan, mut map) = plan_of(kinds);
        for (n, item) in (0..plan.items().len()).enumerate() {
            if mask & (1 << (n % 4)) != 0 {
                plan.set_open(item, true, &mut map);
            }
        }
        plan.sync();
        // Every visible record's row maps back to that record.
        for record in 0..known {
            let row = plan.row_of_record(record, &map);
            assert!(row < plan.rows(known, &map), "record {record} row {row}");
            match plan.at(row, known, &map) {
                Spot::Record { record: got, sub } => {
                    assert_eq!((got, sub), (record, 0), "mask {mask}");
                }
                Spot::Body { record: got, line } | Spot::Part { record: got, line } => {
                    panic!("record {record} landed on row {line} under {got}");
                }
                // A folded record's row is its group's row.
                Spot::Group { item } => {
                    let it = plan.item(item).expect("item");
                    assert!(record >= it.first && record < it.first + it.count);
                }
            }
        }
        // And every row maps to something.
        assert_eq!(walk(&plan, &map, known).len(), plan.rows(known, &map));
    }
}

/// A record that joins a group *while the group is open* keeps its own tree.
///
/// Classification runs ahead of the painted window, but the dump path and `zR`
/// open records ahead of it too, so a record can be expanded before the lens
/// reaches it. Closing it on the way into an already-open run threw that
/// expansion away for good — `tread --lens agent log.jsonl > out.txt` emitted
/// the body of only the first 65 records of a 300-record run.
#[test]
fn joining_an_open_group_does_not_close_the_records_tree() {
    let mut plan = Plan::new(Box::new(Fake));
    let mut map = RowMap::default();
    let s = crate::json::parse(br#"{"k":"s"}"#).expect("fixture");
    // Two steps make a group, which the reader then opens.
    plan.classify(0, Some(&s), &mut map);
    plan.classify(1, Some(&s), &mut map);
    plan.sync();
    assert!(plan.set_open(0, true, &mut map));
    plan.sync();
    // Records the viewport has already expanded, ahead of classification.
    for r in 0..4 {
        assert!(map.open(r, 3), "record {r} should expand");
    }
    for r in 2..4 {
        plan.classify(r, Some(&s), &mut map);
    }
    plan.sync();
    for r in 0..4 {
        assert!(map.is_open(r), "record {r} lost its tree on joining the run");
    }
    // And the arithmetic still agrees: one group row, then four records each
    // carrying three tree rows.
    assert_eq!(plan.rows(4, &map), 1 + 4 * 4);
}

/// The same records joining a *closed* group are closed, which is what keeps
/// a hidden record from owning rows nothing can reach.
#[test]
fn joining_a_closed_group_still_closes_the_records_tree() {
    let mut plan = Plan::new(Box::new(Fake));
    let mut map = RowMap::default();
    let s = crate::json::parse(br#"{"k":"s"}"#).expect("fixture");
    plan.classify(0, Some(&s), &mut map);
    assert!(map.open(0, 3));
    plan.classify(1, Some(&s), &mut map);
    assert!(!map.is_open(0), "the first record of a new group is closed with it");
    assert!(map.open(2, 3));
    plan.classify(2, Some(&s), &mut map);
    assert!(!map.is_open(2), "a record swallowed by a shut group is closed");
    plan.sync();
    assert_eq!(plan.rows(3, &map), 1, "a shut group is one row");
}


/// The sharpest edge in the file: an item's own rows now depend on the width,
/// so every fold combination is re-checked with bodies of 0, 1, 6 and 200 rows.
/// Every record's row maps back to that record, every row maps to *something*,
/// and the two totals agree.
#[test]
fn rows_and_records_round_trip_with_a_body_at_every_height() {
    let kinds = "msssm?mssm";
    // Two records the lens has not reached: the unclassified tail must keep
    // being one row each with a body present.
    let known = kinds.chars().count() + 2;
    for height in [0usize, 1, 6, 200] {
        for mask in 0..8u32 {
            let (mut plan, mut map) = plan_of(kinds);
            for item in 0..plan.items().len() {
                if mask & (1 << (item % 3)) != 0 {
                    plan.set_open(item, true, &mut map);
                }
            }
            // A record opened into its tree as well, under the body.
            map.open(0, 3);
            with_bodies(&mut plan, height);
            let total = plan.rows(known, &map);
            for record in 0..known {
                let row = plan.row_of_record(record, &map);
                assert!(row < total, "record {record} at {row} of {total}");
                match plan.at(row, known, &map) {
                    Spot::Record { record: got, sub } => {
                        assert_eq!((got, sub), (record, 0), "height {height} mask {mask}");
                    }
                    Spot::Body { record: got, line } | Spot::Part { record: got, line } => {
                        panic!("record {record} landed on row {line} under {got}");
                    }
                    Spot::Group { item } => {
                        let it = plan.item(item).expect("item");
                        assert!(record >= it.first && record < it.first + it.count);
                    }
                }
            }
            assert_eq!(walk(&plan, &map, known).len(), total, "height {height} mask {mask}");
        }
    }
}

/// The order inside one item: the summary row, then the message, then the
/// record's own tree. Getting this wrong reads a body row as a tree row.
#[test]
fn a_body_sits_between_the_summary_row_and_the_tree() {
    let (mut plan, mut map) = plan_of("mm");
    map.open(0, 2);
    with_bodies(&mut plan, 3);
    assert_eq!(
        walk(&plan, &map, 2),
        vec!["0.0", "0b0", "0b1", "0b2", "0.1", "0.2", "1.0", "1b0", "1b1", "1b2"]
    );
    assert_eq!(plan.row_of_record(1, &map), 6);
}

/// A step never has a body, so a run is exactly as tall as the steps in it —
/// which is what leaves the open-group arithmetic alone.
#[test]
fn an_open_run_is_untouched_by_bodies() {
    let (mut plan, mut map) = plan_of("mssssm");
    with_bodies(&mut plan, 4);
    plan.set_open(1, true, &mut map);
    plan.sync();
    assert_eq!(
        walk(&plan, &map, 6),
        vec![
            "0.0", "0b0", "0b1", "0b2", "0b3", "group 1", "1.0", "2.0", "3.0", "4.0", "5.0",
            "5b0", "5b1", "5b2", "5b3"
        ]
    );
}

/// A width change is the one thing that invalidates a height. The plan says so
/// and rebuilds its totals from the new measurements — without reclassifying,
/// which would read every record a second time.
#[test]
fn a_width_change_re_lays_every_body() {
    let (mut plan, map) = plan_of("mm");
    with_bodies(&mut plan, 2);
    assert_eq!(plan.rows(2, &map), 6);
    let classified = plan.classified();
    assert!(plan.set_width(40), "a new width changed something");
    assert!(!plan.set_width(40), "the same width changes nothing");
    // The heights the caller re-measures at the new width.
    with_bodies(&mut plan, 5);
    assert_eq!(plan.rows(2, &map), 12);
    assert_eq!(plan.classified(), classified, "nothing was reclassified");
    assert_eq!(plan.width(), 40);
}

