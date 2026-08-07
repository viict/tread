//! Tests for the error sentences.
//!
//! These are user-visible strings: they land in a status bar and in the error
//! row a bad `.jsonl` line renders as, so they are asserted verbatim rather
//! than by shape.

use super::*;

fn say(reason: Reason, offset: usize) -> String {
    Error { offset, reason }.to_string()
}

#[test]
fn the_spec_sentence() {
    assert_eq!(say(Reason::Unexpected(b'}'), 41207), "unexpected } at byte 41207");
}

#[test]
fn every_reason_reads_as_a_sentence() {
    assert_eq!(say(Reason::Eof, 0), "unexpected end of input at byte 0");
    assert_eq!(say(Reason::Trailing(b'{'), 3), "trailing { after the value at byte 3");
    assert_eq!(say(Reason::BadLiteral("true"), 7), "expected `true` at byte 7");
    assert_eq!(say(Reason::BadNumber, 2), "invalid number at byte 2");
    assert_eq!(say(Reason::BadEscape(b'q'), 4), "invalid escape \\q at byte 4");
    assert_eq!(say(Reason::BadHex, 5), "invalid \\u escape at byte 5");
    assert_eq!(
        say(Reason::Control(0x1f), 9),
        "unescaped control character 0x1f in string at byte 9"
    );
    assert_eq!(say(Reason::TooDeep(10_000), 10_000), "nesting deeper than 10000 levels at byte 10000");
}

#[test]
fn an_unprintable_byte_is_shown_as_hex_and_never_verbatim() {
    // A raw ESC, CSI or NUL in the document must not reach the terminal
    // through the *error message* either.
    assert_eq!(say(Reason::Unexpected(0), 1), "unexpected byte 0x00 at byte 1");
    assert_eq!(say(Reason::Unexpected(0x1b), 1), "unexpected byte 0x1b at byte 1");
    assert_eq!(say(Reason::Unexpected(0x9b), 1), "unexpected byte 0x9b at byte 1");
    assert_eq!(say(Reason::Unexpected(0xff), 1), "unexpected byte 0xff at byte 1");
    assert_eq!(say(Reason::Trailing(b'\n'), 1), "trailing byte 0x0a after the value at byte 1");
    // The printable range is inclusive at both ends.
    assert_eq!(say(Reason::Unexpected(b' '), 0), "unexpected   at byte 0");
    assert_eq!(say(Reason::Unexpected(b'~'), 0), "unexpected ~ at byte 0");
    assert_eq!(say(Reason::Unexpected(0x7f), 0), "unexpected byte 0x7f at byte 0");
}

#[test]
fn an_error_is_a_std_error_and_is_copy() {
    let e = Error { offset: 1, reason: Reason::Eof };
    let copy = e;
    assert_eq!(copy, e);
    let boxed: Box<dyn std::error::Error> = Box::new(e);
    assert_eq!(boxed.to_string(), "unexpected end of input at byte 1");
}
