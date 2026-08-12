//! What a record's parts **are**, said without saying anything about rows.
//!
//! A summary is a headline and a [`Body`] is what was said. Between the two and
//! the raw JSON tree there is a level that reads a record the way a person would
//! describe it — the whole message, then its tool calls listed *as tool calls*
//! (SPEC.md §Lenses, the open level). This module is the vocabulary a dialect
//! uses to say what is in there, and it is deliberately row-free: no widths, no
//! indents, no fold markers, no order on the screen. `src/source/record/parts.rs`
//! turns a `Vec<Part>` into rows, and it is the only thing that does.
//!
//! # What a part may hold
//!
//! The same ceiling a [`Body`] has, for the same reason. [`Lens::detail`] is
//! called **on demand, for the record the reader opened** — never once per
//! record and never stored on a [`Summary`] — so parts do not cost anything per
//! document. But one *record* can carry five calls whose outputs are fifteen
//! kilobytes apiece, and a level that read them into memory to count its rows
//! would make opening one step cost what the step cost. So every stretch of
//! text in a `Part` is a [`Body`]: a bounded head, the true byte and line
//! counts, and the path back into the record.
//!
//! Two of those paths are real and one is not, and the difference is stated
//! rather than hidden:
//!
//! * a **result** sits at a path of static keys and one index
//!   (`observation.results[3].content`), so opening it reads the whole of it
//!   back out of the record being painted;
//! * an **argument** sits under a key whose name is the argument's own
//!   (`arguments.command`), and [`Step::Key`] is `&'static str` — a path there
//!   would have to allocate the key. An argument is therefore a `Body` with an
//!   **empty path**, which [`Body::text_in`] resolves to the head. That is not a
//!   silent clip: the head carries the true `bytes` and `lines`, so the row
//!   under it says `⋯ +N more` exactly as a clipped message does, and every byte
//!   is still one `zt` away in the record's own tree.
//!
//! A lens never hides anything (SPEC.md §Lenses). A part may be clipped; it may
//! not be quiet about it.
#![deny(unsafe_code)]

use super::Body;
use crate::json::Value;

/// The name an argument list gets when it is not an object at all.
///
/// A dialect reads a call's `arguments` as the object the schema says it is,
/// but a real file writes an array, a bare string, or a JSON-encoded string
/// that streaming cut in half. Dropping those left the open level saying the
/// call had **no** arguments, which is the one thing a clip may never do
/// (SPEC.md §Lenses) — so they become one argument under this name, holding
/// exactly what the file said.
pub const RAW_ARGS: &str = "arguments";

/// A call's `arguments` as the list the open level lists, whatever shape they
/// arrived in.
///
/// An object is its members, in the record's own order. Anything else is one
/// entry named [`RAW_ARGS`]: a string as itself, and any other value as the
/// JSON it is — which is what the record's tree would show, and what `zt` is
/// one press away from.
///
/// Every value is a [`Body`] with an **empty path**: an argument lives under a
/// key that is its own name and a [`Step::Key`](super::Step::Key) is
/// `&'static str`, so the value is a bounded head that states its true size
/// rather than a path back into the record.
pub fn args_of(arguments: &Value) -> Vec<(String, Body)> {
    // Absent, `null` and `[]` all mean "no arguments", and a row saying
    // `arguments  null` would be noise rather than a fact.
    if matches!(arguments, Value::Null) || arguments.as_array().is_some_and(|a| a.is_empty()) {
        return Vec::new();
    }
    let Some(members) = arguments.as_object() else {
        return vec![(RAW_ARGS.to_string(), Body::new(&as_text(arguments), Vec::new()))];
    };
    members
        .iter()
        .map(|m| (m.key.clone(), Body::new(&as_text(&m.value), Vec::new())))
        .collect()
}

/// One value as the text an argument row shows: a string as itself, anything
/// else as its own JSON.
pub fn as_text(v: &Value) -> String {
    match v.as_str() {
        Some(s) => s.to_string(),
        None => v.to_json(),
    }
}

/// One thing a record turns out to contain, once you read it as what it is.
///
/// Deliberately two variants and not more: text someone (or something) wrote,
/// and a call to a tool. A dialect that finds a third kind of thing in a record
/// says so here, once, and every dialect gets the rows for free.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// A stretch of text the record holds under a name: the model's reasoning,
    /// a second text block a message row could only excerpt.
    ///
    /// `label` is `&'static str` because a dialect names it in its own source
    /// (`thinking`, `message`) — the same reason [`Step::Key`](super::Step::Key)
    /// is.
    Text {
        label: &'static str,
        body: Body,
    },
    /// A call to a tool: what was called, the one argument that says what it
    /// did, all of its arguments, and what came back.
    ///
    /// `result` is `None` where the answer is not in this record — the `agent`
    /// dialect's tool results arrive as a *later* record, and a `Body`'s path
    /// starts at the record it belongs to. The row then says what was called
    /// and does not pretend to know the answer.
    Call {
        /// The tool's own name: `bash`, `Read`.
        tool: String,
        /// The one argument a headline shows: the command, the path, the
        /// pattern. Empty when the tool's arguments name none.
        arg: String,
        /// Every argument, in the record's own order, each value as text.
        args: Vec<(String, Body)>,
        /// What the call returned.
        result: Option<Body>,
    },
}

impl Part {
    /// Is this a call with something under it — arguments, or an answer?
    ///
    /// A [`Part::Text`] is already showing everything it has, and so is a call
    /// made with no arguments that returned nothing: a rung that repaints the
    /// same screen is not a rung, so that row carries no fold marker and its
    /// `Enter` belongs to the record rather than to the call.
    pub fn opens(&self) -> bool {
        match self {
            Part::Text { .. } => false,
            Part::Call { args, result, .. } => !args.is_empty() || result.is_some(),
        }
    }
}
