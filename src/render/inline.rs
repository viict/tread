//! Inline rendering: `Vec<Inline>` -> wrappable atoms of styled text.
//!
//! Style composition is by value: nesting `**_x_**` yields one span carrying
//! both BOLD and ITALIC. Links carry their URL on every span so the pager can
//! emit OSC 8 and show the target in the status bar.
#![deny(unsafe_code)]

use super::Span;
use crate::md::ast::Inline;
use crate::term::Style;
use crate::theme;

/// The unit the wrapper packs: a word, a collapsible space, or a hard break.
/// A space carries the link of the run it sits in, so a multi-word link stays
/// one continuous hyperlink instead of one per word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Atom {
    Word(Span),
    Space(Style, Option<String>),
    Break,
}

/// Flatten inline content into atoms, starting from `base` style.
pub(crate) fn flatten(items: &[Inline], base: Style) -> Vec<Atom> {
    let mut out = Vec::new();
    walk(items, base, None, &mut out);
    out
}

/// Flatten to a single line of spans (tables, status bar, outline entries):
/// spaces and breaks collapse to one space, nothing wraps.
pub(crate) fn line_spans(items: &[Inline], base: Style) -> Vec<Span> {
    let atoms = flatten(items, base);
    let mut out: Vec<Span> = Vec::new();
    for a in atoms {
        let (space, url) = match a {
            Atom::Word(s) => {
                push_span(&mut out, s);
                continue;
            }
            Atom::Space(st, u) => (space_style(st), u),
            Atom::Break => (Style::new(), None),
        };
        let trailing = out.last().map(|s| s.text.ends_with(' ')).unwrap_or(true);
        if !trailing {
            push_span(&mut out, Span { text: " ".into(), style: space, link: url });
        }
    }
    if let Some(last) = out.last_mut() {
        let trimmed = last.text.trim_end().to_string();
        last.text = trimmed;
    }
    while out.last().map(|s| s.text.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

/// Append, merging into the previous span when style and link match.
pub(crate) fn push_span(out: &mut Vec<Span>, s: Span) {
    if s.text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(p) if p.style == s.style && p.link == s.link => p.text.push_str(&s.text),
        _ => out.push(s),
    }
}

/// An interior space carries the style of the run it sits in, so a multi-word
/// link is underlined continuously and a strikethrough is not dotted. Only
/// interior spaces reach here: the wrapper drops a pending space at a line
/// break and `line_spans` trims the trailing one, so no styled whitespace ever
/// lands at the edge of a row or a table cell. The foreground is kept as well:
/// terminals draw the underline in the foreground colour, so dropping it would
/// give a multi-word link a two-tone underline (and would cost an extra SGR
/// transition per space).
pub(crate) fn space_style(st: Style) -> Style {
    st
}

fn link_style(st: Style) -> Style {
    st.fg(theme::LINK).underline()
}

/// A code span keeps the code background, but inside a link it keeps the link
/// colour: `[`x`](u)` is a link first (SPEC.md §Inline: "Links: blue").
fn code_style(st: Style, in_link: bool) -> Style {
    let fg = if in_link { theme::LINK } else { theme::CODE_SPAN_FG };
    Style { fg: Some(fg), bg: Some(theme::CODE_SPAN_BG), attrs: st.attrs }
}

fn walk(items: &[Inline], st: Style, link: Option<&str>, out: &mut Vec<Atom>) {
    for it in items {
        match it {
            Inline::Text(s) => push_text(s, st, link, out),
            Inline::Code(s) => push_text(s, code_style(st, link.is_some()), link, out),
            Inline::Emph(k) => walk(k, st.italic(), link, out),
            Inline::Strong(k) => walk(k, st.bold(), link, out),
            Inline::Strike(k) => walk(k, st.strike(), link, out),
            Inline::Link { text, url, .. } => {
                let ls = link_style(st);
                if text.is_empty() {
                    push_text(url, ls, Some(url), out);
                } else {
                    walk(text, ls, Some(url), out);
                }
            }
            Inline::Autolink(u) => push_text(u, link_style(st), Some(u), out),
            Inline::Image { alt, url } => {
                let label = if alt.is_empty() { "image" } else { alt.as_str() };
                push_text(&format!("[{}]", label), st.dim(), Some(url), out);
            }
            Inline::FootnoteRef(l) => {
                push_text(&format!("[^{}]", l), Style::new().fg(theme::MUTED_FG), link, out)
            }
            Inline::Html(s) => push_text(s, Style::new().fg(theme::MUTED_FG).dim(), link, out),
            Inline::SoftBreak => out.push(Atom::Space(st, link.map(str::to_string))),
            Inline::HardBreak => out.push(Atom::Break),
        }
    }
}

/// Split literal text into word / space atoms. Tabs count as spaces.
fn push_text(s: &str, st: Style, link: Option<&str>, out: &mut Vec<Atom>) {
    let mut word = String::new();
    for c in s.chars() {
        if c == ' ' || c == '\t' {
            flush(&mut word, st, link, out);
            if !matches!(out.last(), Some(Atom::Space(..))) {
                out.push(Atom::Space(st, link.map(str::to_string)));
            }
        } else if c == '\n' {
            flush(&mut word, st, link, out);
            out.push(Atom::Space(st, link.map(str::to_string)));
        } else {
            word.push(c);
        }
    }
    flush(&mut word, st, link, out);
}

fn flush(word: &mut String, st: Style, link: Option<&str>, out: &mut Vec<Atom>) {
    if word.is_empty() {
        return;
    }
    let text = std::mem::take(word);
    out.push(Atom::Word(match link {
        Some(u) => Span::linked(text, st, u),
        None => Span::new(text, st),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md::ast::Inline as I;
    use crate::term::{BOLD, ITALIC, STRIKE, UNDERLINE};

    fn t(s: &str) -> I {
        I::Text(s.into())
    }

    fn text_of(atoms: &[Atom]) -> String {
        atoms
            .iter()
            .map(|a| match a {
                Atom::Word(s) => s.text.clone(),
                Atom::Space(..) => " ".into(),
                Atom::Break => "\n".into(),
            })
            .collect()
    }

    #[test]
    fn words_and_spaces_split() {
        let a = flatten(&[t("one  two")], Style::new());
        assert_eq!(text_of(&a), "one two");
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn nested_styles_compose() {
        let a = flatten(&[I::Strong(vec![I::Emph(vec![t("x")])])], Style::new());
        let Atom::Word(s) = &a[0] else { panic!("word") };
        assert!(s.style.has(BOLD) && s.style.has(ITALIC));
    }

    #[test]
    fn strike_and_code_inside_a_link() {
        let a = flatten(
            &[I::Link {
                text: vec![I::Strike(vec![t("a")]), I::Code("b".into())],
                url: "u".into(),
                title: None,
            }],
            Style::new(),
        );
        let Atom::Word(s0) = &a[0] else { panic!() };
        assert!(s0.style.has(STRIKE) && s0.style.has(UNDERLINE));
        assert_eq!(s0.style.fg, Some(theme::LINK));
        assert_eq!(s0.link.as_deref(), Some("u"));
        let Atom::Word(s1) = &a[1] else { panic!() };
        assert_eq!(s1.style.bg, Some(theme::CODE_SPAN_BG));
        // Code inside a link stays link-coloured and underlined.
        assert_eq!(s1.style.fg, Some(theme::LINK));
        assert!(s1.style.has(UNDERLINE));
        assert_eq!(s1.link.as_deref(), Some("u"));
    }

    #[test]
    fn autolinks_and_empty_links_show_the_url() {
        let a = flatten(&[I::Autolink("https://x".into())], Style::new());
        assert_eq!(text_of(&a), "https://x");
        let b = flatten(&[I::Link { text: vec![], url: "u".into(), title: None }], Style::new());
        assert_eq!(text_of(&b), "u");
    }

    #[test]
    fn images_footnotes_and_html_are_muted_literals() {
        let a = flatten(
            &[
                I::Image { alt: "logo".into(), url: "p.png".into() },
                I::FootnoteRef("1".into()),
                I::Html("<br>".into()),
            ],
            Style::new(),
        );
        assert_eq!(text_of(&a), "[logo][^1]<br>");
    }

    #[test]
    fn spaces_inside_a_link_carry_the_url() {
        let a = flatten(
            &[I::Link { text: vec![t("two words")], url: "u".into(), title: None }],
            Style::new(),
        );
        assert!(matches!(&a[1], Atom::Space(_, Some(u)) if u == "u"));
    }

    #[test]
    fn interior_spaces_keep_the_run_style() {
        let sp = space_style(Style::new().fg(1).underline().bold());
        assert!(sp.has(UNDERLINE) && sp.has(BOLD));
        assert_eq!(sp.fg, Some(1));
    }

    #[test]
    fn code_outside_a_link_keeps_the_code_colour() {
        let a = flatten(&[I::Code("x".into())], Style::new());
        let Atom::Word(s) = &a[0] else { panic!() };
        assert_eq!(s.style.fg, Some(theme::CODE_SPAN_FG));
        assert_eq!(s.style.bg, Some(theme::CODE_SPAN_BG));
    }

    #[test]
    fn line_spans_collapse_breaks_and_trim() {
        let s = line_spans(&[t("a"), I::HardBreak, t("b"), I::SoftBreak], Style::new());
        let joined: String = s.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(joined, "a b");
    }

    #[test]
    fn line_spans_merge_equal_styles() {
        let s = line_spans(&[t("ab"), t("cd")], Style::new());
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "abcd");
    }
}
