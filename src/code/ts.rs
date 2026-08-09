//! JavaScript and TypeScript: the lexer.
//!
//! Harder than Rust, for two reasons that have no equivalent there.
//!
//! * **Template literals nest arbitrarily.** `` `a ${ f(`b ${c}`) } d` `` is one
//!   literal containing code containing another literal. The `${…}` parts are
//!   *code* — braces in them count — so this needs a stack, not a flag.
//! * **`/` is ambiguous.** `a / b` divides; `/ab+/` is a regex. Nothing local
//!   distinguishes them: it is decided by the previous significant token. Read a
//!   division as a regex and everything to the next `/` — braces included —
//!   disappears into a literal.
//!
//! JSX needs no special handling: its `{…}` expressions balance like any other
//! braces, and `<` and `>` are not counted.
#![deny(unsafe_code)]

use super::scan::{Cursor, Span, Tok};

/// Classify `src` into tokens that tile it end to end.
///
/// Total, like the Rust lexer: any input produces a stream, and an unterminated
/// literal runs to the end of the file for [`balanced`] to report.
pub fn lex(src: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut c = Cursor::new(src);
    let mut code_from = 0usize;
    // Depth of `${}` interpolations we are inside, and the brace depth each was
    // opened at, so the matching `}` resumes its template rather than the file.
    let mut templates: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    // What decides `/`: the last significant byte, and — when that byte is part
    // of an identifier — the word it belongs to, because `return /re/` is a
    // regex while `total /re/` is a division.
    let mut prev = 0u8;
    let mut word = String::new();
    let mut in_word = false;

    while !c.done() {
        let start = c.at;
        let b = match c.peek(0) {
            Some(b) => b,
            None => break,
        };
        let tok = if c.at_str(b"//") {
            c.skip_line();
            Some(Tok::Line { doc: false })
        } else if c.at_str(b"/*") {
            let doc = c.peek(2) == Some(b'*');
            block_comment(&mut c);
            Some(Tok::Block { doc })
        } else if b == b'/' && regex_here(prev, &word) {
            regex(&mut c);
            Some(Tok::Str)
        } else if b == b'"' || b == b'\'' {
            quoted(&mut c, b);
            Some(Tok::Str)
        } else if b == b'`' {
            // Runs to the literal's end or to the first `${`, which starts code.
            c.bump();
            if template_body(&mut c) {
                templates.push(depth);
                depth += 1;
            }
            Some(Tok::Str)
        } else {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth = depth.saturating_sub(1);
                    // Closing an interpolation resumes the template it was in.
                    if templates.last() == Some(&depth) {
                        templates.pop();
                        c.bump();
                        // Resuming, not opening: the next backtick *closes*
                        // this literal, so it must not be consumed as an
                        // opening one.
                        if template_body(&mut c) {
                            templates.push(depth);
                            depth += 1;
                        }
                        flush(&mut out, &mut code_from, start);
                        out.push(Span {
                            tok: Tok::Str,
                            start,
                            end: c.at,
                        });
                        code_from = c.at;
                        prev = b'`';
                        continue;
                    }
                }
                _ => {}
            }
            c.bump();
            // `word` must survive the space in `return /re/`, so whitespace
            // ends the word without erasing it; anything else clears it.
            match is_ident(b) {
                true => {
                    if !in_word {
                        word.clear();
                        in_word = true;
                    }
                    word.push(b as char);
                }
                false => {
                    in_word = false;
                    if !b.is_ascii_whitespace() {
                        word.clear();
                    }
                }
            }
            if !b.is_ascii_whitespace() {
                prev = b;
            }
            None
        };
        if let Some(tok) = tok {
            flush(&mut out, &mut code_from, start);
            out.push(Span {
                tok,
                start,
                end: c.at,
            });
            code_from = c.at;
            // A literal or comment counts as a value for the next `/`.
            // A literal is a value, so a `/` after it divides.
            if !matches!(tok, Tok::Line { .. } | Tok::Block { .. }) {
                prev = b'x';
                word.clear();
                in_word = false;
            }
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

fn flush(out: &mut Vec<Span>, code_from: &mut usize, start: usize) {
    if start > *code_from {
        out.push(Span {
            tok: Tok::Code,
            start: *code_from,
            end: start,
        });
    }
    *code_from = start;
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

/// Keywords after which a `/` can only begin a regex. After anything else that
/// ends a value — an identifier, a number, a closing bracket — it divides.
const OPERAND_EXPECTED: [&str; 14] = [
    "return", "typeof", "instanceof", "in", "of", "new", "delete", "void",
    "throw", "case", "do", "else", "yield", "await",
];

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// Is a `/` here the start of a regex rather than a division?
///
/// There is no local answer — it depends on what came before. After a value
/// (identifier, number, closing bracket) `/` divides; after an operator, a
/// comma, an opening bracket or an operand-expecting keyword it opens a regex.
/// `}` is genuinely ambiguous (a block ends, or an object literal does) and is
/// read as a value, the commoner case in code a reader opens.
///
/// `<` is excluded because of JSX: `</div>` is a closing tag, and reading it as
/// a regex swallows everything to the next `/` — which in a JSX tree is most of
/// the component.
fn regex_here(prev: u8, word: &str) -> bool {
    if is_ident(prev) {
        return OPERAND_EXPECTED.contains(&word);
    }
    !matches!(prev, b')' | b']' | b'}' | b'<')
}

/// `/re/flags`, honouring escapes and character classes — a `/` inside `[...]`
/// does not end it.
fn regex(c: &mut Cursor) {
    c.bump();
    let mut in_class = false;
    while let Some(b) = c.bump() {
        match b {
            b'\\' => {
                c.bump();
            }
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class => return,
            b'\n' => return, // a regex cannot span lines: treat it as ended
            _ => {}
        }
    }
}

/// `'…'` or `"…"`, with escapes. A newline ends it — an unterminated quote
/// should not swallow the file.
fn quoted(c: &mut Cursor, quote: u8) {
    c.bump();
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

/// Scan the text of a template literal, from just inside the opening backtick
/// or from just after the `}` that closed an interpolation. Returns true when
/// it stopped at a `${`, meaning code follows.
fn template_body(c: &mut Cursor) -> bool {
    while let Some(b) = c.peek(0) {
        match b {
            b'\\' => {
                c.skip(2);
            }
            b'`' => {
                c.bump();
                return false;
            }
            b'$' if c.peek(1) == Some(b'{') => {
                c.skip(2);
                return true;
            }
            _ => {
                c.bump();
            }
        }
    }
    false
}

/// Whether the file lexed cleanly: brackets balanced, nothing left open.
///
/// The same safety valve as Rust's — a file that fails this gets no outline and
/// opens as plain source (SPEC.md §Code).
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
            // A template that ran to the end of the file never closed.
            Tok::Str => {
                if s.end == src.len() && src[s.start..s.end].starts_with('`') {
                    let body = &src[s.start..s.end];
                    if body.len() < 2 || !body.ends_with('`') {
                        return false;
                    }
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
#[path = "ts_tests.rs"]
mod tests;
