//! Rust: the lexer.
//!
//! Two things here are the whole reason this cannot be a regex.
//!
//! * **Block comments nest.** `/* /* */ */` is one comment, and treating the
//!   first `*/` as the end leaves the rest of the file lexing as code.
//! * **`'` is ambiguous.** `'a` is a lifetime and `'x'` is a char literal. Read
//!   a lifetime as an unterminated string and everything after it — every brace
//!   in the file — is inside that string.
//!
//! Raw strings are the third: `r#"..."#` ends only at a quote followed by the
//! same number of hashes, so `"` alone does not close it.
#![deny(unsafe_code)]

use super::scan::{Cursor, Span, Tok};

/// Classify `src` into tokens that tile it end to end.
///
/// Total: any input produces a token stream. An unterminated string or comment
/// runs to the end of the file and is reported by [`balanced`] instead, which
/// is what makes a mis-lex visible rather than silently wrong.
pub fn lex(src: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut c = Cursor::new(src);
    let mut code_from = 0usize;

    while !c.done() {
        let start = c.at;
        let tok = match next_token(&mut c) {
            Some(t) => t,
            None => continue, // ordinary code byte, already stepped over
        };
        // Flush the code run that ended where this token began.
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
    if code_from < src.len() {
        out.push(Span {
            tok: Tok::Code,
            start: code_from,
            end: src.len(),
        });
    }
    out
}

/// Consume one non-code token at the cursor, or step over one code byte and
/// return `None`.
fn next_token(c: &mut Cursor) -> Option<Tok> {
    if c.at_str(b"//") {
        let doc = matches!(c.peek(2), Some(b'/') | Some(b'!'));
        c.skip_line();
        return Some(Tok::Line { doc });
    }
    if c.at_str(b"/*") {
        let doc = matches!(c.peek(2), Some(b'*') | Some(b'!'));
        block_comment(c);
        return Some(Tok::Block { doc });
    }
    if raw_string(c) || byte_or_string(c) {
        return Some(Tok::Str);
    }
    if c.peek(0) == Some(b'\'') && char_literal(c) {
        return Some(Tok::Str);
    }
    c.bump();
    None
}

/// `/* ... */`, honouring nesting. Runs to the end if never closed.
fn block_comment(c: &mut Cursor) {
    c.skip(2);
    let mut depth = 1usize;
    while !c.done() && depth > 0 {
        if c.at_str(b"/*") {
            depth += 1;
            c.skip(2);
        } else if c.at_str(b"*/") {
            depth -= 1;
            c.skip(2);
        } else {
            c.bump();
        }
    }
}

/// `r"..."`, `r#"..."#`, `br#"..."#`. Returns false if this is not one.
fn raw_string(c: &mut Cursor) -> bool {
    let mut n = 0usize; // bytes of prefix before the hashes
    if c.peek(0) == Some(b'b') && c.peek(1) == Some(b'r') {
        n = 2;
    } else if c.peek(0) == Some(b'r') {
        n = 1;
    }
    if n == 0 {
        return false;
    }
    let mut hashes = 0usize;
    while c.peek(n + hashes) == Some(b'#') {
        hashes += 1;
    }
    if c.peek(n + hashes) != Some(b'"') {
        return false; // `r` was just an identifier, or `r#ident` raw identifier
    }
    c.skip(n + hashes + 1);
    // Ends at a quote followed by exactly the hashes we opened with.
    while !c.done() {
        if c.peek(0) == Some(b'"') && (1..=hashes).all(|i| c.peek(i) == Some(b'#')) {
            c.skip(hashes + 1);
            return true;
        }
        c.bump();
    }
    true // unterminated: consumed to the end, `balanced` will say so
}

/// `"..."` or `b"..."`, with backslash escapes.
fn byte_or_string(c: &mut Cursor) -> bool {
    let open = match (c.peek(0), c.peek(1)) {
        (Some(b'b'), Some(b'"')) => 2,
        (Some(b'"'), _) => 1,
        _ => return false,
    };
    c.skip(open);
    while let Some(b) = c.bump() {
        match b {
            b'\\' => {
                c.bump();
            }
            b'"' => return true,
            _ => {}
        }
    }
    true
}

/// A char literal at the cursor — *not* a lifetime.
///
/// `'a` is a lifetime, `'x'` is a char. The rule: a single character followed
/// by a closing quote is a literal, and so is anything starting with a
/// backslash; `'ident` with no closing quote is a lifetime and must be left as
/// code. `'a'` really is a char literal, which is why the closing quote is what
/// decides rather than the first character.
fn char_literal(c: &mut Cursor) -> bool {
    if c.peek(1) == Some(b'\\') {
        // `'\n'`, `'\''`, `'\u{1f600}'` — scan to the closing quote.
        let mut i = 2;
        while let Some(b) = c.peek(i) {
            match b {
                b'\'' => {
                    c.skip(i + 1);
                    return true;
                }
                b'\n' => return false,
                _ => i += 1,
            }
        }
        return false;
    }
    // One character — however many bytes it takes — and then a quote.
    let mut i = 2;
    while matches!(c.peek(i), Some(b) if (b & 0xc0) == 0x80) {
        i += 1;
    }
    if c.peek(i) == Some(b'\'') {
        c.skip(i + 1);
        return true;
    }
    false
}

/// Whether the file lexed cleanly: every brace, bracket and paren closed, and
/// no comment or string left open.
///
/// This is the safety valve for the whole feature. A file that fails it gets no
/// outline at all and opens as raw source (SPEC.md §Code), because a wrong
/// outline is worse than none — a mis-lexed `{` swallows the rest of the file
/// into one symbol's body and hides it.
pub fn balanced(src: &str, toks: &[Span]) -> bool {
    let bytes = src.as_bytes();
    let (mut curly, mut square, mut round) = (0i32, 0i32, 0i32);
    for s in toks {
        match s.tok {
            // An unterminated comment or string reaches the end of the file.
            Tok::Line { .. } => continue,
            Tok::Block { .. } | Tok::Str => {
                if s.end == src.len() && !closed(src, *s) {
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
                return false; // closed something that was never opened
            }
        }
    }
    curly == 0 && square == 0 && round == 0
}

/// Did this trailing comment or string actually close?
fn closed(src: &str, s: Span) -> bool {
    let text = &src[s.start..s.end];
    match s.tok {
        Tok::Block { .. } => text.len() > 3 && text.ends_with("*/"),
        Tok::Str => text.len() > 1 && (text.ends_with('"') || text.ends_with('\'')),
        _ => true,
    }
}

#[cfg(test)]
#[path = "rust_tests.rs"]
mod tests;
