//! The [`Source`] implementation itself: what the pager sees.
//!
//! Split from `mod.rs`, which holds the state and the file access, so both
//! stay under the size limit. The order of the sections below is the order of
//! the trait's own (`src/source/mod.rs`), and every method that a CSV has no
//! honest answer for says so rather than inventing one.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;

impl Source for CsvSource {
    // -- layout ---------------------------------------------------------------

    fn set_width(&mut self, cols: usize) {
        self.view = cols.max(1);
        if self.grid.is_empty() {
            self.sample();
        }
        self.grid.fit(self.view);
        self.col = self.col.min(self.grid.arity().saturating_sub(1));
    }

    fn len(&self) -> usize {
        if self.grid.is_empty() || self.known() == 0 {
            return 0;
        }
        HEAD_ROWS + self.data_len() + usize::from(self.complete())
    }

    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        // Index past the window so the viewport can move again next frame.
        self.index_to(rows.end.saturating_add(LOOKAHEAD), FRAME_BYTES);
        let end = rows.end.min(self.len());
        let start = rows.start.min(end);
        self.window = start.saturating_sub(HEAD_ROWS)..end.saturating_sub(HEAD_ROWS);
        (start..end).filter_map(|r| self.row_line(r)).collect()
    }

    // -- viewport affordances ---------------------------------------------------

    fn pinned(&self) -> usize {
        match self.len() {
            0 => 0,
            _ => HEAD_ROWS,
        }
    }

    fn full_width(&self) -> bool {
        true
    }

    /// The offset is computed against `view`, the columns the terminal is
    /// showing, never against the layout width in `self.view`: `--width 200` on
    /// a 40-column terminal lays the grid out wide and shows 40 of it, and
    /// clamping to `total - 200` there would pin the offset at 0 and make `l`
    /// a no-op with the columns past the right edge unreachable.
    fn hscroll(&mut self, hoff: usize, dir: isize, view: usize) -> Option<usize> {
        let n = self.grid.arity();
        if n == 0 {
            return Some(0);
        }
        // Start from something the reader can actually see, so `l` after a
        // resize does not step off from a column that scrolled away.
        let here = self.col.min(n - 1).max(self.grid.first_visible(hoff));
        self.col = match dir < 0 {
            true => here.saturating_sub(1),
            false => (here + 1).min(n - 1),
        };
        Some(self.grid.scroll_to(self.col, hoff, view))
    }

    fn widen(&mut self) -> Option<String> {
        let name = self.grid.name_of(self.col)?.to_string();
        let mut want = self.grid.width_of(self.col);
        for d in self.window.clone() {
            if let Some(f) = self.fields(d) {
                let text = f.get(self.col).map(String::as_str).map(render::clean).unwrap_or_default();
                want = want.max(crate::render::str_width(&text));
            }
        }
        let got = self.grid.set_fixed(self.col, want, self.view)?;
        Some(format!("column \u{201c}{name}\u{201d} fitted to {got} columns"))
    }

    fn position_text(&self, row: usize) -> Option<String> {
        let name = self.grid.name_of(self.col).unwrap_or("");
        let known = self.data_len();
        let (store, complete) = (self.store.borrow(), self.complete());
        let pct = store.progress().percent();
        drop(store);
        let total = match complete {
            true => format!("{known}"),
            false => format!("\u{2265}{known} (indexing {pct}%)"),
        };
        Some(match self.kind(row) {
            Some(Kind::Data(d)) => format!("row {}/{total}  \u{b7}  {name}", d + 1),
            // The bottom border is past the last row, and saying "header" there
            // — which is what a catch-all arm did — is simply false.
            Some(Kind::Bottom) => format!("end of {total} rows  \u{b7}  {name}"),
            _ => format!("header of {total}  \u{b7}  {name}"),
        })
    }

    /// `G`. The last *data* row, not the bottom border: a border is not
    /// something a cursor can yank, search or name in the status bar. Until the
    /// row index has reached the end of the file there is no honest answer at
    /// all, so this reports how far the scan has got and lets the pager come
    /// back (SPEC.md §CSV).
    fn end(&self) -> End {
        if !self.complete() {
            return End::Scanning(self.store.borrow().progress().percent());
        }
        match self.data_len() {
            0 => End::At(0),
            n => End::At(HEAD_ROWS + n - 1),
        }
    }

    fn extend(&mut self) -> bool {
        let mut guard = self.store.borrow_mut();
        let s = &mut *guard;
        if s.index.complete() {
            return false;
        }
        s.index.ensure_bytes(IDLE_BYTES, &mut s.reader);
        !s.index.complete()
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
    // A CSV has no sections: there is nothing to fold, nothing to outline and
    // no anchor to jump to. Every one of these is the honest empty answer
    // rather than an invented one.

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
    /// would be a lie. The pager only ever asks "is there one?".
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

    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let picked = self.rows_csv(rows);
        let what = match picked.len() {
            1 => "1 row".to_string(),
            n => format!("{n} rows"),
        };
        CsvSource::yank(yank::records(&picked, self.delim), what)
    }

    /// `y` with nothing selected: the cell under the cursor, re-quoted so a
    /// value holding the delimiter still parses as one field.
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let d = match self.kind(row)? {
            Kind::Data(d) => d,
            _ => return None,
        };
        let value = self.fields(d)?.get(self.col)?.clone();
        let name = self.grid.name_of(self.col).unwrap_or("").to_string();
        CsvSource::yank(
            format!("{}\n", yank::field(&value, self.delim)),
            format!("cell {name} \u{b7} row {}", d + 1),
        )
    }

    /// `Enter`: the row under the cursor, one field per line.
    ///
    /// Built from the *raw* record, not the display fields, so a row carrying
    /// more values than the header named shows all of them — the grid is
    /// header-shaped and cannot, which is the whole reason this exists.
    fn detail(&self, row: usize) -> Option<Detail> {
        let (title, fields) = match self.kind(row)? {
            Kind::Data(d) => (format!("Row {}", d + 1), self.raw_row(d + 1)?),
            Kind::Header => ("Header".to_string(), self.raw_row(0)?),
            _ => return None,
        };
        let named = fields
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                let label = match self.grid.name_of(i) {
                    Some(n) if !n.is_empty() => n.to_string(),
                    // Past the header, or a column the header left blank: name
                    // it by position. An unnamed field is still a field.
                    _ => format!("[{}]", i + 1),
                };
                (label, render::clean(&value))
            })
            .collect();
        Some(Detail {
            title,
            fields: named,
        })
    }

    /// `Y`: the row under the cursor as one valid CSV record.
    fn yank_section(&self, row: usize) -> Option<Yank> {
        let (label, fields) = match self.kind(row)? {
            Kind::Data(d) => (format!("row {}", d + 1), self.raw_row(d + 1)?),
            _ => ("header row".to_string(), self.raw_row(0)?),
        };
        CsvSource::yank(yank::record(&fields, self.delim), label)
    }

    /// `c`: the column under the cursor, header first, as a one-column CSV.
    fn yank_block(&self, _row: usize) -> Option<Yank> {
        let name = self.grid.name_of(self.col)?.to_string();
        let take = self.data_len().min(COLUMN_CAP);
        let mut values = Vec::with_capacity(take);
        for d in 0..take {
            let f = self.fields(d).unwrap_or_default();
            values.push(f.get(self.col).cloned().unwrap_or_default());
        }
        let more = match self.data_len() > take || !self.complete() {
            true => format!("first {take} of \u{2265}{} rows", self.data_len()),
            false => format!("{take} rows"),
        };
        CsvSource::yank(
            yank::column(&name, &values, self.delim),
            format!("column \u{201c}{name}\u{201d} ({more})"),
        )
    }
}
