//! [`RowIndex`] / [`RowStore`] unit tests.
//!
//! Everything is generated at run time into the temp dir and deleted on drop:
//! the point of these tests is behaviour on files nobody would check in, up to
//! and including one over 100MB. The two properties that matter most are that
//! *opening indexes nothing* and that the offsets agree byte for byte with
//! what the shared state machine says when it sees the whole file at once —
//! two implementations of the quote rules would diverge, and every offset
//! after the divergence would be wrong.
#![deny(unsafe_code)]

use std::path::PathBuf;

use super::*;
use crate::csv::parse;
use crate::csv::read::MAX_ROW_BYTES;

const COMMA: u8 = b',';

/// A temp file that removes itself.
struct Tmp {
    path: PathBuf,
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn tmp_path(name: &str) -> Tmp {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("tread-idx-{}-{nanos}-{name}.csv", std::process::id()));
    Tmp { path: p }
}

fn tmp(name: &str, body: &[u8]) -> Tmp {
    let t = tmp_path(name);
    std::fs::write(&t.path, body).expect("write fixture");
    t
}

fn store(t: &Tmp) -> RowStore {
    RowStore::open(&t.path, COMMA).expect("open fixture")
}

/// Row `i`'s text, for a fixture small enough to assert on whole.
fn row(s: &mut RowStore, i: usize) -> Option<String> {
    s.row(i).map(|sp| String::from_utf8_lossy(&sp.data).into_owned())
}

/// Offsets the shared machine reports when it sees the whole file at once —
/// the reference the lazy, windowed index must match exactly.
fn reference(bytes: &[u8]) -> Vec<u64> {
    let mut sc = parse::Scanner::new(COMMA);
    let origin = parse::bom_len(bytes) as u64;
    if bytes.len() as u64 <= origin {
        return Vec::new();
    }
    let mut out = vec![origin];
    let end = bytes.len() as u64;
    parse::scan_row_ends(&mut sc, &bytes[origin as usize..], origin, |at| {
        if at < end {
            out.push(at);
        }
    });
    out
}

fn all_offsets(s: &mut RowStore) -> Vec<u64> {
    s.scan_all(&mut |_| false);
    (0..s.known()).map(|i| s.index.offset(i).expect("indexed offset")).collect()
}

/// Assert the lazy index agrees with the whole-file reference.
fn agrees(name: &str, body: &[u8]) -> RowStore {
    let t = tmp(name, body);
    let mut s = store(&t);
    assert_eq!(all_offsets(&mut s), reference(body), "{name}: offsets diverged");
    s
}

// -- shape ------------------------------------------------------------------

#[test]
fn plain_rows() {
    let mut s = agrees("plain", b"a,b\n1,2\n3,4\n");
    assert_eq!(s.known(), 3);
    assert!(s.complete());
    assert_eq!(s.index.total(), Some(3));
    assert_eq!(row(&mut s, 0).as_deref(), Some("a,b"));
    assert_eq!(row(&mut s, 2).as_deref(), Some("3,4"));
    assert_eq!(row(&mut s, 3), None);
}

#[test]
fn no_trailing_newline_still_has_a_last_row() {
    let mut s = agrees("no-eol", b"a,b\n1,2");
    assert_eq!(s.known(), 2);
    assert_eq!(row(&mut s, 1).as_deref(), Some("1,2"));
}

#[test]
fn a_file_that_is_one_row() {
    let mut s = agrees("one-row", b"just,one,row");
    assert_eq!(s.known(), 1);
    assert_eq!(row(&mut s, 0).as_deref(), Some("just,one,row"));
    assert_eq!(row(&mut s, 1), None);
}

#[test]
fn an_empty_file_has_no_rows() {
    let t = tmp("empty", b"");
    let mut s = store(&t);
    assert_eq!(s.ensure(10), 0);
    assert!(s.complete());
    assert_eq!(s.index.total(), Some(0));
    assert_eq!(row(&mut s, 0), None);
    assert_eq!(s.progress().percent(), 100);
}

#[test]
fn a_file_that_is_only_a_bom_has_no_rows() {
    let t = tmp("bom-only", &parse::BOM);
    let mut s = store(&t);
    assert_eq!(s.ensure(10), 0);
    assert!(s.complete());
}

#[test]
fn the_bom_is_not_part_of_row_zero() {
    let mut body = parse::BOM.to_vec();
    body.extend_from_slice(b"a,b\n1,2\n");
    let mut s = agrees("bom", &body);
    assert_eq!(s.index.origin(), 3);
    assert_eq!(row(&mut s, 0).as_deref(), Some("a,b"));
}

#[test]
fn crlf_and_bare_cr_terminators() {
    let mut s = agrees("crlf", b"a,b\r\n1,2\r\n3,4\r");
    assert_eq!(s.known(), 3);
    assert_eq!(row(&mut s, 0).as_deref(), Some("a,b"));
    assert_eq!(row(&mut s, 2).as_deref(), Some("3,4"));
    let mut s = agrees("bare-cr", b"a,b\r1,2\r");
    assert_eq!(s.known(), 2);
    assert_eq!(row(&mut s, 1).as_deref(), Some("1,2"));
}

#[test]
fn blank_rows_are_rows() {
    let mut s = agrees("blank", b"a\n\n\nb\n");
    assert_eq!(s.known(), 4);
    assert_eq!(row(&mut s, 1).as_deref(), Some(""));
}

// -- quoting ----------------------------------------------------------------

#[test]
fn a_newline_inside_quotes_is_not_a_row_boundary() {
    let body = b"a,b\n\"line one\nline two\",x\nlast,row\n";
    let mut s = agrees("quoted-nl", body);
    assert_eq!(s.known(), 3);
    assert_eq!(row(&mut s, 1).as_deref(), Some("\"line one\nline two\",x"));
    assert_eq!(row(&mut s, 2).as_deref(), Some("last,row"));
}

#[test]
fn doubled_quotes_and_embedded_crlf() {
    let body = b"h\n\"say \"\"hi\"\"\r\nand bye\"\r\ntail\r\n";
    let mut s = agrees("quoted-escapes", body);
    assert_eq!(s.known(), 3);
    assert_eq!(row(&mut s, 2).as_deref(), Some("tail"));
}

#[test]
fn an_unterminated_quote_at_eof_is_one_row() {
    let mut s = agrees("unterminated", b"a\n\"open,forever\nand ever\n");
    assert_eq!(s.known(), 2);
    assert!(row(&mut s, 1).is_some());
}

#[test]
fn a_trailing_newline_inside_quotes_is_data_not_a_terminator() {
    // The last row runs to EOF, so its final `LF` is inside the quoted field
    // and belongs to the row. Stripping it by shape would hand the field
    // parser one byte less than the whole-file parse sees.
    let mut s = agrees("quoted-lf-eof", b"h\n\"trailing\n\"");
    assert_eq!(s.known(), 2);
    assert_eq!(row(&mut s, 1).as_deref(), Some("\"trailing\n\""));
    assert!(!s.index.terminated(1), "a row that ran off the end has no terminator");

    let mut s = agrees("open-lf-eof", b"h\n\"never closed\n");
    assert_eq!(row(&mut s, 1).as_deref(), Some("\"never closed\n"));

    // A bare `CR` at EOF is the other side of it: that one *is* a terminator
    // and must still come off, even though the row is not settled.
    let mut s = agrees("bare-cr-eof", b"h\na\r");
    assert_eq!(row(&mut s, 1).as_deref(), Some("a"));
    assert!(s.index.terminated(1));
}

#[test]
fn embedded_nuls_are_content() {
    let mut s = agrees("nul", b"a,b\n\0,\0\n");
    assert_eq!(s.known(), 2);
    assert_eq!(s.row(1).expect("row").data, b"\0,\0");
}

// -- window boundaries ------------------------------------------------------

#[test]
fn a_crlf_split_across_the_window_is_one_terminator() {
    let mut body = vec![b'x'; WINDOW - 1];
    body.extend_from_slice(b"\r\nb\n");
    let mut s = agrees("split-crlf", &body);
    assert_eq!(s.known(), 2);
    assert_eq!(s.index.offset(1), Some(WINDOW as u64 + 1));
    assert_eq!(row(&mut s, 1).as_deref(), Some("b"));
}

#[test]
fn a_quoted_field_spanning_windows_holds_its_state() {
    let mut body = vec![b'"'];
    body.extend_from_slice(&vec![b'y'; WINDOW - 2]);
    body.extend_from_slice(b"\nz\"\nafter\n");
    let mut s = agrees("split-quote", &body);
    assert_eq!(s.known(), 2, "the quoted newline became a row boundary");
    assert_eq!(row(&mut s, 1).as_deref(), Some("after"));
}

#[test]
fn a_row_longer_than_the_window_is_indexed_without_buffering_it() {
    let mut body = Vec::new();
    body.extend_from_slice(b"head\n");
    body.extend_from_slice(&vec![b'z'; MAX_ROW_BYTES + WINDOW]);
    body.extend_from_slice(b"\ntail\n");
    let mut s = agrees("long-row", &body);
    assert_eq!(s.known(), 3);
    assert_eq!(row(&mut s, 2).as_deref(), Some("tail"));
    let long = s.row(1).expect("long row");
    assert_eq!(long.data.len(), MAX_ROW_BYTES);
    assert!(long.truncated, "a row past the cap must say so");
}

#[test]
fn generated_files_agree_with_the_whole_file_scan() {
    // Enough rows to cross several windows, with every quoting shape mixed in
    // and each `pad` shifting the whole file against the window grid.
    for pad in 0..4 {
        let body = gen(9_000, pad);
        agrees(&format!("gen{pad}"), &body);
    }
}

/// A CSV with quoted newlines, doubled quotes, CRLF and bare-CR endings, in a
/// deterministic pseudo-random mix. `pad` shifts every offset.
fn gen(rows: usize, pad: usize) -> Vec<u8> {
    let mut out = vec![b'p'; pad];
    if pad > 0 {
        out.push(b'\n');
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    for i in 0..rows {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let pick = (seed >> 33) % 5;
        let cell: Vec<u8> = match pick {
            0 => format!("plain{i},{i},ok").into_bytes(),
            1 => format!("\"emb,edded\nnewline {i}\",{i},ok").into_bytes(),
            2 => format!("\"he said \"\"{i}\"\"\",{i},ok").into_bytes(),
            3 => format!("\" leading,{i}\",\"\",ok").into_bytes(),
            _ => format!("{i},,\"tail\r\nwrap\"").into_bytes(),
        };
        out.extend_from_slice(&cell);
        match pick {
            1 => out.extend_from_slice(b"\r\n"),
            4 => out.push(b'\r'),
            _ => out.push(b'\n'),
        }
    }
    out
}

#[path = "index_big_tests.rs"]
mod big;
