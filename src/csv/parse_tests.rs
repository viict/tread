//! Tests for the RFC 4180 machine.
//!
//! Two things are worth more than any single case here: that [`scan_row_ends`]
//! and [`Records`] always agree about where a row ends (`agree` below, run over
//! every fixture and over pseudo-random documents), and that the answer does
//! not depend on how the input is chopped into chunks (`chunked_ends`).

use super::*;
use crate::csv::delim::CANDIDATES;

/// Fields of every record, for terse assertions.
fn rows(src: &str, delim: u8) -> Vec<Vec<String>> {
    records(src.as_bytes(), delim)
}

fn csv(src: &str) -> Vec<Vec<String>> {
    rows(src, b',')
}

/// Row end offsets as the *renderer* sees them.
fn record_ends(bytes: &[u8], delim: u8) -> Vec<u64> {
    Records::new(bytes, delim).map(|r| r.end as u64).collect()
}

/// Row end offsets as the *indexer* sees them, feeding one chunk.
fn scanner_ends(bytes: &[u8], delim: u8) -> Vec<u64> {
    chunked_ends(bytes, delim, bytes.len().max(1))
}

/// Row end offsets from the indexer when the file arrives in `chunk`-sized
/// pieces — the real shape of a streaming pass over a multi-GB file.
fn chunked_ends(bytes: &[u8], delim: u8, chunk: usize) -> Vec<u64> {
    let base = bom_len(bytes);
    let body = strip_bom(bytes);
    let mut sc = Scanner::new(delim);
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < body.len() {
        let end = (at + chunk).min(body.len());
        scan_row_ends(&mut sc, &body[at..end], (base + at) as u64, |o| out.push(o));
        at = end;
    }
    if finish_row_end(&mut sc).has_row() {
        out.push(bytes.len() as u64);
    }
    out
}

/// The invariant that makes the row index safe: the two callers of the machine
/// see the same boundaries, and re-parsing each row's slice on its own gives
/// exactly the fields the streaming parse gave.
fn agree(bytes: &[u8], delim: u8) {
    let recs: Vec<Record> = Records::new(bytes, delim).collect();
    let ends: Vec<u64> = recs.iter().map(|r| r.end as u64).collect();
    assert_eq!(scanner_ends(bytes, delim), ends, "scanner vs records: {bytes:?}");
    for w in 1..=bytes.len().max(1) {
        assert_eq!(chunked_ends(bytes, delim, w), ends, "chunk {w}: {bytes:?}");
    }
    let mut start = bom_len(bytes);
    for r in &recs {
        assert_eq!(r.start, start, "record starts are contiguous: {bytes:?}");
        assert_eq!(record(&bytes[r.start..r.end], delim), r.fields, "row slice: {bytes:?}");
        start = r.end;
    }
    assert_eq!(start, if recs.is_empty() { bom_len(bytes) } else { bytes.len() });
}

fn agree_csv(src: &str) {
    agree(src.as_bytes(), b',');
}

// -- the ordinary cases -------------------------------------------------------

#[test]
fn plain_rows() {
    assert_eq!(csv("a,b,c\n1,2,3\n"), vec![vec!["a", "b", "c"], vec!["1", "2", "3"]]);
    agree_csv("a,b,c\n1,2,3\n");
}

#[test]
fn empty_file_has_no_rows() {
    assert!(csv("").is_empty());
    assert_eq!(scanner_ends(b"", b','), Vec::<u64>::new());
    agree_csv("");
}

#[test]
fn header_only_file() {
    assert_eq!(csv("a,b,c"), vec![vec!["a", "b", "c"]]);
    assert_eq!(csv("a,b,c\n"), vec![vec!["a", "b", "c"]]);
    agree_csv("a,b,c");
    agree_csv("a,b,c\n");
}

#[test]
fn a_trailing_newline_does_not_make_a_phantom_row() {
    assert_eq!(csv("a\n").len(), 1);
    assert_eq!(csv("a\n\n").len(), 2, "a genuinely blank line is a row");
    assert_eq!(csv("a\n\n").pop(), Some(vec![String::new()]));
    agree_csv("a\n\n");
}

#[test]
fn empty_and_trailing_empty_fields() {
    assert_eq!(csv("a,,c\n"), vec![vec!["a", "", "c"]]);
    assert_eq!(csv("a,b,\n"), vec![vec!["a", "b", ""]], "a trailing delimiter adds a field");
    assert_eq!(csv(",\n"), vec![vec!["", ""]]);
    assert_eq!(csv("\n"), vec![vec![""]]);
    agree_csv("a,b,\n,\n\n");
}

// -- quoting ------------------------------------------------------------------

#[test]
fn quoted_field_holds_the_delimiter() {
    assert_eq!(csv("\"a,b\",c\n"), vec![vec!["a,b", "c"]]);
    agree_csv("\"a,b\",c\n");
}

#[test]
fn quoted_field_holds_a_newline() {
    assert_eq!(csv("\"a\nb\",c\n"), vec![vec!["a\nb", "c"]]);
    let ends = record_ends(b"\"a\nb\",c\nx\n", b',');
    assert_eq!(ends, vec![8, 10], "the LF inside the quotes is not a boundary");
    agree_csv("\"a\nb\",c\nx\n");
}

#[test]
fn quoted_field_holds_a_crlf_verbatim() {
    let src = "\"a\r\nb\",c\nx,y\n";
    assert_eq!(rows(src, b','), vec![vec!["a\r\nb", "c"], vec!["x", "y"]]);
    agree_csv(src);
}

#[test]
fn quoted_field_holds_a_bare_cr() {
    assert_eq!(csv("\"a\rb\"\n"), vec![vec!["a\rb"]]);
    agree_csv("\"a\rb\"\n");
}

#[test]
fn doubled_quotes_are_one_literal_quote() {
    assert_eq!(csv("\"a\"\"b\"\n"), vec![vec!["a\"b"]]);
    assert_eq!(csv("\"\"\"\"\n"), vec![vec!["\""]]);
    assert_eq!(csv("\"\"\n"), vec![vec![""]], "an empty quoted field");
    assert_eq!(csv("\"\",\"\"\n"), vec![vec!["", ""]]);
    agree_csv("\"a\"\"b\",\"\"\"\",\"\"\n");
}

#[test]
fn whitespace_around_quotes_is_padding_not_content() {
    assert_eq!(csv(" \"a,b\" ,c\n"), vec![vec!["a,b", "c"]]);
    assert_eq!(csv("  \"x\"\n"), vec![vec!["x"]]);
    assert_eq!(csv("\"x\"  \n"), vec![vec!["x"]], "padding before a terminator");
    assert_eq!(csv("\"x\"  "), vec![vec!["x"]], "padding before EOF");
    assert_eq!(csv("\"x\"  \r\n"), vec![vec!["x"]], "padding before a CRLF");
    agree_csv(" \"a,b\" ,c\n\"x\"  \r\n\"x\"  ");
}

#[test]
fn whitespace_outside_quotes_is_content() {
    assert_eq!(csv(" a , b \n"), vec![vec![" a ", " b "]]);
    assert_eq!(csv("  ,x\n"), vec![vec!["  ", "x"]]);
    assert_eq!(csv("a,  \n"), vec![vec!["a", "  "]], "held-back padding survives to EOL");
    assert_eq!(csv("a,  "), vec![vec!["a", "  "]], "and to EOF");
    agree_csv(" a , b \n  ,x\na,  \n");
}

#[test]
fn alternating_quoted_and_unquoted_fields() {
    let src = "a,\"b,1\",c,\"d\"\"e\",f\n\"g\nh\",i,\"j\",k,\"\"\n";
    assert_eq!(
        rows(src, b','),
        vec![
            vec!["a", "b,1", "c", "d\"e", "f"],
            vec!["g\nh", "i", "j", "k", ""],
        ]
    );
    agree_csv(src);
}

#[test]
fn a_quote_inside_an_unquoted_field_is_literal() {
    assert_eq!(csv("a\"b,c\n"), vec![vec!["a\"b", "c"]]);
    agree_csv("a\"b,c\n");
}

// -- line endings and the BOM -------------------------------------------------

#[test]
fn crlf_rows() {
    assert_eq!(csv("a,b\r\nc,d\r\n"), vec![vec!["a", "b"], vec!["c", "d"]]);
    assert_eq!(record_ends(b"a,b\r\nc,d\r\n", b','), vec![5, 10], "the terminator is two bytes");
    agree_csv("a,b\r\nc,d\r\n");
}

#[test]
fn bare_cr_rows() {
    assert_eq!(csv("a,b\rc,d\r"), vec![vec!["a", "b"], vec!["c", "d"]]);
    assert_eq!(record_ends(b"a,b\rc,d\r", b','), vec![4, 8]);
    agree_csv("a,b\rc,d\r");
}

#[test]
fn mixed_line_endings_in_one_file() {
    let src = "a\r\nb\nc\rd";
    assert_eq!(rows(src, b','), vec![vec!["a"], vec!["b"], vec!["c"], vec!["d"]]);
    agree_csv(src);
}

#[test]
fn a_cr_at_end_of_file_closes_the_row() {
    assert_eq!(csv("a\r"), vec![vec!["a"]]);
    assert_eq!(scanner_ends(b"a\r", b','), vec![2]);
    agree_csv("a\r");
}

#[test]
fn a_lone_cr_is_an_empty_row() {
    assert_eq!(csv("\r\n\r"), vec![vec![""], vec![""]]);
    agree_csv("\r\n\r");
}

#[test]
fn bom_is_stripped_from_the_first_field_only() {
    let src = b"\xef\xbb\xbfa,b\nc,\xef\xbb\xbfd\n";
    assert_eq!(bom_len(src), 3);
    assert_eq!(strip_bom(src), &src[3..]);
    assert_eq!(bom_len(b"a"), 0);
    assert_eq!(records(src, b','), vec![vec!["a", "b"], vec!["c", "\u{feff}d"]]);
    assert_eq!(record_ends(src, b','), vec![7, 14]);
    agree(src, b',');
}

// -- ragged rows --------------------------------------------------------------

#[test]
fn ragged_rows_report_their_true_arity() {
    let got = csv("a,b,c\n1,2\n1,2,3,4\n");
    assert_eq!(got.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![3, 2, 4]);
    assert_eq!(got[2], vec!["1", "2", "3", "4"], "nothing is discarded");
    agree_csv("a,b,c\n1,2\n1,2,3,4\n");
}

#[test]
fn fit_pads_short_rows_and_reports_the_overflow() {
    let mut short = vec!["a".to_string()];
    assert_eq!(fit(&mut short, 3), 0);
    assert_eq!(short, vec!["a", "", ""]);

    let mut long: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
    assert_eq!(fit(&mut long, 2), 2, "the caller is told what overflows");
    assert_eq!(long.len(), 4, "and the data is still there");

    let mut none: Vec<String> = Vec::new();
    assert_eq!(fit(&mut none, 0), 0);
    assert!(none.is_empty());
}

#[test]
fn the_tail_says_whether_a_last_row_exists_and_whether_it_is_terminated() {
    // Three different endings the row index has to tell apart. `has_row`
    // decides the row count; `terminated` decides how many bytes are data.
    let cases: &[(&[u8], Tail)] = &[
        (b"", Tail::None),
        (b"a\n", Tail::None),
        (b"a\r\n", Tail::None),
        (b"a", Tail::Open),
        (b"\"a\n", Tail::Open),
        (b"\"a\r", Tail::Open),
        (b"a\r", Tail::Cr),
        (b"a\n\rb\r", Tail::Cr),
    ];
    for (src, want) in cases {
        let mut sc = Scanner::new(b',');
        scan_row_ends(&mut sc, src, 0, |_| {});
        assert_eq!(finish_row_end(&mut sc), *want, "{src:?}");
    }
    assert!(!Tail::None.has_row() && Tail::None.terminated());
    assert!(Tail::Open.has_row() && !Tail::Open.terminated());
    assert!(Tail::Cr.has_row() && Tail::Cr.terminated());
}

#[test]
fn a_row_slice_with_no_bytes_is_one_empty_field() {
    // `record` is told these bytes are a row, so a blank line is a row with one
    // empty field — the same thing the whole-file parse counts it as. `records`
    // over an empty *file* still sees no rows at all.
    assert_eq!(record(b"", b','), vec![String::new()]);
    assert_eq!(record(b"\n", b','), vec![String::new()]);
    assert_eq!(records(b"", b','), Vec::<Vec<String>>::new());
    assert_eq!(records(b"\n\n", b','), vec![vec![String::new()], vec![String::new()]]);
}

// -- malformed input ----------------------------------------------------------

#[test]
fn an_unterminated_quote_runs_to_eof() {
    assert_eq!(csv("a,\"b,c\n1,2"), vec![vec!["a", "b,c\n1,2"]]);
    assert_eq!(csv("\""), vec![vec![""]]);
    agree_csv("a,\"b,c\n1,2");
    agree_csv("\"");
}

#[test]
fn a_stray_quote_closes_the_field_and_the_tail_is_literal() {
    assert_eq!(csv("\"a\"b\",c\n"), vec![vec!["ab\"", "c"]]);
    assert_eq!(csv("\"a\" x,b\n"), vec![vec!["a x", "b"]]);
    agree_csv("\"a\"b\",c\n\"a\" x,b\n");
}

#[test]
fn nul_and_control_bytes_are_content() {
    let src = b"a\x00b,c\x07\n\x1b[31m,x\n";
    let got = records(src, b',');
    assert_eq!(got, vec![vec!["a\0b", "c\u{7}"], vec!["\u{1b}[31m", "x"]]);
    agree(src, b',');
}

#[test]
fn invalid_utf8_is_replaced_not_rejected() {
    let src = b"a\xffb,\xc3\n\xf0\x9f\x92\xa9,ok\n";
    let got = records(src, b',');
    assert_eq!(got[0], vec!["a\u{fffd}b", "\u{fffd}"]);
    assert_eq!(got[1], vec!["\u{1f4a9}", "ok"]);
    agree(src, b',');
}

#[test]
fn a_multibyte_char_is_never_split_by_the_machine() {
    let src = "名前,値\n日本語,2\n";
    assert_eq!(rows(src, b','), vec![vec!["名前", "値"], vec!["日本語", "2"]]);
    agree_csv(src);
}

#[test]
fn every_single_byte_input_is_survivable() {
    for b in 0u8..=255 {
        let src = [b];
        for &d in &CANDIDATES {
            let recs = records(&src, d);
            assert!(recs.len() <= 1);
            assert_eq!(scanner_ends(&src, d).len(), recs.len());
        }
    }
}

// -- size ---------------------------------------------------------------------

#[test]
fn one_ten_megabyte_quoted_field() {
    const N: usize = 10 * 1024 * 1024;
    let mut src = Vec::with_capacity(N + 8);
    src.push(b'"');
    src.extend(std::iter::repeat(b'x').take(N));
    src.extend_from_slice(b"\",b\n");
    let recs: Vec<Record> = Records::new(&src, b',').collect();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].fields.len(), 2);
    assert_eq!(recs[0].fields[0].len(), N);
    assert_eq!(recs[0].end, src.len());
    assert_eq!(scanner_ends(&src, b','), vec![src.len() as u64]);
}

#[test]
fn ten_thousand_columns() {
    let src: String = (0..10_000).map(|i| format!("c{i},")).collect::<String>() + "last\n";
    let got = rows(&src, b',');
    assert_eq!(got[0].len(), 10_001);
}

// -- the machine itself -------------------------------------------------------

#[test]
fn scanner_reports_when_a_newline_would_be_content() {
    let mut sc = Scanner::new(b',');
    assert!(sc.at_row_start() && !sc.in_quotes());
    for b in b"a,\"x" {
        sc.step(*b);
    }
    assert!(sc.in_quotes(), "inside quotes a newline is data");
    assert!(!sc.at_row_start());
    sc.step(b'"');
    assert!(!sc.in_quotes(), "the closing quote leaves the quoted field");
    assert_eq!(sc.step(b'\n').event, Event::EndRow);
    assert!(sc.at_row_start());
    assert_eq!(sc.delim(), b',');
}

#[test]
fn a_bare_cr_ends_the_row_before_the_next_byte() {
    let mut sc = Scanner::new(b',');
    assert_eq!(sc.step(b'a').event, Event::Continue);
    assert_eq!(sc.step(b'\r').event, Event::Continue, "the CR alone decides nothing");
    let step = sc.step(b'b');
    assert_eq!(step.event, Event::EndRowBefore, "so `b` starts the next row");
    assert_eq!(step.push, None);
    assert_eq!(sc.step(b'b').event, Event::Continue, "and is re-fed");
}

#[test]
fn finish_is_idempotent_and_empty_after_a_terminator() {
    let mut sc = Scanner::new(b',');
    for b in b"a,b\n" {
        sc.step(*b);
    }
    assert_eq!(sc.finish(), None);
    assert_eq!(sc.finish(), None);

    let mut sc = Scanner::new(b',');
    sc.step(b'a');
    assert_eq!(sc.finish().map(|s| s.event), Some(Event::EndRow));
    assert_eq!(sc.finish(), None, "the row is not reported twice");
}

// The property-style corpus lives next door, so neither file outgrows the
// 500-line limit; it reuses the helpers above.
#[path = "parse_props.rs"]
mod props;
