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
//! What everything above calls a **block** — the unit `Tab`/`S-Tab` jump by,
//! and what the status bar counts (SPEC.md §Lenses) — is an item *while that
//! item is shut*, and not the same count once one is open: a boundary
//! descends into a run the reader has opened, so an open group is one item
//! and `1 + count`
//! blocks. `item` survives here because it is this file's own arithmetic;
//! `block` is the only word the reader, the keymap and the trait ever see.
//! Count blocks with [`Plan::blocks_of_item`] or the `bstarts` prefix sum
//! beside it, never `items().len()`; `plan_block.rs` defines every boundary.
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

use crate::lens::{Lens, Summary};

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
    /// Row `line` of the record's **parts** — the level between what was said
    /// and the raw tree: its tool calls, listed as tool calls, each of which
    /// opens into its arguments and its output.
    ///
    /// A fourth variant rather than more `sub`, for the reason [`Spot::Record`]
    /// gives: `sub` has meant "row `sub - 1` of the tree" since before there
    /// were bodies, and every caller's `sub > 0` would have gone quietly wrong.
    Part { record: usize, line: usize },
    /// A group's own row.
    Group { item: usize },
}

pub struct Plan {
    lens: Box<dyn Lens>,
    /// One entry per classified record, in order. `None` is a record the lens
    /// did not recognise: it keeps its own row and renders as the generic tree.
    seen: Vec<Option<Summary>>,
    items: Vec<Item>,
    /// Rows record `r` shows **under its own summary row**, at [`Plan::width`]:
    /// its body, then its parts. One entry per classified record rather than
    /// per item, because a member of an open run has them too — a step's
    /// reasoning is text and is shown wherever the step is.
    ///
    /// Held rather than recomputed because `own` is asked per painted row and a
    /// wrap is O(text).
    under: Vec<Under>,
    /// Which records are at the **open** level, as exceptions to
    /// [`Plan::all_full`] — the "default plus what disagrees" shape the fold
    /// state uses, which is what lets one record clip again after `zR`.
    exc: Vec<bool>,
    /// The call rows the reader has opened: `(record, part)`. Exceptions again,
    /// so this is as long as what was opened and not as long as the file.
    open_parts: Vec<(usize, usize)>,
    /// The under-rows of the **members of open runs**, as a second prefix sum
    /// beside the tree one.
    ///
    /// A group's own rows are `1 + count` and [`Plan::inside`] places its
    /// members one row apart; a member that shows its reasoning underneath
    /// breaks both. Rather than a third arithmetic, a member's extra rows get
    /// exactly the treatment a member's *tree* rows already get — attributed at
    /// that record's index, added on top by a prefix sum, and closed when the
    /// record is hidden. `own`, `inside`, `row_of_record` and `blocks_of_item`
    /// therefore keep the shape they had.
    extra: RowMap,
    /// Groups opened since the last measurement, for the caller to re-measure:
    /// a member's rows need a width and a record, and this module has neither.
    pending: Vec<usize>,
    /// Own rows of every item before this one — no tree rows in it.
    starts: Vec<usize>,
    /// Blocks before item `i`. An item is one block while it is shut and
    /// `1 + count` while it is an open group, because a block boundary descends
    /// into a run the reader has opened ([`Plan::blocks_of_item`]). Kept as a
    /// prefix sum next to `starts`, and rebuilt by the same `sync`, so the block
    /// index the status bar prints and the boundary `Tab` jumps to are read off
    /// one table rather than counted twice.
    bstarts: Vec<usize>,
    /// First index of `starts` that may be wrong.
    dirty: usize,
    /// Columns the bodies were laid out for.
    width: usize,
    /// Every record is at the open level: what a dump is, and what `zR` leaves
    /// behind.
    all_full: bool,
}

/// The rows one record shows under its own summary row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Under {
    /// Rows of the record's own text: what was said, or what it was thinking.
    pub body: usize,
    /// Rows of its parts: the tool calls, and whichever one is opened.
    pub parts: usize,
}

impl Under {
    fn rows(self) -> usize {
        self.body + self.parts
    }
}

impl Plan {
    pub fn new(lens: Box<dyn Lens>) -> Plan {
        Plan {
            lens,
            seen: Vec::new(),
            items: Vec::new(),
            under: Vec::new(),
            exc: Vec::new(),
            open_parts: Vec::new(),
            extra: RowMap::default(),
            pending: Vec::new(),
            starts: Vec::new(),
            bstarts: Vec::new(),
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
        let sum = value.and_then(|v| self.lens.read(v));
        let step = matches!(sum, Some(Summary { class: crate::lens::Class::Step, .. }));
        self.seen.push(sum);
        self.under.push(Under::default());
        self.exc.push(false);
        let extends = matches!(self.items.last(), Some(last) if last.step && step);
        if !extends {
            self.items.push(Item { first: record, count: 1, step, open: false });
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
            self.hide(record, map);
            if grouped {
                self.hide(first, map);
            }
            return;
        }
        // A record joining a run the reader has open is visible at once, so its
        // own rows have to be measured before anything asks for a row.
        self.pending.push(at);
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

    // -- row arithmetic ---------------------------------------------------------

    /// Own rows of item `i`: its summary row, whatever that record shows under
    /// it, and one per member record when it is an open group.
    ///
    /// A **group** owns one row per member and nothing more, whatever those
    /// members show underneath: a member's own rows are the [`Plan::extra`]
    /// prefix sum's, exactly as its tree rows are the [`RowMap`]'s. That is what
    /// keeps [`Plan::inside`] and [`Plan::blocks_of_item`] the shape they were.
    fn own(&self, i: usize) -> usize {
        match self.items.get(i) {
            Some(it) if it.is_group() => match it.open {
                true => 1 + it.count,
                false => 1,
            },
            Some(it) => 1 + self.under_rows(it.first),
            None => 0,
        }
    }

    /// Rows spliced in before `record` by everything that opens: the trees the
    /// reader expanded, and the under-rows of the members of open runs. The one
    /// place the two prefix sums are added, so no caller can add only one.
    fn before(&self, record: usize, map: &RowMap) -> usize {
        map.extra_before(record) + self.extra.extra_before(record)
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
        self.bstarts.truncate(self.dirty);
        let mut run = match self.dirty {
            0 => 0,
            n => self.starts[n - 1] + self.own(n - 1),
        };
        let mut blocks = match self.dirty {
            0 => 0,
            n => self.bstarts[n - 1] + self.blocks_of_item(n - 1),
        };
        for i in self.dirty..self.items.len() {
            self.starts.push(run);
            self.bstarts.push(blocks);
            run += self.own(i);
            blocks += self.blocks_of_item(i);
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
        own + self.before(self.seen.len(), map)
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
            (Some(start), Some(it)) => start + self.before(it.first, map),
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
                let under = self.before(record, map) - self.before(it.first, map);
                base + 1 + inside + under
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
            return self.place(it.first, off);
        }
        if !it.open || off == 0 {
            return Spot::Group { item: i };
        }
        self.inside(it, off - 1, map)
    }

    /// Where a record's own `sub`-th row falls: its summary row, then what was
    /// said, then its parts, then its tree.
    ///
    /// **The** order, and the only statement of it: a message and a step inside
    /// an open run are placed by this same function, so the two cannot drift.
    fn place(&self, record: usize, sub: usize) -> Spot {
        let u = self.under_of(record);
        match sub {
            0 => Spot::Record { record, sub: 0 },
            n if n <= u.body => Spot::Body { record, line: n - 1 },
            n if n <= u.rows() => Spot::Part { record, line: n - u.body - 1 },
            n => Spot::Record { record, sub: n - u.rows() },
        }
    }

    /// A row inside an open group: which member, and how far into that member.
    fn inside(&self, it: &Item, off: usize, map: &RowMap) -> Spot {
        let before = self.before(it.first, map);
        // Rows consumed by members `first..k` is `(k - first) + trees`, which
        // only grows: one binary search rather than a walk, because a group can
        // be hundreds of records long and this runs per painted row.
        let (mut lo, mut hi) = (0usize, it.count);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let used = mid + (self.before(it.first + mid, map) - before);
            match used <= off {
                true => lo = mid,
                false => hi = mid - 1,
            }
        }
        let record = it.first + lo;
        let used = lo + (self.before(record, map) - before);
        self.place(record, off - used)
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

/// What a record shows under its own row — the level it is at, its measured
/// heights, and which of its calls is open. A child module for the same reason
/// as `plan_block.rs`: it is this file's own state, and splitting it out is what
/// keeps both under the size limit.
#[path = "plan_rows.rs"]
mod rows;

/// The double both test files are built on. One copy, so the two halves of the
/// arithmetic cannot be checked against two different plans.
#[cfg(test)]
#[path = "plan_fixture.rs"]
mod fixture;

#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_level_tests.rs"]
mod level_tests;
