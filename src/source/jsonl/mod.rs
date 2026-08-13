//! `.jsonl` / `.ndjson`: a record per *line* (SPEC.md §JSON, "`.jsonl` /
//! `.ndjson`").
//!
//! # All that is left here is where a record starts and stops
//!
//! Rows, folding, search, yanking, the outline and the lens machinery are
//! [`crate::source::record`], shared with every other record format. This file
//! is the [`Store`] behind them, and it says one thing: a record is a line.
//!
//! * **The index is the CSV one.** A line-oriented file is a CSV without
//!   quoting, so this reuses [`RowStore`] whole — the lazy byte-offset index,
//!   the block-delta offset encoding, the sliding read window, the progress
//!   report — through [`crate::csv::index::RowStore::lines`], a scanner with
//!   quoting turned off. Nothing about a multi-GB file is read on the open
//!   path: a `stat` and a 3-byte BOM peek, exactly as a CSV. There is exactly
//!   one line indexer in the crate and this is a caller of it, not a copy.
//! * **A line that is not JSON is an error row** carrying the reason and the
//!   line number, and the file keeps rendering: half a log is still worth
//!   reading.
#![deny(unsafe_code)]

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lensrow_tests.rs"]
mod lensrow_tests;

#[cfg(test)]
#[path = "block_tests.rs"]
mod block_tests;

#[cfg(test)]
#[path = "usage_tests.rs"]
mod usage_tests;

use std::cell::RefCell;
use std::io;
use std::path::Path;

use crate::csv::index::RowStore;
use crate::csv::read::{self, Reader};
use crate::json;
use crate::source::record::{Record, RecordSource, Store};

/// A `.jsonl` file behind the record seam.
pub type JsonlSource = RecordSource<Lines>;

/// The lazy line index, as a record store.
pub struct Lines {
    /// Interior mutability because reading a line is a *file* read and every
    /// [`Store`] method is `&self`. Every borrow is taken and dropped inside
    /// one method; none nest.
    store: RefCell<RowStore>,
}

impl Store for Lines {
    fn known(&self) -> usize {
        self.store.borrow().known()
    }

    fn complete(&self) -> bool {
        self.store.borrow().complete()
    }

    fn progress(&self) -> u8 {
        self.store.borrow().progress().percent()
    }

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

    fn extend(&self, budget: u64) -> bool {
        let mut guard = self.store.borrow_mut();
        let s = &mut *guard;
        match s.index.complete() {
            true => false,
            false => {
                let before = s.index.known();
                s.index.ensure_bytes(budget, &mut s.reader);
                !s.index.complete() || s.index.known() > before
            }
        }
    }

    /// The raw bytes of line `n`, terminator stripped.
    fn raw(&self, record: usize) -> Vec<u8> {
        let mut store = self.store.borrow_mut();
        store.row(record).map(|s| s.data).unwrap_or_default()
    }

    /// Read and parse line `n`. The only place a line becomes a value.
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

    fn unit(&self) -> &'static str {
        "line"
    }
}

impl JsonlSource {
    /// Open `path`. Stats it and reads three bytes; no line is indexed and no
    /// record is parsed until one is asked for.
    pub fn open(path: &Path) -> io::Result<JsonlSource> {
        Ok(RecordSource::new(Lines::new(RowStore::lines(Reader::open(path)?))))
    }

    /// A source over bytes that arrived on a pipe.
    pub fn from_bytes(data: Vec<u8>) -> JsonlSource {
        RecordSource::new(Lines::new(RowStore::lines(Reader::memory(data))))
    }
}

impl Lines {
    fn new(store: RowStore) -> Lines {
        Lines { store: RefCell::new(store) }
    }
}
