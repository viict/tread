//! The structural scanner, against byte slices only — no file, no source.
//!
//! Two properties are worth more than the rest and are tested hardest: the walk
//! must agree with the parser about where a member begins and ends (a string
//! full of brackets is data, not structure), and feeding it one byte at a time
//! must give exactly the same answer as feeding it whole, because that is what
//! "resumable" means for the viewport.
#![deny(unsafe_code)]

use super::*;

/// Every member of the container that starts at `at`, fed in `chunk`-byte
/// slices. Returns the members and the container's end, and checks the
/// scanner's own count agrees with what it emitted.
fn scan_at(doc: &[u8], at: usize, chunk: usize) -> (Vec<Member>, Option<u64>) {
    let obj = doc[at] == b'{';
    let mut s = Scan::new(at as u64, obj);
    let mut out = Vec::new();
    while !s.done() {
        let from = s.pos() as usize;
        if from >= doc.len() {
            s.finish(doc.len() as u64, &mut |m| out.push(m));
            break;
        }
        let to = (from + chunk).min(doc.len());
        s.feed(&doc[from..to], &mut |m| out.push(m));
    }
    assert_eq!(s.count(), out.len(), "the scanner counts what it emits");
    (out, s.end())
}

/// The members of the root container as `(key, value)` text.
fn members(doc: &str, chunk: usize) -> Vec<(String, String)> {
    let b = doc.as_bytes();
    let (at, _) = root(b, 0).expect("a root");
    let (ms, _) = scan_at(b, at as usize, chunk);
    ms.iter()
        .map(|m| {
            let key = m
                .key
                .map(|(s, e)| String::from_utf8_lossy(&b[s as usize..e as usize]).into_owned())
                .unwrap_or_default();
            let val = String::from_utf8_lossy(&b[m.start as usize..m.end as usize]).into_owned();
            (key, val)
        })
        .collect()
}

#[test]
fn an_array_yields_its_elements() {
    let got = members("[1, \"two\", true, null, 3.5e2]", 4096);
    let vals: Vec<&str> = got.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(vals, vec!["1", "\"two\"", "true", "null", "3.5e2"]);
    assert!(got.iter().all(|(k, _)| k.is_empty()), "no keys in an array");
}

#[test]
fn an_object_yields_keys_and_values() {
    let got = members("{\"a\": 1, \"b\": {\"c\": [1,2]}, \"a\": 2}", 4096);
    assert_eq!(
        got,
        vec![
            ("\"a\"".to_string(), "1".to_string()),
            ("\"b\"".to_string(), "{\"c\": [1,2]}".to_string()),
            // Duplicate keys are kept, in order, like the value tree.
            ("\"a\"".to_string(), "2".to_string()),
        ]
    );
}

#[test]
fn structure_inside_a_string_is_data() {
    let doc = r#"["a,b", "]", "{\"x\": 1}", "esc\\", "q\"q"]"#;
    let got = members(doc, 4096);
    let vals: Vec<&str> = got.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(
        vals,
        vec![
            "\"a,b\"",
            "\"]\"",
            "\"{\\\"x\\\": 1}\"",
            "\"esc\\\\\"",
            "\"q\\\"q\""
        ]
    );
}

/// A key holding a colon, a brace and an escaped quote still ends where the
/// key ends: getting this wrong swallows the value into the key.
#[test]
fn a_hostile_key_is_still_one_key() {
    let doc = r#"{"a:{}\"b": 7}"#;
    assert_eq!(members(doc, 4096), vec![(r#""a:{}\"b""#.to_string(), "7".to_string())]);
}

#[test]
fn whitespace_around_members_is_not_part_of_them() {
    let doc = "{\n  \"a\"  :   1 ,\n  \"b\" : [ ]\n}";
    assert_eq!(
        members(doc, 4096),
        vec![
            ("\"a\"".to_string(), "1".to_string()),
            ("\"b\"".to_string(), "[ ]".to_string()),
        ]
    );
}

#[test]
fn empty_containers_have_no_members() {
    for doc in ["[]", "{}", "[  ]", "{\n}"] {
        assert!(members(doc, 4096).is_empty(), "{doc}");
    }
}

/// The whole point of the scanner: the answer must not depend on where the
/// chunks fall, including a chunk boundary inside a string, inside an escape
/// and inside a number.
#[test]
fn chunking_never_changes_the_answer() {
    let doc = r#"{"a": [1, {"b": "x\"y"}, "é"], "c": -1.5e-3, "d": {}}"#;
    let whole = members(doc, 4096);
    for chunk in 1..=doc.len() {
        assert_eq!(members(doc, chunk), whole, "chunk size {chunk}");
    }
}

#[test]
fn the_container_end_is_the_byte_past_its_bracket() {
    let doc = b"[1,2] trailing";
    let (ms, end) = scan_at(doc, 0, 3);
    assert_eq!(ms.len(), 2);
    assert_eq!(end, Some(5), "one past the `]`");
}

/// Expanding a node indexes *that node*, which is the same walk one level in.
#[test]
fn a_nested_container_is_indexed_by_its_own_scan() {
    let doc = br#"{"outer": [10, 20, 30]}"#;
    let (top, _) = scan_at(doc, 0, 7);
    assert_eq!(top.len(), 1);
    let inner = top[0];
    let (kids, end) = scan_at(doc, inner.start as usize, 5);
    let vals: Vec<String> = kids
        .iter()
        .map(|m| String::from_utf8_lossy(&doc[m.start as usize..m.end as usize]).into_owned())
        .collect();
    assert_eq!(vals, vec!["10", "20", "30"]);
    assert_eq!(end, Some(inner.end), "the child's own end agrees with its span");
}

/// Ten thousand levels of `[[[[` must cost ten thousand integer increments and
/// no stack. This is the crash the whole design exists to avoid.
#[test]
fn deep_nesting_costs_no_stack() {
    const DEPTH: usize = 10_000;
    let mut doc = vec![b'['; DEPTH];
    doc.extend(std::iter::repeat(b']').take(DEPTH));
    let (ms, end) = scan_at(&doc, 0, 64);
    assert_eq!(ms.len(), 1, "one member: the whole inner nest");
    assert_eq!(end, Some(doc.len() as u64));
    assert_eq!(ms[0].start, 1);
    assert_eq!(ms[0].end, (doc.len() - 1) as u64);
}

#[test]
fn a_truncated_document_keeps_what_it_has() {
    let doc = br#"[1, 2, {"a": 3"#;
    let mut s = Scan::new(0, false);
    let mut got = Vec::new();
    s.feed(doc, &mut |m| got.push(m));
    s.finish(doc.len() as u64, &mut |m| got.push(m));
    assert!(s.truncated(), "the container was cut off, and says so");
    let (ms, end) = scan_at(doc, 0, 4);
    assert_eq!(ms.len(), 3, "the half-written object is still a member");
    assert_eq!(end, Some(doc.len() as u64));
    let last = ms[2];
    assert_eq!(&doc[last.start as usize..last.end as usize], br#"{"a": 3"#);
}

#[test]
fn a_trailing_comma_adds_no_member() {
    let (ms, _) = scan_at(b"[1,2,]", 0, 2);
    assert_eq!(ms.len(), 2);
}

/// Malformed input never disappears: a key with no value is still a row, and
/// its value range is *empty* so the row reports a parse error at that offset.
///
/// It must never borrow the key's own bytes for the value: those are valid
/// JSON, so `{"beta":` would render as `"beta": "beta"` — a value the document
/// does not contain, which is the exact fidelity break the source rules exist
/// to prevent.
#[test]
fn a_key_without_a_value_is_a_member_with_an_empty_value_range() {
    for doc in [&br#"{"a"}"#[..], &br#"{"a":}"#[..], &br#"{"a":"#[..], &br#"{"a""#[..]] {
        let (ms, _) = scan_at(doc, 0, 8);
        assert_eq!(ms.len(), 1, "{}", String::from_utf8_lossy(doc));
        assert_eq!(ms[0].key, Some((1, 4)));
        assert_eq!(ms[0].start, 4, "{}", String::from_utf8_lossy(doc));
        assert_eq!(ms[0].end, 4, "the value range is empty, not the key's bytes");
        assert_eq!(ms[0].len(), 0);
    }
    // The empty range survives the compact store, key and all.
    let mut store = Members::new(0);
    store.push(Member { key: Some((1, 4)), start: 4, end: 4 });
    let back = store.get(0).expect("a member");
    assert_eq!(back.key, Some((1, 4)));
    assert_eq!((back.start, back.end), (4, 4));
}

#[test]
fn the_root_is_found_past_a_bom_and_whitespace() {
    assert_eq!(root(b"\xef\xbb\xbf  [1]", 0), Some((5, Shape::Array)));
    assert_eq!(root(b"\n\t {}", 0), Some((3, Shape::Object)));
    assert_eq!(root(b"  \"hi\"", 0), Some((2, Shape::Str)));
    assert_eq!(root(b"-3", 0), Some((0, Shape::Number)));
    assert_eq!(root(b"true", 0), Some((0, Shape::Bool)));
    assert_eq!(root(b"nope", 0), Some((0, Shape::Null)));
    assert_eq!(root(b"@", 0), Some((0, Shape::Bad)));
    assert_eq!(root(b"   ", 0), None);
    assert_eq!(root(b"", 0), None);
}

#[test]
fn shapes_name_what_a_collapsed_row_counts() {
    assert_eq!(Shape::Object.unit(1), "key");
    assert_eq!(Shape::Object.unit(5), "keys");
    assert_eq!(Shape::Array.unit(1), "item");
    assert_eq!(Shape::Array.unit(0), "items");
    assert_eq!(Shape::Array.brackets(), ("[", "]"));
    assert_eq!(Shape::Object.brackets(), ("{", "}"));
    assert!(Shape::Array.is_container() && Shape::Object.is_container());
    assert!(!Shape::Str.is_container() && !Shape::Bad.is_container());
}

#[test]
fn members_round_trip_through_the_compact_store() {
    let mut ms = Members::new(100);
    let a = Member { key: Some((101, 106)), start: 108, end: 110 };
    let b = Member { key: None, start: 112, end: 120 };
    ms.push(a);
    ms.push(b);
    assert_eq!(ms.len(), 2);
    assert_eq!(ms.get(0), Some(a));
    assert_eq!(ms.get(1), Some(b));
    assert_eq!(ms.get(2), None);
    assert!(!ms.is_empty());
}

/// A container larger than 4GiB cannot be held in `u32` deltas; it promotes and
/// stays exact rather than wrapping every offset past the boundary.
#[test]
fn a_huge_container_promotes_to_wide_offsets() {
    let mut ms = Members::new(0);
    let small = Member { key: None, start: 8, end: 9 };
    let far = Member { key: None, start: 5_000_000_000, end: 5_000_000_009 };
    ms.push(small);
    ms.push(far);
    assert_eq!(ms.get(0), Some(small), "earlier members survive the promotion");
    assert_eq!(ms.get(1), Some(far));
}

#[test]
fn a_member_costs_sixteen_bytes() {
    let mut ms = Members::new(0);
    for i in 0..1024u64 {
        ms.push(Member { key: None, start: i * 4, end: i * 4 + 3 });
    }
    assert!(ms.bytes() <= 1024 * 16 * 2, "{} bytes for 1024 members", ms.bytes());
    assert_eq!(ms.get(1023).map(|m| m.start), Some(1023 * 4));
}
