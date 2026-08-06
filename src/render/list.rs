//! Lists: depth-varying bullets, source-numbered ordered lists, task
//! checkboxes, and a hanging indent aligned to the text rather than the
//! bullet (SPEC.md §Blocks).
#![deny(unsafe_code)]

use super::block::{render_block, spaces, Ctx, Pfx};
use super::width::{pad_right, str_width};
use super::wrap::with_prefix;
use super::{LineKind, Span};
use crate::md::ast::{Block, ListItem, ListKind};
use crate::theme;

pub(crate) fn render(
    ctx: &mut Ctx,
    kind: ListKind,
    tight: bool,
    items: &[ListItem],
    src: usize,
    pfx: &Pfx,
    depth: usize,
) {
    let markers: Vec<String> = (0..items.len()).map(|i| marker(kind, i, depth)).collect();
    let col = markers.iter().map(|m| str_width(m)).max().unwrap_or(1) + 1;
    // Past the indent budget the marker still prints, but nothing indents any
    // further -- a two-hundred-deep list must stay readable, not become a
    // column of single characters.
    let indent = pfx.cont_width() + col <= ctx.indent_budget();
    for (i, item) in items.iter().enumerate() {
        if !tight && i > 0 {
            ctx.blank(item_line(item, src));
        }
        let mut first = pfx.first.clone();
        first.push(Span::new(pad_right(&markers[i], col), marker_style(kind)));
        let mut cont = pfx.cont.clone();
        if indent {
            cont.push(spaces(col));
        }
        let mut ip = Pfx { first, cont };
        if let Some(checked) = item.task {
            let box_span = Span::new(format!("{} ", theme::checkbox(checked)), check_style(checked));
            ip.first = with_prefix(&ip.first, vec![box_span]);
            if indent {
                ip.cont = with_prefix(&ip.cont, vec![spaces(2)]);
            }
        }
        item_blocks(ctx, item, &ip, depth, tight, src);
    }
}

fn item_line(item: &ListItem, fallback: usize) -> usize {
    item.blocks.first().map(Block::source_line).unwrap_or(fallback)
}

fn item_blocks(ctx: &mut Ctx, item: &ListItem, ip: &Pfx, depth: usize, tight: bool, src: usize) {
    if item.blocks.is_empty() {
        ctx.line(ip.first.clone(), LineKind::List, src);
        return;
    }
    let rest = Pfx::uniform(ip.cont.clone());
    let before = ctx.lines.len();
    for (n, b) in item.blocks.iter().enumerate() {
        let p = if n == 0 { ip } else { &rest };
        if n > 0 && !tight && !matches!(b, Block::List { .. }) {
            ctx.blank(b.source_line());
        }
        match b {
            Block::List { kind, tight: t, items, source_line } => {
                render(ctx, *kind, *t, items, *source_line, p, depth + 1)
            }
            _ => render_block(ctx, b, p),
        }
    }
    for line in ctx.lines[before..].iter_mut() {
        if line.kind == LineKind::Paragraph {
            line.kind = LineKind::List;
        }
    }
}

fn marker(kind: ListKind, index: usize, depth: usize) -> String {
    match kind {
        ListKind::Bullet => theme::bullet(depth).to_string(),
        ListKind::Ordered { start } => {
            format!("{}.", start.saturating_add(index as u64))
        }
    }
}

fn marker_style(kind: ListKind) -> crate::term::Style {
    match kind {
        ListKind::Bullet => theme::bullet_style(),
        ListKind::Ordered { .. } => theme::muted().bold(),
    }
}

fn check_style(checked: bool) -> crate::term::Style {
    if checked {
        theme::task_done()
    } else {
        theme::muted()
    }
}

#[cfg(test)]
mod tests {
    use crate::md::parse;
    use crate::render::{render_document, RenderOpts};

    fn lay(src: &str, width: usize) -> Vec<String> {
        render_document(&parse(src), &RenderOpts::new(width))
            .iter()
            .map(|l| l.text().trim_end().to_string())
            .collect()
    }

    #[test]
    fn bullets_vary_by_depth() {
        let out = lay("- a\n  - b\n    - c\n", 40);
        assert_eq!(out[0], "  \u{2022} a");
        assert_eq!(out[1], "    \u{25e6} b");
        assert_eq!(out[2], "      \u{25aa} c");
    }

    #[test]
    fn ordered_lists_keep_source_numbering() {
        let out = lay("5. five\n6. six\n", 40);
        assert_eq!(out[0], "  5. five");
        assert_eq!(out[1], "  6. six");
    }

    #[test]
    fn ordered_markers_align_across_widths() {
        let out = lay("9. nine\n10. ten\n", 40);
        assert_eq!(out[0], "  9.  nine");
        assert_eq!(out[1], "  10. ten");
    }

    #[test]
    fn task_boxes_render() {
        let out = lay("- [ ] todo\n- [x] done\n", 40);
        assert_eq!(out[0], "  \u{2022} \u{2610} todo");
        assert_eq!(out[1], "  \u{2022} \u{2611} done");
    }

    #[test]
    fn hanging_indent_aligns_to_the_text() {
        // gutter(2) + bullet(2) = text starts at column 4
        let out = lay("- aaaa bbbb cccc\n", 12);
        assert_eq!(out[0], "  \u{2022} aaaa");
        assert_eq!(out[1], "    bbbb");
        assert_eq!(out[2], "    cccc");
    }

    #[test]
    fn nesting_stops_indenting_at_the_budget_instead_of_squeezing_the_text() {
        // 200 levels at 2 columns each would push the text 400 columns right.
        let mut src = String::new();
        for i in 0..200 {
            src.push_str(&"  ".repeat(i));
            src.push_str(&format!("- level {i}\n"));
        }
        let out = lay(&src, 40);
        let widest = out.iter().map(|l| l.chars().count()).max().unwrap();
        assert!(widest <= 40, "a line grew to {widest} columns");
        // The deepest level is still legible: its marker and the word "level"
        // fit on one row rather than degenerating to one character per line.
        assert!(
            out.iter().any(|l| l.trim_start().ends_with("level")),
            "text was squeezed below a word: {:?}",
            &out[out.len() - 4..]
        );
        assert!(out.iter().any(|l| l.contains("199")), "deepest level lost");
        // Two rows per item at worst -- linear, not quadratic, in depth.
        assert!(out.len() <= 400, "{} rows for 200 items", out.len());
    }

    #[test]
    fn deep_nesting_keeps_indenting() {
        let src = "- a\n  - b\n    - c\n      - d\n        - e\n";
        let out = lay(src, 60);
        let indents: Vec<usize> = out
            .iter()
            .map(|l| l.len() - l.trim_start().len())
            .collect();
        assert_eq!(indents, vec![2, 4, 6, 8, 10]);
    }
}
