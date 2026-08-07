//! The lens behind the record source: the state, and the rows it produces.
//!
//! One per record the lens read, one per run it folded.
//!
//! Everything visual about a lens is here, so a second dialect adds a module
//! under `src/lens/` and changes nothing about how a row looks. The grammar:
//!
//! ```text
//! ▾ user       21:29  I want a reader for the terminal…
//!   ▸ ⟨6 steps · 4 tool calls⟩              21:31
//!     assistant  21:31  Bash(cargo test)
//! ```
//!
//! Three columns — who, when, what — because that is the order a reader
//! scanning a log asks in. The clock is `HH:MM` of the timestamp the file
//! records (UTC in every agent log seen so far): the date is the same for
//! nearly every row of a session, and the seconds are noise.
//!
//! A row is never wrapped, the same rule the tree rows follow: it scrolls
//! sideways, so one record is always one row and the fold arithmetic holds.
#![deny(unsafe_code)]

use super::*;
use crate::lens::{Class, Summary};
use crate::render::{str_width, visible};
use crate::theme;

/// Columns the speaker's name gets before the clock.
const ACTOR: usize = 10;

/// Columns the clock gets. `HH:MM`.
const TIME: usize = 5;

impl JsonlSource {
    // -- the lens ---------------------------------------------------------------

    /// Read this file through `lens` (SPEC.md §Lenses).
    ///
    /// A transform over the records, not a different source: the file, the
    /// index, the parser and every row below a summary are unchanged. What the
    /// lens decides is what each record's *one* row says and which runs of
    /// records share one.
    pub fn set_lens(&mut self, lens: Box<dyn Lens>) {
        self.plan = Some(Plan::new(lens));
    }

    /// The lens in force, for the status bar.
    pub fn lens_name(&self) -> Option<&'static str> {
        self.plan.as_ref().map(|p| p.lens_name())
    }

    /// Classify records up to and including `upto`, parsing each one once.
    ///
    /// The prefix the lens has read decides the grouping, so this runs *ahead*
    /// of the painted window: rows above the viewport never move because their
    /// records were classified before they were painted.
    pub(super) fn classify_to(&mut self, upto: usize) {
        if self.plan.is_none() {
            return;
        }
        let target = upto.saturating_add(1).min(self.known());
        // Both are moved out for the loop: reading a record borrows `self`
        // immutably through the `RefCell`s, and the classification writes to
        // these two. Put back before returning, always.
        let mut plan = match self.plan.take() {
            Some(p) => p,
            None => return,
        };
        let mut map = std::mem::take(&mut self.map);
        while plan.classified() < target {
            let record = plan.classified();
            self.with_record(record, |rec| plan.classify(record, rec.value(), &mut map));
        }
        plan.sync();
        self.plan = Some(plan);
        self.map = map;
    }

    /// Where a screen row falls. The one translation between rows and records,
    /// with or without a lens.
    pub(crate) fn spot(&self, row: usize) -> Spot {
        match &self.plan {
            Some(p) => p.at(row, self.known(), &self.map),
            None => {
                let (record, sub) = self.map.at(row);
                Spot::Record { record, sub }
            }
        }
    }

    /// The row a record's summary sits on — the group's row when a lens has
    /// folded it away.
    pub(crate) fn row_of_record(&self, record: usize) -> usize {
        match &self.plan {
            Some(p) => p.row_of_record(record, &self.map),
            None => self.map.row_of(record),
        }
    }

    /// Is `record` currently a row of its own?
    pub(super) fn record_visible(&self, record: usize) -> bool {
        match &self.plan {
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
    pub(super) fn lens_row(&self, record: usize, inset: bool) -> Option<Line> {
        let sum = self.plan.as_ref()?.summary(record)?;
        let foldable = self.tree_len(record) > 0;
        let spans = lens_spans(sum, foldable, inset);
        Some(summary_line(spans, record + 1))
    }

    /// A folded run of mechanics: `⟨6 steps · 4 tool calls⟩`, with the clock of
    /// the first step so the run still sits on the timeline.
    ///
    /// It always carries a fold marker, because a group only exists when there
    /// is something inside it to open.
    pub(super) fn group_row(&self, item: usize) -> Line {
        let (first, count, calls) = self.group_counts(item);
        let time = self
            .plan
            .as_ref()
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
    fn group_counts(&self, item: usize) -> (usize, usize, usize) {
        let Some(plan) = self.plan.as_ref() else {
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

/// `who · when · what`, with the gutter a foldable row needs.
fn lens_spans(sum: &Summary, foldable: bool, inset: bool) -> Vec<Span> {
    let rest = vec![
        Span::new(pad(&sum.actor, ACTOR), sum.who.style()),
        Span::new(pad(sum.time.as_deref().unwrap_or(""), TIME), theme::lens_time()),
        Span::new("  ", theme::text()),
        Span::new(visible(&sum.what), body_style(sum.class)),
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
pub(super) fn inset_after_gutter(mut spans: Vec<Span>, inset: bool) -> Vec<Span> {
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

#[cfg(test)]
#[path = "lensrow_tests.rs"]
mod tests;
