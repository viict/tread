//! Where a **block** starts and ends.
//!
//! A block is an [`Item`](super::Item) — a run of records that share a row —
//! seen from above: it is the unit `j`/`k` move by and the unit the status bar
//! counts (SPEC.md §Lenses). This is a child module of `plan` rather than a
//! sibling so it can use the prefix sums (`starts`, `own`, `prefix_rows`)
//! without making them public: a block's extent is that arithmetic read one
//! more time, not a second notion of a boundary.
#![deny(unsafe_code)]

use std::ops::Range;

use super::super::rowmap::RowMap;
use super::Plan;

impl Plan {
    /// The rows item `i` occupies: its own rows — summary, the message under
    /// it, one per member of an open group — plus the tree rows of every record
    /// inside it. This is a **block**, the unit `j`/`k` move by; "item" is this
    /// module's older word for the same run of records, and `src/source/mod.rs`
    /// upwards there is only "block".
    ///
    /// Exact and O(log n): the end is item `i + 1`'s start whenever there is
    /// one, computed the same way rather than by walking rows. Valid only after
    /// [`Plan::sync`], like every other row question here.
    pub fn rows_of_item(&self, i: usize, map: &RowMap) -> Option<Range<usize>> {
        let it = self.items.get(i)?;
        let start = self.settle().get(i).copied()?;
        let end = start + self.own(i) + map.extra_before(it.first + it.count);
        let start = start + map.extra_before(it.first);
        Some(start..end.max(start + 1))
    }

    /// The rows of the block `row` falls in.
    ///
    /// `None` past the classified prefix: grouping there is not decided yet, so
    /// the honest answer is that this row is not in a block — the caller frames
    /// the row alone rather than being told an extent that classification is
    /// about to change.
    pub fn block_at(&self, row: usize, map: &RowMap) -> Option<Range<usize>> {
        if self.items.is_empty() || row >= self.prefix_rows(map) {
            return None;
        }
        self.rows_of_item(self.item_at_row(row, map), map)
    }

    /// The index of the block `row` is on, and how many blocks are classified —
    /// for the status bar. `None` past the classified prefix, where the block a
    /// record will end up in is not decided.
    pub fn block_of_row(&self, row: usize, map: &RowMap) -> Option<(usize, usize)> {
        if self.items.is_empty() || row >= self.prefix_rows(map) {
            return None;
        }
        Some((self.item_at_row(row, map), self.items.len()))
    }

    /// The item a record belongs to.
    pub fn item_of_record(&self, record: usize) -> Option<usize> {
        let i = self.items.partition_point(|it| it.first <= record);
        let i = i.checked_sub(1)?;
        let it = &self.items[i];
        (record < it.first + it.count).then_some(i)
    }

    /// The last item whose row is at or before `row`.
    pub(super) fn item_at_row(&self, row: usize, map: &RowMap) -> usize {
        let (mut lo, mut hi) = (0usize, self.items.len().saturating_sub(1));
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            match self.row_of_item(mid, map) <= row {
                true => lo = mid,
                false => hi = mid - 1,
            }
        }
        lo
    }

    /// Rows a closed group hides: one per record it holds.
    pub fn hidden(&self, item: usize) -> usize {
        match self.items.get(item) {
            Some(it) if it.is_group() && !it.open => it.count,
            _ => 0,
        }
    }
}
