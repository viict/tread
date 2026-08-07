//! The [`Source`] implementation itself: what the pager sees.
//!
//! Split from `mod.rs`, which holds the state, so both stay under the size
//! limit. The order of the sections is the trait's own
//! (`src/source/mod.rs`), and every method a JSON document has no honest answer
//! for says so rather than inventing one.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;
use crate::source::{Detail, End, FoldState, Hit, Mark, MatchSpan, Source};

impl Source for JsonSource {
    // -- layout ---------------------------------------------------------------

    /// Nothing is laid out to a width: a tree row is never wrapped, it scrolls.
    /// The first call is where the first screen's worth of rows is found.
    fn set_width(&mut self, cols: usize) {
        self.view = cols.max(1);
        if self.flat.len() == 0 && !self.flat.done() {
            self.grow(FIRST_SCREEN, FRAME_BYTES);
        }
    }

    fn len(&self) -> usize {
        self.flat.len()
    }

    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        // Find rows past the window so the viewport can move again next frame.
        self.grow(rows.end.saturating_add(LOOKAHEAD), FRAME_BYTES);
        let end = rows.end.min(self.len());
        let start = rows.start.min(end);
        self.build_entries(start..end);
        (start..end).filter_map(|r| self.row_line(r)).collect()
    }

    // -- viewport affordances ---------------------------------------------------

    /// A tree is not prose: a wide terminal should show a wide tree.
    fn full_width(&self) -> bool {
        true
    }

    /// `.users[3].name`, plus how far down the document the cursor is
    /// (SPEC.md §JSON: "The status bar names the path of the row under the
    /// cursor"). The total is `\u{2265}N` until the walk has reached the end,
    /// because until then there is no honest total.
    fn position_text(&self, row: usize) -> Option<String> {
        let known = self.len();
        let total = match self.flat.done() {
            true => format!("{known}"),
            false => format!(
                "\u{2265}{known} (indexing {}%)",
                self.doc.borrow().progress()
            ),
        };
        Some(format!(
            "row {}/{total}  \u{b7}  {}",
            row.saturating_add(1),
            self.path_of(row)
        ))
    }

    /// `G`. Until the walk has found the last row there is no honest answer, so
    /// this reports progress and lets the pager drive it there a slice at a
    /// time (SPEC.md §CSV, and the same rule here).
    fn end(&self) -> End {
        match self.flat.done() {
            true => End::At(self.len().saturating_sub(1)),
            false => End::Scanning(self.doc.borrow().progress()),
        }
    }

    fn extend(&mut self) -> bool {
        let want = self.flat.len().saturating_add(IDLE_ROWS);
        self.grow(want, IDLE_BYTES);
        let counting = self.doc.borrow_mut().extend_counts(IDLE_BYTES / 8);
        !self.flat.done() || counting
    }

    // -- positions --------------------------------------------------------------

    fn anchor(&self, row: usize) -> Option<Anchor> {
        (row < self.len()).then_some(Anchor(row))
    }

    fn row_of(&self, anchor: Anchor) -> Option<usize> {
        (anchor.0 < self.len()).then_some(anchor.0)
    }

    fn reveal(&mut self, anchor: Anchor) -> Option<usize> {
        let n = self.len();
        (n > 0).then(|| anchor.0.min(n - 1))
    }

    fn mark(&self, row: usize) -> Option<Mark> {
        (row < self.len()).then_some(Mark(row))
    }

    fn locate(&self, mark: Mark) -> Option<usize> {
        let n = self.len();
        (n > 0).then(|| mark.0.min(n - 1))
    }

    // -- structure -----------------------------------------------------------------

    fn outline(&self) -> &[Entry] {
        &self.entries
    }

    /// The section a row belongs to, in the window's outline. A row outside the
    /// last painted window clamps to its nearest end rather than answering
    /// `None`, which would read as "nothing to fold here" on a document that is
    /// nothing but foldable things.
    fn section_at(&self, row: usize) -> Option<usize> {
        if self.entries.is_empty() {
            return None;
        }
        if row < self.win.start {
            return Some(0);
        }
        match self.row_entry.get(row - self.win.start) {
            Some(&e) => Some(e),
            None => Some(self.entries.len() - 1),
        }
    }

    fn set_fold(&mut self, entry: usize, closed: bool) -> bool {
        let Some(e) = self.entries.get(entry) else {
            return false;
        };
        let id = e.id.clone();
        if !self.folds.set(&id, !closed) {
            return false;
        }
        self.refold();
        true
    }

    /// `zM` / `zR`. One boolean each: expanding everything must not mean
    /// enumerating everything (see [`flat::Folds`]).
    fn fold_all(&mut self, closed: bool) {
        self.folds.all(!closed);
        self.refold();
    }

    fn folds(&self) -> FoldState {
        self.folds.state()
    }

    fn set_folds(&mut self, folds: FoldState) {
        self.folds.restore(folds);
        self.refold();
    }

    /// Nothing: a collapsed row already says what it hides — `{…5 keys}` —
    /// and the painter's `(N lines)` on top of that would say it twice.
    fn hidden_at(&self, _row: usize) -> Option<usize> {
        None
    }

    /// `Tab` / `S-Tab`: the next open container's own line. Cheap by
    /// construction — an opening row is a row of a node, so finding one costs
    /// no read at all.
    fn next_landmark(&self, row: usize, forward: bool) -> Option<usize> {
        let n = self.len();
        let is_head = |r: usize| matches!(self.flat.get(r).map(Row::part), Some(Part::Head));
        match forward {
            true => (row + 1..n).find(|r| is_head(*r)),
            false => (0..row).rev().find(|r| is_head(*r)),
        }
    }

    /// A JSON document has no anchor links, so there is no id to jump to.
    fn goto_id(&mut self, _id: &str) -> Option<usize> {
        None
    }

    fn links(&self) -> &[LinkSite] {
        &self.none_links
    }

    // -- search --------------------------------------------------------------

    fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.sensitive = crate::source::search::case_sensitive(query);
        self.needle = match self.sensitive {
            true => query.to_string(),
            false => query.to_lowercase(),
        };
        self.found = None;
    }

    /// 1 while the last sweep has a hit, 0 otherwise: a lazily walked document
    /// cannot be counted without reading all of it, and claiming a number would
    /// be a lie.
    fn match_count(&self) -> usize {
        usize::from(self.found.is_some())
    }

    fn current_match(&self) -> Option<usize> {
        self.found.map(|_| 0)
    }

    fn preview_match(&mut self, origin: Anchor, dir: Dir) -> Option<Hit> {
        let found = self.sweep(origin.0, dir, true)?;
        self.found = Some(found.0);
        Some(Hit { anchor: Anchor(found.0), wrapped: found.1 })
    }

    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit> {
        let start = self.found.unwrap_or(from.0);
        let found = self.sweep(start, dir, false)?;
        self.found = Some(found.0);
        Some(Hit { anchor: Anchor(found.0), wrapped: found.1 })
    }

    fn matches_on(&self, row: usize) -> Vec<MatchSpan> {
        if self.query.is_empty() {
            return Vec::new();
        }
        let current = self.found == Some(row);
        crate::source::search::find_in(&self.row_text(row), &self.query, self.sensitive)
            .into_iter()
            .map(|(start, end)| MatchSpan { start, end, current })
            .collect()
    }

    // -- yank -----------------------------------------------------------------

    /// A selection of rows is copied as the *source slice* they cover — from
    /// the first row's value to the last row's — rather than as the tree lines
    /// on screen. It is what the document actually says there, and for a run of
    /// members it pastes straight back.
    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let last = rows.end.saturating_sub(1).min(self.len().saturating_sub(1));
        let (_, start, _) = self.span_of(rows.start)?;
        let (_, _, end) = self.span_of(last)?;
        let (bytes, clipped) = self.doc.borrow_mut().bytes(start, end.max(start));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let n = last.saturating_sub(rows.start) + 1;
        let what = match (n, clipped) {
            (_, true) => format!("{n} rows (clipped at {})", render::size(PARSE_CAP)),
            (1, false) => "1 row".to_string(),
            (n, false) => format!("{n} rows"),
        };
        JsonSource::yank(text, what)
    }

    /// `y`: the value under the cursor. A string is copied as its text, without
    /// the quotes the screen shows it with — on screen the quotes are what tell
    /// `"1"` from `1`, but in a paste buffer they are in the way.
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let raw = self.raw_of(row).ok()?;
        let text = match crate::json::parse(raw.as_bytes()) {
            Ok(v) => v.as_str().map(str::to_string).unwrap_or(raw),
            Err(_) => raw,
        };
        JsonSource::yank(text, self.what(row))
    }

    /// `Y`: the subtree as valid JSON — the document's own bytes with the
    /// insignificant whitespace taken out, so numbers, escapes and duplicate
    /// keys survive exactly.
    fn yank_section(&self, row: usize) -> Option<Yank> {
        let raw = self.raw_of(row).ok()?;
        JsonSource::yank(export::minify(raw.as_bytes()), self.what(row))
    }

    /// `c`: the value verbatim, exactly as it is written in the file —
    /// line breaks, indentation and all.
    fn yank_block(&self, row: usize) -> Option<Yank> {
        let raw = self.raw_of(row).ok()?;
        JsonSource::yank(raw, format!("{} verbatim", self.what(row)))
    }

    /// `Enter` toggles a fold here, so there is no row detail to open: a JSON
    /// row is already one field per line.
    fn detail(&self, _row: usize) -> Option<Detail> {
        None
    }
}
