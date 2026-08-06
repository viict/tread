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

/// A folded region: the heading at `head`, whose own rows end at `body`, hiding
/// `body..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fold {
    pub head: usize,
    pub body: usize,
    pub end: usize,
}

impl Fold {
    pub fn hidden(&self) -> usize {
        self.end.saturating_sub(self.body)
    }
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
fn own_end(lines: &[Line], head: usize) -> usize {
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

/// The outermost folds implied by `collapsed`. Folds never overlap: a heading
/// nested inside a folded section is already hidden and contributes nothing.
pub fn folds(lines: &[Line], collapsed: &[String]) -> Vec<Fold> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let h = match &lines[i].heading {
            Some(h) if collapsed.contains(&h.id) => h,
            _ => {
                i += 1;
                continue;
            }
        };
        let body = own_end(lines, i);
        let end = section_end(lines, i, h.level).max(body);
        out.push(Fold { head: i, body, end });
        i = end.max(i + 1);
    }
    out
}

/// Indices of the lines that are on screen, in order.
pub fn visible_lines(lines: &[Line], collapsed: &[String]) -> Vec<usize> {
    let folds = folds(lines, collapsed);
    let mut out = Vec::with_capacity(lines.len());
    let mut f = 0;
    let mut i = 0;
    while i < lines.len() {
        if f < folds.len() && i == folds[f].body {
            i = folds[f].end;
            f += 1;
            continue;
        }
        if f < folds.len() && i >= folds[f].end {
            f += 1;
            continue;
        }
        out.push(i);
        i += 1;
    }
    out
}

/// How many lines each visible folded heading hides, as `(head, count)`.
pub fn fold_counts(lines: &[Line], collapsed: &[String]) -> Vec<(usize, usize)> {
    folds(lines, collapsed)
        .into_iter()
        .map(|f| (f.head, f.hidden()))
        .collect()
}

/// The heading (index into `lines`) at or above `line`.
pub fn heading_at_or_above(lines: &[Line], line: usize) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    let start = line.min(lines.len() - 1);
    (0..=start).rev().find(|i| lines[*i].heading.is_some())
}

/// Drop fold ids until `target` is visible, outermost fold first. Returns true
/// when something was expanded.
pub fn reveal(lines: &[Line], collapsed: &mut Vec<String>, target: usize) -> bool {
    let mut changed = false;
    loop {
        let fold = folds(lines, collapsed)
            .into_iter()
            .find(|f| target >= f.body && target < f.end);
        let fold = match fold {
            Some(f) => f,
            None => return changed,
        };
        let id = match &lines[fold.head].heading {
            Some(h) => h.id.clone(),
            None => return changed,
        };
        collapsed.retain(|c| *c != id);
        changed = true;
    }
}

/// Every heading id in the document, for `zM`.
pub fn all_ids(lines: &[Line]) -> Vec<String> {
    headings(lines).into_iter().map(|h| h.id).collect()
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
    fn folding_a_heading_hides_until_an_equal_level_heading() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let collapsed = vec![hs[0].id.clone()];
        let vis = visible_lines(&lines, &collapsed);
        let text: Vec<String> = vis.iter().map(|i| lines[*i].text().trim().to_string()).collect();
        assert!(text.iter().any(|t| t.contains("Alpha")));
        assert!(text.iter().any(|t| t.contains("Beta")));
        assert!(!text.iter().any(|t| t == "one two"));
        assert!(!text.iter().any(|t| t.contains("Alpha One")));
        assert!(text.iter().any(|t| t == "beta text"));
    }

    #[test]
    fn nested_fold_hides_only_its_own_subtree() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let collapsed = vec![hs[1].id.clone()];
        let vis = visible_lines(&lines, &collapsed);
        let text: Vec<String> = vis.iter().map(|i| lines[*i].text().trim().to_string()).collect();
        assert!(text.iter().any(|t| t == "one two"));
        assert!(text.iter().any(|t| t.contains("Alpha One")));
        assert!(!text.iter().any(|t| t == "deep text"));
        assert!(text.iter().any(|t| t == "beta text"));
    }

    #[test]
    fn the_heading_row_itself_stays_visible_with_its_rule() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let f = folds(&lines, &[hs[0].id.clone()]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].head, hs[0].index);
        // H2 emits a title row plus a rule row; both belong to the heading.
        assert!(f[0].body >= f[0].head + 2);
        assert!(f[0].hidden() > 0);
    }

    #[test]
    fn folds_do_not_overlap_when_parent_and_child_are_both_closed() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let collapsed = vec![hs[0].id.clone(), hs[1].id.clone()];
        let f = folds(&lines, &collapsed);
        assert_eq!(f.len(), 1, "nested fold must be absorbed by its parent");
        let vis = visible_lines(&lines, &collapsed);
        assert!(vis.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn fold_state_survives_relayout_at_a_different_width() {
        let wide = render(DOC, 100);
        let narrow = render(DOC, 24);
        let id = headings(&wide)[0].id.clone();
        assert_eq!(headings(&narrow)[0].id, id);
        let a = visible_lines(&wide, std::slice::from_ref(&id));
        let b = visible_lines(&narrow, std::slice::from_ref(&id));
        let shown = |ls: &[Line], v: &[usize]| -> Vec<String> {
            v.iter()
                .map(|i| ls[*i].text().trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        };
        assert!(shown(&wide, &a).iter().any(|t| t.contains("Beta")));
        assert!(shown(&narrow, &b).iter().any(|t| t.contains("Beta")));
        assert!(!shown(&narrow, &b).iter().any(|t| t == "one two"));
    }

    #[test]
    fn reveal_expands_the_enclosing_folds() {
        let lines = render(DOC, 40);
        let hs = headings(&lines);
        let mut collapsed = vec![hs[0].id.clone(), hs[1].id.clone()];
        let hidden = folds(&lines, &collapsed)[0].body;
        assert!(reveal(&lines, &mut collapsed, hidden));
        assert!(visible_lines(&lines, &collapsed).contains(&hidden));
        assert!(!reveal(&lines, &mut collapsed, hidden));
    }

    #[test]
    fn empty_document_folds_to_nothing() {
        let lines = render("", 40);
        assert!(headings(&lines).is_empty());
        assert!(visible_lines(&lines, &[]).is_empty());
        assert!(folds(&lines, &["x".to_string()]).is_empty());
        assert_eq!(heading_at_or_above(&lines, 0), None);
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

    #[test]
    fn collapsing_everything_leaves_only_heading_rows() {
        let lines = render(DOC, 40);
        let ids = all_ids(&lines);
        let vis = visible_lines(&lines, &ids);
        assert!(vis.len() < lines.len());
        let counts = fold_counts(&lines, &ids);
        assert!(!counts.is_empty());
        assert!(counts.iter().all(|(_, n)| *n > 0));
    }
}
