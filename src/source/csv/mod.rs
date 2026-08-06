//! CSV behind the [`Source`] seam: the windowed view of a file too big to load
//! (SPEC.md §CSV).
//!
//! # What makes a multi-GB file pleasant
//!
//! * **Nothing whole-file happens on the open path.** Opening stats the file,
//!   skips a BOM, sniffs the delimiter from the first chunk and samples
//!   [`grid::SAMPLE_ROWS`] rows to size the columns. [`Source::lines`] renders
//!   *only* the window it was asked for, reading each row from its byte offset
//!   through [`RowStore`]. The row index grows a bounded amount per frame
//!   ([`LOOKAHEAD`]) and per idle tick ([`Source::extend`]), so `q` never waits
//!   on a scan and the total is honestly reported as `\u{2265}N` until it is
//!   actually known.
//! * **The header is pinned.** Rows `0..`[`HEAD_ROWS`] are the top border, the
//!   header and the separator; [`Source::pinned`] tells the pager to freeze
//!   them at the top of the viewport. Header and body are drawn from one
//!   [`Grid`] at one horizontal offset, so they cannot drift apart.
//! * **`h`/`l` move a column at a time** ([`Source::hscroll`]), and the column
//!   they land on is the one the status bar names, the one `w` widens and the
//!   one `y` copies. One cursor, three affordances.
//! * **`w` fits the column under the cursor to the widest value currently on
//!   screen** (never narrower than it already is, never wider than the
//!   viewport). That rule is deliberate: it is instant on any file size, it
//!   depends only on what the reader can see, and pressing it twice on the same
//!   screen changes nothing.
//!
//! Yanks are source-faithful and re-quoted in [`yank`]; nothing above ever sees
//! the padded display form.
#![deny(unsafe_code)]

pub mod grid;
pub mod render;
mod view;
pub mod yank;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::cell::RefCell;
use std::io;
use std::ops::Range;
use std::path::Path;

use super::search::{self, Dir};
use super::{Anchor, End, Entry, FoldState, Hit, LinkSite, Mark, MatchSpan, Source};
use crate::csv::index::RowStore;
use crate::csv::{delim, parse, read};
use crate::render::Line;
use crate::select::Yank;
use grid::Grid;
use render::Edge;

/// Grid furniture above the data: top border, header text, separator. Also the
/// number of rows the pager freezes at the top of the viewport.
pub const HEAD_ROWS: usize = 3;

/// Rows the index is pushed past the painted window on every frame, so the
/// viewport can always move one more page and `len()` keeps growing.
pub const LOOKAHEAD: usize = 1024;

/// Bytes of scanning one [`Source::lines`] call may spend on that lookahead.
/// A file of megabyte rows hits this before it hits [`LOOKAHEAD`].
const FRAME_BYTES: u64 = 4 * 1024 * 1024;

/// Bytes of scanning one idle tick may spend. The input loop wakes about ten
/// times a second, so a file indexes in the background at tens of MB/s while
/// staying instantly responsive.
const IDLE_BYTES: u64 = 8 * 1024 * 1024;

/// Rows one search sweep reads before giving up.
///
/// A sweep must fit inside a keystroke: this many rows of a typical file is a
/// fifth of a second, and every character typed into `/` starts a new one.
/// Search therefore covers the neighbourhood of the cursor rather than a whole
/// multi-GB file — which is the same trade the row index makes.
const SEARCH_ROWS: usize = 20_000;

/// Values one `c` (yank column) collects. A column of a 10M-row file is not a
/// clipboard's business, and the label says how many came back.
const COLUMN_CAP: usize = 100_000;

/// Bytes sniffed to choose the delimiter.
const SNIFF_BYTES: usize = 64 * 1024;

/// Bytes the open-path sample may scan. A thousand ordinary rows are a few
/// hundred KB; the cap is what stops a file of megabyte rows from making
/// opening depend on its size after all.
const SAMPLE_BYTES: u64 = 32 * 1024 * 1024;

/// What a source row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Top,
    Header,
    Sep,
    /// Data row `n`, counting from the first row after the header.
    Data(usize),
    Bottom,
}

pub struct CsvSource {
    /// Interior mutability because reading a row is a *file* read and half the
    /// trait is `&self` (`matches_on`, the yanks). Every borrow below is taken
    /// and dropped inside one small helper; none nest.
    store: RefCell<RowStore>,
    delim: u8,
    grid: Grid,
    /// Viewport width last given to [`Source::set_width`].
    view: usize,
    /// The column `h`/`l` last landed on: what the status bar names, `w`
    /// widens and `y` copies.
    col: usize,
    /// Data rows last rendered, so `w` can fit a column to what is on screen.
    window: Range<usize>,
    query: String,
    /// The query folded for a case-insensitive sweep, so the folding is done
    /// once per search rather than once per row.
    needle: String,
    sensitive: bool,
    /// Source row of the current match, when the last sweep found one.
    found: Option<usize>,
    /// Always empty: a CSV has no sections and no links. Held so the trait can
    /// hand out slices.
    none_outline: Vec<Entry>,
    none_links: Vec<LinkSite>,
}

impl CsvSource {
    /// Open `path`. Reads at most a BOM, a sniff chunk and the sample rows.
    pub fn open(path: &Path, delim: Option<u8>) -> io::Result<CsvSource> {
        let store = RowStore::open(path, delim.unwrap_or(delim::DEFAULT_DELIM))?;
        Ok(CsvSource::new(store, delim))
    }

    /// A source over bytes that arrived on a pipe. Unnamed: the label the
    /// status bar shows belongs to the *input*, and `open.rs` owns it.
    pub fn from_bytes(data: Vec<u8>, delim: Option<u8>) -> CsvSource {
        let store = RowStore::memory(data, delim.unwrap_or(delim::DEFAULT_DELIM));
        CsvSource::new(store, delim)
    }

    fn new(mut store: RowStore, want: Option<u8>) -> CsvSource {
        let delim = match want {
            Some(d) => d,
            None => {
                let sample = store.reader.chunk(0, SNIFF_BYTES).to_vec();
                delim::sniff(&sample)
            }
        };
        // The index was created for whatever delimiter the caller passed in;
        // the sniff may disagree, so rebuild it before a single row is
        // recorded rather than trust offsets taken under another grammar.
        let store = with_delim(store, delim);
        CsvSource {
            store: RefCell::new(store),
            delim,
            grid: Grid::default(),
            view: 80,
            col: 0,
            window: 0..0,
            query: String::new(),
            needle: String::new(),
            sensitive: false,
            found: None,
            none_outline: Vec::new(),
            none_links: Vec::new(),
        }
    }

    /// The column names, for `--toc`. Available as soon as the header is
    /// sampled, which [`Source::set_width`] does.
    pub fn columns(&self) -> Vec<String> {
        self.grid.cols.iter().map(|c| c.name.clone()).collect()
    }

    // -- the store ------------------------------------------------------------

    /// Grow the index toward `rows` file rows, spending at most `budget` bytes.
    fn index_to(&self, rows: usize, budget: u64) {
        let mut guard = self.store.borrow_mut();
        let s = &mut *guard;
        let mut spent = 0;
        while s.index.known() < rows && !s.index.complete() && spent < budget {
            let step = s.index.ensure_bytes(read::WINDOW as u64, &mut s.reader);
            if step == 0 {
                break;
            }
            spent += step;
        }
    }

    /// File rows known so far, header included.
    fn known(&self) -> usize {
        self.store.borrow().known()
    }

    fn complete(&self) -> bool {
        self.store.borrow().complete()
    }

    /// Data rows known so far — file rows less the header.
    fn data_len(&self) -> usize {
        self.known().saturating_sub(1)
    }

    /// One file row as raw text, fields and delimiters included, for the
    /// search sweep: no fields are built and no row is laid out, which is what
    /// keeps a sweep over tens of thousands of rows inside a keystroke.
    fn raw_text(&self, file_row: usize) -> String {
        match self.store.borrow_mut().row(file_row) {
            Some(span) => String::from_utf8_lossy(&span.data).into_owned(),
            None => String::new(),
        }
    }

    /// The fields of one file row, exactly as they are in the file.
    fn raw_row(&self, file_row: usize) -> Option<Vec<String>> {
        let span = self.store.borrow_mut().row(file_row)?;
        Some(parse::record(&span.data, self.delim))
    }

    /// The fields of one data row, padded to the header's arity for display.
    fn fields(&self, data_row: usize) -> Option<Vec<String>> {
        let mut f = self.raw_row(data_row + 1)?;
        parse::fit(&mut f, self.grid.arity());
        Some(f)
    }

    // -- rows -----------------------------------------------------------------

    fn kind(&self, row: usize) -> Option<Kind> {
        let data = self.data_len();
        if self.grid.is_empty() {
            return None;
        }
        match row {
            0 => Some(Kind::Top),
            1 => Some(Kind::Header),
            2 => Some(Kind::Sep),
            r if r < HEAD_ROWS + data => Some(Kind::Data(r - HEAD_ROWS)),
            r if r == HEAD_ROWS + data && self.complete() => Some(Kind::Bottom),
            _ => None,
        }
    }

    /// One rendered row. `&self` so the painter's `&Pager` paths (search
    /// highlighting) can ask for the same text the window was drawn from.
    fn row_line(&self, row: usize) -> Option<Line> {
        Some(match self.kind(row)? {
            Kind::Top => render::border(&self.grid, Edge::Top, 0),
            Kind::Header => render::header(&self.grid),
            Kind::Sep => render::border(&self.grid, Edge::Mid, 1),
            Kind::Bottom => render::border(&self.grid, Edge::Bottom, self.known() + 1),
            Kind::Data(d) => {
                let fields = self.fields(d).unwrap_or_default();
                render::data(&self.grid, &fields, d + 2)
            }
        })
    }

    fn row_text(&self, row: usize) -> String {
        self.row_line(row).map(|l| l.text()).unwrap_or_default()
    }

    /// Sample the first rows to size the columns (SPEC.md §CSV).
    fn sample(&mut self) {
        self.index_to(grid::SAMPLE_ROWS + 2, SAMPLE_BYTES);
        let header = self.raw_row(0).unwrap_or_default();
        self.grid = Grid::new(&header);
        if self.grid.is_empty() {
            return;
        }
        for d in 0..grid::SAMPLE_ROWS.min(self.data_len()) {
            if let Some(f) = self.raw_row(d + 1) {
                self.grid.sample(&f);
            }
        }
    }

    // -- search ---------------------------------------------------------------

    /// Look for the query from source row `from`, wrapping once. Bounded by
    /// [`SEARCH_ROWS`]: a hit further away than that is reported as no hit
    /// rather than as a freeze.
    fn sweep(&self, from: usize, dir: Dir, inclusive: bool) -> Option<(usize, bool)> {
        let last = HEAD_ROWS + self.data_len();
        if self.query.is_empty() || last <= HEAD_ROWS {
            return None;
        }
        let step: isize = match dir {
            Dir::Forward => 1,
            Dir::Backward => -1,
        };
        let mut row = from.clamp(HEAD_ROWS, last - 1) as isize;
        if !inclusive {
            row += step;
        }
        let mut wrapped = false;
        for _ in 0..SEARCH_ROWS.min(last - HEAD_ROWS) + 1 {
            if row < HEAD_ROWS as isize {
                row = last as isize - 1;
                wrapped = true;
            } else if row >= last as isize {
                row = HEAD_ROWS as isize;
                wrapped = true;
            }
            if self.hits(row as usize) {
                return Some((row as usize, wrapped));
            }
            row += step;
        }
        None
    }

    /// Does the row match? Tested against the row's *source* text rather than
    /// its rendered form: it is far cheaper, and it also finds a value whose
    /// tail a narrow column truncated — the cursor lands on the right row even
    /// when the hit is off the side of the cell.
    fn hits(&self, row: usize) -> bool {
        let text = match self.kind(row) {
            Some(Kind::Data(d)) => self.raw_text(d + 1),
            Some(Kind::Header) => self.raw_text(0),
            _ => return false,
        };
        match self.sensitive {
            true => text.contains(&self.needle),
            false => text.to_lowercase().contains(&self.needle),
        }
    }

    fn hit(&mut self, found: Option<(usize, bool)>) -> Option<Hit> {
        let (row, wrapped) = found?;
        self.found = Some(row);
        Some(Hit { anchor: Anchor(row), wrapped })
    }

    // -- yank -----------------------------------------------------------------

    fn yank(text: String, what: String) -> Option<Yank> {
        match text.trim().is_empty() {
            true => None,
            false => Some(Yank { text, what }),
        }
    }

    /// The rows of `rows` as CSV records, the header included when the range
    /// covers it.
    fn rows_csv(&self, rows: Range<usize>) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for row in rows {
            match self.kind(row) {
                Some(Kind::Header) => out.push(self.raw_row(0).unwrap_or_default()),
                Some(Kind::Data(d)) => out.push(self.raw_row(d + 1).unwrap_or_default()),
                _ => {}
            }
        }
        out
    }
}

/// Adopt a delimiter: a fresh index over the same open file, because offsets
/// recorded under one delimiter cannot be trusted under another.
fn with_delim(store: RowStore, delim: u8) -> RowStore {
    let RowStore { reader, index } = store;
    let index = crate::csv::index::RowIndex::new(index.origin(), delim);
    RowStore { reader, index }
}
