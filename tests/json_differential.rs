//! The same JSON, read both ways, must look the same.
//!
//! There are two JSON sources — one document read by byte range
//! (`src/source/json/`), one record per line read as parsed values
//! (`src/source/jsonl/`) — and they build their rows from completely different
//! material. Nothing stops them from drifting except a test that reads the same
//! bytes through both and compares, which is what this file is: a JSON object
//! must not look like a different object because it arrived in a `.jsonl`.
//!
//! Both are driven through the real binary, so this covers the wiring as well:
//! extension detection, `--format`, the stdin sniff, and the dump path that
//! unfolds everything before writing (a folded block in a pipe is missing
//! output, not something the reader can open).
//!
//! # The one row that is allowed to differ
//!
//! Row 0. In the document reader it is the root — `▾ {`. In the record reader
//! it is the row that stands for the record in a *list* of records, so it keeps
//! the collapsed summary and adds a preview of the first short scalars
//! (`▾ {…4 keys}  · type: "assistant"`): a thousand rows that all read `▾ {`
//! are not a reader. Everything below it — every indent, every marker, every
//! bracket, every summary, every scalar — must be byte-identical.

mod harness;

use harness::{render, render_stdin, strip};
use std::path::PathBuf;

/// A document with one of everything the grammar can draw: nested objects and
/// arrays, an empty object, an empty array, every scalar kind, a key that
/// cannot be written with a dot, duplicate keys, a number no `f64` holds, and a
/// string with a control character in it.
const BODY: &str = concat!(
    r#"{"type":"assistant","n":2,"big":1e999,"dup":1,"dup":2,"odd key":"x","#,
    r#""items":[10,{"a":null,"b":[true,"tab\there"],"deep":{"er":[[]]}}],"#,
    r#""empty":{},"none":[],"flag":false}"#,
);

struct Doc(PathBuf);

impl Drop for Doc {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn temp(name: &str, ext: &str, body: &[u8]) -> Doc {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("tread-jsondiff-{}-{nanos}-{name}{ext}", std::process::id()));
    std::fs::write(&p, body).expect("write temp doc");
    Doc(p)
}

fn rows(out: &str) -> Vec<String> {
    strip(out).lines().map(|l| l.to_string()).collect()
}

/// The document as `.json` and as a one-record `.jsonl`, both dumped.
fn both() -> (Vec<String>, Vec<String>) {
    let json = temp("doc", ".json", BODY.as_bytes());
    let jsonl = temp("rec", ".jsonl", format!("{BODY}\n").as_bytes());
    let args = ["--no-alt", "--plain", "--width", "200"];
    (
        rows(&render(&json.0, &args)),
        rows(&render(&jsonl.0, &args)),
    )
}

#[test]
fn the_same_content_read_as_a_document_and_as_a_record_has_the_same_tree() {
    let (doc, rec) = both();
    assert!(doc.len() > 15, "the fixture should draw a real tree: {doc:#?}");
    assert_eq!(doc.len(), rec.len(), "different row counts\n{doc:#?}\n{rec:#?}");
    for (i, (a, b)) in doc.iter().zip(rec.iter()).enumerate().skip(1) {
        assert_eq!(a, b, "row {i} differs\ndocument: {a:?}\nrecord:   {b:?}");
    }
}

/// Row 0 is the documented exception, and even it agrees up to the preview: the
/// record row is the document's *collapsed* root row with a suffix.
#[test]
fn the_record_row_is_the_collapsed_root_row_plus_a_preview() {
    let (doc, rec) = both();
    assert_eq!(doc[0], "\u{25be} {");
    let summary = "\u{25be} {\u{2026}10 keys}";
    assert!(rec[0].starts_with(summary), "{:?}", rec[0]);
    assert!(rec[0].contains("type: \"assistant\""), "{:?}", rec[0]);
    // Duplicate keys are kept, so the count is 10 rather than 9.
    assert!(doc[1..].iter().filter(|r| r.contains("\"dup\"")).count() == 2);
}

/// Every visual decision comes from the shared grammar, so these are the same
/// on both sides by construction — and this is the list that would silently
/// diverge again if either source grew its own renderer.
#[test]
fn the_shared_grammar_shows_in_both_renders() {
    let (doc, rec) = both();
    for what in [&doc, &rec] {
        let all = what.join("\n");
        // Empty containers say what they are rather than counting to zero.
        assert!(all.contains("\"empty\": {"), "{all}");
        assert!(all.contains("\"none\": ["), "{all}");
        // Numbers keep their source text.
        assert!(all.contains("\"big\": 1e999"), "{all}");
        // Strings are shown as the literal the file holds: the escape as the
        // two characters it is, never a raw control character and never a
        // stand-in glyph claiming the document holds something it does not.
        assert!(all.contains(r#""tab\there""#), "{all}");
        assert!(!all.contains('\t'), "a raw tab reached the frame");
        // An array element carries no index label in either reader.
        assert!(!all.contains("[0]:"), "{all}");
        // Two columns of gutter per level, and a closing bracket in the same
        // column as its opening one.
        assert!(what.iter().any(|r| r == "  \u{25be} \"items\": ["), "{all}");
        assert!(what.iter().any(|r| r == "    ]"), "{all}");
    }
}

/// The same fact through the other three doors: `--format`, a piped document
/// and a piped record stream. A format reached by a different route must render
/// the same rows, or detection is deciding what a document *is*.
#[test]
fn every_route_into_the_two_sources_renders_the_same_tree() {
    let (doc, rec) = both();
    let args = ["--no-alt", "--plain", "--width", "200"];

    let forced = temp("forced", ".txt", BODY.as_bytes());
    let mut with_flag = args.to_vec();
    with_flag.extend(["--format", "json"]);
    assert_eq!(rows(&render(&forced.0, &with_flag)), doc, "--format json");

    let piped = rows(&render_stdin(BODY.as_bytes(), &args));
    assert_eq!(piped, doc, "a piped document sniffs as one document");

    // Two records of the same content: a record stream, and each record is the
    // tree the document reader draws.
    let stream = format!("{BODY}\n{BODY}\n");
    let piped = rows(&render_stdin(stream.as_bytes(), &args));
    assert_eq!(piped.len(), rec.len() * 2, "{piped:#?}");
    assert_eq!(&piped[..rec.len()], &rec[..]);
    assert_eq!(&piped[rec.len()..], &rec[..]);
}

/// Ten thousand levels of `[[[[` read as a document and as a record: neither
/// may blow the stack, and both must still agree.
///
/// Both stop opening at the shared presentation depth (`jsonrow::MAX_DEPTH`,
/// 256) and paint one refusal note for everything below it — the "flat render"
/// SPEC.md §JSON asks for, and what keeps the document source's per-level byte
/// re-walk from turning this file into a hang. The count below is that shape: a
/// bracket row and a closing row for the root and each of the 256 levels under
/// it, plus the note.
#[test]
fn hostile_nesting_renders_the_same_and_does_not_recurse() {
    const CAP: usize = 256;
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let json = temp("deep", ".json", deep.as_bytes());
    let jsonl = temp("deep", ".jsonl", format!("{deep}\n").as_bytes());
    let args = ["--no-alt", "--plain", "--width", "200"];
    let doc = rows(&render(&json.0, &args));
    let rec = rows(&render(&jsonl.0, &args));
    assert_eq!(doc.len(), (CAP + 1) * 2 + 1, "two rows per opened level, plus the note");
    assert!(doc[CAP + 1].contains("nested deeper than 256 levels"), "{:?}", doc[CAP + 1]);
    assert_eq!(doc.len(), rec.len());
    for (i, (a, b)) in doc.iter().zip(rec.iter()).enumerate().skip(1) {
        assert_eq!(a, b, "row {i}");
    }
}

/// Nesting to exactly the limit is rendered whole by both sources, down to the
/// innermost scalar: the refusal above is a bound on hostile input and not a
/// ceiling ordinary documents meet.
#[test]
fn nesting_to_the_limit_renders_the_same_and_is_not_refused() {
    const CAP: usize = 256;
    let deep = format!("{}\"end\"{}", "[".repeat(CAP), "]".repeat(CAP));
    let json = temp("atcap", ".json", deep.as_bytes());
    let jsonl = temp("atcap", ".jsonl", format!("{deep}\n").as_bytes());
    let args = ["--no-alt", "--plain", "--width", "200"];
    let doc = rows(&render(&json.0, &args));
    let rec = rows(&render(&jsonl.0, &args));
    assert_eq!(doc.len(), CAP * 2 + 1);
    assert!(doc[CAP].contains("\"end\""), "{:?}", doc[CAP]);
    assert_eq!(doc.len(), rec.len());
    for (i, (a, b)) in doc.iter().zip(rec.iter()).enumerate().skip(1) {
        assert_eq!(a, b, "row {i}");
    }
}
