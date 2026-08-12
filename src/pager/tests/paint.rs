//! Status bar, painting, and the `--width N` wider-than-the-terminal case.
//! Split out of `tests/mod.rs` to keep both files under the size limit.
#![deny(unsafe_code)]

use super::*;

// -- status bar and painting -------------------------------------------------

#[test]
fn status_bar_shows_the_document_and_position() {
    let p = pager(DOC, 80, 10);
    let s = crate::pager::view::status_text(&p);
    assert!(s.starts_with("doc.md"));
    assert!(s.contains("line 1/"));
    assert!(s.contains('%'));
}

#[test]
fn transient_messages_replace_the_status_bar() {
    let mut p = pager(DOC, 80, 10);
    p.notify("yanked");
    assert_eq!(crate::pager::view::status_text(&p), "yanked");
    p.message = None;
    assert!(crate::pager::view::status_text(&p).starts_with("doc.md"));
}

#[test]
fn the_search_prompt_owns_the_status_bar_while_typing() {
    let mut p = pager(DOC, 80, 10);
    press(&mut p, "/nee");
    assert_eq!(crate::pager::view::status_text(&p), "/nee");
    key(&mut p, Key::Esc);
    press(&mut p, "?be");
    assert_eq!(crate::pager::view::status_text(&p), "?be");
}

#[test]
fn a_folded_heading_shows_a_line_count() {
    let mut p = pager(DOC, 60, 20);
    key(&mut p, Key::Tab);
    press(&mut p, "zc");
    let mut f = Frame::new(true);
    p.paint(&mut f);
    assert!(f.as_str().contains("lines)"), "frame was: {}", f.as_str());
    assert!(f.as_str().contains(crate::theme::MARKER_CLOSED));
}

/// The focused link keeps its own colour, so which link leaves the reader is
/// visible even while it is the one under focus (SPEC.md §Navigation).
#[test]
fn the_focused_link_keeps_the_external_colour() {
    let src = "one [in](models/A.md) two [out](https://example.com/x) three\n";
    let mut p = pager(src, 80, 8);
    let focused_fg = |p: &mut Pager| -> Option<u8> {
        crate::pager::view::row_spans(p, p.cursor)
            .iter()
            .find(|s| s.style.has(crate::term::REVERSE))
            .and_then(|s| s.style.fg)
    };
    // The first link on the cursor row is focused already; no `n` needed.
    assert_eq!(p.focused_link().map(|s| s.url.clone()).as_deref(), Some("models/A.md"));
    assert_eq!(focused_fg(&mut p), Some(crate::theme::LINK));
    press(&mut p, "n");
    assert_eq!(
        p.focused_link().map(|s| s.url.clone()).as_deref(),
        Some("https://example.com/x")
    );
    assert_eq!(focused_fg(&mut p), Some(crate::theme::LINK_EXTERNAL));
}

#[test]
fn painting_never_writes_a_mouse_sequence() {
    let mut p = pager(DOC, 60, 12);
    press(&mut p, "/needle");
    key(&mut p, Key::Enter);
    let mut f = Frame::new(false);
    p.paint(&mut f);
    for bad in ["?1000", "?1002", "?1003", "?1006", "?1015"] {
        assert!(!f.as_str().contains(bad));
    }
}

#[test]
fn quit_sets_the_flag() {
    let mut p = pager(DOC, 60, 12);
    assert!(!p.should_quit());
    press(&mut p, "q");
    assert!(p.should_quit());
}

#[test]
fn every_action_is_reachable_and_survives_being_fired_blind() {
    let mut p = pager(DOC, 40, 6);
    fire_every_action(&mut p);
    assert!(!p.should_quit());
}

/// The same barrage against a document that reads in **blocks**, where `j`/`k`
/// and `Tab` run framing code markdown never reaches. A tiny viewport is the
/// point: the arithmetic that brings a block on screen is where an off-by-one
/// panics.
#[test]
fn every_action_survives_being_fired_blind_at_a_lensed_document() {
    let mut src = crate::source::jsonl::JsonlSource::from_bytes(super::lens::run().into_bytes());
    src.set_lens(crate::lens::find("agent").expect("the agent lens"));
    let mut p = Pager::new(Box::new(src), "session.jsonl".into(), 24, 3, None);
    let _ = p.visible_text();
    fire_every_action(&mut p);
    fire_every_action(&mut p);
    assert!(!p.should_quit());
    assert!(p.cursor < p.line_count());
}

fn fire_every_action(p: &mut Pager) {
    for a in [
        Action::LineDown,
        Action::LineUp,
        Action::HalfDown,
        Action::HalfUp,
        Action::PageDown,
        Action::PageUp,
        Action::Top,
        Action::Bottom,
        Action::ScrollLeft,
        Action::ScrollRight,
        Action::ArrowLeft,
        Action::ArrowRight,
        Action::ToggleCollapse,
        Action::OpenSection,
        Action::CloseSection,
        Action::CollapseAll,
        Action::ExpandAll,
        Action::NextHeading,
        Action::PrevHeading,
        Action::Outline,
        Action::Help,
        Action::NextMatch,
        Action::PrevMatch,
    ] {
        p.mode = Mode::Normal;
        p.act(a);
        let mut f = Frame::new(true);
        p.paint(&mut f);
    }
}

#[test]
fn dirty_flag_gates_repaints() {
    let mut p = pager(DOC, 40, 10);
    assert!(p.take_dirty());
    assert!(!p.dirty());
    press(&mut p, "j");
    assert!(p.take_dirty());
}

// -- `--width N` wider than the terminal (SPEC.md §CLI) ----------------------

const WIDE_DOC: &str = "\
## Wide

This paragraph is deliberately long enough that laying it out at two hundred
columns produces rows far wider than a forty column viewport, which is exactly
the case a forced width has to keep reachable.
";

fn forced(src: &str, cols: usize, rows: usize, width: usize) -> Pager {
    let source = MarkdownSource::new(md::parse(src));
    Pager::new(Box::new(source), "doc.md".into(), cols, rows, Some(width))
}

#[test]
fn a_forced_width_wider_than_the_terminal_stays_horizontally_reachable() {
    let mut p = forced(WIDE_DOC, 40, 12, 200);
    assert_eq!(p.width, 200);
    let rows = all_rows(&mut p);
    let widest = rows.iter().map(|l| l.width()).max().unwrap();
    assert!(widest > 40, "nothing overflows the viewport: {widest}");
    // The renderer marks nothing `scroll` here — these are wrapped paragraph
    // rows — yet they must still be scrollable.
    assert!(rows.iter().all(|l| !l.scroll));
    assert_eq!(p.max_hoff(), widest - 40);
    for l in &rows {
        assert_eq!(crate::pager::scrollable(l, 40), l.width() > 40);
    }
}

/// Every row the pager would show, for the assertions that need the lot.
fn all_rows(p: &mut Pager) -> Vec<crate::render::Line> {
    let n = p.line_count();
    p.src.lines(0..n)
}

#[test]
fn h_and_l_reach_the_hidden_text_of_a_forced_width() {
    let mut p = forced(WIDE_DOC, 40, 12, 200);
    let max = p.max_hoff();
    assert!(max > 0);
    for _ in 0..100 {
        press(&mut p, "l");
    }
    assert_eq!(p.hoff, max, "`l` never reached the right edge");
    for _ in 0..100 {
        press(&mut p, "h");
    }
    assert_eq!(p.hoff, 0);
}

#[test]
fn the_cut_marker_shows_on_an_overflowing_row() {
    let mut p = forced(WIDE_DOC, 40, 12, 200);
    let row = |p: &mut Pager| -> String {
        let at = all_rows(p)
            .iter()
            .position(|l| l.width() > 40)
            .expect("an overflowing row");
        crate::pager::view::row_spans(p, at)
            .iter()
            .map(|s| s.text.as_str())
            .collect()
    };
    assert!(row(&mut p).ends_with('\u{203a}'), "no right cut marker: {:?}", row(&mut p));
    press(&mut p, "l");
    let scrolled = row(&mut p);
    assert!(scrolled.starts_with('\u{2039}'), "no left cut marker: {scrolled:?}");
}

#[test]
fn a_width_inside_the_viewport_scrolls_nothing() {
    let mut p = forced(WIDE_DOC, 100, 12, 60);
    assert_eq!(p.max_hoff(), 0);
    assert!(all_rows(&mut p)
        .iter()
        .all(|l| !crate::pager::scrollable(l, 100)));
}
