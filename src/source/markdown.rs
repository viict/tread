//! Markdown behind the [`Source`] seam.
//!
//! A thin adapter over the pipeline that was there before the seam existed:
//! [`crate::md::parse`] -> [`render_document`] -> `Vec<Line>`, eagerly, for the
//! whole document. Markdown files are small (SPEC.md §The `Source` seam), and
//! nothing about their behaviour changes by moving behind the trait: the fold
//! tree, search, the outline, links and the yank commands are the same code
//! they were, just called from here instead of from the pager.
//!
//! Coordinates: the pager's *rows* are indices into [`Self::visible`], which is
//! the fold-filtered view of `lines`; an [`Anchor`] is an index into `lines`; a
//! [`Mark`] is a 1-based source line.
#![deny(unsafe_code)]

use std::ops::Range;

use super::collapse::{self, HeadingRef};
use super::search::{self, Dir, Match};
use super::{Anchor, Entry, FoldState, Hit, LinkSite, Mark, MatchSpan, Source};
use crate::md::Document;
use crate::render::{render_document, Line, RenderOpts};
use crate::select::{self, Yank};

pub struct MarkdownSource {
    doc: Document,
    /// The whole document, laid out at `width`, folds *not* applied.
    lines: Vec<Line>,
    /// Indices into `lines` that a fold does not hide. Rows index this.
    visible: Vec<usize>,
    /// `(line index of a folded heading, rows it hides)`.
    counts: Vec<(usize, usize)>,
    /// Folded heading ids. Keyed by id so folds survive re-layout.
    collapsed: FoldState,
    outline: Vec<Entry>,
    links: Vec<LinkSite>,
    query: String,
    matches: Vec<Match>,
    current: Option<usize>,
}

impl MarkdownSource {
    /// Wrap a parsed document. Nothing is laid out until [`Source::set_width`],
    /// which the pager calls before the first paint.
    pub fn new(doc: Document) -> MarkdownSource {
        // Metadata starts folded. Open, a `related:` list of seven entries
        // pushes the document's own first paragraph off the screen; closed, its
        // one summary row still says the status, the owner and how much is
        // behind it. `za` on that row opens it.
        let collapsed = match doc
            .blocks
            .first()
            .is_some_and(|b| matches!(b, crate::md::ast::Block::FrontMatter { .. }))
        {
            true => vec![crate::render::METADATA_ID.to_string()],
            false => Vec::new(),
        };
        MarkdownSource {
            doc,
            lines: Vec::new(),
            visible: Vec::new(),
            counts: Vec::new(),
            collapsed,
            outline: Vec::new(),
            links: Vec::new(),
            query: String::new(),
            matches: Vec::new(),
            current: None,
        }
    }

    /// Line index behind a row.
    fn at(&self, row: usize) -> Option<usize> {
        self.visible.get(row).copied()
    }

    /// Recompute the visible list and the fold summaries from `collapsed`.
    fn refresh_view(&mut self) {
        self.visible = collapse::visible_lines(&self.lines, &self.collapsed);
        self.counts = collapse::fold_counts(&self.lines, &self.collapsed);
        for e in self.outline.iter_mut() {
            e.folded = self.collapsed.contains(&e.id);
        }
    }

    /// Recompute the matches for the live query, as the pager's `rescan` did.
    fn rescan(&mut self) {
        self.matches = search::find_all(&self.lines, &self.query);
        if self.matches.is_empty() {
            self.current = None;
        }
    }

    /// Turn a resolved match index into a [`Hit`].
    fn hit(&mut self, found: Option<(usize, bool)>) -> Option<Hit> {
        let (i, wrapped) = found?;
        self.current = Some(i);
        let line = self.matches.get(i)?.line;
        Some(Hit { anchor: Anchor(line), wrapped })
    }

    /// Rows -> line indices, clamped, for the yank paths.
    fn line_rows(&self, rows: Range<usize>) -> Vec<usize> {
        let end = rows.end.min(self.visible.len());
        let start = rows.start.min(end);
        self.visible[start..end].to_vec()
    }
}

fn entries(lines: &[Line], collapsed: &[String]) -> Vec<Entry> {
    collapse::headings(lines)
        .into_iter()
        .map(|h: HeadingRef| Entry {
            level: h.level,
            folded: collapsed.contains(&h.id),
            id: h.id,
            text: h.text,
            anchor: Anchor(h.index),
        })
        .collect()
}

fn link_sites(lines: &[Line]) -> Vec<LinkSite> {
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        for (col, url) in l.links() {
            out.push(LinkSite {
                anchor: Anchor(i),
                col,
                url: url.to_string(),
            });
        }
    }
    out
}

impl Source for MarkdownSource {
    // -- layout --------------------------------------------------------------

    fn set_width(&mut self, cols: usize) {
        self.lines = render_document(&self.doc, &RenderOpts::new(cols));
        self.outline = entries(&self.lines, &self.collapsed);
        self.links = link_sites(&self.lines);
        self.refresh_view();
        self.rescan();
    }

    fn len(&self) -> usize {
        self.visible.len()
    }

    fn lines(&mut self, rows: Range<usize>) -> Vec<Line> {
        self.line_rows(rows)
            .into_iter()
            .map(|i| self.lines[i].clone())
            .collect()
    }

    // -- positions -----------------------------------------------------------

    fn anchor(&self, row: usize) -> Option<Anchor> {
        self.at(row).map(Anchor)
    }

    fn row_of(&self, anchor: Anchor) -> Option<usize> {
        self.visible.binary_search(&anchor.0).ok()
    }

    fn reveal(&mut self, anchor: Anchor) -> Option<usize> {
        if collapse::reveal(&self.lines, &mut self.collapsed, anchor.0) {
            self.refresh_view();
        }
        if self.visible.is_empty() {
            return None;
        }
        let at = match self.visible.binary_search(&anchor.0) {
            Ok(at) | Err(at) => at,
        };
        Some(at.min(self.visible.len() - 1))
    }

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

    // -- structure -----------------------------------------------------------

    fn outline(&self) -> &[Entry] {
        &self.outline
    }

    fn section_at(&self, row: usize) -> Option<usize> {
        let line = self.at(row)?;
        let head = collapse::heading_at_or_above(&self.lines, line)?;
        self.outline.iter().position(|e| e.anchor == Anchor(head))
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
        self.refresh_view();
        true
    }

    fn fold_all(&mut self, closed: bool) {
        self.collapsed = match closed {
            true => collapse::all_ids(&self.lines),
            false => Vec::new(),
        };
        self.refresh_view();
    }

    fn folds(&self) -> FoldState {
        self.collapsed.clone()
    }

    fn set_folds(&mut self, folds: FoldState) {
        self.collapsed = folds;
        self.refresh_view();
    }

    fn hidden_at(&self, row: usize) -> Option<usize> {
        let line = self.at(row)?;
        self.lines[line].heading.as_ref()?;
        self.counts.iter().find(|(h, _)| *h == line).map(|(_, n)| *n)
    }

    fn next_landmark(&self, row: usize, forward: bool) -> Option<usize> {
        let heading = |r: &usize| {
            self.at(*r)
                .map(|i| self.lines[i].heading.is_some())
                .unwrap_or(false)
        };
        match forward {
            true => (row + 1..self.visible.len()).find(heading),
            false => (0..row).rev().find(heading),
        }
    }

    fn goto_id(&mut self, id: &str) -> Option<usize> {
        let found = self
            .lines
            .iter()
            .position(|l| matches!(&l.heading, Some(h) if h.id == id))?;
        self.collapsed.retain(|c| c != id);
        self.refresh_view();
        self.reveal(Anchor(found))
    }

    // -- links ---------------------------------------------------------------

    fn links(&self) -> &[LinkSite] {
        &self.links
    }

    // -- search --------------------------------------------------------------

    fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.rescan();
    }

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn current_match(&self) -> Option<usize> {
        self.current
    }

    fn preview_match(&mut self, origin: Anchor, dir: Dir) -> Option<Hit> {
        let found = search::seek(&self.matches, origin.0, 0, dir, true);
        self.hit(found)
    }

    fn cycle_match(&mut self, from: Anchor, dir: Dir) -> Option<Hit> {
        let (line, col) = match self.current.and_then(|i| self.matches.get(i)) {
            Some(m) => (m.line, m.start),
            None => (from.0, 0),
        };
        let found = search::seek(&self.matches, line, col, dir, false);
        self.hit(found)
    }

    fn matches_on(&self, row: usize) -> Vec<MatchSpan> {
        let line = match self.at(row) {
            Some(l) => l,
            None => return Vec::new(),
        };
        let current = self.current.and_then(|i| self.matches.get(i)).copied();
        search::on_line(&self.matches, line)
            .into_iter()
            .map(|(start, end)| MatchSpan {
                start,
                end,
                current: current
                    .map(|c| c.line == line && c.start == start)
                    .unwrap_or(false),
            })
            .collect()
    }

    // -- yank ----------------------------------------------------------------

    fn yank_rows(&self, rows: Range<usize>) -> Option<Yank> {
        let picked = self.line_rows(rows);
        select::selection_yank(&self.doc, &self.lines, &picked)
    }

    /// `y` with nothing selected, on a metadata row: that field's value.
    ///
    /// The same shape as a CSV cell yank, for the same reason — a metadata row
    /// is a `key: value`, and what you want from it is the value, not the whole
    /// block. `Y` still copies the block as pasteable YAML. Everywhere else in
    /// a markdown document there is no "smallest thing worth copying", so this
    /// returns `None` and the pager falls back to the focused link.
    fn yank_point(&self, row: usize) -> Option<Yank> {
        let line = self.lines.get(self.at(row)?)?;
        if !matches!(
            self.doc.blocks.get(line.block),
            Some(crate::md::ast::Block::FrontMatter { .. })
        ) {
            return None;
        }
        // The value is the last span; the first is the dim label. A rule row
        // has one span and no value, so it yields nothing to copy.
        let value = line.spans.last().filter(|_| line.spans.len() > 1)?;
        let text = value.text.trim();
        (!text.is_empty()).then(|| Yank {
            text: format!("{text}\n"),
            what: format!("metadata \u{b7} {text}"),
        })
    }

    fn yank_section(&self, row: usize) -> Option<Yank> {
        select::section_yank(&self.doc, &self.lines, self.at(row)?)
    }

    fn yank_block(&self, row: usize) -> Option<Yank> {
        // No row (empty document) means "from the top", which is what the
        // pager's `cursor_line().unwrap_or(0)` used to mean.
        let line = self.at(row).unwrap_or(self.lines.len());
        select::code_yank(&self.doc, &self.lines, line)
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
