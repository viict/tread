//! Source reconstruction: AST -> markdown, per block and inline type.
#![deny(unsafe_code)]

use super::*;
// -- source reconstruction ---------------------------------------------------

fn md_of(src: &str) -> String {
    blocks_markdown(&md::parse(src).blocks)
}

#[test]
fn headings_and_paragraphs_round_trip() {
    assert_eq!(md_of("## Install\n\nRun it.\n"), "## Install\n\nRun it.\n");
    assert_eq!(md_of("###### deep\n"), "###### deep\n");
}

#[test]
fn inline_styles_come_back_as_markdown() {
    let src = "a **b** *c* ~~d~~ `e` [t](u) ![alt](i) <https://x.y>\n";
    let out = md_of(src);
    assert!(out.contains("**b**"), "{out}");
    assert!(out.contains("*c*"));
    assert!(out.contains("~~d~~"));
    assert!(out.contains("`e`"));
    assert!(out.contains("[t](u)"));
    assert!(out.contains("![alt](i)"));
    assert!(out.contains("<https://x.y>"));
    assert!(!out.contains('\u{1b}'), "no ANSI ever reaches the clipboard");
}

#[test]
fn code_spans_widen_their_fence_when_they_hold_backticks() {
    let out = md_of("use ``a ` b`` here\n");
    assert!(out.contains("``a ` b``"), "{out}");
}

#[test]
fn code_blocks_keep_language_and_body_verbatim() {
    let out = md_of("```rust\nfn main() {\n    let x = 1;\n}\n```\n");
    assert_eq!(out, "```rust\nfn main() {\n    let x = 1;\n}\n```\n");
}

#[test]
fn code_blocks_containing_fences_get_a_longer_fence() {
    let out = md_of("````\n```\ninner\n```\n````\n");
    assert!(out.starts_with("````\n```\ninner\n```\n````"), "{out}");
}

#[test]
fn lists_come_back_usable() {
    let out = md_of("- one\n- two\n  - deep\n");
    assert_eq!(out, "- one\n- two\n  - deep\n");
    let ordered = md_of("3. c\n4. d\n");
    assert_eq!(ordered, "3. c\n4. d\n");
    let tasks = md_of("- [x] done\n- [ ] todo\n");
    assert_eq!(tasks, "- [x] done\n- [ ] todo\n");
}

#[test]
fn tables_come_back_as_markdown_tables_with_alignment() {
    let src = "| a | b | c |\n| :--- | :---: | ---: |\n| 1 | 2 | 3 |\n";
    let out = md_of(src);
    let rows: Vec<&str> = out.trim_end().split('\n').collect();
    assert_eq!(rows[0], "| a | b | c |");
    assert_eq!(rows[1], "| :--- | :---: | ---: |");
    assert_eq!(rows[2], "| 1 | 2 | 3 |");
}

#[test]
fn quotes_rules_html_and_footnotes_reconstruct() {
    assert_eq!(md_of("> quoted\n"), "> quoted\n");
    assert_eq!(md_of("***\n"), "---\n");
    assert_eq!(md_of("<div>\nx\n</div>\n"), "<div>\nx\n</div>\n");
    let fnote = md_of("[^a]: note text\n");
    assert!(fnote.starts_with("[^a]: note text"), "{fnote}");
}

#[test]
fn nested_quotes_nest_their_bars() {
    let out = md_of("> outer\n>\n> > inner\n");
    assert!(out.contains("> > inner"), "{out}");
}

#[test]
fn code_body_is_exactly_what_sat_between_the_fences() {
    let body = vec!["  indented".to_string(), "".to_string(), "tail".into()];
    assert_eq!(code_body(&body), "  indented\n\ntail\n");
    assert_eq!(code_body(&[]), "");
}

#[test]
fn inline_markdown_keeps_soft_and_hard_breaks() {
    let doc = md::parse("one\ntwo\n");
    let text = match &doc.blocks[0] {
        Block::Paragraph { content, .. } => inline_markdown(content),
        other => panic!("expected a paragraph, got {other:?}"),
    };
    assert_eq!(text, "one\ntwo");
}

#[test]
fn a_single_block_serialises_without_a_trailing_newline() {
    let doc = md::parse("# Title\n");
    assert_eq!(block_markdown(&doc.blocks[0]), "# Title");
}

