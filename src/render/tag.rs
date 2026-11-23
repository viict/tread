//! Status tags in a heading: `[PROPOSED]`, `[BLOCKED: waiting on infra]`
//! (SPEC.md §Headings).
//!
//! A tag is coloured whole — brackets, keyword and text — in the colour of its
//! status, so a blocked section is visible from across the screen. Only a
//! keyword `theme::tag_style` knows is a tag; `[FOO]` and `[1]` stay text.
//!
//! The scan runs over atoms, after `flatten`, because that is the one form a
//! heading row and an outline row share: a tag with text spans several words,
//! and an outline row has no AST left to scan.
#![deny(unsafe_code)]

use super::inline::{atoms_to_line, flatten, Atom};
use super::Span;
use crate::md::ast::Inline;
use crate::term::Style;
use crate::theme;

/// `text` as one line in `base`, with its status tags coloured: an outline row,
/// which keeps a heading's text but not its inlines.
pub fn tagged(text: &str, base: Style) -> Vec<Span> {
    let mut atoms = flatten(&[Inline::Text(text.into())], base);
    tint(&mut atoms);
    atoms_to_line(atoms)
}

/// Recolour every status tag in `atoms`, in place.
pub(crate) fn tint(atoms: &mut Vec<Atom>) {
    let mut i = 0;
    while i < atoms.len() {
        i = match tag_at(atoms, i) {
            Some((style, end, cut)) => paint(atoms, i, end, cut, style),
            None => i + 1,
        };
    }
}

/// The tag opening at word `i`: its style, the atom that closes it, and the
/// byte just past that atom's `]`.
fn tag_at(atoms: &[Atom], i: usize) -> Option<(Style, usize, usize)> {
    let open = plain_word(atoms.get(i)?)?.strip_prefix('[')?;
    let len = open
        .find(|c: char| !(c.is_ascii_alphabetic() || c == '-' || c == '_'))
        .unwrap_or(open.len());
    let style = theme::tag_style(&open[..len])?;
    match open[len..].chars().next()? {
        ']' => Some((style, i, 1 + len + 1)),
        ':' => close(atoms, i, 1 + len + 1).map(|(end, cut)| (style, end, cut)),
        _ => None,
    }
}

/// The first `]` at or after byte `from` of word `i`. A line break or a link
/// ends the search: a tag never runs across either.
fn close(atoms: &[Atom], i: usize, from: usize) -> Option<(usize, usize)> {
    let mut skip = from;
    for (k, atom) in atoms.iter().enumerate().skip(i) {
        match atom {
            Atom::Break => return None,
            Atom::Space(_, link) if link.is_some() => return None,
            Atom::Space(..) => {}
            Atom::Word(_) => {
                let text = plain_word(atom)?;
                if let Some(at) = text.get(skip..).and_then(|t| t.find(']')) {
                    return Some((k, skip + at + 1));
                }
            }
        }
        skip = 0;
    }
    None
}

/// Restyle atoms `i..=end`, splitting `end` after its `]` so what follows keeps
/// the heading's own style. Returns the index to resume scanning from.
fn paint(atoms: &mut Vec<Atom>, i: usize, end: usize, cut: usize, style: Style) -> usize {
    for atom in &mut atoms[i..end] {
        match atom {
            Atom::Word(s) => s.style = style,
            Atom::Space(st, _) => *st = style,
            Atom::Break => {}
        }
    }
    let Atom::Word(last) = &mut atoms[end] else {
        return end + 1;
    };
    let rest = last.text.split_off(cut);
    let tail = Span::new(rest, last.style);
    last.style = style;
    if tail.text.is_empty() {
        return end + 1;
    }
    // No space between them, so the wrapper keeps the two as one cluster.
    atoms.insert(end + 1, Atom::Word(tail));
    end + 1
}

/// A word's text, when it is not part of a link: a link's text is the link's.
fn plain_word(atom: &Atom) -> Option<&str> {
    match atom {
        Atom::Word(s) if s.link.is_none() => Some(&s.text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tinted(text: &str) -> Vec<Span> {
        tagged(text, Style::new())
    }

    fn fg_of(spans: &[Span], text: &str) -> Option<u8> {
        spans.iter().find(|s| s.text == text).and_then(|s| s.style.fg)
    }

    #[test]
    fn a_bare_tag_takes_its_status_colour() {
        let s = tinted("Auth rewrite [PROPOSED]");
        assert_eq!(fg_of(&s, "[PROPOSED]"), Some(theme::STATUS_OPEN));
        assert_eq!(fg_of(&s, "Auth rewrite "), None, "the title keeps its style: {s:?}");
        let s = tinted("Cache [BLOCKED]");
        assert_eq!(fg_of(&s, "[BLOCKED]"), Some(theme::STATUS_BLOCKED));
    }

    #[test]
    fn a_tag_with_text_is_coloured_whole() {
        let s = tinted("Cache [BLOCKED: waiting on infra] now");
        assert_eq!(fg_of(&s, "[BLOCKED: waiting on infra]"), Some(theme::STATUS_BLOCKED), "{s:?}");
        assert_eq!(fg_of(&s, " now"), None, "{s:?}");
    }

    #[test]
    fn the_keyword_ignores_case_and_punctuation_after_the_tag_is_plain() {
        let s = tinted("[done], then more");
        assert_eq!(fg_of(&s, "[done]"), Some(theme::STATUS_LIVE), "{s:?}");
        assert_eq!(fg_of(&s, ", then more"), None, "{s:?}");
    }

    #[test]
    fn two_tags_in_one_heading_both_colour() {
        let s = tinted("[WIP] Search [BLOCKED: index]");
        assert_eq!(fg_of(&s, "[WIP]"), Some(theme::STATUS_OPEN), "{s:?}");
        assert_eq!(fg_of(&s, "[BLOCKED: index]"), Some(theme::STATUS_BLOCKED), "{s:?}");
    }

    #[test]
    fn what_is_not_a_status_tag_stays_text() {
        for text in ["Thing [FOO]", "Notes [1]", "[PROPOSED", "[BLOCKED: never closed", "[ BLOCKED]"] {
            let s = tinted(text);
            assert!(s.iter().all(|x| x.style.fg.is_none()), "{text:?} -> {s:?}");
        }
        // The keyword must be followed by `]` or `:`, not by more of a word.
        assert!(tinted("[DONEish]").iter().all(|x| x.style.fg.is_none()));
    }

    #[test]
    fn a_tag_inside_a_link_stays_the_link() {
        let mut atoms = flatten(
            &[Inline::Link {
                text: vec![Inline::Text("[PROPOSED]".into())],
                url: "x.md".into(),
                title: None,
            }],
            Style::new(),
        );
        let before = atoms.clone();
        tint(&mut atoms);
        assert_eq!(atoms, before);
    }

    #[test]
    fn a_tag_never_runs_across_a_line_break() {
        let mut atoms = flatten(
            &[Inline::Text("[BLOCKED: a".into()), Inline::HardBreak, Inline::Text("b]".into())],
            Style::new(),
        );
        let before = atoms.clone();
        tint(&mut atoms);
        assert_eq!(atoms, before);
    }
}
