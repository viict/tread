//! Delimiter sniffing and `--delim` parsing.
//!
//! The sniffer is a heuristic, so these pin the cases where the *wrong* guess
//! would be plausible: a delimiter that appears inside quoted text, a sample
//! whose last row is cut in half, and a file with nothing to go on at all.

use super::*;

#[test]
fn sniffs_each_candidate() {
    assert_eq!(sniff(b"a,b,c\n1,2,3\n"), b',');
    assert_eq!(sniff(b"a\tb\tc\n1\t2\t3\n"), b'\t');
    assert_eq!(sniff(b"a;b;c\n1;2;3\n"), b';');
    assert_eq!(sniff(b"a|b|c\n1|2|3\n"), b'|');
}

#[test]
fn sniffing_prefers_the_consistent_delimiter() {
    // Commas appear, but only the semicolon splits every row the same way.
    let src = b"name;note\nx;a, b, c\ny;d, e\nz;f\n";
    assert_eq!(sniff(src), b';');
}

#[test]
fn sniffing_ignores_delimiters_inside_quotes() {
    let src = b"a|b\n\"x|y\"|z\n\"p|q\"|r\n";
    assert_eq!(sniff(src), b'|');
}

#[test]
fn sniffing_falls_back_to_comma_when_there_is_nothing_to_go_on() {
    assert_eq!(sniff(b""), DEFAULT_DELIM);
    assert_eq!(sniff(b"just one column\nand another line\n"), DEFAULT_DELIM);
    assert_eq!(sniff(b"\n\n\n"), DEFAULT_DELIM);
}

#[test]
fn sniffing_ignores_a_truncated_last_row() {
    // The sample stops mid-row; that row must not out-vote the real ones.
    let src = b"a,b,c\n1,2,3\n4,5";
    assert_eq!(sniff(src), b',');
}

#[test]
fn sniffing_skips_a_bom() {
    assert_eq!(sniff(b"\xef\xbb\xbfa;b\n1;2\n"), b';');
}

#[test]
fn parse_delim_accepts_names_and_single_characters() {
    assert_eq!(parse_delim("tab"), Some(b'\t'));
    assert_eq!(parse_delim("\\t"), Some(b'\t'));
    assert_eq!(parse_delim("comma"), Some(b','));
    assert_eq!(parse_delim("semicolon"), Some(b';'));
    assert_eq!(parse_delim("pipe"), Some(b'|'));
    assert_eq!(parse_delim(";"), Some(b';'));
    assert_eq!(parse_delim(" "), Some(b' '));
    assert_eq!(parse_delim(""), None);
    assert_eq!(parse_delim("ab"), None);
    assert_eq!(parse_delim("é"), None, "the delimiter must be one byte");
    assert_eq!(parse_delim("\""), None, "a quote would make the grammar ambiguous");
    assert_eq!(parse_delim("\n"), None);
}

