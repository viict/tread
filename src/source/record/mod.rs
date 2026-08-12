//! Records in general, above any one record *format* (SPEC.md §Lenses).
//!
//! # What lives here
//!
//! **The** record source, and everything a lens does to it: [`RecordSource`]
//! ([`source`], with [`view`][`source`]'s `Source` impl and its rows beside
//! it), [`tree`] (one record laid out as tree rows), [`plan`] (which records
//! share a row, and the two-level row arithmetic), [`rowmap`] (which rows an
//! open record owns), [`lensrow`] (the who/when/what rows a lens paints, and
//! the row-to-record translation under them), [`ops`] (what `zR`, `zM`, `Tab`,
//! `Y` and a fold id off the outline do to a plan), and the shared gutter and
//! fold vocabulary below — [`marker`], [`leaf`], and `/4` for a record beside
//! [`plan::group_id`]'s `g4` for a group, the two spellings that must not
//! collide.
//!
//! Nothing here opens a file, names a format, or mentions a line. What a record
//! *format* supplies is [`Store`]: how many records the index has found, how to
//! push it along inside a byte budget, and record `i` as bytes or as a value.
//! `src/source/jsonl/` is a record per line over the CSV lazy line index and
//! `src/source/jsonarray/` is an array inside a JSON document over the
//! structural one; both get rows, grouping, folding, search, yanking and the
//! outline for nothing, and neither holds a row number. A change all lenses
//! need belongs here, not in a dialect under `src/lens/`.
//!
//! What the format still owns is what costs a *parse*: opening one record into
//! its own tree, and how many rows that tree has. That is the line — grouping
//! is decided from summaries the plan already holds, so it is free and lives
//! here; expansion reads the record, so it stays with whoever can read it.
//!
//! # Why the trait is generic rather than `dyn`
//!
//! [`Records::with_value`] hands the record to a closure instead of returning a
//! reference, which makes the trait non-object-safe — deliberately. A record
//! can be tens of megabytes and lives behind a `RefCell` in the format that
//! owns it, so handing out a clone to read one field would undo the point of
//! its cache, and a borrow cannot outlive the cell. The `&mut dyn FnMut`
//! alternative would also forbid *returning* a `T` from the closure, which
//! every caller of the jsonl equivalent relies on.
//!
//! The non-object-safety costs nothing above the seam: the functions in
//! [`lensrow`] are free and generic, they are monomorphised inside this module,
//! and the format itself keeps implementing [`crate::source::Source`]
//! concretely — the pager still holds a `Box<dyn Source>` and never learns that
//! any of this exists.
#![deny(unsafe_code)]

pub mod lensrow;
pub mod ops;
pub mod plan;
pub mod rowmap;
mod source;
pub mod store;
pub mod tree;

pub use source::RecordSource;
pub use store::{Record, Store};

use crate::json::Value;
use crate::render::Span;
use crate::source::jsonrow;

/// A document that is a sequence of records, as the lens machinery needs to see
/// one: how many records are indexed, how to reach record `i`, and whether that
/// record has anything under it to open.
///
/// Three methods, and each is here because something above it cannot be
/// answered without the format: the index is lazy so `known` grows, reading a
/// record is a file read the format owns, and "does this record open" is a
/// count of the rows the format's own tree renderer would produce.
pub trait Records {
    /// Records indexed so far. Grows as the index is extended; never the whole
    /// file unless the file has been read.
    fn known(&self) -> usize;

    /// Run `f` against record `i`'s value, parsing it if the format does not
    /// already have it in hand. `None` is a record that is not valid JSON —
    /// which is not an error here: it keeps its own row and renders as
    /// whatever the format renders a bad record as.
    fn with_value<T>(&self, i: usize, f: impl FnOnce(Option<&Value>) -> T) -> T;

    /// Has record `i` a tree under it? Decides whether a lens row gets a fold
    /// marker. The format answers because the tree is the format's.
    fn foldable(&self, i: usize) -> bool;
}

/// The fold id of record `r`, in the shared vocabulary
/// ([`jsonrow::ALL_OPEN`]): a record document's root is the implicit list of
/// records, so record 4 is `/4`. The twin of [`plan::group_id`]'s `g4`, and
/// they live together so "these two cannot collide" is one fact in one place.
pub(crate) fn fold_id(record: usize) -> String {
    jsonrow::child_id("", record)
}

/// A row that can be opened: the fold marker the painter rewrites to `\u{25b8}`
/// when the record is shut, then the summary.
pub(crate) fn marker(mut rest: Vec<Span>) -> Vec<Span> {
    // Always the *open* glyph: the painter rewrites it to `\u{25b8}` on any row
    // `hidden_at` claims, so emitting the closed one here would double-negate.
    let glyph = crate::theme::MARKER_OPEN;
    let mut spans = vec![Span::new(format!("{glyph} "), crate::theme::json_marker())];
    spans.append(&mut rest);
    spans
}

/// A row with nothing under it: the gutter stays, empty, so the values on it
/// line up with the ones on rows that do open.
pub(crate) fn leaf(mut rest: Vec<Span>) -> Vec<Span> {
    let mut spans = vec![Span::plain("  ")];
    spans.append(&mut rest);
    spans
}
