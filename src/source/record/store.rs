//! Where records come from: the one thing a record *format* still owns.
//!
//! Everything above this — the plan, the row arithmetic, folding, search, the
//! outline, every painted row — is [`super::RecordSource`], written once. What
//! a format has to answer is narrow and entirely about bytes: how many records
//! the index has found, how to push that index along inside a budget, and how
//! to hand over record `i` as bytes or as a value.
//!
//! Two formats implement it today and they could hardly be less alike: a
//! `.jsonl` file, where a record is a line of the CSV lazy line index
//! ([`crate::source::jsonl`]), and an array *inside* a JSON document, where a
//! record is a member of the lazy structural index
//! ([`crate::source::jsonarray`]). Neither knows the other exists, and neither
//! contains a single row number.
//!
//! # Everything here is `&self`
//!
//! Reading a record is a *file* read, and half of [`crate::source::Source`] is
//! `&self`; a store therefore keeps its reader behind a `RefCell` exactly as
//! the two sources above it used to. The rule that comes with it is the one
//! that was already in force: take the borrow and drop it inside one small
//! helper, never across a call back into the seam.
#![deny(unsafe_code)]

use crate::json::Value;

/// A sequence of records, as bytes.
pub trait Store {
    /// Records the index has found so far. Grows as the index is pushed.
    fn known(&self) -> usize;

    /// The index has reached the end of the records: [`Store::known`] is now
    /// the real count.
    fn complete(&self) -> bool;

    /// How far the index has got, 0..=100, for the status bar's honest
    /// `\u{2265}N (indexing 40%)`.
    fn progress(&self) -> u8;

    /// Push the index toward `records`, spending at most `budget` bytes. May
    /// stop short: a caller asks again next frame.
    fn index_to(&self, records: usize, budget: u64);

    /// Spend an idle tick on the index. True while there is more to do *or*
    /// this slice found new records.
    fn extend(&self, budget: u64) -> bool;

    /// Record `i` as the bytes the file holds — no value is built, which is
    /// what keeps a search sweep inside a keystroke and what `c` copies.
    fn raw(&self, record: usize) -> Vec<u8>;

    /// Read and parse record `i`. The only place a record becomes a value.
    fn load(&self, record: usize) -> Record;

    /// What one record is called, for a message a person reads: a `.jsonl`
    /// record is a `line`, a member of a document's array is a `record`.
    fn unit(&self) -> &'static str;
}

/// A record, as far as this reader got with it.
pub enum Record {
    /// Valid JSON.
    Value(Value),
    /// Not JSON, or too big to parse, and why. Rendered as an error row rather
    /// than stopping the file (SPEC.md §JSON).
    Bad(String),
}

impl Record {
    pub(crate) fn value(&self) -> Option<&Value> {
        match self {
            Record::Value(v) => Some(v),
            Record::Bad(_) => None,
        }
    }
}

/// Parsed records kept in hand. Small on purpose: a record can be megabytes,
/// and the viewport only ever asks about a screenful plus whatever the cursor
/// just left.
const CACHE: usize = 64;

/// The parsed-record cache: most-recently-used first, [`CACHE`] deep.
#[derive(Default)]
pub(crate) struct Cache {
    pub(crate) items: Vec<(usize, Record)>,
}

impl Cache {
    pub(crate) fn position(&self, record: usize) -> Option<usize> {
        self.items.iter().position(|(r, _)| *r == record)
    }

    /// Insert at the front, dropping the oldest if the cache is full.
    pub(crate) fn push(&mut self, record: usize, rec: Record) {
        if self.items.len() >= CACHE {
            self.items.pop();
        }
        self.items.insert(0, (record, rec));
    }
}
