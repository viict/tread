//! The `usage` lens: a Claude Code session log read as what it *spent*.
//!
//! Same file as `--lens agent`, a different question. `agent` asks what was
//! said; this asks what each record cost, and answers in a fixed-width numeric
//! block a reader can scan straight down:
//!
//! ```text
//! assistant  21:31  in  1.2k  out  380  read  18k  new 2.1k  ·  Bash(cargo test)
//! ```
//!
//! Named `usage` and not `cost`: no price table is compiled in and no money is
//! shown. A lens must not promise a currency it cannot compute.
//!
//! # Where the numbers are
//!
//! Exactly one path, `message.usage`, and nothing else is read for a number.
//! Measured over whole session logs it is on every `assistant` record and on no
//! record of any other type — so a row that shows no numbers is never an
//! `assistant` whose counters this lens failed to find.
//!
//! It is **not** the only usage object a session log contains. A `user` record
//! that carries the result of a subagent call has one at
//! `toolUseResult.usage`, with the same key names. This lens does not read it,
//! and such a record shows `user` and no number columns at all. That is the
//! right answer here: every number in this column is one request's spend, and
//! the counters on a subagent result are the *total* of a whole run of them —
//! putting a sum in a column of per-request numbers would make the column mean
//! two things and make it un-addable. Nothing is lost: those records are a
//! handful per corpus, the object is in the record's own tree one `r` away, and
//! the subagent's own session log has the same spend a request at a time.
//!
//! | key | column |
//! | --- | --- |
//! | `input_tokens` | `in` |
//! | `output_tokens` | `out` |
//! | `cache_read_input_tokens` | `read` |
//! | `cache_creation_input_tokens` | `new` |
//!
//! `output_tokens_details.thinking_tokens` is a *subset* of `output_tokens` and
//! gets no column. **`usage.iterations[]` is never summed**: it is a list whose
//! elements repeat the outer counters per attempt, so adding both counts every
//! token twice ([`usage::Tokens::total`] says so where the adding happens).
//!
//! # The one decision that shapes the document
//!
//! A **human** turn — a `user` record that is not carrying a tool result — is a
//! [`Class::Message`] and breaks the run. Everything else is a [`Class::Step`].
//! So the run between two human turns is exactly one turn's mechanics, and the
//! group row over it totals what that turn cost.
//!
//! Drawing that line anywhere else would ruin the lens. A `user` record whose
//! content blocks are `tool_result` is mechanics — `agent.rs` already draws that
//! same line — and if it were conversation instead, tool results would shred
//! every run into pairs and no group row would total anything.
//!
//! # What this lens does not show
//!
//! Any message text at all. [`Summary::body`] is `None` on every row, because
//! "what was said" is the question `--lens agent` answers. It also means this
//! dialect allocates nothing per record but its own one-line `what`, so a log
//! whose longest line is most of a megabyte costs it a parse and no more.
#![deny(unsafe_code)]

use super::usage::{self, Field, Tokens};
use super::{record_clock, record_type, Class, Lens, Part, Summary, Who};
use crate::json::Value;

pub const NAME: &str = "usage";
pub const ABOUT: &str = "Claude Code session logs: what each turn spent, not what it said";

/// The subagent mark, with **no space after it**.
///
/// `agent.rs` writes `↳ assistant`, which is eleven columns and pushes every
/// column on that row one to the right. More than half of the records in a real
/// session carry `isSidechain`, and on a lens whose whole product is a column of
/// numbers a shift on half the rows is fatal — so here it is `↳assistant`,
/// exactly the ten columns the actor field is wide. `Who` still carries the
/// colour, so the row still paints as an assistant.
const SIDE: char = '\u{21b3}';

#[derive(Default)]
pub struct Usage;

impl Lens for Usage {
    fn name(&self) -> &'static str {
        NAME
    }

    fn about(&self) -> &'static str {
        ABOUT
    }

    fn read(&mut self, v: &Value) -> Option<Summary> {
        // No string `type` is not this dialect's record: it falls to the
        // generic tree with nothing hidden.
        let kind = record_type(v)?;
        let time = record_clock(v);
        let (who, actor) = speaker(v, kind);
        let calls = call_blocks(v).count();
        let sum = match read_usage(v) {
            Some(t) => Summary {
                class: class_of(v, kind),
                who,
                actor,
                time: None,
                what: usage::row_text(&t, &Field::ALL, &action(v, kind)),
                calls,
                tokens: t.total(),
                body: None,
            },
            // Nothing was spent here that the file recorded, so there is no
            // number column at all — not a row of zeroes, and not a row of
            // dashes. A `type` this dialect has never seen prints its own name
            // rather than being swallowed.
            None => Summary {
                class: class_of(v, kind),
                who,
                actor,
                time: None,
                what: kind.to_string(),
                calls,
                tokens: 0,
                body: None,
            },
        };
        Some(sum.at(time))
    }

    /// The exact numbers, and everything the row had no column for.
    ///
    /// Four columns of floored numbers are what a row can honestly carry; this
    /// is where they become the integers the file wrote, and where the fields a
    /// column would have been 99.96% noise for finally appear — a `service_tier`
    /// or a `speed` that is *not* `standard` is exactly the anomaly a reader
    /// opened the row to find, and one that is `standard` says nothing and is
    /// not shown.
    ///
    /// `Part::Text` only: this lens is not re-telling the conversation, so a
    /// call gets no part of its own. `&self` and nothing stored — a `Vec<Part>`
    /// is per keystroke.
    fn detail(&self, v: &Value) -> Vec<Part> {
        let mut out: Vec<Part> = Vec::new();
        if let Some(u) = v.get("message").and_then(|m| m.get("usage")) {
            out.extend(usage::part("tokens", token_lines(u)));
            out.extend(usage::part("request", request_lines(u)));
        }
        out.extend(usage::part("model", model_lines(v)));
        out.extend(usage::part("envelope", envelope_lines(v)));
        out
    }
}

/// Every counter the record wrote, exact — including the two the row has no
/// column for, and the cache-creation breakdown when the record splits it.
fn token_lines(u: &Value) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for key in [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ] {
        lines.extend(usage::exact(u, key));
    }
    if let Some(c) = u.get("cache_creation") {
        for key in ["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"] {
            lines.extend(usage::exact(c, key));
        }
    }
    // A subset of `output_tokens`, which is why it gets no column and is never
    // added to the total.
    if let Some(d) = u.get("output_tokens_details") {
        lines.extend(usage::exact(d, "thinking_tokens"));
    }
    lines
}

/// How the request itself went, when it went in a way worth saying.
fn request_lines(u: &Value) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for key in ["service_tier", "speed"] {
        if let Some(text) = u.get(key).and_then(|t| t.as_str()) {
            // Nearly every record says `standard`, so saying it again is noise;
            // anything else is the reason a reader is here.
            if text != "standard" {
                lines.push(usage::line(key, text));
            }
        }
    }
    // A list whose elements repeat the outer counters per attempt. Its *length*
    // is the fact — the request was retried — and its contents are never summed
    // (`usage::Tokens::total` says why).
    if let Some(n) = u.get("iterations").and_then(|i| i.as_array()).map(|a| a.len()) {
        if n > 1 {
            lines.push(usage::line("iterations", n));
        }
    }
    lines
}

/// Which model the numbers were spent on. Without it a count is a number with
/// no price attached to it.
fn model_lines(v: &Value) -> Vec<String> {
    let Some(m) = v.get("message") else {
        return Vec::new();
    };
    ["model", "id", "stop_reason"]
        .into_iter()
        .filter_map(|key| usage::named(m, key))
        .collect()
}

/// What wrote the record. `version` above all: it is the schema-drift signal, so
/// a row whose numbers look wrong can be checked against the build that wrote it.
fn envelope_lines(v: &Value) -> Vec<String> {
    ["requestId", "sessionId", "gitBranch", "version"]
        .into_iter()
        .filter_map(|key| usage::named(v, key))
        .collect()
}

/// `message.usage`, read into the four counters. `None` when the record carries
/// no usage object at all.
fn read_usage(v: &Value) -> Option<Tokens> {
    let usage = v.get("message")?.get("usage")?;
    let t = Tokens {
        input: count(usage, "input_tokens"),
        output: count(usage, "output_tokens"),
        cache_read: count(usage, "cache_read_input_tokens"),
        cache_new: count(usage, "cache_creation_input_tokens"),
    };
    // A `usage` that is there but says none of the four is still a record with
    // no numbers, and it reads better as its own kind than as four dashes.
    match t.any() {
        true => Some(t),
        false => None,
    }
}

/// One counter, when it is there and is a non-negative integer.
///
/// Exact, through the number's own literal text: these are added up into a
/// session total, and a count that went through an `f64` would stop being the
/// number the file wrote somewhere past 2⁵³. A negative or fractional value is
/// not a token count and is refused rather than clamped.
///
/// **Refusing spells it `-`, the same cell an unwritten field gets**, so a
/// malformed counter is not told apart from one the record never wrote. That is
/// deliberate and it is the one place the three-way distinction in `usage.rs`
/// is blurred: no record of any session measured has ever carried a counter
/// that is not a non-negative integer, and a fourth spelling in a four-column
/// block would be vocabulary a reader has to learn for something they will not
/// see. The value is not hidden either way — `r` shows the record's own tree,
/// with whatever it really wrote in it.
pub(super) fn count(usage: &Value, key: &str) -> Option<u64> {
    u64::try_from(usage.get(key)?.as_number()?.as_i64()?).ok()
}

/// Conversation or mechanics. Only a human turn is conversation; see the module
/// docs for why the line is drawn exactly there.
fn class_of(v: &Value, kind: &str) -> Class {
    match kind == "user" && !carries_results(v) {
        true => Class::Message,
        false => Class::Step,
    }
}

/// Is this `user` record the tool answering rather than a person typing?
fn carries_results(v: &Value) -> bool {
    blocks(v).is_some_and(|bs| bs.iter().any(|b| block_type(b) == "tool_result"))
}

fn blocks(v: &Value) -> Option<&[Value]> {
    v.get("message")?.get("content")?.as_array()
}

fn block_type(b: &Value) -> &str {
    b.get("type").and_then(|t| t.as_str()).unwrap_or("")
}

fn call_blocks(v: &Value) -> impl Iterator<Item = &Value> {
    blocks(v)
        .unwrap_or(&[])
        .iter()
        .filter(|b| block_type(b) == "tool_use")
}

/// What the record did, after the numbers: its tool calls, or its own kind when
/// it made none. The numbers come first because they are the column being
/// scanned; the action is what tells one row of them from another.
fn action(v: &Value, kind: &str) -> String {
    let calls: Vec<String> = call_blocks(v).map(call_text).collect();
    match calls.is_empty() {
        true => kind.to_string(),
        false => usage::collapse(calls).join(" \u{b7} "),
    }
}

/// `Bash(cargo test)`, through the same argument budget every dialect uses.
fn call_text(block: &Value) -> String {
    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
    match block.get("input").and_then(super::agent::tool_arg) {
        Some(arg) => format!("{name}({arg})"),
        None => name.to_string(),
    }
}

/// Who the row is attributed to, with a subagent's own conversation marked.
///
/// Three names and no more — `user`, `assistant`, `system` — because the actor
/// field is ten columns and a record whose `type` is `file-history-snapshot`
/// would push the numbers of every row it sits between. The `type` is not lost:
/// on a record with no usage it *is* the row.
fn speaker(v: &Value, kind: &str) -> (Who, String) {
    let role = v
        .get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .unwrap_or(kind);
    let (who, name) = match role {
        "user" => (Who::User, "user"),
        "assistant" => (Who::Assistant, "assistant"),
        _ => (Who::System, "system"),
    };
    let side = v.get("isSidechain").and_then(|s| s.as_bool()).unwrap_or(false);
    let name = match side {
        true => format!("{SIDE}{name}"),
        false => name.to_string(),
    };
    (who, name)
}

#[cfg(test)]
#[path = "usage_agent_tests.rs"]
mod tests;
