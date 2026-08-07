//! Tests for the value tree.
//!
//! The accessors are the easy half. The half that matters is that none of the
//! traits recurse: `Drop`, `Clone`, `PartialEq`, `Debug` and `depth` are all
//! run here against a tree tens of thousands deep, built without the parser so
//! the depth limit is not in the way.

use super::*;

/// A tower of `depth` arrays with a `1` at the bottom, built iteratively —
/// building it recursively in a test would be the crash under test.
fn tower(depth: usize) -> Value {
    let mut v = Value::number("1");
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}

/// The same, but through objects.
fn obj_tower(depth: usize) -> Value {
    let mut v = Value::number("1");
    for i in 0..depth {
        v = Value::Object(vec![Member::new(format!("k{i}"), v)]);
    }
    v
}

// -- numbers ----------------------------------------------------------------

#[test]
fn a_number_is_its_source_text() {
    let n = Number::new("1e999");
    assert_eq!(n.text(), "1e999");
    assert_eq!(n.to_string(), "1e999");
    assert!(n.as_f64().is_infinite());
    assert_eq!(n.as_i64(), None);
}

#[test]
fn as_i64_is_exact_where_f64_would_not_be() {
    let n = Number::new("9007199254740993"); // 2^53 + 1
    assert_eq!(n.as_i64(), Some(9_007_199_254_740_993));
    assert_ne!(n.as_f64() as i64, 9_007_199_254_740_993);
}

#[test]
fn is_integer_follows_the_literal_not_the_value() {
    assert!(Number::new("12").is_integer());
    assert!(Number::new("-12").is_integer());
    assert!(!Number::new("12.0").is_integer());
    assert!(!Number::new("1e2").is_integer());
    assert!(!Number::new("1E2").is_integer());
}

#[test]
fn numbers_compare_by_text_because_the_document_does() {
    assert_ne!(Value::number("1"), Value::number("1.0"));
    assert_ne!(Value::number("1e2"), Value::number("100"));
    assert_eq!(Value::number("1"), Value::number("1"));
}

// -- accessors --------------------------------------------------------------

#[test]
fn kinds_and_names() {
    assert_eq!(Value::Null.kind(), Kind::Null);
    assert_eq!(Value::Bool(true).kind(), Kind::Bool);
    assert_eq!(Value::number("1").kind(), Kind::Number);
    assert_eq!(Value::string("s").kind(), Kind::String);
    assert_eq!(Value::Array(vec![]).kind(), Kind::Array);
    assert_eq!(Value::Object(vec![]).kind(), Kind::Object);
    assert_eq!(Kind::Object.name(), "object");
    assert_eq!(Kind::Null.name(), "null");
}

#[test]
fn scalar_accessors_answer_none_for_the_wrong_kind() {
    let v = Value::string("s");
    assert_eq!(v.as_str(), Some("s"));
    assert_eq!(v.as_bool(), None);
    assert!(v.as_number().is_none());
    assert!(v.as_array().is_none());
    assert!(v.as_object().is_none());
    assert!(!v.is_null());
    assert!(!v.is_container());
    assert!(Value::Null.is_null());
    assert!(Value::Array(vec![]).is_container());
}

#[test]
fn len_counts_members_and_a_scalar_has_none() {
    let arr = Value::Array(vec![Value::Null, Value::Null, Value::Null]);
    assert_eq!(arr.len(), 3);
    assert!(!arr.is_empty());
    assert_eq!(Value::Array(vec![]).len(), 0);
    assert!(Value::Array(vec![]).is_empty());
    assert_eq!(Value::number("12345").len(), 0);
    assert!(Value::number("12345").is_empty());
}

#[test]
fn index_reaches_arrays_and_objects_alike() {
    let arr = Value::Array(vec![Value::number("7"), Value::number("8")]);
    assert_eq!(arr.index(1), Some(&Value::number("8")));
    assert_eq!(arr.index(2), None);
    let obj = Value::Object(vec![Member::new("a", Value::number("7"))]);
    assert_eq!(obj.index(0), Some(&Value::number("7")));
    assert_eq!(obj.index(1), None);
    assert_eq!(Value::Null.index(0), None);
}

#[test]
fn get_takes_the_first_duplicate_and_get_all_takes_every_one() {
    let obj = Value::Object(vec![
        Member::new("a", Value::number("1")),
        Member::new("b", Value::number("2")),
        Member::new("a", Value::number("3")),
    ]);
    assert_eq!(obj.get("a"), Some(&Value::number("1")));
    assert_eq!(obj.get("b"), Some(&Value::number("2")));
    assert_eq!(obj.get("zzz"), None);
    assert_eq!(obj.get_all("a").count(), 2);
    assert_eq!(obj.get_all("zzz").count(), 0);
    // A scalar has no members, and asking is not an error.
    assert_eq!(Value::Null.get("a"), None);
    assert_eq!(Value::Null.get_all("a").count(), 0);
}

#[test]
fn take_leaves_a_null_behind() {
    let mut v = Value::Array(vec![Value::number("1")]);
    let got = v.take();
    assert_eq!(got.len(), 1);
    assert_eq!(v, Value::Null);
    assert_eq!(Value::default(), Value::Null);
}

// -- depth ------------------------------------------------------------------

#[test]
fn depth_counts_levels() {
    assert_eq!(Value::Null.depth(), 1);
    assert_eq!(Value::Array(vec![]).depth(), 1);
    assert_eq!(tower(1).depth(), 2);
    assert_eq!(tower(9).depth(), 10);
    // The deepest branch wins, not the last one.
    let mixed = Value::Array(vec![tower(5), Value::Null, tower(2)]);
    assert_eq!(mixed.depth(), 7);
}

// -- the non-recursive traits ----------------------------------------------

#[test]
fn dropping_a_hundred_thousand_deep_tree_does_not_overflow_the_stack() {
    let v = tower(100_000);
    drop(v);
    let v = obj_tower(100_000);
    drop(v);
}

#[test]
fn cloning_a_deep_tree_does_not_overflow_and_copies_it_exactly() {
    let v = tower(50_000);
    let w = v.clone();
    assert_eq!(v, w);
    assert_eq!(w.depth(), 50_001);
}

#[test]
fn comparing_deep_trees_does_not_overflow() {
    assert_eq!(tower(50_000), tower(50_000));
    assert_ne!(tower(50_000), tower(49_999));
    assert_ne!(tower(50_000), obj_tower(50_000));
}

#[test]
fn cloning_copies_structure_and_not_a_shared_reference() {
    let v = Value::Object(vec![
        Member::new("a", Value::Array(vec![Value::number("1"), Value::string("two")])),
        Member::new("a", Value::Bool(false)),
        Member::new("b", Value::Null),
    ]);
    let mut w = v.clone();
    assert_eq!(v, w);
    w.take();
    assert_eq!(w, Value::Null);
    assert_eq!(v.len(), 3);
    assert_eq!(v.to_json(), r#"{"a":[1,"two"],"a":false,"b":null}"#);
}

#[test]
fn equality_is_order_sensitive_for_objects() {
    let ab = Value::Object(vec![
        Member::new("a", Value::number("1")),
        Member::new("b", Value::number("2")),
    ]);
    let ba = Value::Object(vec![
        Member::new("b", Value::number("2")),
        Member::new("a", Value::number("1")),
    ]);
    assert_ne!(ab, ba);
    assert_eq!(ab, ab.clone());
    // Same keys, different values.
    let ab2 = Value::Object(vec![
        Member::new("a", Value::number("1")),
        Member::new("b", Value::number("3")),
    ]);
    assert_ne!(ab, ab2);
}

#[test]
fn equality_across_kinds_is_false_not_a_panic() {
    let cases = [
        Value::Null,
        Value::Bool(false),
        Value::number("0"),
        Value::string(""),
        Value::Array(vec![]),
        Value::Object(vec![]),
    ];
    for (i, a) in cases.iter().enumerate() {
        for (j, b) in cases.iter().enumerate() {
            assert_eq!(a == b, i == j, "{a:?} vs {b:?}");
        }
    }
    assert_ne!(Value::Array(vec![Value::Null]), Value::Array(vec![]));
}

#[test]
fn debug_and_display_are_compact_json_and_do_not_recurse() {
    let v = Value::Object(vec![Member::new("k", Value::Array(vec![Value::Null]))]);
    assert_eq!(format!("{v:?}"), r#"{"k":[null]}"#);
    assert_eq!(format!("{v}"), r#"{"k":[null]}"#);
    // Deep: the derived Debug would have overflowed here.
    assert_eq!(format!("{}", tower(20_000)).len(), 20_000 * 2 + 1);
}

#[test]
fn a_tree_built_by_hand_serialises_the_same_as_one_that_was_parsed() {
    let built = Value::Object(vec![
        Member::new("n", Value::number("1e999")),
        Member::new("s", Value::string("a\"b")),
        Member::new("t", Value::Bool(true)),
    ]);
    let parsed = super::super::parse_str(r#"{"n":1e999,"s":"a\"b","t":true}"#).unwrap();
    assert_eq!(built, parsed);
    assert_eq!(built.to_json(), parsed.to_json());
}
