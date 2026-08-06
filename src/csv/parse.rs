//! RFC 4180 parsing, hand-written, byte by byte (SPEC.md §CSV).
//!
//! # One state machine, two callers
//!
//! The row index needs to know where a row *ends* without materialising a
//! single field; the renderer needs the fields of one row without rescanning
//! the file. If those two answers came from two pieces of code they would
//! eventually disagree about a newline inside a quoted field, and every byte
//! offset after that point would be wrong. So there is exactly one machine —
//! [`Scanner`] — and both callers drive it:
//!
//! * the indexer feeds it bytes and watches for [`Event::EndRow`], via
//!   [`scan_row_ends`], keeping only offsets;
//! * the renderer feeds it one row's bytes and collects the values, via
//!   [`Records`] / [`record`].
//!
//! [`Scanner`] is `Copy`, holds no buffers, and is resumable across arbitrary
//! chunk boundaries — including one that splits a `\r\n`.
//!
//! # What it accepts
//!
//! Quoted fields with embedded delimiters, `LF`, `CR` and `CRLF`; `""` as a
//! literal quote; unquoted, empty and trailing-empty fields; a leading BOM;
//! `LF`, `CRLF` and bare-`CR` row endings; padding spaces around a quoted
//! field. Ragged rows are reported at their true arity — trimming or padding a
//! row to the header's arity is the caller's policy, not the parser's, so no
//! data is silently dropped here (see [`fit`]).
//!
//! # What it does with garbage
//!
//! Never panics, never refuses. An unterminated quote runs to EOF and yields
//! the row it had; a stray quote inside a quoted field (`"a"b"`) closes the
//! field and the rest is taken literally, which is what Python's non-strict
//! `csv` does; a quote inside an unquoted field is literal; NUL and other
//! control bytes are content. Invalid UTF-8 becomes `U+FFFD` rather than an
//! error — the same lossy decode the markdown side does in
//! [`crate::md::sanitize::decode`]. Field text is returned *raw*: the caller
//! sanitises before painting, because a cell legitimately containing a `\r\n`
//! must survive as data even though it can never be sent to a terminal.
#![deny(unsafe_code)]
// This module is the CSV format's foundation and is complete on its own: it is
// the row index and the CSV `Source` above it that call most of the surface,
// and until both are wired in the binary reaches only part of it. Everything
// here is exercised by `parse_tests.rs`; drop this allow once the CSV source is
// the one driving it.
#![allow(dead_code)]

// Which byte separates fields is policy and lives in `super::delim`; the
// machine below is told which one to use and never guesses.

/// RFC 4180 fixes the quote character; there is no option for it.
pub const QUOTE: u8 = b'"';

/// UTF-8 byte-order mark, which a spreadsheet export very often writes.
pub const BOM: [u8; 3] = [0xef, 0xbb, 0xbf];

/// `3` when `bytes` starts with a UTF-8 BOM, else `0`. The row index adds this
/// to the first row's offset so the BOM never lands inside a field.
pub fn bom_len(bytes: &[u8]) -> usize {
    usize::from(bytes.starts_with(&BOM)) * BOM.len()
}

/// `bytes` without a leading BOM.
pub fn strip_bom(bytes: &[u8]) -> &[u8] {
    &bytes[bom_len(bytes)..]
}

/// What one byte did to the record being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Still inside the same field, or between fields with nothing decided.
    Continue,
    /// The current field is complete; the byte itself is the delimiter.
    EndField,
    /// The current field and row are complete; the byte is the last byte of
    /// the terminator, so the row ends *after* it.
    EndRow,
    /// The row ended *before* this byte — a bare `CR` whose successor turned
    /// out not to be `LF`. The row ends before the byte, and the byte must be
    /// fed to [`Scanner::step`] again as the first byte of the next row.
    EndRowBefore,
}

/// The effect of one byte: text to append to the current field, then `event`.
///
/// `spaces` is padding that was being held back in case a quote followed it
/// (` "a,b"` is one field, `  x` is a field with two leading spaces); when the
/// held-back run turns out to be ordinary content it is released here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub spaces: u32,
    pub push: Option<u8>,
    pub event: Event,
}

impl Step {
    const SKIP: Step = Step { spaces: 0, push: None, event: Event::Continue };

    fn keep(b: u8) -> Step {
        Step { push: Some(b), ..Step::SKIP }
    }
}

/// Where the machine is inside a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    /// Start of a field, nothing seen yet: a quote here opens a quoted field.
    FieldStart,
    /// Only spaces so far: a quote still opens a quoted field.
    Pad,
    /// An ordinary field. Quotes from here on are literal.
    Unquoted,
    /// Inside quotes: delimiters and newlines are content.
    Quoted,
    /// Just saw a quote inside a quoted field — closing quote, or the first
    /// half of an escaped `""`.
    QuoteEnd,
    /// Spaces after a closing quote, held back like [`St::Pad`].
    QuoteTail,
    /// Saw a `CR` outside quotes; the row has ended but the terminator's
    /// length is not known until the next byte (or EOF) arrives.
    Cr,
}

/// The RFC 4180 state machine: one delimiter, seven states, no allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scanner {
    delim: u8,
    st: St,
    pad: u32,
    /// Has the current row consumed a byte? Distinguishes "file ends after a
    /// terminator" (no trailing row) from "file ends mid-row" (one more row).
    started: bool,
}

impl Scanner {
    pub fn new(delim: u8) -> Scanner {
        Scanner { delim, st: St::FieldStart, pad: 0, started: false }
    }

    pub fn delim(&self) -> u8 {
        self.delim
    }

    /// True when a newline right now would be *content*, not a row boundary.
    /// The row index consults this when it resumes on a chunk boundary.
    pub fn in_quotes(&self) -> bool {
        self.st == St::Quoted
    }

    /// True when nothing of the current row has been consumed yet.
    pub fn at_row_start(&self) -> bool {
        !self.started && self.st == St::FieldStart
    }

    /// True when the machine is holding a `CR` whose successor has not arrived:
    /// the row has ended, but whether its terminator is one byte or two is not
    /// decided until the next byte is (or the input stops).
    pub fn pending_cr(&self) -> bool {
        self.st == St::Cr
    }

    /// Consume one byte. The only way state advances.
    pub fn step(&mut self, b: u8) -> Step {
        if self.st == St::Cr {
            self.reset_row();
            let event = if b == b'\n' { Event::EndRow } else { Event::EndRowBefore };
            return Step { event, ..Step::SKIP };
        }
        self.started = true;
        match self.st {
            St::Quoted if b == QUOTE => {
                self.st = St::QuoteEnd;
                Step::SKIP
            }
            St::Quoted => Step::keep(b),
            St::QuoteEnd => self.after_quote(b),
            _ => self.outside(b),
        }
    }

    /// Report the row that the end of input leaves open, if any. `None` when
    /// the input ended cleanly on a terminator (no phantom trailing row) or
    /// was empty. An unterminated quote ends here too, with what it had.
    pub fn finish(&mut self) -> Option<Step> {
        if self.st == St::Cr {
            self.reset_row();
            return Some(Step { event: Event::EndRow, ..Step::SKIP });
        }
        if self.at_row_start() {
            return None;
        }
        let spaces = self.close_pad();
        self.reset_row();
        Some(Step { spaces, push: None, event: Event::EndRow })
    }

    // -- internals ------------------------------------------------------------

    /// `FieldStart`, `Pad`, `Unquoted` and `QuoteTail`: outside any quotes.
    fn outside(&mut self, b: u8) -> Step {
        if b == self.delim {
            return self.end_field();
        }
        match b {
            b'\n' => self.end_row(),
            b'\r' => {
                let spaces = self.close_pad();
                self.st = St::Cr;
                Step { spaces, push: None, event: Event::Continue }
            }
            QUOTE if matches!(self.st, St::FieldStart | St::Pad) => {
                self.pad = 0;
                self.st = St::Quoted;
                Step::SKIP
            }
            b' ' if matches!(self.st, St::FieldStart | St::Pad | St::QuoteTail) => {
                self.pad += 1;
                if self.st == St::FieldStart {
                    self.st = St::Pad;
                }
                Step::SKIP
            }
            _ => {
                self.st = St::Unquoted;
                Step { spaces: self.take_pad(), push: Some(b), event: Event::Continue }
            }
        }
    }

    /// Just after a quote that closed a quoted field.
    fn after_quote(&mut self, b: u8) -> Step {
        if b == self.delim {
            return self.end_field();
        }
        match b {
            QUOTE => {
                self.st = St::Quoted;
                Step::keep(QUOTE)
            }
            b'\n' => self.end_row(),
            b'\r' => {
                self.st = St::Cr;
                Step::SKIP
            }
            b' ' => {
                self.st = St::QuoteTail;
                self.pad += 1;
                Step::SKIP
            }
            // `"a"b` — a stray quote. Non-strict recovery: the quote closed
            // the field and the tail is literal, so nothing is lost but the
            // quote itself.
            _ => {
                self.st = St::Unquoted;
                Step::keep(b)
            }
        }
    }

    fn end_field(&mut self) -> Step {
        let spaces = self.close_pad();
        self.st = St::FieldStart;
        Step { spaces, push: None, event: Event::EndField }
    }

    fn end_row(&mut self) -> Step {
        let spaces = self.close_pad();
        self.reset_row();
        Step { spaces, push: None, event: Event::EndRow }
    }

    /// Release held-back padding as the field closes. Padding *after* a
    /// closing quote (`"a" ,b`) is layout, not data, so it is dropped;
    /// padding anywhere else (`a  ,b`) is content and survives.
    fn close_pad(&mut self) -> u32 {
        if self.st == St::QuoteTail {
            self.pad = 0;
        }
        self.take_pad()
    }

    fn reset_row(&mut self) {
        self.st = St::FieldStart;
        self.pad = 0;
        self.started = false;
    }

    fn take_pad(&mut self) -> u32 {
        std::mem::replace(&mut self.pad, 0)
    }
}

/// Feed `bytes` — a chunk whose first byte is at file offset `base` — to `sc`,
/// reporting the absolute offset one past each row terminator that falls
/// inside it. A row still open when the chunk runs out is not reported; keep
/// `sc` and call again with the next chunk.
///
/// This is the row index's whole contract with the parser: no fields are built
/// and nothing is allocated, but the boundaries are the *same* boundaries
/// [`Records`] sees, because they come from the same [`Scanner`].
pub fn scan_row_ends(sc: &mut Scanner, bytes: &[u8], base: u64, mut on_row: impl FnMut(u64)) {
    let mut i = 0usize;
    while i < bytes.len() {
        match sc.step(bytes[i]).event {
            Event::Continue | Event::EndField => i += 1,
            Event::EndRow => {
                i += 1;
                on_row(base + i as u64);
            }
            // The row ended before this byte; re-feed it as the next row's
            // first byte. `EndRowBefore` only ever comes out of `St::Cr`, and
            // the re-fed byte starts from `St::FieldStart`, so this cannot loop.
            Event::EndRowBefore => on_row(base + i as u64),
        }
    }
}

/// How the input ended, which the row index needs in two different ways.
///
/// Whether there is a trailing row decides the row *count*; whether that row
/// carries a terminator decides how many bytes of it are data. Those are not
/// the same question — a file ending in a bare `CR` has a terminated last row
/// whose terminator might still grow into a `CRLF` — so they are two variants
/// rather than one `bool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tail {
    /// Input ended on a complete terminator. There is no trailing row.
    None,
    /// A row ran to end of input with no terminator at all: every byte of it,
    /// including a final `LF` or `CR` inside quotes, is data.
    Open,
    /// A row ended on a bare `CR` that end-of-input froze. It *is* terminated,
    /// but one more byte could turn that `CR` into a `CRLF`, so a file that
    /// grows must rescan the row rather than assume where it ended.
    Cr,
}

impl Tail {
    /// A trailing row exists — the row count is one higher than the number of
    /// terminators seen.
    pub fn has_row(self) -> bool {
        self != Tail::None
    }

    /// The trailing row's last byte is a terminator, so stripping one is right.
    pub fn terminated(self) -> bool {
        self != Tail::Open
    }
}

/// Close the last row at end of input. See [`Tail`].
pub fn finish_row_end(sc: &mut Scanner) -> Tail {
    let cr = sc.pending_cr();
    match (sc.finish().is_some(), cr) {
        (false, _) => Tail::None,
        (true, true) => Tail::Cr,
        (true, false) => Tail::Open,
    }
}

/// One parsed record and the byte range it occupied, terminator included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub fields: Vec<String>,
    pub start: usize,
    pub end: usize,
}

/// Records of an in-memory buffer, in order. Lazy: each `next` parses exactly
/// one row.
pub struct Records<'a> {
    bytes: &'a [u8],
    i: usize,
    sc: Scanner,
}

impl<'a> Records<'a> {
    /// Starts after a BOM, if there is one.
    pub fn new(bytes: &'a [u8], delim: u8) -> Records<'a> {
        Records { bytes, i: bom_len(bytes), sc: Scanner::new(delim) }
    }
}

impl Iterator for Records<'_> {
    type Item = Record;

    fn next(&mut self) -> Option<Record> {
        let start = self.i;
        let mut fields: Vec<String> = Vec::new();
        let mut cur: Vec<u8> = Vec::new();
        loop {
            let Some(&b) = self.bytes.get(self.i) else {
                let step = self.sc.finish()?;
                apply(step, &mut cur);
                fields.push(decode(cur));
                return Some(Record { fields, start, end: self.i });
            };
            let step = self.sc.step(b);
            apply(step, &mut cur);
            match step.event {
                Event::Continue => self.i += 1,
                Event::EndField => {
                    self.i += 1;
                    fields.push(decode(std::mem::take(&mut cur)));
                }
                Event::EndRow => {
                    self.i += 1;
                    fields.push(decode(cur));
                    return Some(Record { fields, start, end: self.i });
                }
                Event::EndRowBefore => {
                    fields.push(decode(cur));
                    return Some(Record { fields, start, end: self.i });
                }
            }
        }
    }
}

fn apply(step: Step, cur: &mut Vec<u8>) {
    for _ in 0..step.spaces {
        cur.push(b' ');
    }
    if let Some(b) = step.push {
        cur.push(b);
    }
}

/// Lossy, like every other document tread opens: a file is never rejected for
/// its encoding. Structural bytes are all ASCII, so a multi-byte scalar is
/// never split by the machine and only genuinely invalid input degrades.
fn decode(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

/// The fields of a single record — the row index's `offset -> row` call.
/// Anything after the first terminator in `bytes` is ignored, so handing it a
/// row slice with or without its terminator gives the same answer.
///
/// The caller has already decided that these bytes *are* a row, so an empty
/// slice is one empty field rather than nothing: a blank line in the middle of
/// a file is a row, and [`records`] counts it as one. Returning `[]` here
/// instead would make the row index disagree with the whole-file parse about
/// every blank row.
pub fn record(bytes: &[u8], delim: u8) -> Vec<String> {
    match Records::new(bytes, delim).next() {
        Some(r) => r.fields,
        None => vec![String::new()],
    }
}

/// Every record of a buffer. For sniffing, tests and small files only — a file
/// too big to load must go through the row index instead.
pub fn records(bytes: &[u8], delim: u8) -> Vec<Vec<String>> {
    Records::new(bytes, delim).map(|r| r.fields).collect()
}

/// Ragged-row policy, applied by the *caller* once it knows the header's
/// arity: pad a short row with empty cells, and report how many cells a long
/// row has beyond `arity` so the display can mark them rather than pretend
/// they were never there.
pub fn fit(fields: &mut Vec<String>, arity: usize) -> usize {
    let extra = fields.len().saturating_sub(arity);
    fields.resize(arity.max(fields.len()), String::new());
    extra
}

/// Strip one row terminator (`\n`, `\r\n` or a lone `\r`) from the end of a
/// row's bytes.
///
/// Which byte sequences count as a terminator is the machine's rule, so it is
/// stated once, here, rather than open-coded by whoever holds the bytes. The
/// caller must already know the row *has* a terminator — a row that ran off the
/// end of the file does not, and its last byte can legitimately be an `LF`
/// inside a quoted field (see [`Tail::terminated`]).
pub fn strip_terminator(data: &mut Vec<u8>) {
    if data.last() == Some(&b'\n') {
        data.pop();
    }
    if data.last() == Some(&b'\r') {
        data.pop();
    }
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
