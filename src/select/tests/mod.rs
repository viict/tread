//! Selection maths, line/section/code yanking and the pager wiring. Source
//! reconstruction and clipboard framing live in the sibling test modules.
//! No terminal and no clipboard is involved: everything here is pure.
#![deny(unsafe_code)]

mod clip;
mod source;

use super::clip::{
    clipboard_sequence, detect_mux, display_path, fallback_path, line_count, yank_message, Mux,
    SCREEN_CHUNK,
};
use super::source::{block_markdown, blocks_markdown, code_body, inline_markdown};
use super::*;
use crate::key::{Key, KeyEvent};
use crate::md;
use crate::pager::Pager;
use crate::render::{render_document, RenderOpts};
use crate::term::{base64, ClipReport};
use std::path::PathBuf;

fn doc_and_lines(src: &str, width: usize) -> (md::Document, Vec<Line>) {
    let doc = md::parse(src);
    let lines = render_document(&doc, &RenderOpts::new(width));
    (doc, lines)
}

// -- selection range maths ---------------------------------------------------

#[test]
fn a_fresh_selection_is_one_line() {
    let s = Selection::new(4);
    assert_eq!(s.range(), (4, 4));
    assert_eq!(s.len(), 1);
    assert!(s.contains(4) && !s.contains(3) && !s.contains(5));
    assert_eq!(s.status(), "-- VISUAL --  1 line selected");
}

#[test]
fn selection_extends_in_both_directions() {
    let mut s = Selection::new(10);
    s.set_head(13);
    assert_eq!(s.range(), (10, 13));
    assert_eq!(s.len(), 4);
    s.set_head(6);
    assert_eq!(s.range(), (6, 10), "dragging above the anchor flips the range");
    assert_eq!(s.len(), 5);
    assert!(s.contains(6) && s.contains(10) && !s.contains(11));
    assert_eq!(s.status(), "-- VISUAL --  5 lines selected");
    s.set_head(10);
    assert_eq!(s.len(), 1);
}

// -- line selection ----------------------------------------------------------

const DOC: &str = "\
# Guide

Intro paragraph.

## Install

Run the tool.

```sh
mdr README.md
mdr --index ~/codex
```

## Tables

| k | v |
| --- | --- |
| a | 1 |
";

#[test]
fn selecting_a_paragraph_yanks_its_source_not_its_rendering() {
    let (doc, lines) = doc_and_lines(DOC, 40);
    let row = lines
        .iter()
        .position(|l| l.text().contains("Intro paragraph"))
        .expect("paragraph row");
    let text = selection_text(&doc, &lines, &[row]);
    assert_eq!(text, "Intro paragraph.\n");
}

#[test]
fn selection_never_carries_gutters_arrows_or_ansi() {
    let (doc, lines) = doc_and_lines(DOC, 40);
    let rows: Vec<usize> = (0..lines.len()).collect();
    let text = selection_text(&doc, &lines, &rows);
    assert!(!text.contains('\u{1b}'));
    assert!(!text.contains('\u{25be}') && !text.contains('\u{25b8}'));
    assert!(text.contains("| k | v |"), "tables stay tables: {text}");
    assert!(text.contains("```sh"));
    assert!(text.starts_with("# Guide"));
}

#[test]
fn a_narrow_width_does_not_inject_wraps_into_the_yank() {
    let src = "Some fairly long paragraph text that will certainly wrap hard.\n";
    let (doc, wide) = doc_and_lines(src, 200);
    let (_, narrow) = doc_and_lines(src, 24);
    assert!(narrow.len() > wide.len(), "the narrow layout must wrap");
    let rows: Vec<usize> = (0..narrow.len()).collect();
    assert_eq!(selection_text(&doc, &narrow, &rows), src);
}

#[test]
fn a_partial_code_selection_is_line_exact() {
    let (doc, lines) = doc_and_lines(DOC, 60);
    let row = lines
        .iter()
        .position(|l| l.text().contains("mdr --index"))
        .expect("code row");
    let text = selection_text(&doc, &lines, &[row]);
    assert_eq!(text, "mdr --index ~/codex\n");
}

#[test]
fn selecting_every_code_row_restores_the_fences() {
    let (doc, lines) = doc_and_lines(DOC, 60);
    let rows: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LineKind::Code)
        .map(|(i, _)| i)
        .collect();
    let text = selection_text(&doc, &lines, &rows);
    assert_eq!(text, "```sh\nmdr README.md\nmdr --index ~/codex\n```\n");
}

#[test]
fn touching_one_table_row_yanks_the_whole_table() {
    let (doc, lines) = doc_and_lines(DOC, 60);
    let row = lines
        .iter()
        .position(|l| l.text().contains('1') && l.kind == LineKind::Table)
        .expect("table row");
    let text = selection_text(&doc, &lines, &[row]);
    assert!(text.contains("| k | v |") && text.contains("| a | 1 |"), "{text}");
}

#[test]
fn selecting_only_blank_rows_yanks_nothing() {
    let (doc, lines) = doc_and_lines(DOC, 40);
    let blanks: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LineKind::Blank)
        .map(|(i, _)| i)
        .collect();
    assert!(!blanks.is_empty());
    assert_eq!(selection_text(&doc, &lines, &blanks), "");
    assert_eq!(selection_yank(&doc, &lines, &blanks), None);
}

// -- sections ----------------------------------------------------------------

#[test]
fn a_section_runs_to_the_next_equal_or_shallower_heading() {
    let (doc, lines) = doc_and_lines(DOC, 60);
    let install = lines
        .iter()
        .position(|l| matches!(&l.heading, Some(h) if h.text == "Install"))
        .expect("heading");
    let (head, end) = section_range(&lines, install + 1).expect("range");
    assert_eq!(head, install);
    let next = lines
        .iter()
        .position(|l| matches!(&l.heading, Some(h) if h.text == "Tables"))
        .expect("next heading");
    assert_eq!(end, next);
    let y = section_yank(&doc, &lines, install + 1).expect("yank");
    assert_eq!(y.what, "section \u{201c}Install\u{201d}");
    assert!(y.text.starts_with("## Install"), "heading is included: {}", y.text);
    assert!(y.text.contains("```sh"));
    assert!(!y.text.contains("## Tables"), "stops at the next H2");
}

#[test]
fn a_parent_section_carries_its_children() {
    let (doc, lines) = doc_and_lines(DOC, 60);
    let y = section_yank(&doc, &lines, 0).expect("yank");
    assert!(y.text.starts_with("# Guide"));
    assert!(y.text.contains("## Install") && y.text.contains("## Tables"));
}

#[test]
fn text_above_the_first_heading_has_no_section() {
    let (doc, lines) = doc_and_lines("plain text\n\n## H\n\nbody\n", 40);
    assert_eq!(section_range(&lines, 0), None);
    assert_eq!(section_yank(&doc, &lines, 0), None);
}

// -- code lookup -------------------------------------------------------------

#[test]
fn code_lookup_finds_the_block_under_the_cursor() {
    let doc = md::parse("intro\n\n```sh\na\nb\n```\n\ntail\n");
    let codes = code_blocks(&doc.blocks);
    assert_eq!(codes.len(), 1);
    assert_eq!(codes[0].lang, Some("sh"));
    assert_eq!(codes[0].lines, ["a".to_string(), "b".to_string()]);
    // fence at 3, body 4-5, closing fence 6
    for src in 3..=6 {
        assert_eq!(code_at_or_below(&codes, src), Some(0), "source line {src}");
    }
}

#[test]
fn code_lookup_falls_forward_to_the_nearest_block_below() {
    let doc = md::parse("a\n\n```\nfirst\n```\n\nb\n\n```\nsecond\n```\n");
    let codes = code_blocks(&doc.blocks);
    assert_eq!(codes.len(), 2);
    assert_eq!(code_at_or_below(&codes, 1), Some(0), "cursor above them all");
    let after_first = codes[0].source_line + codes[0].lines.len() + 2;
    assert_eq!(code_at_or_below(&codes, after_first), Some(1));
    let past_end = codes[1].source_line + 10;
    assert_eq!(code_at_or_below(&codes, past_end), None);
}

#[test]
fn code_lookup_sees_blocks_nested_in_lists_and_quotes() {
    let doc = md::parse("- item\n\n      indented code\n\n> quoted\n>\n> ```\n> in quote\n> ```\n");
    let codes = code_blocks(&doc.blocks);
    assert!(!codes.is_empty(), "nested code must be reachable");
    assert!(codes.windows(2).all(|w| w[0].source_line <= w[1].source_line));
}

#[test]
fn c_yanks_the_body_only_with_no_fences_or_padding() {
    let (doc, lines) = doc_and_lines(DOC, 30);
    let top = 0;
    let y = code_yank(&doc, &lines, top).expect("code yank");
    assert_eq!(y.text, "mdr README.md\nmdr --index ~/codex\n");
    assert_eq!(y.what, "sh code block (2 lines)");
}

#[test]
fn c_reports_when_there_is_no_code_below() {
    let (doc, lines) = doc_and_lines("# a\n\ntext only\n", 40);
    assert_eq!(code_yank(&doc, &lines, lines.len() - 1), None);
}

#[test]
fn link_yank_copies_a_bare_url() {
    let y = link_yank("https://example.com/a").expect("yank");
    assert_eq!(y.text, "https://example.com/a\n");
    assert!(y.what.contains("https://example.com/a"));
}

// -- pager wiring ------------------------------------------------------------

fn pager(src: &str) -> Pager {
    Pager::new(md::parse(src), "doc.md".into(), 60, 12, None)
}

fn press(p: &mut Pager, s: &str) {
    for c in s.chars() {
        p.handle(KeyEvent::plain(Key::Char(c)));
    }
}

#[test]
fn v_enters_visual_mode_and_esc_leaves_it() {
    let mut p = pager(DOC);
    assert!(!p.in_visual());
    press(&mut p, "v");
    assert!(p.in_visual());
    p.handle(KeyEvent::plain(Key::Esc));
    assert!(!p.in_visual());
    press(&mut p, "vv");
    assert!(!p.in_visual(), "v toggles");
}

#[test]
fn motion_extends_the_selection_and_the_status_bar_counts_it() {
    let mut p = pager(DOC);
    press(&mut p, "vjj");
    assert!(p.status_line().contains("3 lines selected"), "{}", p.status_line());
    press(&mut p, "k");
    assert!(p.status_line().contains("2 lines selected"), "{}", p.status_line());
    press(&mut p, "G");
    assert!(p.in_visual(), "G extends rather than cancelling");
}

#[test]
fn y_yanks_the_selection_and_exits_visual_mode() {
    let mut p = pager(DOC);
    press(&mut p, "jv");
    press(&mut p, "y");
    let y = p.peek_yank().expect("a yank was queued");
    assert!(!y.text.is_empty());
    assert!(y.what.ends_with("line") || y.what.ends_with("lines"));
    assert!(!p.in_visual());
}

#[test]
fn y_without_a_selection_says_so_and_queues_nothing() {
    let mut p = pager(DOC);
    press(&mut p, "y");
    assert_eq!(p.peek_yank(), None);
    assert!(p.status_line().contains("nothing selected"), "{}", p.status_line());
}

#[test]
fn capital_y_yanks_the_enclosing_section() {
    let mut p = pager(DOC);
    for _ in 0..40 {
        press(&mut p, "j");
    }
    press(&mut p, "Y");
    let y = p.peek_yank().expect("section yank");
    assert!(y.what.starts_with("section "), "{}", y.what);
    assert!(y.text.starts_with('#'), "the heading leads the yank: {}", y.text);
}

#[test]
fn c_yanks_the_code_block_verbatim_from_anywhere_above_it() {
    let mut p = pager(DOC);
    press(&mut p, "c");
    let y = p.peek_yank().expect("code yank");
    assert_eq!(y.text, "mdr README.md\nmdr --index ~/codex\n");
}

#[test]
fn resizing_drops_a_live_selection_rather_than_mis_addressing_it() {
    let mut p = pager(DOC);
    press(&mut p, "vjj");
    p.resize(30, 20);
    assert!(!p.in_visual());
}

/// Real-corpus check: every code block `c` would yank must be byte-identical
/// to the file's own lines between the fences. Silently skipped when the corpus
/// is not on this machine, so it never turns into a red herring in CI.
#[test]
fn corpus_code_yanks_match_the_file_byte_for_byte() {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let root = PathBuf::from(home).join("rmarktui/codex");
    let mut checked = 0usize;
    for name in ["archive/OLD_AUDIT.md", "decisions/2026-06-13-sample-decision.md"] {
        let src = match std::fs::read_to_string(root.join(name)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let file: Vec<&str> = src.split('\n').collect();
        let doc = md::parse(&src);
        for code in code_blocks(&doc.blocks) {
            let fence = file[code.source_line - 1];
            assert!(
                fence.trim_start().starts_with("```") || fence.trim_start().starts_with("~~~"),
                "{name}:{} is not a fence: {fence:?}",
                code.source_line
            );
            for (i, line) in code.lines.iter().enumerate() {
                assert_eq!(line, file[code.source_line + i], "{name} code line {i}");
            }
            assert!(!code_body(code.lines).contains('\u{1b}'));
            checked += 1;
        }
    }
    if checked == 0 {
        return; // corpus absent
    }
    assert!(checked > 0);
}
