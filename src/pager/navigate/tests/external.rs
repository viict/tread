//! `Enter` on a link that leaves the reader, and `←`/`→` walking the links on
//! one row: the halves of SPEC.md §"Opening a link outside the reader" and
//! §"Selecting links on a line" that need a corpus to test.
//!
//! Nothing here launches anything. The pager only ever *queues* a URL — `main`
//! is what hands it to `sys::browser` — so the whole decision, including every
//! refused scheme, is asserted without a process being started.
#![deny(unsafe_code)]

use super::*;

/// Reversed by SPEC.md §"Opening a link outside the reader": this test used to
/// assert that `Enter` on an external link only *showed* it. It now asserts the
/// URL is queued for the system opener — and that the pager still spawns nothing
/// itself, which is why the test can run at all.
#[test]
fn external_links_are_handed_to_the_system_opener() {
    let mut p = pager_at("/c/README.md");
    seek_status(&mut p, "https://example.com/x");
    let before = current(&p);
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), before, "must not navigate the reader");
    assert_eq!(p.take_open().as_deref(), Some("https://example.com/x"));
    // Taken once: a second event loop turn must not open it again.
    assert_eq!(p.take_open(), None);
    assert_eq!(
        p.focused_link_yank().as_deref(),
        Some("https://example.com/x")
    );
}

/// `--no-browser` restores exactly the message the reader used to give.
#[test]
fn no_browser_shows_the_url_and_refuses() {
    let mut p = pager_at("/c/README.md");
    p.set_browser(false);
    seek_status(&mut p, "https://example.com/x");
    key(&mut p, Key::Enter);
    assert_eq!(p.take_open(), None, "nothing may be queued");
    assert_eq!(
        p.message.as_deref(),
        Some("external link (not opened): https://example.com/x")
    );
}

/// Every scheme off the allowlist is refused *by name* and never queued, so
/// nothing hostile in a document can reach the OS (SPEC.md §"Opening a link
/// outside the reader").
#[test]
fn a_scheme_off_the_allowlist_is_refused_by_name() {
    for (url, scheme) in [
        ("javascript:alert", "javascript"),
        ("file:///etc/passwd", "file"),
    ] {
        let mut p = pager_at("/c/README.md");
        seek_link(&mut p, url, |p| {
            p.focused_link().map(|s| s.url.as_str()) == Some(url)
        });
        key(&mut p, Key::Enter);
        assert_eq!(p.take_open(), None, "{url} must never be queued");
        let msg = p.message.clone().unwrap_or_default();
        assert!(msg.contains(scheme), "{msg} must name the scheme");
        assert!(msg.contains("refusing"), "{msg}");
        // Still yankable, which is the only way to get such a URL out at all.
        assert_eq!(p.focused_link_yank().as_deref(), Some(url));
    }
}

/// `mailto:` is on the allowlist and takes the same path as `https`.
#[test]
fn a_mailto_link_opens_like_any_other_external_link() {
    let src = "Write to [me](mailto:someone@example.com).\n";
    let source = MarkdownSource::new(md::parse(src));
    let mut p = Pager::new(Box::new(source), "x".into(), 80, 10, Some(80));
    // No corpus at all: a piped document still opens what leaves the reader.
    press(&mut p, "n");
    key(&mut p, Key::Enter);
    assert_eq!(p.take_open().as_deref(), Some("mailto:someone@example.com"));
}

// -- `←` / `→` on a row of links (SPEC.md §"Selecting links on a line") ------

/// Focus the link with `url` by walking `n`, then confirm the cursor is on its
/// row: the arrows are row-local, so every test below needs that established.
fn focus_url(p: &mut Pager, url: &str) {
    seek_link(p, url, |p| {
        p.focused_link().map(|s| s.url.as_str()) == Some(url)
    });
    let anchor = p.focused_link().unwrap().anchor;
    assert_eq!(p.row_of(anchor), Some(p.cursor), "cursor is on the link's row");
}

fn focused_url(p: &Pager) -> Option<String> {
    p.focused_link().map(|s| s.url.clone())
}

#[test]
fn the_arrows_walk_the_links_on_the_cursor_row() {
    let mut p = pager_at("/c/README.md");
    focus_url(&mut p, "https://example.com/s");
    assert_eq!(p.row_links().len(), 3, "three links share that row");
    key(&mut p, Key::Left);
    assert_eq!(focused_url(&p).as_deref(), Some("models/B.md"));
    key(&mut p, Key::Left);
    assert_eq!(focused_url(&p).as_deref(), Some("models/A.md"));
    key(&mut p, Key::Right);
    assert_eq!(focused_url(&p).as_deref(), Some("models/B.md"));
    // Enter still follows whatever the arrows chose.
    key(&mut p, Key::Enter);
    assert_eq!(current(&p), "/c/models/B.md");
}

/// The walk stops at the row's ends: `n`/`N` are the document-wide motion, and
/// an arrow must never move the cursor off the line being read.
#[test]
fn the_arrows_stop_at_the_ends_of_the_row_and_never_move_the_cursor() {
    let mut p = pager_at("/c/README.md");
    focus_url(&mut p, "https://example.com/s");
    let (row, top) = (p.cursor, p.top);
    for _ in 0..5 {
        key(&mut p, Key::Right);
    }
    assert_eq!(focused_url(&p).as_deref(), Some("https://example.com/s"));
    for _ in 0..5 {
        key(&mut p, Key::Left);
    }
    assert_eq!(focused_url(&p).as_deref(), Some("models/A.md"));
    assert_eq!((p.cursor, p.top), (row, top), "nothing vertical happens");
    assert_eq!(p.message, None, "and nothing is said about it");
}

/// `h`/`l` scroll everywhere regardless, so they must not have become link keys.
#[test]
fn h_and_l_are_still_only_scrolling() {
    let mut p = pager_at("/c/README.md");
    focus_url(&mut p, "https://example.com/s");
    press(&mut p, "hl");
    assert_eq!(focused_url(&p).as_deref(), Some("https://example.com/s"));
}

/// A table wider than the terminal, its links in one row. Laid out at 200
/// columns into a 40-column viewport, which is both the wide-table case and the
/// `--width 200 on an 80-column terminal` case.
fn wide_table(cols: usize) -> Pager {
    let src = "\
| Name | Site | Spec | Padding |\n\
|---|---|---|---|\n\
| A | [site](https://a.example.com/very/long/path/here) | [spec](models/A.md) \
| pppppppppppppppppppppppppppppppppppppppp |\n";
    let source = MarkdownSource::new(md::parse(src));
    Pager::new(Box::new(source), "x".into(), cols, 12, Some(200))
}

/// Regression. SPEC.md §"Selecting links on a line" used to claim "the two cases
/// never apply to the same row"; [`crate::render::table`] marks *every* row of an
/// over-wide table scrollable, so they do — and under the old "scroll always
/// wins" precedence the arrows could never select a link on a linked table, which
/// is the one case the binding was written for and what the target corpus's own
/// README is made of.
#[test]
fn the_arrows_walk_the_links_on_a_table_row_that_also_scrolls() {
    let mut p = wide_table(40);
    press(&mut p, "n");
    let site = "https://a.example.com/very/long/path/here";
    assert_eq!(focused_url(&p).as_deref(), Some(site));
    // The premise the old rule rested on, asserted as false.
    assert!(p.cursor_scrolls(), "this row really does scroll");
    assert_eq!(p.row_links().len(), 2, "and really does hold two links");

    key(&mut p, Key::Right);
    assert_eq!(focused_url(&p).as_deref(), Some("models/A.md"));
    assert_eq!(p.hoff, 0, "the arrow selected instead of scrolling");
    key(&mut p, Key::Left);
    assert_eq!(focused_url(&p).as_deref(), Some(site));
    assert_eq!(p.hoff, 0);

    // The scroll the arrows gave up is still one keypress away, as SPEC.md
    // promises `h`/`l` are.
    press(&mut p, "l");
    assert!(p.hoff > 0, "h/l still scroll this row");
}

/// The other half of the precedence: a scrollable row with fewer than two links
/// keeps scrolling, so code blocks, CSV rows, text lines and a table row holding
/// one link are unaffected.
#[test]
fn a_scrollable_row_with_one_link_still_scrolls() {
    let src = "\
| Name | Spec | Padding |\n\
|---|---|---|\n\
| A | [spec](models/A.md) | pppppppppppppppppppppppppppppppppppppppppppppppppp |\n";
    let source = MarkdownSource::new(md::parse(src));
    let mut p = Pager::new(Box::new(source), "x".into(), 40, 12, Some(200));
    press(&mut p, "n");
    assert_eq!(focused_url(&p).as_deref(), Some("models/A.md"));
    assert!(p.cursor_scrolls());
    assert_eq!(p.row_links().len(), 1);
    key(&mut p, Key::Right);
    assert!(p.hoff > 0, "one link is no reason to stop scrolling");
    assert_eq!(focused_url(&p).as_deref(), Some("models/A.md"));
}

/// A row that is wide enough to scroll *and* holds two links: `←`/`→` must not
/// have started moving the cursor to reach them (SPEC.md: "without `n` carrying
/// the cursor off it").
#[test]
fn selecting_on_a_wide_row_never_moves_the_cursor() {
    let mut p = wide_table(40);
    press(&mut p, "n");
    let (row, top) = (p.cursor, p.top);
    for _ in 0..6 {
        key(&mut p, Key::Right);
    }
    for _ in 0..6 {
        key(&mut p, Key::Left);
    }
    assert_eq!((p.cursor, p.top), (row, top));
    assert_eq!(p.message, None);
}
