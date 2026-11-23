//! Heading status tags in the outline overlay (SPEC.md §Headings).
#![deny(unsafe_code)]

use super::*;
use crate::pager::view::outline_rows;
use crate::theme;

#[test]
fn the_outline_colours_a_status_tag_like_the_heading_does() {
    let p = pager("## Auth [PROPOSED]\n\nx\n\n## Cache [BLOCKED: infra]\n\ny\n", 60, 20);
    let rows = outline_rows(&p);
    let find = |row: usize, text: &str| {
        rows[row].iter().find(|s| s.text == text).map(|s| s.style.fg)
    };
    assert_eq!(find(0, "[PROPOSED]"), Some(Some(theme::STATUS_OPEN)), "{:?}", rows[0]);
    assert_eq!(find(1, "[BLOCKED: infra]"), Some(Some(theme::STATUS_BLOCKED)), "{:?}", rows[1]);
    // The title around the tag keeps the heading's own style.
    let title = rows[0].iter().find(|s| s.text.contains("Auth")).expect("title span");
    assert_eq!(title.style, theme::heading(2));
}
