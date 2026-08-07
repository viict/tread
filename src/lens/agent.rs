//! The `agent` lens: Claude Code session logs.
//!
//! # The dialect, as found in a real session file
//!
//! `~/.claude/projects/<slug>/<session-uuid>.jsonl`, one JSON object per line,
//! in wall-clock order. Every record carries a `type`; the ones that matter:
//!
//! | `type` | what it is |
//! | --- | --- |
//! | `user` | `message.role = "user"`, `message.content` a string (a typed prompt) or an array of blocks |
//! | `assistant` | `message.role = "assistant"`, `message.content` an array of blocks, `message.model` the model |
//! | `system` | a `subtype` (`turn_duration`, …) and whatever fields it needs |
//! | `attachment`, `mode`, `permission-mode`, `last-prompt`, `ai-title`, `bridge-session`, `queue-operation`, `file-history-snapshot`, `file-history-delta`, `summary` | bookkeeping the transcript writes alongside the conversation |
//!
//! A content block is `{"type": …}`:
//!
//! * `text` — `text`: what was said. The only thing that makes a record a
//!   [`Class::Message`].
//! * `thinking` — `thinking` plus a `signature`; often empty (redacted).
//! * `tool_use` — `id`, `name`, `input`. The call.
//! * `tool_result` — `tool_use_id`, `content` (a string, or a block array),
//!   `is_error`. The result, which arrives as a `user` record: **the model's
//!   turn and the tool's answer are both "messages" in the API sense**, which
//!   is exactly why the generic tree is useless here and why a `user` record
//!   whose only content is a `tool_result` is mechanics, not conversation.
//!
//! Shared record fields: `timestamp` (ISO-8601, UTC, `…Z`), `uuid`,
//! `parentUuid`, `sessionId`, `cwd`, `gitBranch`, `version`, and
//! **`isSidechain`** — `true` for a subagent's own conversation, which is how a
//! `Task` run is marked. `isMeta` marks a record injected by the harness rather
//! than by the user.
//!
//! Nothing here assumes any of that is present. Every field is optional, every
//! type is checked, and a record this does not recognise returns `None` and
//! renders as the generic tree with nothing hidden.
#![deny(unsafe_code)]

use super::{excerpt, record_clock, record_type, Class, Lens, Summary, Who, ARG, EXCERPT};
use crate::json::Value;

pub const NAME: &str = "agent";
pub const ABOUT: &str = "Claude Code session logs: the conversation, mechanics folded away";

/// Tool calls remembered so a result can name the tool that produced it.
///
/// A ring rather than a map: results follow their call within a record or two,
/// and a log with a million calls must not grow a million entries.
const RECENT_CALLS: usize = 64;

/// What one message's content blocks hold, in the order a row says them.
#[derive(Default)]
struct Blocks {
    /// Text blocks — the only thing that makes a record a message.
    spoken: Vec<String>,
    calls: Vec<String>,
    results: Vec<String>,
    thoughts: usize,
}

#[derive(Default)]
pub struct Agent {
    /// `(tool_use_id, tool name)`, newest last, [`RECENT_CALLS`] deep.
    calls: Vec<(String, String)>,
}

impl Lens for Agent {
    fn name(&self) -> &'static str {
        NAME
    }

    fn about(&self) -> &'static str {
        ABOUT
    }

    fn read(&mut self, v: &Value) -> Option<Summary> {
        let kind = record_type(v)?;
        let time = record_clock(v);
        let sum = match kind {
            "user" | "assistant" => self.message(v, kind)?,
            "system" => Summary::step(Who::System, "system", system_text(v)),
            other => Summary::step(Who::System, "system", bookkeeping(v, other)?),
        };
        Some(sum.at(time))
    }
}

impl Agent {
    /// A `user` or `assistant` record.
    fn message(&mut self, v: &Value, kind: &str) -> Option<Summary> {
        let msg = v.get("message")?;
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or(kind);
        let actor = actor_name(v, role);
        let who = match role {
            "user" => Who::User,
            _ => Who::Assistant,
        };
        // A prompt typed by a person: `content` is a bare string.
        if let Some(text) = msg.get("content").and_then(|c| c.as_str()) {
            return Some(said(who, actor, excerpt(text, EXCERPT)));
        }
        let blocks = msg.get("content")?.as_array()?;
        Some(self.blocks_row(blocks, who, actor))
    }

    /// The block array of one message, read into one row.
    fn blocks_row(&mut self, blocks: &[Value], who: Who, actor: String) -> Summary {
        let seen = self.scan(blocks);
        // Anything a person would read keeps the record on screen; everything
        // else is a step, however many blocks it took.
        if !seen.spoken.is_empty() {
            let mut what = seen.spoken.join(" \u{b7} ");
            if !seen.calls.is_empty() {
                what.push_str(" \u{b7} ");
                what.push_str(&seen.calls.join(" \u{b7} "));
            }
            return Summary { calls: seen.calls.len(), ..said(who, actor, what) };
        }
        let what = match (seen.calls.is_empty(), seen.results.is_empty()) {
            (false, _) => seen.calls.join(" \u{b7} "),
            (true, false) => seen.results.join(" \u{b7} "),
            (true, true) if seen.thoughts > 0 => thinking_text(seen.thoughts),
            (true, true) => "(no content)".to_string(),
        };
        // A record carrying tool results is the tool's turn, whatever the API
        // calls the role it arrived under.
        let (who, actor) = match seen.results.is_empty() {
            true => (who, actor),
            false => (Who::Tool, "tool".to_string()),
        };
        Summary { class: Class::Step, who, actor, time: None, what, calls: seen.calls.len() }
    }

    /// Read the blocks of one message into the four things a row can say.
    fn scan(&mut self, blocks: &[Value]) -> Blocks {
        let mut seen = Blocks::default();
        for b in blocks {
            match b.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                "text" => match b.get("text").and_then(|t| t.as_str()) {
                    Some(t) if !t.trim().is_empty() => seen.spoken.push(excerpt(t, EXCERPT)),
                    _ => {}
                },
                "thinking" => seen.thoughts += 1,
                "tool_use" => {
                    self.remember(b);
                    seen.calls.push(call_text(b));
                }
                "tool_result" => seen.results.push(self.result_text(b)),
                _ => {}
            }
        }
        seen
    }

    /// Remember a `tool_use` so its result can name it.
    fn remember(&mut self, block: &Value) {
        let (Some(id), Some(name)) = (
            block.get("id").and_then(|i| i.as_str()),
            block.get("name").and_then(|n| n.as_str()),
        ) else {
            return;
        };
        if self.calls.len() >= RECENT_CALLS {
            self.calls.remove(0);
        }
        self.calls.push((id.to_string(), name.to_string()));
    }

    /// The tool a result belongs to, when the call is still remembered.
    fn tool_of(&self, id: &str) -> Option<&str> {
        self.calls
            .iter()
            .rev()
            .find(|(k, _)| k == id)
            .map(|(_, n)| n.as_str())
    }

    /// `Bash → 42 lines`, or `Bash → error`.
    fn result_text(&self, block: &Value) -> String {
        let name = block
            .get("tool_use_id")
            .and_then(|i| i.as_str())
            .and_then(|id| self.tool_of(id))
            .unwrap_or("result");
        let failed = block.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
        if failed {
            return format!("{name} \u{2192} error");
        }
        match result_size(block.get("content")) {
            Some(size) => format!("{name} \u{2192} {size}"),
            None => format!("{name} \u{2192} ok"),
        }
    }
}

/// A record someone would read.
fn said(who: Who, actor: String, what: String) -> Summary {
    Summary {
        class: Class::Message,
        who,
        actor,
        time: None,
        what,
        calls: 0,
    }
}

/// The speaker's name, with a subagent's conversation marked: `isSidechain` is
/// how a `Task` run's own transcript is written into the same file as the run
/// that spawned it.
fn actor_name(v: &Value, role: &str) -> String {
    let side = v.get("isSidechain").and_then(|s| s.as_bool()).unwrap_or(false);
    match side {
        true => format!("\u{21b3} {role}"),
        false => role.to_string(),
    }
}

/// `thinking`, with a count when a record holds several blocks of it.
fn thinking_text(n: usize) -> String {
    match n {
        1 => "thinking".to_string(),
        n => format!("thinking \u{d7}{n}"),
    }
}

/// `Bash(cargo test)` — the call and the one argument worth seeing.
fn call_text(block: &Value) -> String {
    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
    match block.get("input").and_then(tool_arg) {
        Some(arg) => format!("{name}({arg})"),
        None => name.to_string(),
    }
}

/// The argument that says what a call *did*, by the name tools give it.
///
/// Ordered, not guessed: `command` before `file_path` before `pattern`, so a
/// tool carrying several still shows the one a reader scans for. A tool this
/// does not know shows its name alone rather than an arbitrary field.
const ARG_KEYS: [&str; 10] = [
    "command",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "subagent_type",
    "prompt",
    "description",
    "skill",
];

fn tool_arg(input: &Value) -> Option<String> {
    for key in ARG_KEYS {
        if let Some(text) = input.get(key).and_then(|v| v.as_str()) {
            let cut = excerpt(text, ARG);
            if !cut.is_empty() {
                return Some(cut);
            }
        }
    }
    None
}

/// How much a tool returned: `42 lines`, or the size when it is one long line.
fn result_size(content: Option<&Value>) -> Option<String> {
    let text = match content? {
        Value::Str(s) => s.clone(),
        // The block-array form: `[{"type":"text","text":…}]`.
        Value::Array(items) => items
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    if text.is_empty() {
        return Some("empty".to_string());
    }
    let lines = text.lines().count();
    match lines {
        0 | 1 => Some(crate::source::jsonrow::size(text.len() as u64)),
        n => Some(format!("{n} lines")),
    }
}

/// A `system` record: its subtype, plus whatever that subtype is about.
fn system_text(v: &Value) -> String {
    let subtype = v
        .get("subtype")
        .and_then(|s| s.as_str())
        .unwrap_or("system");
    if let Some(ms) = v.get("durationMs").and_then(|d| d.as_number()) {
        let secs = ms.as_f64() / 1000.0;
        return format!("{subtype} {secs:.1}s");
    }
    match v.get("content").and_then(|c| c.as_str()) {
        Some(text) => format!("{subtype}: {}", excerpt(text, EXCERPT)),
        None => subtype.to_string(),
    }
}

/// The transcript's own bookkeeping, one line each. `None` for a `type` this
/// dialect has never seen, which is what sends the record to the generic tree
/// rather than summarising it wrongly.
fn bookkeeping(v: &Value, kind: &str) -> Option<String> {
    let text = match kind {
        "mode" => field(v, "mode"),
        "permission-mode" => field(v, "permissionMode"),
        "ai-title" => field(v, "aiTitle"),
        "last-prompt" => field(v, "lastPrompt"),
        "bridge-session" => field(v, "bridgeSessionId"),
        "summary" => field(v, "summary"),
        "queue-operation" => {
            let op = v.get("operation").and_then(|o| o.as_str()).unwrap_or("");
            let what = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
            Some(format!("{op} {}", excerpt(what, ARG)))
        }
        "attachment" => v
            .get("attachment")
            .and_then(|a| a.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t.to_string()),
        "file-history-snapshot" => Some(String::new()),
        "file-history-delta" => field(v, "trackingPath"),
        _ => return None,
    };
    let detail = text.unwrap_or_default();
    Some(match detail.is_empty() {
        true => kind.to_string(),
        false => format!("{kind} {detail}"),
    })
}

fn field(v: &Value, key: &str) -> Option<String> {
    Some(excerpt(v.get(key)?.as_str()?, ARG))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
