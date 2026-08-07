//! RFC 8259 parsing, hand-written, byte by byte (SPEC.md §JSON).
//!
//! # No recursion, ever
//!
//! Nesting depth in a JSON document is attacker-controlled: `[[[[[…` is four
//! bytes per level, so a 40KB file is ten thousand levels deep. A recursive
//! descent parser dies on that with a stack overflow, which on most platforms
//! is a hard abort no `Result` can catch. So the parser below keeps its own
//! [`Frame`] stack on the heap and its call graph is flat: [`Parser::parse`]
//! costs the same stack for `[[[[…` a million deep as it does for `1`. Depth is
//! bounded by [`DEFAULT_MAX_DEPTH`] anyway, so a hostile file is *refused with a
//! reason* rather than left to exhaust memory — and refusing is a choice about
//! memory, not a workaround for the stack.
//!
//! The same discipline is kept by the value tree's `Drop`, `Clone` and
//! `PartialEq` (see [`super::value`]) and by the serialiser
//! ([`super::write`]): an iterative parser behind a recursive walker is still a
//! crash.
//!
//! # What it accepts
//!
//! RFC 8259 exactly, and no more: no trailing commas, no comments, no unquoted
//! keys, no single quotes, no leading `+`, no leading zeros, no `NaN` or
//! `Infinity`, no bare control characters inside strings. Any of those is an
//! [`Error`] carrying a byte offset and a reason, never a panic and never a
//! silent reinterpretation — a reader that quietly repairs a document is
//! showing something the file does not say. A top-level scalar (`5`, `"x"`,
//! `true`) is a valid document, as RFC 8259 §2 requires.
//!
//! Two things *are* lenient, both because a reader must open what it is given:
//! invalid UTF-8 inside a string becomes `U+FFFD` (the same lossy decode the
//! markdown and CSV sides do), and a lone surrogate escape — `\uD800` with no
//! pair — becomes `U+FFFD` rather than an error, since there is no other
//! character it could be.
//!
//! # One value, or a whole document
//!
//! [`parse`] requires the input to be exactly one value; [`parse_prefix`]
//! parses one value from the front and reports where it ended. The second is
//! what a `.jsonl` line and a concatenated stream both run on, so there is only
//! one parser to be right.
#![deny(unsafe_code)]
// The parser is complete on its own; the JSON `Source` above it (a later roll)
// is what reaches the rest of this surface. Everything here is exercised by
// `parse_tests.rs`.
#![allow(dead_code)]

use super::error::{Error, Reason};
use super::value::{Member, Number, Value};

/// How deep nesting may go before a document is refused.
///
/// Not a stack limit — there is no recursion to overflow — but a memory one:
/// every level is a heap frame plus a `Vec`, so an unbounded depth lets four
/// bytes of input buy an allocation. Ten thousand is far past any real
/// document (a large agent trajectory runs to about nine) and far short of
/// anything that costs.
pub const DEFAULT_MAX_DEPTH: usize = 10_000;

/// Parse `bytes` as exactly one JSON document.
pub fn parse(bytes: &[u8]) -> Result<Value, Error> {
    Parser::new().parse(bytes)
}

/// Parse `text` as exactly one JSON document.
pub fn parse_str(text: &str) -> Result<Value, Error> {
    Parser::new().parse(text.as_bytes())
}

/// Parse one value from the front of `bytes`, returning it and the offset one
/// past its last byte. Leading whitespace is skipped; trailing whitespace is
/// not consumed, so the offset points exactly at the end of the value.
pub fn parse_prefix(bytes: &[u8]) -> Result<(Value, usize), Error> {
    Parser::new().parse_prefix(bytes)
}

/// A configured parser. The only knob is the depth limit.
#[derive(Clone, Copy, Debug)]
pub struct Parser {
    max_depth: usize,
}

impl Default for Parser {
    fn default() -> Parser {
        Parser::new()
    }
}

impl Parser {
    pub fn new() -> Parser {
        Parser { max_depth: DEFAULT_MAX_DEPTH }
    }

    /// Refuse documents nested deeper than `n` containers.
    pub fn max_depth(mut self, n: usize) -> Parser {
        self.max_depth = n;
        self
    }

    /// One value, and nothing but whitespace after it.
    pub fn parse(&self, bytes: &[u8]) -> Result<Value, Error> {
        let mut p = Scan { b: bytes, i: 0, max_depth: self.max_depth };
        let v = p.value()?;
        p.ws();
        match p.b.get(p.i) {
            None => Ok(v),
            Some(&b) => Err(Error { offset: p.i, reason: Reason::Trailing(b) }),
        }
    }

    /// One value from the front, plus where it ended.
    pub fn parse_prefix(&self, bytes: &[u8]) -> Result<(Value, usize), Error> {
        let mut p = Scan { b: bytes, i: 0, max_depth: self.max_depth };
        let v = p.value()?;
        Ok((v, p.i))
    }
}

/// A container being filled. `Obj` carries the key whose value is being read.
enum Frame {
    Arr(Vec<Value>),
    Obj(Vec<Member>, String),
}

/// The cursor: input, position, limit. Holds no output — that lives on the
/// [`Frame`] stack inside [`Scan::value`].
struct Scan<'a> {
    b: &'a [u8],
    i: usize,
    max_depth: usize,
}

impl Scan<'_> {
    /// Parse one value, iteratively.
    ///
    /// `'down` restarts at "read a value here"; the inner loop hands a finished
    /// value to its parent and closes as many containers as the input closes.
    fn value(&mut self) -> Result<Value, Error> {
        let mut stack: Vec<Frame> = Vec::new();
        'down: loop {
            self.ws();
            let mut v = match self.peek()? {
                b'[' => match self.open(&mut stack, b']')? {
                    Some(empty) => empty,
                    None => continue 'down,
                },
                b'{' => match self.open(&mut stack, b'}')? {
                    Some(empty) => empty,
                    None => {
                        self.key(&mut stack)?;
                        continue 'down;
                    }
                },
                _ => self.scalar()?,
            };
            loop {
                let Some(frame) = stack.last_mut() else { return Ok(v) };
                let obj = matches!(frame, Frame::Obj(..));
                push_member(frame, v);
                self.ws();
                match (self.peek()?, obj) {
                    (b',', false) => {
                        self.i += 1;
                        continue 'down;
                    }
                    (b',', true) => {
                        self.i += 1;
                        self.key(&mut stack)?;
                        continue 'down;
                    }
                    (b']', false) | (b'}', true) => {
                        self.i += 1;
                        v = close(&mut stack);
                    }
                    (other, _) => return Err(self.at(Reason::Unexpected(other))),
                }
            }
        }
    }

    /// Consume `[` or `{`, push its frame, and report whether the container was
    /// closed immediately: `Some(empty container)` if so, `None` when a first
    /// member follows.
    fn open(&mut self, stack: &mut Vec<Frame>, end: u8) -> Result<Option<Value>, Error> {
        if stack.len() >= self.max_depth {
            return Err(self.at(Reason::TooDeep(self.max_depth)));
        }
        self.i += 1;
        stack.push(match end {
            b']' => Frame::Arr(Vec::new()),
            _ => Frame::Obj(Vec::new(), String::new()),
        });
        self.ws();
        if self.peek()? != end {
            return Ok(None);
        }
        self.i += 1;
        Ok(Some(close(stack)))
    }

    /// Read `"key" :` and park the key on the open object frame.
    fn key(&mut self, stack: &mut [Frame]) -> Result<(), Error> {
        self.ws();
        if self.peek()? != b'"' {
            return Err(self.at(Reason::Unexpected(self.b[self.i])));
        }
        let name = self.string()?;
        self.ws();
        if self.peek()? != b':' {
            return Err(self.at(Reason::Unexpected(self.b[self.i])));
        }
        self.i += 1;
        if let Some(Frame::Obj(_, key)) = stack.last_mut() {
            *key = name;
        }
        Ok(())
    }

    /// A value with no children: string, number, `true`, `false` or `null`.
    fn scalar(&mut self) -> Result<Value, Error> {
        match self.peek()? {
            b'"' => Ok(Value::Str(self.string()?)),
            b'-' | b'0'..=b'9' => self.number(),
            b't' => self.literal(b"true", Value::Bool(true)),
            b'f' => self.literal(b"false", Value::Bool(false)),
            b'n' => self.literal(b"null", Value::Null),
            b => Err(self.at(Reason::Unexpected(b))),
        }
    }

    fn literal(&mut self, word: &'static [u8], v: Value) -> Result<Value, Error> {
        if !self.b[self.i..].starts_with(word) {
            let name = match word[0] {
                b't' => "true",
                b'f' => "false",
                _ => "null",
            };
            return Err(self.at(Reason::BadLiteral(name)));
        }
        self.i += word.len();
        Ok(v)
    }

    /// A number, kept as the source text it was written with. The grammar is
    /// RFC 8259 §6 exactly: optional `-`, an integer part with no leading zero,
    /// an optional fraction, an optional exponent.
    fn number(&mut self) -> Result<Value, Error> {
        let start = self.i;
        self.eat(b'-');
        match self.b.get(self.i) {
            None => return Err(self.eof()),
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.i += 1;
                self.digits();
            }
            Some(_) => return Err(self.at(Reason::BadNumber)),
        }
        if self.eat(b'.') && !self.digits() {
            return Err(self.at(Reason::BadNumber));
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if !self.digits() {
                return Err(self.at(Reason::BadNumber));
            }
        }
        // `01`, `1.2.3` and `1abc` end a valid number and then continue with
        // something only a number could have meant; reporting them here beats
        // reporting a stray `1` two frames up.
        if matches!(self.b.get(self.i), Some(c) if c.is_ascii_alphanumeric() || *c == b'.') {
            return Err(self.at(Reason::BadNumber));
        }
        let text = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
        Ok(Value::Number(Number::new(text)))
    }

    /// A string, starting at its opening quote. Escapes are resolved; the raw
    /// bytes between them are copied in runs and lossily decoded at the end, so
    /// invalid UTF-8 becomes `U+FFFD` instead of an error.
    fn string(&mut self) -> Result<String, Error> {
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        let mut run = self.i;
        loop {
            let Some(&b) = self.b.get(self.i) else { return Err(self.eof()) };
            match b {
                b'"' => {
                    out.extend_from_slice(&self.b[run..self.i]);
                    self.i += 1;
                    return Ok(decode(out));
                }
                b'\\' => {
                    out.extend_from_slice(&self.b[run..self.i]);
                    self.i += 1;
                    self.escape(&mut out)?;
                    run = self.i;
                }
                0x00..=0x1f => return Err(self.at(Reason::Control(b))),
                _ => self.i += 1,
            }
        }
    }

    /// One escape, the backslash already consumed.
    fn escape(&mut self, out: &mut Vec<u8>) -> Result<(), Error> {
        let Some(&b) = self.b.get(self.i) else { return Err(self.eof()) };
        self.i += 1;
        let plain = match b {
            b'"' => b'"',
            b'\\' => b'\\',
            b'/' => b'/',
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'u' => {
                let c = self.unicode()?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                return Ok(());
            }
            other => {
                self.i -= 1;
                return Err(self.at(Reason::BadEscape(other)));
            }
        };
        out.push(plain);
        Ok(())
    }

    /// A `\uXXXX` escape, the `u` already consumed, including the surrogate
    /// pairing rule. A high surrogate takes the following `\uXXXX` as its low
    /// half; anything else leaves it unpaired, and an unpaired surrogate — high
    /// or low — is `U+FFFD`, because it names no character. The second escape
    /// is *not* consumed when it turns out not to be a low surrogate: it is
    /// re-read as an escape of its own.
    fn unicode(&mut self) -> Result<char, Error> {
        let u = self.hex4()?;
        if (0xdc00..0xe000).contains(&u) {
            return Ok(char::REPLACEMENT_CHARACTER);
        }
        if !(0xd800..0xdc00).contains(&u) {
            return Ok(char::from_u32(u as u32).unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        let save = self.i;
        if self.b[self.i..].starts_with(b"\\u") {
            self.i += 2;
            match self.hex4() {
                Ok(lo) if (0xdc00..0xe000).contains(&lo) => {
                    let c = 0x1_0000 + ((u as u32 - 0xd800) << 10) + (lo as u32 - 0xdc00);
                    return Ok(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                _ => self.i = save,
            }
        }
        Ok(char::REPLACEMENT_CHARACTER)
    }

    /// Four hex digits, consumed.
    fn hex4(&mut self) -> Result<u16, Error> {
        let mut v = 0u16;
        for k in 0..4 {
            let Some(&b) = self.b.get(self.i + k) else { return Err(self.eof()) };
            let Some(d) = hex(b) else {
                return Err(Error { offset: self.i + k, reason: Reason::BadHex });
            };
            v = v * 16 + d as u16;
        }
        self.i += 4;
        Ok(v)
    }

    // -- cursor -------------------------------------------------------------

    /// RFC 8259 §2 whitespace: space, tab, LF, CR. Nothing else — a vertical
    /// tab or a NBSP between tokens is an error, not padding.
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn peek(&self) -> Result<u8, Error> {
        match self.b.get(self.i) {
            Some(&b) => Ok(b),
            None => Err(self.eof()),
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        let hit = self.b.get(self.i) == Some(&b);
        self.i += usize::from(hit);
        hit
    }

    /// Consume a run of ASCII digits; true when there was at least one.
    fn digits(&mut self) -> bool {
        let start = self.i;
        while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i > start
    }

    fn at(&self, reason: Reason) -> Error {
        Error { offset: self.i, reason }
    }

    /// End of input is always reported at the end, not at the cursor.
    fn eof(&self) -> Error {
        Error { offset: self.b.len(), reason: Reason::Eof }
    }
}

/// Attach a finished value to the container on top of the stack.
fn push_member(frame: &mut Frame, v: Value) {
    match frame {
        Frame::Arr(items) => items.push(v),
        Frame::Obj(members, key) => {
            members.push(Member { key: std::mem::take(key), value: v });
        }
    }
}

/// Pop the top frame and turn it into the value it was collecting.
fn close(stack: &mut Vec<Frame>) -> Value {
    match stack.pop() {
        Some(Frame::Arr(items)) => Value::Array(items),
        Some(Frame::Obj(members, _)) => Value::Object(members),
        // Unreachable: every caller has just checked the stack is non-empty.
        None => Value::Null,
    }
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Lossy, like every other document tread opens: a file is never rejected for
/// its encoding. Structural bytes are all ASCII, so only genuinely invalid
/// input degrades.
fn decode(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "depth_tests.rs"]
mod depth_tests;

#[cfg(test)]
#[path = "stream_tests.rs"]
mod stream_tests;
