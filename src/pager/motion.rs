//! Where the cursor goes: the motions themselves, and the framing that decides
//! what the viewport shows once it gets there.
//!
//! Split out of `input.rs` — which dispatches keys to them — to keep both files
//! under the size limit; these are the same `Pager` methods, one door down.
//!
//! Every motion here counts **rows**, including `j`/`k`: a row is the unit the
//! screen is made of, and a closed block is exactly one row, so a coarser
//! default could only ever skip rows that are on the screen. Blocks are a
//! *jump* — [`Pager::jump_heading`], `Tab`/`S-Tab` — and framing them is
//! [`Pager::frame_block`].
#![deny(unsafe_code)]

use super::Pager;
use crate::source::Anchor;

impl Pager {
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

    /// `j` / `k`, and what every other vertical motion is counted in: rows.
    /// [`Pager::clamp`] drags the window along once the cursor leaves it, so a
    /// block taller than the viewport is walked a row at a time and the text
    /// moves from the press that reaches the bottom of the window onward.
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

    /// `Tab` / `S-Tab`: the **fast jump**, one structural landmark at a time —
    /// the next heading in prose, the next declaration in code, and under a
    /// lens the next **block**: a message, a shut run of mechanics, or, inside
    /// a run the reader has opened, one of its steps.
    ///
    /// Block rather than message, deliberately. A message *is* a block, so a
    /// block jump reaches every message a message jump would; the reverse is
    /// false, and a jump that skipped the runs would leave the mechanics — the
    /// thing a trajectory is mostly made of — reachable only a row at a time.
    /// It is also the general answer: one boundary, `Source::next_landmark`,
    /// for every format, rather than a second one that exists under a lens.
    ///
    /// `S-Tab` is the exact mirror: the same landmark table walked backwards,
    /// framed the same way, so `Tab` then `S-Tab` comes back to the block it
    /// started on.
    pub(super) fn jump_heading(&mut self, forward: bool) {
        match self.src.next_landmark(self.cursor, forward) {
            Some(row) => match self.src.blocks() {
                true => self.frame_block(row),
                false => self.goto(row),
            },
            // Named after the unit this document is actually read in: a
            // trajectory has no headings, and the status bar one line down is
            // printing `block 96/≥181`. Same predicate as the framing above.
            None => self.notify(match (self.src.blocks(), forward) {
                (true, true) => "no further block",
                (true, false) => "no previous block",
                (false, true) => "no further heading",
                (false, false) => "no previous heading",
            }),
        }
    }
}
