//! Property-style tests for the RFC 4180 machine: deterministic pseudo-random
//! documents, checked against the invariants that keep the row index honest.
//! No `rand` crate — the corpus is generated from a fixed seed, so a failure
//! here reproduces exactly.

use super::*;
use crate::csv::delim::{sniff, CANDIDATES};

/// A tiny xorshift so the corpus is pseudo-random but identical on every run —
/// no `rand` crate, and a failure is always reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
}

/// Fragments chosen to hit every state and every recovery path.
const FRAGMENTS: &[&str] = &[
    "a", "", " ", "  x ", "\"q\"", "\"a,b\"", "\"a\nb\"", "\"a\r\nb\"", "\"\"\"\"", "\"a\"\"b\"",
    "\"unterminated", "\"a\"b\"", "x\"y", "\t", ";", "|", ",", "\0", "é", "日本",
];

const SEPARATORS: &[&str] = &[",", "\n", "\r\n", "\r", ",,", "\n\n"];

fn fuzz_doc(seed: u64, len: usize) -> Vec<u8> {
    let mut rng = Rng(seed);
    let mut out = String::new();
    for _ in 0..len {
        out.push_str(rng.pick(FRAGMENTS));
        out.push_str(rng.pick(SEPARATORS));
    }
    out.into_bytes()
}

#[test]
fn fuzzed_documents_never_panic_and_the_two_readers_always_agree() {
    for seed in 1..40u64 {
        let doc = fuzz_doc(seed, 12);
        for &delim in &CANDIDATES {
            agree(&doc, delim);
        }
    }
}

#[test]
fn fuzzed_documents_round_trip_through_the_row_index() {
    // What the CSV source actually does: index the offsets in one streaming
    // pass, then later parse one row from its slice. The fields must be the
    // same ones a single pass over the whole buffer produced.
    for seed in 100..140u64 {
        let doc = fuzz_doc(seed, 10);
        let delim = sniff(&doc);
        let ends = chunked_ends(&doc, delim, 7);
        let whole = records(&doc, delim);
        assert_eq!(ends.len(), whole.len(), "seed {seed}");
        let mut start = bom_len(&doc) as u64;
        for (end, want) in ends.iter().zip(&whole) {
            assert!(*end > start || want.iter().all(String::is_empty));
            assert_eq!(&record(&doc[start as usize..*end as usize], delim), want, "seed {seed}");
            start = *end;
        }
    }
}

#[test]
fn every_byte_of_the_document_belongs_to_exactly_one_row() {
    for seed in 200..230u64 {
        let doc = fuzz_doc(seed, 8);
        let recs: Vec<Record> = Records::new(&doc, b',').collect();
        let mut at = bom_len(&doc);
        for r in &recs {
            assert_eq!(r.start, at);
            assert!(r.end > r.start, "a row always consumes at least one byte");
            at = r.end;
        }
        assert_eq!(at, doc.len(), "seed {seed}: the whole file is covered");
    }
}
