//! What a lens does to the *shape* of a record document (SPEC.md §Lenses).
//!
//! # Items, not records
//!
//! Without a lens a record document is one row per record. With one, consecutive
//! mechanical records — a tool call, its result, a thought — collapse into a
//! single row that opens:
//!
//! ```text
//! user       21:29  I want a reader for the terminal.
//! assistant  21:31  Goal: build a Rust TUI markdown pager.
//!   ▸ ⟨6 steps · 4 tool calls⟩            21:31
//! assistant  21:36  The skeleton builds; here is what it does.
//! ```
//!
//! So the document is a list of **items**, each covering a run of records: a
//! message is an item of one, a run of steps is an item of many. Everything
//! else — search, folding, yanking, the status bar — keeps speaking records,
//! and this module is the only place that translates.
//!
//! An item is what everything above this module calls a **block** — the unit
//! `j`/`k` move by, and what the status bar counts (SPEC.md §Lenses). The two
//! words name one thing; `item` survives here because it is this file's own
//! arithmetic, and `block` is the only one the reader, the keymap and the trait
//! ever see. There is no third notion of a boundary anywhere.
//!
//! # Laziness survives it
//!
//! Classifying a record means parsing it, so the plan is built the same way the
//! line index is: **a prefix, extended as the viewport moves**. Records past
//! the classified prefix are one row each, ungrouped, until they are reached;
//! `len()` therefore shrinks slightly as grouping catches up, exactly as it
//! grows as the line index finds more records. Nothing above the viewport ever
//! moves, because classification always runs ahead of the rows being painted.
//!
//! # The two-level arithmetic
//!
//! An item owns rows; a record inside it can also be opened into its generic
//! tree, and those rows belong to [`RowMap`]. Rather than duplicate the
//! prefix-sum trick, this holds the *own* rows of the items (which do not
//! depend on any tree, though they do depend on the **width** — a message's
//! body is wrapped, so [`Plan::set_width`] re-lays every one of them) and adds
//! [`RowMap::extra_before`] on top. That is only
//! correct while a *hidden* record has no tree rows, which is why closing a
//! group closes its members ([`Plan::set_open`]) and why a step run growing
//! past one record closes the record it swallows — but only while that run is
//! shut. A member of an open group is visible and keeps its own tree.
//!
//! Nothing here parses, reads a file, names a record format, or recurses.
#![deny(unsafe_code)]

use crate::lens::{Body, Class, Lens, Summary};

use super::rowmap::RowMap;

/// One item: a run of records that share a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// First record of the run.
    pub first: usize,
    /// Records in the run. Only a step run is ever longer than one.
    pub count: usize,
    /// Mechanics rather than conversation.
    pub step: bool,
    /// Open, when this is a group. Meaningless on an item of one.
    pub open: bool,
    /// The message under this row is shown whole rather than clipped.
    /// Meaningless on anything but a message.
    pub full: bool,
}

impl Item {
    /// A run of steps long enough to be worth folding. A single step is more
    /// useful shown than hidden behind `⟨1 step⟩`.
    pub fn is_group(&self) -> bool {
        self.step && self.count > 1
    }
}

/// Where a screen row falls, in the vocabulary the source already speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// A record's summary row (`sub == 0`), or row `sub - 1` of its tree.
    ///
    /// The body rows between the two are [`Spot::Body`] and never `sub`: `sub`
    /// has meant "row `sub - 1` of the tree" everywhere since before there were
    /// bodies, and overloading it would have made every caller's `sub > 0`
    /// silently wrong.
    Record { record: usize, sub: usize },
    /// Row `line` of the message under a record's summary row.
    Body { record: usize, line: usize },
    /// A group's own row.
    Group { item: usize },
}

pub struct Plan {
    lens: Box<dyn Lens>,
    /// One entry per classified record, in order. `None` is a record the lens
    /// did not recognise: it keeps its own row and renders as the generic tree.
    seen: Vec<Option<Summary>>,
    items: Vec<Item>,
    /// Rows the message under item `i` occupies at [`Plan::width`], `0` for an
    /// item with no message. Held rather than recomputed because `own` is asked
    /// per painted row and a wrap is O(message).
    body: Vec<usize>,
    /// Own rows of every item before this one — no tree rows in it.
    starts: Vec<usize>,
    /// First index of `starts` that may be wrong.
    dirty: usize,
    /// Columns the bodies were laid out for.
    width: usize,
    /// Every body is shown whole: what a dump is, and what `zR` leaves behind.
    all_full: bool,
}

impl Plan {
    pub fn new(lens: Box<dyn Lens>) -> Plan {
        Plan {
            lens,
            seen: Vec::new(),
            items: Vec::new(),
            body: Vec::new(),
            starts: Vec::new(),
            dirty: 0,
            width: 80,
            all_full: false,
        }
    }

    pub fn lens_name(&self) -> &'static str {
        self.lens.name()
    }

    /// Records classified so far.
    pub fn classified(&self) -> usize {
        self.seen.len()
    }

    /// Read the next record — it must be record [`Plan::classified`] — and fold
    /// it into the item list.
    ///
    /// `map` is touched only to keep the invariant above: a record that has
    /// just been swallowed by a group is closed, because a hidden record may
    /// not own rows.
    pub fn classify(&mut self, record: usize, value: Option<&crate::json::Value>, map: &mut RowMap) {
        if record != self.seen.len() {
            return;
        }
        let mut sum = value.and_then(|v| self.lens.read(v));
        let step = matches!(sum, Some(Summary { class: Class::Step, .. }));
        // Mechanics stay one line, whatever a dialect put on them. The
        // invariant is load-bearing rather than decorative: a group's members
        // are steps, and `inside` places them one row apart.
        if step {
            if let Some(s) = sum.as_mut() {
                s.body = None;
            }
        }
        self.seen.push(sum);
        let extends = matches!(self.items.last(), Some(last) if last.step && step);
        if !extends {
            self.items.push(Item { first: record, count: 1, step, open: false, full: false });
            self.body.push(0);
            self.mark(self.items.len() - 1);
            return;
        }
        let at = self.items.len() - 1;
        let last = &mut self.items[at];
        last.count += 1;
        let (first, grouped, open) = (last.first, last.count == 2, last.open);
        self.mark(at);
        // A record inside a *closed* group owns no rows of its own, so anything
        // the swallowed record had open is closed with it. An open group is the
        // opposite case: its members each keep their own tree, the row
        // arithmetic above accounts for them, and closing one here would throw
        // away an expansion the reader can see (and can never get back, since
        // classification only ever runs once per record).
        if !open {
            map.close(record);
            if grouped {
                map.close(first);
            }
        }
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn item(&self, i: usize) -> Option<&Item> {
        self.items.get(i)
    }

    /// The lens's reading of a record, when it had one.
    pub fn summary(&self, record: usize) -> Option<&Summary> {
        self.seen.get(record).and_then(|s| s.as_ref())
    }

    // -- bodies -----------------------------------------------------------------

    /// The message under item `i`, and whether it is shown whole. `None` for a
    /// group, a step and an unread record: the three with no message under them.
    pub fn body_at(&self, item: usize) -> Option<(&Body, bool)> {
        let it = self.items.get(item)?;
        if it.is_group() {
            return None;
        }
        let body = self.summary(it.first)?.body.as_ref()?;
        Some((body, it.full != self.all_full))
    }

    /// Rows the message under item `i` occupies at the current width.
    pub fn body_rows(&self, item: usize) -> usize {
        self.body.get(item).copied().unwrap_or(0)
    }

    /// Tell the plan how tall item `i`'s message is. The measurement is the
    /// caller's because a message longer than [`crate::lens::BODY_KEEP`] is
    /// only whole inside the record, and this module never reads one.
    pub fn set_body(&mut self, item: usize, rows: usize) {
        if self.body.get(item).copied() == Some(rows) {
            return;
        }
        if let Some(slot) = self.body.get_mut(item) {
            *slot = rows;
            self.mark(item);
        }
    }

    /// Columns the bodies are laid out for.
    pub fn width(&self) -> usize {
        self.width
    }

    /// A new layout width. True when it changed — the caller's cue to re-measure
    /// every body before asking a row question, since a body is wrapped and a
    /// resize therefore moves rows in a way no fold does.
    pub fn set_width(&mut self, cols: usize) -> bool {
        let cols = cols.max(1);
        if self.width == cols {
            return false;
        }
        self.width = cols;
        self.mark(0);
        true
    }

    /// Show item `i`'s message whole, or clipped again. True when it changed.
    ///
    /// [`Item::full`] is an *exception* to [`Plan::all_full`] rather than a
    /// state of its own — the "default plus what disagrees with it" shape the
    /// fold state uses — which is what lets one body shut again after `zR`.
    pub fn set_full(&mut self, item: usize, full: bool) -> bool {
        let all = self.all_full;
        let Some(it) = self.items.get_mut(item) else {
            return false;
        };
        if (it.full != all) == full {
            return false;
        }
        it.full = !it.full;
        self.mark(item);
        true
    }

    /// Every message whole (a dump, and what `zR` leaves behind), or every
    /// message back to its clip. Exceptions are dropped either way: this is
    /// "everything, now".
    pub fn set_all_full(&mut self, full: bool) {
        self.all_full = full;
        for it in &mut self.items {
            it.full = false;
        }
        self.mark(0);
    }

    /// Open or close a group. Closing closes the trees of the records it hides.
    pub fn set_open(&mut self, item: usize, open: bool, map: &mut RowMap) -> bool {
        let Some(it) = self.items.get_mut(item) else {
            return false;
        };
        if !it.is_group() || it.open == open {
            return false;
        }
        it.open = open;
        let (first, count) = (it.first, it.count);
        if !open {
            for r in first..first + count {
                map.close(r);
            }
        }
        self.mark(item);
        true
    }

    /// Close every group.
    pub fn close_all(&mut self, map: &mut RowMap) {
        for i in 0..self.items.len() {
            self.set_open(i, false, map);
        }
    }

    /// Open every group up to and including the one on `row`, so `zR` opens
    /// what the reader can see rather than parsing the whole file.
    pub fn open_upto(&mut self, upto_row: usize, map: &mut RowMap) {
        for i in 0..self.items.len() {
            // Each opening moves everything after it, so the totals are
            // brought up to date before the next item is placed.
            self.sync();
            if self.row_of_item(i, map) > upto_row {
                return;
            }
            self.set_open(i, true, map);
        }
        self.sync();
    }

    // -- row arithmetic ---------------------------------------------------------

    /// Own rows of item `i`: its summary row, the message under it, and one per
    /// member record when it is an open group.
    ///
    /// A group has no message of its own and its members are steps, which is
    /// what keeps [`Plan::inside`] one row per member.
    fn own(&self, i: usize) -> usize {
        match self.items.get(i) {
            Some(it) if it.is_group() && it.open => 1 + it.count,
            Some(_) => 1 + self.body_rows(i),
            None => 0,
        }
    }

    /// Recompute [`Plan::starts`] from the first dirty item, exactly as
    /// [`RowMap`] restates its running totals: a change late in a long log
    /// touches only what follows it.
    fn mark(&mut self, from: usize) {
        self.dirty = self.dirty.min(from);
    }

    fn settle(&self) -> &[usize] {
        // `starts` is rebuilt through `sync`, which the source calls before
        // asking anything. Kept separate so every query below is `&self`.
        &self.starts
    }

    /// Bring the running totals up to date. Called by the source before any
    /// row question, and cheap when nothing changed.
    pub fn sync(&mut self) {
        if self.dirty >= self.items.len() && self.starts.len() == self.items.len() {
            return;
        }
        self.starts.truncate(self.dirty);
        let mut run = match self.dirty {
            0 => 0,
            n => self.starts[n - 1] + self.own(n - 1),
        };
        for i in self.dirty..self.items.len() {
            self.starts.push(run);
            run += self.own(i);
        }
        self.dirty = self.items.len();
    }

    /// Rows the classified prefix occupies, tree rows included.
    fn prefix_rows(&self, map: &RowMap) -> usize {
        let starts = self.settle();
        let own = match starts.len() {
            0 => 0,
            n => starts[n - 1] + self.own(n - 1),
        };
        own + map.extra_before(self.seen.len())
    }

    /// Rows in the document: the classified prefix, plus one per record the
    /// lens has not reached yet.
    pub fn rows(&self, known: usize, map: &RowMap) -> usize {
        self.prefix_rows(map) + known.saturating_sub(self.seen.len())
    }

    /// The row item `i`'s own summary sits on.
    pub fn row_of_item(&self, i: usize, map: &RowMap) -> usize {
        let starts = self.settle();
        match (starts.get(i), self.items.get(i)) {
            (Some(start), Some(it)) => start + map.extra_before(it.first),
            _ => 0,
        }
    }

    /// The row a record's summary sits on. A record inside a closed group has
    /// no row of its own; the group's row stands for it.
    pub fn row_of_record(&self, record: usize, map: &RowMap) -> usize {
        if record >= self.seen.len() {
            return self.prefix_rows(map) + (record - self.seen.len());
        }
        let Some(i) = self.item_of_record(record) else {
            return 0;
        };
        let base = self.row_of_item(i, map);
        let it = &self.items[i];
        match (it.is_group(), it.open) {
            (true, false) => base,
            (true, true) => {
                let inside = record - it.first;
                let trees = map.extra_before(record) - map.extra_before(it.first);
                base + 1 + inside + trees
            }
            (false, _) => base,
        }
    }

    /// Where a screen row falls.
    pub fn at(&self, row: usize, known: usize, map: &RowMap) -> Spot {
        let prefix = self.prefix_rows(map);
        if row >= prefix {
            let past = row - prefix;
            let record = (self.seen.len() + past).min(known.saturating_sub(1));
            return Spot::Record { record, sub: 0 };
        }
        let i = self.item_at_row(row, map);
        let base = self.row_of_item(i, map);
        let off = row - base;
        let it = &self.items[i];
        if !it.is_group() {
            // Summary row, then the message, then the record's own tree: the
            // order they are painted in, and the one `own` counted.
            let body = self.body_rows(i);
            return match off {
                0 => Spot::Record { record: it.first, sub: 0 },
                n if n <= body => Spot::Body { record: it.first, line: n - 1 },
                n => Spot::Record { record: it.first, sub: n - body },
            };
        }
        if !it.open || off == 0 {
            return Spot::Group { item: i };
        }
        self.inside(it, off - 1, map)
    }

    /// A row inside an open group: which member, and how far into its tree.
    fn inside(&self, it: &Item, off: usize, map: &RowMap) -> Spot {
        let before = map.extra_before(it.first);
        // Rows consumed by members `first..k` is `(k - first) + trees`, which
        // only grows: one binary search rather than a walk, because a group can
        // be hundreds of records long and this runs per painted row.
        let (mut lo, mut hi) = (0usize, it.count);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let used = mid + (map.extra_before(it.first + mid) - before);
            match used <= off {
                true => lo = mid,
                false => hi = mid - 1,
            }
        }
        let record = it.first + lo;
        let used = lo + (map.extra_before(record) - before);
        Spot::Record { record, sub: off - used }
    }

}

/// The fold id of a group, in a vocabulary that cannot collide with a record's
/// (`/4`): a group is `g4`, named by its first record so the id survives the
/// run growing as classification catches up.
pub fn group_id(first: usize) -> String {
    format!("g{first}")
}

/// The first record a group id names.
pub fn group_first(id: &str) -> Option<usize> {
    id.strip_prefix('g')?.parse().ok()
}

/// Where a block starts and ends — a child module so it can still reach the
/// prefix sums above, which are this file's own.
#[path = "plan_block.rs"]
mod block;

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
