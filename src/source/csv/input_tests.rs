//! [`CsvSource`] on the input it did not choose: hostile bytes, a record opened
//! as a form with `Enter`, and Excel's `sep=` directive.
//!
//! Split out of `tests.rs` — which covers the grid, laziness, scrolling, the
//! status text, the yanks and search — to keep both files under the size limit.
//! The helpers come from the parent module.
#![deny(unsafe_code)]

use super::tests::{all, src_from, text, SMALL};
use super::*;

// -- hostile input -----------------------------------------------------------

#[test]
fn malformed_input_degrades_and_never_panics() {
    let cases: Vec<String> = vec![
        "a,b\n\"unterminated,x\n1,2\n".to_string(),
        "a,b\r\n1,2\r\n".to_string(),
        "\u{feff}a,b\n1,2\n".to_string(),
        "a,b\n\"two\nlines\",x\n".to_string(),
        format!("a,b\n{},x\n", "z".repeat(200_000)),
        "a,b\n1,2,3,4,5\n6\n".to_string(),
        "a,b\n\0,\x07\n".to_string(),
        (0..2000).map(|i| format!("c{i}")).collect::<Vec<_>>().join(",") + "\n1\n",
    ];
    for body in cases {
        let mut s = src_from(&body);
        let n = s.len();
        let rows = s.lines(0..n + 5);
        let w = s.grid.total();
        for r in &rows {
            assert_eq!(r.width(), w, "{:?} in {:?}", r.text(), &body[..20.min(body.len())]);
            assert!(!r.text().contains('\n'));
        }
        // Every &self path is safe on every row, in range or not.
        for row in 0..n + 3 {
            let _ = s.position_text(row);
            let _ = s.matches_on(row);
            let _ = s.yank_point(row);
            let _ = s.yank_section(row);
        }
        let _ = s.yank_block(0);
        let _ = s.widen();
        let _ = s.hscroll(0, 1, 20);
    }
}

#[test]
fn a_tab_separated_file_is_sniffed_and_a_delimiter_can_be_forced() {
    let s = src_from("a\tb\n1\t2\n3\t4\n");
    assert_eq!(s.delim, b'\t');
    assert_eq!(s.yank_section(3).unwrap().text, "1\t2\n");
    // Forcing the wrong delimiter gives one wide column rather than an error.
    let mut forced = CsvSource::from_bytes(b"a,b\n1,2\n".to_vec(), Some(b';'));
    forced.set_width(80);
    assert_eq!(forced.grid.arity(), 1);
    assert!(text(&mut forced, 3).contains("1,2"));
}

#[test]
fn crlf_and_bom_leave_no_debris_in_the_cells() {
    let mut s = src_from("\u{feff}a,b\r\n1,2\r\n");
    assert_eq!(s.grid.name_of(0), Some("a"));
    assert_eq!(s.yank_section(3).unwrap().text, "1,2\n");
    assert!(!text(&mut s, 1).contains('\u{feff}'));
}

// -- the row detail (Enter) ---------------------------------------------------

/// The data a header-shaped grid cannot show must still be reachable. This is
/// the whole reason `Enter` opens a row.
#[test]
fn a_ragged_row_keeps_the_fields_the_grid_cannot_show() {
    let mut s = src_from("id,name\n1,alice\n2,bo,extra,more\n");
    // Row 2 is the second data row: 0 top, 1 header, 2 sep, 3 first data.
    let d = s.detail(4).expect("data row has a detail");
    assert_eq!(d.title, "Row 2");
    assert_eq!(
        d.fields,
        vec![
            ("id".to_string(), "2".to_string()),
            ("name".to_string(), "bo".to_string()),
            // Past the header, so named by position rather than dropped.
            ("[3]".to_string(), "extra".to_string()),
            ("[4]".to_string(), "more".to_string()),
        ]
    );
    // And the grid marks it, so the reader knows to press Enter.
    assert!(text(&mut s, 4).starts_with(crate::theme::MARKER_MORE));
    assert!(!text(&mut s, 3).starts_with(crate::theme::MARKER_MORE));
}

#[test]
fn every_row_opens_not_just_ragged_ones() {
    let s = src_from(SMALL);
    let d = s.detail(3).expect("first data row");
    assert_eq!(d.title, "Row 1");
    assert_eq!(d.fields.len(), 3);
    assert_eq!(d.fields[2], ("city".to_string(), "berlin".to_string()));
    assert_eq!(s.detail(1).expect("header row").title, "Header");
}

/// The model keeps the bytes; only painting makes them safe. That split is
/// what lets `y` in the form copy the real value rather than the dotted one.
#[test]
fn a_detail_keeps_the_raw_value_and_the_painter_makes_it_visible() {
    let s = src_from("a,b\n1,\"two\nlines\"\n");
    let d = s.detail(3).expect("detail");
    assert_eq!(d.fields[1].1, "two\nlines", "the model is verbatim");
    assert_eq!(
        crate::render::visible(&d.fields[1].1),
        "two\u{b7}lines",
        "and is safe once painted"
    );
}

#[test]
fn borders_and_separators_have_no_detail() {
    let s = src_from(SMALL);
    for row in [0, 2] {
        assert!(s.detail(row).is_none(), "row {row} is not a record");
    }
}

// -- the `sep=` directive -----------------------------------------------------

/// Excel writes `sep=;` ahead of the header. It names the delimiter, and it is
/// not a row: leaving it in place would make it the header.
#[test]
fn a_sep_directive_sets_the_delimiter_and_is_not_a_row() {
    let mut s = src_from("sep=;\nid;name\n1;alice\n2;bo\n");
    assert_eq!(s.delim, b';');
    let rows = all(&mut s);
    assert!(rows[1].contains("id") && rows[1].contains("name"), "{:?}", rows[1]);
    assert!(!rows.iter().any(|r| r.contains("sep=")), "the directive is not shown");
    // Two data rows, correctly split by the declared delimiter.
    assert_eq!(s.detail(3).expect("row 1").fields[1].1, "alice");
    assert_eq!(s.detail(4).expect("row 2").fields[1].1, "bo");
}

#[test]
fn a_sep_directive_survives_a_bom_and_crlf() {
    let s = src_from("\u{feff}sep=;\r\nid;name\r\n1;alice\r\n");
    assert_eq!(s.delim, b';');
    assert_eq!(s.detail(3).expect("row 1").fields[1].1, "alice");
}

/// `--delim` is the reader's override and beats what the file claims.
#[test]
fn an_explicit_delimiter_beats_the_directive() {
    let s = CsvSource::from_bytes(b"sep=;\na,b\n1,2\n".to_vec(), Some(b','));
    assert_eq!(s.delim, b',');
}

/// A column genuinely called `separator`, or a `sep=` with nothing after it,
/// is data and must be treated as such.
#[test]
fn a_row_that_merely_looks_like_a_directive_is_data() {
    let mut s = src_from("separator,x\n1,2\n");
    assert!(all(&mut s)[1].contains("separator"), "still the header");
}
