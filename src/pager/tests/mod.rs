//! Pager state-machine tests. No terminal is involved anywhere: the pager is
//! driven with synthetic key events and inspected through its state.
#![deny(unsafe_code)]

use super::keys::{self, Action};
use super::search::Dir;
use super::{Mode, Pager};
use crate::key::{Key, KeyEvent};
use crate::md;
use crate::source::markdown::MarkdownSource;
use crate::term::Frame;

const DOC: &str = "\
intro line

## Alpha

alpha one
alpha two
alpha three

### Alpha Deep

buried needle here

## Beta

beta text
beta needle

## Gamma

gamma text
";

fn pager(src: &str, cols: usize, rows: usize) -> Pager {
    let source = MarkdownSource::new(md::parse(src));
    Pager::new(Box::new(source), "doc.md".into(), cols, rows, None)
}

fn press(p: &mut Pager, s: &str) {
    for c in s.chars() {
        p.handle(KeyEvent::plain(Key::Char(c)));
    }
}

fn key(p: &mut Pager, k: Key) {
    p.handle(KeyEvent::plain(k));
}

fn heading_ids(p: &Pager) -> Vec<String> {
    p.outline().iter().map(|e| e.id.clone()).collect()
}

mod csv;
mod lens;
mod paint;

// -- scrolling ---------------------------------------------------------------

#[test]
fn scrolling_clamps_at_both_ends() {
    let mut p = pager(DOC, 60, 10);
    for _ in 0..500 {
        press(&mut p, "j");
    }
    assert_eq!(p.cursor, p.line_count() - 1);
    assert!(p.top + p.body_rows() >= p.line_count());
    for _ in 0..500 {
        press(&mut p, "k");
    }
    assert_eq!((p.cursor, p.top), (0, 0));
}

#[test]
fn paging_and_jumps_stay_in_range() {
    let mut p = pager(DOC, 60, 8);
    press(&mut p, "G");
    assert_eq!(p.cursor, p.line_count() - 1);
    press(&mut p, "g");
    assert_eq!(p.cursor, 0);
    for _ in 0..50 {
        press(&mut p, " ");
    }
    assert_eq!(p.cursor, p.line_count() - 1);
    for _ in 0..50 {
        press(&mut p, "b");
    }
    assert_eq!(p.cursor, 0);
    press(&mut p, "d");
    assert!(p.cursor > 0 && p.cursor < p.line_count());
    press(&mut p, "u");
    assert_eq!(p.cursor, 0);
}

#[test]
fn a_zero_height_terminal_does_not_panic() {
    let mut p = pager(DOC, 20, 0);
    press(&mut p, "jjGg dub");
    let mut f = Frame::new(true);
    p.paint(&mut f);
    assert_eq!(p.body_rows(), 0);
}

#[test]
fn a_one_column_terminal_does_not_panic() {
    let mut p = pager(DOC, 1, 1);
    press(&mut p, "jGl");
    let mut f = Frame::new(true);
    p.paint(&mut f);
    assert!(p.cursor < p.line_count().max(1));
}

#[test]
fn an_empty_document_renders_an_empty_view() {
    let mut p = pager("", 40, 10);
    assert_eq!(p.line_count(), 0);
    assert_eq!(p.cursor_row(), None);
    press(&mut p, "jkGg");
    press(&mut p, "za");
    key(&mut p, Key::Tab);
    let mut f = Frame::new(true);
    p.paint(&mut f);
    assert!(!p.should_quit());
}

#[test]
fn horizontal_scrolling_needs_a_scrollable_row() {
    let src = "text only\n\n```\nlonglonglonglonglonglonglonglonglongline\n```\n";
    let mut p = pager(src, 20, 12);
    press(&mut p, "l");
    let with_code_in_view = p.hoff;
    assert!(with_code_in_view > 0, "code block should scroll");
    press(&mut p, "hhh");
    assert_eq!(p.hoff, 0);
    let mut plain = pager("just a short paragraph\n", 40, 10);
    assert_eq!(plain.max_hoff(), 0);
}

/// SPEC.md §"Selecting links on a line": the arrows scroll a row that scrolls,
/// and on any other row they are link keys — which on a row with no links means
/// they do nothing at all, silently. `h`/`l` scroll from either row.
#[test]
fn the_arrows_scroll_only_a_row_that_scrolls() {
    let src = "text only\n\n```\nlonglonglonglonglonglonglonglonglongline\n```\n";
    let mut p = pager(src, 20, 12);
    assert_eq!(p.cursor_text(), "text only");
    key(&mut p, Key::Right);
    assert_eq!(p.hoff, 0, "a plain row has nothing to scroll");
    assert_eq!(p.message, None, "and says nothing about it");
    key(&mut p, Key::Left);
    assert_eq!((p.hoff, p.message.clone()), (0, None));
    // `l` scrolls from the very same row: it scrolls everywhere regardless.
    press(&mut p, "l");
    assert!(p.hoff > 0, "`l` should have scrolled the window");
    press(&mut p, "hhhhh");
    assert_eq!(p.hoff, 0);
    // Now put the cursor on the code row, which does scroll.
    for _ in 0..10 {
        if p.cursor_text().contains("longline") {
            break;
        }
        press(&mut p, "j");
    }
    assert!(p.cursor_text().contains("longline"), "never reached the code row");
    key(&mut p, Key::Right);
    assert!(p.hoff > 0, "the arrows scroll a scrollable row");
    let out = p.hoff;
    key(&mut p, Key::Left);
    assert!(p.hoff < out, "and scroll back");
}

// -- collapsing --------------------------------------------------------------

#[test]
fn toggling_a_heading_hides_its_section() {
    let mut p = pager(DOC, 60, 40);
    let before = p.line_count();
    press(&mut p, "za");
    assert_eq!(p.line_count(), before, "cursor is not on a heading yet");
    key(&mut p, Key::Tab);
    press(&mut p, "za");
    assert!(p.line_count() < before);
    assert!(!p.visible_text().iter().any(|t| t.contains("alpha one")));
    assert!(p.visible_text().iter().any(|t| t.contains("Beta")));
    press(&mut p, "za");
    assert_eq!(p.line_count(), before);
}

#[test]
fn zo_and_zc_are_idempotent() {
    let mut p = pager(DOC, 60, 40);
    key(&mut p, Key::Tab);
    press(&mut p, "zc");
    let closed = p.line_count();
    press(&mut p, "zc");
    assert_eq!(p.line_count(), closed);
    press(&mut p, "zo");
    press(&mut p, "zo");
    assert!(p.line_count() > closed);
}

#[test]
fn collapse_all_then_expand_all() {
    let mut p = pager(DOC, 60, 40);
    let full = p.line_count();
    press(&mut p, "zM");
    assert!(p.line_count() < full);
    assert_eq!(p.folds().len(), p.outline().len());
    press(&mut p, "zR");
    assert_eq!(p.line_count(), full);
    assert!(p.folds().is_empty());
}

#[test]
fn fold_state_is_keyed_by_heading_id_and_survives_resize() {
    let mut p = pager(DOC, 80, 20);
    key(&mut p, Key::Tab);
    press(&mut p, "zc");
    let ids = p.folds();
    assert_eq!(ids.len(), 1);
    p.resize(32, 12);
    assert_eq!(p.folds(), ids);
    assert!(heading_ids(&p).contains(&ids[0]));
    assert!(!p.visible_text().iter().any(|t| t.contains("alpha one")));
    p.resize(120, 50);
    assert_eq!(p.folds(), ids);
    assert!(!p.visible_text().iter().any(|t| t.contains("alpha one")));
}

#[test]
fn resize_keeps_the_cursor_on_the_same_source_line() {
    let mut p = pager(DOC, 80, 20);
    press(&mut p, "jjjjj");
    let src = p.cursor_mark();
    p.resize(40, 20);
    let after = p.cursor_mark();
    assert_eq!(src, after);
}

#[test]
fn tab_walks_headings_in_both_directions() {
    let mut p = pager(DOC, 60, 40);
    key(&mut p, Key::Tab);
    assert!(p.cursor_text().contains("Alpha"));
    key(&mut p, Key::Tab);
    assert!(p.cursor_text().contains("Alpha Deep"));
    key(&mut p, Key::Tab);
    assert!(p.cursor_text().contains("Beta"));
    key(&mut p, Key::BackTab);
    assert!(p.cursor_text().contains("Alpha Deep"));
    key(&mut p, Key::BackTab);
    assert!(p.cursor_text().contains("Alpha"));
    key(&mut p, Key::BackTab);
    assert!(p.message.is_some(), "no previous heading is reported");
}

// -- outline -----------------------------------------------------------------

#[test]
fn outline_lists_every_heading_and_jumps() {
    let mut p = pager(DOC, 60, 20);
    press(&mut p, "o");
    assert_eq!(p.mode, Mode::Outline);
    let texts: Vec<&str> = p.outline().iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, vec!["Alpha", "Alpha Deep", "Beta", "Gamma"]);
    press(&mut p, "jjj");
    key(&mut p, Key::Enter);
    assert_eq!(p.mode, Mode::Normal);
    assert!(p.cursor_text().contains("Gamma"));
}

#[test]
fn outline_esc_cancels_without_moving() {
    let mut p = pager(DOC, 60, 20);
    press(&mut p, "jj");
    let where_we_were = p.cursor;
    press(&mut p, "o");
    press(&mut p, "jj");
    key(&mut p, Key::Esc);
    assert_eq!(p.mode, Mode::Normal);
    assert_eq!(p.cursor, where_we_were);
}

#[test]
fn outline_selection_starts_at_the_section_being_read() {
    let mut p = pager(DOC, 60, 20);
    press(&mut p, "G");
    press(&mut p, "o");
    assert_eq!(p.outline()[p.outline_sel].text, "Gamma");
}

// -- help --------------------------------------------------------------------

#[test]
fn help_overlay_opens_and_closes() {
    let mut p = pager(DOC, 60, 20);
    key(&mut p, Key::F(1));
    assert_eq!(p.mode, Mode::Help);
    let mut f = Frame::new(true);
    p.paint(&mut f);
    assert!(f.as_str().contains("line down"), "frame was: {}", f.as_str());
    // Far enough to reach the bottom of the table, whatever it has grown to.
    for _ in 0..keys::BINDINGS.len() {
        press(&mut p, "j");
    }
    let mut f = Frame::new(true);
    p.paint(&mut f);
    assert!(f.as_str().contains("quit"), "frame was: {}", f.as_str());
    key(&mut p, Key::Esc);
    assert_eq!(p.mode, Mode::Normal);
    press(&mut p, "H");
    assert_eq!(p.mode, Mode::Help);
}

// -- search ------------------------------------------------------------------

#[test]
fn incremental_search_jumps_to_the_first_hit() {
    let mut p = pager(DOC, 60, 30);
    press(&mut p, "/needle");
    assert_eq!(p.mode, Mode::Search(Dir::Forward));
    assert!(p.match_count() > 0);
    key(&mut p, Key::Enter);
    assert_eq!(p.mode, Mode::Normal);
    assert!(p.cursor_text().contains("needle"));
}

#[test]
fn search_is_smartcase() {
    let mut p = pager("alpha\n\nAlpha\n", 40, 10);
    press(&mut p, "/alpha");
    assert_eq!(p.match_count(), 2);
    key(&mut p, Key::Enter);
    press(&mut p, "/Alpha");
    assert_eq!(p.match_count(), 1);
}

#[test]
fn n_and_shift_n_cycle_and_report_the_wrap() {
    let mut p = pager(DOC, 60, 30);
    press(&mut p, "/needle");
    key(&mut p, Key::Enter);
    let first = p.current_match().unwrap();
    press(&mut p, "n");
    assert_ne!(p.current_match().unwrap(), first);
    p.message = None;
    press(&mut p, "n");
    assert_eq!(p.current_match().unwrap(), first);
    assert!(p.message.as_deref().unwrap_or("").contains("hit bottom"));
    p.message = None;
    press(&mut p, "N");
    assert!(p.message.as_deref().unwrap_or("").contains("hit top"));
}

#[test]
fn search_expands_a_fold_to_reveal_a_hit() {
    let mut p = pager(DOC, 60, 30);
    press(&mut p, "zM");
    assert!(!p.visible_text().iter().any(|t| t.contains("buried needle")));
    press(&mut p, "/buried");
    key(&mut p, Key::Enter);
    assert!(p.cursor_text().contains("buried needle"));
    // The hit is on screen: the fold hiding it was expanded, not skipped.
    assert!(p.current_match().is_some());
    assert!(p.visible_text().iter().any(|t| t.contains("buried needle")));
}

#[test]
fn a_failed_search_reports_and_does_not_move() {
    let mut p = pager(DOC, 60, 30);
    press(&mut p, "jj");
    let at = p.cursor;
    press(&mut p, "/zzzznope");
    key(&mut p, Key::Enter);
    assert_eq!(p.cursor, at);
    assert!(p.message.as_deref().unwrap_or("").contains("not found"));
}

#[test]
fn escaping_a_search_restores_the_starting_position() {
    let mut p = pager(DOC, 60, 30);
    press(&mut p, "jjj");
    let at = p.cursor;
    press(&mut p, "/needle");
    key(&mut p, Key::Esc);
    assert_eq!(p.mode, Mode::Normal);
    assert_eq!(p.cursor, at);
    assert_eq!(p.match_count(), 0);
}

#[test]
fn backspacing_the_whole_query_leaves_search_mode() {
    let mut p = pager(DOC, 60, 30);
    press(&mut p, "/ne");
    key(&mut p, Key::Backspace);
    key(&mut p, Key::Backspace);
    key(&mut p, Key::Backspace);
    assert_eq!(p.mode, Mode::Normal);
}

#[test]
fn search_survives_a_resize() {
    let mut p = pager(DOC, 80, 30);
    press(&mut p, "/needle");
    key(&mut p, Key::Enter);
    let hits = p.match_count();
    p.resize(40, 20);
    assert_eq!(p.match_count(), hits);
}

#[test]
fn collapsing_everything_clamps_the_viewport_before_it_jumps() {
    // Folding shrinks the document under the viewport, and the jump that
    // follows `zM` tests "is the cursor on screen?" against `top`. Clamping
    // first is what the pre-seam pager did inside every fold mutation: without
    // it the window settles rows below where it used to, with the top of the
    // collapsed document scrolled off (SPEC.md §The `Source` seam — nothing
    // about markdown's behaviour may change behind the trait).
    let mut doc = String::from("# Title\n\nintro\n");
    for s in 1..=6 {
        doc.push_str(&format!("\n## Section {s}\n\n"));
        for l in 0..20 {
            doc.push_str(&format!("body {s}.{l}\n"));
        }
    }
    let mut p = pager(&doc, 60, 12);
    key(&mut p, Key::Tab);
    key(&mut p, Key::Tab);
    assert!(p.top > 0, "the document did not scroll");
    press(&mut p, "zM");
    assert_eq!(p.top, 0, "viewport left behind by the collapse");
    assert!(p.cursor >= p.top && p.cursor < p.top + p.body_rows());
    // And the same key sequence one section further down, where the collapsed
    // document is still taller than the screen: the window follows the cursor
    // rather than staying where the expanded document had it.
    let mut p = pager(&doc, 60, 12);
    for _ in 0..3 {
        key(&mut p, Key::Tab);
    }
    press(&mut p, "zM");
    assert_eq!((p.cursor, p.top), (12, 9));
}

// -- metadata (frontmatter) ---------------------------------------------------

const META: &str = concat!(
    "---\nstatus: Active\nrelated:\n  - models/A.md\n  - models/B.md\n---\n\n",
    "# Title\n\nbody\n"
);

/// Closed on open, and `za` on the summary row opens it.
#[test]
fn metadata_starts_folded_and_za_opens_it() {
    let mut p = pager(META, 60, 12);
    let shut = p.visible_text();
    assert!(shut[0].contains("Active"), "{:?}", shut[0]);
    assert!(shut[0].contains("2 related"), "counts what it hides");
    assert!(!shut.iter().any(|t| t.contains("models/A.md")), "fields hidden");

    p.cursor = 0;
    press(&mut p, "za");
    let open = p.visible_text();
    assert!(open.iter().any(|t| t.contains("models/A.md")), "fields shown");
}

/// `y` on a metadata row copies that field's value, the way it copies a cell
/// in a CSV. Elsewhere in a markdown document it still falls back to the link.
#[test]
fn y_on_a_metadata_row_copies_the_field() {
    let mut p = pager(META, 60, 12);
    p.cursor = 0;
    press(&mut p, "za"); // open it
    p.cursor = 2; // the `related` row
    press(&mut p, "y");
    let y = p.peek_yank().expect("a field yank");
    assert_eq!(y.text, "models/A.md\n");
}

/// Prose has no blocks, so `j`/`k` are one rendered line — the default that
/// must not move under a format that never opted in. `Ctrl-E`/`Ctrl-Y` are the
/// same one row here, which is what makes them safe to press anywhere.
#[test]
fn j_and_k_move_one_row_on_a_document_with_no_blocks() {
    let mut p = pager(DOC, 60, 20);
    assert!(!p.src_blocks());
    for expected in 1..6 {
        press(&mut p, "j");
        assert_eq!(p.cursor, expected);
    }
    press(&mut p, "k");
    assert_eq!(p.cursor, 4);
    key(&mut p, Key::Ctrl('e'));
    assert_eq!(p.cursor, 5);
    key(&mut p, Key::Ctrl('y'));
    assert_eq!(p.cursor, 4);
}
