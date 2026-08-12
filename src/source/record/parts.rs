//! The **open level**: a record read as what it is, one rung above its
//! headline and one below its JSON (SPEC.md §Lenses).
//!
//! ```text
//! ▾ assistant  10:55  Reading the failing test first, since the suite names a
//!                     fixture that no longer exists.
//!                     thinking
//!                       The fixture was renamed two commits ago.
//!                     ▾ bash     cargo test -q parse            → 32 lines
//!                         command   cargo test -q parse
//!                         timeout   120
//!                       output
//!                         test parse::empty_input … ok
//!                         ⋯ +26 lines
//!                     ▸ read     src/parse.rs                   → 40 lines
//! ```
//!
//! One row per call, and **that row opens too**: the arguments it was given,
//! one per line, and the output it returned, clipped exactly as a message is
//! clipped and saying exactly what it left out.
//!
//! # What this module decides, and what it does not
//!
//! It decides rows: the order, the indents, the glyph, the alignment, the
//! colours. It decides **nothing** about what a record contains — that is
//! [`crate::lens::Lens::detail`], a dialect's answer, and a `Part` reaching this
//! file has already been said. That split is the point of the seam: `atif` and
//! `agent` both come through here, and adding a third dialect adds no rows.
//!
//! The fold glyph is painted **in the text**, not in the gutter, and that is
//! deliberate: the gutter marker is rewritten by the painter on any row
//! `Source::hidden_at` claims, and a call row is not a record's fold — it is a
//! row inside one. A glyph of its own cannot be rewritten by the wrong hand.
//!
//! # Height and rows are one walk
//!
//! [`lay`] builds the rows, and the height is `rows.len()`. There is no second
//! function to disagree with it — the same discipline [`super::body`] follows,
//! and for the same reason: a height that is one out moves every row below it.
#![deny(unsafe_code)]

use crate::json::Value;
use crate::lens::{Body, Part};
use crate::render::{str_width, visible, Line, LineKind, Span};
use crate::theme;

use super::body::{self, Shape};

/// Columns an argument's name gets before its value, so a call's arguments read
/// as a column rather than as a paragraph.
const KEY: usize = 10;

/// Columns a name written with [`column`] actually occupies: the field, and the
/// one space it always ends with. The value's wrap is laid out under exactly
/// this, so the name overwrites the pad rather than being pushed in front of it
/// — one column out and every argument's first row sat `KEY + 1` columns right
/// of its own continuation rows.
const KEY_COL: usize = KEY + 1;

/// Columns the tool's own name gets before the argument on a call row.
const TOOL: usize = 9;

/// Columns a call row's `→ what came back` is pushed out to, so the answers of
/// several calls line up under one another. A row wider than this simply
/// carries on: a summary row scrolls sideways, and so does this.
const ANSWER: usize = 56;

/// The rows of a record's parts, and which part each row belongs to.
pub struct Laid {
    pub rows: Vec<Line>,
    /// `owner[i]` is the index into `parts` of the part row `i` came from —
    /// what `Enter` on that row acts on.
    pub owner: Vec<usize>,
    /// Whether part `p` has anything under it to open. A call does; a named
    /// stretch of text is already showing what it has.
    pub opens: Vec<bool>,
}

impl Laid {
    /// Nothing at all — a record with no parts, or one that was not read.
    pub fn empty() -> Laid {
        Laid { rows: Vec::new(), owner: Vec::new(), opens: Vec::new() }
    }

    /// The part row `line` belongs to, when that part is one `Enter` opens.
    pub fn call_at(&self, line: usize) -> Option<usize> {
        let part = self.owner.get(line).copied()?;
        self.opens.get(part).copied().unwrap_or(false).then_some(part)
    }
}

/// Lay out every part of one record.
///
/// `open` says which calls are showing their arguments and output; `value` is
/// the record being painted, which is how an output longer than the head it
/// was summarised from is read back whole (and `None` where the caller has no
/// record in hand, which falls back to the head and still says what it left
/// out). `base` is the column the whole level starts at — [`INDENT`], plus the
/// two columns a member of an open run is inset by, so a step's calls sit under
/// the step's own words rather than two columns to the left of them.
pub fn lay(
    parts: &[Part],
    open: &dyn Fn(usize) -> bool,
    value: Option<&Value>,
    width: usize,
    base: usize,
    source_line: usize,
) -> Laid {
    let mut out = Laid::empty();
    for (i, part) in parts.iter().enumerate() {
        let before = out.rows.len();
        let opens = part.opens();
        match part {
            Part::Text { label, body } => text_part(&mut out.rows, label, body, value, width, base, source_line),
            Part::Call { tool, arg, args, result } => {
                let shown = opens && open(i);
                out.rows.push(call_row(tool, arg, result.as_ref(), opens.then_some(shown), base, source_line));
                if shown {
                    call_detail(&mut out.rows, args, result.as_ref(), value, width, base, source_line);
                }
            }
        }
        out.owner.resize(out.rows.len(), i);
        out.opens.push(opens);
        debug_assert!(out.rows.len() > before, "a part is at least one row");
    }
    out
}

/// A named stretch of text: its name, then the text under it, **whole**.
///
/// Whole rather than clipped because parts exist only at the open level, and
/// the open level is "the whole of that text" (SPEC.md §Lenses). A thought
/// beside a message is a `Part::Text`, and clipping it here left it cut at six
/// rows at every rung with no key that would ever expand it.
fn text_part(
    rows: &mut Vec<Line>,
    label: &str,
    body: &Body,
    value: Option<&Value>,
    width: usize,
    base: usize,
    source_line: usize,
) {
    rows.push(line(vec![pad_span(base), Span::new(visible(label), theme::lens_group())], source_line));
    let shape = Shape::under(width, base + 2);
    rows.extend(body::rows(body, body.text_in(value), shape, true, source_line));
}

/// `▾ bash     cargo test -q parse                → 32 lines`.
///
/// `state` is `None` for a call with nothing under it — no arguments and no
/// result — which then carries no glyph. A fold marker on a row that opens to
/// the same screen is the one thing `opens_further` refuses one rung up, and
/// the same rule holds here.
fn call_row(
    tool: &str,
    arg: &str,
    result: Option<&Body>,
    state: Option<bool>,
    base: usize,
    source_line: usize,
) -> Line {
    let glyph = match state {
        Some(true) => theme::MARKER_OPEN,
        Some(false) => theme::MARKER_CLOSED,
        None => ' ',
    };
    let mut spans = vec![
        pad_span(base),
        Span::new(format!("{glyph} "), theme::json_marker()),
        Span::new(column(tool, TOOL), theme::lens_tool()),
    ];
    let arg = visible(arg);
    let mut used = base + 2 + str_width(&spans[2].text);
    if !arg.is_empty() {
        spans.push(Span::new(arg.clone(), theme::text()));
        used += str_width(&arg);
    }
    if let Some(body) = result {
        let gap = ANSWER.saturating_sub(used).max(1);
        spans.push(Span::plain(" ".repeat(gap)));
        spans.push(Span::new(format!("\u{2192} {}", size_of(body)), theme::lens_more()));
    }
    line(spans, source_line)
}

/// What one call was given and what it gave back.
///
/// The output gets a **named row of its own**, exactly as a [`Part::Text`]
/// does. Without it, output whose lines happen to read `key   value` sat at the
/// argument-name column with nothing between it and the arguments, and a reader
/// could not tell which of the two they were looking at.
fn call_detail(
    rows: &mut Vec<Line>,
    args: &[(String, Body)],
    result: Option<&Body>,
    value: Option<&Value>,
    width: usize,
    base: usize,
    source_line: usize,
) {
    for (key, body) in args {
        arg_rows(rows, key, body, value, width, base, source_line);
    }
    if let Some(body) = result {
        rows.push(line(
            vec![pad_span(base + 2), Span::new("output".to_string(), theme::lens_group())],
            source_line,
        ));
        let shape = Shape::under(width, base + 4);
        rows.extend(body::rows(body, body.text_in(value), shape, false, source_line));
    }
}

/// One argument: its name in a column, its value beside it, and the value's own
/// continuation lines aligned under it.
///
/// The value is clipped like everything else here — an argument may be a
/// sixty-line patch — and the clip says what it left out in the argument's own
/// lines or bytes.
fn arg_rows(
    rows: &mut Vec<Line>,
    key: &str,
    body: &Body,
    value: Option<&Value>,
    width: usize,
    base: usize,
    source_line: usize,
) {
    let shape = Shape::under(width, base + 4 + KEY_COL);
    let mut laid = body::rows(body, body.text_in(value), shape, false, source_line);
    let label = format!("{}{}", " ".repeat(base + 4), column(key, KEY));
    match laid.first_mut() {
        // The value's first row starts in the column the wrap left for the
        // name; every row after it is already aligned under that.
        Some(first) => name_into(first, &label, shape.indent),
        // An argument whose value is empty is still an argument.
        None => rows.push(line(vec![Span::new(label.clone(), theme::lens_group())], source_line)),
    }
    rows.extend(laid);
}

/// Put the argument's name into the space its value's indent left for it,
/// **overwriting** that pad rather than being pushed in front of it.
///
/// Measured in display columns against the indent the value was wrapped to, not
/// in bytes against the pad's own length: a key with a multi-byte character in
/// it is shorter in columns than in bytes, and comparing the two put the value's
/// first row at an indent none of its continuation rows shared.
fn name_into(row: &mut Line, label: &str, cols: usize) {
    let w = str_width(label);
    let text = match w < cols {
        true => format!("{label}{}", " ".repeat(cols - w)),
        false => label.to_string(),
    };
    match row.spans.first_mut() {
        Some(pad) if pad.text.trim().is_empty() && str_width(&pad.text) == cols => {
            *pad = Span::new(text, theme::lens_group());
        }
        _ => row.spans.insert(0, Span::new(text, theme::lens_group())),
    }
}

/// How much a call returned: its own lines, or its size when it is one line.
/// The same vocabulary the row above it uses, so the two cannot disagree.
fn size_of(body: &Body) -> String {
    match (body.bytes, body.lines) {
        // `Body::lines` is `.max(1)`, so emptiness is a byte count and not a
        // line count — reading it off `lines` said `0 bytes` where the summary
        // row one rung up said `empty`.
        (0, _) => "empty".to_string(),
        (bytes, 0 | 1) => crate::source::jsonrow::size(bytes as u64),
        (_, n) => format!("{n} lines"),
    }
}

/// `text`, padded to `cols` display columns and one space, and cut when it is
/// wider — a tool name is not allowed to push the whole column out of line.
fn column(text: &str, cols: usize) -> String {
    let mut s = crate::lens::excerpt(text, cols);
    let w = str_width(&s);
    if w < cols {
        s.push_str(&" ".repeat(cols - w));
    }
    s.push(' ');
    s
}

fn pad_span(cols: usize) -> Span {
    Span::plain(" ".repeat(cols))
}

/// One part row. Not a heading and not a landmark: the summary row above owns
/// the fold that `Tab` and the outline see, and this is a row inside it.
fn line(spans: Vec<Span>, source_line: usize) -> Line {
    Line {
        spans,
        block: 0,
        source_line,
        heading: None,
        scroll: true,
        kind: LineKind::Paragraph,
    }
}

#[cfg(test)]
#[path = "parts_tests.rs"]
mod tests;
