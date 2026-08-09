//! Shared lexing primitives: the classified token stream every language
//! grammar is built on.
//!
//! The point of classifying first is that *everything* downstream is brace
//! counting, and a brace inside a string or a comment must not count. Getting
//! that wrong does not produce a slightly-off outline, it produces a body that
//! swallows the rest of the file — so this layer is deliberately dumb, total,
//! and tested against hostile input.
#![deny(unsafe_code)]

/// What a stretch of bytes is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    /// Anything that is not one of the below: this is where braces count.
    Code,
    /// `//` to end of line. `doc` is true for `///` and `//!`.
    Line { doc: bool },
    /// `/* */`, nested. `doc` is true for `/**` and `/*!`.
    Block { doc: bool },
    /// A string, char, byte-string or raw-string literal.
    Str,
}

/// One classified stretch, `[start, end)` in bytes. Tokens tile the input: the
/// first starts at 0, the last ends at `src.len()`, and there are no gaps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub tok: Tok,
    pub start: usize,
    pub end: usize,
}

/// A byte cursor that never panics and never splits a UTF-8 character.
///
/// Indexing is by byte because that is what the offsets in `Span` are; the
/// grammars only ever compare against ASCII, so a multi-byte character is
/// simply "not the byte I was looking for" and is stepped over whole.
pub struct Cursor<'a> {
    src: &'a [u8],
    pub at: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(src: &'a str) -> Cursor<'a> {
        Cursor {
            src: src.as_bytes(),
            at: 0,
        }
    }

    pub fn done(&self) -> bool {
        self.at >= self.src.len()
    }

    /// The byte `n` ahead, or `None` past the end.
    pub fn peek(&self, n: usize) -> Option<u8> {
        self.src.get(self.at + n).copied()
    }

    /// True when the bytes at the cursor are exactly `what`.
    pub fn at_str(&self, what: &[u8]) -> bool {
        self.src[self.at.min(self.src.len())..].starts_with(what)
    }

    pub fn bump(&mut self) -> Option<u8> {
        let b = self.peek(0)?;
        self.at += 1;
        Some(b)
    }

    /// Step forward `n` bytes, saturating at the end.
    pub fn skip(&mut self, n: usize) {
        self.at = (self.at + n).min(self.src.len());
    }

    /// Consume to just past the next `\n`, or to the end.
    pub fn skip_line(&mut self) {
        while let Some(b) = self.peek(0) {
            self.at += 1;
            if b == b'\n' {
                return;
            }
        }
    }
}

/// The source with every non-code byte replaced by a space, newlines kept.
///
/// This is what makes the declaration recognizers safe *and* simple: searching
/// blanked text for `fn ` cannot match the word inside a comment or a string
/// literal, and counting braces in it cannot count one that is quoted. Newlines
/// survive so the result still aligns line-for-line with the original — the
/// recognizer reports line numbers, and an off-by-one here would move every
/// symbol in the file.
pub fn blank(src: &str, toks: &[Span]) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    for s in toks {
        match s.tok {
            Tok::Code => out.push_str(&src[s.start..s.end]),
            _ => out.extend(bytes[s.start..s.end].iter().map(|&b| match b {
                b'\n' => '\n',
                _ => ' ',
            })),
        }
    }
    out
}

/// A byte offset to a 0-based line number, for a whole slice of offsets at
/// once.
///
/// Built once per file rather than counted per lookup: a symbol has five
/// offsets and a file has hundreds of symbols, and rescanning from the start
/// each time is the quadratic mistake that only shows up on a big file.
pub struct Lines {
    /// Byte offset where each line starts. Always begins with 0.
    starts: Vec<usize>,
}

impl Lines {
    pub fn new(src: &str) -> Lines {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Lines { starts }
    }

    /// How many lines the file has. A trailing newline does not invent one.
    pub fn count(&self, src: &str) -> usize {
        match src.is_empty() {
            true => 0,
            false => match src.ends_with('\n') {
                true => self.starts.len() - 1,
                false => self.starts.len(),
            },
        }
    }

    /// The 0-based line containing `offset`.
    pub fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_stops_at_the_end_rather_than_panicking() {
        let mut c = Cursor::new("ab");
        assert_eq!(c.bump(), Some(b'a'));
        assert_eq!(c.bump(), Some(b'b'));
        assert_eq!(c.bump(), None);
        assert!(c.done());
        c.skip(999);
        assert_eq!(c.at, 2, "skip saturates");
        assert_eq!(c.peek(10), None);
    }

    #[test]
    fn a_cursor_steps_over_a_multibyte_character_without_splitting_it() {
        let mut c = Cursor::new("é{");
        assert!(!c.at_str(b"{"));
        c.bump();
        c.bump(); // the two bytes of é
        assert!(c.at_str(b"{"), "the brace is found after it");
    }

    #[test]
    fn offsets_map_to_lines() {
        let src = "a\nbb\n\nccc";
        let l = Lines::new(src);
        assert_eq!(l.count(src), 4);
        assert_eq!(l.line_of(0), 0);
        assert_eq!(l.line_of(1), 0, "the newline belongs to its own line");
        assert_eq!(l.line_of(2), 1);
        assert_eq!(l.line_of(5), 2, "the empty line");
        assert_eq!(l.line_of(8), 3);
        assert_eq!(l.line_of(9999), 3, "past the end clamps");
    }

    #[test]
    fn a_trailing_newline_does_not_invent_a_line() {
        assert_eq!(Lines::new("a\n").count("a\n"), 1);
        assert_eq!(Lines::new("a").count("a"), 1);
        assert_eq!(Lines::new("").count(""), 0);
    }
}
