//! Where this format meets the record seam: the [`Records`] impl, and the thin
//! forwarders the rest of the source calls.
//!
//! Nothing about lenses is decided here. [`crate::source::record`] owns the
//! plan, the row arithmetic, group folding and every painted row; this file only
//! says how to reach record `i` of a `.jsonl` file, and holds the lens state on
//! the source because that is where the fold state already lives. Every method
//! below is one line long on purpose: the moment one of them grows a decision,
//! the next record format will need the same decision and will not get it.
#![deny(unsafe_code)]

use super::*;

/// A `.jsonl` file is a sequence of records: one per line of the CSV lazy line
/// index, parsed when it is shown and not before.
impl Records for JsonlSource {
    fn known(&self) -> usize {
        JsonlSource::known(self)
    }

    /// Straight through the parsed-record cache, so the seam pays nothing extra
    /// for asking: a record the frame is about to paint is already in hand.
    fn with_value<T>(&self, record: usize, f: impl FnOnce(Option<&Value>) -> T) -> T {
        self.with_record(record, |rec| f(rec.value()))
    }

    fn foldable(&self, record: usize) -> bool {
        self.tree_len(record) > 0
    }
}

impl JsonlSource {
    /// Read this file through `lens` (SPEC.md §Lenses).
    ///
    /// A transform over the records, not a different source: the file, the
    /// index, the parser and every row below a summary are unchanged. What the
    /// lens decides is what each record's *one* row says and which runs of
    /// records share one.
    pub fn set_lens(&mut self, lens: Box<dyn Lens>) {
        self.plan = Some(Plan::new(lens));
    }

    /// The lens in force, for the status bar.
    pub fn lens_name(&self) -> Option<&'static str> {
        self.plan.as_ref().map(|p| p.lens_name())
    }

    /// Classify records up to and including `upto`.
    pub(super) fn classify_to(&mut self, upto: usize) {
        // Both are moved out for the call: reading a record borrows `self`
        // immutably through the `RefCell`s, and the classification writes to
        // these two. Put back before returning, always.
        let Some(mut plan) = self.plan.take() else {
            return;
        };
        let mut map = std::mem::take(&mut self.map);
        lensrow::classify_to(&*self, &mut plan, &mut map, upto);
        self.plan = Some(plan);
        self.map = map;
    }

    /// Where a screen row falls.
    pub(crate) fn spot(&self, row: usize) -> Spot {
        lensrow::spot(self.plan.as_ref(), &self.map, self.known(), row)
    }

    /// The row a record's summary sits on.
    pub(crate) fn row_of_record(&self, record: usize) -> usize {
        lensrow::row_of_record(self.plan.as_ref(), &self.map, record)
    }

    /// Is `record` currently a row of its own?
    pub(super) fn record_visible(&self, record: usize) -> bool {
        lensrow::record_visible(self.plan.as_ref(), record)
    }

    /// A record's row as the lens reads it, or `None` when the lens did not
    /// recognise it — the caller then renders the generic tree row.
    pub(super) fn lens_row(&self, record: usize, inset: bool) -> Option<Line> {
        lensrow::lens_row(self, self.plan.as_ref(), record, inset)
    }

    /// A folded run of mechanics: `⟨6 steps · 4 tool calls⟩`.
    pub(super) fn group_row(&self, item: usize) -> Line {
        lensrow::group_row(self.plan.as_ref(), item)
    }

    /// The record a row belongs to.
    pub(crate) fn record_at(&self, row: usize) -> usize {
        lensrow::record_at(self.plan.as_ref(), &self.map, self.known(), row)
    }

    /// The first record of a plan item.
    pub(crate) fn item_first(&self, item: usize) -> usize {
        lensrow::item_first(self.plan.as_ref(), item)
    }

    /// Is this record inside a group that is open? Its row is inset if so.
    pub(super) fn in_open_group(&self, record: usize) -> bool {
        lensrow::in_open_group(self.plan.as_ref(), record)
    }

    // -- folding ---------------------------------------------------------------
    //
    // Group folding is the seam's, whole: these forward and decide nothing. The
    // only fold this file still owns is opening a *record* into its tree, which
    // costs a parse and so cannot be answered above the format.

    /// `zR` under a lens opens the groups the viewport has reached.
    pub(super) fn open_groups(&mut self, upto_row: usize) {
        ops::open_upto(self.plan.as_mut(), &mut self.map, upto_row);
    }

    /// `zM`.
    pub(super) fn close_groups(&mut self) {
        ops::close_all(self.plan.as_mut(), &mut self.map);
    }

    /// Open the group holding `record`, so a search hit is never left folded.
    pub(super) fn reveal_record(&mut self, record: usize) {
        ops::reveal(self.plan.as_mut(), &mut self.map, record);
    }

    /// The fold id of whatever sits on `at` — a group's or a record's.
    pub(super) fn fold_id_at(&self, at: Spot) -> String {
        ops::id_at(self.plan.as_ref(), at)
    }

    /// Apply a fold id that may name a group; `None` leaves it to the record
    /// half of [`Source::set_fold`].
    pub(super) fn set_group_by_id(&mut self, id: &str, open: bool) -> Option<bool> {
        ops::set_by_id(self.plan.as_mut(), &mut self.map, id, open)
    }

    /// The open groups, as fold ids.
    pub(super) fn group_folds(&self) -> Vec<String> {
        ops::open_ids(self.plan.as_ref())
    }

    /// Shut every group, then reopen the ones the fold state names.
    pub(super) fn restore_groups(&mut self, folds: &[String]) {
        ops::restore(self.plan.as_mut(), &mut self.map, folds);
    }

    /// `Tab` / `S-Tab` under a lens: the next item, not the next record.
    pub(super) fn next_item(&self, row: usize, forward: bool) -> Option<usize> {
        ops::next_item(self.plan.as_ref(), &self.map, self.known(), row, forward)
    }

    /// `Y` on a group's row: every record the run holds.
    pub(super) fn yank_group(&self, item: usize) -> Option<Yank> {
        ops::yank_group(self, self.plan.as_ref(), item)
    }
}

#[cfg(test)]
#[path = "lensrow_tests.rs"]
mod lensrow_tests;
