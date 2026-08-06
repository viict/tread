//! Layout tests. Text assertions run on ANSI-stripped `Line::text()`; style
//! assertions look at spans separately, so palette tweaks never break layout
//! (SPEC.md §Testing).
#![deny(unsafe_code)]

use super::*;
use crate::md::parse;
use crate::term::{BOLD, UNDERLINE};
use crate::theme;

fn lines(src: &str, width: usize) -> Vec<Line> {
    render_document(&parse(src), &RenderOpts::new(width))
}

fn texts(src: &str, width: usize) -> Vec<String> {
    lines(src, width).iter().map(|l| l.text().trim_end().to_string()).collect()
}

// ---------------------------------------------------------------------------
// Headings
// ---------------------------------------------------------------------------

#[test]
fn h1_renders_as_a_banner_with_a_gutter_marker() {
    let out = lines("# Example\n\nbody\n", 80);
    assert_eq!(out.len(), theme::BANNER_ROWS + 2);
    let first = &out[0];
    assert!(first.text().starts_with("\u{25be} "));
    assert!(first.text().contains('\u{2588}'));
    let h = first.heading.as_ref().expect("heading metadata");
    assert_eq!((h.level, h.id.as_str()), (1, "example"));
    // only the first row carries the metadata
    assert!(out[1..theme::BANNER_ROWS].iter().all(|l| l.heading.is_none()));
    assert_eq!(out.last().unwrap().text(), "  body");
}

#[test]
fn h1_falls_back_to_bold_uppercase_with_a_rule() {
    // A colon-free but unmappable character forces the fallback.
    let out = texts("# Caf\u{e9} notes\n", 80);
    assert_eq!(out[0], "\u{25be} CAF\u{c9} NOTES");
    assert!(out[1].trim_start().starts_with('\u{2501}'));
    let styled = lines("# Caf\u{e9} notes\n", 80);
    assert!(styled[0].spans.iter().any(|s| s.style.has(BOLD)));
}

#[test]
fn h1_falls_back_when_the_banner_does_not_fit() {
    let out = texts("# Example Codex\n", 20);
    assert_eq!(out[0], "\u{25be} EXAMPLE CODEX");
}

#[test]
fn h2_gets_a_full_width_rule() {
    let out = texts("## Section\n", 20);
    assert_eq!(out[0], "\u{25be} Section");
    assert_eq!(out[1].trim_start().chars().count(), 18);
    assert!(out[1].trim_start().chars().all(|c| c == '\u{2500}'));
}

#[test]
fn h3_to_h6_indent_by_two() {
    let src = "### three\n\n#### four\n\n##### five\n\n###### six\n";
    let out = texts(src, 40);
    let bodies: Vec<&String> = out.iter().filter(|l| !l.is_empty()).collect();
    // gutter marker, then the per-level indent from the theme (2/4/6/8)
    assert_eq!(bodies[0], "\u{25be}   three");
    assert_eq!(bodies[1], "\u{25be}     four");
    assert_eq!(bodies[2], "\u{25be}       five");
    assert_eq!(bodies[3], "\u{25be}         six");
}

#[test]
fn heading_styles_come_from_the_theme() {
    let out = lines("### three\n", 40);
    let s = out[0].spans.iter().find(|s| s.text == "three").unwrap();
    assert_eq!(s.style, theme::heading(3));
}

/// The renderer has no collapse path of its own: it always emits every block
/// with the open marker, and `pager::collapse` hides rows afterwards. Two
/// implementations used to exist and disagreed about quote-nested headings.
#[test]
fn the_renderer_never_hides_a_section_and_always_marks_it_open() {
    let src = "## A\n\nbody\n\n### deeper\n\ntext\n\n## B\n\nlast\n";
    let out = render_document(&parse(src), &RenderOpts::new(40));
    let joined: Vec<String> = out.iter().map(|l| l.text().trim_end().to_string()).collect();
    for needle in ["\u{25be} A", "\u{25be} B"] {
        assert!(joined.iter().any(|l| l == needle), "missing {needle}");
    }
    for needle in ["body", "deeper", "text", "last"] {
        assert!(joined.iter().any(|l| l.contains(needle)), "missing {needle}");
    }
    assert!(
        !joined.iter().any(|l| l.contains('\u{25b8}')),
        "the closed marker is the pager's to paint"
    );
}

/// A heading inside a block quote still carries `heading` metadata, so the
/// pager can fold it. The deleted block-level `skip_section` only walked
/// `doc.blocks` and could never see one.
#[test]
fn a_heading_inside_a_quote_is_still_a_foldable_heading() {
    let out = render_document(&parse("> ## Quoted\n>\n> body\n"), &RenderOpts::new(40));
    let head = out
        .iter()
        .find_map(|l| l.heading.as_ref())
        .expect("quoted heading metadata");
    assert_eq!((head.level, head.text.as_str()), (2, "Quoted"));
}

// ---------------------------------------------------------------------------
// Paragraphs, quotes, rules, html, footnotes
// ---------------------------------------------------------------------------

#[test]
fn paragraphs_wrap_to_the_content_width() {
    let out = texts("alpha beta gamma delta\n", 12);
    assert_eq!(out, vec!["  alpha beta", "  gamma", "  delta"]);
    for l in texts("alpha beta gamma delta\n", 12) {
        assert!(str_width(&l) <= 12);
    }
}

#[test]
fn blocks_are_separated_by_one_blank_line() {
    let out = texts("one\n\ntwo\n\n\nthree\n", 40);
    assert_eq!(out, vec!["  one", "", "  two", "", "  three"]);
}

#[test]
fn quotes_get_a_recursive_bar() {
    let out = texts("> outer\n>\n> > inner\n", 40);
    assert_eq!(out[0], "  \u{258f} outer");
    assert!(out.last().unwrap().starts_with("  \u{258f} \u{258f} inner"));
}

#[test]
fn quote_bars_are_dim_and_text_is_muted() {
    let out = lines("> quoted\n", 40);
    let bar = out[0].spans.iter().find(|s| s.text.starts_with('\u{258f}')).unwrap();
    assert_eq!(bar.style, theme::quote_bar());
    let body = out[0].spans.iter().find(|s| s.text == "quoted").unwrap();
    assert_eq!(body.style.fg, Some(theme::QUOTE_FG));
    assert_eq!(out[0].kind, LineKind::Quote);
}

#[test]
fn quote_bars_stop_at_the_indent_budget() {
    // 500 bars at 2 columns each would leave no room for the word at all.
    let src = format!("{} deep enough to wrap\n", ">".repeat(500));
    let out = texts(&src, 40);
    let widest = out.iter().map(|l| l.chars().count()).max().unwrap();
    assert!(widest <= 40, "a quote line grew to {widest} columns");
    assert!(
        out.iter().any(|l| l.trim_start().ends_with("deep")),
        "quote text was squeezed below a word: {out:?}"
    );
    assert!(out.len() <= 8, "{} rows for one short quote", out.len());
}

#[test]
fn thematic_break_spans_the_content_width() {
    let out = texts("a\n\n---\n\nb\n", 20);
    let rule = out.iter().find(|l| l.contains('\u{2500}')).unwrap();
    assert_eq!(rule.trim_start().chars().count(), 18);
}

#[test]
fn html_blocks_are_dim_literals() {
    let out = lines("<div class=\"x\">\n", 40);
    assert_eq!(out[0].kind, LineKind::Html);
    assert_eq!(out[0].text().trim(), "<div class=\"x\">");
}

// ---------------------------------------------------------------------------
// Inline
// ---------------------------------------------------------------------------

#[test]
fn links_are_blue_underlined_and_carry_the_url() {
    let out = lines("see [docs](models/DNS.md) now\n", 60);
    let s = out[0].spans.iter().find(|s| s.text == "docs").unwrap();
    assert_eq!(s.style.fg, Some(theme::LINK));
    assert!(s.style.has(UNDERLINE));
    assert_eq!(s.link.as_deref(), Some("models/DNS.md"));
    assert_eq!(out[0].links(), vec![(6, "models/DNS.md")]);
    // the URL is hidden in the body
    assert_eq!(out[0].text().trim(), "see docs now");
}

#[test]
fn code_spans_are_tinted_and_do_not_bleed_across_a_wrap() {
    let out = lines("`alpha beta` tail\n", 9);
    for line in &out {
        let last = line.spans.last().unwrap();
        assert!(!last.text.ends_with(' ') || last.style.bg.is_none());
    }
    let tinted: Vec<&Span> = out
        .iter()
        .flat_map(|l| l.spans.iter())
        .filter(|s| s.style.bg == Some(theme::CODE_SPAN_BG))
        .collect();
    assert_eq!(
        tinted.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
        vec!["alpha", "beta"]
    );
}

#[test]
fn nested_emphasis_composes_into_one_span() {
    let out = lines("**bold _and italic_**\n", 60);
    let s = out[0].spans.iter().find(|s| s.text.contains("italic")).unwrap();
    assert!(s.style.has(BOLD) && s.style.has(crate::term::ITALIC));
}

#[test]
fn escaped_markers_stay_literal() {
    assert_eq!(texts("a \\*b\\* c\n", 40)[0], "  a *b* c");
}

// ---------------------------------------------------------------------------
// Line metadata and slicing
// ---------------------------------------------------------------------------

#[test]
fn lines_carry_their_block_index_and_source_line() {
    let out = lines("# T\n\npara\n\n## S\n", 80);
    let para = out.iter().find(|l| l.text().contains("para")).unwrap();
    assert_eq!(para.block, 1);
    assert_eq!(para.source_line, 3);
    let s = out.iter().find(|l| l.text().contains('S')).unwrap();
    assert_eq!(s.block, 2);
    assert_eq!(s.source_line, 5);
}

#[test]
fn slicing_clips_by_display_columns() {
    let spans = vec![Span::plain("abcdef")];
    let got = slice_spans(&spans, 2, 3);
    assert_eq!(got[0].text, "cde");
    assert!(slice_spans(&spans, 10, 3).is_empty());
}

#[test]
fn slicing_replaces_a_straddling_wide_char_with_a_space() {
    let spans = vec![Span::plain("a\u{4e2d}b")];
    let got: String = slice_spans(&spans, 2, 2).iter().map(|s| s.text.as_str()).collect();
    assert_eq!(got, " b");
    let got: String = slice_spans(&spans, 0, 2).iter().map(|s| s.text.as_str()).collect();
    assert_eq!(got, "a ");
}

#[test]
fn slicing_preserves_style_and_link() {
    let spans = vec![Span::linked("docs", theme::heading(2), "u")];
    let got = slice_spans(&spans, 1, 2);
    assert_eq!(got[0].text, "oc");
    assert_eq!(got[0].link.as_deref(), Some("u"));
    assert_eq!(got[0].style, theme::heading(2));
}

#[test]
fn no_line_exceeds_the_width_except_declared_scrollers() {
    let src = "# Example\n\nsome fairly long prose that has to wrap a few times over\n\n\
               | a | b |\n| --- | --- |\n| 1 | 2 |\n\n```sh\nshort\n```\n\n- item one\n- item two\n";
    for l in lines(src, 40) {
        assert!(l.scroll || l.width() <= 40, "overflow: {:?}", l.text());
    }
}

#[test]
fn a_tiny_width_does_not_panic() {
    for w in 1..8 {
        let _ = lines("# Hi\n\ntext here\n\n| a | b |\n| - | - |\n| 1 | 2 |\n", w);
    }
}

#[test]
fn plain_text_round_trips_without_escape_sequences() {
    for l in lines("# Hi\n\n`code` and [link](x)\n", 40) {
        assert!(!l.text().contains('\u{1b}'));
    }
}

// -- frontmatter --------------------------------------------------------------

const FM: &str = concat!(
    "---\n",
    "status: Active\n",
    "owner: alice\n",
    "deciders:\n",
    "  - alice\n",
    "related:\n",
    "  - models/DNS.md\n",
    "  - models/PULSE.md\n",
    "---\n\n# Title\n"
);

/// The fold handle: the status, the short scalars, and a count per list.
#[test]
fn the_metadata_summary_says_the_status_and_counts_the_lists() {
    let out = texts(FM, 70);
    assert_eq!(
        out[0],
        "\u{25be} Active  \u{b7}  alice  \u{b7}  1 decider  \u{b7}  2 related",
        "singular for one, the key verbatim for many"
    );
}

#[test]
fn the_fields_are_an_aligned_key_value_block_under_the_summary() {
    let out = texts(FM, 70);
    assert_eq!(out[1], "  status    Active");
    assert_eq!(out[2], "  owner     alice");
    assert_eq!(out[3], "  deciders  alice");
    assert_eq!(out[4], "  related   models/DNS.md");
    // A list's later values align under the first, with no repeated key.
    assert_eq!(out[5], "            models/PULSE.md");
    assert!(out[6].trim_start().starts_with('\u{2500}'), "closed by a rule");
}

#[test]
fn a_document_path_in_the_metadata_is_a_link() {
    let out = lines(FM, 70);
    let links: Vec<(usize, &str)> = out[4].links();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].1, "models/DNS.md");
    assert!(out[2].links().is_empty(), "a plain value is not a link");
}

#[test]
fn the_status_is_coloured_by_what_it_says() {
    let style = |src: &str| {
        let l = lines(src, 40);
        l[1].spans.last().unwrap().style
    };
    let live = style("---\nstatus: Accepted\n---\n\nx\n");
    let open = style("---\nstatus: Draft\n---\n\nx\n");
    let old = style("---\nstatus: Superseded \u{2014} by ADR 12\n---\n\nx\n");
    assert_ne!(live, open, "live differs from in-flight");
    assert_ne!(live, old, "live differs from historical");
    // A trailing explanation must not stop it reading as superseded.
    assert_eq!(old, crate::theme::status_of("Superseded"));
}

/// The summary row is the fold handle, and it counts its own contents — so the
/// painter must not also append `(N lines)`.
#[test]
fn the_metadata_block_is_a_self_summarising_foldable_section() {
    let out = lines(FM, 70);
    let head = out[0].heading.as_ref().expect("the summary is the handle");
    assert_eq!(head.level, 1);
    assert_eq!(head.id, crate::render::METADATA_ID);
    assert!(head.summarised);
    assert!(out[1].heading.is_none(), "only the summary is a heading");
}
