//! The `--lens` seam (SPEC.md §Lenses).
//!
//! # What a lens is, and what it is not
//!
//! A lens is a **transform over records**, not a format. It never reads a file,
//! never owns a [`crate::source::Source`], and never decides where a row goes
//! on the screen: given one parsed record it says *what that record is* — who
//! spoke, when, what happened, and whether the record is conversation or
//! mechanics. The record seam ([`crate::source::record`]) does the rest: it
//! turns runs of mechanics into one foldable group row and leaves the
//! conversation alone.
//!
//! That split is the whole point. A second dialect — opencode, OpenAI Codex,
//! ATIF — is a new module here and one entry in [`LENSES`]; nothing about
//! folding, row arithmetic, the levels a record has, search or yanking is
//! written twice. [`part`] is the vocabulary the second half of that answer is
//! given in: what a record's parts *are*, said without saying anything about
//! rows.
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
//!   and thrown away with it — and a [`Body`] keeps that true for the message
//!   text as well, by holding a bounded head and *where the rest is* rather
//!   than a copy of it.
//! * [`Lens::detail`] is the exception that proves it: the parts of a record —
//!   its tool calls, with their arguments and what came back — are read **when
//!   the reader opens that record** and are never stored. A `Summary` is per
//!   record and forever; a `Vec<Part>` is per keystroke and momentary.
#![deny(unsafe_code)]

pub mod agent;
pub mod atif;
pub mod num;
pub mod part;
pub mod usage;
pub mod usage_agent;
pub mod usage_atif;

pub use num::tokens;
pub use part::Part;

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

/// One step of the way back to a message inside its own record: a key, or an
/// index into an array. Static keys because a dialect names them in its own
/// source (`message`, `content`, `text`) — nothing is allocated to remember
/// where a message lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Key(&'static str),
    At(usize),
}

/// Bytes of a message the seam keeps in memory, per record.
///
/// The ceiling on what a summary costs: a 40 KB message contributes 1 KB and a
/// path, not 40 KB. Enough to lay a clipped body out at any width up to about
/// 190 columns, and past that the clip shows fewer rows and still says what it
/// is not showing — which is the half that may not slip.
pub const BODY_KEEP: usize = 1024;

/// The message text under a summary row (SPEC.md §Lenses).
///
/// # Why this is a head and a path, not the text
///
/// A summary is kept for **every** classified record, and this module's
/// contract is that nothing here allocates per document. Holding each message
/// in full would make a long log's summaries as big as the log. So a `Body`
/// keeps:
///
/// * `head` — the first [`BODY_KEEP`] bytes, which is what a clipped row paints
///   and what the row *arithmetic* measures, so a resize re-lays every body
///   without reading a file;
/// * `bytes` and `lines` — what the whole message is, so a clipped body can say
///   what it is not showing, and so its height at a width can be *derived*
///   rather than measured;
/// * `at` — where the text sits inside the record, so painting reads the whole
///   of it back out of the record that is being painted anyway.
///
/// The dialect supplies text and a path and makes no layout decision: how many
/// rows this becomes is [`crate::source::record::body`]'s answer, and it
/// depends on the width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Body {
    /// The first [`BODY_KEEP`] bytes of the message, on a character boundary.
    pub head: String,
    /// Bytes the whole message has.
    pub bytes: usize,
    /// Lines the whole message has, counted as the file wrote them.
    pub lines: usize,
    /// The path from the record's root to the text.
    pub at: Vec<Step>,
}

impl Body {
    /// The body of `text`, which lives at `at` inside its record.
    pub fn new(text: &str, at: Vec<Step>) -> Body {
        Body {
            head: head_of(text, BODY_KEEP).to_string(),
            bytes: text.len(),
            lines: text.lines().count().max(1),
            at,
        }
    }

    /// Is the head the whole message? Then the record never has to be read
    /// again to paint it.
    pub fn whole(&self) -> bool {
        self.head.len() == self.bytes
    }

    /// The whole message: out of the record when the head is short of it, and
    /// out of the head when it is not. `record` is `None` where the caller has
    /// no record in hand, and the head is then the honest answer it can give.
    pub fn text_in<'a>(&'a self, record: Option<&'a Value>) -> &'a str {
        if self.whole() {
            return &self.head;
        }
        match record.and_then(|v| self.walk(v)) {
            Some(text) if text.len() == self.bytes => text,
            _ => &self.head,
        }
    }

    fn walk<'a>(&self, root: &'a Value) -> Option<&'a str> {
        let mut at = root;
        for step in &self.at {
            at = match step {
                Step::Key(k) => at.get(k)?,
                Step::At(n) => at.index(*n)?,
            };
        }
        at.as_str()
    }
}

/// The first `bytes` of `text`, cut on a character boundary: a message may be
/// any UTF-8 at all, and cutting mid-character would panic.
fn head_of(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut cut = bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    &text[..cut]
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
    /// Every token unit this record recorded, exact; `0` when it records none.
    ///
    /// Exact and not a spelling, because the two places a total is shown add
    /// these up — the group row over a run ([`crate::source::record::lensrow`])
    /// and the status bar over the document
    /// ([`crate::source::record::view`]) — and a sum of rounded numbers is not
    /// the rounding of a sum. [`tokens`] is the *only* spelling of one, so the
    /// row and the total cannot disagree about what `18k` means.
    ///
    /// A dialect that reads no counters leaves it `0`, and the two clauses that
    /// would show it are then omitted entirely rather than saying `0 tokens`.
    pub tokens: u64,
    /// The record's own text, under the row: what was said for a message, and
    /// what the model was thinking for a step that only thought.
    ///
    /// A message's body is **one wrap split in two** — the summary row is its
    /// first line and the rows under it are the rest. A step's is not: a step's
    /// row is the description of what it did (`thinking · bash(make)`), and its
    /// text goes wholly underneath. [`Class`] is what tells the two apart, and
    /// `src/source/record/body.rs` is where that decision is spent.
    pub body: Option<Body>,
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
            tokens: 0,
            body: None,
        }
    }

    /// The same summary with a clock time attached.
    pub fn at(mut self, time: Option<String>) -> Summary {
        self.time = time;
        self
    }
}

/// Where a dialect's records live.
///
/// A lens is a transform over records, and this is the one thing it says about
/// where they come from — because that decides which *file* the flag can be
/// pointed at, and nothing else about the dialect does. It is a declaration,
/// not a reader: `src/open/lens.rs` turns it into a format, and
/// `src/source/record/` reaches the records the same way whichever it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordsAt {
    /// One record per line: a `.jsonl` / `.ndjson` file. The default, and what
    /// every dialect meant before there was a second answer.
    Lines,
    /// The root of one JSON document is the array of records.
    ///
    /// No dialect declares this yet — `atif`, the only document dialect, names
    /// a member. The source implements it all the same
    /// ([`crate::source::jsonarray::At::Root`], exercised by its tests),
    /// because "the records are the whole document" is the *simpler* of the two
    /// shapes and leaving it out would mean the next dialect to want it had to
    /// add a variant, a routing arm and a source path at once.
    #[allow(dead_code)]
    Root,
    /// The records are the array under this top-level key of one JSON
    /// document; every *other* top-level key is record 0, so nothing the
    /// document says about the run is lost (SPEC.md §Lenses).
    Member(&'static str),
}

/// One dialect of record file.
pub trait Lens {
    /// The name `--lens` takes.
    fn name(&self) -> &'static str;

    /// One line for `--lens list`.
    fn about(&self) -> &'static str;

    /// Where this dialect's records are. A record per line unless the dialect
    /// says otherwise, which is what every dialect written before documents
    /// were readable still means.
    fn records_at(&self) -> RecordsAt {
        RecordsAt::Lines
    }

    /// Read one record. `None` means "not mine" — the record renders as the
    /// generic tree, whole.
    ///
    /// Called once per record, in file order.
    fn read(&mut self, value: &Value) -> Option<Summary>;

    /// What this record's parts **are**, for the level between the headline and
    /// the raw JSON tree (SPEC.md §Lenses): the tool calls it made, and any text
    /// it holds that the body under its row is not already showing.
    ///
    /// Unlike [`Lens::read`] this is **not** called once per record. It is
    /// called for the record the reader opened, when they open it, and its
    /// answer is thrown away when they close it — which is why a [`Part`] may
    /// hold [`Body`]s at all and why nothing here is stored on a [`Summary`].
    ///
    /// `&self`, not `&mut self`, and that is the contract rather than an
    /// accident: `read` runs far ahead of the viewport, so any state a dialect
    /// carried across records is long past by the time a reader presses `Enter`.
    /// A dialect that cannot answer from **this record alone** must return the
    /// parts it can and leave the rest `None` — the raw tree is still one `r`
    /// away, and a guess would be worse than a gap.
    ///
    /// The default is no parts, which is a level with nothing in it: the ladder
    /// then has one rung fewer and `Enter` goes straight on to the tree. A
    /// dialect implements this when it can say something a person would say.
    fn detail(&self, value: &Value) -> Vec<Part> {
        let _ = value;
        Vec::new()
    }
}

/// How a lens is built. A function pointer rather than a value because a lens
/// carries state across records, so every open needs its own.
type Make = fn() -> Box<dyn Lens>;

/// Every lens there is. **Adding a dialect is one module and one line here**
/// (docs/lenses.md says what the module has to provide).
const LENSES: &[(&str, Make)] = &[
    (agent::NAME, || Box::new(agent::Agent::default())),
    (atif::NAME, || Box::new(atif::Atif)),
    (usage_agent::NAME, || Box::new(usage_agent::Usage)),
    (usage_atif::NAME, || Box::new(usage_atif::UsageAtif)),
];

/// The lens called `name`, or `None`.
pub fn find(name: &str) -> Option<Box<dyn Lens>> {
    LENSES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, make)| make())
}

/// Where the lens called `name` keeps its records. The routing asks before the
/// file is opened, because the answer decides which formats `--lens <name>`
/// will accept.
pub fn records_at(name: &str) -> Option<RecordsAt> {
    find(name).map(|l| l.records_at())
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
///
/// The gutter is *after* the padding rather than inside it: `usage-atif` is
/// exactly ten columns, and a name that filled the field would otherwise run
/// straight into its own description.
pub fn list_text() -> String {
    let mut s = String::from("lenses (--lens <name>):\n");
    for (name, make) in LENSES {
        s.push_str(&format!("    {name:<10}  {}\n", make().about()));
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
