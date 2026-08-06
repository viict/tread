//! Column geometry: sampled widths, the box-drawing grid, and where each
//! column sits in display columns (SPEC.md §CSV, "Column widths are sampled").
//!
//! Nothing here reads a file. It is handed the first [`SAMPLE_ROWS`] rows'
//! fields, keeps the widest display width it saw per column, and answers every
//! geometry question the renderer and the horizontal scroller ask. That split
//! is what makes the layout testable without a 2GB fixture — and what keeps the
//! pinned header aligned with the body, since both are drawn from this one
//! object.
//!
//! Widths are display widths ([`str_width`]), never `.len()`: a CJK cell is two
//! columns per character and a combining mark is zero (SPEC.md §Width &
//! unicode).
#![deny(unsafe_code)]

use crate::render::str_width;

/// Rows sampled to choose the column widths. Open time must not depend on file
/// size, so this is a constant number of rows and not a fraction of the file.
pub const SAMPLE_ROWS: usize = 1000;

/// No sampled column is laid out wider than this, however long its values are:
/// one 300-character note field must not push every other column off screen.
/// `w` overrides it for one column on demand.
pub const MAX_COL: usize = 60;

/// Spaces either side of a cell's text, as in a markdown table.
pub const PAD: usize = 1;

/// Columns one cell costs on top of its content: two pads and the bar to its
/// left, which is what makes `bar(i + 1) - bar(i) == width + CELL_EXTRA`.
pub const CELL_EXTRA: usize = PAD * 2 + 1;

/// One column: its header text, the widest value seen while sampling, and the
/// width it is actually laid out at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Col {
    pub name: String,
    /// Widest display width seen in the sample (header included).
    pub sampled: usize,
    /// Width forced by `w`; survives a resize, re-capped to the new viewport.
    pub fixed: Option<usize>,
    /// The width in force, pads excluded. Always >= 1.
    pub width: usize,
}

/// Every column, plus the arithmetic that turns them into a grid.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grid {
    pub cols: Vec<Col>,
}

impl Grid {
    /// A grid for `header`, sized to the header text alone. Feed it rows with
    /// [`Grid::sample`] and then call [`Grid::fit`].
    pub fn new(header: &[String]) -> Grid {
        Grid {
            cols: header
                .iter()
                .map(|name| Col {
                    sampled: str_width(name).max(1),
                    name: name.clone(),
                    fixed: None,
                    width: str_width(name).max(1),
                })
                .collect(),
        }
    }

    pub fn arity(&self) -> usize {
        self.cols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }

    /// Let one row's values grow the sampled widths. Extra fields beyond the
    /// header's arity are ignored here — the row renderer marks them instead.
    pub fn sample(&mut self, fields: &[String]) {
        for (col, value) in self.cols.iter_mut().zip(fields) {
            col.sampled = col.sampled.max(str_width(value));
        }
    }

    /// Apply the viewport cap and any `w` override. Called after sampling and
    /// again on every resize, so a narrower terminal re-caps rather than
    /// re-samples (which would mean re-reading the file).
    pub fn fit(&mut self, view: usize) {
        let cap = cap_for(view);
        for col in self.cols.iter_mut() {
            col.width = match col.fixed {
                Some(w) => w.clamp(1, cap),
                None => col.sampled.clamp(1, cap.min(MAX_COL)),
            };
        }
    }

    /// `w`: pin column `i` to `want` columns of content, capped to the
    /// viewport. Returns the width actually adopted.
    pub fn set_fixed(&mut self, i: usize, want: usize, view: usize) -> Option<usize> {
        let cap = cap_for(view);
        let col = self.cols.get_mut(i)?;
        let width = want.clamp(1, cap);
        col.fixed = Some(width);
        col.width = width;
        Some(width)
    }

    pub fn width_of(&self, i: usize) -> usize {
        self.cols.get(i).map(|c| c.width).unwrap_or(0)
    }

    pub fn name_of(&self, i: usize) -> Option<&str> {
        self.cols.get(i).map(|c| c.name.as_str())
    }

    /// Display column of the vertical bar to the left of column `i`. Column
    /// `i`'s content therefore starts at `bar(i) + 1 + PAD`, and the bar to its
    /// right is at `bar(i) + width + CELL_EXTRA`.
    pub fn bar(&self, i: usize) -> usize {
        self.cols
            .iter()
            .take(i)
            .map(|c| c.width + CELL_EXTRA)
            .sum::<usize>()
    }

    /// Total drawn width of the grid, both outer bars included.
    pub fn total(&self) -> usize {
        match self.cols.is_empty() {
            true => 0,
            false => {
                self.cols
                    .iter()
                    .map(|c| c.width + CELL_EXTRA)
                    .sum::<usize>()
                    + 1
            }
        }
    }

    /// The horizontal offset that brings column `i` into view from `hoff`,
    /// moving as little as possible: nothing when it already fits, its left bar
    /// at the left edge when it is off to the left (or wider than the
    /// viewport), and its right bar at the right edge when it is off to the
    /// right. Clamped so the grid's right edge is never scrolled past.
    pub fn scroll_to(&self, i: usize, hoff: usize, view: usize) -> usize {
        let view = view.max(1);
        let start = self.bar(i);
        let span = self.width_of(i) + CELL_EXTRA + 1;
        let max = self.total().saturating_sub(view);
        let off = if span > view || start < hoff {
            start
        } else if start + span > hoff + view {
            start + span - view
        } else {
            hoff
        };
        off.min(max)
    }

    /// The first column whose cell is visible at offset `hoff` — used to keep
    /// the column cursor somewhere the reader can see it.
    pub fn first_visible(&self, hoff: usize) -> usize {
        (0..self.cols.len())
            .find(|i| self.bar(*i) + self.width_of(*i) + CELL_EXTRA > hoff)
            .unwrap_or(0)
    }
}

/// Widest a single column may be laid out: the viewport less the pads and the
/// bars either side, and never zero.
fn cap_for(view: usize) -> usize {
    view.saturating_sub(CELL_EXTRA + 1).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn grid() -> Grid {
        let mut g = Grid::new(&strs(&["id", "name"]));
        g.sample(&strs(&["1", "alice"]));
        g.sample(&strs(&["22222", "bo"]));
        g.fit(200);
        g
    }

    #[test]
    fn widths_come_from_the_widest_sampled_value() {
        let g = grid();
        assert_eq!(g.width_of(0), 5);
        assert_eq!(g.width_of(1), 5);
        // "│ 22222 │ alice │" is 17 columns.
        assert_eq!(g.total(), 17);
        assert_eq!(g.bar(0), 0);
        assert_eq!(g.bar(1), 8);
    }

    #[test]
    fn header_alone_sizes_a_column_with_no_values() {
        let mut g = Grid::new(&strs(&["longheader"]));
        g.fit(200);
        assert_eq!(g.width_of(0), 10);
    }

    #[test]
    fn wide_chars_count_two_columns() {
        let mut g = Grid::new(&strs(&["k"]));
        g.sample(&strs(&["\u{4e2d}\u{6587}"]));
        g.fit(200);
        assert_eq!(g.width_of(0), 4);
        // A combining mark adds nothing.
        let mut g = Grid::new(&strs(&["k"]));
        g.sample(&strs(&["cafe\u{301}"]));
        g.fit(200);
        assert_eq!(g.width_of(0), 4);
    }

    #[test]
    fn a_huge_value_is_capped_rather_than_taking_the_screen() {
        let mut g = Grid::new(&strs(&["note"]));
        g.sample(&[String::from("x").repeat(5000)]);
        g.fit(200);
        assert_eq!(g.width_of(0), MAX_COL);
        // ... and by the viewport when that is the tighter bound.
        g.fit(20);
        assert_eq!(g.width_of(0), 16);
    }

    #[test]
    fn widening_survives_a_resize_but_not_the_viewport() {
        let mut g = grid();
        assert_eq!(g.set_fixed(1, 30, 200), Some(30));
        assert_eq!(g.width_of(1), 30);
        g.fit(200);
        assert_eq!(g.width_of(1), 30, "the override survives a re-fit");
        g.fit(20);
        assert_eq!(g.width_of(1), 16, "but is capped to a narrow viewport");
        g.fit(200);
        assert_eq!(g.width_of(1), 30);
    }

    #[test]
    fn scrolling_moves_the_minimum_to_reveal_a_column() {
        let g = grid();
        // Everything fits: no movement.
        assert_eq!(g.scroll_to(1, 0, 100), 0);
        // A 10-column viewport: column 1 spans 8..17, so the grid scrolls just
        // far enough to put its right bar at the right edge.
        assert_eq!(g.scroll_to(1, 0, 10), 7);
        assert_eq!(g.scroll_to(0, 8, 10), 0);
        // Never past the right edge.
        assert!(g.scroll_to(1, 0, 10) + 10 <= g.total());
    }

    #[test]
    fn a_column_wider_than_the_viewport_starts_at_its_left_edge() {
        let mut g = Grid::new(&strs(&["a", "b"]));
        g.sample(&["x".repeat(40), "y".repeat(40)]);
        g.fit(200);
        assert_eq!(g.scroll_to(1, 0, 10), g.bar(1));
    }

    #[test]
    fn empty_grids_answer_zero_rather_than_panicking() {
        let g = Grid::default();
        assert_eq!(g.total(), 0);
        assert_eq!(g.bar(0), 0);
        assert_eq!(g.width_of(9), 0);
        assert_eq!(g.scroll_to(3, 7, 0), 0);
        assert_eq!(g.first_visible(99), 0);
    }
}
