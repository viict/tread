//! Where the cursor goes: the motions themselves, and the framing that decides
//! what the viewport shows once it gets there.
//!
//! Split out of `input.rs` — which dispatches keys to them — to keep both files
//! under the size limit; these are the same `Pager` methods, one door down.
//!
//! Two units live here and the difference between them is the whole of
//! SPEC.md §Lenses' reading model: [`Pager::step_block`] moves by whatever the
//! document says a block is, and [`Pager::move_cursor`] moves by rows, which is
//! what every other motion counts in.
#![deny(unsafe_code)]

use super::Pager;
use crate::source::Anchor;

impl Pager {
    /// `j` / `k`: one **block** where the document has them, one row where it
    /// does not (SPEC.md §Lenses). Everything else keeps counting rows —
    /// `Ctrl-E`/`Ctrl-Y` one, `d`/`u` half a screen, `space`/`b` a whole one —
    /// so a block motion is never the only way to reach a row.
    ///
    /// The fallback to a row step is load-bearing rather than defensive.
    /// `Source::next_landmark` has no answer in two places a reader really
    /// gets to: past a lazily classified prefix (the tail of a big trajectory,
    /// where grouping is not decided yet) and at the two ends. Freezing there
    /// would make `j` do nothing on the last screen of a file.
    pub(super) fn step_block(&mut self, forward: bool) {
        let step = match forward {
            true => 1,
            false => -1,
        };
        if !self.src.blocks() {
            return self.move_cursor(step);
        }
        match self.src.next_landmark(self.cursor, forward) {
            Some(row) => self.frame_block(row),
            None => self.move_cursor(step),
        }
    }

    /// Land on a block and show the block, not just the row it starts on: the
    /// minimum scroll that brings all of it on screen, or its first row at the
    /// top when it is taller than the window.
    ///
    /// `Source::block_at` is asked for the extent rather than
    /// `Source::next_landmark` a second time — the format has the arithmetic,
    /// and the landmark pair cannot answer at the last block or from inside one
    /// (`src/source/mod.rs` says why). `None` frames the landing row alone.
    ///
    /// Order matters: cursor, then `top`, then [`Pager::clamp`], which can only
    /// pull `top` back to the end of the document — the cursor is inside
    /// `top..top + h` in both branches, so nothing else it does can fire.
    pub(super) fn frame_block(&mut self, row: usize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        self.cursor = row.min(n - 1);
        let h = self.content_rows().max(1);
        let end = match self.src.block_at(self.cursor) {
            Some(r) => r.end.max(self.cursor + 1),
            None => self.cursor + 1,
        };
        let last = (end - 1).min(n - 1).max(self.cursor);
        match last - self.cursor >= h {
            true => self.top = self.cursor,
            false => {
                if self.cursor < self.top {
                    self.top = self.cursor;
                }
                if last >= self.top + h {
                    self.top = last + 1 - h;
                }
            }
        }
        self.clamp();
    }

    /// `Ctrl-E` / `Ctrl-Y`: **scroll** one row — the window moves and the
    /// cursor rides along, keeping its place on the screen.
    ///
    /// Moving the cursor alone would not do: [`Pager::frame_block`] parks it at
    /// the top of the window on a block taller than the viewport, and a cursor
    /// step only drags `top` once it leaves the window — so the first screenful
    /// of presses would leave the text frozen on exactly the block this key
    /// exists to read (SPEC.md §Lenses). The window moves first, so the first
    /// press moves the text.
    ///
    /// At the two ends the window runs out before the cursor does: `top` stops
    /// and the cursor keeps going, which is what keeps the last row of the
    /// document reachable one row at a time.
    pub(super) fn scroll_row(&mut self, delta: isize) {
        if self.len() == 0 {
            return;
        }
        let h = self.content_rows().max(1);
        let pin = self.pinned();
        let max_top = self.len().saturating_sub(h).max(pin);
        self.top = (self.top as isize)
            .saturating_add(delta)
            .clamp(pin as isize, max_top as isize) as usize;
        // `move_cursor` re-clamps; with both moved by the same delta the cursor
        // is still inside `top..top + h`, so it has nothing left to correct.
        self.move_cursor(delta);
    }

    pub(super) fn move_cursor(&mut self, delta: isize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        let max = n as isize - 1;
        let next = (self.cursor as isize).saturating_add(delta).clamp(0, max);
        self.cursor = next as usize;
        self.clamp();
    }

    pub(super) fn goto(&mut self, row: usize) {
        if self.len() == 0 {
            return;
        }
        self.cursor = row.min(self.len() - 1);
        self.clamp();
    }

    /// `h` / `l`. A format may scroll by its own unit — a CSV moves a whole
    /// column, and its pinned header moves with the body because both are
    /// painted at this one offset (SPEC.md §CSV).
    ///
    /// The format is told the *viewport* width, the same `cols` the painter
    /// slices rows to and [`Pager::max_hoff`] measures against — not the layout
    /// width, which `--width` can set far wider than the terminal.
    pub(super) fn scroll_h(&mut self, delta: isize) {
        let max = self.max_hoff();
        let hoff = self.hoff;
        let view = self.cols.max(1);
        let next = match self.src.hscroll(hoff, delta.signum(), view) {
            Some(off) => off as isize,
            None => (hoff as isize).saturating_add(delta),
        };
        self.hoff = next.clamp(0, max as isize) as usize;
    }

    /// `w`: let the format widen whatever is under the cursor.
    pub(super) fn widen(&mut self) {
        match self.src.widen() {
            Some(msg) => {
                self.notify(msg);
                self.clamp();
            }
            None => self.notify("nothing to widen here"),
        }
    }

    /// Move the cursor to a row that is already visible, scrolling it in.
    pub(super) fn jump_to_row(&mut self, row: usize) {
        self.cursor = row.min(self.len().saturating_sub(1));
        let h = self.content_rows().max(1);
        if self.cursor < self.top || self.cursor >= self.top + h {
            self.top = self.cursor.saturating_sub(h / 3);
        }
        self.clamp();
    }

    /// Move the cursor to a place in the document, unfolding whatever hides it.
    pub(super) fn jump_to(&mut self, anchor: Anchor) {
        match self.src.reveal(anchor) {
            Some(row) => self.jump_to_row(row),
            None => self.clamp(),
        }
    }

    /// `Tab` / `S-Tab`. On a document that reads in blocks this is the next
    /// *message* — the conversation turn — because `j`/`k` already step between
    /// blocks; everywhere else the two are the same landmark and nothing about
    /// `Tab` changes.
    ///
    /// A file whose dialect nothing recognises has no messages, and a
    /// trajectory can end in a long run of mechanics. `Tab` there falls back to
    /// the next block rather than dead-ending, and only says so when there is
    /// nothing either way — which is also what stops it running past the end,
    /// since neither answer is ever a row outside the document.
    pub(super) fn jump_heading(&mut self, forward: bool) {
        let to = self
            .src
            .next_message(self.cursor, forward)
            .or_else(|| self.src.next_landmark(self.cursor, forward));
        match to {
            Some(row) => match self.src.blocks() {
                true => self.frame_block(row),
                false => self.goto(row),
            },
            None => self.notify(match forward {
                true => "no further heading",
                false => "no previous heading",
            }),
        }
    }
}
