//! JSON behind the [`Source`] seam: a document read by byte range, never
//! loaded (SPEC.md §JSON).
//!
//! # Why a 900MB JSON opens as fast as a 900MB CSV
//!
//! Because nothing whole-file happens on the open path, and nothing whole-file
//! happens on any later path either:
//!
//! * **Opening** stats the file and reads 64 bytes to find where the root value
//!   starts. That is all, at any size.
//! * **The structural index** ([`crate::json::index`]) finds a container's
//!   immediate members by walking bytes, building no values. It is budgeted per
//!   frame and per idle tick, so `q` never waits on a scan and the row count is
//!   honestly `\u{2265}N` until it is really known.
//! * **Expanding a node indexes that node**, with its own scan. Laziness is at
//!   every level, so an object holding one enormous array is instant.
//! * **A member is parsed only when it is shown**, and only if it is under
//!   [`tree::PARSE_CAP`]; past that the row says how big it is.
//! * **Counts for a collapsed summary come from the index**, so `{…5 keys}`
//!   costs a walk of that container and no parse at all.
//!
//! # Nothing recurses on nesting
//!
//! The parser is iterative ([`crate::json::parse`]), the value tree's `Drop`,
//! `Clone` and `PartialEq` are iterative, the serialiser is iterative, the
//! structural scan is a byte loop, and the flatten in [`flat`] is an explicit
//! stack. Ten thousand levels of `[[[[` are ten thousand heap frames and no
//! stack frames anywhere.
#![deny(unsafe_code)]

pub mod export;
pub mod flat;
pub mod render;
pub mod tree;
mod view;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::cell::RefCell;
use std::io;
use std::ops::Range;
use std::path::Path;

use super::search::Dir;
use super::{Anchor, Entry, LinkSite};
use crate::render::Line;
use crate::select::Yank;
use flat::{Flat, Folds, Part, Row};
use tree::{Doc, NodeId, PARSE_CAP};

/// Rows the flatten is pushed past the painted window on every frame, so the
/// viewport can always move one more page and `len()` keeps growing.
const LOOKAHEAD: usize = 512;

/// Rows found before the first paint, so `len()` is not zero when the pager
/// first asks.
const FIRST_SCREEN: usize = 256;

/// Bytes of structural scanning one [`Source::lines`] call may spend.
const FRAME_BYTES: u64 = 4 * 1024 * 1024;

/// Bytes one idle tick may spend. The input loop wakes about ten times a
/// second, so a document indexes in the background while staying responsive.
const IDLE_BYTES: u64 = 8 * 1024 * 1024;

/// Rows one idle tick may find.
///
/// The frame budget is a *row* count because a frame only needs the next
/// screen; the idle budget must not be, or a file of small members indexes at
/// one screenful per tick — a hundred megabytes would take half a minute to
/// walk in the background, against a second for the same CSV. This is the
/// backstop that keeps one tick bounded when the members are already indexed
/// and rows cost nothing to find.
const IDLE_ROWS: usize = 65_536;

/// Bytes a re-fold may spend re-walking the tree. No file is re-read: the
/// indexes survive a fold change, only the row list is rebuilt.
const REFOLD_BYTES: u64 = 16 * 1024 * 1024;

/// Members `--toc` lists.
const TOC_MEMBERS: usize = 1000;

/// Rows one search sweep looks at. A sweep must fit inside a keystroke, so
/// search covers the neighbourhood of the cursor rather than a multi-GB file —
/// the same trade the CSV side makes.
const SEARCH_ROWS: usize = 20_000;

pub struct JsonSource {
    /// Interior mutability because reading a member is a *file* read and half
    /// the trait is `&self`. Every borrow is taken and dropped inside one small
    /// helper; none nest.
    doc: RefCell<Doc>,
    flat: Flat,
    folds: Folds,
    /// Viewport width last given to `set_width`.
    view: usize,
    /// Outline entries for the last painted window, and the row each window row
    /// belongs to. A whole-document outline is not available for the same
    /// reason a whole-document row count is not: it would mean walking the
    /// file. What the reader can fold is what the reader can see.
    entries: Vec<Entry>,
    win: Range<usize>,
    row_entry: Vec<usize>,
    query: String,
    needle: String,
    sensitive: bool,
    found: Option<usize>,
    none_links: Vec<LinkSite>,
}

impl JsonSource {
    /// Open `path`. Reads 64 bytes; indexes nothing.
    pub fn open(path: &Path) -> io::Result<JsonSource> {
        Ok(JsonSource::new(Doc::open(path)?))
    }

    /// A source over bytes that arrived on a pipe.
    pub fn from_bytes(data: Vec<u8>) -> JsonSource {
        JsonSource::new(Doc::memory(data))
    }

    fn new(doc: Doc) -> JsonSource {
        JsonSource {
            doc: RefCell::new(doc),
            flat: Flat::default(),
            folds: Folds::new(),
            view: 80,
            entries: Vec::new(),
            win: 0..0,
            row_entry: Vec::new(),
            query: String::new(),
            needle: String::new(),
            sensitive: false,
            found: None,
            none_links: Vec::new(),
        }
    }

    /// `--toc`: the root's immediate members, one path per line.
    ///
    /// Capped at [`TOC_MEMBERS`], like the record file's: a table of contents
    /// listing five million array elements is neither a table of contents nor
    /// quick. Only the root is walked, so this does not depend on how deep the
    /// document goes.
    pub fn toc(&mut self) -> Vec<String> {
        let mut doc = self.doc.borrow_mut();
        let Some(root) = doc.root() else {
            return Vec::new();
        };
        let n = doc.index(root, TOC_MEMBERS, u64::MAX).min(TOC_MEMBERS);
        (0..n).map(|i| doc.path_of(root, i)).collect()
    }

    // -- rows ------------------------------------------------------------------

    /// Grow the row list toward `want`, spending at most `budget` bytes.
    fn grow(&mut self, want: usize, budget: u64) {
        let mut doc = self.doc.borrow_mut();
        self.flat.extend(&mut doc, &self.folds, want, budget);
    }

    /// The fold state changed: the rows are a function of it, so they are
    /// rebuilt. Every byte range the tree has indexed survives, which is why
    /// this costs no I/O on a document already walked.
    fn refold(&mut self) {
        let want = self.flat.len().max(FIRST_SCREEN);
        self.flat.reset();
        self.grow(want, REFOLD_BYTES);
    }

    fn row(&self, row: usize) -> Option<Row> {
        self.flat.get(row)
    }

    fn row_line(&self, row: usize) -> Option<Line> {
        let r = self.row(row)?;
        let mut doc = self.doc.borrow_mut();
        Some(render::line(&mut doc, &self.folds, r, row))
    }

    fn row_text(&self, row: usize) -> String {
        self.row_line(row).map(|l| l.text()).unwrap_or_default()
    }

    /// Is the row's own fold id a container at all? A scalar member's id names
    /// a member that cannot be opened, so `za` there folds its parent instead.
    fn foldable_id(&self, r: Row) -> String {
        let member = self.fold_key(r).1;
        let owner = self.doc.borrow().fold_id(r.node);
        match member {
            Some(i) => crate::source::jsonrow::child_id(&owner, i),
            None => owner,
        }
    }

    /// The same thing [`JsonSource::foldable_id`] names, as a pair that costs
    /// nothing to compute or compare. Two rows share a fold id exactly when
    /// they share this, so the outline dedups on it and spells an id only for
    /// the entries it actually keeps — on a document nested ten thousand deep
    /// an id is ten thousand characters long, and one per row is quadratic.
    fn fold_key(&self, r: Row) -> (NodeId, Option<usize>) {
        let mut doc = self.doc.borrow_mut();
        let member = match r.part() {
            Part::Head | Part::Tail => None,
            Part::Member(i) => match doc.node(r.node).member(i) {
                Some(m) if doc.shape_of(m).is_container() => Some(i),
                _ => None,
            },
        };
        (r.node, member)
    }

    /// The readable path of a row: `.users[3].name`.
    fn path_of(&self, row: usize) -> String {
        let Some(r) = self.row(row) else {
            return String::new();
        };
        let mut doc = self.doc.borrow_mut();
        let path = match r.part() {
            Part::Head | Part::Tail => doc.dpath(r.node),
            Part::Member(i) => doc.path_of(r.node, i),
        };
        match path.is_empty() {
            true => ".".to_string(),
            false => path,
        }
    }

    /// Byte range of the value a row shows, with its key when it has one.
    fn span_of(&self, row: usize) -> Option<(Option<String>, u64, u64)> {
        let r = self.row(row)?;
        let mut doc = self.doc.borrow_mut();
        match r.part() {
            Part::Head | Part::Tail => {
                let n = doc.node(r.node);
                Some((n.key.clone(), n.start, n.end))
            }
            Part::Member(i) => {
                let m = doc.node(r.node).member(i)?;
                Some((doc.key_text(m), m.start, m.end))
            }
        }
    }

    /// The raw source bytes of the value a row shows, or the reason they cannot
    /// be handed over.
    fn raw_of(&self, row: usize) -> Result<String, String> {
        let (_, start, end) = self.span_of(row).ok_or_else(|| "nothing here".to_string())?;
        let len = end.saturating_sub(start);
        if len > PARSE_CAP {
            return Err(format!(
                "{} \u{2014} over the {} copy limit",
                render::size(len),
                render::size(PARSE_CAP)
            ));
        }
        let (bytes, clipped) = self.doc.borrow_mut().bytes(start, end);
        match clipped {
            true => Err("the value was clipped".to_string()),
            false => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        }
    }

    // -- the outline of the painted window ---------------------------------------

    /// Rebuild the window's outline. One entry per foldable section in view,
    /// with runs of scalars sharing their parent's entry, so the `o` overlay
    /// lists containers rather than every line on screen.
    fn build_entries(&mut self, win: Range<usize>) {
        self.entries.clear();
        self.row_entry.clear();
        let mut last: Option<(NodeId, Option<usize>)> = None;
        for row in win.clone() {
            let Some(r) = self.row(row) else { break };
            let key = self.fold_key(r);
            if last != Some(key) {
                last = Some(key);
                let id = self.foldable_id(r);
                let text = self.row_text(row).trim().to_string();
                let level = self.depth_of(r).saturating_add(1).min(u8::MAX as usize) as u8;
                self.entries.push(Entry {
                    level,
                    folded: !self.folds.is_open(&id),
                    id,
                    text,
                    anchor: Anchor(row),
                });
            }
            self.row_entry.push(self.entries.len() - 1);
        }
        self.win = win;
    }

    fn depth_of(&self, r: Row) -> usize {
        let doc = self.doc.borrow();
        let d = doc.node(r.node).depth as usize;
        match r.part() {
            Part::Member(_) => d + 1,
            _ => d,
        }
    }

    // -- search -------------------------------------------------------------------

    /// Look for the query from `from`, wrapping once, bounded by
    /// [`SEARCH_ROWS`]: a hit further away is reported as no hit rather than as
    /// a freeze.
    fn sweep(&self, from: usize, dir: Dir, inclusive: bool) -> Option<(usize, bool)> {
        let last = self.flat.len();
        if self.query.is_empty() || last == 0 {
            return None;
        }
        let step: isize = match dir {
            Dir::Forward => 1,
            Dir::Backward => -1,
        };
        let mut row = from.min(last - 1) as isize;
        if !inclusive {
            row += step;
        }
        let mut wrapped = false;
        for _ in 0..SEARCH_ROWS.min(last) + 1 {
            if row < 0 {
                row = last as isize - 1;
                wrapped = true;
            } else if row >= last as isize {
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

    fn hits(&self, row: usize) -> bool {
        let text = self.row_text(row);
        match self.sensitive {
            true => text.contains(&self.needle),
            false => text.to_lowercase().contains(&self.needle),
        }
    }

    // -- yank ----------------------------------------------------------------------

    fn yank(text: String, what: String) -> Option<Yank> {
        match text.is_empty() {
            true => None,
            false => Some(Yank { text, what }),
        }
    }

    /// What a yank of `row` is called in the status bar.
    fn what(&self, row: usize) -> String {
        self.path_of(row)
    }
}
