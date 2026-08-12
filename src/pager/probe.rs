//! Test-only accessors on [`Pager`].
//!
//! The pager's own state is private, and the state-machine tests need to read it
//! — the row count, the status line, the fold set, the link count. They live here
//! rather than in `mod.rs` so the production file stays inside the size limit,
//! and behind `#[cfg(test)]` so none of it ships.
#![deny(unsafe_code)]

use super::{view, Anchor, Pager};

impl Pager {
    pub(crate) fn line_count(&self) -> usize {
        self.src.len()
    }
    /// True when a frame should be repainted.
    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }
    /// True while `v` visual line-select mode is active.
    pub(crate) fn in_visual(&self) -> bool {
        self.select.is_some()
    }
    /// Whether the document under the pager reads in blocks.
    pub(crate) fn src_blocks(&self) -> bool {
        self.src.blocks()
    }
    /// The rows of the block on `row`, as the source answers it — for the
    /// framing tests, which have to know what they were framing.
    pub(crate) fn src_block_at(&self, row: usize) -> Option<std::ops::Range<usize>> {
        self.src.block_at(row)
    }
    /// The text the status bar would show right now.
    pub(crate) fn status_line(&self) -> String {
        view::status_text(self)
    }
    pub(crate) fn cursor_text(&mut self) -> String {
        let row = self.cursor;
        self.src
            .line(row)
            .map(|l| l.text().trim().to_string())
            .unwrap_or_default()
    }
    pub(crate) fn visible_text(&mut self) -> Vec<String> {
        let n = self.src.len();
        self.src
            .lines(0..n)
            .iter()
            .map(|l| l.text().trim().to_string())
            .collect()
    }
    /// Fold state, as the source spells it.
    pub(crate) fn folds(&self) -> Vec<String> {
        self.src.folds()
    }
    pub(crate) fn outline(&self) -> &[crate::source::Entry] {
        self.src.outline()
    }
    pub(crate) fn match_count(&self) -> usize {
        self.src.match_count()
    }
    pub(crate) fn current_match(&self) -> Option<usize> {
        self.src.current_match()
    }
    pub(crate) fn link_count(&self) -> usize {
        self.src.links().len()
    }
    pub(crate) fn row_of(&self, anchor: Anchor) -> Option<usize> {
        self.src.row_of(anchor)
    }
    /// The re-layout-stable mark under the cursor.
    pub(crate) fn cursor_mark(&self) -> Option<crate::source::Mark> {
        self.src.mark(self.cursor)
    }
}
