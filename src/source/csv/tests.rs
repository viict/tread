//! [`CsvSource`] against the trait's contract: the pinned header, the sampled
//! grid, column scrolling, `w`, the status text, the yanks and the promise
//! that a huge file opens without being read.
#![deny(unsafe_code)]

use std::time::Instant;

use super::*;
use crate::csv::read::tests::{tmp, Tmp};

fn src_from(body: &str) -> CsvSource {
    let mut s = CsvSource::from_bytes(body.as_bytes().to_vec(), None);
    s.set_width(80);
    s
}

fn text(s: &mut CsvSource, row: usize) -> String {
    s.lines(row..row + 1).pop().map(|l| l.text()).unwrap_or_default()
}

fn all(s: &mut CsvSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text()).collect()
}

const SMALL: &str = "id,name,city\n1,alice,berlin\n2,bo,rome\n3,carolina,oslo\n";

// -- the grid ----------------------------------------------------------------

#[test]
fn a_csv_looks_like_a_markdown_table() {
    let mut s = src_from(SMALL);
    let rows = all(&mut s);
    assert_eq!(rows[0], "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}");
    assert_eq!(rows[1], "\u{2502} id \u{2502} name     \u{2502} city   \u{2502}");
    assert!(rows[2].starts_with('\u{251c}'));
    assert_eq!(rows[3], "\u{2502} 1  \u{2502} alice    \u{2502} berlin \u{2502}");
    assert_eq!(rows[5], "\u{2502} 3  \u{2502} carolina \u{2502} oslo   \u{2502}");
    assert!(rows[6].starts_with('\u{2514}'), "{}", rows[6]);
    assert_eq!(s.len(), 7);
    // Every row is the same width, header and borders included.
    let w = s.lines(0..7)[0].width();
    assert!(s.lines(0..7).iter().all(|l| l.width() == w));
}

#[test]
fn the_header_is_pinned_and_the_cursor_never_enters_it() {
    let mut s = src_from(SMALL);
    assert_eq!(s.pinned(), HEAD_ROWS);
    assert!(s.full_width());
    // The pinned rows are rows 0..3 of the document, always available.
    let head = s.lines(0..HEAD_ROWS);
    assert_eq!(head.len(), 3);
    assert!(head[1].text().contains("name"));
}

#[test]
fn an_empty_file_is_an_empty_document() {
    let mut s = src_from("");
    assert_eq!(s.len(), 0);
    assert_eq!(s.pinned(), 0);
    assert!(s.lines(0..10).is_empty());
    assert!(s.anchor(0).is_none());
    assert!(s.yank_point(0).is_none());
    assert!(s.position_text(0).is_some());
}

#[test]
fn a_header_only_file_has_no_data_rows() {
    let mut s = src_from("a,b\n");
    assert_eq!(s.len(), HEAD_ROWS + 1);
    assert!(all(&mut s)[3].starts_with('\u{2514}'));
}

#[test]
fn widths_are_sampled_and_later_values_are_marked() {
    // A value far past the sample overflows its column and is truncated with a
    // visible marker rather than breaking the grid (SPEC.md §CSV).
    let mut body = String::from("k,v\n");
    for i in 0..grid::SAMPLE_ROWS + 5 {
        let v = if i == grid::SAMPLE_ROWS + 4 { "x".repeat(40) } else { "s".into() };
        body.push_str(&format!("{i},{v}\n"));
    }
    let mut s = src_from(&body);
    let last = s.len() - 2;
    let row = text(&mut s, last);
    assert!(row.contains(render::ELLIPSIS), "{row}");
    assert_eq!(s.lines(last..last + 1)[0].width(), s.lines(3..4)[0].width());
}

#[test]
fn cjk_sizes_by_display_width_not_bytes() {
    let mut s = src_from("k,v\n\u{4e2d}\u{6587},x\n");
    let rows = all(&mut s);
    assert_eq!(rows[3], "\u{2502} \u{4e2d}\u{6587} \u{2502} x \u{2502}");
    assert_eq!(rows[1], "\u{2502} k    \u{2502} v \u{2502}");
}

// -- laziness ----------------------------------------------------------------

/// A file too big to load: 400k rows, well over 10MB.
fn big() -> (Tmp, usize) {
    let mut body = String::from("id,name,note\n");
    for i in 0..400_000 {
        body.push_str(&format!("{i},name{i},a note about row {i}\n"));
    }
    let n = body.len();
    (tmp("source-big", body.as_bytes()), n)
}

#[test]
fn opening_a_huge_file_paints_a_screen_without_reading_it() {
    let (t, size) = big();
    assert!(size > 10 * 1024 * 1024);
    let start = Instant::now();
    let mut s = CsvSource::open(&t.path, None).expect("open");
    s.set_width(100);
    let window = s.lines(0..40);
    let open = start.elapsed();
    assert_eq!(window.len(), 40);
    assert!(window[3].text().contains("name0"));
    // Only the sample and the lookahead were touched, not the file.
    let scanned = s.store.borrow().progress().bytes;
    assert!(scanned * 4 < size as u64, "scanned {scanned} of {size}");
    assert!(!s.complete());
    assert!(open.as_millis() < 500, "opening took {open:?}");
}

#[test]
fn scrolling_extends_the_index_and_quitting_never_waits() {
    let (t, _) = big();
    let mut s = CsvSource::open(&t.path, None).expect("open");
    s.set_width(100);
    let first = s.len();
    // A page further down: still bounded work, and `len` has grown.
    let far = first + 500;
    let start = Instant::now();
    let window = s.lines(far..far + 40);
    assert_eq!(window.len(), 40);
    assert!(s.len() > first);
    assert!(start.elapsed().as_millis() < 500);
    // Each idle slice consumes a bounded number of bytes, whatever the file
    // size, which is what keeps the input loop responsive. (Bytes, not wall
    // time: a debug build is slow enough to make a clock assertion flaky.)
    let mut slices = 0;
    loop {
        let before = s.store.borrow().progress().bytes;
        let more = s.extend();
        let spent = s.store.borrow().progress().bytes - before;
        assert!(
            spent <= IDLE_BYTES + read::WINDOW as u64,
            "an idle slice scanned {spent} bytes"
        );
        slices += 1;
        if !more {
            break;
        }
    }
    assert!(slices > 1, "the whole file was indexed in one slice");
    assert!(s.complete());
}

#[test]
fn the_end_is_the_last_data_row_and_is_not_guessed_at() {
    // Small file: fully indexed by the time it is laid out, so `G` has an
    // answer straight away -- the last *data* row, not the bottom border.
    let s = src_from(SMALL);
    assert_eq!(s.end(), End::At(HEAD_ROWS + 2));
    assert!(matches!(s.kind(HEAD_ROWS + 2), Some(Kind::Data(2))));
    // Empty and header-only files have no data row to land on.
    assert_eq!(src_from("").end(), End::At(0));
    assert_eq!(src_from("a,b\n").end(), End::At(0));
}

#[test]
fn the_end_of_a_half_indexed_file_is_reported_as_a_scan_not_as_a_row() {
    let (t, _) = big();
    let mut s = CsvSource::open(&t.path, None).expect("open");
    s.set_width(100);
    s.lines(0..40);
    // The indexed prefix is *not* the end of the file, and saying so would put
    // the cursor in the middle of the document with no sign of it.
    let percent = match s.end() {
        End::Scanning(p) => p,
        other => panic!("{other:?}"),
    };
    assert!(percent < 100, "{percent}");
    assert!(!s.complete());
    while s.extend() {}
    assert_eq!(s.end(), End::At(HEAD_ROWS + 400_000 - 1));
    // And that row really is the last one the cursor can be on.
    assert!(matches!(s.kind(HEAD_ROWS + 400_000 - 1), Some(Kind::Data(_))));
    assert!(matches!(s.kind(HEAD_ROWS + 400_000), Some(Kind::Bottom)));
    assert!(s.kind(HEAD_ROWS + 400_001).is_none());
}

#[test]
fn the_bottom_border_does_not_claim_to_be_the_header() {
    let s = src_from(SMALL);
    let bottom = s.len() - 1;
    assert!(matches!(s.kind(bottom), Some(Kind::Bottom)));
    let text = s.position_text(bottom).unwrap();
    assert!(text.starts_with("end of 3 rows"), "{text}");
    assert!(s.position_text(1).unwrap().starts_with("header of 3"));
}

#[test]
fn the_total_is_honest_until_the_index_is_complete() {
    let (t, _) = big();
    let mut s = CsvSource::open(&t.path, None).expect("open");
    s.set_width(100);
    s.lines(0..40);
    let lazy = s.position_text(5).unwrap();
    assert!(lazy.starts_with("row 3/\u{2265}"), "{lazy}");
    assert!(lazy.contains("indexing"), "{lazy}");
    assert!(lazy.ends_with("id"), "{lazy}");
    while s.extend() {}
    let known = s.position_text(5).unwrap();
    assert_eq!(known, "row 3/400000  \u{b7}  id");
}

// -- horizontal scrolling ----------------------------------------------------

#[test]
fn h_and_l_move_a_whole_column_and_the_header_moves_with_it() {
    let mut s = src_from(SMALL);
    s.set_width(20);
    assert_eq!(s.col, 0);
    let off = s.hscroll(0, 1, 20).unwrap();
    assert_eq!(s.col, 1);
    // The offset lands on a column boundary of the grid, which is the same
    // grid the pinned header is drawn from — so they cannot drift.
    let bars: Vec<usize> = (0..3).map(|i| s.grid.bar(i)).collect();
    let right: Vec<usize> = (0..3)
        .map(|i| s.grid.bar(i) + s.grid.width_of(i) + grid::CELL_EXTRA + 1)
        .collect();
    assert!(
        bars.contains(&off) || right.iter().any(|r| *r == off + 20),
        "offset {off} is not a column edge: bars {bars:?} right {right:?}"
    );
    // Stepping back returns to the left edge, and never below zero.
    s.hscroll(off, -1, 20);
    assert_eq!(s.col, 0);
    assert_eq!(s.hscroll(0, -1, 20), Some(0));
    assert_eq!(s.col, 0);
    // Stepping right past the last column stays on it.
    for _ in 0..10 {
        let o = s.hscroll(s.grid.bar(s.col), 1, 20).unwrap();
        assert!(o < s.grid.total());
    }
    assert_eq!(s.col, 2);
}

#[test]
fn w_fits_the_column_to_what_is_on_screen() {
    let mut body = String::from("k,note\n");
    body.push_str("1,short\n");
    for i in 0..40 {
        body.push_str(&format!("{},{}\n", i + 2, "n".repeat(30)));
    }
    let mut s = src_from(&body);
    // Sampling saw everything here, so pin the column narrow first and then
    // let `w` grow it back from the visible rows.
    s.grid.set_fixed(1, 6, 80);
    s.lines(0..10);
    s.col = 1;
    let msg = s.widen().expect("widened");
    assert!(msg.contains("note"), "{msg}");
    assert_eq!(s.grid.width_of(1), 30);
    // Pressing it again on the same screen changes nothing: predictable.
    let before = s.grid.width_of(1);
    s.widen();
    assert_eq!(s.grid.width_of(1), before);
}

#[test]
fn widening_never_exceeds_the_viewport() {
    let mut s = src_from("k,v\n1,ok\n");
    s.set_width(20);
    s.col = 1;
    s.grid.set_fixed(1, 500, 20);
    // One column, its pads and both its bars still fit on screen; the grid as
    // a whole may of course be wider, which is what scrolling is for.
    assert!(s.grid.width_of(1) + grid::CELL_EXTRA < 20);
}

// -- status ------------------------------------------------------------------

#[test]
fn the_status_names_the_row_the_total_and_the_column() {
    let mut s = src_from(SMALL);
    s.lines(0..s.len());
    assert_eq!(s.position_text(3).unwrap(), "row 1/3  \u{b7}  id");
    assert_eq!(s.position_text(5).unwrap(), "row 3/3  \u{b7}  id");
    s.hscroll(0, 1, 80);
    assert!(s.position_text(3).unwrap().ends_with("name"));
    assert!(s.position_text(1).unwrap().starts_with("header"));
}

// -- yank --------------------------------------------------------------------

#[test]
fn y_yanks_the_cell_re_quoted() {
    let mut s = src_from("a,b\nplain,\"has,comma\"\n");
    s.lines(0..s.len());
    let cell = s.yank_point(3).unwrap();
    assert_eq!(cell.text, "plain\n");
    assert!(cell.what.contains('a'));
    s.hscroll(0, 1, 80);
    let quoted = s.yank_point(3).unwrap();
    assert_eq!(quoted.text, "\"has,comma\"\n");
}

#[test]
fn capital_y_yanks_the_row_as_valid_csv() {
    let s = src_from("a,b\nplain,\"has,comma\"\n");
    let y = s.yank_section(3).unwrap();
    assert_eq!(y.text, "plain,\"has,comma\"\n");
    assert_eq!(
        crate::csv::parse::records(y.text.as_bytes(), b','),
        vec![vec!["plain".to_string(), "has,comma".to_string()]]
    );
    // On the header, `Y` copies the header row.
    assert_eq!(s.yank_section(1).unwrap().text, "a,b\n");
}

#[test]
fn c_yanks_the_column_under_the_cursor() {
    let mut s = src_from("a,b\n1,x\n2,\"y,z\"\n");
    s.hscroll(0, 1, 80);
    let y = s.yank_block(3).unwrap();
    assert_eq!(y.text, "b\nx\n\"y,z\"\n");
    assert!(y.what.contains("column"), "{}", y.what);
}

#[test]
fn a_visual_selection_yanks_rows_as_csv() {
    let s = src_from(SMALL);
    let y = s.yank_rows(3..5).unwrap();
    assert_eq!(y.text, "1,alice,berlin\n2,bo,rome\n");
    assert_eq!(y.what, "2 rows");
    // A selection that reaches into the header carries the header.
    assert!(s.yank_rows(0..4).unwrap().text.starts_with("id,name,city\n"));
}

#[test]
fn a_yank_never_returns_the_display_form() {
    // The value is wider than its column, so the screen shows a marker; the
    // yank must not.
    let mut s = src_from("k\nshort\n");
    s.grid.set_fixed(0, 2, 80);
    assert!(text(&mut s, 3).contains(render::ELLIPSIS));
    assert_eq!(s.yank_section(3).unwrap().text, "short\n");
    assert_eq!(s.yank_point(3).unwrap().text, "short\n");
}

// -- search ------------------------------------------------------------------

#[test]
fn search_finds_a_row_and_highlights_it() {
    let mut s = src_from(SMALL);
    s.set_query("carolina");
    assert_eq!(s.match_count(), 0, "nothing has been swept yet");
    let hit = s.preview_match(Anchor(3), Dir::Forward).expect("hit");
    assert_eq!(hit.anchor, Anchor(5));
    assert_eq!(s.match_count(), 1);
    let spans = s.matches_on(5);
    assert_eq!(spans.len(), 1);
    assert!(spans[0].current);
    assert!(s.matches_on(3).is_empty());
}

#[test]
fn search_wraps_and_reports_a_miss() {
    let mut s = src_from(SMALL);
    s.set_query("alice");
    let hit = s.preview_match(Anchor(5), Dir::Forward).expect("hit");
    assert_eq!(hit.anchor, Anchor(3));
    assert!(hit.wrapped);
    s.set_query("nowhere");
    assert!(s.preview_match(Anchor(3), Dir::Forward).is_none());
    assert_eq!(s.match_count(), 0);
}

#[test]
fn search_cycles_through_every_hit() {
    let mut s = src_from("a\nx\ny\nx\n");
    s.set_query("x");
    let first = s.preview_match(Anchor(3), Dir::Forward).unwrap();
    assert_eq!(first.anchor, Anchor(3));
    let next = s.cycle_match(Anchor(3), Dir::Forward).unwrap();
    assert_eq!(next.anchor, Anchor(5));
    let back = s.cycle_match(Anchor(5), Dir::Backward).unwrap();
    assert_eq!(back.anchor, Anchor(3));
}

// -- the parts of the trait a CSV has no answer for --------------------------

#[test]
fn a_csv_has_no_sections_and_no_links() {
    let mut s = src_from(SMALL);
    assert!(s.outline().is_empty());
    assert!(s.links().is_empty());
    assert_eq!(s.section_at(4), None);
    assert!(!s.set_fold(0, true));
    s.fold_all(true);
    assert!(s.folds().is_empty());
    s.set_folds(vec!["x".to_string()]);
    assert_eq!(s.hidden_at(4), None);
    assert_eq!(s.next_landmark(3, true), None);
    assert_eq!(s.goto_id("x"), None);
    assert_eq!(s.len(), 7, "none of that changed the document");
}

#[test]
fn positions_are_stable_and_never_panic() {
    let mut s = src_from(SMALL);
    assert_eq!(s.anchor(4), Some(Anchor(4)));
    assert_eq!(s.anchor(99), None);
    assert_eq!(s.row_of(Anchor(4)), Some(4));
    assert_eq!(s.row_of(Anchor(99)), None);
    assert_eq!(s.reveal(Anchor(99)), Some(s.len() - 1));
    assert_eq!(s.mark(4), Some(Mark(4)));
    assert_eq!(s.locate(Mark(99)), Some(s.len() - 1));
    // A width change keeps the mark meaningful.
    s.set_width(30);
    assert_eq!(s.locate(Mark(4)), Some(4));
}

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
