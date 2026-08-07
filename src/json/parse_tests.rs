//! Tests for the RFC 8259 parser.
//!
//! Three things here are worth more than any individual case: that deep
//! nesting is bounded rather than fatal (`deep_*` — the crash this module
//! exists to prevent, including the *drop* of what it built), that every
//! malformed shape produces an `Error` with a plausible offset rather than a
//! panic (`malformed_*`, plus the byte-prefix sweep), and that numbers come
//! back as the text the document wrote (`number_*`).

use super::*;
use crate::json::{Member, Value};

/// `pub(super)` so the sibling test files (`depth_tests`, `stream_tests`)
/// share one definition rather than three.
pub(super) fn ok(src: &str) -> Value {
    match parse_str(src) {
        Ok(v) => v,
        Err(e) => panic!("{src:?} should parse: {e}"),
    }
}

pub(super) fn err(src: &str) -> Error {
    match parse_str(src) {
        Ok(v) => panic!("{src:?} should not parse, got {v}"),
        Err(e) => e,
    }
}

/// The compact re-serialisation, which is the terse way to assert a shape.
fn json(src: &str) -> String {
    ok(src).to_json()
}

fn arr(items: Vec<Value>) -> Value {
    Value::Array(items)
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| Member::new(k, v)).collect())
}

// -- the grammar ------------------------------------------------------------

#[test]
fn the_three_literals() {
    assert_eq!(ok("true"), Value::Bool(true));
    assert_eq!(ok("false"), Value::Bool(false));
    assert_eq!(ok("null"), Value::Null);
}

#[test]
fn a_top_level_scalar_is_a_document() {
    // RFC 8259 §2: any value, not just an object or array.
    assert_eq!(ok("5"), Value::number("5"));
    assert_eq!(ok("\"x\""), Value::string("x"));
    assert_eq!(ok(" \t\r\n null \n"), Value::Null);
}

#[test]
fn empty_containers() {
    assert_eq!(ok("[]"), arr(vec![]));
    assert_eq!(ok("{}"), obj(vec![]));
    assert_eq!(ok("[ ]"), arr(vec![]));
    assert_eq!(ok("{\n}"), obj(vec![]));
    assert_eq!(ok("[[],{}]"), arr(vec![arr(vec![]), obj(vec![])]));
}

#[test]
fn nested_containers_keep_their_shape() {
    assert_eq!(json(r#"{"a":[1,{"b":[true,null]}]}"#), r#"{"a":[1,{"b":[true,null]}]}"#);
    assert_eq!(json("[[[[1]]]]"), "[[[[1]]]]");
}

#[test]
fn whitespace_is_allowed_everywhere_between_tokens() {
    let spaced = " { \"a\" : [ 1 , 2 ] , \"b\" : null } ";
    assert_eq!(json(spaced), r#"{"a":[1,2],"b":null}"#);
    assert_eq!(json("[\n\t1,\r\n2\n]"), "[1,2]");
}

#[test]
fn only_the_four_rfc_whitespace_bytes_are_whitespace() {
    // A vertical tab or a form feed is not JSON whitespace.
    assert_eq!(err("[1,\x0b2]").reason, Reason::Unexpected(0x0b));
    assert_eq!(err("\x0c1").reason, Reason::Unexpected(0x0c));
}

#[test]
fn empty_input_is_an_error_not_a_null() {
    assert_eq!(err("").reason, Reason::Eof);
    assert_eq!(err("").offset, 0);
    assert_eq!(err("   ").reason, Reason::Eof);
    assert_eq!(err("   ").offset, 3);
}

// -- numbers ----------------------------------------------------------------

#[test]
fn numbers_keep_their_source_text() {
    for text in ["0", "-0", "1", "-1", "1.0", "0.1", "1e2", "1E2", "1e+2", "1e-2", "-1.5E+10"] {
        let v = ok(text);
        assert_eq!(v.as_number().unwrap().text(), text, "{text}");
        assert_eq!(v.to_json(), text, "{text}");
    }
}

#[test]
fn numbers_that_f64_cannot_hold_still_display_as_written() {
    let big = "12345678901234567890123456789012345678901234567890";
    assert_eq!(ok(big).as_number().unwrap().text(), big);
    assert_eq!(ok("1e999").as_number().unwrap().text(), "1e999");
    assert_eq!(ok("0.1").as_number().unwrap().text(), "0.1");
    // ...and the lossy accessor is the only place the loss shows.
    assert!(ok("1e999").as_number().unwrap().as_f64().is_infinite());
    assert_eq!(ok("0.30000000000000004").as_number().unwrap().as_f64(), 0.1 + 0.2);
    assert_ne!(ok(big).as_number().unwrap().as_f64().to_string(), big);
}

#[test]
fn a_document_of_precise_numbers_round_trips_byte_for_byte() {
    let src = "[1e999,-1e-999,0.1,3.141592653589793238462643383279,10000000000000000001]";
    assert_eq!(json(src), src);
}

#[test]
fn malformed_numbers_are_refused() {
    for src in ["01", "-01", "+1", ".5", "1.", "1.e2", "1e", "1e+", "1E-", "-", "1.2.3", "0x10"] {
        let e = err(src);
        assert!(
            matches!(e.reason, Reason::BadNumber | Reason::Eof | Reason::Unexpected(_)),
            "{src:?} gave {e}"
        );
    }
}

#[test]
fn nan_and_infinity_are_not_json() {
    assert!(matches!(err("NaN").reason, Reason::Unexpected(b'N')));
    assert!(matches!(err("Infinity").reason, Reason::Unexpected(b'I')));
    assert!(matches!(err("-Infinity").reason, Reason::BadNumber));
}

#[test]
fn a_number_inside_a_container_is_bounded_by_its_delimiters() {
    assert_eq!(json("[1,2,3]"), "[1,2,3]");
    assert_eq!(json(r#"{"a":1}"#), r#"{"a":1}"#);
    assert_eq!(err("[01]").reason, Reason::BadNumber);
}

// -- strings ----------------------------------------------------------------

#[test]
fn every_two_character_escape() {
    let v = ok(r#""\" \\ \/ \b \f \n \r \t""#);
    assert_eq!(v.as_str().unwrap(), "\" \\ / \u{8} \u{c} \n \r \t");
}

#[test]
fn unicode_escapes_decode() {
    assert_eq!(ok(r#""\u0041""#).as_str().unwrap(), "A");
    assert_eq!(ok(r#""\u00e9""#).as_str().unwrap(), "é");
    assert_eq!(ok(r#""\u4e2d\u6587""#).as_str().unwrap(), "中文");
    assert_eq!(ok(r#""\u0000""#).as_str().unwrap(), "\u{0}");
    // Case-insensitive hex.
    assert_eq!(ok(r#""\uABCD""#).as_str().unwrap(), ok(r#""\uabcd""#).as_str().unwrap());
}

#[test]
fn surrogate_pairs_become_one_scalar() {
    assert_eq!(ok(r#""\ud83d\ude00""#).as_str().unwrap(), "😀");
    assert_eq!(ok(r#""\uD83D\uDE00""#).as_str().unwrap(), "😀");
    assert_eq!(ok(r#""\ud834\udd1e""#).as_str().unwrap(), "\u{1d11e}");
    assert_eq!(ok(r#""a\ud83d\ude00b""#).as_str().unwrap(), "a😀b");
}

#[test]
fn a_lone_surrogate_is_replaced_not_a_panic_and_not_a_wrong_character() {
    assert_eq!(ok(r#""\ud83d""#).as_str().unwrap(), "\u{fffd}");
    assert_eq!(ok(r#""\udead""#).as_str().unwrap(), "\u{fffd}");
    // A high surrogate followed by something that is not its pair: the
    // surrogate is replaced and the follower is still read as itself.
    assert_eq!(ok(r#""\ud83dA""#).as_str().unwrap(), "\u{fffd}A");
    assert_eq!(ok(r#""\ud83d\u0041""#).as_str().unwrap(), "\u{fffd}A");
    assert_eq!(ok(r#""\ud83d\ud83d\ude00""#).as_str().unwrap(), "\u{fffd}😀");
    // Low then high: two unpaired surrogates, two replacements.
    assert_eq!(ok(r#""\ude00\ud83d""#).as_str().unwrap(), "\u{fffd}\u{fffd}");
}

#[test]
fn a_truncated_unicode_escape_is_an_error() {
    assert_eq!(err(r#""\u12""#).reason, Reason::BadHex);
    assert_eq!(err(r#""\uZZZZ""#).reason, Reason::BadHex);
    assert_eq!(err(r#""\u"#).reason, Reason::Eof);
    // The offset points at the offending digit, not the backslash.
    assert_eq!(err(r#""\u12g4""#).offset, 5);
}

#[test]
fn an_unknown_escape_is_an_error_at_the_escape() {
    let e = err(r#""a\qb""#);
    assert_eq!(e.reason, Reason::BadEscape(b'q'));
    assert_eq!(e.offset, 3);
    assert_eq!(e.to_string(), "invalid escape \\q at byte 3");
    assert_eq!(err(r#""\x41""#).reason, Reason::BadEscape(b'x'));
}

#[test]
fn raw_control_characters_in_a_string_are_refused() {
    assert_eq!(err("\"a\nb\"").reason, Reason::Control(b'\n'));
    assert_eq!(err("\"a\tb\"").reason, Reason::Control(b'\t'));
    assert_eq!(err("\"a\0b\"").reason, Reason::Control(0));
    assert_eq!(err("\"a\0b\"").offset, 2);
    // DEL is not a C0 control and RFC 8259 allows it unescaped.
    assert_eq!(ok("\"a\u{7f}b\"").as_str().unwrap(), "a\u{7f}b");
}

#[test]
fn an_unterminated_string_is_an_error_at_end_of_input() {
    let e = err(r#""abc"#);
    assert_eq!(e.reason, Reason::Eof);
    assert_eq!(e.offset, 4);
    assert_eq!(err(r#""abc\"#).reason, Reason::Eof);
    assert_eq!(err(r#"["a"#).reason, Reason::Eof);
}

#[test]
fn invalid_utf8_inside_a_string_is_replaced_not_rejected() {
    let mut bytes = b"\"a".to_vec();
    bytes.extend_from_slice(&[0xff, 0xfe]);
    bytes.extend_from_slice(b"b\"");
    let v = parse(&bytes).expect("lossy, never refused");
    assert_eq!(v.as_str().unwrap(), "a\u{fffd}\u{fffd}b");
}

#[test]
fn a_truncated_utf8_sequence_is_replaced() {
    // The lead byte of a three-byte scalar with its continuation bytes cut off.
    let bytes = b"[\"\xe4\xb8\", \"ok\"]".to_vec();
    let v = parse(&bytes).expect("lossy");
    assert_eq!(v.index(0).unwrap().as_str().unwrap(), "\u{fffd}");
    assert_eq!(v.index(1).unwrap().as_str().unwrap(), "ok");
}

#[test]
fn a_huge_string_parses() {
    let big = "x".repeat(2_000_000);
    let src = format!("[\"{big}\"]");
    let v = ok(&src);
    assert_eq!(v.index(0).unwrap().as_str().unwrap().len(), 2_000_000);
}

#[test]
fn a_huge_string_of_escapes_parses() {
    let src = format!("\"{}\"", r"\n".repeat(200_000));
    assert_eq!(ok(&src).as_str().unwrap().len(), 200_000);
}

// -- objects ----------------------------------------------------------------

#[test]
fn duplicate_keys_are_kept_in_document_order() {
    let v = ok(r#"{"a":1,"b":2,"a":3,"a":4}"#);
    let members = v.as_object().unwrap();
    assert_eq!(members.len(), 4);
    let keys: Vec<&str> = members.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, ["a", "b", "a", "a"]);
    assert_eq!(v.get("a"), Some(&Value::number("1")));
    let all: Vec<String> = v.get_all("a").map(|x| x.to_json()).collect();
    assert_eq!(all, ["1", "3", "4"]);
    // ...and writing it back keeps every one of them.
    assert_eq!(v.to_json(), r#"{"a":1,"b":2,"a":3,"a":4}"#);
}

#[test]
fn keys_may_be_empty_or_escaped_or_duplicated_after_unescaping() {
    let v = ok(r#"{"":1,"\u0061":2,"a":3}"#);
    let keys: Vec<&str> = v.as_object().unwrap().iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, ["", "a", "a"]);
}

#[test]
fn an_unquoted_key_is_refused() {
    assert_eq!(err("{a:1}").reason, Reason::Unexpected(b'a'));
    assert_eq!(err("{'a':1}").reason, Reason::Unexpected(b'\''));
    assert_eq!(err("{1:2}").reason, Reason::Unexpected(b'1'));
}

#[test]
fn a_missing_colon_or_value_is_refused() {
    assert_eq!(err(r#"{"a" 1}"#).reason, Reason::Unexpected(b'1'));
    assert_eq!(err(r#"{"a":}"#).reason, Reason::Unexpected(b'}'));
    assert_eq!(err(r#"{"a"}"#).reason, Reason::Unexpected(b'}'));
    assert_eq!(err(r#"{"a":1"#).reason, Reason::Eof);
}

// -- malformed input --------------------------------------------------------

#[test]
fn trailing_commas_are_refused() {
    assert_eq!(err("[1,]").reason, Reason::Unexpected(b']'));
    assert_eq!(err(r#"{"a":1,}"#).reason, Reason::Unexpected(b'}'));
    assert_eq!(err("[,]").reason, Reason::Unexpected(b','));
    assert_eq!(err("[1,,2]").reason, Reason::Unexpected(b','));
}

#[test]
fn mismatched_brackets_are_refused_with_the_offset_of_the_wrong_one() {
    let e = err("[1,2}");
    assert_eq!(e.reason, Reason::Unexpected(b'}'));
    assert_eq!(e.offset, 4);
    assert_eq!(e.to_string(), "unexpected } at byte 4");
    assert_eq!(err(r#"{"a":1]"#).reason, Reason::Unexpected(b']'));
}

#[test]
fn trailing_data_after_a_complete_value_is_refused() {
    let e = err("{} {}");
    assert_eq!(e.reason, Reason::Trailing(b'{'));
    assert_eq!(e.offset, 3);
    assert_eq!(err("1 2").reason, Reason::Trailing(b'2'));
    assert_eq!(err("truex").reason, Reason::Trailing(b'x'));
}

#[test]
fn a_broken_literal_names_what_was_expected() {
    assert_eq!(err("tru").reason, Reason::BadLiteral("true"));
    assert_eq!(err("[nul]").reason, Reason::BadLiteral("null"));
    assert_eq!(err("[fals]").reason, Reason::BadLiteral("false"));
    assert_eq!(err("tru").to_string(), "expected `true` at byte 0");
}

#[test]
fn nul_bytes_outside_a_string_are_reported_in_hex() {
    let e = parse(b"[\0]").unwrap_err();
    assert_eq!(e.reason, Reason::Unexpected(0));
    assert_eq!(e.to_string(), "unexpected byte 0x00 at byte 1");
    // A NUL after a complete document is trailing data, also in hex.
    assert_eq!(parse(b"{}\0").unwrap_err().to_string(), "trailing byte 0x00 after the value at byte 2");
}

#[test]
fn a_bom_is_not_json() {
    // Unlike CSV, RFC 8259 §8.1 forbids a byte-order mark; refusing it with an
    // offset is more honest than silently eating bytes the document has.
    let e = parse("\u{feff}{}".as_bytes()).unwrap_err();
    assert_eq!(e.reason, Reason::Unexpected(0xef));
    assert_eq!(e.offset, 0);
}

#[test]
fn every_byte_prefix_of_a_real_looking_document_either_parses_or_errors() {
    let src = r#"{"a":[1,-2.5e3,"s\u00e9\n",true,false,null,{"b":{},"c":[]}],"d":"\ud83d\ude00"}"#;
    let bytes = src.as_bytes();
    for cut in 0..=bytes.len() {
        // No panic, and only the complete document parses.
        let got = parse(&bytes[..cut]);
        assert_eq!(got.is_ok(), cut == bytes.len(), "prefix of {cut} bytes: {got:?}");
        if let Err(e) = got {
            assert!(e.offset <= cut, "offset {} past input {cut}", e.offset);
        }
    }
}

#[test]
fn every_single_byte_input_is_handled() {
    for b in 0u8..=255 {
        let got = parse(&[b]);
        // A single digit is the only one-byte JSON document there is.
        assert_eq!(got.is_ok(), b.is_ascii_digit(), "byte {b:#04x} gave {got:?}");
    }
}

#[test]
fn garbage_of_every_shape_errors_without_panicking() {
    let cases = [
        "[", "]", "{", "}", ":", ",", "\"", "\\", "[}", "{]", "[[[", "}}}", "[\"a\":1]",
        "{\"a\",1}", "[1 2]", "--1", "[-]", "\"\\u{41}\"", "{}{}", "nulll", "[true,]", "'x'",
        "/*c*/1", "1//c", "[1,2", "{\"a\":[}", "\u{feff}", "\u{0}\u{0}\u{0}",
    ];
    for src in cases {
        let e = err(src);
        assert!(e.offset <= src.len(), "{src:?}: offset {} past input", e.offset);
        assert!(!e.to_string().is_empty());
    }
}
