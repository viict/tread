//! The link cursor: which link on the row is focused, and the two motions that
//! move it.
//!
//! Split out of `navigate.rs` because it is a distinct idea from *following* a
//! link: everything here is about the cursor and the row it sits on, and none of
//! it touches a path, the history stack or the filesystem. Both motions live
//! together on purpose — `n`/`N` walk the document's links and move the cursor to
//! do it, `\u{2190}`/`\u{2192}` walk one row and never move the cursor at all
//! (SPEC.md §"Selecting links on a line") — and they share the sticky focus
//! index, which is the state that makes either of them repeatable.
#![deny(unsafe_code)]

use super::{Pager, HSTEP};
use crate::source::{Anchor, LinkSite};

impl Pager {
    /// The focused link: the one `n` last landed on if it is still under the
    /// cursor row, otherwise the first link on that row.
    pub(crate) fn focus_index(&self) -> Option<usize> {
        let row = self.cursor_row()?;
        let here = |site: &LinkSite| self.src.row_of(site.anchor) == Some(row);
        if let Some(i) = self.link_cursor {
            if self.src.links().get(i).map(&here).unwrap_or(false) {
                return Some(i);
            }
        }
        self.src.links().iter().position(here)
    }

    pub(crate) fn focused_link(&self) -> Option<&LinkSite> {
        self.src.links().get(self.focus_index()?)
    }

    /// What the status bar shows for the focused link: the resolved path for
    /// internal links, the raw URL for external ones.
    pub(crate) fn link_status(&self) -> Option<String> {
        let site = self.focused_link()?;
        Some(match &self.nav {
            Some(nav) => nav.resolve(&site.url).describe(nav.root()),
            None => site.url.clone(),
        })
    }

    /// Text `y` puts on the clipboard when a link is focused: the resolved path
    /// for an internal link, the raw URL for an external one. Yankable whatever
    /// `Enter` would do with it — including a scheme the allowlist refuses, which
    /// is the only way to get a `file:` URL out of a document at all.
    pub fn focused_link_yank(&self) -> Option<String> {
        let site = self.focused_link()?;
        Some(match &self.nav {
            Some(nav) => nav.resolve(&site.url).yank_text(nav.root()),
            None => site.url.clone(),
        })
    }

    /// `n` / `N`: move to the next or previous link and scroll it into view.
    pub(super) fn step_link(&mut self, forward: bool) {
        if self.src.links().is_empty() {
            return self.notify("no links in this document");
        }
        let here = self.focus_index();
        let next = match (here, forward) {
            (Some(i), true) => i + 1,
            (Some(0), false) => return self.notify("no previous link"),
            (Some(i), false) => i - 1,
            (None, _) => self.nearest_link(forward),
        };
        match self.src.links().get(next).map(|s| s.anchor) {
            Some(anchor) => {
                self.link_cursor = Some(next);
                self.jump_to(anchor);
            }
            None => self.notify(match forward {
                true => "no further link",
                false => "no previous link",
            }),
        }
    }

    /// Every link on the cursor's row, as indices into `src.links()`, in
    /// reading order.
    pub(crate) fn row_links(&self) -> Vec<usize> {
        let row = match self.cursor_row() {
            Some(r) => r,
            None => return Vec::new(),
        };
        self.src
            .links()
            .iter()
            .enumerate()
            .filter(|(_, s)| self.src.row_of(s.anchor) == Some(row))
            .map(|(i, _)| i)
            .collect()
    }

    /// `←` / `→` on a row that does not scroll: move the link focus along the
    /// row (SPEC.md §"Selecting links on a line").
    ///
    /// The motion **stops at the row's ends** rather than carrying to the next
    /// row's links. That is the whole reason the binding exists: `n`/`N` already
    /// walk links document-wide and move the cursor to do it, so an arrow that
    /// spilled onto the next row would be a second, worse `n` — and it would
    /// take the cursor off the line the reader is reading, which is exactly what
    /// SPEC.md asks these keys to avoid ("so a line holding several links can be
    /// walked without `n` carrying the cursor off it"). Nothing vertical ever
    /// happens here.
    ///
    /// Silent at both ends, and silent on a row with no links at all: these are
    /// held-down keys, and a message on every press would be noise rather than
    /// information (SPEC.md: "does nothing, silently").
    pub(super) fn step_row_link(&mut self, forward: bool) {
        let row = self.row_links();
        if row.is_empty() {
            return;
        }
        // With no sticky focus the first link on the row is the one shown, so
        // that is where the walk starts from.
        let at = self
            .focus_index()
            .and_then(|i| row.iter().position(|&r| r == i))
            .unwrap_or(0);
        let next = match forward {
            true => (at + 1).min(row.len() - 1),
            false => at.saturating_sub(1),
        };
        self.link_cursor = Some(row[next]);
    }

    /// First link at or after (before) the cursor row when none is focused.
    fn nearest_link(&self, forward: bool) -> usize {
        let here = self.cursor_anchor().unwrap_or(Anchor(0));
        let links = self.src.links();
        match forward {
            true => links
                .iter()
                .position(|s| s.anchor >= here)
                .unwrap_or(links.len()),
            false => links
                .iter()
                .rposition(|s| s.anchor <= here)
                .unwrap_or(usize::MAX),
        }
    }

    /// `\u{2190}` / `\u{2192}`. Two jobs, one key, decided by the row under the
    /// cursor (SPEC.md §"Selecting links on a line").
    ///
    /// **Links win where there is a choice to make.** A row carrying more than
    /// one link gets the link walk, whether or not it also scrolls, because that
    /// is the motion nothing else in the keymap can make: `n` carries the cursor
    /// to the next link *anywhere*, and a table row holding four links needs a
    /// way to pick the third without leaving the line. Scrolling such a row is
    /// still one keypress away — `h`/`l` scroll everywhere, unconditionally, and
    /// SPEC.md promises they do.
    ///
    /// Every other row scrolls if it can: there is text off-screen and reaching
    /// it is what an arrow obviously means. That covers a code block, a CSV row,
    /// a text line, and a table row with one link or none. A row that neither
    /// scrolls nor holds a second link is silent.
    ///
    /// This precedence used to be the other way round, on the strength of
    /// SPEC.md's claim that "the two cases never apply to the same row". They do:
    /// [`crate::render::table`] marks *every* row of a table wider than the
    /// terminal scrollable, the rows holding links included, and so does any
    /// prose row under a `--width` greater than the terminal's. Under the old
    /// order the arrows could never select a link on a wide linked table — the
    /// one case the binding was written for, in a corpus SPEC.md describes as
    /// table-heavy with its links in tables. SPEC.md's premise has been corrected
    /// to match.
    ///
    /// "Scrollable" is [`super::scrollable`], the same predicate the painter uses
    /// to draw the cut markers and [`Pager::max_hoff`] uses to bound the offset —
    /// so the arrows scroll exactly the rows that visibly can, and no second
    /// notion of scrollability exists to drift from it.
    pub(super) fn arrow(&mut self, forward: bool) {
        if self.row_links().len() > 1 {
            return self.step_row_link(forward);
        }
        if self.cursor_scrolls() {
            let step = HSTEP as isize;
            return self.scroll_h(if forward { step } else { -step });
        }
        self.step_row_link(forward);
    }

    /// Does the cursor's own row scroll horizontally?
    pub(super) fn cursor_scrolls(&mut self) -> bool {
        let view = self.cols.max(1);
        let row = self.cursor;
        match self.src.line(row) {
            Some(line) => super::scrollable(&line, view),
            None => false,
        }
    }
}
