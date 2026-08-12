//! Which record a screen row belongs to, when some records are expanded.
//!
//! A record document is one row per record until the reader opens one, at
//! which point that record's tree rows are spliced in under it. The mapping has
//! to work both ways and it has to be O(log n): `lines()` turns rows into
//! records, and `end`, `goto_id` and the fold code turn records back into rows.
//!
//! Two properties make the arithmetic simple. Expanding a record only ever adds
//! rows *after* its own summary row, so a record's summary row is unaffected by
//! its own fold; and expansions are stored sorted by record with a running
//! total, so "how many extra rows come before record `r`" is one binary search.
//!
//! Nothing here reads the file or parses a record. The row *count* of an
//! expansion is given to [`RowMap::open`] by the caller, which is the only
//! place that pays for a parse.
#![deny(unsafe_code)]

/// One expanded record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Exp {
    /// Record index, 0-based.
    record: usize,
    /// Rows its tree occupies, not counting the summary row.
    extra: usize,
    /// Extra rows contributed by every expansion *before* this one.
    before: usize,
}

/// The expansions, sorted by record.
#[derive(Default)]
pub struct RowMap {
    items: Vec<Exp>,
}

impl RowMap {
    /// Rows added to the document by every expansion.
    pub fn extra_total(&self) -> usize {
        self.items.last().map(|e| e.before + e.extra).unwrap_or(0)
    }

    /// How many records are open. The source counts through [`RowMap::records`];
    /// the tests spell the same fact this way.
    #[cfg(test)]
    pub fn open_count(&self) -> usize {
        self.items.len()
    }

    /// The open records, in order.
    pub fn records(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().map(|e| e.record)
    }

    /// Is `record` open?
    pub fn is_open(&self, record: usize) -> bool {
        self.find(record).is_ok()
    }

    /// Rows `record`'s tree occupies, `0` when it is closed. Only the tests
    /// ask: the source knows what it opened.
    #[cfg(test)]
    pub fn extra_of(&self, record: usize) -> usize {
        match self.find(record) {
            Ok(i) => self.items[i].extra,
            Err(_) => 0,
        }
    }

    /// The screen row `record`'s summary sits on.
    pub fn row_of(&self, record: usize) -> usize {
        record + self.extra_before(record)
    }

    /// `(record, sub)` for a screen row: `sub` is `0` on the summary row and
    /// `1..=extra` inside the tree, so `sub - 1` indexes the tree's own rows.
    pub fn at(&self, row: usize) -> (usize, usize) {
        let i = match self.items.partition_point(|e| e.record + e.before <= row) {
            0 => return (row, 0),
            n => n - 1,
        };
        let e = self.items[i];
        let base = e.record + e.before;
        match row <= base + e.extra {
            true => (e.record, row - base),
            false => (row - (e.before + e.extra), 0),
        }
    }

    /// Open `record` with a tree of `extra` rows. False when it was already
    /// open or the tree is empty — a record with nothing under it is a leaf,
    /// not a closed container, and pretending otherwise would give it a fold
    /// marker that does nothing.
    pub fn open(&mut self, record: usize, extra: usize) -> bool {
        if extra == 0 {
            return false;
        }
        let at = match self.find(record) {
            Ok(_) => return false,
            Err(i) => i,
        };
        let before = self.extra_before(record);
        self.items.insert(at, Exp { record, extra, before });
        self.restate(at + 1);
        true
    }

    /// Close `record`. False when it was not open.
    pub fn close(&mut self, record: usize) -> bool {
        let at = match self.find(record) {
            Ok(i) => i,
            Err(_) => return false,
        };
        self.items.remove(at);
        self.restate(at);
        true
    }

    /// Close everything.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Extra rows contributed by every expansion strictly before `record`.
    ///
    /// Public because the lens plan does the same prefix arithmetic one level
    /// up: "how many tree rows come before this item" is exactly this sum, and
    /// recomputing it there would be a second copy of the invariant.
    pub fn extra_before(&self, record: usize) -> usize {
        match self.items.partition_point(|e| e.record < record) {
            0 => 0,
            n => {
                let e = self.items[n - 1];
                e.before + e.extra
            }
        }
    }

    fn find(&self, record: usize) -> Result<usize, usize> {
        self.items.binary_search_by_key(&record, |e| e.record)
    }

    /// Recompute the running totals from `at` on. Only the entries after a
    /// change move, and appending in order (which is what expand-all does)
    /// touches nothing.
    fn restate(&mut self, at: usize) {
        let mut run = match at {
            0 => 0,
            n => {
                let e = self.items[n - 1];
                e.before + e.extra
            }
        };
        for e in &mut self.items[at..] {
            e.before = run;
            run += e.extra;
        }
    }
}

#[cfg(test)]
#[path = "rowmap_tests.rs"]
mod tests;
