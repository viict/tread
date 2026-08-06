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
use crate::md::Document;
use crate::nav::history::Snapshot;
use crate::nav::link::Target;
use crate::nav::Navigator;
use crate::render::Line;

/// One link occurrence in the rendered document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSite {
    /// Index into `Pager::lines`.
    pub line: usize,
    /// Display column the link starts at.
    pub col: usize,
    pub url: String,
}

/// Every link in the rendered document, in reading order.
pub fn sites(lines: &[Line]) -> Vec<LinkSite> {
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        for (col, url) in l.links() {
            out.push(LinkSite {
                line: i,
                col,
                url: url.to_string(),
            });
        }
    }
    out
}

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
        let line = self.cursor_line()?;
        if let Some(i) = self.link_cursor {
            if self.links.get(i).map(|s| s.line) == Some(line) {
                return Some(i);
            }
        }
        self.links.iter().position(|s| s.line == line)
    }

    pub(crate) fn focused_link(&self) -> Option<&LinkSite> {
        self.links.get(self.focus_index()?)
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
        if self.links.is_empty() {
            return self.notify("no links in this document");
        }
        let here = self.focus_index();
        let next = match (here, forward) {
            (Some(i), true) => i + 1,
            (Some(0), false) => return self.notify("no previous link"),
            (Some(i), false) => i - 1,
            (None, _) => self.nearest_link(forward),
        };
        match self.links.get(next) {
            Some(_) => {
                self.link_cursor = Some(next);
                let line = self.links[next].line;
                self.jump_to(line);
            }
            None => self.notify(match forward {
                true => "no further link",
                false => "no previous link",
            }),
        }
    }

    /// First link at or after (before) the cursor row when none is focused.
    fn nearest_link(&self, forward: bool) -> usize {
        let line = self.cursor_line().unwrap_or(0);
        match forward {
            true => self
                .links
                .iter()
                .position(|s| s.line >= line)
                .unwrap_or(self.links.len()),
            false => self
                .links
                .iter()
                .rposition(|s| s.line <= line)
                .unwrap_or(usize::MAX),
        }
    }

    // -- following -----------------------------------------------------------

    /// Enter: follow the focused link, or fall back to toggling the section.
    pub(super) fn follow(&mut self) {
        let url = match self.focused_link() {
            Some(s) => s.url.clone(),
            None => return self.fold(None),
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

    /// Scroll to a heading by slug, expanding it if it is folded.
    pub(super) fn goto_anchor(&mut self, slug: &str) {
        let found = self
            .lines
            .iter()
            .position(|l| matches!(&l.heading, Some(h) if h.id == slug));
        match found {
            Some(i) => {
                self.collapsed.retain(|c| c != slug);
                self.refresh_view();
                self.jump_to(i);
            }
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
            collapsed: self.collapsed.clone(),
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
            .map(|n| n.current() == s.path)
            .unwrap_or(true);
        if !same {
            let doc = match self.nav.as_ref().map(|n| n.load(&s.path)) {
                Some(Ok(d)) => d,
                Some(Err(e)) => return self.notify(e),
                None => return,
            };
            self.swap_doc(s.path.clone(), doc);
        }
        self.collapsed = s.collapsed.clone();
        self.refresh_view();
        self.cursor = s.cursor.min(self.visible.len().saturating_sub(1));
        self.top = s.top.min(self.cursor);
        self.link_cursor = s.link;
        self.clamp();
    }

    // -- opening documents ---------------------------------------------------

    pub(super) fn open_doc(&mut self, path: PathBuf, anchor: Option<String>, push: bool) {
        if self.nav.as_ref().map(|n| n.current() == path).unwrap_or(false) {
            if let Some(a) = anchor {
                self.goto_anchor(&a);
            }
            return;
        }
        let doc = match self.nav.as_ref().map(|n| n.load(&path)) {
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
    fn swap_doc(&mut self, path: PathBuf, doc: Document) {
        self.doc = doc;
        self.collapsed.clear();
        self.link_cursor = None;
        self.cursor = 0;
        self.top = 0;
        self.hoff = 0;
        self.matches.clear();
        self.current = None;
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
                n.entries().iter().position(|e| e.path == n.current()),
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
