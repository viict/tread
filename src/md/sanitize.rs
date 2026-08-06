//! Make arbitrary bytes safe to put on a terminal.
//!
//! A markdown file is untrusted input. If it contains a literal `ESC`, a `BEL`
//! or a C1 control, forwarding it verbatim lets the document repaint the
//! screen, set the window title, or desynchronise the frame buffer's width
//! accounting (a control byte occupies zero columns but one `char`). Every
//! document is therefore normalised once, at the parser's front door, so no
//! module downstream has to think about it.
//!
//! Kept: `\n`, `\t`, and every printable scalar value.
//! Rewritten: `\r\n` and lone `\r` become `\n`; all other C0 controls, `DEL`,
//! and the C1 range `U+0080..=U+009F` become `U+FFFD`.
#![deny(unsafe_code)]

use std::borrow::Cow;

/// True when `c` must not reach the terminal as-is.
fn is_hostile(c: char) -> bool {
    match c {
        '\n' | '\t' => false,
        '\r' => true,
        '\u{0}'..='\u{1f}' | '\u{7f}' => true,
        '\u{80}'..='\u{9f}' => true,
        _ => false,
    }
}

/// Normalise line endings and neutralise control characters.
///
/// Borrows when the input is already clean, which is the common case, so a
/// large well-formed document costs one scan and no allocation.
pub fn clean(src: &str) -> Cow<'_, str> {
    if !src.chars().any(is_hostile) {
        return Cow::Borrowed(src);
    }
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                // CRLF collapses; a lone CR is still a line break.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            c if is_hostile(c) => out.push('\u{fffd}'),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Decode a document's raw bytes. A reader never refuses to open a file:
/// invalid UTF-8 becomes `U+FFFD` and a leading byte-order mark is dropped.
/// Control characters are left for [`clean`], which runs inside `parse`.
pub fn decode(bytes: Vec<u8>) -> String {
    let mut text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    text
}

/// Read a file the same way everywhere: lossy, BOM-stripped, never fatal for
/// content reasons — only for I/O ones.
pub fn read_file(path: &std::path::Path) -> Result<String, std::io::Error> {
    std::fs::read(path).map(decode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_strips_a_leading_bom_only() {
        assert_eq!(decode("\u{feff}# H\n".as_bytes().to_vec()), "# H\n");
        assert_eq!(decode("a\u{feff}b".as_bytes().to_vec()), "a\u{feff}b");
    }

    #[test]
    fn decode_replaces_invalid_utf8_instead_of_failing() {
        assert_eq!(decode(vec![b'a', 0xff, 0xfe, b'b']), "a\u{fffd}\u{fffd}b");
        assert_eq!(decode(Vec::new()), "");
    }

    #[test]
    fn decode_then_clean_neutralises_a_hostile_file() {
        let raw = b"\xef\xbb\xbf# T\r\n\r\n\x1b]0;x\x07 \xffz\r\n".to_vec();
        let text = decode(raw);
        let got = clean(&text);
        assert!(!got.contains('\u{1b}') && !got.contains('\r'));
        assert_eq!(got, "# T\n\n\u{fffd}]0;x\u{fffd} \u{fffd}z\n");
    }

    #[test]
    fn clean_text_is_borrowed() {
        assert!(matches!(clean("# hi\n\ntext\twith tab\n"), Cow::Borrowed(_)));
    }

    #[test]
    fn crlf_becomes_lf() {
        assert_eq!(clean("a\r\nb\r\n"), "a\nb\n");
    }

    #[test]
    fn lone_cr_becomes_lf() {
        assert_eq!(clean("a\rb\rc"), "a\nb\nc");
    }

    #[test]
    fn escape_sequences_cannot_reach_the_terminal() {
        let got = clean("text \u{1b}[31mRED\u{1b}[0m\n");
        assert!(!got.contains('\u{1b}'));
        assert_eq!(got, "text \u{fffd}[31mRED\u{fffd}[0m\n");
    }

    #[test]
    fn nul_bel_del_and_c1_are_replaced() {
        assert_eq!(clean("a\0b"), "a\u{fffd}b");
        assert_eq!(clean("a\u{7}b"), "a\u{fffd}b");
        assert_eq!(clean("a\u{7f}b"), "a\u{fffd}b");
        assert_eq!(clean("a\u{9b}b"), "a\u{fffd}b");
    }

    #[test]
    fn tabs_and_newlines_survive() {
        assert_eq!(clean("a\tb\nc\n"), "a\tb\nc\n");
    }

    #[test]
    fn non_ascii_text_is_untouched() {
        let s = "日本語 — ✅ \u{1f469}\u{200d}\u{1f4bb}\n";
        assert_eq!(clean(s), s);
        assert!(matches!(clean(s), Cow::Borrowed(_)));
    }

    #[test]
    fn empty_input() {
        assert_eq!(clean(""), "");
    }
}
