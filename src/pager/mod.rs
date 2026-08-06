//! The interactive pager: viewport state, scrolling, the collapse tree,
//! search, and the outline/help overlays.
//!
//! The pager is a pure state machine. It owns no file descriptors and performs
//! no I/O: `main` feeds it decoded [`KeyEvent`]s and resize notifications, and
//! asks it to paint into a [`Frame`]. That is what makes all of the logic below
//! unit-testable without a terminal.
#![deny(unsafe_code)]

pub mod collapse;
mod input;
pub mod keys;
pub mod navigate;
pub mod search;
mod view;
mod yank;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use crate::dump::layout_width;
use crate::md::Document;
use crate::render::{render_document, Line, RenderOpts};
use crate::nav::Navigator;
use crate::select::{Selection, Yank};
use crate::term::Frame;
use collapse::HeadingRef;
use navigate::LinkSite;
use search::{Dir, Match};

/// How long a transient status message stays up (SPEC.md §Status bar).
pub const MESSAGE_TTL: Duration = Duration::from_millis(2000);
/// Columns moved per `h`/`l`.
pub const HSTEP: usize = 4;

/// What the pager is showing on top of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Outline,
    Help,
    /// The corpus index list view (`i`).
    Index,
    /// Typing a search query; the field remembers the direction.
    Search(Dir),
}

pub struct Pager {
    pub(crate) doc: Document,
    pub(crate) label: String,
    width_override: Option<usize>,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) width: usize,
    pub(crate) lines: Vec<Line>,
    /// Indices into `lines` that are not hidden by a fold.
    pub(crate) visible: Vec<usize>,
    /// `(heading line index, hidden line count)` for each visible fold.
    pub(crate) counts: Vec<(usize, usize)>,
    /// Folded heading ids. Keyed by id so folds survive re-layout.
    pub(crate) collapsed: Vec<String>,
    pub(crate) top: usize,
    /// Cursor position as an index into `visible`.
    pub(crate) cursor: usize,
    pub(crate) hoff: usize,
    pub(crate) mode: Mode,
    pub(crate) query: String,
    pub(crate) dir: Dir,
    pub(crate) matches: Vec<Match>,
    pub(crate) current: Option<usize>,
    pub(crate) outline: Vec<HeadingRef>,
    pub(crate) outline_sel: usize,
    pub(crate) help_top: usize,
    /// The corpus, when one was discovered. `None` for a lone file or stdin.
    pub(crate) nav: Option<Navigator>,
    /// Every link in the rendered document, recomputed on re-layout.
    pub(crate) links: Vec<LinkSite>,
    /// Sticky link focus set by `n`/`N`; an index into `links`.
    pub(crate) link_cursor: Option<usize>,
    /// Selected index entry (an index into `nav.entries()`).
    pub(crate) index_sel: usize,
    pub(crate) index_filter: String,
    pub(crate) index_typing: bool,
    /// Visual line-select state (`v`); `None` outside visual mode.
    pub(crate) select: Option<Selection>,
    /// Text produced by `y`/`Y`/`c`, waiting for `main` to put it on the
    /// clipboard. The pager itself performs no I/O.
    pending_yank: Option<Yank>,
    pub(crate) message: Option<String>,
    message_at: Option<Instant>,
    pending: Option<char>,
    search_origin: usize,
    dirty: bool,
    quit: bool,
}

impl Pager {
    /// `width_override` is `--width`; `cols`/`rows` are the terminal size.
    pub fn new(
        doc: Document,
        label: String,
        cols: usize,
        rows: usize,
        width_override: Option<usize>,
    ) -> Pager {
        let mut p = Pager {
            doc,
            label,
            width_override,
            cols,
            rows,
            width: 80,
            lines: Vec::new(),
            visible: Vec::new(),
            counts: Vec::new(),
            collapsed: Vec::new(),
            top: 0,
            cursor: 0,
            hoff: 0,
            mode: Mode::Normal,
            query: String::new(),
            dir: Dir::Forward,
            matches: Vec::new(),
            current: None,
            outline: Vec::new(),
            outline_sel: 0,
            help_top: 0,
            nav: None,
            links: Vec::new(),
            link_cursor: None,
            index_sel: 0,
            index_filter: String::new(),
            index_typing: false,
            select: None,
            pending_yank: None,
            message: None,
            message_at: None,
            pending: None,
            search_origin: 0,
            dirty: true,
            quit: false,
        };
        p.relayout();
        p
    }

    // -- queries ------------------------------------------------------------

    pub fn should_quit(&self) -> bool {
        self.quit
    }
    /// Rows available for document content, above the status bar.
    pub fn body_rows(&self) -> usize {
        self.rows.saturating_sub(1)
    }
    /// Index into `lines` of the row under the cursor.
    pub fn cursor_line(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }
    pub fn paint(&self, frame: &mut Frame) {
        view::paint(self, frame);
    }

    /// Expire the transient status message. Call once per event-loop turn.
    pub fn tick(&mut self) {
        if let Some(at) = self.message_at {
            if at.elapsed() >= MESSAGE_TTL {
                self.message = None;
                self.message_at = None;
                self.dirty = true;
            }
        }
    }

    /// The selected rows, as indices into `lines`. Empty outside visual mode.
    pub(crate) fn selected_rows(&self) -> Vec<usize> {
        let sel = match self.select {
            Some(s) => s,
            None => return Vec::new(),
        };
        let (lo, hi) = sel.range();
        let hi = hi.min(self.visible.len().saturating_sub(1));
        match lo > hi {
            true => Vec::new(),
            false => self.visible[lo..=hi].to_vec(),
        }
    }

    /// Take the text a yank produced, for `main` to hand to the clipboard.
    /// The pager owns no file descriptors, so it cannot send it itself.
    pub fn take_yank(&mut self) -> Option<Yank> {
        self.pending_yank.take()
    }

    pub(crate) fn queue_yank(&mut self, yank: Yank) {
        self.pending_yank = Some(yank);
        self.dirty = true;
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.message = Some(msg.into());
        self.message_at = Some(Instant::now());
        self.dirty = true;
    }

    // -- layout -------------------------------------------------------------

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if (cols, rows) == (self.cols, self.rows) {
            self.dirty = true;
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.relayout();
    }

    /// Re-render at the current width, keeping the cursor on the same source
    /// line and the fold state (which is keyed by heading id) intact.
    fn relayout(&mut self) {
        // Selection indices address the pre-layout `visible` list; a re-wrap
        // moves every row, so the selection cannot survive it.
        self.select = None;
        let anchor = self.cursor_line().map(|i| self.lines[i].source_line);
        self.width = layout_width(self.width_override, Some(self.cols));
        self.lines = render_document(&self.doc, &RenderOpts::new(self.width));
        self.outline = collapse::headings(&self.lines);
        self.links = navigate::sites(&self.lines);
        if self.link_cursor.map(|i| i >= self.links.len()).unwrap_or(false) {
            self.link_cursor = None;
        }
        self.refresh_view();
        if let Some(src) = anchor {
            let at = self
                .visible
                .iter()
                .position(|i| self.lines[*i].source_line >= src)
                .unwrap_or(self.visible.len().saturating_sub(1));
            self.cursor = at;
        }
        self.rescan();
        self.clamp();
        self.dirty = true;
    }

    /// Recompute the visible line list from the fold state.
    fn refresh_view(&mut self) {
        self.visible = collapse::visible_lines(&self.lines, &self.collapsed);
        self.counts = collapse::fold_counts(&self.lines, &self.collapsed);
        self.clamp();
    }

    fn clamp(&mut self) {
        let n = self.visible.len();
        self.cursor = self.cursor.min(n.saturating_sub(1));
        if n == 0 {
            self.cursor = 0;
            self.top = 0;
            self.hoff = 0;
            return;
        }
        let h = self.body_rows().max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        }
        if self.cursor >= self.top + h {
            self.top = self.cursor + 1 - h;
        }
        self.top = self.top.min(n.saturating_sub(h).min(n - 1));
        self.hoff = self.hoff.min(self.max_hoff());
        self.dirty = true;
    }

    /// Furthest useful horizontal offset: the widest row currently in the
    /// viewport, minus the viewport width.
    ///
    /// Any row that does not fit counts, not just the ones the renderer marked
    /// `scroll`. `--width 200` in an 80-column terminal lays out *wrapped* rows
    /// that are still wider than the viewport; without this they would be
    /// clipped with no way to reach the hidden text (SPEC.md §CLI `--width`).
    pub fn max_hoff(&self) -> usize {
        let h = self.body_rows();
        let w = self.cols.max(1);
        self.visible
            .iter()
            .skip(self.top)
            .take(h)
            .map(|i| self.lines[*i].width().saturating_sub(w))
            .max()
            .unwrap_or(0)
    }
}

/// True when a row must be scrolled horizontally rather than shown whole: the
/// renderer said so (code, wide tables), or it is simply wider than the
/// viewport. Shared by the painter and [`Pager::max_hoff`] so the cut markers
/// and the reachable offset can never disagree.
pub(crate) fn scrollable(line: &Line, viewport: usize) -> bool {
    line.scroll || line.width() > viewport
}

#[cfg(test)]
impl Pager {
    pub(crate) fn line_count(&self) -> usize {
        self.visible.len()
    }
    /// True when a frame should be repainted.
    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }
    /// True while `v` visual line-select mode is active.
    pub(crate) fn in_visual(&self) -> bool {
        self.select.is_some()
    }
    /// The text the status bar would show right now.
    pub(crate) fn status_line(&self) -> String {
        view::status_text(self)
    }
    pub(crate) fn cursor_text(&self) -> String {
        self.cursor_line()
            .map(|i| self.lines[i].text().trim().to_string())
            .unwrap_or_default()
    }
    pub(crate) fn visible_text(&self) -> Vec<String> {
        self.visible
            .iter()
            .map(|i| self.lines[*i].text().trim().to_string())
            .collect()
    }
}
