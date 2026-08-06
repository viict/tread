//! Fenced/indented code blocks: background-tinted, language label in the
//! corner, never wrapped — long rows are marked horizontally scrollable and
//! keep their full text so the pager can offset them.
#![deny(unsafe_code)]

use super::block::{Ctx, Pfx};
use super::width::{pad_right, str_width};
use super::wrap::with_prefix;
use super::{LineKind, Span};
use crate::term::Style;
use crate::theme;

const TAB: &str = "    ";
/// One column of padding inside the tint on each side.
const PAD: usize = 1;

pub(crate) fn render(
    ctx: &mut Ctx,
    lang: Option<&str>,
    lines: &[String],
    src: usize,
    pfx: &Pfx,
) {
    let avail = ctx.width.saturating_sub(pfx.first_width()).max(4);
    let body: Vec<String> = lines.iter().map(|l| l.replace('\t', TAB)).collect();
    let natural = body.iter().map(|l| str_width(l)).max().unwrap_or(0) + PAD * 2;
    let label_w = lang.map(|l| str_width(l) + PAD * 2).unwrap_or(0);
    let want = natural.max(label_w);
    let scroll = want > avail;
    let box_w = if scroll { want } else { avail.min(want.max(4)) };
    if let Some(l) = lang {
        label(ctx, l, box_w, src, pfx, scroll);
    }
    for (i, text) in body.iter().enumerate() {
        let mut spans = vec![Span::new(" ", theme::code())];
        spans.extend(highlight(ctx, lang, text));
        let used = PAD + str_width(text);
        if box_w > used {
            spans.push(Span::new(pad_right("", box_w - used), theme::code()));
        }
        ctx.emit(with_prefix(&pfx.first, spans), LineKind::Code, src + 1 + i, scroll, None);
    }
}

/// The language label row, right-aligned in the tinted box.
fn label(ctx: &mut Ctx, lang: &str, box_w: usize, src: usize, pfx: &Pfx, scroll: bool) {
    let text = lang.to_string();
    let lead = box_w.saturating_sub(str_width(&text) + PAD);
    let spans = vec![
        Span::new(pad_right("", lead), theme::code()),
        Span::new(text, theme::code_label()),
        Span::new(" ", theme::code()),
    ];
    ctx.emit(with_prefix(&pfx.first, spans), LineKind::Code, src, scroll, None);
}

/// Apply the highlighter seam. With the v1 `NoHighlight` this returns the
/// whole line as one tinted span.
fn highlight(ctx: &Ctx, lang: Option<&str>, text: &str) -> Vec<Span> {
    let base = theme::code();
    let mut ranges = ctx.opts.highlighter.spans(lang, text);
    ranges.retain(|(a, b, _)| a < b && *b <= text.len() && text.is_char_boundary(*a) && text.is_char_boundary(*b));
    ranges.sort_by_key(|(a, _, _)| *a);
    if ranges.is_empty() {
        return vec![Span::new(text.to_string(), base)];
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    for (a, b, st) in ranges {
        if a < at {
            continue;
        }
        if a > at {
            out.push(Span::new(text[at..a].to_string(), base));
        }
        out.push(Span::new(text[a..b].to_string(), merge(base, st)));
        at = b;
    }
    if at < text.len() {
        out.push(Span::new(text[at..].to_string(), base));
    }
    out
}

/// A highlighter may set the foreground and attributes; the tint stays.
fn merge(base: Style, over: Style) -> Style {
    Style {
        fg: over.fg.or(base.fg),
        bg: base.bg,
        attrs: base.attrs | over.attrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md::parse;
    use crate::render::{render_document, Highlighter, RenderOpts};

    fn lines_of(src: &str, width: usize) -> Vec<String> {
        let doc = parse(src);
        render_document(&doc, &RenderOpts::new(width))
            .iter()
            .map(|l| l.text())
            .collect()
    }

    #[test]
    fn code_is_not_wrapped_and_is_marked_scrollable() {
        let long = "x".repeat(60);
        let src = format!("```\n{}\n```\n", long);
        let doc = parse(&src);
        let out = render_document(&doc, &RenderOpts::new(30));
        let code: Vec<_> = out.iter().filter(|l| l.kind == LineKind::Code).collect();
        assert_eq!(code.len(), 1);
        assert!(code[0].text().contains(&long));
        assert!(code[0].scroll);
    }

    #[test]
    fn short_code_is_padded_not_scrollable() {
        let out = lines_of("```\nhi\n```\n", 40);
        let row = out.iter().find(|l| l.contains("hi")).expect("code row");
        assert_eq!(row.trim_end(), "   hi");
        let doc = parse("```\nhi\n```\n");
        let rendered = render_document(&doc, &RenderOpts::new(40));
        assert!(rendered.iter().all(|l| !l.scroll));
    }

    #[test]
    fn language_label_sits_in_the_corner() {
        let out = lines_of("```rust\nfn a() {}\n```\n", 40);
        let label = out.iter().find(|l| l.contains("rust")).expect("label row");
        assert!(label.trim_end().ends_with("rust"));
    }

    #[test]
    fn tabs_expand_to_spaces() {
        let out = lines_of("```\n\tx\n```\n", 40);
        assert!(out.iter().any(|l| l.contains("     x")));
    }

    struct FirstWord;
    impl Highlighter for FirstWord {
        fn spans(&self, _lang: Option<&str>, line: &str) -> Vec<(usize, usize, Style)> {
            let end = line.find(' ').unwrap_or(line.len());
            vec![(0, end, Style::new().fg(200))]
        }
    }

    #[test]
    fn the_highlighter_seam_is_wired() {
        let doc = parse("```rust\nlet x = 1\n```\n");
        let h = FirstWord;
        let opts = RenderOpts::new(40).with_highlighter(&h);
        let out = render_document(&doc, &opts);
        let row = out.iter().find(|l| l.text().contains("let")).unwrap();
        let kw = row.spans.iter().find(|s| s.text == "let").expect("styled keyword");
        assert_eq!(kw.style.fg, Some(200));
        // the tint is preserved underneath
        assert_eq!(kw.style.bg, Some(theme::CODE_BG));
    }
}
