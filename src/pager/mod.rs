//! The interactive pager: viewport state, scrolling, search UI and the
//! outline/help overlays.
//!
//! The pager is a pure state machine. It owns no file descriptors and performs
//! no I/O: `main` feeds it decoded [`KeyEvent`]s and resize notifications, and
//! asks it to paint into a [`Frame`]. That is what makes all of the logic below
//! unit-testable without a terminal.
//!
//! # What the pager owns, and what the format owns
//!
//! Everything the pager holds is *viewport* state — where the window is, where
//! the cursor is, what is selected, what mode we are in. Everything that
//! depends on what a document **is** — how it lays out, what a section is, what
//! a match is, what yanked text should look like — lives behind
//! [`crate::source::Source`] (SPEC.md §The `Source` seam). So the collapse
//! tree, the outline, the link list and the search index moved out of here:
//! "the rows this heading hides" is not a fact about a viewport, and a table
//! format's answer is nothing like markdown's. The cursor row, the scroll
//! offsets and the visual selection stayed: they are the same idea in every
//! format, and a source that owned them would have to re-implement clamping.
#![deny(unsafe_code)]

mod input;
pub mod keys;
mod link;
pub mod navigate;
mod view;
mod yank;

#[cfg(test)]
mod probe;
#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use crate::dump::layout_width;
use crate::nav::Navigator;
use crate::select::{Selection, Yank};
use crate::source::{Anchor, End, Source};
use crate::term::Frame;

// The collapse tree and the search index live under `source` now, but they are
// still the pager's neighbours in every other sense — `view`, `input` and
// `select` name them through here, and the search direction is part of the
// pager's own `Mode`.
pub use crate::source::{collapse, search};

use search::Dir;

/// How long a transient status message stays up (SPEC.md §Status bar).
pub const MESSAGE_TTL: Duration = Duration::from_millis(2000);
/// Columns moved per `h`/`l`.
pub const HSTEP: usize = 4;
/// Wall clock one idle tick may spend driving a `G` scan.
///
/// A budget in *time* rather than in bytes is what makes the promise the same
/// on a slow disk as on a fast one: whatever the file, the next key press waits
/// at most this long. It is also the scan's step size, so a 1GB file is about
/// forty of these rather than one freeze.
const SEEK_SLICE: Duration = Duration::from_millis(200);

/// What the pager is showing on top of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Outline,
    Help,
    /// The corpus index list view (`i`).
    Index,
    /// One row expanded into labelled fields (`Enter`, in a record format).
    Detail,
    /// Typing a search query; the field remembers the direction.
    Search(Dir),
}

pub struct Pager {
    /// The document, whatever format it is in. The pager never asks.
    pub(crate) src: Box<dyn Source>,
    pub(crate) label: String,
    width_override: Option<usize>,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) width: usize,
    pub(crate) top: usize,
    /// Cursor position, as a row of the source's visible rows.
    pub(crate) cursor: usize,
    pub(crate) hoff: usize,
    pub(crate) mode: Mode,
    pub(crate) query: String,
    pub(crate) dir: Dir,
    pub(crate) outline_sel: usize,
    /// The row the detail overlay is showing, and how far it is scrolled. Held
    /// here rather than re-fetched per frame: the row is a file read.
    pub(crate) detail: Option<crate::source::Detail>,
    pub(crate) detail_sel: usize,
    pub(crate) help_top: usize,
    /// The corpus, when one was discovered. `None` for a lone file or stdin.
    pub(crate) nav: Option<Navigator>,
    /// Sticky link focus set by `n`/`N`; an index into `src.links()`.
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
    /// A URL `Enter` accepted for the system opener, waiting for `main` to hand
    /// it to [`crate::sys::browser`] — the same arrangement as `pending_yank`,
    /// and for the same reason: the pager starts no processes (SPEC.md §"Opening
    /// a link outside the reader").
    pending_open: Option<String>,
    /// False under `--no-browser`: an external link is then shown and refused
    /// instead of opened.
    browser: bool,
    pub(crate) message: Option<String>,
    message_at: Option<Instant>,
    pending: Option<char>,
    /// Where `/` was pressed, so Esc can go back there.
    search_origin: Option<Anchor>,
    /// `G` is waiting for the format to find the end of the document. Driven
    /// from the idle tick, abandoned by the next key press.
    seek_end: bool,
    dirty: bool,
    quit: bool,
}

impl Pager {
    /// `width_override` is `--width`; `cols`/`rows` are the terminal size.
    pub fn new(
        src: Box<dyn Source>,
        label: String,
        cols: usize,
        rows: usize,
        width_override: Option<usize>,
    ) -> Pager {
        let mut p = Pager {
            src,
            label,
            width_override,
            cols,
            rows,
            width: 80,
            top: 0,
            cursor: 0,
            hoff: 0,
            mode: Mode::Normal,
            query: String::new(),
            dir: Dir::Forward,
            outline_sel: 0,
            detail: None,
            detail_sel: 0,
            help_top: 0,
            nav: None,
            link_cursor: None,
            index_sel: 0,
            index_filter: String::new(),
            index_typing: false,
            select: None,
            pending_yank: None,
            pending_open: None,
            browser: true,
            message: None,
            message_at: None,
            pending: None,
            search_origin: None,
            seek_end: false,
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
    /// Rows the source freezes at the top of the viewport — a CSV's header
    /// (SPEC.md §CSV). Zero for markdown, and never so many that there is no
    /// scrolling window left.
    pub(crate) fn pinned(&self) -> usize {
        let n = self.src.len();
        self.src
            .pinned()
            .min(self.body_rows().saturating_sub(1))
            .min(n.saturating_sub(1))
    }
    /// Rows of the scrolling window, the pinned rows excluded.
    pub(crate) fn content_rows(&self) -> usize {
        self.body_rows() - self.pinned()
    }
    /// Rows the document currently shows, folds applied.
    pub(crate) fn len(&self) -> usize {
        self.src.len()
    }
    /// The row under the cursor, or `None` when the document is empty.
    pub(crate) fn cursor_row(&self) -> Option<usize> {
        match self.cursor < self.src.len() {
            true => Some(self.cursor),
            false => None,
        }
    }
    /// An anchor for the cursor row, for the source calls that take one.
    pub(crate) fn cursor_anchor(&self) -> Option<Anchor> {
        self.src.anchor(self.cursor)
    }
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }
    pub fn paint(&mut self, frame: &mut Frame) {
        view::paint(self, frame);
    }

    /// Idle work: let a format that is still discovering its document — a
    /// CSV's lazy row index — spend one bounded slice on it. Called from the
    /// input loop's timeout, so it must never block (SPEC.md §CSV: `q` may
    /// never wait on a scan).
    pub fn idle(&mut self) {
        if self.seek_end {
            return self.drive_seek();
        }
        // `extend` is true while there is more to discover, which is also
        // exactly while the status bar's `\u{2265}N` keeps changing.
        if self.src.extend() {
            self.dirty = true;
        }
    }

    /// `G` on a document whose end is not known yet: one [`SEEK_SLICE`] of
    /// scanning per idle tick, then ask again. Between slices the event loop
    /// gets control back, which is the whole of "interruptible" — a key press
    /// lands, [`Pager::handle`] clears the flag, and `q` exits as promptly as
    /// it does on any other file (SPEC.md §CSV).
    fn drive_seek(&mut self) {
        let start = Instant::now();
        while start.elapsed() < SEEK_SLICE && self.src.extend() {}
        self.dirty = true;
        self.goto_end();
    }

    /// `G`. Lands immediately when the format knows where the end is, and
    /// otherwise starts (or continues) the scan, reporting how far it has got.
    pub(crate) fn goto_end(&mut self) {
        match self.src.end() {
            End::At(row) => {
                // A scan that just finished leaves its last percentage on the
                // status bar; the arrival is the answer, so take it down rather
                // than let "scanning… 98%" sit there over the last row.
                if std::mem::take(&mut self.seek_end) {
                    self.message = None;
                    self.message_at = None;
                }
                self.goto(row);
            }
            End::Scanning(pct) => {
                self.seek_end = true;
                // Re-issued every slice, so the message never times out while
                // the scan is running and the percentage counts up in place.
                self.notify(format!(
                    "scanning to end of file\u{2026} {pct}%  \u{b7}  any key stops"
                ));
            }
        }
    }

    /// Abandon a running `G` scan. Whatever was indexed is kept, so pressing
    /// `G` again resumes rather than starts over.
    pub(crate) fn stop_seek(&mut self) -> bool {
        std::mem::take(&mut self.seek_end)
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

    /// The selected rows, as a half-open row range. Empty outside visual mode.
    pub(crate) fn selected_rows(&self) -> std::ops::Range<usize> {
        let sel = match self.select {
            Some(s) => s,
            None => return 0..0,
        };
        let (lo, hi) = sel.range();
        let hi = hi.min(self.src.len().saturating_sub(1));
        match lo > hi {
            true => 0..0,
            false => lo..hi + 1,
        }
    }

    /// Take the text a yank produced, for `main` to hand to the clipboard.
    /// The pager owns no file descriptors, so it cannot send it itself.
    pub fn take_yank(&mut self) -> Option<Yank> {
        self.pending_yank.take()
    }

    /// `--no-browser`: `false` restores the documented old behaviour of showing
    /// an external link's URL and refusing to open it.
    pub fn set_browser(&mut self, enabled: bool) {
        self.browser = enabled;
    }

    /// Take the URL `Enter` accepted for the system opener, for `main` to spawn.
    /// The pager starts no processes itself, so this is how the one process the
    /// feature runs stays outside the state machine — and how every test on this
    /// path proves the decision without launching anything.
    pub fn take_open(&mut self) -> Option<String> {
        self.pending_open.take()
    }

    pub(crate) fn queue_open(&mut self, url: String) {
        self.pending_open = Some(url);
        self.dirty = true;
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

    /// Re-render at the current width, keeping the cursor on the same content
    /// and the fold state (which the source keys by id) intact.
    pub(crate) fn relayout(&mut self) {
        // Selection indices address the pre-layout rows; a re-wrap moves every
        // row, so the selection cannot survive it.
        self.select = None;
        let mark = self.src.mark(self.cursor);
        // Prose gets a reading measure; a grid gets the whole terminal. Either
        // way `--width` wins (SPEC.md §CLI).
        self.width = match (self.width_override, self.src.full_width()) {
            (Some(w), _) => layout_width(Some(w), Some(self.cols)),
            (None, true) => self.cols.max(crate::render::MIN_WIDTH),
            (None, false) => layout_width(None, Some(self.cols)),
        };
        self.src.set_width(self.width);
        // The query survives a re-layout; the match list behind it does not,
        // and the source rebuilt it in `set_width`.
        if self.link_cursor.map(|i| i >= self.src.links().len()).unwrap_or(false) {
            self.link_cursor = None;
        }
        if let Some(at) = mark.and_then(|m| self.src.locate(m)) {
            self.cursor = at;
        }
        self.clamp();
        self.dirty = true;
    }

    /// The source's fold state just changed, so the row count did too.
    ///
    /// Re-clamp the viewport against the new count *before* anything reads
    /// `top`. Folding a document shut can leave `top` far past its last row,
    /// and a jump that then asks "is the cursor on screen?" against that stale
    /// `top` settles the window rows away from where the pre-seam pager put it:
    /// the old `refresh_view` clamped as part of every fold mutation, and
    /// nothing about markdown's behaviour may change behind the trait
    /// (SPEC.md §The `Source` seam).
    pub(crate) fn folds_changed(&mut self) {
        self.clamp();
    }

    /// `zt`: the raw thing under the cursor, where the format has one — a
    /// record behind a lens row. The same shape `a` has, and for the same
    /// reason: the source answers, or the pager says there was nothing there
    /// rather than appearing to do nothing.
    fn open_tree(&mut self) {
        match self.src.toggle_tree(self.cursor) {
            Some(msg) => {
                self.folds_changed();
                self.notify(msg);
            }
            None => self.notify("nothing to open here"),
        }
    }

    fn clamp(&mut self) {
        let n = self.src.len();
        self.cursor = self.cursor.min(n.saturating_sub(1));
        if n == 0 {
            self.cursor = 0;
            self.top = 0;
            self.hoff = 0;
            return;
        }
        // The pinned rows are painted above the window and the cursor never
        // enters them, so both it and the window start below them.
        let pin = self.pinned();
        self.cursor = self.cursor.max(pin);
        let h = self.content_rows().max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        }
        if self.cursor >= self.top + h {
            self.top = self.cursor + 1 - h;
        }
        self.top = self.top.min(n.saturating_sub(h).min(n - 1)).max(pin);
        // `max_hoff` re-renders the window to measure it, so only pay for it
        // when there is a horizontal offset to clamp. Vertical scrolling — the
        // common case, and the one a 10k-column CSV makes expensive — does not.
        if self.hoff > 0 {
            self.hoff = self.hoff.min(self.max_hoff());
        }
        self.dirty = true;
    }

    /// Furthest useful horizontal offset: the widest row currently in the
    /// viewport, minus the viewport width.
    ///
    /// Any row that does not fit counts, not just the ones the renderer marked
    /// `scroll`. `--width 200` in an 80-column terminal lays out *wrapped* rows
    /// that are still wider than the viewport; without this they would be
    /// clipped with no way to reach the hidden text (SPEC.md §CLI `--width`).
    ///
    /// Only the painted window is measured — the same rows the next frame will
    /// ask for — so this stays cheap on a document too large to hold.
    pub fn max_hoff(&mut self) -> usize {
        let h = self.content_rows();
        let pin = self.pinned();
        let w = self.cols.max(1);
        let top = self.top;
        let widest = |lines: Vec<crate::render::Line>| {
            lines
                .iter()
                .map(|l| l.width().saturating_sub(w))
                .max()
                .unwrap_or(0)
        };
        // The pinned rows are on screen too, and a CSV's header is exactly as
        // wide as its widest body row, so they count.
        widest(self.src.lines(0..pin)).max(widest(self.src.lines(top..top.saturating_add(h))))
    }
}

/// True when a row must be scrolled horizontally rather than shown whole: the
/// renderer said so (code, wide tables), or it is simply wider than the
/// viewport. Shared by the painter and [`Pager::max_hoff`] so the cut markers
/// and the reachable offset can never disagree.
pub(crate) fn scrollable(line: &crate::render::Line, viewport: usize) -> bool {
    line.scroll || line.width() > viewport
}
