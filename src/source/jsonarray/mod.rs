//! Records that are the elements of an **array inside a JSON document**
//! (SPEC.md §Lenses, "Records inside a document").
//!
//! # Why this exists
//!
//! A `.jsonl` file is a record per line, and until now that was the only shape
//! `--lens` could read. A trajectory in the ATIF interchange format is one JSON
//! document whose records are the elements of a named array, alongside
//! top-level keys describing the run. Same records, different envelope — so
//! this is a [`Store`], not a second source: rows, folding, search, yanking,
//! the outline and every lens live in [`crate::source::record`], written once.
//!
//! # Nothing is parsed to find a record
//!
//! The records are found through the *existing* lazy structural index
//! ([`crate::source::json::tree`]): [`Doc`] walks bytes and hands back a
//! member's byte range, building no values. A record is read and parsed when it
//! is painted, exactly as a line is, and a 32KB step nobody scrolls to costs
//! nothing.
//!
//! # What it does cost, said plainly
//!
//! Finding the array is a **structural scan of the document's top level**: the
//! index emits a member only once it has walked past that member's last byte,
//! and the array is one member. So the first row waits on one byte walk of the
//! file — no parse, no allocation per record, constant memory — and the status
//! bar reports it as `\u{2265}N (indexing P%)` like any other scan. That is the
//! honest price of records living inside a document rather than one per line,
//! and SPEC.md §Lenses states it. It also buys the session row below: the
//! document's other top-level keys are all known by the time record 0 is shown,
//! so that row can never silently gain a key later.
//!
//! # Nothing is lost
//!
//! The keys that are *not* the record array — `schema_version`, `session_id`,
//! `agent` — are record **0**, a synthesised object that opens into the generic
//! tree like any other record. A lens adds interpretation and never hides data
//! (SPEC.md §Lenses), and a document reader that showed only the array would be
//! hiding the three keys that say what the run was. The cost is that record
//! numbering is shifted by one against the array's own indices: `steps[0]` is
//! record 1, which is what the status bar and `#n` say.
#![deny(unsafe_code)]

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

/// The three levels a record has under a lens, driven through the seam.
#[cfg(test)]
#[path = "level_tests.rs"]
mod level_tests;

use std::cell::RefCell;
use std::io;
use std::path::Path;

use crate::json::index::Member;
use crate::json::value::{Member as Field, Value};
use crate::source::json::tree::{Doc, NodeId, PARSE_CAP};
use crate::source::jsonrow;
use crate::source::record::{Record, RecordSource, Store};

/// Where in a document the records are.
///
/// The source's half of [`crate::lens::RecordsAt`] — the lens declares it, the
/// routing translates it, and nothing below this line knows a lens exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    /// The document's root *is* the array of records. No session row: there
    /// are no other keys for one to hold.
    Root,
    /// The records are the elements of the array under this top-level key.
    Key(&'static str),
}

/// A document whose records are an array inside it.
pub type ArraySource = RecordSource<Array>;

/// How far the search for the record array has got.
enum Found {
    /// The document's top level is still being scanned.
    Looking,
    /// The array, and which member of the root it is (`None` when the root
    /// itself is the array).
    At(NodeId, Option<usize>),
    /// The document holds no such array. Everything it *does* hold is still
    /// record 0, so nothing is lost — there is simply nothing to fold.
    Absent,
}

pub struct Array {
    /// Interior mutability because reading a member is a *file* read and every
    /// [`Store`] method is `&self`. Every borrow is taken and dropped inside
    /// one method; none nest.
    doc: RefCell<Doc>,
    at: At,
    found: RefCell<Found>,
}

impl ArraySource {
    /// Open `path`. Stats it and reads 64 bytes; nothing is indexed and no
    /// record is parsed until one is asked for.
    pub fn open(path: &Path, at: At) -> io::Result<ArraySource> {
        Ok(RecordSource::new(Array::new(Doc::open(path)?, at)))
    }

    /// A source over bytes that arrived on a pipe.
    pub fn from_bytes(data: Vec<u8>, at: At) -> ArraySource {
        RecordSource::new(Array::new(Doc::memory(data), at))
    }
}

impl Array {
    fn new(doc: Doc, at: At) -> Array {
        Array {
            doc: RefCell::new(doc),
            at,
            found: RefCell::new(Found::Looking),
        }
    }

    /// Is there a session record at index 0? Only when the records sit *inside*
    /// a document, because only then are there other keys to keep.
    fn head(&self) -> usize {
        usize::from(matches!(self.at, At::Key(_)))
    }

    /// Walk the document's top level until the record array is found, spending
    /// at most `budget` bytes. Returns nothing: ask [`Array::found`] after.
    fn locate(&self, budget: u64) {
        if !matches!(&*self.found.borrow(), Found::Looking) {
            return;
        }
        let mut doc = self.doc.borrow_mut();
        let Some(root) = doc.root() else {
            *self.found.borrow_mut() = Found::Absent;
            return;
        };
        // The top level is walked to its end before any record is served: a
        // member is only emitted once the scan has passed it, and the session
        // row must not gain keys after it has been read (see the header).
        doc.index(root, usize::MAX, budget);
        if !doc.node(root).complete() {
            return;
        }
        let key = match self.at {
            At::Root => {
                *self.found.borrow_mut() = match doc.node(root).shape.is_container() {
                    true => Found::At(root, None),
                    false => Found::Absent,
                };
                return;
            }
            At::Key(key) => key,
        };
        *self.found.borrow_mut() = match Self::named(&mut doc, root, key) {
            Some(i) => match doc.open_child(root, i) {
                Some(id) => Found::At(id, Some(i)),
                None => Found::Absent,
            },
            None => Found::Absent,
        };
    }

    /// Find the array if it is still being looked for, then index `want` of its
    /// members — all inside **one** `budget`, which is what a caller is paying
    /// for. The walk the search spent is deducted from the walk the members
    /// get, so the tick that finds the array does not spend twice.
    fn grow(&self, want: usize, budget: u64) {
        let start = self.doc.borrow().walked();
        self.locate(budget);
        let Some(id) = self.array() else {
            return;
        };
        let spent = self.doc.borrow().walked().saturating_sub(start);
        let mut doc = self.doc.borrow_mut();
        doc.index(id, want, budget.saturating_sub(spent));
    }

    /// Which member of `root` carries `key`.
    ///
    /// Through [`Doc::key_text`], which parses the key's own bytes, rather than
    /// comparing the quoted bytes: a document is free to spell `steps` as
    /// `"steps"` and it is still that key.
    fn named(doc: &mut Doc, root: NodeId, key: &str) -> Option<usize> {
        for i in 0..doc.node(root).count() {
            let m = doc.node(root).member(i)?;
            if doc.key_text(m).is_some_and(|k| k == key) {
                return Some(i);
            }
        }
        None
    }

    /// The array node, once it has been found.
    fn array(&self) -> Option<NodeId> {
        match &*self.found.borrow() {
            Found::At(id, _) => Some(*id),
            _ => None,
        }
    }

    /// The byte range of record `i`, or `None` for the session record and for
    /// anything past the index.
    fn member(&self, record: usize) -> Option<Member> {
        let id = self.array()?;
        let i = record.checked_sub(self.head())?;
        self.doc.borrow().node(id).member(i)
    }

    /// The document's other top-level keys, as one synthesised record.
    ///
    /// Built from byte ranges the index already has, one bounded parse per
    /// member: a key too large to display says so in place of its value rather
    /// than being loaded, exactly as the document reader's row does.
    fn session(&self) -> Record {
        let skip = match &*self.found.borrow() {
            Found::At(_, i) => *i,
            _ => None,
        };
        let mut doc = self.doc.borrow_mut();
        let Some(root) = doc.root() else {
            return Record::Bad("empty document".to_string());
        };
        let node = doc.node(root);
        let (shape, start, end, count) = (node.shape, node.start, node.end, node.count());
        if !shape.is_container() {
            return Record::Value(value_at(&mut doc, Member { key: None, start, end }));
        }
        let mut fields: Vec<Field> = Vec::new();
        for i in 0..count {
            if Some(i) == skip {
                continue;
            }
            let Some(m) = doc.node(root).member(i) else {
                continue;
            };
            let key = doc.key_text(m).unwrap_or_else(|| format!("{i}"));
            let value = value_at(&mut doc, m);
            fields.push(Field { key, value });
        }
        Record::Value(Value::Object(fields))
    }
}

/// One member as a value: parsed when it is small enough, and a note saying how
/// big it is when it is not (SPEC.md §JSON — a member past the parse cap
/// reports its size instead of being loaded).
fn value_at(doc: &mut Doc, m: Member) -> Value {
    if m.len() > PARSE_CAP {
        return Value::Str(jsonrow::oversize(m.len(), PARSE_CAP));
    }
    let (bytes, clipped) = doc.bytes(m.start, m.end);
    if clipped {
        return Value::Str(jsonrow::oversize(m.len(), PARSE_CAP));
    }
    match crate::json::parse(&bytes) {
        Ok(v) => v,
        Err(e) => Value::Str(format!(
            "\u{27e8}not JSON: {} at byte {}\u{27e9}",
            e.reason,
            m.start + e.offset as u64
        )),
    }
}

impl Store for Array {
    /// The session record, plus the array members indexed so far.
    fn known(&self) -> usize {
        match &*self.found.borrow() {
            Found::Looking => 0,
            Found::At(id, _) => self.head() + self.doc.borrow().node(*id).count(),
            // A document with no root value holds nothing at all, so there is
            // no session record either: an empty file is empty, not one error
            // row about being empty.
            Found::Absent => match self.doc.borrow().root() {
                Some(_) => self.head(),
                None => 0,
            },
        }
    }

    fn complete(&self) -> bool {
        match &*self.found.borrow() {
            Found::Looking => false,
            Found::At(id, _) => self.doc.borrow().node(*id).complete(),
            Found::Absent => true,
        }
    }

    /// The top-level scan while the array is still being looked for, then the
    /// array's own. Both are the same honest "how far the bytes got".
    fn progress(&self) -> u8 {
        let doc = self.doc.borrow();
        match &*self.found.borrow() {
            Found::Looking => doc.progress(),
            Found::Absent => 100,
            Found::At(id, _) => {
                let node = doc.node(*id);
                let total = node.end.saturating_sub(node.start);
                match node.complete() || total == 0 {
                    true => 100,
                    false => ((node.scanned().saturating_sub(node.start) * 100) / total) as u8,
                }
            }
        }
    }

    fn index_to(&self, records: usize, budget: u64) {
        self.grow(records.saturating_sub(self.head()), budget);
    }

    /// An idle tick spends its whole budget on the index, and asks for every
    /// member the budget will reach.
    ///
    /// Asking for `known + 1` instead made [`Doc::index`] return at the first
    /// 4KB chunk that yielded a member and threw the rest of the slice away:
    /// the index crawled forward one chunk per tick, and a 4.6MB trajectory
    /// took a minute and a half to settle. [`crate::source::jsonl`] hands its
    /// budget straight to the line index, which is the contract the pager's
    /// idle tick is written against.
    fn extend(&self, budget: u64) -> bool {
        let before = self.known();
        self.grow(usize::MAX, budget);
        !self.complete() || self.known() > before
    }

    /// The record's own bytes out of the document. The session record has none
    /// of its own — it is the keys around the array — so it is serialised.
    fn raw(&self, record: usize) -> Vec<u8> {
        let Some(m) = self.member(record) else {
            return match record < self.head() {
                true => self.session().value().map(|v| v.to_json()).unwrap_or_default().into_bytes(),
                false => Vec::new(),
            };
        };
        self.doc.borrow_mut().bytes(m.start, m.end).0
    }

    fn load(&self, record: usize) -> Record {
        let Some(m) = self.member(record) else {
            return match record < self.head() {
                true => self.session(),
                false => Record::Bad("no such record".to_string()),
            };
        };
        if m.len() > PARSE_CAP {
            let note = jsonrow::oversize(m.len(), PARSE_CAP);
            return Record::Bad(format!("{note}, not parsed"));
        }
        let (bytes, clipped) = self.doc.borrow_mut().bytes(m.start, m.end);
        if clipped {
            return Record::Bad(jsonrow::oversize(m.len(), PARSE_CAP));
        }
        match crate::json::parse(&bytes) {
            Ok(v) => Record::Value(v),
            Err(e) => Record::Bad(e.to_string()),
        }
    }

    fn unit(&self) -> &'static str {
        "record"
    }
}
