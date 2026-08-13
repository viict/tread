//! The `usage-atif` lens: an ATIF trajectory read as what it *spent*.
//!
//! The document twin of `usage`, over the same [`usage`] vocabulary. Same
//! question, same numeric block, a different file shape and a different set of
//! counters — and the difference in the counters is visible on the row, which is
//! the point.
//!
//! # Two entries over one module, and why
//!
//! [`Lens::records_at`] returns **one** answer, and `src/open/lens.rs` refuses a
//! record-per-line lens on a document and a document lens on a `.jsonl`. So one
//! name cannot legally cover both shapes: `usage` declares `Lines` and this
//! declares `Member("steps")`. Everything that is not that declaration is shared.
//!
//! # Where the numbers are
//!
//! `steps[].metrics`, not `message.usage`:
//!
//! | key | column |
//! | --- | --- |
//! | `prompt_tokens` | `in` |
//! | `completion_tokens` | `out` |
//! | `cached_tokens` | `read` |
//!
//! And **there is no fourth**. ATIF-v1.7 records no cache-*creation* counter of
//! any kind, so this dialect's field set is three and a `usage-atif` row never
//! shows a `new` column at all — not a `-`, which would say this record failed
//! to record something the format has, and above all not a `0`, which would say
//! the agent wrote nothing to cache when the truth is that the format does not
//! record cache writes. A format-level absence removes the column for the whole
//! file; a record-level absence inside a format that has the field prints `-`.
//!
//! `metrics.extra.reasoning_tokens` is a subset of `completion_tokens` and is
//! never added to the total; `llm_call_count` is a count of calls to a model and
//! not a count of tokens, so it is not a column either. Neither is lost — both
//! are on the open level.
//!
//! There is no `isSidechain` here and nothing that means it, so no row is
//! marked as a subagent. Inventing a mark would be a guess.
#![deny(unsafe_code)]

use super::atif::STEPS;
use super::usage::{self, Field, Tokens};
use super::usage_agent::count;
use super::{record_clock, Class, Lens, Part, RecordsAt, Summary, Who};
use crate::json::Value;

pub const NAME: &str = "usage-atif";
pub const ABOUT: &str = "ATIF agent trajectories: what each step spent, not what it said";

/// The three columns this format has. There is no cache-creation counter in
/// ATIF-v1.7, so there is no fourth column — see the module docs.
const FIELDS: [Field; 3] = [Field::In, Field::Out, Field::Read];

#[derive(Default)]
pub struct UsageAtif;

impl Lens for UsageAtif {
    fn name(&self) -> &'static str {
        NAME
    }

    fn about(&self) -> &'static str {
        ABOUT
    }

    fn records_at(&self) -> RecordsAt {
        RecordsAt::Member(STEPS)
    }

    fn read(&mut self, v: &Value) -> Option<Summary> {
        match v.get("step_id") {
            Some(_) => step(v),
            None => session(v),
        }
    }

    /// The exact numbers, and the two fields the row has no column for.
    ///
    /// Nothing is invented for the four counters this format does not have:
    /// there is no cache-creation part here under any circumstance, because
    /// there is no cache-creation number to show.
    fn detail(&self, v: &Value) -> Vec<Part> {
        let mut out: Vec<Part> = Vec::new();
        if let Some(m) = v.get("metrics") {
            out.extend(usage::part("tokens", metric_lines(m)));
        }
        out.extend(usage::part("model", model_lines(v)));
        out
    }
}

/// Every counter `metrics` wrote, exact, plus the two that get no column:
/// `reasoning_tokens`, which is a subset of the completion count, and
/// `llm_call_count`, which counts calls to a model rather than tokens.
fn metric_lines(m: &Value) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for key in ["prompt_tokens", "completion_tokens", "cached_tokens", "total_tokens"] {
        lines.extend(usage::exact(m, key));
    }
    if let Some(extra) = m.get("extra") {
        lines.extend(usage::exact(extra, "reasoning_tokens"));
    }
    lines
}

/// Which model the numbers were spent on, and how many times it was called.
fn model_lines(v: &Value) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    lines.extend(usage::named(v, "model_name"));
    lines.extend(usage::exact(v, "llm_call_count"));
    lines
}

/// One step of the trajectory.
fn step(v: &Value) -> Option<Summary> {
    // There is no `type` field on an ATIF step; `source` is the discriminator,
    // and it is also the turn boundary.
    let source = v.get("source").and_then(|s| s.as_str()).unwrap_or("agent");
    let (who, actor, class) = match source {
        "user" => (Who::User, "user", Class::Message),
        _ => (Who::Assistant, "agent", Class::Step),
    };
    let calls = calls_of(v);
    let metrics = read_metrics(v);
    let what = match &metrics {
        Some(t) => usage::row_text(t, &FIELDS, &action(&calls, source)),
        // No metrics: the kind, and nothing more. No dashes and no zeroes,
        // because the step recorded nothing at all.
        None => source.to_string(),
    };
    let tokens = metrics.map(|t| t.total()).unwrap_or(0);
    Some(
        Summary {
            class,
            who,
            actor: actor.to_string(),
            time: None,
            what,
            calls: calls.len(),
            tokens,
            // What was said is `--lens atif`'s question.
            body: None,
        }
        .at(record_clock(v)),
    )
}

/// Record 0: the document's own keys, which the source keeps so this row can
/// exist. It names the agent and never a total — ATIF records no document-level
/// counters, and a total invented here would be one the file does not contain.
///
/// A [`Class::Message`], so it never folds into the run that follows it.
fn session(v: &Value) -> Option<Summary> {
    let schema = text(v, "schema_version");
    let id = text(v, "session_id");
    if schema.is_none() && id.is_none() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(a) = v.get("agent") {
        parts.push(text(a, "name").unwrap_or_else(|| "agent".to_string()));
        parts.extend(text(a, "model_name"));
    }
    parts.extend(schema);
    parts.extend(id);
    Some(Summary {
        class: Class::Message,
        who: Who::System,
        actor: "session".to_string(),
        time: None,
        what: parts.join(" \u{b7} "),
        calls: 0,
        tokens: 0,
        body: None,
    })
}

/// `steps[].metrics`, read into the three counters this format has.
fn read_metrics(v: &Value) -> Option<Tokens> {
    let m = v.get("metrics")?;
    let t = Tokens {
        input: count(m, "prompt_tokens"),
        output: count(m, "completion_tokens"),
        cache_read: count(m, "cached_tokens"),
        // ATIF-v1.7 has no cache-creation counter, and `FIELDS` gives it no
        // column, so this is never read. It is spelled out rather than left
        // implicit so the next reader does not go looking for the field.
        cache_new: None,
    };
    match t.any() {
        true => Some(t),
        false => None,
    }
}

/// What the step did, after the numbers: its tool calls, or its own kind.
fn action(calls: &[String], source: &str) -> String {
    match calls.is_empty() {
        true => source.to_string(),
        false => usage::collapse(calls.to_vec()).join(" \u{b7} "),
    }
}

/// The tool calls of one step, as the row names them.
fn calls_of(v: &Value) -> Vec<String> {
    let Some(items) = v.get("tool_calls").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    items.iter().map(call_text).collect()
}

/// `bash(cargo test)`, through the same argument budget every dialect uses.
fn call_text(call: &Value) -> String {
    let name = text(call, "function_name").unwrap_or_else(|| "tool".to_string());
    match super::atif::arg_of(call) {
        Some(arg) => format!("{name}({arg})"),
        None => name,
    }
}

fn text(v: &Value, key: &str) -> Option<String> {
    Some(v.get(key)?.as_str()?.to_string())
}

#[cfg(test)]
#[path = "usage_atif_tests.rs"]
mod tests;
