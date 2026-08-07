//! The `--lens` seam (SPEC.md §Lenses).
//!
//! # What a lens is, and what it is not
//!
//! A lens is a **transform over records**, not a format. It never reads a file,
//! never owns a [`crate::source::Source`], and never decides where a row goes
//! on the screen: given one parsed record it says *what that record is* — who
//! spoke, when, what happened, and whether the record is conversation or
//! mechanics. The record source ([`crate::source::jsonl`]) does the rest: it
//! turns runs of mechanics into one foldable group row and leaves the
//! conversation alone.
//!
//! That split is the whole point. A second dialect — opencode, OpenAI Codex,
//! ATIF — is a new module here and one entry in [`LENSES`]; nothing about
//! folding, row arithmetic, search or yanking is written twice.
//!
//! # The contract
//!
//! * [`Lens::read`] returns `None` for a record it does not recognise, and that
//!   record renders as the generic JSON tree. **A lens adds interpretation; it
//!   must never lose data** (SPEC.md §Lenses: "an unrecognised record falls
//!   back to the generic rendering rather than being hidden"). Even a record it
//!   *does* recognise keeps every byte: the summary is a headline, and opening
//!   the row still shows the whole record as a tree.
//! * Records are read **in file order, once each**, which is what lets a lens
//!   carry a little state across them — the agent dialect matches a tool result
//!   back to the call that made it. `read` therefore takes `&mut self`. A lens
//!   whose state is wrong (a record read out of order, a truncated log) must
//!   still return a usable summary rather than nothing.
//! * Nothing here allocates per *document*: a summary is built from one record
//!   and thrown away with it.
#![deny(unsafe_code)]

pub mod agent;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use crate::json::Value;
use crate::term::Style;
use crate::theme;

/// Whether a record is conversation or mechanics.
///
/// This is the one decision that changes the *shape* of the document: a run of
/// consecutive [`Class::Step`] records collapses into a single summary row that
/// opens, while a [`Class::Message`] is always on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Someone said something. Never folded away.
    Message,
    /// A mechanical step: a tool call, its result, a thought, bookkeeping.
    Step,
}

/// Who a row is attributed to. Colour only — the text is [`Summary::actor`],
/// so a dialect can say `subagent` and still be painted as an assistant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    User,
    Assistant,
    Tool,
    System,
}

impl Who {
    pub fn style(self) -> Style {
        match self {
            Who::User => theme::lens_user(),
            Who::Assistant => theme::lens_assistant(),
            Who::Tool => theme::lens_tool(),
            Who::System => theme::lens_system(),
        }
    }
}

/// One record, as a lens reads it.
///
/// Everything on it is display text the source paints; no styling and no
/// layout decisions, so the same summary reads the same way in the pager, in a
/// dump and in a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub class: Class,
    pub who: Who,
    /// `assistant`, `user`, `tool`, `↳ assistant` for a subagent — whatever the
    /// dialect calls the speaker.
    pub actor: String,
    /// Clock time, already formatted by the dialect (the agent lens uses
    /// `HH:MM` of the UTC timestamp the file records). `None` when the record
    /// carries no time.
    pub time: Option<String>,
    /// What happened, in one line: an excerpt of what was said, or
    /// `Bash(git status)`.
    pub what: String,
    /// Tool calls this record makes, for the group row's `· 4 tool calls`.
    pub calls: usize,
}

impl Summary {
    /// A step with no clock and no tool calls — the shape most bookkeeping
    /// records take.
    pub fn step(who: Who, actor: impl Into<String>, what: impl Into<String>) -> Summary {
        Summary {
            class: Class::Step,
            who,
            actor: actor.into(),
            time: None,
            what: what.into(),
            calls: 0,
        }
    }

    /// The same summary with a clock time attached.
    pub fn at(mut self, time: Option<String>) -> Summary {
        self.time = time;
        self
    }
}

/// One dialect of record file.
pub trait Lens {
    /// The name `--lens` takes.
    fn name(&self) -> &'static str;

    /// One line for `--lens list`.
    fn about(&self) -> &'static str;

    /// Read one record. `None` means "not mine" — the record renders as the
    /// generic tree, whole.
    ///
    /// Called once per record, in file order.
    fn read(&mut self, value: &Value) -> Option<Summary>;
}

/// How a lens is built. A function pointer rather than a value because a lens
/// carries state across records, so every open needs its own.
type Make = fn() -> Box<dyn Lens>;

/// Every lens there is. **Adding a dialect is one module and one line here**
/// (docs/lenses.md says what the module has to provide).
const LENSES: &[(&str, Make)] = &[(agent::NAME, || Box::new(agent::Agent::default()))];

/// The lens called `name`, or `None`.
pub fn find(name: &str) -> Option<Box<dyn Lens>> {
    LENSES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, make)| make())
}

/// Is `name` a lens? The CLI asks before the file is opened.
pub fn exists(name: &str) -> bool {
    LENSES.iter().any(|(n, _)| *n == name)
}

/// The names, for an error message.
pub fn names() -> Vec<&'static str> {
    LENSES.iter().map(|(n, _)| *n).collect()
}

/// `--lens list`: the available lenses, one per line. The description comes
/// from the lens itself, so a dialect says what it is in one place.
pub fn list_text() -> String {
    let mut s = String::from("lenses (--lens <name>):\n");
    for (name, make) in LENSES {
        s.push_str(&format!("    {name:<10}{}\n", make().about()));
    }
    s.push_str("\nWithout --lens, records render as the generic JSON tree.\n");
    s
}

// -- helpers every dialect wants ------------------------------------------------

/// Columns a one-line excerpt of what someone said may occupy.
pub const EXCERPT: usize = 160;

/// Columns a tool's argument may occupy on a summary row.
pub const ARG: usize = 56;

/// The first line of `text`, whitespace collapsed, cut to `cols` columns.
///
/// A summary row stands for a record that may be forty kilobytes; this is what
/// makes it one line. The record itself is untouched — opening the row shows
/// all of it.
pub fn excerpt(text: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(ch);
    }
    if crate::render::str_width(&out) <= cols {
        return out;
    }
    let mut cut = crate::render::truncate_width(&out, cols.saturating_sub(1)).to_string();
    cut.push('\u{2026}');
    cut
}

/// `HH:MM` of an ISO-8601 timestamp such as `2026-08-05T21:28:58.659Z`.
///
/// No timezone database and no dependencies, so this is the *recorded* time,
/// which in every agent log seen so far is UTC. Showing `21:28` rather than the
/// 24-character original is the whole point: a reader scanning a log wants the
/// clock, and the date is the same for almost every row in a session.
/// `None` when the string is not shaped like a timestamp, which is how a
/// dialect avoids showing an arbitrary substring of some other field.
/// Works on bytes rather than on `&str` slices on purpose: the text comes
/// straight out of the log, so byte 16 is not known to be a character boundary
/// and `&ts[14..16]` would panic on `2026-08-05T21:€z`. Every byte this accepts
/// is an ASCII digit, so the answer is built from those digits directly.
pub fn clock(ts: &str) -> Option<String> {
    let b = ts.as_bytes();
    if b.len() < 16 || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    let (hh, mm) = (&b[11..13], &b[14..16]);
    if !hh.iter().chain(mm).all(u8::is_ascii_digit) {
        return None;
    }
    let mut out = String::with_capacity(5);
    out.extend(hh.iter().map(|&c| c as char));
    out.push(':');
    out.extend(mm.iter().map(|&c| c as char));
    Some(out)
}

/// The `timestamp` field of a record as a clock time.
pub fn record_clock(v: &Value) -> Option<String> {
    clock(v.get("timestamp")?.as_str()?)
}

/// A record's `type`, when it has one that is a string.
pub fn record_type(v: &Value) -> Option<&str> {
    v.get("type")?.as_str()
}
