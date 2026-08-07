//! One row of the document tree, built out of the shared grammar.
//!
//! Every visual decision — the indent, the fold marker, the key spelling, the
//! collapsed summary, the scalar colours — belongs to [`super::super::jsonrow`]
//! and not to this module: the record source renders the same shapes and the
//! two must not drift (see that module's header). What is left here is the part
//! that is genuinely this source's own — *where the facts come from*: a member
//! is a byte range, its count comes from the structural index, and it is parsed
//! here, when it is shown, and never before.
//!
//! One member past [`PARSE_CAP`] says how big it is and what the limit is
//! instead of being loaded, and one that does not parse says why and where:
//! half a document is still worth reading.
#![deny(unsafe_code)]

use crate::json::index::{Member, Shape};
use crate::render::{Line, Span};
use crate::source::jsonrow::{self, Body, Mark};

use super::flat::{Folds, Part, Row};
use super::tree::{Doc, NodeId, MAX_DEPTH, PARSE_CAP};

/// Re-exported so this source's own code and tests have one name for it.
pub use crate::source::jsonrow::size;

/// Bytes one collapsed summary may spend counting, per frame.
const COUNT_BYTES: u64 = 1 << 20;

/// Render one row.
pub fn line(doc: &mut Doc, folds: &Folds, row: Row, at: usize) -> Line {
    let spans = match row.part() {
        Part::Head => head(doc, folds, row.node),
        Part::Tail => tail(doc, row.node),
        Part::Member(i) => member(doc, row.node, i),
    };
    jsonrow::line(spans, at + 1)
}

/// A container's own line: `▾ "users": [`, or the whole summary when it is
/// shut. The root has no key, so it is just the bracket.
fn head(doc: &mut Doc, folds: &Folds, id: NodeId) -> Vec<Span> {
    let open = match folds.uniform() {
        Some(o) => o,
        None => folds.is_open(&doc.fold_id(id)),
    };
    let node = doc.node(id);
    let (depth, shape) = (node.depth as usize, node.shape);
    let (count, done) = (node.count(), node.complete());
    let (start, end) = (node.start, node.end);
    let key = doc.node(id).key.clone();
    let mark = Mark::of(shape, open);
    // A document that is one scalar — `42`, `"hi"` — is a root with no members
    // and no bracket. It is still a row, and it still says what it is.
    if !shape.is_container() {
        let m = Member { key: None, start, end };
        return with_value(doc, depth, mark, key, m, shape);
    }
    let body = match open {
        true => Body::Bracket(shape),
        false => Body::Summary(shape, count, done),
    };
    jsonrow::spans(depth, mark, key.as_deref(), body)
}

/// The closing bracket of an open container.
fn tail(doc: &mut Doc, id: NodeId) -> Vec<Span> {
    let node = doc.node(id);
    jsonrow::spans(node.depth as usize, Mark::Leaf, None, Body::Close(node.shape))
}

/// One member of an open container, shown whole: a scalar, or a container
/// summarised from the index.
fn member(doc: &mut Doc, node: NodeId, i: usize) -> Vec<Span> {
    let Some(m) = doc.node(node).member(i) else {
        return vec![Span::new("\u{2026}", crate::theme::muted())];
    };
    let depth = doc.node(node).depth as usize + 1;
    let shape = doc.shape_of(m);
    let key = doc.key_text(m);
    // Past the depth limit a container cannot be opened, so it must not be
    // painted with a fold marker that would do nothing when pressed.
    if shape.is_container() && doc.too_deep(node) {
        let note = jsonrow::too_deep(MAX_DEPTH as usize);
        return jsonrow::spans(depth, Mark::Leaf, key.as_deref(), Body::Note(note));
    }
    with_value(doc, depth, Mark::of(shape, false), key, m, shape)
}

/// A row whose body is the member's value: summarised if it is a container,
/// parsed if it is small enough, and explained if it is not.
fn with_value(
    doc: &mut Doc,
    depth: usize,
    mark: Mark,
    key: Option<String>,
    m: Member,
    shape: Shape,
) -> Vec<Span> {
    let key = key.as_deref();
    if shape.is_container() {
        let (n, done) = doc.count(m, COUNT_BYTES);
        return jsonrow::spans(depth, mark, key, Body::Summary(shape, n, done));
    }
    if m.len() > PARSE_CAP {
        let note = jsonrow::oversize(m.len(), PARSE_CAP);
        return jsonrow::spans(depth, mark, key, Body::Note(note));
    }
    let (bytes, clipped) = doc.bytes(m.start, m.end);
    if clipped {
        let note = jsonrow::oversize(m.len(), PARSE_CAP);
        return jsonrow::spans(depth, mark, key, Body::Note(note));
    }
    match crate::json::parse(&bytes) {
        Ok(v) => jsonrow::spans(depth, mark, key, Body::Scalar(&v)),
        Err(e) => {
            let note = format!(
                "\u{27e8}not JSON: {} at byte {}\u{27e9}",
                e.reason,
                m.start + e.offset as u64
            );
            jsonrow::spans(depth, mark, key, Body::Note(note))
        }
    }
}
