//! Visual mode and the yank commands (`v`, `y`, `Y`, `c`).
//!
//! The pager only *builds* the text: `main` performs the clipboard write and
//! the cache-file fallback, then reports the outcome through
//! [`Pager::notify`]. Nothing here touches a file descriptor.
#![deny(unsafe_code)]

use super::Pager;
use crate::select::{self, Selection};

impl Pager {
    /// `v`: start a selection anchored at the cursor, or drop the current one.
    pub(super) fn toggle_visual(&mut self) {
        match self.select {
            Some(_) => self.cancel_visual(),
            None if self.src.is_empty() => self.notify("nothing to select"),
            None => self.select = Some(Selection::new(self.cursor)),
        }
    }

    /// `Esc`: leave visual mode without yanking.
    pub(super) fn cancel_visual(&mut self) {
        self.select = None;
        self.notify("selection cancelled");
    }

    /// `y`: yank the visual selection, or — outside visual mode, with a link
    /// focused — that link's target.
    ///
    /// SPEC.md §Navigation requires external links to be yankable, and `n`
    /// focusing a link is the only way to name one, so `y` means "copy the
    /// focused link" whenever there is nothing selected.
    pub(super) fn yank_selection(&mut self) {
        if self.select.is_none() {
            return self.yank_link();
        }
        let rows = self.selected_rows();
        self.select = None;
        match self.src.yank_rows(rows) {
            Some(y) => self.queue_yank(y),
            None => self.notify("nothing to yank"),
        }
    }

    /// `y` with no selection: the smallest thing the format offers at the
    /// cursor — a CSV's cell — and otherwise the focused link.
    fn yank_link(&mut self) {
        if let Some(y) = self.cursor_row().and_then(|r| self.src.yank_point(r)) {
            return self.queue_yank(y);
        }
        self.yank_focused_link()
    }

    /// External URLs are copied verbatim (they are never opened); internal ones
    /// as the path relative to the index root.
    fn yank_focused_link(&mut self) {
        let target = match self.focused_link_yank() {
            Some(t) => t,
            None => {
                return self
                    .notify("nothing selected \u{2014} press v to select, or n to focus a link")
            }
        };
        match select::link_yank(&target) {
            Some(y) => self.queue_yank(y),
            None => self.notify("that link has no target to copy"),
        }
    }

    /// `Y`: yank the whole section under the cursor, heading included.
    pub(super) fn yank_section(&mut self) {
        let row = match self.cursor_row() {
            Some(r) => r,
            None => return self.notify("nothing to yank"),
        };
        self.select = None;
        match self.src.yank_section(row) {
            Some(y) => self.queue_yank(y),
            None => self.notify("no section here"),
        }
    }

    /// `c`: yank the code block under (or nearest below) the cursor, verbatim.
    pub(super) fn yank_code(&mut self) {
        let row = self.cursor;
        self.select = None;
        match self.src.yank_block(row) {
            Some(y) => self.queue_yank(y),
            None => self.notify("no code block below the cursor"),
        }
    }

    /// Test seam: the yank a command produced, without a terminal.
    #[cfg(test)]
    pub(crate) fn peek_yank(&mut self) -> Option<crate::select::Yank> {
        self.take_yank()
    }
}
