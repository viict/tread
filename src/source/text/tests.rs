//! What the plain-text source promises (SPEC.md §Plain text).
//!
//! Every assertion here is about *not inventing anything*: the rows are the
//! file's lines, the structure methods are empty, and the only transform
//! between a byte and a cell is the one that makes it safe to paint.
#![deny(unsafe_code)]

use super::*;
use crate::source::Source;

fn src(body: &str) -> TextSource {
    let mut s = TextSource::from_bytes(body.as_bytes().to_vec());
    s.set_width(80);
    s
}

/// Every row's painted text, top to bottom.
fn rows(s: &mut TextSource) -> Vec<String> {
    let n = s.len();
    s.lines(0..n).iter().map(|l| l.text()).collect()
}

/// Drive the lazy index to the end, the way the pager's idle tick does.
fn finish(s: &mut TextSource) {
    while s.extend() {}
}

// -- verbatim ---------------------------------------------------------------

#[test]
fn the_rows_are_the_lines_of_the_file() {
    let mut s = src("alpha\nbeta\ngamma\n");
    assert_eq!(s.len(), 3);
    assert_eq!(rows(&mut s), ["alpha", "beta", "gamma"]);
}

/// The whole point of the format: a shell script is not markdown.
#[test]
fn markdown_syntax_is_text_and_nothing_else() {
    let body = "#!/bin/sh\n# deploy the thing\n**not bold** [not a link](x.md)\n> not a quote\n";
    let mut s = src(body);
    assert_eq!(
        rows(&mut s),
        [
            "#!/bin/sh",
            "# deploy the thing",
            "**not bold** [not a link](x.md)",
            "> not a quote",
        ]
    );
    // No headings, no folds, no links: the honest empty answers.
    assert!(s.outline().is_empty());
    assert!(s.links().is_empty());
    assert_eq!(s.section_at(0), None);
    assert_eq!(s.hidden_at(0), None);
    assert_eq!(s.next_landmark(0, true), None);
    assert_eq!(s.next_landmark(2, false), None);
    assert_eq!(s.goto_id("deploy-the-thing"), None);
    assert!(!s.set_fold(0, true));
    assert!(s.folds().is_empty());
    // And asking to fold or unfold everything changes nothing.
    s.fold_all(true);
    s.fold_all(false);
    s.set_folds(vec!["whatever".to_string()]);
    assert_eq!(s.len(), 4);
}

/// A long line scrolls; it is never wrapped, whatever the width.
#[test]
fn a_long_line_is_one_scrollable_row_at_any_width() {
    let long = "x".repeat(500);
    let mut s = src(&format!("short\n{long}\n"));
    for w in [20, 80, 200] {
        s.set_width(w);
        let lines = s.lines(0..s.len());
        assert_eq!(lines.len(), 2, "width {w} re-flowed the file");
        assert_eq!(lines[1].text(), long);
        assert!(lines.iter().all(|l| l.scroll), "width {w}");
    }
}

// -- line endings -----------------------------------------------------------

/// No trailing newline, CRLF and a lone CR: three files that must each render
/// exactly their lines, with no phantom row and no visible `\r`.
#[test]
fn every_line_ending_renders_correctly() {
    let mut none = src("a\nb");
    assert_eq!(rows(&mut none), ["a", "b"], "no trailing newline");

    let mut crlf = src("a\r\nb\r\n");
    assert_eq!(rows(&mut crlf), ["a", "b"], "CRLF");

    let mut cr = src("a\rb\r");
    assert_eq!(rows(&mut cr), ["a", "b"], "lone CR");

    // A trailing terminator does not make an extra empty row, and a blank line
    // in the middle is a row of its own.
    let mut blanks = src("a\n\nb\n");
    assert_eq!(rows(&mut blanks), ["a", "", "b"]);

    let mut empty = src("");
    assert_eq!(empty.len(), 0);
    assert!(empty.lines(0..10).is_empty());
    assert_eq!(empty.anchor(0), None);
    assert_eq!(empty.reveal(Anchor(0)), None);
    assert_eq!(empty.locate(Mark(3)), None);
}

// -- painting ---------------------------------------------------------------

/// Tabs expand to the next 8-column stop, so indentation survives; every other
/// control character is the shared dot, so nothing reaches the terminal that
/// could move its cursor.
#[test]
fn tabs_expand_to_a_tab_stop_and_other_controls_are_dotted() {
    assert_eq!(display("\tif x:"), "        if x:");
    assert_eq!(display("a\tb"), "a       b");
    assert_eq!(display("abcdefg\tx"), "abcdefg x", "one column to the stop");
    assert_eq!(display("abcdefgh\tx"), "abcdefgh        x", "a full stop");
    assert_eq!(display("\t\tdeep"), "                deep");
    // A wide character costs two columns, so the next stop accounts for it.
    assert_eq!(display("\u{4e2d}\tx"), "\u{4e2d}      x");
    assert_eq!(crate::render::str_width(&display("\u{4e2d}\tx")), 9);

    assert_eq!(display("esc\u{1b}[31m"), "esc\u{b7}[31m");
    assert_eq!(display("nul\0"), "nul\u{b7}");
    assert_eq!(display("plain"), "plain");

    let mut s = src("\tindented\n\u{1b}[2J\n");
    let painted = rows(&mut s);
    assert_eq!(painted[0], "        indented");
    assert!(!painted[1].contains('\u{1b}'), "{:?}", painted[1]);
    assert_eq!(painted[1], "\u{b7}[2J");
}

#[test]
fn invalid_utf8_is_replaced_not_rejected() {
    let mut s = TextSource::from_bytes(b"good\n\xff\xfe bad\nend\n".to_vec());
    s.set_width(80);
    let painted = rows(&mut s);
    assert_eq!(painted.len(), 3);
    assert!(painted[1].contains('\u{fffd}'), "{:?}", painted[1]);
    assert_eq!(painted[2], "end");
}

/// A UTF-8 BOM is consumed, exactly as it is for every other format.
#[test]
fn a_byte_order_mark_is_not_content() {
    let mut s = TextSource::from_bytes(b"\xef\xbb\xbffirst\nsecond\n".to_vec());
    s.set_width(80);
    assert_eq!(rows(&mut s), ["first", "second"]);
}

// -- positions --------------------------------------------------------------

#[test]
fn positions_are_rows_and_never_panic_out_of_range() {
    let mut s = src("a\nb\nc\n");
    assert_eq!(s.anchor(2), Some(Anchor(2)));
    assert_eq!(s.anchor(3), None);
    assert_eq!(s.row_of(Anchor(1)), Some(1));
    assert_eq!(s.row_of(Anchor(99)), None);
    assert_eq!(s.reveal(Anchor(99)), Some(2), "clamped, not lost");
    assert_eq!(s.mark(0), Some(Mark(0)));
    assert_eq!(s.locate(Mark(99)), Some(2));
    // `lines` clamps and stays in agreement with `len`.
    assert_eq!(s.lines(2..99).len(), 1);
    assert_eq!(s.lines(99..200).len(), 0);
    // A backwards range is a normal input, not a panic. Spelled out rather
    // than as `2..1`, which is a compile-time lint.
    let backwards = std::ops::Range { start: 2, end: 1 };
    assert!(s.lines(backwards).is_empty());
    assert_eq!(s.line(1).map(|l| l.text()), Some("b".to_string()));
    assert_eq!(s.line(9), None);
}

#[test]
fn the_status_bar_counts_lines_and_says_when_it_is_still_counting() {
    let mut s = src("a\nb\nc\nd\n");
    finish(&mut s);
    assert_eq!(s.position_text(0).unwrap(), "0%  \u{b7}  line 1/4");
    assert_eq!(s.position_text(3).unwrap(), "100%  \u{b7}  line 4/4");
    assert_eq!(s.end(), End::At(3));

    // A file the index has not reached the end of reports `\u{2265}N`, never a
    // total it does not know (SPEC.md §CSV, the same contract).
    let big: String = (0..100_000).map(|i| format!("line {i}\n")).collect();
    let mut lazy = src(&big);
    assert!(!lazy.complete(), "set_width indexed the whole file");
    let text = lazy.position_text(0).unwrap();
    assert!(text.contains('\u{2265}') && text.contains("indexing"), "{text}");
    assert!(matches!(lazy.end(), End::Scanning(_)), "{:?}", lazy.end());
    finish(&mut lazy);
    assert_eq!(lazy.len(), 100_000);
    assert_eq!(lazy.end(), End::At(99_999));
    assert_eq!(lazy.position_text(99_999).unwrap(), "100%  \u{b7}  line 100000/100000");
}

/// Opening and painting the first screen must not depend on the file's size.
///
/// The bound is in *bytes*, not lines: the index advances a read window
/// ([`read::WINDOW`], 256KB) at a time and stops as soon as it has enough, so a
/// file of very short lines overshoots by up to one window's worth of them.
/// What matters is that the overshoot is a constant, and that the rest of a
/// 200k-line file is still unscanned after a screen has been painted.
#[test]
fn nothing_reads_the_whole_file_to_paint_one_screen() {
    let lines = 200_000;
    let big: String = (0..lines).map(|i| format!("line {i}\n")).collect();
    // The most lines one window can hold here: the shortest of them is
    // `line 0\n`.
    let window_lines = read::WINDOW / "line 0\n".len();
    let mut s = src(&big);
    assert!(!s.complete(), "set_width indexed the whole file");
    assert!(
        s.known() < FIRST_LINES + window_lines,
        "set_width indexed {} lines",
        s.known()
    );
    let before = s.known();
    s.lines(0..40);
    assert!(
        s.known() < before + LOOKAHEAD + window_lines,
        "painting one screen indexed {} more lines",
        s.known() - before
    );
    assert!(!s.complete(), "{} of {lines} lines", s.known());
}

// -- search -----------------------------------------------------------------

#[test]
fn search_walks_the_file_and_wraps() {
    let mut s = src("alpha\nbeta\ngamma\nbeta again\n");
    finish(&mut s);
    s.set_query("beta");
    let hit = s.preview_match(Anchor(0), Dir::Forward).unwrap();
    assert_eq!(hit.anchor, Anchor(1));
    assert!(!hit.wrapped);
    assert_eq!(s.match_count(), 1);
    assert_eq!(s.current_match(), Some(0));

    let next = s.cycle_match(Anchor(1), Dir::Forward).unwrap();
    assert_eq!(next.anchor, Anchor(3));
    assert!(!next.wrapped);
    let wrapped = s.cycle_match(Anchor(3), Dir::Forward).unwrap();
    assert_eq!(wrapped.anchor, Anchor(1));
    assert!(wrapped.wrapped, "the sweep came round again");

    let back = s.cycle_match(Anchor(1), Dir::Backward).unwrap();
    assert_eq!(back.anchor, Anchor(3));

    s.set_query("nowhere");
    assert!(s.cycle_match(Anchor(0), Dir::Forward).is_none());
    assert_eq!(s.match_count(), 0);
    assert!(s.matches_on(0).is_empty());
}

/// A capital letter makes the query case-sensitive, the shared rule.
#[test]
fn case_folding_is_the_shared_rule() {
    let mut s = src("Alpha\nalpha\n");
    finish(&mut s);
    s.set_query("alpha");
    assert_eq!(s.preview_match(Anchor(0), Dir::Forward).unwrap().anchor, Anchor(0));
    s.set_query("Alpha");
    assert_eq!(s.preview_match(Anchor(1), Dir::Forward).unwrap().anchor, Anchor(0));
}

/// Highlight columns are measured on the painted row, so a match after a tab
/// sits under the characters the reader can see.
#[test]
fn match_columns_are_the_painted_columns() {
    let mut s = src("\tneedle here\n");
    finish(&mut s);
    s.set_query("needle");
    s.preview_match(Anchor(0), Dir::Forward);
    let spans = s.matches_on(0);
    assert_eq!(spans.len(), 1);
    assert_eq!((spans[0].start, spans[0].end), (8, 14));
    assert!(spans[0].current);
}

// -- yank -------------------------------------------------------------------

/// Yanked text is the file's bytes: the tab is a tab, not the eight spaces the
/// screen shows. Sanitising is a display transform.
#[test]
fn yanks_are_verbatim() {
    let mut s = src("\tone\ntwo\nthree\n");
    finish(&mut s);

    let point = s.yank_point(0).unwrap();
    assert_eq!(point.text, "\tone\n");
    assert_eq!(point.what, "line 1");
    assert_eq!(s.yank_point(99), None);

    let range = s.yank_rows(0..3).unwrap();
    assert_eq!(range.text, "\tone\ntwo\nthree\n");
    assert_eq!(range.what, "3 lines");
    assert_eq!(s.yank_rows(1..2).unwrap().what, "1 line");
    assert_eq!(s.yank_rows(0..99).unwrap().what, "3 lines", "clamped");
    assert_eq!(s.yank_rows(99..100), None);

    // A text file has no sections and no blocks; `Y` and `c` say so.
    assert_eq!(s.yank_section(0), None);
    assert_eq!(s.yank_block(0), None);
    // And no row detail: `Enter` keeps whatever else it means.
    assert_eq!(s.detail(0), None);
}

// -- the seam ---------------------------------------------------------------

/// The trait's floor: no method panics on a row that does not exist, on an
/// empty document, or on a stale handle.
#[test]
fn no_method_panics_on_a_document_it_does_not_have() {
    for body in ["", "\n", "one line", "a\nb\n"] {
        let mut s = src(body);
        s.set_query("x");
        for row in [0usize, 1, 99, usize::MAX] {
            s.position_text(row);
            s.matches_on(row);
            s.yank_point(row);
            s.yank_section(row);
            s.yank_block(row);
            s.hidden_at(row);
            s.section_at(row);
            s.next_landmark(row, true);
            s.next_landmark(row, false);
            s.anchor(row);
            s.mark(row);
            s.row_of(Anchor(row));
            s.locate(Mark(row));
            s.reveal(Anchor(row));
            s.lines(row..row.saturating_add(3));
        }
        assert!(s.hscroll(0, 1, 80).is_none(), "h/l is the pager's own step");
        assert_eq!(s.widen(), None);
        assert_eq!(s.pinned(), 0);
        assert!(s.full_width());
    }
}

// -- one line indexer -------------------------------------------------------

/// A `"` is not a quote here, and the two record-per-line sources agree.
///
/// Both build their store with [`RowStore::lines`], which is the *only* line
/// indexer in the crate: text and `.jsonl` share it so that they cannot drift
/// apart, and it exists separately from [`RowStore::open`] because the CSV
/// grammar's quoting would make one unbalanced `"` swallow every line after it.
/// This pins both halves of that — the row count is the newline count, and it
/// is the same count the record reader sees for the same bytes — so a change
/// that routed either source through the CSV grammar fails here rather than
/// silently merging a log's lines.
#[test]
fn an_unbalanced_quote_does_not_swallow_the_rest_of_the_file() {
    let body = "say \"hello\nunbalanced \" here\nprintf '%s' \"x\ndone\n";
    let mut s = src(body);
    finish(&mut s);
    assert_eq!(s.len(), 4, "one row per newline, whatever the quotes say");
    assert_eq!(
        rows(&mut s),
        ["say \"hello", "unbalanced \" here", "printf '%s' \"x", "done"]
    );

    // The other caller of the same indexer, over the same bytes: a line that is
    // not JSON is an error row, but it is still exactly one row per line.
    let mut rec = crate::source::jsonl::JsonlSource::from_bytes(body.as_bytes().to_vec());
    rec.set_width(80);
    while rec.extend() {}
    assert_eq!(rec.len(), s.len(), "the two sources index the same lines");
}

// -- the big-file promise, for a file with no line breaks in it -------------

/// Regression, the module's own opening claim ("a 2GB log must open instantly
/// and quit instantly ... [`Source::lines`] reads only the rows it was asked to
/// paint") and SPEC.md §CSV, which it inherits verbatim.
///
/// This is the shape SPEC.md §Plain text made reachable: a file whose extension
/// names no parser is text, so `.bin`, `.zip`, `.mp4` and a minified one-line
/// bundle all arrive here — and none of them has a line terminator in its tail.
/// Painting the first (and only) row used to go through the *unbounded*
/// `RowIndex::ensure`, because proving there is no second row means scanning to
/// end-of-file. Measured through a real pty on a 2GB `.bin`: first frame at
/// 17.3s, `q` written at 2.0s and ignored, the whole 2GiB read.
///
/// What is asserted is that the bytes consumed are a function of the *budgets*
/// and not of the file: [`FIRST_LINES`]'s [`FRAME_BYTES`] on `set_width`, another
/// [`FRAME_BYTES`] for the lookahead in [`Source::lines`], and one
/// [`crate::csv::read::MAX_ROW_BYTES`] to settle the row — 9MB, whether the file
/// is 48MB or 2GB.
///
/// Held in memory rather than written to disk: the count comes from
/// [`crate::csv::index::RowStore::progress`] either way, and this keeps the test
/// fast. The on-disk equivalent, at 64MB through a real file, is
/// `csv::index::tests::big::painting_a_row_never_scans_a_file_that_holds_no_terminator`.
#[test]
fn a_file_with_no_line_break_paints_without_reading_all_of_it() {
    let size = 48 << 20;
    let mut s = TextSource::from_bytes(vec![b'x'; size]);
    s.set_width(80);
    let painted = s.lines(0..24);

    let read = s.store.borrow().progress().bytes;
    let budget = 2 * FRAME_BYTES + crate::csv::read::MAX_ROW_BYTES as u64 + read::WINDOW as u64;
    assert!(read <= budget, "read {read} bytes, budget {budget}");
    assert!(read * 4 < size as u64, "read {read} of {size} bytes to paint one screen");
    assert!(!s.complete(), "the file must not have been indexed to its end");
    assert_eq!(s.len(), 1, "one line, because there is one line");
    assert_eq!(painted.len(), 1);
    // The row is the file's bytes, clipped at the row cap — not the whole file,
    // and not empty.
    let text = painted[0].text();
    assert_eq!(text.len(), crate::csv::read::MAX_ROW_BYTES);
    assert!(text.bytes().all(|b| b == b'x'));
    // And it scrolls, as every text row does.
    assert!(painted[0].scroll);
}

/// The same file, but with a terminator at the end: nothing about the ordinary
/// case may have changed — the row is settled, so it is not reported as clipped
/// and its terminator is stripped.
#[test]
fn a_short_last_line_is_still_settled_and_whole() {
    let mut s = src("alpha\nomega\n");
    finish(&mut s);
    assert_eq!(rows(&mut s), ["alpha", "omega"]);
    let span = s.store.borrow_mut().row(1).expect("row 1");
    assert!(!span.truncated, "an ordinary last row is not clipped");
    assert_eq!(span.data, b"omega");
    // A file that ends *without* a terminator keeps its last row whole too.
    let mut t = src("alpha\nomega");
    finish(&mut t);
    assert_eq!(rows(&mut t), ["alpha", "omega"]);
    assert!(!t.store.borrow_mut().row(1).expect("row 1").truncated);
}
