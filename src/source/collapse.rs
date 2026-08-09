//! Collapse tree over the *rendered* line list.
//!
//! The renderer is asked for the whole document, uncollapsed; folding then
//! happens here as a pure index computation. That keeps the fold state keyed by
//! heading id (never by rendered line index), so it survives a resize and
//! re-layout, and it lets a folded heading report how many lines it hides.
#![deny(unsafe_code)]

use crate::render::{Line, LineKind};

/// A heading found in the rendered lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingRef {
    /// Index of the row that starts the heading.
    pub index: usize,
    pub level: u8,
    pub id: String,
    pub text: String,
    /// 1-based source line of the heading, used to restore position after
    /// re-layout.
    pub source_line: usize,
}



/// Every heading in the rendered document, in order.
pub fn headings(lines: &[Line]) -> Vec<HeadingRef> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            l.heading.as_ref().map(|h| HeadingRef {
                index: i,
                level: h.level,
                id: h.id.clone(),
                text: h.text.clone(),
                source_line: l.source_line,
            })
        })
        .collect()
}

/// End (exclusive) of the rows belonging to the heading itself: the wrapped
/// title rows plus any rule row the theme adds under it.
pub fn own_end(lines: &[Line], head: usize) -> usize {
    let block = lines[head].block;
    let mut j = head + 1;
    while j < lines.len()
        && lines[j].heading.is_none()
        && lines[j].kind == LineKind::Heading
        && lines[j].block == block
    {
        j += 1;
    }
    j
}

/// End (exclusive) of the section owned by a heading of `level` starting at
/// `head`: the next heading of equal-or-shallower level, or the end.
pub fn section_end(lines: &[Line], head: usize, level: u8) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(head + 1)
        .find(|(_, l)| matches!(&l.heading, Some(h) if h.level <= level))
        .map(|(i, _)| i)
        .unwrap_or(lines.len())
}




/// The heading (index into `lines`) at or above `line`.
pub fn heading_at_or_above(lines: &[Line], line: usize) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let start = line.min(lines.len() - 1);
    (0..=start).rev().find(|i| lines[*i].heading.is_some())
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::md;
    use crate::render::{render_document, RenderOpts};

    fn render(src: &str, width: usize) -> Vec<Line> {
        render_document(&md::parse(src), &RenderOpts::new(width))
    }

    const DOC: &str = "\
intro line

## Alpha

one
two

### Alpha One

deep text

## Beta

beta text
";

    #[test]
    fn headings_are_found_in_order() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let names: Vec<_> = hs.iter().map(|h| (h.level, h.text.as_str())).collect();
        assert_eq!(
            names,
            vec![(2, "Alpha"), (3, "Alpha One"), (2, "Beta")]
        );
    }

    #[test]
    fn heading_lookup_walks_upwards() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let inside = hs[0].index + 3;
        assert_eq!(heading_at_or_above(&lines, inside), Some(hs[0].index));
        assert_eq!(heading_at_or_above(&lines, hs[1].index), Some(hs[1].index));
        assert_eq!(heading_at_or_above(&lines, 0), None);
    }
}
