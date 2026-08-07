//! Fold state and the flatten, without a renderer in the way.
#![deny(unsafe_code)]

use super::*;
use crate::source::json::tree::MAX_DEPTH;

fn doc(text: &str) -> Doc {
    Doc::memory(text.as_bytes().to_vec())
}

/// Every row of `text` under `folds`, as `(node, part)`.
fn rows(d: &mut Doc, folds: &Folds) -> Vec<(NodeId, Part)> {
    let mut f = Flat::default();
    f.extend(d, folds, usize::MAX, u64::MAX);
    (0..f.len())
        .filter_map(|i| f.get(i))
        .map(|r| (r.node, r.part()))
        .collect()
}

fn parts(d: &mut Doc, folds: &Folds) -> Vec<Part> {
    rows(d, folds).into_iter().map(|(_, p)| p).collect()
}

#[test]
fn a_document_opens_with_its_root_open_and_everything_under_it_folded() {
    let mut d = doc(r#"{"a": 1, "b": {"c": 2}, "d": [1,2]}"#);
    assert_eq!(
        parts(&mut d, &Folds::new()),
        vec![Part::Head, Part::Member(0), Part::Member(1), Part::Member(2), Part::Tail],
        "three member rows, whatever they contain"
    );
}

#[test]
fn a_scalar_root_is_one_row() {
    for text in ["42", "\"hi\"", "true", "null"] {
        let mut d = doc(text);
        assert_eq!(parts(&mut d, &Folds::new()), vec![Part::Head], "{text}");
    }
}

#[test]
fn an_empty_document_has_no_rows() {
    let mut d = doc("   ");
    assert!(parts(&mut d, &Folds::new()).is_empty());
}

#[test]
fn opening_a_member_puts_its_own_rows_in_line() {
    let mut d = doc(r#"{"a": 1, "b": {"c": 2, "e": 3}}"#);
    let mut folds = Folds::new();
    assert!(folds.set("/1", true), "open the object under `b`");
    assert_eq!(
        parts(&mut d, &folds),
        vec![
            Part::Head,       // {
            Part::Member(0),  //   "a": 1
            Part::Head,       //   "b": {
            Part::Member(0),  //     "c": 2
            Part::Member(1),  //     "e": 3
            Part::Tail,       //   }
            Part::Tail,       // }
        ]
    );
}

#[test]
fn closing_the_root_hides_everything() {
    let mut d = doc(r#"[1,2,3]"#);
    let mut folds = Folds::new();
    assert!(folds.set("", false));
    assert_eq!(parts(&mut d, &folds), vec![Part::Head]);
}

/// `zR`: everything open, without enumerating anything.
#[test]
fn expanding_everything_is_one_boolean() {
    let mut folds = Folds::new();
    folds.all(true);
    assert!(folds.is_open(""));
    assert!(folds.is_open("/0/1/2"), "a node nobody has ever seen");
    let mut d = doc(r#"{"a": [1, {"b": 2}]}"#);
    assert_eq!(
        parts(&mut d, &folds),
        vec![
            Part::Head,      // {
            Part::Head,      //   "a": [
            Part::Member(0), //     1
            Part::Head,      //     {
            Part::Member(0), //       "b": 2
            Part::Tail,      //     }
            Part::Tail,      //   ]
            Part::Tail,      // }
        ]
    );
    folds.all(false);
    assert!(!folds.is_open(""), "zM shuts the root too");
    assert_eq!(parts(&mut d, &folds), vec![Part::Head]);
}

#[test]
fn fold_state_round_trips_through_the_pager() {
    let mut folds = Folds::new();
    folds.set("/3", true);
    folds.set("/3/1", true);
    let state = folds.state();
    let mut other = Folds::default();
    other.restore(state.clone());
    for id in ["", "/3", "/3/1", "/4"] {
        assert_eq!(other.is_open(id), folds.is_open(id), "{id}");
    }
    // And the same for the "everything open" state, which is one marker.
    folds.all(true);
    folds.set("/2", false);
    let mut third = Folds::default();
    third.restore(folds.state());
    assert!(third.is_open("/9"));
    assert!(!third.is_open("/2"));
}

#[test]
fn setting_a_fold_to_what_it_already_is_changes_nothing() {
    let mut folds = Folds::new();
    assert!(!folds.set("", true), "the root is already open");
    assert!(folds.set("", false));
    assert!(!folds.set("", false));
    assert!(!folds.any_below(), "only the root's own state differs");
    folds.set("/1", true);
    assert!(folds.any_below());
}

/// The walk is resumable: a stingy budget finds some rows now and the rest
/// later, and the answer is the same either way.
#[test]
fn a_budget_stops_the_walk_and_the_next_call_resumes_it() {
    let text = format!("[{}]", vec!["1"; 50_000].join(","));
    let mut d = doc(&text);
    let folds = Folds::new();
    let mut f = Flat::default();
    let mut last = 0;
    for _ in 0..4 {
        f.extend(&mut d, &folds, usize::MAX, 4096);
        assert!(f.len() > last, "each slice finds more rows");
        assert!(!f.done());
        last = f.len();
    }
    f.extend(&mut d, &folds, usize::MAX, u64::MAX);
    assert!(f.done());
    assert_eq!(f.len(), 50_000 + 2, "head, every member, tail");
}

#[test]
fn only_the_rows_asked_for_are_found() {
    let text = format!("[{}]", vec!["1"; 50_000].join(","));
    let mut d = doc(&text);
    let mut f = Flat::default();
    f.extend(&mut d, &Folds::new(), 10, u64::MAX);
    assert_eq!(f.len(), 10);
    assert!(!f.done());
    assert!(d.walked() <= 8192, "walked {} bytes for ten rows", d.walked());
}

#[test]
fn a_reset_keeps_the_index_and_rebuilds_the_rows() {
    let mut d = doc(r#"[1,2,3]"#);
    let folds = Folds::new();
    let mut f = Flat::default();
    f.extend(&mut d, &folds, usize::MAX, u64::MAX);
    let before = d.walked();
    f.reset();
    assert_eq!(f.len(), 0);
    f.extend(&mut d, &folds, usize::MAX, u64::MAX);
    assert_eq!(f.len(), 5);
    assert_eq!(d.walked(), before, "no byte is walked twice");
}

/// Ten thousand levels, every one of them asked to open. The flatten is an
/// explicit stack, so this is heap and not stack — the crash this design exists
/// to avoid — and it stops at [`MAX_DEPTH`], where the row below the deepest
/// opened container is the refusal note rather than another bracket.
#[test]
fn flattening_ten_thousand_levels_does_not_recurse() {
    const DEPTH: usize = 10_000;
    let cap = MAX_DEPTH as usize;
    let text = format!("{}1{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    let mut d = doc(&text);
    let mut folds = Folds::new();
    folds.all(true);
    let mut f = Flat::default();
    f.extend(&mut d, &folds, usize::MAX, u64::MAX);
    assert!(f.done());
    // One head and one tail for each level from the root down to the limit,
    // plus the one row that stands for everything past it.
    assert_eq!(f.len(), (cap + 1) * 2 + 1);
    assert_eq!(f.get(cap + 1).map(Row::part), Some(Part::Member(0)));
}

/// Under the limit, every level opens and the innermost scalar is a row of its
/// own: the arithmetic above is a limit, not a truncation of ordinary
/// documents.
#[test]
fn flattening_stops_only_past_the_limit() {
    let depth = MAX_DEPTH as usize;
    let text = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let mut d = doc(&text);
    let mut folds = Folds::new();
    folds.all(true);
    let mut f = Flat::default();
    f.extend(&mut d, &folds, usize::MAX, u64::MAX);
    assert!(f.done());
    assert_eq!(f.len(), depth * 2 + 1);
    assert_eq!(f.get(depth).map(Row::part), Some(Part::Member(0)));
}
