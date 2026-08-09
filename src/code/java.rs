//! Java: the lexer.
//!
//! The simplest of the three so far — braces delimit blocks, comments do not
//! nest, and there is no regex ambiguity to resolve. Two things still need
//! care: a **text block** (`"""…"""`, Java 15) spans lines and may contain
//! quotes, and a char literal `'}'` is a brace that must not count.
#![deny(unsafe_code)]

use super::scan::{Cursor, Span, Tok};

/// Classify `src` into tokens that tile it end to end.
pub fn lex(src: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut c = Cursor::new(src);
    let mut code_from = 0usize;

    while !c.done() {
        let start = c.at;
        let tok = if c.at_str(b"//") {
            c.skip_line();
            Some(Tok::Line { doc: false })
        } else if c.at_str(b"/*") {
            // `/**` is javadoc, which is the comment a reader wants kept.
            let doc = c.peek(2) == Some(b'*');
            block_comment(&mut c);
            Some(Tok::Block { doc })
        } else if c.at_str(b"\"\"\"") {
            text_block(&mut c);
            Some(Tok::Str)
        } else if matches!(c.peek(0), Some(b'"') | Some(b'\'')) {
            quoted(&mut c);
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

/// `/* … */`. Unlike Rust's, these do not nest.
fn block_comment(c: &mut Cursor) {
    c.skip(2);
    while !c.done() {
        if c.at_str(b"*/") {
            c.skip(2);
            return;
        }
        c.bump();
    }
}

/// `"""… """` — a text block, which spans lines and may hold bare quotes.
fn text_block(c: &mut Cursor) {
    c.skip(3);
    while !c.done() {
        if c.at_str(b"\\") {
            c.skip(2);
            continue;
        }
        if c.at_str(b"\"\"\"") {
            c.skip(3);
            return;
        }
        c.bump();
    }
}

/// `"…"` or `'…'`, with escapes. A newline ends it: an unterminated literal
/// must not swallow the rest of the file.
fn quoted(c: &mut Cursor) {
    let quote = match c.bump() {
        Some(q) => q,
        None => return,
    };
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

/// Whether the file lexed cleanly: brackets balanced, nothing left open.
pub fn balanced(src: &str, toks: &[Span]) -> bool {
    let bytes = src.as_bytes();
    let (mut curly, mut square, mut round) = (0i32, 0i32, 0i32);
    for s in toks {
        match s.tok {
            Tok::Line { .. } => continue,
            Tok::Block { .. } => {
                if s.end == src.len() && !src[s.start..s.end].ends_with("*/") {
                    return false;
                }
                continue;
            }
            Tok::Str => {
                let text = &src[s.start..s.end];
                if s.end == src.len() && text.starts_with("\"\"\"") && !text.ends_with("\"\"\"") {
                    return false;
                }
                continue;
            }
            Tok::Code => {}
        }
        for &b in &bytes[s.start..s.end] {
            match b {
                b'{' => curly += 1,
                b'}' => curly -= 1,
                b'[' => square += 1,
                b']' => square -= 1,
                b'(' => round += 1,
                b')' => round -= 1,
                _ => {}
            }
            if curly < 0 || square < 0 || round < 0 {
                return false;
            }
        }
    }
    curly == 0 && square == 0 && round == 0
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
        let src = "class A { /* c */ String s = \"x\"; }";
        let toks = lex(src);
        assert_eq!(toks[0].start, 0);
        assert_eq!(toks.last().unwrap().end, src.len());
        for w in toks.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn a_brace_in_a_literal_does_not_count() {
        assert!(ok("class A { String s = \"}\"; }"));
        assert!(ok("class A { char c = '}'; }"));
        assert!(ok("class A { /* } */ }"));
        assert!(ok("class A { // }\n}"));
    }

    /// A text block spans lines and may contain bare quotes.
    #[test]
    fn a_text_block_is_one_string() {
        let src = "class A {\n  String s = \"\"\"\n    a \"quoted\" }\n    \"\"\";\n}";
        assert!(ok(src), "{}", code(src));
        assert!(!code(src).contains("quoted"), "{}", code(src));
    }

    #[test]
    fn javadoc_is_marked_apart_from_an_ordinary_comment() {
        let toks = lex("/** doc */\n/* plain */\n");
        let docs: Vec<bool> = toks
            .iter()
            .filter_map(|t| match t.tok {
                Tok::Block { doc } => Some(doc),
                _ => None,
            })
            .collect();
        assert_eq!(docs, vec![true, false]);
    }

    /// Java divides; nothing here is a regex, so `/` is always just a slash.
    #[test]
    fn a_slash_is_always_division() {
        assert!(ok("class A { int x() { return a / b; } }"));
        assert!(ok("class A { int x = 1 / 2; }"));
    }

    #[test]
    fn an_unbalanced_or_unterminated_file_is_reported() {
        assert!(!ok("class A {"));
        assert!(!ok("class A }"));
        assert!(!ok("/* open"));
        assert!(!ok("class A { String s = \"\"\"\n open"));
        assert!(ok(""));
    }

    #[test]
    fn hostile_input_never_panics() {
        for c in ["\"", "'", "/*", "\"\"\"", "\"\"", "\\", "{{{", "}}}", "'\\", "\u{fffd}"] {
            let toks = lex(c);
            let _ = balanced(c, &toks);
            if !toks.is_empty() {
                assert_eq!(toks[0].start, 0, "{c:?}");
                assert_eq!(toks.last().unwrap().end, c.len(), "{c:?}");
            }
        }
    }
}
