//! JSON behind the format seam (SPEC.md §JSON).
//!
//! Deliberately minimal, like [`crate::csv`]: the pieces live in their own
//! modules and this file only names them and re-exports the surface everything
//! else uses.
//!
//! * [`error`] — what a parse failure says: a byte offset and a reason.
//! * [`index`] — the lazy structural scanner: a container's immediate members
//!   as byte ranges, found by a linear walk that builds no values and is
//!   resumable a chunk at a time.
//! * [`parse`] — the hand-written RFC 8259 reader. Iterative, so nesting is
//!   heap and never stack; fallible with a byte offset, so a bad `.jsonl` line
//!   becomes an error row rather than a dead file.
//! * [`value`] — the tree it builds. Source-faithful: numbers keep their
//!   literal text, duplicate object keys are kept in order.
//! * [`write`] — the tree back to compact JSON, which is what `Y` yanks.
//!
//! Nothing here reads a file, decides a viewport or paints: this is the format,
//! not the source. The lazy structural index and the `Source` implementation
//! sit above it.
#![deny(unsafe_code)]

pub mod error;
pub mod index;
pub mod parse;
pub mod value;
pub mod write;

// The format's surface, named once here so callers above it write
// `json::parse(...)` and `json::Value` rather than reaching into submodules.
// Nothing outside this module uses it yet — the JSON `Source` is a later roll —
// and in a binary crate an unused re-export is a warning; drop the allow once
// the source is wired in.
#[allow(unused_imports)]
pub use error::{Error, Reason};
#[allow(unused_imports)]
pub use parse::{parse, parse_prefix, parse_str, Parser};
#[allow(unused_imports)]
pub use value::{Kind, Member, Number, Value};
#[allow(unused_imports)]
pub use write::to_compact;

// The seam between the three: text parsed to a value and written back must
// come out as the same document, and a value written and re-parsed must come
// out as the same value. Neither module owns that agreement, so the
// round-trip tests live here.
#[cfg(test)]
#[path = "roundtrip_tests.rs"]
mod roundtrip_tests;
