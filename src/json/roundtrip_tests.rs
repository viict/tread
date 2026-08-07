//! The agreement between the parser and the serialiser.
//!
//! Neither module owns it: the parser could be right about a document the
//! serialiser then writes back wrong, and a reader whose `Y` yanks something
//! that does not re-parse is worse than one that refuses. So the two laws are
//! tested here, over a generated corpus rather than a handful of literals:
//!
//! 1. **text → value → text** is the identity for already-compact input.
//! 2. **value → text → value** is the identity for any value the parser built.
//!
//! The corpus is deterministic — a small xorshift, no `rand` crate — so a
//! failure is reproducible from the seed printed in the assertion.

use super::*;

/// Compact JSON in, the same bytes out.
fn same(src: &str) {
    let v = parse_str(src).unwrap_or_else(|e| panic!("{src:?}: {e}"));
    assert_eq!(v.to_json(), src, "text -> value -> text");
    let again = parse_str(&v.to_json()).expect("re-parse");
    assert_eq!(again, v, "value -> text -> value");
}

#[test]
fn compact_documents_round_trip_byte_for_byte() {
    let cases = [
        "null",
        "true",
        "[]",
        "{}",
        "0",
        "-0",
        "1e999",
        "0.1",
        "123456789012345678901234567890123456789012",
        r#""""#,
        r#""plain""#,
        r#""a\"b\\c\nd\te""#,
        r#""\u0000\u001f""#,
        r#"[1,2,[3,[4,[5]]]]"#,
        r#"{"a":1,"a":2,"b":{"c":[true,false,null]}}"#,
        r#"{"":[]}"#,
        r#"["é中😀"]"#,
    ];
    for src in cases {
        same(src);
    }
}

#[test]
fn escapes_normalise_to_the_shortest_form_that_means_the_same_thing() {
    // Not byte-identical, deliberately: `A` is `A`, and a reader copying a
    // subtree should get the character rather than the escape. What must hold
    // is that the *value* survives.
    let v = parse_str(r#"["A\/é😀"]"#).unwrap();
    assert_eq!(v.to_json(), "[\"A/é😀\"]");
    assert_eq!(parse_str(&v.to_json()).unwrap(), v);
}

/// A deterministic 32-bit xorshift, so the corpus below is the same on every
/// machine and every run.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// Build a pseudo-random compact JSON document, iteratively (a recursive
/// generator would cap the depth this can reach at whatever the test stack
/// allows, which is exactly the thing under test).
fn document(rng: &mut Rng, size: usize) -> String {
    let mut out = String::new();
    // Each entry is the closing bracket owed, and whether a comma is owed.
    let mut open: Vec<(char, bool)> = Vec::new();
    let mut emitted = 0usize;
    loop {
        if let Some((_, seen)) = open.last_mut() {
            if *seen {
                out.push(',');
            }
            *seen = true;
            if open.last().map(|f| f.0) == Some('}') {
                out.push_str(&format!("\"k{}\":", rng.below(4)));
            }
        }
        let container = emitted < size && open.len() < 60 && rng.below(3) != 0;
        if container {
            let close = if rng.below(2) == 0 { ']' } else { '}' };
            out.push(if close == ']' { '[' } else { '{' });
            open.push((close, false));
        } else {
            out.push_str(scalar(rng));
        }
        emitted += 1;
        // Close containers at random, and always once the budget is spent.
        while let Some(&(close, _)) = open.last() {
            if emitted < size && rng.below(4) != 0 {
                break;
            }
            out.push(close);
            open.pop();
        }
        if open.is_empty() {
            return out;
        }
    }
}

fn scalar(rng: &mut Rng) -> &'static str {
    const POOL: [&str; 12] = [
        "null",
        "true",
        "false",
        "0",
        "-0",
        "1e999",
        "0.1",
        "-12345678901234567890",
        r#""""#,
        r#""x""#,
        r#""\u0000\u001f""#,
        r#""é中😀""#,
    ];
    POOL[rng.below(POOL.len() as u32) as usize]
}

#[test]
fn generated_documents_round_trip() {
    let mut rng = Rng(0x1234_5678);
    for seed in 0..300u32 {
        let src = document(&mut rng, 40);
        let v = parse_str(&src).unwrap_or_else(|e| panic!("seed {seed}: {src}\n{e}"));
        assert_eq!(v.to_json(), src, "seed {seed}");
        assert_eq!(parse_str(&v.to_json()).unwrap(), v, "seed {seed}");
        assert_eq!(v.clone(), v, "seed {seed}");
    }
}

#[test]
fn truncating_a_generated_document_anywhere_errors_but_never_panics() {
    let mut rng = Rng(0x9e37_79b9);
    for _ in 0..40 {
        let src = document(&mut rng, 30);
        let bytes = src.as_bytes();
        // A container document has no proper prefix that is itself a whole
        // value, so every cut must be an error; a generated bare scalar
        // (`12` cut to `1`) does, so it is only checked for not panicking.
        let container = bytes[0] == b'[' || bytes[0] == b'{';
        for cut in 0..bytes.len() {
            let got = parse(&bytes[..cut]);
            let shown = String::from_utf8_lossy(&bytes[..cut]);
            assert!(got.is_err() || !container, "{shown:?} should be incomplete");
        }
        // ...and mutating one byte never panics either.
        for at in 0..bytes.len() {
            for b in [b'"', b'\\', b'{', b'}', 0u8, 0xff, b'e'] {
                let mut broken = bytes.to_vec();
                broken[at] = b;
                let _ = parse(&broken);
            }
        }
    }
}

#[test]
fn a_generated_document_that_is_deep_survives_the_whole_pipeline() {
    // Parse, clone, compare, serialise, re-parse, drop — every walk in the
    // crate, against something no recursive implementation would survive.
    let src = format!("{}1{}", "[".repeat(9_000), "]".repeat(9_000));
    let v = parse_str(&src).unwrap();
    let w = v.clone();
    assert_eq!(v, w);
    let text = w.to_json();
    assert_eq!(text, src);
    assert_eq!(parse_str(&text).unwrap(), v);
}
