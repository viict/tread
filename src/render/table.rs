//! Tables — first class, because the corpus is table-heavy.
//!
//! Cells are rendered to styled spans first and measured with `str_width`, so
//! a cell full of inline markup sizes by what is displayed, not by its source.
//! When the drawn table is wider than the viewport its rows are marked
//! horizontally scrollable rather than being squeezed or wrapped.
#![deny(unsafe_code)]

use super::block::{Ctx, Pfx};
use super::inline::line_spans;
use super::width::repeat;
use super::wrap::with_prefix;
use super::{LineKind, Span};
use crate::md::ast::{Align, Inline};
use crate::term::Style;
use crate::theme;

/// One space of padding inside each cell, on both sides.
const CELL_PAD: usize = 1;

type Row = Vec<Vec<Span>>;

pub(crate) fn render(
    ctx: &mut Ctx,
    align: &[Align],
    head: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    src: usize,
    pfx: &Pfx,
) {
    let cols = align.len().max(head.len());
    if cols == 0 {
        return;
    }
    let head_row: Row = cells(head, cols, theme::table_head());
    let body: Vec<Row> = rows.iter().map(|r| cells(r, cols, Style::new())).collect();
    let widths = measure(&head_row, &body, cols);
    let total: usize = widths.iter().map(|w| w + CELL_PAD * 2 + 1).sum::<usize>() + 1;
    let avail = ctx.width.saturating_sub(pfx.first_width());
    let scroll = total > avail;
    let mut line = 0;
    emit(ctx, border(&widths, "\u{250c}", "\u{252c}", "\u{2510}"), src, pfx, scroll);
    emit(ctx, data_row(&head_row, &widths, align), src, pfx, scroll);
    emit(ctx, border(&widths, "\u{251c}", "\u{253c}", "\u{2524}"), src + 1, pfx, scroll);
    for r in &body {
        line += 1;
        emit(ctx, data_row(r, &widths, align), src + 1 + line, pfx, scroll);
    }
    emit(ctx, border(&widths, "\u{2514}", "\u{2534}", "\u{2518}"), src + 1 + line, pfx, scroll);
}

fn emit(ctx: &mut Ctx, spans: Vec<Span>, src: usize, pfx: &Pfx, scroll: bool) {
    ctx.emit(with_prefix(&pfx.first, spans), LineKind::Table, src, scroll, None);
}

/// Render every cell of a row, padding ragged rows out to `cols`.
fn cells(row: &[Vec<Inline>], cols: usize, base: Style) -> Row {
    let mut out: Row = row.iter().map(|c| line_spans(c, base)).collect();
    out.truncate(cols);
    while out.len() < cols {
        out.push(Vec::new());
    }
    out
}

fn cell_width(cell: &[Span]) -> usize {
    cell.iter().map(Span::width).sum()
}

fn measure(head: &Row, body: &[Row], cols: usize) -> Vec<usize> {
    let mut w = vec![1usize; cols];
    for row in std::iter::once(head).chain(body.iter()) {
        for (i, c) in row.iter().enumerate().take(cols) {
            w[i] = w[i].max(cell_width(c));
        }
    }
    w
}

fn border(widths: &[usize], left: &str, mid: &str, right: &str) -> Vec<Span> {
    let mut s = String::from(left);
    for (i, w) in widths.iter().enumerate() {
        if i > 0 {
            s.push_str(mid);
        }
        s.push_str(&repeat('\u{2500}', w + CELL_PAD * 2));
    }
    s.push_str(right);
    vec![Span::new(s, theme::table_border())]
}

fn data_row(row: &Row, widths: &[usize], align: &[Align]) -> Vec<Span> {
    let bar = || Span::new("\u{2502}", theme::table_border());
    let mut out = vec![bar()];
    for (i, w) in widths.iter().enumerate() {
        let empty: Vec<Span> = Vec::new();
        let cell = row.get(i).unwrap_or(&empty);
        let a = align.get(i).copied().unwrap_or(Align::None);
        let (lead, trail) = pads(*w - cell_width(cell).min(*w), a);
        out.push(Span::plain(repeat(' ', CELL_PAD + lead)));
        out.extend(cell.iter().cloned());
        out.push(Span::plain(repeat(' ', trail + CELL_PAD)));
        out.push(bar());
    }
    out
}

fn pads(slack: usize, align: Align) -> (usize, usize) {
    match align {
        Align::Right => (slack, 0),
        Align::Center => (slack / 2, slack - slack / 2),
        _ => (0, slack),
    }
}

#[cfg(test)]
mod tests {
    use crate::md::parse;
    use crate::render::{render_document, Line, RenderOpts};
    use crate::theme;

    fn lay(src: &str, width: usize) -> Vec<Line> {
        render_document(&parse(src), &RenderOpts::new(width))
    }

    fn texts(src: &str, width: usize) -> Vec<String> {
        lay(src, width).iter().map(|l| l.text()).collect()
    }

    const T: &str = "| Field | Meaning |\n| --- | --- |\n| `a` | one |\n| bb | two |\n";

    #[test]
    fn draws_a_box_with_sized_columns() {
        let out = texts(T, 60);
        assert_eq!(out[0], "  \u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}");
        assert_eq!(out[1], "  \u{2502} Field \u{2502} Meaning \u{2502}");
        assert_eq!(out[3], "  \u{2502} a     \u{2502} one     \u{2502}");
        assert_eq!(out[4], "  \u{2502} bb    \u{2502} two     \u{2502}");
        assert!(out[5].starts_with("  \u{2514}"));
    }

    #[test]
    fn styled_cells_measure_by_rendered_width() {
        // "`a`" is three source characters but one display column.
        let out = texts(T, 60);
        assert_eq!(out[3].chars().count(), out[4].chars().count());
        let rendered = lay(T, 60);
        let code = rendered[3]
            .spans
            .iter()
            .find(|s| s.text == "a")
            .expect("code span");
        assert_eq!(code.style.bg, Some(theme::CODE_SPAN_BG));
    }

    #[test]
    fn alignment_is_honoured() {
        let src = "| l | c | r |\n| :-- | :-: | --: |\n| 1 | 2 | 3 |\n";
        let out = texts(src, 60);
        assert_eq!(out[3], "  \u{2502} 1 \u{2502} 2 \u{2502} 3 \u{2502}");
        let src = "| left | center | right |\n| :-- | :-: | --: |\n| 1 | 2 | 3 |\n";
        let out = texts(src, 60);
        assert_eq!(out[3], "  \u{2502} 1    \u{2502}   2    \u{2502}     3 \u{2502}");
    }

    #[test]
    fn wide_tables_scroll_instead_of_mangling() {
        let src = "| a | b |\n| --- | --- |\n| this is a very long cell indeed | and another long one |\n";
        let out = lay(src, 30);
        assert!(out.iter().all(|l| l.scroll || l.is_blank()));
        let row = out.iter().find(|l| l.text().contains("very long")).unwrap();
        assert!(row.text().contains("this is a very long cell indeed"));
        assert!(row.width() > 30);
    }

    #[test]
    fn cjk_cells_size_by_display_width() {
        let src = "| k | v |\n| --- | --- |\n| \u{4e2d}\u{6587} | x |\n";
        let out = texts(src, 60);
        // "中文" is 4 columns wide, so the first column is 4 wide.
        assert_eq!(out[3], "  \u{2502} \u{4e2d}\u{6587} \u{2502} x \u{2502}");
        assert_eq!(out[1], "  \u{2502} k    \u{2502} v \u{2502}");
    }

    #[test]
    fn ragged_rows_are_padded() {
        let src = "| a | b |\n| --- | --- |\n| 1 |\n";
        let out = texts(src, 60);
        assert_eq!(out[3], "  \u{2502} 1 \u{2502}   \u{2502}");
    }

    #[test]
    fn links_in_cells_keep_their_url() {
        let src = "| doc |\n| --- |\n| [DNS](models/DNS.md) |\n";
        let row = lay(src, 60)
            .into_iter()
            .find(|l| l.text().contains("DNS"))
            .unwrap();
        assert_eq!(row.link_at(4), Some("models/DNS.md"));
    }
}
