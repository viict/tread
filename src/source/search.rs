//! Incremental search over the ANSI-stripped text of the rendered lines.
//!
//! Matching is smartcase (case-insensitive unless the query contains an
//! uppercase character) and runs over *every* line, including lines currently
//! hidden by a fold — the pager expands the fold to reveal a hit.
#![deny(unsafe_code)]

use crate::render::{char_width, Line};

/// One match, in display columns of its row (so highlighting lines up with the
/// painted spans, gutter included).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Direction of the last `/` or `?`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Forward,
    Backward,
}

impl Dir {
    pub fn flip(self) -> Dir {
        match self {
            Dir::Forward => Dir::Backward,
            Dir::Backward => Dir::Forward,
        }
    }
}

/// SPEC: case-sensitive only when the query itself carries an uppercase char.
pub fn case_sensitive(query: &str) -> bool {
    query.chars().any(|c| c.is_uppercase())
}

/// Column ranges of every (non-overlapping) occurrence of `query` in `text`.
pub fn find_in(text: &str, query: &str, sensitive: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let hay: Vec<char> = match sensitive {
        true => text.chars().collect(),
        false => text.chars().flat_map(|c| c.to_lowercase()).collect(),
    };
    let needle: Vec<char> = match sensitive {
        true => query.chars().collect(),
        false => query.chars().flat_map(|c| c.to_lowercase()).collect(),
    };
    if needle.is_empty() || needle.len() > hay.len() {
        return out;
    }
    // Column of each char index, so matches are reported in display columns.
    let mut cols = Vec::with_capacity(hay.len() + 1);
    let mut col = 0;
    for c in &hay {
        cols.push(col);
        col += char_width(*c);
    }
    cols.push(col);
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push((cols[i], cols[i + needle.len()]));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// Every match in the document, ordered by `(line, column)`.
pub fn find_all(lines: &[Line], query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let sensitive = case_sensitive(query);
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        for (start, end) in find_in(&l.text(), query, sensitive) {
            out.push(Match { line: i, start, end });
        }
    }
    out
}

/// Index of the match after (`Forward`) or before (`Backward`) the position
/// `(line, col)`, plus whether the search wrapped around the document.
///
/// `matches` must be sorted, which [`find_all`] guarantees.
pub fn seek(
    matches: &[Match],
    line: usize,
    col: usize,
    dir: Dir,
    inclusive: bool,
) -> Option<(usize, bool)> {
    if matches.is_empty() {
        return None;
    }
    let after = |m: &Match| (m.line, m.start) > (line, col) || (inclusive && m.line == line && m.start >= col);
    match dir {
        Dir::Forward => match matches.iter().position(after) {
            Some(i) => Some((i, false)),
            None => Some((0, true)),
        },
        Dir::Backward => {
            let before = |m: &Match| {
                (m.line, m.start) < (line, col) || (inclusive && m.line == line && m.start <= col)
            };
            match matches.iter().rposition(before) {
                Some(i) => Some((i, false)),
                None => Some((matches.len() - 1, true)),
            }
        }
    }
}

/// Matches that fall on `line`, as `(start, end)` column ranges.
pub fn on_line(matches: &[Match], line: usize) -> Vec<(usize, usize)> {
    matches
        .iter()
        .filter(|m| m.line == line)
        .map(|m| (m.start, m.end))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md;
    use crate::render::{render_document, RenderOpts};

    fn lines(src: &str) -> Vec<Line> {
        render_document(&md::parse(src), &RenderOpts::new(60))
    }

    #[test]
    fn smartcase_rules() {
        assert!(!case_sensitive("model"));
        assert!(case_sensitive("Model"));
        assert!(!case_sensitive("dns-1"));
    }

    #[test]
    fn lowercase_query_matches_any_case() {
        assert_eq!(find_in("Alpha alpha", "alpha", false), vec![(0, 5), (6, 11)]);
        assert_eq!(find_in("Alpha alpha", "Alpha", true), vec![(0, 5)]);
    }

    #[test]
    fn matches_are_reported_in_display_columns() {
        // Wide CJK chars are two columns each.
        assert_eq!(find_in("\u{4e2d}\u{6587}ab", "ab", false), vec![(4, 6)]);
        assert_eq!(find_in("ab\u{4e2d}", "\u{4e2d}", false), vec![(2, 4)]);
    }

    #[test]
    fn overlapping_occurrences_do_not_double_count() {
        assert_eq!(find_in("aaaa", "aa", false), vec![(0, 2), (2, 4)]);
        assert!(find_in("abc", "", false).is_empty());
        assert!(find_in("a", "abc", false).is_empty());
    }

    #[test]
    fn document_matches_are_ordered() {
        let ls = lines("# One\n\nneedle here\n\n## Two\n\nanother needle\n");
        let ms = find_all(&ls, "needle");
        assert_eq!(ms.len(), 2);
        assert!(ms[0].line < ms[1].line);
        assert!(ms.windows(2).all(|w| (w[0].line, w[0].start) < (w[1].line, w[1].start)));
    }

    #[test]
    fn search_finds_text_that_a_fold_would_hide() {
        let ls = lines("## A\n\nhidden needle\n\n## B\n");
        let ms = find_all(&ls, "needle");
        assert_eq!(ms.len(), 1);
        use crate::source::fold::{Folds, Sections};
        let shut = vec![ls
            .iter()
            .find_map(|l| l.heading.as_ref().map(|h| h.id.clone()))
            .unwrap()];
        let regions = Sections.regions(&ls);
        let folds = crate::source::fold::active(&regions, &shut);
        // The hit is inside the folded range: the pager must expand to show it.
        assert!(ms[0].line >= folds[0].body && ms[0].line < folds[0].end);
    }

    fn m(line: usize, start: usize) -> Match {
        Match { line, start, end: start + 1 }
    }

    #[test]
    fn forward_seek_wraps_at_the_end() {
        let ms = vec![m(1, 0), m(4, 2), m(4, 8)];
        assert_eq!(seek(&ms, 0, 0, Dir::Forward, false), Some((0, false)));
        assert_eq!(seek(&ms, 1, 0, Dir::Forward, false), Some((1, false)));
        assert_eq!(seek(&ms, 4, 2, Dir::Forward, false), Some((2, false)));
        assert_eq!(seek(&ms, 4, 8, Dir::Forward, false), Some((0, true)));
    }

    #[test]
    fn backward_seek_wraps_at_the_start() {
        let ms = vec![m(1, 0), m(4, 2), m(4, 8)];
        assert_eq!(seek(&ms, 4, 8, Dir::Backward, false), Some((1, false)));
        assert_eq!(seek(&ms, 1, 0, Dir::Backward, false), Some((2, true)));
        assert_eq!(seek(&ms, 0, 0, Dir::Backward, false), Some((2, true)));
    }

    #[test]
    fn inclusive_seek_keeps_a_match_on_the_current_line() {
        let ms = vec![m(3, 5)];
        assert_eq!(seek(&ms, 3, 5, Dir::Forward, true), Some((0, false)));
        assert_eq!(seek(&ms, 3, 5, Dir::Forward, false), Some((0, true)));
        assert_eq!(seek(&[], 0, 0, Dir::Forward, true), None);
    }

    #[test]
    fn per_line_highlight_ranges() {
        let ms = vec![m(2, 1), m(2, 7), m(5, 0)];
        assert_eq!(on_line(&ms, 2), vec![(1, 2), (7, 8)]);
        assert!(on_line(&ms, 9).is_empty());
    }
}
