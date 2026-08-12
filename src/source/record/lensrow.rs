//! The lens over a record document: the rows it produces, and the arithmetic
//! that places them.
//!
//! One per record the lens read, one per run it folded.
//!
//! Everything visual about a lens is here, so a second dialect adds a module
//! under `src/lens/` and changes nothing about how a row looks. The grammar:
//!
//! ```text
//! ▾ user       21:29  I want a reader for the terminal that understands
//!                     markdown.
//!   ▸ ⟨6 steps · 4 tool calls⟩              21:31
//!     assistant  21:31  Bash(cargo test)
//! ```
//!
//! The message starts in the `what` column and continues under it; a step —
//! `Bash(cargo test)` — has no message and is its own one line.
//!
//! Three columns — who, when, what — because that is the order a reader
//! scanning a log asks in. The clock is `HH:MM` of the timestamp the file
//! records (UTC in every agent log seen so far): the date is the same for
//! nearly every row of a session, and the seconds are noise.
//!
//! A *summary* row is never wrapped, the same rule the tree rows follow: it
//! scrolls sideways, so the headline of a record is always one row. For a
//! record that said something that headline **is the message's first line**
//! ([`headline`]), and the rest of what it said goes under it, wrapped, as the
//! body rows [`super::body`] lays out — one wrap, split between the row and the
//! rows below it, so nothing is painted twice. That is why an item's own rows
//! depend on the width and why a resize re-measures every one of them
//! ([`remeasure`]).
//!
//! # Free functions, not methods
//!
//! Nothing here is a method on a source, because the plan, the row map and the
//! records are three separate borrows: reading a record is `&self` on the
//! format, and classifying writes to the other two. Taking them as arguments is
//! what lets one loop hold all three, and it is why [`Records`] is a trait the
//! format implements rather than a struct that owns the lens state.
#![deny(unsafe_code)]

use super::plan::{Plan, Spot};
use super::rowmap::RowMap;
use super::{body, leaf, marker, Records};
use crate::lens::{Class, Summary};
use crate::render::{str_width, visible, Line, LineKind, Span};
use crate::theme;

/// Columns the speaker's name gets before the clock.
const ACTOR: usize = 10;

/// Columns the clock gets. `HH:MM`.
const TIME: usize = 5;

/// Classify records up to and including `upto`, parsing each one once.
///
/// The prefix the lens has read decides the grouping, so this runs *ahead*
/// of the painted window: rows above the viewport never move because their
/// records were classified before they were painted.
pub(crate) fn classify_to<R: Records>(src: &R, plan: &mut Plan, map: &mut RowMap, upto: usize) {
    let target = upto.saturating_add(1).min(src.known());
    while plan.classified() < target {
        let record = plan.classified();
        src.with_value(record, |v| plan.classify(record, v, map));
        // The item the record landed in — a new one, or the run it extended.
        measure(src, plan, plan.items().len().saturating_sub(1));
    }
    plan.sync();
}

/// How tall item `i`'s message is at the plan's width.
///
/// The record is read only for a body that is **open and longer than
/// [`crate::lens::BODY_KEEP`]**, which is the one case the head cannot answer.
/// Every other item — a step, a group, a clipped body, a short message — is
/// measured from the summary alone, so a resize costs no parse.
pub(crate) fn measure<R: Records>(src: &R, plan: &mut Plan, item: usize) {
    let width = plan.width();
    let rows = match plan.body_at(item) {
        None => 0,
        Some((body, full)) if full && !body.whole() => {
            let record = plan.item(item).map(|it| it.first).unwrap_or(0);
            src.with_value(record, |v| body::height(body, body.text_in(v), width, full))
        }
        Some((body, full)) => body::height(body, body.text_in(None), width, full),
    };
    plan.set_body(item, rows);
}

/// Re-lay every body, which is what a width change means here. Classification
/// is not repeated — it runs once per record, in file order — only the wrap.
pub(crate) fn remeasure<R: Records>(src: &R, plan: &mut Plan) {
    for i in 0..plan.items().len() {
        measure(src, plan, i);
    }
    plan.sync();
}

/// The rows of the message under `record`, at `width`.
pub(crate) fn body_rows<R: Records>(src: &R, plan: Option<&Plan>, record: usize, width: usize) -> Vec<Line> {
    let Some(plan) = plan else {
        return Vec::new();
    };
    let Some(item) = plan.item_of_record(record) else {
        return Vec::new();
    };
    let Some((body, full)) = plan.body_at(item) else {
        return Vec::new();
    };
    let line = record + 1;
    match full && !body.whole() {
        true => src.with_value(record, |v| body::rows(body, body.text_in(v), width, full, line)),
        false => body::rows(body, body.text_in(None), width, full, line),
    }
}

/// The whole message under `record`, as text — what `y` copies off a body row.
pub(crate) fn body_text<R: Records>(src: &R, plan: Option<&Plan>, record: usize) -> Option<String> {
    let plan = plan?;
    let (body, _) = plan.body_at(plan.item_of_record(record)?)?;
    match body.whole() {
        true => Some(body.head.clone()),
        false => Some(src.with_value(record, |v| body.text_in(v).to_string())),
    }
}

/// Where a screen row falls. The one translation between rows and records,
/// with or without a lens.
pub(crate) fn spot(plan: Option<&Plan>, map: &RowMap, known: usize, row: usize) -> Spot {
    match plan {
        Some(p) => p.at(row, known, map),
        None => {
            let (record, sub) = map.at(row);
            Spot::Record { record, sub }
        }
    }
}

/// The row a record's summary sits on — the group's row when a lens has
/// folded it away.
pub(crate) fn row_of_record(plan: Option<&Plan>, map: &RowMap, record: usize) -> usize {
    match plan {
        Some(p) => p.row_of_record(record, map),
        None => map.row_of(record),
    }
}

/// The record a row belongs to. A group's row stands for its first record,
/// which is what keeps `record N/M`, search and yanking agreeing with what the
/// cursor is on.
pub(crate) fn record_at(plan: Option<&Plan>, map: &RowMap, known: usize, row: usize) -> usize {
    match spot(plan, map, known, row) {
        Spot::Record { record, .. } => record,
        // A body row belongs to the record that said it, so `record N/M`,
        // search and `Y` all still name what the cursor is reading.
        Spot::Body { record, .. } => record,
        Spot::Group { item } => item_first(plan, item),
    }
}

/// The first record of a plan item.
pub(crate) fn item_first(plan: Option<&Plan>, item: usize) -> usize {
    plan.and_then(|p| p.item(item)).map(|it| it.first).unwrap_or(0)
}

/// Is item `i` an open group? What the outline reports as unfolded.
pub(crate) fn item_open(plan: Option<&Plan>, item: usize) -> bool {
    plan.and_then(|p| p.item(item)).map(|it| it.open).unwrap_or(false)
}

/// Is this record a member of a group that is currently open? Its row is then
/// indented under the group's, so a run reads as one thing.
pub(crate) fn in_open_group(plan: Option<&Plan>, record: usize) -> bool {
    let Some(plan) = plan else {
        return false;
    };
    plan.item_of_record(record)
        .and_then(|i| plan.item(i))
        .map(|it| it.is_group() && it.open)
        .unwrap_or(false)
}

/// Is `record` currently a row of its own?
pub(crate) fn record_visible(plan: Option<&Plan>, record: usize) -> bool {
    match plan {
        None => true,
        Some(p) => match p.item_of_record(record) {
            None => true,
            Some(i) => p.item(i).map(|it| !it.is_group() || it.open).unwrap_or(true),
        },
    }
}

/// A record's row as the lens reads it, or `None` when the lens did not
/// recognise the record — which is not a failure: the caller then renders
/// the generic tree row, and nothing is lost (SPEC.md §Lenses).
pub(crate) fn lens_row<R: Records>(
    src: &R,
    plan: Option<&Plan>,
    record: usize,
    inset: bool,
) -> Option<Line> {
    let plan = plan?;
    let sum = plan.summary(record)?;
    let what = headline(src, plan, record, sum);
    let spans = lens_spans(sum, &what, src.foldable(record), inset);
    Some(summary_line(spans, record + 1))
}

/// What the `what` column paints.
///
/// For a record that said something it is the message's **first wrapped line**
/// at the layout width, and [`body::rows`] paints the rest — so the opening
/// words are on the screen once rather than twice. For a step, which has no
/// message under it, it is the summary's own one-line description, unchanged.
///
/// The width is the plan's, not the caller's: it is the width every body was
/// measured at ([`measure`]), and a headline wrapped to a different one would
/// be a first line the rows below it do not continue.
///
/// The **text** is chosen the same way, and for the same reason: whatever
/// [`body_rows`] wraps, this wraps, because the two are one wrap split in two.
/// Wrapping the head here while the rows wrapped the whole record put the
/// characters between the two first rows on no row at all, on any terminal
/// wide enough for a row to hold more than [`crate::lens::BODY_KEEP`] bytes.
fn headline<R: Records>(src: &R, plan: &Plan, record: usize, sum: &Summary) -> String {
    let Some(b) = sum.body.as_ref() else {
        return visible(&sum.what);
    };
    let width = plan.width();
    let full = plan
        .item_of_record(record)
        .and_then(|i| plan.body_at(i))
        .map(|(_, full)| full)
        .unwrap_or(false);
    match full && !b.whole() {
        true => src.with_value(record, |v| body::first_line(b, b.text_in(v), width)),
        false => body::first_line(b, b.text_in(None), width),
    }
    .unwrap_or_default()
}

/// One `--toc` line for a record the lens read: the same three columns the
/// painted row has, tab-separated for a shell rather than padded for a screen.
///
/// Through the sanitiser, exactly as the painted row is: a record may hold any
/// byte, and `--toc` writes straight to a terminal (SPEC.md §JSON, the shared
/// sanitiser). `None` is a record the lens did not read — the caller then
/// prints the generic one, and nothing is hidden.
pub(crate) fn toc_line(plan: Option<&Plan>, record: usize) -> Option<String> {
    let sum = plan?.summary(record)?;
    let time = sum.time.clone().unwrap_or_default();
    let actor = visible(&sum.actor);
    let what = visible(&sum.what);
    Some(format!("{}\t{actor}\t{time}\t{what}", record + 1))
}

/// A folded run of mechanics: `⟨6 steps · 4 tool calls⟩`, with the clock of
/// the first step so the run still sits on the timeline.
///
/// It always carries a fold marker, because a group only exists when there
/// is something inside it to open.
pub(crate) fn group_row(plan: Option<&Plan>, item: usize) -> Line {
    let (first, count, calls) = group_counts(plan, item);
    let time = plan
        .and_then(|p| p.summary(first))
        .and_then(|s| s.time.clone())
        .unwrap_or_default();
    let rest = vec![
        Span::new(pad("", ACTOR), theme::text()),
        Span::new(pad(&time, TIME), theme::lens_time()),
        Span::new("  ", theme::text()),
        Span::new(group_text(count, calls), theme::lens_group()),
    ];
    summary_line(marker(rest), first + 1)
}

/// `(first record, records, tool calls)` of a group.
fn group_counts(plan: Option<&Plan>, item: usize) -> (usize, usize, usize) {
    let Some(plan) = plan else {
        return (0, 0, 0);
    };
    let Some(it) = plan.item(item) else {
        return (0, 0, 0);
    };
    let calls = (it.first..it.first + it.count)
        .filter_map(|r| plan.summary(r))
        .map(|s| s.calls)
        .sum();
    (it.first, it.count, calls)
}

/// One summary row: never wrapped, and a landmark for `Tab`.
fn summary_line(spans: Vec<Span>, source_line: usize) -> Line {
    Line {
        spans,
        block: 0,
        source_line,
        heading: None,
        scroll: true,
        kind: LineKind::Heading,
    }
}

/// `who · when · what`, with the gutter a foldable row needs. `what` is
/// [`headline`]'s, already through the sanitiser.
fn lens_spans(sum: &Summary, what: &str, foldable: bool, inset: bool) -> Vec<Span> {
    let rest = vec![
        Span::new(pad(&sum.actor, ACTOR), sum.who.style()),
        Span::new(pad(sum.time.as_deref().unwrap_or(""), TIME), theme::lens_time()),
        Span::new("  ", theme::text()),
        Span::new(what.to_string(), body_style(sum.class)),
    ];
    let spans = match foldable {
        true => marker(rest),
        false => leaf(rest),
    };
    inset_after_gutter(spans, inset)
}

/// Indent a row that belongs to an open run — *after* its fold marker, never
/// before it.
///
/// The gutter has to stay the first span on the row: the painter rewrites the
/// open marker to the closed one there when a row is folded shut, and a
/// leading indent span would leave every member of an open run showing the
/// wrong glyph.
pub(crate) fn inset_after_gutter(mut spans: Vec<Span>, inset: bool) -> Vec<Span> {
    if inset && !spans.is_empty() {
        spans.insert(1, Span::plain("  "));
    }
    spans
}

/// Speech is the document; mechanics recede.
fn body_style(class: Class) -> crate::term::Style {
    match class {
        Class::Message => theme::text(),
        Class::Step => theme::muted(),
    }
}

/// `⟨6 steps · 4 tool calls⟩`, and the singulars, and no call clause when the
/// run made no calls (a run of thoughts and bookkeeping).
fn group_text(steps: usize, calls: usize) -> String {
    let steps = match steps {
        1 => "1 step".to_string(),
        n => format!("{n} steps"),
    };
    let calls = match calls {
        0 => String::new(),
        1 => " \u{b7} 1 tool call".to_string(),
        n => format!(" \u{b7} {n} tool calls"),
    };
    format!("\u{27e8}{steps}{calls}\u{27e9}")
}

/// Pad to `cols` display columns — display columns, not bytes, because an
/// actor may be `↳ assistant`.
fn pad(text: &str, cols: usize) -> String {
    let mut s = visible(text);
    let w = str_width(&s);
    if w < cols {
        s.push_str(&" ".repeat(cols - w));
    }
    s.push(' ');
    s
}
