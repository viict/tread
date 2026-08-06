//! The pager driving a CSV: the pinned header, column scrolling, `w`, the
//! status bar and the three yanks (SPEC.md §CSV, "Reading affordances").
//!
//! These are pager tests, not format tests — everything below goes through key
//! presses and the painted frame, so they check the *seam*: that a format that
//! pins rows, scrolls by columns and counts in rows gets all three without the
//! pager knowing what a CSV is.
#![deny(unsafe_code)]

use super::*;
use crate::source::csv::{CsvSource, HEAD_ROWS};

const ROWS: usize = 12;

fn csv_body() -> String {
    let mut s = String::from("id,name,city\n");
    for i in 1..=40 {
        s.push_str(&format!("{i},person{i},town{i}\n"));
    }
    s
}

fn csv_pager(body: &str, cols: usize, rows: usize) -> Pager {
    let src = CsvSource::from_bytes(body.as_bytes().to_vec(), None);
    Pager::new(Box::new(src), "people.csv".into(), cols, rows, None)
}

/// The painted frame as one string per terminal row.
///
/// A pager frame is positioned with `CSI r;cH` rather than newlines, so the
/// rows are recovered by following those moves — which is also a check that
/// the painter puts each row where it says it does.
fn frame_rows(p: &mut Pager) -> Vec<String> {
    let mut f = Frame::new(true);
    p.paint(&mut f);
    let mut rows = vec![String::new(); p.rows.max(1)];
    let mut at = 0usize;
    let mut cs = f.as_str().chars().peekable();
    while let Some(c) = cs.next() {
        if c != '\u{1b}' {
            let last = rows.len() - 1;
            rows[at.min(last)].push(c);
            continue;
        }
        let mut params = String::new();
        if cs.peek() == Some(&'[') {
            cs.next();
        }
        for c in cs.by_ref() {
            if c.is_ascii_alphabetic() {
                if c == 'H' {
                    let row: usize = params.split(';').next().unwrap_or("1").parse().unwrap_or(1);
                    at = row.saturating_sub(1);
                }
                break;
            }
            params.push(c);
        }
    }
    rows.iter().map(|r| r.trim_end().to_string()).collect()
}

#[test]
fn the_header_stays_on_screen_while_the_body_scrolls() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    let first = frame_rows(&mut p);
    assert!(first[1].contains("name"), "{:?}", first[1]);
    assert!(first[3].contains("person1"));
    press(&mut p, "GG");
    let last = frame_rows(&mut p);
    // Same three rows at the top, whatever the body is showing.
    assert_eq!(&last[..3], &first[..3]);
    assert!(last[3].contains("person"), "{:?}", last[3]);
    assert!(!last[3].contains("person1,"), "the body did scroll");
    // ... and the header is exactly as wide as the body rows it sits over.
    let w = |s: &str| crate::render::str_width(s);
    assert_eq!(w(&last[1]), w(&last[4]));
}

#[test]
fn the_cursor_never_enters_the_pinned_header() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    assert_eq!(p.cursor, HEAD_ROWS);
    press(&mut p, "kkkkk");
    assert_eq!(p.cursor, HEAD_ROWS, "k stopped at the first data row");
    press(&mut p, "g");
    assert_eq!(p.cursor, HEAD_ROWS);
    press(&mut p, "uu");
    assert_eq!(p.cursor, HEAD_ROWS);
    assert!(p.top >= HEAD_ROWS);
}

#[test]
fn scrolling_down_and_back_up_lands_where_it_started() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    let start = frame_rows(&mut p);
    press(&mut p, "  ");
    press(&mut p, "bb");
    assert_eq!(frame_rows(&mut p), start);
}

#[test]
fn h_and_l_move_by_column_and_the_header_moves_with_the_body() {
    // Narrow enough that the grid does not fit.
    let mut p = csv_pager(&csv_body(), 18, ROWS);
    let before = frame_rows(&mut p);
    assert!(before[1].contains("id"));
    press(&mut p, "l");
    let after = frame_rows(&mut p);
    assert!(after[1].contains("name"), "header did not scroll: {:?}", after[1]);
    // The header and the body are cut at the same place, which is the whole
    // point of pinning: the bars line up.
    // Display columns, not byte offsets: the grid is drawn in 3-byte glyphs.
    let bars = |s: &str, glyph: char| -> Vec<usize> {
        s.chars()
            .enumerate()
            .filter(|(_, c)| *c == glyph)
            .map(|(i, _)| i)
            .collect()
    };
    let sep = '\u{2502}';
    assert!(bars(&after[1], sep).len() >= 2);
    assert_eq!(bars(&after[1], sep), bars(&after[4], sep));
    // The top border's interior joins sit under the same bars.
    assert_eq!(
        bars(&after[0], '\u{252c}'),
        bars(&after[1], sep)
            .into_iter()
            .filter(|i| *i > 0 && *i + 1 < after[1].chars().count())
            .collect::<Vec<_>>()
    );
    press(&mut p, "hh");
    assert_eq!(frame_rows(&mut p)[1], before[1], "h came back");
}

#[test]
fn the_status_bar_names_the_row_the_total_and_the_column() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    let s = p.status_line();
    assert!(s.starts_with("people.csv"), "{s}");
    assert!(s.contains("row 1/40"), "{s}");
    assert!(s.ends_with("id"), "{s}");
    press(&mut p, "jjl");
    let s = p.status_line();
    assert!(s.contains("row 3/40"), "{s}");
    assert!(s.ends_with("name"), "{s}");
}

#[test]
fn w_widens_the_column_under_the_cursor() {
    let mut body = String::from("k,note\n");
    body.push_str("1,short\n");
    for i in 2..20 {
        body.push_str(&format!("{i},{}\n", "n".repeat(30)));
    }
    let mut p = csv_pager(&body, 80, ROWS);
    press(&mut p, "l");
    let before = frame_rows(&mut p)[4].clone();
    press(&mut p, "w");
    let after = frame_rows(&mut p)[4].clone();
    assert!(p.message.as_deref().unwrap_or("").contains("note"));
    assert!(
        crate::render::str_width(&after) >= crate::render::str_width(&before),
        "{before:?} -> {after:?}"
    );
    // Markdown has nothing to widen, and says so rather than doing nothing.
    let mut md = pager(DOC, 60, ROWS);
    press(&mut md, "w");
    assert_eq!(md.message.as_deref(), Some("nothing to widen here"));
}

#[test]
fn y_copies_the_cell_capital_y_the_row_and_c_the_column() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    press(&mut p, "jy");
    assert_eq!(p.peek_yank().map(|y| y.text), Some("2\n".to_string()));
    press(&mut p, "Y");
    assert_eq!(
        p.peek_yank().map(|y| y.text),
        Some("2,person2,town2\n".to_string())
    );
    press(&mut p, "lc");
    let col = p.peek_yank().expect("column").text;
    assert!(col.starts_with("name\nperson1\nperson2\n"), "{col:?}");
    // A visual selection yanks whole rows, as CSV.
    press(&mut p, "vjy");
    assert_eq!(
        p.peek_yank().map(|y| y.text),
        Some("2,person2,town2\n3,person3,town3\n".to_string())
    );
}

#[test]
fn search_moves_the_cursor_and_highlights_the_row() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    press(&mut p, "/person30");
    key(&mut p, Key::Enter);
    assert!(p.cursor_text().contains("person30"), "{}", p.cursor_text());
    assert_eq!(p.match_count(), 1);
    press(&mut p, "/nobody");
    key(&mut p, Key::Enter);
    assert!(p.status_line().contains("pattern not found"));
}

#[test]
fn a_csv_has_no_outline_no_links_and_nothing_to_fold() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    press(&mut p, "o");
    assert_eq!(p.mode, Mode::Normal, "there is no outline to open");
    assert_eq!(p.message.as_deref(), Some("no headings"));
    press(&mut p, "za");
    assert_eq!(p.message.as_deref(), Some("no heading here"));
    key(&mut p, Key::Tab);
    assert_eq!(p.message.as_deref(), Some("no further heading"));
    assert_eq!(p.link_count(), 0);
    assert_eq!(p.line_count(), HEAD_ROWS + 41);
}

#[test]
fn a_resize_keeps_the_header_pinned_and_the_cursor_put() {
    let mut p = csv_pager(&csv_body(), 60, ROWS);
    press(&mut p, "jjjjj");
    let row = p.cursor;
    let cell = p.cursor_text();
    p.resize(24, 8);
    assert_eq!(p.cursor, row, "the cursor stayed on its row");
    assert!(p.cursor_text().starts_with(&cell[..6]), "{}", p.cursor_text());
    let rows = frame_rows(&mut p);
    assert!(rows[1].contains("id"), "header still pinned: {:?}", rows[1]);
    assert!(rows[0].starts_with('\u{250c}'));
    // Narrower: the grid is re-fitted, so nothing is wider than the terminal
    // except by way of horizontal scrolling.
    assert!(p.line_count() > 0);
    p.resize(200, 40);
    assert!(frame_rows(&mut p)[1].contains("city"));
}

#[test]
fn an_empty_csv_paints_and_quits() {
    let mut p = csv_pager("", 40, ROWS);
    assert_eq!(p.line_count(), 0);
    let rows = frame_rows(&mut p);
    assert!(rows[0].is_empty());
    press(&mut p, "jkGgly");
    press(&mut p, "q");
    assert!(p.should_quit());
}

#[test]
fn a_terminal_too_short_for_the_header_still_paints() {
    for rows in 1..=4 {
        let mut p = csv_pager(&csv_body(), 40, rows);
        let painted = frame_rows(&mut p);
        assert!(painted.len() >= rows);
        press(&mut p, "jjGg");
        assert!(p.cursor < p.line_count().max(1));
    }
}

// -- `G` on a file whose end is not known yet --------------------------------

/// A body with more rows than the open path indexes, so its end is genuinely
/// unknown when `G` is pressed.
fn lazy_body(rows: usize) -> String {
    let mut s = String::from("id,name,city\n");
    for i in 1..=rows {
        s.push_str(&format!("{i},person{i},town{i}\n"));
    }
    s
}

#[test]
fn g_scans_to_the_real_end_instead_of_the_end_of_the_index() {
    const N: usize = 20_000;
    let mut p = csv_pager(&lazy_body(N), 60, ROWS);
    frame_rows(&mut p);
    let indexed = p.line_count();
    assert!(indexed < N, "the file was fully indexed on open: {indexed}");
    let before = p.cursor;

    press(&mut p, "G");
    // Not a jump into the middle of the file dressed up as the end.
    assert_eq!(p.cursor, before, "G moved before the end was known");
    let msg = p.status_line();
    assert!(msg.starts_with("scanning to end of file"), "{msg}");
    assert!(msg.contains('%'), "{msg}");

    // The idle tick drives it; a bounded number of them gets there.
    for _ in 0..200 {
        if p.cursor != before {
            break;
        }
        p.idle();
    }
    assert_eq!(p.cursor, HEAD_ROWS + N - 1, "G landed off the last data row");
    // (The cells themselves are truncated: the columns were sized from the
    // first thousand rows, where no id is five digits long. That is the
    // sampling trade SPEC.md §CSV makes, and `w` is the answer to it.)
    assert!(p.cursor_text().contains("person"), "{}", p.cursor_text());
    // The bottom border is below the cursor, not under it.
    assert_eq!(p.line_count(), HEAD_ROWS + N + 1);
    assert!(p.status_line().contains(&format!("row {N}/{N}")), "{}", p.status_line());
}

#[test]
fn a_key_press_stops_the_scan_and_q_never_waits_for_it() {
    let mut p = csv_pager(&lazy_body(20_000), 60, ROWS);
    frame_rows(&mut p);
    let before = p.cursor;
    press(&mut p, "G");
    press(&mut p, "j");
    assert_eq!(p.status_line(), "scan stopped");
    assert_eq!(p.cursor, before + 1, "the key that stopped the scan still acted");
    // Idle ticks keep indexing in the background, but the cursor stays put:
    // the abandoned scan does not come back and move it later.
    for _ in 0..50 {
        p.idle();
    }
    assert_eq!(p.cursor, before + 1);

    // And `q` mid-scan quits at once rather than waiting for the scan.
    let mut p = csv_pager(&lazy_body(20_000), 60, ROWS);
    frame_rows(&mut p);
    press(&mut p, "G");
    assert!(!p.should_quit());
    press(&mut p, "q");
    assert!(p.should_quit());
}

#[test]
fn g_on_a_fully_known_document_still_lands_at_once() {
    // Markdown, and a CSV small enough to be indexed on open: neither has
    // anything to scan, so `G` is the plain jump it always was.
    let mut p = super::pager("# a\n\ntext\n\n# b\n\nmore\n", 40, ROWS);
    press(&mut p, "G");
    assert_eq!(p.cursor, p.line_count() - 1);
    assert!(!p.status_line().contains("scanning"));

    let mut c = csv_pager(&csv_body(), 60, ROWS);
    frame_rows(&mut c);
    press(&mut c, "G");
    assert_eq!(c.cursor, HEAD_ROWS + 40 - 1);
    assert!(!c.status_line().contains("scanning"), "{}", c.status_line());
}


#[test]
fn width_wider_than_the_terminal_still_scrolls_by_column() {
    // `--width 200` in an 18-column terminal: the grid is laid out 200 wide and
    // only 18 of it is on screen, so `l` must keep walking the columns instead
    // of clamping the offset against a viewport that is not the one being
    // painted (SPEC.md §CSV, "h/l scroll by column").
    let src = CsvSource::from_bytes(csv_body().as_bytes().to_vec(), None);
    let mut p = Pager::new(Box::new(src), "people.csv".into(), 18, ROWS, Some(200));
    let before = frame_rows(&mut p);
    assert!(before[1].contains("id"));
    // Two columns right is off the right edge of an 18-column screen, so the
    // grid has to move under it.
    press(&mut p, "ll");
    assert!(p.hoff > 0, "the offset never moved: hoff {}", p.hoff);
    let after = frame_rows(&mut p);
    assert!(after[1].contains("city"), "last column unreachable: {:?}", after[1]);
    let bars = |s: &str| s.chars().filter(|c| *c == '\u{2502}').count();
    assert_eq!(bars(&after[1]), bars(&after[4]), "header and body disagree");
    press(&mut p, "hh");
    assert_eq!(p.hoff, 0);
    assert_eq!(frame_rows(&mut p)[1], before[1], "h came back");
}
