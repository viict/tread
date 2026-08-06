//! Greedy word wrapping over styled atoms, with hanging indents.
//!
//! The first output line may have a different width than the rest (a bullet or
//! quote marker occupies the first line's prefix), which is what gives list
//! items a hanging indent aligned to their text.
#![deny(unsafe_code)]

use super::inline::{push_span, Atom};
use super::width::{char_width, str_width};
use super::Span;

/// Wrap `atoms` into lines of spans. `first` and `rest` are the available
/// widths of the first and subsequent lines. Returns at least one line.
pub(crate) fn wrap(atoms: &[Atom], first: usize, rest: usize) -> Vec<Vec<Span>> {
    let mut w = Wrapper::new(first.max(1), rest.max(1));
    for a in atoms {
        match a {
            Atom::Break => w.hard_break(),
            Atom::Space(st, url) => w.space(*st, url.as_deref()),
            Atom::Word(s) => w.word(s),
        }
    }
    w.finish()
}

struct Wrapper {
    rest: usize,
    avail: usize,
    used: usize,
    cur: Vec<Span>,
    /// Words with no space between them (`(`, a code span, `)`): there is no
    /// break opportunity inside a cluster, so it is placed as one unit.
    cluster: Vec<Span>,
    cluster_w: usize,
    pending: Option<Span>,
    out: Vec<Vec<Span>>,
}

impl Wrapper {
    fn new(first: usize, rest: usize) -> Self {
        Wrapper {
            rest,
            avail: first,
            used: 0,
            cur: Vec::new(),
            cluster: Vec::new(),
            cluster_w: 0,
            pending: None,
            out: Vec::new(),
        }
    }

    fn newline(&mut self) {
        self.pending = None;
        self.out.push(std::mem::take(&mut self.cur));
        self.used = 0;
        self.avail = self.rest;
    }

    fn hard_break(&mut self) {
        self.flush_cluster();
        self.newline();
    }

    /// Spaces are held back so they never survive at the end of a line.
    fn space(&mut self, style: crate::term::Style, url: Option<&str>) {
        self.flush_cluster();
        if !self.cur.is_empty() {
            self.pending = Some(Span {
                text: " ".into(),
                style: super::inline::space_style(style),
                link: url.map(str::to_string),
            });
        }
    }

    fn take_pending(&mut self) {
        if let Some(s) = self.pending.take() {
            self.used += 1;
            push_span(&mut self.cur, s);
        }
    }

    fn word(&mut self, s: &Span) {
        self.cluster_w += str_width(&s.text);
        self.cluster.push(s.clone());
    }

    /// Place the accumulated cluster, breaking before it if it does not fit.
    fn flush_cluster(&mut self) {
        if self.cluster.is_empty() {
            return;
        }
        let w = self.cluster_w;
        let sep = usize::from(self.pending.is_some());
        if !self.cur.is_empty() && self.used + sep + w > self.avail {
            self.newline();
        }
        self.take_pending();
        let cluster = std::mem::take(&mut self.cluster);
        self.cluster_w = 0;
        if w <= self.avail.saturating_sub(self.used) {
            self.used += w;
            for s in cluster {
                push_span(&mut self.cur, s);
            }
            return;
        }
        for s in &cluster {
            self.hard_split(s);
        }
    }

    /// A cluster wider than the line: split it at char boundaries.
    fn hard_split(&mut self, s: &Span) {
        let mut chunk = String::new();
        let mut used = self.used;
        for c in s.text.chars() {
            let cw = char_width(c);
            if cw > 0 && used + cw > self.avail {
                push_span(&mut self.cur, respan(s, std::mem::take(&mut chunk)));
                self.newline();
                used = 0;
            }
            chunk.push(c);
            used += cw;
        }
        if !chunk.is_empty() {
            push_span(&mut self.cur, respan(s, chunk));
        }
        self.used = used;
    }

    fn finish(mut self) -> Vec<Vec<Span>> {
        self.flush_cluster();
        if !self.cur.is_empty() || self.out.is_empty() {
            self.out.push(std::mem::take(&mut self.cur));
        }
        self.out
    }
}

fn respan(model: &Span, text: String) -> Span {
    Span { text, style: model.style, link: model.link.clone() }
}

/// Prepend `prefix` to `line`, merging where styles allow.
pub(crate) fn with_prefix(prefix: &[Span], line: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(prefix.len() + line.len());
    for p in prefix {
        push_span(&mut out, p.clone());
    }
    for s in line {
        push_span(&mut out, s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::md::ast::Inline;
    use super::super::inline::flatten;
    use crate::term::Style;

    fn lay(src: &str, first: usize, rest: usize) -> Vec<String> {
        let atoms = flatten(&[Inline::Text(src.into())], Style::new());
        wrap(&atoms, first, rest)
            .into_iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect()
    }

    #[test]
    fn wraps_on_word_boundaries() {
        assert_eq!(lay("alpha beta gamma", 11, 11), vec!["alpha beta", "gamma"]);
    }

    #[test]
    fn no_trailing_space_survives_a_break() {
        for l in lay("alpha beta gamma delta", 12, 12) {
            assert_eq!(l, l.trim_end());
        }
    }

    #[test]
    fn hanging_indent_uses_a_narrower_first_line() {
        assert_eq!(lay("aaa bbb ccc", 7, 11), vec!["aaa bbb", "ccc"]);
        assert_eq!(lay("aaa bbb ccc", 3, 7), vec!["aaa", "bbb ccc"]);
    }

    #[test]
    fn overlong_word_is_hard_split() {
        assert_eq!(lay("abcdefghij", 4, 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn overlong_word_after_text_starts_a_new_line() {
        assert_eq!(lay("hi abcdefghij", 4, 4), vec!["hi", "abcd", "efgh", "ij"]);
    }

    #[test]
    fn wide_chars_never_overflow_the_line() {
        let out = lay("\u{4e2d}\u{6587}\u{6d4b}\u{8bd5}", 5, 5);
        assert_eq!(out, vec!["\u{4e2d}\u{6587}", "\u{6d4b}\u{8bd5}"]);
        for l in &out {
            assert!(str_width(l) <= 5);
        }
    }

    #[test]
    fn wide_and_narrow_mixed_wraps_by_display_width() {
        // "中文" is one 4-column word: it does not fit after "ab " in 6 columns.
        let out = lay("ab \u{4e2d}\u{6587} cd", 6, 6);
        assert_eq!(out, vec!["ab", "\u{4e2d}\u{6587}", "cd"]);
        assert_eq!(lay("ab \u{4e2d}\u{6587} cd", 7, 7), vec!["ab \u{4e2d}\u{6587}", "cd"]);
    }

    #[test]
    fn combining_marks_stay_with_their_base() {
        let out = lay("ae\u{301}iou", 3, 3);
        assert_eq!(out, vec!["ae\u{301}i", "ou"]);
    }

    #[test]
    fn glued_atoms_are_not_split_apart() {
        // "(" and the code span next to it are separate atoms with no space
        // between them: they must move to the next line together.
        let atoms = flatten(
            &[
                Inline::Text("see (".into()),
                Inline::Code("verylongthing".into()),
                Inline::Text(")".into()),
            ],
            Style::new(),
        );
        let out: Vec<String> = wrap(&atoms, 18, 18)
            .into_iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert_eq!(out, vec!["see", "(verylongthing)"]);
    }

    #[test]
    fn a_cluster_wider_than_the_line_still_splits() {
        let atoms = flatten(
            &[Inline::Text("(".into()), Inline::Code("abcdefghij".into())],
            Style::new(),
        );
        let out: Vec<String> = wrap(&atoms, 4, 4)
            .into_iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect())
            .collect();
        assert_eq!(out, vec!["(abc", "defg", "hij"]);
    }

    #[test]
    fn hard_break_starts_a_line() {
        let atoms = flatten(&[Inline::Text("a".into())], Style::new());
        let mut all = atoms.clone();
        all.push(Atom::Break);
        all.extend(flatten(&[Inline::Text("b".into())], Style::new()));
        let out = wrap(&all, 40, 40);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn empty_input_yields_one_empty_line() {
        assert_eq!(wrap(&[], 10, 10), vec![Vec::<Span>::new()]);
    }

    #[test]
    fn prefix_merges_and_precedes() {
        let line = vec![Span::plain("x")];
        let out = with_prefix(&[Span::plain("  ")], line);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "  x");
    }
}
