//! `.jsonl` / `.ndjson` behind the [`Source`] seam (SPEC.md §JSON, "`.jsonl` /
//! `.ndjson`").
//!
//! # A record per line, and none of them read until they are shown
//!
//! * **The index is the CSV one.** A line-oriented file is a CSV without
//!   quoting, so this reuses [`RowStore`] whole — the lazy byte-offset index,
//!   the block-delta offset encoding, the sliding read window, the progress
//!   report — through [`crate::csv::parse::Scanner::lines`], a scanner with
//!   quoting turned off. Nothing about a multi-GB file is read on the open
//!   path: a `stat` and a 3-byte BOM peek, exactly as a CSV.
//! * **A record is parsed when it is shown, and only then.** The user's own
//!   trajectory has a single 41KB line among 2285; laying out a screen by
//!   parsing every line would be as bad as reading the whole file. Rendering a
//!   row parses that row's record, and a small [`Cache`] keeps the ones the
//!   viewport keeps asking about.
//! * **The default view is a list of records**, one row each
//!   ([`tree::record_spans`]). `Enter` / `za` expands one into the tree
//!   [`tree`] renders, spliced in under its row by [`RowMap`].
//! * **A line that is not JSON is an error row** carrying the reason and the
//!   line number, and the file keeps rendering: half a log is still worth
//!   reading.
//!
//! # What is not folded is not parsed
//!
//! Fold state here is the *open* records, not the closed ones: everything
//! starts closed, so the closed set of a million-record file would be a million
//! ids. That is the same "default plus exceptions" shape the document source
//! uses, in the same vocabulary ([`jsonrow::ALL_OPEN`]) — a fold id is a
//! member-index path from the root, and a record file's root is the implicit
//! list of records, so record 4 is `/4`. One scheme, two sources.
#![deny(unsafe_code)]

pub mod lensrow;
pub mod plan;
pub mod rowmap;
mod rows;
pub mod tree;
mod view;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::cell::RefCell;
use std::io;
use std::ops::Range;
use std::path::Path;

use super::search::{self, Dir};
use super::{Anchor, End, Entry, FoldState, Hit, LinkSite, Mark, MatchSpan, Source};
use crate::csv::index::{self, RowIndex, RowStore};
use crate::csv::parse::Scanner;
use crate::csv::read::{self, Reader};
use crate::json::{self, Value};
use crate::lens::Lens;
use crate::render::{Line, LineKind, Span};
use crate::source::jsonrow;
use crate::select::Yank;
use plan::{Plan, Spot};
use rowmap::RowMap;

/// Records the index is pushed past the painted window on every frame.
const LOOKAHEAD: usize = 1024;

/// Records indexed when the layout width is first set, so the first screen is
/// there without waiting for an idle tick.
const FIRST_RECORDS: usize = 256;

/// Bytes of scanning one [`Source::lines`] call may spend on that lookahead.
const FRAME_BYTES: u64 = 4 * 1024 * 1024;

/// Bytes of scanning one idle tick may spend.
const IDLE_BYTES: u64 = 8 * 1024 * 1024;

/// Records one search sweep reads before giving up, as in the CSV source: a
/// sweep must fit inside a keystroke, so search covers the neighbourhood of the
/// cursor rather than a whole multi-GB file.
const SEARCH_RECORDS: usize = 20_000;

/// Records the lens classifies past the painted window, so grouping is decided
/// before the rows it changes are asked for. Smaller than [`LOOKAHEAD`]:
/// classifying a record parses it, and the index is far cheaper than that.
const CLASS_AHEAD: usize = 256;

/// Records read through the lens before the first frame: a screenful, not the
/// [`FIRST_RECORDS`] the index reaches, because classifying costs a parse
/// apiece and the frame extends it as far as it actually needs.
const FIRST_CLASS: usize = 64;

/// Records classified per idle tick, so `G` reaches a lens-read end without a
/// keystroke ever waiting on the whole file.
const CLASS_SLICE: usize = 2048;

/// Parsed records kept in hand. Small on purpose: a record can be megabytes,
/// and the viewport only ever asks about a screenful plus whatever the cursor
/// just left.
const CACHE: usize = 64;

/// Records `zR` will open at once.
///
/// Opening *every* record of a million-record log means parsing the whole file,
/// which is the one thing this source exists not to do. `zR` therefore opens as
/// far as the reader is looking and says so, rather than freezing (see
/// [`Source::fold_all`]).
const EXPAND_CAP: usize = 50_000;

/// A record, as far as this reader got with it.
pub enum Record {
    /// Valid JSON.
    Value(Value),
    /// Not JSON, and why. Rendered as an error row rather than stopping the
    /// file (SPEC.md §JSON).
    Bad(String),
}

impl Record {
    fn value(&self) -> Option<&Value> {
        match self {
            Record::Value(v) => Some(v),
            Record::Bad(_) => None,
        }
    }
}

/// The parsed-record cache: most-recently-used first, [`CACHE`] deep.
#[derive(Default)]
struct Cache {
    items: Vec<(usize, Record)>,
}

impl Cache {
    fn position(&self, record: usize) -> Option<usize> {
        self.items.iter().position(|(r, _)| *r == record)
    }

    /// Insert at the front, dropping the oldest if the cache is full.
    fn push(&mut self, record: usize, rec: Record) {
        if self.items.len() >= CACHE {
            self.items.pop();
        }
        self.items.insert(0, (record, rec));
    }
}

pub struct JsonlSource {
    /// Interior mutability because reading a line is a *file* read and half the
    /// trait is `&self`. Every borrow is taken and dropped inside one helper.
    store: RefCell<RowStore>,
    cache: RefCell<Cache>,
    /// The rows of the one expanded record the viewport is working through.
    /// Capacity one: rebuilding it costs a walk of that record, and a frame
    /// asks for it once per painted row.
    laid: RefCell<Option<(usize, Vec<Line>)>>,
    /// Which records are open, and what that does to the row numbering.
    map: RowMap,
    /// The `--lens` reading of this file, when one was asked for. `None` is
    /// the generic record tree, unchanged in every respect (SPEC.md §Lenses:
    /// "without it, records render as the generic tree").
    plan: Option<Plan>,
    /// `zR` was pressed (or a dump asked for everything): records are opened as
    /// the viewport reaches them, up to [`EXPAND_CAP`].
    expand_all: bool,
    /// Records `expand_all` has already considered.
    filled: usize,
    view: usize,
    /// Outline entries for the records last painted — see [`Source::outline`].
    outline: Vec<Entry>,
    /// Rows last painted.
    window: Range<usize>,
    query: String,
    needle: String,
    sensitive: bool,
    /// Row of the current match, when the last sweep found one.
    found: Option<usize>,
    /// Always empty: a record file has no links. Held so the trait can hand
    /// out a slice.
    none_links: Vec<LinkSite>,
}

impl JsonlSource {
    /// Open `path`. Stats it and reads three bytes; no line is indexed and no
    /// record is parsed until one is asked for.
    pub fn open(path: &Path) -> io::Result<JsonlSource> {
        Ok(JsonlSource::new(store(Reader::open(path)?)))
    }

    /// A source over bytes that arrived on a pipe.
    pub fn from_bytes(data: Vec<u8>) -> JsonlSource {
        JsonlSource::new(store(Reader::memory(data)))
    }

    fn new(store: RowStore) -> JsonlSource {
        JsonlSource {
            store: RefCell::new(store),
            cache: RefCell::new(Cache::default()),
            laid: RefCell::new(None),
            map: RowMap::default(),
            plan: None,
            expand_all: false,
            filled: 0,
            view: 80,
            outline: Vec::new(),
            window: 0..0,
            query: String::new(),
            needle: String::new(),
            sensitive: false,
            found: None,
            none_links: Vec::new(),
        }
    }

    // -- the store ------------------------------------------------------------

    /// Grow the line index toward `records`, spending at most `budget` bytes.
    fn index_to(&self, records: usize, budget: u64) {
        let mut guard = self.store.borrow_mut();
        let s = &mut *guard;
        let mut spent = 0;
        while s.index.known() < records && !s.index.complete() && spent < budget {
            let step = s.index.ensure_bytes(read::WINDOW as u64, &mut s.reader);
            if step == 0 {
                break;
            }
            spent += step;
        }
    }

    /// Records indexed so far.
    pub(crate) fn known(&self) -> usize {
        self.store.borrow().known()
    }

    fn complete(&self) -> bool {
        self.store.borrow().complete()
    }

    /// The raw bytes of record `n`, terminator stripped.
    fn raw(&self, record: usize) -> Vec<u8> {
        let mut store = self.store.borrow_mut();
        store.row(record).map(|s| s.data).unwrap_or_default()
    }

    /// The raw text of record `n`, for the search sweep: no value is built and
    /// no row is laid out, which is what keeps a sweep inside a keystroke.
    fn raw_text(&self, record: usize) -> String {
        String::from_utf8_lossy(&self.raw(record)).into_owned()
    }

    // -- records ---------------------------------------------------------------

    /// Read and parse record `n`. The only place a line becomes a value.
    fn load(&self, record: usize) -> Record {
        let span = {
            let mut store = self.store.borrow_mut();
            match store.row(record) {
                Some(s) => s,
                None => return Record::Bad("no such line".to_string()),
            }
        };
        if span.truncated {
            let mb = span.data.len() as f64 / (1024.0 * 1024.0);
            return Record::Bad(format!("line longer than {mb:.0} MiB, not parsed"));
        }
        if span.data.iter().all(|b| b.is_ascii_whitespace()) {
            return Record::Bad("blank line".to_string());
        }
        match json::parse(&span.data) {
            Ok(v) => Record::Value(v),
            Err(e) => Record::Bad(e.to_string()),
        }
    }

    /// Run `f` against record `n`, parsing it if it is not already in hand.
    ///
    /// A closure rather than a returned reference because the record lives in a
    /// [`RefCell`] and can be tens of megabytes: handing out a clone to read one
    /// field would undo the point of the cache.
    fn with_record<T>(&self, record: usize, f: impl FnOnce(&Record) -> T) -> T {
        if self.cache.borrow().position(record).is_none() {
            let loaded = self.load(record);
            self.cache.borrow_mut().push(record, loaded);
        }
        let cache = self.cache.borrow();
        match cache.position(record).and_then(|i| cache.items.get(i)) {
            Some((_, rec)) => f(rec),
            // Unreachable: it was just inserted. Answering rather than
            // panicking is the rule this whole seam is held to.
            None => f(&Record::Bad("unavailable".to_string())),
        }
    }

    /// Rows record `n`'s tree occupies. `0` for a scalar or an error row, which
    /// is what makes them leaves with nothing to open.
    fn tree_len(&self, record: usize) -> usize {
        self.with_record(record, |r| r.value().map(tree::row_count).unwrap_or(0))
    }

    /// Run `f` against the laid-out rows of record `n`'s tree, laying them out
    /// if the cache is holding a different record.
    fn with_tree<T>(&self, record: usize, f: impl FnOnce(&[Line]) -> T) -> T {
        let hit = matches!(&*self.laid.borrow(), Some((r, _)) if *r == record);
        if !hit {
            let rows = self.with_record(record, |r| match r.value() {
                Some(v) => tree::rows(v, record + 1),
                None => Vec::new(),
            });
            *self.laid.borrow_mut() = Some((record, rows));
        }
        match &*self.laid.borrow() {
            Some((_, rows)) => f(rows),
            None => f(&[]),
        }
    }

    // -- search -------------------------------------------------------------------

    /// Look for the query from row `from`, wrapping once, over record *source
    /// text* — far cheaper than laying rows out, and it finds a value whose
    /// row the viewport has cut off the right-hand side.
    /// Returns the *record* it found and whether the sweep wrapped; turning
    /// that into a row is [`JsonlSource::hit`]'s job, because it may have to
    /// open a fold first.
    fn sweep(&self, from: usize, dir: Dir, inclusive: bool) -> Option<(usize, bool)> {
        let n = self.known();
        if self.query.is_empty() || n == 0 {
            return None;
        }
        let step: isize = match dir {
            Dir::Forward => 1,
            Dir::Backward => -1,
        };
        let start = self.record_at(from.min(self.len_rows().saturating_sub(1)));
        let mut record = start as isize + isize::from(!inclusive) * step;
        let mut wrapped = false;
        for _ in 0..SEARCH_RECORDS.min(n) + 1 {
            if record < 0 {
                record = n as isize - 1;
                wrapped = true;
            } else if record >= n as isize {
                record = 0;
                wrapped = true;
            }
            if self.hits(record as usize) {
                return Some((record as usize, wrapped));
            }
            record += step;
        }
        None
    }

    fn hits(&self, record: usize) -> bool {
        let text = self.raw_text(record);
        match self.sensitive {
            true => text.contains(&self.needle),
            false => text.to_lowercase().contains(&self.needle),
        }
    }

    /// Turn a swept *record* into a row, opening whatever folds it away: a
    /// match the reader cannot see is not a match (SPEC.md §Lenses — a lens
    /// may fold, but it may never lose a record).
    fn hit(&mut self, found: Option<(usize, bool)>) -> Option<Hit> {
        let (record, wrapped) = found?;
        self.reveal_record(record);
        let row = self.row_of_record(record);
        self.found = Some(row);
        Some(Hit { anchor: Anchor(row), wrapped })
    }

    // -- yank ----------------------------------------------------------------------

    fn yank(text: String, what: String) -> Option<Yank> {
        match text.is_empty() {
            true => None,
            false => Some(Yank { text, what }),
        }
    }

    /// One row as source-faithful text: a string's own characters, anything
    /// else as compact JSON. A string is the value a reader wants to paste
    /// somewhere; re-quoting it would be exporting rather than copying, which
    /// is the same rule the CSV form applies to a field.
    fn row_json(&self, row: usize) -> Option<String> {
        self.at_row(row, |_, v| match v {
            Value::Str(s) => s.clone(),
            other => other.to_json(),
        })
    }
}

/// The CSV big-file access layer, driven by the *line* grammar: the same lazy
/// index, sliding window and progress report, with quoting off.
///
/// A JSONL record is one line by definition — RFC 8259 forbids a raw newline
/// inside a string — so the simpler grammar is the correct one here, and
/// running the CSV grammar over it would let one `"` swallow the rest of the
/// file. Everything else about a multi-GB file is inherited unchanged.
fn store(mut reader: Reader) -> RowStore {
    let index = RowIndex::with_scanner(index::origin(&mut reader), Scanner::lines());
    RowStore { reader, index }
}

/// The fold id of record `r`, in the shared vocabulary
/// ([`jsonrow::ALL_OPEN`]): a record file's root is the implicit list of
/// records, so record 4 is `/4`.
fn fold_id(record: usize) -> String {
    jsonrow::child_id("", record)
}

/// A row that can be opened: the fold marker the painter rewrites to `\u{25b8}`
/// when the record is shut, then the summary.
fn marker(mut rest: Vec<Span>) -> Vec<Span> {
    // Always the *open* glyph: the painter rewrites it to `\u{25b8}` on any row
    // `hidden_at` claims, so emitting the closed one here would double-negate.
    let glyph = crate::theme::MARKER_OPEN;
    let mut spans = vec![Span::new(format!("{glyph} "), crate::theme::json_marker())];
    spans.append(&mut rest);
    spans
}

/// A row with nothing under it: the gutter stays, empty, so the values on it
/// line up with the ones on rows that do open.
fn leaf(mut rest: Vec<Span>) -> Vec<Span> {
    let mut spans = vec![Span::plain("  ")];
    spans.append(&mut rest);
    spans
}
