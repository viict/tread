//! The [`Source`] implementation for a code file.
//!
//! Mirrors `MarkdownSource` closely, because both are the same shape: a whole
//! document laid out once, a collapse tree over it, and rows that index the
//! visible lines. Where they differ is only what makes a heading.
#![deny(unsafe_code)]

use std::ops::Range;

use super::*;
use crate::render::str_width;
use crate::select::Yank;
use crate::source::search::Dir;
use crate::source::{Detail, End, Hit, LinkSite, Mark, MatchSpan, Source};

impl Source for CodeSource {
    // -- layout ---------------------------------------------------------------

    /// Code is never reflowed, so width changes nothing about the layout — a
    /// row too wide scrolls sideways instead (SPEC.md §Code).
    fn set_width(&mut self, _cols: usize) {}

    fn len(&self) -> usize {
        self.visible.len()
    }

    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        self.line_rows(rows)
            .into_iter()
            .map(|i| self.lines[i].clone())
            .collect()
    }

    fn full_width(&self) -> bool {
        true
    }

    /// `file.rs · rust · 23 symbols` — or why there are none.
    fn position_text(&self, row: usize) -> Option<String> {
        if self.unparsed {
            return Some(format!("{}  \u{b7}  unparsed, shown raw", self.lang));
        }
        let line = self.at(row).map(|i| self.lines[i].source_line).unwrap_or(0);
        Some(format!(
            "{}  \u{b7}  {} symbols  \u{b7}  line {line}",
            self.lang,
            self.outline.len()
        ))
    }

    fn end(&self) -> End {
        End::At(self.visible.len().saturating_sub(1))
    }

    // -- positions ------------------------------------------------------------

    fn anchor(&self, row: usize) -> Option<Anchor> {
        self.at(row).map(Anchor)
    }

    fn row_of(&self, anchor: Anchor) -> Option<usize> {
        self.visible.binary_search(&anchor.0).ok()
    }

    fn reveal(&mut self, anchor: Anchor) -> Option<usize> {
        if fold::reveal(&self.regions, &mut self.collapsed, anchor.0) {
            self.refresh();
        }
        if self.visible.is_empty() {
            return None;
        }
        let at = match self.visible.binary_search(&anchor.0) {
            Ok(at) | Err(at) => at,
        };
        Some(at.min(self.visible.len() - 1))
    }

    /// Marks are source lines, so a position survives folding and unfolding —
    /// which is the whole point when `r` changes what is on screen.
    fn mark(&self, row: usize) -> Option<Mark> {
        self.at(row).map(|i| Mark(self.lines[i].source_line))
    }

    fn locate(&self, mark: Mark) -> Option<usize> {
        if self.visible.is_empty() {
            return None;
        }
        Some(
            self.visible
                .iter()
                .position(|i| self.lines[*i].source_line >= mark.0)
                .unwrap_or(self.visible.len() - 1),
        )
    }

    // -- structure ------------------------------------------------------------

    fn outline(&self) -> &[Entry] {
        &self.outline
    }

    fn section_at(&self, row: usize) -> Option<usize> {
        let line = self.at(row)?;
        let head = fold::at(&self.regions, line)?.head;
        self.outline.iter().position(|e| e.anchor == Anchor(head))
    }

    /// The innermost region at this row — a branch, a loop, or the declaration
    /// itself when the cursor is not inside one.
    fn fold_here(&mut self, row: usize) -> Option<bool> {
        let line = self.at(row)?;
        let id = fold::at(&self.regions, line)?.id.clone();
        let closed = !self.collapsed.contains(&id);
        match closed {
            true => self.collapsed.push(id),
            false => self.collapsed.retain(|c| *c != id),
        }
        self.refresh();
        Some(closed)
    }

    fn set_fold(&mut self, entry: usize, closed: bool) -> bool {
        let id = match self.outline.get(entry) {
            Some(e) => e.id.clone(),
            None => return false,
        };
        if self.collapsed.contains(&id) == closed {
            return false;
        }
        match closed {
            true => self.collapsed.push(id),
            false => self.collapsed.retain(|c| *c != id),
        }
        self.refresh();
        true
    }

    fn fold_all(&mut self, closed: bool) {
        self.collapsed = match closed {
            true => self.foldable.clone(),
            false => Vec::new(),
        };
        self.refresh();
    }

    fn folds(&self) -> FoldState {
        self.collapsed.clone()
    }

    fn set_folds(&mut self, folds: FoldState) {
        self.collapsed = folds;
        self.refresh();
    }

    /// What a folded import run says instead of a line count.
    fn fold_note(&self, row: usize) -> Option<String> {
        let line = self.at(row)?;
        let id = &self.lines[line].heading.as_ref()?.id;
        self.notes
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, n)| n.clone())
    }

    /// The count sits on the signature row, which is the last row the heading
    /// owns — so this does not require the row to carry the heading itself.
    fn hidden_at(&self, row: usize) -> Option<usize> {
        let line = self.at(row)?;
        self.counts.iter().find(|(h, _)| *h == line).map(|(_, n)| *n)
    }

    /// `]` and `[`: the next declaration, which is what a reader steps through.
    fn next_landmark(&self, row: usize, forward: bool) -> Option<usize> {
        let is_head = |r: usize| {
            self.at(r)
                .map(|i| self.lines[i].heading.is_some())
                .unwrap_or(false)
        };
        match forward {
            true => (row + 1..self.visible.len()).find(|&r| is_head(r)),
            false => (0..row).rev().find(|&r| is_head(r)),
        }
    }

    /// `#some::symbol` — the anchor a jump targets. The ids are the symbol
    /// paths, so this is already "go to definition" within a file.
    fn goto_id(&mut self, id: &str) -> Option<usize> {
        let entry = self.outline.iter().find(|e| e.id == id)?;
        let anchor = entry.anchor;
        self.reveal(anchor)
    }

    /// One per import that resolves to a file here, so `n` walks them and
    /// `Enter` opens the module — jumping between sources without leaving the
    /// reader. An identifier is deliberately *not* a link: resolving one needs
    /// types, and a wrong jump is worse than none (SPEC.md §Code).
    fn links(&self) -> &[LinkSite] {
        &self.links
    }

    /// `r`: symbols, or source.
    fn toggle_hidden(&mut self) -> Option<String> {
        match self.unparsed {
            true => Some("this file did not parse; it is already raw".into()),
            false => Some(self.flip_source()),
        }
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
        self.step(origin, dir, false)
    }

    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit> {
        self.step(from, dir, true)
    }

    fn matches_on(&self, row: usize) -> Vec<MatchSpan> {
        if self.query.is_empty() || !self.matches.contains(&row) {
            return Vec::new();
        }
        let Some(i) = self.at(row) else {
            return Vec::new();
        };
        let text = self.lines[i].text();
        let hay = text.to_lowercase();
        let needle = self.query.to_lowercase();
        let mut out = Vec::new();
        let mut at = 0;
        while let Some(found) = hay[at..].find(&needle) {
            let byte = at + found;
            let start = str_width(&text[..byte]);
            out.push(MatchSpan {
                start,
                end: start + str_width(&text[byte..byte + needle.len()]),
                current: self.current == Some(row),
            });
            at = byte + needle.len().max(1);
        }
        out
    }

    // -- yank -----------------------------------------------------------------

    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let text: String = self
            .line_rows(rows.clone())
            .into_iter()
            .map(|i| format!("{}\n", self.lines[i].text().trim_end()))
            .collect();
        yank(text, format!("{} rows", rows.end - rows.start))
    }

    fn yank_point(&self, row: usize) -> Option<Yank> {
        let i = self.at(row)?;
        let text = self.lines[i].text();
        yank(format!("{}\n", text.trim_end()), "the line")
    }

    /// `Y`: the whole symbol under the cursor, body included even when folded —
    /// which is what copying a function should mean.
    fn yank_section(&self, row: usize) -> Option<Yank> {
        let entry = self.section_at(row)?;
        let head = self.outline.get(entry)?;
        let start = head.anchor.0;
        let end = collapse::headings(&self.lines)
            .into_iter()
            .find(|h| h.index > start && h.level <= head.level)
            .map(|h| h.index)
            .unwrap_or(self.lines.len());
        let text: String = self.lines[start..end]
            .iter()
            .map(|l| format!("{}\n", l.text().trim_end()))
            .collect();
        yank(text, head.text.clone())
    }

    /// `c`: the symbol's path, which is what you paste into a message.
    fn yank_block(&self, row: usize) -> Option<Yank> {
        let entry = self.section_at(row)?;
        let head = self.outline.get(entry)?;
        yank(format!("{}\n", head.id), head.id.clone())
    }

    fn detail(&self, _row: usize) -> Option<Detail> {
        None
    }
}

fn yank(text: String, what: impl Into<String>) -> Option<Yank> {
    match text.trim().is_empty() {
        true => None,
        false => Some(Yank {
            text,
            what: what.into(),
        }),
    }
}

impl CodeSource {
    /// The next match in `dir` from `origin`, wrapping.
    fn step(&mut self, origin: Anchor, dir: Dir, commit: bool) -> Option<Hit> {
        if self.matches.is_empty() {
            return None;
        }
        let here = self.visible.binary_search(&origin.0).unwrap_or(0);
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
            anchor: Anchor(self.visible.get(next).copied().unwrap_or(0)),
            wrapped,
        })
    }
}
