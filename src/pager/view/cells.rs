//! A row as a grid of display cells, so the pager can restyle or overwrite
//! individual columns (search highlight, cursor tint, cut indicators) without
//! the layout engine knowing about any of it.
#![deny(unsafe_code)]

use crate::render::{char_width, Span};
use crate::term::Style;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cell {
    ch: char,
    /// Display width: 2 for wide CJK, 0 for combining marks.
    w: usize,
    style: Style,
    link: Option<String>,
}

#[derive(Debug, Default)]
pub struct Cells {
    cells: Vec<Cell>,
}

impl Cells {
    pub fn from_spans(spans: &[Span]) -> Cells {
        let mut cells = Vec::new();
        for s in spans {
            for ch in s.text.chars() {
                cells.push(Cell {
                    ch,
                    w: char_width(ch),
                    style: s.style,
                    link: s.link.clone(),
                });
            }
        }
        Cells { cells }
    }

    /// Total display width. Test-only: the painter works in viewport columns.
    #[cfg(test)]
    pub fn width(&self) -> usize {
        self.cells.iter().map(|c| c.w).sum()
    }

    /// Test-only.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Apply `style` to every cell overlapping the column range `[start, end)`.
    pub fn restyle(&mut self, start: usize, end: usize, style: Style) {
        if end <= start {
            return;
        }
        let mut col = 0;
        for c in self.cells.iter_mut() {
            let next = col + c.w;
            let overlaps = col < end && next.max(col + 1) > start;
            if overlaps {
                c.style = style;
            }
            col = next;
        }
    }

    /// Blend a background over the whole row, keeping each cell's foreground.
    pub fn tint(&mut self, style: Style) {
        for c in self.cells.iter_mut() {
            if let Some(bg) = style.bg {
                if c.style.bg.is_none() {
                    c.style = c.style.bg(bg);
                }
            }
        }
    }

    /// Overwrite the single column `col` with `ch`. A wide cell being replaced
    /// keeps the grid intact by leaving a space behind.
    pub fn set(&mut self, col: usize, ch: char, style: Style) {
        let mut x = 0;
        for i in 0..self.cells.len() {
            let w = self.cells[i].w;
            if col < x + w.max(1) {
                let wide = w == 2;
                self.cells[i] = Cell { ch, w: 1, style, link: None };
                if wide {
                    self.cells.insert(
                        i + 1,
                        Cell { ch: ' ', w: 1, style, link: None },
                    );
                }
                return;
            }
            x += w;
        }
        // Past the end of the row: pad out to `col` and append.
        while x < col {
            self.cells.push(Cell { ch: ' ', w: 1, style, link: None });
            x += 1;
        }
        self.cells.push(Cell { ch, w: 1, style, link: None });
    }

    /// Merge adjacent cells that share a style and link back into spans.
    pub fn into_spans(self) -> Vec<Span> {
        let mut out: Vec<Span> = Vec::new();
        for c in self.cells {
            match out.last_mut() {
                Some(last) if last.style == c.style && last.link == c.link => last.text.push(c.ch),
                _ => out.push(Span {
                    text: c.ch.to_string(),
                    style: c.style,
                    link: c.link,
                }),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<Span> {
        vec![Span::plain(text)]
    }

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn round_trips_text() {
        let c = Cells::from_spans(&spans("hello"));
        assert_eq!(c.width(), 5);
        assert_eq!(text_of(&c.into_spans()), "hello");
    }

    #[test]
    fn restyle_splits_a_span_at_column_boundaries() {
        let mut c = Cells::from_spans(&spans("abcdef"));
        c.restyle(2, 4, Style::new().bold());
        let out = c.into_spans();
        assert_eq!(text_of(&out), "abcdef");
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].text, "cd");
        assert!(out[1].style.has(crate::term::BOLD));
        assert!(!out[0].style.has(crate::term::BOLD));
    }

    #[test]
    fn restyle_covers_wide_characters_it_overlaps() {
        let mut c = Cells::from_spans(&spans("\u{4e2d}x"));
        c.restyle(0, 1, Style::new().bold());
        let out = c.into_spans();
        assert_eq!(out[0].text, "\u{4e2d}");
        assert!(out[0].style.has(crate::term::BOLD));
        assert!(!out[1].style.has(crate::term::BOLD));
    }

    #[test]
    fn restyle_ignores_empty_and_out_of_range_windows() {
        let mut c = Cells::from_spans(&spans("abc"));
        c.restyle(2, 2, Style::new().bold());
        c.restyle(9, 12, Style::new().bold());
        assert!(c.into_spans().iter().all(|s| s.style.is_default()));
    }

    #[test]
    fn tint_only_fills_cells_without_a_background() {
        let mut c = Cells::from_spans(&[
            Span::plain("a"),
            Span::new("b", Style::new().bg(5)),
        ]);
        c.tint(Style::new().bg(238));
        let out = c.into_spans();
        assert_eq!(out[0].style.bg, Some(238));
        assert_eq!(out[1].style.bg, Some(5));
    }

    #[test]
    fn set_overwrites_one_column() {
        let mut c = Cells::from_spans(&spans("abc"));
        c.set(0, '<', Style::new());
        c.set(2, '>', Style::new());
        assert_eq!(text_of(&c.into_spans()), "<b>");
    }

    #[test]
    fn set_keeps_the_grid_when_it_replaces_a_wide_cell() {
        let mut c = Cells::from_spans(&spans("\u{4e2d}b"));
        c.set(0, '<', Style::new());
        let out = c.into_spans();
        assert_eq!(text_of(&out), "< b");
        assert_eq!(Cells::from_spans(&out).width(), 3);
    }

    #[test]
    fn set_past_the_end_pads_with_spaces() {
        let mut c = Cells::from_spans(&spans("ab"));
        c.set(4, '>', Style::new());
        assert_eq!(c.width(), 5);
        assert_eq!(text_of(&c.into_spans()), "ab  >");
    }

    #[test]
    fn emptiness_is_reported() {
        assert!(Cells::from_spans(&[]).is_empty());
        assert!(!Cells::from_spans(&spans("x")).is_empty());
    }
}
