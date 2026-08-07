//! `theme.rs` tests: the palette pins, the heading styles and the banner glyph
//! font. Beside the code, one file over, so both stay under the 500-line limit
//! (`src/cli_tests.rs` and `src/plat/path_tests.rs` do the same).
#![deny(unsafe_code)]

use super::*;

#[test]
fn every_glyph_is_three_by_five() {
    for (c, rows) in GLYPHS {
        for r in rows {
            assert_eq!(r.chars().count(), GLYPH_W, "glyph {c:?} row {r:?}");
            assert!(r.chars().all(|x| x == '#' || x == '.'), "glyph {c:?}");
        }
    }
}

#[test]
fn glyph_table_has_no_duplicates() {
    let mut seen: Vec<char> = GLYPHS.iter().map(|(c, _)| *c).collect();
    let n = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), n);
}

#[test]
fn covers_the_required_character_set() {
    for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 -.,:!?'/()".chars() {
        assert!(glyph(c).is_some(), "missing glyph for {c:?}");
    }
}

#[test]
fn banner_shape_and_width() {
    let b = banner("HI", 80).expect("HI fits");
    assert_eq!(b.len(), BANNER_ROWS);
    // two glyphs: 3 + 1 gap + 3
    assert_eq!(banner_width("HI"), Some(7));
    for row in &b {
        assert_eq!(row.chars().count(), 7);
    }
    assert_eq!(b[0], "\u{2588} \u{2588} \u{2588}\u{2588}\u{2588}");
}

#[test]
fn lowercase_folds_to_uppercase() {
    assert_eq!(banner("example", 80), banner("EXAMPLE", 80));
}

#[test]
fn unmappable_characters_return_none() {
    assert_eq!(banner("A@B", 80), None);
    assert_eq!(banner("caf\u{e9}", 80), None);
    assert_eq!(banner("", 80), None);
    assert_eq!(banner("   ", 80), None);
}

#[test]
fn too_wide_returns_none() {
    assert_eq!(banner_width("ABC"), Some(11));
    assert!(banner("ABC", 11).is_some());
    assert!(banner("ABC", 10).is_none());
}

#[test]
fn banner_is_trimmed_before_layout() {
    assert_eq!(banner("  HI  ", 80), banner("HI", 80));
}

#[test]
fn heading_styles_follow_the_spec() {
    assert!(heading(1).has(crate::term::BOLD) && heading(1).fg == Some(ACCENT));
    assert!(heading(2).has(crate::term::BOLD));
    assert!(heading(3).has(crate::term::BOLD) && heading(3).fg == Some(H3_FG));
    assert!(!heading(4).has(crate::term::BOLD));
    assert!(heading(5).has(crate::term::DIM));
    assert!(heading(6).has(crate::term::DIM) && heading(6).has(crate::term::ITALIC));
    assert_eq!(heading(9).attrs, heading(6).attrs);
}

#[test]
fn heading_indents_step_by_two_from_h3() {
    let got: Vec<usize> = (1..=6).map(heading_indent).collect();
    assert_eq!(got, vec![0, 0, 2, 4, 6, 8]);
}

#[test]
fn bullets_cycle_by_depth() {
    assert_eq!(bullet(0), '\u{2022}');
    assert_eq!(bullet(1), '\u{25e6}');
    assert_eq!(bullet(2), '\u{25aa}');
    assert_eq!(bullet(3), bullet(0));
}

#[test]
fn link_colour_is_the_spec_blue() {
    assert_eq!(LINK, 39);
}

/// A link that leaves the reader must be *visibly* another colour, and the
/// chooser must be the only thing that decides which (SPEC.md §Navigation).
#[test]
fn an_external_link_is_a_different_colour_from_an_internal_one() {
    assert_ne!(LINK, LINK_EXTERNAL);
    assert_eq!(link_fg(false), LINK);
    assert_eq!(link_fg(true), LINK_EXTERNAL);
    // Not the search or selection background either: a focused external
    // link is painted reversed, so its foreground becomes a background.
    assert_ne!(LINK_EXTERNAL, SEARCH_BG);
    assert_ne!(LINK_EXTERNAL, SELECTION_BG);
}
