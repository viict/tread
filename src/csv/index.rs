//! The lazy byte-offset row index (SPEC.md §CSV, "Row index").
//!
//! A multi-GB CSV must open and quit instantly, so nothing here runs on the open
//! path except a `stat` and one 3-byte BOM peek. The index records the byte
//! offset of each row's first byte and grows *only* as far as somebody asks:
//! painting the first screen indexes the first screen, scrolling extends it, the
//! idle tick extends it further. Every one goes through [`RowIndex::ensure`] or
//! [`RowIndex::ensure_bytes`] — the whole driving surface — so a forced full scan
//! (`G`) is a caller spending slice after slice, which is what makes it
//! interruptible and what leaves the progress report to [`RowIndex::progress`].
//! Rows are never held in memory: [`RowStore::row`] seeks and reads, O(1).
//!
//! # Where a row ends
//!
//! A newline inside a quoted field is not a row boundary, and one wrong boundary
//! corrupts every offset after it. The rules therefore live in exactly one place
//! — [`super::parse::Scanner`], the machine the renderer splits fields with —
//! and this module only drives it, through [`super::parse::scan_row_ends`] and
//! [`super::parse::finish_row_end`]. Nothing here knows what a quote is, which
//! is what lets `.jsonl` drive this same index with
//! [`super::parse::Scanner::lines`]. The scanner resumes across chunk
//! boundaries, one splitting a `\r\n` included, so the index walks the file a
//! window at a time with no lookback.
//!
//! # Memory
//!
//! Offsets are stored as a `u32` delta from a per-[`BLOCK`] (1024-row) `u64`
//! base, so a row costs 4 bytes plus 8 bytes per 1024 rows: ~40MB for 10M rows,
//! against ~80MB for a flat `Vec<u64>`. Lookup stays O(1) — one index into
//! `bases`, one into `deltas`. A single block spanning more than 4GiB (a file of
//! megabyte rows) puts its offsets in the `spill` map, still O(1) and exact.
#![deny(unsafe_code)]

use std::collections::HashMap;

use super::parse::{self, Scanner};
use super::read::{Reader, WINDOW};

/// [`RowStore`] — the file, its window and this index together — split out to
/// keep both files under the size limit. Re-exported, so it stays
/// `crate::csv::index::RowStore` to every caller: the access layer is one idea
/// and moving it between files is not a change to the seam.
#[path = "index_store.rs"]
mod store;
pub use store::RowStore;

/// Rows per offset block. Each block carries one `u64` base.
pub const BLOCK: usize = 1024;

/// Delta sentinel meaning "this row's offset is in `spill`".
const SPILL: u32 = u32::MAX;

/// Bytes a test scan consumes per slice, standing in for the pager's own
/// per-tick budget. Nothing in the binary reads it: the size of a slice is the
/// caller's choice, and the pager's is a wall-clock one.
#[cfg(test)]
pub const TICK_BYTES: u64 = 4 * 1024 * 1024;

/// How far a scan has got, for the progress report `G` must show instead of
/// freezing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Rows indexed so far.
    pub rows: usize,
    /// Bytes of the file consumed so far.
    pub bytes: u64,
    /// Bytes there are to consume, as last observed.
    pub total: u64,
    /// The whole file is indexed: `rows` is the final row count.
    pub complete: bool,
}

impl Progress {
    /// Fraction scanned, 0..=100. Never divides by zero.
    pub fn percent(&self) -> u8 {
        if self.complete || self.total == 0 {
            return 100;
        }
        ((self.bytes.min(self.total) * 100) / self.total) as u8
    }
}

/// The offsets themselves, in the block-delta encoding described above.
///
/// Split out of [`RowIndex`] so the scan callback can borrow it while the
/// scanner state is borrowed alongside.
#[derive(Default)]
struct Offsets {
    bases: Vec<u64>,
    deltas: Vec<u32>,
    spill: HashMap<usize, u64>,
}

impl Offsets {
    fn len(&self) -> usize {
        self.deltas.len()
    }

    fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }

    fn get(&self, i: usize) -> Option<u64> {
        let delta = *self.deltas.get(i)?;
        if delta == SPILL {
            return self.spill.get(&i).copied();
        }
        Some(self.bases[i / BLOCK] + delta as u64)
    }

    /// Record the next row's start. Offsets only ever grow, so the delta from
    /// the block's base cannot underflow.
    fn push(&mut self, offset: u64) {
        let i = self.deltas.len();
        if i % BLOCK == 0 {
            self.bases.push(offset);
        }
        let delta = offset.saturating_sub(self.bases[i / BLOCK]);
        if delta < SPILL as u64 {
            self.deltas.push(delta as u32);
        } else {
            self.deltas.push(SPILL);
            self.spill.insert(i, offset);
        }
    }

    /// Only [`RowIndex::shrink_to`] needs this, and only a file that shrank
    /// under an open reader reaches that.
    #[cfg(test)]
    fn truncate(&mut self, n: usize) {
        if n >= self.deltas.len() {
            return;
        }
        for i in n..self.deltas.len() {
            self.spill.remove(&i);
        }
        self.deltas.truncate(n);
        self.bases.truncate(n.div_ceil(BLOCK));
    }
}

/// Where row 0 starts: past a UTF-8 BOM. All that opening a file costs.
pub fn origin(r: &mut Reader) -> u64 {
    parse::bom_len(r.chunk(0, parse::BOM.len())) as u64
}

/// Byte offsets of every row indexed so far.
pub struct RowIndex {
    offs: Offsets,
    /// Offset of row 0 — past a UTF-8 BOM when there is one.
    origin: u64,
    /// Where scanning resumes. Always inside the last row pushed, which has
    /// not seen its terminator yet.
    cursor: u64,
    /// Mid-row state of the shared machine, carried across windows so a row
    /// larger than the window costs no extra memory.
    scanner: Scanner,
    /// The pristine machine: what a restart resets to, and not
    /// `Scanner::new(delim)`, which would forget the line grammar. Only a file
    /// that moved under an open reader restarts, which is a `cfg(test)` path.
    #[cfg_attr(not(test), allow(dead_code))]
    start: Scanner,
    /// End of data, meaningful once `complete`.
    end: u64,
    complete: bool,
    /// The last row's end was not settled by a terminator the scanner had
    /// finished reading — it either ran off the end of the file or stopped on a
    /// bare `CR` that one more byte could turn into a `CRLF`. Either way a file
    /// that grows must rescan that row instead of trusting where it ended.
    unterminated: bool,
    /// The last row's final byte *is* a terminator, so [`trim_terminator`] may
    /// strip it. Distinct from `unterminated`: a file ending in a bare `CR` is
    /// both terminated and not settled.
    last_terminated: bool,
}

impl RowIndex {
    /// An index over a file starting at `origin` (past the BOM), `delim`-separated.
    pub fn new(origin: u64, delim: u8) -> RowIndex {
        RowIndex::with_scanner(origin, Scanner::new(delim))
    }

    /// The constructor `new` is made of: the row grammar is the caller's.
    pub fn with_scanner(origin: u64, scanner: Scanner) -> RowIndex {
        RowIndex {
            offs: Offsets::default(),
            origin,
            cursor: origin,
            scanner,
            start: scanner,
            end: origin,
            complete: false,
            unterminated: false,
            last_terminated: true,
        }
    }

    /// Rows indexed so far. Not the row count unless [`RowIndex::complete`].
    pub fn known(&self) -> usize {
        self.offs.len()
    }

    /// The whole file has been indexed.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Total rows, once they are known. The status bar asks
    /// [`RowIndex::known`] and [`RowIndex::complete`]; this is the tests'
    /// spelling of the same fact.
    #[cfg(test)]
    pub fn total(&self) -> Option<usize> {
        self.complete.then(|| self.offs.len())
    }

    /// Offset of row 0.
    pub fn origin(&self) -> u64 {
        self.origin
    }

    /// Byte offset of row `i`. O(1). Reading a row wants the whole
    /// [`RowIndex::span`], so only the tests ask for one end of it.
    #[cfg(test)]
    pub fn offset(&self, i: usize) -> Option<u64> {
        self.offs.get(i)
    }

    /// `start..end` of row `i` *including* its terminator, or `None` when the
    /// row is not indexed far enough yet or does not exist.
    pub fn span(&self, i: usize) -> Option<(u64, u64)> {
        let start = self.offs.get(i)?;
        let end = match self.offs.get(i + 1) {
            Some(next) => next,
            None if self.complete => self.end,
            None => return None,
        };
        Some((start, end.max(start)))
    }

    /// Index one more window's worth of rows. False when there is nothing left.
    pub fn step(&mut self, reader: &mut Reader) -> bool {
        if self.complete {
            return false;
        }
        let size = reader.refresh_size();
        if self.offs.is_empty() {
            if size <= self.origin {
                self.finish(self.origin, parse::Tail::None);
                return true;
            }
            self.offs.push(self.origin);
        }
        let buf = reader.chunk(self.cursor, WINDOW).to_vec();
        let at_eof = self.cursor + buf.len() as u64 >= size;
        self.scan_buf(&buf, size);
        self.cursor += buf.len() as u64;
        // An empty read means end-of-file, or a file that vanished under us:
        // either way stop, and let `refresh` restart the scan if it grows.
        if at_eof || buf.is_empty() {
            let tail = parse::finish_row_end(&mut self.scanner);
            self.finish(self.cursor, tail);
        }
        true
    }

    /// Feed one window to the shared state machine, pushing a row start for
    /// every boundary it reports.
    fn scan_buf(&mut self, buf: &[u8], size: u64) {
        let base = self.cursor;
        let offs = &mut self.offs;
        // `scan_row_ends` reports the offset one *past* each terminator, i.e.
        // the next row's first byte. At end-of-data that is not a row.
        parse::scan_row_ends(&mut self.scanner, buf, base, |at| {
            if at < size {
                offs.push(at);
            }
        });
    }

    fn finish(&mut self, end: u64, tail: parse::Tail) {
        self.end = end;
        self.complete = true;
        self.unterminated = tail.has_row();
        self.last_terminated = tail.terminated();
    }

    /// True when row `i`'s last byte is a terminator the scanner consumed, and
    /// so may be stripped from the bytes handed out. Only the final row of a
    /// fully indexed file can fail this: every other row is followed by another
    /// one, which is only true because a terminator separated them.
    pub fn terminated(&self, i: usize) -> bool {
        self.last_terminated || !self.complete || i + 1 < self.offs.len()
    }

    /// Extend the index until at least `n` rows are known or the file ends.
    /// Returns the rows now known.
    ///
    /// **Unbounded**, and so not for the paint path: on a file whose tail holds
    /// no terminator, proving that row `n - 1` does not exist means scanning to
    /// end-of-file. Only the tests and the deliberate full scans call this; a
    /// frame asks [`RowIndex::span_within`], which spends a slice and settles
    /// for a clipped row rather than a stall.
    #[cfg(test)]
    pub fn ensure(&mut self, n: usize, reader: &mut Reader) -> usize {
        while self.offs.len() < n && !self.complete {
            self.step(reader);
        }
        self.offs.len()
    }

    /// Row `i`'s byte span for *painting*, spending at most `budget` bytes of
    /// scanning to find where it ends. `(start, end, settled)`.
    ///
    /// This exists because [`RowIndex::span`] can only answer once row `i + 1`
    /// has been found or the file has been indexed to its end, and "find the
    /// next terminator" is an unbounded amount of work: a 2GB file holding no
    /// `LF` after its first bytes has exactly one row, and asking where that row
    /// ends reads all 2GB. Behind [`RowStore::row`] — which every frame calls —
    /// that meant the first paint was blocked for seconds to minutes and `q` sat
    /// unread in the input queue, against SPEC.md §CSV ("a multi-GB file must
    /// open instantly and quit instantly ... `q` must never wait on a scan").
    /// The same path is reached by plain text and `.jsonl`, and since SPEC.md
    /// §Plain text made *every* unknown extension a text file it is reached by
    /// `.bin`, `.zip` and a minified one-line bundle as well.
    ///
    /// The way out is that nothing above ever needs more of a row than
    /// [`super::read::MAX_ROW_BYTES`]: a longer row is handed back clipped. So
    /// the end only has to be looked for within that budget. Past it, `end` is
    /// wherever the scan happens to have got to and `settled` is false — bytes
    /// that provably belong to row `i`, because the scanner reported no boundary
    /// inside them — and the span comes back clipped and flagged, exactly as a
    /// genuinely megabyte-long row does. `budget` bytes is the *whole* cost of
    /// painting one such row, so the frame is drawn and the keystroke queue is
    /// read; the rest of the file is indexed by the idle tick, a slice at a
    /// time, as it always was.
    ///
    /// `settled == false` cannot mean "the terminator was just past where we
    /// stopped": on that path [`RowIndex::ensure_bytes`] returned without the
    /// index being complete, so it consumed its whole budget, so
    /// `end - start >= budget` and the row is over the clip limit regardless of
    /// where it really ends.
    pub fn span_within(
        &mut self,
        i: usize,
        budget: u64,
        reader: &mut Reader,
    ) -> Option<(u64, u64, bool)> {
        if let Some((start, end)) = self.span(i) {
            return Some((start, end, true));
        }
        self.ensure_bytes(budget, reader);
        if let Some((start, end)) = self.span(i) {
            return Some((start, end, true));
        }
        // Row `i` was not even reached inside the budget: it is more than
        // `budget` bytes of scanning beyond the index, which no caller does —
        // every source's `len()` is the rows already known. Report no row rather
        // than scan on.
        let start = self.offs.get(i)?;
        Some((start, self.cursor.max(start), false))
    }

    /// Extend the index by at most `budget` bytes of scanning; returns the
    /// bytes actually consumed. Lets a caller spend a bounded slice of a frame
    /// on indexing and come back for the rest next frame.
    pub fn ensure_bytes(&mut self, budget: u64, reader: &mut Reader) -> u64 {
        let from = self.cursor;
        while !self.complete && self.cursor - from < budget {
            self.step(reader);
        }
        self.cursor - from
    }

    /// Where the scan has got to.
    pub fn progress(&self, reader: &Reader) -> Progress {
        Progress {
            rows: self.offs.len(),
            bytes: self.cursor.saturating_sub(self.origin),
            total: reader.size().saturating_sub(self.origin),
            complete: self.complete,
        }
    }

    /// Scan to the end a slice at a time, `tick` deciding after each whether
    /// to stop. A *test* driver, not the product's: `G` is driven from the
    /// pager, which spends a wall-clock slice per idle tick through
    /// [`RowIndex::ensure_bytes`] and hands control back to the input loop
    /// between them. Both share what is asserted here — whatever was scanned
    /// is kept, and calling again resumes from there.
    #[cfg(test)]
    pub fn scan_all(
        &mut self,
        reader: &mut Reader,
        tick: &mut dyn FnMut(Progress) -> bool,
    ) -> Progress {
        while !self.complete {
            self.ensure_bytes(TICK_BYTES, reader);
            if self.complete || tick(self.progress(reader)) {
                break;
            }
        }
        self.progress(reader)
    }

    /// Re-stat the file and reconcile a file that moved under us: one appended
    /// to resumes indexing, one truncated drops the rows that no longer exist.
    /// Returns true when the index changed. Never panics, whatever the file
    /// did.
    ///
    /// `tread` reads the snapshot it opened, so no path in the binary reaches
    /// this; a *lazy* index must survive the file it is half-way through being
    /// rewritten under it anyway, which is what the tests below pin.
    #[cfg(test)]
    pub fn refresh(&mut self, reader: &mut Reader) -> bool {
        let size = reader.refresh_size();
        if size < self.cursor || (self.complete && size < self.end) {
            self.shrink_to(size);
            return true;
        }
        if self.complete && size > self.end {
            self.grow_from();
            return true;
        }
        false
    }

    /// The file shrank: keep the rows that still start inside it and rescan
    /// the last of them, whose tail may have been cut off.
    #[cfg(test)]
    fn shrink_to(&mut self, size: u64) {
        let keep = self.rows_before(size);
        self.offs.truncate(keep);
        self.cursor = match keep {
            0 => self.origin,
            n => self.offs.get(n - 1).unwrap_or(self.origin),
        };
        self.scanner = self.start;
        self.complete = false;
        self.unterminated = false;
        self.last_terminated = true;
        self.end = self.cursor;
    }

    /// The file grew. Where scanning resumes depends on how it ended: an
    /// unterminated last row did not end where we thought and is rescanned
    /// from its start, whereas a file that ended on a terminator has gained a
    /// brand new row starting exactly there.
    #[cfg(test)]
    fn grow_from(&mut self) {
        if self.unterminated && !self.offs.is_empty() {
            let last = self.offs.len() - 1;
            self.cursor = self.offs.get(last).unwrap_or(self.origin);
        } else if !self.offs.is_empty() {
            self.offs.push(self.end);
            self.cursor = self.end;
        }
        self.scanner = self.start;
        self.complete = false;
        self.unterminated = false;
        self.last_terminated = true;
    }

    /// How many rows start strictly before `limit`.
    #[cfg(test)]
    fn rows_before(&self, limit: u64) -> usize {
        let (mut lo, mut hi) = (0, self.offs.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.offs.get(mid) {
                Some(off) if off < limit => lo = mid + 1,
                _ => hi = mid,
            }
        }
        lo
    }
}


#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
