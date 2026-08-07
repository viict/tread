//! The [`Source`] implementation for a directory listing.
//!
//! The order below follows the trait's own. A listing has no sections, no folds
//! and no anchors to jump to, and every one of those methods gives the honest
//! empty answer rather than an invented one — the same discipline the CSV and
//! text sources follow.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;
use crate::select::Yank;
use crate::source::search::Dir;
use crate::source::{Detail, End, Entry, FoldState, Hit, Mark, MatchSpan, Source};

impl Source for DirSource {
    // -- layout ---------------------------------------------------------------

    /// A listing is a column of short rows: nothing about it depends on the
    /// width, so a resize needs no work.
    fn set_width(&mut self, _cols: usize) {}

    fn len(&self) -> usize {
        self.rows.len()
    }

    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        let end = rows.end.min(self.rows.len());
        let start = rows.start.min(end);
        self.rows[start..end].to_vec()
    }

    fn position_text(&self, row: usize) -> Option<String> {
        let n = self.shown().count();
        // Which entry is this row? The header and its blank come first.
        let idx = row.checked_sub(2);
        Some(match idx.and_then(|i| self.shown().nth(i)) {
            Some(item) => format!("{} of {n}  \u{b7}  {}", idx.unwrap_or(0) + 1, item.url()),
            None => format!("{n} {}", entries(n)),
        })
    }

    fn end(&self) -> End {
        End::At(self.rows.len().saturating_sub(1))
    }

    // -- positions ------------------------------------------------------------

    fn anchor(&self, row: usize) -> Option<Anchor> {
        (row < self.rows.len()).then_some(Anchor(row))
    }

    fn row_of(&self, anchor: Anchor) -> Option<usize> {
        (anchor.0 < self.rows.len()).then_some(anchor.0)
    }

    fn reveal(&mut self, anchor: Anchor) -> Option<usize> {
        let n = self.rows.len();
        (n > 0).then(|| anchor.0.min(n - 1))
    }

    fn mark(&self, row: usize) -> Option<Mark> {
        (row < self.rows.len()).then_some(Mark(row))
    }

    fn locate(&self, mark: Mark) -> Option<usize> {
        let n = self.rows.len();
        (n > 0).then(|| mark.0.min(n - 1))
    }

    // -- structure ------------------------------------------------------------
    //
    // A listing has none: nothing to fold, nothing to outline, nowhere to jump.

    fn outline(&self) -> &[Entry] {
        &[]
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

    /// Every entry, so `n`, `←`/`→` and `Enter` are the navigation the pager
    /// already has. This is what makes walking a tree free.
    fn links(&self) -> &[LinkSite] {
        &self.links
    }

    // -- search ---------------------------------------------------------------

    fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.rematch();
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn current_match(&self) -> Option<usize> {
        self.current
    }

    fn preview_match(&mut self, origin: Anchor, dir: Dir) -> Option<Hit> {
        self.step_match(origin, dir, false)
    }

    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit> {
        self.step_match(from, dir, true)
    }

    fn matches_on(&self, row: usize) -> Vec<MatchSpan> {
        if self.query.is_empty() || !self.matches.contains(&row) {
            return Vec::new();
        }
        let text = match self.rows.get(row) {
            Some(l) => l.text(),
            None => return Vec::new(),
        };
        let hay = text.to_lowercase();
        let needle = self.query.to_lowercase();
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(i) = hay[at..].find(&needle) {
            let byte = at + i;
            // Byte offsets to display columns, which is what the painter wants.
            let start = str_width(&text[..byte]);
            let end = start + str_width(&text[byte..byte + needle.len()]);
            out.push(MatchSpan {
                start,
                end,
                current: self.current == Some(row),
            });
            at = byte + needle.len().max(1);
        }
        out
    }

    // -- yank -----------------------------------------------------------------

    /// The listing as text, which is what a selection of rows looks like.
    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let end = rows.end.min(self.rows.len());
        let start = rows.start.min(end);
        let text: String = self.rows[start..end]
            .iter()
            .map(|l| format!("{}\n", l.text().trim_end()))
            .collect();
        DirSource::yank(text, format!("{} rows", end - start))
    }

    /// `y`: the entry's name, which is the thing worth copying out of a
    /// listing — not the padded row with its size column.
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let item = row.checked_sub(2).and_then(|i| self.shown().nth(i))?;
        DirSource::yank(format!("{}\n", item.url()), item.url())
    }

    /// `Y`: the whole listing, one name per line, so it can be piped.
    fn yank_section(&self, _row: usize) -> Option<Yank> {
        let text: String = self.shown().map(|i| format!("{}\n", i.url())).collect();
        DirSource::yank(text, format!("{}", self.path.display()))
    }

    /// `c`: the absolute path of the entry under the cursor.
    fn yank_block(&self, row: usize) -> Option<Yank> {
        let item = row.checked_sub(2).and_then(|i| self.shown().nth(i))?;
        let p = self.path.join(&item.name);
        DirSource::yank(format!("{}\n", p.display()), p.display().to_string())
    }

    fn toggle_hidden(&mut self) -> Option<String> {
        Some(self.flip_hidden())
    }

    /// A listing row is a name, not a record: there is nothing to expand.
    fn detail(&self, _row: usize) -> Option<Detail> {
        None
    }
}

impl DirSource {
    fn yank(text: String, what: impl Into<String>) -> Option<Yank> {
        match text.trim().is_empty() {
            true => None,
            false => Some(Yank {
                text,
                what: what.into(),
            }),
        }
    }

    /// The next match in `dir` from `origin`, wrapping. `commit` makes it the
    /// current one, which is the only difference between preview and cycle.
    fn step_match(&mut self, origin: Anchor, dir: Dir, commit: bool) -> Option<Hit> {
        if self.matches.is_empty() {
            return None;
        }
        let here = origin.0;
        let (next, wrapped) = match dir {
            Dir::Forward => match self.matches.iter().find(|&&m| m > here) {
                Some(&m) => (m, false),
                None => (self.matches[0], true),
            },
            Dir::Backward => match self.matches.iter().rev().find(|&&m| m < here) {
                Some(&m) => (m, false),
                None => (*self.matches.last()?, true),
            },
        };
        if commit {
            self.current = Some(next);
        }
        Some(Hit {
            anchor: Anchor(next),
            wrapped,
        })
    }
}
