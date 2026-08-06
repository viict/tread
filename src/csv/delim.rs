//! Choosing the delimiter (SPEC.md §CSV, "Delimiter sniffed among `,` `\t` `;`
//! `|`, overridable").
//!
//! This is policy, not grammar: [`super::parse`] is told which byte separates
//! fields and has no opinion about which one a file uses. Keeping the guess
//! here is what stops the state machine from growing a second job — and it is
//! why the sniffer is allowed to be heuristic while the parser is not.
#![deny(unsafe_code)]
// The detector and `--delim` that will call this are the CSV `Source`'s wiring,
// a later roll; until then only `delim_tests.rs` drives it. Drop this allow
// once the source is the one choosing the delimiter.
#![allow(dead_code)]

use super::parse::{Records, QUOTE};

/// The delimiters [`sniff`] considers, in tie-break preference order.
pub const CANDIDATES: [u8; 4] = [b',', b'\t', b';', b'|'];

/// Used when a file gives the sniffer nothing to go on.
pub const DEFAULT_DELIM: u8 = b',';

/// How many rows [`sniff`] looks at.
const SNIFF_ROWS: usize = 32;

/// Guess the delimiter of `sample` (the first bytes of the file are enough).
///
/// For each candidate the sample is parsed and the *modal* field count taken;
/// the winner is the one whose rows agree with that mode most often, then the
/// one that splits into more columns, then the earliest in [`CANDIDATES`]. A
/// candidate that never yields more than one field cannot win — it is not
/// present in the data. With nothing to go on the answer is
/// [`DEFAULT_DELIM`]; the caller may always override it (`--delim`).
pub fn sniff(sample: &[u8]) -> u8 {
    let mut best = (0usize, 0usize, DEFAULT_DELIM);
    for &delim in &CANDIDATES {
        let (agree, mode) = score(sample, delim);
        if mode > 1 && (agree, mode) > (best.0, best.1) {
            best = (agree, mode, delim);
        }
    }
    best.2
}

/// `(rows matching the modal field count, the modal field count)`.
fn score(sample: &[u8], delim: u8) -> (usize, usize) {
    let mut counts: Vec<usize> =
        Records::new(sample, delim).take(SNIFF_ROWS).map(|r| r.fields.len()).collect();
    // The sample is a prefix of the file, so its last row is probably cut in
    // half; judging it would punish the right delimiter.
    if counts.len() > 1 {
        counts.pop();
    }
    let mut best = (0usize, 0usize);
    for &c in &counts {
        let agree = counts.iter().filter(|&&x| x == c).count();
        if (agree, c) > best {
            best = (agree, c);
        }
    }
    best
}

/// Map a user-supplied delimiter spec (`--delim`) to a byte: a single
/// character, or the names `tab`, `comma`, `semicolon`, `pipe`, or `\t`.
pub fn parse_delim(spec: &str) -> Option<u8> {
    let byte = match spec {
        "tab" | "\\t" => b'\t',
        "comma" => b',',
        "semicolon" => b';',
        "pipe" | "bar" => b'|',
        _ => {
            let mut it = spec.bytes();
            let b = it.next()?;
            if it.next().is_some() || !b.is_ascii() {
                return None;
            }
            b
        }
    };
    // A quote or a newline as the delimiter would make the grammar ambiguous.
    match byte {
        QUOTE | b'\n' | b'\r' => None,
        b => Some(b),
    }
}

#[cfg(test)]
#[path = "delim_tests.rs"]
mod tests;

/// A leading `sep=;` line, as Excel writes when it exports with a delimiter
/// that is not the locale's default.
///
/// It is a directive, not data: it names the delimiter for the rest of the
/// file and must not become the header row. Returns the delimiter and the
/// length of the line *including* its terminator, so the caller can start the
/// document after it.
///
/// `bytes` must already be past any BOM. Only a single-byte delimiter is
/// accepted — the convention has no way to express more, and `sep=,,` is far
/// likelier to be data than a directive.
pub fn sep_line(bytes: &[u8]) -> Option<(u8, usize)> {
    const TAG: &[u8] = b"sep=";
    // Excel writes it lowercase; other tools have been seen shouting it.
    let head = bytes.get(..TAG.len())?;
    if !head.eq_ignore_ascii_case(TAG) {
        return None;
    }
    let delim = *bytes.get(TAG.len())?;
    if delim == b'\n' || delim == b'\r' {
        return None; // `sep=` with nothing after it names nothing
    }
    match bytes.get(TAG.len() + 1)? {
        b'\n' => Some((delim, TAG.len() + 2)),
        b'\r' if bytes.get(TAG.len() + 2) == Some(&b'\n') => Some((delim, TAG.len() + 3)),
        // A lone CR still ends a line elsewhere in this parser, so honour it.
        b'\r' => Some((delim, TAG.len() + 2)),
        _ => None,
    }
}

#[cfg(test)]
mod sep_tests {
    use super::*;

    #[test]
    fn a_sep_directive_names_the_delimiter_and_its_own_length() {
        assert_eq!(sep_line(b"sep=;\na;b\n"), Some((b';', 6)));
        assert_eq!(sep_line(b"sep=,\r\na,b\r\n"), Some((b',', 7)));
        assert_eq!(sep_line(b"sep=\t\na\tb\n"), Some((b'\t', 6)));
        assert_eq!(sep_line(b"SEP=|\na|b\n"), Some((b'|', 6)));
    }

    #[test]
    fn anything_that_is_not_the_directive_is_data() {
        assert_eq!(sep_line(b"a,b\n"), None);
        assert_eq!(sep_line(b"sep=\n"), None, "names nothing");
        assert_eq!(sep_line(b"sep=;;\n"), None, "not a single byte");
        assert_eq!(sep_line(b"separator,x\n"), None, "a real column called separator");
        assert_eq!(sep_line(b"sep=;"), None, "no terminator: it is the only row");
        assert_eq!(sep_line(b""), None);
    }
}
