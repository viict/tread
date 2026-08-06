//! The pager's half of corpus navigation: the link cursor, following links,
//! the history stack and the index overlay.
//!
//! All path and filesystem work lives in [`crate::nav`]; this file is the glue
//! that turns key presses into navigation and puts the result back into the
//! viewport. Document state is captured into a [`Snapshot`] before every move,
//! so going back restores the scroll position, the fold set and the link cursor
//! exactly as they were.
#![deny(unsafe_code)]

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use super::{Mode, Pager};
use crate::key::{Key, KeyEvent};
use crate::nav::history::Snapshot;
use crate::nav::link::Target;
use crate::nav::Navigator;
use crate::source::{Anchor, LinkSite, Source};

impl Pager {
    /// Give the pager a corpus to walk. Relabels the status bar with the path
    /// relative to the index root.
    pub fn attach_nav(&mut self, nav: Navigator) {
        self.label = nav.label(nav.current());
        self.nav = Some(nav);
        self.dirty = true;
    }

    // -- the link cursor -----------------------------------------------------

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

    /// Text `y` puts on the clipboard when a link is focused (SPEC.md: external
    /// links are never opened, only shown and yanked).
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

    // -- following -----------------------------------------------------------

    /// Enter: follow the focused link, or fall back to toggling the section.
    pub(super) fn follow(&mut self) {
        let url = match self.focused_link() {
            Some(s) => s.url.clone(),
            None => return self.activate(),
        };
        let nav = match &self.nav {
            Some(n) => n,
            None => return self.notify(url),
        };
        match nav.resolve(&url) {
            Target::Anchor(slug) => self.goto_anchor(&slug),
            Target::Doc { path, anchor } => self.open_doc(path, anchor, true),
            Target::External(u) => self.notify(format!("external link (not opened): {u}")),
            Target::Other(p) => {
                let rel = nav.label(&p);
                self.notify(format!("not markdown: {rel}"))
            }
            Target::Broken { raw, why } => self.notify(format!("{raw}: {why}")),
        }
    }

    /// `Enter` with no link under the cursor.
    ///
    /// Ask the format for a row detail first and fall back to folding. The
    /// pager stays format-blind: markdown has no detail and folds as it always
    /// did, a record format has no folds and opens the row. Neither knows the
    /// other exists.
    fn activate(&mut self) {
        if let Some(d) = self.src.detail(self.cursor) {
            self.detail = Some(d);
            self.detail_sel = 0;
            self.mode = Mode::Detail;
            return;
        }
        // Nothing to open here, and nothing to fold either: a format with no
        // sections at all would otherwise answer with `fold`'s "no heading
        // here", which is true but reads as nonsense in a file that has no
        // headings anywhere. Asking the source how it is shaped keeps the
        // pager format-blind while still saying something the reader believes.
        match self.src.outline().is_empty() {
            true => self.notify("nothing to open here"),
            false => self.fold(None),
        }
    }

    /// Scroll to a heading by slug, expanding it if it is folded.
    pub(super) fn goto_anchor(&mut self, slug: &str) {
        match self.src.goto_id(slug) {
            Some(row) => self.jump_to_row(row),
            None => self.notify(format!("no heading #{slug}")),
        }
    }

    // -- history -------------------------------------------------------------

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            path: self
                .nav
                .as_ref()
                .map(|n| n.current().to_path_buf())
                .unwrap_or_default(),
            top: self.top,
            cursor: self.cursor,
            collapsed: self.src.folds(),
            link: self.link_cursor,
        }
    }

    pub(super) fn go_back(&mut self) {
        let here = self.snapshot();
        let prev = self.nav.as_mut().and_then(|n| n.back(here));
        match prev {
            Some(s) => self.restore(s),
            None => self.notify("no previous document"),
        }
    }

    pub(super) fn go_forward(&mut self) {
        let here = self.snapshot();
        let next = self.nav.as_mut().and_then(|n| n.forward(here));
        match next {
            Some(s) => self.restore(s),
            None => self.notify("no document to go forward to"),
        }
    }

    /// True when `q` should step back instead of quitting.
    pub(super) fn can_pop(&self) -> bool {
        self.nav.as_ref().map(|n| n.depth() > 0).unwrap_or(false)
    }

    fn restore(&mut self, s: Snapshot) {
        let same = self
            .nav
            .as_ref()
            .map(|n| n.is_current(&s.path))
            .unwrap_or(true);
        if !same {
            let doc = match self.nav.as_ref().map(|n| n.load_source(&s.path)) {
                Some(Ok(d)) => d,
                Some(Err(e)) => return self.notify(e),
                None => return,
            };
            self.swap_doc(s.path.clone(), doc);
        }
        self.src.set_folds(s.collapsed.clone());
        self.cursor = s.cursor.min(self.src.len().saturating_sub(1));
        self.top = s.top.min(self.cursor);
        self.link_cursor = s.link;
        self.clamp();
    }

    // -- opening documents ---------------------------------------------------

    pub(super) fn open_doc(&mut self, path: PathBuf, anchor: Option<String>, push: bool) {
        if self.nav.as_ref().map(|n| n.is_current(&path)).unwrap_or(false) {
            if let Some(a) = anchor {
                self.goto_anchor(&a);
            }
            return;
        }
        let doc = match self.nav.as_ref().map(|n| n.load_source(&path)) {
            Some(Ok(d)) => d,
            Some(Err(e)) => return self.notify(e),
            None => return self.notify("no corpus attached"),
        };
        if push {
            let here = self.snapshot();
            if let Some(n) = self.nav.as_mut() {
                n.push(here);
            }
        }
        self.swap_doc(path, doc);
        if let Some(a) = anchor {
            self.goto_anchor(&a);
        }
    }

    /// Replace the document being shown, resetting per-document state.
    ///
    /// The pager takes a `Box<dyn Source>` and never asks what is inside it;
    /// which format a corpus document is in is [`Navigator`]'s business.
    fn swap_doc(&mut self, path: PathBuf, src: Box<dyn Source>) {
        self.src = src;
        // The typed query outlives the document: the new one is searched for
        // the same text, exactly as the old eager rescan did.
        self.src.set_query(&self.query);
        self.link_cursor = None;
        self.cursor = 0;
        self.top = 0;
        self.hoff = 0;
        if let Some(n) = self.nav.as_mut() {
            n.set_current(path.clone());
        }
        self.label = match &self.nav {
            Some(n) => n.label(&path),
            None => path.to_string_lossy().into_owned(),
        };
        self.relayout();
        self.cursor = 0;
        self.top = 0;
        self.clamp();
    }

    /// `]` / `[`: the next or previous document in index order.
    pub(super) fn step_doc(&mut self, delta: isize) {
        let next = self.nav.as_ref().and_then(|n| n.sibling(delta));
        match next {
            Some(p) => self.open_doc(p, None, true),
            None => self.notify(match delta > 0 {
                true => "no next document in the index",
                false => "no previous document in the index",
            }),
        }
    }

    // -- the index overlay ---------------------------------------------------

    pub(super) fn open_index(&mut self) {
        let (empty, at) = match &self.nav {
            Some(n) => (
                n.entries().is_empty(),
                n.position_of(n.current()),
            ),
            None => (true, None),
        };
        if empty {
            return self.notify("no corpus index");
        }
        self.mode = Mode::Index;
        self.index_filter.clear();
        self.index_typing = false;
        self.index_sel = at.unwrap_or(0);
    }

    /// Index entries surviving the `/` filter, as indices into `nav.entries()`.
    pub(crate) fn index_rows(&self) -> Vec<usize> {
        let nav = match &self.nav {
            Some(n) => n,
            None => return Vec::new(),
        };
        let needle = self.index_filter.to_lowercase();
        nav.entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| needle.is_empty() || e.haystack().contains(&needle))
            .map(|(i, _)| i)
            .collect()
    }

    /// Where the selection sits inside the filtered row list.
    pub(crate) fn index_row_pos(&self) -> usize {
        self.index_rows()
            .iter()
            .position(|i| *i == self.index_sel)
            .unwrap_or(0)
    }

    /// Keys consumed by the index overlay. Returns true when handled.
    pub(super) fn index_key(&mut self, ev: KeyEvent) -> bool {
        if self.index_typing {
            return self.index_filter_key(ev);
        }
        let rows = self.index_rows();
        let page = self.body_rows().max(1);
        match ev.key {
            Key::Esc | Key::Char('q') | Key::Char('i') => self.mode = Mode::Normal,
            Key::Char('j') | Key::Down => self.index_move(&rows, 1),
            Key::Char('k') | Key::Up => self.index_move(&rows, -1),
            Key::PageDown | Key::Char('f') => self.index_move(&rows, page as isize),
            Key::PageUp | Key::Char('b') => self.index_move(&rows, -(page as isize)),
            Key::Char('g') | Key::Home => self.index_move(&rows, isize::MIN / 2),
            Key::Char('G') | Key::End => self.index_move(&rows, isize::MAX / 2),
            Key::Char('/') => {
                self.index_typing = true;
                self.index_filter.clear();
            }
            Key::Enter => self.index_open(),
            _ => {}
        }
        true
    }

    fn index_filter_key(&mut self, ev: KeyEvent) -> bool {
        match ev.key {
            Key::Esc => {
                self.index_typing = false;
                self.index_filter.clear();
            }
            Key::Enter => self.index_typing = false,
            Key::Backspace => {
                if self.index_filter.pop().is_none() {
                    self.index_typing = false;
                }
            }
            Key::Char(c) => self.index_filter.push(c),
            _ => return true,
        }
        // Keep the selection on a row that survived the filter.
        if let Some(first) = self.index_rows().first() {
            if !self.index_rows().contains(&self.index_sel) {
                self.index_sel = *first;
            }
        }
        true
    }

    fn index_move(&mut self, rows: &[usize], delta: isize) {
        if rows.is_empty() {
            return;
        }
        let at = rows
            .iter()
            .position(|i| *i == self.index_sel)
            .unwrap_or(0) as isize;
        let next = at.saturating_add(delta).clamp(0, rows.len() as isize - 1);
        self.index_sel = rows[next as usize];
    }

    fn index_open(&mut self) {
        self.mode = Mode::Normal;
        let entry = self
            .nav
            .as_ref()
            .and_then(|n| n.entries().get(self.index_sel))
            .map(|e| (e.path.clone(), e.anchor.clone()));
        if let Some((path, anchor)) = entry {
            self.open_doc(path, anchor, true);
        }
    }
}
