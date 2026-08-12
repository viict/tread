//! Where a **block** starts and ends.
//!
//! A block is the unit `j`/`k` move by and the unit the status bar counts
//! (SPEC.md §Lenses). It is *usually* an [`Item`](super::Item) — a run of
//! records that share a row — but **a boundary descends into an open group**:
//! opening a run is the reader saying "show me what is in here", so the steps
//! inside it become the blocks. The sequence over an open run is the run's own
//! row, then one block per member — a member and whatever tree it has open being
//! one block, exactly as a message and its body are. A **shut** group stays one
//! block; that is what makes it a summary.
//!
//! Everything here is read off one table: [`Plan::block_index_at`] and
//! [`Plan::row_of_block`] are inverses over the prefix sum `bstarts`, and the
//! extent, the boundary and the counter are all derived from that pair. Two
//! notions of a boundary is the drift this module exists to prevent.
//!
//! This is a child module of `plan` rather than a sibling so it can use the
//! prefix sums (`starts`, `bstarts`, `own`, `prefix_rows`, `inside`) without
//! making them public.
#![deny(unsafe_code)]

use std::ops::Range;

use super::super::rowmap::RowMap;
use super::{Plan, Spot};

impl Plan {
    /// Blocks item `i` contributes: one, plus one per member while it is an
    /// **open** group. This is the whole of the descent rule, and the only
    /// place it is stated.
    pub(super) fn blocks_of_item(&self, i: usize) -> usize {
        match self.items.get(i) {
            Some(it) if it.is_group() && it.open => 1 + it.count,
            Some(_) => 1,
            None => 0,
        }
    }

    /// Blocks in the classified prefix. Grouping makes this *shrink* as
    /// classification catches up, and opening a run makes it *grow*; both are
    /// why the status bar prints it with `≥` until the file is read.
    pub fn blocks(&self) -> usize {
        match self.items.len() {
            0 => 0,
            n => self.bstarts.get(n - 1).copied().unwrap_or(0) + self.blocks_of_item(n - 1),
        }
    }

    /// The row block `b` starts on. `None` for a block that does not exist.
    ///
    /// O(log n): one search of `bstarts`, then the same row arithmetic
    /// [`Plan::row_of_item`] and [`Plan::row_of_record`] already do.
    pub fn row_of_block(&self, b: usize, map: &RowMap) -> Option<usize> {
        if b >= self.blocks() {
            return None;
        }
        let i = self.bstarts.partition_point(|&s| s <= b).checked_sub(1)?;
        let it = self.items.get(i)?;
        match b - self.bstarts[i] {
            0 => Some(self.row_of_item(i, map)),
            sub => Some(self.row_of_record(it.first + (sub - 1), map)),
        }
    }

    /// The index of the block `row` falls in.
    ///
    /// `None` past the classified prefix: grouping there is not decided yet, so
    /// the honest answer is that this row is not in a block — the caller frames
    /// the row alone rather than being told an extent classification is about
    /// to change.
    pub fn block_index_at(&self, row: usize, map: &RowMap) -> Option<usize> {
        if self.items.is_empty() || row >= self.prefix_rows(map) {
            return None;
        }
        let i = self.item_at_row(row, map);
        let base = self.bstarts.get(i).copied()?;
        let it = self.items.get(i)?;
        let off = row - self.row_of_item(i, map);
        if !it.is_group() || !it.open || off == 0 {
            return Some(base);
        }
        // Whatever kind of row it is — the member's own, a row of what it said,
        // one of its parts, one of its tree — it belongs to *that member's*
        // block. Naming only `Spot::Record` here was right only while a member
        // had nothing under it: once a step shows its reasoning, the rows in
        // between were attributed to the run and `j` walked backwards out of
        // them.
        match self.inside(it, off - 1, map) {
            Spot::Record { record, .. }
            | Spot::Body { record, .. }
            | Spot::Part { record, .. } => Some(base + 1 + (record - it.first)),
            Spot::Group { .. } => Some(base),
        }
    }

    /// The rows of the block `row` falls in: from its first row to the first row
    /// of the block after it, which is the end of the classified prefix for the
    /// last one. A member's block is that member and its open tree, never the
    /// whole run.
    pub fn block_at(&self, row: usize, map: &RowMap) -> Option<Range<usize>> {
        let b = self.block_index_at(row, map)?;
        let start = self.row_of_block(b, map)?;
        let end = self.row_of_block(b + 1, map).unwrap_or_else(|| self.prefix_rows(map));
        Some(start..end.max(start + 1))
    }

    /// The next or previous block boundary, strictly after (before) `row` —
    /// `j`/`k`, and `Tab`'s fallback. From inside a block the first press back
    /// goes to that block's own first row, which for a member of an open run is
    /// the member's row rather than the run's: `k` mirrors `j` step for step,
    /// including stepping back out of the run to the block above it.
    ///
    /// `None` at either end and past the classified prefix; the pager then moves
    /// one row, so no row is ever unreachable.
    pub fn next_block(&self, row: usize, map: &RowMap, forward: bool) -> Option<usize> {
        let b = self.block_index_at(row, map)?;
        let start = self.row_of_block(b, map)?;
        match forward {
            true => self.row_of_block(b + 1, map),
            false if row > start => Some(start),
            false => self.row_of_block(b.checked_sub(1)?, map),
        }
    }

    /// The index of the block `row` is on, and how many blocks are classified —
    /// for the status bar. The same pair `j` steps by, so the counter cannot
    /// disagree with what the key does.
    pub fn block_of_row(&self, row: usize, map: &RowMap) -> Option<(usize, usize)> {
        Some((self.block_index_at(row, map)?, self.blocks()))
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
