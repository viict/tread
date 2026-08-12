//! The `atif` lens: agent trajectories in the ATIF interchange format.
//!
//! # The dialect, as found in a real ATIF-v1.7 trajectory
//!
//! One JSON *document*, not a record per line: `schema_version`, `session_id`
//! and `agent` describe the run, and the records are the elements of `steps`.
//! That is what [`Lens::records_at`] declares, and it is the only thing about
//! this dialect that is not a `read` of one record — the document half is
//! [`crate::source::jsonarray`], which knows nothing about ATIF.
//!
//! A step:
//!
//! | key | what it is |
//! | --- | --- |
//! | `step_id` | an integer, ascending. Present on every step; the recogniser |
//! | `source` | `user` or `agent` |
//! | `timestamp` | ISO-8601 with a numeric offset (`+00:00`), and **absent on the opening step** |
//! | `message` | what was said. Present on every step and *empty* on most of them |
//! | `reasoning_content` | the model's thinking, when it recorded any |
//! | `tool_calls[]` | `function_name`, `arguments` (an object), `tool_call_id` |
//! | `observation.results[]` | `source_call_id`, `content` — matched back to the call it answers |
//! | `model_name`, `metrics`, `llm_call_count` | bookkeeping, left to the opened tree |
//!
//! # The one decision that shapes the document
//!
//! **A step whose `message` says something is conversation**; everything else
//! is mechanics. So a step with a message is a [`Class::Message`] and stays on
//! screen with its tool calls collapsed to a count on its row, and a step with
//! an empty message is a [`Class::Step`] that folds into a run with its
//! neighbours. Nothing is hidden either way: every row opens into the step's
//! own tree, whole.
//!
//! Nothing here assumes any field is present. `null`, `[]` and absent all mean
//! the same thing, `arguments` is tolerated as an object *or* as the
//! JSON-encoded string the OpenAI-shaped wire format emits, and a step this
//! does not recognise returns `None` and renders as the generic tree.
#![deny(unsafe_code)]

use super::{excerpt, record_clock, Body, Class, Lens, RecordsAt, Step, Summary, Who, ARG, EXCERPT};
use crate::json::Value;

pub const NAME: &str = "atif";
pub const ABOUT: &str = "ATIF agent trajectories: the conversation, mechanics folded away";

/// The top-level key whose array holds the records.
pub const STEPS: &str = "steps";

/// The argument that says what a call *did*, by the name ATIF tools give it.
///
/// Ordered, not guessed: a `glob` call carries both `path` and `pattern`, and
/// the pattern is what a reader scans for. A tool whose arguments name none of
/// these shows its own name alone rather than an arbitrary field.
const ARG_KEYS: [&str; 5] = ["command", "filePath", "pattern", "query", "url"];

#[derive(Default)]
pub struct Atif;

impl Lens for Atif {
    fn name(&self) -> &'static str {
        NAME
    }

    fn about(&self) -> &'static str {
        ABOUT
    }

    /// The records are `steps` inside one document — everything else at the top
    /// level is record 0 and is never hidden (SPEC.md §Lenses).
    fn records_at(&self) -> RecordsAt {
        RecordsAt::Member(STEPS)
    }

    fn read(&mut self, v: &Value) -> Option<Summary> {
        match v.get("step_id") {
            Some(_) => step(v),
            None => session(v),
        }
    }
}

/// Record 0: the document's own keys, which the source keeps precisely so this
/// row can exist. `None` when they are not an ATIF envelope, which sends the
/// record to the generic tree with nothing lost.
fn session(v: &Value) -> Option<Summary> {
    let schema = text(v, "schema_version");
    let id = text(v, "session_id");
    if schema.is_none() && id.is_none() {
        return None;
    }
    let agent = v.get("agent");
    let mut parts: Vec<String> = Vec::new();
    parts.extend(schema);
    if let Some(a) = agent {
        let name = text(a, "name").unwrap_or_else(|| "agent".to_string());
        parts.push(match text(a, "version") {
            Some(ver) => format!("{name} {ver}"),
            None => name,
        });
        parts.extend(text(a, "model_name"));
    }
    parts.extend(id);
    Some(Summary {
        class: Class::Message,
        who: Who::System,
        actor: "session".to_string(),
        time: None,
        what: parts.join(" \u{b7} "),
        calls: 0,
        // The envelope is a headline over a tree, not something anyone said:
        // its keys are the row, and `Enter` opens them.
        body: None,
    })
}

/// One step of the trajectory.
fn step(v: &Value) -> Option<Summary> {
    let source = v.get("source").and_then(|s| s.as_str()).unwrap_or("agent");
    let time = record_clock(v);
    let said = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
    let calls = calls_of(v);
    let thinking = v
        .get("reasoning_content")
        .and_then(|r| r.as_str())
        .is_some_and(|r| !r.trim().is_empty());
    let sum = match said.trim().is_empty() {
        false => spoken(source, said, thinking, calls.len()),
        true => worked(source, v, thinking, calls),
    };
    Some(sum.at(time))
}

/// A step that says something: the message, and a count of what it did
/// alongside. The calls are deliberately *not* rows — a message is the thing a
/// reader came for, and its mechanics are one clause on the end of it.
///
/// The message itself goes under the row as the [`Body`], read straight back
/// out of `message` when the reader opens it.
fn spoken(source: &str, said: &str, thinking: bool, calls: usize) -> Summary {
    let (who, actor) = speaker(source);
    let mut what = excerpt(said, EXCERPT);
    if thinking {
        what.push_str(" \u{b7} thinking");
    }
    if calls > 0 {
        what.push_str(&format!(" \u{b7} {}", call_count(calls)));
    }
    let body = Body::new(said, vec![Step::Key("message")]);
    Summary { class: Class::Message, who, actor, time: None, what, calls, body: Some(body) }
}

/// A step that only did things: the thought and the calls it made, each with
/// what came back, and adjacent repeats collapsed.
fn worked(source: &str, v: &Value, thinking: bool, calls: Vec<Call>) -> Summary {
    let n = calls.len();
    let mut results = results_of(v);
    let mut parts: Vec<String> = Vec::new();
    if thinking {
        parts.push("thinking".to_string());
    }
    for call in &calls {
        let answer = match take_result(&mut results, call.id.as_deref()) {
            Some(text) => format!(" \u{2192} {text}"),
            None => String::new(),
        };
        parts.push(format!("{}{answer}", call.text()));
    }
    // Results the calls in this step did not claim: an orphan `source_call_id`,
    // or a step that carries answers and no calls at all. Counted rather than
    // dropped — the row is a headline, but it may not lie about what is there.
    if !results.is_empty() {
        parts.push(result_count(results.len()));
    }
    if parts.is_empty() {
        parts.push("(no content)".to_string());
    }
    let (who, actor) = match n {
        0 => speaker(source),
        _ => (Who::Tool, "tool".to_string()),
    };
    Summary {
        class: Class::Step,
        who,
        actor,
        time: None,
        what: collapse(parts).join(" \u{b7} "),
        calls: n,
        // Mechanics stay one line: a step has no body, which is also what keeps
        // a folded run exactly as tall as the steps inside it.
        body: None,
    }
}

/// Who a step is attributed to. An unknown `source` reads as the assistant
/// rather than as nothing: dropping a step to the generic tree in the middle of
/// a conversation would be worse than naming it approximately.
fn speaker(source: &str) -> (Who, String) {
    match source {
        "user" => (Who::User, "user".to_string()),
        _ => (Who::Assistant, "assistant".to_string()),
    }
}

/// One tool call, as much of it as a row can say.
struct Call {
    name: String,
    arg: Option<String>,
    id: Option<String>,
}

impl Call {
    /// `bash(cargo test)`, or the bare name for a tool whose arguments name
    /// nothing on [`ARG_KEYS`].
    fn text(&self) -> String {
        match &self.arg {
            Some(arg) => format!("{}({arg})", self.name),
            None => format!("{}()", self.name),
        }
    }
}

fn calls_of(v: &Value) -> Vec<Call> {
    let Some(items) = v.get("tool_calls").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|c| Call {
            name: text(c, "function_name").unwrap_or_else(|| "tool".to_string()),
            arg: arg_of(c),
            id: text(c, "tool_call_id"),
        })
        .collect()
}

/// The argument worth showing, from `arguments` — an object, or the
/// JSON-encoded string the wire format this schema descends from emits. A
/// value that is not a string (a `todos` list, a numeric `limit`) names
/// nothing, which is why the tool then shows its name alone.
fn arg_of(call: &Value) -> Option<String> {
    let raw = call.get("arguments")?;
    let decoded;
    let args = match raw.as_str() {
        Some(text) => {
            decoded = crate::json::parse(text.as_bytes()).ok()?;
            &decoded
        }
        None => raw,
    };
    for key in ARG_KEYS {
        if let Some(text) = args.get(key).and_then(|v| v.as_str()) {
            let cut = excerpt(text, ARG);
            if !cut.is_empty() {
                return Some(cut);
            }
        }
    }
    None
}

/// `(source_call_id, what came back)` for every result on this step.
fn results_of(v: &Value) -> Vec<(Option<String>, String)> {
    let Some(items) = v
        .get("observation")
        .and_then(|o| o.get("results"))
        .and_then(|r| r.as_array())
    else {
        return Vec::new();
    };
    items
        .iter()
        .map(|r| (text(r, "source_call_id"), size_of(r.get("content"))))
        .collect()
}

/// The result a call is owed, removed from the list so a second call with the
/// same id cannot claim it twice. Matching is *within the step*, which is where
/// every answer sat in the trajectory this was written against; an id that
/// matches nothing is left in the list and counted rather than attached.
fn take_result(results: &mut Vec<(Option<String>, String)>, id: Option<&str>) -> Option<String> {
    let id = id?;
    let at = results.iter().position(|(k, _)| k.as_deref() == Some(id))?;
    Some(results.remove(at).1)
}

/// How much a tool returned: `42 lines`, or the size when it is one long line.
fn size_of(content: Option<&Value>) -> String {
    let Some(text) = content.and_then(|c| c.as_str()) else {
        return "ok".to_string();
    };
    if text.is_empty() {
        return "empty".to_string();
    }
    match text.lines().count() {
        0 | 1 => crate::source::jsonrow::size(text.len() as u64),
        n => format!("{n} lines"),
    }
}

fn call_count(n: usize) -> String {
    match n {
        1 => "1 tool call".to_string(),
        n => format!("{n} tool calls"),
    }
}

fn result_count(n: usize) -> String {
    match n {
        1 => "1 result".to_string(),
        n => format!("{n} results"),
    }
}

/// Adjacent entries that read the same collapse to `bash(make) ×3`. A run of
/// identical calls is one fact, and spelling it three times pushes what
/// happened *next* off the row.
fn collapse(parts: Vec<String>) -> Vec<String> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for p in parts {
        match out.last_mut() {
            Some((last, n)) if *last == p => *n += 1,
            _ => out.push((p, 1)),
        }
    }
    out.into_iter()
        .map(|(text, n)| match n {
            1 => text,
            n => format!("{text} \u{d7}{n}"),
        })
        .collect()
}

/// A string field, when it is there and is a string.
fn text(v: &Value, key: &str) -> Option<String> {
    Some(v.get(key)?.as_str()?.to_string())
}

#[cfg(test)]
#[path = "atif_tests.rs"]
mod tests;
