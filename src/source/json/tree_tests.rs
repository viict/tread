//! The tree, against documents held in memory — the same code path a real
//! file takes, minus the file.
//!
//! What these pin is the *cost model*: how much of a document is touched to
//! answer a question. A test that only checked the answers would pass on a
//! reader that parsed the whole file first, which is the one thing this module
//! exists not to do.
#![deny(unsafe_code)]

use super::*;

fn doc(text: &str) -> Doc {
    Doc::memory(text.as_bytes().to_vec())
}

/// The root, indexed in full.
fn indexed(text: &str) -> (Doc, NodeId) {
    let mut d = doc(text);
    let root = d.root().expect("a root");
    d.index(root, usize::MAX, u64::MAX);
    (d, root)
}

fn value_text(d: &mut Doc, node: NodeId, i: usize) -> String {
    let m = d.node(node).member(i).expect("a member");
    d.text(m.start, m.end)
}

#[test]
fn the_root_is_found_without_reading_the_document() {
    let big = format!("[{}]", (0..50_000).map(|i| i.to_string()).collect::<Vec<_>>().join(","));
    let d = doc(&big);
    assert!(d.size() > 200_000);
    assert_eq!(d.walked(), 0, "opening walks nothing at all");
    assert!(d.root().is_some());
    assert_eq!(d.node(d.root().unwrap()).shape, Shape::Array);
}

#[test]
fn a_document_with_no_value_has_no_root() {
    assert!(doc("   \n").root().is_none());
    assert!(doc("").root().is_none());
}

#[test]
fn members_are_indexed_only_as_far_as_they_are_asked_for() {
    let big = format!("[{}]", ["\"xxxxxxxxxxxxxxxx\""; 20_000].join(","));
    let mut d = doc(&big);
    let root = d.root().unwrap();
    let known = d.index(root, 4, u64::MAX);
    assert!(known >= 4, "the first four members are there");
    assert!(
        d.walked() <= 8192,
        "walked {} bytes of a {} byte document to find four members",
        d.walked(),
        d.size()
    );
    assert!(!d.node(root).complete(), "and the rest is still unknown");
}

#[test]
fn a_budget_stops_a_scan_and_the_next_call_resumes_it() {
    let big = format!("[{}]", vec!["1"; 200_000].join(","));
    let mut d = doc(&big);
    let root = d.root().unwrap();
    let mut last = 0;
    for _ in 0..3 {
        let now = d.index(root, usize::MAX, 4096);
        assert!(now > last, "each slice makes progress");
        last = now;
    }
    assert!(!d.node(root).complete());
    d.index(root, usize::MAX, u64::MAX);
    assert_eq!(d.node(root).count(), 200_000);
    assert!(d.node(root).complete());
}

#[test]
fn expanding_a_node_indexes_that_node_and_nothing_else() {
    let inner = format!("[{}]", vec!["7"; 20_000].join(","));
    let text = format!("{{\"small\": [1,2], \"big\": {inner}}}");
    let (mut d, root) = indexed(&text);
    let cheap = d.walked();
    // The root's own walk had to step *over* the big array, so it has read the
    // whole file once. What matters is that opening `small` does not walk
    // `big` again — each level is indexed on its own.
    let small = d.open_child(root, 0).expect("an array");
    d.index(small, usize::MAX, u64::MAX);
    assert_eq!(d.node(small).count(), 2);
    assert!(
        d.walked() - cheap < 100,
        "opening a two-element array walked {} bytes",
        d.walked() - cheap
    );
    assert_eq!(d.dpath(small), ".small");
    assert_eq!(d.fold_id(small), "/0", "fold ids are positional");
}

/// The point of the whole design: an object holding one enormous array must
/// open without walking the array.
#[test]
fn one_object_holding_one_enormous_array_stays_cheap() {
    let inner = format!("[{}]", vec!["123456"; 100_000].join(","));
    let text = format!("{{\"head\": 1, \"big\": {inner}}}");
    let mut d = doc(&text);
    let root = d.root().unwrap();
    // Enough to paint a first screen: two members.
    d.index(root, 1, u64::MAX);
    assert!(
        d.walked() <= 8192,
        "walked {} bytes of {} to show the first member",
        d.walked(),
        d.size()
    );
}

#[test]
fn a_collapsed_container_is_counted_from_the_index() {
    let (mut d, root) = indexed(r#"{"a": [1,2,3], "b": {"x": 1}, "c": 4}"#);
    let a = d.node(root).member(0).unwrap();
    assert_eq!(d.count(a, u64::MAX), (3, true));
    let b = d.node(root).member(1).unwrap();
    assert_eq!(d.count(b, u64::MAX), (1, true));
    let c = d.node(root).member(2).unwrap();
    assert_eq!(d.count(c, u64::MAX), (0, true), "a scalar counts nothing");
}

#[test]
fn a_count_too_big_for_one_budget_reports_what_it_has_and_converges() {
    let inner = format!("[{}]", vec!["1"; 100_000].join(","));
    let (mut d, root) = indexed(&format!("[{inner}]"));
    let m = d.node(root).member(0).unwrap();
    let (partial, done) = d.count(m, 4096);
    assert!(partial > 0 && partial < 100_000, "counted {partial} so far");
    assert!(!done, "and says it is not finished");
    while d.extend_counts(1 << 20) {}
    assert_eq!(d.count(m, 0), (100_000, true));
}

#[test]
fn keys_are_decoded_and_paths_are_readable() {
    let (mut d, root) = indexed(r#"{"name": 1, "odd key": 2, "a\nb": 3, "2x": 4}"#);
    assert_eq!(d.path_of(root, 0), ".name");
    assert_eq!(d.path_of(root, 1), r#"["odd key"]"#);
    assert_eq!(d.path_of(root, 2), r#"["a\nb"]"#);
    assert_eq!(d.path_of(root, 3), r#"["2x"]"#, "a key that is not an identifier");
    let m = d.node(root).member(2).unwrap();
    assert_eq!(d.key_text(m).as_deref(), Some("a\nb"), "escapes are resolved");
}

#[test]
fn an_array_element_is_addressed_by_its_index() {
    let (mut d, root) = indexed(r#"[{"name": "ada"}, 2]"#);
    assert_eq!(d.path_of(root, 0), "[0]");
    assert_eq!(d.path_of(root, 1), "[1]");
    let child = d.open_child(root, 0).unwrap();
    d.index(child, usize::MAX, u64::MAX);
    assert_eq!(d.path_of(child, 0), "[0].name");
}

#[test]
fn a_member_keeps_its_source_text_exactly() {
    let (mut d, root) = indexed(r#"[1e999, 0.1, 12345678901234567890123456789012345678901234]"#);
    assert_eq!(value_text(&mut d, root, 0), "1e999");
    assert_eq!(value_text(&mut d, root, 1), "0.1");
    assert_eq!(
        value_text(&mut d, root, 2),
        "12345678901234567890123456789012345678901234"
    );
}

#[test]
fn an_oversized_member_is_reported_by_size_and_never_loaded() {
    let huge = "x".repeat((PARSE_CAP + 4096) as usize);
    let (mut d, root) = indexed(&format!("[\"{huge}\"]"));
    let m = d.node(root).member(0).unwrap();
    assert!(m.len() > PARSE_CAP);
    let (bytes, clipped) = d.bytes(m.start, m.end);
    assert!(clipped, "the read is clipped rather than the member loaded");
    assert!(bytes.len() as u64 <= PARSE_CAP);
}

#[test]
fn progress_is_honest_while_the_root_is_being_walked() {
    let big = format!("[{}]", vec!["1"; 100_000].join(","));
    let mut d = doc(&big);
    let root = d.root().unwrap();
    assert_eq!(d.progress(), 0);
    d.index(root, usize::MAX, 8192);
    let mid = d.progress();
    assert!(mid > 0 && mid < 100, "{mid}%");
    d.index(root, usize::MAX, u64::MAX);
    assert_eq!(d.progress(), 100);
}

#[test]
fn a_truncated_document_still_yields_its_members() {
    let (mut d, root) = indexed(r#"[1, 2, {"a": "unterminated"#);
    assert_eq!(d.node(root).count(), 3);
    assert!(d.node(root).complete(), "the walk settles rather than hanging");
    assert_eq!(value_text(&mut d, root, 1), "2");
}

/// Ten thousand levels, opened one at a time. The tree holds a node per open
/// container and no recursion anywhere; what would break here is a recursive
/// path builder or a recursive drop. It stops handing back nodes at
/// [`MAX_DEPTH`], which is what bounds the per-level byte re-walk.
#[test]
fn ten_thousand_levels_can_be_opened_one_by_one() {
    const DEPTH: usize = 10_000;
    let text = format!("{}{}", "[".repeat(DEPTH), "]".repeat(DEPTH));
    let mut d = doc(&text);
    let mut node = d.root().unwrap();
    for _ in 0..DEPTH - 1 {
        d.index(node, 1, u64::MAX);
        node = match d.open_child(node, 0) {
            Some(c) => c,
            None => break,
        };
    }
    assert_eq!(d.node(node).depth as usize, MAX_DEPTH as usize);
    // Dropping the tree must not recurse either.
    drop(d);
}

// -- identity, and what it costs -------------------------------------------

/// The fold id and the readable path are *derived* from the parent chain, not
/// stored on the node. Multi-digit indices and a mix of objects and arrays are
/// where a hand-rolled spelling goes wrong.
#[test]
fn a_nodes_fold_id_and_path_are_spelled_from_its_parent_chain() {
    let text = format!(
        r#"{{"rows":[{}{{"deep":{{"in":[1]}}}}]}}"#,
        "0,".repeat(123)
    );
    let (mut d, root) = indexed(&text);
    assert_eq!(d.fold_id(root), "", "the root has the empty id");
    assert_eq!(d.dpath(root), "");

    let rows = d.open_child(root, 0).expect("rows is a container");
    assert_eq!(d.fold_id(rows), "/0");
    assert_eq!(d.dpath(rows), ".rows");

    d.index(rows, usize::MAX, u64::MAX);
    let obj = d.open_child(rows, 123).expect("the object after 123 zeroes");
    assert_eq!(d.fold_id(obj), "/0/123", "multi-digit indices survive");
    assert_eq!(d.dpath(obj), ".rows[123]");

    d.index(obj, usize::MAX, u64::MAX);
    let deep = d.open_child(obj, 0).expect("deep");
    d.index(deep, usize::MAX, u64::MAX);
    let inner = d.open_child(deep, 0).expect("in");
    assert_eq!(d.fold_id(inner), "/0/123/0/0");
    assert_eq!(d.dpath(inner), ".rows[123].deep.in");
}

/// Nesting is refused at the *shared presentation* depth, so a shape readable
/// as a line of a log is readable as a file too — and at a depth shallower than
/// the parser's own refusal, because the tree pays a byte re-walk per level and
/// the parser does not.
#[test]
fn the_tree_stops_opening_at_the_shared_presentation_depth() {
    assert_eq!(MAX_DEPTH as usize, crate::source::jsonrow::MAX_DEPTH);
    assert!(MAX_DEPTH as usize <= crate::json::parse::DEFAULT_MAX_DEPTH);
}

/// A document of nothing but `[` must cost memory in proportion to its *depth*,
/// not to its depth squared. Storing a whole fold id per node made a 200KB file
/// allocate gigabytes; deriving it makes every node the same size.
#[test]
fn opening_a_deeply_nested_document_costs_a_bounded_amount_per_level() {
    // As deep as the tree will open, so the deepest node really is the last
    // level and its derived id really is that long.
    let levels = MAX_DEPTH as usize + 1;
    let text = format!("{}1{}", "[".repeat(levels), "]".repeat(levels));
    let mut d = doc(&text);
    let mut node = d.root().expect("a root");
    for _ in 0..levels - 1 {
        d.index(node, 1, u64::MAX);
        let Some(next) = d.open_child(node, 0) else { break };
        node = next;
    }
    // Every node holds a parent link and its own short segment, so the deepest
    // node's id is spelled on demand and never stored.
    assert_eq!(d.node(node).depth as usize, levels - 1);
    assert_eq!(d.dpath(node), "[0]".repeat(levels - 1));
    assert_eq!(d.fold_id(node), "/0".repeat(levels - 1));
}

/// Past the limit the tree hands back nothing to open, rather than growing a
/// node per level for ever.
#[test]
fn a_container_past_the_depth_limit_is_not_opened() {
    let levels = MAX_DEPTH as usize + 10;
    let text = format!("{}1{}", "[".repeat(levels), "]".repeat(levels));
    let mut d = doc(&text);
    let mut node = d.root().expect("a root");
    let mut opened = 0usize;
    loop {
        d.index(node, 1, u64::MAX);
        assert_eq!(d.too_deep(node), d.node(node).depth >= MAX_DEPTH);
        let Some(next) = d.open_child(node, 0) else { break };
        node = next;
        opened += 1;
    }
    assert_eq!(opened, MAX_DEPTH as usize, "one node per level, and no more");
    assert!(d.too_deep(node), "the last node opened is at the limit");
}
