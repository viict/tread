//! Python: the lexer.
//!
//! The literal forms are the whole difficulty. A **triple-quoted** string spans
//! lines and may contain single quotes freely, so `'''` must be tested before
//! `'`; and a literal may carry a prefix — `r`, `f`, `b`, `rb`, `f'''` — which
//! has to be stepped over to find the quote that opens it.
//!
//! Blocks are indentation, not braces, so nothing here counts a `{` for
//! structure — brackets are tracked only to know whether the file is whole, and
//! by `super::py_decl` to know when a parameter list is still open.
#![deny(unsafe_code)]

use super::scan::{Cursor, Span, Tok};

/// Classify `src` into tokens that tile it end to end.
pub fn lex(src: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut c = Cursor::new(src);
    let mut code_from = 0usize;

    while !c.done() {
        let start = c.at;
        let tok = if c.peek(0) == Some(b'#') {
            c.skip_line();
            Some(Tok::Line { doc: false })
        } else if string(&mut c) {
            Some(Tok::Str)
        } else {
            c.bump();
            None
        };
        if let Some(tok) = tok {
            if start > code_from {
                out.push(Span {
                    tok: Tok::Code,
                    start: code_from,
                    end: start,
                });
            }
            out.push(Span {
                tok,
                start,
                end: c.at,
            });
            code_from = c.at;
        }
    }
    if code_from < src.len() {
        out.push(Span {
            tok: Tok::Code,
            start: code_from,
            end: src.len(),
        });
    }
    out
}

/// A string literal at the cursor, prefix and all. False if this is not one.
fn string(c: &mut Cursor) -> bool {
    // `r`, `b`, `f`, `u`, and the two-letter combinations, in any case.
    let mut n = 0usize;
    while n < 2 {
        match c.peek(n) {
            Some(b) if matches!(b | 0x20, b'r' | b'b' | b'f' | b'u') => n += 1,
            _ => break,
        }
    }
    let quote = match c.peek(n) {
        Some(q @ (b'"' | b'\'')) => q,
        // The prefix letters were just an identifier.
        _ => return false,
    };
    let triple = c.peek(n + 1) == Some(quote) && c.peek(n + 2) == Some(quote);
    c.skip(n + if triple { 3 } else { 1 });
    match triple {
        true => consume_triple(c, quote),
        false => consume_single(c, quote),
    }
    true
}

/// Runs to the matching `'''`, across as many lines as it takes.
fn consume_triple(c: &mut Cursor, quote: u8) {
    while !c.done() {
        if c.peek(0) == Some(b'\\') {
            c.skip(2);
            continue;
        }
        if c.peek(0) == Some(quote) && c.peek(1) == Some(quote) && c.peek(2) == Some(quote) {
            c.skip(3);
            return;
        }
        c.bump();
    }
}

/// A one-line literal. A newline ends it — an unterminated quote must not
/// swallow the file.
fn consume_single(c: &mut Cursor, quote: u8) {
    while let Some(b) = c.bump() {
        match b {
            b'\\' => {
                c.bump();
            }
            b'\n' => return,
            b if b == quote => return,
            _ => {}
        }
    }
}

/// Whether the file lexed cleanly: brackets balanced, no literal left open.
///
/// Indentation is *not* checked. A file may be indented in ways this reader has
/// no opinion about, and refusing to outline it for that would be inventing a
/// rule Python itself does not have.
pub fn balanced(src: &str, toks: &[Span]) -> bool {
    let bytes = src.as_bytes();
    let (mut square, mut round, mut curly) = (0i32, 0i32, 0i32);
    for s in toks {
        match s.tok {
            Tok::Line { .. } => continue,
            Tok::Str => {
                // A triple-quoted literal that reached the end never closed.
                let text = &src[s.start..s.end];
                if s.end == src.len() && !closed_triple(text) {
                    return false;
                }
                continue;
            }
            _ => {}
        }
        for &b in &bytes[s.start..s.end] {
            match b {
                b'[' => square += 1,
                b']' => square -= 1,
                b'(' => round += 1,
                b')' => round -= 1,
                b'{' => curly += 1,
                b'}' => curly -= 1,
                _ => {}
            }
            if square < 0 || round < 0 || curly < 0 {
                return false;
            }
        }
    }
    square == 0 && round == 0 && curly == 0
}

/// Did a literal that runs to the end of the file actually close?
fn closed_triple(text: &str) -> bool {
    let body = text.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    for q in ["\"\"\"", "'''"] {
        if body.starts_with(q) {
            return body.len() >= 6 && body.ends_with(q);
        }
    }
    // A one-line literal: it either closed or ran to the newline, and either
    // way it did not eat the file.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(src: &str) -> String {
        lex(src)
            .into_iter()
            .filter(|s| s.tok == Tok::Code)
            .map(|s| &src[s.start..s.end])
            .collect()
    }

    fn ok(src: &str) -> bool {
        balanced(src, &lex(src))
    }

    #[test]
    fn tokens_tile_the_input() {
        let src = "x = 1  # note\ndef f():\n    return 'a'\n";
        let toks = lex(src);
        assert_eq!(toks[0].start, 0);
        assert_eq!(toks.last().unwrap().end, src.len());
        for w in toks.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    /// A docstring spans lines and may hold quotes and `#` freely.
    #[test]
    fn a_triple_quoted_string_spans_lines() {
        let src = "def f():\n    \"\"\"Doc with 'quotes' and # hash.\n\n    More.\n    \"\"\"\n    return 1\n";
        assert!(ok(src));
        let c = code(src);
        assert!(!c.contains("Doc with"), "the docstring is not code: {c}");
        assert!(c.contains("return 1"), "and the body after it is: {c}");
    }

    #[test]
    fn a_prefixed_literal_is_still_a_literal() {
        for src in ["s = r'\\d+'", "s = f\"{x}\"", "s = b'x'", "s = rb'''y'''", "s = F'z'"] {
            assert!(ok(src), "{src}");
            assert!(!code(src).contains('x') || src.contains("{x}"), "{src}");
        }
        // A bare `r` is an identifier, not a prefix.
        assert_eq!(code("r = 1"), "r = 1");
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let src = "s = '# not a comment'\nx = 1\n";
        assert!(code(src).contains("x = 1"));
        assert!(!code(src).contains("not a comment"));
    }

    #[test]
    fn brackets_must_balance_but_indentation_is_not_judged() {
        assert!(ok("f(\n  1,\n  2,\n)\n"));
        assert!(!ok("f(\n  1,\n"));
        assert!(!ok("s = '''open\n"));
        // Ragged indentation is Python's business, not this reader's.
        assert!(ok("def f():\n        x = 1\n"));
        assert!(ok(""));
    }

    #[test]
    fn hostile_input_never_panics() {
        for c in ["'", "\"\"\"", "'''", "r'", "f\"", "#", "\\", "(((", ")))", "rb'''", "\u{fffd}"] {
            let toks = lex(c);
            let _ = balanced(c, &toks);
            if !toks.is_empty() {
                assert_eq!(toks[0].start, 0, "{c:?}");
                assert_eq!(toks.last().unwrap().end, c.len(), "{c:?}");
            }
        }
    }
}
