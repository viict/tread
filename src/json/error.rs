//! What a parse failure says (SPEC.md §JSON: "a line that is not valid JSON
//! renders as an error row carrying the reason and the line number").
//!
//! An error is a byte offset and a [`Reason`], and its `Display` is the
//! sentence a status bar or an error row shows: `unexpected } at byte 41207`.
//! Both halves matter — the offset is what lets a reader point at the damage in
//! a 4MB record, and the reason is what tells them whether the file is
//! truncated or was never JSON.
//!
//! Bytes in a message are printed as themselves only when they are printable
//! ASCII; anything else is written as hex, because an error message goes to a
//! terminal and a raw `ESC` in a document must never reach one.
#![deny(unsafe_code)]
#![allow(dead_code)]

use std::fmt;

/// Why a parse stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Input ran out mid-value.
    Eof,
    /// A byte that cannot appear here.
    Unexpected(u8),
    /// A complete value was followed by something other than whitespace.
    Trailing(u8),
    /// `true`, `false` or `null` started but did not finish.
    BadLiteral(&'static str),
    /// Not a number the RFC 8259 grammar allows: `01`, `.5`, `1.`, `1e`, `+1`.
    BadNumber,
    /// `\q` — a backslash escape with no meaning.
    BadEscape(u8),
    /// `\u` not followed by four hex digits.
    BadHex,
    /// A raw control character inside a string, which RFC 8259 §7 forbids.
    Control(u8),
    /// Nesting past the configured limit.
    TooDeep(usize),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Reason::Eof => f.write_str("unexpected end of input"),
            Reason::Unexpected(b) => write!(f, "unexpected {}", Byte(b)),
            Reason::Trailing(b) => write!(f, "trailing {} after the value", Byte(b)),
            Reason::BadLiteral(w) => write!(f, "expected `{w}`"),
            Reason::BadNumber => f.write_str("invalid number"),
            Reason::BadEscape(b) => write!(f, "invalid escape \\{}", b as char),
            Reason::BadHex => f.write_str("invalid \\u escape"),
            Reason::Control(b) => write!(f, "unescaped control character 0x{b:02x} in string"),
            Reason::TooDeep(n) => write!(f, "nesting deeper than {n} levels"),
        }
    }
}

/// A byte in a message: itself when it is printable, its hex otherwise, so a
/// NUL or a stray `0x9b` cannot smuggle an escape sequence into the status bar.
struct Byte(u8);

impl fmt::Display for Byte {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0x20..=0x7e => write!(f, "{}", self.0 as char),
            b => write!(f, "byte 0x{b:02x}"),
        }
    }
}

/// A parse failure: what went wrong, and where.
///
/// The offset is a byte offset into the input that was handed to the parser,
/// which for a `.jsonl` line means an offset within that line — the caller adds
/// the line's own offset if it wants a file position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub reason: Reason,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.offset)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
