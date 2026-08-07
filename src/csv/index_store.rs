//! [`RowStore`]: the file, its read window and its row index as one thing.
//!
//! Split from `index.rs`, which owns the index itself, so both stay under the
//! size limit; it is re-exported there and every caller still spells it
//! `crate::csv::index::RowStore`. This is the whole surface everything above
//! the format seam uses to read a big file: the constructors that decide which
//! row *grammar* the index runs ([`RowStore::open`] for CSV,
//! [`RowStore::lines`] for a record per line), and [`RowStore::row`], which is
//! O(1) once indexed.
#![deny(unsafe_code)]

use std::io;
use std::path::Path;

use super::super::parse;
use super::super::read::{self, Reader, Span};
use super::{origin, Progress, RowIndex, Scanner};

/// A file, its read window and its row index: the whole big-file access layer.
///
/// Everything above the seam asks this for row *bytes*; splitting a row into
/// fields is [`super::parse`]'s job and laying them out is the source's.
pub struct RowStore {
    pub reader: Reader,
    pub index: RowIndex,
}

impl RowStore {
    /// Open `path`. Stats the file and reads at most three bytes to skip a
    /// BOM; no row is indexed until somebody asks for one, so open time does
    /// not depend on file size.
    pub fn open(path: &Path, delim: u8) -> io::Result<RowStore> {
        let mut reader = Reader::open(path)?;
        let index = RowIndex::new(origin(&mut reader), delim);
        Ok(RowStore { reader, index })
    }

    /// A store over bytes that arrived on a pipe. See [`Reader::memory`].
    pub fn memory(data: Vec<u8>, delim: u8) -> RowStore {
        let mut reader = Reader::memory(data);
        let index = RowIndex::new(origin(&mut reader), delim);
        RowStore { reader, index }
    }

    /// The same store over the *line* grammar: rows end at `LF`, `CRLF` or a
    /// bare `CR` and at nothing else ([`super::parse::Scanner::lines`]).
    ///
    /// This is the constructor for every format that is a record per line —
    /// `.jsonl` and plain text — and it lives here, next to `open` and
    /// `memory`, because there is exactly one line indexer in this crate and
    /// two copies of the three lines that build it would be the way it
    /// silently grew a second. Running the *CSV* grammar over such a file
    /// instead would let one `"` in a comment swallow the rest of it.
    pub fn lines(mut reader: Reader) -> RowStore {
        let index = RowIndex::with_scanner(origin(&mut reader), Scanner::lines());
        RowStore { reader, index }
    }

    /// Rows known so far.
    pub fn known(&self) -> usize {
        self.index.known()
    }

    /// True once every row is indexed.
    pub fn complete(&self) -> bool {
        self.index.complete()
    }

    /// Index far enough to answer for `n` rows. Returns rows now known. The
    /// source drives the index itself, under one borrow of the store; this is
    /// the tests' one-liner.
    #[cfg(test)]
    pub fn ensure(&mut self, n: usize) -> usize {
        self.index.ensure(n, &mut self.reader)
    }

    /// See [`RowIndex::scan_all`] — a test driver, not the product's.
    #[cfg(test)]
    pub fn scan_all(&mut self, tick: &mut dyn FnMut(Progress) -> bool) -> Progress {
        self.index.scan_all(&mut self.reader, tick)
    }

    /// Where the scan has got to.
    pub fn progress(&self) -> Progress {
        self.index.progress(&self.reader)
    }

    /// Pick up growth or truncation of the open file. See
    /// [`RowIndex::refresh`].
    #[cfg(test)]
    pub fn refresh(&mut self) -> bool {
        self.index.refresh(&mut self.reader)
    }

    /// The bytes of row `i`, terminator stripped, indexing on demand.
    ///
    /// O(1) once indexed: one seek and one read, normally served out of the
    /// window so a screenful of rows costs a single syscall. `None` past the
    /// last row. A row longer than [`super::read::MAX_ROW_BYTES`] comes back
    /// clipped with [`Span::truncated`] set rather than allocated in full.
    ///
    /// **Bounded.** Row `i` ends where row `i + 1` starts, and finding that is
    /// an unbounded scan on a file whose tail holds no terminator — which is why
    /// this goes through [`RowIndex::span_within`] and spends at most one
    /// [`super::read::MAX_ROW_BYTES`] of scanning. This function is on the paint
    /// path of every format that is a record per line, so an unbounded call here
    /// is a frame that never flushes and a `q` that is never read.
    pub fn row(&mut self, i: usize) -> Option<Span> {
        let budget = read::MAX_ROW_BYTES as u64;
        let (start, end, settled) = self.index.span_within(i, budget, &mut self.reader)?;
        let mut span = self.reader.bytes(start, end);
        // `end` is only where the bounded scan stopped, so there is more of this
        // row in the file: the bytes handed out are a prefix and must say so.
        // `Reader::bytes` cannot tell — from its side `start..end` was served in
        // full — and a row silently reported as intact when it is not is how a
        // clipped value would come to look like the whole value.
        span.truncated |= !settled;
        // Only strip bytes the parser actually consumed as a terminator. The
        // last row of a file that ends mid-row has none, and its final byte can
        // perfectly well be an `LF` inside a quoted field — stripping that by
        // shape would silently eat data the field parser keeps. A clipped or
        // short-read span is not the row's real tail either, so leave it alone —
        // and neither is a span whose end is only where the scan stopped
        // (`!settled`), which by construction is clipped as well.
        if settled && !span.truncated && self.index.terminated(i) {
            parse::strip_terminator(&mut span.data);
        }
        Some(span)
    }
}
