//! Which format a document is in (SPEC.md §Multi-format reading).
//!
//! "File extension first; content sniff when there is no name, which is the
//! stdin case; `--format` overrides both." All three rules live here as pure
//! functions of their inputs, so the whole decision table is host-tested and
//! `main` only has to ask.
//!
//! The sniff is deliberately conservative: markdown is the default and a file
//! has to *look* like a delimited table — several rows agreeing on a field
//! count under one delimiter — before it is treated as one. Prose that happens
//! to contain a comma stays markdown.
#![deny(unsafe_code)]

use std::path::Path;

use crate::csv::delim;
use crate::csv::parse::{strip_bom, Records};

/// The formats compiled into this binary. There is no other kind: formats are
/// never loaded at runtime (SPEC.md §Multi-format reading).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Markdown,
    Csv,
    /// One JSON document: `.json` (SPEC.md §JSON).
    Json,
    /// One JSON value per line: `.jsonl`, `.ndjson` (SPEC.md §JSON).
    Jsonl,
    /// A source file, read as its comments and declarations
    /// (SPEC.md §Code).
    Code,
    /// Lines, verbatim: anything whose extension names no parser
    /// (SPEC.md §Plain text).
    Text,
}

/// `--format <md|csv|json|jsonl|text>`. Accepts the obvious spellings and
/// nothing else.
pub fn parse_format(spec: &str) -> Option<Format> {
    match spec.to_ascii_lowercase().as_str() {
        "md" | "markdown" => Some(Format::Markdown),
        "csv" | "tsv" | "table" => Some(Format::Csv),
        "json" => Some(Format::Json),
        "jsonl" | "ndjson" | "jsonlines" => Some(Format::Jsonl),
        "text" | "txt" | "plain" | "plaintext" => Some(Format::Text),
        _ => None,
    }
}

/// What a format is called, for a message a person reads.
pub fn name_of(format: Format) -> &'static str {
    match format {
        Format::Markdown => "markdown",
        Format::Csv => "CSV",
        Format::Json => "a JSON document",
        Format::Jsonl => "a record file",
        Format::Code => "code",
        Format::Text => "plain text",
    }
}

/// Format from a file name's extension, or `None` when it names no parser.
///
/// `None` is not "unknown": [`decide`] reads it as plain text, because a named
/// file whose extension claims no parser is exactly what plain text is for
/// (SPEC.md §Plain text). This function stays the narrower question — *does
/// this extension name a parser?* — because two callers need that one
/// (`crate::open::lens`, which treats a named extension as evidence a `--lens`
/// may not overrule, and `crate::open`, which reads only a BOM when it does).
///
/// `.txt` and `.text` name the text reader now that there is one. They used to
/// name the markdown parser, which was only ever true for want of an
/// alternative: `# TODO` at the top of a `notes.txt` is a comment, and
/// rendering it as a banner heading is the exact failure SPEC.md §Plain text
/// names.
pub fn from_path(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "csv" | "tsv" | "tab" => Some(Format::Csv),
        "json" => Some(Format::Json),
        "jsonl" | "ndjson" => Some(Format::Jsonl),
        "md" | "markdown" | "mdown" | "mkd" => Some(Format::Markdown),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" | "pyi" | "java" => {
            Some(Format::Code)
        }
        "txt" | "text" => Some(Format::Text),
        _ => None,
    }
}

/// An encoding tread cannot read, named for the error message.
///
/// tread is a UTF-8 reader: invalid bytes become `U+FFFD` and the document
/// still opens, which is the right answer for a stray byte in an otherwise
/// UTF-8 file. It is the wrong answer for a file that is *entirely* another
/// encoding. UTF-16 text is half NUL bytes, so a lossy decode renders every
/// letter with a `\u{fffd}` or a `\u{b7}` between it and the next one, the
/// delimiter sniff sees no delimiter, and the reader is left looking at
/// mojibake with nothing to say what happened. A file that announces itself
/// with a UTF-16 or UTF-32 byte-order mark is therefore refused by name.
///
/// Only a BOM counts. Guessing an encoding from NUL frequency would refuse
/// binary files people legitimately open to look at, and a UTF-16 file without
/// a BOM is indistinguishable from one.
pub fn unreadable_encoding(head: &[u8]) -> Option<&'static str> {
    // UTF-32's BOMs start with UTF-16's, so they are tested first.
    if head.starts_with(&[0xff, 0xfe, 0x00, 0x00]) || head.starts_with(&[0x00, 0x00, 0xfe, 0xff]) {
        return Some("UTF-32");
    }
    if head.starts_with(&[0xff, 0xfe]) || head.starts_with(&[0xfe, 0xff]) {
        return Some("UTF-16");
    }
    None
}

/// Rows a content sniff looks at, and the fewest that must agree.
const SNIFF_ROWS: usize = 16;
const MIN_ROWS: usize = 3;

/// Format of unnamed input — the stdin case — from its first bytes.
///
/// Markdown wins unless the sample parses as a table: at least [`MIN_ROWS`]
/// rows, all with the same field count, at least two fields, under the sniffed
/// delimiter. A leading `#`, `>` or list bullet is markdown whatever the
/// commas say, and a leading `{`/`[` is structured data we do not read as a
/// table either — one document or a stream of records, which [`is_jsonl`]
/// settles.
pub fn sniff(sample: &[u8]) -> Format {
    let bytes = strip_bom(sample);
    let head = bytes.iter().position(|b| *b == b'\n').unwrap_or(bytes.len());
    let first = String::from_utf8_lossy(&bytes[..head]);
    let lead = first.trim_start();
    // A JSON document may be indented or start on a later line, so the bracket
    // is looked for past *all* the leading whitespace rather than on the first
    // line only.
    if matches!(bytes.iter().find(|b| !b.is_ascii_whitespace()), Some(b'{' | b'[')) {
        return json_shape(bytes);
    }
    if lead.starts_with('#')
        || lead.starts_with('>')
        || lead.starts_with("- ")
        || lead.starts_with("* ")
        || lead.starts_with('|')
    {
        return Format::Markdown;
    }
    let d = delim::sniff(bytes);
    let counts: Vec<usize> = Records::new(bytes, d)
        .take(SNIFF_ROWS)
        .map(|r| r.fields.len())
        .collect();
    // The sample is a prefix, so its last row is probably cut in half.
    let counts = match counts.len() > MIN_ROWS {
        true => &counts[..counts.len() - 1],
        false => &counts[..],
    };
    let agree = counts.len() >= MIN_ROWS
        && counts[0] >= 2
        && counts.iter().all(|c| *c == counts[0]);
    match agree {
        true => Format::Csv,
        false => Format::Markdown,
    }
}

/// A sample that begins with a bracket or a brace: one JSON document, a stream
/// of one JSON value per line, or markdown that happens to start with one?
fn json_shape(bytes: &[u8]) -> Format {
    match json_or_markdown(bytes) {
        Format::Json if is_jsonl(bytes) => Format::Jsonl,
        other => other,
    }
}

/// Is this sample a record *stream* rather than one document?
///
/// **The rule.** The first line is a complete JSON value on its own, and there
/// is a second line that is also a complete JSON value — or that starts one and
/// runs off the end of the sample, which is what the second line of a large
/// file looks like from its first 8KB.
///
/// The rule is sound in one direction and only one: two complete JSON values in
/// a row *cannot* be a single JSON document, so a sample that satisfies it is
/// never a document mistaken for a stream. A pretty-printed document fails at
/// the first line (`{` alone is not a value, `[1,` is not one either), and a
/// compact one has no second line at all.
///
/// **Its failure mode** is the other direction: a record file can be missed and
/// read as one document. That happens when there is only one record, and when
/// the first record is longer than the sample so no line ends inside it — the
/// user's own agent trajectory has a 41KB line, and piping a file whose *first*
/// line was that long would land here. What it costs is bounded: the first
/// value renders as the document tree, which since
/// [`crate::source::jsonrow`] is row-for-row the same tree the record reader
/// would have drawn for it. The remaining records are then trailing content the
/// document reader ignores, so `--format jsonl` is the fix, and a named
/// `.jsonl` never reaches this code at all.
fn is_jsonl(bytes: &[u8]) -> bool {
    let mut lines = bytes.split(|b| *b == b'\n');
    let Some(first) = lines.next() else {
        return false;
    };
    // A last line with no terminator is not known to be a whole line, so the
    // first line only counts when the sample actually held its newline.
    if first.len() == bytes.len() || crate::json::parse(first).is_err() {
        return false;
    }
    for next in lines {
        if next.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        return match crate::json::parse_prefix(next) {
            Ok(_) => true,
            // The sample stopped mid-value: it still started one.
            Err(e) => e.reason == crate::json::Reason::Eof,
        };
    }
    false
}

/// A sample that begins with a bracket: JSON, or markdown that happens to
/// start with one?
///
/// The question is settled by *reading* it rather than by the first character:
/// `[link]: https://x` and `[x] a task` both begin with `[` and are markdown.
/// Running out of input is not a failure here — the sample is the head of a
/// file that may be gigabytes long, and a well-formed document looks exactly
/// like that from the first 8KB.
fn json_or_markdown(bytes: &[u8]) -> Format {
    match crate::json::parse_prefix(bytes) {
        Ok(_) => Format::Json,
        Err(e) if e.reason == crate::json::Reason::Eof => Format::Json,
        Err(_) => Format::Markdown,
    }
}

/// The whole decision, in one place: `--format` wins, then the extension, then
/// — only for input with no name at all — the content.
///
/// **A named file is never sniffed.** Its extension names a parser or it is
/// plain text, and that is the whole rule (SPEC.md §Plain text). A file called
/// `deploy` or `deploy.sh` full of `#` comments is indistinguishable from
/// markdown to any sniffer worth writing, and guessing wrong turns a comment
/// into a banner heading — strictly worse than the dumb rule, which renders it
/// verbatim and is never surprising. `--format` is there for the file that
/// really is markdown under another name.
///
/// The sniff survives for the one input that has no name to read: a pipe.
pub fn decide(forced: Option<Format>, path: Option<&Path>, sample: &[u8]) -> Format {
    if let Some(f) = forced {
        return f;
    }
    match path {
        Some(p) => from_path(p).unwrap_or(Format::Text),
        None => sniff(sample),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn extensions_decide_when_they_can() {
        assert_eq!(from_path(&p("a.csv")), Some(Format::Csv));
        assert_eq!(from_path(&p("a.TSV")), Some(Format::Csv));
        assert_eq!(from_path(&p("a.md")), Some(Format::Markdown));
        assert_eq!(from_path(&p("a.jsonl")), Some(Format::Jsonl));
        assert_eq!(from_path(&p("a.NDJSON")), Some(Format::Jsonl));
        // `.txt` names the text reader now that there is one; it used to name
        // the markdown parser for want of an alternative (see `from_path`).
        assert_eq!(from_path(&p("a.txt")), Some(Format::Text));
        assert_eq!(from_path(&p("a.TEXT")), Some(Format::Text));
        // `.rs` names the code reader now that there is one; like `.txt`
        // before it, it used to fall through to plain text.
        assert_eq!(from_path(&p("a.rs")), Some(Format::Code));
        assert_eq!(from_path(&p("a.RS")), Some(Format::Code));
        assert_eq!(from_path(&p("a.py")), Some(Format::Code));
        assert_eq!(from_path(&p("a.java")), Some(Format::Code));
        // A language this build does not know is still plain text.
        assert_eq!(from_path(&p("a.rb")), None);
        assert_eq!(from_path(&p("plain")), None);
    }

    /// An extension that names no parser, and no extension at all, are both
    /// plain text — decided by the *name*, never by the bytes (SPEC.md §Plain
    /// text).
    #[test]
    fn a_file_no_parser_claims_is_plain_text() {
        let shell = b"#!/bin/sh\n# deploy the thing\nset -eu\n";
        assert_eq!(decide(None, Some(&p("deploy.sh")), shell), Format::Text);
        assert_eq!(decide(None, Some(&p("deploy")), shell), Format::Text);
        assert_eq!(decide(None, Some(&p("nginx.conf")), b"server { }\n"), Format::Text);
        assert_eq!(decide(None, Some(&p("a.txt")), b"# not a heading\n"), Format::Text);
        // And `--format text` forces it for a file whose extension would
        // otherwise claim a parser.
        assert_eq!(parse_format("text"), Some(Format::Text));
        assert_eq!(parse_format("TXT"), Some(Format::Text));
        assert_eq!(
            decide(Some(Format::Text), Some(&p("notes.md")), b"# hi\n"),
            Format::Text
        );
        // A known extension still keeps its parser.
        assert_eq!(decide(None, Some(&p("notes.md")), shell), Format::Markdown);
        assert_eq!(decide(None, Some(&p("d.csv")), shell), Format::Csv);
    }

    #[test]
    fn the_flag_overrides_everything() {
        let csv = b"a,b\n1,2\n3,4\n5,6\n";
        assert_eq!(
            decide(Some(Format::Markdown), Some(&p("x.csv")), csv),
            Format::Markdown
        );
        assert_eq!(decide(Some(Format::Csv), Some(&p("x.md")), b"# hi"), Format::Csv);
        assert_eq!(parse_format("CSV"), Some(Format::Csv));
        assert_eq!(parse_format("markdown"), Some(Format::Markdown));
        // `json` named no format until there was one to name.
        assert_eq!(parse_format("json"), Some(Format::Json));
        assert_eq!(parse_format("yaml"), None);
        assert_eq!(decide(Some(Format::Json), Some(&p("x.md")), b"# hi"), Format::Json);
        assert_eq!(parse_format("JSONL"), Some(Format::Jsonl));
        assert_eq!(parse_format("ndjson"), Some(Format::Jsonl));
        assert_eq!(decide(Some(Format::Jsonl), Some(&p("x.csv")), b"a,b\n"), Format::Jsonl);
    }

    #[test]
    fn a_delimited_sample_sniffs_as_csv() {
        for body in [
            &b"id,name\n1,alice\n2,bob\n3,carol\n"[..],
            &b"id\tname\n1\talice\n2\tbob\n3\tcarol\n"[..],
            &b"id;name\n1;alice\n2;bob\n3;carol\n"[..],
            &b"\xef\xbb\xbfid,name\n1,a\n2,b\n3,c\n"[..],
        ] {
            assert_eq!(sniff(body), Format::Csv, "{:?}", String::from_utf8_lossy(body));
        }
    }

    #[test]
    fn prose_stays_markdown_however_many_commas() {
        for body in [
            &b"# Title\n\nOne, two, three.\n\nAnd more, prose, here.\n"[..],
            &b"Some words, and more words.\n"[..],
            &b"> quoted, text\n> more, text\n> and, more\n"[..],
            &b"- a, b\n- c, d\n- e, f\n"[..],
            &b"| a | b |\n| --- | --- |\n| 1 | 2 |\n"[..],
            &b""[..],
            // Markdown that merely *starts* with a bracket: a reference link
            // definition and a task item are not JSON, and reading them says so.
            &b"[link]: https://example.com\n\nprose\n"[..],
            &b"[x] a task, and another\n"[..],
            &b"[unclosed markdown\n"[..],
        ] {
            assert_eq!(
                sniff(body),
                Format::Markdown,
                "{:?}",
                String::from_utf8_lossy(body)
            );
        }
    }

    /// A leading `{` or `[` used to be read as "structured data, so markdown".
    /// It is now read as what it is: `--format json` and `.json` are not the
    /// only ways to open a JSON document, and a pipe has neither (SPEC.md
    /// §Multi-format reading, "a leading `{`/`[`").
    #[test]
    fn a_json_sample_sniffs_as_json() {
        for body in [
            &b"{\"a\": 1, \"b\": 2}\n"[..],
            &b"[1, 2, 3]\n"[..],
            &b"  \n {\n  \"deep\": {\"er\": [true, null]}\n}\n"[..],
            // The head of a document far too big to have been read whole: it
            // stops mid-value, which is not a reason to call it prose.
            &b"[{\"id\": 1, \"name\": \"ada\"}, {\"id\": 2, \"nam"[..],
        ] {
            assert_eq!(sniff(body), Format::Json, "{:?}", String::from_utf8_lossy(body));
        }
    }

    /// The rule: a first line that is a whole JSON value, and a second line
    /// that is one too (or that starts one and runs off the sample).
    #[test]
    fn a_record_stream_sniffs_as_jsonl_and_a_document_does_not() {
        for body in [
            &b"{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n"[..],
            &b"[1,2]\n[3,4]\n"[..],
            // A blank line between records is not a second record; the one
            // after it is.
            &b"{\"a\":1}\n\n{\"a\":2}\n"[..],
            // The head of a huge log: the second record is cut in half.
            &b"{\"a\":1}\n{\"a\":2,\"b\":\"xxxxxxx"[..],
            // A BOM in front of the first record.
            &b"\xef\xbb\xbf{\"a\":1}\n{\"a\":2}\n"[..],
        ] {
            assert_eq!(sniff(body), Format::Jsonl, "{:?}", String::from_utf8_lossy(body));
        }
        for body in [
            // Pretty-printed: the first line is not a value on its own.
            &b"{\n  \"a\": 1\n}\n"[..],
            &b"[\n  1,\n  2\n]\n"[..],
            // Compact, one line: there is no second record.
            &b"{\"a\":1,\"b\":[2,3]}\n"[..],
            // An array over lines, which is one document and not a stream.
            &b"[{\"a\":1},\n{\"a\":2}]\n"[..],
        ] {
            assert_eq!(sniff(body), Format::Json, "{:?}", String::from_utf8_lossy(body));
        }
    }

    /// The rule's failure mode, named: a single record, or a first record
    /// longer than the sample, reads as one document. Both render the same
    /// tree (`crate::source::jsonrow`), and `--format jsonl` is the override.
    #[test]
    fn a_stream_the_sniff_cannot_see_falls_back_to_one_document() {
        assert_eq!(sniff(b"{\"only\":1}\n"), Format::Json);
        let long = format!("{{\"pad\":\"{}\"}}", "x".repeat(4096));
        let sample = long.into_bytes();
        assert_eq!(sniff(&sample), Format::Json, "no line ended inside the sample");
        assert_eq!(
            decide(Some(Format::Jsonl), None, &sample),
            Format::Jsonl,
            "--format is the override"
        );
    }

    #[test]
    fn a_byte_order_mark_for_an_encoding_we_cannot_read_is_named() {
        assert_eq!(unreadable_encoding(b"\xff\xfei\x00d\x00"), Some("UTF-16"));
        assert_eq!(unreadable_encoding(b"\xfe\xff\x00i\x00d"), Some("UTF-16"));
        assert_eq!(unreadable_encoding(b"\xff\xfe\x00\x00i\x00\x00\x00"), Some("UTF-32"));
        assert_eq!(unreadable_encoding(b"\x00\x00\xfe\xff\x00\x00\x00i"), Some("UTF-32"));
        // UTF-8, with and without its BOM, and short or empty input.
        assert_eq!(unreadable_encoding(b"\xef\xbb\xbfid,name\n"), None);
        assert_eq!(unreadable_encoding(b"id,name\n"), None);
        assert_eq!(unreadable_encoding(b"\xff"), None);
        assert_eq!(unreadable_encoding(b""), None);
        // A NUL-heavy file with no BOM is still opened: guessing would refuse
        // files people legitimately look at.
        assert_eq!(unreadable_encoding(b"a\x00b\x00c\x00"), None);
    }

    #[test]
    fn ragged_rows_are_not_a_table() {
        assert_eq!(sniff(b"a,b\n1,2,3\n4\n5,6\n"), Format::Markdown);
    }

    /// Only *unnamed* input is sniffed.
    ///
    /// This test used to assert that `data` — a named file with no extension —
    /// sniffed as CSV. That expectation is now wrong: SPEC.md §Plain text makes
    /// a named file's extension the whole rule, precisely so that a file whose
    /// content resembles a format it is not never gets read as one. Sniffing is
    /// what a pipe gets, because a pipe has no name to read.
    #[test]
    fn only_unnamed_input_is_sniffed() {
        let csv = b"a,b\n1,2\n3,4\n5,6\n";
        assert_eq!(decide(None, None, csv), Format::Csv);
        assert_eq!(decide(None, Some(&p("data")), csv), Format::Text);
        assert_eq!(decide(None, Some(&p("data.csv")), csv), Format::Csv);
        assert_eq!(decide(None, Some(&p("notes.md")), csv), Format::Markdown);
        assert_eq!(decide(None, None, b"# hi\n"), Format::Markdown);
    }
}
