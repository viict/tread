//! The differential test between the two CSV layers.
//!
//! [`super::parse`] answers "what are this row's fields?" and [`super::index`]
//! answers "where does this row start?". They are supposed to be the *same*
//! state machine seen from two sides, so the only way to prove it is to run a
//! file through both and compare: once with [`parse::records`] over the whole
//! buffer, once with [`RowStore`] seeking to each indexed offset and
//! re-parsing that slice alone.
//!
//! Every disagreement this can find is a real corruption — a row boundary in
//! the wrong place shifts every offset after it, and a terminator stripped from
//! the wrong end silently eats a byte of data. The fixtures therefore lead with
//! the shapes that make the two sides drift: newlines inside quotes, doubled
//! quotes, `CRLF` and bare-`CR` endings, ragged arity, and a last row that runs
//! off the end of the file without a terminator.
#![deny(unsafe_code)]

use std::path::PathBuf;

use super::index::RowStore;
use super::parse;

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

fn tmp(name: &str, body: &[u8]) -> Tmp {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("tread-diff-{}-{nanos}-{name}.csv", std::process::id()));
    let t = Tmp { path: p };
    std::fs::write(&t.path, body).expect("write fixture");
    t
}

/// Rows as the field parser sees them: one pass over the whole buffer.
fn by_parser(body: &[u8]) -> Vec<Vec<String>> {
    parse::records(body, COMMA)
}

/// Rows as the viewport sees them: index the file, then fetch each row by its
/// byte offset and parse that slice on its own.
fn by_index(store: &mut RowStore) -> Vec<Vec<String>> {
    store.scan_all(&mut |_| false);
    (0..store.known())
        .map(|i| {
            let span = store.row(i).expect("an indexed row must be fetchable");
            parse::record(&span.data, COMMA)
        })
        .collect()
}

/// The assertion this whole file exists for.
fn differential(name: &str, body: &[u8]) {
    let t = tmp(name, body);
    let mut store = RowStore::open(&t.path, COMMA).expect("open fixture");
    let want = by_parser(body);
    let got = by_index(&mut store);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "{name}: row {i} diverged");
    }
    assert_eq!(got.len(), want.len(), "{name}: row count diverged");
}

// -- the shapes that make the two layers drift --------------------------------

#[test]
fn plain_rows_agree() {
    differential("plain", b"a,b,c\n1,2,3\n4,5,6\n");
}

#[test]
fn a_newline_inside_quotes_agrees() {
    differential("quoted-nl", b"h1,h2\n\"one\ntwo\",x\n\"a\nb\nc\",y\nlast,row\n");
}

#[test]
fn doubled_quotes_agree() {
    differential("doubled", b"h\n\"say \"\"hi\"\"\"\n\"\"\"\"\n\"a\"\"\"\ntail\n");
}

#[test]
fn crlf_and_bare_cr_agree() {
    differential("crlf", b"a,b\r\n1,2\r\n3,4\r5,6\r\n");
}

#[test]
fn a_quoted_crlf_is_content_not_two_rows() {
    differential("quoted-crlf", b"h\n\"wrap\r\nped\"\r\nafter\r\n");
}

#[test]
fn ragged_rows_agree() {
    differential("ragged", b"a,b,c\n1\n1,2,3,4,5\n\n1,2\n");
}

#[test]
fn a_last_row_without_a_terminator_agrees() {
    differential("no-eol", b"a,b\n1,2");
}

#[test]
fn a_last_row_whose_quoted_field_ends_in_a_newline_agrees() {
    // The trap: the row's final byte is an LF that lives *inside* quotes, so it
    // is data, not a terminator. Anything stripping a trailing newline by shape
    // rather than by what the parser consumed eats it.
    differential("quoted-lf-eof", b"h\n\"trailing\n\"");
    differential("quoted-cr-eof", b"h\n\"trailing\r\"");
}

#[test]
fn an_unterminated_quote_ending_in_a_newline_agrees() {
    differential("open-lf-eof", b"h\n\"never closed\n");
    differential("open-crlf-eof", b"h\n\"never closed\r\n");
    differential("open-cr-eof", b"h\n\"never closed\r");
}

#[test]
fn a_bom_does_not_shift_the_rows() {
    let mut body = parse::BOM.to_vec();
    body.extend_from_slice(b"a,b\n\"q\nq\",2\n3,4");
    differential("bom", &body);
}

#[test]
fn embedded_nuls_and_stray_quotes_agree() {
    differential("garbage", b"a,b\n\0,\"x\"y\"\nz\"w,\"\n\"\n");
}

#[test]
fn an_empty_file_agrees() {
    differential("empty", b"");
    differential("bom-only", &parse::BOM);
    differential("newline-only", b"\n");
    differential("blank-rows", b"\n\n\n");
}

// -- generated corpora --------------------------------------------------------

#[test]
fn generated_files_agree_row_for_row() {
    // Big enough to cross several read windows, and `pad` slides every offset
    // against the window grid so a boundary lands mid-terminator.
    for pad in 0..4 {
        let body = gen(4_000, pad, true);
        differential(&format!("gen{pad}"), &body);
    }
}

#[test]
fn generated_files_ending_mid_row_agree_row_for_row() {
    // Same corpus with the final terminator withheld: the last row runs to EOF,
    // which is where the terminator-stripping rule is easiest to get wrong.
    for pad in 0..4 {
        let body = gen(2_000, pad, false);
        differential(&format!("gen-open{pad}"), &body);
    }
}

/// A deterministic pseudo-random CSV mixing quoted newlines, doubled quotes,
/// `CRLF`, bare `CR` and ragged arity. `pad` shifts every offset; `closed` ends
/// the file on a terminator, otherwise the last row is left open.
fn gen(rows: usize, pad: usize, closed: bool) -> Vec<u8> {
    let mut out = vec![b'p'; pad];
    if pad > 0 {
        out.push(b'\n');
    }
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    for i in 0..rows {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let pick = (seed >> 33) % 7;
        let cell = match pick {
            0 => format!("plain{i},{i},ok"),
            1 => format!("\"emb,edded\nnewline {i}\",{i},ok"),
            2 => format!("\"he said \"\"{i}\"\"\",{i},ok"),
            3 => format!("\" leading,{i}\",\"\",ok"),
            4 => format!("{i},,\"tail\r\nwrap\""),
            5 => format!("short{i}"),
            _ => format!("wide{i},1,2,3,4,5,6"),
        };
        out.extend_from_slice(cell.as_bytes());
        if i + 1 == rows && !closed {
            break;
        }
        match pick {
            1 => out.extend_from_slice(b"\r\n"),
            4 => out.push(b'\r'),
            _ => out.push(b'\n'),
        }
    }
    out
}
