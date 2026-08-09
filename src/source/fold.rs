//! What folds, and what folding hides.
//!
//! Two documents disagree about where a fold *ends*, and the disagreement is
//! not cosmetic.
//!
//! Prose is written in sections: a heading owns everything until the next
//! heading of equal or shallower level, and nothing marks the end but the
//! beginning of the next thing. Code is written in blocks, which end where they
//! close — and inferring an end from "the next heading" would make
//!
//! ```text
//! fn f() {
//!     if a {      <- a fold starting here…
//!         x();
//!     }
//!     y();        <- …would swallow this, which is not inside it
//! }
//! ```
//!
//! hide a statement that is not in the branch. So the *regions* are the seam:
//! a document says which it has, and everything after that — what is on screen,
//! what a fold hides, what to open to reach a row — is arithmetic over them.
#![deny(unsafe_code)]

use crate::render::Line;

/// One foldable region of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// Stable name, which the fold state is keyed by. Must survive re-layout.
    pub id: String,
    /// Nesting depth, 1 = outermost. Only used to nest regions inside regions.
    pub level: u8,
    /// First row the region owns.
    pub head: usize,
    /// End (exclusive) of the rows that stay on screen when it is shut.
    pub body: usize,
    /// End (exclusive) of everything the region covers.
    pub end: usize,
}

impl Region {
    /// Rows hidden when this region is shut.
    pub fn hidden(&self) -> usize {
        self.end.saturating_sub(self.body)
    }

    fn covers(&self, row: usize) -> bool {
        row >= self.body && row < self.end
    }
}

/// How a document decides what folds.
///
/// Implemented once for prose ([`Sections`]) and once for code, which is the
/// whole reason this is a trait rather than a function.
pub trait Folds {
    fn regions(&self, lines: &[Line]) -> Vec<Region>;
}

/// The prose model: a heading owns everything up to the next heading of equal
/// or shallower level.
///
/// What markdown has always done, and what a JSON tree and a directory listing
/// would want if they folded.
pub struct Sections;

impl Folds for Sections {
    fn regions(&self, lines: &[Line]) -> Vec<Region> {
        super::collapse::headings(lines)
            .into_iter()
            .map(|h| {
                let body = super::collapse::own_end(lines, h.index);
                Region {
                    end: super::collapse::section_end(lines, h.index, h.level).max(body),
                    id: h.id,
                    level: h.level,
                    head: h.index,
                    body,
                }
            })
            .collect()
    }
}

/// The regions actually doing the hiding: those that are shut and not already
/// inside another shut one.
///
/// A region nested in a folded region contributes nothing — its rows are gone
/// either way — and counting it twice would hide the same lines twice.
pub fn active<'a>(regions: &'a [Region], collapsed: &[String]) -> Vec<&'a Region> {
    let mut shut: Vec<&Region> = regions
        .iter()
        .filter(|r| collapsed.contains(&r.id))
        .collect();
    shut.sort_by_key(|r| (r.head, std::cmp::Reverse(r.end)));
    let mut out: Vec<&Region> = Vec::new();
    for r in shut {
        // Skip anything the last kept region already hides.
        if out.last().map(|p| r.head >= p.body && r.end <= p.end).unwrap_or(false) {
            continue;
        }
        out.push(r);
    }
    out
}

/// Indices of the rows on screen, in order.
pub fn visible(len: usize, regions: &[Region], collapsed: &[String]) -> Vec<usize> {
    let shut = active(regions, collapsed);
    let mut out = Vec::with_capacity(len);
    let mut i = 0usize;
    while i < len {
        match shut.iter().find(|r| r.covers(i)) {
            // Jump over what this fold hides, in one step.
            Some(r) => i = r.end.max(i + 1),
            None => {
                out.push(i);
                i += 1;
            }
        }
    }
    out
}

/// Which row a fold's summary belongs on.
///
/// The two models disagree, and neither is wrong. A section's summary belongs
/// on its **title** — the theme may draw a rule under it, so the last row a
/// heading owns is not the row anyone reads. A declaration's belongs on its
/// **signature**, which is the last row it owns, because a three-line doc
/// comment above it is not where `(9 lines)` makes sense.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    Head,
    LastOwn,
}

/// `(row, rows hidden)` for each fold currently doing the hiding.
pub fn counts(regions: &[Region], collapsed: &[String], on: Note) -> Vec<(usize, usize)> {
    active(regions, collapsed)
        .into_iter()
        .map(|r| {
            let row = match on {
                Note::Head => r.head,
                Note::LastOwn => r.body.saturating_sub(1),
            };
            (row, r.hidden())
        })
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// Open every fold hiding `target`, and say whether anything changed.
pub fn reveal(regions: &[Region], collapsed: &mut Vec<String>, target: usize) -> bool {
    let mut changed = false;
    loop {
        let hiding: Option<String> = active(regions, collapsed)
            .into_iter()
            .find(|r| r.covers(target))
            .map(|r| r.id.clone());
        match hiding {
            Some(id) => {
                collapsed.retain(|c| *c != id);
                changed = true;
            }
            None => return changed,
        }
    }
}

/// The innermost region owning `row`, which is the section a cursor is in.
pub fn at(regions: &[Region], row: usize) -> Option<&Region> {
    regions
        .iter()
        .filter(|r| row >= r.head && row < r.end.max(r.body))
        .max_by_key(|r| r.head)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(id: &str, level: u8, head: usize, body: usize, end: usize) -> Region {
        Region {
            id: id.into(),
            level,
            head,
            body,
            end,
        }
    }

    /// The case that motivated the seam: a block ends where it closes, and what
    /// follows it is not inside it.
    #[test]
    fn a_region_hides_only_what_it_covers() {
        // if a { x() }  y()
        let rs = vec![region("if", 3, 1, 2, 4)];
        assert_eq!(visible(6, &rs, &["if".into()]), vec![0, 1, 4, 5]);
        assert_eq!(visible(6, &rs, &[]), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_region_inside_a_shut_one_does_not_hide_twice() {
        let rs = vec![region("outer", 1, 0, 1, 8), region("inner", 2, 2, 3, 5)];
        let shut = active(&rs, &["outer".into(), "inner".into()]);
        assert_eq!(shut.len(), 1, "only the outer one does any hiding");
        assert_eq!(shut[0].id, "outer");
        assert_eq!(visible(8, &rs, &["outer".into(), "inner".into()]), vec![0]);
    }

    #[test]
    fn revealing_opens_every_fold_over_a_row() {
        let rs = vec![region("outer", 1, 0, 1, 8), region("inner", 2, 2, 3, 5)];
        let mut shut = vec![String::from("outer"), String::from("inner")];
        assert!(reveal(&rs, &mut shut, 4), "row 4 was hidden twice over");
        assert!(shut.is_empty(), "both were opened: {shut:?}");
        assert!(!reveal(&rs, &mut shut, 4), "and again changes nothing");
    }

    #[test]
    fn a_count_sits_on_the_last_row_the_fold_keeps() {
        let rs = vec![region("f", 1, 0, 2, 9)];
        assert_eq!(counts(&rs, &["f".into()], Note::LastOwn), vec![(1, 7)]);
        assert_eq!(counts(&rs, &["f".into()], Note::Head), vec![(0, 7)], "prose");
        // A fold that hides nothing says nothing.
        let empty = vec![region("g", 1, 0, 1, 1)];
        assert!(counts(&empty, &["g".into()], Note::LastOwn).is_empty());
    }

    /// The prose model, end to end: these moved here with the behaviour when
    /// `collapse` stopped owning it.
    #[test]
    fn a_section_runs_to_the_next_heading_of_equal_or_shallower_level() {
        use crate::render::{HeadingLine, LineKind, Span};
        let head = |level: u8, id: &str| Line {
            spans: vec![Span::plain(id)],
            block: 0,
            source_line: 1,
            heading: Some(HeadingLine {
                level,
                id: id.into(),
                text: id.into(),
                summarised: false,
            }),
            scroll: false,
            kind: LineKind::Heading,
        };
        let body = || Line {
            spans: vec![Span::plain("x")],
            block: 0,
            source_line: 1,
            heading: None,
            scroll: false,
            kind: LineKind::Paragraph,
        };
        // # A / body / ## B / body / # C
        let lines = vec![head(1, "a"), body(), head(2, "b"), body(), head(1, "c")];
        let rs = Sections.regions(&lines);
        assert_eq!(rs.len(), 3);
        assert_eq!((rs[0].head, rs[0].end), (0, 4), "A owns B and both bodies");
        assert_eq!((rs[1].head, rs[1].end), (2, 4), "B stops at the next level-1");
        // Folding A hides everything under it, nested heading included.
        assert_eq!(visible(5, &rs, &["a".into()]), vec![0, 4]);
    }

    #[test]
    fn the_innermost_region_owns_the_row() {
        let rs = vec![region("outer", 1, 0, 1, 8), region("inner", 2, 2, 3, 5)];
        assert_eq!(at(&rs, 4).map(|r| r.id.as_str()), Some("inner"));
        assert_eq!(at(&rs, 6).map(|r| r.id.as_str()), Some("outer"));
        assert_eq!(at(&rs, 99), None);
    }
}
