//! What the fold keys do to a plan: `zR`, `zM`, a fold id off the outline,
//! `Tab`, `Y` on a group's row, and revealing a search hit.
//!
//! # Why these are here and not on the format
//!
//! Every function below touches a [`Plan`] and a [`RowMap`] and nothing else —
//! no file, no parser, no `.jsonl`. They were methods on the record source once,
//! which meant a second record format would have implemented [`Records`],
//! watched `zR`, `zM`, `Tab` and fold-state restore do nothing, and copied them
//! across; two copies of the group arithmetic is exactly the drift the seam
//! exists to prevent.
//!
//! # The `Option<&mut Plan>` in every signature
//!
//! A document with no `--lens` has no plan, and "no lens" is the answer these
//! give: nothing to open, nothing to close, no group ids in the fold state. The
//! `None` arm lives here rather than at each call site so a format cannot get it
//! wrong in one place out of six.
//!
//! # The fold-id vocabulary
//!
//! [`id_at`] is the one place a group's `g4` and a record's `/4` meet
//! ([`super::fold_id`], [`plan::group_id`]), and [`set_by_id`] is the one place
//! an id off the outline is told apart: `None` is "this names no group", which
//! the format then reads as one of its own records — the only half of folding
//! that needs the format at all, because opening a record costs a parse.
#![deny(unsafe_code)]

use crate::select::Yank;

use super::plan::{self, Plan, Spot};
use super::rowmap::RowMap;
use super::{fold_id, lensrow, Records};

/// `zR`: open the groups the viewport has reached, the same bounded way a
/// format opens records. Opening *every* group of a million-record log would
/// mean classifying all of it, which is the one thing a lens must never do.
pub(crate) fn open_upto(plan: Option<&mut Plan>, map: &mut RowMap, upto_row: usize) {
    if let Some(plan) = plan {
        plan.open_upto(upto_row, map);
    }
}

/// `zM`: shut every group, which is free — nothing is parsed to close one.
pub(crate) fn close_all(plan: Option<&mut Plan>, map: &mut RowMap) {
    if let Some(plan) = plan {
        plan.close_all(map);
        plan.sync();
    }
}

/// Open the group holding `record`, so a search hit is never left behind a
/// fold (SPEC.md §Lenses: a lens may fold, but it may never lose a record).
/// Does nothing when there is no lens, or the record is not in a group.
pub(crate) fn reveal(plan: Option<&mut Plan>, map: &mut RowMap, record: usize) {
    let Some(plan) = plan else {
        return;
    };
    if let Some(item) = plan.item_of_record(record) {
        plan.set_open(item, true, map);
        plan.sync();
    }
}

/// Open or close the group whose first record is `first`.
pub(crate) fn set_group(plan: Option<&mut Plan>, map: &mut RowMap, first: usize, open: bool) -> bool {
    let Some(plan) = plan else {
        return false;
    };
    let changed = match plan.item_of_record(first) {
        Some(item) => plan.set_open(item, open, map),
        None => false,
    };
    plan.sync();
    changed
}

/// The fold id of whatever sits on `at`: a group's `g4`, or a record's `/4`.
pub(crate) fn id_at(plan: Option<&Plan>, at: Spot) -> String {
    match at {
        Spot::Group { item } => plan::group_id(lensrow::item_first(plan, item)),
        // A body row answers with its record's id, so `za` on the message and
        // `za` on the row above it are the same fold.
        Spot::Record { record, .. } | Spot::Body { record, .. } => fold_id(record),
    }
}

/// Apply a fold id that may name a group, returning whether anything changed.
///
/// `None` is an id that names no group — the format reads it as one of its own
/// records, which is the half of folding it has to own because opening a record
/// costs a parse.
pub(crate) fn set_by_id(plan: Option<&mut Plan>, map: &mut RowMap, id: &str, open: bool) -> Option<bool> {
    let first = plan::group_first(id)?;
    Some(set_group(plan, map, first, open))
}

/// The ids of the groups that are open: the exceptions a `FoldState` carries,
/// since a group starts shut.
pub(crate) fn open_ids(plan: Option<&Plan>) -> Vec<String> {
    let Some(plan) = plan else {
        return Vec::new();
    };
    let open = plan.items().iter().filter(|it| it.is_group() && it.open);
    open.map(|it| plan::group_id(it.first)).collect()
}

/// Restore group folds from a `FoldState`: everything shut, then the ids that
/// disagree opened. Ids naming no group are left for the format.
pub(crate) fn restore(plan: Option<&mut Plan>, map: &mut RowMap, folds: &[String]) {
    let Some(plan) = plan else {
        return;
    };
    close_all(Some(&mut *plan), map);
    for id in folds {
        if let Some(first) = plan::group_first(id) {
            set_group(Some(&mut *plan), map, first, true);
        }
    }
}

/// `j` / `k` under a lens, and what `Tab` falls back to: the next block
/// boundary — a message, a shut run, or, inside a run the reader has opened,
/// one of the steps in it. [`Plan::next_block`] is the crate's single
/// definition of where a block starts, and `Plan::block_at` is the same table
/// read for an extent.
pub(crate) fn next_block(plan: Option<&Plan>, map: &RowMap, row: usize, forward: bool) -> Option<usize> {
    plan?.next_block(row, map, forward)
}

/// The rows of the block `row` falls in — what the pager frames when `j`/`k`
/// land on one. `None` with no lens (there are no blocks) and `None` past the
/// classified prefix.
pub(crate) fn block_at(plan: Option<&Plan>, map: &RowMap, row: usize) -> Option<std::ops::Range<usize>> {
    plan?.block_at(row, map)
}

/// `(index, count)` of the block `row` is on, for the status bar.
pub(crate) fn block_of_row(plan: Option<&Plan>, map: &RowMap, row: usize) -> Option<(usize, usize)> {
    plan?.block_of_row(row, map)
}

/// `Tab` / `S-Tab` under a lens: the next **message** — the conversation turn —
/// now that `j`/`k` step between blocks and a block is as often a folded run of
/// mechanics as it is something someone said.
///
/// The test is [`super::plan::Item::step`], so an *unrecognised* record is a
/// message here: it is not mechanics, the lens said nothing about it, and it is
/// exactly the thing SPEC.md §Lenses promises is never lost. It is also what
/// makes `Tab` keep moving through a file whose dialect nothing recognises,
/// where every block is one of these.
///
/// **`Tab` does not descend into an open run, and that is deliberate.** `j`
/// descends because opening a run means "show me what is in here"; `Tab` is the
/// conversation turn, and every member of a run is by construction a step — so
/// descending would only make `Tab` stop on mechanics, which is the one thing it
/// exists not to do. Opening a run therefore changes what `j` steps by and
/// leaves `Tab` exactly where it was.
///
/// `None` when there is no further message; the caller then falls back to the
/// next block rather than dead-ending on a trailing run of mechanics — and
/// *that* fallback does descend, since it is the block boundary.
pub(crate) fn next_message(
    plan: Option<&Plan>,
    map: &RowMap,
    known: usize,
    row: usize,
    forward: bool,
) -> Option<usize> {
    let plan = plan?;
    let record = lensrow::record_at(Some(plan), map, known, row);
    let cur = plan.item_of_record(record)?;
    let items = plan.items();
    let at = |i: usize| plan.row_of_item(i, map);
    match forward {
        true => (cur + 1..items.len()).find(|&i| !items[i].step).map(at),
        // Inside a message, the first press goes back to its own row — the same
        // rule [`Plan::next_block`] follows, so `Tab` and `S-Tab` agree about
        // where a thing starts. The two answers part only inside an open run,
        // where a block starts on the member's row and a message on the run's.
        false if row > at(cur) && !items[cur].step => Some(at(cur)),
        false => (0..cur).rev().find(|&i| !items[i].step).map(at),
    }
}

/// `Y` on a group's row: every record the run holds, one JSON document per
/// line — the folded rows as data, so what was copied is what was hidden.
pub(crate) fn yank_group<R: Records>(src: &R, plan: Option<&Plan>, item: usize) -> Option<Yank> {
    let it = plan?.item(item)?;
    let (first, count) = (it.first, it.count);
    let mut out = String::new();
    for record in first..first + count {
        if let Some(text) = src.with_value(record, |v| v.map(|v| v.to_json())) {
            out.push_str(&text);
            out.push('\n');
        }
    }
    (!out.is_empty()).then(|| Yank { text: out, what: format!("{count} records") })
}
