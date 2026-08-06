//! CSV behind the format seam (SPEC.md §CSV).
//!
//! Deliberately minimal: the pieces of CSV support live in their own modules
//! and this file only names them. [`parse`] is the format's foundation — one
//! hand-written RFC 4180 state machine that both the row index (where does a
//! row *end*?) and the renderer (what are this row's *fields*?) run on, so the
//! two can never disagree about a row boundary.
#![deny(unsafe_code)]

pub mod delim;
pub mod index;
pub mod parse;
pub mod read;

// The seam between the two: `parse` decides where a row ends, `index` records
// it, and this asserts that a file read through both comes out identical. It
// lives here rather than inside either module because neither owns the
// agreement — the crate does.
#[cfg(test)]
#[path = "diff_tests.rs"]
mod diff_tests;
