//! Navigation tests driven through the pager's public key interface, against
//! an in-memory corpus. No terminal and no disk are involved.
#![deny(unsafe_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::key::{Key, KeyEvent};
use crate::md;
use crate::nav::link::Fs;
use crate::nav::Navigator;
use crate::pager::{Mode, Pager};
use crate::source::markdown::MarkdownSource;

struct MapFs(HashMap<PathBuf, String>);

impl Fs for MapFs {
    fn is_file(&self, p: &Path) -> bool {
        self.0.contains_key(p)
    }
    fn is_dir(&self, p: &Path) -> bool {
        self.0.keys().any(|f| f.starts_with(p) && f != p)
    }
    fn read(&self, p: &Path) -> Result<String, String> {
        self.0
            .get(p)
            .cloned()
            .ok_or_else(|| format!("{}: not found", p.display()))
    }
}

const README: &str = "\
# Corpus

## Models

| Doc | Status |
|---|---|
| [models/A.md](models/A.md) — the A model | Active |
| [models/B.md](models/B.md) — the B model | Draft |

## Elsewhere

See [the site](https://example.com/x) and [escape](../../etc/passwd)
and [deep](models/A.md#second-heading) and [back home](#models).
";

const DOC_A: &str = "\
# A

alpha body

## Second Heading

buried text
";

fn corpus() -> MapFs {
    let mut m = HashMap::new();
    m.insert(PathBuf::from("/c/README.md"), README.to_string());
    m.insert(PathBuf::from("/c/models/A.md"), DOC_A.to_string());
    m.insert(PathBuf::from("/c/models/B.md"), "# B\n\nbee\n".to_string());
    MapFs(m)
}

/// A pager showing the index of the in-memory corpus.
fn pager_at(path: &str) -> Pager {
    let fs = corpus();
    let text = fs.read(Path::new(path)).unwrap();
    let src = MarkdownSource::new(md::parse(&text));
    let mut p = Pager::new(Box::new(src), "x".into(), 80, 24, Some(80));
    let nav = Navigator::with_fs(Box::new(corpus()), Path::new(path), None, Path::new("/c"));
    p.attach_nav(nav);
    p
}

fn press(p: &mut Pager, s: &str) {
    for c in s.chars() {
        p.handle(KeyEvent::plain(Key::Char(c)));
    }
}

fn key(p: &mut Pager, k: Key) {
    p.handle(KeyEvent::plain(k));
}

/// The current document's absolute path, always spelled with `/`.
///
/// The real value is a `PathBuf`, and on Windows joining `/c` with `models/A.md`
/// correctly yields `\c\models\A.md` — that is a filesystem path and it should
/// wear the native separator. The fixture keys below are written unix-style, so
/// the comparison, not the product, is what needs normalising.
fn current(p: &Pager) -> String {
    let s = p.nav.as_ref().unwrap().current().display().to_string();
    match crate::plat::path::sep(crate::plat::Platform::HOST) {
        '/' => s,
        native => s.replace(native, "/"),
    }
}

/// Press `n` until the focused link satisfies `hit`, then stop.
///
/// Bounded on purpose. The obvious `while status != want { press("n") }` spins
/// forever the moment the expectation is wrong on some platform, and a test
/// that hangs burns a CI runner until someone cancels it by hand instead of
/// failing in a second with a useful message. This one names what it saw.
fn seek_link(p: &mut Pager, what: &str, hit: impl Fn(&Pager) -> bool) {
    for _ in 0..p.link_count().max(1) * 2 {
        if hit(p) {
            return;
        }
        press(p, "n");
    }
    panic!(
        "never focused {what} after walking every link; last status was {:?}",
        p.link_status()
    );
}

/// The common case: seek until the status bar reads exactly `want`.
fn seek_status(p: &mut Pager, want: &str) {
    seek_link(p, want, |p| p.link_status().as_deref() == Some(want));
}

// -- the link cursor ---------------------------------------------------------

#[test]
fn n_walks_the_links_and_the_status_bar_names_the_target() {
    let mut p = pager_at("/c/README.md");
    assert!(p.link_count() >= 6, "{} links", p.link_count());
    press(&mut p, "n");
    assert_eq!(p.link_status().as_deref(), Some("models/A.md"));
    press(&mut p, "n");
    assert_eq!(p.link_status().as_deref(), Some("models/B.md"));
    // External links show the raw URL, internal ones the resolved path.
    seek_status(&mut p, "https://example.com/x");
    press(&mut p, "N");
    assert_eq!(p.link_status().as_deref(), Some("models/B.md"));
}

#[test]
fn n_scrolls_the_focused_link_into_view() {
    let mut p = pager_at("/c/README.md");
    p.resize(80, 6);
    for _ in 0..8 {
        press(&mut p, "n");
    }
    let anchor = p.focused_link().unwrap().anchor;
    let row = p.row_of(anchor).expect("the focused link is visible");
    let shown = p.top..p.top + p.body_rows();
    assert!(shown.contains(&row), "focused link row {row} not on screen");
}

#[test]
fn a_document_without_links_says_so() {
    let mut p = pager_at("/c/models/B.md");
    press(&mut p, "n");
    assert_eq!(p.message.as_deref(), Some("no links in this document"));
}

// -- following ---------------------------------------------------------------

#[test]
fn enter_on_a_link_loads_the_real_target() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), "/c/models/A.md");
    assert_eq!(p.label, "models/A.md");
    assert!(p.visible_text().iter().any(|t| t == "alpha body"));
    assert_eq!(p.nav.as_ref().unwrap().depth(), 1);
}

#[test]
fn enter_without_a_link_still_toggles_the_section() {
    let mut p = pager_at("/c/models/A.md");
    let heading = p
        .outline()
        .iter()
        .find(|e| e.id == "second-heading")
        .map(|e| e.anchor)
        .unwrap();
    p.jump_to(heading);
    key(&mut p, Key::Enter);
    assert_eq!(p.folds(), vec!["second-heading".to_string()]);
    assert!(!p.visible_text().iter().any(|t| t == "buried text"));
}

#[test]
fn external_links_are_shown_never_opened() {
    let mut p = pager_at("/c/README.md");
    seek_status(&mut p, "https://example.com/x");
    let before = current(&p);
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), before, "must not navigate");
    assert_eq!(
        p.message.as_deref(),
        Some("external link (not opened): https://example.com/x")
    );
    assert_eq!(
        p.focused_link_yank().as_deref(),
        Some("https://example.com/x")
    );
}

#[test]
fn escaping_links_are_refused_with_a_status_message() {
    let mut p = pager_at("/c/README.md");
    seek_link(&mut p, "the escaping link", |p| {
        p.focused_link()
            .map(|s| s.url.contains("etc/passwd"))
            .unwrap_or(false)
    });
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), "/c/README.md");
    let msg = p.message.clone().unwrap_or_default();
    assert!(msg.contains("escapes the index root"), "{msg}");
}

#[test]
fn same_document_anchors_jump_and_expand() {
    let mut p = pager_at("/c/models/A.md");
    press(&mut p, "zM");
    assert!(p.folds().contains(&"second-heading".to_string()));
    p.goto_anchor("second-heading");
    assert!(!p.folds().contains(&"second-heading".to_string()));
    assert_eq!(p.cursor_text(), "\u{25be} Second Heading");
    p.goto_anchor("nope");
    assert_eq!(p.message.as_deref(), Some("no heading #nope"));
}

#[test]
fn cross_document_anchors_land_on_the_heading() {
    let mut p = pager_at("/c/README.md");
    seek_status(&mut p, "models/A.md#second-heading");
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), "/c/models/A.md");
    assert_eq!(p.cursor_text(), "\u{25be} Second Heading");
}

// -- history -----------------------------------------------------------------

#[test]
fn back_restores_scroll_position_and_collapse_state_exactly() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "zM");
    let folds = p.folds();
    assert!(!folds.is_empty());
    press(&mut p, "zR");
    press(&mut p, "jjjj");
    press(&mut p, "n");
    let (top, cursor, collapsed) = (p.top, p.cursor, p.folds());
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), "/c/models/A.md");
    key(&mut p, Key::Backspace);
    assert_eq!(current(&p), "/c/README.md");
    assert_eq!((p.top, p.cursor), (top, cursor));
    assert_eq!(p.folds(), collapsed);
    assert_eq!(p.label, "README.md");
    assert_eq!(p.nav.as_ref().unwrap().depth(), 0);
}

#[test]
fn back_restores_folds_made_before_leaving() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "zM");
    press(&mut p, "n");
    // `n` reveals the fold hiding the link it lands on; what is left folded is
    // what must come back.
    let folded = p.folds();
    assert!(!folded.is_empty());
    key(&mut p, Key::Enter);
    assert!(p.folds().is_empty(), "new document starts unfolded");
    press(&mut p, "-");
    assert_eq!(p.folds(), folded);
}

#[test]
fn forward_redoes_a_pop() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    key(&mut p, Key::Enter);
    press(&mut p, "jj");
    let cursor = p.cursor;
    press(&mut p, "-");
    assert_eq!(current(&p), "/c/README.md");
    press(&mut p, "+");
    assert_eq!(current(&p), "/c/models/A.md");
    assert_eq!(p.cursor, cursor);
    press(&mut p, "+");
    assert_eq!(p.message.as_deref(), Some("no document to go forward to"));
}

#[test]
fn back_at_the_bottom_of_the_stack_says_so() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "-");
    assert_eq!(p.message.as_deref(), Some("no previous document"));
}

#[test]
fn q_pops_the_stack_before_quitting() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    key(&mut p, Key::Enter);
    press(&mut p, "q");
    assert!(!p.should_quit());
    assert_eq!(current(&p), "/c/README.md");
    press(&mut p, "q");
    assert!(p.should_quit());
}

// -- the index overlay -------------------------------------------------------

#[test]
fn the_index_lists_the_corpus_grouped_by_section() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "i");
    assert_eq!(p.mode, Mode::Index);
    let entries = p.nav.as_ref().unwrap().entries().to_vec();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].section, "Models");
    assert!(entries[0].row().contains("the A model"));
    press(&mut p, "j");
    assert_eq!(p.index_sel, 1);
    key(&mut p, Key::Enter);
    assert_eq!(p.mode, Mode::Normal);
    assert_eq!(current(&p), "/c/models/B.md");
}

#[test]
fn the_index_filters_on_slash_and_closes_on_esc() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "i");
    press(&mut p, "/");
    assert!(p.index_typing);
    press(&mut p, "b model");
    assert_eq!(p.index_rows(), vec![1]);
    assert_eq!(p.index_sel, 1);
    key(&mut p, Key::Enter); // stop typing, keep the filter
    assert!(!p.index_typing);
    key(&mut p, Key::Enter); // open the surviving row
    assert_eq!(current(&p), "/c/models/B.md");

    let mut p = pager_at("/c/README.md");
    press(&mut p, "i");
    key(&mut p, Key::Esc);
    assert_eq!(p.mode, Mode::Normal);
}

#[test]
fn a_corpus_without_an_index_says_so() {
    let fs = MapFs(
        [(PathBuf::from("/lone/N.md"), "# N\n".to_string())]
            .into_iter()
            .collect(),
    );
    let src = MarkdownSource::new(md::parse("# N\n"));
    let mut p = Pager::new(Box::new(src), "N.md".into(), 80, 24, Some(80));
    p.attach_nav(Navigator::with_fs(
        Box::new(fs),
        Path::new("/lone/N.md"),
        None,
        Path::new("/lone"),
    ));
    press(&mut p, "i");
    assert_eq!(p.message.as_deref(), Some("no corpus index"));
    assert_eq!(p.mode, Mode::Normal);
}

// -- sequential reading ------------------------------------------------------

#[test]
fn bracket_keys_walk_the_corpus_in_index_order() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "]");
    assert_eq!(current(&p), "/c/models/A.md");
    press(&mut p, "]");
    assert_eq!(current(&p), "/c/models/B.md");
    press(&mut p, "]");
    assert_eq!(p.message.as_deref(), Some("no next document in the index"));
    press(&mut p, "[");
    assert_eq!(current(&p), "/c/models/A.md");
    press(&mut p, "[");
    assert_eq!(current(&p), "/c/README.md");
}

#[test]
fn sequential_moves_are_undoable() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "]");
    press(&mut p, "]");
    assert_eq!(p.nav.as_ref().unwrap().depth(), 2);
    press(&mut p, "-");
    press(&mut p, "-");
    assert_eq!(current(&p), "/c/README.md");
}

// -- painting ----------------------------------------------------------------

#[test]
fn the_status_bar_shows_depth_and_the_focused_target() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    let s = crate::pager::view::status_text(&p);
    assert!(s.starts_with("README.md"), "{s}");
    assert!(s.ends_with("models/A.md"), "{s}");
    key(&mut p, Key::Enter);
    let s = crate::pager::view::status_text(&p);
    assert!(s.contains("[1 back]"), "{s}");
    assert!(s.starts_with("models/A.md"), "{s}");
}

#[test]
fn the_focused_link_is_styled_apart_from_its_neighbours() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    let mut frame = crate::term::Frame::new(false);
    p.paint(&mut frame);
    assert!(frame.as_str().contains("models/A.md"));
    // Reverse video is only used for the focused link on the body rows.
    assert!(frame.as_str().contains("\u{1b}["), "styled output expected");
}

// -- yanking a link (SPEC.md §Navigation) ------------------------------------

#[test]
fn y_on_an_external_link_yanks_the_url() {
    let mut p = pager_at("/c/README.md");
    seek_status(&mut p, "https://example.com/x");
    press(&mut p, "y");
    let y = p.peek_yank().expect("external link yank");
    assert_eq!(y.text, "https://example.com/x\n");
    assert!(y.what.contains("https://example.com/x"));
}

#[test]
fn y_on_an_internal_link_yanks_the_path_relative_to_the_root() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    assert_eq!(p.link_status().as_deref(), Some("models/A.md"));
    press(&mut p, "y");
    assert_eq!(p.peek_yank().expect("link yank").text, "models/A.md\n");
}

#[test]
fn y_with_neither_a_selection_nor_a_link_explains_itself() {
    let mut p = pager_at("/c/models/B.md");
    assert_eq!(p.link_count(), 0);
    press(&mut p, "y");
    assert!(p.peek_yank().is_none());
    let msg = p.message.clone().expect("a message");
    assert!(msg.contains("press v") && msg.contains('n'), "{msg}");
}

#[test]
fn a_visual_selection_still_wins_over_the_focused_link() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    press(&mut p, "vjy");
    let y = p.peek_yank().expect("selection yank");
    assert!(!y.text.starts_with("models/A.md\n"), "yanked the link, not the rows");
}

// -- Ctrl-C ------------------------------------------------------------------

#[test]
fn ctrl_c_quits_from_every_mode() {
    for setup in ["", "o", "H", "i", "/"] {
        let mut p = pager_at("/c/README.md");
        press(&mut p, setup);
        assert!(!p.should_quit(), "{setup:?} already quit");
        p.handle(KeyEvent::plain(Key::Ctrl('c')));
        assert!(p.should_quit(), "Ctrl-C did not quit from {setup:?}");
    }
}

#[test]
fn ctrl_c_does_not_step_back_through_the_history_first() {
    let mut p = pager_at("/c/README.md");
    press(&mut p, "n");
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), "/c/models/A.md");
    assert!(p.can_pop());
    p.handle(KeyEvent::plain(Key::Ctrl('c')));
    assert!(p.should_quit());
    // `q`, by contrast, pops first.
    let mut q = pager_at("/c/README.md");
    press(&mut q, "n");
    key(&mut q, Key::Enter);
    press(&mut q, "q");
    assert!(!q.should_quit());
    assert_eq!(current(&q), "/c/README.md");
}
