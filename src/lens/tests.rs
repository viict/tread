//! The seam itself: the registry a `--lens` name is resolved against, and the
//! two text helpers every dialect leans on.
#![deny(unsafe_code)]

use super::*;

#[test]
fn the_registry_answers_by_name() {
    assert!(exists("agent"));
    assert!(!exists("opencode"), "not implemented yet, so not offered");
    assert!(!exists(""));
    assert_eq!(find("agent").map(|l| l.name()), Some("agent"));
    assert!(find("nope").is_none());
    assert!(names().contains(&"agent"));
}

/// Every lens in the table can be built, names itself the same way it is
/// registered, and says what it is. The check that keeps a new dialect honest.
#[test]
fn every_lens_agrees_with_its_entry() {
    for name in names() {
        let lens = find(name).expect("registered lens builds");
        assert_eq!(lens.name(), name);
        assert!(!lens.about().is_empty(), "{name} says nothing about itself");
        assert!(list_text().contains(name));
        assert!(list_text().contains(lens.about()));
    }
}

/// A dialect declares where its records live, and the default is the one every
/// dialect written before documents were readable still means: a record per
/// line. Getting this wrong routes `--lens` at the wrong reader, so it is
/// pinned for the default *and* for a dialect that overrides it.
#[test]
fn a_dialect_says_where_its_records_are_and_lines_is_the_default() {
    struct Quiet;
    impl Lens for Quiet {
        fn name(&self) -> &'static str {
            "quiet"
        }
        fn about(&self) -> &'static str {
            "a dialect that says nothing about where it lives"
        }
        fn read(&mut self, _: &Value) -> Option<Summary> {
            None
        }
    }
    assert_eq!(Quiet.records_at(), RecordsAt::Lines);

    struct Rooted;
    impl Lens for Rooted {
        fn name(&self) -> &'static str {
            "rooted"
        }
        fn about(&self) -> &'static str {
            "a dialect whose records are a document's root array"
        }
        fn records_at(&self) -> RecordsAt {
            RecordsAt::Root
        }
        fn read(&mut self, _: &Value) -> Option<Summary> {
            None
        }
    }
    assert_eq!(Rooted.records_at(), RecordsAt::Root);

    // And the registered ones, which is what routing actually asks.
    assert_eq!(records_at("agent"), Some(RecordsAt::Lines));
    assert_eq!(records_at("atif"), Some(RecordsAt::Member("steps")));
    assert_eq!(records_at("nope"), None);
}

/// Two lenses built from the same entry are independent: a lens carries state
/// across records, so sharing one between two open files would cross them.
#[test]
fn each_open_gets_its_own_lens() {
    let a = find("agent").expect("agent");
    let b = find("agent").expect("agent");
    assert_eq!(a.name(), b.name());
    // Distinct allocations, which is what `Make` being a fn pointer buys.
    assert!(!std::ptr::eq(&*a as *const dyn Lens, &*b as *const dyn Lens));
}

#[test]
fn an_excerpt_is_one_line_of_collapsed_whitespace() {
    assert_eq!(excerpt("hello", 40), "hello");
    assert_eq!(excerpt("  hello   world \n", 40), "hello world");
    assert_eq!(excerpt("first\nsecond\nthird", 40), "first second third");
    assert_eq!(excerpt("", 40), "");
    assert_eq!(excerpt("   \n\t ", 40), "");
}

#[test]
fn an_excerpt_too_wide_is_cut_with_an_ellipsis() {
    let cut = excerpt("abcdefghij", 5);
    assert_eq!(cut, "abcd\u{2026}");
    assert_eq!(crate::render::str_width(&cut), 5);
    // Wide characters are counted in columns, not bytes.
    let wide = excerpt("\u{4f60}\u{597d}\u{4e16}\u{754c}", 5);
    assert!(crate::render::str_width(&wide) <= 5, "{wide}");
}

/// A control character in a record must not reach the row as itself; the
/// excerpt drops the whitespace ones and the painter sanitises the rest.
#[test]
fn an_excerpt_carries_no_line_breaks() {
    let text = excerpt("a\r\nb\tc", 40);
    assert!(!text.contains('\n') && !text.contains('\r') && !text.contains('\t'));
    assert_eq!(text, "a b c");
}

#[test]
fn a_clock_is_the_hour_and_minute_of_an_iso_timestamp() {
    assert_eq!(clock("2026-08-05T21:28:58.659Z").as_deref(), Some("21:28"));
    assert_eq!(clock("2026-08-05T00:00:00Z").as_deref(), Some("00:00"));
    // Anything not shaped like a timestamp is refused rather than sliced.
    for bad in ["", "2026-08-05", "yesterday", "2026-08-05 21:28:58", "xxxxxxxxxxTxx:xx"] {
        assert!(clock(bad).is_none(), "{bad} should not be a clock");
    }
}

/// The timestamp comes out of the log, so it is arbitrary text. A multi-byte
/// character straddling one of the offsets this reads must be *refused*, not
/// sliced: `&ts[14..16]` through the middle of a `€` is a panic, and one
/// malformed timestamp anywhere in a multi-GB log would kill the reader on the
/// record that reached the viewport.
#[test]
fn a_timestamp_with_a_multibyte_character_is_refused_rather_than_split() {
    for bad in [
        "2026-08-05T21:\u{20ac}z",     // 3 bytes at 14..17
        "2026-08-05T2\u{20ac}:28:58Z", // straddles 12..15, so 13 is not a ':'
        "2026-08-05T21:2\u{1f600}",    // 4 bytes at 15..19
        "2026-08-05T\u{20ac}1:28Z",
        "\u{1f600}\u{1f600}\u{1f600}\u{1f600}T21:28Z",
    ] {
        assert!(clock(bad).is_none(), "{bad:?} should not be a clock");
    }
    // And a real clock followed by multi-byte text still reads.
    assert_eq!(clock("2026-08-05T21:28\u{20ac}").as_deref(), Some("21:28"));
}

#[test]
fn record_helpers_read_only_what_is_there() {
    let v = crate::json::parse(br#"{"type":"user","timestamp":"2026-08-05T21:28:58.659Z"}"#)
        .expect("parse");
    assert_eq!(record_type(&v), Some("user"));
    assert_eq!(record_clock(&v).as_deref(), Some("21:28"));

    let bare = crate::json::parse(br#"{"type":7}"#).expect("parse");
    assert_eq!(record_type(&bare), None, "a non-string type is not a type");
    assert_eq!(record_clock(&bare), None);

    let array = crate::json::parse(b"[1,2]").expect("parse");
    assert_eq!(record_type(&array), None);
}
