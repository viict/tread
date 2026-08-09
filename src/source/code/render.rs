//! Turning a file and its symbols into rows.
//!
//! Every line of the file becomes a row, in order — nothing is dropped. What
//! makes the collapsed view is *marking* the signature rows as headings: the
//! collapse tree then hides everything between one heading and the next of the
//! same or lower level, which for code is exactly a body.
//!
//! That is the whole trick, and it is why the two views need no second
//! renderer. "Raw source" is this same row list with every fold open; the
//! summary is it with every fold shut. One document, two fold states.
#![deny(unsafe_code)]

use super::paint::{self, Run};

/// A run of symbols that folds as one: the imports at the top of a file, which
/// are a wall of lines a reader scrolls past rather than reads.
#[derive(Clone, Debug)]
pub struct Group {
    /// Index of the first symbol in the run, and how many it covers.
    pub first: usize,
    pub count: usize,
    /// Fold id, and how the run reads in the outline.
    pub id: String,
    pub label: String,
    /// What the fold says when shut: `38 symbols from 12 modules`.
    pub note: String,
}
use crate::code::{Kind, Symbol};
use crate::render::{HeadingLine, Line, LineKind, Span};
use crate::theme;

/// Tab stop used when expanding code. Four matches how this crate is written
/// and, more to the point, is what the file's own alignment assumes.
const TAB: usize = 4;

/// Build the rows for `src`, marking each symbol's doc comment and signature
/// as the rows its heading owns.
///
/// The doc comment has to be *part of* the heading, not a line above it. The
/// collapse tree hides everything from the end of a heading's own rows to the
/// next heading, so a doc comment left outside would fall inside the *previous*
/// symbol's fold and vanish — precisely backwards, since the comments are what
/// the collapsed view exists to show. `collapse::own_end` walks rows that share
/// the heading's block and carry `LineKind::Heading`, so giving the doc comment
/// and the signature one block makes them all survive the fold.
/// `links` places a target on a byte range of a line: `(line, from, to, url)`.
/// `groups` names runs of symbols that fold together as one — the imports.
pub fn rows(
    lang: &str,
    src: &str,
    symbols: &[Symbol],
    links: &[(usize, usize, usize, String)],
    groups: &[Group],
) -> Vec<Line> {
    let painted = paint::runs(lang, src);
    let total = src.lines().count();
    // Per line: which symbol's block it belongs to, whether it is one of that
    // symbol's heading rows, and whether a heading starts there.
    let mut block = vec![0usize; total + 1];
    let mut is_head = vec![false; total + 1];
    let mut starts: Vec<Option<&Symbol>> = vec![None; total + 1];
    // A symbol inside a group but not at its head declares no heading of its
    // own: the whole run folds behind one.
    let swallowed = |n: usize| {
        groups
            .iter()
            .any(|g| n > g.first && n < g.first + g.count)
    };
    for (n, s) in symbols.iter().enumerate() {
        // The heading starts at the doc comment, never at the blank line above
        // it. A blank heading row is where the painter would put the fold's
        // `(N lines)`, stranding the count on an empty line instead of on the
        // signature. The blank is therefore hidden with the previous body, and
        // the collapsed view is tight — the muted comments separate it.
        let (from, _) = s.span();
        if !swallowed(n) {
            if let Some(slot) = starts.get_mut(from) {
                *slot = Some(s);
            }
        }
        // Only the run's head keeps heading rows; the rest become body, which
        // is what makes them fold away behind it.
        let head_rows = match swallowed(n) {
            true => from..from,
            false => from..s.sig.1.max(from),
        };
        for i in head_rows {
            if let (Some(b), Some(h)) = (block.get_mut(i), is_head.get_mut(i)) {
                *b = n + 1;
                *h = true;
            }
        }
        for i in s.body.0..s.body.1 {
            if let Some(b) = block.get_mut(i) {
                *b = n + 1;
            }
        }
    }
    src.lines()
        .enumerate()
        .map(|(i, text)| {
            let here: Vec<(usize, usize, &str)> = links
                .iter()
                .filter(|(l, ..)| *l == i)
                .map(|(_, a, b, u)| (*a, *b, u.as_str()))
                .collect();
            let mut line = plain_row(
                text,
                i,
                painted.get(i).map(Vec::as_slice).unwrap_or(&[]),
                &here,
            );
            line.block = block[i];
            if is_head[i] {
                line.kind = LineKind::Heading;
            }
            if let Some(s) = starts[i] {
                let group = groups.iter().find(|g| symbols.get(g.first).map(|h| h.span().0) == Some(i));
                line.heading = Some(HeadingLine {
                    // Depth 0 becomes level 1: a method inside an `impl` must
                    // nest under it, or folding the impl leaves its methods.
                    level: s.depth + 1,
                    id: match group {
                        Some(g) => g.id.clone(),
                        None => s.path.clone(),
                    },
                    text: match group {
                        Some(g) => g.label.clone(),
                        None => outline_text(s),
                    },
                    summarised: false,
                });
            }
            line
        })
        .collect()
}

/// How a symbol reads in the `o` outline: `fn sniff` , `impl DirSource`.
///
/// The kind is spelled out because a bare name loses what it is, and the
/// outline is the one place a reader is scanning types and functions together.
pub fn outline_text(s: &Symbol) -> String {
    match s.kind {
        // A `use` line is already its own path; labelling it twice is noise.
        Kind::Import => s.name.clone(),
        k => format!("{} {}", k.label(), s.name),
    }
}

/// An ordinary row: the source line, coloured, tabs expanded, never wrapped.
///
/// `scroll` is set for the same reason a fenced code block sets it — code must
/// not be reflowed, so a row wider than the viewport scrolls sideways instead
/// (SPEC.md §Code).
fn plain_row(text: &str, i: usize, runs: &[Run], links: &[(usize, usize, &str)]) -> Line {
    Line {
        spans: spans_for(text, runs, links),
        block: 0,
        source_line: i + 1,
        heading: None,
        scroll: true,
        kind: LineKind::Code,
    }
}

/// Split the line at every boundary the painter or a link introduces,
/// expanding tabs as we go.
///
/// Links are attached to spans rather than recorded as columns, because
/// `Line::links` derives the display column from the spans themselves — which
/// is the only way a link stays in the right place on a line containing tabs.
///
/// Tabs are expanded *here* rather than before highlighting: expansion changes
/// every byte offset after it, so colouring first and expanding second would
/// paint the wrong columns.
fn spans_for(text: &str, runs: &[Run], links: &[(usize, usize, &str)]) -> Vec<Span> {
    // Every offset where the style or the link target may change.
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for (a, b, _) in runs {
        cuts.push(*a);
        cuts.push(*b);
    }
    for (a, b, _) in links {
        cuts.push(*a);
        cuts.push(*b);
    }
    cuts.retain(|c| *c <= text.len() && text.is_char_boundary(*c));
    cuts.sort_unstable();
    cuts.dedup();

    let mut spans: Vec<Span> = Vec::new();
    let mut col = 0usize;
    for w in cuts.windows(2) {
        let (from, to) = (w[0], w[1]);
        let style = runs
            .iter()
            .find(|(a, b, _)| *a <= from && to <= *b)
            .map(|(_, _, s)| *s)
            .unwrap_or_else(theme::text);
        let link = links
            .iter()
            .find(|(a, b, _)| *a <= from && to <= *b)
            .map(|(_, _, u)| u.to_string());
        let t = expand_tabs(&text[from..to], col);
        col += str_cols(&t);
        spans.push(Span { text: t, style, link });
    }
    if spans.is_empty() {
        spans.push(Span::new(String::new(), theme::text()));
    }
    spans
}

/// Display columns of already-expanded text.
fn str_cols(s: &str) -> usize {
    crate::render::str_width(s)
}

/// Expand tabs to the next stop. A code file's alignment is built on tab stops,
/// and painting a tab as one space breaks every aligned comment in the file.
fn expand_tabs(text: &str, from_col: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut col = from_col;
    for c in text.chars() {
        match c {
            '\t' => {
                let n = TAB - (col % TAB);
                // Not `iter::repeat_n`: that is stable since 1.82 and this
                // crate builds on 1.75 (see `rust-version` in Cargo.toml).
                out.push_str(&" ".repeat(n));
                col += n;
            }
            _ => {
                out.push(c);
                col += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::rust_decl::symbols;

    #[test]
    fn every_line_of_the_file_becomes_a_row_in_order() {
        let src = "// top\nfn a() {\n    1\n}\nfn b() {}\n";
        let syms = symbols(src).unwrap();
        let rows = rows("rust", src, &syms, &[], &[]);
        assert_eq!(rows.len(), 5, "one row per line, nothing dropped");
        let text: Vec<String> = rows.iter().map(|r| r.text()).collect();
        assert_eq!(text[0], "// top");
        assert_eq!(text[2], "    1");
        // Source lines are 1-based and match the file.
        assert_eq!(rows[4].source_line, 5);
    }

    #[test]
    fn a_signature_row_is_a_heading_and_a_method_nests_under_its_impl() {
        let src = "impl S {\n    fn m(&self) {\n    }\n}\n";
        let rows = rows("rust", src, &symbols(src).unwrap(), &[], &[]);
        let h0 = rows[0].heading.as_ref().expect("impl is a heading");
        let h1 = rows[1].heading.as_ref().expect("method is a heading");
        assert_eq!((h0.level, h0.id.as_str()), (1, "S"));
        assert_eq!((h1.level, h1.id.as_str()), (2, "S::m"));
        assert!(rows[2].heading.is_none(), "a body line is not a heading");
    }

    #[test]
    fn the_outline_says_what_kind_of_symbol_it_is() {
        let src = "use a::b;\nstruct T;\nfn f() {}\n";
        let rows = rows("rust", src, &symbols(src).unwrap(), &[], &[]);
        let texts: Vec<&str> = rows
            .iter()
            .filter_map(|r| r.heading.as_ref().map(|h| h.text.as_str()))
            .collect();
        assert_eq!(texts, vec!["a::b", "type T", "fn f"]);
    }

    #[test]
    fn code_rows_scroll_rather_than_wrap_and_tabs_expand() {
        let rows = rows("rust", "\tx\n", &[], &[], &[]);
        assert!(rows[0].scroll, "code is never reflowed");
        assert_eq!(rows[0].text(), "    x");
        assert_eq!(expand_tabs("a\tb", 0), "a   b", "to the next stop, not four");
        assert_eq!(expand_tabs("plain", 0), "plain");
        assert_eq!(expand_tabs("\tx", 2), "  x", "a span starting mid-line");
    }
}
