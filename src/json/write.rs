//! Values back to JSON text (SPEC.md §JSON — `Y` yanks a subtree "as valid
//! JSON").
//!
//! Source-faithful, which for a reader means two specific things:
//!
//! * a number is written as the document wrote it, never round-tripped through
//!   `f64` — see [`super::value::Number`];
//! * a string is re-escaped to the RFC 8259 minimum: `"` and `\` are escaped,
//!   the C0 controls take their short forms where they have one and `\u00xx`
//!   otherwise, and every other scalar — including `/`, `DEL` and anything
//!   above ASCII — is written literally, because escaping it would change the
//!   text a reader is trying to copy without changing what it means.
//!
//! The walk is iterative for the same reason the parser is: a subtree yanked
//! out of a pathologically nested document must serialise, not crash. Only the
//! frame stack grows with depth, one small frame per level.
#![deny(unsafe_code)]
#![allow(dead_code)]

use super::value::{Member, Value};

/// `value` as compact JSON: no spaces, no newlines.
pub fn to_compact(value: &Value) -> String {
    let mut out = String::new();
    write_compact(value, &mut out);
    out
}

/// One open container being written: what is left of it, and whether a comma
/// is owed before the next member.
enum Frame<'a> {
    Arr(std::slice::Iter<'a, Value>),
    Obj(std::slice::Iter<'a, Member>),
}

/// Append `value` to `out` as compact JSON.
pub fn write_compact(value: &Value, out: &mut String) {
    let mut stack: Vec<(Frame<'_>, bool)> = Vec::new();
    let mut next = Some(value);
    loop {
        if let Some(v) = next.take() {
            match v {
                Value::Array(items) => {
                    out.push('[');
                    stack.push((Frame::Arr(items.iter()), false));
                }
                Value::Object(members) => {
                    out.push('{');
                    stack.push((Frame::Obj(members.iter()), false));
                }
                scalar => write_scalar(scalar, out),
            }
        }
        match stack.last_mut() {
            None => return,
            Some((frame, seen)) => {
                if !advance(frame, seen, out, &mut next) {
                    stack.pop();
                }
            }
        }
    }
}

/// Take the next member of `frame`, writing its separator and (for an object)
/// its key. Returns false when the container is exhausted, having written its
/// closing bracket.
fn advance<'a>(
    frame: &mut Frame<'a>,
    seen: &mut bool,
    out: &mut String,
    next: &mut Option<&'a Value>,
) -> bool {
    let value = match frame {
        Frame::Arr(it) => match it.next() {
            Some(v) => v,
            None => {
                out.push(']');
                return false;
            }
        },
        Frame::Obj(it) => match it.next() {
            Some(m) => {
                if *seen {
                    out.push(',');
                }
                *seen = true;
                escape_into(&m.key, out);
                out.push(':');
                *next = Some(&m.value);
                return true;
            }
            None => {
                out.push('}');
                return false;
            }
        },
    };
    if *seen {
        out.push(',');
    }
    *seen = true;
    *next = Some(value);
    true
}

/// A value with no children. Containers are handled by the walk itself.
fn write_scalar(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        // Verbatim. The whole point of keeping the source text.
        Value::Number(n) => out.push_str(n.text()),
        Value::Str(s) => escape_into(s, out),
        // Only reachable if a caller hands a container to a scalar writer;
        // an empty literal keeps the output valid JSON either way.
        Value::Array(_) => out.push_str("[]"),
        Value::Object(_) => out.push_str("{}"),
    }
}

/// Write `s` as a quoted, escaped JSON string.
pub fn escape_into(s: &str, out: &mut String) {
    out.push('"');
    // Fast path: nothing in the string needs a backslash, so it is copied whole.
    if !s.bytes().any(needs_escape) {
        out.push_str(s);
        out.push('"');
        return;
    }
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => push_hex(c as u32, out),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `s` as a quoted, escaped JSON string.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    escape_into(s, &mut out);
    out
}

fn needs_escape(b: u8) -> bool {
    b < 0x20 || b == b'"' || b == b'\\'
}

/// `\u00xx` for a control character with no short form.
fn push_hex(c: u32, out: &mut String) {
    const HEX: [u8; 16] = *b"0123456789abcdef";
    out.push_str("\\u00");
    out.push(HEX[(c >> 4 & 0xf) as usize] as char);
    out.push(HEX[(c & 0xf) as usize] as char);
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
