//! `MarkdownSource` unit tests: the seam's contract, exercised through the
//! trait alone. Split out of `markdown.rs` to keep both files under the size
//! limit.
#![deny(unsafe_code)]

use super::*;
use crate::md;

const DOC: &str = "\
intro

## Alpha

one needle
two

### Deep

buried

## Beta

[a link](b.md)
";

fn src(width: usize) -> MarkdownSource {
    let mut s = MarkdownSource::new(md::parse(DOC));
    s.set_width(width);
    s
}

fn text(s: &mut MarkdownSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n)
        .iter()
        .map(|l| l.text().trim().to_string())
        .collect()
}

#[test]
fn lines_are_windowed_and_clamped() {
    let mut s = src(60);
    let n = s.len();
    assert!(n > 5);
    assert_eq!(s.lines(2..5).len(), 3);
    assert_eq!(s.lines(n - 1..n + 40).len(), 1);
    assert!(s.lines(n + 1..n + 2).is_empty());
    assert_eq!(s.line(0).unwrap().text(), s.lines(0..1)[0].text());
    assert!(s.line(n).is_none());
}

#[test]
fn folding_hides_rows_and_reports_the_count() {
    let mut s = src(60);
    let full = s.len();
    let alpha = s.outline().iter().position(|e| e.text == "Alpha").unwrap();
    assert!(s.set_fold(alpha, true));
    assert!(!s.set_fold(alpha, true), "already closed");
    assert!(s.len() < full);
    assert!(!text(&mut s).iter().any(|t| t == "one needle"));
    let row = s.row_of(s.outline()[alpha].anchor).unwrap();
    assert!(s.hidden_at(row).unwrap() > 0);
    assert_eq!(s.folds(), vec![s.outline()[alpha].id.clone()]);
    s.set_folds(Vec::new());
    assert_eq!(s.len(), full);
}

#[test]
fn folds_and_marks_survive_a_relayout() {
    let mut s = src(80);
    s.fold_all(true);
    let ids = s.folds();
    let row = s.len() - 1;
    let mark = s.mark(row).unwrap();
    s.set_width(30);
    assert_eq!(s.folds(), ids);
    assert_eq!(s.mark(s.locate(mark).unwrap()), Some(mark));
    s.fold_all(false);
    assert!(text(&mut s).iter().any(|t| t == "buried"));
}

#[test]
fn search_finds_folded_text_and_reveal_exposes_it() {
    let mut s = src(60);
    s.fold_all(true);
    s.set_query("buried");
    assert_eq!(s.match_count(), 1);
    let hit = s.preview_match(Anchor(0), Dir::Forward).unwrap();
    let row = s.reveal(hit.anchor).unwrap();
    assert_eq!(s.current_match(), Some(0));
    assert_eq!(s.line(row).unwrap().text().trim(), "buried");
    assert_eq!(s.matches_on(row).len(), 1);
    assert!(s.matches_on(row)[0].current);
    s.set_query("");
    assert_eq!(s.match_count(), 0);
    assert!(s.matches_on(row).is_empty());
}

#[test]
fn cycling_wraps_and_reports_it() {
    let mut s = src(60);
    s.set_query("needle");
    assert_eq!(s.match_count(), 1);
    let first = s.cycle_match(Anchor(0), Dir::Forward).unwrap();
    assert!(!first.wrapped);
    let again = s.cycle_match(Anchor(0), Dir::Forward).unwrap();
    assert_eq!(again.anchor, first.anchor);
    assert!(again.wrapped);
}

#[test]
fn structure_navigation_agrees_with_the_outline() {
    let mut s = src(60);
    let texts: Vec<&str> = s.outline().iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, vec!["Alpha", "Deep", "Beta"]);
    let first = s.next_landmark(0, true).unwrap();
    assert!(s.line(first).unwrap().text().contains("Alpha"));
    assert_eq!(s.next_landmark(first, false), None);
    assert_eq!(s.section_at(first), Some(0));
    assert_eq!(s.section_at(0), None, "intro is above every heading");
    let deep = s.goto_id("deep").unwrap();
    assert!(s.line(deep).unwrap().text().contains("Deep"));
    assert_eq!(s.goto_id("nope"), None);
}

#[test]
fn links_carry_anchors_that_map_back_to_rows() {
    let s = src(60);
    assert_eq!(s.links().len(), 1);
    let site = &s.links()[0];
    assert_eq!(site.url, "b.md");
    assert!(s.row_of(site.anchor).is_some());
}

#[test]
fn yanks_are_source_faithful() {
    let s = src(60);
    let row = s.next_landmark(0, true).unwrap();
    let section = s.yank_section(row).unwrap();
    assert!(section.text.starts_with("## Alpha"));
    assert!(s.yank_rows(0..1).unwrap().text.contains("intro"));
    assert_eq!(s.yank_rows(0..0), None);
    assert_eq!(s.yank_block(row), None, "no code block in this document");
    let mut code = MarkdownSource::new(md::parse("# t\n\n```\nx = 1\n```\n"));
    code.set_width(40);
    assert_eq!(code.yank_block(0).unwrap().text, "x = 1\n");
}

#[test]
fn an_empty_document_answers_everything_without_panicking() {
    let mut s = MarkdownSource::new(md::parse(""));
    s.set_width(40);
    assert!(s.is_empty());
    assert!(s.lines(0..10).is_empty());
    assert_eq!(s.anchor(0), None);
    assert_eq!(s.mark(0), None);
    assert_eq!(s.locate(Mark(3)), None);
    assert_eq!(s.reveal(Anchor(9)), None);
    assert_eq!(s.row_of(Anchor(0)), None);
    assert_eq!(s.section_at(0), None);
    assert_eq!(s.hidden_at(0), None);
    assert_eq!(s.next_landmark(0, true), None);
    assert!(!s.set_fold(0, true));
    s.fold_all(true);
    assert!(s.outline().is_empty());
    s.set_query("x");
    assert_eq!(s.preview_match(Anchor(0), Dir::Forward), None);
    assert_eq!(s.cycle_match(Anchor(0), Dir::Backward), None);
    assert_eq!(s.yank_section(0), None);
    assert_eq!(s.yank_block(0), None);
    assert_eq!(s.yank_rows(0..3), None);
}

/// `j` stays one row here as everywhere (SPEC.md §"Moving through a document").
/// What `blocks()` answers is narrower: whether `Tab` should *frame* its
/// landing. A heading starts and ends on its own row, so there is nothing to
/// frame, and prose keeps the `false` default.
#[test]
fn prose_does_not_read_in_blocks() {
    let s = MarkdownSource::new(md::parse(DOC));
    assert!(!s.blocks());
    assert_eq!(s.block_at(0), None);
}
