//! A record document behind the [`Source`] seam: the state, and the reads.
//!
//! # One source, two formats
//!
//! This was `JsonlSource` until a second record format arrived. Nothing in it
//! was ever about a line: the rows, the folding, the search sweep, the outline
//! and the status bar all speak *records*, and the only line-shaped thing was
//! where the bytes came from. That is now [`Store`], so `.jsonl` and a JSON
//! document's array share one implementation rather than two that drift.
//!
//! # None of them read until they are shown
//!
//! * **The index is the store's**, and it is lazy in both: a `stat` and a
//!   handful of bytes on the open path, pushed a budget at a time from the
//!   frame and from the idle tick.
//! * **A record is parsed when it is shown, and only then.** A real trajectory
//!   has a single 32KB record among fifty; laying out a screen by parsing every
//!   record would be as bad as reading the whole file. Rendering a row parses
//!   that row's record, and a small [`Cache`] keeps the ones the viewport keeps
//!   asking about.
//! * **The default view is a list of records**, one row each
//!   ([`tree::record_spans`]). `Enter` / `za` expands one into the tree
//!   [`tree`] renders, spliced in under its row by [`RowMap`].
//! * **A record that is not JSON is an error row** carrying the reason and the
//!   record number, and the file keeps rendering: half a log is still worth
//!   reading.
//!
//! # What is not folded is not parsed
//!
//! Fold state here is the *open* records, not the closed ones: everything
//! starts closed, so the closed set of a million-record file would be a million
//! ids. That is the same "default plus exceptions" shape the document source
//! uses, in the same vocabulary ([`jsonrow::ALL_OPEN`]) — a fold id is a
//! member-index path from the root, and a record document's root is the
//! implicit list of records, so record 4 is `/4`. One scheme, two sources.
#![deny(unsafe_code)]

/// The [`Source`] implementation itself, and the rows under it. Submodules of
/// this file rather than siblings, so all three see one set of imports and the
/// state above stays private to them.
#[path = "view.rs"]
mod view;

#[path = "rows.rs"]
mod rows;

use std::cell::RefCell;
use std::ops::Range;

use super::super::search::{self, Dir};
use super::super::{Anchor, End, Entry, FoldState, Hit, LinkSite, Mark, MatchSpan, Source};
use super::plan::{Plan, Spot};
use super::rowmap::RowMap;
use super::store::{Cache, Record, Store};
use super::tree;
use super::{body, fold_id, leaf, lensrow, marker, ops, Records};
use crate::json::Value;
use crate::lens::Lens;
use crate::render::{Line, LineKind, Span};
use crate::select::Yank;
use crate::source::jsonrow;

/// Records the index is pushed past the painted window on every frame.
pub(crate) const LOOKAHEAD: usize = 1024;

/// Records indexed when the layout width is first set, so the first screen is
/// there without waiting for an idle tick.
const FIRST_RECORDS: usize = 256;

/// Bytes of scanning one [`Source::lines`] call may spend on that lookahead.
pub(crate) const FRAME_BYTES: u64 = 4 * 1024 * 1024;

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

/// Records `zR` will open at once.
///
/// Opening *every* record of a million-record log means parsing the whole file,
/// which is the one thing this source exists not to do. `zR` therefore opens as
/// far as the reader is looking and says so, rather than freezing (see
/// [`Source::fold_all`]).
pub(crate) const EXPAND_CAP: usize = 50_000;

pub struct RecordSource<S: Store> {
    /// Where the records come from. The one part of this that a format owns.
    pub(crate) store: S,
    cache: RefCell<Cache>,
    /// The rows of the one expanded record the viewport is working through.
    /// Capacity one: rebuilding it costs a walk of that record, and a frame
    /// asks for it once per painted row.
    laid: RefCell<Option<(usize, Vec<Line>)>>,
    /// The wrapped message rows of the one record the viewport is reading,
    /// keyed by the width they were laid out for. Same discipline as `laid`,
    /// and for the same reason: a frame asks per painted row, and wrapping a
    /// 40 KB message once per row of it would be quadratic.
    body_laid: RefCell<Option<(usize, usize, Vec<Line>)>>,
    /// Which records are open, and what that does to the row numbering.
    pub(crate) map: RowMap,
    /// The `--lens` reading of this file, when one was asked for. `None` is
    /// the generic record tree, unchanged in every respect (SPEC.md §Lenses:
    /// "without it, records render as the generic tree").
    pub(crate) plan: Option<Plan>,
    /// `zR` was pressed (or a dump asked for everything): records are opened as
    /// the viewport reaches them, up to [`EXPAND_CAP`].
    pub(crate) expand_all: bool,
    /// Records `expand_all` has already considered.
    pub(crate) filled: usize,
    pub(crate) view: usize,
    /// Outline entries for the records last painted — see [`Source::outline`].
    pub(crate) outline: Vec<Entry>,
    /// Rows last painted.
    pub(crate) window: Range<usize>,
    query: String,
    needle: String,
    sensitive: bool,
    /// Row of the current match, when the last sweep found one.
    found: Option<usize>,
    /// Always empty: a record file has no links. Held so the trait can hand
    /// out a slice.
    pub(crate) none_links: Vec<LinkSite>,
}

/// A record document is a sequence of records: whatever the store indexed,
/// parsed when it is shown and not before.
impl<S: Store> Records for RecordSource<S> {
    fn known(&self) -> usize {
        self.store.known()
    }

    /// Straight through the parsed-record cache, so the seam pays nothing extra
    /// for asking: a record the frame is about to paint is already in hand.
    fn with_value<T>(&self, record: usize, f: impl FnOnce(Option<&Value>) -> T) -> T {
        self.with_record(record, |rec| f(rec.value()))
    }

    fn foldable(&self, record: usize) -> bool {
        self.tree_len(record) > 0
    }
}

impl<S: Store> RecordSource<S> {
    pub(crate) fn new(store: S) -> RecordSource<S> {
        RecordSource {
            store,
            cache: RefCell::new(Cache::default()),
            laid: RefCell::new(None),
            body_laid: RefCell::new(None),
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

    /// Grow the index toward `records`, spending at most `budget` bytes.
    pub(crate) fn index_to(&self, records: usize, budget: u64) {
        self.store.index_to(records, budget);
    }

    /// Records indexed so far.
    pub(crate) fn known(&self) -> usize {
        self.store.known()
    }

    pub(crate) fn complete(&self) -> bool {
        self.store.complete()
    }

    /// The raw bytes of record `n`.
    pub(crate) fn raw(&self, record: usize) -> Vec<u8> {
        self.store.raw(record)
    }

    /// The raw text of record `n`, for the search sweep: no value is built and
    /// no row is laid out, which is what keeps a sweep inside a keystroke.
    fn raw_text(&self, record: usize) -> String {
        String::from_utf8_lossy(&self.raw(record)).into_owned()
    }

    // -- records ---------------------------------------------------------------

    /// Run `f` against record `n`, parsing it if it is not already in hand.
    ///
    /// A closure rather than a returned reference because the record lives in a
    /// [`RefCell`] and can be tens of megabytes: handing out a clone to read one
    /// field would undo the point of the cache.
    pub(crate) fn with_record<T>(&self, record: usize, f: impl FnOnce(&Record) -> T) -> T {
        if self.cache.borrow().position(record).is_none() {
            let loaded = self.store.load(record);
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
    pub(crate) fn tree_len(&self, record: usize) -> usize {
        self.with_record(record, |r| r.value().map(tree::row_count).unwrap_or(0))
    }

    /// Run `f` against the laid-out rows of record `n`'s tree, laying them out
    /// if the cache is holding a different record.
    pub(crate) fn with_tree<T>(&self, record: usize, f: impl FnOnce(&[Line]) -> T) -> T {
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
    /// that into a row is [`RecordSource::hit`]'s job, because it may have to
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
        // On a body row the value under the cursor is what was said, whole —
        // not the record wrapped around it. `Y` still copies the record.
        if let Spot::Body { record, .. } = self.spot(row) {
            return self.body_text(record);
        }
        self.at_row(row, |_, v| match v {
            Value::Str(s) => s.clone(),
            other => other.to_json(),
        })
    }
}

/// The lens state on the source, and the thin forwarders the rest of it calls.
///
/// Nothing about lenses is decided here. [`super::plan`], [`super::lensrow`]
/// and [`super::ops`] own the plan, the row arithmetic, group folding and every
/// painted row; this block only holds the state, because that is where the fold
/// state already lives. Every method is one line long on purpose.
impl<S: Store> RecordSource<S> {
    /// Read this file through `lens` (SPEC.md §Lenses).
    ///
    /// A transform over the records, not a different source: the file, the
    /// index, the parser and every row below a summary are unchanged. What the
    /// lens decides is what each record's *one* row says and which runs of
    /// records share one.
    pub fn set_lens(&mut self, lens: Box<dyn Lens>) {
        self.plan = Some(Plan::new(lens));
    }

    /// The lens in force, for the status bar.
    pub fn lens_name(&self) -> Option<&'static str> {
        self.plan.as_ref().map(|p| p.lens_name())
    }

    /// Classify records up to and including `upto`.
    pub(crate) fn classify_to(&mut self, upto: usize) {
        // Both are moved out for the call: reading a record borrows `self`
        // immutably through the `RefCell`s, and the classification writes to
        // these two. Put back before returning, always.
        let Some(mut plan) = self.plan.take() else {
            return;
        };
        let mut map = std::mem::take(&mut self.map);
        lensrow::classify_to(&*self, &mut plan, &mut map, upto);
        self.plan = Some(plan);
        self.map = map;
    }

    /// Where a screen row falls.
    pub(crate) fn spot(&self, row: usize) -> Spot {
        lensrow::spot(self.plan.as_ref(), &self.map, self.known(), row)
    }

    /// The row a record's summary sits on.
    pub(crate) fn row_of_record(&self, record: usize) -> usize {
        lensrow::row_of_record(self.plan.as_ref(), &self.map, record)
    }

    /// Is `record` currently a row of its own?
    pub(crate) fn record_visible(&self, record: usize) -> bool {
        lensrow::record_visible(self.plan.as_ref(), record)
    }

    /// A record's row as the lens reads it, or `None` when the lens did not
    /// recognise it — the caller then renders the generic tree row.
    pub(crate) fn lens_row(&self, record: usize, inset: bool) -> Option<Line> {
        lensrow::lens_row(self, self.plan.as_ref(), record, inset)
    }

    /// A folded run of mechanics: `⟨6 steps · 4 tool calls⟩`.
    pub(crate) fn group_row(&self, item: usize) -> Line {
        lensrow::group_row(self.plan.as_ref(), item)
    }

    /// The record a row belongs to.
    pub(crate) fn record_at(&self, row: usize) -> usize {
        lensrow::record_at(self.plan.as_ref(), &self.map, self.known(), row)
    }

    /// The first record of a plan item.
    pub(crate) fn item_first(&self, item: usize) -> usize {
        lensrow::item_first(self.plan.as_ref(), item)
    }

    /// Is this record inside a group that is open? Its row is inset if so.
    pub(crate) fn in_open_group(&self, record: usize) -> bool {
        lensrow::in_open_group(self.plan.as_ref(), record)
    }

    // -- folding ---------------------------------------------------------------
    //
    // Group folding is the seam's, whole: these forward and decide nothing. The
    // only fold left is opening a *record* into its tree, which costs a parse.

    /// `zR` under a lens opens the groups the viewport has reached.
    pub(crate) fn open_groups(&mut self, upto_row: usize) {
        ops::open_upto(self.plan.as_mut(), &mut self.map, upto_row);
    }

    /// `zM`.
    pub(crate) fn close_groups(&mut self) {
        ops::close_all(self.plan.as_mut(), &mut self.map);
    }

    /// Open the group holding `record`, so a search hit is never left folded.
    pub(crate) fn reveal_record(&mut self, record: usize) {
        ops::reveal(self.plan.as_mut(), &mut self.map, record);
    }

    /// The fold id of whatever sits on `at` — a group's or a record's.
    pub(crate) fn fold_id_at(&self, at: Spot) -> String {
        ops::id_at(self.plan.as_ref(), at)
    }

    /// Apply a fold id that may name a group; `None` leaves it to the record
    /// half of [`Source::set_fold`].
    pub(crate) fn set_group_by_id(&mut self, id: &str, open: bool) -> Option<bool> {
        ops::set_by_id(self.plan.as_mut(), &mut self.map, id, open)
    }

    /// The open groups, as fold ids.
    pub(crate) fn group_folds(&self) -> Vec<String> {
        ops::open_ids(self.plan.as_ref())
    }

    /// Shut every group, then reopen the ones the fold state names.
    pub(crate) fn restore_groups(&mut self, folds: &[String]) {
        ops::restore(self.plan.as_mut(), &mut self.map, folds);
    }

    /// `Tab` / `S-Tab` under a lens: the next item, not the next record.
    pub(crate) fn next_item(&self, row: usize, forward: bool) -> Option<usize> {
        ops::next_item(self.plan.as_ref(), &self.map, self.known(), row, forward)
    }

    /// The rows of the block a row falls in.
    pub(crate) fn block_rows(&self, row: usize) -> Option<std::ops::Range<usize>> {
        ops::block_at(self.plan.as_ref(), &self.map, row)
    }

    /// `(index, count)` of the block a row is on, for the status bar.
    pub(crate) fn block_of_row(&self, row: usize) -> Option<(usize, usize)> {
        ops::block_of_row(self.plan.as_ref(), &self.map, row)
    }

    /// The status bar's block clause — `  ·  block 2/23` — or nothing at all.
    ///
    /// Nothing in three cases, each of them honest rather than tidy: with no
    /// lens there are no blocks, and past the classified prefix the block a row
    /// will end up in is not decided yet, so a number there would be one the
    /// next keystroke changes. The total carries the same `≥` the record count
    /// does, and for the same reason twice over: the lens has only read a
    /// prefix, and grouping makes that total *shrink* as it catches up.
    pub(crate) fn block_text(&self, row: usize) -> String {
        let Some((i, n)) = self.block_of_row(row) else {
            return String::new();
        };
        let classified = self.plan.as_ref().map(|p| p.classified()).unwrap_or(0);
        let total = match self.complete() && classified >= self.known() {
            true => format!("{n}"),
            false => format!("\u{2265}{n}"),
        };
        format!("  \u{b7}  block {}/{total}", i.saturating_add(1))
    }

    /// `Tab` / `S-Tab` under a lens: the next message, not the next block.
    pub(crate) fn next_message_row(&self, row: usize, forward: bool) -> Option<usize> {
        ops::next_message(self.plan.as_ref(), &self.map, self.known(), row, forward)
    }

    /// `Y` on a group's row: every record the run holds.
    pub(crate) fn yank_group(&self, item: usize) -> Option<Yank> {
        ops::yank_group(self, self.plan.as_ref(), item)
    }
}
