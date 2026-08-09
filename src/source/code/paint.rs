//! Colouring code.
//!
//! The highlighter is the lexer that is already there. `code::rust::lex` and
//! `code::ts::lex` classify comments and string literals for the declaration
//! finder; asking the same tokens which colour a byte should be costs nothing
//! extra and — because it runs over the *whole file* — gets multi-line block
//! comments and multi-line strings right, which anything working line by line
//! cannot.
//!
//! Deliberately four colours and no more: keyword, string, number, comment.
//! A reader is scanning for shape, not admiring a rainbow, and every extra hue
//! is one more thing competing with the search highlight and the cursor row.
#![deny(unsafe_code)]

use crate::code::scan::{Span, Tok};
use crate::code::{java, py, rust, ts};
use crate::term::Style;
use crate::theme;

/// Words that colour as keywords. One list per language, kept short: control
/// flow and declarations, the words a reader's eye uses to find structure.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "union", "unsafe", "use", "where", "while",
];

const TS_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "declare",
    "default", "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for",
    "from", "function", "get", "if", "implements", "import", "in", "instanceof", "interface",
    "let", "new", "null", "of", "readonly", "return", "set", "static", "switch", "this", "throw",
    "true", "try", "type", "typeof", "undefined", "var", "void", "while", "yield",
];

/// A styled run of bytes within one line, `[start, end)`.
pub type Run = (usize, usize, Style);

const PY_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];

const JAVA_KEYWORDS: &[&str] = &[
    "abstract", "assert", "break", "case", "catch", "class", "continue", "default", "do", "else",
    "enum", "extends", "final", "finally", "for", "if", "implements", "import", "instanceof",
    "interface", "native", "new", "package", "private", "protected", "public", "record", "return",
    "static", "super", "switch", "synchronized", "this", "throw", "throws", "transient", "try",
    "void", "volatile", "while", "true", "false", "null",
];

fn keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "rust" => RUST_KEYWORDS,
        "typescript" => TS_KEYWORDS,
        "python" => PY_KEYWORDS,
        "java" => JAVA_KEYWORDS,
        _ => &[],
    }
}

fn lexer(lang: &str) -> Option<fn(&str) -> Vec<Span>> {
    match lang {
        "rust" => Some(rust::lex),
        "typescript" => Some(ts::lex),
        "python" => Some(py::lex),
        "java" => Some(java::lex),
        _ => None,
    }
}

/// Style runs for every line of `src`, in order. Offsets are relative to each
/// line, so a caller that expands tabs can do so run by run.
pub fn runs(lang: &str, src: &str) -> Vec<Vec<Run>> {
    let Some(lex) = lexer(lang) else {
        return vec![Vec::new(); src.lines().count()];
    };
    let toks = lex(src);
    let kw = keywords(lang);
    let mut out = Vec::with_capacity(src.lines().count());
    let mut at = 0usize; // byte offset of the current line
    for line in src.lines() {
        out.push(line_runs(line, at, &toks, kw));
        // `lines()` drops the terminator, which is one byte for `\n` and two
        // for `\r\n`; both must be stepped over or every later line is skewed.
        let after = at + line.len();
        at = match src.as_bytes().get(after) {
            Some(b'\r') => after + 2,
            _ => after + 1,
        };
    }
    out
}

/// The runs covering one line, given the file's tokens.
fn line_runs(line: &str, start: usize, toks: &[Span], kw: &[&str]) -> Vec<Run> {
    let end = start + line.len();
    let mut out: Vec<Run> = Vec::new();
    for s in toks.iter().filter(|s| s.end > start && s.start < end) {
        let from = s.start.max(start) - start;
        let to = (s.end.min(end)) - start;
        if from >= to {
            continue;
        }
        match s.tok {
            Tok::Line { .. } | Tok::Block { .. } => out.push((from, to, comment())),
            Tok::Str => out.push((from, to, string())),
            // Code is scanned for the words and numbers worth colouring.
            Tok::Code => words(&line[from..to], from, kw, &mut out),
        }
    }
    out
}

/// Keywords and numeric literals inside a stretch of code.
fn words(text: &str, offset: usize, kw: &[&str], out: &mut Vec<Run>) {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            i += 1;
            continue;
        }
        let from = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let word = &text[from..i];
        // A word starting with a digit is a number — `0x1f`, `1_000`, `3.0`
        // colours as far as the identifier scan took it, which is enough.
        let style = match word.as_bytes()[0].is_ascii_digit() {
            true => Some(number()),
            false => kw.contains(&word).then(keyword),
        };
        if let Some(style) = style {
            out.push((offset + from, offset + i, style));
        }
    }
}

fn keyword() -> Style {
    Style::new().fg(theme::SYNTAX_KEYWORD)
}

fn string() -> Style {
    Style::new().fg(theme::SYNTAX_STRING)
}

fn number() -> Style {
    Style::new().fg(theme::SYNTAX_NUMBER)
}

fn comment() -> Style {
    theme::muted()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text each run covers, with a label — what the eye actually sees.
    fn painted(lang: &str, src: &str) -> Vec<Vec<(String, String)>> {
        runs(lang, src)
            .into_iter()
            .zip(src.lines())
            .map(|(rs, line)| {
                rs.into_iter()
                    .map(|(a, b, st)| {
                        let name = match st.fg {
                            Some(theme::SYNTAX_KEYWORD) => "kw",
                            Some(theme::SYNTAX_STRING) => "str",
                            Some(theme::SYNTAX_NUMBER) => "num",
                            Some(theme::MUTED_FG) => "comment",
                            _ => "?",
                        };
                        (line[a..b].to_string(), name.to_string())
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn keywords_strings_numbers_and_comments_are_coloured() {
        let got = painted("rust", "let x = 42; // note\n");
        assert_eq!(got[0][0], ("let".into(), "kw".into()));
        assert_eq!(got[0][1], ("42".into(), "num".into()));
        assert_eq!(got[0][2], ("// note".into(), "comment".into()));
    }

    /// The reason this runs over the whole file: a block comment spanning
    /// lines is a comment on every one of them, which a line-by-line
    /// highlighter cannot know.
    #[test]
    fn a_multi_line_comment_is_a_comment_on_every_line() {
        let got = painted("rust", "/* one\n   two\n   three */\nfn f() {}\n");
        for (i, line) in got.iter().take(3).enumerate() {
            assert_eq!(line[0].1, "comment", "line {i}: {line:?}");
        }
        assert_eq!(got[3][0], ("fn".into(), "kw".into()), "and code resumes");
    }

    #[test]
    fn a_keyword_inside_a_string_or_comment_is_not_a_keyword() {
        let got = painted("rust", "let s = \"let fn\"; // let fn\n");
        let kinds: Vec<&str> = got[0].iter().map(|(_, k)| k.as_str()).collect();
        assert_eq!(kinds, vec!["kw", "str", "comment"], "{:?}", got[0]);
    }

    #[test]
    fn typescript_keywords_are_its_own() {
        let got = painted("typescript", "export const f = async () => 1;\n");
        let words: Vec<&str> = got[0]
            .iter()
            .filter(|(_, k)| k == "kw")
            .map(|(w, _)| w.as_str())
            .collect();
        assert_eq!(words, vec!["export", "const", "async"]);
    }

    /// A template literal is a string on every line it covers.
    #[test]
    fn a_multi_line_template_is_a_string_throughout() {
        let got = painted("typescript", "const s = `one\ntwo`;\nlet x = 1;\n");
        assert_eq!(got[0].last().unwrap().1, "str");
        assert_eq!(got[1][0].1, "str");
        assert_eq!(got[2][0], ("let".into(), "kw".into()));
    }

    #[test]
    fn line_offsets_survive_crlf_and_an_unknown_language() {
        let got = painted("rust", "let a = 1;\r\nlet b = 2;\r\n");
        assert_eq!(got[1][0], ("let".into(), "kw".into()), "not skewed by \\r\\n");
        assert!(runs("cobol", "IDENTIFICATION DIVISION.\n")[0].is_empty());
    }

    #[test]
    fn every_run_is_inside_its_line_and_they_do_not_overlap() {
        let src = include_str!("../../code/rust_decl.rs");
        for (rs, line) in runs("rust", src).into_iter().zip(src.lines()) {
            let mut last = 0usize;
            for (a, b, _) in rs {
                assert!(a < b && b <= line.len(), "{a}..{b} in {:?}", line.len());
                assert!(a >= last, "runs are ordered and disjoint");
                assert!(line.is_char_boundary(a) && line.is_char_boundary(b));
                last = b;
            }
        }
    }
}
