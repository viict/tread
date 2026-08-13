//! The double every `plan` test is built on: a lens with no dialect, a plan
//! over a string of record kinds, and the two helpers that give records the
//! heights a source's measurement would have produced.
//!
//! One copy, shared by `plan_tests.rs` and `plan_level_tests.rs` — two halves of
//! one file, split to stay under the size limit. A second copy would drift, and
//! the whole point of these tests is that two answers agree.
#![deny(unsafe_code)]

use super::*;
use crate::json::Value;
use crate::lens::{Class, Summary, Who};

/// A lens with no dialect: the fixture says what each record is.
///
/// `{"k":"m"}` a message, `{"k":"s"}` a step, anything else unrecognised.
pub(super) struct Fake;

impl crate::lens::Lens for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn about(&self) -> &'static str {
        "test double"
    }
    fn read(&mut self, v: &Value) -> Option<Summary> {
        let class = match v.get("k").and_then(|k| k.as_str())? {
            "m" => Class::Message,
            "s" => Class::Step,
            _ => return None,
        };
        Some(Summary {
            class,
            who: Who::System,
            actor: "x".to_string(),
            time: None,
            what: String::new(),
            calls: 1,
            // `"t"` is what this record claims to have spent, so the seam's
            // running total can be exercised without a dialect.
            tokens: v.get("t").and_then(|t| t.as_number()).and_then(|n| n.as_i64()).unwrap_or(0) as u64,
            // A body is the source's to measure; these tests give one to an
            // item directly (`with_bodies`), which is the same number the
            // measurement would have produced and keeps this file file-free.
            body: None,
        })
    }
}

/// Build a plan over `kinds` — `m`, `s`, or `?` for a record the lens does not
/// know — with a fresh row map.
pub(super) fn plan_of(kinds: &str) -> (Plan, RowMap) {
    let mut plan = Plan::new(Box::new(Fake));
    let mut map = RowMap::default();
    for (i, ch) in kinds.chars().enumerate() {
        let json = format!(r#"{{"k":"{ch}"}}"#);
        let value = crate::json::parse(json.as_bytes()).expect("fixture");
        plan.classify(i, Some(&value), &mut map);
    }
    plan.sync();
    (plan, map)
}

/// The same, with every record claiming to have spent `each` tokens.
pub(super) fn plan_of_spend(kinds: &str, each: u64) -> (Plan, RowMap) {
    let mut plan = Plan::new(Box::new(Fake));
    let mut map = RowMap::default();
    for (i, ch) in kinds.chars().enumerate() {
        let json = format!(r#"{{"k":"{ch}","t":{each}}}"#);
        let value = crate::json::parse(json.as_bytes()).expect("fixture");
        plan.classify(i, Some(&value), &mut map);
    }
    plan.sync();
    (plan, map)
}

/// Every row of the document, as `(record, sub)` or `group N`.
pub(super) fn walk(plan: &Plan, map: &RowMap, known: usize) -> Vec<String> {
    (0..plan.rows(known, map))
        .map(|row| match plan.at(row, known, map) {
            Spot::Group { item } => format!("group {item}"),
            Spot::Body { record, line } => format!("{record}b{line}"),
            Spot::Part { record, line } => format!("{record}p{line}"),
            Spot::Record { record, sub } => format!("{record}.{sub}"),
        })
        .collect()
}

/// Give every message record a body `rows` tall — the number the source's
/// measurement would have produced at some width, which is all the arithmetic
/// here ever sees.
pub(super) fn with_bodies(plan: &mut Plan, rows: usize) {
    let items: Vec<(usize, usize, bool)> = plan
        .items()
        .iter()
        .map(|it| (it.first, it.count, it.step))
        .collect();
    for (first, count, step) in items {
        if !step {
            plan.set_under(first, Under { body: rows, parts: 0 }, false);
        }
        let _ = count;
    }
    plan.sync();
}

/// Give every record — messages *and* the steps inside an open run — a body
/// and a parts block. A step's reasoning is text and is shown wherever the
/// step is, which is the arithmetic this exercises.
pub(super) fn with_under(plan: &mut Plan, under: Under) {
    let items: Vec<(usize, usize, bool, bool)> = plan
        .items()
        .iter()
        .map(|it| (it.first, it.count, it.is_group(), it.open))
        .collect();
    for (first, count, group, open) in items {
        match group {
            false => plan.set_under(first, under, false),
            true if open => {
                for r in first..first + count {
                    plan.set_under(r, under, true);
                }
            }
            true => {}
        }
    }
    plan.sync();
}
