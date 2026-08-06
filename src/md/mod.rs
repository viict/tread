//! Markdown parsing: source text -> `Document`.
//!
//! `ast` holds the types, `block` the block-level parser (with `list` and
//! `table` split out to keep files small), `inline` the span-level parser.
//! Everything here is safe, allocation-only Rust with no I/O.
#![deny(unsafe_code)]

pub mod ast;
pub mod block;
pub mod inline;
mod list;
pub mod sanitize;
mod scan;
mod table;

// The renderer and pager (later rolls) consume these; in a binary crate a
// `pub use` re-export on its own does not count as a use.
#[allow(unused_imports)]
pub use ast::{Align, Block, Document, Inline, LinkRefs, ListItem, ListKind};

/// Parse a markdown document. A leading `---` YAML frontmatter block is
/// treated as metadata and skipped.
///
/// The source is normalised first (see [`sanitize::clean`]): CRLF becomes LF
/// and control characters are neutralised, so no untrusted byte in the file can
/// reach the terminal as an escape sequence.
pub fn parse(src: &str) -> Document {
    block::parse_document(&sanitize::clean(src))
}
