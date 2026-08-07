//! Adversarial JSON and JSONL, run as part of `cargo test`.
//!
//! The sibling of `tests/robustness.rs`, for the JSON half. Everything here
//! drives the real binary, so it covers the detector, the lazy index, the row
//! grammar and the dump path together.
//!
//! What lives here is what is *fast and deterministic*. The cases that need a
//! 50MB string, a million-element array or a hundred thousand levels of nesting
//! are tooling — `tools/jsongen.py` writes them and `tools/soak_json.sh` runs
//! them — because a test suite that takes a minute stops being run.

mod harness;

use harness::{render, render_stdin, strip, temp_doc_ext};

const WIDTHS: [&str; 4] = ["1", "20", "80", "200"];

/// The shared presentation depth both JSON sources stop opening at
/// (`src/source/jsonrow.rs`). Spelled out rather than imported: these tests
/// drive the real binary and are a check on the number, not a restatement of
/// it.
const CAP: usize = 256;

/// Rows a document nested past [`CAP`] renders as: a bracket row and a closing
/// row for the root and each of the `CAP` levels under it, plus the one note
/// row that stands for everything deeper.
const CAP_ROWS: usize = (CAP + 1) * 2 + 1;

/// Render at every width, assert nothing leaks, and return the width-100 text.
///
/// A hostile document may render as *anything*; what it may never do is panic,
/// exit non-zero, hang, or put a control byte on a pipe. `render` already fails
/// the first two.
fn survives(name: &str, ext: &str, body: &[u8]) -> String {
    let path = temp_doc_ext(name, ext, body);
    for w in WIDTHS {
        let out = render(&path, &["--width", w, "--plain"]);
        assert!(
            !out.contains('\u{1b}'),
            "{name}@{w}: escape leaked into a piped render"
        );
        for c in out.chars() {
            assert!(
                c == '\n' || c == '\t' || !c.is_control(),
                "{name}@{w}: raw control {c:?} reached the output"
            );
        }
        assert_eq!(strip(&out), out, "{name}@{w}: stripping changed a plain render");
    }
    // `--toc` walks the root by a different route and must survive the same
    // inputs, as must the same bytes arriving with no file name to go on.
    render(&path, &["--toc"]);
    render_stdin(body, &["--width", "100", "--plain", "--format", ext]);
    render(&path, &["--width", "100", "--plain"])
}

/// One render, at one width. The deep cases are the slow ones and their point
/// is the row *count*, not the layout: rendering them at five widths would cost
/// a minute of `cargo test` to assert the same thing five times.
fn once(name: &str, ext: &str, body: &[u8]) -> String {
    let path = temp_doc_ext(name, ext, body);
    let out = render(&path, &["--width", "100", "--plain"]);
    assert!(!out.contains('\u{1b}'), "{name}: escape leaked");
    out
}

// -- malformed documents ----------------------------------------------------

/// Every way of not being JSON, in one pass. None of these may stop the file:
/// a member that does not parse says why and where, and its siblings still
/// render (SPEC.md §JSON — "half a document is still worth reading").
#[test]
fn every_malformed_form_renders_a_reason_and_keeps_the_rest() {
    let cases: &[(&str, &[u8], &str)] = &[
        ("truncated_string", br#"{"a":1,"b":"never clos"#, "a"),
        ("trailing_comma", br#"{"a":[1,2,3,],"b":2}"#, "b"),
        ("unquoted_key", br#"{a:1,"b":2}"#, "b"),
        ("single_quotes", br#"{'a':'b',"c":3}"#, "c"),
        ("nan", br#"{"a":NaN,"b":2}"#, "b"),
        ("infinity", br#"{"a":Infinity,"b":2}"#, "b"),
        ("neg_infinity", br#"{"a":-Infinity,"b":2}"#, "b"),
        ("bare_comma", br#"{"a":1,,"b":2}"#, "b"),
        ("two_values", br#"{"a":1}{"b":2}"#, "a"),
        ("close_only", b"]}", ""),
    ];
    for (name, body, must_show) in cases {
        let out = survives(name, "json", body);
        assert!(
            out.contains(must_show),
            "{name}: lost the rest of the document: {out:?}"
        );
    }
}

/// A NaN or an unquoted key is a *member* that does not parse, and the row says
/// so by reason and byte offset rather than going blank.
#[test]
fn a_member_that_does_not_parse_names_its_reason_and_its_offset() {
    let out = survives("reasons", "json", br#"{"a":NaN,"b":"x\qy","c":1}"#);
    assert!(out.contains("not JSON"), "{out}");
    assert!(out.contains("at byte"), "no offset given: {out}");
    assert!(out.contains("\"c\": 1"), "the good member still renders: {out}");
}

/// Duplicate keys are kept, in order (SPEC.md §JSON, "Values").
#[test]
fn duplicate_keys_are_all_kept_in_order() {
    let out = survives("dups", "json", br#"{"a":1,"a":2,"a":3}"#);
    let rows: Vec<&str> = out.lines().filter(|r| r.contains("\"a\"")).collect();
    assert_eq!(rows.len(), 3, "all three kept: {out}");
    assert!(rows[0].contains('1') && rows[1].contains('2') && rows[2].contains('3'));
}

/// A number keeps its source text: no round trip through `f64`.
#[test]
fn numbers_keep_the_text_the_document_wrote() {
    let out = survives(
        "numbers",
        "json",
        br#"[1e999,0.1,-0,12345678901234567890123456789012345678901234]"#,
    );
    for want in ["1e999", "0.1", "-0", "12345678901234567890123456789012345678901234"] {
        assert!(out.contains(want), "lost {want}: {out}");
    }
}

// -- hostile bytes ----------------------------------------------------------

/// A lone surrogate names no character, so it becomes U+FFFD rather than being
/// refused or smuggled through as an unpaired code unit. A real pair still
/// decodes.
#[test]
fn lone_surrogates_become_replacement_characters_and_pairs_still_decode() {
    let out = survives(
        "surrogates",
        "json",
        r#"{"hi":"\uD800","lo":"\uDC00","half":"a\uD800b","pair":"😀"}"#.as_bytes(),
    );
    assert_eq!(out.matches('\u{fffd}').count(), 3, "{out}");
    assert!(out.contains('\u{1f600}'), "a real pair still decodes: {out}");
}

/// Invalid UTF-8 in the file is replaced, never rejected and never passed
/// through: a render must be valid UTF-8 by construction (the harness asserts
/// that when it decodes stdout).
#[test]
fn invalid_utf8_is_replaced_not_rejected() {
    let out = survives("badutf8", "json", b"{\"a\":\"\xff\xfe\",\"b\":\"caf\xe9\",\"c\":1}");
    assert!(out.contains('\u{fffd}'), "{out}");
    assert!(out.contains("\"c\": 1"));
}

/// A NUL inside a string is an unescaped control character, which RFC 8259
/// forbids: the member says so and the file continues.
#[test]
fn nul_bytes_do_not_reach_the_output() {
    let out = survives("nuls", "json", b"{\"a\":\"x\x00y\",\"c\":1}");
    assert!(!out.contains('\0'));
    assert!(out.contains("\"c\": 1"), "{out}");
}

/// A BOM in front of the root is consumed, not read as part of the document.
#[test]
fn a_bom_is_consumed_by_both_formats() {
    let doc = survives("bom", "json", "\u{feff}{\"a\":1}".as_bytes());
    assert!(doc.contains("\"a\": 1"), "{doc}");
    assert!(!doc.contains('\u{feff}'), "the BOM reached the screen: {doc:?}");
    let rec = survives("bomrec", "jsonl", "\u{feff}{\"a\":1}\n{\"a\":2}\n".as_bytes());
    assert!(!rec.contains('\u{feff}'), "{rec:?}");
}

// -- empty and degenerate files ---------------------------------------------

#[test]
fn empty_and_whitespace_only_files_render_nothing_and_exit_clean() {
    for (name, ext, body) in [
        ("empty", "json", &b""[..]),
        ("empty", "jsonl", &b""[..]),
        ("ws", "json", &b"   \t\r\n  "[..]),
        ("newlines", "json", &b"\n\n\n\n\n"[..]),
    ] {
        let out = survives(name, ext, body);
        assert!(out.trim().is_empty(), "{name}.{ext} rendered {out:?}");
    }
}

/// A file of nothing but newlines read as records is a run of blank lines, and
/// each one is reported rather than silently dropped — a record file's line
/// numbers have to keep meaning what they say.
#[test]
fn a_record_file_of_only_newlines_reports_every_line() {
    let out = survives("onlynl", "jsonl", &b"\n".repeat(20));
    assert_eq!(out.lines().count(), 20, "{out}");
    assert!(out.contains("line 20"), "{out}");
}

/// A document that is one scalar is still a document, and still a row.
#[test]
fn a_bare_scalar_is_a_document() {
    assert!(survives("scalar", "json", b"42\n").contains("42"));
    assert!(survives("string", "json", br#""hi""#).contains("\"hi\""));
    assert!(survives("null", "json", b"null").contains("null"));
}

// -- record files -----------------------------------------------------------

/// One bad line does not stop the file: it becomes an error row carrying the
/// reason and the line number, and the records around it still render
/// (SPEC.md §JSON — "Half a log is still worth reading").
#[test]
fn one_invalid_record_becomes_an_error_row_and_the_file_continues() {
    let body = b"{\"a\":1}\n{\"a\":2,,}\nnot json at all\n{\"a\":3}\n";
    let out = survives("badline", "jsonl", body);
    assert!(out.contains("line 2"), "the line number is named: {out}");
    assert!(out.contains("line 3"), "{out}");
    assert!(out.contains("\"a\": 1") && out.contains("\"a\": 3"), "{out}");
}

/// Line endings, a missing final newline and blank lines are all ordinary.
#[test]
fn record_files_survive_crlf_bare_cr_and_a_missing_final_newline() {
    for (name, body) in [
        ("crlf", &b"{\"a\":1}\r\n{\"a\":2}\r\n{\"a\":3}\r\n"[..]),
        ("barecr", &b"{\"a\":1}\r{\"a\":2}\r{\"a\":3}\r"[..]),
        ("nonl", &b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}"[..]),
        ("blanks", &b"{\"a\":1}\n\n\n{\"a\":2}\n"[..]),
    ] {
        let out = survives(name, "jsonl", body);
        assert!(out.contains("\"a\": 1"), "{name}: {out}");
        assert!(out.contains("\"a\": 2"), "{name}: {out}");
        assert!(!out.contains('\r'), "{name}: a CR reached the output");
    }
}

/// A single line of tens of kilobytes is the shape a real agent trajectory
/// reaches, and it must render as one record rather than being cut into rows.
#[test]
fn a_forty_kilobyte_record_renders_as_one_record() {
    let big = "z".repeat(41 * 1024);
    let body = format!("{{\"seq\":1,\"message\":\"{big}\"}}\n{{\"seq\":2}}\n");
    let out = survives("longline", "jsonl", body.as_bytes());
    assert!(out.contains("\"seq\": 1"), "{}", &out[..out.len().min(400)]);
    assert!(out.contains("\"seq\": 2"), "the record after it survives");
}

// -- nesting ----------------------------------------------------------------

/// The stack-overflow case. Ten thousand levels must render, as a document and
/// as a record, without recursing — and without the memory cost being quadratic
/// in the depth, which is what a fold id stored per node made it.
///
/// Both sources open down to the shared presentation depth ([`CAP`]) and paint
/// one refusal note for everything under it — the flat render SPEC.md §JSON
/// asks for, and what keeps the document source's per-level byte re-walk from
/// making a 20KB file take seconds. So the render is a bracket row and a
/// closing row for the root and each opened level, plus that note.
#[test]
fn ten_thousand_levels_render_as_a_document_and_as_a_record() {
    let deep = format!("{}1{}", "[".repeat(10_000), "]".repeat(10_000));
    let doc = once("deep", "json", deep.as_bytes());
    assert_eq!(doc.lines().count(), CAP_ROWS, "two rows per opened level, plus the note");
    let rec = once("deep", "jsonl", format!("{deep}\n").as_bytes());
    assert_eq!(rec.lines().count(), CAP_ROWS);
}

/// Nesting to exactly the limit is rendered whole, down to the innermost
/// scalar: the refusal above is a bound on hostile input, not a ceiling
/// ordinary documents meet.
#[test]
fn nesting_to_the_limit_renders_every_level() {
    let deep = format!("{}1{}", "[".repeat(CAP), "]".repeat(CAP));
    let doc = once("atcap", "json", deep.as_bytes());
    assert_eq!(doc.lines().count(), CAP * 2 + 1);
    assert!(doc.lines().nth(CAP).is_some_and(|r| r.contains('1')), "{doc}");
    let rec = once("atcap", "jsonl", format!("{deep}\n").as_bytes());
    assert_eq!(rec.lines().count(), CAP * 2 + 1);
}

/// Past the limit the document says so and stops, rather than opening a node
/// per level for ever. A 200KB file of `[` used 8.5GB before this.
#[test]
fn nesting_past_the_limit_is_refused_by_name_rather_than_opened() {
    let n = 10_100;
    let deep = format!("{}1{}", "[".repeat(n), "]".repeat(n));
    let out = once("toodeep", "json", deep.as_bytes());
    assert!(
        out.contains("nested deeper than 256 levels"),
        "no refusal shown: {}",
        &out[out.len().saturating_sub(400)..]
    );
    // Nodes at depth 0..=256 open and close, with the refusal between them.
    assert_eq!(out.lines().count(), CAP_ROWS, "the render is bounded");
    // The record form draws the line in the same place, in its own words.
    let rec = once("toodeep", "jsonl", format!("{deep}\n").as_bytes());
    assert!(rec.contains("nesting deeper than 10000 levels"), "{rec}");
}

/// Ten thousand opening brackets and nothing else: deep *and* truncated.
#[test]
fn nesting_that_is_never_closed_still_terminates() {
    let out = once("unclosed", "json", "[".repeat(10_000).as_bytes());
    assert_eq!(out.lines().count(), CAP_ROWS, "every opened level is closed");
}

// -- lenses -----------------------------------------------------------------

/// A record a lens does not recognise falls back to the generic tree and is
/// never hidden (SPEC.md §Lenses).
#[test]
fn a_lens_falls_back_to_the_generic_tree_on_records_it_does_not_know() {
    let body = b"{\"id\":0,\"colour\":\"blue\"}\n{\"id\":1,\"colour\":\"red\"}\n";
    let path = temp_doc_ext("notraj", "jsonl", body);
    let lensed = render(&path, &["--width", "100", "--plain", "--lens", "agent"]);
    let plain = render(&path, &["--width", "100", "--plain"]);
    assert_eq!(lensed, plain, "an unrecognised record renders the same either way");
    assert!(lensed.contains("\"colour\": \"blue\""), "{lensed}");
}

/// Trajectory-shaped keys carrying the wrong types must not be trusted into a
/// panic: the lens either reads them or falls back, and never crashes.
#[test]
fn a_lens_survives_records_shaped_like_it_expects_but_typed_wrong() {
    let body = concat!(
        "{\"type\":42,\"message\":[1,2,3]}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":\"a bare string\"}}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\"}]}}\n",
        "{\"message\":null}\n",
        "{}\n",
    );
    let path = temp_doc_ext("wrongtypes", "jsonl", body.as_bytes());
    let out = render(&path, &["--width", "100", "--plain", "--lens", "agent"]);
    assert!(!out.is_empty(), "the file rendered as nothing");
    assert!(!out.contains('\u{1b}'));
}

/// A lens is for record files. Anything else is a usage error naming what the
/// file actually is, not a silent no-op.
#[test]
fn a_lens_on_a_file_that_is_not_records_is_refused_with_the_reason() {
    use std::process::Command;
    for (ext, body, says) in [
        ("json", &b"{\"a\":1}"[..], "JSON document"),
        ("md", &b"# hi\n"[..], "markdown"),
    ] {
        let path = temp_doc_ext("notrecords", ext, body);
        let out = Command::new(env!("CARGO_BIN_EXE_tread"))
            .args(["--no-alt", "--plain", "--lens", "agent"])
            .arg(&path)
            .output()
            .expect("run tread");
        assert_eq!(out.status.code(), Some(2), "expected a usage error for .{ext}");
        assert!(out.stdout.is_empty(), "refused but still painted something");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(says), "unhelpful refusal for .{ext}: {err}");
        assert!(err.contains("--format jsonl"), "no way forward offered: {err}");
    }
}

/// A lens groups a long run of mechanics into one foldable row, but `zR` and
/// the dump path must still be able to open every record in it. Classification
/// runs lazily, ahead of the viewport; a record that joined the run *after* it
/// was opened used to have its expanded tree discarded, so only the first
/// `FIRST_CLASS + 1` records of the run ever showed a body.
#[test]
fn a_lens_opens_every_record_of_a_long_run_not_just_the_first_screenful() {
    let n = 300;
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&format!("{{\"type\":\"mode\",\"mode\":\"M{i:04}\",\"pad\":{{\"x\":{i}}}}}\n"));
    }
    let path = temp_doc_ext("longrun", "jsonl", body.as_bytes());
    let lensed = render(&path, &["--width", "100", "--plain", "--lens", "agent"]);
    let plain = render(&path, &["--width", "100", "--plain"]);
    let count = |s: &str| s.lines().filter(|r| r.contains("\"mode\":")).count();
    assert_eq!(count(&plain), n, "without a lens every record has a body");
    assert_eq!(count(&lensed), n, "the lens dropped the body of {} records", n - count(&lensed));
}

/// A key whose value never arrived — a truncated file, or `{"k":}` — must not
/// be shown holding its own key as its value. The key's bytes are valid JSON,
/// so borrowing them for the value made `{"beta":` render as
/// `"beta": "beta"`: text the document does not contain.
#[test]
fn a_key_with_no_value_says_so_rather_than_echoing_the_key() {
    for (name, body) in [
        ("truncated", &br#"{"alpha": 1, "beta":"#[..]),
        ("empty_member", &br#"{"o":{"k":}}"#[..]),
        ("no_colon", &br#"{"alpha": 1, "beta""#[..]),
    ] {
        let out = survives(name, "json", body);
        assert!(
            !out.contains(r#""beta": "beta""#) && !out.contains(r#""k": "k""#),
            "{name}: the key was shown as its own value: {out}"
        );
        assert!(out.contains("not JSON"), "{name}: no reason given: {out}");
        assert!(out.contains("at byte"), "{name}: no offset given: {out}");
    }
    // The members before it still render: half a document is worth reading.
    let out = survives("truncated2", "json", br#"{"alpha": 1, "beta":"#);
    assert!(out.contains("\"alpha\": 1"), "{out}");
}

/// A timestamp is text out of the log, so it may be any bytes at all. The lens
/// reads `HH:MM` out of it by offset, and a multi-byte character straddling one
/// of those offsets used to panic the whole reader on the record that reached
/// the viewport — one bad line killing a multi-GB log.
#[test]
fn a_lens_survives_a_timestamp_with_multibyte_characters_in_it() {
    let body = concat!(
        "{\"type\":\"user\",\"timestamp\":\"2026-08-05T21:\u{20ac}z\",",
        "\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
        "{\"type\":\"user\",\"timestamp\":\"\u{1f600}\u{1f600}\u{1f600}\u{1f600}T21:28Z\",",
        "\"message\":{\"role\":\"user\",\"content\":\"second\"}}\n",
        "{\"type\":\"user\",\"timestamp\":\"2026-08-05T2\u{20ac}:28:58Z\",",
        "\"message\":{\"role\":\"user\",\"content\":\"third\"}}\n",
        "{\"type\":\"user\",\"timestamp\":42,",
        "\"message\":{\"role\":\"user\",\"content\":\"fourth\"}}\n",
        "{\"type\":\"user\",\"timestamp\":\"2026-08-05T21:28:58.659Z\",",
        "\"message\":{\"role\":\"user\",\"content\":\"fifth\"}}\n",
    );
    let path = temp_doc_ext("badstamp", "jsonl", body.as_bytes());
    let out = render(&path, &["--width", "100", "--plain", "--lens", "agent"]);
    for want in ["first", "second", "third", "fourth", "fifth"] {
        assert!(out.contains(want), "lost {want}: {out}");
    }
    assert!(out.contains("21:28"), "the one good clock still reads: {out}");
}
