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
//! Exactly one path, `message.usage`, and nothing else is read for a number. In
//! the sessions this was written against it is present on every `assistant`
//! record and on no record of any other type, so a record without it is not a
//! record whose numbers are hidden somewhere else — it is a record that spent
//! nothing this file recorded, and it shows its `type` and stops.
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
use super::{record_clock, record_type, Class, Lens, Summary, Who};
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
/// not a token count and is refused rather than clamped — the record then shows
/// `-` in that cell, which is true.
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
