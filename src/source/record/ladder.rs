//! The **ladder**: how far into a record `Enter` goes, and what that costs.
//!
//! SPEC.md §Lenses gives a record two levels, and this is the one place the key
//! that walks them lives:
//!
//! ```text
//! clipped  <->  open
//! ```
//!
//! The raw JSON tree is **not** a rung: it is `r`, which is orthogonal to the
//! level and lives in `rows.rs`. This module never opens a tree and never shuts
//! one, which is the whole of why `Enter` can no longer undo `r`'s work.
//!
//! A third `impl RecordSource`, beside `rows.rs` and `view.rs`, split out for
//! the same reason they were: three sides of one type, each under the size
//! limit. What is here rather than in `plan_rows.rs` is everything that needs a
//! **record** — the level a record is at is the plan's, but whether descending
//! to it would show anything, and how tall it is when it does, are questions
//! only whoever can read the file may answer.
#![deny(unsafe_code)]

use super::*;

impl<S: Store> RecordSource<S> {
    /// Run `f` against the laid-out parts of `record` — the open level's rows,
    /// and which call each of them belongs to.
    ///
    /// The same one-record cache `body_laid` is, keyed by width, and for the
    /// same reason: a frame asks per painted row, and asking a dialect what a
    /// record contains once per row of it would be quadratic.
    pub(crate) fn with_parts<T>(&self, record: usize, f: impl FnOnce(&parts::Laid) -> T) -> T {
        let width = self.view;
        let hit = matches!(&*self.parts_laid.borrow(), Some((r, w, _)) if *r == record && *w == width);
        if !hit {
            let laid = match self.plan.as_ref() {
                Some(plan) => self.with_value(record, |v| lensrow::part_rows(plan, record, v, width)),
                None => parts::Laid::empty(),
            };
            *self.parts_laid.borrow_mut() = Some((record, width, laid));
        }
        match &*self.parts_laid.borrow() {
            Some((_, _, laid)) => f(laid),
            None => f(&parts::Laid::empty()),
        }
    }

    /// `Enter` / `za`: **the record's two levels** (SPEC.md §Lenses).
    ///
    /// ```text
    /// clipped  <->  open
    /// ```
    ///
    /// A record with nothing under its headline and no calls has no *open*
    /// level, so it has no rung at all and says `None` — at **either** level,
    /// asked before the level is consulted, because the two show the same rows.
    /// `None` here does not mean the key is free: [`Source::fold_here`] claims
    /// it for the record's row anyway, since the fall-through above is the
    /// outline and a record's outline entry is the raw tree that belongs to
    /// `r`. Its bytes are `r`, and only `r`.
    ///
    /// **With the record's tree open this leaves the tree alone** and toggles
    /// the record's own rows underneath it. `r` owns the tree, and a key that
    /// silently shut what another key opened is exactly what the third rung was
    /// removed for; the way out of a tree is the `r` that opened it.
    ///
    /// On a **call row** the key belongs to that call rather than to the record:
    /// it shows the arguments it was made with and the output it returned, and
    /// shuts them again. That is the one place a row inside a record has a fold
    /// of its own, and it is why this is not simply a level flag.
    ///
    /// `None` on a group's row: `Enter` there opens the run, through the
    /// outline, exactly as it did.
    pub(crate) fn descend(&mut self, row: usize) -> Option<bool> {
        let record = match self.spot(row) {
            // A call row's key is the call's. Anything else on the open
            // level — a named stretch of text — is a row of the record, and
            // the key means what it means on a body row.
            Spot::Part { record, line } => match self.with_parts(record, |l| l.call_at(line)) {
                Some(part) => return self.toggle_part(record, part),
                None => record,
            },
            Spot::Body { record, .. } => record,
            Spot::Record { record, sub: 0 } => record,
            _ => return None,
        };
        // No rung at all — the clip already shows the whole of this record's
        // text and it made no calls. Asked *before* the level is consulted,
        // not only on the way in: `zR` puts every record at the open level,
        // and a rungless one that answered "was open, now clipped" would spend
        // a press repainting rows that are the same at both levels.
        // `opens_further` is a question about the record and the width, never
        // about which level it is at, so it is the same answer either way.
        if !self.opens_further(record) {
            return None;
        }
        let plan = self.plan.as_ref()?;
        // Two levels, and nothing here touches `self.map`: a tree the reader
        // opened with `r` stays open across both of them.
        if !plan.full_at(record) {
            return self.set_level(record, true);
        }
        // Already open: back to the clip.
        self.set_level(record, false)
    }

    /// Is there an **open** rung on this record — anything the clip is not
    /// already showing? A whole message with no tool calls has none, and the
    /// key would otherwise repaint the same rows and call it a descent.
    fn opens_further(&self, record: usize) -> bool {
        let Some(plan) = self.plan.as_ref() else {
            return false;
        };
        let clips = match plan.body_of(record) {
            None => false,
            Some((body, _)) => {
                let shape = lensrow::shape_of(plan, record, plan.width());
                body::clips(body, body.text_in(None), shape)
            }
        };
        clips || self.with_value(record, |v| v.is_some_and(|v| !plan.detail(v).is_empty()))
    }

    /// Put `record` at the open level or back to its clip, re-measuring it.
    /// `None` when it was already there, so the caller can fall through.
    fn set_level(&mut self, record: usize, full: bool) -> Option<bool> {
        let plan = self.plan.as_mut()?;
        let was = plan.full_at(record);
        if !plan.set_full(record, full) {
            return None;
        }
        self.remeasure_record(record);
        Some(was)
    }

    /// `Enter` on a call row: its arguments and its output, or neither.
    fn toggle_part(&mut self, record: usize, part: usize) -> Option<bool> {
        let plan = self.plan.as_mut()?;
        let was = plan.part_open(record, part);
        // One call opens at a time, so opening this one may have shut a call on
        // another record — whose rows are gone and whose height has to be
        // restated before anything asks for a row.
        let other = plan.open_part_record().filter(|r| *r != record);
        if !plan.set_part_open(record, part, !was) {
            return None;
        }
        if let Some(other) = other {
            self.remeasure_record(other);
        }
        self.remeasure_record(record);
        Some(was)
    }

    /// Re-lay one record — what a rung of the ladder costs.
    fn remeasure_record(&mut self, record: usize) {
        let item = self.plan.as_ref().and_then(|p| p.item_of_record(record));
        if let Some(item) = item {
            self.remeasure_item(item);
        }
    }
}
