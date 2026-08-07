//! The JSON value tree (SPEC.md §JSON, "Values").
//!
//! # What a reader has to preserve
//!
//! A reader shows what is in the file, so this tree is deliberately *not* the
//! convenient in-memory model a serialiser would pick:
//!
//! * **Numbers keep their source text.** [`Number`] is the literal as written.
//!   `1e999`, `0.1` and a 40-digit integer all survive exactly; [`Number::as_f64`]
//!   exists for anything that wants arithmetic and is documented as lossy.
//! * **Objects are an ordered `Vec` of [`Member`]s, not a map.** Duplicate keys
//!   are kept, in document order, because the document has them.
//! * **Strings are already decoded** — escapes resolved, invalid UTF-8 replaced
//!   with `U+FFFD` — but *not* sanitised: a string legitimately containing a
//!   `\r` is data, and neutralising controls for the terminal is the painter's
//!   job (`crate::md::sanitize`), exactly as it is for a CSV cell.
//!
//! # Nothing here recurses on nesting
//!
//! An iterative parser behind a recursive walker is still a crash, and that
//! includes the walkers the compiler writes: a derived `Drop`, `Clone`,
//! `PartialEq` or `Debug` on a tree ten thousand deep overflows the stack just
//! as surely as a hand-written one. So every one of those is implemented here
//! with an explicit stack, and `Vec<Value>` is never destroyed by the derived
//! recursive drop glue. See [`Value::drop`] in particular: it is load-bearing,
//! not a micro-optimisation.
#![deny(unsafe_code)]
// The value tree is complete on its own; the renderer and the `Source` above it
// (a later roll) are what reach the rest of this surface. Everything here is
// exercised by `value_tests.rs`.
#![allow(dead_code)]

use std::fmt;

use super::write;

/// What a value is, with its payload left out — for status lines, colouring and
/// error messages, none of which need the value itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

impl Kind {
    /// The name RFC 8259 uses, for messages.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Null => "null",
            Kind::Bool => "boolean",
            Kind::Number => "number",
            Kind::String => "string",
            Kind::Array => "array",
            Kind::Object => "object",
        }
    }
}

/// A number, stored as the source text that produced it.
///
/// The parser only ever builds one from bytes it has checked against the RFC
/// 8259 grammar, so the text is always a valid JSON number.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Number(String);

impl Number {
    /// Wrap literal text. Callers other than the parser are trusted to pass a
    /// valid JSON number; nothing here re-validates, because the parser has
    /// already done it and re-checking on every construction would be a cost
    /// paid per value.
    pub fn new(text: impl Into<String>) -> Number {
        Number(text.into())
    }

    /// The literal, exactly as the document wrote it.
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The value as an `f64`. **Lossy**, and knowingly so: `1e999` becomes
    /// `inf`, a 40-digit integer loses its tail, and `0.1` becomes the nearest
    /// double. Never display the result — display [`Number::text`]. This exists
    /// for code that needs to compare or compute, not for the screen.
    ///
    /// A literal Rust cannot parse (which the grammar should make impossible)
    /// yields `NaN` rather than panicking.
    pub fn as_f64(&self) -> f64 {
        self.0.parse::<f64>().unwrap_or(f64::NAN)
    }

    /// The value as an `i64` when it is an integer literal that fits, else
    /// `None`. Exact by construction: it is a parse of the source text, not a
    /// cast of [`Number::as_f64`].
    pub fn as_i64(&self) -> Option<i64> {
        self.0.parse::<i64>().ok()
    }

    /// True when the literal has no fraction and no exponent — `12` but not
    /// `12.0` and not `1e2`, because the document distinguishes them.
    pub fn is_integer(&self) -> bool {
        !self.0.contains(['.', 'e', 'E'])
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One key/value pair of an object. Objects hold a `Vec` of these, so a
/// duplicate key is two members and not a silent overwrite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub key: String,
    pub value: Value,
}

impl Member {
    pub fn new(key: impl Into<String>, value: Value) -> Member {
        Member { key: key.into(), value }
    }
}

/// An RFC 8259 value.
#[derive(Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<Member>),
}

impl Value {
    /// Convenience constructor for a number given as text.
    pub fn number(text: impl Into<String>) -> Value {
        Value::Number(Number::new(text))
    }

    /// Convenience constructor for a string.
    pub fn string(text: impl Into<String>) -> Value {
        Value::Str(text.into())
    }

    pub fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Bool(_) => Kind::Bool,
            Value::Number(_) => Kind::Number,
            Value::Str(_) => Kind::String,
            Value::Array(_) => Kind::Array,
            Value::Object(_) => Kind::Object,
        }
    }

    /// True for arrays and objects — the values that have members to index,
    /// fold and summarise.
    pub fn is_container(&self) -> bool {
        matches!(self, Value::Array(_) | Value::Object(_))
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<&Number> {
        match self {
            Value::Number(n) => Some(n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[Member]> {
        match self {
            Value::Object(members) => Some(members),
            _ => None,
        }
    }

    /// Members of a container, `0` for a scalar. This is the count a collapsed
    /// row shows (`{…5 keys}`), so a scalar answering `0` rather than `None`
    /// keeps the caller free of a special case.
    pub fn len(&self) -> usize {
        match self {
            Value::Array(items) => items.len(),
            Value::Object(members) => members.len(),
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `n`th member of an array (or the `n`th value of an object, which is
    /// how a path like `.users[3]` addresses a row of either).
    pub fn index(&self, n: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(n),
            Value::Object(members) => members.get(n).map(|m| &m.value),
            _ => None,
        }
    }

    /// The **first** member named `key`. Duplicates are kept in the tree; a
    /// caller that only wants one gets the first, which is the one a
    /// last-writer-wins consumer would disagree about — see [`Value::get_all`].
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.iter().find(|m| m.key == key).map(|m| &m.value)
    }

    /// Every member named `key`, in document order.
    pub fn get_all<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
        self.as_object()
            .unwrap_or(&[])
            .iter()
            .filter(move |m| m.key == key)
            .map(|m| &m.value)
    }

    /// Replace this value with `null` and return what was there. The way to get
    /// ownership of a subtree: [`Value`] has a hand-written [`Drop`], which
    /// costs it by-value destructuring.
    pub fn take(&mut self) -> Value {
        std::mem::replace(self, Value::Null)
    }

    /// Nesting depth: `1` for a scalar or an empty container, `2` for `[1]`.
    /// Iterative, like everything else here.
    pub fn depth(&self) -> usize {
        let mut deepest = 0usize;
        let mut stack: Vec<(&Value, usize)> = vec![(self, 1)];
        while let Some((v, d)) = stack.pop() {
            deepest = deepest.max(d);
            match v {
                Value::Array(items) => stack.extend(items.iter().map(|c| (c, d + 1))),
                Value::Object(ms) => stack.extend(ms.iter().map(|m| (&m.value, d + 1))),
                _ => {}
            }
        }
        deepest
    }

    /// This value as compact JSON — numbers verbatim, strings re-escaped. See
    /// [`super::write::to_compact`].
    pub fn to_json(&self) -> String {
        write::to_compact(self)
    }
}

/// Dismantle the tree with an explicit stack.
///
/// Without this, dropping a value nested ten thousand deep overflows the stack
/// inside the compiler's own drop glue — the parser being iterative buys
/// nothing if letting go of its result crashes. Each value taken off the stack
/// has its children moved onto the stack *before* it is dropped, so the
/// recursive glue only ever runs against already-emptied containers and the
/// real depth is 1.
impl Drop for Value {
    fn drop(&mut self) {
        let mut stack: Vec<Value> = Vec::new();
        drain_into(self, &mut stack);
        while let Some(mut v) = stack.pop() {
            drain_into(&mut v, &mut stack);
            // `v` drops here, already childless.
        }
    }
}

/// Move a container's children onto `stack`, leaving it empty. A scalar is a
/// no-op, which is the common case and costs nothing.
fn drain_into(v: &mut Value, stack: &mut Vec<Value>) {
    match v {
        Value::Array(items) => stack.append(items),
        Value::Object(members) => stack.extend(members.drain(..).map(|m| m.value)),
        _ => {}
    }
}

impl Clone for Value {
    /// Iterative deep copy: children are cloned bottom-up off a work stack and
    /// reassembled, so depth costs heap and never stack.
    fn clone(&self) -> Value {
        let mut work: Vec<Step<'_>> = vec![Step::Val(self)];
        let mut done: Vec<Value> = Vec::new();
        while let Some(step) = work.pop() {
            match step {
                Step::Val(Value::Array(items)) => {
                    work.push(Step::CloseArr(items.len()));
                    work.extend(items.iter().rev().map(Step::Val));
                }
                Step::Val(Value::Object(ms)) => {
                    work.push(Step::CloseObj(ms));
                    work.extend(ms.iter().rev().map(|m| Step::Val(&m.value)));
                }
                Step::Val(scalar) => done.push(clone_scalar(scalar)),
                Step::CloseArr(n) => {
                    let kids = done.split_off(done.len() - n);
                    done.push(Value::Array(kids));
                }
                Step::CloseObj(ms) => {
                    let kids = done.split_off(done.len() - ms.len());
                    let pairs = ms.iter().zip(kids);
                    done.push(Value::Object(
                        pairs.map(|(m, v)| Member { key: m.key.clone(), value: v }).collect(),
                    ));
                }
            }
        }
        done.pop().unwrap_or(Value::Null)
    }
}

/// One unit of the iterative clone: a value to copy, or a container to
/// reassemble from the last `n` finished children.
enum Step<'a> {
    Val(&'a Value),
    CloseArr(usize),
    CloseObj(&'a [Member]),
}

/// Copy a value that has no children. Callers must not pass a container.
fn clone_scalar(v: &Value) -> Value {
    match v {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) => Value::Number(n.clone()),
        Value::Str(s) => Value::Str(s.clone()),
        // Unreachable via `Value::clone`, which never routes a container here;
        // an empty stand-in is still better than a panic in a reader.
        Value::Array(_) => Value::Array(Vec::new()),
        Value::Object(_) => Value::Object(Vec::new()),
    }
}

impl PartialEq for Value {
    /// Iterative structural equality. Objects compare member-wise *in order*,
    /// so `{"a":1,"b":2}` and `{"b":2,"a":1}` differ — the document order is
    /// part of what this tree preserves. Numbers compare by source text, so
    /// `1` and `1.0` differ, for the same reason.
    fn eq(&self, other: &Value) -> bool {
        let mut stack: Vec<(&Value, &Value)> = vec![(self, other)];
        while let Some((a, b)) = stack.pop() {
            match (a, b) {
                (Value::Null, Value::Null) => {}
                (Value::Bool(x), Value::Bool(y)) if x == y => {}
                (Value::Number(x), Value::Number(y)) if x == y => {}
                (Value::Str(x), Value::Str(y)) if x == y => {}
                (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
                    stack.extend(x.iter().zip(y.iter()));
                }
                (Value::Object(x), Value::Object(y)) if x.len() == y.len() => {
                    if x.iter().zip(y.iter()).any(|(m, n)| m.key != n.key) {
                        return false;
                    }
                    stack.extend(x.iter().zip(y.iter()).map(|(m, n)| (&m.value, &n.value)));
                }
                _ => return false,
            }
        }
        true
    }
}

impl Eq for Value {}

/// Compact JSON, which is both the honest rendering of a value and a form that
/// pastes back into the document it came from. Not derived: the derived version
/// recurses on nesting.
impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&write::to_compact(self))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&write::to_compact(self))
    }
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod tests;
