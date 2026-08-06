//! Drawing one CSV row as one [`Line`], in the same box-drawing style a
//! markdown table uses (`src/render/table.rs`).
//!
//! A reader must not feel like the program changed when the file did, so the
//! glyphs, the one-space cell padding and the header style are the markdown
//! table's, taken from the same [`theme`]. What is different is what the sizes
//! come from: a markdown table measures every row because it has them all,
//! whereas here the widths were sampled ([`super::grid`]) and a later value can
//! overflow. Such a value is truncated with a visible `\u{2026}` rather than
//! being allowed to push the grid out of alignment — the pinned header and the
//! body are the same [`Grid`], so any drift here would show up instantly as a
//! header sitting over the wrong column.
#![deny(unsafe_code)]

use super::grid::{Grid, PAD};
use crate::render::{repeat, str_width, take_width, Line, LineKind, Span};
use crate::term::Style;
use crate::theme;

/// Shown in place of the tail of a value too wide for its column.
pub const ELLIPSIS: char = '\u{2026}';

/// Control bytes are data in a CSV cell (a quoted field may hold a newline) but
/// can never be sent to a terminal, so they are drawn as one visible dot each.
pub const CONTROL: char = '\u{b7}';

const BAR: &str = "\u{2502}";
const DASH: char = '\u{2500}';

/// Which border to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Mid,
    Bottom,
}

impl Edge {
    fn glyphs(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Edge::Top => ("\u{250c}", "\u{252c}", "\u{2510}"),
            Edge::Mid => ("\u{251c}", "\u{253c}", "\u{2524}"),
            Edge::Bottom => ("\u{2514}", "\u{2534}", "\u{2518}"),
        }
    }
}

/// One cell's text, with control characters made visible.
///
/// Returns the input unchanged (and unallocated in the common case is not
/// worth the API noise) when there is nothing to replace.
pub fn clean(raw: &str) -> String {
    match raw.chars().any(char::is_control) {
        false => raw.to_string(),
        true => raw
            .chars()
            .map(|c| if c.is_control() { CONTROL } else { c })
            .collect(),
    }
}

/// A horizontal rule across the whole grid.
pub fn border(grid: &Grid, edge: Edge, source_line: usize) -> Line {
    let (left, mid, right) = edge.glyphs();
    let mut s = String::from(left);
    for (i, col) in grid.cols.iter().enumerate() {
        if i > 0 {
            s.push_str(mid);
        }
        s.push_str(&repeat(DASH, col.width + PAD * 2));
    }
    s.push_str(right);
    line(vec![Span::new(s, theme::table_border())], source_line)
}

/// The header row, in the markdown table's header style.
pub fn header(grid: &Grid) -> Line {
    let names: Vec<String> = grid.cols.iter().map(|c| c.name.clone()).collect();
    row(grid, &names, theme::table_head(), 1)
}

/// One data row. `fields` is padded or truncated to the grid's arity by the
/// caller (`parse::fit`); anything beyond it is not drawn, though a yank still
/// carries it.
pub fn data(grid: &Grid, fields: &[String], source_line: usize) -> Line {
    row(grid, fields, Style::new(), source_line)
}

/// One row of cells.
///
/// A cell and its two pad columns are one span, not three: a file with 10k
/// columns is a row with 10k spans rather than 40k, and the spans a frame
/// allocates is what a wide CSV's paint time is made of. Nothing is lost —
/// the pads carry no background in either style used here.
fn row(grid: &Grid, fields: &[String], base: Style, source_line: usize) -> Line {
    let bar = || Span::new(BAR, theme::table_border());
    // A row with more fields than the header named keeps them — they are simply
    // past the right edge of a header-shaped grid. Say so where the eye already
    // is, by standing the marker in for the left border rather than adding a
    // column that would misalign this row against every other one.
    let lead = match fields.len() > grid.cols.len() {
        true => Span::new(theme::MARKER_MORE.to_string(), theme::more()),
        false => bar(),
    };
    let mut spans = vec![lead];
    for (i, col) in grid.cols.iter().enumerate() {
        let text = clean(fields.get(i).map_or("", String::as_str));
        cell(&mut spans, &text, col.width, base);
        spans.push(bar());
    }
    line(spans, source_line)
}

/// Append a cell: the value padded to `width` between its two pad columns, or
/// clipped to `width - 1` with a marker so the overflow is visible and the grid
/// still lines up.
fn cell(out: &mut Vec<Span>, text: &str, width: usize, base: Style) {
    let pad = repeat(' ', PAD);
    let w = str_width(text);
    if w <= width {
        let body = format!("{pad}{text}{}{pad}", repeat(' ', width - w));
        out.push(Span::new(body, base));
        return;
    }
    let (kept, used) = take_width(text, width.saturating_sub(1));
    // A wide character straddling the cut leaves a spare column: pad it, or the
    // marker would sit one column early and every bar after it would shift.
    let slack = width.saturating_sub(used + 1);
    out.push(Span::new(format!("{pad}{kept}{}", repeat(' ', slack)), base));
    out.push(Span::new(format!("{ELLIPSIS}{pad}"), theme::muted()));
}

fn line(spans: Vec<Span>, source_line: usize) -> Line {
    Line {
        spans,
        block: 0,
        source_line,
        heading: None,
        // Always horizontally scrollable: a grid is never wrapped, and marking
        // it so is what makes the pinned header and the body share one offset.
        scroll: true,
        kind: LineKind::Table,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn g() -> Grid {
        let mut g = Grid::new(&strs(&["id", "name"]));
        g.sample(&strs(&["1", "alice"]));
        g.fit(100);
        g
    }

    #[test]
    fn a_row_looks_like_a_markdown_table_row() {
        let g = g();
        assert_eq!(header(&g).text(), "\u{2502} id \u{2502} name  \u{2502}");
        assert_eq!(
            data(&g, &strs(&["7", "bo"]), 2).text(),
            "\u{2502} 7  \u{2502} bo    \u{2502}"
        );
        assert_eq!(border(&g, Edge::Top, 0).text(), "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}");
        assert!(border(&g, Edge::Bottom, 0).text().starts_with("\u{2514}"));
    }

    #[test]
    fn every_row_is_exactly_the_grid_width() {
        let g = g();
        let rows = [
            header(&g),
            border(&g, Edge::Mid, 0),
            data(&g, &strs(&["7", "bo"]), 2),
            data(&g, &strs(&[]), 3),
            data(&g, &strs(&["overlong", "overlong too"]), 4),
            data(&g, &strs(&["\u{4e2d}\u{6587}\u{4e2d}", "\u{4e2d}"]), 5),
        ];
        for r in rows {
            assert_eq!(r.width(), g.total(), "{:?}", r.text());
        }
    }

    #[test]
    fn an_overlong_value_is_marked_not_silently_cut() {
        let g = g();
        let text = data(&g, &strs(&["1", "alexandra"]), 2).text();
        assert!(text.contains(&format!("alex{ELLIPSIS}")), "{text}");
        assert_eq!(str_width(&text), g.total());
    }

    #[test]
    fn a_wide_char_straddling_the_cut_keeps_the_grid() {
        let mut g = Grid::new(&strs(&["k"]));
        g.sample(&strs(&["abcd"]));
        g.fit(100);
        // Cutting "中中" to 3 columns keeps one wide char and pads the gap.
        let l = data(&g, &strs(&["\u{4e2d}\u{4e2d}\u{4e2d}"]), 2);
        assert_eq!(l.width(), g.total());
        assert!(l.text().ends_with(&format!("{ELLIPSIS} \u{2502}")));
    }

    #[test]
    fn control_characters_are_shown_not_emitted() {
        assert_eq!(clean("a\nb\tc\0"), "a\u{b7}b\u{b7}c\u{b7}");
        assert_eq!(clean("plain"), "plain");
        let g = g();
        let l = data(&g, &strs(&["1", "a\nb"]), 2);
        assert!(!l.text().contains('\n'));
    }

    #[test]
    fn ragged_rows_do_not_break_the_grid() {
        let g = g();
        assert_eq!(data(&g, &strs(&["only"]), 2).width(), g.total());
        assert_eq!(data(&g, &strs(&["a", "b", "c", "d"]), 2).width(), g.total());
    }

    #[test]
    fn a_row_with_more_fields_than_the_header_is_marked() {
        let g = g();
        let over = data(&g, &strs(&["a", "b", "c", "d"]), 2);
        assert_eq!(over.spans[0].text, theme::MARKER_MORE.to_string());
        // The marker stands in for the border, so the grid does not shift.
        assert_eq!(over.width(), g.total());

        let exact = data(&g, &strs(&["a", "b"]), 2);
        assert_eq!(exact.spans[0].text, BAR);
        let short = data(&g, &strs(&["a"]), 2);
        assert_eq!(short.spans[0].text, BAR, "a short row lost nothing");
    }
}
