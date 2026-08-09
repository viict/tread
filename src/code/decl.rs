//! The line arithmetic every language's declaration recognizer needs.
//!
//! Finding *where* a declaration begins is language-specific; everything after
//! that — how deep the braces are, where the body ends, which comments belong
//! to it — is the same for Rust and for TypeScript, and would be the same for
//! Go. Keeping one copy is not tidiness: two copies drift, and a drift here
//! means the same file folds differently depending on its extension.
//!
//! All of it runs over the *blanked* source ([`scan::blank`]), where a brace
//! cannot be quoted and a keyword cannot be inside a comment.
#![deny(unsafe_code)]

use super::scan::{blank, Span};
use super::{disambiguate, Kind, Symbol};

/// What a language's recognizer answers for one line: what is declared, and
/// what it is called.
///
/// Given the blanked line *and* the raw one. Detection must use the blanked
/// text — that is what stops a `function` inside a comment from being a
/// declaration — but a name that lives inside a string literal, like an import
/// path, has been blanked to spaces and can only be read from the raw line.
pub type Recognise = fn(&str, &str) -> Option<(Kind, String)>;

/// Kinds that contain other declarations, whose members nest one level under
/// them and are named `Container::member`.
pub fn is_container(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Impl | Kind::Trait | Kind::Class | Kind::Interface
    )
}

/// Walk `src` and build its symbols, given a language's lexer and recognizer.
///
/// `None` when the file does not lex cleanly — the safety valve that makes a
/// mis-read brace show up as "no outline" rather than as a body that swallows
/// the rest of the file (SPEC.md §Code).
pub fn symbols(
    src: &str,
    lex: fn(&str) -> Vec<Span>,
    balanced: fn(&str, &[Span]) -> bool,
    recognise: Recognise,
) -> Option<Vec<Symbol>> {
    let toks = lex(src);
    if !balanced(src, &toks) {
        return None;
    }
    let blanked = blank(src, &toks);
    let lines: Vec<&str> = blanked.lines().collect();
    // Depth and keywords come from the blanked text; doc comments have to come
    // from the raw text, because blanking is exactly what erased them.
    let raw: Vec<&str> = src.lines().collect();
    let depths = depths(&lines);

    let mut out: Vec<Symbol> = Vec::new();
    let mut container: Option<(String, i32)> = None;
    for i in 0..lines.len() {
        if let Some((_, d)) = &container {
            if depths[i] <= *d {
                container = None;
            }
        }
        let depth = depths[i];
        // Top level, or one level inside a container. Deeper is a local item —
        // a helper declared inside a function — which is noise in an outline.
        if depth > 1 || (depth == 1 && container.is_none()) {
            continue;
        }
        let Some((kind, name)) = recognise(lines[i], raw.get(i).copied().unwrap_or("")) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let (sig_end, body) = extent(&lines, &depths, i);
        let path = match &container {
            Some((c, _)) if !c.is_empty() => format!("{c}::{name}"),
            _ => name.clone(),
        };
        if is_container(kind) {
            container = Some((name.clone(), depth));
        }
        out.push(Symbol {
            kind,
            name,
            path,
            depth: (depth == 1) as u8,
            doc: doc_above(&raw, i),
            sig: (i, sig_end),
            body,
        });
    }
    disambiguate(&mut out);
    Some(out)
}

/// Brace depth at the *start* of each line.
pub fn depths(lines: &[&str]) -> Vec<i32> {
    let mut out = Vec::with_capacity(lines.len());
    let mut d = 0i32;
    for l in lines {
        out.push(d);
        for b in l.bytes() {
            match b {
                b'{' => d += 1,
                b'}' => d -= 1,
                _ => {}
            }
        }
    }
    out
}

/// Where the signature ends, and what the body covers.
///
/// Bracket depth is tracked *within* the line, not just across lines, because
/// the two shapes that matter are told apart by exactly that:
///
/// ```text
/// fn f(                          const BINDINGS: &[Binding] = &[
///     a: u8,                         Binding { … },
/// ) -> u8 {   <- a body opens         Binding { … },   <- not a body
/// ```
///
/// A `{` opens the body only at depth zero. Reading the *first* `{` on any line
/// as the opener makes a declaration whose initializer contains braces — an
/// array of structs, an object literal — end at its first element, folding one
/// entry and stranding the rest.
///
/// When no `{` opens at depth zero, the initializer itself is the body, so a
/// long array folds behind its `const` line rather than filling the screen.
pub fn extent(lines: &[&str], _depths: &[i32], i: usize) -> (usize, (usize, usize)) {
    let mut depth = 0i32;
    let mut opener: Option<usize> = None;
    let mut first_open: Option<usize> = None;
    for (n, line) in lines.iter().enumerate().skip(i) {
        for b in line.bytes() {
            match b {
                b'{' => {
                    if depth == 0 && opener.is_none() {
                        opener = Some(n);
                    }
                    depth += 1;
                }
                b'(' | b'[' => depth += 1,
                b'}' | b')' | b']' => depth -= 1,
                _ => {}
            }
            if depth > 0 && first_open.is_none() {
                first_open = Some(n);
            }
        }
        if depth > 0 {
            continue;
        }
        // Everything opened on this declaration has closed.
        if let Some(o) = opener {
            return (o + 1, (o + 1, n + 1));
        }
        if line.contains(';') {
            return match first_open {
                // `= &[ … ];` — the initializer is what folds.
                Some(f) if f < n => (f + 1, (f + 1, n + 1)),
                _ => (n + 1, (n + 1, n + 1)),
            };
        }
    }
    (i + 1, (i + 1, i + 1))
}

/// The contiguous comments and attributes immediately above line `i`.
///
/// Reads the *raw* lines. Ordinary comments count alongside doc comments — a
/// reader wants the note someone left above a function, and the compiler's
/// opinion about which comments are documentation is not the reader's.
pub fn doc_above(raw: &[&str], i: usize) -> (usize, usize) {
    let mut start = i;
    while start > 0 {
        let prev = raw[start - 1].trim_start();
        let is_doc = prev.starts_with("//")
            || prev.starts_with('#')
            || prev.starts_with("/*")
            || prev.starts_with('*')
            || prev.starts_with('@'); // a decorator belongs to what follows
        if !is_doc {
            break;
        }
        start -= 1;
    }
    (start, i)
}

/// `word(s, "fn")` matches `fn` only as a whole word, returning what follows.
pub fn word<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    starts_word(s, kw).then(|| s[kw.len()..].trim_start())
}

pub fn starts_word(s: &str, kw: &str) -> bool {
    s.starts_with(kw) && !s[kw.len()..].starts_with(is_ident_char)
}

pub fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The leading identifier of `s`, or empty when it does not start with one.
pub fn ident(s: &str) -> String {
    let s = s.trim_start();
    let end = s.find(|c: char| !is_ident_char(c)).unwrap_or(s.len());
    s[..end].to_string()
}

/// A foldable block inside a body: `(head line, first hidden line, end)`.
pub type Block = (usize, usize, usize);

/// Every brace-delimited block in `lines`, innermost included.
///
/// This is what lets a reader collapse a branch rather than a whole function.
/// A block *ends where it closes*, which is why regions are stated rather than
/// inferred from the next heading (`source::fold`).
///
/// Blocks shorter than `min` are skipped: a two-line `if` folds to a marker
/// that is no shorter than what it hid, and a file of them reads as noise.
pub fn blocks(lines: &[&str], min: usize) -> Vec<Block> {
    let mut out = Vec::new();
    // Lines where a `{` opened, innermost last.
    let mut open: Vec<usize> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        for b in line.bytes() {
            match b {
                b'{' => open.push(n),
                b'}' => {
                    if let Some(head) = open.pop() {
                        // A block entirely on one line hides nothing.
                        if n > head && n - head >= min {
                            out.push((head, head + 1, n + 1));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_by_key(|(h, _, _)| *h);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_is_counted_at_the_start_of_each_line() {
        let lines = vec!["fn a() {", "    x", "}", "fn b() {}"];
        assert_eq!(depths(&lines), vec![0, 1, 1, 0]);
    }

    #[test]
    fn a_body_runs_to_the_line_that_closes_it() {
        let lines = vec!["fn a() {", "    x", "}", ""];
        let d = depths(&lines);
        // Signature ends after line 0; body is lines 1..3, closing brace included.
        assert_eq!(extent(&lines, &d, 0), (1, (1, 3)));
    }

    /// The bug this replaced: an initializer full of braces ended the
    /// declaration at its first element, folding one entry of an array and
    /// leaving the other two hundred on screen.
    #[test]
    fn an_initializer_full_of_braces_folds_as_one() {
        let lines = vec![
            "const BINDINGS: &[Binding] = &[",
            "    Binding {",
            "        keys: \"j\",",
            "    },",
            "    Binding {",
            "        keys: \"k\",",
            "    },",
            "];",
            "",
        ];
        let d = depths(&lines);
        // The signature is the `const` line; the whole array is the body.
        assert_eq!(extent(&lines, &d, 0), (1, (1, 8)));
    }

    /// ...while a `{` that really does open a body is still found, even when
    /// the signature spans lines before it.
    #[test]
    fn a_multi_line_signature_still_finds_its_body() {
        let lines = vec!["fn f(", "    a: u8,", ") -> u8 {", "    a", "}", ""];
        let d = depths(&lines);
        assert_eq!(extent(&lines, &d, 0), (3, (3, 5)), "params stay visible");
    }

    #[test]
    fn a_declaration_with_no_body_ends_at_its_semicolon() {
        let lines = vec!["use a::b;", ""];
        let d = depths(&lines);
        assert_eq!(extent(&lines, &d, 0), (1, (1, 1)));
    }

    #[test]
    fn comments_and_decorators_above_a_declaration_belong_to_it() {
        let raw = vec!["", "// note", "/// doc", "#[attr]", "@Component", "fn f() {}"];
        assert_eq!(doc_above(&raw, 5), (1, 5));
        assert_eq!(doc_above(&raw, 1), (1, 1), "a blank line stops it");
    }

    #[test]
    fn blocks_are_found_innermost_and_short_ones_skipped() {
        let lines = vec![
            "fn f() {",      // 0
            "    if a {",    // 1
            "        x();",  // 2
            "        y();",  // 3
            "    }",         // 4
            "    z();",      // 5
            "}",             // 6
            "fn g() { h() }", // 7 — one line, hides nothing
        ];
        let got = blocks(&lines, 2);
        assert_eq!(got, vec![(0, 1, 7), (1, 2, 5)], "the fn and the if, not the one-liner");
        // `z()` is outside the `if`: a block ends where it closes.
        let (h, b, e) = got[1];
        assert!(5 >= e || 5 < b, "row 5 is not inside {h}..{e}");
        // A higher floor drops the small one.
        assert_eq!(blocks(&lines, 5), vec![(0, 1, 7)]);
    }

    #[test]
    fn words_match_whole_and_identifiers_stop_at_punctuation() {
        assert!(starts_word("fn f()", "fn"));
        assert!(!starts_word("fnord()", "fn"));
        assert_eq!(word("fn  f()", "fn"), Some("f()"));
        assert_eq!(ident("  name(x)"), "name");
        assert_eq!(ident("(x)"), "");
    }
}
