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
}

/// `--format <md|csv>`. Accepts the obvious spellings and nothing else.
pub fn parse_format(spec: &str) -> Option<Format> {
    match spec.to_ascii_lowercase().as_str() {
        "md" | "markdown" => Some(Format::Markdown),
        "csv" | "tsv" | "table" => Some(Format::Csv),
        _ => None,
    }
}

/// Format from a file name's extension, or `None` when it says nothing.
pub fn from_path(path: &Path) -> Option<Format> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "csv" | "tsv" | "tab" => Some(Format::Csv),
        "md" | "markdown" | "mdown" | "mkd" | "text" | "txt" => Some(Format::Markdown),
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
/// table either.
pub fn sniff(sample: &[u8]) -> Format {
    let bytes = strip_bom(sample);
    let head = bytes.iter().position(|b| *b == b'\n').unwrap_or(bytes.len());
    let first = String::from_utf8_lossy(&bytes[..head]);
    let lead = first.trim_start();
    if lead.starts_with('#')
        || lead.starts_with('>')
        || lead.starts_with("- ")
        || lead.starts_with("* ")
        || lead.starts_with('{')
        || lead.starts_with('[')
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

/// The whole decision, in one place: `--format` wins, then the extension, then
/// the content, then markdown.
pub fn decide(forced: Option<Format>, path: Option<&Path>, sample: &[u8]) -> Format {
    if let Some(f) = forced {
        return f;
    }
    if let Some(f) = path.and_then(from_path) {
        return f;
    }
    // A named file whose extension says nothing still gets sniffed: a `data`
    // with no suffix is as much a CSV as `data.csv`.
    sniff(sample)
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
        assert_eq!(from_path(&p("a.rs")), None);
        assert_eq!(from_path(&p("plain")), None);
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
        assert_eq!(parse_format("json"), None);
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
            &b"{\"a\": 1, \"b\": 2}\n"[..],
            &b""[..],
        ] {
            assert_eq!(
                sniff(body),
                Format::Markdown,
                "{:?}",
                String::from_utf8_lossy(body)
            );
        }
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

    #[test]
    fn an_unnamed_or_odd_named_file_is_sniffed() {
        let csv = b"a,b\n1,2\n3,4\n5,6\n";
        assert_eq!(decide(None, None, csv), Format::Csv);
        assert_eq!(decide(None, Some(&p("data")), csv), Format::Csv);
        assert_eq!(decide(None, Some(&p("notes.md")), csv), Format::Markdown);
        assert_eq!(decide(None, None, b"# hi\n"), Format::Markdown);
    }
}
