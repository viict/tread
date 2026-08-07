//! Plain text behind the [`Source`] seam (SPEC.md §Plain text).
//!
//! # Verbatim, and nothing else
//!
//! A file whose extension names no parser renders as its own lines: no
//! headings, no inline markup, no wrapping. `# comment` in a shell script is a
//! comment, and a reader that turned it into a banner would be worse than one
//! that did nothing. A long line is therefore horizontally scrollable exactly
//! as a code block is ([`Line::scroll`]) — wrapping a shell script changes
//! what it says.
//!
//! # The same big-file discipline as CSV
//!
//! A 2GB log must open instantly and quit instantly, so this reuses the CSV
//! access layer whole — [`RowStore`], the lazy byte-offset index, the block
//! delta encoding, the sliding read window and the progress report — driven by
//! [`RowStore::lines`], the quoting-free grammar `.jsonl` already drives it
//! with. Nothing is written here that indexes a line: there is one line indexer
//! in this crate and this is its third caller. Opening
//! costs a `stat` and a 3-byte BOM peek; [`Source::lines`] reads only the rows
//! it was asked to paint.
//!
//! # Tabs are expanded, control characters are dotted
//!
//! A `\t` is expanded to the next [`TAB`]-column tab stop rather than shown as
//! the sanitiser's `\u{b7}`. This is the one deliberate difference from a CSV
//! cell, and it is deliberate because of what the format *is*: a Makefile, a Go
//! file and an indented shell script are mostly tabs, and dotting them would
//! turn the first screen of the common case into noise with every indent level
//! collapsed to one column. A tab means "advance to the next stop", the
//! terminal itself would do exactly that, and doing it here — rather than
//! emitting the byte — keeps the column arithmetic ours, so horizontal
//! scrolling, search columns and the cut markers all agree. Every *other*
//! control character still goes through the shared rule
//! ([`crate::render::visible`]'s [`crate::render::CONTROL`]): a `\x1b` painted
//! raw would let a document repaint the screen.
//!
//! Yanks are unaffected: `y` copies the bytes the file holds, tabs included.
//! Sanitising is a display transform, never a change to the data.
//!
//! # What a text file has no answer for
//!
//! An outline, folds and links: all the honest empty answer, exactly as the CSV
//! source gives for the same methods. Search, selection, yank and horizontal
//! scrolling do not depend on structure, so they all work.
#![deny(unsafe_code)]

mod view;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::cell::RefCell;
use std::io;
use std::path::Path;

use super::search::{self, Dir};
use super::{Anchor, End, Entry, FoldState, Hit, LinkSite, Mark, MatchSpan, Source};
use crate::csv::index::RowStore;
use crate::csv::read::{self, Reader};
use crate::render::{Line, LineKind, Span};
use crate::select::Yank;

/// Columns between tab stops. Eight is what a terminal's own `HT` does and what
/// every one of these files was written against.
pub const TAB: usize = 8;

/// Lines the index is pushed past the painted window on every frame, so the
/// viewport can always move one more page and `len()` keeps growing.
const LOOKAHEAD: usize = 1024;

/// Lines indexed when the layout width is first set, so the first screen is
/// there without waiting for an idle tick.
const FIRST_LINES: usize = 256;

/// Bytes of scanning one [`Source::lines`] call may spend on that lookahead.
const FRAME_BYTES: u64 = 4 * 1024 * 1024;

/// Bytes of scanning one idle tick may spend.
const IDLE_BYTES: u64 = 8 * 1024 * 1024;

/// Lines one search sweep reads before giving up. A sweep must fit inside a
/// keystroke, so search covers the neighbourhood of the cursor rather than a
/// whole multi-GB file — the same trade the CSV source makes.
const SEARCH_LINES: usize = 20_000;

pub struct TextSource {
    /// Interior mutability because reading a line is a *file* read and half the
    /// trait is `&self` (`matches_on`, the yanks). Every borrow is taken and
    /// dropped inside one small helper; none nest.
    store: RefCell<RowStore>,
    query: String,
    /// The query folded for a case-insensitive sweep, so the folding happens
    /// once per search rather than once per line.
    needle: String,
    sensitive: bool,
    /// Row of the current match, when the last sweep found one.
    found: Option<usize>,
    /// Always empty: a text file has no sections and no links. Held so the
    /// trait can hand out slices.
    none_outline: Vec<Entry>,
    none_links: Vec<LinkSite>,
}

impl TextSource {
    /// Open `path`. Stats it and reads three bytes; no line is indexed and no
    /// line is read until one is asked for.
    pub fn open(path: &Path) -> io::Result<TextSource> {
        Ok(TextSource::new(RowStore::lines(Reader::open(path)?)))
    }

    /// A source over bytes that arrived on a pipe.
    pub fn from_bytes(data: Vec<u8>) -> TextSource {
        TextSource::new(RowStore::lines(Reader::memory(data)))
    }

    fn new(store: RowStore) -> TextSource {
        TextSource {
            store: RefCell::new(store),
            query: String::new(),
            needle: String::new(),
            sensitive: false,
            found: None,
            none_outline: Vec::new(),
            none_links: Vec::new(),
        }
    }

    // -- the store ------------------------------------------------------------

    /// Grow the line index toward `lines`, spending at most `budget` bytes.
    fn index_to(&self, lines: usize, budget: u64) {
        let mut guard = self.store.borrow_mut();
        let s = &mut *guard;
        let mut spent = 0;
        while s.index.known() < lines && !s.index.complete() && spent < budget {
            let step = s.index.ensure_bytes(read::WINDOW as u64, &mut s.reader);
            if step == 0 {
                break;
            }
            spent += step;
        }
    }

    /// Lines indexed so far.
    fn known(&self) -> usize {
        self.store.borrow().known()
    }

    fn complete(&self) -> bool {
        self.store.borrow().complete()
    }

    /// One line exactly as the file holds it, terminator stripped and invalid
    /// UTF-8 replaced — the same lossy decode every other format applies.
    fn raw(&self, row: usize) -> Option<String> {
        let span = self.store.borrow_mut().row(row)?;
        Some(String::from_utf8_lossy(&span.data).into_owned())
    }

    /// One rendered row. `&self` so the painter's `&Pager` paths (search
    /// highlighting) can ask for the same text the window was drawn from.
    fn row_line(&self, row: usize) -> Option<Line> {
        let raw = self.raw(row)?;
        Some(line(display(&raw), row + 1))
    }

    fn row_text(&self, row: usize) -> String {
        self.raw(row).map(|s| display(&s)).unwrap_or_default()
    }

    // -- search ---------------------------------------------------------------

    /// Look for the query from row `from`, wrapping once. Bounded by
    /// [`SEARCH_LINES`]: a hit further away than that is reported as no hit
    /// rather than as a freeze.
    fn sweep(&self, from: usize, dir: Dir, inclusive: bool) -> Option<(usize, bool)> {
        let n = self.known();
        if self.query.is_empty() || n == 0 {
            return None;
        }
        let step: isize = match dir {
            Dir::Forward => 1,
            Dir::Backward => -1,
        };
        let mut row = from.min(n - 1) as isize;
        if !inclusive {
            row += step;
        }
        let mut wrapped = false;
        for _ in 0..SEARCH_LINES.min(n) + 1 {
            if row < 0 {
                row = n as isize - 1;
                wrapped = true;
            } else if row >= n as isize {
                row = 0;
                wrapped = true;
            }
            if self.hits(row as usize) {
                return Some((row as usize, wrapped));
            }
            row += step;
        }
        None
    }

    /// Does the row match? Tested against the line's *source* text, so a hit
    /// off the right-hand side of the viewport still lands the cursor on the
    /// right row — and so a tab in the file matches a tab in the query.
    fn hits(&self, row: usize) -> bool {
        let text = self.raw(row).unwrap_or_default();
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
        match text.is_empty() {
            true => None,
            false => Some(Yank { text, what }),
        }
    }
}

/// One line as it is painted: tabs to the next [`TAB`] stop, every other
/// control character as [`crate::render::CONTROL`], everything else untouched.
///
/// See the module docs for why a tab is expanded rather than dotted.
pub fn display(raw: &str) -> String {
    if !raw.chars().any(char::is_control) {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut col = 0;
    for c in raw.chars() {
        match c {
            '\t' => {
                let stop = TAB - col % TAB;
                out.extend(std::iter::repeat(' ').take(stop));
                col += stop;
            }
            c if c.is_control() => {
                out.push(crate::render::CONTROL);
                col += 1;
            }
            c => {
                col += crate::render::char_width(c);
                out.push(c);
            }
        }
    }
    out
}

/// One row of the file as one [`Line`], unstyled.
///
/// `scroll` is always true: a text line is never wrapped, so a line wider than
/// the viewport scrolls, exactly as a code block does (SPEC.md §Plain text).
/// [`LineKind::Code`] for the same reason — it is the crate's word for
/// "verbatim, laid out as written".
fn line(text: String, source_line: usize) -> Line {
    Line {
        spans: vec![Span::plain(text)],
        block: 0,
        source_line,
        heading: None,
        scroll: true,
        kind: LineKind::Code,
    }
}
