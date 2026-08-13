//! What a record *spent*, said without naming a format.
//!
//! The half the two usage dialects share: the counters a record can carry, the
//! exact total that becomes [`crate::lens::Summary::tokens`], and the one
//! spelling of the numeric block a row shows them in. `usage_agent.rs` reads a
//! Claude Code session log into it and `usage_atif.rs` reads an ATIF trajectory
//! into it; neither spells a number, and neither decides a row — that is still
//! `src/source/record/`'s.
//!
//! # Three different things a cell can say
//!
//! This is the whole design, and it is why every counter is an [`Option`]
//! rather than a `u64` that defaults to zero:
//!
//! * **a number** — the format recorded that value, and `0` means zero tokens
//!   were spent, which is a real and observable thing;
//! * **`-`** — this *record* did not record a field its format has. The column
//!   stays, because other records in the same file fill it;
//! * **no column at all** — the *format* has no such field. ATIF-v1.7 records no
//!   cache-creation counter of any kind, so a `usage-atif` row has three fields
//!   and never a fourth. A `0` there would read as "this agent wrote nothing to
//!   cache" when the truth is "this format does not record cache writes", and a
//!   `-` on every row of every file would be a column of nothing.
//!
//! The rule stated once and applied twice: a *format*-level absence removes the
//! column for the whole file, because alignment is what a number column is for
//! and alignment is per file; a *record*-level absence inside a format that has
//! the field prints `-`.
//!
//! # Why the block is a constant width
//!
//! A reader of this lens is scanning down one column. Four fields are
//! `4×8 + 3×2 = 38` columns and three are `3×8 + 2×2 = 28`, always, whatever
//! the numbers are — which is what [`crate::lens::tokens`] flooring to four
//! columns buys. The row is never wrapped (it scrolls), so a narrow terminal
//! pans across it rather than losing the action off the end.
#![deny(unsafe_code)]

use crate::json::Value;
use crate::lens::{tokens, Body, Part};

/// Columns a field's label gets, left-justified.
const LABEL: usize = 4;

/// Columns a field's number gets, right-aligned. [`crate::lens::tokens`]
/// promises never to exceed it.
const NUMBER: usize = 4;

/// Columns one field occupies: its label then its number, no separator, because
/// the label is short and the number is right-aligned into the gap.
pub const FIELD: usize = LABEL + NUMBER;

/// Columns between two fields.
const GAP: &str = "  ";

/// Between the numbers and what the record did.
const TURN: &str = "  \u{b7}  ";

/// What a record with no numbers at all shows in the number cell of a field its
/// format does have. Never `0`: a recorded zero and an unrecorded field are
/// different claims, and this lens exists to keep them apart.
const ABSENT: &str = "-";

/// The counters a record recorded, exactly as the file recorded them.
///
/// `None` is "this record did not say", not "zero" — see the module docs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Tokens sent to the model.
    pub input: Option<u64>,
    /// Tokens the model produced.
    pub output: Option<u64>,
    /// Tokens served from cache rather than re-sent.
    pub cache_read: Option<u64>,
    /// Tokens written *to* cache. No ATIF field maps to it.
    pub cache_new: Option<u64>,
}

impl Tokens {
    /// Every unit this record recorded, added once.
    ///
    /// **The four counters and nothing else.** That is the one thing a later
    /// reader of this file will be tempted to change, so it is written here and
    /// pinned by a test rather than left in the documentation.
    ///
    /// * A record's **reasoning** count is a *subset* of its output, so adding
    ///   it would count the same token twice.
    /// * A Claude Code `usage.iterations[]` list is one element per attempt at
    ///   the request, and on the rare record with more than one the outer
    ///   counters are the **last** element's — never the sum of them. Adding
    ///   the list would therefore count the surviving attempt twice *and* bill
    ///   the abandoned ones, which is not a number the file states anywhere.
    ///
    /// What that costs is real and is the point: on a retried request this
    /// total is what the **last attempt** spent, not what the attempts spent
    /// between them. The file records no total across attempts, so a reader who
    /// needs one adds the elements themselves — the row says the request was
    /// retried (`iterations` on the open level), and `r` has the list.
    pub fn total(&self) -> u64 {
        [self.input, self.output, self.cache_read, self.cache_new]
            .into_iter()
            .flatten()
            .fold(0u64, |a, b| a.saturating_add(b))
    }

    /// Did this record record anything at all? A record that recorded nothing
    /// gets no number columns — it shows what kind of record it is and stops.
    pub fn any(&self) -> bool {
        self.input.is_some()
            || self.output.is_some()
            || self.cache_read.is_some()
            || self.cache_new.is_some()
    }
}

/// One column of the numeric block: which counter, under which label.
///
/// A dialect names the set its format actually has, once, and a counter that is
/// not in the set never reaches a row — that is the "absent column" of the
/// three-way distinction above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    In,
    Out,
    Read,
    New,
}

impl Field {
    /// The four a format that records cache writes shows.
    pub const ALL: [Field; 4] = [Field::In, Field::Out, Field::Read, Field::New];

    fn label(self) -> &'static str {
        match self {
            Field::In => "in",
            Field::Out => "out",
            Field::Read => "read",
            Field::New => "new",
        }
    }

    fn of(self, t: &Tokens) -> Option<u64> {
        match self {
            Field::In => t.input,
            Field::Out => t.output,
            Field::Read => t.cache_read,
            Field::New => t.cache_new,
        }
    }
}

/// The row a usage-bearing record shows: the numbers, then what it did.
///
/// Every field is [`FIELD`] columns whatever it holds, so the block is
/// `fields.len() * FIELD + (fields.len() - 1) * 2` columns for every record in
/// the file and the column reads straight down.
pub fn row_text(t: &Tokens, fields: &[Field], action: &str) -> String {
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push_str(GAP);
        }
        out.push_str(&cell(*f, t));
    }
    if !action.is_empty() {
        out.push_str(TURN);
        out.push_str(action);
    }
    out
}

/// One field: its label left-justified, its number right-aligned into what is
/// left. ASCII throughout — the labels are this module's own and
/// [`crate::lens::tokens`] emits digits and one suffix letter — so columns and
/// bytes agree here and the padding is exact.
fn cell(f: Field, t: &Tokens) -> String {
    let number = match f.of(t) {
        Some(n) => tokens(n),
        None => ABSENT.to_string(),
    };
    let label = f.label();
    let mut s = String::with_capacity(FIELD);
    s.push_str(label);
    for _ in label.len()..LABEL {
        s.push(' ');
    }
    for _ in number.len()..NUMBER {
        s.push(' ');
    }
    s.push_str(&number);
    s
}

/// Adjacent entries that read the same collapsed to `Read ×3`.
///
/// A turn that read three files made one kind of move, and spelling it three
/// times pushes what happened next off the row. The same rule `atif` applies to
/// its own row, for the same reason.
pub fn collapse(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for it in items {
        match out.last_mut() {
            Some((last, n)) if *last == it => *n += 1,
            _ => out.push((it, 1)),
        }
    }
    out.into_iter()
        .map(|(text, n)| match n {
            1 => text,
            n => format!("{text} \u{d7}{n}"),
        })
        .collect()
}

// -- the open level -------------------------------------------------------------
//
// Where the row's floored four columns become exact, and where everything the
// row had no column for lands. Nothing a record holds is unreachable, which is
// the seam's rule — and this level is built from `Part::Text` alone, because
// this lens is not re-telling the conversation.

/// One `name  value` line of an open level, aligned so a column of numbers
/// still reads as a column when it is exact.
pub fn line(name: &str, value: impl std::fmt::Display) -> String {
    format!("{name:<28}{value}")
}

/// A `name  value` line for an integer field, when the record has one.
///
/// The **exact** integer, not [`tokens`]: the row above is floored to four
/// columns and this is the rung a reader descends to when they need the real
/// number — `1999` on the row is `1.9k`, and here it is `1999`.
pub fn exact(v: &Value, key: &str) -> Option<String> {
    Some(line(key, v.get(key)?.as_number()?.as_i64()?))
}

/// A `name  value` line for a string field, when the record has one.
pub fn named(v: &Value, key: &str) -> Option<String> {
    Some(line(key, v.get(key)?.as_str()?))
}

/// The lines gathered under `label` as one part, or nothing when there were
/// none — an empty part is a row that says a name and shows nothing under it.
///
/// The body has an **empty path**: it is a summary this module composed, not a
/// string node of the record, so there is nothing to walk back to. Every byte
/// of it is short, and the record's own tree is one `r` away regardless.
pub fn part(label: &'static str, lines: Vec<String>) -> Option<Part> {
    match lines.is_empty() {
        true => None,
        false => Some(Part::Text { label, body: Body::new(&lines.join("\n"), Vec::new()) }),
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
