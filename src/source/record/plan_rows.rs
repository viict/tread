//! What a record shows **under its own summary row**, and how much of it.
//!
//! Three pieces of state, all keyed by *record* rather than by item, because a
//! member of an open run is as much a thing a reader opens as a message is:
//!
//! | | what it is | who moves rows |
//! | --- | --- | --- |
//! | the **level** | clipped, or open — an exception to [`Plan::all_full`] | `Under::body` and `Under::parts` |
//! | the **heights** | rows the body and the parts occupy at this width | `Plan::own`, `Plan::extra` |
//! | the **open call** | which part row is showing its arguments and output | `Under::parts` |
//!
//! # The ladder
//!
//! `Enter` / `za` descends one rung and wraps (SPEC.md §Lenses):
//!
//! ```text
//! clipped  ->  open  ->  the raw JSON tree  ->  clipped
//! ```
//!
//! **clipped** is the headline and a few rows of what was said; **open** is the
//! whole of it and the record's tool calls listed as tool calls; the **tree** is
//! the record itself. The rungs a record has depend on what is in it — a record
//! with nothing under the headline and no calls has only the tree, and one with
//! neither has no ladder at all and leaves the key to the outline.
//!
//! The tree rung is deliberately **not** a level here. Tree rows are
//! [`RowMap`]'s, keyed by record, shared with the no-lens path and toggled by
//! `zt` from any rung; the ladder writes *through* to it rather than owning it,
//! which is what leaves `zt` orthogonal and the prefix sums untouched.
//!
//! # What is stored, and what is not
//!
//! The heights are stored; the *parts* are not. [`crate::lens::Lens::detail`]
//! is asked for the record being measured or painted and its answer is dropped
//! — so a document's cost is one `Under` (two `usize`) per record, and never a
//! tool's output. That is the same bargain [`crate::lens::Body`] strikes one
//! level down, and for the same reason.
#![deny(unsafe_code)]

use crate::json::Value;
use crate::lens::{Body, Part};

use super::super::rowmap::RowMap;
use super::{Plan, Under};

impl Plan {
    /// A record that has just been folded away owns no rows: its tree is shut
    /// and its under-rows go with it, which is the invariant the two prefix
    /// sums above are only correct under.
    pub(super) fn hide(&mut self, record: usize, map: &mut RowMap) {
        map.close(record);
        self.extra.close(record);
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
        match open {
            // Its members are visible now: what each shows under its own row is
            // a wrap at a width, so the caller re-measures before asking for a
            // row ([`Plan::take_pending`]).
            true => self.pending.push(item),
            false => {
                for r in first..first + count {
                    self.hide(r, map);
                }
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


    // -- the level ---------------------------------------------------------------

    /// Is `record` at the **open** level — its text whole, its parts listed?
    ///
    /// An *exception* to [`Plan::all_full`] rather than a state of its own, so
    /// that `zR` can mean "everything, now" and one record can still clip again
    /// afterwards without the two disagreeing.
    pub fn full_at(&self, record: usize) -> bool {
        self.exc.get(record).copied().unwrap_or(false) != self.all_full
    }

    /// Put `record` at the open level, or back to its clip. True when it moved.
    pub fn set_full(&mut self, record: usize, full: bool) -> bool {
        if record >= self.exc.len() || self.full_at(record) == full {
            return false;
        }
        self.exc[record] = !self.exc[record];
        // A record leaving the open level takes its opened call with it: the
        // rows are gone, and a call left open would reappear on the next
        // descent without the reader having asked for it.
        if !full {
            self.open_parts.retain(|(r, _)| *r != record);
        }
        self.mark_record(record);
        true
    }

    /// Every record at the open level (a dump, and what `zR` leaves behind), or
    /// every record back to its clip. Exceptions are dropped either way: this is
    /// "everything, now".
    ///
    /// Opened calls are dropped with them rather than being inverted. `zR` is
    /// not "expand every argument of every call in the file" — a run of steps
    /// would become thousands of rows behind one keystroke — it is every record
    /// showing what it holds, with each call one row that still opens. The
    /// record's own tree, which `zR` also opens, is where every byte is.
    pub fn set_all_full(&mut self, full: bool) {
        self.all_full = full;
        for e in &mut self.exc {
            *e = false;
        }
        self.open_parts.clear();
        self.mark(0);
    }

    // -- the parts ----------------------------------------------------------------

    /// Is part `part` of `record` showing its arguments and its output?
    pub fn part_open(&self, record: usize, part: usize) -> bool {
        self.open_parts.contains(&(record, part))
    }

    /// The record whose call is open, if any. One at a time, so there is at
    /// most one — and the caller needs it to re-measure the record that is
    /// about to lose its rows.
    pub fn open_part_record(&self) -> Option<usize> {
        self.open_parts.first().map(|(r, _)| *r)
    }

    /// Open or shut one call. True when it changed, which is the caller's cue
    /// to re-measure that record.
    ///
    /// **One call opens at a time** (SPEC.md §Lenses): opening one shuts
    /// whichever was open, wherever it was. Two open calls put two screens of
    /// arguments and output on the screen at once, which is what the level
    /// exists to avoid — and the records that lost theirs are marked, so the
    /// rows go with it.
    pub fn set_part_open(&mut self, record: usize, part: usize, open: bool) -> bool {
        let at = self.open_parts.iter().position(|p| *p == (record, part));
        match (at, open) {
            (Some(_), true) | (None, false) => false,
            (Some(i), false) => {
                self.open_parts.remove(i);
                self.mark_record(record);
                true
            }
            (None, true) => {
                let shut = std::mem::take(&mut self.open_parts);
                for (r, _) in shut {
                    self.mark_record(r);
                }
                self.open_parts.push((record, part));
                self.mark_record(record);
                true
            }
        }
    }

    /// The parts of `record`, as its dialect reads them. Computed here and now
    /// and thrown away by the caller — never stored, never per document.
    pub fn detail(&self, value: &Value) -> Vec<Part> {
        self.lens.detail(value)
    }

    // -- what a record shows underneath -------------------------------------------

    /// The text under `record`'s row, and whether it is shown whole. `None` for
    /// a record the lens did not read and for one with nothing to say.
    pub fn body_of(&self, record: usize) -> Option<(&Body, bool)> {
        let body = self.summary(record)?.body.as_ref()?;
        Some((body, self.full_at(record)))
    }

    /// Rows `record` shows under its own row, split into the two kinds.
    pub fn under_of(&self, record: usize) -> Under {
        self.under.get(record).copied().unwrap_or_default()
    }

    /// Rows `record` shows under its own row, in total.
    pub fn under_rows(&self, record: usize) -> usize {
        self.under_of(record).rows()
    }

    /// Tell the plan how tall `record`'s body and parts are at the current
    /// width. The measurement is the caller's because text longer than
    /// [`crate::lens::BODY_KEEP`] is only whole inside the record, and this
    /// module never reads one.
    ///
    /// `member` is whether the record is inside a run the reader has opened, in
    /// which case its rows go into the second prefix sum rather than into its
    /// item's own — the one place the two paths differ.
    pub fn set_under(&mut self, record: usize, under: Under, member: bool) {
        if record >= self.under.len() {
            return;
        }
        let changed = self.under[record] != under;
        self.under[record] = under;
        if member {
            self.extra.close(record);
            self.extra.open(record, under.rows());
        }
        if changed || member {
            self.mark_record(record);
        }
    }

    /// Groups opened since this was last asked, for the caller to measure the
    /// members of. Draining rather than reading, because measuring is the
    /// answer to it.
    pub fn take_pending(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pending)
    }

    /// Everything after `record`'s item may have moved.
    fn mark_record(&mut self, record: usize) {
        if let Some(i) = self.item_of_record(record) {
            self.mark(i);
        }
    }
}
