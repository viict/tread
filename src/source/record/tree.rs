//! One record laid out as tree rows.
//!
//! # One grammar, shared with the document source
//!
//! Nothing about how a row *looks* is decided here. The indent, the fold
//! marker, the key spelling, the collapsed summary, the scalar colours and the
//! path spelling all come from [`crate::source::jsonrow`], which the JSON
//! document source renders through as well. What this module owns is the walk:
//! where the rows come from when the record is already a [`Value`] rather than
//! a byte range. `tests/json_differential.rs` reads the same content both ways
//! and asserts the rows are identical, which is what keeps the two honest.
//!
//! # One walker, three callers
//!
//! A record's row *count*, its rows and the path of the row under the cursor
//! have to agree exactly: the count decides how many rows the record occupies
//! in the document, so a walker that disagreed with the row builder by one
//! would shift every row after it and put the cursor on the wrong value. There
//! is therefore exactly one traversal — [`walk`] — and the three callers are
//! consumers of the same [`Step`] stream, the same discipline
//! [`crate::csv::parse::Scanner`] applies to row boundaries.
//!
//! # Nothing here recurses
//!
//! [`walk`] carries its own `Vec` of open containers, so a record nested ten
//! thousand deep costs heap and never stack (SPEC.md §JSON: "the renderer, the
//! serialiser and the fold-range computation must not recurse either").
#![deny(unsafe_code)]

use crate::json::value::{Member, Value};
use crate::render::{str_width, Line, Span};
use crate::source::jsonrow::{self, Body, Mark};
use crate::theme;

/// The shared collapsed-container spelling, under the name this module's
/// callers know it by.
pub use crate::source::jsonrow::summary_text as shape_summary;

/// Short scalars shown beside a collapsed record's `{…N keys}`.
const PREVIEW_FIELDS: usize = 3;

/// The widest a previewed scalar may be before it is not "short" any more.
const PREVIEW_WIDTH: usize = 40;

/// What names a node inside its parent: an object key, or nothing at all for an
/// array element (an element has no name in JSON, and the tree does not invent
/// one — its position is in the path the status bar shows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label<'a> {
    Key(&'a str),
    Index(usize),
}

impl Label<'_> {
    /// This step of a path, in the `.users[3].name` spelling.
    pub fn path_step(&self) -> String {
        match self {
            Label::Key(k) => jsonrow::path_step(Some(k), 0),
            Label::Index(i) => jsonrow::path_step(None, *i),
        }
    }

    /// The key a row is labelled with, if any.
    fn key(&self) -> Option<&str> {
        match self {
            Label::Key(k) => Some(k),
            Label::Index(_) => None,
        }
    }
}

/// One row of the tree.
///
/// A container costs two rows — its opening bracket and its closing one — with
/// its members between them, which is exactly what the document source's
/// flatten emits for an open container.
pub enum Step<'a> {
    /// A value on its own row: a scalar, or a container's opening line.
    Node {
        depth: usize,
        label: Label<'a>,
        value: &'a Value,
        /// A container the walk refused to open because it sits past
        /// [`jsonrow::MAX_DEPTH`]. It is one row, has no closing bracket, and
        /// says so — exactly what the document source paints for the same
        /// shape.
        deep: bool,
    },
    /// The closing bracket of the container opened at `depth`.
    Close { depth: usize, value: &'a Value },
}

impl Step<'_> {
    fn depth(&self) -> usize {
        match self {
            Step::Node { depth, .. } => *depth,
            Step::Close { depth, .. } => *depth,
        }
    }
}

/// The record itself and every descendant, in reading order, one call per row.
///
/// The record's own row is emitted first, at depth 0, exactly as the document
/// source emits its root: that is what makes the two row streams comparable.
pub fn walk<'a>(root: &'a Value, mut emit: impl FnMut(Step<'a>)) {
    emit(Step::Node { depth: 0, label: Label::Index(0), value: root, deep: false });
    // (container, next child, depth of that container). Popped and re-pushed so
    // the stack is the open containers and nothing else.
    let mut stack: Vec<(&'a Value, usize, usize)> = Vec::new();
    if root.is_container() {
        stack.push((root, 0, 0));
    }
    while let Some((parent, i, depth)) = stack.pop() {
        let Some((label, child)) = child_at(parent, i) else {
            emit(Step::Close { depth, value: parent });
            continue;
        };
        stack.push((parent, i + 1, depth));
        // The same line the document source draws, in the same place: a
        // container whose parent already sits at the limit is one note row and
        // is not descended into (SPEC.md §JSON, "a refusal or a flat render").
        let deep = child.is_container() && depth >= jsonrow::MAX_DEPTH;
        emit(Step::Node { depth: depth + 1, label, value: child, deep });
        if child.is_container() && !deep {
            stack.push((child, 0, depth + 1));
        }
    }
}

/// The `i`th child of a container, with the label that names it.
fn child_at(parent: &Value, i: usize) -> Option<(Label<'_>, &Value)> {
    match parent {
        Value::Array(items) => items.get(i).map(|v| (Label::Index(i), v)),
        Value::Object(ms) => ms.get(i).map(|m: &Member| (Label::Key(&m.key), &m.value)),
        _ => None,
    }
}

/// How many rows `root` expands into *below* its own summary row. Allocates
/// nothing: the counting consumer of [`walk`], so it cannot drift from [`rows`].
pub fn row_count(root: &Value) -> usize {
    let mut n = 0usize;
    walk(root, |_| n += 1);
    n.saturating_sub(1)
}

/// One expanded record's rows below its summary row, in order.
///
/// `line` is the 1-based file line the record came from, carried on every row
/// so anything reading [`Line::source_line`] sees the record it belongs to.
pub fn rows(root: &Value, line: usize) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    walk(root, |step| out.push(jsonrow::line(step_spans(&step, true), line)));
    out.remove(0);
    out
}

/// The `i`th row below the record's summary row: its path and the value on it.
///
/// The consumer behind the status bar's `.users[3].name` and behind `y`, and
/// the reason [`walk`] carries the depth: truncating the segment stack to the
/// node's depth before pushing its own step turns a pre-order walk into a
/// running path with no second traversal. A closing bracket names the container
/// it closes, which is what the document source's tail row does too.
pub fn node_at(root: &Value, i: usize) -> Option<(String, &Value)> {
    let mut segs: Vec<String> = Vec::new();
    let mut found: Option<(String, &Value)> = None;
    let mut n = 0usize;
    walk(root, |step| {
        match &step {
            Step::Node { depth, label, .. } if *depth > 0 => {
                segs.truncate(depth - 1);
                segs.push(label.path_step());
            }
            _ => segs.truncate(step.depth()),
        }
        if n > 0 && n - 1 == i {
            let value = match step {
                Step::Node { value, .. } => value,
                Step::Close { value, .. } => value,
            };
            found = Some((segs.concat(), value));
        }
        n += 1;
    });
    found
}

/// One tree row's spans. `open` is whether a container on this row is expanded,
/// which for a record's own tree is always true below the summary row.
fn step_spans(step: &Step<'_>, open: bool) -> Vec<Span> {
    match step {
        Step::Node { depth, label, deep: true, .. } => jsonrow::spans(
            *depth,
            Mark::Leaf,
            label.key(),
            Body::Note(jsonrow::too_deep(jsonrow::MAX_DEPTH)),
        ),
        Step::Node { depth, label, value, .. } => {
            let shape = jsonrow::shape_of(value);
            let key = label.key();
            let body = match shape.is_container() && open {
                true => Body::Bracket(shape),
                false => value_body(value, shape),
            };
            jsonrow::spans(*depth, Mark::of(shape, open), key, body)
        }
        Step::Close { depth, value } => jsonrow::spans(
            *depth,
            Mark::Leaf,
            None,
            Body::Close(jsonrow::shape_of(value)),
        ),
    }
}

/// A value as it appears when it is *not* expanded: a scalar in full, a
/// container as the summary that says how much it holds.
fn value_body(v: &Value, shape: crate::json::index::Shape) -> Body<'_> {
    match shape.is_container() {
        true => Body::Summary(shape, v.len(), true),
        false => Body::Scalar(v),
    }
}

/// A value as it appears on its own collapsed row, with no gutter.
pub fn value_spans(v: &Value) -> Vec<Span> {
    jsonrow::body_spans(value_body(v, jsonrow::shape_of(v)))
}

/// `{…5 keys}` / `[…120 items]`, in the shared spelling.
pub fn summary_text(v: &Value) -> String {
    match v.is_container() {
        true => shape_summary(jsonrow::shape_of(v), v.len(), true),
        false => String::new(),
    }
}

/// The spans of a record's own row: the shared collapsed-container row, plus
/// the first few short scalars it holds.
///
/// `{…12 keys}` alone is honest but says nothing about *which* record this is,
/// and a list of a thousand identical-looking rows is not a reader. The scalars
/// are what tell them apart — `type`, `role`, a timestamp — so up to
/// [`PREVIEW_FIELDS`] of them ride along, in document order, only while they
/// are short enough to be a label rather than the content itself.
///
/// This is the one place a record row says more than the document source's
/// equivalent row would, and it is deliberate: a record file is a *list*, and
/// the summary row is what stands for a record in that list. It is a suffix, so
/// the row up to it is byte-identical to the document's collapsed root row.
pub fn record_spans(v: &Value) -> Vec<Span> {
    let mut spans = value_spans(v);
    if !v.is_container() {
        return spans;
    }
    for (label, value) in previews(v) {
        spans.push(Span::new("  \u{b7} ", theme::json_punct()));
        if let Some(k) = label {
            spans.push(Span::new(crate::render::visible(&k), theme::json_key()));
            spans.push(Span::new(": ", theme::json_punct()));
        }
        spans.extend(jsonrow::scalar_spans(&value));
    }
    spans
}

/// Up to [`PREVIEW_FIELDS`] short scalar members, with their keys when the
/// container has any.
fn previews(v: &Value) -> Vec<(Option<String>, Value)> {
    let mut out = Vec::new();
    for i in 0..v.len() {
        if out.len() == PREVIEW_FIELDS {
            break;
        }
        let child = match v.index(i) {
            Some(c) => c,
            None => break,
        };
        if !short_scalar(child) {
            continue;
        }
        let key = v.as_object().and_then(|ms| ms.get(i)).map(|m| m.key.clone());
        out.push((key, child.clone()));
    }
    out
}

/// A scalar small enough, and telling enough, to stand on a summary row.
///
/// `null` is neither: a preview exists to tell one record from its neighbours,
/// and `parentUuid: null` says nothing that the key alone did not. It is still
/// in the count, and still on its own row when the record is opened — this is
/// only about what earns space on the one line that stands for the record.
fn short_scalar(v: &Value) -> bool {
    match v {
        Value::Str(s) => str_width(s) <= PREVIEW_WIDTH && !s.contains('\n'),
        Value::Number(n) => n.text().len() <= PREVIEW_WIDTH,
        Value::Bool(_) => true,
        _ => false,
    }
}

/// Colours for the parts of a row that are not a value: every one of them is
/// [`theme`]'s rather than this module's, because the document source and the
/// record source must not drift into two palettes.
pub mod style {
    use super::*;
    use crate::term::Style;

    pub fn error() -> Style {
        theme::error()
    }
}
