//! What folds in a record document, and what a fold hides.
//!
//! The fold half of what `view.rs` answers: the outline entry a row belongs to,
//! the three keys that change a fold (`Enter`/`za`, `zR`/`zM`, and an entry
//! toggled off the outline), the fold *state* a session saves and restores, and
//! the two questions a painted row asks about hiding — how much is under this
//! row, and where the next block starts. Split out of `view.rs` so both stay
//! under the size limit: `view.rs` keeps the layout, the positions and the
//! reads, and every method here is the body of one of its trait methods, named
//! apart from it so the forwarder is never the thing it forwards to.
//!
//! Nothing here decides what a *lens* folds — that is `ops.rs` over `plan.rs`,
//! reached through the group forwarders in `lensstate.rs`. This file only knows
//! that a record can be open or shut and that a group id is not a record id.
#![deny(unsafe_code)]

use super::*;

impl<S: Store> RecordSource<S> {
    /// The outline entry the row under the cursor belongs to.
    pub(crate) fn section_of(&self, row: usize) -> Option<usize> {
        let id = self.fold_id_at(self.spot(row));
        self.outline.iter().position(|e| e.id == id)
    }

    /// An outline entry opened or shut by name.
    pub(crate) fn fold_entry(&mut self, entry: usize, closed: bool) -> bool {
        let id = match self.outline.get(entry) {
            Some(e) => e.id.clone(),
            None => return false,
        };
        // A group id (`g12`) cannot collide with a record id (`/12`); the seam
        // tells them apart, and `None` is "not a group" — this file only has to
        // answer for its own records, which is the half that costs a parse.
        if let Some(changed) = self.set_group_by_id(&id, !closed) {
            if changed {
                self.mark_entry(entry, closed);
            }
            return changed;
        }
        let record: usize = match jsonrow::top_index(&id) {
            Some(r) => r,
            None => return false,
        };
        let changed = match closed {
            true => self.map.close(record),
            false => self.open_record(record),
        };
        if changed {
            self.mark_entry(entry, closed);
        }
        changed
    }

    /// The outline is what the last frame painted, so an entry that has since
    /// scrolled away simply is not there to update.
    fn mark_entry(&mut self, entry: usize, closed: bool) {
        if let Some(e) = self.outline.get_mut(entry) {
            e.folded = closed;
        }
    }

    /// `zM` shuts every record, which is free. `zR` opens them as the viewport
    /// reaches them, up to [`EXPAND_CAP`]: opening a million records means
    /// parsing a million records, and a reader that froze on one keystroke
    /// would be worse than one that opens what you can see.
    pub(crate) fn fold_every(&mut self, closed: bool) {
        self.expand_all = !closed;
        self.filled = 0;
        self.map.clear();
        // Messages go with it: `zR` is "show me everything here", and a dump
        // (`--plain`, which is `fold_all(false)` and nothing else) must print
        // the whole of what was said rather than a viewport's clip of it.
        self.set_all_bodies(!closed);
        if closed {
            self.close_groups();
        }
        if !closed {
            let upto = self.window.end.max(1).saturating_add(LOOKAHEAD);
            self.fill_expansion(upto);
        }
    }

    /// `Enter` / `za`: the record's two levels — clipped and open, and back
    /// again (SPEC.md §Lenses). The row's own fold, which is not an outline
    /// entry and could not be reached through one. The raw tree is `r`, not a
    /// rung, so this never opens or shuts one. A group row falls through to the
    /// outline, which is what opens a run.
    ///
    /// **A record's row claims the key even when it has no rung.** Falling
    /// through would hand `Enter` to the outline, and a record's outline entry
    /// *is* its raw tree — so on exactly the records with nothing between the
    /// headline and the JSON (a message the clip already showed whole, which
    /// made no calls) the key would open and shut what `r` owns, and the tree
    /// `r` had opened would vanish under a press of `Enter`. Under a lens the
    /// answer there is "nothing to descend into", and it is answered here.
    /// With **no lens** there is no ladder and no lens rows: a record row is a
    /// collapsed tree and the outline is what opens it, so the key falls
    /// through exactly as it always did.
    pub(crate) fn fold_at_row(&mut self, row: usize) -> Option<bool> {
        if let Some(was) = self.descend(row) {
            return Some(was);
        }
        // No lens, no ladder: the key belongs to the outline, as it always did.
        self.plan.as_ref()?;
        match self.spot(row) {
            // A run is opened *through* the outline, which is the one thing
            // here that is an outline entry in its own right.
            Spot::Group { .. } => None,
            // Claimed, and nothing moved: no level changed, so "was open" is
            // false and the pager repaints the rows it already had.
            _ => Some(false),
        }
    }

    /// The shared fold-id vocabulary ([`jsonrow::ALL_OPEN`]): a default plus the
    /// ids that disagree with it, each id a member-index path from the root.
    /// A record file's root is the implicit list of records, so record 4 is
    /// `/4` — the same id the document reader would give member 4 of its root.
    ///
    /// The default here is *shut*, so the exceptions are the open records: a
    /// million-record file has a million closed ones and listing those is the
    /// one thing this source must never do. `zR` is [`jsonrow::ALL_OPEN`] and
    /// nothing else, exactly as it is on the document side.
    pub(crate) fn fold_ids(&self) -> FoldState {
        let mut out: Vec<String> = Vec::new();
        if self.expand_all {
            out.push(jsonrow::ALL_OPEN.to_string());
        }
        out.extend(self.group_folds());
        out.extend(self.map.records().map(fold_id));
        out
    }

    /// The same vocabulary read back: everything shut, then the ids that said
    /// otherwise.
    pub(crate) fn restore_folds(&mut self, folds: FoldState) {
        self.map.clear();
        self.expand_all = folds.iter().any(|s| s == jsonrow::ALL_OPEN);
        self.filled = 0;
        self.restore_groups(&folds);
        for id in &folds {
            if let Some(record) = jsonrow::top_index(id) {
                self.open_record(record);
            }
        }
        if self.expand_all {
            self.fill_expansion(self.window.end.max(1).saturating_add(LOOKAHEAD));
        }
    }

    /// How many rows this row is hiding, for the gutter.
    pub(crate) fn hidden_under(&self, row: usize) -> Option<usize> {
        let (record, sub) = match self.spot(row) {
            // A body row hides nothing: the clip says what it is not showing.
            // Nor does a part row — a shut call carries its own glyph, and the
            // gutter marker this answer drives belongs to the record.
            Spot::Body { .. } | Spot::Part { .. } => return None,
            // A closed group hides one row per record it holds; their trees
            // are closed with it, so there is nothing else under it.
            Spot::Group { item } => {
                let hidden = self.plan.as_ref().map(|p| p.hidden(item)).unwrap_or(0);
                return (hidden > 0).then_some(hidden);
            }
            Spot::Record { record, sub } => (record, sub),
        };
        if sub != 0 || self.map.is_open(record) || record >= self.known() {
            return None;
        }
        match self.tree_len(record) {
            0 => None,
            n => Some(n),
        }
    }

    /// Block boundaries — what `Tab` / `S-Tab` jump between; see
    /// `plan_block.rs`. With no lens a record is its own block, and the tree
    /// rows under an open one are inside it.
    pub(crate) fn landmark(&self, row: usize, forward: bool) -> Option<usize> {
        if self.plan.is_some() {
            return self.next_block_row(row, forward);
        }
        let (record, sub) = self.map.at(row);
        match forward {
            true if record + 1 < self.known() => Some(self.map.row_of(record + 1)),
            true => None,
            false if sub > 0 => Some(self.map.row_of(record)),
            false if record > 0 => Some(self.map.row_of(record - 1)),
            false => None,
        }
    }
}
