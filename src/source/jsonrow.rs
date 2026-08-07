//! The one tree-row grammar (SPEC.md §JSON, "The tree").
//!
//! # Why this module exists
//!
//! There are two JSON sources — [`super::json`] reads one document by byte
//! range, [`super::jsonl`] reads a record per line — and they arrive at a row
//! from opposite directions: one from a structural index that has never parsed
//! anything, one from a [`Value`] it has. If each spelled a row for itself, the
//! same object would look like two different things depending on which file it
//! came from: a different indent unit, a different fold glyph, `{…0 keys}` in
//! one and `{}` in the other, an index label on array elements in one and not
//! the other. That is not a theoretical drift — it is what the two of them
//! actually did before this module.
//!
//! So every visual decision about a tree row lives here and nowhere else: the
//! indent, the fold marker, how a key is written, how a collapsed container
//! counts itself, how a scalar is coloured and cut, how a path segment is
//! spelled. Both sources build rows only by calling [`spans`], and
//! `tests/json_differential.rs` reads the same content both ways and asserts
//! the rows come out identical.
//!
//! # The grammar
//!
//! ```text
//! ▾ {                          an open container: fold marker, then its bracket
//!     "name": "ada"            a member shown whole
//!   ▸ "runs": [… 120 items]    a folded container, counted from the index
//!   }                          the closing bracket of an open container
//! ```
//!
//! A row is never wrapped: a 40KB string scrolls sideways, as a code block
//! does, because wrapping would break the one-node-one-row correspondence every
//! fold and row calculation on both sides rests on.
#![deny(unsafe_code)]

use crate::json::index::Shape;
use crate::json::{self, Kind, Value};
use crate::render::{str_width, truncate_width, Line, LineKind, Span};
use crate::theme;

/// Display columns one level of nesting indents by.
pub const INDENT: usize = 2;

/// The fold-id vocabulary both JSON sources speak.
///
/// A fold id is a `/`-separated path of *member indices* from the root — `/0/3`
/// is member 3 of member 0 — and the root itself is the empty string.
/// Positional rather than by key, because duplicate keys are kept (SPEC.md
/// §JSON, "Values") and a fold id has to be unique. A record file's root is the
/// implicit list of records, so record 4 is `/4` and there is one scheme rather
/// than two.
///
/// A [`crate::source::FoldState`] is a *default plus exceptions*: the ids whose
/// state is the opposite of the default, with [`ALL_OPEN`] present when the
/// default is open. Never a list of the open nodes — `zR` on a 900MB document
/// would then have to enumerate every container in it.
pub const ALL_OPEN: &str = "*";

/// The fold id of member `i` of the node with id `parent`.
pub fn child_id(parent: &str, i: usize) -> String {
    format!("{parent}/{i}")
}

/// The member index a top-level fold id names, or `None` when it is not one.
/// A record file reads its own ids back with this.
pub fn top_index(id: &str) -> Option<usize> {
    id.strip_prefix('/')?.parse().ok()
}

/// Columns a single scalar may occupy on its row before it is cut.
///
/// The row still scrolls; this is the backstop that keeps one pathological
/// member — the user's own trajectory has a 41KB line — from turning every
/// frame into a megabyte of spans.
pub const MAX_VALUE: usize = 4096;

/// The gutter in front of a row: a fold marker, or the two columns that keep a
/// leaf lined up with its foldable siblings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Open,
    Closed,
    Leaf,
}

impl Mark {
    /// The marker a container of `shape` gets. A scalar is never foldable, so
    /// it is a [`Mark::Leaf`] whatever the fold state says.
    pub fn of(shape: Shape, open: bool) -> Mark {
        match (shape.is_container(), open) {
            (false, _) => Mark::Leaf,
            (true, true) => Mark::Open,
            (true, false) => Mark::Closed,
        }
    }
}

/// What sits to the right of the label on a row.
pub enum Body<'a> {
    /// An open container's own line: `{`.
    Bracket(Shape),
    /// An open container's closing line: `}`.
    Close(Shape),
    /// A collapsed container: `{…5 keys}`. `done` is false while the count is
    /// still being walked, which shows as `≥`.
    Summary(Shape, usize, bool),
    /// A scalar, parsed.
    Scalar(&'a Value),
    /// Something the reader has to be told rather than shown: a member too big
    /// to display, or one that does not parse.
    Note(String),
}

/// One row's spans. The only way either source builds a tree row.
pub fn spans(depth: usize, mark: Mark, key: Option<&str>, body: Body<'_>) -> Vec<Span> {
    let mut out = indent(depth);
    out.push(marker(mark));
    if let Some(k) = key {
        out.push(Span::new(json::write::escape(k), theme::json_key()));
        out.push(Span::new(": ", theme::json_punct()));
    }
    out.extend(body_spans(body));
    out
}

/// A tree row as a [`Line`]. Never wrapped, always scrollable.
pub fn line(spans: Vec<Span>, source_line: usize) -> Line {
    Line {
        spans,
        block: 0,
        source_line,
        heading: None,
        scroll: true,
        kind: LineKind::Table,
    }
}

/// Just the body of a row, with no indent and no gutter — for a caller that
/// supplies its own gutter (the record source's summary row, which is a row in
/// a *list* of records as well as a tree row).
pub fn body_spans(body: Body<'_>) -> Vec<Span> {
    match body {
        Body::Bracket(s) => vec![Span::new(s.brackets().0, theme::json_punct())],
        Body::Close(s) => vec![Span::new(s.brackets().1, theme::json_punct())],
        Body::Summary(s, n, done) => {
            vec![Span::new(summary_text(s, n, done), theme::muted())]
        }
        Body::Scalar(v) => scalar_spans(v),
        Body::Note(t) => vec![Span::new(t, theme::error())],
    }
}

/// `{…5 keys}` / `[…120 items]`, and `{}` / `[]` for a container that is
/// genuinely empty — "0 keys" is a number where a shape would do.
///
/// `≥` while the container is still being walked: a count that is not final
/// says so, exactly as a CSV's row total does, rather than showing a number
/// that will change under the reader.
pub fn summary_text(shape: Shape, n: usize, done: bool) -> String {
    let (l, r) = shape.brackets();
    if done && n == 0 {
        return format!("{l}{r}");
    }
    let ge = match done {
        true => "",
        false => "\u{2265}",
    };
    format!("{l}\u{2026}{ge}{n} {}{r}", shape.unit(n))
}

/// A scalar value, coloured by what it is.
///
/// Strings are shown as the JSON *literal* — quoted and re-escaped — not as the
/// decoded text in quotes. Decoded, `"has \"quotes\""` prints as
/// `"has "quotes""`, which is not what the file says and is not even valid
/// JSON; `\\` prints as `\`, which in JSON means something else entirely. The
/// escaper is the parser's own, so what is displayed re-parses to the value
/// being displayed. Numbers keep their source text for the same reason: a
/// round-trip through `f64` would show something the document does not say.
pub fn scalar_spans(v: &Value) -> Vec<Span> {
    let (text, style) = match v.kind() {
        Kind::String => (
            cut_literal(&json::write::escape(v.as_str().unwrap_or(""))),
            theme::json_string(),
        ),
        Kind::Number => (
            cut(v.as_number().map(|n| n.text()).unwrap_or_default()),
            theme::json_number(),
        ),
        Kind::Bool => (v.as_bool().unwrap_or(false).to_string(), theme::json_bool()),
        Kind::Null => ("null".to_string(), theme::json_null()),
        // A container reaching here has already been parsed, so showing it
        // compact is honest and cheap; the tree shows it properly one row down.
        _ => (cut(&v.to_json()), theme::text()),
    };
    vec![Span::new(text, style)]
}

/// What a parsed value is, in the vocabulary the structural index uses. The
/// bridge that lets a `Value`-backed row and a byte-range-backed row ask the
/// same questions.
pub fn shape_of(v: &Value) -> Shape {
    match v {
        Value::Object(_) => Shape::Object,
        Value::Array(_) => Shape::Array,
        Value::Str(_) => Shape::Str,
        Value::Number(_) => Shape::Number,
        Value::Bool(_) => Shape::Bool,
        Value::Null => Shape::Null,
    }
}

/// The step a member adds to the path the status bar shows: `.name`,
/// `["odd key"]` for a key that cannot be written with a dot, or `[3]` for an
/// array element (SPEC.md §JSON: "The status bar names the path of the row
/// under the cursor: `.users[3].name`").
pub fn path_step(key: Option<&str>, index: usize) -> String {
    match key {
        None => format!("[{index}]"),
        Some(k) if plain_key(k) => format!(".{k}"),
        Some(k) => format!("[{}]", Value::string(k.to_string()).to_json()),
    }
}

/// A key that can be written `.like_this` rather than `["like this"]`.
fn plain_key(k: &str) -> bool {
    !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !k.starts_with(|c: char| c.is_ascii_digit())
}

/// A byte count a person can read.
pub fn size(n: u64) -> String {
    const UNITS: [(u64, &str); 3] = [(1 << 30, "GB"), (1 << 20, "MB"), (1 << 10, "KB")];
    for (scale, name) in UNITS {
        if n >= scale {
            return format!("{:.1} {name}", n as f64 / scale as f64);
        }
    }
    format!("{n} bytes")
}

/// The deepest container either source will *open*.
///
/// A presentation limit, not a parse limit: the value parser refuses at
/// [`crate::json::parse::DEFAULT_MAX_DEPTH`] (ten thousand) and keeps doing so.
/// This is the shallower line the *tree* draws, and it belongs here because both
/// sources have to draw it in the same place — a shape must not be readable as a
/// line of a log and unreadable as a file, or the other way round.
///
/// Why it is not ten thousand. The document source indexes a container by
/// walking its bytes, so a chain of nested containers re-walks the bytes it
/// spans once per level: a file of `N` nested brackets costs
/// `min(N, MAX_DEPTH) × filesize` byte steps to open all the way down. At ten
/// thousand that made a 20KB file of `[[[[…` take seconds and a 100KB one take
/// half a minute — a hang, where SPEC.md §JSON asks for "a refusal or a flat
/// render". At 256 the same files render in milliseconds, and everything past
/// the limit says [`too_deep`] instead: the flat render, arrived at promptly.
///
/// 256 is far past any real document — a large agent trajectory nests about
/// nine deep — and far short of anything that costs.
pub const MAX_DEPTH: usize = 256;

/// The note a container nested past the reader's depth limit shows instead of a
/// fold marker that would not open. The same sentence on both sources, so a
/// `.json` document and a `.jsonl` line say the same thing.
pub fn too_deep(limit: usize) -> String {
    format!("\u{27e8}nested deeper than {limit} levels \u{2014} not opened\u{27e9}")
}

/// The note a member too large to display shows instead of its value.
pub fn oversize(len: u64, cap: u64) -> String {
    format!(
        "\u{27e8}{} \u{2014} over the {} display limit\u{27e9}",
        size(len),
        size(cap)
    )
}

/// Cut a string literal, keeping it a literal.
///
/// The quotes are part of the escaped text now, so plain [`cut`] would drop the
/// closing one and leave `"xxx…` — an unbalanced quote reads as a rendering
/// bug. Cut inside the quotes instead, and never end on a half-written escape:
/// a trailing lone `\` would claim the next character is escaped when it is the
/// ellipsis.
fn cut_literal(lit: &str) -> String {
    if str_width(lit) <= MAX_VALUE {
        return lit.to_string();
    }
    let kept = truncate_width(lit, MAX_VALUE.saturating_sub(2));
    let kept = kept.trim_end_matches('\\');
    format!("{kept}\u{2026}\"")
}

/// Cut a scalar that is too wide even for a scrollable row.
fn cut(text: &str) -> String {
    if str_width(text) <= MAX_VALUE {
        return text.to_string();
    }
    let mut s = truncate_width(text, MAX_VALUE.saturating_sub(1)).to_string();
    s.push('\u{2026}');
    s
}

/// Levels of nesting the indent keeps growing for.
///
/// Past this a row is indented off the right of any terminal, so further
/// indenting says nothing a reader can see — and on a document nested ten
/// thousand deep it would make the *rows* quadratic in the depth, which is a
/// way to run out of memory rendering a file that opened instantly. This is the
/// "flat render" SPEC.md §JSON allows for hostile nesting; the path in the
/// status bar still says exactly how deep the cursor is.
pub const MAX_INDENT: usize = 64;

fn indent(depth: usize) -> Vec<Span> {
    match depth.min(MAX_INDENT) {
        0 => Vec::new(),
        d => vec![Span::plain(" ".repeat(d * INDENT))],
    }
}

fn marker(mark: Mark) -> Span {
    let glyph = match mark {
        Mark::Open => theme::MARKER_OPEN,
        Mark::Closed => theme::MARKER_CLOSED,
        Mark::Leaf => return Span::plain("  "),
    };
    Span::new(format!("{glyph} "), theme::json_marker())
}

#[cfg(test)]
#[path = "jsonrow_tests.rs"]
mod tests;
