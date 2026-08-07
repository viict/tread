//! The [`Source`] implementation itself: what the pager sees.
//!
//! Split from `mod.rs`, which holds the state and the file access, so both stay
//! under the size limit. The order of the sections below is the order of the
//! trait's own (`src/source/mod.rs`), and every method a text file has no
//! honest answer for says so rather than inventing one.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;

impl Source for TextSource {
    // -- layout ---------------------------------------------------------------

    /// Width changes nothing about the rows: a line is never wrapped, so one
    /// line stays one row at any width and every [`Mark`] survives a resize.
    /// The width is therefore not even kept — holding a field nothing reads
    /// would be state that could go stale without anything noticing. A first
    /// slice is indexed so `len()` is not zero before the first paint.
    fn set_width(&mut self, _cols: usize) {
        self.index_to(FIRST_LINES, FRAME_BYTES);
    }

    fn len(&self) -> usize {
        self.known()
    }

    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        // Index past the window so the viewport can move again next frame.
        self.index_to(rows.end.saturating_add(LOOKAHEAD), FRAME_BYTES);
        let end = rows.end.min(self.len());
        let start = rows.start.min(end);
        (start..end).filter_map(|r| self.row_line(r)).collect()
    }

    // -- viewport affordances ---------------------------------------------------

    /// A log is not prose: a 200-column terminal should show 200 columns of it
    /// rather than the reading measure wrapping would want, because nothing
    /// here is wrapped. `--width` still wins.
    fn full_width(&self) -> bool {
        true
    }

    /// `line 120/840`, or `\u{2265}N (indexing 12%)` while the lazy index is
    /// still catching up (SPEC.md §CSV, the same contract). The percentage
    /// through the document is only shown once the total is actually known:
    /// a position out of a total that is still growing would move backwards
    /// under the reader.
    fn position_text(&self, row: usize) -> Option<String> {
        let known = self.known();
        let cur = match known {
            0 => 0,
            _ => row.min(known - 1) + 1,
        };
        if !self.complete() {
            let pct = self.store.borrow().progress().percent();
            return Some(format!("line {cur}/\u{2265}{known} (indexing {pct}%)"));
        }
        let pct = match known <= 1 {
            true => 100,
            false => (cur - 1) * 100 / (known - 1),
        };
        Some(format!("{pct}%  \u{b7}  line {cur}/{known}"))
    }

    /// `G`. Until the line index has reached the end of the file there is no
    /// honest answer, so this reports how far the scan got and lets the pager
    /// drive it (SPEC.md §CSV).
    fn end(&self) -> End {
        if !self.complete() {
            return End::Scanning(self.store.borrow().progress().percent());
        }
        End::At(self.len().saturating_sub(1))
    }

    fn extend(&mut self) -> bool {
        let mut guard = self.store.borrow_mut();
        let s = &mut *guard;
        if s.index.complete() {
            return false;
        }
        let before = s.index.known();
        s.index.ensure_bytes(IDLE_BYTES, &mut s.reader);
        // True while there is more to do *or* this slice found new lines: a
        // caller that walks the document by `len()` — `dump::write_source` —
        // asks again only while this says yes.
        !s.index.complete() || s.index.known() > before
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

    // -- structure ---------------------------------------------------------------
    //
    // A text file has no sections: nothing to fold, nothing to outline, no
    // anchor to jump to and no links to follow. Every one of these is the
    // honest empty answer rather than an invented one (SPEC.md §Plain text:
    // "what a text file has no answer for ... says so rather than pretending").

    fn outline(&self) -> &[Entry] {
        &self.none_outline
    }

    fn section_at(&self, _row: usize) -> Option<usize> {
        None
    }

    fn set_fold(&mut self, _entry: usize, _closed: bool) -> bool {
        false
    }

    fn fold_all(&mut self, _closed: bool) {}

    fn folds(&self) -> FoldState {
        Vec::new()
    }

    fn set_folds(&mut self, _folds: FoldState) {}

    fn hidden_at(&self, _row: usize) -> Option<usize> {
        None
    }

    fn next_landmark(&self, _row: usize, _forward: bool) -> Option<usize> {
        None
    }

    fn goto_id(&mut self, _id: &str) -> Option<usize> {
        None
    }

    fn links(&self) -> &[LinkSite] {
        &self.none_links
    }

    // -- search --------------------------------------------------------------

    fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.sensitive = search::case_sensitive(query);
        self.needle = match self.sensitive {
            true => query.to_string(),
            false => query.to_lowercase(),
        };
        self.found = None;
    }

    /// 1 while the last sweep has a hit, 0 otherwise: a lazily indexed file
    /// cannot be counted without reading all of it, and claiming a number
    /// would be a lie — the same answer the CSV source gives.
    fn match_count(&self) -> usize {
        usize::from(self.found.is_some())
    }

    fn current_match(&self) -> Option<usize> {
        self.found.map(|_| 0)
    }

    fn preview_match(&mut self, origin: Anchor, dir: Dir) -> Option<Hit> {
        let found = self.sweep(origin.0, dir, true);
        self.hit(found)
    }

    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit> {
        let start = self.found.unwrap_or(from.0);
        let found = self.sweep(start, dir, false);
        self.hit(found)
    }

    /// Columns of the matches on a painted row. Measured against the *painted*
    /// text, tabs already expanded, so the highlight sits under the characters
    /// the reader can see.
    fn matches_on(&self, row: usize) -> Vec<MatchSpan> {
        if self.query.is_empty() {
            return Vec::new();
        }
        let current = self.found == Some(row);
        search::find_in(&self.row_text(row), &self.query, self.sensitive)
            .into_iter()
            .map(|(start, end)| MatchSpan { start, end, current })
            .collect()
    }

    // -- yank -----------------------------------------------------------------

    /// A visual selection: those lines, verbatim. The bytes the file holds —
    /// tabs are tabs and a control character is itself, because sanitising is
    /// what the screen needs and not what the clipboard wants.
    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let end = rows.end.min(self.len());
        let start = rows.start.min(end);
        let mut out = String::new();
        let mut n = 0;
        for row in start..end {
            if let Some(text) = self.raw(row) {
                out.push_str(&text);
                out.push('\n');
                n += 1;
            }
        }
        let what = match n {
            1 => "1 line".to_string(),
            n => format!("{n} lines"),
        };
        TextSource::yank(out, what)
    }

    /// `y` with nothing selected: the line under the cursor, verbatim. A line
    /// is the smallest thing a text file has — there are no cells and no
    /// values — and it is what the cursor is on.
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let raw = self.raw(row)?;
        TextSource::yank(format!("{raw}\n"), format!("line {}", row + 1))
    }

    /// `Y` and `c` have nothing to name here: a text file has no sections and
    /// no blocks, and copying the whole file because the cursor is somewhere in
    /// it would be inventing a structure the format does not have. `y` (a line)
    /// and `v`+`y` (a range) are the yanks that mean something.
    fn yank_section(&self, _row: usize) -> Option<Yank> {
        None
    }

    fn yank_block(&self, _row: usize) -> Option<Yank> {
        None
    }
}
