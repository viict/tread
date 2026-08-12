//! The lens state on the source, and the thin forwarders the rest of it calls.
//!
//! A fourth `impl RecordSource`, split out for the size limit and for one more
//! reason: nothing about lenses is *decided* here. `plan`, `lensrow`, `parts`
//! and `ops` own the plan, the row arithmetic, the levels, group folding and
//! every painted row; this holds the state, because that is where the fold state
//! already lives, and forwards. Every method below is one line long on purpose —
//! the one exception, [`RecordSource::settle`], says in its own doc comment why
//! it cannot be.
#![deny(unsafe_code)]

use super::*;

impl<S: Store> RecordSource<S> {
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
    pub(crate) fn classify_to(&mut self, upto: usize) {
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
    pub(crate) fn record_visible(&self, record: usize) -> bool {
        lensrow::record_visible(self.plan.as_ref(), record)
    }

    /// A record's row as the lens reads it, or `None` when the lens did not
    /// recognise it — the caller then renders the generic tree row.
    pub(crate) fn lens_row(&self, record: usize, inset: bool) -> Option<Line> {
        lensrow::lens_row(self, self.plan.as_ref(), record, inset)
    }

    /// A folded run of mechanics: `⟨6 steps · 4 tool calls⟩`.
    pub(crate) fn group_row(&self, item: usize) -> Line {
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
    pub(crate) fn in_open_group(&self, record: usize) -> bool {
        lensrow::in_open_group(self.plan.as_ref(), record)
    }

    // -- folding ---------------------------------------------------------------
    //
    // Group folding is the seam's, whole: these forward and decide nothing. The
    // only fold left is opening a *record* into its tree, which costs a parse.

    /// Measure whatever a fold has just made visible.
    ///
    /// Opening a run is the plan's decision; how tall its members are is not —
    /// a member's reasoning is a wrap at a width over text only the format can
    /// read. Every path that opens a group ends here, because a row question
    /// asked in between would be asked of a prefix sum that has not been told
    /// about the rows it is now supposed to include.
    fn settle(&mut self) {
        let Some(mut plan) = self.plan.take() else {
            return;
        };
        lensrow::flush(&*self, &mut plan);
        plan.sync();
        self.plan = Some(plan);
        self.drop_laid();
    }

    /// `zR` under a lens opens the groups the viewport has reached.
    pub(crate) fn open_groups(&mut self, upto_row: usize) {
        ops::open_upto(self.plan.as_mut(), &mut self.map, upto_row);
        self.settle();
    }

    /// `zM`.
    pub(crate) fn close_groups(&mut self) {
        ops::close_all(self.plan.as_mut(), &mut self.map);
    }

    /// Open the group holding `record`, so a search hit is never left folded.
    pub(crate) fn reveal_record(&mut self, record: usize) {
        ops::reveal(self.plan.as_mut(), &mut self.map, record);
        self.settle();
    }

    /// The fold id of whatever sits on `at` — a group's or a record's.
    pub(crate) fn fold_id_at(&self, at: Spot) -> String {
        ops::id_at(self.plan.as_ref(), at)
    }

    /// Apply a fold id that may name a group; `None` leaves it to the record
    /// half of [`Source::set_fold`].
    pub(crate) fn set_group_by_id(&mut self, id: &str, open: bool) -> Option<bool> {
        let changed = ops::set_by_id(self.plan.as_mut(), &mut self.map, id, open);
        self.settle();
        changed
    }

    /// The open groups, as fold ids.
    pub(crate) fn group_folds(&self) -> Vec<String> {
        ops::open_ids(self.plan.as_ref())
    }

    /// Shut every group, then reopen the ones the fold state names.
    pub(crate) fn restore_groups(&mut self, folds: &[String]) {
        ops::restore(self.plan.as_mut(), &mut self.map, folds);
        self.settle();
    }

    /// `Tab` / `S-Tab` under a lens: the next block boundary, not the next
    /// record — and inside an open run, its steps are blocks.
    pub(crate) fn next_block_row(&self, row: usize, forward: bool) -> Option<usize> {
        ops::next_block(self.plan.as_ref(), &self.map, row, forward)
    }

    /// The rows of the block a row falls in.
    pub(crate) fn block_rows(&self, row: usize) -> Option<std::ops::Range<usize>> {
        ops::block_at(self.plan.as_ref(), &self.map, row)
    }

    /// `(index, count)` of the block a row is on, for the status bar.
    pub(crate) fn block_of_row(&self, row: usize) -> Option<(usize, usize)> {
        ops::block_of_row(self.plan.as_ref(), &self.map, row)
    }

    /// The status bar's block clause — `  ·  block 2/23` — or nothing at all.
    ///
    /// Nothing in three cases, each of them honest rather than tidy: with no
    /// lens there are no blocks, and past the classified prefix the block a row
    /// will end up in is not decided yet, so a number there would be one the
    /// next keystroke changes. The total carries the same `≥` the record count
    /// does, and for the same reason twice over: the lens has only read a
    /// prefix, and grouping makes that total *shrink* as it catches up. It
    /// *grows* on the keystroke that opens a run, whose steps are blocks while
    /// it is open; both numbers are read off the one table `Tab` jumps by.
    pub(crate) fn block_text(&self, row: usize) -> String {
        let Some((i, n)) = self.block_of_row(row) else {
            return String::new();
        };
        let classified = self.plan.as_ref().map(|p| p.classified()).unwrap_or(0);
        let total = match self.complete() && classified >= self.known() {
            true => format!("{n}"),
            false => format!("\u{2265}{n}"),
        };
        format!("  \u{b7}  block {}/{total}", i.saturating_add(1))
    }

    /// `Y` on a group's row: every record the run holds.
    pub(crate) fn yank_group(&self, item: usize) -> Option<Yank> {
        ops::yank_group(self, self.plan.as_ref(), item)
    }
}
