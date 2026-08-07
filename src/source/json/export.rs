//! `--to-jsonl`, and the structural minifier it shares with `Y`
//! (SPEC.md §JSON, "`--to-jsonl`").
//!
//! > Writes a top-level array to stdout as one element per line […] It streams
//! > — it must not hold the document in memory to write it.
//!
//! So it does not parse. The array is walked by the same structural scanner the
//! reader indexes with, and each element is copied from the file to the output
//! with its insignificant whitespace removed — a byte loop with an in-string
//! flag and an escape flag. Nothing is buffered beyond one read window, a
//! pretty-printed element becomes one line without ever being a value, and a
//! number keeps the exact text the document wrote (a round-trip through `f64`
//! would not).
//!
//! An export, never a cache: the reader writes it only when asked.
#![deny(unsafe_code)]

use std::io::{self, Write};

use crate::csv::read::{Reader, WINDOW};
use crate::json::index::{root, Member, Scan, Shape};

/// Turn the document behind `reader` into JSON Lines on `out`.
///
/// Refuses anything but a top-level array, with the reason: an object has keys
/// that a line-per-record file cannot carry, and a scalar is one record already.
pub fn to_jsonl(mut reader: Reader, out: &mut dyn Write) -> Result<(), String> {
    let size = reader.size();
    let head = reader.chunk(0, 64).to_vec();
    let (start, shape) = root(&head, 0).ok_or("the document is empty")?;
    if shape != Shape::Array {
        return Err(format!(
            "{} \u{2014} --to-jsonl writes one array element per line",
            describe(shape)
        ));
    }
    let mut scan = Scan::new(start, false);
    let mut spans: Vec<Member> = Vec::new();
    // One output buffer for the whole export, reused element by element: five
    // million records must not be five million allocations.
    let mut line: Vec<u8> = Vec::with_capacity(WINDOW);
    while !scan.done() {
        let at = scan.pos();
        let buf = match at >= size {
            true => Vec::new(),
            false => reader.chunk(at, WINDOW).to_vec(),
        };
        // No bytes left: settle the array here rather than spinning on an
        // offset that will never yield another one. A document cut off
        // mid-element still writes the elements it did have.
        match buf.is_empty() {
            true => scan.finish(at.min(size), &mut |m| spans.push(m)),
            false => scan.feed(&buf, &mut |m| spans.push(m)),
        }
        for m in spans.drain(..) {
            match write_member(m, &buf, at, &mut reader, out, &mut line) {
                Ok(()) => {}
                // `| head` closes the pipe: the reader has what it asked for,
                // and that is not a failure of the export (main.rs treats a
                // closed stdout the same way).
                Err(ref e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    match out.flush() {
        Err(ref e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        other => other.map_err(|e| e.to_string()),
    }
}

fn describe(shape: Shape) -> &'static str {
    match shape {
        Shape::Object => "the top-level value is an object, not an array",
        Shape::Str => "the top-level value is a string, not an array",
        Shape::Number => "the top-level value is a number, not an array",
        Shape::Bool => "the top-level value is a boolean, not an array",
        Shape::Null => "the top-level value is null, not an array",
        Shape::Bad => "the document does not begin with a JSON value",
        Shape::Array => "the top-level value is an array",
    }
}

/// One element, taken from the window it was found in when it fits there and
/// re-read from the file when it straddles a boundary.
fn write_member(
    m: Member,
    buf: &[u8],
    base: u64,
    reader: &mut Reader,
    out: &mut dyn Write,
    line: &mut Vec<u8>,
) -> io::Result<()> {
    let end = base + buf.len() as u64;
    if m.start < base || m.end > end {
        return write_one(m, reader, out, line);
    }
    let (s, e) = ((m.start - base) as usize, (m.end - base) as usize);
    line.clear();
    Min::default().push(&buf[s..e], line);
    line.push(b'\n');
    out.write_all(line)
}

/// One element, streamed from the file a window at a time: an element larger
/// than the read window never exists in memory whole.
fn write_one(
    m: Member,
    reader: &mut Reader,
    out: &mut dyn Write,
    piece: &mut Vec<u8>,
) -> io::Result<()> {
    let mut min = Min::default();
    let mut at = m.start;
    while at < m.end {
        let want = (m.end - at).min(WINDOW as u64) as usize;
        let chunk = reader.chunk(at, want).to_vec();
        if chunk.is_empty() {
            break;
        }
        at += chunk.len() as u64;
        piece.clear();
        min.push(&chunk, piece);
        out.write_all(piece)?;
    }
    out.write_all(b"\n")
}

/// A resumable structural minifier: whitespace between tokens is dropped,
/// whitespace inside a string is kept.
///
/// Not a serialiser — the bytes are copied, not re-encoded — so a number, an
/// escape and a duplicate key all come out exactly as the document wrote them.
#[derive(Default)]
pub struct Min {
    in_str: bool,
    esc: bool,
}

impl Min {
    pub fn push(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        for &b in chunk {
            if self.in_str {
                out.push(b);
                if self.esc {
                    self.esc = false;
                } else if b == b'\\' {
                    self.esc = true;
                } else if b == b'"' {
                    self.in_str = false;
                }
                continue;
            }
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => {}
                b'"' => {
                    self.in_str = true;
                    out.push(b);
                }
                _ => out.push(b),
            }
        }
    }
}

/// `src` with its insignificant whitespace removed. What `Y` copies: the
/// subtree exactly as the document has it, on one line.
pub fn minify(src: &[u8]) -> String {
    let mut out = Vec::with_capacity(src.len());
    Min::default().push(src, &mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
