//! The content blocks of one `agent` record, read as [`Part`]s.
//!
//! The half of this dialect that answers [`Lens::detail`](super::Lens::detail):
//! what a record's blocks *are*, once you stop reading them as JSON. Split from
//! `agent.rs` so both stay under the size limit; nothing here carries state, and
//! every path it builds starts at the record it was given.
#![deny(unsafe_code)]

use super::super::{part, Body, Part, Step};
use super::tool_arg;
use crate::json::Value;

/// Which content block goes **under** the record's row as its [`Body`].
///
/// What someone would read first: the first text block that says something,
/// and failing that the first thought that does. A record whose only content is
/// thinking is a step, and its row says `thinking` — reasoning is text, so the
/// text goes under the row rather than being summarised out of existence.
///
/// One definition, used by [`Agent::scan`] to build the body and by
/// [`Lens::detail`] to skip it. Two would paint a paragraph twice.
pub(super) fn body_block(blocks: &[Value]) -> Option<usize> {
    let says = |b: &Value, key: &str| {
        b.get("type").and_then(|t| t.as_str()) == Some(key)
            && b.get(key).and_then(|t| t.as_str()).is_some_and(|t| !t.trim().is_empty())
    };
    if let Some(i) = blocks.iter().position(|b| says(b, "text")) {
        return Some(i);
    }
    blocks.iter().position(|b| says(b, "thinking"))
}

/// Block `i` as a body, whichever of the two text-bearing kinds it is.
pub(super) fn block_body(block: &Value, i: usize) -> Option<Body> {
    let kind = block.get("type").and_then(|t| t.as_str())?;
    let text = block.get(kind)?.as_str()?;
    let at = match kind {
        "text" => text_at(i),
        _ => thinking_at(i),
    };
    Some(Body::new(text, at))
}

/// A text-bearing block as a [`Part`], when it says anything at all.
pub(super) fn text_part(block: &Value, key: &str, label: &'static str, at: Vec<Step>) -> Option<Part> {
    let text = block.get(key)?.as_str()?;
    match text.trim().is_empty() {
        true => None,
        false => Some(Part::Text { label, body: Body::new(text, at) }),
    }
}

/// A `tool_use` block: the call, with every argument it was given.
///
/// `result` is `None` and stays `None` — the answer is a later record, which is
/// this dialect's shape and not something a `Body` can address.
pub(super) fn call_part(block: &Value) -> Part {
    let tool = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
    let input = block.get("input");
    Part::Call {
        tool: tool.to_string(),
        arg: input.and_then(tool_arg).unwrap_or_default(),
        args: input.map(input_args).unwrap_or_default(),
        result: None,
    }
}

/// Every argument of a call — [`part::args_of`], exactly as `atif` reads them,
/// so the two dialects cannot disagree about what an argument list is. An
/// `input` that is not an object keeps what it said under one row rather than
/// being dropped.
pub(super) fn input_args(input: &Value) -> Vec<(String, Body)> {
    part::args_of(input)
}

/// Where block `i`'s text sits inside its record, so the whole of it can be
/// read back when the reader opens the body.
pub(super) fn text_at(i: usize) -> Vec<Step> {
    vec![
        Step::Key("message"),
        Step::Key("content"),
        Step::At(i),
        Step::Key("text"),
    ]
}

/// What a `tool_result` block returned, as a body.
///
/// Two shapes, and only one of them has a path. A `content` that is a string is
/// one node of this record (`message.content[i].content`) and opens whole. The
/// block-array form is several nodes joined with a newline, and no path
/// addresses a join — so that one is a bounded head that states its true size,
/// which is the same promise a clipped message makes.
pub(super) fn result_body(content: Option<&Value>, i: usize) -> Option<Body> {
    let content = content?;
    if let Some(text) = content.as_str() {
        let at = vec![
            Step::Key("message"),
            Step::Key("content"),
            Step::At(i),
            Step::Key("content"),
        ];
        return Some(Body::new(text, at));
    }
    let text = joined_text(content)?;
    Some(Body::new(&text, Vec::new()))
}

/// The block-array form of a result, as one string.
///
/// **Every** block, not only the ones with a `text` key. A result may hold an
/// image beside its text, and keeping the text alone made the join — and so the
/// `bytes` and `lines` the row states — the size of what survived rather than
/// of what came back: the clip then had nothing to report and the image was
/// gone unannounced, which is the one thing a lens may not do (SPEC.md
/// §Lenses). A block that is not text is its own JSON, which is what the
/// record's tree shows.
pub(super) fn joined_text(content: &Value) -> Option<String> {
    let items = content.as_array()?;
    Some(
        items
            .iter()
            .map(|b| match b.get("text").and_then(|t| t.as_str()) {
                Some(text) => text.to_string(),
                None => part::as_text(b),
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}


/// Where block `i`'s thinking sits inside its record.
pub(super) fn thinking_at(i: usize) -> Vec<Step> {
    vec![
        Step::Key("message"),
        Step::Key("content"),
        Step::At(i),
        Step::Key("thinking"),
    ]
}
