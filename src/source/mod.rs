//! The format seam (SPEC.md §Multi-format reading).
//!
//! A [`Source`] is *anything that can produce rendered rows on demand*. The
//! pager, `nav` and `select` hold a `Box<dyn Source>` and never learn which
//! format they are showing — the same discipline that lets the whole crate
//! above `sys` stay platform-agnostic. Adding a format is one module here plus
//! one arm in the detector; formats are compiled in, never loaded at runtime.
//!
//! # Coordinate systems
//!
//! Three of them, and mixing them up is the way to corrupt a viewport:
//!
//! * **Row** — an index into what is currently *on screen*, `0..len()`. Folded
//!   away rows have no row index at all. Every method that takes or returns a
//!   position the pager will put a cursor on speaks rows. Rows are invalidated
//!   by anything that changes the fold state or the width.
//! * **[`Anchor`]** — an opaque handle to a place in the document that survives
//!   folding but *not* re-layout. Ordered: `a < b` means `a` comes earlier in
//!   the document. Produced by [`Source::anchor`], turned back into a row by
//!   [`Source::row_of`] (visible now?) or [`Source::reveal`] (make it visible).
//! * **[`Mark`]** — a handle that survives re-layout, so the pager can put the
//!   cursor back on the same *content* after a resize. Markdown uses the source
//!   line; a format with no such notion may use the row index, which degrades
//!   to "the same place in the file", never to a panic.
//!
//! # What a format must guarantee
//!
//! Every method is documented with its contract below. Three rules apply
//! throughout:
//!
//! 1. **No panics, ever.** Out-of-range rows, stale anchors and marks from a
//!    previous layout are all *normal* inputs — clamp or return `None`.
//! 2. **`len()` and `lines()` agree.** `lines(a..b)` returns exactly
//!    `b.min(len()) - a.min(len())` rows, in order.
//! 3. **Only what is asked for is computed.** The pager never asks for a row it
//!    is not about to paint, and a format for large files must not read the
//!    whole file to answer `lines()`, `len()` or `set_width()`.
#![deny(unsafe_code)]

pub mod collapse;
pub mod csv;
pub mod detect;
pub mod markdown;
pub mod search;

use std::ops::Range;

use crate::render::Line;
use crate::select::Yank;
use search::Dir;

/// A place in the document that survives folding but not re-layout.
///
/// Opaque to everything above the seam: the pager only ever compares anchors
/// (they order the same way the document reads) and hands them back to the
/// source it got them from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Anchor(pub usize);

/// A place in the *content* that survives re-layout, so a resize can put the
/// cursor back where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mark(pub usize);

/// Opaque, format-defined fold state: the ids of the closed sections.
///
/// The pager stores it in a history [`Snapshot`](crate::nav::history::Snapshot)
/// and hands it back verbatim; it never inspects an id. Ids must be stable
/// across re-layout, which is what lets folds survive a resize.
pub type FoldState = Vec<String>;

/// One entry of the document outline (`o`, and the collapse tree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Nesting depth, 1 = outermost. Drives indentation and the fold ranges.
    pub level: u8,
    /// Stable id for this section: the key fold state is stored under, and the
    /// target of an anchor link (`#some-heading`).
    pub id: String,
    /// Text shown in the outline overlay.
    pub text: String,
    /// Where the section starts.
    pub anchor: Anchor,
    /// True when this section is currently folded shut.
    pub folded: bool,
}

/// One link occurrence in the document, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSite {
    /// The row the link sits on, as an anchor (it may be folded away).
    pub anchor: Anchor,
    /// Display column the link starts at, within its row.
    pub col: usize,
    pub url: String,
}

/// One search match on a row, in display columns of that row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
    /// True for the match the cursor is currently sitting on.
    pub current: bool,
}

/// Where `G` lands, and whether the format still has work to do to know.
///
/// A format that discovers its document lazily — a CSV's row index — genuinely
/// does not know where the end is until it has scanned there, and the *worst*
/// answer is the confident one: jumping to the end of whatever happens to be
/// indexed puts the cursor in the middle of the file and says nothing about it.
/// [`End::Scanning`] is that honest "not yet", carrying the percentage the
/// status bar shows while the pager drives the scan a slice at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The last row `G` should put the cursor on.
    At(usize),
    /// The end is not known yet; `0..=100` of the way there.
    Scanning(u8),
}

/// Where a search landed, and whether it wrapped around the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub anchor: Anchor,
    pub wrapped: bool,
}

/// One row expanded into labelled fields.
///
/// A grid shows as many columns as fit and no more, which is exactly wrong for
/// the row you actually care about: a wide CSV hides most of it off-screen, and
/// a ragged row can carry fields the header never named. This is that row read
/// the other way round — one field per line, label beside value, nothing
/// hidden. A future tree format would return a node's children the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    /// What the overlay is titled, e.g. `Row 41`.
    pub title: String,
    /// `(label, value)`, in the format's own order. A field the format has no
    /// name for still appears — labelled positionally rather than dropped.
    ///
    /// Values are **raw**: exactly the bytes the document holds, control
    /// characters and all. Painting them is what makes them safe
    /// ([`crate::render::visible`]), so that copying one yields the real value
    /// rather than the dotted display form.
    pub fields: Vec<(String, String)>,
}

/// A document behind the format seam.
///
/// Implement this and a second format is complete: the pager, the painter, the
/// yank commands, search and the outline overlay all work through it and
/// nothing else. Nothing in this trait mentions markdown.
pub trait Source {
    // -- layout --------------------------------------------------------------

    /// Lay the document out for a wrap width of `cols` columns.
    ///
    /// Called once before the first paint and again on every resize. Fold state
    /// must survive (it is keyed by id, never by row), search matches must be
    /// recomputed for the query last given to [`Source::set_query`], and every
    /// [`Anchor`] handed out before the call is invalidated. A [`Mark`] is not.
    fn set_width(&mut self, cols: usize);

    /// Number of rows currently on screen — the document's rows minus whatever
    /// the fold state hides. `0` for an empty document; never a panic.
    fn len(&self) -> usize;

    /// True when there is nothing to show.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The rows in `rows`, in order, clamped to `0..len()`.
    ///
    /// This is the only way rows reach the screen, and the pager only ever asks
    /// for the window it is about to paint. A format whose files do not fit in
    /// memory renders exactly these rows and no others.
    fn lines(&mut self, rows: Range<usize>) -> Vec<Line>;

    /// One row, or `None` when `row` is past the end.
    ///
    /// Convenience over [`Source::lines`]. The paint loop always holds a whole
    /// window and never calls it, which is why it is `allow(dead_code)`; the
    /// tests and any future single-row lookup do.
    #[allow(dead_code)]
    fn line(&mut self, row: usize) -> Option<Line> {
        self.lines(row..row.saturating_add(1)).pop()
    }

    // -- viewport affordances --------------------------------------------------
    //
    // Five hooks with defaults that reproduce today's markdown behaviour exactly,
    // so a format opts into each one or is unaffected by it. They are viewport
    // *policy* the pager cannot invent — how many rows are frozen at the top,
    // what one press of `h` means, what `w` widens — and the pager still never
    // learns which format answered (SPEC.md §The `Source` seam).

    /// Rows frozen at the top of the viewport: they are painted above the
    /// scrolling window on every frame, the cursor never enters them, and they
    /// are always rows `0..pinned()` of the document.
    ///
    /// A CSV pins its header (SPEC.md §CSV, "the header row stays pinned while
    /// scrolling vertically"). Markdown pins nothing.
    fn pinned(&self) -> usize {
        0
    }

    /// True when the format wants the whole terminal rather than the reading
    /// measure `--width` / `dump::layout_width` picks for prose. A grid is not
    /// prose: a 200-column terminal should show 200 columns of table. `--width`
    /// still wins over both.
    fn full_width(&self) -> bool {
        false
    }

    /// `h` / `l`: the horizontal offset one step in `dir` (`-1` left, `+1`
    /// right) from `hoff`, when the format scrolls by something other than a
    /// fixed number of columns. `None` — the default — leaves the pager's own
    /// character step alone.
    ///
    /// A CSV steps by whole columns, which is also what keeps the pinned header
    /// aligned with the body: both are painted at the same offset.
    ///
    /// `view` is the *viewport* — how many columns the terminal actually shows
    /// — which is not the layout width [`Source::set_width`] was given: with
    /// `--width 200` on an 80-column terminal the document is laid out 200 wide
    /// and only 80 of it is on screen. Deciding "does this column already fit?"
    /// or "how far may the offset go?" against the layout width is what makes
    /// `h`/`l` dead on exactly that combination, so the pager passes the number
    /// it paints with.
    fn hscroll(&mut self, hoff: usize, dir: isize, view: usize) -> Option<usize> {
        let _ = (hoff, dir, view);
        None
    }

    /// `w`: widen whatever is under the cursor, returning the message to show.
    /// `None` — the default — means the format has nothing to widen.
    fn widen(&mut self) -> Option<String> {
        None
    }

    /// The position segment of the status bar (`42%  ·  line 12/840`), when the
    /// format counts in something other than rendered lines. `None` keeps the
    /// pager's own text.
    fn position_text(&self, row: usize) -> Option<String> {
        let _ = row;
        None
    }

    /// `G`: the last row the cursor should land on.
    ///
    /// The default is the last row there is, which is right for any format
    /// whose document is fully known the moment it is opened. A lazily indexed
    /// format answers [`End::Scanning`] until it has found the real end, and
    /// the pager drives it there through [`Source::extend`] rather than
    /// pretending the indexed prefix is the file (SPEC.md §CSV: the scan `G`
    /// forces is interruptible and reports progress).
    fn end(&self) -> End {
        End::At(self.len().saturating_sub(1))
    }

    /// Spend a bounded slice of work on whatever this format is still
    /// discovering — a CSV's lazy row index — and return true while there is
    /// more to do. Called when the input loop is idle, so it must always return
    /// promptly: `q` may never wait on a scan (SPEC.md §CSV).
    fn extend(&mut self) -> bool {
        false
    }

    // -- positions -----------------------------------------------------------

    /// An anchor for a visible row, or `None` when `row` is past the end.
    fn anchor(&self, row: usize) -> Option<Anchor>;

    /// The row an anchor is on *right now*, or `None` when it is folded away
    /// or the anchor is stale.
    fn row_of(&self, anchor: Anchor) -> Option<usize>;

    /// Make `anchor` visible — opening whatever folds hide it — and return the
    /// row it landed on. `None` only when the document is empty.
    fn reveal(&mut self, anchor: Anchor) -> Option<usize>;

    /// A re-layout-stable mark for a visible row.
    fn mark(&self, row: usize) -> Option<Mark>;

    /// The first visible row at or after `mark`, clamped to the last row.
    /// `None` when the document is empty.
    fn locate(&self, mark: Mark) -> Option<usize>;

    // -- structure -----------------------------------------------------------

    /// The document outline, in reading order. Drives the `o` overlay, the
    /// collapse tree and anchor links. Empty when the format has no sections.
    fn outline(&self) -> &[Entry];

    /// Index into [`Source::outline`] of the section containing `row`, or
    /// `None` when the row sits above the first section (or there are none).
    fn section_at(&self, row: usize) -> Option<usize>;

    /// Fold (`closed`) or unfold the outline entry `entry`. Returns true when
    /// something actually changed. A folded section hides everything it owns
    /// but keeps its own row on screen.
    fn set_fold(&mut self, entry: usize, closed: bool) -> bool;

    /// `zM` / `zR`: fold or unfold every section at once.
    fn fold_all(&mut self, closed: bool);

    /// The current fold state, to be stored and handed back later.
    fn folds(&self) -> FoldState;

    /// Restore a fold state produced by [`Source::folds`]. Ids that no longer
    /// exist are ignored rather than an error.
    fn set_folds(&mut self, folds: FoldState);

    /// How many rows the folded section *starting on* `row` hides, or `None`
    /// when the row does not start a folded section. Drives the `(N lines)`
    /// summary in the gutter.
    fn hidden_at(&self, row: usize) -> Option<usize>;

    /// `Tab` / `S-Tab`: the next or previous structural landmark strictly after
    /// (before) `row`, in rows. `None` when there is none that way.
    fn next_landmark(&self, row: usize, forward: bool) -> Option<usize>;

    /// Jump to a section by its [`Entry::id`] — an anchor link (`#slug`).
    /// Opens the section and everything hiding it, and returns its row.
    fn goto_id(&mut self, id: &str) -> Option<usize>;

    // -- links ---------------------------------------------------------------

    /// Every link in the document, in reading order. Recomputed on re-layout,
    /// so the indices into it are only valid until the next [`Source::set_width`].
    fn links(&self) -> &[LinkSite];

    // -- search --------------------------------------------------------------

    /// Set the live query and recompute every match, *including* matches on
    /// rows a fold currently hides — the pager expands a fold to reveal a hit.
    /// An empty query clears the matches and the current-match cursor.
    fn set_query(&mut self, query: &str);

    /// How many matches the live query has.
    fn match_count(&self) -> usize;

    /// Index of the match the cursor is on, if any.
    ///
    /// The painter learns which match is current from [`MatchSpan::current`],
    /// so nothing on the binary's path asks for the index itself; it is part of
    /// the contract (a status bar saying "3/7" is the obvious next caller) and
    /// the tests assert on it.
    #[allow(dead_code)]
    fn current_match(&self) -> Option<usize>;

    /// Incremental search while the query is being typed: the first match at or
    /// after `origin`, wrapping. Sets the current match.
    fn preview_match(&mut self, origin: Anchor, dir: Dir) -> Option<Hit>;

    /// `n` / `N`: the next match after the current one — or after `from` when
    /// there is no current match — wrapping. Sets the current match.
    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit>;

    /// Matches on a visible row, in that row's display columns, so the painter
    /// can highlight them without knowing what a match is.
    fn matches_on(&self, row: usize) -> Vec<MatchSpan>;

    // -- yank ----------------------------------------------------------------

    /// `y`: the rows `rows` as source-faithful text — never the padded,
    /// wrapped, gutter-prefixed display form. `None` when there is nothing to
    /// copy.
    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank>;

    /// `y` with nothing selected: the smallest thing at `row` worth copying —
    /// one cell in a table format. `None` — the default — lets the pager fall
    /// back to copying the focused link, which is what markdown wants.
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let _ = row;
        None
    }

    /// `Y`: the whole section under `row`, its heading included.
    fn yank_section(&self, row: usize) -> Option<Yank>;

    /// `c`: the verbatim block under (or nearest below) `row` — a code block in
    /// markdown, a column in a table format.
    fn yank_block(&self, row: usize) -> Option<Yank>;

    // -- row detail -----------------------------------------------------------

    /// `Enter` on `row`: that row expanded into labelled fields, for formats
    /// where a row is a record rather than prose.
    ///
    /// `None` — the default — means the format has no such thing, and `Enter`
    /// keeps whatever else it means there (follow a link, toggle a fold).
    fn detail(&self, row: usize) -> Option<Detail> {
        let _ = row;
        None
    }
}
